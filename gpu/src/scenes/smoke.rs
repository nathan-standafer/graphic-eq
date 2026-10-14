use super::*;

/// Eleven nozzles along the floor, one per EQ band, each blowing smoke upward. A louder band is a stronger
/// jet, a drum hit is a puff that rolls up into a vortex head, and the smoke is a real fluid: a 2D
/// stable-fluids solver (vorticity confinement, a pressure projection, semi-Lagrangian advection) in
/// fragment passes on a grid of about four-pixel cells, with the dye advected at twice that resolution.
///
/// The solver runs on a fixed timestep with an accumulator, so it behaves the same at 60 and 120 fps. A
/// shading pass at dye resolution turns the dye into a light field (lamps on the floor, a bump normal from
/// the density gradient, self-shadowing by the smoke below); the screen pass colours it from the theme
/// and draws the floor strip with a lamp in every nozzle.
pub struct Smoke {
    curl: wgpu::RenderPipeline,
    force: wgpu::RenderPipeline,
    div: wgpu::RenderPipeline,
    jacobi0: wgpu::RenderPipeline,
    jacobi: wgpu::RenderPipeline,
    grad: wgpu::RenderPipeline,
    adv_vel_a: wgpu::RenderPipeline,
    adv_vel_b: wgpu::RenderPipeline,
    adv_dye_a: wgpu::RenderPipeline,
    adv_dye_b: wgpu::RenderPipeline,
    shade: wgpu::RenderPipeline,
    screen: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    ubuf: wgpu::Buffer,
    dummy: wgpu::TextureView,
    _dummy_tex: wgpu::Texture,
    sim: Option<Sim>,
    /// Which texture of each ping-pong pair holds the current state.
    vel: usize,
    pres: usize,
    dye: usize,
    acc: f32,
    sim_time: f32,
    seed: f32,
    iters: u32,
    last_frame: u64,
    last_time: f32,
    clear: bool,
}

/// Textures and bind groups for one window size.
struct Sim {
    size: [u32; 2],
    geo: Geo,
    vel: [wgpu::TextureView; 2],
    pres: [wgpu::TextureView; 2],
    /// The plain advection result of the velocity, which the MacCormack correction starts from.
    vel_hat: wgpu::TextureView,
    curl: wgpu::TextureView,
    div: wgpu::TextureView,
    dye: [wgpu::TextureView; 2],
    /// The first (plain) advection result the MacCormack correction starts from.
    hat: wgpu::TextureView,
    lit: wgpu::TextureView,
    bg_curl: [wgpu::BindGroup; 2],
    bg_force: [[wgpu::BindGroup; 2]; 2],
    bg_div: [wgpu::BindGroup; 2],
    bg_jac: [wgpu::BindGroup; 2],
    bg_grad: [[wgpu::BindGroup; 2]; 2],
    bg_adv_vel: [wgpu::BindGroup; 2],
    bg_adv_vel_fix: [wgpu::BindGroup; 2],
    bg_adv_dye: [[wgpu::BindGroup; 2]; 2],
    bg_adv_fix: [[wgpu::BindGroup; 2]; 2],
    bg_shade: [wgpu::BindGroup; 2],
    bg_screen: wgpu::BindGroup,
    _tex: Vec<wgpu::Texture>,
}

/// Mirrors `SimU` in `smoke_common.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SimU {
    vel: [f32; 4],
    dye: [f32; 4],
    scr: [f32; 4],
    step: [f32; 4],
    force: [f32; 4],
    misc: [f32; 4],
    extra: [f32; 4],
    diff: [f32; 4],
    look: [f32; 4],
    noz: [[f32; 4]; 11],
    inj: [[f32; 4]; 11],
}

/// Sizes of the textures for one window.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Geo {
    size: [u32; 2],
    grid: [u32; 2],
    dye: [u32; 2],
}

