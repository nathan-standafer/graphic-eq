//! The app clock: seconds since startup, shared by every thread so audio events and frames agree.
//! Deterministic captures drive their own simulated time instead (see `audio::Audio`).

use std::sync::OnceLock;
use std::time::Instant;

static EPOCH: OnceLock<Instant> = OnceLock::new();

pub fn init() {
    EPOCH.get_or_init(Instant::now);
}

/// Wall-clock seconds since `init`.
pub fn now() -> f64 {
    EPOCH.get_or_init(Instant::now).elapsed().as_secs_f64()
}
