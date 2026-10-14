use super::*;
use super::particles::{Particles, SpawnPlan};
use crate::modulation::{Hit, RiseHits};
use std::collections::VecDeque;

/// Shells climb the dark sky while the music is loud, and a hit detonates one. Sustained energy launches
/// rockets from pads along a line (bass left, treble right, scattered in depth); a hit found in the rise
/// (`modulation::RiseHits`, the fountain's rise judged against the music's own hardest hits) sets off the
/// rocket nearest its place on the line, at once and at the height it has reached. The stars of the burst
/// carry the spectrum of the moment, hang for several seconds, droop and fall onto a glass floor that lights up
/// beneath them and mirrors them. Where the hit landed sets the kind: a willow in the lows, a peony in the
/// mids, a crackle in the highs.
///
/// The rockets are plain Rust, closed-form ballistic arcs known at any moment; the stars are the particle
/// system's (a compute pass, see `particles.rs`).
pub struct Fireworks {
    floor: wgpu::RenderPipeline,
    refl: wgpu::RenderPipeline,
    sprites: wgpu::RenderPipeline,
    shells: wgpu::RenderPipeline,
    shells_refl: wgpu::RenderPipeline,
    parts: Particles,
    show: Show,
    hits: RiseHits,
    fw_buf: wgpu::Buffer,
    fw_bg: wgpu::BindGroup,
    last_frame: u64,
}

const COUNT: u32 = 65_536;
/// Longest a star can live (see `fireworks_sim.wgsl`), seconds.
const MAX_LIFE: f32 = 4.3;

const MAX_SHELLS: usize = 16;
const MAX_POOLS: usize = 8;
/// `PartU.extra` holds one centre per burst, so a frame sets off at most this many.
const MAX_BURSTS_PER_FRAME: usize = 4;

/// Gravity on a rocket (units/s²) and the time it takes to climb to its apex, which sets its height.
const SHELL_G: f32 = 9.0;
const APEX_S: (f32, f32) = (1.1, 1.5);
/// Pads scatter this far either side of the line in depth, so simultaneous bursts overlap with parallax.
const DEPTH: f32 = 6.0;
/// Below this gate level the music has stopped: nothing launches and the rockets in the air fizzle out.
const FIZZLE_GATE: f32 = 0.12;
const FIZZLE_S: f32 = 0.35;
/// Rockets leave a stretch of the line (lows, mids, highs) at this rate in silence-plus and per unit of the
/// stretch's sustained level, so the total runs from about 0.4 to 3 a second; at most three in the air per stretch.
const LAUNCH_BASE: f32 = 0.12;
const LAUNCH_GAIN: f32 = 0.95;
const IN_FLIGHT_MAX: usize = 3;
/// A rocket must have climbed this long and this high to be set off by a hit; otherwise a short-fuse mortar pops.
const MIN_FLIGHT_S: f32 = 0.35;
const MIN_HEIGHT: f32 = 3.5;
/// How far (as a share of the line's width) the nearest rocket may be from a hit's place on the line.
const HIT_REACH: f32 = 0.28;
const MORTAR_HEIGHT: (f32, f32) = (3.8, 6.0);
/// A rocket nobody set off bursts at its apex, small and dim; it is a decoration, so it gives way in a busy sky.
const APEX_STRENGTH: (f32, f32) = (0.10, 0.18);
const APEX_ALIVE_MAX: usize = 4;

/// Bursts count as alive for this long, which is what the sky is limited by: no new burst is set off by a hit
/// beyond `MAX_ALIVE`, and a burst's stars thin out as more are alive.
const ALIVE_S: f32 = 2.6;
const MAX_ALIVE: usize = 7;
const THIN_PER_BURST: f32 = 0.20;
const STARS_MIN: f32 = 100.0;
const STARS_MAX: f32 = 900.0;

/// The kind of burst a hit makes, by its place on the line. Keep in step with `kind_of` in `fireworks_sim.wgsl`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Willow,
    Peony,
    Crackle,
}

