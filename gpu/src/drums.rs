//! Drum detection: a small trained model on the analyzer's 1024-point spectrum, one frame every 10 ms.
//!
//! The older detector (`modulation::RiseHits`, kept for the test baseline) looks for steep rises in the log spectrum,
//! which any note attack has. A drum is different in shape (broadband, noise-like energy that arrives all at once, a kick
//! as a thump with a click on top) and the model learns that shape from labelled audio (`drum_train.rs`).
//!
//! Three outputs, independent so that a kick and a hat landing together are both reported:
//!
//! * `Low`: kick drums and toms
//! * `Snare`: snares, claps, rim shots
//! * `High`: hi-hats and cymbals
//!
//! The pieces: `Frontend` turns a spectrum into features (band levels in nats, their rise over one and two
//! hops, their contrast with a slow average, a few aggregates; a loudness follower keeps quiet recordings in
//! range), `Model` is a one-hidden-layer network, `Picker` turns the three probabilities into discrete hits
//! with a refractory time and one frame of confirmation. `DrumDetector` ties them together and is what
//! `analyzer::Dsp` runs on every hop.


pub const NB: usize = 16;
/// FFT bin ranges (47 Hz apart at 1024 points) of the 16 bands: single bins in the lows, about a third of an
/// octave above 1 kHz, up to 19.7 kHz.
const BAND_BINS: [(usize, usize); NB] =
    [(1, 1), (2, 2), (3, 3), (4, 5), (6, 8), (9, 12), (13, 18), (19, 26), (27, 38), (39, 55), (56, 80), (81, 115), (116, 160), (161, 220), (221, 300), (301, 420)];
/// Per band: the contrast with the slow average over the last four frames, then the frequency-median rise and
/// the share of bins that rose, for this frame and the one before. Then nine aggregates.
pub const PER_BAND: usize = 8;
pub const NF: usize = NB * PER_BAND + 9;
pub const BINS: usize = 513;
pub const HOP_S: f64 = 0.01;

/// Strength of the compression in front of the log: `ln(1 + GAIN * magnitude)`; the same constant the
/// analyzer's rise uses, with the magnitude scaled so a full-scale sine reads 1.
const GAIN: f32 = 2000.0;
/// Time constant of the slow per-band average the contrast is taken against.
const BG_TAU_S: f32 = 1.0;
/// The loudness follower: fast up, slow down. The spectrum is scaled so the music sits at `AGC_TARGET` whatever the
/// volume it is played at: quiet recordings are lifted by at most `AGC_MAX`, loud ones lowered by at most `AGC_MIN`.
const LOUD_UP_S: f32 = 0.05;
const LOUD_DOWN_S: f32 = 4.0;
const AGC_TARGET: f32 = 0.2;
const AGC_MAX: f32 = 40.0;
const AGC_MIN: f32 = 0.05;
/// Below this loudness (about -76 dB) nothing is reported.
const GATE: f32 = 1.5e-4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Low,
    Snare,
    High,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Low, Kind::Snare, Kind::High];
}

