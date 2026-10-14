use super::*;
use super::fireworks::{lerp, Rng};
use super::particles::{Particles, SpawnPlan};
use crate::modulation::{Hit, RiseHits};

/// Each hit strikes the glass floor as a bolt of branching light, where it landed on the line (kicks on the left,
/// snares on the right), and the floor lights up beneath it and mirrors it. Hits come from the same detector as
/// the Fireworks (`modulation::RiseHits`, the Fountain's rise judged against the music's own hardest hits), so
/// Sensitivity means the same thing. The bolt itself is built in the vertex shader from the strike's seed
/// (midpoint displacement, see `lightning.wgsl`); the CPU only keeps the table of strikes and decides how many
/// return strokes and branches each gets. Lows strike thick, with few branches, and send a thunder ring across
/// the floor; highs are thin, many-branched and short. Between strikes faint discharges stand on the spectrum
/// line and rise with the level.
pub struct Lightning {
    floor: wgpu::RenderPipeline,
    bolts: wgpu::RenderPipeline,
    bolts_refl: wgpu::RenderPipeline,
    corona: wgpu::RenderPipeline,
    corona_refl: wgpu::RenderPipeline,
    refl: wgpu::RenderPipeline,
    sprites: wgpu::RenderPipeline,
    parts: Particles,
    storm: Storm,
    hits: RiseHits,
    lt_buf: wgpu::Buffer,
    lt_bg: wgpu::BindGroup,
    last_frame: u64,
}

/// Splash sparks: capacity, and the longest one lives (see `lightning_sim.wgsl`).
const COUNT: u32 = 16_384;
const MAX_LIFE: f32 = 1.2;

/// Strikes the table holds; a strike stays in it as long as its pool of light and thunder ring.
const MAX_BOLTS: usize = 8;
const BOLT_LIFE_S: f32 = 1.9;
/// Segments of the main channel, of one branch, and branches at most (`lightning.wgsl` keeps the same numbers).
const MAIN_SEGS: u32 = 64;
const BRANCH_SEGS: u32 = 16;
const MAX_BRANCHES: u32 = 6;
const SEGS: u32 = MAIN_SEGS + MAX_BRANCHES * BRANCH_SEGS;
const CORONAS: u32 = 24;
const CORONA_SEGS: u32 = 8;
/// Strikes are scattered this far either side of the line in depth, so near ones are bigger and brighter.
const DEPTH: f32 = 5.0;
/// No part of the line flashes more than this many times a second: a strike that finds this many full strikes
/// within `FLASH_REGION` of its place in the last second is still drawn, but as a dim ghost with no flash on the
/// floor. (A bolt's return strokes are the flicker inside one flash, as in a real one; the floor's light comes up
/// in a few frames and does not follow them.)
const FLASHES_PER_S: usize = 3;
const FLASH_REGION: f32 = 0.20;

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How much of a low strike a position is (1 at the left end, 0 from the middle on), and of a high one.
pub fn lowness(line: f32) -> f32 {
    1.0 - smooth(0.15, 0.50, line)
}

pub fn highness(line: f32) -> f32 {
    smooth(0.50, 0.85, line)
}

/// Return strokes in a strike: a harder hit flickers more.
pub fn strokes_for(strength: f32) -> u32 {
    if strength < 0.5 { 1 } else if strength < 0.85 { 2 } else { 3 }
}

/// Branches of a strike: the highs have many, the lows few, a harder hit more; Density scales them.
pub fn branches_for(line: f32, strength: f32, density: f32) -> u32 {
    let n = (1.5 + 3.5 * line.clamp(0.0, 1.0)) * (0.5 + 0.7 * strength.clamp(0.0, 1.0)) * density.clamp(0.25, 1.5);
    (n.round() as u32).clamp(1, MAX_BRANCHES)
}

/// Where the channel starts: the lows come from the clouds above the frame, a high one is short and fades in from
/// nothing partway up.
pub fn top_for(line: f32) -> f32 {
    lerp(22.0, 13.0, highness(line))
}

/// World radius of the main channel's core: a hard hit and a low one are thick.
pub fn sigma_for(line: f32, strength: f32) -> f32 {
    (0.022 + 0.020 * strength.clamp(0.0, 1.0)) * (1.0 + 0.55 * lowness(line) - 0.30 * highness(line))
}

/// How far the thunder ring reaches in brightness: only the lows send one.
pub fn ring_for(line: f32, strength: f32) -> f32 {
    lowness(line) * (0.4 + 0.6 * strength.clamp(0.0, 1.0))
}

