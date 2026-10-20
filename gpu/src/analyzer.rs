//! Audio analysis: an 8192-point FFT for band levels and log-frequency curves, plus a fast
//! 1024-point FFT for onset envelopes and stereo correlation. The DSP (`Dsp`) is independent of
//! where samples come from: the live `parec` thread, the synthetic source, or a frame-stepped
//! deterministic driver all feed it the same way.

use crate::bands;
use crate::clock;
use crate::drums::{DrumDetector, DrumHit};
use rustfft::{num_complex::Complex, Fft, FftPlanner};
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

pub const FFT_SIZE: usize = 8192;
pub const HOP: usize = 480; // 10 ms at 48 kHz -> ~100 updates/s
pub const CURVE_POINTS: usize = 192;
pub const FLOOR_DB: f32 = -140.0;
pub const RATE: f32 = bands::SAMPLE_RATE;
const CAL_DB: f32 = 3.01; // so that a sine at -6 dBFS reads -6 dB

/// Onset analysis window (21 ms) and its three frequency regions, as FFT bin ranges.
const FLUX_FFT: usize = 1024;
const FLUX_REGIONS: [(usize, usize); 3] = [(1, 5), (6, 85), (86, 330)]; // ~47-280 Hz, ~280 Hz-4 kHz, ~4-15.5 kHz
/// Typical flux (in nats of compressed magnitude, averaged over the region's bins) for a clear hit.
const FLUX_REF: [f32; 3] = [1.2, 0.45, 0.30];
/// Time constants of the onset envelopes' exponential decay (seconds).
pub const ENV_TAU: [f32; 3] = [0.14, 0.09, 0.06];
/// Minimum time between events in the same region, and the absolute floor for triggering.
const REFRACTORY: [f64; 3] = [0.11, 0.07, 0.05];
/// Tonal beating in sustained chords reaches ~0.7 of the low reference; hits reach 1.5-3.
const EVENT_FLOOR: [f32; 3] = [0.95, 0.55, 0.55];
/// After a hit the next one must exceed this fraction of the decaying peak, so a pitch glide
/// or a ringing tail is not mistaken for a second hit.
const PEAK_SUPPRESS: f32 = 0.6;
const PEAK_TAU: f32 = 0.18;
const STARTUP_SAMPLES: u64 = 24_000; // ignore the fill-up transient

/// A discrete event timestamped at the source, in the app clock domain (seconds).
#[derive(Clone, Copy, Debug)]
pub struct Onset {
    /// 0 low, 1 mid, 2 high.
    pub band: u8,
    /// About 1.0 for a typical clear hit.
    pub strength: f32,
    /// Flux-weighted position on the log-frequency axis, 0 (30 Hz) to 1 (20 kHz).
    pub centroid: f32,
    pub t: f64,
}

#[derive(Clone)]
pub struct Levels {
    pub bands: [f32; bands::COUNT],
    /// Waterfall curves: mixed signal, and the left / right channels on their own.
    pub curve: [f32; CURVE_POINTS],
    pub curve_l: [f32; CURVE_POINTS],
    pub curve_r: [f32; CURVE_POINTS],
    /// Onset envelopes (low, mid, high) as of `t`, and stereo correlation (-1..1).
    pub fx: [f32; 4],
    /// How steeply the level is rising at each waterfall bin right now, in nats per analysis hop (about
    /// 10 ms) of the log-compressed magnitude, with a fast attack and a ~50 ms fall so a hit stays visible
    /// for a few frames. A steady tone reads near 0; a hit reads 1-3 across the frequencies it excites.
    pub rise: [f32; CURVE_POINTS],
    /// App-clock time of the newest sample these levels include.
    pub t: f64,
    pub seq: u64,
}

impl Default for Levels {
    fn default() -> Self {
        Levels {
            bands: [FLOOR_DB; bands::COUNT],
            curve: [FLOOR_DB; CURVE_POINTS],
            curve_l: [FLOOR_DB; CURVE_POINTS],
            curve_r: [FLOOR_DB; CURVE_POINTS],
            fx: [0.0; 4],
            rise: [0.0; CURVE_POINTS],
            t: 0.0,
            seq: 0,
        }
    }
}

/// Width (in octaves, as f32 bits) of the analysis window behind each waterfall bin.
static CURVE_OCTAVES: AtomicU32 = AtomicU32::new(0x3eaaaaab); // 1/3 octave

pub fn set_curve_octaves(oct: f32) {
    CURVE_OCTAVES.store(oct.to_bits(), Ordering::Relaxed);
}

pub fn curve_freq(i: usize) -> f32 {
    30.0 * (20000.0f32 / 30.0).powf(i as f32 / (CURVE_POINTS as f32 - 1.0))
}

/// Position of a frequency on the 30 Hz - 20 kHz log axis (0..1).
fn log_pos(f: f32) -> f32 {
    ((f / 30.0).ln() / (20000.0f32 / 30.0).ln()).clamp(0.0, 1.0)
}

/// Time constant of the rise envelope's fall, seconds.
const RISE_TAU: f32 = 0.05;

/// The 1024-point FFT bins each waterfall bin listens to for a rise: its frequency plus or minus a sixth of
/// an octave. At the low end that is one or two bins (47 Hz apart), which is all the 21 ms window resolves.
fn rise_bins() -> Vec<(usize, usize)> {
    let hz = RATE / FLUX_FFT as f32;
    let w = 2f32.powf(1.0 / 6.0);
    (0..CURVE_POINTS)
        .map(|i| {
            let f = curve_freq(i);
            let lo = ((f / w / hz).floor() as usize).clamp(1, FLUX_FFT / 2);
            let hi = ((f * w / hz).ceil() as usize).clamp(lo, FLUX_FFT / 2);
            (lo, hi)
        })
        .collect()
}