impl Default for Kind {
    fn default() -> Kind {
        Kind::Low
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DrumHit {
    /// Report time: the app-clock time of the newest sample the detector had seen.
    pub t: f64,
    pub kind: Kind,
    pub pos: f32,
    /// 0..1, about 1 for the hardest recent hit of its kind.
    pub strength: f32,
    /// The model's confidence.
    pub p: f32,
}

#[derive(Clone)]
pub struct Features {
    pub x: [f32; NF],
    /// Broadband loudness (amplitude units), for the gate.
    pub loud: f32,
    /// Positive rise over two hops, averaged over the low, middle and high bands (for strength).
    pub flux: [f32; 3],
    /// The same per band, for placing a hit along the frequency axis.
    pub band_flux: [f32; NB],
}

/// The bins the per-bin features look at (up to 19.7 kHz), and the half width of the median over frequency.
const TOP_BIN: usize = 420;
const MEDIAN_HALF: usize = 4;
/// A bin counts as having risen when its log magnitude is this much (nats) above its recent past.
const RISEN: f32 = 0.25;

pub struct Frontend {
    lv: [[f32; NB]; 4],
    bg: [f32; NB],
    /// Last frame's frequency-median rise and breadth per band.
    prev_pm: [f32; NB],
    prev_broad: [f32; NB],
    follow: f32,
    frames: u64,
    prev: [f32; 3],
    /// Log magnitude per bin, newest first: the last three frames.
    bins: [[f32; TOP_BIN + 1]; 3],
}

impl Frontend {
    pub fn new() -> Frontend {
        Frontend { lv: [[0.0; NB]; 4], bg: [0.0; NB], prev_pm: [0.0; NB], prev_broad: [0.0; NB], follow: 0.0, frames: 0, prev: [0.0; 3], bins: [[0.0; TOP_BIN + 1]; 3] }
    }

    /// `mag` is the spectrum's magnitude per FFT bin (513 of them), scaled so a full-scale sine reads about 1.
    pub fn push(&mut self, mag: &[f32]) -> Features {
        debug_assert!(mag.len() >= BINS);
        let power: f32 = mag[1..BINS].iter().map(|m| m * m).sum();
        let loud = power.sqrt();
        let a = |tau: f32| 1.0 - (-(HOP_S as f32) / tau).exp();
        self.follow += (loud - self.follow) * if loud > self.follow { a(LOUD_UP_S) } else { a(LOUD_DOWN_S) };
        let gain = GAIN * (AGC_TARGET / self.follow.max(1e-6)).clamp(AGC_MIN, AGC_MAX);
        self.lv.rotate_right(1);
        self.bins.rotate_right(1);
        for k in 1..=TOP_BIN {
            self.bins[0][k] = (1.0 + gain * mag[k]).ln();
        }
        for (b, (lo, hi)) in BAND_BINS.iter().enumerate() {
            self.lv[0][b] = self.bins[0][*lo..=*hi].iter().sum::<f32>() / (hi - lo + 1) as f32;
        }
        self.frames += 1;
        if self.frames == 1 {
            self.bg = self.lv[0];
            self.lv[1] = self.lv[0];
            self.lv[2] = self.lv[0];
            self.lv[3] = self.lv[0];
            self.bins[1] = self.bins[0];
            self.bins[2] = self.bins[0];
        }
        // how much each bin rose over its own last two frames, then the median of that across neighbouring
        // frequencies: a drum lifts a whole stretch of the spectrum at once, a note only its harmonics
        let mut rise = [0.0f32; TOP_BIN + 1];
        for k in 1..=TOP_BIN {
            rise[k] = (self.bins[0][k] - self.bins[1][k].max(self.bins[2][k])).max(0.0);
        }
        let mut pm = [0.0f32; NB];
        let mut broad = [0.0f32; NB];
        for (b, (lo, hi)) in BAND_BINS.iter().enumerate() {
            let (mut sm, mut nb) = (0.0, 0.0);
            for k in *lo..=*hi {
                let (a, z) = (k.saturating_sub(MEDIAN_HALF).max(1), (k + MEDIAN_HALF).min(TOP_BIN));
                let mut w = [0.0f32; 2 * MEDIAN_HALF + 1];
                let n = z - a + 1;
                w[..n].copy_from_slice(&rise[a..=z]);
                w[..n].sort_by(|p, q| p.partial_cmp(q).unwrap());
                sm += w[n / 2];
                if rise[k] > RISEN {
                    nb += 1.0;
                }
            }
            let cnt = (hi - lo + 1) as f32;
            pm[b] = sm / cnt;
            broad[b] = nb / cnt;
        }
        let mut x = [0.0f32; NF];
        let mut pos2 = [0.0f32; NB];
        let mut breadth = 0.0;
        for b in 0..NB {
            let o = PER_BAND * b;
            for j in 0..4 {
                x[o + j] = self.lv[j][b] - self.bg[b];
            }
            x[o + 4] = pm[b];
            x[o + 5] = broad[b];
            x[o + 6] = self.prev_pm[b];
            x[o + 7] = self.prev_broad[b];
            let d2 = self.lv[0][b] - self.lv[2][b];
            pos2[b] = d2.max(0.0);
            if d2 > 0.3 {
                breadth += 1.0;
            }
        }
        self.prev_pm = pm;
        self.prev_broad = broad;
        let mean = |r: std::ops::Range<usize>| pos2[r.clone()].iter().sum::<f32>() / r.len() as f32;
        let flux = [mean(0..5), mean(5..11), mean(11..NB)];
        let total = pos2.iter().sum::<f32>() / NB as f32;
        let o = NB * PER_BAND;
        x[o] = total;
        x[o + 1] = flux[0];
        x[o + 2] = flux[1];
        x[o + 3] = flux[2];
        x[o + 4] = breadth / NB as f32;
        x[o + 5] = (loud + 1e-4).ln();
        x[o + 6] = self.prev[0];
        x[o + 7] = self.prev[1];
        x[o + 8] = self.prev[2];
        self.prev = [total, flux[0], flux[2]];
        let k = a(BG_TAU_S);
        for b in 0..NB {
            self.bg[b] += (self.lv[0][b] - self.bg[b]) * k;
        }
        if self.frames < 3 {
            x = [0.0; NF];
        }
        Features { x, loud, flux, band_flux: pos2 }
    }
}

#[derive(Clone)]
pub struct Model {
    pub nh: usize,
    pub mean: Vec<f32>,
    pub inv_std: Vec<f32>,
    pub w1: Vec<f32>,
    pub b1: Vec<f32>,
    pub w2: Vec<f32>,
    pub b2: Vec<f32>,
}

fn sigmoid(z: f32) -> f32 {
    1.0 / (1.0 + (-z).exp())
}

/// The built-in ensemble: the networks and the picker's thresholds, in `drum_model.bin` (written by `drum_train.rs`).
/// Layout, little-endian: the number of models (u32), the thresholds (3 x f32), then for each model its hidden units (u32),
/// the feature means and 1/std (`NF` each), the first layer (hidden x `NF`) and its biases, the output layer (3 x hidden)
/// and its biases.
static MODEL_BYTES: &[u8] = include_bytes!("drum_model.bin");

pub fn builtin() -> (Vec<Model>, [f32; 3]) {
    // tests can score another model file without rebuilding: `GEQ_MODEL_FILE=/path/drum_model.bin cargo test ...`
    #[cfg(test)]
    if let Some(p) = std::env::var_os("GEQ_MODEL_FILE") {
        return parse_model(&std::fs::read(&p).unwrap_or_else(|e| panic!("{p:?}: {e}")));
    }
    parse_model(MODEL_BYTES)
}

fn parse_model(bytes: &[u8]) -> (Vec<Model>, [f32; 3]) {
    let mut at = 0usize;
    let u32_ = |at: &mut usize| {
        let v = u32::from_le_bytes(bytes[*at..*at + 4].try_into().unwrap());
        *at += 4;
        v as usize
    };
    let n = u32_(&mut at);
    let mut thr = [0.0f32; 3];
    for t in thr.iter_mut() {
        *t = f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        at += 4;
    }
    let floats = |at: &mut usize, k: usize| -> Vec<f32> {
        let v = (0..k).map(|i| f32::from_le_bytes(bytes[*at + 4 * i..*at + 4 * i + 4].try_into().unwrap())).collect();
        *at += 4 * k;
        v
    };
    let models = (0..n)
        .map(|_| {
            let nh = u32_(&mut at);
            Model { nh, mean: floats(&mut at, NF), inv_std: floats(&mut at, NF), w1: floats(&mut at, nh * NF), b1: floats(&mut at, nh), w2: floats(&mut at, 3 * nh), b2: floats(&mut at, 3) }
        })
        .collect();
    (models, thr)
}

impl Model {
    /// The bytes `builtin` reads for this model (without the file header).
    #[cfg(test)]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = (self.nh as u32).to_le_bytes().to_vec();
        for v in [&self.mean, &self.inv_std, &self.w1, &self.b1, &self.w2, &self.b2] {
            for x in v.iter() {
                b.extend_from_slice(&x.to_le_bytes());
            }
        }
        b
    }

    /// Hidden activations of one input (tanh), for the forward and the backward pass.
    pub fn hidden(&self, x: &[f32; NF], h: &mut [f32]) {
        for j in 0..self.nh {
            let mut s = self.b1[j];
            let row = &self.w1[j * NF..(j + 1) * NF];
            for i in 0..NF {
                s += row[i] * (x[i] - self.mean[i]) * self.inv_std[i];
            }
            h[j] = s.tanh();
        }
    }

    pub fn out(&self, h: &[f32]) -> [f32; 3] {
        let mut o = [0.0; 3];
        for c in 0..3 {
            let mut s = self.b2[c];
            for j in 0..self.nh {
                s += self.w2[c * self.nh + j] * h[j];
            }
            o[c] = sigmoid(s);
        }
        o
    }

    pub fn forward(&self, x: &[f32; NF]) -> [f32; 3] {
        let mut h = vec![0.0; self.nh];
        self.hidden(x, &mut h);
        self.out(&h)
    }
}

/// Turns the three per-frame probabilities into discrete hits.
pub struct Picker {
    /// Probability above which a hit of each kind starts, and the least time between two of the same kind.
    pub thr: [f32; 3],
    pub refractory: [f64; 3],
    last: [f64; 3],
    /// A hit that has crossed the threshold and waits one more frame to see whether it climbs: its probability,
    /// its flux and the flux per band.
    pending: [Option<(f32, f32, [f32; NB])>; 3],
    /// Hardest recent flux of each kind, the reference strength is judged against.
    reference: [f32; 3],
}

/// The bands each kind's position is taken from, and the stretch of the emitter line it lands on.
const PLACE: [((usize, usize), (f32, f32)); 3] = [((0, 7), (0.02, 0.34)), ((2, 13), (0.36, 0.74)), ((9, 15), (0.76, 0.99))];

impl Picker {
    pub fn new() -> Picker {
        Picker { thr: [0.7; 3], refractory: [0.06, 0.07, 0.045], last: [f64::NEG_INFINITY; 3], pending: [None; 3], reference: [REF_FLOOR; 3] }
    }

