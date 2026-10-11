use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    pub gains: [f32; 11],
    pub preset: String,
    pub eq_on: bool,
    pub scene: u32,       // 0 columns, 1 waterfall
    pub theme: u32,       // 0 classic, 1 amber, 2 ice, 3 neon, 4 rainbow
    pub range: f32,       // dB below full scale shown
    pub sensitivity: f32, // dB added to what is drawn
    pub fall: u32,        // 0 slow, 1 medium, 2 fast
    pub bloom: f32,
    pub detail: u32,      // waterfall smoothing: 0 fine (1/6 oct), 1 medium (1/3), 2 smooth (2/3)
    pub peak_caps: bool,
    pub source_pre: bool, // analyze before the EQ
    pub output: Option<String>,
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
            detail: 1,
            peak_caps: true,
            source_pre: false,
            output: None,
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
        std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
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