/// Cumulative one-sided power spectrum (mean-square contribution per bin). Keep the sum in f64:
/// quiet treble bins must not round away after a loud bass bin has raised the running total.
fn cumulate(spec: &[Complex<f32>], norm: f32, cum: &mut [f64]) {
    cum[0] = 0.0;
    for k in 0..FFT_SIZE / 2 {
        cum[k + 1] = cum[k] + f64::from(spec[k].norm_sqr()) * f64::from(norm);
    }
}

pub struct Dsp {
    fft: Arc<dyn Fft<f32>>,
    fft_small: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    window_small: Vec<f32>,
    norm: f32,
    bin_hz: f32,
    band_edges: Vec<(f32, f32)>,
    half: f32,
    ring_l: Vec<f32>,
    ring_r: Vec<f32>,
    /// Samples consumed into the rings / pushed in total (consumed + still pending).
    consumed: u64,
    pushed: u64,
    pend_l: Vec<f32>,
    pend_r: Vec<f32>,
    spec_l: Vec<Complex<f32>>,
    spec_r: Vec<Complex<f32>>,
    spec_m: Vec<Complex<f32>>,
    cum_m: Vec<f64>,
    cum_l: Vec<f64>,
    cum_r: Vec<f64>,
    small: Vec<Complex<f32>>,
    prev_c: Vec<f32>,
    /// For each waterfall bin, the span of 1024-point FFT bins it listens to for a rise.
    rise_bins: Vec<(usize, usize)>,
    rise: [f32; CURVE_POINTS],
    env: [f32; 3],
    x_prev: [f32; 3],
    peak: [f32; 3],
    mean: [f32; 3],
    dev: [f32; 3],
    last_event: [f64; 3],
    corr: f32,
    seq: u64,
    /// Latest normalized flux per region (diagnostics and tests).
    pub last_x: [f32; 3],
    /// The drum detector, run on every hop, and the hits it has found since they were last taken.
    pub drums: DrumDetector,
    drum_hits: Vec<DrumHit>,
}

