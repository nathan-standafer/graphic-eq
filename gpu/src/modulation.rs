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
