use super::*;

/// Aurora: broad translucent curtains of light standing over a dark lake, their height and folds following the
/// spectrum and trailing back through its history. A handful of vertical sheets (six at 32:9, fewer when the
/// picture is narrower) stand at different distances: the nearest shows the newest spectrum and each farther
/// one a little older, so a hit appears near and echoes back into the distance. Screen x is frequency on every
/// sheet (bass at the left). Sustained loudness lifts the columns, the mids fold the sheets in depth (a fold
/// turned edge-on to the viewer shows as a bright pleat), and an onset runs a small highlight up the rays where
/// its energy was. The slow energy trend spreads the stack apart. In silence the sheets settle to a dim, slow glow.
///
/// The ribbons are drawn at half resolution into an offscreen target (the scene's render scale, see `scale`),
/// farthest first, each dimming a little what is behind it. A full-resolution pass then shows that target over
/// the stars, behind a treeline, and mirrors it in a lake. The target holds palette-role weights rather than
/// colours, so two looks of the scene crossfade with their own palettes from one render.
pub struct Aurora {
    ribbons: wgpu::RenderPipeline,
    screen: wgpu::RenderPipeline,
    screen_bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    ubuf: wgpu::Buffer,
    rib_bg: wgpu::BindGroup,
    target: Option<Target>,
    /// Fraction of the scene's size the ribbons are rendered at.
    scale: f32,
    clocks: Clocks,
    seed: u64,
    last_frame: u64,
    last_time: f32,
}

struct Target {
    size: [u32; 2],
    view: wgpu::TextureView,
    screen_bg: wgpu::BindGroup,
    _tex: wgpu::Texture,
}

/// Mirrors `AuroraU` in `aurora_common.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct AuroraU {
    a: [f32; 4],
    b: [f32; 4],
    c: [f32; 4],
    rib: [[f32; 4]; CAP * 4],
}

/// Most ribbons the uniform holds (the table below has six).
const CAP: usize = 8;
/// Columns across a ribbon; keep in step with `NCOL` in `aurora_common.wgsl`.
const NCOL: u32 = 256;
/// The history texture's rows a second; keep in step with `HISTORY_RATE` in `app.rs`.
const ROWS_PER_S: f32 = 60.0;

/// The camera stands at the water's edge and looks a little above the horizon, so the lake takes the lower
/// third of a wide picture. The vertical field of view is the same at every aspect: a wider picture shows more
/// sky sideways, not more height.
const FOV_DEG: f32 = 40.0;
const PITCH: f32 = 0.136;
const DEFAULT_SCALE: f32 = 0.5;
/// The most the view may turn sideways. The ribbons reach 1.4 screen half widths past the middle and fade out
/// from 1.12, so a turn that would show their ends is not allowed.
const MAX_YAW: f32 = 0.11;

// --------------------------------------------------------------------------------------- the ribbons

/// One ribbon of the full set.
struct Spec {
    /// Distance in front of the eye (world units).
    dist: f32,
    /// Elevation of the base edge over the horizon, and how much more the tallest column adds (degrees).
    base: f32,
    height: f32,
    /// Seconds of history behind the newest row, and the seconds before a hit's highlight reaches it.
    delay: f32,
    lag: f32,
    /// Brightness, and how soft (0 sharp and near, 1 faint and far).
    gain: f32,
    soft: f32,
    /// Rays across the screen.
    ray: f32,
    /// Fold depth as a fraction of the distance, the two fold waves (radians per half screen width), and
    /// the speed they drift at.
    fold: f32,
    f1: f32,
    f2: f32,
    rate: f32,
    /// How far the base edge wanders (tangent of the angle).
    arc: f32,
}