pub fn kind_of(line: f32) -> Kind {
    if line < 0.30 {
        Kind::Willow
    } else if line < 0.66 {
        Kind::Peony
    } else {
        Kind::Crackle
    }
}

/// How far a burst's stars travel in all, in world units, from its strength (0..1): a willow is the biggest, a
/// crackle small. The camera is fixed, so this is also its size on screen as a share of the height.
pub fn radius_for(line: f32, strength: f32) -> f32 {
    let base = 2.4 + (6.0 - 2.4) * strength.clamp(0.0, 1.0).powf(0.85);
    base * match kind_of(line) {
        Kind::Willow => 1.12,
        Kind::Peony => 1.0,
        Kind::Crackle => 0.62,
    }
}

/// How many stars a burst has: more for a harder hit, fewer for a crackle, scaled by Density, thinned as more
/// bursts are alive.
pub fn stars_for(line: f32, strength: f32, density: f32, alive: usize) -> u32 {
    let base = STARS_MIN + (STARS_MAX - STARS_MIN) * strength.clamp(0.0, 1.0).powf(1.2);
    let kind = if kind_of(line) == Kind::Crackle { 0.6 } else { 1.0 };
    let thin = 1.0 / (1.0 + THIN_PER_BURST * alive as f32);
    ((base * kind * density.clamp(0.1, 2.0) * thin).round() as u32).max(24)
}

/// Bursts are scaled down on a narrower picture so a big willow does not take most of a 16:9 frame; at 32:9, where
/// the line is at its widest, they are full size. (The line's width is `Fireworks::width`: 14 to 40.)
pub fn radius_scale(width: f32) -> f32 {
    (0.7 + 0.3 * (width - 14.0) / 26.0).clamp(0.7, 1.0)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// xorshift32: the choreography's randomness, seeded by `reset` so captures repeat.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / 16_777_216.0
    }
}

/// One rocket: a ballistic arc from its pad.
#[derive(Clone, Copy, Debug)]
struct Shell {
    t0: f32,
    p0: Vec3,
    v0: Vec3,
    /// Its pad's place on the line (0 bass .. 1 treble), which tints the streak.
    line: f32,
    /// When the music stopped under it: it fades out in the air.
    fizzle: Option<f32>,
}

impl Shell {
    fn apex_s(&self) -> f32 {
        self.v0.y / SHELL_G
    }

    fn pos(&self, now: f32) -> Vec3 {
        let t = now - self.t0;
        self.p0 + self.v0 * t - Vec3::Y * (0.5 * SHELL_G * t * t)
    }

    fn vel(&self, now: f32) -> Vec3 {
        self.v0 - Vec3::Y * (SHELL_G * (now - self.t0))
    }
}

/// The light a burst throws on the floor.
#[derive(Clone, Copy, Debug)]
struct Pool {
    x: f32,
    z: f32,
    t0: f32,
    amp: f32,
    line: f32,
    radius: f32,
    life: f32,
}

/// Why a burst was set off.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Origin {
    /// A hit set off a rocket that was in the air.
    Detonated,
    /// A hit with no rocket near: a short-fuse mortar popped low.
    Mortar,
    /// A rocket nobody set off, bursting at its apex.
    Apex,
}

/// A burst to set off: what the compute pass needs.
#[derive(Clone, Copy, Debug)]
pub struct Burst {
    pub centre: Vec3,
    pub line: f32,
    pub strength: f32,
    pub seed: f32,
    pub stars: u32,
    pub radius: f32,
    pub origin: Origin,
}

pub struct Input<'a> {
    pub now: f32,
    pub dt: f32,
    pub gate: f32,
    /// Sustained low / mid / high (normalized, gated).
    pub sus: [f32; 3],
    pub hits: &'a [Hit],
    /// Width of the line the pads stand along.
    pub width: f32,
    pub density: f32,
}

/// The choreography: launches, detonations, apex bursts, the floor's pools of light. No GPU in here.
pub struct Show {
    shells: Vec<Shell>,
    pools: Vec<Pool>,
    carry: [f32; 3],
    rng: Rng,
    /// When the bursts still counted as alive were set off.
    recent: Vec<f32>,
    queue: VecDeque<Burst>,
    /// Rockets launched so far (for tests and diagnostics).
    launched: u32,
}

