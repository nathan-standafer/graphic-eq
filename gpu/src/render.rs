//! The compositor. Scenes (see `scenes/`) draw into HDR targets that this module owns; it also
//! crossfades between two live scenes in linear HDR, runs a layered bloom chain, and provides the
//! final composite that egui draws into its own pass (tone mapping happens once, there).

use crate::scenes::{self, Ctx, Frame, Manager, SceneParams, Uniforms};
use bytemuck::{Pod, Zeroable};
use eframe::egui_wgpu::{self, CallbackResources, ScreenDescriptor};
use eframe::wgpu;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub const HIST_W: u32 = crate::analyzer::CURVE_POINTS as u32;
pub const HIST_H: u32 = 256;
/// The normalized rise (see `modulation::RiseNorm`; 1 is the hardest recent hit) that fills the history
/// row's alpha byte.
pub const RISE_FULL: f32 = 1.5;
pub use crate::modulation::{SPARK_FULL, SPARK_START};
pub const SCENE_FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DEPTH_FMT: wgpu::TextureFormat = scenes::DEPTH_FMT;
const MSAA: u32 = scenes::MSAA;
pub const BLOOM_LEVELS: usize = 6;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PostParams {
    dir: [f32; 2],
    texel: [f32; 2],
    a: [f32; 4],
}

/// Per-frame post-processing look: how much of the scene feeds the bloom, and how each level of
/// the chain contributes. Levels run from ½ resolution (a tight halo) to 1/64 (a broad glow).
#[derive(Clone, Copy)]
pub struct PostLook {
    pub threshold: f32,
    pub knee: f32,
    pub weights: [f32; BLOOM_LEVELS],
    /// The user's Glow setting.
    pub master: f32,
    pub vignette: f32,
}

pub struct Draw {
    pub scene: u32,
    pub p: SceneParams,
}

pub struct SceneCallback {
    /// One scene, or the outgoing then the incoming scene during a crossfade.
    pub draws: Vec<Draw>,
    /// Weight of the incoming scene (0..1); only used with two draws.
    pub mix: f32,
    /// Keep showing the outgoing scene's last image instead of rendering it again. Used when
    /// two live scenes would not fit the frame budget; ignored until that image exists.
    pub freeze_outgoing: bool,
    pub post: PostLook,
    pub time: f32,
    pub size_px: [u32; 2],
    /// New history rows: HIST_W texels of (left, right, mixed, rise) bytes (Rgba8).
    pub rows: Vec<Vec<u8>>,
    /// Stereo samples since the previous frame, oldest first.
    pub wave: Vec<[f32; 2]>,
}

struct Targets {
    size: [u32; 2],
    msaa_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    depth1_view: wgpu::TextureView,
    scene_view: wgpu::TextureView,
    out_a: wgpu::TextureView,
    out_b: wgpu::TextureView,
    down_bgs: Vec<wgpu::BindGroup>,
    down_bufs: Vec<wgpu::Buffer>,
    down_views: Vec<wgpu::TextureView>,
    up_bgs: Vec<wgpu::BindGroup>,
    up_bufs: Vec<wgpu::Buffer>,
    up_views: Vec<wgpu::TextureView>,
    xfade_bg: wgpu::BindGroup,
    xfade_buf: wgpu::Buffer,
    comp_bg: wgpu::BindGroup,
    comp_buf: wgpu::Buffer,
    _keep: Vec<wgpu::Texture>,
}

pub struct Gpu {
    scene_bgl: wgpu::BindGroupLayout,
    scene_layout: wgpu::PipelineLayout,
    single_bgl: wgpu::BindGroupLayout,
    pair_bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    hist_tex: wgpu::Texture,
    hist_view: wgpu::TextureView,
    head: u32,
    total_rows: u32,
    frame_no: u64,
    mgr: Manager,
    down0_pipe: wgpu::RenderPipeline,
    down_pipe: wgpu::RenderPipeline,
    up_pipe: wgpu::RenderPipeline,
    xfade_pipe: wgpu::RenderPipeline,
    comp_pipe: wgpu::RenderPipeline,
    targets: Option<Targets>,
    /// The outgoing side of the crossfade target holds a finished image of the current transition.
    out_a_valid: bool,
    pub timer: Option<GpuTimer>,
}

