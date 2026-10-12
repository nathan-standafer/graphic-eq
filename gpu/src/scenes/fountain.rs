use super::*;
use super::particles::{Particles, SpawnPlan};

/// The 192-bin spectrum line launches particles up into arcs: each jet's speed follows its level, bass is
/// broad heavy arcs and treble fine spray, onsets throw bursts, and everything bounces on a glass floor
/// that mirrors it. A compute pass (see `particles.rs`) simulates up to 262,144 particles.
pub struct Fountain {
    floor: wgpu::RenderPipeline,
    refl: wgpu::RenderPipeline,
    sprites: wgpu::RenderPipeline,
    parts: Particles,
    /// Fractional spawns carried to the next frame, so the rate is exact at any frame rate.
    carry: f32,
    last_hit: [f64; 3],
}

const COUNT: u32 = 262_144;
/// Longest a particle can live (see `fountain_sim.wgsl`), seconds.
const MAX_LIFE: f32 = 4.0;
/// Continuous spawn slots per second at full energy (jets thin themselves out below that).
const RATE: f32 = 2_600.0;
/// Particles in a full-strength burst, per onset band (low, mid, high). Onsets arrive several times a
/// second, so these set most of the density; a clear hit has strength 1-2 (`h[1]` below).
const BURST: [f32; 3] = [450.0, 150.0, 250.0];
/// Bursts weaker than this are ignored, per band, and the same band cannot burst more often than `HIT_GAP`.
const HIT_MIN: [f32; 3] = [0.30, 0.50, 0.40];
const HIT_GAP: f64 = 0.07;

impl Fountain {
    pub fn new(ctx: &Ctx) -> Fountain {
        let parts = Particles::new(ctx, "fountain", include_str!("fountain_sim.wgsl"), "cs_main", COUNT, MAX_LIFE);
        let src = format!("{}\n{}\n{}", include_str!("particles_common.wgsl"), include_str!("particles_draw.wgsl"), include_str!("fountain.wgsl"));
        let sh = ctx.module("fountain", &src);
        let floor = Pipe { depth_write: false, always: false, blend: None, msaa: false };
        let glow = Pipe { depth_write: false, always: false, blend: ADDITIVE, msaa: false };
        Fountain {
            floor: ctx.pipeline(&sh, "fountain_floor", "vs_floor", "fs_floor", floor),
            refl: ctx.pipeline_with(&parts.draw_layout, &sh, "fountain_refl", "vs_part_refl", "fs_sprite", glow),
            sprites: ctx.pipeline_with(&parts.draw_layout, &sh, "fountain_sprites", "vs_part", "fs_sprite", glow),
            parts,
            carry: 0.0,
            last_hit: [f64::NEG_INFINITY; 3],
        }
    }

    /// The emitter line's width: it fills most of the screen at any aspect, within sensible bounds.
    fn width(aspect: f32) -> f32 {
        (8.5 * aspect).clamp(10.0, 34.0)
    }

    /// This frame's spawns, from the signal: a continuous stream whose rate follows the spectrum's energy,
    /// plus a burst for each onset.
    fn plan(&mut self, f: &Frame, dt: f32) -> SpawnPlan {
        let p = f.p;
        let density = p.density.clamp(0.1, 2.0);
        // emission stops when the gate closes; what already flies expires on its own
        let energy = (p.levels.iter().sum::<f32>() / 11.0).clamp(0.0, 1.0);
        let curve = ((energy - 0.04) / 0.5).clamp(0.0, 1.0).powf(1.2);
        self.carry += RATE * density * p.gate.clamp(0.0, 1.0) * curve * dt;
        let cont = self.carry.floor();
        self.carry -= cont;
        let mut plan = SpawnPlan { cont: cont as u32, bursts: Vec::new() };
        let t = p.time as f64;
        for h in p.hits.iter().filter(|h| h[1] > 0.0) {
            let band = (h[0] as usize).min(2);
            if h[1] >= HIT_MIN[band] && t - self.last_hit[band] >= HIT_GAP && plan.bursts.len() < 6 {
                self.last_hit[band] = t;
                plan.bursts.push([h[2].clamp(0.0, 1.0), h[1], band as f32, BURST[band] * density * (h[1] / 1.5).clamp(0.25, 1.0) * p.gate]);
            }
        }
        plan
    }
}

impl Scene for Fountain {
    fn update(&mut self, _device: &wgpu::Device, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, f: &Frame) {
        if let Some(dt) = self.parts.begin(f) {
            let plan = self.plan(f, dt);
            self.parts.advance(queue, enc, f, dt, &plan, [[0.0; 4]; 4]);
        }
    }

    fn uses_update(&self, _p: &SceneParams) -> bool {
        true
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, _f: &Frame) {
        pass.set_pipeline(&self.floor);
        pass.draw(0..6, 0..1);
        pass.set_pipeline(&self.refl);
        self.parts.draw(pass, 0..6);
        pass.set_pipeline(&self.sprites);
        self.parts.draw(pass, 0..6);
    }

    fn wants_msaa(&self) -> bool {
        false // soft sprites and an analytically anti-aliased grid
    }

    fn camera(&self, aspect: f32, p: &SceneParams) -> (Mat4, Vec3) {
        let o = Orbit { eye: Vec3::new(0.0, 2.6, 12.5), target: Vec3::new(0.0, 3.2, 0.0), fov_deg: 46.0 };
        orbit(&o, aspect, &p.cam)
    }

    fn shape(&self, aspect: f32) -> [f32; 3] {
        [Self::width(aspect), 0.0, 0.0]
    }

    fn reset(&mut self, seed: u64) {
        self.parts.reset(seed);
        self.carry = 0.0;
        self.last_hit = [f64::NEG_INFINITY; 3];
    }
}