impl Show {
    pub fn new(seed: u64) -> Show {
        Show {
            shells: Vec::new(),
            pools: Vec::new(),
            carry: [0.0; 3],
            rng: Rng((seed as u32).wrapping_mul(2654435761).wrapping_add(0x9e3779b9) | 1),
            recent: Vec::new(),
            queue: VecDeque::new(),
            launched: 0,
        }
    }

    /// Empty sky, and the random sequence starts again.
    pub fn reset(&mut self, seed: u64) {
        *self = Show::new(seed);
    }

    /// Empty sky, the random sequence carries on.
    pub fn clear(&mut self) {
        self.shells.clear();
        self.pools.clear();
        self.carry = [0.0; 3];
        self.recent.clear();
        self.queue.clear();
    }

    fn launch(&mut self, stretch: usize, now: f32, width: f32) {
        let flying = self.shells.iter().filter(|s| s.fizzle.is_none() && ((s.line * 3.0) as usize).min(2) == stretch).count();
        if self.shells.len() >= MAX_SHELLS || flying >= IN_FLIGHT_MAX {
            return;
        }
        let line = (stretch as f32 + self.rng.next()) / 3.0;
        let x = (line - 0.5) * width;
        let z = (self.rng.next() - 0.5) * 2.0 * DEPTH;
        let apex = lerp(APEX_S.0, APEX_S.1, self.rng.next());
        let v0 = Vec3::new((self.rng.next() - 0.5) * 1.6, SHELL_G * apex, (self.rng.next() - 0.5) * 0.8);
        self.shells.push(Shell { t0: now, p0: Vec3::new(x, 0.1, z), v0, line, fizzle: None });
        self.launched += 1;
    }

    /// The rocket in the air nearest a hit's place on the line that has climbed far enough to be worth setting off.
    fn nearest_shell(&self, now: f32, x: f32, width: f32) -> Option<usize> {
        self.shells
            .iter()
            .enumerate()
            .filter(|(_, s)| s.fizzle.is_none() && now - s.t0 >= MIN_FLIGHT_S && now - s.t0 < s.apex_s() && s.pos(now).y >= MIN_HEIGHT)
            .map(|(i, s)| (i, (s.pos(now).x - x).abs()))
            .filter(|(_, d)| *d <= HIT_REACH * width)
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .map(|(i, _)| i)
    }

    /// Queue a burst. An apex burst's floor light is dimmer.
    fn burst(&mut self, centre: Vec3, line: f32, strength: f32, origin: Origin, now: f32, density: f32, width: f32) {
        let apex = origin == Origin::Apex;
        let alive = self.recent.len();
        self.recent.push(now);
        let radius = radius_for(line, strength) * radius_scale(width);
        let stars = stars_for(line, strength, density, alive);
        let seed = self.rng.next();
        if self.pools.len() >= MAX_POOLS {
            self.pools.remove(0);
        }
        self.pools.push(Pool {
            x: centre.x,
            z: centre.z,
            t0: now,
            amp: (0.3 + 0.7 * strength) * if apex { 0.5 } else { 1.0 },
            line,
            radius,
            life: 2.2 + 1.8 * strength,
        });
        self.queue.push_back(Burst { centre, line, strength, seed, stars, radius, origin });
    }