fn tex(device: &wgpu::Device, w: u32, h: u32, fmt: wgpu::TextureFormat, samples: u32, label: &str) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format: fmt,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

impl Gpu {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, target_format: wgpu::TextureFormat) -> Gpu {
        let post_sh = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post"),
            source: wgpu::ShaderSource::Wgsl(include_str!("post.wgsl").into()),
        });

        let scene_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let smp_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let uni_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let single_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("single_bgl"),
            entries: &[tex_entry(0), smp_entry(1), uni_entry(2)],
        });
        let pair_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pair_bgl"),
            entries: &[tex_entry(0), tex_entry(1), smp_entry(2), uni_entry(3)],
        });

        let layout = |bgl: &wgpu::BindGroupLayout| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(bgl)], immediate_size: 0 })
        };
        let scene_layout = layout(&scene_bgl);
        let single_layout = layout(&single_bgl);
        let pair_layout = layout(&pair_bgl);

        let post_pipe = |label: &str, layout: &wgpu::PipelineLayout, fs: &str, fmt: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState { module: &post_sh, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
                fragment: Some(wgpu::FragmentState {
                    module: &post_sh,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(fmt.into())],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let down0_pipe = post_pipe("down0", &single_layout, "fs_down0", SCENE_FMT);
        let down_pipe = post_pipe("down", &single_layout, "fs_down", SCENE_FMT);
        let up_pipe = post_pipe("up", &pair_layout, "fs_up", SCENE_FMT);
        let xfade_pipe = post_pipe("xfade", &pair_layout, "fs_xfade", SCENE_FMT);
        let comp_pipe = post_pipe("comp", &pair_layout, "fs_comp", target_format);

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let hist_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("history"),
            size: wgpu::Extent3d { width: HIST_W, height: HIST_H, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let hist_view = hist_tex.create_view(&Default::default());

        Gpu {
            scene_bgl, scene_layout, single_bgl, pair_bgl, sampler, hist_tex, hist_view, head: 0, total_rows: 0, frame_no: 0,
            mgr: Manager::new(scenes::ALL.len()),
            down0_pipe, down_pipe, up_pipe, xfade_pipe, comp_pipe,
            targets: None,
            out_a_valid: false,
            timer: GpuTimer::new(device, queue),
        }
    }

    /// Build a scene's pipelines ahead of its first frame. Returns the build time in milliseconds
    /// if anything was built. The caller keeps showing the current scene until this has run.
    pub fn prepare_scene(&mut self, device: &wgpu::Device, id: u32) -> Option<f32> {
        let ctx = Ctx { device, layout: &self.scene_layout, bgl: &self.scene_bgl };
        let built = self.mgr.ensure(id, &ctx, &self.scene_bgl, &self.hist_view);
        // a scene built after the targets exist gets its size now, so it allocates what depends on it before its first frame
        if built.is_some() {
            if let (Some(t), Some(s)) = (&self.targets, self.mgr.get_mut(id)) {
                s.scene.resize(device, t.size);
            }
        }
        built
    }

    /// Restore a scene to a deterministic initial state.
    pub fn reset_scene(&mut self, id: u32, seed: u64) {
        if let Some(s) = self.mgr.get_mut(id) {
            s.scene.reset(seed);
        }
    }

    fn ensure_targets(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if self.targets.as_ref().map_or(false, |t| t.size == size) {
            return;
        }
        self.out_a_valid = false;
        let [w, h] = size;
        let msaa = tex(device, w, h, SCENE_FMT, MSAA, "msaa");
        let depth = tex(device, w, h, DEPTH_FMT, MSAA, "depth");
        let depth1 = tex(device, w, h, DEPTH_FMT, 1, "depth1");
        let scene = tex(device, w, h, SCENE_FMT, 1, "scene");
        let a = tex(device, w, h, SCENE_FMT, 1, "scene_a");
        let b = tex(device, w, h, SCENE_FMT, 1, "scene_b");
        let v = |t: &wgpu::Texture| t.create_view(&Default::default());

        // bloom pyramid: down[i] and up[i] share a size
        let mut sizes = Vec::new();
        let (mut lw, mut lh) = (w, h);
        for _ in 0..BLOOM_LEVELS {
            lw = (lw / 2).max(1);
            lh = (lh / 2).max(1);
            sizes.push([lw, lh]);
        }
        let down: Vec<wgpu::Texture> = sizes.iter().map(|s| tex(device, s[0], s[1], SCENE_FMT, 1, "bloom_down")).collect();
        let up: Vec<wgpu::Texture> = sizes.iter().map(|s| tex(device, s[0], s[1], SCENE_FMT, 1, "bloom_up")).collect();
        let down_views: Vec<_> = down.iter().map(v).collect();
        let up_views: Vec<_> = up.iter().map(v).collect();
        let scene_v = v(&scene);

        let params_buf = |label: &str| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: std::mem::size_of::<PostParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let single_bg = |src: &wgpu::TextureView, buf: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("single_bg"),
                layout: &self.single_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(src) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 2, resource: buf.as_entire_binding() },
                ],
            })
        };
        let pair_bg = |a: &wgpu::TextureView, b: &wgpu::TextureView, buf: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("pair_bg"),
                layout: &self.pair_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(a) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(b) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 3, resource: buf.as_entire_binding() },
                ],
            })
        };

        // down[0] reads the scene; down[i] reads down[i-1]. Their texel size is fixed per level.
        let mut down_bufs = Vec::new();
        let mut down_bgs = Vec::new();
        for i in 0..BLOOM_LEVELS {
            let buf = params_buf("down_p");
            down_bgs.push(single_bg(if i == 0 { &scene_v } else { &down_views[i - 1] }, &buf));
            down_bufs.push(buf);
        }
        // up[i] = down[i] * w + tent(up[i+1]); the last level has nothing below it
        let mut up_bufs = Vec::new();
        let mut up_bgs = Vec::new();
        for i in 0..BLOOM_LEVELS {
            let buf = params_buf("up_p");
            let lower = if i + 1 < BLOOM_LEVELS { &up_views[i + 1] } else { &down_views[i] };
            up_bgs.push(pair_bg(&down_views[i], lower, &buf));
            up_bufs.push(buf);
        }
        let xfade_buf = params_buf("xfade_p");
        let xfade_bg = pair_bg(&v(&a), &v(&b), &xfade_buf);
        let comp_buf = params_buf("comp_p");
        let comp_bg = pair_bg(&scene_v, &up_views[0], &comp_buf);

        let out_a = v(&a);
        let out_b = v(&b);
        let mut keep = vec![msaa, depth, scene, a, b, depth1];
        keep.extend(down);
        keep.extend(up);
        let (msaa_view, depth_view, depth1_view) = (v(&keep[0]), v(&keep[1]), v(&keep[5]));
        self.targets = Some(Targets {
            size, msaa_view, depth_view, depth1_view, scene_view: scene_v, out_a, out_b,
            down_bgs, down_bufs, down_views, up_bgs, up_bufs, up_views,
            xfade_bg, xfade_buf, comp_bg, comp_buf, _keep: keep,
        });
        for id in scenes::ALL {
            if let Some(s) = self.mgr.get_mut(id) {
                s.scene.resize(device, size);
            }
        }
    }
}

