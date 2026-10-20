use super::*;
use crate::analyzer::CURVE_POINTS;

/// Ferrofluid Orb: a glossy black sphere floating over a reflective floor, with broad elastic lobes under
/// fine spikes. The 192 spikes sit on a Fibonacci spiral from the bottom of the sphere to the top, one per
/// frequency of the waterfall's spectrum, so a rising sweep climbs the orb; each is a spring that the spectrum
/// pulls up to its level and a hit throws out and lets settle. Eleven lobes, one per EQ band, are softer
/// springs that bulge with their band's level and wobble on onsets. Sustained bass makes the whole orb breathe.
/// Each floor LED also anchors a tapered spike whose height follows that EQ band's displayed level, with
/// the same attack and fall as the 3D columns, and a reflection in the polished floor.
///
/// A small offscreen pass (`ferro_height.wgsl`) turns the springs into a height map and its gradient, and the
/// mesh (`ferro.wgsl`) is displaced by it: rasterised geometry, no ray marching. The springs, the lobes and the
/// sites are plain Rust below (`Body`, `fib_site`), unit-tested.
pub struct Ferrofluid {
    height: wgpu::RenderPipeline,
    orb: wgpu::RenderPipeline,
    refl: wgpu::RenderPipeline,
    floor: wgpu::RenderPipeline,
    floor_spikes: wgpu::RenderPipeline,
    floor_spikes_refl: wgpu::RenderPipeline,
    ubuf: wgpu::Buffer,
    bg_height: wgpu::BindGroup,
    bg_draw: wgpu::BindGroup,
    hmap: wgpu::TextureView,
    _tex: wgpu::Texture,
    body: Body,
    sites: Sites,
    last_frame: u64,
    last_time: f32,
}

/// Mirrors `OrbU` in `ferro_common.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct OrbU {
    a: [f32; 4],
    b: [f32; 4],
    c: [f32; 4],
    sites: [[f32; 4]; SPIKES],
    lobes: [[f32; 4]; LOBES],
}

const SPIKES: usize = CURVE_POINTS;
const LOBES: usize = 11;
/// Size of the height map (a longitude / latitude image of the sphere) and of the mesh that samples it.
const MAP: [u32; 2] = [256, 128];
const MESH: [u32; 2] = [384, 192];
const ORB_VERTS: u32 = MESH[0] * MESH[1] * 6;
/// The mirror image uses every second vertex of the mesh.
const REFL_VERTS: u32 = MESH[0] / 2 * (MESH[1] / 2) * 6;
/// Matches FLOOR_SPIKE_SIDES and FLOOR_SPIKE_ROWS in ferro.wgsl.
const FLOOR_SPIKE_VERTS: u32 = 32 * 24 * 6;

/// The sphere's radius and the height of its centre over the floor, in world units, and the radius of the ring of
/// LEDs it floats over.
const RADIUS: f32 = 1.0;
const HOVER: f32 = 2.0;
const RING_RADIUS: f32 = 2.0;
/// The tallest a spike gets at Spikes = 1, as a fraction of the radius, and how far a spring may overshoot it
/// (or dip below the surface) before the shape is held.
const SPIKE_MAX: f32 = 0.40;
const SPIKE_HI: f32 = 1.30;
const SPIKE_LO: f32 = -0.10;
/// The biggest a lobe gets, as a fraction of the radius.
const LOBE_MAX: f32 = 0.16;
/// Half the width of a spike's base (radians). The 192 sites are about 0.256 rad apart, so neighbours' bases just
/// meet, as in a real ferrofluid's hexagonal lattice of spikes.
const SPIKE_HALF_WIDTH: f32 = 0.17;
/// How fast a lobe's bump falls off: exp(-kappa (1 - cos alpha)), about 49 degrees to 1/e.
const LOBE_KAPPA: f32 = 2.8;
const GOLDEN: f32 = 2.399_963_2;
/// A scene that was not updated for this many frames starts again from a calm orb.
const STALE_FRAMES: u64 = 20;