/// Spread of the pool of light on the glass (world sigma).
pub fn pool_for(line: f32, strength: f32) -> f32 {
    (1.4 + 1.8 * strength.clamp(0.0, 1.0)) * (1.0 + 0.3 * lowness(line))
}

/// Sparks thrown at the strike point.
pub fn sparks_for(strength: f32, density: f32) -> u32 {
    ((40.0 + 160.0 * strength.clamp(0.0, 1.0)) * density.clamp(0.1, 2.0)).round() as u32
}

/// One strike.
#[derive(Clone, Copy, Debug)]
pub struct Bolt {
    pub x: f32,
    pub z: f32,
    pub t0: f32,
    pub strength: f32,
    pub line: f32,
    pub strokes: u32,
    /// Too many flashes lately in this part of the line: drawn as a dim channel with no stroke flash or floor light.
    pub ghost: bool,
    pub seed: u32,
    pub branches: u32,
}

pub struct Input<'a> {
    pub now: f32,
    pub gate: f32,
    pub hits: &'a [Hit],
    /// Width of the line the strikes land along.
    pub width: f32,
    pub density: f32,
}

/// The choreography: hits become strikes. No GPU in here.
pub struct Storm {
    bolts: Vec<Bolt>,
    /// When and where (line position) the full strikes of the last second landed.
    flashes: Vec<(f32, f32)>,
    rng: Rng,
}

impl Storm {
    pub fn new(seed: u64) -> Storm {
        Storm { bolts: Vec::new(), flashes: Vec::new(), rng: Rng::seeded(seed) }
    }

    /// Empty sky, and the random sequence starts again.
    pub fn reset(&mut self, seed: u64) {
        *self = Storm::new(seed);
    }

    /// Empty sky, the random sequence carries on.
    pub fn clear(&mut self) {
        self.bolts.clear();
        self.flashes.clear();
    }

    /// Advance to `i.now` and return the strikes made this frame, strongest hit first.
    pub fn step(&mut self, i: &Input) -> Vec<Bolt> {
        let now = i.now;
        self.bolts.retain(|b| now - b.t0 < BOLT_LIFE_S);
        self.flashes.retain(|(t, _)| now - t < 1.0);
        let mut hits: Vec<Hit> = i.hits.to_vec();
        hits.sort_by(|a, b| b.strength.partial_cmp(&a.strength).unwrap());
        let mut out = Vec::new();
        if i.gate < 0.2 {
            return out; // the music has stopped: no new strikes, what is up fades out on its own
        }
        for h in hits {
            let ghost = self.flashes.iter().filter(|(_, l)| (l - h.pos).abs() < FLASH_REGION).count() >= FLASHES_PER_S;
            let b = Bolt {
                x: (h.pos - 0.5) * i.width,
                z: (self.rng.next() - 0.5) * 2.0 * DEPTH,
                t0: now,
                strength: h.strength.clamp(0.05, 1.0),
                line: h.pos,
                strokes: if ghost { 1 } else { strokes_for(h.strength) },
                ghost,
                seed: ((self.rng.next() * 16_777_000.0) as u32) | 1,
                branches: branches_for(h.pos, h.strength, i.density),
            };
            if !ghost {
                self.flashes.push((now, h.pos));
            }
            if self.bolts.len() >= MAX_BOLTS {
                self.bolts.remove(0);
            }
            self.bolts.push(b);
            out.push(b);
        }
        out
    }

    /// The GPU's table of strikes as they are at `now`.
    fn pack(&self, now: f32) -> LtU {
        let mut u = LtU::zeroed();
        for (k, b) in self.bolts.iter().take(MAX_BOLTS).enumerate() {
            u.bolts[4 * k] = [b.x, b.z, now - b.t0, b.strength];
            u.bolts[4 * k + 1] = [b.line, b.seed as f32, b.strokes as f32, top_for(b.line)];
            u.bolts[4 * k + 2] = [b.branches as f32, if b.ghost { 1.0 } else { 0.0 }, sigma_for(b.line, b.strength), ring_for(b.line, b.strength)];
            u.bolts[4 * k + 3] = [lerp(1.0, 0.7, highness(b.line)), pool_for(b.line, b.strength), 0.0, 0.0];
        }
        u
    }
}

/// Mirrors `LtU` in `lightning.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LtU {
    bolts: [[f32; 4]; 4 * MAX_BOLTS],
}