    pub fn update(&mut self, p: [f32; 3], f: &Features, t: f64, out: &mut Vec<DrumHit>) {
        let decay = (-(HOP_S as f32) / REF_TAU_S).exp();
        let gated = f.loud < GATE;
        for c in 0..3 {
            // the flux that measures this kind: the lows' for kicks, everything for snares, the highs' for hats
            let v = match c {
                0 => f.flux[0],
                1 => (f.flux[0] + f.flux[1] + f.flux[2]) / 3.0,
                _ => f.flux[2],
            };
            self.reference[c] = (self.reference[c] * decay).max(v).max(REF_FLOOR);
            if gated {
                self.pending[c] = None;
                continue;
            }
            match self.pending[c].take() {
                Some((p0, v0, mut bands)) => {
                    for b in 0..NB {
                        bands[b] = bands[b].max(f.band_flux[b]);
                    }
                    self.emit(c, p0.max(p[c]), v0.max(v), &bands, t, out);
                }
                None => {
                    if p[c] >= self.thr[c] && t - self.last[c] >= self.refractory[c] {
                        self.pending[c] = Some((p[c], v, f.band_flux));
                    }
                }
            }
        }
    }

    fn emit(&mut self, c: usize, p: f32, v: f32, bands: &[f32; NB], t: f64, out: &mut Vec<DrumHit>) {
        self.last[c] = t;
        let kind = Kind::ALL[c];
        let strength = (v / self.reference[c]).clamp(0.0, 1.0);
        // where the new energy sits within the kind's own bands: a tom sits right of a kick, a crash right of a hat
        let ((a, z), (lo, hi)) = PLACE[c];
        let (mut w, mut m) = (0.0f32, 0.0f32);
        for b in a..=z {
            w += bands[b];
            m += bands[b] * (b - a) as f32;
        }
        let frac = if w > 1e-6 { m / w / (z - a) as f32 } else { 0.5 };
        out.push(DrumHit { t, kind, pos: lo + (hi - lo) * frac.clamp(0.0, 1.0), strength, p });
    }
}

/// Reference flux of a kind never falls below this (nats), so a quiet stretch's small hits stay small, and
/// decays with this time constant.
const REF_FLOOR: f32 = 0.35;
const REF_TAU_S: f32 = 8.0;

pub struct DrumDetector {
    front: Frontend,
    /// The models whose probabilities are averaged (independently trained networks make different mistakes).
    pub models: Vec<Model>,
    pub picker: Picker,
    pub last: Features,
    pub probs: [f32; 3],
}

impl DrumDetector {
    pub fn new() -> DrumDetector {
        let (models, thr) = builtin();
        DrumDetector { front: Frontend::new(), models, picker: Picker { thr, ..Picker::new() }, last: Features { x: [0.0; NF], loud: 0.0, flux: [0.0; 3], band_flux: [0.0; NB] }, probs: [0.0; 3] }
    }

