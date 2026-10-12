use super::*;
use super::particles::{Particles, SpawnPlan};

/// Embers and sparks over the 3D columns: each lit column sheds embers from the top of its strip in
/// proportion to its level, and a column that pushes its peak cap higher throws a fan of sparks. A small
/// system on the same compute machinery as the fountain, drawn inside the columns' own pass so the
/// columns' depth hides what is behind them.
pub struct Sparks {
    parts: Particles,
    sprites: wgpu::RenderPipeline,
    refl: wgpu::RenderPipeline,
    carry: f32,
}

const COUNT: u32 = 16_384;
const MAX_LIFE: f32 = 3.0;
/// Ember spawn slots per second for one fully lit column.
const EMBER_RATE: f32 = 60.0;
/// A column must be above this level to shed embers.
const EMBER_FLOOR: f32 = 0.12;

impl Sparks {
    pub fn new(ctx: &Ctx) -> Sparks {
        let parts = Particles::new(ctx, "sparks", include_str!("sparks_sim.wgsl"), "cs_main", COUNT, MAX_LIFE);
        let src = format!("{}\n{}\n{}", include_str!("particles_common.wgsl"), include_str!("particles_draw.wgsl"), include_str!("sparks.wgsl"));
        let sh = ctx.module("sparks", &src);
        // additive, depth-tested against the columns, no depth write; the columns' pass is multisampled
        let glow = Pipe { depth_write: false, always: false, blend: ADDITIVE, msaa: true };
        Sparks {
            sprites: ctx.pipeline_with(&parts.draw_layout, &sh, "sparks", "vs_spark", "fs_sprite", glow),
            refl: ctx.pipeline_with(&parts.draw_layout, &sh, "sparks_refl", "vs_spark_refl", "fs_sprite", glow),
            parts,
            carry: 0.0,
        }
    }

    pub fn update(&mut self, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, f: &Frame) {
        let Some(dt) = self.parts.begin(f) else { return };
        let p = f.p;
        // which column sheds: running sums of a weight that grows with the level above a floor
        let mut cdf = [0f32; 12];
        let mut sum = 0.0;
        for i in 0..11 {
            let l = p.levels[i];
            if l > EMBER_FLOOR {
                sum += ((l - EMBER_FLOOR) / (1.0 - EMBER_FLOOR)).powf(1.6);
            }
            cdf[i] = sum;
        }
        self.carry += EMBER_RATE * sum * p.gate.clamp(0.0, 1.0) * p.density.clamp(0.1, 2.0) * dt;
        let cont = self.carry.floor();
        self.carry -= cont;
        let mut plan = SpawnPlan { cont: cont as u32, bursts: Vec::new() };
        for (i, s) in p.peak_hits.iter().enumerate().filter(|(_, s)| **s > 0.0) {
            if plan.bursts.len() < 6 {
                plan.bursts.push([i as f32, *s, 0.0, (30.0 + 140.0 * s.min(1.0)) * p.gate]);
            }
        }
        let extra = [[cdf[0], cdf[1], cdf[2], cdf[3]], [cdf[4], cdf[5], cdf[6], cdf[7]], [cdf[8], cdf[9], cdf[10], 0.0], [0.0; 4]];
        self.parts.advance(queue, enc, f, dt, &plan, extra);
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.refl);
        self.parts.draw(pass, 0..6);
        pass.set_pipeline(&self.sprites);
        self.parts.draw(pass, 0..6);
    }

    pub fn reset(&mut self, seed: u64) {
        self.parts.reset(seed);
        self.carry = 0.0;
    }
}
