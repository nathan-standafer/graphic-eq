//! Scene modules. Each scene owns its pipelines and shader (`<name>.wgsl`, concatenated after
//! `common.wgsl`) and draws into the shared HDR scene pass. The compositor in `render.rs` owns the
//! targets, transitions, bloom and tone mapping, so scenes only describe what to draw.

use crate::analyzer::CURVE_POINTS;
use crate::render::{HIST_W, SCENE_FMT};
use bytemuck::{Pod, Zeroable};
use eframe::wgpu;
use glam::{Mat4, Vec3};
use std::collections::HashMap;

pub mod aurora;
pub mod columns;
pub mod ferro;
pub mod fireworks;
pub mod fountain;
pub mod lightning;
pub mod particles;
pub mod sand;
pub mod smoke;
pub mod sparks;
pub mod tunnel;
pub mod vectorscope;
pub mod waterfall;

/// Stable scene IDs; they are persisted in the settings file.
pub const COLUMNS: u32 = 0;
pub const WATERFALL: u32 = 1;
pub const TUNNEL: u32 = 2;
pub const VECTORSCOPE: u32 = 3;
pub const FOUNTAIN: u32 = 4;
pub const SMOKE: u32 = 5;
pub const CHLADNI: u32 = 6;
pub const FERROFLUID: u32 = 7;
pub const AURORA: u32 = 8;
pub const FIREWORKS: u32 = 9;
pub const LIGHTNING: u32 = 10;
pub const ALL: [u32; 11] = [COLUMNS, WATERFALL, TUNNEL, VECTORSCOPE, FOUNTAIN, SMOKE, CHLADNI, FERROFLUID, AURORA, FIREWORKS, LIGHTNING];

pub const DEPTH_FMT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
pub const MSAA: u32 = 4;
const COMMON_WGSL: &str = include_str!("common.wgsl");

pub const ALPHA: Option<wgpu::BlendState> = Some(wgpu::BlendState::ALPHA_BLENDING);
pub const ADDITIVE: Option<wgpu::BlendState> = Some(wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent::OVER,
});

// ------------------------------------------------------------------------------ uniforms

/// Mirrors `U` in `common.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Uniforms {
    pub vp: [[f32; 4]; 4],
    pub cam: [f32; 4],
    pub time: f32,
    pub aspect: f32,
    pub mode: f32,
    pub theme: f32,
    pub head: f32,
    pub rows: f32,
    pub exposure: f32,
    pub flash: f32,
    pub res: [f32; 4],
    pub fx: [f32; 4],
    pub sus: [f32; 4],
    pub ctl: [f32; 4],
    pub levels: [[f32; 4]; 3],
    pub peaks: [[f32; 4]; 3],
    pub rings: [[f32; 4]; 8],
}

/// Camera state the user and the idle path control; each scene turns it into a view.
#[derive(Clone, Copy)]
pub struct CamState {
    /// Orbit offsets from the scene's default view (radians) and a distance multiplier.
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    /// Middle-drag translation in screen points (scaled by view distance).
    pub pan: [f32; 2],
    /// Idle path offsets: yaw, pitch, zoom (fraction), roll (radians). Zero when motion is off.
    pub idle: [f32; 4],
}

impl Default for CamState {
    fn default() -> Self {
        CamState { yaw: 0.0, pitch: 0.0, zoom: 1.0, pan: [0.0; 2], idle: [0.0; 4] }
    }
}

/// Everything one scene needs to draw one frame.
#[derive(Clone)]
pub struct SceneParams {
    pub time: f32,
    /// Scene-specific variant (waterfall: 0 mixed channels, 1 stereo overlay).
    pub mode: u32,
    pub theme: u32,
    pub exposure: f32,
    pub flash: f32,
    pub motion: f32,
    pub gate: f32,
    pub levels: [f32; 11],
    pub peaks: [f32; 11],
    pub fx: [f32; 4],
    pub sus: [f32; 4],
    pub rings: [[f32; 4]; 8],
    pub cam: CamState,
    /// Fraction (0..1) of the way to the next history row.
    pub row_frac: f32,
    /// Phosphor persistence in seconds (the vectorscope's afterglow).
    pub persist: f32,
    /// Sparks overlay on the 3D columns.
    pub sparks: bool,
    /// Particle density multiplier (fountain emission, a quality setting).
    pub density: f32,
    /// How steeply a frequency's volume must rise to throw a spark (the fountain's sensitivity, 1 = designed).
    pub gradient: f32,
    /// Per column: how far its peak cap rose this frame in a burst-worthy push (0 = no burst).
    pub peak_hits: [f32; 11],
    /// How thick the smoke jets blow (1 = designed).
    pub smoke: f32,
    /// Where each EQ band's slider stands across the scene, as a fraction of its width (the smoke nozzles
    /// stand over them in windowed mode).
    pub nozzle_x: [f32; 11],
    /// How much sand lies on the Chladni plates (1 = designed).
    pub grains: f32,
    /// How tall the ferrofluid orb's spikes stand (1 = designed).
    pub spikes: f32,
    /// Most curtains the aurora may show (the picture's shape can allow fewer).
    pub curtains: u32,
    /// The 192-point spectrum as displayed (0..1, after range and sensitivity), and how steeply each point is
    /// rising now, relative to the music's own hardest recent hits (about 1 at those).
    pub spectrum: [f32; CURVE_POINTS],
    pub rise: [f32; CURVE_POINTS],
}

