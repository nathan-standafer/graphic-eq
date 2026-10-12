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

const RING_CAP: usize = 8;
const RING_LIFE: f64 = 3.2;
/// Minimum onset strength that launches a ring, per band (low, mid, high; high never does).
const RING_MIN: [f32; 3] = [0.45, 1.0, f32::INFINITY];
const RING_SPACING: [f64; 3] = [0.10, 0.12, 1.0];
/// Discrete hits per frame, and per band the weakest hit worth reporting and the shortest gap between hits.
pub const HIT_CAP: usize = 4;
const HIT_MIN: [f32; 3] = [0.30, 0.50, 0.40];
const HIT_GAP: [f64; 3] = [0.07, 0.07, 0.05];

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
    /// Onsets that arrived this frame as (band, strength, centroid, 0), for scenes that throw a burst of
    /// particles per hit. Weak ones and ones too close after the previous hit of their band are dropped.
    pub hits: [[f32; 4]; HIT_CAP],
}

pub struct Modulation {
    reference: [f32; 3],
    sus: [f32; 3],
    trend: f32,
    gate: f32,
    open: bool,
    rings: Vec<Ring>,
    last_ring: [f64; 3],
    last_hit: [f64; 3],
}

/// A peak cap pushes higher on every frame its level sits at the peak, so rises are summed, and a burst
/// is reported only once they add up to something visible, no more often than every `PEAK_HIT_GAP`
/// seconds per column. Small old rises fade away instead of adding up for ever.
pub struct PeakHits {
    prev: [f32; 11],
    acc: [f32; 11],
    last: [f64; 11],
}

/// How far a cap must have risen (of the full scale) for a burst, and the shortest gap between bursts.
const PEAK_HIT_MIN: f32 = 0.03;
const PEAK_HIT_GAP: f64 = 0.12;
const PEAK_HIT_MEMORY_S: f32 = 0.5;

impl PeakHits {
    pub fn new() -> PeakHits {
        PeakHits { prev: [0.0; 11], acc: [0.0; 11], last: [f64::NEG_INFINITY; 11] }
    }

    /// `peaks` are the caps' positions (0..1) this frame. Returns each column's burst strength, 0 for none.
    pub fn update(&mut self, peaks: &[f32; 11], now: f64, dt: f32) -> [f32; 11] {
        let mut out = [0.0; 11];
        for i in 0..11 {
            let rise = peaks[i] - self.prev[i];
            self.prev[i] = peaks[i];
            if rise > 0.0 {
                self.acc[i] += rise;
            } else {
                self.acc[i] *= (-dt / PEAK_HIT_MEMORY_S).exp();
            }
            if self.acc[i] >= PEAK_HIT_MIN && now - self.last[i] >= PEAK_HIT_GAP {
                out[i] = self.acc[i];
                self.acc[i] = 0.0;
                self.last[i] = now;
            }
        }
        out
    }
}

fn smooth(cur: &mut f32, target: f32, dt: f32, tau_up: f32, tau_down: f32) {
    let tau = if target > *cur { tau_up } else { tau_down };
    *cur += (target - *cur) * (1.0 - (-dt / tau).exp());
}