// ----------------------------------------------------------------------------------- the sites

/// Direction of site `i` of `n` on a Fibonacci spiral: index 0 at the bottom, `n - 1` at the top (y is up).
fn fib_site(i: usize, n: usize, phase: f32) -> [f32; 3] {
    let y = -1.0 + (2.0 * i as f32 + 1.0) / n as f32;
    let r = (1.0 - y * y).max(0.0).sqrt();
    let a = i as f32 * GOLDEN + phase;
    [r * a.sin(), y, r * a.cos()]
}

/// Where the spikes and the lobes sit (fixed).
struct Sites {
    spike: [[f32; 3]; SPIKES],
    lobe: [[f32; 3]; LOBES],
}

impl Sites {
    fn new() -> Sites {
        Sites {
            spike: std::array::from_fn(|i| fib_site(i, SPIKES, 0.0)),
            // turned a little, so the lobes do not line up with the spikes
            lobe: std::array::from_fn(|i| fib_site(i, LOBES, 0.9)),
        }
    }
}

// ------------------------------------------------------------------------------------- the body

/// The springs' fixed step, and the most steps one frame may run (a stall runs slower than real time).
const STEP: f32 = 1.0 / 240.0;
const MAX_STEPS: u32 = 12;

const SPIKE_HZ: f32 = 7.0;
const LOBE_HZ: f32 = 2.0;
const BREATH_HZ: f32 = 1.1;
const HOP_HZ: f32 = 2.2;
/// Force a rise (1 = the music's hardest recent hit) puts on a spike, and the rise below which nothing happens.
const HIT_FORCE: f32 = 2400.0;
const HIT_MIN: f32 = 0.22;
/// Force an onset envelope puts on a lobe and on the whole orb's hop.
const LOBE_KICK: f32 = 9.5;
const HOP_KICK: f32 = 38.0;
const HOP_MAX: f32 = 0.10;
/// How far sustained bass makes the whole orb swell, as a fraction of the radius.
const BREATH_MAX: f32 = 0.04;
/// Light rig turn rate at Motion = 1 (radians a second), slowest in silence.
const RIG_SPEED: f32 = 0.05;

/// The state of the springs.
struct Body {
    spike: [f32; SPIKES],
    spike_v: [f32; SPIKES],
    lobe: [f32; LOBES],
    lobe_v: [f32; LOBES],
    breath: f32,
    breath_v: f32,
    hop: f32,
    hop_v: f32,
    rig: f32,
    clock: f32,
    acc: f32,
}