#[cfg(test)]
impl SceneParams {
    /// Silent, flat parameters for unit tests: a theme, the gate open, and nothing happening.
    pub fn for_tests() -> SceneParams {
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
            spectrum: [0.0; CURVE_POINTS],
            rise: [0.0; CURVE_POINTS],
        }
    }
}

/// What `update` and `draw` see besides the pipeline state: the frame's parameters and target size.
#[allow(dead_code)] // some fields are only for scenes that run offscreen work (persistence, simulation)
pub struct Frame<'a> {
    pub p: &'a SceneParams,
    pub size: [u32; 2],
    pub aspect: f32,
    /// Stereo samples (left, right) that arrived since the previous frame, oldest first. Each sample
    /// is delivered once, so a scene that is not drawn simply misses them.
    pub wave: &'a [[f32; 2]],
    /// This draw's uniform bind group (group 0), for scenes that run their own passes in `update`.
    pub bg: &'a wgpu::BindGroup,
    /// Like `ts`, for a scene whose `update` is a compute pass.
    pub ts_compute: Option<wgpu::ComputePassTimestampWrites<'a>>,
    /// Counts frames; a scene that sees it skip knows it missed some of `wave`. A scene drawn twice in
    /// one frame (a crossfade between two looks of it) sees the same number both times.
    pub frame_no: u64,
    /// GPU timing for the scene's update pass (only with `GEQ_GPUTIME`, and only for scenes that say
    /// `uses_update`). Attach it to one render pass.
    pub ts: Option<wgpu::RenderPassTimestampWrites<'a>>,
}

// ------------------------------------------------------------------------------- metadata

pub struct CamLimits {
    /// Allowed user yaw / pitch offsets (radians); `f32::INFINITY` yaw means free orbit.
    pub yaw: f32,
    pub pitch: (f32, f32),
    pub zoom: (f32, f32),
}

/// Scene-specific options shown in the Display menu while the scene is active.
#[derive(Clone, Copy, PartialEq)]
pub enum Control {
    PeakCaps,
    WaterfallChannels,
    WaterfallDetail,
    /// Phosphor afterglow time of the vectorscope.
    ScopePersistence,
    /// Embers and sparks over the 3D columns.
    Sparks,
    /// How many particles the fountain launches (and stars in a firework, branches and sparks in a strike).
    Density,
    /// How steeply the volume must rise to throw sparks (and to detonate a shell).
    Gradient,
    /// How thick the smoke jets blow.
    SmokeThickness,
    /// How much sand lies on the Chladni plates.
    SandGrains,
    /// How tall the ferrofluid orb's spikes stand.
    OrbSpikes,
    /// How many curtains the aurora shows.
    AuroraCurtains,
}

pub struct SceneInfo {
    pub id: u32,
    pub name: &'static str,
    pub cam: CamLimits,
    pub controls: &'static [Control],
    /// GPU time of one frame at 5120x1440 on the reference card (RX 580), in milliseconds. The
    /// compositor uses it to decide whether two scenes can crossfade live within the frame budget.
    pub cost_ms: f32,
}