/// Farthest first, which is the order they are drawn in. Farther sheets stand lower in the sky (the same
/// altitude seen from further away), smaller, fainter and softer, and show older spectrum.
const SPECS: [Spec; 6] = [
    Spec { dist: 58.0, base: 2.2, height: 5.0, delay: 1.30, lag: 0.55, gain: 0.22, soft: 1.00, ray: 60.0, fold: 0.10, f1: 2.1, f2: 5.4, rate: 0.17, arc: 0.018 },
    Spec { dist: 44.0, base: 3.9, height: 6.5, delay: 0.95, lag: 0.42, gain: 0.34, soft: 0.80, ray: 80.0, fold: 0.12, f1: 2.6, f2: 6.2, rate: 0.21, arc: 0.024 },
    Spec { dist: 33.0, base: 5.2, height: 8.0, delay: 0.65, lag: 0.30, gain: 0.50, soft: 0.60, ray: 100.0, fold: 0.14, f1: 2.3, f2: 5.8, rate: 0.19, arc: 0.030 },
    Spec { dist: 24.0, base: 7.6, height: 9.5, delay: 0.40, lag: 0.20, gain: 0.68, soft: 0.42, ray: 125.0, fold: 0.16, f1: 2.9, f2: 7.0, rate: 0.26, arc: 0.036 },
    Spec { dist: 17.0, base: 8.6, height: 11.0, delay: 0.18, lag: 0.10, gain: 0.85, soft: 0.22, ray: 150.0, fold: 0.18, f1: 2.5, f2: 6.5, rate: 0.23, arc: 0.040 },
    Spec { dist: 12.0, base: 10.8, height: 12.5, delay: 0.00, lag: 0.00, gain: 1.00, soft: 0.00, ray: 175.0, fold: 0.20, f1: 3.1, f2: 7.6, rate: 0.30, arc: 0.044 },
];

/// How many ribbons a picture of this shape shows, given the user's limit. A narrower picture gets fewer
/// instead of the same crowd cropped: 32:9 shows all six, 21:9 five, 16:9 four.
pub fn ribbons_for(aspect: f32, curtains: u32) -> usize {
    let auto = if aspect >= 2.9 {
        6
    } else if aspect >= 2.1 {
        5
    } else if aspect >= 1.5 {
        4
    } else {
        3
    };
    auto.min(curtains.clamp(2, 6) as usize)
}

/// Which of the six a set of `n` uses: spread evenly, always including the farthest and the nearest.
fn pick(n: usize) -> Vec<usize> {
    let n = n.clamp(2, SPECS.len());
    (0..n).map(|j| ((j * (SPECS.len() - 1)) as f32 / (n - 1) as f32).round() as usize).collect()
}

fn splitmix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A repeatable number in 0..1 for (seed, k).
fn unit(seed: u64, k: u64) -> f32 {
    (splitmix(seed.wrapping_mul(0x0100_0193) ^ k) >> 40) as f32 / (1u64 << 24) as f32
}

/// The four vec4s of each ribbon, farthest first. `spread` (0..1, the slow energy trend) moves the stack's
/// bases apart a little; the seed picks each ribbon's fold phases and ray pattern.
fn pack_ribbons(n: usize, seed: u64, spread: f32) -> [[f32; 4]; CAP * 4] {
    let mut out = [[0.0f32; 4]; CAP * 4];
    let lift = 0.92 + 0.16 * spread.clamp(0.0, 1.0);
    for (j, &i) in pick(n).iter().enumerate() {
        let s = &SPECS[i];
        let (p1, p2, sd) = (unit(seed, 3 * i as u64) * std::f32::consts::TAU, unit(seed, 3 * i as u64 + 1) * std::f32::consts::TAU, unit(seed, 3 * i as u64 + 2) * 50.0);
        out[4 * j] = [s.dist, s.base.to_radians().tan() * lift, s.height.to_radians().tan(), s.delay * ROWS_PER_S];
        out[4 * j + 1] = [s.fold, s.f1, p1, s.f2];
        out[4 * j + 2] = [p2, s.gain, s.soft, s.ray];
        out[4 * j + 3] = [sd.floor(), s.arc, s.lag, s.rate];
    }
    out
}