fn smooth01(x: f32, a: f32, b: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// One semi-implicit Euler step of a damped spring pulled toward `target` with an extra `force`.
fn spring(x: &mut f32, v: &mut f32, target: f32, force: f32, hz: f32, zeta: f32, dt: f32) {
    let w = std::f32::consts::TAU * hz;
    *v += (w * w * (target - *x) - 2.0 * zeta * w * *v + force) * dt;
    *x += *v * dt;
}

/// The height a spike aims for at a displayed level (0..1), while there is a signal: every site carries a short
/// spike (the field is on), and the loud ones rise out of that to the full height, smoothly.
fn spike_target(level: f32) -> f32 {
    0.15 + 0.85 * smooth01(level, 0.22, 0.95)
}

impl Body {
    fn new() -> Body {
        Body {
            spike: [0.0; SPIKES],
            spike_v: [0.0; SPIKES],
            lobe: [0.0; LOBES],
            lobe_v: [0.0; LOBES],
            breath: 0.0,
            breath_v: 0.0,
            hop: 0.0,
            hop_v: 0.0,
            rig: 0.0,
            clock: 0.0,
            acc: 0.0,
        }
    }

    /// Back to a calm orb (keeps the light rig where it is, so a restart does not jerk the reflections).
    fn calm(&mut self) {
        let rig = self.rig;
        *self = Body::new();
        self.rig = rig;
    }

    /// Advance by `dt` seconds of real time, in fixed steps, whatever the frame rate.
    fn advance(&mut self, p: &SceneParams, dt: f32) {
        let motion = p.motion.clamp(0.0, 1.5);
        let m = motion.min(1.0);
        let gate = p.gate.clamp(0.0, 1.0);

        self.acc += dt;
        let steps = ((self.acc / STEP).round().max(0.0) as u32).min(MAX_STEPS);
        self.acc = (self.acc - steps as f32 * STEP).clamp(-0.5 * STEP, 0.5 * STEP); // a stall's backlog is dropped
        self.clock += dt;
        self.rig += dt * RIG_SPEED * motion * (0.35 + 0.65 * gate);

        // the spikes: pulled to their level, thrown out by a rise
        let zeta_s = (1.0 - 0.7 * m).max(0.30);
        let mut target = [0.0f32; SPIKES];
        let mut force = [0.0f32; SPIKES];
        for i in 0..SPIKES {
            let level = p.spectrum[i].clamp(0.0, 1.0);
            target[i] = spike_target(level) * gate;
            force[i] = HIT_FORCE * m * gate * (p.rise[i] - HIT_MIN).max(0.0) * (0.4 + 0.6 * level);
        }
        // the lobes: a band louder than the others' average bulges and a quieter one sinks, so the volume holds
        let zeta_l = (1.0 - 0.75 * m).max(0.25);
        let mean = p.levels.iter().sum::<f32>() / LOBES as f32;
        let mut lobe_target = [0.0f32; LOBES];
        let mut lobe_force = [0.0f32; LOBES];
        for k in 0..LOBES {
            let idle = 0.012 * motion * (0.31 * self.clock + 2.17 * k as f32).sin();
            lobe_target[k] = LOBE_MAX * gate * ((p.levels[k] - mean) * 1.8).clamp(-1.0, 1.0) + idle;
            lobe_force[k] = LOBE_KICK * m * onset_for(k, &p.fx) * gate;
        }
        let breath_target = BREATH_MAX * m * p.sus[0].clamp(0.0, 1.2);
        let hop_target = 0.025 * m * (0.55 * self.clock).sin();
        let hop_force = HOP_KICK * m * p.fx[0].clamp(0.0, 1.5) * gate;

        for _ in 0..steps {
            for i in 0..SPIKES {
                spring(&mut self.spike[i], &mut self.spike_v[i], target[i], force[i], SPIKE_HZ, zeta_s, STEP);
                if self.spike[i] > SPIKE_HI || self.spike[i] < SPIKE_LO {
                    self.spike[i] = self.spike[i].clamp(SPIKE_LO, SPIKE_HI);
                    self.spike_v[i] = 0.0;
                }
            }
            for k in 0..LOBES {
                spring(&mut self.lobe[k], &mut self.lobe_v[k], lobe_target[k], lobe_force[k], LOBE_HZ, zeta_l, STEP);
                self.lobe[k] = self.lobe[k].clamp(-1.5 * LOBE_MAX, 1.5 * LOBE_MAX);
            }
            spring(&mut self.breath, &mut self.breath_v, breath_target, 0.0, BREATH_HZ, 0.7, STEP);
            spring(&mut self.hop, &mut self.hop_v, hop_target, hop_force, HOP_HZ, 0.35, STEP);
            self.hop = self.hop.clamp(-HOP_MAX, HOP_MAX);
        }
    }

    /// What the shaders read.
    fn pack(&self, sites: &Sites, spikes: f32) -> OrbU {
        let amp = SPIKE_MAX * spikes.clamp(0.2, 1.8);
        OrbU {
            a: [RADIUS, HOVER + self.hop, 1.0 + self.breath, self.rig],
            b: [MAP[0] as f32, MAP[1] as f32, SPIKE_HALF_WIDTH, LOBE_KAPPA],
            c: [RING_RADIUS, SPIKE_MAX, 0.0, 0.0],
            sites: std::array::from_fn(|i| {
                let d = sites.spike[i];
                [d[0], d[1], d[2], amp * self.spike[i].clamp(SPIKE_LO, SPIKE_HI).max(0.0)]
            }),
            lobes: std::array::from_fn(|k| {
                let d = sites.lobe[k];
                [d[0], d[1], d[2], self.lobe[k]]
            }),
        }
    }
}

// ----------------------------------------------------------------------------------- the scene

impl Ferrofluid {
    pub fn new(ctx: &Ctx) -> Ferrofluid {
        let dev = ctx.device;
        let common = include_str!("ferro_common.wgsl");
        let height_sh = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ferro_height"),
            source: wgpu::ShaderSource::Wgsl(format!("{common}\n{}", include_str!("ferro_height.wgsl")).into()),
        });
        let draw_sh = ctx.module("ferro", &format!("{common}\n{}", include_str!("ferro.wgsl")));

        let uniform = |visibility| wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let height_bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ferro_height_bgl"),
            entries: &[uniform(wgpu::ShaderStages::FRAGMENT)],
        });
        let both = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let draw_bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ferro_draw_bgl"),
            entries: &[
                uniform(both),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: both,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: both, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let height_layout = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&height_bgl)], immediate_size: 0 });
        let draw_layout = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(ctx.bgl), Some(&draw_bgl)], immediate_size: 0 });

        let fmt = wgpu::TextureFormat::Rgba16Float;
        let height = dev.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ferro_height"),
            layout: Some(&height_layout),
            vertex: wgpu::VertexState { module: &height_sh, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &height_sh,
                entry_point: Some("fs_height"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: fmt, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let tex = dev.create_texture(&wgpu::TextureDescriptor {
            label: Some("ferro_map"),
            size: wgpu::Extent3d { width: MAP[0], height: MAP[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: fmt,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let hmap = tex.create_view(&Default::default());
        // around the sphere the map wraps; at the poles it stops
        let sampler = dev.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let ubuf = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ferro_u"),
            size: std::mem::size_of::<OrbU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bg_height = dev.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ferro_height_bg"),
            layout: &height_bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }],
        });
        let bg_draw = dev.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ferro_draw_bg"),
            layout: &draw_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&hmap) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });

        let floor = Pipe { depth_write: false, always: false, blend: None, msaa: true };
        let mirror = Pipe { depth_write: false, always: false, blend: ADDITIVE, msaa: true };
        Ferrofluid {
            height,
            // the mirror image flips the winding, so it culls the other face
            orb: ctx.pipeline_cull(&draw_layout, &draw_sh, "ferro_orb", "vs_orb", "fs_orb", Pipe::OPAQUE, Some(wgpu::Face::Back)),
            refl: ctx.pipeline_cull(&draw_layout, &draw_sh, "ferro_refl", "vs_orb_refl", "fs_orb_refl", mirror, Some(wgpu::Face::Front)),
            floor: ctx.pipeline_with(&draw_layout, &draw_sh, "ferro_floor", "vs_floor", "fs_floor", floor),
            floor_spikes: ctx.pipeline_cull(&draw_layout, &draw_sh, "ferro_floor_spikes", "vs_floor_spike", "fs_floor_spike", Pipe::OPAQUE, Some(wgpu::Face::Back)),
            floor_spikes_refl: ctx.pipeline_cull(&draw_layout, &draw_sh, "ferro_floor_spikes_refl", "vs_floor_spike_refl", "fs_floor_spike_refl", mirror, Some(wgpu::Face::Front)),
            ubuf,
            bg_height,
            bg_draw,
            hmap,
            _tex: tex,
            body: Body::new(),
            sites: Sites::new(),
            last_frame: 0,
            last_time: 0.0,
        }
    }
}

