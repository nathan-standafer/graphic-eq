//! The shared modulation layer: turns analyzer output into the signals scenes use, each with its
//! own response time. Everything is frame-rate independent (time constants, not per-frame factors).
//!
//! | signal            | role                                              |
//! |-------------------|---------------------------------------------------|
//! | onset envelopes   | brief, localized accents (rings, flashes, sparks)  |
//! | sustained l/m/h   | broad deformation, surface movement, lighting     |
//! | slow energy trend | gradual change (haze, spread, camera framing)     |
//!
//! Sustained signals use a bounded adaptive normalization with a silence gate, so quiet recordings
//! stay expressive while noise does not animate the scene. This only shapes artistic motion; the
//! analyzer's calibrated levels and the EQ gains are untouched.

use crate::analyzer::{Levels, Onset, ENV_TAU};
use crate::drums::{DrumHit, Kind};

const RING_CAP: usize = 8;
const RING_LIFE: f64 = 3.2;
/// Minimum onset strength that launches a ring, per band (low, mid, high; high never does).
const RING_MIN: [f32; 3] = [0.45, 1.0, f32::INFINITY];
const RING_SPACING: [f64; 3] = [0.10, 0.12, 1.0];

struct Ring {
    t0: f64,
    strength: f32,
    band: u8,
    centroid: f32,
}

/// What a frame's scenes receive.
pub struct Signals {
    /// Onset envelopes (low, mid, high) and stereo correlation.
    pub fx: [f32; 4],
    /// Sustained low / mid / high (normalized, gated) and the slow energy trend.
    pub sus: [f32; 4],
    pub gate: f32,
    pub rings: [[f32; 4]; RING_CAP],
}

pub struct Modulation {
    reference: [f32; 3],
    sus: [f32; 3],
    trend: f32,
    gate: f32,
    open: bool,
    rings: Vec<Ring>,
    last_ring: [f64; 3],
}

/// Makes the rise (see `analyzer::Levels::rise`) comparable between recordings. How steeply a recording's
/// volume rises at a hit depends on how sharp its drums are and how loud it is: the synthetic songs reach
/// 6-7 nats, a real recording was measured peaking near 3. So the rise is divided by a slowly decaying
/// peak of its own recent maximum, and what the fountain's threshold sees is "how close to the hardest
/// recent hit". The reference never falls below `REF_MIN`, so a stretch of steady music (whose rise is only
/// jitter) is not scaled up until the jitter looks like hits.
pub struct RiseNorm {
    reference: f32,
}

/// The normalized rise (1 = the hardest recent hit) at which a frequency starts to throw sparks, and at which
/// it throws its strongest, at sensitivity 1. Calibrated on a real recording, whose hardest hits rise about
/// 3.3 nats: the setting the owner found worked well on it was an absolute 0.87 to 2.2 nats, which is
/// 0.26 to 0.66 of that, trimmed a little (it matched that setting within 10% on the same audio). (The fountain's shader reads these through `render::SPARK_START` and `SPARK_FULL`.)
pub const SPARK_START: f32 = 0.28;
pub const SPARK_FULL: f32 = 0.72;
/// The reference is kept between these (nats), and falls with this time constant (seconds).
const REF_MIN: f32 = 3.0;
const REF_MAX: f32 = 7.0;
const REF_TAU_S: f32 = 8.0;

/// Higher frequencies rise less than the lows at a hit (a snare's rise up high is about half a kick's in the
/// lows, measured on both the synthetic songs and a real recording), so they are weighted up to keep both
/// sides of the fountain's line equally easy to light.
pub fn rise_tilt(bin: usize, bins: usize) -> f32 {
    0.85 + (1.8 - 0.85) * bin as f32 / (bins - 1) as f32
}

impl RiseNorm {
    pub fn new() -> RiseNorm {
        RiseNorm { reference: REF_MIN }
    }

    /// The tilted rise of every bin divided by the reference: about 1 at the hardest recent hit.
    pub fn update<const N: usize>(&mut self, rise: &[f32; N], dt: f32) -> [f32; N] {
        let mut out = [0.0f32; N];
        let mut peak = 0.0f32;
        for (i, r) in rise.iter().enumerate() {
            out[i] = r * rise_tilt(i, N);
            peak = peak.max(out[i]);
        }
        self.reference = (self.reference * (-dt / REF_TAU_S).exp()).max(peak).clamp(REF_MIN, REF_MAX);
        for v in out.iter_mut() {
            *v /= self.reference;
        }
        out
    }
}

/// A drum hit as the scenes react to it: where along the emitter line it landed (0 bass .. 1 treble) and how
/// hard it was, 0 at the lightest hit worth reporting and 1 at the hardest.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub pos: f32,
    pub strength: f32,
}

/// The lightest hit that is reported, as a strength (0 is the fountain's own threshold, `SPARK_START`).
const HIT_MIN: f32 = 0.18;
/// Peaks closer than this (in bins, of 192) are one hit; farther apart they are two only when the rise
/// between them dips below `HIT_VALLEY` of the lower peak, so a snare's broad plateau stays one hit while a
/// kick and a snare landing together are two. Beyond `HIT_FAR` bins they are always two.
/// How sure the drum model must be of a hit (its probability) at Sensitivity 1, how much each doubling of
/// Sensitivity lowers that, and the range it is kept to (the detector itself reports nothing below
/// `DRUM_P_MIN`). 0.9 is the design point: against 0.8 it costs a few percent of the kicks, snares and cymbals
/// and a tenth of the hats on the test songs, and halves the false hits on recordings with no drums in them.
const DRUM_P: f32 = 0.9;
const DRUM_P_PER_OCTAVE: f32 = 0.14;
const DRUM_P_MIN: f32 = 0.7;
const DRUM_P_MAX: f32 = 0.97;
/// A passage dense with hits is thinned to its harder ones: hits in the last `HIT_BUDGET_S` count by their kind
/// (a hat is `HAT_WEIGHT` of a kick or a snare), the first `DENSE_FREE` are free, and every unit beyond raises the
/// strength the next one needs by `DENSE_RAISE`. Eighth-note hats alone (four a second) are never thinned; a
/// groove with hats on top of its kicks and snares keeps its harder hats and loses the faint ones.
const DENSE_FREE: f32 = 4.0;
const DENSE_RAISE: f32 = 0.1;
const HAT_WEIGHT: f32 = 0.75;