impl Lightning {
    pub fn new(ctx: &Ctx) -> Lightning {
        let dev = ctx.device;
        let parts = Particles::new(ctx, "lightning", include_str!("lightning_sim.wgsl"), "cs_main", COUNT, MAX_LIFE);
        let src = format!("{}\n{}\n{}", include_str!("particles_common.wgsl"), include_str!("particles_draw.wgsl"), include_str!("lightning.wgsl"));
        let sh = ctx.module("lightning", &src);

        // the table of strikes, in group 1 (the splash sparks use group 1 for the particles instead)
        let bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lightning_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let layout = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(ctx.bgl), Some(&bgl)], immediate_size: 0 });
        let lt_buf = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lightning_u"),
            size: std::mem::size_of::<LtU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let lt_bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lightning_bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry { binding: 2, resource: lt_buf.as_entire_binding() }],
        });

        let floor = Pipe { depth_write: false, always: false, blend: None, msaa: false };
        let glow = Pipe { depth_write: false, always: false, blend: ADDITIVE, msaa: false };
        Lightning {
            floor: ctx.pipeline_with(&layout, &sh, "lightning_floor", "vs_floor", "fs_floor", floor),
            bolts: ctx.pipeline_with(&layout, &sh, "lightning_bolts", "vs_bolt", "fs_bolt", glow),
            bolts_refl: ctx.pipeline_with(&layout, &sh, "lightning_bolts_refl", "vs_bolt_refl", "fs_bolt", glow),
            corona: ctx.pipeline_with(&layout, &sh, "lightning_corona", "vs_corona", "fs_bolt", glow),
            corona_refl: ctx.pipeline_with(&layout, &sh, "lightning_corona_refl", "vs_corona_refl", "fs_bolt", glow),
            refl: ctx.pipeline_with(&parts.draw_layout, &sh, "lightning_refl", "vs_spark_refl", "fs_sprite", glow),
            sprites: ctx.pipeline_with(&parts.draw_layout, &sh, "lightning_sparks", "vs_spark", "fs_sprite", glow),
            parts,
            storm: Storm::new(1),
            hits: RiseHits::new(),
            lt_buf,
            lt_bg,
            last_frame: 0,
        }
    }

    /// The line's width: it fills most of the picture at any aspect, within sensible bounds (the Fireworks' too,
    /// since a rectilinear view stretches whatever stands near the edges).
    fn width(aspect: f32) -> f32 {
        (12.5 * aspect).clamp(14.0, 40.0)
    }
}

impl Scene for Lightning {
    fn update(&mut self, _device: &wgpu::Device, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, f: &Frame) {
        let gap = f.frame_no.saturating_sub(self.last_frame);
        let Some(dt) = self.parts.begin(f) else { return };
        if self.last_frame == 0 || gap > 20 {
            // the sparks start again from an empty sky after a gap, and so does the storm
            self.storm.clear();
            self.hits.reset();
        }
        self.last_frame = f.frame_no;

        let p = f.p;
        let hits = self.hits.update(&p.rise, p.gradient.clamp(0.2, 5.0), p.time as f64, p.gate > 0.5);
        let strikes = self.storm.step(&Input { now: p.time, gate: p.gate, hits: &hits, width: Self::width(f.aspect), density: p.density });
        if std::env::var_os("GEQ_LTDEBUG").is_some() {
            // development aid: one line per strike, for checking timing against a fixture's exact event times
            for b in &strikes {
                eprintln!(
                    "[lightning] t={:.3} strike line {:.2} strength {:.2} strokes {} branches {}{} at ({:.1}, {:.1})",
                    p.time, b.line, b.strength, b.strokes, b.branches, if b.ghost { " (ghost)" } else { "" }, b.x, b.z
                );
            }
        }
        let mut plan = SpawnPlan::default();
        let mut extra = [[0.0f32; 4]; 4];
        for b in strikes.iter().filter(|b| !b.ghost).take(extra.len()) {
            let k = plan.bursts.len();
            plan.bursts.push([b.line, b.strength, b.seed as f32, sparks_for(b.strength, p.density) as f32]);
            extra[k] = [b.x, 0.0, b.z, 0.0];
        }
        self.parts.advance(queue, enc, f, dt, &plan, extra);
        queue.write_buffer(&self.lt_buf, 0, bytemuck::bytes_of(&self.storm.pack(p.time)));
    }

