use super::*;
use super::particles::{Particles, SpawnPlan};
use crate::analyzer::CURVE_POINTS;
use crate::modulation::{DrumHits, Hit, RiseHits, SPARK_START};
use std::collections::VecDeque;

/// Comet trails trace the live spectrum along the glass. Each detected drum opens a shell above its
/// frequency in the same frame. Other sharp attacks open smaller shells. The particle simulation
/// carries their stars.
pub struct Fireworks {
    floor: wgpu::RenderPipeline,
    refl: wgpu::RenderPipeline,
    sprites: wgpu::RenderPipeline,
    comets: wgpu::RenderPipeline,
    comets_refl: wgpu::RenderPipeline,
    parts: Particles,
    show: Show,
    hits: FireworkHits,
    fw_buf: wgpu::Buffer,
    fw_bg: wgpu::BindGroup,
    last_frame: u64,
}

const COUNT: u32 = 65_536;
/// Longest a star can live (see `fireworks_sim.wgsl`), seconds.
const MAX_LIFE: f32 = 2.0;
const COMETS: usize = 24;
const MAX_POOLS: usize = 8;

const THIN_PER_BURST: f32 = 0.22;
const STARS_MIN: f32 = 65.0;
const STARS_MAX: f32 = 800.0;
/// A clear attack in the spectrum corroborates a less certain drum classification.
const ATTACK_SENSITIVITY: f32 = 1.6;

/// The kind of burst a hit makes, by its place on the line. Keep in step with `kind_of` in `fireworks_sim.wgsl`.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Willow,
    Peony,
    Crackle,
}

fn kind_of(line: f32) -> Kind {
    if line < 0.35 {
        Kind::Willow
    } else if line < 0.75 {
        Kind::Peony
    } else {
        Kind::Crackle
    }
}

/// How far a burst's stars travel in all, in world units, from its strength (0..1): a willow is the biggest, a
/// crackle small.
fn radius_for(line: f32, strength: f32) -> f32 {
    let base = 1.3 + 4.5 * strength.clamp(0.0, 1.0);
    base * match kind_of(line) {
        Kind::Willow => 1.12,
        Kind::Peony => 1.0,
        Kind::Crackle => 0.62,
    }
}

/// How many stars a burst has: more for a harder hit, fewer for a crackle, scaled by Density, thinned as more
/// bursts are alive.
fn stars_for(line: f32, strength: f32, density: f32, alive: usize) -> u32 {
    let base = STARS_MIN + (STARS_MAX - STARS_MIN) * strength.clamp(0.0, 1.0).powf(1.2);
    let kind = if kind_of(line) == Kind::Crackle { 0.6 } else { 1.0 };
    let thin = 1.0 / (1.0 + THIN_PER_BURST * alive as f32);
    ((base * kind * density.clamp(0.1, 2.0) * thin).round() as u32).max(24)
}

/// Bursts are scaled down on a narrower picture so a big willow does not take most of a 16:9 frame; at 32:9, where
/// the line is at its widest, they are full size. (The line's width is `Fireworks::width`: 14 to 40.)
fn radius_scale(width: f32) -> f32 {
    (0.7 + 0.3 * (width - 14.0) / 26.0).clamp(0.7, 1.0)
}

pub(super) fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// xorshift32: the choreography's randomness, seeded by `reset` so captures repeat.
pub(super) struct Rng(u32);

impl Rng {
    pub(super) fn seeded(seed: u64) -> Rng {
        Rng((seed as u32).wrapping_mul(2654435761).wrapping_add(0x9e3779b9) | 1)
    }

    pub(super) fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / 16_777_216.0
    }
}

/// A drum and a spectral attack can describe the same note. The rise picker already waits for
/// the peak. Give the drum classifier one more analysis hop to identify it before using
/// that peak for a smaller accent. Confirmed drums always go straight through.
struct FireworkHits {
    drums: DrumHits,
    rises: RiseHits,
    pending: Option<(f64, Hit)>,
    last_burst: f64,
}

impl FireworkHits {
    fn new() -> Self {
        Self { drums: DrumHits::new(), rises: RiseHits::new(), pending: None, last_burst: f64::NEG_INFINITY }
    }

