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

/// The LED strip of a 3D column has this many segments (`columns.wgsl`: `COL_MAX_H / SEG`), so a peak cap's
/// position is counted in them.
pub const LED_SEGMENTS: f32 = 30.0;

/// A peak cap pushes higher on every frame its level sits at the peak, so rises are summed over a short
/// window, and a burst is reported when a push has added up to `PEAK_HIT_MIN` and settled: one hit is one
/// burst, as big as the whole push (a hit's cap climbs over a few frames, and reporting the first part would
/// split it in two and understate both). Rises older than the window no longer count, so a slow climb never
/// adds up to a hit, and pushes that follow each other within `PEAK_HIT_GAP` are held and reported together.
pub struct PeakHits {
    prev: [f32; 11],
    /// Each column's recent rises as (time, size), the push now being summed.
    rises: [Vec<(f64, f32)>; 11],
    /// Time since the cap last rose, and when the push now being summed began.
    quiet: [f32; 11],
    since: [f64; 11],
    last: [f64; 11],
}

/// How far a cap must have risen (of the full scale) for a burst: five LED segments.
pub const PEAK_HIT_MIN: f32 = 5.0 / LED_SEGMENTS;
/// The shortest gap between bursts of one column, and how long a rise counts toward a push.
const PEAK_HIT_GAP: f64 = 0.12;
const PEAK_HIT_WINDOW_S: f64 = 0.25;
/// A push is over when the cap has not risen for this long (a frame between two analysis hops may see no
/// change), and is reported anyway once it has lasted `PEAK_HIT_MAX_WAIT`, so a long climb is not held back.
const PEAK_HIT_SETTLE_S: f32 = 0.025;
const PEAK_HIT_MAX_WAIT: f64 = 0.08;

impl PeakHits {
    pub fn new() -> PeakHits {
        PeakHits {
            prev: [0.0; 11],
            rises: std::array::from_fn(|_| Vec::new()),
            quiet: [1.0; 11],
            since: [f64::NEG_INFINITY; 11],
            last: [f64::NEG_INFINITY; 11],
        }
    }

