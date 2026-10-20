use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// Synthetic-audio runs must not overwrite the user's saved settings.
static READ_ONLY: AtomicBool = AtomicBool::new(false);

pub fn set_read_only() {
    READ_ONLY.store(true, Ordering::Relaxed);
}

/// A user's saved changes to a visual preset (the preset itself is never overwritten).
#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct Variant {
    pub theme: u32,
    pub motion: f32,
    pub flash: f32,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    pub gains: [f32; 11],
    pub preset: String,
    pub eq_on: bool,
    pub scene: u32,       // Stable ID from scenes; retired IDs fall back to columns at startup.
    pub theme: u32,       // 0 classic, 1 amber, 2 ice, 3 neon, 4 rainbow
    pub range: f32,       // dB below full scale shown
    pub sensitivity: f32, // dB added to what is drawn
    pub fall: u32,        // 0 slow, 1 medium, 2 fast
    pub bloom: f32,
    pub stereo: bool,     // waterfall: overlay left and right channels
    pub detail: u32,      // waterfall smoothing: 0 fine (1/6 oct), 1 medium (1/3), 2 smooth (2/3)
    pub peak_caps: bool,
    pub source_pre: bool, // analyze before the EQ
    pub output: Option<String>,
    pub motion: f32,      // camera and geometry movement, 0 = still
    pub flash: f32,       // onset accents and flashes, 0 = none
    #[serde(deserialize_with = "looks_compat")]
    pub look: [u32; SCENES], // selected visual preset per scene
    pub sparks: bool,     // columns: embers and sparks over the columns
    pub density: f32,     // fountain particle density (1 = the designed amount)
    pub spark_sensitivity: f32, // fountain: how close to the music's hardest hits a rise must get to throw sparks (1 = designed)
    pub orb_spikes: f32,  // ferrofluid orb: how tall the spikes stand (1 = designed)
    pub aurora_curtains: u32, // aurora: most curtains shown (the picture's shape may allow fewer)
    pub ripple_life: f32, // ripple pond: how long a ripple lives (1 = designed)
    pub favorites: Vec<String>,
    pub variants: BTreeMap<String, Variant>,
    pub crossfade: bool,
    pub auto_cycle: bool,
    pub cycle_secs: f32,
}

/// Saved looks indexed by stable scene ID. Keep retired slots so later scenes retain their looks.
pub const SCENES: usize = crate::scenes::ID_SLOTS;

fn looks_compat<'de, D: serde::Deserializer<'de>>(d: D) -> Result<[u32; SCENES], D::Error> {
    let v = Vec::<u32>::deserialize(d)?;
    let mut out = [0u32; SCENES];
    for (o, x) in out.iter_mut().zip(v) {
        *o = x;
    }
    Ok(out)
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            gains: [0.0; 11],
            preset: "Flat".into(),
            eq_on: true,
            scene: 0,
            theme: 0,
            range: 60.0,
            sensitivity: 0.0,
            fall: 1,
            bloom: 0.9,
            stereo: false,
            detail: 1,
            peak_caps: true,
            source_pre: false,
            output: None,
            motion: 0.7,
            flash: 0.8,
            look: [0; SCENES],
            sparks: true,
            density: 1.0,
            spark_sensitivity: 1.0,
            orb_spikes: 1.0,
            aurora_curtains: 6,
            ripple_life: 1.0,
            favorites: Vec::new(),
            variants: BTreeMap::new(),
            crossfade: true,
            auto_cycle: false,
            cycle_secs: 45.0,
        }
    }
}

fn path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("graphic-eq").join("gpu-settings.json")
}

impl Settings {
    pub fn load() -> Settings {
        if std::env::var("GEQ_SETTINGS").map_or(false, |v| v == "default") {
            return Settings::default();
        }
        std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        if READ_ONLY.load(Ordering::Relaxed) {
            return;
        }
        let p = path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(p, s);
        }
    }
}