impl Dsp {
    pub fn new() -> Dsp {
        let mut planner = FftPlanner::<f32>::new();
        let hann = |n: usize| -> Vec<f32> {
            (0..n).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / n as f32).cos()).collect()
        };
        let window = hann(FFT_SIZE);
        let wsq: f32 = window.iter().map(|w| w * w).sum();
        Dsp {
            fft: planner.plan_fft_forward(FFT_SIZE),
            fft_small: planner.plan_fft_forward(FLUX_FFT),
            window,
            window_small: hann(FLUX_FFT),
            norm: 2.0 / (FFT_SIZE as f32 * wsq),
            bin_hz: RATE / FFT_SIZE as f32,
            band_edges: bands::FREQS.iter().map(|f| bands::edges(*f)).collect(),
            half: bands::step().sqrt(),
            ring_l: vec![0.0; FFT_SIZE],
            ring_r: vec![0.0; FFT_SIZE],
            consumed: 0,
            pushed: 0,
            pend_l: Vec::new(),
            pend_r: Vec::new(),
            spec_l: vec![Complex::new(0.0, 0.0); FFT_SIZE],
            spec_r: vec![Complex::new(0.0, 0.0); FFT_SIZE],
            spec_m: vec![Complex::new(0.0, 0.0); FFT_SIZE],
            cum_m: vec![0.0; FFT_SIZE / 2 + 1],
            cum_l: vec![0.0; FFT_SIZE / 2 + 1],
            cum_r: vec![0.0; FFT_SIZE / 2 + 1],
            small: vec![Complex::new(0.0, 0.0); FLUX_FFT],
            prev_c: vec![0.0; FLUX_FFT / 2 + 1],
            rise_bins: rise_bins(),
            rise: [0.0; CURVE_POINTS],
            env: [0.0; 3],
            x_prev: [0.0; 3],
            peak: [0.0; 3],
            mean: [0.0; 3],
            dev: [0.0; 3],
            last_event: [f64::NEG_INFINITY; 3],
            corr: 0.0,
            seq: 0,
            last_x: [0.0; 3],
            drums: DrumDetector::new(),
            drum_hits: Vec::new(),
        }
    }

    /// Feed new stereo samples. `t_end` is the app-clock time of the last sample in the slice.
    /// Returns updated levels (if at least one hop completed) and any onsets found.
    pub fn push(&mut self, l: &[f32], r: &[f32], t_end: f64) -> (Option<Levels>, Vec<Onset>) {
        self.pend_l.extend_from_slice(l);
        self.pend_r.extend_from_slice(r);
        self.pushed += l.len() as u64;
        let mut events = Vec::new();
        let mut hops = 0;
        while self.pend_l.len() >= HOP {
            let hl: Vec<f32> = self.pend_l.drain(..HOP).collect();
            let hr: Vec<f32> = self.pend_r.drain(..HOP).collect();
            for k in 0..HOP {
                let j = (self.consumed as usize + k) % FFT_SIZE;
                self.ring_l[j] = hl[k];
                self.ring_r[j] = hr[k];
            }
            self.consumed += HOP as u64;
            // time of the newest sample in this hop
            let t_hop = t_end - (self.pushed - self.consumed) as f64 / RATE as f64;
            self.hop_features(&hl, &hr, t_hop, &mut events);
            hops += 1;
        }
        if hops == 0 || self.consumed < FFT_SIZE as u64 {
            return (None, events);
        }
        let t_last = t_end - (self.pushed - self.consumed) as f64 / RATE as f64;
        (Some(self.spectra(t_last)), events)
    }

    /// Onset flux, envelopes, events and stereo correlation for one hop of new samples.
    fn hop_features(&mut self, hl: &[f32], hr: &[f32], t: f64, events: &mut Vec<Onset>) {
        // stereo correlation over this hop, smoothed over ~150 ms
        let (mut ll, mut rr, mut lr) = (0.0f32, 0.0f32, 0.0f32);
        for k in 0..hl.len() {
            ll += hl[k] * hl[k];
            rr += hr[k] * hr[k];
            lr += hl[k] * hr[k];
        }
        let n = hl.len() as f32;
        let target = if (ll + rr) / n > 1e-9 { (lr / (ll * rr).sqrt().max(1e-12)).clamp(-1.0, 1.0) } else { 0.0 };
        let a = 1.0 - (-(HOP as f32 / RATE) / 0.15).exp();
        self.corr += (target - self.corr) * a;

        // 1024-point spectrum of the newest samples (mono)
        for i in 0..FLUX_FFT {
            let j = (self.consumed as usize + FFT_SIZE - FLUX_FFT + i) % FFT_SIZE;
            self.small[i] = Complex::new((self.ring_l[j] + self.ring_r[j]) * 0.5 * self.window_small[i], 0.0);
        }
        self.fft_small.process(&mut self.small);
        let scale = 2.0 / (FLUX_FFT as f32 * 0.5);
        let hop_dt = HOP as f32 / RATE;
        let warm = self.consumed > STARTUP_SAMPLES;
        let mut c = [0f32; FLUX_FFT / 2 + 1];
        let mut mag = [0f32; FLUX_FFT / 2 + 1];
        for k in 0..=FLUX_FFT / 2 {
            mag[k] = self.small[k].norm() * scale;
            c[k] = (1.0 + 2000.0 * mag[k]).ln();
        }
        if warm {
            self.drums.push(&mag, t, &mut self.drum_hits);
        }
        // how steeply each frequency's level is rising: the positive change of the compressed magnitude since
        // the previous hop, largest over the bins a waterfall bin covers, held with a short fall
        let keep = (-hop_dt / RISE_TAU).exp();
        for (i, (lo, hi)) in self.rise_bins.iter().enumerate() {
            // the mean over the bins, not the largest: a hit lifts all of them together, while the jitter of
            // noise-like sounds (hats, cymbals) lifts some and drops others, and a maximum would keep the jitter
            let mut d = 0.0f32;
            for k in *lo..=*hi {
                d += (c[k] - self.prev_c[k]).max(0.0);
            }
            d /= (*hi - *lo + 1) as f32;
            self.rise[i] = if warm { (self.rise[i] * keep).max(d) } else { 0.0 };
        }
        for r in 0..3 {
            let (lo, hi) = FLUX_REGIONS[r];
            let (mut flux, mut wsum, mut psum) = (0.0f32, 0.0f32, 0.0f32);
            for k in lo..=hi {
                let d = (c[k] - self.prev_c[k]).max(0.0);
                flux += d;
                wsum += d;
                psum += d * log_pos(k as f32 * RATE / FLUX_FFT as f32);
            }
            let x = (flux / (hi - lo + 1) as f32 / FLUX_REF[r]).min(3.0);
            self.env[r] = if warm { (self.env[r] * (-hop_dt / ENV_TAU[r]).exp()).max(x) } else { 0.0 };

            // adaptive trigger: above the recent level by a margin, on the way up, past the refractory period
            let thr = EVENT_FLOOR[r].max(self.mean[r] + 2.0 * self.dev[r]).max(PEAK_SUPPRESS * self.peak[r]);
            self.peak[r] = (self.peak[r] * (-hop_dt / PEAK_TAU).exp()).max(0.0);
            if warm && x > thr && x > self.x_prev[r] * 0.9 && t - self.last_event[r] > REFRACTORY[r] {
                self.last_event[r] = t;
                events.push(Onset {
                    band: r as u8,
                    strength: x.clamp(0.3, 2.0),
                    centroid: if wsum > 0.0 { psum / wsum } else { 0.5 },
                    // the hit entered somewhere in this hop; the centre halves the worst-case error
                    t: t - 0.5 * hop_dt as f64,
                });
            }
            if warm {
                self.peak[r] = self.peak[r].max(x);
            }
            let k_stat = 1.0 - (-hop_dt / 1.5).exp();
            self.mean[r] += (x - self.mean[r]) * k_stat;
            self.dev[r] += ((x - self.mean[r]).abs() - self.dev[r]) * k_stat;
            self.x_prev[r] = x;
            self.last_x[r] = x;
        }
        self.prev_c.copy_from_slice(&c);
    }

    /// The drum hits found since the last call.
    pub fn take_drums(&mut self) -> Vec<DrumHit> {
        std::mem::take(&mut self.drum_hits)
    }

    /// Band levels and curves from the newest 8192 samples.
    fn spectra(&mut self, t: f64) -> Levels {
        for i in 0..FFT_SIZE {
            let j = (self.consumed as usize + i) % FFT_SIZE;
            self.spec_l[i] = Complex::new(self.ring_l[j] * self.window[i], 0.0);
            self.spec_r[i] = Complex::new(self.ring_r[j] * self.window[i], 0.0);
        }
        self.fft.process(&mut self.spec_l);
        self.fft.process(&mut self.spec_r);
        for i in 0..FFT_SIZE {
            self.spec_m[i] = (self.spec_l[i] + self.spec_r[i]) * 0.5; // FFT is linear: mono = (L+R)/2
        }
        cumulate(&self.spec_m, self.norm, &mut self.cum_m);
        cumulate(&self.spec_l, self.norm, &mut self.cum_l);
        cumulate(&self.spec_r, self.norm, &mut self.cum_r);

        let bin_hz = self.bin_hz;
        let energy = |cum: &[f64], f: f32| -> f64 {
            let pos = f64::from(f) / f64::from(bin_hz);
            let i = pos as usize;
            if i >= FFT_SIZE / 2 {
                return cum[FFT_SIZE / 2];
            }
            cum[i] + (pos - i as f64) * (cum[i + 1] - cum[i])
        };
        let db = |cum: &[f64], (lo, hi): (f32, f32), scale: f32| -> f32 {
            let e = (energy(cum, hi) - energy(cum, lo)) * f64::from(scale);
            if e > 1e-14 { (10.0 * e.log10() as f32 + CAL_DB).max(FLOOR_DB) } else { FLOOR_DB }
        };
        // Waterfall bins: window width is user-selectable; energy is scaled to what a
        // full band-wide window would collect so levels stay comparable with the bars.
        let w = (2f32.powf(f32::from_bits(CURVE_OCTAVES.load(Ordering::Relaxed)) * 0.5)).max(1.01);
        let curve_scale = (self.half - 1.0 / self.half) / (w - 1.0 / w);
        let mut lv = Levels::default();
        for (i, e) in self.band_edges.iter().enumerate() {
            lv.bands[i] = db(&self.cum_m, *e, 1.0);
        }
        for i in 0..CURVE_POINTS {
            let e = (curve_freq(i) / w, curve_freq(i) * w);
            lv.curve[i] = db(&self.cum_m, e, curve_scale);
            lv.curve_l[i] = db(&self.cum_l, e, curve_scale);
            lv.curve_r[i] = db(&self.cum_r, e, curve_scale);
        }
        self.seq += 1;
        lv.seq = self.seq;
        lv.t = t;
        lv.fx = [self.env[0], self.env[1], self.env[2], self.corr];
        lv.rise = self.rise;
        lv
    }
}