    /// `peaks` are the caps' positions (0..1) this frame. Returns each column's burst strength (how far the
    /// cap rose, as a share of the full scale), 0 for none.
    pub fn update(&mut self, peaks: &[f32; 11], now: f64, dt: f32) -> [f32; 11] {
        let mut out = [0.0; 11];
        for i in 0..11 {
            let rise = peaks[i] - self.prev[i];
            self.prev[i] = peaks[i];
            if rise > 0.0 {
                if self.quiet[i] >= PEAK_HIT_SETTLE_S {
                    self.since[i] = now;
                }
                self.rises[i].push((now, rise));
                self.quiet[i] = 0.0;
            } else {
                self.quiet[i] += dt;
            }
            self.rises[i].retain(|(t, _)| now - t <= PEAK_HIT_WINDOW_S);
            let push: f32 = self.rises[i].iter().map(|(_, r)| r).sum();
            let over = self.quiet[i] >= PEAK_HIT_SETTLE_S || now - self.since[i] >= PEAK_HIT_MAX_WAIT;
            if push >= PEAK_HIT_MIN && over && now - self.last[i] >= PEAK_HIT_GAP {
                out[i] = push;
                self.rises[i].clear();
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

    /// Feeds one frame per entry of `rises` (in LED segments) to column 3, then `quiet` frames with no
    /// change, and returns each burst as (frame, segments).
    fn bursts_for(ph: &mut PeakHits, caps: &mut [f32; 11], t: &mut f64, rises: &[f32], quiet: usize) -> Vec<(usize, f32)> {
        let dt = 1.0 / 60.0;
        let mut out = Vec::new();
        for (k, r) in rises.iter().cloned().chain(std::iter::repeat(0.0).take(quiet)).enumerate() {
            caps[3] += r / LED_SEGMENTS;
            *t += dt as f64;
            let h = ph.update(caps, *t, dt)[3];
            if h > 0.0 {
                out.push((k, h * LED_SEGMENTS));
            }
        }
        out
    }

    #[test]
    fn peak_bursts_need_a_push_of_five_segments_and_are_sized_by_it() {
        let (mut ph, mut caps, mut t) = (PeakHits::new(), [0.0f32; 11], 0.0);
        // four segments in one frame is no event, five is
        assert!(bursts_for(&mut ph, &mut caps, &mut t, &[4.0], 30).is_empty(), "a four-segment push threw sparks");
        let (mut ph, mut caps, mut t) = (PeakHits::new(), [0.0f32; 11], 0.0);
        let b = bursts_for(&mut ph, &mut caps, &mut t, &[5.01], 30);
        assert_eq!(b.len(), 1, "a five-segment push is one burst");
        assert!((b[0].1 - 5.01).abs() < 0.01, "its strength is the push");
        // a hit whose cap climbs over four frames is one burst of the whole climb, after it has settled
        let (mut ph, mut caps, mut t) = (PeakHits::new(), [0.0f32; 11], 0.0);
        let b = bursts_for(&mut ph, &mut caps, &mut t, &[3.3, 3.8, 1.1, 1.0], 30);
        assert_eq!(b.len(), 1, "one hit, one burst: {b:?}");
        assert!((b[0].1 - 9.2).abs() < 0.01, "got {b:?}");
        assert!(b[0].0 <= 8, "reported within a few frames of the climb ending: {b:?}");
        // a bigger push is a bigger burst
        let (mut ph, mut caps, mut t) = (PeakHits::new(), [0.0f32; 11], 0.0);
        let big = bursts_for(&mut ph, &mut caps, &mut t, &[14.0], 30);
        assert!(big[0].1 > b[0].1);
        // and only the column that rose reports
        let mut ph = PeakHits::new();
        let mut caps = [0.0f32; 11];
        caps[5] = 0.4;
        let dt = 1.0 / 60.0;
        assert!(ph.update(&caps, 1.0, dt).iter().all(|v| *v == 0.0), "reported before the push settled");
        let settled = [1.0 + 1.0 * dt as f64, 1.0 + 2.0 * dt as f64].map(|now| ph.update(&caps, now, dt));
        assert!((settled[1][5] - 0.4).abs() < 1e-6 && settled[1][4] == 0.0, "{settled:?}");
    }

    #[test]
    fn slow_climbs_and_falling_caps_throw_nothing() {
        // a cap that creeps up a hair a frame is no event
        let (mut ph, mut caps, mut t) = (PeakHits::new(), [0.0f32; 11], 0.0);
        assert!(bursts_for(&mut ph, &mut caps, &mut t, &[0.05; 120], 30).is_empty(), "bursts from a slow creep");
        // a staircase of one segment every tenth of a second (3 segments a second) never adds up
        let (mut ph, mut caps, mut t) = (PeakHits::new(), [0.0f32; 11], 0.0);
        let stairs: Vec<f32> = (0..300).map(|k| if k % 6 == 0 { 1.0 } else { 0.0 }).collect();
        assert!(bursts_for(&mut ph, &mut caps, &mut t, &stairs, 30).is_empty(), "bursts from a staircase");
        // a falling cap is no event
        let mut ph = PeakHits::new();
        let mut caps = [0.0f32; 11];
        caps[5] = 0.8;
        ph.update(&caps, 1.0, 1.0 / 60.0);
        caps[5] = 0.3;
        assert_eq!(ph.update(&caps, 2.0, 1.0 / 60.0)[5], 0.0);
    }

    #[test]
    fn a_long_climb_is_reported_without_waiting_for_it_to_end_and_bursts_are_rate_limited() {
        let (mut ph, mut caps, mut t) = (PeakHits::new(), [0.0f32; 11], 0.0);
        // two segments every frame for a second: reported while it climbs, no faster than the rate limit
        let b = bursts_for(&mut ph, &mut caps, &mut t, &[2.0; 60], 0);
        assert!(b.len() >= 5 && b.len() <= 9, "got {b:?}");
        assert!(b.windows(2).all(|w| (w[1].0 - w[0].0) as f64 / 60.0 >= PEAK_HIT_GAP - 1e-6), "{b:?}");
        // a second push right after a burst is held back by the rate limit, then reported
        let (mut ph, mut caps, mut t) = (PeakHits::new(), [0.0f32; 11], 0.0);
        let b = bursts_for(&mut ph, &mut caps, &mut t, &[8.0, 0.0, 0.0, 0.0, 0.0, 0.0, 10.0], 30);
        assert_eq!(b.len(), 2, "{b:?}");
        assert!((b[1].0 - b[0].0) as f64 / 60.0 >= PEAK_HIT_GAP - 1e-6, "{b:?}");
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