/// Turns the drum detector's hits (`drums::DrumHit`) into what a scene reacts to. Each hit arrives once: the
/// frame's list is read with a memory of what has been seen, so a list offered twice does not fire twice.
/// Sensitivity is the model's confidence a hit needs (1 = the designed 0.9; higher accepts less certain hits)
/// and the strength of a hit is how hard it was against the hardest recent hit of its kind.
pub struct DrumHits {
    last: [f64; 3],
    /// Hits reported lately as (time, weight), for thinning dense music.
    recent: Vec<(f64, f32)>,
}

impl DrumHits {
    pub fn new() -> DrumHits {
        DrumHits { last: [f64::NEG_INFINITY; 3], recent: Vec::new() }
    }

    pub fn reset(&mut self) {
        *self = DrumHits::new();
    }

    /// The confidence a hit needs at this sensitivity.
    pub fn needed_p(sens: f32) -> f32 {
        (DRUM_P - DRUM_P_PER_OCTAVE * sens.max(0.05).log2()).clamp(DRUM_P_MIN, DRUM_P_MAX)
    }

    /// `drums` are the hits found since the last frame, `now` is the scene clock, `gate_open` whether the music is
    /// loud enough to count. Returns the hits to react to now.
    pub fn update(&mut self, drums: &[DrumHit], sens: f32, now: f64, gate_open: bool) -> Vec<Hit> {
        self.recent.retain(|(t, _)| now - t < HIT_BUDGET_S);
        let need = Self::needed_p(sens);
        let mut out = Vec::new();
        for h in drums {
            let k = h.kind as usize;
            if h.t <= self.last[k] {
                continue;
            }
            self.last[k] = h.t;
            if !gate_open || h.p < need {
                continue;
            }
            let load: f32 = self.recent.iter().map(|(_, w)| w).sum();
            let strength = (0.2 + 0.8 * h.strength).clamp(0.0, 1.0);
            if strength < HIT_MIN + DENSE_RAISE * (load - DENSE_FREE).max(0.0) {
                continue;
            }
            self.recent.push((now, if h.kind == Kind::High { HAT_WEIGHT } else { 1.0 }));
            out.push(Hit { pos: h.pos, strength });
        }
        out
    }
}

/// A burst from a column of the 3D columns' Sparks overlay is sized by how many LED segments its peak cap
/// jumped: from `PUSH_MIN_SEGMENTS` (the smallest burst) to `PUSH_FULL_SEGMENTS` (the biggest).
pub const PUSH_MIN_SEGMENTS: f32 = 5.0;
pub const PUSH_FULL_SEGMENTS: f32 = 25.0;

/// Sparks are rare and big on purpose: a hi-hat only throws them when it is an accent, this much of the hardest
/// recent one (a tick of hats at 8 a second would keep the treble columns in a constant shower).
pub const SPARK_HAT_MIN: f32 = 0.85;
/// Burst size follows the strength to this power: most hits throw a modest fan, the hardest a big one.
const SPARK_SIZE_POWER: f32 = 1.5;

/// The drum hits the Sparks overlay reacts to: kicks, snares and toms, and hats and cymbals only at an accent.
pub fn spark_worthy(drums: &[DrumHit]) -> Vec<DrumHit> {
    drums.iter().filter(|d| d.kind != Kind::High || d.strength >= SPARK_HAT_MIN).copied().collect()
}

/// Which column of the 3D columns each hit lands on (bass left, treble right) and how big a burst it throws there,
/// as the cap push (share of the full scale) the Sparks scene expects; 0 for no burst.
pub fn column_pushes(hits: &[Hit]) -> [f32; 11] {
    let mut out = [0.0f32; 11];
    for h in hits {
        let col = ((h.pos * 10.0).round() as usize).min(10);
        let segments = PUSH_MIN_SEGMENTS + (PUSH_FULL_SEGMENTS - PUSH_MIN_SEGMENTS) * h.strength.clamp(0.0, 1.0).powf(SPARK_SIZE_POWER);
        out[col] = out[col].max(segments / LED_SEGMENTS);
    }
    out
}

#[cfg(test)]
const HIT_NEAR: usize = 10;
#[cfg(test)]
const HIT_FAR: usize = 60;
#[cfg(test)]
const HIT_VALLEY: f32 = 0.7;
/// A hit is reported once its rise has stopped climbing (it reaches its height within about two analysis hops,
/// so the first frame over the threshold understates it), or at the latest after this long.
#[cfg(test)]
const HIT_SETTLE_S: f64 = 0.015;
/// One hit per position per this long, where a position is `HIT_NEAR_POS` of the line either side.
#[cfg(test)]
const HIT_REFRACTORY_S: f64 = 0.12;
#[cfg(test)]
const HIT_NEAR_POS: f32 = 0.12;
/// Dense music is thinned to its hardest hits: every hit reported in the last `HIT_BUDGET_S` raises the
/// strength the next one needs by `HIT_RAISE`.
const HIT_BUDGET_S: f64 = 1.0;
#[cfg(test)]
const HIT_RAISE: f32 = 0.12;
/// One event is at most this many hits: a snare is broadband, with a body, a noisy middle and a crack, and
/// every part clears the threshold, but it is one event. Hits that start within `HIT_EVENT_S` of each other
/// count as one event, and only its two strongest parts are reported (a kick and a snare landing together
/// are still a pair).
#[cfg(test)]
const HIT_EVENT_MAX: usize = 2;
#[cfg(test)]
const HIT_EVENT_S: f64 = 0.06;
/// A hit has to stand out from what its frequency is already doing: each bin's rise is judged after taking off
/// `HIT_BG_K` times its own slow average (time constant `HIT_BG_TAU_S`). A sustained chord whose close partials
/// beat inside the 21 ms analysis window makes the rise jitter all the time, at the very frequencies of the
/// chord, and without this it detonated about twice a second; a drum rises far above its own background.
#[cfg(test)]
const HIT_BG_K: f32 = 2.0;
#[cfg(test)]
const HIT_BG_TAU_S: f32 = 0.6;