/// The solver's fixed timestep, and the most steps one frame may run (a slower frame runs the
/// simulation slower than real time instead of spiralling).
const SIM_DT: f32 = 1.0 / 120.0;
const MAX_STEPS: u32 = 4;
/// Screen pixels per simulation cell, and dye texels per cell (per axis).
const CELL_PX: f32 = 4.0;
const DYE_SCALE: u32 = 2;
/// Fewest grid rows: a smaller window shows a simulation of this size, enlarged, rather than a coarser (and
/// much blurrier and fatter) one.
const MIN_ROWS: u32 = 192;
/// Jacobi iterations of the pressure solve (the pressure of the previous step is the starting guess).
const ITERS: u32 = 18;
/// A scene that was not updated for this many frames starts again from clear air.
const STALE_FRAMES: u64 = 30;
/// Full-strength jet speed at the lowest and the highest band, in screen heights per second.
const NOMINAL: (f32, f32) = (0.64, 1.10);

// ------------------------------------------------------------------------------------ the drive

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// How hard a band blows (0..1): its displayed level above a noise floor, shaped so a quiet band gives
/// faint wisps. A closed signal gate blows nothing.
fn strength(level: f32, gate: f32) -> f32 {
    ((level - 0.10) / 0.90).clamp(0.0, 1.0).powf(1.4) * gate.clamp(0.0, 1.0)
}

/// The onset envelope that reaches band `i`: the low envelope for the bass bands, the mid one in the
/// middle and the high one at the top, blended over the seams.
pub(super) fn onset_for(i: usize, fx: &[f32; 4]) -> f32 {
    let i = i as f32;
    let lo = 1.0 - ((i - 2.0) / 2.0).clamp(0.0, 1.0);
    let hi = ((i - 6.0) / 2.0).clamp(0.0, 1.0);
    lo * fx[0] + (1.0 - lo - hi).max(0.0) * fx[1] + hi * fx[2]
}

/// Grid, dye and window sizes for a window: cells are square, so the physics does not depend on the aspect.
fn geometry(size: [u32; 2]) -> Geo {
    let gh = ((size[1] as f32 / CELL_PX).round() as u32).clamp(MIN_ROWS, 480);
    let gw = ((size[0] as f32 * gh as f32 / size[1].max(1) as f32).round() as u32).max(16);
    Geo { size, grid: [gw, gh], dye: [gw * DYE_SCALE, gh * DYE_SCALE] }
}

/// Everything the passes read this frame: the nozzles (place, width, speed, lamp), what they inject, and
/// the solver and look parameters. Lengths are in cells, speeds in cells per second, and the sizes scale with
/// the grid height so the picture looks the same at any resolution.
fn drive(g: &Geo, p: &SceneParams, sim_time: f32, seed: f32) -> SimU {
    let (gw, gh) = (g.grid[0] as f32, g.grid[1] as f32);
    let spacing = (p.nozzle_x[10] - p.nozzle_x[0]) / 10.0 * gw; // cells between neighbouring nozzles
    let thick = p.smoke.clamp(0.3, 2.0);
    let motion = p.motion.clamp(0.0, 1.5);
    let gate = p.gate.clamp(0.0, 1.0);

    let mut noz = [[0.0f32; 4]; 11];
    let mut inj = [[0.0f32; 4]; 11];
    for i in 0..11 {
        let t = i as f32 / 10.0;
        let s = strength(p.levels[i], gate);
        let puff = (onset_for(i, &p.fx) * p.flash).clamp(0.0, 1.6);
        // bass nozzles are wide and slow, treble nozzles narrow and fast; a puff flares the nozzle
        let sigma = (spacing * lerp(0.13, 0.05, t) * (1.0 + 0.6 * puff)).clamp(2.2, 0.10 * gh);
        let nominal = lerp(NOMINAL.0, NOMINAL.1, t) * gh;
        let live = smooth01(s + 0.5 * puff, 0.0, 0.06);
        let speed = nominal * (0.30 + 0.70 * s) * live + gh * puff * lerp(2.4, 1.7, t);
        noz[i] = [p.nozzle_x[i] * gw, sigma, speed, (0.12 + 0.88 * s + 0.6 * puff).min(1.6)];
        inj[i] = [
            0.15 * nominal * thick * (s.powf(0.9) + 1.4 * puff),
            0.07 * nominal * (0.7 * s.powf(0.9) + 1.2 * puff),
            0.05 * nominal * puff,
            t,
        ];
    }
    SimU {
        vel: [gw, gh, 1.0 / gw, 1.0 / gh],
        dye: [g.dye[0] as f32, g.dye[1] as f32, 1.0 / g.dye[0] as f32, 1.0 / g.dye[1] as f32],
        scr: [g.size[0] as f32, g.size[1] as f32, g.size[0] as f32 / gw, (0.010 * g.size[1] as f32).max(6.0)],
        step: [SIM_DT, 0.40, 0.50 + 0.9 * (1.0 - gate), 2.4],
        force: [0.006 * gh * motion, 0.5 * gh, 0.9, 0.04 * motion],
        misc: [sim_time, seed, 3.0, 0.10],
        extra: [2.6 * gh, 45.0, 0.0, 40.0],
        // smoothing in cells would blur a small window far more than a large one; scale it with the grid
        diff: [0.12 * (gh / 360.0).min(1.0), 0.10 * (gh / 360.0).min(1.0), 0.0, 0.0],
        look: [1.5, 26.0, 0.25, 0.05],
        noz,
        inj,
    }
}