/// State shared between an analysis thread and the UI.
#[derive(Default)]
pub struct Shared {
    pub levels: Mutex<Levels>,
    pub events: Mutex<Vec<Onset>>,
    /// Drum hits found by the detector (`drums.rs`), oldest first, until the UI takes them.
    pub drums: Mutex<Vec<DrumHit>>,
}

impl Shared {
    pub fn publish(&self, lv: Option<Levels>, ev: Vec<Onset>, drums: Vec<DrumHit>) {
        if let Some(lv) = lv {
            *self.levels.lock().unwrap() = lv;
        }
        if !drums.is_empty() {
            let mut q = self.drums.lock().unwrap();
            q.extend(drums);
            let n = q.len();
            if n > 64 {
                q.drain(..n - 64);
            }
        }
        if !ev.is_empty() {
            let mut q = self.events.lock().unwrap();
            q.extend(ev);
            let n = q.len();
            if n > 64 {
                q.drain(..n - 64);
            }
        }
    }
}

pub struct Analyzer {
    child: Child,
    pub shared: Arc<Shared>,
}

impl Analyzer {
    pub fn start(device: &str) -> Result<Analyzer, String> {
        let mut child = Command::new("parec")
            .args([
                &format!("--device={device}"),
                "--format=float32le",
                "--rate=48000",
                "--channels=2",
                "--latency-msec=10",
                "--client-name=SpectraForgeEQ analyzer",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("could not launch parec: {e}"))?;
        let mut out = child.stdout.take().ok_or("no parec stdout")?;
        let shared = Arc::new(Shared::default());
        let sh = shared.clone();
        std::thread::spawn(move || {
            let mut dsp = Dsp::new();
            let mut buf = vec![0u8; HOP * 8];
            let mut pending: Vec<u8> = Vec::new();
            let (mut l, mut r) = (Vec::new(), Vec::new());
            loop {
                let n = match out.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let t = clock::now();
                pending.extend_from_slice(&buf[..n]);
                let frames = pending.len() / 8;
                l.clear();
                r.clear();
                for f in 0..frames {
                    let o = f * 8;
                    l.push(f32::from_le_bytes(pending[o..o + 4].try_into().unwrap()));
                    r.push(f32::from_le_bytes(pending[o + 4..o + 8].try_into().unwrap()));
                }
                pending.drain(..frames * 8);
                if frames == 0 {
                    continue;
                }
                let (lv, ev) = dsp.push(&l, &r, t);
                sh.publish(lv, ev, dsp.take_drums());
            }
        });
        Ok(Analyzer { child, shared })
    }
}

impl Drop for Analyzer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::{Fixture, Synth};

    /// Run a fixture through the DSP for `secs` and collect events.
    fn run(fx: Fixture, secs: f32) -> (Vec<Onset>, Levels) {
        let mut synth = Synth::new(fx);
        let mut dsp = Dsp::new();
        let (mut l, mut r) = (Vec::new(), Vec::new());
        let mut events = Vec::new();
        let mut last = Levels::default();
        let total = (secs * RATE) as usize;
        let mut done = 0;
        while done < total {
            let n = 800.min(total - done);
            l.clear();
            r.clear();
            synth.render(n, &mut l, &mut r);
            done += n;
            let (lv, ev) = dsp.push(&l, &r, done as f64 / RATE as f64);
            events.extend(ev);
            if let Some(lv) = lv {
                last = lv;
            }
        }
        (events, last)
    }