#[cfg(test)]
struct PendingHit {
    pos: f32,
    best: f32,
    since: f64,
}

/// Finds hits in the normalized, tilted rise of the 192 bins (see `RiseNorm`): a peak along the bins that
/// clears the fountain's threshold is one hit, reported once it has peaked and then held off for a refractory
/// time at its position. Strength is measured above the fountain's threshold (`SPARK_START` to `SPARK_FULL`
/// at the sensitivity), so the Sensitivity setting means the same thing here as it does there. Fireworks
/// detonates a shell on each hit; a scene of lightning strikes would take the same events.
#[cfg(test)]
pub struct RiseHits {
    pending: Vec<PendingHit>,
    /// Hits found lately as (position, time, whether it was reported), for the refractory time (which a hit the
    /// budget turned away also starts, so its decaying tail is not mistaken for a new one) and the budget.
    recent: Vec<(f32, f64, bool)>,
    /// When the pending hits of the current event began.
    starts: Vec<f64>,
    /// Each bin's slow average rise, and when it was last updated.
    background: Vec<f32>,
    last: Option<f64>,
    /// The lightest hit reported and the background factor (the constants, unless a test sweeps them).
    min_strength: f32,
    bg_k: f32,
}

#[cfg(test)]
impl RiseHits {
    pub fn new() -> RiseHits {
        RiseHits { pending: Vec::new(), recent: Vec::new(), starts: Vec::new(), background: Vec::new(), last: None, min_strength: HIT_MIN, bg_k: HIT_BG_K }
    }

    pub fn reset(&mut self) {
        *self = RiseHits::new();
    }

    /// The peaks of `strength` per bin (0..2, see `update`) that clear `HIT_MIN`, strongest first, as (position, height).
    fn peaks<const N: usize>(strength: &[f32; N], min: f32) -> Vec<(f32, f32)> {
        let mut found: Vec<(usize, f32)> = Vec::new();
        for i in 0..N {
            let v = strength[i];
            let left = if i > 0 { strength[i - 1] } else { 0.0 };
            let right = if i + 1 < N { strength[i + 1] } else { 0.0 };
            if v >= min && v >= left && v > right {
                found.push((i, v));
            }
        }
        found.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let mut kept: Vec<(usize, f32)> = Vec::new();
        for (i, v) in found {
            let merged = kept.iter().any(|(j, _)| {
                let d = i.abs_diff(*j);
                if d <= HIT_NEAR {
                    return true;
                }
                if d >= HIT_FAR {
                    return false;
                }
                let (lo, hi) = (i.min(*j), i.max(*j));
                let valley = strength[lo..=hi].iter().cloned().fold(f32::MAX, f32::min);
                valley >= HIT_VALLEY * v
            });
            if !merged {
                kept.push((i, v));
            }
        }
        // the position is the centre of mass of the peak's shoulders, so a plateau reads in its middle
        kept.iter()
            .map(|(i, v)| {
                let (lo, hi) = (i.saturating_sub(3), (i + 3).min(N - 1));
                let (mut w, mut m) = (0.0f32, 0.0f32);
                for k in lo..=hi {
                    let s = strength[k].max(0.0);
                    w += s;
                    m += s * k as f32;
                }
                (m / w.max(1e-6) / (N - 1) as f32, *v)
            })
            .collect()
    }

    /// `rise` is `RiseNorm::update`'s output for this frame, `sens` the fountain's sensitivity, `gate_open`
    /// whether the music is loud enough to count. Returns the hits reported now.
    pub fn update<const N: usize>(&mut self, rise: &[f32; N], sens: f32, now: f64, gate_open: bool) -> Vec<Hit> {
        self.recent.retain(|(_, t, _)| now - t < HIT_BUDGET_S);
        self.starts.retain(|t| now - t < HIT_EVENT_S);
        let span = (SPARK_FULL - SPARK_START).max(1e-3);
        if self.background.len() != N {
            self.background = vec![0.0; N];
        }
        let dt = self.last.map_or(0.0, |t| (now - t).clamp(0.0, 0.1)) as f32;
        self.last = Some(now);
        let follow = 1.0 - (-dt / HIT_BG_TAU_S).exp();
        let mut strength = [0.0f32; N];
        for ((s, r), bg) in strength.iter_mut().zip(rise).zip(self.background.iter_mut()) {
            // the contrast is taken before the average moves, so a hit does not count against itself
            let contrast = (r - self.bg_k * *bg).max(0.0);
            *bg += (r - *bg) * follow;
            if gate_open {
                // above 1 the strength is only tracked, so a hit still climbing past full is seen to be climbing
                *s = ((contrast * sens - SPARK_START) / span).clamp(0.0, 2.0);
            }
        }
        let peaks = Self::peaks(&strength, self.min_strength);
        let held = |recent: &[(f32, f64, bool)], pos: f32| recent.iter().any(|(p, t, _)| now - t < HIT_REFRACTORY_S && (p - pos).abs() < HIT_NEAR_POS);

        let mut out = Vec::new();
        let mut matched = vec![false; peaks.len()];
        let mut still = Vec::new();
        for mut pend in std::mem::take(&mut self.pending) {
            let at = peaks.iter().position(|(p, _)| (p - pend.pos).abs() < HIT_NEAR_POS);
            let mut done = at.is_none() || now - pend.since >= HIT_SETTLE_S;
            if let Some(k) = at {
                matched[k] = true;
                let v = peaks[k].1;
                if v > pend.best {
                    pend.best = v;
                    pend.pos = peaks[k].0;
                } else if v < pend.best - 0.04 {
                    done = true; // it has peaked and is falling
                }
            }
            if !done {
                still.push(pend);
                continue;
            }
            let reported = self.recent.iter().filter(|r| r.2).count();
            let needed = self.min_strength + HIT_RAISE * reported as f32;
            if !held(&self.recent, pend.pos) {
                let ok = pend.best >= needed;
                self.recent.push((pend.pos, now, ok));
                if ok {
                    out.push(Hit { pos: pend.pos, strength: pend.best.min(1.0) });
                }
            }
        }
        for (k, (pos, v)) in peaks.iter().enumerate() {
            if !matched[k] && self.starts.len() < HIT_EVENT_MAX && !held(&self.recent, *pos) && !still.iter().any(|p| (p.pos - pos).abs() < HIT_NEAR_POS) {
                still.push(PendingHit { pos: *pos, best: *v, since: now });
                self.starts.push(now);
            }
        }
        self.pending = still;
        out
    }
}