    fn update(&mut self, p: &SceneParams) -> Vec<Hit> {
        let now = p.time as f64;
        let sensitivity = p.gradient.clamp(0.2, 5.0);
        let attack = p.rise.iter().any(|&r| r >= SPARK_START);
        let open = p.gate > 0.12 || attack;
        let drum_sensitivity = if attack { sensitivity * ATTACK_SENSITIVITY } else { sensitivity };
        let drums = self.drums.update(p.drums(), drum_sensitivity, now, open);
        let rises = self.rises.update(&p.rise, sensitivity, now, open);
        if !drums.is_empty() {
            self.last_burst = now;
            self.pending = None;
            return drums;
        }
        if !open || now - self.last_burst < 0.09 {
            self.pending = None;
            return Vec::new();
        }
        if let Some(h) = rises.into_iter().max_by(|a, b| a.strength.total_cmp(&b.strength)) {
            let (_, best) = self.pending.get_or_insert((now, h));
            if h.strength > best.strength {
                *best = h;
            }
        }
        if let Some((start, hit)) = self.pending {
            if now - start >= 0.01 {
                self.pending = None;
                self.last_burst = now;
                return vec![Hit { strength: 0.65 * hit.strength, ..hit }];
            }
        }
        Vec::new()
    }
}

/// Each comet covers an equal stretch of the log spectrum. The displayed level holds a note up,
/// and the fast spectral rise gives its attack a taller, brighter head. Both signals already
/// have a release envelope.
fn spectrum_comets(p: &SceneParams) -> [[f32; 4]; COMETS] {
    std::array::from_fn(|band| {
        let start = band * CURVE_POINTS / COMETS;
        let end = (band + 1) * CURVE_POINTS / COMETS;
        let peak = |bins: &[f32]| bins.iter().copied().fold(0.0, f32::max);
        let rise = ((peak(&p.rise[start..end]) - 0.12) / 0.88).clamp(0.0, 1.0);
        // The gate opens gradually. Let a clear attack from silence show immediately.
        let level = peak(&p.spectrum[start..end]).clamp(0.0, 1.0) * p.gate.max(rise);
        let line = (band as f32 + 0.5) / COMETS as f32;
        let height = 3.0 * level * level + 2.5 * rise;
        let trail = 0.24 * height + 1.6 * rise;
        [line, height, trail, 0.7 * level * level + 1.8 * rise]
    })
}

struct Pool {
    t0: f32,
    centre: Vec3,
    amp: f32,
    line: f32,
    radius: f32,
    life: f32,
}

struct Burst {
    centre: Vec3,
    line: f32,
    strength: f32,
    seed: f32,
    stars: u32,
    radius: f32,
}

struct Show {
    pools: VecDeque<Pool>,
    rng: Rng,
}

impl Show {
    fn new(seed: u64) -> Show {
        Show { pools: VecDeque::with_capacity(MAX_POOLS), rng: Rng::seeded(seed) }
    }

    fn step(&mut self, p: &SceneParams, hits: &[Hit], width: f32) -> (FwU, Vec<Burst>) {
        let mut table = FwU { comets: spectrum_comets(p), pools: [[0.0; 4]; 2 * MAX_POOLS] };
        self.pools.retain(|pool| p.time - pool.t0 < pool.life);

        // A slow display frame can collect several taps of one drum. Show its strongest tap now,
        // leaving space for simultaneous drums in the other registers (the GPU has four centres).
        let mut accents: [Option<Hit>; 3] = [None; 3];
        for &h in hits {
            let slot = &mut accents[kind_of(h.pos) as usize];
            if slot.is_none_or(|prev| h.strength > prev.strength) {
                *slot = Some(h);
            }
        }
        let mut bursts = Vec::new();
        let crowd = self.pools.len();
        for h in accents.into_iter().flatten() {
            let band = ((h.pos * COMETS as f32) as usize).min(COMETS - 1);
            let height = 2.1 + 4.8 * h.strength + 0.35 * table.comets[band][1];
            let centre = Vec3::new((h.pos - 0.5) * width, height, lerp(-2.0, 2.0, self.rng.next()));
            let radius = radius_for(h.pos, h.strength) * radius_scale(width);
            let stars = stars_for(h.pos, h.strength, p.density, crowd);
            bursts.push(Burst { centre, line: h.pos, strength: h.strength, seed: self.rng.next(), stars, radius });
            if self.pools.len() == MAX_POOLS {
                self.pools.pop_front();
            }
            self.pools.push_back(Pool {
                t0: p.time, centre, amp: 0.15 + 0.85 * h.strength,
                line: h.pos, radius, life: 0.5 + 0.9 * h.strength,
            });
        }
        for (k, pool) in self.pools.iter().enumerate() {
            table.pools[2 * k] = [pool.centre.x, pool.centre.z, p.time - pool.t0, pool.amp];
            table.pools[2 * k + 1] = [pool.line, pool.radius, pool.life, 0.0];
        }
        (table, bursts)
    }
}

