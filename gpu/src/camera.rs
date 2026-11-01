//! Optional helicopter-style camera flight, independent of the audio signal.
use crate::scenes::{self, CamState};
use serde::{Deserialize, Serialize};
use std::f32::consts::TAU;

pub fn supports_flight(scene: u32) -> bool {
    matches!(scene, scenes::COLUMNS | scenes::WATERFALL | scenes::FOUNTAIN | scenes::FIREWORKS)
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct OrbitSettings {
    pub enabled: bool,
    pub seconds: f32,
}

impl Default for OrbitSettings {
    fn default() -> Self { Self { enabled: false, seconds: 60.0 } }
}

#[derive(Clone, Copy, Default)]
pub struct Flight {
    phase: f32,
    blend: f32,
    speed: f32,
    pause: f32,
}

impl Flight {
    pub fn step(&mut self, cam: &mut CamState, options: OrbitSettings, dt: f32, motion: f32, touched: bool) {
        let dt = dt.clamp(0.0, 0.1);
        if touched {
            self.pause = 4.0;
            self.speed = 0.0;
        }
        if self.pause > 0.0 {
            self.pause = (self.pause - dt).max(0.0);
            return;
        }
        // Motion=0 freezes the current flight pose, including its height and bank.
        if motion <= 0.0 { return; }
        let target = if options.enabled { 1.0 } else { 0.0 };
        let ease = 1.0 - (-dt / 2.0).exp();
        self.blend += (target - self.blend) * ease;
        self.speed += (target - self.speed) * ease;
        let seconds = if options.seconds.is_finite() { options.seconds.clamp(20.0, 180.0) } else { 60.0 };
        let delta = dt * self.speed * TAU / seconds;
        self.phase = (self.phase + delta).rem_euclid(TAU);
        // Accumulate yaw in the manual camera: stopping flight keeps the current heading.
        if delta > 0.0 {
            cam.yaw = (cam.yaw + delta + std::f32::consts::PI).rem_euclid(TAU) - std::f32::consts::PI;
        }
    }

    pub fn offsets(&self, idle: [f32; 4], aspect: f32) -> [f32; 4] {
        // These scenes spread their emitters on wide displays. Pull farther back so a
        // side-on flight does not run through the near end of the row.
        let wide = ((aspect - 1.5) * 0.6).clamp(0.0, 0.9);
        let path = [0.0, 0.30 + 0.06 * self.phase.sin(),
                    0.20 + wide + 0.08 * (1.0 - self.phase.cos()), 0.018 * self.phase.sin()];
        std::array::from_fn(|i| idle[i] * (1.0 - self.blend) + path[i] * self.blend)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flight_circles_continuously_and_pauses_for_manual_input_and_zero_motion() {
        let options = OrbitSettings { enabled: true, seconds: 20.0 };
        let mut flight = Flight::default();
        let mut cam = CamState::default();
        let mut travel = 0.0;
        for _ in 0..1800 {
            let old = cam.yaw;
            flight.step(&mut cam, options, 1.0 / 60.0, 1.0, false);
            travel += (cam.yaw - old).rem_euclid(TAU);
        }
        assert!(travel > TAU, "flight must complete full circles, not rock side to side");
        let heading = cam.yaw;
        let offsets = flight.offsets([0.0; 4], 1.5);
        for _ in 0..120 { flight.step(&mut cam, options, 1.0 / 60.0, 0.0, false); }
        assert_eq!(cam.yaw, heading);
        assert_eq!(flight.offsets([0.0; 4], 1.5), offsets);
        flight.step(&mut cam, options, 1.0 / 60.0, 1.0, true);
        for _ in 0..180 { flight.step(&mut cam, options, 1.0 / 60.0, 1.0, false); }
        assert_eq!(cam.yaw, heading);
        for _ in 0..180 { flight.step(&mut cam, options, 1.0 / 60.0, 1.0, false); }
        assert_ne!(cam.yaw, heading);
    }

    #[test]
    fn stopping_eases_out_without_resetting_heading() {
        let mut flight = Flight::default();
        let mut cam = CamState::default();
        let options = OrbitSettings { enabled: true, seconds: 60.0 };
        for _ in 0..600 { flight.step(&mut cam, options, 1.0 / 60.0, 1.0, false); }
        let heading = cam.yaw;
        let pitch = flight.offsets([0.0; 4], 1.5)[1];
        flight.step(&mut cam, OrbitSettings::default(), 1.0 / 60.0, 1.0, false);
        assert!((cam.yaw - heading).abs() < 0.002);
        assert!((flight.offsets([0.0; 4], 1.5)[1] - pitch).abs() < 0.004);
        for _ in 0..1800 { flight.step(&mut cam, OrbitSettings::default(), 1.0 / 60.0, 1.0, false); }
        assert!(cam.yaw >= heading);
        assert!(flight.offsets([0.0; 4], 1.5).iter().all(|x| x.abs() < 1e-5));
    }

    #[test]
    fn disabled_flight_preserves_manual_view_and_invalid_speed_is_safe() {
        let mut flight = Flight::default();
        let mut cam = CamState { yaw: 0.7, ..Default::default() };
        flight.step(&mut cam, OrbitSettings::default(), 0.1, 1.0, false);
        assert!((cam.yaw - 0.7).abs() < 1e-6);
        let idle = [0.1, 0.2, 0.03, 0.01];
        assert_eq!(flight.offsets(idle, 1.5), idle);
        flight.step(&mut cam, OrbitSettings { enabled: true, seconds: f32::NAN }, 0.1, 1.0, false);
        assert!(cam.yaw.is_finite());
        assert!(flight.offsets(idle, 1.5).iter().all(|x| x.is_finite()));
        for scene in scenes::ALL {
            assert_eq!(supports_flight(scene), [0, 1, 4, 9].contains(&scene));
        }
    }
}
