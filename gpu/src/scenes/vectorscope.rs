use super::*;

/// The raw stereo signal drawn the way an analogue X/Y scope draws it: X = (R - L)/√2, Y = (L + R)/√2, so
/// mono is a vertical line and out-of-phase material tips toward the horizontal. The beam is bright where
/// it dwells and faint where it races, and it leaves a fading phosphor trail. On wide screens the left and
/// right waveforms run across the outer thirds as triggered sweeps.
///
/// Each frame the samples that arrived since the last one are drawn as beam segments, additively, into a
/// persistence buffer that is first multiplied by exp(-dt/τ) (so the picture does not depend on the frame
/// rate). A full-screen pass then shows that buffer as phosphor over a fixed graticule.
pub struct Vectorscope {
    fade: wgpu::RenderPipeline,
    beam: wgpu::RenderPipeline,
    comp: wgpu::RenderPipeline,
    comp_bgl: wgpu::BindGroupLayout,
    ring: wgpu::Texture,
    ubuf: wgpu::Buffer,
    beam_bg: wgpu::BindGroup,
    persist: Option<Persist>,
    tracer: Tracer,
    /// Next ring slot to write, and whether the slot before it holds a sample the next segment can start from.
    head: u32,
    prev_valid: bool,
    last_frame: u64,
    last_time: f32,
    clear: bool,
}

struct Persist {
    size: [u32; 2],
    view: wgpu::TextureView,
    comp_bg: wgpu::BindGroup,
    _tex: wgpu::Texture,
}

/// Mirrors `ScopeU` in `vectorscope_common.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ScopeU {
    area: [f32; 4],
    left: [f32; 4],
    right: [f32; 4],
    beam: [f32; 4],
    seg: [u32; 4],
    trace: [f32; 4],
}

/// Samples the ring holds (a 4096 x 1 texture of left, right, left sweep x, right sweep x).
const RING: u32 = 4096;
/// Segments drawn in one frame at most; after a long stall the oldest samples are dropped.
const SEG_CAP: usize = 3000;
/// The analyzer's sample rate.
const RATE: f32 = 48_000.0;
/// One pass of a trace, in samples (24 ms).
const SWEEP_N: u32 = 1152;

/// Brightness a stationary beam settles at in the fast and in the slow channel. A beam that races
/// across the screen deposits roughly 1/speed of that, so these are set for moving traces: a stationary
/// beam is far brighter, as on a real tube, and the signal gate keeps silence from burning a hole.
const BEAM_FAST: f32 = 200.0;
const BEAM_SLOW: f32 = 55.0;
/// The slow afterglow lasts this many times longer than the fast flash.
const SLOW_FACTOR: f32 = 4.0;

// ------------------------------------------------------------------------------------ signal side

/// Follows the loudest recent sample so the picture fills the screen on quiet recordings and does not
/// overshoot on loud ones: instant on the way up, a few seconds on the way down. Only the picture's
/// scale changes; nothing here touches the audio or the analyzer's measurements.
#[derive(Clone, Copy)]
struct Follower {
    peak: f32,
}

impl Follower {
    const RELEASE_S: f32 = 6.0;
    const TARGET: f32 = 0.8;

    fn observe(&mut self, batch_peak: f32, dt: f32) {
        self.peak = batch_peak.max(self.peak * (-dt / Self::RELEASE_S).exp());
    }

    /// Display gain: the recent peak lands at 80% of the radius, within 0.55x..24x.
    fn gain(&self) -> f32 {
        (Self::TARGET / self.peak.max(1e-4)).clamp(0.55, 24.0)
    }
}

/// One trace's sweep: it starts at a rising zero crossing, runs for `SWEEP_N` samples, then waits for the
/// next one (free-running if there is nothing to trigger on, so noise still draws).
#[derive(Clone, Copy)]
struct Sweep {
    /// Samples into the current pass, or -1 while waiting.
    pos: i32,
    armed: bool,
    idle: u32,
}

impl Sweep {
    fn new() -> Sweep {
        Sweep { pos: -1, armed: false, idle: 0 }
    }

    /// Position (0..1) of this sample along the sweep, or -1 when the beam is blanked.
    fn step(&mut self, v: f32, hysteresis: f32) -> f32 {
        if self.pos >= 0 {
            let x = self.pos as f32 / SWEEP_N as f32;
            self.pos += 1;
            if self.pos as u32 >= SWEEP_N {
                self.pos = -1;
                self.armed = false;
                self.idle = 0;
            }
            return x;
        }
        self.idle += 1;
        if v < -hysteresis {
            self.armed = true;
        }
        if (self.armed && v >= 0.0) || self.idle > 2 * SWEEP_N {
            self.pos = 1;
            self.armed = false;
            return 0.0;
        }
        -1.0
    }
}

