use super::*;
use super::fireworks::{lerp, Rng};
use super::smoke::{run_pass, Stamper};
use crate::modulation::{Hit, RiseHits};
use std::collections::VecDeque;

/// A glossy dark pool that keeps a record of the last few seconds of hits. Each hit found by the same detector as
/// the Fireworks and the storm (`modulation::RiseHits`) breaks the water where it landed on the line (bass at the
/// left) at once, and throws a few droplets in arcs; every droplet that lands drops an impulse into the water. The
/// surface is the damped wave equation on a grid of cells 1/20 of a unit wide (a fragment pass per fixed step, like
/// the smoke's solver), with a sponge around its edge so rings leave rather than bounce back. A heavy low hit makes
/// a wide ring and the light, quick drops of the highs small ones. The water is shaded from the slope of the
/// surface: it mirrors a horizon haze and four light towers whose brightness follows the bands, so a ring that
/// passes sweeps their reflections across the pool. The droplets are closed-form arcs the CPU can evaluate at any
/// moment (`Pond`), so the particle system is not needed and the landings are known to the frame.
pub struct Ripple {
    step: wgpu::RenderPipeline,
    slope: wgpu::RenderPipeline,
    mip: wgpu::RenderPipeline,
    sky: wgpu::RenderPipeline,
    water: wgpu::RenderPipeline,
    towers: wgpu::RenderPipeline,
    drops: wgpu::RenderPipeline,
    drops_refl: wgpu::RenderPipeline,
    sim_bgl: wgpu::BindGroupLayout,
    mip_bgl: wgpu::BindGroupLayout,
    water_bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    water_sampler: wgpu::Sampler,
    ubuf: wgpu::Buffer,
    sim: Option<Sim>,
    /// Which of the two state textures holds the current field.
    cur: usize,
    pond: Pond,
    hits: RiseHits,
    /// Impulses made but not yet in the water (a frame that ran no step keeps them for the next).
    pending: VecDeque<Impulse>,
    acc: f32,
    last_frame: u64,
    last_time: f32,
    clear: bool,
    seed: u64,
}

/// Textures and bind groups for one window shape.
struct Sim {
    size: [u32; 2],
    geo: Geo,
    state: [wgpu::TextureView; 2],
    slope: wgpu::TextureView,
    slope_mips: Vec<wgpu::TextureView>,
    bg_step: [wgpu::BindGroup; 2],
    bg_slope: [wgpu::BindGroup; 2],
    bg_mip: Vec<wgpu::BindGroup>,
    bg_water: wgpu::BindGroup,
    _tex: Vec<wgpu::Texture>,
}

/// The pond's place in the world and its grid. Cells are square, so the physics does not depend on the aspect.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Geo {
    x0: f32,
    z0: f32,
    width: f32,
    depth: f32,
    grid: [u32; 2],
}

/// Mirrors `RpU` in `ripple_common.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RpU {
    grid: [f32; 4],
    pond: [f32; 4],
    step: [f32; 4],
    damp: [f32; 4],
    look: [f32; 4],
    imp: [[f32; 4]; MAX_IMPULSES],
    drops: [[f32; 4]; 2 * MAX_DROPS],
}

/// Cells to a world unit.
const CELLS: f32 = 20.0;
/// The fixed timestep of the wave equation, and the most steps a frame may run (a slower frame runs the water
/// slower than real time instead of spiralling).
const STEP_DT: f32 = 1.0 / 240.0;
const MAX_STEPS: u32 = 8;
/// Speed of a ring, world units a second. Explicit time stepping needs `WAVE_SPEED * CELLS * STEP_DT` below 1/sqrt(2).
const WAVE_SPEED: f32 = 7.0;
/// Damping of the open water at Ripple life 1 (1/s), the sponge's at the very edge, its width in cells, the leak
/// that returns a lasting displacement to flat, and the limits that keep a dense passage from blowing the field up.
const DAMPING: f32 = 0.7;
const SPONGE: f32 = 14.0;
const SPONGE_CELLS: f32 = 60.0;
const LEAK: f32 = 0.25;
const H_LIMIT: f32 = 0.40;
const V_LIMIT: f32 = 12.0;
/// A scene that was not updated for this many frames starts again from calm water.
const STALE_FRAMES: u64 = 30;
const MIP_LEVELS: u32 = 4;
const MAX_IMPULSES: usize = 16;
const MAX_DROPS: usize = 64;
/// Gravity on a droplet, and the height it leaves the water from.
const G: f32 = 14.0;
const DROP_Y0: f32 = 0.06;
/// Droplets scatter this far in depth of the line.
const POP_JITTER: f32 = 1.0;
/// The pond: from this z (north) it is this deep, and it is wider than the line the drops start from, which it
/// follows (see `line_width`).
const POND_Z0: f32 = -14.0;
const POND_DEPTH: f32 = 24.0;
/// Below this gate level the music has stopped and nothing is dropped; the water damps out faster.
const CLOSED_GATE: f32 = 0.2;