impl Scene for Ferrofluid {
    fn update(&mut self, _device: &wgpu::Device, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, f: &Frame) {
        if f.frame_no == self.last_frame {
            return; // already advanced for this frame (a crossfade between two looks of this scene)
        }
        let continuous = self.last_frame != 0 && f.frame_no <= self.last_frame + STALE_FRAMES;
        let dt = if continuous { (f.p.time - self.last_time).clamp(0.0, 0.1) } else { 1.0 / 60.0 };
        if !continuous {
            self.body.calm(); // a long absence: whatever the springs were doing is stale
        }
        self.last_frame = f.frame_no;
        self.last_time = f.p.time;

        self.body.advance(f.p, dt);
        let u = self.body.pack(&self.sites, f.p.spikes);
        queue.write_buffer(&self.ubuf, 0, bytemuck::bytes_of(&u));

        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ferro_height"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.hmap,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: f.ts.clone(),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.height);
        pass.set_bind_group(0, &self.bg_height, &[]);
        pass.draw(0..3, 0..1);
    }

    fn uses_update(&self, _p: &SceneParams) -> bool {
        true
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, _f: &Frame) {
        pass.set_bind_group(1, &self.bg_draw, &[]);
        // Opaque fluid first: the depth test hides the floor and reflections behind the orb and spikes.
        pass.set_pipeline(&self.orb);
        pass.draw(0..ORB_VERTS, 0..1);
        pass.set_pipeline(&self.floor_spikes);
        pass.draw(0..FLOOR_SPIKE_VERTS, 0..11);
        pass.set_pipeline(&self.floor);
        pass.draw(0..6, 0..1);
        pass.set_pipeline(&self.refl);
        pass.draw(0..REFL_VERTS, 0..1);
        pass.set_pipeline(&self.floor_spikes_refl);
        pass.draw(0..FLOOR_SPIKE_VERTS, 0..11);
    }