    #[test]
    fn quiet_treble_survives_a_loud_bass_in_the_same_signal() {
        // A quiet broadband component must retain its 16 kHz level when a strong bass tone is added.
        // Accumulating power in f32 can round away every high-frequency bin after the loud bass bins.
        let n = FFT_SIZE * 2;
        let mut seed = 7u32;
        let quiet: Vec<f32> = (0..n).map(|_| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            ((seed >> 8) as f32 / 16777216.0 * 2.0 - 1.0) * 0.0015
        }).collect();
        let mixed: Vec<f32> = quiet.iter().enumerate().map(|(i, x)| {
            x + (0.7 * (std::f64::consts::TAU * 130.0 * i as f64 / RATE as f64).sin()) as f32
        }).collect();
        let levels = |samples: &[f32]| {
            Dsp::new().push(samples, samples, n as f64 / RATE as f64).0.unwrap()
        };
        let reference = levels(&quiet);
        let actual = levels(&mixed);
        assert!(reference.bands[10] > -75.0, "the treble should be visible at an 80 dB range");
        assert!((actual.bands[10] - reference.bands[10]).abs() < 0.2,
            "16 kHz: quiet alone {:.2} dB, with bass {:.2} dB", reference.bands[10], actual.bands[10]);
        for curve in [&actual.curve, &actual.curve_l, &actual.curve_r] {
            assert!((curve[180] - reference.curve[180]).abs() < 0.2);
        }
    }

    #[test]
    #[ignore]
    fn dump_flux() {
        for fx in [Fixture::Tones, Fixture::Percussion, Fixture::Demo, Fixture::Dense] {
            let mut synth = Synth::new(fx);
            let mut dsp = Dsp::new();
            let (mut l, mut r) = (Vec::new(), Vec::new());
            let mut xs: Vec<[f32; 3]> = Vec::new();
            for i in 0..(12 * 100) {
                l.clear();
                r.clear();
                synth.render(480, &mut l, &mut r);
                dsp.push(&l, &r, i as f64 * 0.01);
                if i > 300 {
                    xs.push(dsp.last_x);
                }
            }
            for reg in 0..3 {
                let mut v: Vec<f32> = xs.iter().map(|x| x[reg]).collect();
                v.sort_by(|a, b| a.partial_cmp(b).unwrap());
                let n = v.len();
                eprintln!("{:?} region {reg}: median {:.3} p90 {:.3} p99 {:.3} max {:.3}", fx, v[n / 2], v[n * 9 / 10], v[n * 99 / 100], v[n - 1]);
            }
        }
    }

    /// The live path: parec -> Dsp thread -> shared state. Needs a running PipeWire/PulseAudio
    /// server, so it is ignored by default (`cargo test -- --ignored live_capture`). Read-only: it
    /// opens a monitor stream and changes no routing.
    #[test]
    #[ignore]
    fn live_capture_smoke() {
        let a = Analyzer::start("@DEFAULT_MONITOR@").expect("parec");
        std::thread::sleep(std::time::Duration::from_millis(2500));
        let lv = a.shared.levels.lock().unwrap().clone();
        assert!(lv.seq > 50, "only {} analysis frames in 2.5 s", lv.seq);
        assert!(lv.t > 0.0);
        eprintln!("live: {} frames, bands[0..3] = {:?} dB, corr {:.2}", lv.seq, &lv.bands[..3], lv.fx[3]);
    }

    /// Run `secs` of a signal made by `signal(sample_index)` through the DSP and return the last `Levels`.
    fn rise_after(secs: f32, signal: impl Fn(usize) -> f32) -> Levels {
        let mut dsp = Dsp::new();
        let mut last = Levels::default();
        let total = (secs * RATE) as usize;
        let mut i = 0;
        while i < total {
            let n = 480.min(total - i);
            let l: Vec<f32> = (i..i + n).map(&signal).collect();
            let (lv, _) = dsp.push(&l, &l, (i + n) as f64 / RATE as f64);
            if let Some(lv) = lv {
                last = lv;
            }
            i += n;
        }
        last
    }

    #[test]
    fn a_steady_tone_does_not_rise_but_its_start_does_at_its_own_frequency() {
        // with a noise floor like a real recording's: onsets from exact digital silence splatter everywhere
        let floor = |i: usize| ((((i as u32).wrapping_mul(2654435761)) >> 16) as f32 / 65535.0 - 0.5) * 6e-4;
        let tone = |f: f32| move |i: usize| 0.3 * (std::f32::consts::TAU * f * i as f32 / RATE).sin() + floor(i);
        // 2 s into a steady 100 Hz tone: nothing is rising
        let steady = rise_after(2.0, tone(100.0));
        assert!(steady.rise.iter().all(|r| *r < 0.15), "steady max {}", steady.rise.iter().cloned().fold(0.0, f32::max));
        // a 100 Hz tone that starts at 1.0 s: just after the start, the rise is at 100 Hz and not at 5 kHz
        let t = |i: usize| if i as f32 / RATE >= 1.0 { tone(100.0)(i) } else { floor(i) };
        let start = rise_after(1.02, t);
        let at = |f: f32| (0..CURVE_POINTS).min_by(|a, b| (curve_freq(*a) - f).abs().partial_cmp(&(curve_freq(*b) - f).abs()).unwrap()).unwrap();
        assert!(start.rise[at(100.0)] > 1.0, "rise at 100 Hz {}", start.rise[at(100.0)]);
        assert!(start.rise[at(5000.0)] < 0.3, "rise at 5 kHz {}", start.rise[at(5000.0)]);
        // and a 5 kHz tone starting later rises up there instead
        let t = |i: usize| if i as f32 / RATE >= 1.0 { tone(5000.0)(i) } else { floor(i) };
        let hi = rise_after(1.02, t);
        // an abrupt start has some broadband click in it, but the tone's own frequency rises far more
        assert!(hi.rise[at(5000.0)] > 1.0, "rise at 5 kHz {}", hi.rise[at(5000.0)]);
        assert!(hi.rise[at(5000.0)] > 2.0 * hi.rise[at(100.0)], "rise at 5 kHz {} vs 100 Hz {}", hi.rise[at(5000.0)], hi.rise[at(100.0)]);
    }

    #[test]
    fn the_rise_dies_away_after_a_hit() {
        let floor = |i: usize| ((((i as u32).wrapping_mul(2654435761)) >> 16) as f32 / 65535.0 - 0.5) * 6e-4;
        let t = move |i: usize| floor(i) + if i as f32 / RATE >= 1.0 { 0.3 * (std::f32::consts::TAU * 100.0 * i as f32 / RATE).sin() } else { 0.0 };
        let hit = rise_after(1.01, t);
        let later = rise_after(1.5, t);
        let peak = |l: &Levels| l.rise.iter().cloned().fold(0.0, f32::max);
        assert!(peak(&hit) > 1.0 && peak(&later) < 0.2, "{} then {}", peak(&hit), peak(&later));
    }

    /// The rise per frequency region at the percussion fixture's kicks (t = 0 mod 3) and snares (1.5 mod 3), and
    /// its typical level over the demo song: for setting how the rise turns into sparks.
    #[test]
    #[ignore]
    fn dump_rise() {
        let regions = [(30.0, 150.0), (150.0, 500.0), (500.0, 2000.0), (2000.0, 6000.0), (6000.0, 20000.0)];
        let region_max = |lv: &Levels, r: (f32, f32)| (0..CURVE_POINTS).filter(|i| curve_freq(*i) >= r.0 && curve_freq(*i) < r.1).map(|i| lv.rise[i]).fold(0.0, f32::max);
        for (fx, secs) in [(Fixture::Percussion, 13.0), (Fixture::Demo, 40.0)] {
            let mut synth = Synth::new(fx);
            let mut dsp = Dsp::new();
            let (mut l, mut r) = (Vec::new(), Vec::new());
            let mut rows: Vec<(f64, [f32; 5])> = Vec::new();
            for k in 0..(secs * 100.0) as usize {
                l.clear();
                r.clear();
                synth.render(480, &mut l, &mut r);
                if let (Some(lv), _) = dsp.push(&l, &r, (k + 1) as f64 * 0.01) {
                    rows.push(((k + 1) as f64 * 0.01, std::array::from_fn(|i| region_max(&lv, regions[i]))));
                }
            }
            if fx == Fixture::Percussion {
                for (name, phase) in [("kick", 0.0), ("snare", 1.5)] {
                    let mut peak = [0.0f32; 5];
                    for (t, v) in &rows {
                        let m = (t - phase).rem_euclid(3.0);
                        if *t > 3.0 && m < 0.08 {
                            for i in 0..5 {
                                peak[i] = peak[i].max(v[i]);
                            }
                        }
                    }
                    eprintln!("{name:6} peak rise by region (30-150, 150-500, 500-2k, 2k-6k, 6k-20k Hz): {:.2?}", peak);
                }
                // the quiet moments between: typical max
                let mut quiet: Vec<f32> = rows.iter().filter(|(t, _)| *t > 3.0 && (t % 3.0 > 0.4 && t % 3.0 < 1.4)).map(|(_, v)| v.iter().cloned().fold(0.0, f32::max)).collect();
                quiet.sort_by(|a, b| a.partial_cmp(b).unwrap());
                eprintln!("between hits: median max-rise {:.2}, p95 {:.2}", quiet[quiet.len() / 2], quiet[quiet.len() * 95 / 100]);
            } else {
                for i in 0..5 {
                    let mut v: Vec<f32> = rows.iter().filter(|(t, _)| *t > 3.0).map(|(_, r)| r[i]).collect();
                    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    let n = v.len();
                    eprintln!("demo region {i}: median {:.2} p75 {:.2} p90 {:.2} p99 {:.2}", v[n / 2], v[n * 75 / 100], v[n * 90 / 100], v[n * 99 / 100]);
                }
            }
        }
    }

    /// Percentiles of the rise over every waterfall bin and frame of a song (the background a threshold must clear).
    #[test]
    #[ignore]
    fn dump_rise_distribution() {
        for fx in [Fixture::Percussion, Fixture::Demo, Fixture::Dense] {
            let mut synth = Synth::new(fx);
            let mut dsp = Dsp::new();
            let (mut l, mut r) = (Vec::new(), Vec::new());
            let mut all: Vec<f32> = Vec::new();
            for k in 0..(32 * 100) {
                l.clear();
                r.clear();
                synth.render(480, &mut l, &mut r);
                if let (Some(lv), _) = dsp.push(&l, &r, (k + 1) as f64 * 0.01) {
                    if k > 300 {
                        all.extend(lv.rise.iter());
                    }
                }
            }
            all.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let n = all.len();
            eprintln!("{:?}: per-bin rise median {:.2} p75 {:.2} p90 {:.2} p95 {:.2} p99 {:.2} p99.9 {:.2}", fx, all[n / 2], all[n * 75 / 100], all[n * 90 / 100], all[n * 95 / 100], all[n * 99 / 100], all[n * 999 / 1000]);
        }
    }

    /// What the fountain would do with a stream of per-frame rises (about 120 a second): through `RiseNorm`, the
    /// share of frames in which any frequency is above the spark threshold at sensitivity 1 and at 2, the
    /// typical number of frequencies above it when sparks fly, and the share of frames with a full-strength
    /// spark. Returns one line.
    fn spark_coverage(frames: &[[f32; CURVE_POINTS]]) -> String {
        let mut norm = crate::modulation::RiseNorm::new();
        let (mut any1, mut any2, mut full, mut bins1) = (0usize, 0usize, 0usize, 0usize);
        let mut after = 0usize;
        for (k, f) in frames.iter().enumerate() {
            let n = norm.update(f, 1.0 / 120.0);
            if k < 8 * 120 {
                continue; // let the reference settle
            }
            after += 1;
            let above = |t: f32| n.iter().filter(|v| **v >= t).count();
            let c1 = above(crate::modulation::SPARK_START);
            if c1 > 0 {
                any1 += 1;
                bins1 += c1;
            }
            if above(crate::modulation::SPARK_START / 2.0) > 0 {
                any2 += 1;
            }
            if n.iter().any(|v| *v >= crate::modulation::SPARK_FULL) {
                full += 1;
            }
        }
        let pct = |x: usize| 100.0 * x as f32 / after.max(1) as f32;
        format!(
            "sparks in {:4.1}% of frames at sensitivity 1 ({:.0} bins when they fly), {:4.1}% at 2, full strength in {:4.1}%",
            pct(any1), bins1 as f32 / any1.max(1) as f32, pct(any2), pct(full)
        )
    }

    /// The same, on the synthetic songs at several volumes.
    #[test]
    #[ignore]
    fn dump_spark_coverage() {
        for (fx, secs) in [(Fixture::Percussion, 40.0), (Fixture::Demo, 64.0), (Fixture::Dense, 32.0)] {
            for gain in [1.0f32, 0.5, 0.25, 0.1] {
                let mut synth = Synth::new(fx);
                let mut dsp = Dsp::new();
                let (mut l, mut r) = (Vec::new(), Vec::new());
                let mut frames = Vec::new();
                for k in 0..(secs * 120.0) as usize {
                    l.clear();
                    r.clear();
                    synth.render(400, &mut l, &mut r);
                    for v in l.iter_mut().chain(r.iter_mut()) {
                        *v *= gain;
                    }
                    if let (Some(lv), _) = dsp.push(&l, &r, (k + 1) as f64 / 120.0) {
                        frames.push(lv.rise);
                    } else if let Some(last) = frames.last().copied() {
                        frames.push(last);
                    }
                }
                eprintln!("{:10?} x{gain:<4}: {}", fx, spark_coverage(&frames));
            }
        }
    }

    /// Statistics of the rise on whatever is playing now, from the default output's monitor (read-only, no
    /// routing changes; only numbers are printed, no audio is kept). Run with something playing:
    /// `cargo test --release live_rise_stats -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_rise_stats() {
        let a = Analyzer::start("@DEFAULT_MONITOR@").expect("parec");
        let mut seen = 0u64;
        let mut per_frame_max: Vec<f32> = Vec::new();
        let mut all: Vec<f32> = Vec::new();
        let mut bands_peak = -200.0f32;
        let mut region: [Vec<f32>; 5] = Default::default();
        let mut frames: Vec<[f32; CURVE_POINTS]> = Vec::new();
        let regions = [(30.0, 150.0), (150.0, 500.0), (500.0, 2000.0), (2000.0, 6000.0), (6000.0, 20000.0)];
        let t0 = std::time::Instant::now();
        while t0.elapsed().as_secs_f32() < 30.0 {
            std::thread::sleep(std::time::Duration::from_millis(8));
            let lv = a.shared.levels.lock().unwrap().clone();
            if lv.seq == seen || t0.elapsed().as_secs_f32() < 1.0 {
                continue;
            }
            seen = lv.seq;
            bands_peak = bands_peak.max(lv.bands.iter().cloned().fold(-200.0, f32::max));
            per_frame_max.push(lv.rise.iter().cloned().fold(0.0, f32::max));
            all.extend(lv.rise.iter());
            frames.push(lv.rise);
            for (k, r) in regions.iter().enumerate() {
                region[k].push((0..CURVE_POINTS).filter(|i| curve_freq(*i) >= r.0 && curve_freq(*i) < r.1).map(|i| lv.rise[i]).fold(0.0, f32::max));
            }
        }
        eprintln!("{}", spark_coverage(&frames));
        // the old rule at sensitivity 3.0 (absolute: a tilted rise above 2.6/3 starts sparks, full at 6.5/3)
        let (mut any, mut full, mut bins) = (0usize, 0usize, 0usize);
        for f in frames.iter().skip(8 * 120) {
            let t: Vec<f32> = f.iter().enumerate().map(|(i, r)| r * crate::modulation::rise_tilt(i, CURVE_POINTS)).collect();
            let c = t.iter().filter(|v| **v > 2.6 / 3.0).count();
            if c > 0 {
                any += 1;
                bins += c;
            }
            if t.iter().any(|v| *v > 6.5 / 3.0) {
                full += 1;
            }
        }
        let n = frames.len().saturating_sub(8 * 120).max(1) as f32;
        eprintln!("old rule at sensitivity 3.0: sparks in {:4.1}% of frames ({:.0} bins when they fly), full strength in {:4.1}%", 100.0 * any as f32 / n, bins as f32 / any.max(1) as f32, 100.0 * full as f32 / n);
        let pct = |v: &mut Vec<f32>, p: f32| {
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            if v.is_empty() { 0.0 } else { v[((v.len() - 1) as f32 * p) as usize] }
        };
        eprintln!("frames {}, loudest band {:.1} dB", per_frame_max.len(), bands_peak);
        eprintln!("per-bin rise:      median {:.2} p75 {:.2} p90 {:.2} p95 {:.2} p99 {:.2} p99.9 {:.2}", pct(&mut all, 0.5), pct(&mut all, 0.75), pct(&mut all, 0.9), pct(&mut all, 0.95), pct(&mut all, 0.99), pct(&mut all, 0.999));
        eprintln!("per-frame max bin: median {:.2} p75 {:.2} p90 {:.2} p95 {:.2} p99 {:.2} max {:.2}", pct(&mut per_frame_max, 0.5), pct(&mut per_frame_max, 0.75), pct(&mut per_frame_max, 0.9), pct(&mut per_frame_max, 0.95), pct(&mut per_frame_max, 0.99), pct(&mut per_frame_max, 1.0));
        for (k, r) in regions.iter().enumerate() {
            eprintln!("region {:5.0}-{:5.0} Hz: max-bin rise median {:.2} p90 {:.2} p99 {:.2} max {:.2}", r.0, r.1, pct(&mut region[k], 0.5), pct(&mut region[k], 0.9), pct(&mut region[k], 0.99), pct(&mut region[k], 1.0));
        }
    }

    #[test]
    fn silence_has_no_events() {
        let (ev, lv) = run(Fixture::Silence, 4.0);
        assert!(ev.is_empty());
        assert!(lv.fx[..3].iter().all(|v| *v < 0.01));
    }

    #[test]
    fn impulses_are_detected_quickly() {
        // one click per second starting at t = 0.5 s
        let (ev, _) = run(Fixture::Impulse, 6.0);
        let lows: Vec<&Onset> = ev.iter().filter(|e| e.band == 0).collect();
        assert!(lows.len() >= 4, "found {} low events", lows.len());
        for e in lows {
            let nearest = (e.t - 0.5).rem_euclid(1.0).min(1.0 - (e.t - 0.5).rem_euclid(1.0));
            assert!(nearest < 0.03, "event at {} is {} s from a click", e.t, nearest);
        }
    }

    #[test]
    fn each_kick_is_one_low_event() {
        // kicks at t = 0, 3, 6, ...; snares at 1.5, 4.5, ...
        let (ev, _) = run(Fixture::Percussion, 13.0);
        for k in 1..4 {
            let t0 = 3.0 * k as f64;
            let n = ev.iter().filter(|e| e.band == 0 && e.t >= t0 - 0.01 && e.t < t0 + 0.4).count();
            assert_eq!(n, 1, "kick at {t0} produced {n} low events");
            let first = ev.iter().find(|e| e.band == 0 && e.t >= t0 - 0.01).unwrap();
            assert!(first.t - t0 < 0.03, "late by {}", first.t - t0);
        }
    }

    #[test]
    fn sustained_tones_do_not_retrigger() {
        let (ev, _) = run(Fixture::Tones, 10.0);
        let late = ev.iter().filter(|e| e.t > 3.0).count();
        assert!(late <= 2, "{late} events on a sustained chord");
    }

    #[test]
    fn correlation_tracks_phase() {
        let (_, mono) = run(Fixture::Stereo, 1.0);
        assert!(mono.fx[3] > -1.01 && mono.fx[3] < 1.01);
        let mut synth = Synth::new(Fixture::Phase);
        let mut dsp = Dsp::new();
        let (mut l, mut r) = (Vec::new(), Vec::new());
        let (mut lo, mut hi) = (1.0f32, -1.0f32);
        for i in 0..600 {
            l.clear();
            r.clear();
            synth.render(800, &mut l, &mut r);
            if let (Some(lv), _) = dsp.push(&l, &r, i as f64 * 800.0 / RATE as f64) {
                lo = lo.min(lv.fx[3]);
                hi = hi.max(lv.fx[3]);
            }
        }
        assert!(hi > 0.8 && lo < -0.8, "corr range {lo}..{hi}");
    }

    #[test]
    fn processing_is_deterministic_and_chunk_independent() {
        let render = |chunk: usize| -> Vec<Onset> {
            let mut synth = Synth::new(Fixture::Demo);
            let mut dsp = Dsp::new();
            let (mut l, mut r) = (Vec::new(), Vec::new());
            let mut events = Vec::new();
            let mut done = 0;
            while done < 48_000 * 8 {
                l.clear();
                r.clear();
                synth.render(chunk, &mut l, &mut r);
                done += chunk;
                events.extend(dsp.push(&l, &r, done as f64 / RATE as f64).1);
            }
            events
        };
        let a = render(480);
        let b = render(1200); // multiple of HOP
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(&b) {
            assert_eq!(x.band, y.band);
            assert!((x.t - y.t).abs() < 1e-6);
        }
    }
}