/// Mirrors `FwU` in `fireworks.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FwU {
    comets: [[f32; 4]; COMETS],
    pools: [[f32; 4]; 2 * MAX_POOLS],
}

impl Fireworks {
    pub fn new(ctx: &Ctx) -> Fireworks {
        let dev = ctx.device;
        let parts = Particles::new(ctx, "fireworks", include_str!("fireworks_sim.wgsl"), "cs_main", COUNT, MAX_LIFE);
        let src = format!("{}\n{}\n{}", include_str!("particles_common.wgsl"), include_str!("particles_draw.wgsl"), include_str!("fireworks.wgsl"));
        let sh = ctx.module("fireworks", &src);

        // the comets' and the floor's table, in group 1 (the stars use group 1 for the particles instead)
        let bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fireworks_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let layout = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(ctx.bgl), Some(&bgl)], immediate_size: 0 });
        let fw_buf = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fireworks_u"),
            size: std::mem::size_of::<FwU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let fw_bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fireworks_bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry { binding: 2, resource: fw_buf.as_entire_binding() }],
        });

        let floor = Pipe { depth_write: false, always: false, blend: None, msaa: false };
        let glow = Pipe { depth_write: false, always: false, blend: ADDITIVE, msaa: false };
        Fireworks {
            floor: ctx.pipeline_with(&layout, &sh, "fireworks_floor", "vs_floor", "fs_floor", floor),
            comets: ctx.pipeline_with(&layout, &sh, "fireworks_comets", "vs_comet", "fs_sprite", glow),
            comets_refl: ctx.pipeline_with(&layout, &sh, "fireworks_comets_refl", "vs_comet_refl", "fs_sprite", glow),
            refl: ctx.pipeline_with(&parts.draw_layout, &sh, "fireworks_refl", "vs_star_refl", "fs_sprite", glow),
            sprites: ctx.pipeline_with(&parts.draw_layout, &sh, "fireworks_stars", "vs_star", "fs_sprite", glow),
            parts,
            show: Show::new(1),
            hits: FireworkHits::new(),
            fw_buf,
            fw_bg,
            last_frame: 0,
        }
    }

    /// The pad line's width: it fills most of the picture at any aspect, within sensible bounds. The camera sees
    /// about 16.7 units of width per unit of aspect at the line; at 32:9 the line is held to 40, because a
    /// rectilinear view stretches what stands near the edges (a burst at 50 degrees off the axis is 1.5x too
    /// wide) and the shells are meant to stay round.
    fn width(aspect: f32) -> f32 {
        (12.5 * aspect).clamp(14.0, 40.0)
    }
}