/// The LED strip of a 3D column has this many segments (`columns.wgsl`: `COL_MAX_H / SEG`), so a peak cap's
/// position is counted in them.
pub const LED_SEGMENTS: f32 = 30.0;

fn smooth(cur: &mut f32, target: f32, dt: f32, tau_up: f32, tau_down: f32) {
    let tau = if target > *cur { tau_up } else { tau_down };
    *cur += (target - *cur) * (1.0 - (-dt / tau).exp());
}

impl Modulation {
    pub fn new() -> Modulation {
        Modulation { reference: [0.25; 3], sus: [0.0; 3], trend: 0.0, gate: 0.0, open: false, rings: Vec::new(), last_ring: [f64::NEG_INFINITY; 3] }
    }

    /// `frac` are the displayed band levels (0..1) after range and sensitivity.
    pub fn update(&mut self, now: f64, dt: f32, lv: &Levels, fresh: &[Onset], frac: &[f32; 11]) -> Signals {
        // ---- sustained energy per region, normalized to a slowly-decaying reference
        let mean = |r: std::ops::Range<usize>| frac[r.clone()].iter().sum::<f32>() / r.len() as f32;
        let raw = [mean(0..4), mean(4..8), mean(8..11)];
        let peak = raw.iter().cloned().fold(0.0, f32::max);

        // silence gate with hysteresis; opens quickly, closes slowly
        if peak > 0.09 {
            self.open = true;
        } else if peak < 0.05 {
            self.open = false;
        }
        smooth(&mut self.gate, if self.open { 1.0 } else { 0.0 }, dt, 0.25, 1.2);

        let mut level = 0.0;
        for i in 0..3 {
            // the reference follows peaks up at once and sinks over ~25 s, never below a floor
            self.reference[i] = (self.reference[i] * (-dt / 25.0).exp()).max(raw[i]).max(0.16);
            let norm = (raw[i] / self.reference[i]).clamp(0.0, 1.2) * self.gate;
            smooth(&mut self.sus[i], norm, dt, 0.10, 0.45);
            level += self.sus[i] / 3.0;
        }
        smooth(&mut self.trend, level, dt, 5.0, 8.0);

        // ---- onset envelopes: the analyzer's envelope at its own timestamp, decayed to now
        let age = (now - lv.t).max(0.0) as f32;
        let mut fx = [0.0f32; 4];
        for i in 0..3 {
            fx[i] = (lv.fx[i] * (-age / ENV_TAU[i]).exp()).min(1.5) * self.gate;
        }
        fx[3] = lv.fx[3];

        // ---- rings from discrete events
        for e in fresh {
            let b = (e.band as usize).min(2);
            if self.gate > 0.5 && e.strength >= RING_MIN[b] && e.t - self.last_ring[b] >= RING_SPACING[b] {
                self.last_ring[b] = e.t;
                self.rings.push(Ring { t0: e.t, strength: (0.55 + 0.45 * e.strength).min(1.4), band: e.band, centroid: e.centroid });
            }
        }
        self.rings.retain(|r| now - r.t0 < RING_LIFE);
        if self.rings.len() > RING_CAP {
            let drop = self.rings.len() - RING_CAP;
            self.rings.drain(..drop);
        }
        let mut rings = [[0.0f32; 4]; RING_CAP];
        for (slot, r) in rings.iter_mut().zip(&self.rings) {
            *slot = [(now - r.t0).max(0.0) as f32, r.strength, r.band as f32, r.centroid];
        }

        Signals { fx, sus: [self.sus[0], self.sus[1], self.sus[2], self.trend], gate: self.gate, rings }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lv(env: f32, t: f64) -> Levels {
        let mut l = Levels::default();
        l.fx = [env, 0.0, 0.0, 0.0];
        l.t = t;
        l
    }

    #[test]
    fn gate_closes_on_silence_and_noise_does_not_animate() {
        let mut m = Modulation::new();
        let quiet = [0.02f32; 11];
        let mut s = None;
        for i in 0..600 {
            s = Some(m.update(i as f64 / 60.0, 1.0 / 60.0, &lv(0.0, 0.0), &[], &quiet));
        }
        let s = s.unwrap();
        assert!(s.gate < 0.02 && s.sus[..3].iter().all(|v| *v < 0.02));
    }

    #[test]
    fn quiet_but_real_signal_is_still_expressive() {
        let mut m = Modulation::new();
        let mut frac = [0.0f32; 11];
        for (i, f) in frac.iter_mut().enumerate() {
            *f = 0.12 + 0.05 * (i % 3) as f32; // a recording far below full scale
        }
        let mut s = None;
        for i in 0..600 {
            s = Some(m.update(i as f64 / 60.0, 1.0 / 60.0, &lv(0.0, 0.0), &[], &frac));
        }
        assert!(s.unwrap().sus[0] > 0.4);
    }

    #[test]
    fn rings_launch_on_low_onsets_and_expire() {
        let mut m = Modulation::new();
        let frac = [0.5f32; 11];
        for i in 0..120 {
            m.update(i as f64 / 60.0, 1.0 / 60.0, &lv(0.0, 0.0), &[], &frac);
        }
        let hit = Onset { band: 0, strength: 1.0, centroid: 0.2, t: 2.0 };
        let s = m.update(2.01, 1.0 / 60.0, &lv(1.0, 2.0), &[hit], &frac);
        assert!(s.rings[0][1] > 0.0 && s.rings[0][0] < 0.05);
        let later = m.update(6.0, 1.0 / 60.0, &lv(0.0, 5.9), &[], &frac);
        assert_eq!(later.rings[0][1], 0.0);
        // a high-frequency event never launches a ring
        let tick = Onset { band: 2, strength: 2.0, centroid: 0.9, t: 6.0 };
        let s = m.update(6.01, 1.0 / 60.0, &lv(0.0, 6.0), &[tick], &frac);
        assert_eq!(s.rings[0][1], 0.0);
    }

    fn drum(t: f64, kind: Kind, p: f32, strength: f32) -> DrumHit {
        DrumHit { t, kind, pos: 0.2, strength, p }
    }

    #[test]
    fn a_drum_hit_reaches_a_scene_once_even_if_its_list_is_offered_twice() {
        let mut dh = DrumHits::new();
        let list = [drum(1.0, Kind::Low, 0.95, 0.9), drum(1.0, Kind::High, 0.9, 0.5)];
        assert_eq!(dh.update(&list, 1.0, 1.01, true).len(), 2);
        assert!(dh.update(&list, 1.0, 1.02, true).is_empty());
        // a later hit of the same kind is new
        assert_eq!(dh.update(&[drum(1.2, Kind::Low, 0.95, 0.9)], 1.0, 1.21, true).len(), 1);
    }

    #[test]
    fn sensitivity_sets_how_sure_the_model_must_be_and_a_closed_gate_reports_nothing() {
        let unsure = [drum(2.0, Kind::Snare, 0.8, 0.8)];
        assert!(DrumHits::new().update(&unsure, 1.0, 2.0, true).is_empty());
        assert_eq!(DrumHits::new().update(&unsure, 2.0, 2.0, true).len(), 1, "doubling the sensitivity accepts p 0.8");
        assert!(DrumHits::new().update(&[drum(2.0, Kind::Snare, 0.9, 0.8)], 0.3, 2.0, true).is_empty(), "and lowering it asks for more");
        assert!(DrumHits::new().update(&[drum(2.0, Kind::Snare, 0.99, 0.8)], 1.0, 2.0, false).is_empty());
        assert!((DrumHits::needed_p(1.0) - 0.9).abs() < 1e-6 && DrumHits::needed_p(100.0) >= DRUM_P_MIN);
    }

    #[test]
    fn dense_hats_are_thinned_but_an_ordinary_pattern_and_the_kicks_get_through() {
        // eighth-note hats at 120 BPM (4 a second) are never thinned
        let mut dh = DrumHits::new();
        let steady = (0..24).filter(|i| dh.update(&[drum(5.0 + *i as f64 * 0.25, Kind::High, 0.9, 0.2)], 1.0, 5.0 + *i as f64 * 0.25, true).len() == 1).count();
        assert_eq!(steady, 24);
        // a frantic roll (20 a second) loses some of them
        let mut dh = DrumHits::new();
        let mut roll = 0;
        for i in 0..40 {
            let t = 5.0 + i as f64 * 0.05;
            roll += dh.update(&[drum(t, Kind::High, 0.9, 0.2)], 1.0, t, true).len();
        }
        assert!(roll < 40, "all {roll} of 40 hats of a roll got through");
        // and a full-strength kick in the middle of it is never thinned away
        let kick = dh.update(&[drum(7.01, Kind::Low, 0.99, 1.0)], 1.0, 7.01, true);
        assert_eq!(kick.len(), 1);
        assert!((kick[0].strength - 1.0).abs() < 1e-6 && (kick[0].pos - 0.2).abs() < 1e-6);
    }

    #[test]
    fn sparks_take_kicks_and_snares_but_only_accented_hats() {
        let ds = [drum(1.0, Kind::Low, 0.95, 0.3), drum(1.1, Kind::Snare, 0.95, 0.4), drum(1.2, Kind::High, 0.95, 0.5), drum(1.3, Kind::High, 0.95, 0.9)];
        let kinds: Vec<Kind> = spark_worthy(&ds).iter().map(|d| d.kind).collect();
        assert_eq!(kinds, vec![Kind::Low, Kind::Snare, Kind::High], "a plain hat is left out, an accent is kept");
        assert!(spark_worthy(&ds)[2].strength >= SPARK_HAT_MIN);
    }

    #[test]
    fn hits_land_on_the_column_of_their_place_and_big_hits_throw_bigger_bursts() {
        let p = column_pushes(&[Hit { pos: 0.0, strength: 0.0 }, Hit { pos: 0.52, strength: 1.0 }, Hit { pos: 1.0, strength: 0.5 }]);
        assert!((p[0] * LED_SEGMENTS - PUSH_MIN_SEGMENTS).abs() < 1e-4, "smallest burst {}", p[0] * LED_SEGMENTS);
        assert!((p[5] * LED_SEGMENTS - PUSH_FULL_SEGMENTS).abs() < 1e-4, "biggest burst {}", p[5] * LED_SEGMENTS);
        assert!(p[10] > p[0] && p[10] < p[5]);
        assert_eq!(p.iter().filter(|v| **v > 0.0).count(), 3);
    }

    #[test]
    fn the_rise_is_judged_against_the_music_s_own_hits() {
        let dt = 1.0 / 120.0;
        // a hit stream and the same stream at half the strength: after a few seconds both reach the same
        // normalized height, which is what makes one sensitivity setting work for both
        let run = |scale: f32| {
            let mut n = RiseNorm::new();
            let mut top = 0.0f32;
            for k in 0..(10 * 120) {
                let mut rise = [0.0f32; 192];
                if k % 60 < 5 {
                    rise[10] = 6.0 * scale; // a kick every half second
                }
                let out = n.update(&rise, dt);
                if k > 8 * 120 {
                    top = top.max(out[10]);
                }
            }
            top
        };
        let (loud, soft) = (run(1.0), run(0.55));
        assert!(loud > 0.6, "loud {loud}");
        // below the floor the reference stops following: a soft recording is lifted but not all the way
        assert!(soft > 0.45 * loud / 1.0 && soft < loud, "soft {soft} vs loud {loud}");
        // the jitter of steady music (rise ~1) does not look like hits: the floor keeps it well below 1
        let mut n = RiseNorm::new();
        let mut worst = 0.0f32;
        for k in 0..(20 * 120) {
            let rise = [0.8 + 0.4 * ((k % 7) as f32 / 7.0); 192];
            for v in n.update(&rise, dt) {
                worst = worst.max(v);
            }
        }
        assert!(worst < 0.7, "steady jitter reads {worst}");
        // after a very loud passage the reference decays instead of staying high for ever, down to its floor
        let mut n = RiseNorm::new();
        n.update(&[4.0f32; 192], dt);
        assert!(n.reference > 6.0);
        for _ in 0..(40 * 120) {
            n.update(&[0.2f32; 192], dt);
        }
        assert!((n.reference - REF_MIN).abs() < 0.05, "reference stuck at {}", n.reference);
    }

    // ---- RiseHits: one peak along the bins is one hit

    /// A normalized rise with a triangular bump of `height` centred on bin `at`, `half` bins to a side.
    fn bump(at: usize, half: usize, height: f32) -> [f32; 192] {
        let mut r = [0.0f32; 192];
        for k in at.saturating_sub(half)..=(at + half).min(191) {
            r[k] = height * (1.0 - 0.6 * (k.abs_diff(at) as f32 / (half + 1) as f32));
        }
        r
    }

    fn add(a: &[f32; 192], b: &[f32; 192]) -> [f32; 192] {
        std::array::from_fn(|i| a[i].max(b[i]))
    }

    /// Feeds `frames` at 120 Hz starting at `t0`, followed by `quiet` empty frames, and returns each hit with
    /// the frame it was reported in.
    fn hits_for(rh: &mut RiseHits, frames: &[[f32; 192]], t0: f64, sens: f32, quiet: usize) -> Vec<(usize, Hit)> {
        let mut out = Vec::new();
        for (k, f) in frames.iter().chain(std::iter::repeat(&[0.0f32; 192]).take(quiet)).enumerate() {
            for h in rh.update(f, sens, t0 + k as f64 * DT120, true) {
                out.push((k, h));
            }
        }
        out
    }

    const DT120: f64 = 1.0 / 120.0;

    /// A hit's rise decaying with the analyzer's 50 ms time constant, from `peak` at its first frame.
    fn decaying(at: usize, half: usize, peak: f32, frames: usize) -> Vec<[f32; 192]> {
        (0..frames).map(|k| bump(at, half, peak * (-(k as f64 * DT120) / 0.05).exp() as f32)).collect()
    }

    #[test]
    fn a_hit_is_reported_once_at_its_place_and_strength_within_a_frame_or_two() {
        // a kick in the lows that climbs over three frames, then dies away: one hit, at the height it reached
        let mut f = vec![bump(10, 6, 0.55), bump(10, 6, 0.9)];
        f.extend(decaying(10, 6, 1.1, 30));
        let hits = hits_for(&mut RiseHits::new(), &f, 5.0, 1.0, 120);
        assert_eq!(hits.len(), 1, "{hits:?}");
        let (frame, h) = hits[0];
        assert!(frame <= 2, "reported {frame} frames after the rise crossed the threshold");
        assert!((h.pos - 10.0 / 191.0).abs() < 0.03, "{h:?}");
        assert!(h.strength > 0.95, "measured the first frame, not the peak: {h:?}");
    }

    #[test]
    fn a_kick_and_a_snare_together_are_two_hits_and_a_broad_snare_is_one() {
        let mut snare = bump(150, 28, 0.9);
        for (i, v) in snare.iter_mut().enumerate() {
            *v *= 0.85 + 0.15 * ((i * 7 % 5) as f32 / 4.0); // the jitter of a noise-like plateau
        }
        let both = add(&bump(10, 6, 1.1), &snare);
        let hits = hits_for(&mut RiseHits::new(), &[both; 3], 1.0, 1.0, 60);
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert!(hits.iter().any(|(_, h)| h.pos < 0.1) && hits.iter().any(|(_, h)| h.pos > 0.6), "{hits:?}");
        let hits = hits_for(&mut RiseHits::new(), &[snare; 3], 1.0, 1.0, 60);
        assert_eq!(hits.len(), 1, "one hit for one plateau: {hits:?}");
        assert!(hits[0].1.pos > 0.6 && hits[0].1.pos < 0.95, "{hits:?}");
    }

    #[test]
    fn a_decaying_tail_is_no_second_hit_but_the_next_beat_is() {
        let mut f = decaying(10, 6, 1.1, 24); // 200 ms
        f.extend(std::iter::repeat([0.0f32; 192]).take(36));
        f.extend(decaying(10, 6, 1.1, 24)); // the next hit at 500 ms
        let hits = hits_for(&mut RiseHits::new(), &f, 2.0, 1.0, 60);
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert!(((hits[1].0 - hits[0].0) as f64 * DT120 - 0.5).abs() < 0.03, "{hits:?}");
        // two hits at different places inside the refractory time are both reported
        let two = add(&bump(10, 6, 1.1), &bump(120, 6, 1.1));
        assert_eq!(hits_for(&mut RiseHits::new(), &[two; 3], 1.0, 1.0, 60).len(), 2);
        // and the same place again 60 ms later is held off
        let mut f = decaying(10, 6, 1.1, 7);
        f.extend(decaying(10, 6, 1.1, 20));
        assert_eq!(hits_for(&mut RiseHits::new(), &f, 1.0, 1.0, 60).len(), 1);
    }

    #[test]
    fn sensitivity_moves_the_threshold_and_scales_the_strength() {
        let f = vec![bump(60, 6, 0.55); 6]; // a moderate hit: 0.6 of the hardest at the peak of its bump
        let lo = hits_for(&mut RiseHits::new(), &f, 1.0, 0.5, 60);
        let mid = hits_for(&mut RiseHits::new(), &f, 1.0, 1.0, 60);
        let hi = hits_for(&mut RiseHits::new(), &f, 1.0, 2.0, 60);
        assert!(lo.is_empty(), "below the threshold at low sensitivity: {lo:?}");
        assert_eq!(mid.len(), 1, "{mid:?}");
        assert_eq!(hi.len(), 1, "{hi:?}");
        assert!(hi[0].1.strength > mid[0].1.strength + 0.2, "{mid:?} vs {hi:?}");
    }

    #[test]
    fn a_closed_gate_reports_nothing_and_dense_music_is_thinned_to_its_hardest_hits() {
        let mut rh = RiseHits::new();
        assert!((0..60).all(|k| rh.update(&bump(10, 6, 1.1), 1.0, k as f64 * DT120, false).is_empty()));
        // a hit every 125 ms at a new place each time: a storm. The weak ones thin out, the hard ones come through.
        let mut rh = RiseHits::new();
        let (mut weak, mut hard) = (0, 0);
        for beat in 0..32 {
            let at = 20 + (beat * 37) % 150;
            let hard_one = beat % 4 == 0;
            let h = if hard_one { 1.1 } else { 0.5 };
            let t0 = 3.0 + beat as f64 * 0.125;
            let frames: Vec<[f32; 192]> = (0..15).map(|k| bump(at, 5, h * (-(k as f64 * DT120) / 0.05).exp() as f32)).collect();
            let n = hits_for(&mut rh, &frames, t0, 1.0, 0).len();
            if hard_one { hard += n } else { weak += n }
        }
        assert!(hard >= 6, "the hard hits ({hard}) must come through");
        assert!(weak <= hard + 4, "weak hits ({weak}) should be thinned against hard ({hard})");
        assert!(hard + weak < 30, "a storm of 32 beats reported {}", hard + weak);
    }

    /// Hits found by the real analyzer, the rise normalization and the detector on the percussion fixture, whose
    /// kicks (t = 0 mod 3, 100 Hz) and snares (1.5 mod 3, broadband) are exact: one hit per event, kicks at the
    /// left of the line and snares reaching the right, reported within a few frames of the event.
    fn fixture_hits(fx: crate::synth::Fixture, secs: f64, sens: f32) -> Vec<(f64, Hit)> {
        fixture_hits_with(fx, secs, sens, RiseHits::new())
    }

    fn fixture_hits_with(fx: crate::synth::Fixture, secs: f64, sens: f32, mut rh: RiseHits) -> Vec<(f64, Hit)> {
        use crate::analyzer::{Dsp, CURVE_POINTS};
        use crate::synth::Synth;
        let mut synth = Synth::new(fx);
        let mut dsp = Dsp::new();
        let mut norm = RiseNorm::new();
        let (mut l, mut r) = (Vec::new(), Vec::new());
        let mut rise = [0.0f32; CURVE_POINTS];
        let mut out = Vec::new();
        let frames = (secs * 120.0) as usize;
        for k in 0..frames {
            l.clear();
            r.clear();
            synth.render(400, &mut l, &mut r);
            let now = (k + 1) as f64 / 120.0;
            if let (Some(lv), _) = dsp.push(&l, &r, now) {
                rise = lv.rise;
            }
            let n = norm.update(&rise, 1.0 / 120.0);
            for h in rh.update(&n, sens, now, true) {
                out.push((now, h));
            }
        }
        out
    }

    /// How many hits the detector finds per second on the synthetic songs, and where along the line: for judging
    /// density. `cargo test --release dump_hits -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn dump_hits() {
        use crate::synth::Fixture;
        for (fx, secs) in [(Fixture::Percussion, 31.0), (Fixture::Demo, 64.0), (Fixture::Dense, 32.0)] {
            for sens in [0.6f32, 1.0, 1.8] {
                let hits: Vec<_> = fixture_hits(fx, secs, sens).into_iter().filter(|h| h.0 > 6.0).collect();
                let span = secs - 6.0;
                let mut third = [0; 3];
                for h in &hits {
                    third[((h.1.pos * 3.0) as usize).min(2)] += 1;
                }
                let mean = hits.iter().map(|h| h.1.strength).sum::<f32>() / hits.len().max(1) as f32;
                eprintln!("{fx:?} sens {sens}: {:.2} hits/s (low {} mid {} high {}), mean strength {mean:.2}", hits.len() as f64 / span, third[0], third[1], third[2]);
            }
        }
    }