    /// One spectrum (see `Frontend::push`) at time `t`; pushes any hits found to `out`.
    pub fn push(&mut self, mag: &[f32], t: f64, out: &mut Vec<DrumHit>) {
        self.last = self.front.push(mag);
        let mut p = [0.0f32; 3];
        for m in &self.models {
            let q = m.forward(&self.last.x);
            for c in 0..3 {
                p[c] += q[c];
            }
        }
        let n = self.models.len() as f32;
        self.probs = [p[0] / n, p[1] / n, p[2] / n];
        self.picker.update(self.probs, &self.last, t, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::Dsp;
    use crate::corpus::{self, SR};
    use crate::drum_eval::{score, Det};

    /// The hits the analyzer's detector reports for some audio, fed in chunks of `chunk` frames, at or above the
    /// confidence the scenes ask for at Sensitivity 1.
    fn hits_of(audio: &[[f32; 2]], chunk: usize) -> Vec<DrumHit> {
        let mut dsp = Dsp::new();
        let mut out = Vec::new();
        let (mut l, mut r) = (Vec::new(), Vec::new());
        for (k, c) in audio.chunks(chunk).enumerate() {
            l.clear();
            r.clear();
            for f in c {
                l.push(f[0]);
                r.push(f[1]);
            }
            dsp.push(&l, &r, ((k + 1) * chunk).min(audio.len()) as f64 / SR);
            out.extend(dsp.take_drums().into_iter().filter(|h| h.p >= 0.9));
        }
        out
    }

    fn dets(h: &[DrumHit]) -> Vec<Det> {
        h.iter().map(|h| Det { t: h.t, pos: h.pos, strength: h.strength, class: h.kind as u8, p: h.p }).collect()
    }

    #[test]
    fn silence_and_a_steady_chord_report_nothing() {
        let floor = |i: usize| ((((i as u32).wrapping_mul(2654435761)) >> 16) as f32 / 65535.0 - 0.5) * 6e-4;
        let silence: Vec<[f32; 2]> = (0..6 * 48_000).map(|i| [floor(i), floor(i + 7)]).collect();
        assert!(hits_of(&silence, 480).is_empty());
        let chord: Vec<[f32; 2]> = (0..8 * 48_000)
            .map(|i| {
                let t = i as f32 / 48_000.0;
                let v = [110.0f32, 165.0, 220.0, 277.0, 330.0].iter().map(|f| 0.08 * (std::f32::consts::TAU * f * t).sin()).sum::<f32>() + floor(i);
                [v, v]
            })
            .collect();
        let h = hits_of(&chord, 480);
        assert!(h.is_empty(), "{} hits on a steady chord: {h:?}", h.len());
    }

    #[test]
    fn a_picker_reports_a_hit_once_and_then_holds_off() {
        let mut p = Picker::new();
        let f = Features { x: [0.0; NF], loud: 0.1, flux: [1.0, 1.0, 1.0], band_flux: [0.5; NB] };
        let mut out = Vec::new();
        // a kick's probability stays high for five frames (50 ms): one hit, reported a frame after it crossed
        for k in 0..5 {
            p.update([0.95, 0.0, 0.0], &f, 1.0 + k as f64 * 0.01, &mut out);
        }
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0].kind, Kind::Low);
        assert!((out[0].t - 1.01).abs() < 1e-9, "reported at the confirming frame, not later: {}", out[0].t);
        // and the next one comes once the refractory time has passed
        for k in 5..20 {
            p.update([0.95, 0.0, 0.0], &f, 1.0 + k as f64 * 0.01, &mut out);
        }
        assert!(out.len() >= 2 && out[1].t - out[0].t >= p.refractory[0] - 1e-9, "{out:?}");
        // a closed gate (silence) reports nothing
        let quiet = Features { loud: 1e-6, ..f.clone() };
        let mut none = Vec::new();
        let mut p = Picker::new();
        for k in 0..10 {
            p.update([0.99, 0.99, 0.99], &quiet, k as f64 * 0.01, &mut none);
        }
        assert!(none.is_empty());
    }