/// The line the drops start from: it fills most of the picture at any aspect, within sensible bounds (the Fireworks'
/// and the storm's too, since a rectilinear view stretches whatever stands near the edges).
pub fn line_width(aspect: f32) -> f32 {
    (12.5 * aspect).clamp(14.0, 40.0)
}

fn geometry(aspect: f32) -> Geo {
    let width = (line_width(aspect) * 1.6).clamp(24.0, 64.0);
    // the grid is a whole number of 8-cell blocks so the mip chain halves exactly
    let blocks = |len: f32| ((len * CELLS / 8.0).round() as u32).max(1) * 8;
    Geo { x0: -0.5 * width, z0: POND_Z0, width, depth: POND_DEPTH, grid: [blocks(width), blocks(POND_DEPTH)] }
}

// ---------------------------------------------------------------------------------- the choreography

/// An impulse for the water: where (world), how wide (a Ricker wavelet's sigma, world units) and how deep a
/// crater (negative) or high a hump (positive), in world units of height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Impulse {
    pub x: f32,
    pub z: f32,
    pub sigma: f32,
    pub amp: f32,
}

/// How a hit breaks the water: bass is a broad, deep crater, treble a small, shallow one; a harder hit is bigger.
/// Returns the Ricker sigma (world) and the depth.
pub fn pop_for(line: f32, strength: f32) -> (f32, f32) {
    let t = line.clamp(0.0, 1.0);
    let s = strength.clamp(0.0, 1.0);
    (lerp(0.36, 0.11, t) * (0.75 + 0.45 * s), -(0.080 + 0.170 * s) * lerp(1.0, 0.55, t))
}

/// Droplets that make rings: one for a light hit, up to five for a hard one with Density up; plus a few that only
/// show (they fall back without a ring).
pub fn drops_for(strength: f32, density: f32) -> (usize, usize) {
    let s = strength.clamp(0.0, 1.0);
    let d = density.clamp(0.1, 2.0);
    ((1.0 + (2.4 * s * d).round()).min(5.0) as usize, (3.0 + (5.0 * s * d).round()).min(8.0) as usize)
}

/// One droplet: a ballistic arc from the place its hit broke the water.
#[derive(Clone, Copy, Debug)]
struct Drop {
    t0: f32,
    p0: Vec3,
    v0: Vec3,
    line: f32,
    size: f32,
    /// When it comes back down.
    end: f32,
    /// The ring it makes where it lands (sigma, depth); none for a droplet that only shows.
    ring: Option<(f32, f32)>,
}

impl Drop {
    fn pos(&self, now: f32) -> Vec3 {
        let t = now - self.t0;
        self.p0 + self.v0 * t - Vec3::Y * (0.5 * G * t * t)
    }

    fn vel(&self, now: f32) -> Vec3 {
        self.v0 - Vec3::Y * (G * (now - self.t0))
    }
}

/// Seconds a droplet thrown up at `vy` from `y0` stays in the air.
fn flight_time(vy: f32, y0: f32) -> f32 {
    (vy + (vy * vy + 2.0 * G * y0).sqrt()) / G
}

pub struct Input<'a> {
    pub now: f32,
    pub gate: f32,
    pub hits: &'a [Hit],
    /// Width of the line the drops start from.
    pub width: f32,
    pub density: f32,
}

/// The choreography: hits become craters and droplets, landings become impulses. No GPU in here.
pub struct Pond {
    drops: Vec<Drop>,
    rng: Rng,
}

impl Pond {
    pub fn new(seed: u64) -> Pond {
        Pond { drops: Vec::new(), rng: Rng::seeded(seed) }
    }

    pub fn reset(&mut self, seed: u64) {
        *self = Pond::new(seed);
    }

    /// Calm water, the random sequence carries on.
    pub fn clear(&mut self) {
        self.drops.clear();
    }

    fn throw(&mut self, now: f32, x: f32, z: f32, line: f32, size: f32, range: f32, time: f32, ring: Option<(f32, f32)>) {
        let az = self.rng.next() * std::f32::consts::TAU;
        let vy = 0.5 * G * time;
        let r = range / time;
        let d = Drop {
            t0: now,
            p0: Vec3::new(x, DROP_Y0, z),
            v0: Vec3::new(r * az.cos(), vy, r * az.sin()),
            line,
            size,
            end: now + flight_time(vy, DROP_Y0),
            ring,
        };
        if self.drops.len() >= MAX_DROPS {
            self.drops.remove(0);
        }
        self.drops.push(d);
    }