    /// Sweeps the minimum strength and the background factor: hits a second after the start on sustained tones (which
    /// should fire nothing), a sweep, and the musical fixtures at sensitivities 1 and 1.8.
    #[test]
    #[ignore]
    fn dump_hit_tuning() {
        use crate::synth::Fixture;
        for (min, k) in [(0.18f32, 1.0f32), (0.18, 1.5), (0.18, 2.0), (0.30, 1.0), (0.30, 1.5), (0.30, 2.0), (0.45, 1.0), (0.45, 1.5), (0.45, 2.0)] {
            let run = |fx: Fixture, secs: f64, sens: f32| {
                let mut rh = RiseHits::new();
                rh.min_strength = min;
                rh.bg_k = k;
                let n = fixture_hits_with(fx, secs, sens, rh).iter().filter(|h| h.0 > 4.0).count();
                n as f64 / (secs - 4.0)
            };
            eprintln!(
                "min {min:.2} k {k:.1}: tones {:.2}/s sweep {:.2}/s | perc {:.2} demo {:.2} dense {:.2} /s at sens 1 | perc {:.2} demo {:.2} dense {:.2} /s at sens 1.8",
                run(Fixture::Tones, 40.0, 1.0), run(Fixture::Sweep, 40.0, 1.0),
                run(Fixture::Percussion, 40.0, 1.0), run(Fixture::Demo, 64.0, 1.0), run(Fixture::Dense, 32.0, 1.0),
                run(Fixture::Percussion, 40.0, 1.8), run(Fixture::Demo, 64.0, 1.8), run(Fixture::Dense, 32.0, 1.8),
            );
        }
    }