    fn camera(&self, aspect: f32, p: &SceneParams) -> (Mat4, Vec3) {
        // a tall window backs off so the sphere and its spikes keep fitting
        let pull = (0.66 / aspect).max(1.0);
        let o = Orbit { eye: Vec3::new(0.0, 1.7, 7.6 * pull), target: Vec3::new(0.0, 0.9, 0.0), fov_deg: 44.0 };
        orbit(&o, aspect, &p.cam)
    }

    fn reset(&mut self, _seed: u64) {
        self.body = Body::new();
        self.last_frame = 0;
    }
}

/// The onset envelope that reaches band `i`: the low envelope for the bass bands, the mid one in the
/// middle and the high one at the top, blended over the seams.
fn onset_for(i: usize, fx: &[f32; 4]) -> f32 {
    let i = i as f32;
    let lo = 1.0 - ((i - 2.0) / 2.0).clamp(0.0, 1.0);
    let hi = ((i - 6.0) / 2.0).clamp(0.0, 1.0);
    lo * fx[0] + (1.0 - lo - hi).max(0.0) * fx[1] + hi * fx[2]
}


#[cfg(test)]
mod tests {
    use super::*;

    fn params(level: f32, rise: f32) -> SceneParams {
        let mut p = SceneParams::for_tests();
        p.spectrum = [level; SPIKES];
        p.rise = [rise; SPIKES];
        p.levels = [level; LOBES];
        p
    }

    /// Runs `secs` of the same input at `fps` and returns the body.
    fn run(body: &mut Body, p: &SceneParams, secs: f32, fps: f32) {
        for _ in 0..(secs * fps).round() as usize {
            body.advance(p, 1.0 / fps);
        }
    }