    /// Advance to `i.now`; returns the impulses to put in the water: the craters of this frame's hits and the
    /// landings of droplets that have come down.
    pub fn step(&mut self, i: &Input) -> Vec<Impulse> {
        let now = i.now;
        let mut out = Vec::new();
        // landings: a droplet that is down makes its ring at the place it reached, and leaves the table
        self.drops.retain(|d| {
            if now < d.end {
                return true;
            }
            if let Some((sigma, amp)) = d.ring {
                let p = d.pos(d.end);
                out.push(Impulse { x: p.x, z: p.z, sigma, amp });
            }
            false
        });

        if i.gate < CLOSED_GATE {
            return out; // the music has stopped: nothing is dropped, and what is in the air still comes down
        }
        let mut hits: Vec<Hit> = i.hits.to_vec();
        hits.sort_by(|a, b| b.strength.partial_cmp(&a.strength).unwrap());
        for h in hits {
            let (sigma, amp) = pop_for(h.pos, h.strength);
            let x = (h.pos - 0.5) * i.width;
            let z = (self.rng.next() - 0.5) * 2.0 * POP_JITTER;
            out.push(Impulse { x, z, sigma, amp });
            let (n_ring, n_show) = drops_for(h.strength, i.density);
            for _ in 0..n_ring {
                // a heavy drop of the lows falls near and quickly; the light drops of the highs scatter far
                let w = 0.45 + 0.55 * self.rng.next();
                let time = lerp(0.50, 0.32, h.pos) * (0.8 + 0.5 * self.rng.next());
                let range = lerp(1.6, 4.5, h.pos) * (0.35 + 0.65 * self.rng.next());
                self.throw(now, x, z, h.pos, 0.035 + 0.045 * w, range, time, Some((sigma * (0.45 + 0.40 * w), amp * (0.30 + 0.40 * w))));
            }
            for _ in 0..n_show {
                let time = 0.25 + 0.35 * self.rng.next();
                let range = 1.5 * self.rng.next();
                let size = 0.018 + 0.012 * self.rng.next();
                self.throw(now, x, z, h.pos, size, range, time, None);
            }
        }
        out
    }

    /// The droplets as they are at `now`, for the GPU.
    fn pack(&self, now: f32) -> [[f32; 4]; 2 * MAX_DROPS] {
        let mut out = [[0.0f32; 4]; 2 * MAX_DROPS];
        for (k, d) in self.drops.iter().take(MAX_DROPS).enumerate() {
            let (p, v) = (d.pos(now), d.vel(now));
            out[2 * k] = [p.x, p.y.max(0.0), p.z, d.size];
            out[2 * k + 1] = [v.x, v.y, v.z, d.line];
        }
        out
    }
}

// ---------------------------------------------------------------------------------- the solver's drive

/// How many fixed steps a frame of `dt` seconds runs. Rounding (rather than flooring) keeps a steady frame rate
/// that divides the step rate at exactly its share, instead of the counts clock jitter would alternate between.
fn steps_for(acc: &mut f32, dt: f32) -> u32 {
    *acc += dt;
    let n = ((*acc / STEP_DT).round().max(0.0) as u32).min(MAX_STEPS);
    *acc -= n as f32 * STEP_DT;
    *acc = acc.clamp(-0.5 * STEP_DT, 0.5 * STEP_DT); // after a stall the backlog is dropped, not repaid
    n
}

/// Everything the passes read this frame. `taken` are the impulses going into the water now, `steps` how many
/// steps share them.
fn drive(g: &Geo, p: &SceneParams, taken: &[Impulse], steps: u32, drops: [[f32; 4]; 2 * MAX_DROPS]) -> RpU {
    let gate = p.gate.clamp(0.0, 1.0);
    let life = p.ripple_life.clamp(0.3, 3.0);
    let mut imp = [[0.0f32; 4]; MAX_IMPULSES];
    for (slot, i) in imp.iter_mut().zip(taken) {
        *slot = [(i.x - g.x0) * CELLS, (i.z - g.z0) * CELLS, (i.sigma * CELLS).max(1.2), i.amp];
    }
    RpU {
        grid: [g.grid[0] as f32, g.grid[1] as f32, 1.0 / g.grid[0] as f32, 1.0 / g.grid[1] as f32],
        pond: [g.x0, g.z0, g.width, g.depth],
        step: [STEP_DT, (WAVE_SPEED * CELLS) * (WAVE_SPEED * CELLS), 1.0 / steps.max(1) as f32, taken.len().min(MAX_IMPULSES) as f32],
        // the rings die out in a few seconds, and faster once the music stops, leaving calm glass
        damp: [DAMPING / life + 2.2 * (1.0 - gate), SPONGE, SPONGE_CELLS, H_LIMIT],
        look: [LEAK, V_LIMIT, 0.0, 0.0],
        imp,
        drops,
    }
}

// ---------------------------------------------------------------------------------- the scene

fn texture(device: &wgpu::Device, label: &str, size: [u32; 2], format: wgpu::TextureFormat, mips: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: size[0].max(1), height: size[1].max(1), depth_or_array_layers: 1 },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