    fn uses_update(&self, _p: &SceneParams) -> bool {
        true
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, _f: &Frame) {
        pass.set_pipeline(&self.floor);
        pass.set_bind_group(1, &self.lt_bg, &[]);
        pass.draw(0..6, 0..1);
        pass.set_pipeline(&self.corona_refl);
        pass.draw(0..6, 0..CORONAS * CORONA_SEGS);
        pass.set_pipeline(&self.bolts_refl);
        pass.draw(0..6, 0..MAX_BOLTS as u32 * SEGS);
        pass.set_pipeline(&self.refl);
        self.parts.draw(pass, 0..6);
        pass.set_pipeline(&self.corona);
        pass.set_bind_group(1, &self.lt_bg, &[]);
        pass.draw(0..6, 0..CORONAS * CORONA_SEGS);
        pass.set_pipeline(&self.bolts);
        pass.draw(0..6, 0..MAX_BOLTS as u32 * SEGS);
        pass.set_pipeline(&self.sprites);
        self.parts.draw(pass, 0..6);
    }

    fn wants_msaa(&self) -> bool {
        false // analytic lines, soft sprites and an analytically anti-aliased grid
    }

    fn camera(&self, aspect: f32, p: &SceneParams) -> (Mat4, Vec3) {
        // low and a little above level: the clouds are above the frame and two thirds of it is sky
        let o = Orbit { eye: Vec3::new(0.0, 3.0, 21.0), target: Vec3::new(0.0, 4.8, 0.0), fov_deg: 42.0 };
        orbit(&o, aspect, &p.cam)
    }

    fn shape(&self, aspect: f32) -> [f32; 3] {
        [Self::width(aspect), 0.0, 0.0]
    }

