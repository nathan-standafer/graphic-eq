//! GPU scene: MSAA HDR render of the 3D visualizer, a bloom chain, and a composite
//! that egui draws into its own pass.

use bytemuck::{Pod, Zeroable};
use eframe::egui_wgpu::{self, CallbackResources, ScreenDescriptor};
use eframe::wgpu::{self, util::DeviceExt};
use glam::{Mat4, Vec3};

pub const HIST_W: u32 = crate::analyzer::CURVE_POINTS as u32;
pub const HIST_H: u32 = 256;
const SCENE_FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DEPTH_FMT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const MSAA: u32 = 4;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Uniforms {
    vp: [[f32; 4]; 4],
    cam: [f32; 4],
    time: f32,
    aspect: f32,
    mode: f32,
    theme: f32,
    head: f32,
    rows: f32,
    bloom: f32,
    bass: f32,
    levels: [[f32; 4]; 3],
    peaks: [[f32; 4]; 3],
    res: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PostParams {
    dir: [f32; 2],
    texel: [f32; 2],
    a: [f32; 4],
}

#[derive(Clone)]
pub struct SceneParams {
    pub time: f32,
    pub mode: u32, // 0 columns, 1 waterfall
    pub theme: u32,
    pub bloom: f32,
    pub bass: f32,
    pub levels: [f32; 11],
    pub peaks: [f32; 11],
    /// Orbit offsets from the default view (radians) and distance multiplier.
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    /// Middle-drag translation in screen points (scaled by view distance).
    pub pan: [f32; 2],
    /// Fraction (0..1) of the way to the next waterfall history row.
    pub row_frac: f32,
}

pub struct SceneCallback {
    pub params: SceneParams,
    pub size_px: [u32; 2],
    /// New history rows: HIST_W left bytes interleaved with HIST_W right bytes (Rg8).
    pub rows: Vec<Vec<u8>>,
}

struct Targets {
    size: [u32; 2],
    msaa_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    scene_view: wgpu::TextureView,
    a_view: wgpu::TextureView,
    b_view: wgpu::TextureView,
    c_view: wgpu::TextureView,
    d_view: wgpu::TextureView,
    bright_bg: wgpu::BindGroup,
    blur_h1: wgpu::BindGroup,
    blur_v1: wgpu::BindGroup,
    down_bg: wgpu::BindGroup,
    blur_h2: wgpu::BindGroup,
    blur_v2: wgpu::BindGroup,
    comp_bg: wgpu::BindGroup,
    comp_buf: wgpu::Buffer,
    _keep: Vec<wgpu::Texture>,
    _bufs: Vec<wgpu::Buffer>,
}

pub struct Gpu {
    scene_bgl: wgpu::BindGroupLayout,
    post_bgl: wgpu::BindGroupLayout,
    comp_bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform_buf: wgpu::Buffer,
    scene_bg: wgpu::BindGroup,
    hist_tex: wgpu::Texture,
    head: u32,
    total_rows: u32,
    bg_pipe: wgpu::RenderPipeline,
    col_pipe: wgpu::RenderPipeline,
    floor_pipe: wgpu::RenderPipeline,
    ter_pipe: wgpu::RenderPipeline,
    ter_st_pipe: wgpu::RenderPipeline,
    wfloor_pipe: wgpu::RenderPipeline,
    wall_pipe: wgpu::RenderPipeline,
    bright_pipe: wgpu::RenderPipeline,
    blur_pipe: wgpu::RenderPipeline,
    down_pipe: wgpu::RenderPipeline,
    comp_pipe: wgpu::RenderPipeline,
    targets: Option<Targets>,
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
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Gpu {
        let scene_sh = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("scene.wgsl").replace("const NX: u32 = 96u;", &format!("const NX: u32 = {HIST_W}u;")).into(),
            ),
        });
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
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
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
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let post_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post_bgl"),
            entries: &[tex_entry(0), smp_entry(1), uni_entry(2)],
        });
        let comp_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("comp_bgl"),
            entries: &[tex_entry(0), tex_entry(1), tex_entry(2), smp_entry(3), uni_entry(4)],
        });

        let layout = |bgl: &wgpu::BindGroupLayout| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(bgl)],
                immediate_size: 0,
            })
        };
        let scene_layout = layout(&scene_bgl);
        let post_layout = layout(&post_bgl);
        let comp_layout = layout(&comp_bgl);

        let scene_pipe = |label: &str, vs: &str, fs: &str, depth_write: bool, always: bool, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&scene_layout),
                vertex: wgpu::VertexState {
                    module: &scene_sh,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &scene_sh,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: SCENE_FMT,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FMT,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(if always { wgpu::CompareFunction::Always } else { wgpu::CompareFunction::LessEqual }),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState { count: MSAA, mask: !0, alpha_to_coverage_enabled: false },
                multiview_mask: None,
                cache: None,
            })
        };
        let alpha = Some(wgpu::BlendState::ALPHA_BLENDING);
        let additive = Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        });
        let bg_pipe = scene_pipe("bg", "vs_bg", "fs_bg", false, true, None);
        let col_pipe = scene_pipe("col", "vs_col", "fs_col", true, false, alpha);
        let floor_pipe = scene_pipe("floor", "vs_floor", "fs_floor", false, false, alpha);
        let ter_pipe = scene_pipe("ter", "vs_ter", "fs_ter", true, false, None);
        // stereo overlay: both channels' wireframes add together, no occlusion between them
        let ter_st_pipe = scene_pipe("ter_st", "vs_ter", "fs_ter", false, false, additive);
        let wall_pipe = scene_pipe("wall", "vs_wall", "fs_wall", false, false, alpha);
        let wfloor_pipe = scene_pipe("wfloor", "vs_wfloor", "fs_wfloor", true, false, None);

        let post_pipe = |label: &str, fs: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&post_layout),
                vertex: wgpu::VertexState {
                    module: &post_sh,
                    entry_point: Some("vs_full"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &post_sh,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(SCENE_FMT.into())],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let bright_pipe = post_pipe("bright", "fs_bright");
        let blur_pipe = post_pipe("blur", "fs_blur");
        let down_pipe = post_pipe("down", "fs_down");
        let comp_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("comp"),
            layout: Some(&comp_layout),
            vertex: wgpu::VertexState {
                module: &post_sh,
                entry_point: Some("vs_full"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &post_sh,
                entry_point: Some("fs_comp"),
                compilation_options: Default::default(),
                targets: &[Some(target_format.into())],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene_u"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let hist_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("history"),
            size: wgpu::Extent3d { width: HIST_W, height: HIST_H, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let hist_view = hist_tex.create_view(&Default::default());
        let scene_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene_bg"),
            layout: &scene_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&hist_view) },
            ],
        });

        Gpu {
            scene_bgl, post_bgl, comp_bgl, sampler, uniform_buf, scene_bg, hist_tex, head: 0, total_rows: 0,
            bg_pipe, col_pipe, floor_pipe, ter_pipe, ter_st_pipe, wfloor_pipe, wall_pipe, bright_pipe, blur_pipe, down_pipe, comp_pipe,
            targets: None,
        }
    }

    fn ensure_targets(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if self.targets.as_ref().map_or(false, |t| t.size == size) {
            return;
        }
        let [w, h] = size;
        let (hw, hh) = ((w / 2).max(1), (h / 2).max(1));
        let (qw, qh) = ((w / 4).max(1), (h / 4).max(1));
        let msaa = tex(device, w, h, SCENE_FMT, MSAA, "msaa");
        let depth = tex(device, w, h, DEPTH_FMT, MSAA, "depth");
        let scene = tex(device, w, h, SCENE_FMT, 1, "scene");
        let a = tex(device, hw, hh, SCENE_FMT, 1, "bloom_a");
        let b = tex(device, hw, hh, SCENE_FMT, 1, "bloom_b");
        let c = tex(device, qw, qh, SCENE_FMT, 1, "bloom_c");
        let d = tex(device, qw, qh, SCENE_FMT, 1, "bloom_d");
        let v = |t: &wgpu::Texture| t.create_view(&Default::default());
        let (scene_v, a_v, b_v, c_v, d_v) = (v(&scene), v(&a), v(&b), v(&c), v(&d));

        let mut bufs = Vec::new();
        let mut post_bg = |src: &wgpu::TextureView, dir: [f32; 2], texel: [f32; 2]| {
            let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("post_p"),
                contents: bytemuck::bytes_of(&PostParams { dir, texel, a: [0.0; 4] }),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("post_bg"),
                layout: &self.post_bgl,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(src) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    wgpu::BindGroupEntry { binding: 2, resource: buf.as_entire_binding() },
                ],
            });
            bufs.push(buf);
            bg
        };
        let (htx, qtx) = ([1.0 / hw as f32, 1.0 / hh as f32], [1.0 / qw as f32, 1.0 / qh as f32]);
        let bright_bg = post_bg(&scene_v, [0.0, 0.0], [0.0, 0.0]);
        let blur_h1 = post_bg(&a_v, [1.0, 0.0], htx);
        let blur_v1 = post_bg(&b_v, [0.0, 1.0], htx);
        let down_bg = post_bg(&a_v, [0.0, 0.0], htx);
        let blur_h2 = post_bg(&c_v, [1.0, 0.0], qtx);
        let blur_v2 = post_bg(&d_v, [0.0, 1.0], qtx);

        let comp_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("comp_p"),
            size: std::mem::size_of::<PostParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let comp_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("comp_bg"),
            layout: &self.comp_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&scene_v) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&a_v) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&c_v) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 4, resource: comp_buf.as_entire_binding() },
            ],
        });
        self.targets = Some(Targets {
            size,
            msaa_view: v(&msaa),
            depth_view: v(&depth),
            scene_view: scene_v,
            a_view: a_v,
            b_view: b_v,
            c_view: c_v,
            d_view: d_v,
            bright_bg, blur_h1, blur_v1, down_bg, blur_h2, blur_v2, comp_bg, comp_buf,
            _keep: vec![msaa, depth, scene, a, b, c, d],
            _bufs: bufs,
        });
    }
}