    /// The hits around each drum of the percussion fixture, for judging how many a snare is.
    #[test]
    #[ignore]
    fn dump_percussion_events() {
        let hits = fixture_hits(crate::synth::Fixture::Percussion, 31.0, 1.0);
        for k in 2..10 {
            for (t, name) in [(3.0 * k as f64, "kick "), (3.0 * k as f64 + 1.5, "snare")] {
                let found: Vec<String> = hits.iter().filter(|h| h.0 >= t && h.0 < t + 0.3).map(|h| format!("+{:.0}ms pos {:.2} str {:.2}", (h.0 - t) * 1000.0, h.1.pos, h.1.strength)).collect();
                eprintln!("{name} {t:5.1}: {}", found.join(" | "));
            }
        }
    }

    #[test]
    fn a_hit_must_stand_out_from_its_own_background() {
        // a frequency that jitters between 0.2 and 0.7 all the time (a beating chord) is no hit...
        let mut rh = RiseHits::new();
        let mut n = 0;
        for k in 0..(10 * 120) {
            let h = 0.2 + 0.5 * (((k * 7919) % 13) as f32 / 12.0);
            n += rh.update(&bump(60, 6, h), 1.0, 2.0 + k as f64 * DT120, true).len();
        }
        assert!(n <= 1, "{n} hits from a jittering background");
        // ...but a drum on top of it still is
        let mut hits = 0;
        for k in 0..(4 * 120) {
            let drum = if (k % 60) < 3 { 1.2 } else { 0.0 };
            let jitter = 0.2 + 0.3 * (((k * 7919) % 13) as f32 / 12.0);
            let f = add(&bump(60, 6, jitter), &bump(8, 6, drum));
            hits += rh.update(&f, 1.0, 20.0 + k as f64 * DT120, true).iter().filter(|h| h.pos < 0.1).count();
        }
        assert!(hits >= 6, "{hits} of 8 drums over a jittering background");
    }