// ----------------------------------------------------------------------------- GPU timing

const MAX_TIMED_PASSES: usize = 24;
const READBACKS: usize = 4;

/// Pass timings from timestamp queries (`GEQ_GPUTIME=1`). Results arrive a few frames late.
pub struct GpuTimer {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    reads: Vec<Readback>,
    period_ns: f32,
    names: Vec<&'static str>,
    cur: usize,
    pub accum: std::collections::BTreeMap<&'static str, (f64, u32)>,
}

struct Readback {
    buf: wgpu::Buffer,
    state: u8, // 0 idle, 1 copied (needs map), 2 mapping
    ready: Arc<AtomicBool>,
    names: Vec<&'static str>,
}

impl GpuTimer {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<GpuTimer> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let n = (MAX_TIMED_PASSES * 2) as u32;
        let bytes = n as u64 * 8;
        let reads = (0..READBACKS)
            .map(|_| Readback {
                buf: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("ts_read"),
                    size: bytes,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                state: 0,
                ready: Arc::new(AtomicBool::new(false)),
                names: Vec::new(),
            })
            .collect();
        Some(GpuTimer {
            set: device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("ts"), ty: wgpu::QueryType::Timestamp, count: n }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("ts_resolve"),
                size: bytes,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            reads,
            period_ns: queue.get_timestamp_period(),
            names: Vec::new(),
            cur: 0,
            accum: Default::default(),
        })
    }

    /// Start of a frame: collect finished readbacks and request mapping of the ones copied last frame.
    fn begin_frame(&mut self, device: &wgpu::Device) {
        let _ = device.poll(wgpu::PollType::Poll);
        for r in self.reads.iter_mut() {
            if r.state == 1 {
                let flag = r.ready.clone();
                flag.store(false, Ordering::SeqCst);
                r.buf.slice(..).map_async(wgpu::MapMode::Read, move |res| {
                    if res.is_ok() {
                        flag.store(true, Ordering::SeqCst);
                    }
                });
                r.state = 2;
            } else if r.state == 2 && r.ready.load(Ordering::SeqCst) {
                {
                    let data = r.buf.slice(..).get_mapped_range().expect("mapped range");
                    let ts: &[u64] = bytemuck::cast_slice(&data);
                    for (i, name) in r.names.iter().enumerate() {
                        let (b, e) = (ts[2 * i], ts[2 * i + 1]);
                        if e >= b {
                            let ms = (e - b) as f64 * self.period_ns as f64 / 1.0e6;
                            let ent = self.accum.entry(name).or_insert((0.0, 0));
                            ent.0 += ms;
                            ent.1 += 1;
                        }
                    }
                }
                r.buf.unmap();
                r.state = 0;
            }
        }
        self.names.clear();
    }

    fn slot(&mut self, name: &'static str) -> Option<u32> {
        if self.names.len() >= MAX_TIMED_PASSES {
            return None;
        }
        self.names.push(name);
        Some(self.names.len() as u32 - 1)
    }

    fn writes(&self, slot: Option<u32>) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        slot.map(|i| wgpu::RenderPassTimestampWrites {
            query_set: &self.set,
            beginning_of_pass_write_index: Some(2 * i),
            end_of_pass_write_index: Some(2 * i + 1),
        })
    }

    fn compute_writes(&self, slot: Option<u32>) -> Option<wgpu::ComputePassTimestampWrites<'_>> {
        slot.map(|i| wgpu::ComputePassTimestampWrites {
            query_set: &self.set,
            beginning_of_pass_write_index: Some(2 * i),
            end_of_pass_write_index: Some(2 * i + 1),
        })
    }

    fn end_frame(&mut self, enc: &mut wgpu::CommandEncoder) {
        let n = self.names.len() as u32 * 2;
        if n == 0 {
            return;
        }
        // find an idle readback buffer; if every one is busy, skip this frame's timings
        let Some(k) = (0..READBACKS).map(|j| (self.cur + j) % READBACKS).find(|&k| self.reads[k].state == 0) else { return };
        self.cur = (k + 1) % READBACKS;
        enc.resolve_query_set(&self.set, 0..n, &self.resolve, 0);
        enc.copy_buffer_to_buffer(&self.resolve, 0, &self.reads[k].buf, 0, n as u64 * 8);
        self.reads[k].names = self.names.clone();
        self.reads[k].state = 1;
    }

    /// Mean milliseconds per pass since the last call.
    pub fn take_report(&mut self) -> Vec<(&'static str, f32)> {
        let out = self.accum.iter().map(|(n, (s, c))| (*n, (*s / (*c).max(1) as f64) as f32)).collect();
        self.accum.clear();
        out
    }
}