    /// Advance to `i.now` and return the bursts to set off this frame (at most `MAX_BURSTS_PER_FRAME`; any more
    /// wait for the next frame).
    pub fn step(&mut self, i: &Input) -> Vec<Burst> {
        let now = i.now;
        self.pools.retain(|p| now - p.t0 < p.life);
        self.shells.retain(|s| s.fizzle.map_or(true, |t| now - t < FIZZLE_S));
        self.recent.retain(|t| now - t < ALIVE_S);

        let open = i.gate >= FIZZLE_GATE;
        if open {
            for stretch in 0..3 {
                let rate = LAUNCH_BASE + LAUNCH_GAIN * i.sus[stretch].clamp(0.0, 1.2);
                self.carry[stretch] += rate * i.gate.clamp(0.0, 1.0) * i.dt;
                while self.carry[stretch] >= 1.0 {
                    self.carry[stretch] -= 1.0;
                    self.launch(stretch, now, i.width);
                }
            }
        } else {
            self.carry = [0.0; 3];
            for s in &mut self.shells {
                s.fizzle.get_or_insert(now);
            }
        }

        // a hit sets off the rocket nearest its place, where it is now; with none near, a mortar pops low
        let mut hits: Vec<Hit> = i.hits.to_vec();
        hits.sort_by(|a, b| b.strength.partial_cmp(&a.strength).unwrap());
        for h in hits {
            if self.recent.len() >= MAX_ALIVE {
                break;
            }
            let x = (h.pos - 0.5) * i.width;
            let (centre, origin) = match self.nearest_shell(now, x, i.width) {
                Some(k) => (self.shells.remove(k).pos(now), Origin::Detonated),
                None => {
                    let y = lerp(MORTAR_HEIGHT.0, MORTAR_HEIGHT.1, self.rng.next());
                    (Vec3::new(x + (self.rng.next() - 0.5) * 1.2, y, (self.rng.next() - 0.5) * 2.0 * DEPTH), Origin::Mortar)
                }
            };
            self.burst(centre, h.pos, h.strength, origin, now, i.density, i.width);
        }

        // a rocket nobody set off bursts at its apex, small and dim; in a busy sky it fades out there instead
        let mut k = 0;
        while k < self.shells.len() {
            let s = self.shells[k];
            if s.fizzle.is_none() && now - s.t0 >= s.apex_s() {
                if self.recent.len() < APEX_ALIVE_MAX {
                    self.shells.remove(k);
                    let st = lerp(APEX_STRENGTH.0, APEX_STRENGTH.1, self.rng.next());
                    self.burst(s.pos(now), s.line, st, Origin::Apex, now, i.density, i.width);
                    continue;
                }
                self.shells[k].fizzle = Some(now);
            }
            k += 1;
        }

        let n = self.queue.len().min(MAX_BURSTS_PER_FRAME);
        self.queue.drain(..n).collect()
    }

    /// The GPU's tables: the rockets as they are at `now`, and the floor's pools.
    fn pack(&self, now: f32) -> FwU {
        let mut u = FwU::zeroed();
        for (k, s) in self.shells.iter().take(MAX_SHELLS).enumerate() {
            let alpha = s.fizzle.map_or(1.0, |t| (1.0 - (now - t) / FIZZLE_S).clamp(0.0, 1.0));
            let (p, v) = (s.pos(now), s.vel(now));
            u.shells[2 * k] = [p.x, p.y, p.z, alpha];
            u.shells[2 * k + 1] = [v.x, v.y, v.z, s.line];
        }
        for (k, p) in self.pools.iter().take(MAX_POOLS).enumerate() {
            u.pools[2 * k] = [p.x, p.z, now - p.t0, p.amp];
            u.pools[2 * k + 1] = [p.line, p.radius, p.life, 0.0];
        }
        u
    }
}

/// Mirrors `FwU` in `fireworks.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FwU {
    shells: [[f32; 4]; 2 * MAX_SHELLS],
    pools: [[f32; 4]; 2 * MAX_POOLS],
}