    #[test]
    fn the_kicks_and_snares_of_a_rock_song_are_found_on_time_and_in_the_right_place() {
        let Some(song) = corpus::song("rock_130", 24.0) else { return };
        let h = hits_of(&song.audio, 480);
        let rep = score(&song.labels, &dets(&h), 24.0, true);
        let (kick, snare) = (rep.by_kind[0], rep.by_kind[1]);
        assert!(kick.1 as f64 >= 0.85 * kick.0 as f64, "kicks found {}/{}", kick.1, kick.0);
        assert!(snare.1 as f64 >= 0.85 * snare.0 as f64, "snares found {}/{}", snare.1, snare.0);
        assert!(rep.precision() >= 0.8, "precision {:.2}: {}", rep.precision(), rep.line());
        assert!(rep.lat(0.5) <= 35.0 && rep.lat(0.9) <= 50.0, "latency {}", rep.line());
        // kicks sit left of snares, snares left of hats
        let mean = |k: Kind| {
            let v: Vec<f32> = h.iter().filter(|x| x.kind == k).map(|x| x.pos).collect();
            v.iter().sum::<f32>() / v.len().max(1) as f32
        };
        assert!(mean(Kind::Low) < mean(Kind::Snare) && mean(Kind::Snare) < mean(Kind::High));
    }