/// Right-handed look-at view matrix (camera looks down -Z in view space).
fn look_at(eye: Vec3, center: Vec3) -> Mat4 {
    let f = (center - eye).normalize();
    let s = f.cross(Vec3::Y).normalize();
    let u = s.cross(f);
    Mat4::from_cols_array(&[
        s.x, u.x, -f.x, 0.0,
        s.y, u.y, -f.y, 0.0,
        s.z, u.z, -f.z, 0.0,
        -s.dot(eye), -u.dot(eye), f.dot(eye), 1.0,
    ])
}

/// Right-handed perspective with wgpu's 0..1 clip-space depth.
fn perspective(fovy: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / (fovy / 2.0).tan();
    Mat4::from_cols_array(&[
        f / aspect, 0.0, 0.0, 0.0,
        0.0, f, 0.0, 0.0,
        0.0, 0.0, far / (near - far), -1.0,
        0.0, 0.0, near * far / (near - far), 0.0,
    ])
}

fn camera(mode: u32, aspect: f32, p: &SceneParams) -> (Mat4, Vec3) {
    let (eye0, target, fov) = if mode == 0 {
        (Vec3::new(0.0, 3.2, 10.6), Vec3::new(0.0, 2.1, 0.0), 40f32)
    } else {
        (Vec3::new(0.0, 3.6, 6.5), Vec3::new(0.0, 1.0, -9.5), 52f32)
    };
    // orbit around the target: spherical offset + user yaw/pitch/zoom
    let v = eye0 - target;
    let r = v.length() * p.zoom.clamp(0.35, 3.0);
    let phi = v.x.atan2(v.z) + p.yaw;
    let min_el = (0.5 - target.y).max(-r * 0.9) / r; // keep the eye above the floor
    let th = ((v.y / v.length()).asin() + p.pitch).clamp(min_el.asin(), 1.5);
    let eye = target + r * Vec3::new(th.cos() * phi.sin(), th.sin(), th.cos() * phi.cos());
    // translate eye and target together along the camera's right/up axes
    let f = (target - eye).normalize();
    let right = f.cross(Vec3::Y).normalize();
    let up = right.cross(f);
    let shift = (-right * p.pan[0] + up * p.pan[1]) * r * 0.0011;
    let (eye, target) = (eye + shift, target + shift);
    (perspective(fov.to_radians(), aspect, 0.1, 100.0) * look_at(eye, target), eye)
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
        gpu.ensure_targets(device, [w, h]);

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
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(HIST_W * 2), rows_per_image: Some(1) },
                wgpu::Extent3d { width: HIST_W, height: 1, depth_or_array_layers: 1 },
            );
        }

        let p = &self.params;
        let aspect = w as f32 / h as f32;
        let (vp, eye) = camera(p.mode, aspect, p);
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
            head: gpu.head as f32,
            rows: HIST_H as f32,
            bloom: p.bloom,
            bass: p.bass,
            levels: pack(&p.levels),
            peaks: pack(&p.peaks),
            res: [w as f32, h as f32, p.row_frac, (gpu.total_rows % 5) as f32],
        };
        queue.write_buffer(&gpu.uniform_buf, 0, bytemuck::bytes_of(&u));
        let t = gpu.targets.as_ref().unwrap();
        queue.write_buffer(
            &t.comp_buf,
            0,
            bytemuck::bytes_of(&PostParams { dir: [0.0; 2], texel: [0.0; 2], a: [p.bloom, p.time, 0.0, 0.0] }),
        );

        // ---- scene (MSAA HDR, resolved into scene_view)
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.msaa_view,
                    depth_slice: None,
                    resolve_target: Some(&t.scene_view),
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Discard },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &t.depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &gpu.scene_bg, &[]);
            pass.set_pipeline(&gpu.bg_pipe);
            pass.draw(0..3, 0..1);
            if p.mode == 0 {
                pass.set_pipeline(&gpu.col_pipe);
                pass.draw(0..36, 11..22); // mirrored reflections
                pass.set_pipeline(&gpu.floor_pipe);
                pass.draw(0..6, 0..1);
                pass.set_pipeline(&gpu.col_pipe);
                pass.draw(0..36, 0..11); // columns
                pass.draw(0..36, 22..33); // peak caps
            } else {
                pass.set_pipeline(&gpu.wfloor_pipe);
                pass.draw(0..6, 0..1);
                if p.mode == 2 {
                    pass.set_pipeline(&gpu.ter_st_pipe);
                    pass.draw(0..((HIST_W - 1) * 191 * 6), 0..2); // left, right
                } else {
                    pass.set_pipeline(&gpu.ter_pipe);
                    pass.draw(0..((HIST_W - 1) * 191 * 6), 0..1);
                }
                pass.set_pipeline(&gpu.wall_pipe);
                pass.draw(0..6, 0..1);
            }
        }

        // ---- bloom chain
        let fs_pass = |encoder: &mut wgpu::CommandEncoder, pipe: &wgpu::RenderPipeline, bg: &wgpu::BindGroup, dst: &wgpu::TextureView| {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: dst,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, bg, &[]);
            pass.draw(0..3, 0..1);
        };
        fs_pass(encoder, &gpu.bright_pipe, &t.bright_bg, &t.a_view);
        fs_pass(encoder, &gpu.blur_pipe, &t.blur_h1, &t.b_view);
        fs_pass(encoder, &gpu.blur_pipe, &t.blur_v1, &t.a_view);
        fs_pass(encoder, &gpu.down_pipe, &t.down_bg, &t.c_view);
        fs_pass(encoder, &gpu.blur_pipe, &t.blur_h2, &t.d_view);
        fs_pass(encoder, &gpu.blur_pipe, &t.blur_v2, &t.c_view);
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
