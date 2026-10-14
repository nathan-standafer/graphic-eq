//! Curated visual presets: palette, exposure, bloom, motion and idle-camera combinations for each
//! scene. They are separate from the EQ presets and never touch gains, bypass or routing.

use crate::render::BLOOM_LEVELS;
use crate::scenes::{AURORA, CHLADNI, COLUMNS, FERROFLUID, FIREWORKS, FOUNTAIN, SMOKE, TUNNEL, VECTORSCOPE, WATERFALL};

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
/// The smoke is a flat picture with no camera.
const SMOKE_IDLE: Idle = Idle { yaw: 0.0, pitch: 0.0, zoom: 0.0, roll: 0.0 };
/// The plates are a flat picture too.
const SAND_IDLE: Idle = Idle { yaw: 0.0, pitch: 0.0, zoom: 0.0, roll: 0.0 };
/// The orb is round, so a slow turn about it only slides the reflections; the pitch and dolly stay small.
const ORB_IDLE: Idle = Idle { yaw: 0.10, pitch: 0.025, zoom: 0.04, roll: 0.0 };

/// The aurora is seen from a fixed spot at the water's edge: the idle path only sways the view a few degrees.
const AURORA_IDLE: Idle = Idle { yaw: 0.03, pitch: 0.010, zoom: 0.02, roll: 0.0 };

/// The fireworks are watched from a fixed spot low on the glass: the idle path sways the view a few degrees.
const FIREWORKS_IDLE: Idle = Idle { yaw: 0.04, pitch: 0.012, zoom: 0.03, roll: 0.0 };