impl Ripple {
    pub fn new(ctx: &Ctx) -> Ripple {
        let dev = ctx.device;
        let common = include_str!("ripple_common.wgsl");
        let sim_sh = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ripple_sim"),
            source: wgpu::ShaderSource::Wgsl(format!("{common}\n{}", include_str!("ripple_sim.wgsl")).into()),
        });
        let src = format!("{}\n{}\n{common}\n{}", include_str!("particles_common.wgsl"), include_str!("particles_draw.wgsl"), include_str!("ripple.wgsl"));
        let sh = ctx.module("ripple", &src);

        let uniform = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let sampler = |binding, visibility| wgpu::BindGroupLayoutEntry { binding, visibility, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None };
        let tex = |binding, visibility, filterable| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let frag = wgpu::ShaderStages::FRAGMENT;
        let both = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT;
        // the solver reads its Rg32Float state with textureLoad (a 32-bit float texture is not filterable)
        let sim_bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ripple_sim_bgl"),
            entries: &[uniform(0, frag), sampler(1, frag), tex(2, frag, false)],
        });
        let mip_bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ripple_mip_bgl"),
            entries: &[uniform(0, frag), sampler(1, frag), tex(2, frag, true)],
        });
        // the scene passes: the uniform, the water's sampler and the slope texture, in group 1 after the scene's
        // own group 0 (bindings 2-4, since the sprite code owns 0 and 1 of the group)
        let water_bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ripple_water_bgl"),
            entries: &[uniform(2, both), sampler(3, both), tex(4, both, true)],
        });
        let sim_layout = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&sim_bgl)], immediate_size: 0 });
        let mip_layout = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&mip_bgl)], immediate_size: 0 });
        let scene_layout = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(ctx.bgl), Some(&water_bgl)], immediate_size: 0 });

        let offscreen = |label: &str, fs: &str, format: wgpu::TextureFormat, layout: &wgpu::PipelineLayout| {
            dev.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState { module: &sim_sh, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
                fragment: Some(wgpu::FragmentState {
                    module: &sim_sh,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        use wgpu::TextureFormat::{Rg32Float, Rgba16Float};
        let opaque = Pipe { depth_write: false, always: false, blend: None, msaa: false };
        let glow = Pipe { depth_write: false, always: false, blend: ADDITIVE, msaa: false };

        let linear = |anisotropy| {
            dev.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                anisotropy_clamp: anisotropy,
                ..Default::default()
            })
        };
        let ubuf = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ripple_u"),
            size: std::mem::size_of::<RpU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ripple {
            step: offscreen("ripple_step", "fs_step", Rg32Float, &sim_layout),
            slope: offscreen("ripple_slope", "fs_slope", Rgba16Float, &sim_layout),
            mip: offscreen("ripple_mip", "fs_mip", Rgba16Float, &mip_layout),
            sky: ctx.pipeline_with(&scene_layout, &sh, "ripple_sky", "vs_sky", "fs_sky", opaque),
            water: ctx.pipeline_with(&scene_layout, &sh, "ripple_water", "vs_floor", "fs_water", opaque),
            towers: ctx.pipeline_with(&scene_layout, &sh, "ripple_towers", "vs_tower", "fs_tower", glow),
            drops: ctx.pipeline_with(&scene_layout, &sh, "ripple_drops", "vs_drop", "fs_sprite", glow),
            drops_refl: ctx.pipeline_with(&scene_layout, &sh, "ripple_drops_refl", "vs_drop_refl", "fs_sprite", glow),
            sim_bgl, mip_bgl, water_bgl,
            sampler: linear(1),
            water_sampler: linear(8),
            ubuf,
            sim: None,
            cur: 0,
            pond: Pond::new(1),
            hits: RiseHits::new(),
            pending: VecDeque::new(),
            acc: 0.0,
            last_frame: 0,
            last_time: 0.0,
            clear: true,
            seed: 1,
        }
    }

    fn ensure_sim(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if self.sim.as_ref().map_or(false, |s| s.size == size) {
            return;
        }
        use wgpu::TextureFormat::{Rg32Float, Rgba16Float};
        let geo = geometry(size[0] as f32 / size[1].max(1) as f32);
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let state = [texture(device, "ripple_state", geo.grid, Rg32Float, 1), texture(device, "ripple_state", geo.grid, Rg32Float, 1)];
        let slope = texture(device, "ripple_slope", geo.grid, Rgba16Float, MIP_LEVELS);
        let level = |l: u32| slope.create_view(&wgpu::TextureViewDescriptor { base_mip_level: l, mip_level_count: Some(1), ..Default::default() });
        let slope_mips: Vec<wgpu::TextureView> = (0..MIP_LEVELS).map(level).collect();
        let state_v = [view(&state[0]), view(&state[1])];
        let slope_all = view(&slope);
        let bind = |layout: &wgpu::BindGroupLayout, tex: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ripple_bg"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: self.ubuf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(tex) },
                ],
            })
        };
        let bg_water = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ripple_water_bg"),
            layout: &self.water_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 2, resource: self.ubuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.water_sampler) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&slope_all) },
            ],
        });
        let sim = Sim {
            size,
            geo,
            bg_step: [bind(&self.sim_bgl, &state_v[0]), bind(&self.sim_bgl, &state_v[1])],
            bg_slope: [bind(&self.sim_bgl, &state_v[0]), bind(&self.sim_bgl, &state_v[1])],
            bg_mip: (1..MIP_LEVELS as usize).map(|l| bind(&self.mip_bgl, &slope_mips[l - 1])).collect(),
            bg_water,
            state: state_v,
            slope: slope_all,
            slope_mips,
            _tex: vec![state[0].clone(), state[1].clone(), slope],
        };
        self.sim = Some(sim);
        self.clear = true;
    }
}

