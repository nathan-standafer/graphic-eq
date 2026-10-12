use super::*;
use super::particles::{Particles, SpawnPlan};

/// Sparks fly where the volume is rising. Each of the 192 frequency bins reports how steeply its level is
/// climbing; a spawn picks a frequency by that rise, above an adjustable threshold, and launches a spark
/// from its place on a line along the floor (bass on the left, treble on the right). A kick is a steep
/// rise in the lows, a snare in the highs, steady sound no rise at all, and the steeper the rise the more
/// sparks, and the bigger, brighter and higher they fly. They arc, bounce on a glass floor that mirrors
/// them, and a compute pass (see `particles.rs`) simulates up to 262,144 of them.
pub struct Fountain {
    floor: wgpu::RenderPipeline,
    refl: wgpu::RenderPipeline,
    sprites: wgpu::RenderPipeline,
    parts: Particles,
    /// Fractional spawns carried to the next frame, so the rate is exact at any frame rate.
    carry: f32,
}

const COUNT: u32 = 262_144;
/// Longest a particle can live (see `fountain_sim.wgsl`), seconds.
const MAX_LIFE: f32 = 4.0;
/// Spawn slots per second. Most are turned down by the shader when little is rising; during a hit that
/// excites many frequencies nearly all of them throw a spark.
const RATE: f32 = 6_000.0;

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
        }
    }

    /// The emitter line's width: it fills most of the screen at any aspect, within sensible bounds.
    fn width(aspect: f32) -> f32 {
        (8.5 * aspect).clamp(10.0, 34.0)
    }

    /// This frame's spawn slots. The shader decides which of them become sparks, and where, from the rise.
    fn plan(&mut self, f: &Frame, dt: f32) -> SpawnPlan {
        let p = f.p;
        // nothing is thrown when the gate is closed; what already flies expires on its own
        self.carry += RATE * p.density.clamp(0.1, 2.0) * p.gate.clamp(0.0, 1.0) * dt;
        let cont = self.carry.floor();
        self.carry -= cont;
        SpawnPlan { cont: cont as u32, bursts: Vec::new() }
    }
}

impl Scene for Fountain {
    fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, f: &Frame) {
        if std::env::var_os("GEQ_PARTDEBUG").is_some() && f.frame_no % 30 == 0 {
            self.parts.debug_report(device, queue, "fountain");
        }
        if let Some(dt) = self.parts.begin(f) {
            let plan = self.plan(f, dt);
            self.parts.advance(queue, enc, f, dt, &plan, [[f.p.gradient.clamp(0.2, 5.0), 0.0, 0.0, 0.0], [0.0; 4], [0.0; 4], [0.0; 4]]);
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
    }
}