    #[test]
    fn sites_climb_from_bass_to_treble_and_are_spread_evenly() {
        let s = Sites::new();
        for i in 1..SPIKES {
            assert!(s.spike[i][1] > s.spike[i - 1][1], "site {i} is not above {}", i - 1);
        }
        for d in s.spike.iter().chain(&s.lobe) {
            assert!((d[0] * d[0] + d[1] * d[1] + d[2] * d[2] - 1.0).abs() < 1e-4);
        }
        // nearest neighbours: about 0.256 rad apart (sqrt(4 pi / 192)); none much closer, and the bases meet
        let mut nearest = [f32::MAX; SPIKES];
        for i in 0..SPIKES {
            for j in 0..SPIKES {
                if i != j {
                    let a = s.spike[i];
                    let b = s.spike[j];
                    let c = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]).clamp(-1.0, 1.0);
                    nearest[i] = nearest[i].min(c.acos());
                }
            }
        }
        let (lo, hi) = nearest.iter().fold((f32::MAX, 0.0f32), |(l, h), d| (l.min(*d), h.max(*d)));
        assert!(lo > 0.17 && hi < 0.33, "nearest-neighbour distance runs {lo} to {hi}");
        assert!(SPIKE_HALF_WIDTH * 2.0 > hi, "a spike's base must reach its neighbours");
        // the lowest site is near the bottom pole and the highest near the top
        assert!(s.spike[0][1] < -0.98 && s.spike[SPIKES - 1][1] > 0.98);
    }

    #[test]
    fn spikes_settle_at_their_level() {
        let mut b = Body::new();
        run(&mut b, &params(0.7, 0.0), 2.0, 120.0);
        let want = spike_target(0.7);
        assert!((b.spike[40] - want).abs() < 0.01, "spike at {} wanted {want}", b.spike[40]);
        assert!(b.spike[40] > 0.4 && b.spike[40] < 0.9);
    }

    #[test]
    fn quiet_bins_carry_only_short_spikes() {
        let mut b = Body::new();
        run(&mut b, &params(0.1, 0.0), 2.0, 120.0);
        assert!(b.spike.iter().all(|x| (x - 0.15).abs() < 0.01), "{:?}", &b.spike[..4]);
    }

    #[test]
    fn a_hit_throws_a_spike_out_and_it_settles() {
        let mut b = Body::new();
        run(&mut b, &params(0.5, 0.0), 1.5, 120.0);
        let before = b.spike[100];
        // a rise of 1 (the hardest recent hit) for 50 ms
        let mut hit = params(0.5, 0.0);
        hit.rise[100] = 1.0;
        let mut peak = 0.0f32;
        for _ in 0..6 {
            b.advance(&hit, 1.0 / 120.0);
            peak = peak.max(b.spike[100]);
        }
        let calm = params(0.5, 0.0);
        for _ in 0..60 {
            b.advance(&calm, 1.0 / 120.0);
            peak = peak.max(b.spike[100]);
        }
        assert!(peak > before + 0.25, "a hit only took the spike from {before} to {peak}");
        run(&mut b, &calm, 2.0, 120.0);
        assert!((b.spike[100] - before).abs() < 0.02, "the spike did not settle back: {} vs {before}", b.spike[100]);
    }

    #[test]
    fn motion_zero_has_no_overshoot_and_no_hits() {
        let mut b = Body::new();
        let mut p = params(0.8, 1.0);
        p.motion = 0.0;
        let want = spike_target(0.8);
        let mut peak = 0.0f32;
        for _ in 0..480 {
            b.advance(&p, 1.0 / 120.0);
            peak = peak.max(b.spike[10]);
        }
        assert!(peak <= want + 0.005, "overshoot {peak} over {want} with Motion 0");
        assert!(b.rig == 0.0 && b.hop.abs() < 1e-4 && b.breath.abs() < 1e-4);
    }

    #[test]
    fn the_shape_stays_bounded_under_the_hardest_drive() {
        let mut b = Body::new();
        let mut p = params(1.0, 1.5);
        p.motion = 1.5;
        p.fx = [1.5, 1.5, 1.5, 0.0];
        p.sus = [1.2; 4];
        p.levels = std::array::from_fn(|i| if i % 2 == 0 { 1.0 } else { 0.0 });
        for _ in 0..2400 {
            b.advance(&p, 1.0 / 120.0);
            assert!(b.spike.iter().all(|x| (SPIKE_LO..=SPIKE_HI).contains(x)));
            assert!(b.lobe.iter().all(|x| x.abs() <= 1.5 * LOBE_MAX + 1e-4));
            assert!(b.hop.abs() <= HOP_MAX + 1e-4 && b.breath.abs() < 0.2);
        }
        // and the biggest height that reaches the shader leaves the sphere clear of the floor
        let u = b.pack(&Sites::new(), 1.8);
        let tallest = u.sites.iter().map(|s| s[3]).fold(0.0, f32::max);
        assert!(tallest <= SPIKE_MAX * 1.8 * SPIKE_HI + 1e-4);
        // at the designed Spikes the lowest point the shape can reach (everything at its limit) still clears the floor
        assert!(HOVER - HOP_MAX - RADIUS * (1.0 + BREATH_MAX) - 1.5 * LOBE_MAX - SPIKE_MAX * SPIKE_HI > 0.0, "the lowest spike could reach the floor");
    }

    #[test]
    fn the_motion_does_not_depend_on_the_frame_rate() {
        let p = params(0.6, 0.0);
        // a step in level at t = 0 and a hit lasting 1/30 s from t = 0.5 s; sample after 0.2 s and 1.5 s
        let trace = |fps: f32| {
            let mut b = Body::new();
            let mut out = Vec::new();
            let frames = (1.5 * fps).round() as usize;
            for k in 0..frames {
                let t = k as f32 / fps;
                let mut q = p.clone();
                if t >= 0.5 - 1e-4 && t < 0.5 + 1.0 / 30.0 - 1e-4 {
                    q.rise[20] = 1.0;
                    q.fx = [1.0, 0.0, 0.0, 0.0];
                }
                b.advance(&q, 1.0 / fps);
                let elapsed = (k + 1) as f32 / fps;
                if (elapsed - 0.6).abs() < 0.5 / fps || k + 1 == frames {
                    out.push((b.spike[20], b.lobe[0], b.hop));
                }
            }
            out
        };
        let reference = trace(240.0);
        assert!(reference[0].0 > 0.5 && reference.len() == 2, "the hit did nothing: {reference:?}");
        for fps in [30.0, 60.0, 120.0] {
            let got = trace(fps);
            assert_eq!(got.len(), reference.len());
            for (g, r) in got.iter().zip(&reference) {
                assert!((g.0 - r.0).abs() < 0.03 && (g.1 - r.1).abs() < 0.01 && (g.2 - r.2).abs() < 0.01, "{fps} fps: {g:?} vs {r:?}");
            }
        }
    }

    #[test]
    fn silence_lets_the_orb_settle_to_a_smooth_sphere() {
        let mut b = Body::new();
        let mut loud = params(0.9, 0.8);
        loud.fx = [1.0, 0.6, 0.4, 0.0];
        run(&mut b, &loud, 3.0, 120.0);
        assert!(b.spike.iter().any(|x| *x > 0.5));
        let mut quiet = params(0.0, 0.0);
        quiet.gate = 0.0;
        quiet.sus = [0.0; 4];
        run(&mut b, &quiet, 4.0, 120.0);
        assert!(b.spike.iter().all(|x| x.abs() < 0.01), "spikes left after 4 s of silence");
        assert!(b.lobe.iter().all(|x| x.abs() < 0.03), "lobes left: {:?}", b.lobe);
        assert!(b.breath.abs() < 0.005);
    }

    #[test]
    fn lobes_hold_the_volume() {
        let mut b = Body::new();
        let mut p = params(0.5, 0.0);
        p.levels = std::array::from_fn(|i| i as f32 / 10.0);
        p.motion = 0.0;
        run(&mut b, &p, 3.0, 120.0);
        let mean: f32 = b.lobe.iter().sum::<f32>() / LOBES as f32;
        assert!(mean.abs() < 0.02, "the lobes add volume: {mean}");
        assert!(b.lobe[10] > 0.05 && b.lobe[0] < -0.05, "loud bands bulge and quiet ones sink: {:?}", b.lobe);
    }
}