    #[test]
    fn sustained_tones_and_sweeps_detonate_nothing() {
        use crate::synth::Fixture;
        for fx in [Fixture::Tones, Fixture::Sweep] {
            let hits = fixture_hits(fx, 24.0, 1.0);
            let after: Vec<_> = hits.iter().filter(|h| h.0 > 4.0).collect();
            // a sweep passes each frequency once, a couple of times in its period; steady tones fire nothing
            let allowed = if fx == Fixture::Tones { 1 } else { 8 };
            assert!(after.len() <= allowed, "{fx:?}: {} hits after the start: {after:?}", after.len());
        }
    }

    #[test]
    fn the_percussion_fixture_gives_one_hit_per_drum_in_the_right_place() {
        let hits = fixture_hits(crate::synth::Fixture::Percussion, 31.0, 1.0);
        // events: kicks at 3k, snares at 3k + 1.5. Skip the first 6 s while the reference settles.
        let near = |h: &(f64, Hit), t: f64| h.0 >= t && h.0 < t + 0.12;
        let mut missed = Vec::new();
        for k in 2..10 {
            for (t, name, left) in [(3.0 * k as f64, "kick", true), (3.0 * k as f64 + 1.5, "snare", false)] {
                let found: Vec<_> = hits.iter().filter(|h| near(h, t)).collect();
                if found.is_empty() {
                    missed.push(format!("{name} at {t}"));
                    continue;
                }
                if left {
                    assert!(found.len() == 1 && found[0].1.pos < 0.3, "a kick is one hit on the left: {found:?}");
                } else {
                    // a snare is broadband: its body lands with the kicks and its crack on the right, never more
                    assert!(found.len() <= HIT_EVENT_MAX && found.iter().any(|h| h.1.pos > 0.5), "a snare should reach the right: {found:?}");
                }
            }
        }
        assert!(missed.is_empty(), "missed events: {missed:?}\nall hits: {hits:?}");
        // and nothing between the drums beyond a couple of tails
        let stray = hits.iter().filter(|h| h.0 > 6.0 && (0..10).all(|k| !near(h, 3.0 * k as f64) && !near(h, 3.0 * k as f64 + 1.5))).count();
        assert!(stray <= 2, "{stray} hits fell between the drums: {hits:?}");
    }

    #[test]
    fn envelopes_decay_with_time_not_frames() {
        let mut a = Modulation::new();
        let mut b = Modulation::new();
        let frac = [0.5f32; 11];
        for i in 0..120 {
            a.update(i as f64 / 60.0, 1.0 / 60.0, &lv(0.0, 0.0), &[], &frac);
            b.update(i as f64 / 60.0, 1.0 / 60.0, &lv(0.0, 0.0), &[], &frac);
        }
        // the same elapsed time since the same event, observed at 60 and 240 Hz
        let ea = a.update(2.1, 1.0 / 60.0, &lv(1.0, 2.0), &[], &frac).fx[0];
        let eb = b.update(2.1, 1.0 / 240.0, &lv(1.0, 2.0), &[], &frac).fx[0];
        assert!((ea - eb).abs() < 1e-5);
    }
}