impl Scene for Fireworks {
    fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, f: &Frame) {
        if std::env::var_os("GEQ_PARTDEBUG").is_some() && f.frame_no % 30 == 0 {
            self.parts.debug_report(device, queue, "fireworks");
        }
        let gap = f.frame_no.saturating_sub(self.last_frame);
        let Some(dt) = self.parts.begin(f) else { return };
        if self.last_frame == 0 || gap > 20 {
            // the particles start again from an empty sky after a gap, and so does the show
            self.show.pools.clear();
            self.hits = FireworkHits::new();
        }
        self.last_frame = f.frame_no;

        let p = f.p;
        let hits = self.hits.update(p);
        let (table, bursts) = self.show.step(p, &hits, Self::width(f.aspect));
        if std::env::var_os("GEQ_FWDEBUG").is_some() {
            for h in &hits {
                eprintln!("[fireworks] t={:.3} hit at {:.2} strength {:.2}", p.time, h.pos, h.strength);
            }
            for b in &bursts {
                eprintln!("[fireworks] t={:.3} burst {:?} line {:.2} strength {:.2} stars {} at ({:.1}, {:.1}, {:.1})",
                    p.time, kind_of(b.line), b.line, b.strength, b.stars, b.centre.x, b.centre.y, b.centre.z);
            }
        }
        let mut plan = SpawnPlan::default();
        let mut extra = [[0.0f32; 4]; 4];
        for (k, b) in bursts.iter().enumerate() {
            // the first of a burst's particles is its detonation flash, the rest are stars
            plan.bursts.push([b.line, b.strength, b.seed, (b.stars + 1) as f32]);
            extra[k] = [b.centre.x, b.centre.y, b.centre.z, b.radius];
        }
        self.parts.advance(queue, enc, f, dt, &plan, extra);
        queue.write_buffer(&self.fw_buf, 0, bytemuck::bytes_of(&table));
    }

    fn uses_update(&self, _p: &SceneParams) -> bool {
        true
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, _f: &Frame) {
        pass.set_pipeline(&self.floor);
        pass.set_bind_group(1, &self.fw_bg, &[]);
        pass.draw(0..6, 0..1);
        pass.set_pipeline(&self.comets_refl);
        pass.draw(0..6, 0..COMETS as u32);
        pass.set_pipeline(&self.comets);
        pass.draw(0..6, 0..COMETS as u32);
        pass.set_pipeline(&self.refl);
        self.parts.draw(pass, 0..6);
        pass.set_pipeline(&self.sprites);
        self.parts.draw(pass, 0..6);
    }

    fn wants_msaa(&self) -> bool {
        false // soft sprites and an analytically anti-aliased grid
    }

    fn camera(&self, aspect: f32, p: &SceneParams) -> (Mat4, Vec3) {
        // low and a little above level: two thirds of the frame is sky
        let o = Orbit { eye: Vec3::new(0.0, 2.0, 23.0), target: Vec3::new(0.0, 5.6, 0.0), fov_deg: 40.0 };
        orbit(&o, aspect, &p.cam)
    }

    fn shape(&self, aspect: f32) -> [f32; 3] {
        [Self::width(aspect), 0.0, 0.0]
    }

    fn reset(&mut self, seed: u64) {
        self.parts.reset(seed);
        self.show = Show::new(seed);
        self.hits = FireworkHits::new();
        self.last_frame = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drum_takes_priority_over_its_spectral_attack() {
        use crate::drums::{DrumHit, Kind};
        for fps in [30, 60, 120] {
            let mut tracker = FireworkHits::new();
            let mut p = SceneParams::for_tests();
            let mut delivered = Vec::new();
            let drum_frame = (0.03 * fps as f32).ceil() as usize;
            for frame in 0..fps {
                p.time = frame as f32 / fps as f32;
                p.rise[30] = if p.time < 0.05 { 0.9 } else { 0.0 };
                p.n_drums = usize::from(frame == drum_frame);
                p.drums[0] = DrumHit { kind: Kind::Low, p: 0.96, strength: 1.0, pos: 0.1, t: 0.03 };
                for h in tracker.update(&p) {
                    delivered.push((frame, h));
                }
            }
            assert_eq!(delivered, [(drum_frame, Hit { pos: 0.1, strength: 1.0 })], "at {fps} fps");
        }
    }

    #[test]
    fn a_missed_note_attack_still_bursts_at_its_frequency() {
        for fps in [30, 60, 120] {
            let mut tracker = FireworkHits::new();
            let mut p = SceneParams::for_tests();
            let mut delivered = Vec::new();
            for frame in 0..fps {
                p.time = frame as f32 / fps as f32;
                p.rise[120] = if p.time < 0.04 { 0.85 } else { 0.0 };
                for h in tracker.update(&p) {
                    delivered.push((p.time, h));
                }
            }
            assert_eq!(delivered.len(), 1, "at {fps} fps");
            let (t, h) = delivered[0];
            assert!(t <= 0.08 && (h.pos - 120.0 / (CURVE_POINTS - 1) as f32).abs() < 0.01);
            assert!(h.strength > 0.0 && h.strength <= 0.65);
        }
    }

    #[test]
    fn notes_light_their_own_comets_and_never_detonate_on_a_timer() {
        let mut show = Show::new(1);
        let mut p = SceneParams::for_tests();
        for bin in [2, CURVE_POINTS / 2, CURVE_POINTS - 1] {
            p.spectrum.fill(0.0);
            p.spectrum[bin] = 0.65;
            let band = bin * COMETS / CURVE_POINTS;
            let steady = spectrum_comets(&p);
            assert!(steady[band][1] > 1.0);
            assert_eq!(steady.iter().filter(|c| c[3] > 0.0).count(), 1);

            p.rise[bin] = 0.8;
            let attack = spectrum_comets(&p);
            assert!(attack[band][1] > steady[band][1] && attack[band][3] > steady[band][3]);
            p.rise[bin] = 0.0;
            for frame in 0..180 {
                p.time += 1.0 / 60.0;
                let (table, bursts) = show.step(&p, &[], 24.0);
                assert_eq!(table.comets[band], steady[band]);
                assert!(bursts.is_empty(), "unprompted burst on frame {frame}");
            }
        }
        p.gate = 0.0;
        assert!(spectrum_comets(&p).iter().all(|c| c[1] == 0.0 && c[3] == 0.0));
    }

    #[test]
    fn a_fast_groove_keeps_its_accents_and_thins_the_stars() {
        let mut show = Show::new(1);
        let mut p = SceneParams::for_tests();
        let hits = [Hit { pos: 0.15, strength: 0.95 }, Hit { pos: 0.85, strength: 0.4 }];
        let mut first_count = 0;
        for beat in 0..32 {
            p.time = beat as f32 * 0.125;
            let (table, bursts) = show.step(&p, &hits, 24.0);
            assert_eq!(bursts.len(), 2, "beat {beat}");
            for (burst, hit) in bursts.iter().zip(hits) {
                assert_eq!(burst.centre.x, (hit.pos - 0.5) * 24.0);
                assert_eq!(burst.strength, hit.strength);
            }
            assert!(bursts[0].radius > bursts[1].radius && bursts[0].centre.y > bursts[1].centre.y);
            if beat == 0 { first_count = bursts[0].stars; }
            if beat > 4 { assert!(bursts[0].stars < first_count); }
            assert!(show.pools.len() <= MAX_POOLS);
            assert_eq!(table.pools[2 * (show.pools.len() - 1)][2], 0.0);
        }
        p.time += 2.0;
        let (table, bursts) = show.step(&p, &[], 24.0);
        assert!(bursts.is_empty() && table.pools.iter().all(|v| *v == [0.0; 4]));
    }

    #[test]
    fn a_batched_frame_keeps_each_registers_strongest_hit() {
        let hits = [Hit { pos: 0.1, strength: 0.4 }, Hit { pos: 0.34, strength: 0.7 },
            Hit { pos: 0.74, strength: 0.8 }, Hit { pos: 0.76, strength: 0.6 }];
        let p = SceneParams::for_tests();
        let (_, bursts) = Show::new(7).step(&p, &hits, 24.0);
        assert_eq!(bursts.iter().map(|b| b.line).collect::<Vec<_>>(), [0.34, 0.74, 0.76]);
        let (_, repeat) = Show::new(7).step(&p, &hits, 24.0);
        assert!(repeat.iter().zip(bursts).all(|(a, b)| a.centre == b.centre && a.seed == b.seed));
        assert!(stars_for(0.5, 0.8, 1.0, 2) > stars_for(0.5, 0.8, 0.5, 2));
        assert!(radius_scale(Fireworks::width(16.0 / 9.0)) < radius_scale(Fireworks::width(32.0 / 9.0)));
    }
}