    #[test]
    fn music_without_drums_throws_few_hits() {
        let Some(song) = corpus::song("melodic_88", 24.0) else { return };
        let n = hits_of(&song.audio, 480).iter().filter(|h| h.t > 4.0).count();
        assert!(n <= 6, "{n} hits in 20 s of piano, strings and sax");
        let Some(voice) = corpus::song("speech_only", 24.0) else { return };
        let n = hits_of(&voice.audio, 480).iter().filter(|h| h.t > 4.0).count();
        assert!(n <= 6, "{n} hits in 20 s of speech");
    }

    #[test]
    fn quiet_recordings_are_still_heard() {
        let Some(song) = corpus::song("rock_130", 24.0) else { return };
        let quiet: Vec<[f32; 2]> = song.audio.iter().map(|f| [f[0] * 0.04, f[1] * 0.04]).collect(); // -28 dB
        let rep = score(&song.labels, &dets(&hits_of(&quiet, 480)), 24.0, true);
        assert!(rep.recall() >= 0.8 && rep.precision() >= 0.75, "at -28 dB: {}", rep.line());
    }

    #[test]
    fn the_hits_do_not_depend_on_how_the_audio_is_chunked() {
        use crate::synth::{Fixture, Synth};
        let mut synth = Synth::new(Fixture::Demo);
        let (mut l, mut r) = (Vec::new(), Vec::new());
        synth.render(8 * 48_000, &mut l, &mut r);
        let audio: Vec<[f32; 2]> = l.iter().zip(&r).map(|(a, b)| [*a, *b]).collect();
        let (a, b) = (hits_of(&audio, 480), hits_of(&audio, 1920));
        assert!(!a.is_empty());
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(&b) {
            assert_eq!(x.kind, y.kind);
            assert!((x.t - y.t).abs() < 1e-3, "{} vs {}", x.t, y.t);
        }
    }

    /// What one hop of the detector costs on this machine: `cargo test --release drum_cost -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn drum_cost() {
        let mut d = DrumDetector::new();
        let mut out = Vec::new();
        let mag: Vec<f32> = (0..BINS).map(|k| 0.01 / (1.0 + k as f32 * 0.05)).collect();
        let t0 = std::time::Instant::now();
        let n = 200_000;
        for k in 0..n {
            let mut m = mag.clone();
            m[(k * 7) % BINS] += 0.1;
            d.push(&m, k as f64 * 0.01, &mut out);
        }
        eprintln!("{:.1} us per 10 ms hop ({} hits)", t0.elapsed().as_secs_f64() * 1e6 / n as f64, out.len());
    }
}