fn smooth01(x: f32, a: f32, b: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How many fixed steps a frame of `dt` seconds runs. Rounding (rather than flooring) keeps a steady
/// 120 Hz at exactly one step per frame, instead of the 0 and 2 that clock jitter would alternate between.
fn steps_for(acc: &mut f32, dt: f32) -> u32 {
    *acc += dt;
    let n = ((*acc / SIM_DT).round().max(0.0) as u32).min(MAX_STEPS);
    *acc -= n as f32 * SIM_DT;
    *acc = acc.clamp(-0.5 * SIM_DT, 0.5 * SIM_DT); // after a stall the backlog is dropped, not repaid
    n
}

// ------------------------------------------------------------------------------------ the scene

fn texture(device: &wgpu::Device, label: &str, size: [u32; 2], format: wgpu::TextureFormat) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: size[0].max(1), height: size[1].max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

/// Hands out timestamp writes so the scene's whole update (every solver pass and the shading) is one
/// measured span: the first pass records the start and the last one the end.
pub(super) struct Stamper<'a> {
    pub(super) ts: Option<&'a wgpu::RenderPassTimestampWrites<'a>>,
    pub(super) first: bool,
}

impl<'a> Stamper<'a> {
    pub(super) fn next(&mut self, last: bool) -> Option<wgpu::RenderPassTimestampWrites<'a>> {
        let t = self.ts?;
        let begin = if self.first { t.beginning_of_pass_write_index } else { None };
        let end = if last { t.end_of_pass_write_index } else { None };
        self.first = false;
        (begin.is_some() || end.is_some())
            .then(|| wgpu::RenderPassTimestampWrites { query_set: t.query_set, beginning_of_pass_write_index: begin, end_of_pass_write_index: end })
    }
}

pub(super) fn run_pass(enc: &mut wgpu::CommandEncoder, label: &str, pipe: &wgpu::RenderPipeline, bg: &wgpu::BindGroup, dst: &wgpu::TextureView, ts: Option<wgpu::RenderPassTimestampWrites>) {
    let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: dst,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: ts,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipe);
    pass.set_bind_group(0, bg, &[]);
    pass.draw(0..3, 0..1);
}