pub const LOOKS: [Look; 38] = [
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
    // ---- smoke jets: the smoke is soft, so the halo is wide and gentle and the highlights stay near the lamps
    // (Motion is turbulence here: how much the plumes wobble and roll up)
    Look {
        scene: SMOKE, name: "Ice Vent", theme: 2, exposure: 1.0,
        bloom: [0.26, 0.34, 0.32, 0.24, 0.15, 0.09], threshold: 0.85, knee: 0.5, vignette: 0.55,
        motion: 0.6, flash: 0.8, idle: SMOKE_IDLE,
    },
    Look {
        scene: SMOKE, name: "Ember Smoke", theme: 1, exposure: 1.0,
        bloom: [0.24, 0.32, 0.34, 0.26, 0.17, 0.10], threshold: 0.8, knee: 0.5, vignette: 0.6,
        motion: 0.6, flash: 0.8, idle: SMOKE_IDLE,
    },
    Look {
        scene: SMOKE, name: "Neon Haze", theme: 3, exposure: 0.95,
        bloom: [0.26, 0.34, 0.32, 0.24, 0.15, 0.09], threshold: 0.85, knee: 0.5, vignette: 0.55,
        motion: 0.6, flash: 0.8, idle: SMOKE_IDLE,
    },
    Look {
        scene: SMOKE, name: "Prism Plumes", theme: 4, exposure: 0.9,
        bloom: [0.24, 0.32, 0.30, 0.22, 0.14, 0.08], threshold: 0.9, knee: 0.5, vignette: 0.5,
        motion: 0.6, flash: 0.7, idle: SMOKE_IDLE,
    },
    // ---- chladni sand: fine nodal lines have to survive, so the halo is tight and the broad glow nearly absent
    // (Motion is how hard a hit shakes the plates and how far the lamp drifts)
    Look {
        scene: CHLADNI, name: "Moonlit Sand", theme: 2, exposure: 1.0,
        bloom: [0.26, 0.28, 0.18, 0.09, 0.05, 0.03], threshold: 1.0, knee: 0.5, vignette: 0.55,
        motion: 0.6, flash: 0.8, idle: SAND_IDLE,
    },
    Look {
        scene: CHLADNI, name: "Amber Dust", theme: 1, exposure: 1.0,
        bloom: [0.26, 0.28, 0.20, 0.10, 0.06, 0.03], threshold: 0.95, knee: 0.5, vignette: 0.6,
        motion: 0.6, flash: 0.8, idle: SAND_IDLE,
    },
    Look {
        scene: CHLADNI, name: "Neon Grit", theme: 3, exposure: 0.95,
        bloom: [0.26, 0.30, 0.20, 0.10, 0.06, 0.03], threshold: 1.0, knee: 0.5, vignette: 0.55,
        motion: 0.6, flash: 0.8, idle: SAND_IDLE,
    },
    Look {
        scene: CHLADNI, name: "Prism Sand", theme: 4, exposure: 0.9,
        bloom: [0.24, 0.28, 0.18, 0.09, 0.05, 0.03], threshold: 1.0, knee: 0.5, vignette: 0.5,
        motion: 0.6, flash: 0.7, idle: SAND_IDLE,
    },
    // ---- ferrofluid orb: a black mirror, so the halo is tight and the broad glow faint; only the brightest cores of
    // the strip lights and the tips of the tallest spikes may bloom (Motion is how elastic the fluid is, Flash the
    // light pulses and tip glow)
    Look {
        scene: FERROFLUID, name: "Obsidian Ice", theme: 2, exposure: 1.0,
        bloom: [0.20, 0.22, 0.16, 0.10, 0.06, 0.04], threshold: 1.1, knee: 0.5, vignette: 0.55,
        motion: 0.6, flash: 0.7, idle: ORB_IDLE,
    },
    Look {
        scene: FERROFLUID, name: "Ember Core", theme: 1, exposure: 1.0,
        bloom: [0.20, 0.24, 0.18, 0.11, 0.07, 0.04], threshold: 1.05, knee: 0.5, vignette: 0.6,
        motion: 0.6, flash: 0.7, idle: ORB_IDLE,
    },
    Look {
        scene: FERROFLUID, name: "Neon Magnet", theme: 3, exposure: 0.95,
        bloom: [0.22, 0.26, 0.18, 0.11, 0.07, 0.04], threshold: 1.1, knee: 0.5, vignette: 0.55,
        motion: 0.6, flash: 0.8, idle: ORB_IDLE,
    },
    Look {
        scene: FERROFLUID, name: "Prism Oil", theme: 4, exposure: 0.9,
        bloom: [0.20, 0.22, 0.16, 0.10, 0.06, 0.04], threshold: 1.1, knee: 0.5, vignette: 0.5,
        motion: 0.6, flash: 0.7, idle: ORB_IDLE,
    },
    // ---- aurora: soft light, so the halo is wide and the broad glow generous; the sharp lower edges are the only
    // thing bright enough to bloom hard (Motion is how much the curtains fold and drift, Flash the highlights that
    // run up the rays)
    Look {
        scene: AURORA, name: "Boreal Green", theme: 0, exposure: 1.0,
        bloom: [0.18, 0.26, 0.30, 0.26, 0.18, 0.12], threshold: 0.7, knee: 0.6, vignette: 0.55,
        motion: 0.6, flash: 0.8, idle: AURORA_IDLE,
    },
    Look {
        scene: AURORA, name: "Polar Ice", theme: 2, exposure: 1.0,
        bloom: [0.18, 0.26, 0.30, 0.26, 0.18, 0.12], threshold: 0.7, knee: 0.6, vignette: 0.55,
        motion: 0.6, flash: 0.8, idle: AURORA_IDLE,
    },
    Look {
        scene: AURORA, name: "Neon Veil", theme: 3, exposure: 0.95,
        bloom: [0.20, 0.28, 0.30, 0.26, 0.18, 0.12], threshold: 0.7, knee: 0.6, vignette: 0.55,
        motion: 0.6, flash: 0.8, idle: AURORA_IDLE,
    },
    Look {
        scene: AURORA, name: "Prism Curtains", theme: 4, exposure: 0.9,
        bloom: [0.18, 0.24, 0.28, 0.24, 0.16, 0.10], threshold: 0.75, knee: 0.6, vignette: 0.5,
        motion: 0.6, flash: 0.7, idle: AURORA_IDLE,
    },
    // ---- fireworks: small bright stars want a tight halo, and the broad glow stays faint so the sky stays dark;
    // only the detonation flashes may bloom hard (Motion is how far the stars drift on the wind and how far the
    // camera wanders, Flash the detonation flashes and the light the bursts throw on the floor)
    Look {
        scene: FIREWORKS, name: "Gold Willow", theme: 1, exposure: 1.0,
        bloom: [0.30, 0.34, 0.26, 0.16, 0.09, 0.05], threshold: 0.9, knee: 0.5, vignette: 0.6,
        motion: 0.6, flash: 0.8, idle: FIREWORKS_IDLE,
    },
    Look {
        scene: FIREWORKS, name: "Ice Chrysanthemum", theme: 2, exposure: 1.0,
        bloom: [0.32, 0.36, 0.26, 0.16, 0.09, 0.05], threshold: 0.9, knee: 0.5, vignette: 0.55,
        motion: 0.6, flash: 0.8, idle: FIREWORKS_IDLE,
    },
    Look {
        scene: FIREWORKS, name: "Neon Peony", theme: 3, exposure: 0.95,
        bloom: [0.32, 0.38, 0.28, 0.16, 0.09, 0.05], threshold: 0.9, knee: 0.5, vignette: 0.55,
        motion: 0.6, flash: 0.8, idle: FIREWORKS_IDLE,
    },
    Look {
        scene: FIREWORKS, name: "Prism Finale", theme: 4, exposure: 0.9,
        bloom: [0.30, 0.34, 0.26, 0.16, 0.09, 0.05], threshold: 0.95, knee: 0.5, vignette: 0.5,
        motion: 0.6, flash: 0.7, idle: FIREWORKS_IDLE,
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
