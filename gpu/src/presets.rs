//! Curated visual presets: palette, exposure, bloom, motion and idle-camera combinations for each
//! scene. They are separate from the EQ presets and never touch gains, bypass or routing.

use crate::render::BLOOM_LEVELS;
use crate::scenes::{COLUMNS, FOUNTAIN, TUNNEL, VECTORSCOPE, WATERFALL};

/// Idle camera path amplitudes at Motion = 1: yaw, pitch (radians), dolly (fraction), roll (radians).
#[derive(Clone, Copy)]
pub struct Idle {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    pub roll: f32,
}

#[derive(Clone, Copy)]
pub struct Look {
    pub scene: u32,
    pub name: &'static str,
    pub theme: u32,
    pub exposure: f32,
    /// Contribution of each bloom level, ½ resolution (tight halo) to 1/64 (broad glow).
    pub bloom: [f32; BLOOM_LEVELS],
    pub threshold: f32,
    pub knee: f32,
    pub vignette: f32,
    /// Default Motion and Flash intensities.
    pub motion: f32,
    pub flash: f32,
    pub idle: Idle,
}

const COLUMN_IDLE: Idle = Idle { yaw: 0.10, pitch: 0.03, zoom: 0.05, roll: 0.0 };
const TERRAIN_IDLE: Idle = Idle { yaw: 0.07, pitch: 0.02, zoom: 0.04, roll: 0.0 };
const TUNNEL_IDLE: Idle = Idle { yaw: 0.06, pitch: 0.04, zoom: 0.05, roll: 0.07 };
/// The scope is a flat instrument: nothing drifts.
const SCOPE_IDLE: Idle = Idle { yaw: 0.0, pitch: 0.0, zoom: 0.0, roll: 0.0 };
const FOUNTAIN_IDLE: Idle = Idle { yaw: 0.07, pitch: 0.02, zoom: 0.04, roll: 0.0 };