const INFOS: [SceneInfo; 11] = [
    SceneInfo {
        id: COLUMNS,
        name: "3D columns",
        cam: CamLimits { yaw: f32::INFINITY, pitch: (-1.5, 1.5), zoom: (0.35, 3.0) },
        controls: &[Control::PeakCaps, Control::Sparks],
        cost_ms: 2.4,
    },
    SceneInfo {
        id: WATERFALL,
        name: "Waterfall",
        cam: CamLimits { yaw: f32::INFINITY, pitch: (-1.5, 1.5), zoom: (0.35, 3.0) },
        controls: &[Control::WaterfallChannels, Control::WaterfallDetail],
        cost_ms: 3.6,
    },
    SceneInfo {
        id: TUNNEL,
        name: "Spectrum Tunnel",
        cam: CamLimits { yaw: 0.5, pitch: (-0.35, 0.35), zoom: (0.6, 1.6) },
        controls: &[],
        cost_ms: 2.7,
    },
    SceneInfo {
        id: VECTORSCOPE,
        name: "Vectorscope",
        // a flat instrument: the camera does not move
        cam: CamLimits { yaw: 0.0, pitch: (0.0, 0.0), zoom: (1.0, 1.0) },
        controls: &[Control::ScopePersistence],
        cost_ms: 1.0,
    },
    SceneInfo {
        id: FOUNTAIN,
        name: "Fountain",
        cam: CamLimits { yaw: f32::INFINITY, pitch: (-1.5, 1.5), zoom: (0.35, 3.0) },
        controls: &[Control::Gradient, Control::Density],
        cost_ms: 0.8,
    },
    SceneInfo {
        id: SMOKE,
        name: "Smoke Jets",
        // a flat picture: the camera does not move
        cam: CamLimits { yaw: 0.0, pitch: (0.0, 0.0), zoom: (1.0, 1.0) },
        controls: &[Control::SmokeThickness],
        cost_ms: 1.8,
    },
    SceneInfo {
        id: CHLADNI,
        name: "Chladni Sand",
        // a flat picture: the camera does not move
        cam: CamLimits { yaw: 0.0, pitch: (0.0, 0.0), zoom: (1.0, 1.0) },
        controls: &[Control::SandGrains],
        cost_ms: 1.9,
    },
    SceneInfo {
        id: FERROFLUID,
        name: "Ferrofluid Orb",
        cam: CamLimits { yaw: f32::INFINITY, pitch: (-1.5, 1.5), zoom: (0.5, 2.0) },
        controls: &[Control::OrbSpikes],
        cost_ms: 1.8,
    },
    SceneInfo {
        id: AURORA,
        name: "Aurora",
        // shallow: the camera stands at the water's edge and may only turn and zoom a little
        cam: CamLimits { yaw: 0.09, pitch: (-0.05, 0.05), zoom: (0.85, 1.2) },
        controls: &[Control::AuroraCurtains],
        cost_ms: 1.8,
    },
    SceneInfo {
        id: FIREWORKS,
        name: "Fireworks",
        cam: CamLimits { yaw: f32::INFINITY, pitch: (-1.5, 1.5), zoom: (0.35, 3.0) },
        controls: &[Control::Gradient, Control::Density],
        cost_ms: 0.8,
    },
    SceneInfo {
        id: LIGHTNING,
        name: "Lightning Storm",
        cam: CamLimits { yaw: f32::INFINITY, pitch: (-1.5, 1.5), zoom: (0.35, 3.0) },
        controls: &[Control::Gradient, Control::Density],
        cost_ms: 0.5,
    },
];

pub fn info(id: u32) -> &'static SceneInfo {
    INFOS.iter().find(|i| i.id == id).unwrap_or(&INFOS[0])
}

// ----------------------------------------------------------------------- scene interface

pub trait Scene: Send + Sync {
    /// Offscreen work before the main pass: simulation steps, compute, persistence.
    fn update(&mut self, _device: &wgpu::Device, _queue: &wgpu::Queue, _enc: &mut wgpu::CommandEncoder, _f: &Frame) {}
    /// Whether `update` runs a pass worth timing this frame (it gets `Frame::ts` or `Frame::ts_compute`).
    fn uses_update(&self, _p: &SceneParams) -> bool {
        false
    }
    /// Draw into the HDR scene pass (bind group 0 is already set).
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, f: &Frame);
    /// 2D scenes anti-alias analytically and skip the 4x MSAA target.
    fn wants_msaa(&self) -> bool {
        true
    }
    /// View-projection matrix and eye position.
    fn camera(&self, aspect: f32, p: &SceneParams) -> (Mat4, Vec3);
    /// Scene-specific shape parameters for `ctl.yzw`.
    fn shape(&self, _aspect: f32) -> [f32; 3] {
        [0.0; 3]
    }
    fn clear_color(&self) -> wgpu::Color {
        wgpu::Color { r: 0.005, g: 0.006, b: 0.013, a: 1.0 }
    }
    /// Return to a deterministic initial state (simulation scenes use the seed).
    fn reset(&mut self, _seed: u64) {}
    /// The scene target changed size; resize or reset persistent state deliberately.
    fn resize(&mut self, _device: &wgpu::Device, _size: [u32; 2]) {}
}