// ----------------------------------------------------------------------------------------- the clocks

/// The scene's slow clocks. Every wave, ray and ripple is a function of one of these, which advance by
/// `dt` times a speed, so the picture does not depend on the frame rate, moves slower when the signal is
/// quiet, and stands still at Motion 0 (the levels still lift the columns).
#[derive(Default)]
struct Clocks {
    fold: f32,
    ray: f32,
    lake: f32,
}

/// Motion 0.6 is the designed speed.
fn motion_scale(motion: f32) -> f32 {
    (motion / 0.6).clamp(0.0, 2.0)
}

impl Clocks {
    fn advance(&mut self, p: &SceneParams, dt: f32) {
        let k = (0.25 + 0.75 * p.gate.clamp(0.0, 1.0)) * motion_scale(p.motion) * dt;
        self.fold += k;
        self.ray += k;
        self.lake += k;
    }
}

// ------------------------------------------------------------------------------------------- the view

struct View {
    yaw: f32,
    pitch: f32,
    fov: f32,
    /// Half the width of the view at distance 1.
    half_w: f32,
    /// The horizon's place in the picture (0 top, 1 bottom).
    horizon: f32,
}

fn view(aspect: f32, p: &SceneParams) -> View {
    let c = &p.cam;
    let lim = &info(AURORA).cam;
    let yaw = (c.yaw.clamp(-lim.yaw, lim.yaw) + c.idle[0]).clamp(-MAX_YAW, MAX_YAW);
    let pitch = PITCH + (c.pitch.clamp(lim.pitch.0, lim.pitch.1) + c.idle[1]).clamp(-0.10, 0.10);
    let zoom = (c.zoom * (1.0 + c.idle[2])).clamp(lim.zoom.0, lim.zoom.1);
    let fov = FOV_DEG.to_radians() / zoom;
    let t = (fov * 0.5).tan();
    // with no roll the horizon is a straight line at tan(pitch) over the half height of the view
    View { yaw, pitch, fov, half_w: aspect * t, horizon: 0.5 + pitch.tan() / (2.0 * t) }
}

fn make_u(p: &SceneParams, aspect: f32, clocks: &Clocks, seed: u64) -> AuroraU {
    let v = view(aspect, p);
    let mm = motion_scale(p.motion).min(1.5);
    let mids = p.sus[1].clamp(0.0, 1.0);
    let bass = p.sus[0].clamp(0.0, 1.0);
    let trend = p.sus[3].clamp(0.0, 1.0);
    let n = ribbons_for(aspect, p.curtains);
    AuroraU {
        a: [v.horizon, v.half_w, n as f32, clocks.fold],
        b: [clocks.ray, clocks.lake, 0.35 + 0.65 * p.gate.clamp(0.0, 1.0), trend],
        // the mids deepen the folds (a calm base remains at Motion 0); bass lifts the stack and stirs the lake
        c: [0.35 + 0.65 * (0.25 + 0.75 * mids) * mm, (0.3 + 0.7 * bass) * mm, 0.012 * bass * mm, 0.0],
        rib: pack_ribbons(n, seed, trend),
    }
}

// --------------------------------------------------------------------------------------- the scene