impl Fireworks {
    pub fn new(ctx: &Ctx) -> Fireworks {
        let dev = ctx.device;
        let parts = Particles::new(ctx, "fireworks", include_str!("fireworks_sim.wgsl"), "cs_main", COUNT, MAX_LIFE);
        let src = format!("{}\n{}\n{}", include_str!("particles_common.wgsl"), include_str!("particles_draw.wgsl"), include_str!("fireworks.wgsl"));
        let sh = ctx.module("fireworks", &src);

        // the rockets' and the floor's table, in group 1 (the stars use group 1 for the particles instead)
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
            shells: ctx.pipeline_with(&layout, &sh, "fireworks_shells", "vs_shell", "fs_sprite", glow),
            shells_refl: ctx.pipeline_with(&layout, &sh, "fireworks_shells_refl", "vs_shell_refl", "fs_sprite", glow),
            refl: ctx.pipeline_with(&parts.draw_layout, &sh, "fireworks_refl", "vs_star_refl", "fs_sprite", glow),
            sprites: ctx.pipeline_with(&parts.draw_layout, &sh, "fireworks_stars", "vs_star", "fs_sprite", glow),
            parts,
            show: Show::new(1),
            hits: RiseHits::new(),
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
            self.show.clear();
            self.hits.reset();
        }
        self.last_frame = f.frame_no;

        let p = f.p;
        let hits = self.hits.update(&p.rise, p.gradient.clamp(0.2, 5.0), p.time as f64, p.gate > 0.5);
        let bursts = self.show.step(&Input {
            now: p.time,
            dt,
            gate: p.gate,
            sus: [p.sus[0], p.sus[1], p.sus[2]],
            hits: &hits,
            width: Self::width(f.aspect),
            density: p.density,
        });
        if std::env::var_os("GEQ_FWDEBUG").is_some() {
            // development aid: one line per burst, for checking timing against a fixture's exact event times
            for h in &hits {
                eprintln!("[fireworks] t={:.3} hit at {:.2} strength {:.2}", p.time, h.pos, h.strength);
            }
            for b in &bursts {
                eprintln!(
                    "[fireworks] t={:.3} burst {:?} line {:.2} strength {:.2} stars {} at ({:.1}, {:.1}, {:.1}); {} rockets in the air",
                    p.time, b.origin, b.line, b.strength, b.stars, b.centre.x, b.centre.y, b.centre.z, self.show.shells.len()
                );
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
        queue.write_buffer(&self.fw_buf, 0, bytemuck::bytes_of(&self.show.pack(p.time)));
    }

    fn uses_update(&self, _p: &SceneParams) -> bool {
        true
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, _f: &Frame) {
        pass.set_pipeline(&self.floor);
        pass.set_bind_group(1, &self.fw_bg, &[]);
        pass.draw(0..6, 0..1);
        pass.set_pipeline(&self.shells_refl);
        pass.draw(0..6, 0..MAX_SHELLS as u32);
        pass.set_pipeline(&self.shells);
        pass.draw(0..6, 0..MAX_SHELLS as u32);
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
        self.show.reset(seed);
        self.hits.reset();
        self.last_frame = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f32 = 40.0;

    fn input<'a>(now: f32, gate: f32, sus: f32, hits: &'a [Hit]) -> Input<'a> {
        Input { now, dt: 1.0 / 120.0, gate, sus: [sus; 3], hits, width: W, density: 1.0 }
    }

    /// Steps a show at 120 fps from t = 0 for `secs` with the given sustained level, returning every burst with
    /// the time it was set off.
    fn run(show: &mut Show, secs: f32, gate: f32, sus: f32) -> Vec<(f32, Burst)> {
        let mut out = Vec::new();
        for k in 0..(secs * 120.0) as usize {
            let now = k as f32 / 120.0;
            for b in show.step(&input(now, gate, sus, &[])) {
                out.push((now, b));
            }
        }
        out
    }

    #[test]
    fn a_rocket_flies_a_closed_form_arc_to_its_apex() {
        let s = Shell { t0: 2.0, p0: Vec3::new(1.0, 0.1, -3.0), v0: Vec3::new(0.5, SHELL_G * 1.3, 0.0), line: 0.5, fizzle: None };
        assert!((s.apex_s() - 1.3).abs() < 1e-5);
        let apex = s.pos(2.0 + 1.3);
        assert!((apex.y - (0.1 + 0.5 * SHELL_G * 1.3 * 1.3)).abs() < 1e-3, "{apex:?}");
        // velocity is the derivative of position, and is level at the apex
        let (a, b) = (s.pos(2.5), s.pos(2.5 + 1e-3));
        assert!(((b - a) / 1e-3 - s.vel(2.5)).length() < 0.02);
        assert!(s.vel(3.3).y.abs() < 1e-4);
        // the highest and lowest the table allows fit the frame (see `camera`)
        let (lo, hi) = (0.5 * SHELL_G * APEX_S.0 * APEX_S.0, 0.5 * SHELL_G * APEX_S.1 * APEX_S.1);
        assert!(lo > 4.0 && hi < 11.0, "apex heights {lo}..{hi}");
    }