impl Smoke {
    pub fn new(ctx: &Ctx) -> Smoke {
        let dev = ctx.device;
        let common = include_str!("smoke_common.wgsl");
        let sim_sh = dev.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("smoke_sim"),
            source: wgpu::ShaderSource::Wgsl(format!("{common}\n{}", include_str!("smoke_sim.wgsl")).into()),
        });
        let screen_sh = ctx.module("smoke", &format!("{common}\n{}", include_str!("smoke.wgsl")));

        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
            count: None,
        };
        let bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("smoke_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                tex_entry(2),
                tex_entry(3),
                tex_entry(4),
            ],
        });
        let sim_layout = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let screen_layout = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(ctx.bgl), Some(&bgl)], immediate_size: 0 });

        let offscreen = |label: &str, fs: &str, format: wgpu::TextureFormat| {
            dev.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&sim_layout),
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
        use wgpu::TextureFormat::{R16Float, Rg16Float, Rgba16Float};
        let screen_pipe = Pipe { depth_write: false, always: true, blend: None, msaa: false };

        let sampler = dev.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let ubuf = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("smoke_u"),
            size: std::mem::size_of::<SimU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let dummy_tex = texture(dev, "smoke_dummy", [1, 1], Rgba16Float);
        let dummy = dummy_tex.create_view(&Default::default());
        let iters = std::env::var("GEQ_SMOKE_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(ITERS);

        Smoke {
            curl: offscreen("smoke_curl", "fs_curl", R16Float),
            force: offscreen("smoke_force", "fs_force", Rg16Float),
            div: offscreen("smoke_div", "fs_div", R16Float),
            jacobi0: offscreen("smoke_jacobi0", "fs_jacobi0", R16Float),
            jacobi: offscreen("smoke_jacobi", "fs_jacobi", R16Float),
            grad: offscreen("smoke_grad", "fs_grad", Rg16Float),
            adv_vel_a: offscreen("smoke_adv_vel_a", "fs_adv_vel_a", Rg16Float),
            adv_vel_b: offscreen("smoke_adv_vel_b", "fs_adv_vel_b", Rg16Float),
            adv_dye_a: offscreen("smoke_adv_dye_a", "fs_adv_dye_a", Rgba16Float),
            adv_dye_b: offscreen("smoke_adv_dye_b", "fs_adv_dye_b", Rgba16Float),
            shade: offscreen("smoke_shade", "fs_shade", Rgba16Float),
            screen: ctx.pipeline_with(&screen_layout, &screen_sh, "smoke_screen", "vs_smoke", "fs_smoke", screen_pipe),
            bgl, sampler, ubuf, dummy, _dummy_tex: dummy_tex,
            sim: None,
            vel: 0, pres: 0, dye: 0,
            acc: 0.0,
            sim_time: 0.0,
            seed: 1.0,
            iters: iters.max(1),
            last_frame: 0,
            last_time: 0.0,
            clear: true,
        }
    }

    fn bind(&self, device: &wgpu::Device, tex: [&wgpu::TextureView; 3]) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("smoke_bg"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.ubuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(tex[0]) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(tex[1]) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(tex[2]) },
            ],
        })
    }

    fn ensure_sim(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if self.sim.as_ref().map_or(false, |s| s.size == size) {
            return;
        }
        use wgpu::TextureFormat::{R16Float, Rg16Float, Rgba16Float};
        let geo = geometry(size);
        let pair = |label: &str, dims: [u32; 2], fmt| [texture(device, label, dims, fmt), texture(device, label, dims, fmt)];
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let [vel0, vel1] = pair("smoke_vel", geo.grid, Rg16Float);
        let [pres0, pres1] = pair("smoke_pres", geo.grid, R16Float);
        let [dye0, dye1] = pair("smoke_dye", geo.dye, Rgba16Float);
        let curl = texture(device, "smoke_curl", geo.grid, R16Float);
        let div = texture(device, "smoke_div", geo.grid, R16Float);
        let hat = texture(device, "smoke_hat", geo.dye, Rgba16Float);
        let vel_hat = texture(device, "smoke_vel_hat", geo.grid, Rg16Float);
        let lit = texture(device, "smoke_lit", geo.dye, Rgba16Float);
        let (vel, pres, dye) = ([view(&vel0), view(&vel1)], [view(&pres0), view(&pres1)], [view(&dye0), view(&dye1)]);
        let (curl_v, div_v, lit_v, hat_v, vel_hat_v) = (view(&curl), view(&div), view(&lit), view(&hat), view(&vel_hat));
        let d = &self.dummy;
        let b = |t: [&wgpu::TextureView; 3]| self.bind(device, t);
        let both = |f: &dyn Fn(usize) -> wgpu::BindGroup| [f(0), f(1)];
        let sim = Sim {
            size,
            geo,
            bg_curl: both(&|v| b([&vel[v], d, d])),
            bg_force: [both(&|d_| b([&vel[0], &curl_v, &dye[d_]])), both(&|d_| b([&vel[1], &curl_v, &dye[d_]]))],
            bg_div: both(&|v| b([&vel[v], d, d])),
            bg_jac: both(&|p| b([&pres[p], &div_v, d])),
            bg_grad: [both(&|v| b([&pres[0], &vel[v], d])), both(&|v| b([&pres[1], &vel[v], d]))],
            bg_adv_vel: both(&|v| b([&vel[v], d, d])),
            bg_adv_vel_fix: both(&|v| b([&vel[v], &vel_hat_v, d])),
            bg_adv_dye: [both(&|d_| b([&vel[0], &dye[d_], d])), both(&|d_| b([&vel[1], &dye[d_], d]))],
            bg_adv_fix: [both(&|d_| b([&vel[0], &dye[d_], &hat_v])), both(&|d_| b([&vel[1], &dye[d_], &hat_v]))],
            bg_shade: both(&|d_| b([&dye[d_], d, d])),
            bg_screen: b([&lit_v, d, d]),
            vel,
            pres,
            vel_hat: vel_hat_v,
            curl: curl_v,
            div: div_v,
            dye,
            hat: hat_v,
            lit: lit_v,
            _tex: vec![vel0, vel1, pres0, pres1, dye0, dye1, curl, div, hat, vel_hat, lit],
        };
        self.sim = Some(sim);
        self.clear = true;
    }
}