// --------------------------------------------------------------------------------- frame

/// GEQ_VIEW=scene|bloom bypasses part of the composite while debugging.
fn debug_view() -> f32 {
    match std::env::var("GEQ_VIEW").as_deref() {
        Ok("scene") => 1.0,
        Ok("nan") => 2.0,
        Ok("bloom") => -1.0,
        _ => 0.0,
    }
}

fn fs_pass(
    encoder: &mut wgpu::CommandEncoder,
    timer: &mut Option<GpuTimer>,
    name: &'static str,
    pipe: &wgpu::RenderPipeline,
    bg: &wgpu::BindGroup,
    dst: &wgpu::TextureView,
) {
    let ts_slot = timer.as_mut().and_then(|t| t.slot(name));
    let ts = timer.as_ref().and_then(|t| t.writes(ts_slot));
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(name),
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

impl egui_wgpu::CallbackTrait for SceneCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let gpu: &mut Gpu = resources.get_mut().expect("Gpu resources");
        gpu.frame_no += 1;
        let frame_no = gpu.frame_no;
        let [w, h] = [self.size_px[0].max(2), self.size_px[1].max(2)];
        for d in &self.draws {
            gpu.prepare_scene(device, d.scene); // normally built ahead of time by the app
        }
        gpu.ensure_targets(device, [w, h]);
        if let Some(t) = gpu.timer.as_mut() {
            t.begin_frame(device);
        }

        for row in &self.rows {
            gpu.head = (gpu.head + 1) % HIST_H;
            gpu.total_rows = gpu.total_rows.wrapping_add(1);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &gpu.hist_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: 0, y: gpu.head, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                row,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(HIST_W * 4), rows_per_image: Some(1) },
                wgpu::Extent3d { width: HIST_W, height: 1, depth_or_array_layers: 1 },
            );
        }

        let aspect = w as f32 / h as f32;
        let ids: Vec<u32> = self.draws.iter().map(|d| d.scene).collect();
        gpu.mgr.touch(&ids);
        let two = self.draws.len() > 1;
        let head = gpu.head;
        let total_rows = gpu.total_rows;

        // ---- scene passes (MSAA HDR, resolved into the scene target or into one side of a crossfade)
        let t = gpu.targets.as_ref().unwrap();
        let frozen = two && self.freeze_outgoing && gpu.out_a_valid;
        for (k, d) in self.draws.iter().enumerate() {
            if frozen && k == 0 {
                continue;
            }
            let slot = gpu.mgr.get_mut(d.scene).expect("scene prepared");
            let p = &d.p;
            let (vp, eye) = slot.scene.camera(aspect, p);
            let shape = slot.scene.shape(aspect);
            let pack = |a: &[f32; 11]| {
                let mut o = [[0f32; 4]; 3];
                for (i, v) in a.iter().enumerate() {
                    o[i / 4][i % 4] = *v;
                }
                o
            };
            let u = Uniforms {
                vp: vp.to_cols_array_2d(),
                cam: [eye.x, eye.y, eye.z, 1.0],
                time: p.time,
                aspect,
                mode: p.mode as f32,
                theme: p.theme as f32,
                head: head as f32,
                rows: HIST_H as f32,
                exposure: p.exposure,
                flash: p.flash,
                res: [w as f32, h as f32, p.row_frac, (total_rows % 5) as f32],
                fx: p.fx,
                sus: p.sus,
                ctl: [p.motion, shape[0], shape[1], p.gate.max(shape[2])],
                levels: pack(&p.levels),
                peaks: pack(&p.peaks),
                rings: p.rings,
            };
            queue.write_buffer(&slot.ubuf[k.min(1)], 0, bytemuck::bytes_of(&u));

            // scenes may do offscreen work first; reserve the timing slots before borrowing the timer
            let upd_slot = if slot.scene.uses_update(p) { gpu.timer.as_mut().and_then(|tm| tm.slot("scene_upd")) } else { None };
            let main_slot = gpu.timer.as_mut().and_then(|tm| tm.slot(if k == 0 { "scene" } else { "scene_in" }));
            let ts_upd = gpu.timer.as_ref().and_then(|tm| tm.writes(upd_slot));
            let ts_cmp = gpu.timer.as_ref().and_then(|tm| tm.compute_writes(upd_slot));
            let ts = gpu.timer.as_ref().and_then(|tm| tm.writes(main_slot));
            let frame = Frame { p, size: [w, h], aspect, wave: &self.wave, bg: &slot.bg[k.min(1)], frame_no, ts: ts_upd, ts_compute: ts_cmp };
            slot.scene.update(device, queue, encoder, &frame);

            let target = if !two { &t.scene_view } else if k == 0 { &t.out_a } else { &t.out_b };
            let c = slot.scene.clear_color();
            let mut pass = if slot.scene.wants_msaa() {
                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("scene"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &t.msaa_view,
                        depth_slice: None,
                        resolve_target: Some(target),
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(c), store: wgpu::StoreOp::Discard },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &t.depth_view,
                        depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: ts,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
            } else {
                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("scene"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(c), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &t.depth1_view,
                        depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: ts,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
            };
            pass.set_bind_group(0, &slot.bg[k.min(1)], &[]);
            slot.scene.draw(&mut pass, &frame);
        }

        gpu.out_a_valid = two; // rendered this frame, or kept from an earlier one

        // ---- per-pass parameters
        let post = &self.post;
        let write = |buf: &wgpu::Buffer, p: PostParams| queue.write_buffer(buf, 0, bytemuck::bytes_of(&p));
        let mut sizes = Vec::new();
        let (mut lw, mut lh) = (w, h);
        for _ in 0..BLOOM_LEVELS {
            lw = (lw / 2).max(1);
            lh = (lh / 2).max(1);
            sizes.push([lw as f32, lh as f32]);
        }
        let mut texel = [1.0 / w as f32, 1.0 / h as f32]; // texel size of each down pass's source
        for i in 0..BLOOM_LEVELS {
            write(&t.down_bufs[i], PostParams { dir: [0.0; 2], texel, a: [post.threshold, post.knee, 0.0, 0.0] });
            texel = [1.0 / sizes[i][0], 1.0 / sizes[i][1]];
        }
        for i in 0..BLOOM_LEVELS {
            let (below, tx) = if i + 1 < BLOOM_LEVELS { (1.0, [1.0 / sizes[i + 1][0], 1.0 / sizes[i + 1][1]]) } else { (0.0, [0.0; 2]) };
            write(&t.up_bufs[i], PostParams { dir: [0.0; 2], texel: tx, a: [post.weights[i], below, 0.0, 0.0] });
        }
        write(&t.xfade_buf, PostParams { dir: [0.0; 2], texel: [0.0; 2], a: [self.mix.clamp(0.0, 1.0), 0.0, 0.0, 0.0] });
        write(&t.comp_buf, PostParams { dir: [0.0; 2], texel: [0.0; 2], a: [post.master, post.vignette, self.time, debug_view()] });

        // ---- full-screen passes: crossfade, then the bloom pyramid down and back up
        if two {
            fs_pass(encoder, &mut gpu.timer, "xfade", &gpu.xfade_pipe, &t.xfade_bg, &t.scene_view);
        }
        fs_pass(encoder, &mut gpu.timer, "bloom_down0", &gpu.down0_pipe, &t.down_bgs[0], &t.down_views[0]);
        for i in 1..BLOOM_LEVELS {
            fs_pass(encoder, &mut gpu.timer, "bloom_down", &gpu.down_pipe, &t.down_bgs[i], &t.down_views[i]);
        }
        for i in (0..BLOOM_LEVELS).rev() {
            fs_pass(encoder, &mut gpu.timer, "bloom_up", &gpu.up_pipe, &t.up_bgs[i], &t.up_views[i]);
        }
        if let Some(tm) = gpu.timer.as_mut() {
            tm.end_frame(encoder);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: eframe::egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        let gpu: &Gpu = resources.get().expect("Gpu resources");
        if let Some(t) = &gpu.targets {
            pass.set_pipeline(&gpu.comp_pipe);
            pass.set_bind_group(0, &t.comp_bg, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

/// A windowless check on the real adapter: the same `SceneCallback::prepare` the app runs every frame (scene
/// passes, crossfade, bloom) into offscreen targets, with timestamp queries, and the final composite saved as a
/// PNG. It needs no window and no display, so it works when the monitor is off, and it feeds the scene a synthetic
/// signal (a kick-like hit every half second at a wandering place on the line, a slowly moving spectrum), so the
/// timings are the scene's own cost and not a replacement for the app's frame-time run. Ignored by default:
///   GEQ_HL_SCENE=11 GEQ_HL_SIZE=5120x1440 GEQ_HL_SECS=8 cargo test --release headless_scene -- --ignored --nocapture
/// (GEQ_HL_LOOK picks the look, GEQ_HL_OUT the PNG; `GEQ_HL_SOFTWARE=1` uses the software adapter; `GEQ_HL_SCENE2=n`
/// draws a second scene and crossfades half and half, as a transition does.)
#[cfg(test)]
mod headless {
    use super::*;
    use eframe::egui_wgpu::CallbackTrait;

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        let mut f = std::pin::pin!(f);
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        loop {
            if let std::task::Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
            std::thread::yield_now();
        }
    }

    /// One frame's synthetic parameters at time `t` (seconds): a hit every half second, a slowly moving spectrum.
    fn params(scene: u32, look: &crate::presets::Look, t: f32) -> SceneParams {
        let mut p = SceneParams::for_tests();
        p.time = t;
        p.theme = look.theme;
        p.exposure = look.exposure;
        p.flash = look.flash;
        p.motion = look.motion;
        p.gate = 1.0;
        let k = (t * 2.0).floor();
        let since = t - k / 2.0;
        // where this half second's hit lands, and how far its envelope has decayed
        let pos = ((k * 0.618034).fract() * 0.85 + 0.05).clamp(0.0, 1.0);
        let env = (-since / 0.09).exp();
        for i in 0..crate::analyzer::CURVE_POINTS {
            let x = i as f32 / (crate::analyzer::CURVE_POINTS - 1) as f32;
            let s = 0.50 * (-2.2 * x).exp() + 0.28 + 0.12 * (0.7 * t + 9.0 * x).sin();
            p.spectrum[i] = s.clamp(0.0, 1.0);
            // a rise only for the first three frames of the hit, across a few bins
            p.rise[i] = if since < 0.026 { 1.5 * (-((x - pos) / 0.04).powi(2)).exp() } else { 0.05 };
        }
        for i in 0..11 {
            p.levels[i] = p.spectrum[i * 17].clamp(0.0, 1.0);
            p.peaks[i] = (p.levels[i] + 0.08).min(1.0);
        }
        p.sus = [0.6 + 0.2 * (0.5 * t).sin(), 0.5, 0.45, 0.5];
        p.fx = [if pos < 0.35 { env } else { 0.0 }, if (0.3..0.7).contains(&pos) { env } else { 0.0 }, if pos > 0.6 { env } else { 0.0 }, 0.0];
        let _ = scene;
        p
    }

    fn history_row(t: f32) -> Vec<u8> {
        let n = crate::analyzer::CURVE_POINTS;
        (0..n)
            .flat_map(|i| {
                let x = i as f32 / (n - 1) as f32;
                let s = (0.50 * (-2.2 * x).exp() + 0.28 + 0.12 * (0.7 * t + 9.0 * x).sin()).clamp(0.0, 1.0);
                let v = (s * 255.0) as u8;
                [v, v, v, 0]
            })
            .collect()
    }

    #[test]
    #[ignore]
    fn headless_scene() {
        let env = |k: &str| std::env::var(k).ok();
        let scene: u32 = env("GEQ_HL_SCENE").and_then(|v| v.parse().ok()).unwrap_or(scenes::RIPPLE);
        let look_ix: usize = env("GEQ_HL_LOOK").and_then(|v| v.parse().ok()).unwrap_or(0);
        let (w, h) = env("GEQ_HL_SIZE")
            .and_then(|s| s.split_once('x').and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?))))
            .unwrap_or((5120, 1440));
        let scene2: Option<u32> = env("GEQ_HL_SCENE2").and_then(|v| v.parse().ok());
        let secs: f32 = env("GEQ_HL_SECS").and_then(|v| v.parse().ok()).unwrap_or(8.0);
        let out = env("GEQ_HL_OUT").unwrap_or_else(|| "/tmp/geq-headless.png".into());

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: env("GEQ_HL_SOFTWARE").is_some(),
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .expect("an adapter");
        eprintln!("adapter: {} ({:?})", adapter.get_info().name, adapter.get_info().backend);
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
            required_limits: adapter.limits(),
            ..Default::default()
        }))
        .expect("a device");
        // validation errors are failures here, not log lines
        let errors = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        {
            let errors = errors.clone();
            device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| errors.lock().unwrap().push(e.to_string())));
        }

        let fmt = wgpu::TextureFormat::Rgba8Unorm;
        let mut resources = CallbackResources::default();
        resources.insert(Gpu::new(&device, &queue, fmt));
        let look = crate::presets::get(scene, look_ix);
        let look2 = crate::presets::get(scene2.unwrap_or(scene), 0);
        let post = PostLook { threshold: look.threshold, knee: look.knee, weights: look.bloom, master: 0.9, vignette: look.vignette };
        let screen = ScreenDescriptor { size_in_pixels: [w, h], pixels_per_point: 1.0 };

        let frames = (secs * 120.0) as usize;
        let t_start = std::time::Instant::now();
        let mut cpu_ms = Vec::new();
        for f in 0..frames + 12 {
            let t = f as f32 / 120.0;
            let cb = SceneCallback {
                draws: std::iter::once(Draw { scene, p: params(scene, look, t) })
                    .chain(scene2.map(|s2| Draw { scene: s2, p: params(s2, look2, t) }))
                    .collect(),
                mix: if scene2.is_some() { 0.5 } else { 0.0 },
                freeze_outgoing: false,
                post,
                time: t,
                size_px: [w, h],
                rows: if f % 2 == 0 { vec![history_row(t)] } else { Vec::new() },
                wave: Vec::new(),
            };
            let c0 = std::time::Instant::now();
            let mut enc = device.create_command_encoder(&Default::default());
            cb.prepare(&device, &queue, &screen, &mut enc, &mut resources);
            queue.submit([enc.finish()]);
            cpu_ms.push(c0.elapsed().as_secs_f32() * 1000.0);
            if f % 6 == 5 {
                let _ = device.poll(wgpu::PollType::wait_indefinitely()); // keep the queue short
            }
        }
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let wall = t_start.elapsed().as_secs_f32();

        // the passes' mean GPU time (timestamp queries arrive a few frames late)
        let gpu: &mut Gpu = resources.get_mut().unwrap();
        for _ in 0..6 {
            if let Some(tm) = gpu.timer.as_mut() {
                tm.begin_frame(&device);
            }
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
        }
        match gpu.timer.as_mut() {
            Some(tm) => {
                let report = tm.take_report();
                let sum: f32 = report.iter().map(|(_, ms)| ms).sum();
                eprintln!("gpu (mean per frame, {w}x{h}, scene {scene}, look {look_ix}): {} | sum {sum:.2} ms", report.iter().map(|(n, ms)| format!("{n} {ms:.2}")).collect::<Vec<_>>().join("  "));
            }
            None => eprintln!("no timestamp queries on this adapter"),
        }
        cpu_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
        eprintln!("cpu to record a frame: median {:.2} ms, p99 {:.2} ms; {frames} frames in {wall:.1} s of wall time (submit-bound, not real time)", cpu_ms[cpu_ms.len() / 2], cpu_ms[cpu_ms.len() * 99 / 100]);

        // the final composite, as the app's last pass draws it, saved as a PNG
        let gpu: &Gpu = resources.get().unwrap();
        let t = gpu.targets.as_ref().unwrap();
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("headless_out"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: fmt,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let row = (w * 4).next_multiple_of(256);
        let buf = device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (row * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("headless_comp"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&gpu.comp_pipe);
            pass.set_bind_group(0, &t.comp_bg, &[]);
            pass.draw(0..3, 0..1);
        }
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &target, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        queue.submit([enc.finish()]);
        buf.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("map"));
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let data = buf.slice(..).get_mapped_range().expect("mapped");
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h as usize {
            rgba.extend_from_slice(&data[y * row as usize..y * row as usize + (w * 4) as usize]);
        }
        let (mut sum, mut bright) = (0u64, 0u32);
        for px in rgba.chunks_exact(4) {
            let l = px[0] as u32 + px[1] as u32 + px[2] as u32;
            sum += l as u64;
            bright += (l > 3 * 200) as u32;
        }
        eprintln!("picture: mean level {:.1}/255, {:.3}% of pixels near white; saved {out}", sum as f64 / (w as f64 * h as f64 * 3.0), 100.0 * bright as f64 / (w as f64 * h as f64));
        image::save_buffer(&out, &rgba, w, h, image::ColorType::Rgba8).expect("png");
        let errs = errors.lock().unwrap();
        assert!(errs.is_empty(), "wgpu errors: {errs:?}");
    }
}