pub const PRESETS: [(&str, [f32; 11]); 10] = [
    ("Flat", [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Smile (V)", [6.0, 5.0, 3.0, 1.0, -1.0, -2.0, -1.0, 1.0, 3.0, 5.0, 6.0]),
    ("Bass Boost", [7.0, 6.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Treble Boost", [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 3.0, 5.0, 6.0, 7.0]),
    ("Vocal", [-3.0, -2.0, -1.0, 1.0, 3.0, 4.0, 3.0, 1.0, 0.0, -1.0, -2.0]),
    ("Rock", [5.0, 4.0, 2.0, 0.0, -1.0, -1.0, 1.0, 3.0, 4.0, 5.0, 4.0]),
    ("Pop", [-1.0, 1.0, 3.0, 4.0, 3.0, 0.0, -1.0, -1.0, 0.0, 1.0, 2.0]),
    ("Jazz", [3.0, 2.0, 1.0, 2.0, -2.0, -2.0, 0.0, 1.0, 2.0, 3.0, 3.0]),
    ("Classical", [4.0, 3.0, 2.0, 1.0, 0.0, 0.0, 0.0, -1.0, -1.0, -2.0, -3.0]),
    ("Electronic", [6.0, 5.0, 2.0, 0.0, -2.0, 2.0, 0.0, 1.0, 4.0, 5.0, 4.0]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retired_scenes_fall_back_without_shifting_saved_looks() {
        for scene in [2, 3, 5, 6, 99] {
            let s: Settings = serde_json::from_value(serde_json::json!({
                "scene": scene,
                "look": [0, 1, 2, 3, 1, 2, 3, 2, 1, 3, 2, 1],
                "persistence": 0.5,
                "smoke_density": 1.5,
                "sand_grains": 0.5
            })).unwrap();
            assert_eq!(crate::scenes::info(s.scene).id, crate::scenes::COLUMNS);
            assert_eq!(s.look[crate::scenes::FERROFLUID as usize], 2);
            assert_eq!(s.look[crate::scenes::AURORA as usize], 1);
            assert_eq!(s.look[crate::scenes::FIREWORKS as usize], 3);
            assert_eq!(s.look[crate::scenes::LIGHTNING as usize], 2);
            assert_eq!(s.look[crate::scenes::RIPPLE as usize], 1);
        }
        for scene in crate::scenes::ALL {
            assert!((scene as usize) < SCENES);
            assert_eq!(crate::scenes::info(scene).id, scene);
            assert_eq!(crate::presets::get(scene, 0).scene, scene);
        }
    }

    #[test]
    fn old_settings_with_fewer_looks_still_load() {
        let s: Settings = serde_json::from_str(r#"{"scene": 2, "look": [1, 2, 3], "bloom": 1.2}"#).unwrap();
        assert_eq!(s.look, [1, 2, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(s.scene, 2);
        assert!((s.bloom - 1.2).abs() < 1e-6);
        assert!(s.sparks && (s.density - 1.0).abs() < 1e-6);
        // the fountain's sensitivity changed meaning (relative to the music's own hits), so the old key is not read
        let s: Settings = serde_json::from_str(r#"{"gradient": 3.0}"#).unwrap();
        assert!((s.spark_sensitivity - 1.0).abs() < 1e-6);
        let s: Settings = serde_json::from_str(r#"{"look": [0, 1, 2, 3]}"#).unwrap();
        assert_eq!(s.look, [0, 1, 2, 3, 0, 0, 0, 0, 0, 0, 0, 0]);
        let s: Settings = serde_json::from_str(r#"{"look": [0, 1, 2, 3, 4], "sparks": false}"#).unwrap();
        assert_eq!(s.look, [0, 1, 2, 3, 4, 0, 0, 0, 0, 0, 0, 0]);
        assert!((s.orb_spikes - 1.0).abs() < 1e-6 && s.aurora_curtains == 6 && (s.ripple_life - 1.0).abs() < 1e-6);
        assert!(!s.sparks);
    }
}