impl Scene for Ripple {
    fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, f: &Frame) {
        self.ensure_sim(device, f.size);
        if f.frame_no == self.last_frame {
            return; // already advanced for this frame (a crossfade between two looks of this scene)
        }
        let continuous = self.last_frame != 0 && f.frame_no <= self.last_frame + STALE_FRAMES;
        let dt = if continuous { (f.p.time - self.last_time).clamp(0.0, 0.1) } else { 1.0 / 60.0 };
        if self.last_frame != 0 && !continuous {
            self.clear = true; // a long absence: the water would be stale
        }
        if self.last_frame == 0 || !continuous {
            self.pond.clear();
            self.hits.reset();
            self.pending.clear();
        }
        self.last_frame = f.frame_no;
        self.last_time = f.p.time;
        let Some(sim) = &self.sim else { return };

        let p = f.p;
        let hits = self.hits.update(&p.rise, p.gradient.clamp(0.2, 5.0), p.time as f64, p.gate > 0.5);
        let made = self.pond.step(&Input { now: p.time, gate: p.gate, hits: &hits, width: line_width(f.aspect), density: p.density });
        if std::env::var_os("GEQ_RPDEBUG").is_some() {
            // development aid: one line per impulse, for checking timing against a fixture's exact event times
            for i in &made {
                eprintln!("[ripple] t={:.3} impulse at ({:.1}, {:.1}) sigma {:.2} depth {:.3}", p.time, i.x, i.z, i.sigma, i.amp);
            }
        }
        self.pending.extend(made);

        let steps = if self.clear { 0 } else { steps_for(&mut self.acc, dt) };
        let taken: Vec<Impulse> = if steps > 0 { self.pending.drain(..self.pending.len().min(MAX_IMPULSES)).collect() } else { Vec::new() };
        let u = drive(&sim.geo, p, &taken, steps, self.pond.pack(p.time));
        queue.write_buffer(&self.ubuf, 0, bytemuck::bytes_of(&u));

        if std::mem::take(&mut self.clear) {
            self.acc = 0.0;
            self.cur = 0;
            self.pending.clear();
            for v in &sim.state {
                enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("ripple_clear"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: v,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
            }
        }

        let mut stamp = Stamper { ts: f.ts.as_ref(), first: true };
        let mut cur = self.cur;
        for _ in 0..steps {
            run_pass(enc, "ripple_step", &self.step, &sim.bg_step[cur], &sim.state[1 - cur], stamp.next(false));
            cur = 1 - cur;
        }
        self.cur = cur;
        run_pass(enc, "ripple_slope", &self.slope, &sim.bg_slope[cur], &sim.slope_mips[0], stamp.next(false));
        for l in 1..MIP_LEVELS as usize {
            run_pass(enc, "ripple_mip", &self.mip, &sim.bg_mip[l - 1], &sim.slope_mips[l], stamp.next(l + 1 == MIP_LEVELS as usize));
        }
    }

    fn uses_update(&self, _p: &SceneParams) -> bool {
        true
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, _f: &Frame) {
        let Some(sim) = &self.sim else { return };
        pass.set_bind_group(1, &sim.bg_water, &[]);
        pass.set_pipeline(&self.sky);
        pass.draw(0..6, 0..1);
        pass.set_pipeline(&self.water);
        pass.draw(0..6, 0..1);
        pass.set_pipeline(&self.towers);
        pass.draw(0..6, 0..4);
        pass.set_pipeline(&self.drops_refl);
        pass.draw(0..6, 0..MAX_DROPS as u32);
        pass.set_pipeline(&self.drops);
        pass.draw(0..6, 0..MAX_DROPS as u32);
    }

    fn wants_msaa(&self) -> bool {
        false // a lit plane, soft streaks and analytic towers
    }

    fn camera(&self, aspect: f32, p: &SceneParams) -> (Mat4, Vec3) {
        // a little above the water looking across it: the pool fills the lower two thirds, the towers stand on the far shore
        let o = Orbit { eye: Vec3::new(0.0, 5.5, 17.0), target: Vec3::new(0.0, 0.6, -2.0), fov_deg: 40.0 };
        orbit(&o, aspect, &p.cam)
    }

    fn shape(&self, aspect: f32) -> [f32; 3] {
        [line_width(aspect), 0.0, 0.0]
    }

    fn clear_color(&self) -> wgpu::Color {
        wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }
    }

    fn reset(&mut self, seed: u64) {
        self.clear = true;
        self.seed = seed;
        self.pond.reset(seed);
        self.hits.reset();
        self.pending.clear();
        self.last_frame = 0;
    }

    fn resize(&mut self, _device: &wgpu::Device, _size: [u32; 2]) {
        self.sim = None; // recreated, in calm water, on the next frame
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
    fn a_hit_breaks_the_water_in_the_same_frame_and_where_it_landed() {
        let mut p = Pond::new(3);
        let out = p.step(&input(1.0, 1.0, &[Hit { pos: 0.15, strength: 0.9 }]));
        assert_eq!(out.len(), 1, "{out:?}");
        let i = out[0];
        assert!((i.x - (0.15 - 0.5) * W).abs() < 1e-4 && i.z.abs() <= POP_JITTER && i.amp < 0.0);
        // droplets are in the air, none has landed yet
        assert!(p.drops.len() >= 4);
        assert!(p.drops.iter().all(|d| d.end > 1.2 && d.end < 2.2), "flights: {:?}", p.drops.iter().map(|d| d.end - 1.0).collect::<Vec<_>>());
    }

    #[test]
    fn position_and_strength_set_the_size_of_the_craters_and_the_drops() {
        // bass is broad and deep, treble small and shallow; a harder hit is bigger
        let (bs, ba) = pop_for(0.0, 0.8);
        let (ts, ta) = pop_for(1.0, 0.8);
        assert!(bs > 2.0 * ts && ba < ta * 1.5 && ba.abs() > ta.abs());
        let (s_soft, a_soft) = pop_for(0.5, 0.2);
        let (s_hard, a_hard) = pop_for(0.5, 1.0);
        assert!(s_hard > s_soft && a_hard.abs() > 2.0 * a_soft.abs());
        // strength sets how many drops, within bounds; Density scales them
        let (soft, hard) = (drops_for(0.1, 1.0), drops_for(1.0, 1.0));
        assert!(hard.0 > soft.0 && hard.1 > soft.1 && hard.0 <= 5 && hard.1 <= 8);
        assert!(drops_for(1.0, 2.0).0 <= 5 && drops_for(0.0, 0.1).0 == 1);
    }

    #[test]
    fn a_droplet_lands_exactly_when_its_arc_says_and_makes_one_ring() {
        let mut p = Pond::new(5);
        p.step(&input(2.0, 1.0, &[Hit { pos: 0.5, strength: 1.0 }]));
        let rings: Vec<Drop> = p.drops.iter().copied().filter(|d| d.ring.is_some()).collect();
        assert!(!rings.is_empty());
        for d in &rings {
            // closed form: it is at the water's surface exactly when it ends, and above it before then
            assert!(d.pos(d.end).y.abs() < 1e-3, "{:?}", d.pos(d.end));
            assert!(d.pos(0.5 * (d.t0 + d.end)).y > DROP_Y0 + 0.1);
        }
        let last = rings.iter().map(|d| d.end).fold(0.0, f32::max);
        // nothing before the first landing, then one impulse per ring, and the table empties
        let first = rings.iter().map(|d| d.end).fold(f32::MAX, f32::min);
        assert!(p.step(&input(first - 0.01, 1.0, &[])).is_empty());
        let mut landed = 0;
        let mut t = first - 0.01;
        while t < last + 0.05 {
            t += 1.0 / 120.0;
            landed += p.step(&input(t, 1.0, &[])).len();
        }
        assert_eq!(landed, rings.len());
        assert!(p.drops.is_empty());
        // a droplet that only shows leaves no ring
        assert!(p.step(&input(t + 1.0, 1.0, &[])).is_empty());
    }

    #[test]
    fn the_table_is_bounded_and_silence_drops_nothing_but_lets_the_air_clear() {
        let mut p = Pond::new(1);
        for k in 0..60 {
            p.step(&input(k as f32 * 0.01, 1.0, &[Hit { pos: (k % 9) as f32 / 8.0, strength: 1.0 }]));
            assert!(p.drops.len() <= MAX_DROPS);
        }
        assert_eq!(p.drops.len(), MAX_DROPS);
        // the gate closes: no new craters, and what is in the air still comes down and rings
        let during = p.step(&input(0.7, 0.0, &[Hit { pos: 0.5, strength: 1.0 }]));
        assert!(during.iter().all(|i| i.sigma > 0.0));
        let before = p.drops.len();
        p.step(&input(0.71, 0.0, &[Hit { pos: 0.5, strength: 1.0 }]));
        assert!(p.drops.len() <= before, "a closed gate throws nothing");
        p.step(&input(5.0, 0.0, &[]));
        assert!(p.drops.is_empty());
    }

    #[test]
    fn the_same_seed_repeats() {
        let run = |seed| {
            let mut p = Pond::new(seed);
            (0..400)
                .flat_map(|k| p.step(&input(k as f32 / 120.0, 1.0, &if k % 23 == 0 { vec![Hit { pos: (k % 5) as f32 / 4.0, strength: 0.8 }] } else { vec![] })))
                .map(|i| (i.x.to_bits(), i.z.to_bits(), i.sigma.to_bits()))
                .collect::<Vec<_>>()
        };
        assert!(!run(5).is_empty());
        assert_eq!(run(5), run(5));
        assert_ne!(run(5), run(6));
    }

    #[test]
    fn the_gpu_table_matches_the_shader() {
        // 5 vec4 of parameters, 16 of impulses and 128 of droplets
        assert_eq!(std::mem::size_of::<RpU>(), 16 * (5 + MAX_IMPULSES + 2 * MAX_DROPS));
        assert_eq!((MAX_IMPULSES, MAX_DROPS), (16, 64));
        let mut p = Pond::new(1);
        p.step(&input(1.0, 1.0, &[Hit { pos: 0.3, strength: 1.0 }]));
        let t = p.pack(1.1);
        assert!(t.iter().flatten().all(|v| v.is_finite()));
        assert!(t[0][3] > 0.0 && t[2 * p.drops.len()][3] == 0.0, "used droplets have a radius, unused none");
        assert!(t.iter().step_by(2).all(|d| d[1] >= 0.0), "no droplet is drawn below the water");
    }

    #[test]
    fn the_grid_is_in_whole_blocks_and_covers_the_pond_at_every_aspect() {
        for aspect in [16.0 / 9.0, 21.0 / 9.0, 32.0 / 9.0, 1.0] {
            let g = geometry(aspect);
            assert!(g.grid[0] % 8 == 0 && g.grid[1] % 8 == 0, "{g:?}");
            assert!(((g.grid[0] as f32 / CELLS) - g.width).abs() < 0.3 && ((g.grid[1] as f32 / CELLS) - g.depth).abs() < 0.3, "{g:?}");
            // the drops' line, and the droplets' reach beyond it, stay inside the pond (and outside its sponge when they can)
            assert!(0.5 * line_width(aspect) + 5.0 <= 0.5 * g.width, "{g:?}");
            assert!(g.x0 == -0.5 * g.width);
        }
        assert_eq!(geometry(32.0 / 9.0).grid, [1280, 480]);
    }

    #[test]
    fn steps_follow_real_time_at_any_frame_rate_and_stalls_are_capped() {
        for fps in [30.0f32, 60.0, 120.0, 144.0] {
            let mut acc = 0.0;
            let n: u32 = (0..(fps as usize * 4)).map(|_| steps_for(&mut acc, 1.0 / fps)).sum();
            assert!((n as i32 - 960).abs() <= 2, "{fps} fps ran {n} steps in four seconds");
        }
        let mut acc = 0.0;
        assert_eq!(steps_for(&mut acc, 0.1), MAX_STEPS);
        assert!(acc.abs() <= 0.5 * STEP_DT + 1e-6);
        let mut acc = 0.0;
        assert!((0..600).all(|k| steps_for(&mut acc, 1.0 / 120.0 + if k % 2 == 0 { 0.0003 } else { -0.0003 }) == 2), "a steady 120 Hz is two steps a frame");
    }

    // ---- a CPU copy of the shader's step, for the checks the GPU cannot report back

    struct Field {
        w: usize,
        h: usize,
        s: Vec<[f32; 2]>,
    }

    impl Field {
        fn new(w: usize, h: usize) -> Field {
            Field { w, h, s: vec![[0.0; 2]; w * h] }
        }

        fn at(&self, x: i32, y: i32) -> [f32; 2] {
            self.s[y.clamp(0, self.h as i32 - 1) as usize * self.w + x.clamp(0, self.w as i32 - 1) as usize]
        }

        /// One step, as in `fs_step`.
        fn step(&mut self, gamma0: f32, sponge: f32, sponge_cells: f32, imps: &[(f32, f32, f32, f32)]) {
            let c2 = (WAVE_SPEED * CELLS) * (WAVE_SPEED * CELLS);
            let mut next = self.s.clone();
            for y in 0..self.h as i32 {
                for x in 0..self.w as i32 {
                    let s0 = self.at(x, y);
                    let lap = self.at(x - 1, y)[0] + self.at(x + 1, y)[0] + self.at(x, y - 1)[0] + self.at(x, y + 1)[0] - 4.0 * s0[0];
                    let (qx, qy) = (x as f32 + 0.5, y as f32 + 0.5);
                    let d = qx.min(self.w as f32 - qx).min(qy.min(self.h as f32 - qy));
                    let sp = (1.0 - d / sponge_cells).clamp(0.0, 1.0);
                    let gamma = gamma0 + sponge * sp * sp;
                    let mut v = s0[1] * (-gamma * STEP_DT).exp() + STEP_DT * c2 * lap;
                    let mut hgt = (s0[0] + STEP_DT * v) * (-LEAK * STEP_DT).exp();
                    for &(cx, cy, sigma, amp) in imps {
                        let (dx, dy) = ((qx - cx) / sigma, (qy - cy) / sigma);
                        let q2 = 0.5 * (dx * dx + dy * dy);
                        if q2 < 12.0 {
                            hgt += amp * (1.0 - q2) * (-q2).exp();
                        }
                    }
                    hgt = hgt.clamp(-H_LIMIT, H_LIMIT);
                    v = v.clamp(-V_LIMIT, V_LIMIT);
                    next[y as usize * self.w + x as usize] = [hgt, v];
                }
            }
            self.s = next;
        }

        /// Wave energy: the squared slope plus the squared velocity (in the units the scheme uses).
        fn energy(&self) -> f32 {
            let mut e = 0.0;
            for y in 0..self.h as i32 {
                for x in 0..self.w as i32 {
                    let v = self.at(x, y)[1];
                    let (gx, gy) = (self.at(x + 1, y)[0] - self.at(x - 1, y)[0], self.at(x, y + 1)[0] - self.at(x, y - 1)[0]);
                    e += v * v + (WAVE_SPEED * CELLS).powi(2) * 0.25 * (gx * gx + gy * gy);
                }
            }
            e
        }
    }

    #[test]
    fn the_scheme_is_stable_and_a_sponge_makes_rings_leave_instead_of_coming_back() {
        // explicit stepping needs the wave to cross less than 1/sqrt(2) of a cell a step
        assert!(WAVE_SPEED * CELLS * STEP_DT < 0.70, "CFL number {}", WAVE_SPEED * CELLS * STEP_DT);
        let (w, h) = (160, 120);
        let drop = [(80.0, 60.0, 4.0, -0.15)];
        let run = |sponge: f32| {
            let mut f = Field::new(w, h);
            f.step(0.3, sponge, 40.0, &drop);
            let e0 = f.energy();
            // a ring crosses 70 cells at 140 cells/s in half a second; a second is two crossings of the field
            for _ in 0..(1.0 / STEP_DT) as usize {
                f.step(0.3, sponge, 40.0, &[]);
            }
            (e0, f.energy(), f.s.iter().map(|s| s[0].abs()).fold(0.0, f32::max))
        };
        let (e0, with, hmax) = run(14.0);
        let (_, without, _) = run(0.0);
        assert!(hmax <= H_LIMIT && with.is_finite());
        assert!(with < 0.06 * e0, "with a sponge {with} of {e0} is left after a second");
        assert!(without > 4.0 * with, "without one {without} stays and bounces, with one {with}");
    }

    #[test]
    fn a_dense_passage_cannot_blow_the_field_up_and_the_pond_calms_afterwards() {
        let mut f = Field::new(96, 64);
        let mut rng = Rng::seeded(9);
        let (mut worst_h, mut worst_v): (f32, f32) = (0.0, 0.0);
        for k in 0..1200 {
            // a heavy crater somewhere every few steps (80 a second, far beyond any music), for five seconds
            let imps: Vec<(f32, f32, f32, f32)> = if k % 3 == 0 { vec![(rng.next() * 96.0, rng.next() * 64.0, 3.0 + 4.0 * rng.next(), -0.16)] } else { vec![] };
            f.step(0.7, 14.0, 30.0, &imps);
            worst_h = worst_h.max(f.s.iter().map(|s| s[0].abs()).fold(0.0, f32::max));
            worst_v = worst_v.max(f.s.iter().map(|s| s[1].abs()).fold(0.0, f32::max));
        }
        assert!(worst_h <= H_LIMIT + 1e-6 && worst_v <= V_LIMIT + 1e-4 && f.energy().is_finite(), "worst height {worst_h}, velocity {worst_v}");
        // when it stops, the surface settles: the clamps are not holding something unstable in place
        for _ in 0..(4.0 / STEP_DT) as usize {
            f.step(0.7, 14.0, 30.0, &[]);
        }
        let left = f.s.iter().map(|s| s[0].abs()).fold(0.0, f32::max);
        assert!(left < 0.05, "{left} of height left after four calm seconds");
    }
}