    #[test]
    fn energy_launches_rockets_and_silence_stops_them() {
        // the rate of launches follows the sustained level, from about 0.4 to 3 a second, and nothing happens
        // while the gate is closed
        let (mut loud, mut soft, mut none) = (Show::new(1), Show::new(1), Show::new(1));
        let loud_bursts = run(&mut loud, 20.0, 1.0, 1.0).len();
        let soft_bursts = run(&mut soft, 20.0, 1.0, 0.1).len();
        assert_eq!(run(&mut none, 20.0, 0.0, 1.0).len(), 0);
        assert_eq!(none.launched, 0, "nothing launches while the gate is closed");
        let per_s = |s: &Show| s.launched as f32 / 20.0;
        assert!(per_s(&soft) > 0.25 && per_s(&soft) < 1.0, "soft: {} a second", per_s(&soft));
        assert!(per_s(&loud) > 2.0 && per_s(&loud) < 4.0, "loud: {} a second", per_s(&loud));
        // quiet music still gets delicate bursts; loud music with no hits is held to a few at a time
        assert!(soft_bursts > 0 && loud_bursts > soft_bursts, "{soft_bursts} vs {loud_bursts}");
        assert!(loud_bursts as f32 / 20.0 <= APEX_ALIVE_MAX as f32 / ALIVE_S + 0.3, "{loud_bursts} apex bursts in 20 s");
    }

    #[test]
    fn rockets_in_the_air_fizzle_when_the_music_stops_and_never_burst() {
        let mut show = Show::new(3);
        run(&mut show, 3.0, 1.0, 0.8);
        let flying = show.shells.len();
        assert!(flying > 0);
        // the music stops: no apex bursts, the rockets fade out within the fizzle time
        let mut bursts = 0;
        for k in 0..60 {
            bursts += show.step(&input(3.0 + k as f32 / 120.0, 0.0, 0.0, &[])).len();
        }
        assert_eq!(bursts, 0, "a fizzled rocket must not burst");
        assert!(show.shells.iter().all(|s| s.fizzle.is_some()));
        show.step(&input(3.0 + FIZZLE_S + 0.1, 0.0, 0.0, &[]));
        assert!(show.shells.is_empty());
    }

    #[test]
    fn a_hit_sets_off_the_nearest_rocket_at_its_current_height_in_the_same_frame() {
        let mut show = Show::new(7);
        show.shells.clear();
        let mk = |x: f32, t0: f32| Shell { t0, p0: Vec3::new(x, 0.1, 0.0), v0: Vec3::new(0.0, SHELL_G * 1.4, 0.0), line: 0.5, fizzle: None };
        show.shells.push(mk(-12.0, 0.0));
        show.shells.push(mk(2.0, 0.0));
        let now = 0.8;
        let hit = Hit { pos: 0.55, strength: 0.9 }; // x = 2.0
        let near = show.shells[1];
        let out = show.step(&input(now, 1.0, 0.0, &[hit]));
        assert_eq!(out.len(), 1, "{out:?}");
        let b = out[0];
        assert_eq!(b.origin, Origin::Detonated);
        assert!((b.centre - near.pos(now)).length() < 1e-4, "burst at {:?}, rocket was at {:?}", b.centre, near.pos(now));
        assert!((b.line - 0.55).abs() < 1e-6 && (b.strength - 0.9).abs() < 1e-6);
        assert_eq!(show.shells.len(), 1, "the rocket is used up; the other keeps climbing");
        assert!((show.shells[0].p0.x + 12.0).abs() < 1e-4);
    }