    fn reset(&mut self, seed: u64) {
        self.parts.reset(seed);
        self.storm.reset(seed);
        self.hits.reset();
        self.last_frame = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f32 = 40.0;

    fn input<'a>(now: f32, gate: f32, hits: &'a [Hit]) -> Input<'a> {
        Input { now, gate, hits, width: W, density: 1.0 }
    }

    #[test]
    fn a_hit_strikes_where_it_landed_in_the_same_frame() {
        let mut s = Storm::new(7);
        let out = s.step(&input(1.0, 1.0, &[Hit { pos: 0.25, strength: 0.9 }, Hit { pos: 0.8, strength: 0.4 }]));
        assert_eq!(out.len(), 2);
        // strongest first; a kick is on the left, a snare on the right
        assert!((out[0].line - 0.25).abs() < 1e-6 && (out[0].x - (0.25 - 0.5) * W).abs() < 1e-4);
        assert!((out[1].x - 0.3 * W).abs() < 1e-4);
        assert!(out.iter().all(|b| b.t0 == 1.0 && b.z.abs() <= DEPTH && b.seed != 0 && !b.ghost));
        // and it is in the table at age 0
        let u = s.pack(1.0);
        assert_eq!(u.bolts[0][2], 0.0);
        assert!((u.bolts[0][3] - 0.9).abs() < 1e-6);
        assert!(u.bolts[4 * 2][3] == 0.0, "unused slots have no strength");
    }

    #[test]
    fn position_sets_the_character() {
        // lows are thick with few branches, a thunder ring and a tall channel; highs thin, many-branched, short
        let (lo, hi) = (0.05, 0.95);
        assert!(sigma_for(lo, 0.8) > 1.5 * sigma_for(hi, 0.8));
        assert!(branches_for(hi, 0.8, 1.0) >= 2 * branches_for(lo, 0.8, 1.0));
        assert!(ring_for(lo, 0.8) > 0.5 && ring_for(hi, 0.8) == 0.0 && ring_for(0.5, 0.8) == 0.0);
        assert!(top_for(lo) > top_for(hi) + 5.0);
        // a harder hit is thicker, branches more, flickers more and splashes more
        assert!(sigma_for(0.5, 1.0) > sigma_for(0.5, 0.2));
        assert!(branches_for(0.5, 1.0, 1.0) > branches_for(0.5, 0.2, 1.0));
        assert!(strokes_for(0.3) == 1 && strokes_for(0.7) == 2 && strokes_for(1.0) == 3);
        assert!(sparks_for(1.0, 1.0) > 3 * sparks_for(0.0, 1.0));
        // Density scales branches within bounds
        assert!(branches_for(0.9, 1.0, 1.5) <= MAX_BRANCHES && branches_for(0.0, 0.0, 0.25) >= 1);
    }

    #[test]
    fn no_part_of_the_line_flashes_more_than_three_times_a_second() {
        let mut s = Storm::new(3);
        let mut full = Vec::new();
        let mut ghosts = 0;
        // a hit every 60 ms for four seconds, all in the same place
        for k in 0..66 {
            let now = k as f32 * 0.06;
            for b in s.step(&input(now, 1.0, &[Hit { pos: 0.4, strength: 1.0 }])) {
                if b.ghost {
                    ghosts += 1;
                } else {
                    full.push(now);
                }
            }
        }
        assert!(ghosts > 0, "a flood of hits is answered with ghosts");
        for &t in &full {
            let in_second = full.iter().filter(|&&u| u > t - 1.0 && u <= t).count();
            assert!(in_second <= FLASHES_PER_S, "{in_second} full flashes in the second to {t}");
        }
        // a quiet part of the line is not held back by it
        let b = s.step(&input(4.0, 1.0, &[Hit { pos: 0.95, strength: 0.7 }]));
        assert!(!b[0].ghost);
        // and the budget comes back
        let b = s.step(&input(6.0, 1.0, &[Hit { pos: 0.4, strength: 0.7 }]));
        assert!(!b[0].ghost);
    }

    #[test]
    fn a_ghost_is_a_single_dim_stroke_with_no_floor_light() {
        let mut s = Storm::new(5);
        for k in 0..FLASHES_PER_S {
            s.step(&input(k as f32 * 0.1, 1.0, &[Hit { pos: 0.5, strength: 1.0 }]));
        }
        let g = s.step(&input(0.4, 1.0, &[Hit { pos: 0.5, strength: 1.0 }]))[0];
        assert!(g.ghost && g.strokes == 1);
        let u = s.pack(0.4);
        let k = s.bolts.len() - 1;
        assert_eq!(u.bolts[4 * k + 2][1], 1.0, "the shader skips the pool and ring for a ghost");
    }

    #[test]
    fn strikes_expire_the_table_is_bounded_and_silence_strikes_nothing() {
        let mut s = Storm::new(1);
        for k in 0..30 {
            s.step(&input(k as f32 * 0.02, 1.0, &[Hit { pos: (k % 7) as f32 / 6.0, strength: 0.6 }]));
            assert!(s.bolts.len() <= MAX_BOLTS);
        }
        assert_eq!(s.bolts.len(), MAX_BOLTS);
        s.step(&input(10.0, 1.0, &[]));
        assert!(s.bolts.is_empty(), "strikes leave the table with their pool and ring");
        assert!(s.step(&input(11.0, 0.0, &[Hit { pos: 0.5, strength: 1.0 }])).is_empty(), "a closed gate strikes nothing");
    }

    #[test]
    fn the_same_seed_repeats() {
        let run = |seed| {
            let mut s = Storm::new(seed);
            (0..200)
                .flat_map(|k| s.step(&input(k as f32 / 60.0, 1.0, &if k % 17 == 0 { vec![Hit { pos: (k % 5) as f32 / 4.0, strength: 0.8 }] } else { vec![] })))
                .map(|b| (b.x.to_bits(), b.z.to_bits(), b.seed, b.branches))
                .collect::<Vec<_>>()
        };
        assert!(!run(5).is_empty());
        assert_eq!(run(5), run(5));
        assert_ne!(run(5), run(6));
    }

    #[test]
    fn the_gpu_table_matches_the_shader() {
        // 8 strikes of four vec4 each, and the segment counts the shader draws
        assert_eq!(std::mem::size_of::<LtU>(), 4 * MAX_BOLTS * 16);
        assert_eq!(SEGS, 160);
        assert_eq!((MAIN_SEGS, BRANCH_SEGS, MAX_BRANCHES, CORONAS, CORONA_SEGS), (64, 16, 6, 24, 8));
        let mut s = Storm::new(1);
        s.step(&input(2.0, 1.0, &[Hit { pos: 0.1, strength: 1.0 }]));
        let u = s.pack(2.25);
        assert!(u.bolts[2].iter().all(|v| v.is_finite()) && (u.bolts[0][2] - 0.25).abs() < 1e-6);
        assert_eq!(u.bolts[1][2], 3.0, "a full-strength hit flickers three times");
        assert!(u.bolts[1][1] >= 1.0 && u.bolts[1][1] < 16_777_216.0, "the seed survives the trip through an f32");
        assert!(u.bolts[1][3] > 20.0, "a low strike starts above the frame");
    }
}
