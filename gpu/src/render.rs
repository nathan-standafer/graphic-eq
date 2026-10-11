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
    /// New history rows: HIST_W texels of (left, right, mixed, unused) bytes (Rgba8).
    pub rows: Vec<Vec<u8>>,
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
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
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
            scene_bgl, scene_layout, single_bgl, pair_bgl, sampler, hist_tex, hist_view, head: 0, total_rows: 0,
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
        let ctx = Ctx { device, layout: &self.scene_layout };
        self.mgr.ensure(id, &ctx, &self.scene_bgl, &self.hist_view)
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

            // scenes may do offscreen work first
            let frame = Frame { p, size: [w, h], aspect };
            slot.scene.update(device, queue, encoder, &frame);

            let target = if !two { &t.scene_view } else if k == 0 { &t.out_a } else { &t.out_b };
            let ts_slot = gpu.timer.as_mut().and_then(|tm| tm.slot(if k == 0 { "scene" } else { "scene_in" }));
            let ts = gpu.timer.as_ref().and_then(|tm| tm.writes(ts_slot));
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