struct Tracer {
    xy: Follower,
    lr: Follower,
    sweeps: [Sweep; 2],
}

impl Tracer {
    fn new() -> Tracer {
        Tracer { xy: Follower { peak: 0.0 }, lr: Follower { peak: 0.0 }, sweeps: [Sweep::new(); 2] }
    }

    /// Ring texels for a batch of samples, and the follower updates for `dt` seconds.
    fn process(&mut self, samples: &[[f32; 2]], dt: f32, out: &mut Vec<[f32; 4]>) {
        let (mut pk_xy, mut pk_lr) = (0.0f32, 0.0f32);
        for s in samples {
            let (l, r) = (s[0], s[1]);
            pk_xy = pk_xy.max(((r - l) * std::f32::consts::FRAC_1_SQRT_2).abs()).max(((l + r) * std::f32::consts::FRAC_1_SQRT_2).abs());
            pk_lr = pk_lr.max(l.abs()).max(r.abs());
        }
        self.xy.observe(pk_xy, dt);
        self.lr.observe(pk_lr, dt);
        let hyst = 0.05 * self.lr.peak;
        for s in samples {
            let xl = self.sweeps[0].step(s[0], hyst);
            let xr = self.sweeps[1].step(s[1], hyst);
            out.push([s[0], s[1], xl, xr]);
        }
    }

    fn reset(&mut self) {
        *self = Tracer::new();
    }
}

// ----------------------------------------------------------------------------------- screen layout

/// Where the scope and the traces sit on a screen of `w` x `h` pixels. The scope stays a circle; the
/// traces take the space beside it when there is enough (a narrower window just shows the scope).
struct Layout {
    centre: [f32; 2],
    radius: f32,
    left: [f32; 4],
    right: [f32; 4],
}

fn layout(w: f32, h: f32) -> Layout {
    let centre = [w * 0.5, h * 0.5];
    let radius = (0.44 * h).min(0.46 * w);
    let inner = radius + 0.07 * h;
    let pad = 0.04 * h;
    let (mut left, mut right) = ([0.0; 4], [0.0; 4]);
    if centre[0] - inner - pad >= 0.5 * h {
        let (y0, y1) = (centre[1] - 0.2 * h, centre[1] + 0.2 * h);
        left = [pad, y0, centre[0] - inner, y1];
        right = [centre[0] + inner, y0, w - pad, y1];
    }
    Layout { centre, radius, left, right }
}

// ----------------------------------------------------------------------------------------- scene

impl Vectorscope {
    pub fn new(ctx: &Ctx) -> Vectorscope {
        let dev = ctx.device;
        let common = include_str!("vectorscope_common.wgsl");
        let beam_sh = ctx.module("vectorscope_beam", &format!("{common}\n{}", include_str!("vectorscope_beam.wgsl")));
        let comp_sh = ctx.module("vectorscope", &format!("{common}\n{}", include_str!("vectorscope.wgsl")));

        let beam_bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scope_beam_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let comp_bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scope_comp_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let layout_of = |groups: &[Option<&wgpu::BindGroupLayout>]| {
            dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: groups, immediate_size: 0 })
        };
        let beam_layout = layout_of(&[Some(ctx.bgl), Some(&beam_bgl)]);
        let comp_layout = layout_of(&[Some(ctx.bgl), Some(&comp_bgl)]);
        let fade_layout = layout_of(&[]);

        // the persistence buffer is single-sampled HDR with no depth
        let offscreen = |label: &str, layout: &wgpu::PipelineLayout, vs: &str, fs: &str, blend: Option<wgpu::BlendState>| {
            dev.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState { module: &beam_sh, entry_point: Some(vs), compilation_options: Default::default(), buffers: &[] },
                fragment: Some(wgpu::FragmentState {
                    module: &beam_sh,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format: SCENE_FMT, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        // dst = dst * constant, with the constant set per frame (the fade factor of each channel)
        let scale = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::Constant, operation: wgpu::BlendOperation::Add };
        let fade = offscreen("scope_fade", &fade_layout, "vs_fade", "fs_fade", Some(wgpu::BlendState { color: scale, alpha: scale }));
        let beam = offscreen("scope_beam", &beam_layout, "vs_beam", "fs_beam", ADDITIVE);

        // the screen pass joins the scene's single-sample pass
        let screen = Pipe { depth_write: false, always: true, blend: None, msaa: false };
        let comp = ctx.pipeline_with(&comp_layout, &comp_sh, "scope_comp", "vs_scope", "fs_scope", screen);

        let ring = dev.create_texture(&wgpu::TextureDescriptor {
            label: Some("scope_wave"),
            size: wgpu::Extent3d { width: RING, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let ubuf = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scope_u"),
            size: std::mem::size_of::<ScopeU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let ring_view = ring.create_view(&Default::default());
        let beam_bg = dev.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scope_beam_bg"),
            layout: &beam_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&ring_view) },
                wgpu::BindGroupEntry { binding: 1, resource: ubuf.as_entire_binding() },
            ],
        });
        Vectorscope {
            fade, beam, comp, comp_bgl, ring, ubuf, beam_bg,
            persist: None,
            tracer: Tracer::new(),
            head: 0,
            prev_valid: false,
            last_frame: 0,
            last_time: 0.0,
            clear: true,
        }
    }

    fn ensure_persist(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if self.persist.as_ref().map_or(false, |p| p.size == size) {
            return;
        }
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("scope_persist"),
            size: wgpu::Extent3d { width: size[0].max(1), height: size[1].max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SCENE_FMT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        let comp_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scope_comp_bg"),
            layout: &self.comp_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: self.ubuf.as_entire_binding() },
            ],
        });
        self.persist = Some(Persist { size, view, comp_bg, _tex: tex });
        self.clear = true;
    }

    /// Append texels to the ring, wrapping around its end.
    fn write_ring(&mut self, queue: &wgpu::Queue, texels: &[[f32; 4]]) {
        let mut done = 0;
        while done < texels.len() {
            let at = (self.head as usize + done) % RING as usize;
            let n = (RING as usize - at).min(texels.len() - done);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &self.ring, mip_level: 0, origin: wgpu::Origin3d { x: at as u32, y: 0, z: 0 }, aspect: wgpu::TextureAspect::All },
                bytemuck::cast_slice(&texels[done..done + n]),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(n as u32 * 16), rows_per_image: Some(1) },
                wgpu::Extent3d { width: n as u32, height: 1, depth_or_array_layers: 1 },
            );
            done += n;
        }
    }
}