    #[test]
    fn a_hit_with_no_rocket_near_pops_a_mortar_low_so_it_is_never_unanswered() {
        let mut show = Show::new(9);
        // a rocket that has barely left the pad is not high enough, and one far away is not near
        show.shells.push(Shell { t0: 0.0, p0: Vec3::new(0.0, 0.1, 0.0), v0: Vec3::new(0.0, SHELL_G * 1.2, 0.0), line: 0.5, fizzle: None });
        let out = show.step(&input(0.1, 1.0, 0.0, &[Hit { pos: 0.5, strength: 0.5 }, Hit { pos: 0.95, strength: 0.7 }]));
        assert_eq!(out.len(), 2, "{out:?}");
        for b in &out {
            assert_eq!(b.origin, Origin::Mortar);
            assert!(b.centre.y >= MORTAR_HEIGHT.0 - 1e-3 && b.centre.y <= MORTAR_HEIGHT.1 + 1e-3, "mortar at {:?}", b.centre);
            assert!(b.centre.z.abs() <= DEPTH + 1e-3);
        }
        let right = out.iter().find(|b| b.line > 0.9).unwrap();
        assert!((right.centre.x - 0.45 * W).abs() < 1.0, "popped at {:?}", right.centre);
        assert_eq!(show.shells.len(), 1);
    }

    #[test]
    fn an_unclaimed_rocket_bursts_at_its_apex_small_and_dim() {
        let mut show = Show::new(11);
        show.shells.push(Shell { t0: 0.0, p0: Vec3::new(3.0, 0.1, 1.0), v0: Vec3::new(0.0, SHELL_G * 1.2, 0.0), line: 0.4, fizzle: None });
        assert!(show.step(&input(1.1, 1.0, 0.0, &[])).is_empty(), "not yet at its apex");
        let out = show.step(&input(1.21, 1.0, 0.0, &[]));
        assert_eq!(out.len(), 1);
        let b = out[0];
        assert_eq!(b.origin, Origin::Apex);
        assert!(b.strength <= APEX_STRENGTH.1 + 1e-6);
        assert!((b.centre.y - (0.1 + 0.5 * SHELL_G * 1.2 * 1.2)).abs() < 0.2, "{:?}", b.centre);
        assert!(b.stars < 400, "a quiet burst is delicate: {} stars", b.stars);
        assert!(show.shells.is_empty());
    }