/// What scenes need to build pipelines.
pub struct Ctx<'a> {
    pub device: &'a wgpu::Device,
    /// The layout every scene pass uses: group 0 is the uniforms and the spectrum history.
    pub layout: &'a wgpu::PipelineLayout,
    /// Group 0's layout, for scenes that add groups of their own.
    pub bgl: &'a wgpu::BindGroupLayout,
}

#[derive(Clone, Copy)]
pub struct Pipe {
    pub depth_write: bool,
    pub always: bool,
    pub blend: Option<wgpu::BlendState>,
    /// Multisampled (4x) or single-sample. Both variants use the depth buffer.
    pub msaa: bool,
}

impl Pipe {
    pub const OPAQUE: Pipe = Pipe { depth_write: true, always: false, blend: None, msaa: true };
}

impl<'a> Ctx<'a> {
    /// Compile a scene shader with `common.wgsl` in front of it.
    pub fn module(&self, label: &str, src: &str) -> wgpu::ShaderModule {
        let full = format!("{COMMON_WGSL}\n{src}")
            .replace("const NX: u32 = 96u;", &format!("const NX: u32 = {HIST_W}u;"))
            .replace("const RISE_FULL: f32 = 1.5;", &format!("const RISE_FULL: f32 = {:.1};", crate::render::RISE_FULL))
            .replace("const RISE_START: f32 = 0.42;", &format!("const RISE_START: f32 = {:.3};", crate::render::SPARK_START))
            .replace("const RISE_FULL_SPARK: f32 = 1.0;", &format!("const RISE_FULL_SPARK: f32 = {:.3};", crate::render::SPARK_FULL));
        self.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(full.into()) })
    }

    pub fn pipeline(&self, sh: &wgpu::ShaderModule, label: &str, vs: &str, fs: &str, o: Pipe) -> wgpu::RenderPipeline {
        self.pipeline_with(self.layout, sh, label, vs, fs, o)
    }

    /// Like `pipeline`, for a scene with extra bind groups after group 0.
    pub fn pipeline_with(&self, layout: &wgpu::PipelineLayout, sh: &wgpu::ShaderModule, label: &str, vs: &str, fs: &str, o: Pipe) -> wgpu::RenderPipeline {
        self.pipeline_cull(layout, sh, label, vs, fs, o, None)
    }

    /// Like `pipeline_with`, dropping triangles that face `cull` (front faces are counter-clockwise).
    pub fn pipeline_cull(&self, layout: &wgpu::PipelineLayout, sh: &wgpu::ShaderModule, label: &str, vs: &str, fs: &str, o: Pipe, cull: Option<wgpu::Face>) -> wgpu::RenderPipeline {
        self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(layout),
            vertex: wgpu::VertexState { module: sh, entry_point: Some(vs), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: sh,
                entry_point: Some(fs),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: SCENE_FMT, blend: o.blend, write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState { cull_mode: cull, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FMT,
                depth_write_enabled: Some(o.depth_write),
                depth_compare: Some(if o.always { wgpu::CompareFunction::Always } else { wgpu::CompareFunction::LessEqual }),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState { count: if o.msaa { MSAA } else { 1 }, mask: !0, alpha_to_coverage_enabled: false },
            multiview_mask: None,
            cache: None,
        })
    }
}

fn create(id: u32, ctx: &Ctx) -> Box<dyn Scene> {
    match id {
        WATERFALL => Box::new(waterfall::Waterfall::new(ctx)),
        TUNNEL => Box::new(tunnel::Tunnel::new(ctx)),
        VECTORSCOPE => Box::new(vectorscope::Vectorscope::new(ctx)),
        FOUNTAIN => Box::new(fountain::Fountain::new(ctx)),
        SMOKE => Box::new(smoke::Smoke::new(ctx)),
        CHLADNI => Box::new(sand::Sand::new(ctx)),
        FERROFLUID => Box::new(ferro::Ferrofluid::new(ctx)),
        AURORA => Box::new(aurora::Aurora::new(ctx)),
        FIREWORKS => Box::new(fireworks::Fireworks::new(ctx)),
        LIGHTNING => Box::new(lightning::Lightning::new(ctx)),
        _ => Box::new(columns::Columns::new(ctx)),
    }
}

// --------------------------------------------------------------------------- cameras