impl Aurora {
    pub fn new(ctx: &Ctx) -> Aurora {
        let dev = ctx.device;
        let common = include_str!("aurora_common.wgsl");
        // the ribbon pass is not one of the shared scene passes: it has no depth and blends premultiplied
        let rib_sh = ctx.module("aurora", &format!("{common}\n{}", include_str!("aurora.wgsl")));
        let screen_sh = ctx.module("aurora_screen", &format!("{}\n{common}\n{}", include_str!("sky.wgsl"), include_str!("aurora_screen.wgsl")));

        let uniform = |visibility| wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let rib_bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("aurora_rib_bgl"),
            entries: &[uniform(wgpu::ShaderStages::VERTEX_FRAGMENT)],
        });
        let screen_bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("aurora_screen_bgl"),
            entries: &[
                uniform(wgpu::ShaderStages::FRAGMENT),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let layout_of = |bgl: &wgpu::BindGroupLayout| {
            dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(ctx.bgl), Some(bgl)], immediate_size: 0 })
        };
        let rib_layout = layout_of(&rib_bgl);
        let screen_layout = layout_of(&screen_bgl);

        // premultiplied "over": the light adds, and dims what is behind it by the ribbon's own opacity
        let over = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add };
        let ribbons = dev.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("aurora_ribbons"),
            layout: Some(&rib_layout),
            vertex: wgpu::VertexState { module: &rib_sh, entry_point: Some("vs_rib"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &rib_sh,
                entry_point: Some("fs_rib"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: SCENE_FMT, blend: Some(wgpu::BlendState { color: over, alpha: over }), write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let screen = ctx.pipeline_with(&screen_layout, &screen_sh, "aurora_screen", "vs_aur", "fs_aur", Pipe { depth_write: false, always: true, blend: None, msaa: false });

        let ubuf = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("aurora_u"),
            size: std::mem::size_of::<AuroraU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let rib_bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("aurora_rib_bg"),
            layout: &rib_bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }],
        });
        let sampler = dev.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        // GEQ_AURORA_SCALE=n renders the ribbons at that fraction of the scene's size (to compare quality and cost)
        let scale = std::env::var("GEQ_AURORA_SCALE").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(DEFAULT_SCALE).clamp(0.25, 1.0);
        Aurora { ribbons, screen, screen_bgl, sampler, ubuf, rib_bg, target: None, scale, clocks: Clocks::default(), seed: 1, last_frame: 0, last_time: 0.0 }
    }

    fn ensure_target(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        let sz = [((size[0] as f32 * self.scale).ceil() as u32).max(2), ((size[1] as f32 * self.scale).ceil() as u32).max(2)];
        if self.target.as_ref().map_or(false, |t| t.size == sz) {
            return;
        }
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("aurora_ribbons"),
            size: wgpu::Extent3d { width: sz[0], height: sz[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SCENE_FMT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        let screen_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("aurora_screen_bg"),
            layout: &self.screen_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.ubuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        self.target = Some(Target { size: sz, view, screen_bg, _tex: tex });
    }
}

impl Scene for Aurora {
    fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, f: &Frame) {
        self.ensure_target(device, f.size);
        if f.frame_no == self.last_frame {
            return; // already rendered for this frame (a crossfade between two looks of this scene)
        }
        let continuous = self.last_frame != 0 && f.frame_no <= self.last_frame + 20;
        let dt = if continuous { (f.p.time - self.last_time).clamp(0.0, 0.1) } else { 0.0 };
        self.last_frame = f.frame_no;
        self.last_time = f.p.time;
        self.clocks.advance(f.p, dt);

        let u = make_u(f.p, f.aspect, &self.clocks, self.seed);
        queue.write_buffer(&self.ubuf, 0, bytemuck::bytes_of(&u));

        let Some(t) = &self.target else { return };
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("aurora_ribbons"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &t.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: f.ts.clone(),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.ribbons);
        pass.set_bind_group(0, f.bg, &[]);
        pass.set_bind_group(1, &self.rib_bg, &[]);
        pass.draw(0..(NCOL - 1) * 6, 0..u.a[2] as u32);
    }

    fn uses_update(&self, _p: &SceneParams) -> bool {
        true
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, _f: &Frame) {
        let Some(t) = &self.target else { return };
        pass.set_pipeline(&self.screen);
        pass.set_bind_group(1, &t.screen_bg, &[]);
        pass.draw(0..3, 0..1);
    }

    fn wants_msaa(&self) -> bool {
        false
    }

    fn camera(&self, aspect: f32, p: &SceneParams) -> (Mat4, Vec3) {
        let v = view(aspect, p);
        let eye = Vec3::new(0.0, 1.6, 0.0);
        let dir = Vec3::new(v.yaw.sin() * v.pitch.cos(), v.pitch.sin(), -v.yaw.cos() * v.pitch.cos());
        (perspective(v.fov, aspect, 0.5, 400.0) * look_at(eye, eye + dir, Vec3::Y), eye)
    }

    fn reset(&mut self, seed: u64) {
        self.seed = seed.max(1);
        self.clocks = Clocks::default();
        self.last_frame = 0;
    }

    fn resize(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        self.ensure_target(device, size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> SceneParams {
        let mut p = SceneParams::for_tests();
        p.curtains = 6;
        p
    }

    #[test]
    fn the_uniform_is_a_whole_number_of_vec4s() {
        assert_eq!(std::mem::size_of::<AuroraU>() % 16, 0);
        assert_eq!(std::mem::size_of::<AuroraU>(), (3 + CAP * 4) * 16);
    }

    #[test]
    fn the_table_stands_from_far_to_near() {
        for w in SPECS.windows(2) {
            assert!(w[0].dist > w[1].dist, "nearer ribbons are drawn later");
            assert!(w[0].base < w[1].base, "the same altitude seen from nearer stands higher");
            assert!(w[0].gain < w[1].gain && w[0].soft > w[1].soft, "nearer is brighter and sharper");
            assert!(w[0].delay > w[1].delay, "farther shows older history");
            assert!(w[0].lag > w[1].lag);
        }
        assert_eq!(SPECS[5].delay, 0.0, "the nearest ribbon is the live spectrum");
        let rows = crate::render::HIST_H as f32;
        assert!(SPECS[0].delay * ROWS_PER_S + 3.0 < rows, "the oldest row read stays inside the ring");
        for s in &SPECS {
            assert!(s.arc < 0.05 && s.fold <= 0.2, "folds and wander stay shallow");
        }
    }

    #[test]
    fn a_narrower_picture_shows_fewer_ribbons() {
        assert_eq!(ribbons_for(32.0 / 9.0, 6), 6);
        assert_eq!(ribbons_for(21.0 / 9.0, 6), 5);
        assert_eq!(ribbons_for(16.0 / 9.0, 6), 4);
        assert_eq!(ribbons_for(1.0, 6), 3);
        assert_eq!(ribbons_for(32.0 / 9.0, 3), 3, "the user's limit wins");
        assert_eq!(ribbons_for(32.0 / 9.0, 0), 2, "and never goes below two");
        for a in [0.6, 1.0, 1.78, 2.4, 3.56] {
            for c in 0..8 {
                let n = ribbons_for(a, c);
                assert!((2..=6).contains(&n));
                let set = pick(n);
                assert_eq!(set.len(), n);
                assert_eq!((set[0], *set.last().unwrap()), (0, 5), "the farthest and the nearest always stay");
                assert!(set.windows(2).all(|w| w[0] < w[1]));
            }
        }
    }

    #[test]
    fn the_horizon_sits_in_the_lower_third_and_stays_in_the_picture() {
        for aspect in [16.0 / 9.0, 21.0 / 9.0, 32.0 / 9.0, 1.0] {
            let h = view(aspect, &params()).horizon;
            assert!((0.62..0.76).contains(&h), "horizon {h} at {aspect}");
        }
        // the camera limits keep it on screen
        let mut p = params();
        p.cam.pitch = 9.0;
        p.cam.zoom = 0.1;
        p.cam.idle = [9.0, 9.0, 9.0, 0.0];
        let h = view(32.0 / 9.0, &p).horizon;
        assert!((0.3..0.95).contains(&h));
    }

    #[test]
    fn the_ribbons_reach_past_the_screen_edge_at_the_widest_view() {
        // at the widest zoom and the furthest turn, the screen edge must still fall inside the part of the
        // ribbon that has not faded out (|s| < 1.38 in the shader)
        for aspect in [21.0 / 9.0, 32.0 / 9.0] {
            let mut p = params();
            p.cam.zoom = 0.0;
            p.cam.yaw = 9.0;
            p.cam.idle = [9.0, 0.0, 0.0, 0.0];
            let v = view(aspect, &p);
            assert!((v.yaw - MAX_YAW).abs() < 1e-6);
            let edge = ((v.half_w).atan() + v.yaw).tan() / v.half_w;
            assert!(edge < 1.38, "the screen edge falls at s = {edge} on a ribbon at {aspect}");
        }
    }

    #[test]
    fn the_clocks_do_not_depend_on_the_frame_rate() {
        let p = params();
        let run = |fps: f32| {
            let mut c = Clocks::default();
            for _ in 0..(fps * 4.0) as usize {
                c.advance(&p, 1.0 / fps);
            }
            c
        };
        let (a, b) = (run(30.0), run(120.0));
        assert!((a.fold - b.fold).abs() < 1e-3 && (a.lake - b.lake).abs() < 1e-3);
        assert!((a.fold - 4.0).abs() < 1e-3, "Motion 0.6 and an open gate run at one clock second a second");
    }

    #[test]
    fn motion_zero_stands_still_and_silence_slows_down() {
        let mut p = params();
        let mut c = Clocks::default();
        p.motion = 0.0;
        c.advance(&p, 5.0);
        assert_eq!((c.fold, c.ray, c.lake), (0.0, 0.0, 0.0));
        p.motion = 0.6;
        p.gate = 0.0;
        c.advance(&p, 4.0);
        assert!((c.fold - 1.0).abs() < 1e-4, "a closed gate runs at a quarter speed, not zero: {}", c.fold);
    }

    #[test]
    fn the_folds_follow_the_mids_and_rest_at_motion_zero() {
        let mut p = params();
        let calm = make_u(&p, 3.5, &Clocks::default(), 1).c[0];
        p.sus = [0.0, 1.0, 0.0, 0.0];
        let busy = make_u(&p, 3.5, &Clocks::default(), 1).c[0];
        assert!(busy > calm * 1.3, "mids deepen the folds: {calm} -> {busy}");
        p.motion = 0.0;
        let still = make_u(&p, 3.5, &Clocks::default(), 1).c;
        assert!((0.3..0.4).contains(&still[0]), "a small fixed fold remains: {}", still[0]);
        assert_eq!((still[1], still[2]), (0.0, 0.0), "no ripples and no lift at Motion 0");
    }

    #[test]
    fn silence_dims_the_light_without_switching_it_off() {
        let mut p = params();
        p.gate = 0.0;
        let quiet = make_u(&p, 3.5, &Clocks::default(), 1).b[2];
        p.gate = 1.0;
        let loud = make_u(&p, 3.5, &Clocks::default(), 1).b[2];
        assert!(quiet > 0.3 && quiet < 0.4 && (loud - 1.0).abs() < 1e-6);
    }

    #[test]
    fn the_seed_picks_the_pattern_and_repeats() {
        let a = pack_ribbons(6, 1, 0.5);
        let b = pack_ribbons(6, 1, 0.5);
        let c = pack_ribbons(6, 2, 0.5);
        assert!(a == b, "the same seed gives the same ribbons");
        assert!(a != c, "another seed gives other phases");
        for j in 0..6 {
            assert!((0.0..std::f32::consts::TAU).contains(&a[4 * j + 1][2]));
            assert!(a[4 * j + 3][0] == a[4 * j + 3][0].floor(), "the ray seed is a whole number, for the shader's hash");
        }
        // the slow trend lifts the stack a little
        let calm = pack_ribbons(6, 1, 0.0);
        let wide = pack_ribbons(6, 1, 1.0);
        assert!(wide[0][1] > calm[0][1]);
    }

    #[test]
    fn unused_ribbon_slots_stay_empty() {
        let r = pack_ribbons(4, 1, 0.0);
        assert!(r[16..].iter().all(|v| *v == [0.0; 4]));
        assert!(r[..16].iter().all(|v| v.iter().all(|x| x.is_finite())));
    }
}