    #[test]
    fn the_sky_is_never_full() {
        // a hit on every frame for ten seconds: bursts alive never go past the cap, and the stars thin out
        let mut show = Show::new(13);
        let mut peak_alive = 0;
        let mut total = 0;
        for k in 0..1200 {
            let now = k as f32 / 120.0;
            let hit = Hit { pos: (k % 97) as f32 / 96.0, strength: 1.0 };
            total += show.step(&input(now, 1.0, 1.0, &[hit])).len();
            peak_alive = peak_alive.max(show.recent.len());
            assert!(show.pools.len() <= MAX_POOLS);
            assert!(show.shells.len() <= MAX_SHELLS);
        }
        assert!(peak_alive <= MAX_ALIVE + 1, "{peak_alive} bursts alive");
        assert!(total as f32 / 10.0 <= MAX_ALIVE as f32 / ALIVE_S + 1.0, "{total} bursts in ten seconds");
        // the more are alive, the fewer stars each has; Density scales them
        let (a, b, c) = (stars_for(0.5, 1.0, 1.0, 0), stars_for(0.5, 1.0, 1.0, 6), stars_for(0.5, 1.0, 0.5, 0));
        assert!(a > 700 && b < a / 2 && c < a * 6 / 10, "{a} {b} {c}");
        // a harder hit is bigger and has more stars; a crackle is smaller than a willow
        assert!(radius_for(0.5, 1.0) > radius_for(0.5, 0.2) && stars_for(0.5, 1.0, 1.0, 0) > stars_for(0.5, 0.2, 1.0, 0));
        assert!(radius_for(0.1, 0.8) > radius_for(0.5, 0.8) && radius_for(0.5, 0.8) > radius_for(0.9, 0.8));
        // a narrower picture gets smaller bursts, the widest full size
        assert!(radius_scale(Fireworks::width(16.0 / 9.0)) < 0.85 && (radius_scale(Fireworks::width(32.0 / 9.0)) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn position_sets_the_kind_and_the_same_seed_repeats() {
        assert_eq!((kind_of(0.0), kind_of(0.29), kind_of(0.3), kind_of(0.65), kind_of(0.66), kind_of(1.0)), (Kind::Willow, Kind::Willow, Kind::Peony, Kind::Peony, Kind::Crackle, Kind::Crackle));
        let a: Vec<_> = run(&mut Show::new(5), 12.0, 1.0, 0.7).iter().map(|(t, b)| (*t, b.centre, b.stars)).collect();
        let b: Vec<_> = run(&mut Show::new(5), 12.0, 1.0, 0.7).iter().map(|(t, b)| (*t, b.centre, b.stars)).collect();
        let c: Vec<_> = run(&mut Show::new(6), 12.0, 1.0, 0.7).iter().map(|(t, b)| (*t, b.centre, b.stars)).collect();
        assert!(!a.is_empty());
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn pools_of_light_follow_their_bursts_and_expire() {
        let mut show = Show::new(2);
        show.step(&input(1.0, 1.0, 0.0, &[Hit { pos: 0.2, strength: 1.0 }]));
        assert_eq!(show.pools.len(), 1);
        let u = show.pack(1.3);
        assert!((u.pools[0][2] - 0.3).abs() < 1e-5 && u.pools[0][3] > 0.9, "{:?}", u.pools[0]);
        assert_eq!(u.pools[2][3], 0.0, "unused pools have no amplitude");
        show.step(&input(5.0, 1.0, 0.0, &[]));
        assert!(show.pools.is_empty());
    }

    /// The CPU work of one frame (the hit detector, the choreography and the GPU table) on a dense stream of hits.
    /// `cargo test --release cpu_cost -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn cpu_cost() {
        let mut show = Show::new(1);
        let mut rh = RiseHits::new();
        let frames = 200_000;
        let t0 = std::time::Instant::now();
        let mut sink = 0.0f32;
        for k in 0..frames {
            let now = k as f32 / 120.0;
            let mut rise = [0.05f32; 192];
            let at = (k / 7 * 37) % 180;
            for j in at..(at + 8).min(192) {
                rise[j] = if k % 60 < 4 { 1.1 } else { 0.1 };
            }
            let hits = rh.update(&rise, 1.0, now as f64, true);
            let out = show.step(&input(now, 1.0, 0.8, &hits));
            sink += show.pack(now).shells[0][0] + out.len() as f32;
        }
        let us = t0.elapsed().as_secs_f64() * 1e6 / frames as f64;
        eprintln!("CPU per frame: {us:.2} us ({sink})");
    }

    #[test]
    fn the_gpu_table_matches_the_shader() {
        // 16 rockets and 8 pools, two vec4 each, in uniform-buffer units
        assert_eq!(std::mem::size_of::<FwU>(), (2 * MAX_SHELLS + 2 * MAX_POOLS) * 16);
        assert_eq!(std::mem::size_of::<FwU>() % 16, 0);
        let mut show = Show::new(1);
        show.shells.push(Shell { t0: 0.0, p0: Vec3::new(1.0, 0.1, 2.0), v0: Vec3::new(0.0, 12.0, 0.0), line: 0.25, fizzle: None });
        let u = show.pack(0.5);
        assert_eq!(u.shells[0][3], 1.0);
        assert!((u.shells[0][1] - (0.1 + 12.0 * 0.5 - 0.5 * SHELL_G * 0.25)).abs() < 1e-4);
        assert!((u.shells[1][1] - (12.0 - SHELL_G * 0.5)).abs() < 1e-4 && (u.shells[1][3] - 0.25).abs() < 1e-6);
        assert_eq!(u.shells[2][3], 0.0);
    }
}