impl Modulation {
    pub fn new() -> Modulation {
        Modulation { reference: [0.25; 3], sus: [0.0; 3], trend: 0.0, gate: 0.0, open: false, rings: Vec::new(), last_ring: [f64::NEG_INFINITY; 3], last_hit: [f64::NEG_INFINITY; 3] }
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

        // ---- hits: the same events, as discrete accents for particle bursts
        let mut hits = [[0.0f32; 4]; HIT_CAP];
        let mut n = 0;
        for e in fresh {
            let b = (e.band as usize).min(2);
            if self.gate > 0.5 && n < HIT_CAP && e.strength >= HIT_MIN[b] && e.t - self.last_hit[b] >= HIT_GAP[b] {
                self.last_hit[b] = e.t;
                hits[n] = [b as f32, e.strength, e.centroid, 0.0];
                n += 1;
            }
        }
        Signals { fx, sus: [self.sus[0], self.sus[1], self.sus[2], self.trend], gate: self.gate, rings, hits }
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

    #[test]
    fn peak_bursts_need_a_real_push_and_are_rate_limited() {
        let mut ph = PeakHits::new();
        let mut caps = [0.0f32; 11];
        let dt = 1.0 / 60.0;
        let mut t = 0.0;
        // a cap that creeps up a hair (1.5% of the scale in half a second) is no event
        let mut bursts = 0;
        for _ in 0..30 {
            caps[3] += 0.0005;
            t += dt as f64;
            bursts += (ph.update(&caps, t, dt)[3] > 0.0) as u32;
        }
        assert_eq!(bursts, 0, "bursts from a slow creep");
        // ...but a steady climb of 6% in that time is a push, reported in pieces no faster than the rate limit
        let mut ph2 = PeakHits::new();
        let mut caps2 = [0.0f32; 11];
        let mut n = 0;
        for k in 0..30 {
            caps2[3] += 0.002;
            n += (ph2.update(&caps2, k as f64 * dt as f64, dt)[3] > 0.0) as u32;
        }
        assert_eq!(n, 2, "a 6% climb gives two bursts of 3%");
        // a clear push is one burst, and its strength is how far the cap rose
        let mut ph = PeakHits::new();
        let mut caps = [0.0f32; 11];
        caps[5] = 0.4;
        let out = ph.update(&caps, 1.0, dt);
        assert!((out[5] - 0.4).abs() < 1e-6 && out[4] == 0.0);
        // a second push right after is held back by the rate limit, then reported with both rises
        caps[5] = 0.5;
        assert_eq!(ph.update(&caps, 1.03, dt)[5], 0.0);
        caps[5] = 0.55;
        let later = ph.update(&caps, 1.15, dt)[5];
        assert!((later - 0.15).abs() < 1e-6, "got {later}");
        // a falling cap is no event
        caps[5] = 0.3;
        assert_eq!(ph.update(&caps, 2.0, dt)[5], 0.0);
    }

    #[test]
    fn hits_are_discrete_and_rate_limited() {
        let mut m = Modulation::new();
        let frac = [0.5f32; 11];
        for i in 0..120 {
            m.update(i as f64 / 60.0, 1.0 / 60.0, &lv(0.0, 0.0), &[], &frac);
        }
        let kick = Onset { band: 0, strength: 0.9, centroid: 0.15, t: 2.0 };
        let s = m.update(2.01, 1.0 / 60.0, &lv(1.0, 2.0), &[kick], &frac);
        assert_eq!(s.hits[0], [0.0, 0.9, 0.15, 0.0]);
        assert_eq!(s.hits[1][1], 0.0, "one hit reported once");
        // the next frame carries no event, so no hit; a second low onset right after the first is dropped
        let s = m.update(2.02, 1.0 / 60.0, &lv(1.0, 2.0), &[], &frac);
        assert_eq!(s.hits[0][1], 0.0);
        let again = Onset { band: 0, strength: 0.9, centroid: 0.15, t: 2.03 };
        let s = m.update(2.04, 1.0 / 60.0, &lv(1.0, 2.03), &[again], &frac);
        assert_eq!(s.hits[0][1], 0.0, "too soon after the last one");
        // a weak onset is below the threshold, a mid onset is its own band
        let weak = Onset { band: 1, strength: 0.2, centroid: 0.5, t: 3.0 };
        let mid = Onset { band: 1, strength: 0.8, centroid: 0.5, t: 3.0 };
        assert_eq!(m.update(3.01, 1.0 / 60.0, &lv(0.0, 3.0), &[weak], &frac).hits[0][1], 0.0);
        assert_eq!(m.update(3.02, 1.0 / 60.0, &lv(0.0, 3.0), &[mid], &frac).hits[0][..2], [1.0, 0.8]);
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