pub const LOOKS: [Look; 18] = [
    // ---- columns
    Look {
        scene: COLUMNS, name: "Studio", theme: 0, exposure: 1.0,
        bloom: [0.30, 0.36, 0.30, 0.20, 0.12, 0.08], threshold: 0.85, knee: 0.5, vignette: 0.55,
        motion: 0.6, flash: 0.8, idle: COLUMN_IDLE,
    },
    Look {
        scene: COLUMNS, name: "Amber Rack", theme: 1, exposure: 0.95,
        bloom: [0.26, 0.34, 0.34, 0.26, 0.16, 0.10], threshold: 0.8, knee: 0.5, vignette: 0.6,
        motion: 0.5, flash: 0.7, idle: COLUMN_IDLE,
    },
    Look {
        scene: COLUMNS, name: "Ice Deck", theme: 2, exposure: 1.0,
        bloom: [0.34, 0.38, 0.28, 0.18, 0.10, 0.06], threshold: 0.85, knee: 0.5, vignette: 0.55,
        motion: 0.5, flash: 0.8, idle: COLUMN_IDLE,
    },
    // ---- waterfall
    Look {
        scene: WATERFALL, name: "Neon Terrain", theme: 3, exposure: 1.0,
        bloom: [0.34, 0.42, 0.36, 0.22, 0.12, 0.06], threshold: 0.85, knee: 0.5, vignette: 0.55,
        motion: 0.5, flash: 0.6, idle: TERRAIN_IDLE,
    },
    Look {
        scene: WATERFALL, name: "Classic Mesh", theme: 0, exposure: 1.0,
        bloom: [0.34, 0.42, 0.34, 0.2, 0.1, 0.05], threshold: 0.85, knee: 0.5, vignette: 0.55,
        motion: 0.4, flash: 0.6, idle: TERRAIN_IDLE,
    },
    Look {
        scene: WATERFALL, name: "Ice Sheet", theme: 2, exposure: 1.0,
        bloom: [0.36, 0.40, 0.30, 0.18, 0.10, 0.05], threshold: 0.85, knee: 0.5, vignette: 0.55,
        motion: 0.4, flash: 0.6, idle: TERRAIN_IDLE,
    },
    // ---- tunnel
    Look {
        scene: TUNNEL, name: "Neon Conduit", theme: 3, exposure: 1.0,
        bloom: [0.30, 0.38, 0.34, 0.26, 0.18, 0.12], threshold: 0.85, knee: 0.5, vignette: 0.5,
        motion: 0.7, flash: 0.8, idle: TUNNEL_IDLE,
    },
    Look {
        scene: TUNNEL, name: "Ice Core", theme: 2, exposure: 0.95,
        bloom: [0.34, 0.36, 0.26, 0.18, 0.12, 0.08], threshold: 0.85, knee: 0.5, vignette: 0.55,
        motion: 0.5, flash: 0.7, idle: TUNNEL_IDLE,
    },
    Look {
        scene: TUNNEL, name: "Ember Flue", theme: 1, exposure: 1.0,
        bloom: [0.28, 0.34, 0.36, 0.30, 0.22, 0.16], threshold: 0.8, knee: 0.5, vignette: 0.6,
        motion: 0.6, flash: 0.8, idle: TUNNEL_IDLE,
    },
    Look {
        scene: TUNNEL, name: "Prism", theme: 4, exposure: 0.9,
        bloom: [0.30, 0.36, 0.30, 0.22, 0.14, 0.08], threshold: 0.9, knee: 0.5, vignette: 0.5,
        motion: 0.7, flash: 0.7, idle: TUNNEL_IDLE,
    },
    // ---- vectorscope: a sharp core, a narrow halo and a faint broad glow, the way a phosphor tube blooms
    Look {
        scene: VECTORSCOPE, name: "P31 Green", theme: 0, exposure: 1.0,
        bloom: [0.34, 0.34, 0.26, 0.16, 0.09, 0.05], threshold: 0.8, knee: 0.5, vignette: 0.6,
        motion: 0.0, flash: 0.7, idle: SCOPE_IDLE,
    },
    Look {
        scene: VECTORSCOPE, name: "Amber Scope", theme: 1, exposure: 1.0,
        bloom: [0.30, 0.34, 0.30, 0.20, 0.12, 0.07], threshold: 0.8, knee: 0.5, vignette: 0.6,
        motion: 0.0, flash: 0.7, idle: SCOPE_IDLE,
    },
    Look {
        scene: VECTORSCOPE, name: "Ice Trace", theme: 2, exposure: 1.0,
        bloom: [0.36, 0.34, 0.24, 0.14, 0.08, 0.04], threshold: 0.8, knee: 0.5, vignette: 0.6,
        motion: 0.0, flash: 0.8, idle: SCOPE_IDLE,
    },
    Look {
        scene: VECTORSCOPE, name: "Neon Lissajous", theme: 3, exposure: 0.95,
        bloom: [0.34, 0.38, 0.30, 0.20, 0.12, 0.07], threshold: 0.85, knee: 0.5, vignette: 0.55,
        motion: 0.0, flash: 0.8, idle: SCOPE_IDLE,
    },
    // ---- fountain: bright small sprites want a tight halo and little broad glow, or the spray turns to fog
    Look {
        scene: FOUNTAIN, name: "Ice Geyser", theme: 2, exposure: 1.0,
        bloom: [0.32, 0.34, 0.24, 0.14, 0.08, 0.04], threshold: 0.85, knee: 0.5, vignette: 0.55,
        motion: 0.5, flash: 0.8, idle: FOUNTAIN_IDLE,
    },
    Look {
        scene: FOUNTAIN, name: "Ember Fountain", theme: 1, exposure: 1.0,
        bloom: [0.30, 0.34, 0.28, 0.18, 0.10, 0.05], threshold: 0.8, knee: 0.5, vignette: 0.6,
        motion: 0.5, flash: 0.8, idle: FOUNTAIN_IDLE,
    },
    Look {
        scene: FOUNTAIN, name: "Neon Spray", theme: 3, exposure: 0.95,
        bloom: [0.32, 0.36, 0.26, 0.16, 0.09, 0.05], threshold: 0.85, knee: 0.5, vignette: 0.55,
        motion: 0.5, flash: 0.8, idle: FOUNTAIN_IDLE,
    },
    Look {
        scene: FOUNTAIN, name: "Spectrum Jets", theme: 4, exposure: 0.9,
        bloom: [0.30, 0.34, 0.26, 0.16, 0.09, 0.05], threshold: 0.9, knee: 0.5, vignette: 0.5,
        motion: 0.5, flash: 0.7, idle: FOUNTAIN_IDLE,
    },
];

/// Presets of one scene, with their indices within that scene.
pub fn for_scene(scene: u32) -> Vec<(usize, &'static Look)> {
    LOOKS.iter().filter(|l| l.scene == scene).enumerate().collect()
}

pub fn get(scene: u32, index: usize) -> &'static Look {
    let list = for_scene(scene);
    list.get(index).or(list.first()).map(|(_, l)| *l).unwrap_or(&LOOKS[0])
}

/// Stable key for favorites and saved variants; survives reordering.
pub fn key(l: &Look) -> String {
    format!("{}.{}", l.scene, l.name)
}

/// Position of a look in the catalog (`LOOKS`), which is also its thumbnail index and number-key order.
pub fn catalog_index(l: &Look) -> usize {
    LOOKS.iter().position(|x| x.scene == l.scene && x.name == l.name).unwrap_or(0)
}