impl Scene for Smoke {
    fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, f: &Frame) {
        self.ensure_sim(device, f.size);
        if f.frame_no == self.last_frame {
            return; // already advanced for this frame (a crossfade between two looks of this scene)
        }
        let continuous = self.last_frame != 0 && f.frame_no <= self.last_frame + STALE_FRAMES;
        let dt = if continuous { (f.p.time - self.last_time).clamp(0.0, 0.1) } else { 1.0 / 60.0 };
        if self.last_frame != 0 && !continuous {
            self.clear = true; // a long absence: the smoke would be stale
        }
        self.last_frame = f.frame_no;
        self.last_time = f.p.time;
        let Some(sim) = &self.sim else { return };

        let steps = if self.clear { 0 } else { steps_for(&mut self.acc, dt) };
        let u = drive(&sim.geo, f.p, self.sim_time, self.seed);
        queue.write_buffer(&self.ubuf, 0, bytemuck::bytes_of(&u));
        self.sim_time += steps as f32 * SIM_DT;

        if std::mem::take(&mut self.clear) {
            self.acc = 0.0;
            self.sim_time = 0.0;
            self.vel = 0;
            self.pres = 0;
            self.dye = 0;
            for v in sim.vel.iter().chain(&sim.pres).chain(&sim.dye) {
                enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("smoke_clear"),
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
        let (mut v, mut p, mut d) = (self.vel, self.pres, self.dye);
        for _ in 0..steps {
            run_pass(enc, "smoke_curl", &self.curl, &sim.bg_curl[v], &sim.curl, stamp.next(false));
            run_pass(enc, "smoke_force", &self.force, &sim.bg_force[v][d], &sim.vel[1 - v], stamp.next(false));
            v = 1 - v;
            run_pass(enc, "smoke_div", &self.div, &sim.bg_div[v], &sim.div, stamp.next(false));
            for k in 0..self.iters {
                let pipe = if k == 0 { &self.jacobi0 } else { &self.jacobi };
                run_pass(enc, "smoke_jacobi", pipe, &sim.bg_jac[p], &sim.pres[1 - p], stamp.next(false));
                p = 1 - p;
            }
            run_pass(enc, "smoke_grad", &self.grad, &sim.bg_grad[p][v], &sim.vel[1 - v], stamp.next(false));
            v = 1 - v;
            run_pass(enc, "smoke_adv_vel_a", &self.adv_vel_a, &sim.bg_adv_vel[v], &sim.vel_hat, stamp.next(false));
            run_pass(enc, "smoke_adv_vel_b", &self.adv_vel_b, &sim.bg_adv_vel_fix[v], &sim.vel[1 - v], stamp.next(false));
            v = 1 - v;
            run_pass(enc, "smoke_adv_dye_a", &self.adv_dye_a, &sim.bg_adv_dye[v][d], &sim.hat, stamp.next(false));
            run_pass(enc, "smoke_adv_dye_b", &self.adv_dye_b, &sim.bg_adv_fix[v][d], &sim.dye[1 - d], stamp.next(false));
            d = 1 - d;
        }
        (self.vel, self.pres, self.dye) = (v, p, d);
        run_pass(enc, "smoke_shade", &self.shade, &sim.bg_shade[d], &sim.lit, stamp.next(true));
    }

    fn uses_update(&self, _p: &SceneParams) -> bool {
        true
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, _f: &Frame) {
        let Some(sim) = &self.sim else { return };
        pass.set_pipeline(&self.screen);
        pass.set_bind_group(1, &sim.bg_screen, &[]);
        pass.draw(0..3, 0..1);
    }

    fn wants_msaa(&self) -> bool {
        false // soft smoke, with analytic lamps
    }

    fn camera(&self, _aspect: f32, _p: &SceneParams) -> (Mat4, Vec3) {
        (Mat4::IDENTITY, Vec3::ZERO) // a flat picture
    }

    fn reset(&mut self, seed: u64) {
        self.clear = true;
        self.seed = (seed % 997) as f32 * 0.37 + 1.0;
        self.last_frame = 0;
    }

    fn resize(&mut self, _device: &wgpu::Device, _size: [u32; 2]) {
        self.sim = None; // recreated, in clear air, on the next frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> SceneParams {
        SceneParams {
            time: 0.0,
            mode: 0,
            theme: 2,
            exposure: 1.0,
            flash: 0.8,
            motion: 0.6,
            gate: 1.0,
            levels: [0.0; 11],
            peaks: [0.0; 11],
            fx: [0.0; 4],
            sus: [0.0; 4],
            rings: [[0.0; 4]; 8],
            cam: CamState::default(),
            row_frac: 0.0,
            persist: 0.25,
            sparks: true,
            density: 1.0,
            gradient: 1.0,
            peak_hits: [0.0; 11],
            smoke: 1.0,
            nozzle_x: std::array::from_fn(|i| (i as f32 + 0.5) / 11.0),
            grains: 1.0,
            spikes: 1.0,
            curtains: 6,
            ripple_life: 1.0,
            spectrum: [0.0; crate::analyzer::CURVE_POINTS],
            rise: [0.0; crate::analyzer::CURVE_POINTS],
        }
    }

    #[test]
    fn a_steady_120_hz_runs_one_step_a_frame_whatever_the_jitter() {
        let mut acc = 0.0;
        let mut total = 0;
        for k in 0..600 {
            let jitter = if k % 2 == 0 { 0.0003 } else { -0.0003 };
            let n = steps_for(&mut acc, 1.0 / 120.0 + jitter);
            assert_eq!(n, 1, "frame {k} ran {n} steps");
            total += n;
        }
        assert_eq!(total, 600);
    }

    #[test]
    fn other_frame_rates_average_to_real_time_and_stalls_are_capped() {
        for (fps, steps) in [(60.0, 2), (30.0, 4)] {
            let mut acc = 0.0;
            let n: u32 = (0..300).map(|_| steps_for(&mut acc, 1.0 / fps)).sum();
            assert_eq!(n, 300 * steps, "{fps} fps");
        }
        let mut acc = 0.0;
        assert_eq!(steps_for(&mut acc, 0.1), MAX_STEPS, "a 100 ms stall is capped");
        assert!(acc.abs() <= 0.5 * SIM_DT + 1e-6, "the backlog is dropped");
        let n: u32 = (0..288).map(|_| steps_for(&mut acc, 1.0 / 144.0)).sum();
        assert!((n as i32 - 240).abs() <= 1, "144 fps still advances in real time: {n}");
    }

    #[test]
    fn grid_cells_are_square_and_sized_by_height() {
        let g = geometry([5120, 1440]);
        assert_eq!(g.grid, [1280, 360]);
        assert_eq!(g.dye, [2560, 720]);
        let g = geometry([2560, 1440]);
        assert_eq!(g.grid, [640, 360]);
        let g = geometry([1600, 450]);
        assert_eq!(g.grid, [683, 192], "a small window shows an enlarged simulation, not a coarse one");
        assert!((g.grid[0] as f32 / g.grid[1] as f32 - 1600.0 / 450.0).abs() < 0.02);
    }

    #[test]
    fn nozzles_blow_in_proportion_to_their_band() {
        let g = geometry([5120, 1440]);
        let mut p = params();
        p.levels = [0.0, 0.2, 0.4, 0.6, 0.8, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let u = drive(&g, &p, 0.0, 1.0);
        assert_eq!(u.noz[0][2], 0.0, "a silent band does not blow");
        assert_eq!(u.inj[0][0], 0.0);
        for i in 1..5 {
            let rel = |k: usize| u.noz[k][2] / lerp(NOMINAL.0, NOMINAL.1, k as f32 / 10.0);
            assert!(rel(i + 1) > rel(i), "band {} should blow harder than band {}", i + 1, i);
            assert!(u.inj[i + 1][0] / lerp(NOMINAL.0, NOMINAL.1, (i + 1) as f32 / 10.0) > u.inj[i][0] / lerp(NOMINAL.0, NOMINAL.1, i as f32 / 10.0));
        }
        // bass nozzles are wide, treble nozzles narrow
        p.levels = [1.0; 11];
        let u = drive(&g, &p, 0.0, 1.0);
        assert!(u.noz[0][1] > 2.0 * u.noz[10][1]);
        // and they stand where the sliders do
        assert!((u.noz[0][0] - 0.5 / 11.0 * 1280.0).abs() < 1e-3);
        // a closed gate blows nothing, whatever the levels
        p.gate = 0.0;
        let u = drive(&g, &p, 0.0, 1.0);
        assert!(u.noz.iter().all(|n| n[2] == 0.0) && u.inj.iter().all(|i| i[0] == 0.0));
    }

    #[test]
    fn a_hit_puffs_the_nozzles_of_its_region_and_flash_zero_turns_puffs_off() {
        let g = geometry([5120, 1440]);
        let mut p = params();
        p.levels = [0.5; 11];
        let calm = drive(&g, &p, 0.0, 1.0);
        p.fx = [1.0, 0.0, 0.0, 0.0];
        let kick = drive(&g, &p, 0.0, 1.0);
        assert!(kick.noz[0][2] > 1.5 * calm.noz[0][2], "the bass nozzle puffs");
        assert!(kick.inj[0][2] > 0.0, "and carries an accent");
        assert!((kick.noz[9][2] - calm.noz[9][2]).abs() < 1e-3, "the treble nozzle does not");
        p.flash = 0.0;
        let off = drive(&g, &p, 0.0, 1.0);
        assert!((off.noz[0][2] - calm.noz[0][2]).abs() < 1e-3 && off.inj[0][2] == 0.0);
    }

    #[test]
    fn the_uniform_matches_the_shader_struct() {
        // 9 vec4 of parameters and 22 of nozzles
        assert_eq!(std::mem::size_of::<SimU>(), 16 * (9 + 22));
    }
}