impl Scene for Vectorscope {
    fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, f: &Frame) {
        let [w, h] = f.size;
        self.ensure_persist(device, [w, h]);
        if f.frame_no == self.last_frame {
            return; // already advanced for this frame (a crossfade between two looks of this scene)
        }
        // a gap in frames means samples were missed: the beam starts afresh instead of joining across the gap
        let continuous = self.last_frame != 0 && f.frame_no == self.last_frame + 1;
        let dt = if continuous { (f.p.time - self.last_time).clamp(1.0e-4, 0.1) } else { 1.0 / 60.0 };
        self.last_frame = f.frame_no;
        self.last_time = f.p.time;
        if !continuous {
            self.prev_valid = false;
            self.tracer.reset();
        }

        let samples = if f.wave.len() > SEG_CAP { &f.wave[f.wave.len() - SEG_CAP..] } else { f.wave };
        let mut texels = Vec::with_capacity(samples.len());
        self.tracer.process(samples, dt, &mut texels);
        let n = texels.len() as u32;
        let (first, count) = if self.prev_valid { ((self.head + RING - 1) % RING, n) } else { (self.head, n.saturating_sub(1)) };
        self.write_ring(queue, &texels);
        self.head = (self.head + n) % RING;
        self.prev_valid |= n > 0;

        let lay = layout(w as f32, h as f32);
        let tau = f.p.persist.clamp(0.02, 2.0);
        let sigma = (0.0009 * h as f32).max(0.9);
        let u = ScopeU {
            area: [lay.centre[0], lay.centre[1], lay.radius, self.tracer.xy.gain()],
            left: lay.left,
            right: lay.right,
            // each sample deposits BEAM x (1 / sample rate) / tau, so a stationary beam settles at BEAM whatever tau is
            beam: [sigma, sigma * 4.5, BEAM_FAST / (RATE * tau), BEAM_SLOW / (RATE * tau * SLOW_FACTOR)],
            seg: [first, count, RING, 0],
            trace: [self.tracer.lr.gain(), 0.8, 0.02, 0.0],
        };
        queue.write_buffer(&self.ubuf, 0, bytemuck::bytes_of(&u));

        let Some(p) = &self.persist else { return };
        let clear = std::mem::take(&mut self.clear);
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("scope_persist"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &p.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: if clear { wgpu::LoadOp::Clear(wgpu::Color::BLACK) } else { wgpu::LoadOp::Load }, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: f.ts.clone(),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if !clear {
            let keep = |t: f32| (-dt / t).exp() as f64;
            pass.set_pipeline(&self.fade);
            pass.set_blend_constant(wgpu::Color { r: keep(tau), g: keep(tau * SLOW_FACTOR), b: 1.0, a: 1.0 });
            pass.draw(0..3, 0..1);
        }
        if count > 0 {
            pass.set_pipeline(&self.beam);
            pass.set_bind_group(0, f.bg, &[]);
            pass.set_bind_group(1, &self.beam_bg, &[]);
            pass.draw(0..6, 0..count * 3);
        }
    }

    fn uses_update(&self, _p: &SceneParams) -> bool {
        true
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, _f: &Frame) {
        let Some(p) = &self.persist else { return };
        pass.set_pipeline(&self.comp);
        pass.set_bind_group(1, &p.comp_bg, &[]);
        pass.draw(0..3, 0..1);
    }

    fn wants_msaa(&self) -> bool {
        false // the beam is anti-aliased analytically
    }

    fn camera(&self, _aspect: f32, _p: &SceneParams) -> (Mat4, Vec3) {
        (Mat4::IDENTITY, Vec3::ZERO) // a flat instrument
    }

    fn reset(&mut self, _seed: u64) {
        self.clear = true;
        self.tracer.reset();
        self.prev_valid = false;
        self.last_frame = 0;
    }

    fn resize(&mut self, _device: &wgpu::Device, _size: [u32; 2]) {
        self.persist = None; // recreated, empty, on the next frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(f: f32, n: usize, start: usize) -> Vec<[f32; 2]> {
        (0..n).map(|i| {
            let s = (std::f32::consts::TAU * f * (start + i) as f32 / RATE).sin() * 0.5;
            [s, s]
        }).collect()
    }

    #[test]
    fn a_sweep_starts_on_a_rising_crossing_and_runs_one_pass() {
        let mut t = Tracer::new();
        let mut out = Vec::new();
        t.process(&sine(200.0, 4800, 0), 0.1, &mut out);
        // the first sample that starts a sweep has x = 0 and the sample before it is blanked
        let start = out.iter().position(|o| o[2] == 0.0).expect("a sweep starts");
        assert!(start > 0 && out[start - 1][2] < 0.0);
        // x then rises by one step per sample until the pass ends
        for k in 1..SWEEP_N as usize {
            let (a, b) = (out[start + k - 1][2], out[start + k][2]);
            assert!(b > a && (b - a - 1.0 / SWEEP_N as f32).abs() < 1e-5, "x not linear at {k}: {a} -> {b}");
        }
        // the trigger sample was a rising zero crossing
        assert!(out[start][0] >= 0.0 && out[start - 1][0] < 0.0);
    }

    #[test]
    fn steady_tone_retriggers_at_the_same_phase() {
        let mut t = Tracer::new();
        let mut out = Vec::new();
        t.process(&sine(240.0, 48_000, 0), 1.0, &mut out);
        let starts: Vec<usize> = out.iter().enumerate().filter(|(_, o)| o[2] == 0.0).map(|(i, _)| i).collect();
        assert!(starts.len() > 20, "{} sweeps", starts.len());
        for s in starts {
            // every sweep begins within a sample of a zero crossing of the tone
            assert!(out[s][0].abs() < 0.03, "starts at {}", out[s][0]);
        }
    }

    #[test]
    fn silence_free_runs_instead_of_going_blank() {
        let mut t = Tracer::new();
        let mut out = Vec::new();
        t.process(&vec![[0.0, 0.0]; 20_000], 0.4, &mut out);
        assert!(out.iter().any(|o| o[2] >= 0.0), "no trace is drawn on silence");
    }

    #[test]
    fn follower_fills_the_screen_and_recovers_slowly() {
        let mut f = Follower { peak: 0.0 };
        f.observe(0.05, 0.016);
        assert!((f.gain() - 16.0).abs() < 0.1);
        f.observe(1.4, 0.016); // a loud burst: the gain drops at once
        assert!(f.gain() < 0.6);
        for _ in 0..60 {
            f.observe(0.0, 0.1); // six seconds of nothing
        }
        assert!(f.gain() > 1.5, "gain only {}", f.gain());
        let mut quiet = Follower { peak: 0.0 };
        quiet.observe(1e-6, 0.016);
        assert!(quiet.gain() <= 24.0); // noise floor never blows up
    }

    #[test]
    fn mono_is_a_vertical_line_and_traces_hide_in_narrow_windows() {
        // X = (R - L)/sqrt2 is zero for mono
        let (l, r) = (0.4f32, 0.4f32);
        assert!(((r - l) * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        let wide = layout(5120.0, 1440.0);
        assert!(wide.left[2] > wide.left[0] && wide.right[2] > wide.right[0]);
        assert!(wide.left[2] < wide.centre[0] - wide.radius);
        let narrow = layout(2560.0, 1440.0);
        assert!(narrow.left[2] <= narrow.left[0], "16:9 shows the scope alone");
    }
}