/// Right-handed look-at view matrix (camera looks down -Z in view space).
pub fn look_at(eye: Vec3, center: Vec3, up: Vec3) -> Mat4 {
    let f = (center - eye).normalize();
    let s = f.cross(up).normalize();
    let u = s.cross(f);
    Mat4::from_cols_array(&[
        s.x, u.x, -f.x, 0.0,
        s.y, u.y, -f.y, 0.0,
        s.z, u.z, -f.z, 0.0,
        -s.dot(eye), -u.dot(eye), f.dot(eye), 1.0,
    ])
}

/// Right-handed perspective with wgpu's 0..1 clip-space depth.
pub fn perspective(fovy: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / (fovy / 2.0).tan();
    Mat4::from_cols_array(&[
        f / aspect, 0.0, 0.0, 0.0,
        0.0, f, 0.0, 0.0,
        0.0, 0.0, far / (near - far), -1.0,
        0.0, 0.0, near * far / (near - far), 0.0,
    ])
}

pub struct Orbit {
    pub eye: Vec3,
    pub target: Vec3,
    pub fov_deg: f32,
}

/// Orbit around a target: spherical offset from the default view, user and idle offsets, and a
/// pan that translates eye and target together. The eye never goes below the floor plane.
pub fn orbit(o: &Orbit, aspect: f32, c: &CamState) -> (Mat4, Vec3) {
    let v = o.eye - o.target;
    let r = v.length() * (c.zoom * (1.0 + c.idle[2])).clamp(0.35, 3.0);
    let phi = v.x.atan2(v.z) + c.yaw + c.idle[0];
    let min_el = (0.5 - o.target.y).max(-r * 0.9) / r; // keep the eye above the floor
    let th = ((v.y / v.length()).asin() + c.pitch + c.idle[1]).clamp(min_el.asin(), 1.5);
    let eye = o.target + r * Vec3::new(th.cos() * phi.sin(), th.sin(), th.cos() * phi.cos());
    let f = (o.target - eye).normalize();
    let right = f.cross(Vec3::Y).normalize();
    let up = right.cross(f);
    let shift = (-right * c.pan[0] + up * c.pan[1]) * r * 0.0011;
    let (eye, target) = (eye + shift, o.target + shift);
    // idle roll tilts the horizon about the view direction
    let (sr, cr) = c.idle[3].sin_cos();
    let up = up * cr + right * sr;
    (perspective(o.fov_deg.to_radians(), aspect, 0.1, 100.0) * look_at(eye, target, up), eye)
}

// ----------------------------------------------------------------------------- manager

pub struct Slot {
    pub scene: Box<dyn Scene>,
    /// Two uniform sets, so a crossfade can draw one scene twice with different looks.
    pub ubuf: [wgpu::Buffer; 2],
    pub bg: [wgpu::BindGroup; 2],
    last_used: u64,
}

/// Creates scenes on demand and keeps a bounded number of them alive.
pub struct Manager {
    slots: HashMap<u32, Slot>,
    tick: u64,
    max_cached: usize,
}

impl Manager {
    pub fn new(max_cached: usize) -> Manager {
        Manager { slots: HashMap::new(), tick: 0, max_cached }
    }

    pub fn get_mut(&mut self, id: u32) -> Option<&mut Slot> {
        self.slots.get_mut(&id)
    }

    /// Build the scene if it is not alive yet. Returns the build time in milliseconds when it did work.
    pub fn ensure(&mut self, id: u32, ctx: &Ctx, scene_bgl: &wgpu::BindGroupLayout, hist: &wgpu::TextureView) -> Option<f32> {
        if self.slots.contains_key(&id) {
            return None;
        }
        let t0 = std::time::Instant::now();
        let scene = create(id, ctx);
        let make = || {
            let ubuf = ctx.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("scene_u"),
                size: std::mem::size_of::<Uniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bg = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("scene_bg"),
                layout: scene_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(hist) },
                ],
            });
            (ubuf, bg)
        };
        let ((u0, b0), (u1, b1)) = (make(), make());
        self.slots.insert(id, Slot { scene, ubuf: [u0, u1], bg: [b0, b1], last_used: self.tick });
        Some(t0.elapsed().as_secs_f32() * 1000.0)
    }

    /// Mark scenes as used this frame, then drop the least recently used ones beyond the bound.
    pub fn touch(&mut self, ids: &[u32]) {
        self.tick += 1;
        for id in ids {
            if let Some(s) = self.slots.get_mut(id) {
                s.last_used = self.tick;
            }
        }
        while self.slots.len() > self.max_cached {
            let victim = self.slots.iter().filter(|(id, _)| !ids.contains(id)).min_by_key(|(_, s)| s.last_used).map(|(id, _)| *id);
            match victim {
                Some(id) => {
                    self.slots.remove(&id);
                }
                None => break,
            }
        }
    }
}
