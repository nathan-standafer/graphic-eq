use super::*;
use std::ops::Range;

/// Chladni Sand: three vibrating plates side by side (lows, mids, highs) covered in about half a million grains
/// of sand. Each plate's strongest bands choose the modes it vibrates in; sand is thrown about wherever the plate
/// moves and settles on the lines that stay still, which draws the classic figures. When the music changes the
/// figure dissolves and a new one forms; in silence the last one stays standing.
///
/// The grains live in one storage buffer and are never born or killed: a compute pass (`sand_sim.wgsl`) kicks each
/// one about by the plate's vibration at its place, draws it toward the nearest still line, and adds it into a
/// per-pixel density buffer (fixed-point atomics: half a million tiny quads cost 7 ms at 5120x1440, this costs
/// a fraction of one); the plate's fragment shader (`sand.wgsl`) turns the density into light. The plates' modes, energies and layout are plain
/// Rust below (`Voices`, `layout`, `drive`), unit-tested.
pub struct Sand {
    init: wgpu::ComputePipeline,
    step: wgpu::ComputePipeline,
    plates: wgpu::RenderPipeline,
    comp_bgl: wgpu::BindGroupLayout,
    draw_bgl: wgpu::BindGroupLayout,
    buf: wgpu::Buffer,
    ubuf: wgpu::Buffer,
    dens: Option<Dens>,
    voices: Voices,
    lay: Layout,
    active: u32,
    tick: u32,
    seed: u32,
    last_frame: u64,
    last_time: f32,
    /// How many of the buffer's grains have been set up (`reset` and a change in the number of plates start over).
    init_done: u32,
}

/// The per-pixel sums of the sand for one plate size: two u32 per pixel of each plate (body, glints).
struct Dens {
    side: u32,
    plates: u32,
    buf: wgpu::Buffer,
    comp_bg: wgpu::BindGroup,
    draw_bg: wgpu::BindGroup,
}

/// Mirrors `SandU` in `sand_common.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SandU {
    view: [f32; 4],
    step: [f32; 4],
    ids: [u32; 4],
    lamp: [f32; 4],
    plate: [[f32; 4]; 3],
    hit: [[f32; 4]; 3],
    mm: [[f32; 4]; 3],
    nn: [[f32; 4]; 3],
    sg: [[f32; 4]; 3],
    ww: [[f32; 4]; 3],
}

/// Bytes per grain: position, velocity, facet angle, tone.
const GRAIN_BYTES: u64 = 24;
/// Grains per plate the designed picture uses (at the reference plate size), and the buffer's room per plate:
/// the Grains setting runs from a fraction of the first to the second.
const DESIGN_PER_PLATE: f32 = 163_840.0;
const CAP_PER_PLATE: u32 = 245_760;
/// Plate side the designed grain count is for (px); smaller plates get fewer grains.
const REFERENCE_SIDE: f32 = 1200.0;
/// Grains set up per frame after a reset: the one-off pass (up to 32 tries per grain) is spread over four frames.
const INIT_CHUNK: u32 = CAP_PER_PLATE * 3 / 4;
/// The biggest a plate gets, px (it bounds the density buffer: three plates of this are 78 MB).
const MAX_SIDE: f32 = 1800.0;
/// How much light one grain adds, so that the designed density of grains builds the designed picture.
const GRAIN_ENERGY: f32 = 2.6;
/// A scene that was not updated for this many frames takes its first step as a single 1/60 s one.
const STALE_FRAMES: u64 = 20;

// ------------------------------------------------------------------------------------ the layout

/// Where the plates stand, in pixels from the top-left of the scene.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Layout {
    size: [f32; 2],
    plates: u32,
    side: f32,
    origin: [[f32; 2]; 3],
}

/// Three plates when the picture is wide enough for them side by side (a 5120x1440 display has room for 3 x 1440),
/// otherwise one in the middle. They are evenly spaced across the width.
fn layout(size: [u32; 2]) -> Layout {
    let (w, h) = (size[0].max(1) as f32, size[1].max(1) as f32);
    let plates = if w / h >= 2.6 { 3 } else { 1 };
    // whole pixels, so the sand's per-pixel sums line up with the screen's pixels exactly
    let side = (if plates == 3 { (0.84 * h).min(w / 3.5) } else { (0.84 * h).min(0.9 * w) }).min(MAX_SIDE).floor();
    let gap = (w - plates as f32 * side) / (plates as f32 + 1.0);
    let y = ((h - side) * 0.5 - 0.02 * h).round(); // a little high: the level strips hang below
    let mut origin = [[0.0; 2]; 3];
    for (k, o) in origin.iter_mut().enumerate().take(plates as usize) {
        *o = [(gap + k as f32 * (side + gap)).round(), y];
    }
    Layout { size: [w, h], plates, side, origin }
}

/// Grains per plate: the designed count at the reference plate size, scaled by the setting and by the plate's
/// area (so a small window is not a thicker layer of sand), within the buffer's room.
fn grains_per_plate(side: f32, setting: f32) -> u32 {
    let area = (side / REFERENCE_SIDE).powi(2).clamp(0.15, 1.5);
    (DESIGN_PER_PLATE * setting.clamp(0.25, 1.5) * area).clamp(4_000.0, CAP_PER_PLATE as f32) as u32
}

/// Brightness of one grain: grains per pixel is the same at every size (see `grains_per_plate`) except where its
/// clamps bite, and this makes up for that, so a small window is not brighter or a huge one dimmer.
fn grain_energy(side: f32) -> f32 {
    let wanted = (side / REFERENCE_SIDE).powi(2);
    GRAIN_ENERGY * (wanted / wanted.clamp(0.15, 1.5)).clamp(0.4, 2.5)
}

// -------------------------------------------------------------------------------- the modes

/// The mode of each EQ band as (m, n, s): higher bands vibrate in higher modes, and so draw finer figures. `s` is
/// the sign in `cos(n pi x) cos(m pi y) + s cos(m pi x) cos(n pi y)`. An antisymmetric mode (-1) always keeps the
/// diagonal x = y still, and when m + n is even it collapses into a blob at the centre, so those are avoided; the
/// mix of signs also keeps one line from being still in every figure (sand would end up piled on it).
const MODES: [(f32, f32, f32); 11] = [
    (1.0, 2.0, 1.0), // 40 Hz: an anti-diagonal and four corner arcs
    (1.0, 3.0, 1.0), // a cross
    (2.0, 3.0, 1.0), // an anti-diagonal with S-curves
    (2.0, 4.0, 1.0), // 240 Hz: a diamond around a circle
    (3.0, 4.0, -1.0),
    (2.0, 5.0, 1.0),
    (3.0, 5.0, 1.0), // a circle on a cross
    (3.0, 6.0, 1.0), // 2.7 kHz: diamonds and rings
    (4.0, 7.0, 1.0),
    (5.0, 8.0, -1.0),
    (6.0, 9.0, 1.0), // 16 kHz: rosettes
];

/// Which bands drive each plate: lows, mids, highs; or all of them on a lone plate.
fn groups(plates: u32) -> &'static [Range<usize>] {
    const THREE: &[Range<usize>] = &[0..4, 4..8, 8..11];
    const ONE: &[Range<usize>] = &[0..11];
    if plates == 3 { THREE } else { ONE }
}

/// The band whose figure a plate shows before any music has shaped it (and again after silence is over, until
/// something louder takes it over).
fn default_band(plates: u32, k: usize) -> usize {
    if plates == 3 { [3, 6, 9][k] } else { 3 }
}

const VOICES: usize = 4;
const NO_BAND: u8 = u8::MAX;
/// At most this many bands vibrate a plate at once: the figure of three modes is already busy.
const MAX_ACTIVE: usize = 3;
/// A band must be within this fraction of the strongest (after the bonus below) to take part.
const KEEP_FRACTION: f32 = 0.55;
/// A band already vibrating the plate counts this much louder than it is, so a challenger has to beat it clearly.
const INCUMBENT_BONUS: f32 = 1.3;
/// A plate whose strongest band is below this (a displayed level) keeps its figure instead of picking a new one.
const GROUP_MIN: f32 = 0.14;
/// Below this signal gate nothing about the figures changes: silence leaves the last one standing.
const GATE_MIN: f32 = 0.3;
/// How fast a mode's weight rises and falls, seconds. Slow enough that a figure dissolves and re-forms rather than flickers.
const TAU_UP: f32 = 0.30;
const TAU_DOWN: f32 = 0.55;

#[derive(Clone, Copy, Debug)]
struct Voice {
    band: u8,
    weight: f32,
}

/// The modes one plate vibrates in: four voices, each following one band. A band that becomes strong claims a free
/// voice and its weight rises; one that falls away has its weight eased to zero and then frees the voice.
#[derive(Clone, Debug)]
struct PlateState {
    voices: [Voice; VOICES],
    active: [bool; 11],
}

#[derive(Clone, Debug)]
struct Voices {
    n: u32,
    plates: [PlateState; 3],
}

impl PlateState {
    fn with_band(band: usize) -> PlateState {
        let mut s = PlateState { voices: [Voice { band: NO_BAND, weight: 0.0 }; VOICES], active: [false; 11] };
        s.voices[0] = Voice { band: band as u8, weight: 1.0 };
        s.active[band] = true;
        s
    }

    /// Choose the bands that vibrate the plate: the loudest few of its group, favouring those that already do.
    fn reselect(&mut self, group: &Range<usize>, levels: &[f32; 11]) {
        let score = |b: usize| levels[b] * if self.active[b] { INCUMBENT_BONUS } else { 1.0 };
        let mut order: Vec<usize> = group.clone().collect();
        order.sort_by(|a, b| score(*b).partial_cmp(&score(*a)).unwrap_or(std::cmp::Ordering::Equal));
        let best = score(order[0]);
        let mut now = [false; 11];
        for &b in order.iter().take(MAX_ACTIVE) {
            if score(b) >= KEEP_FRACTION * best {
                now[b] = true;
            }
        }
        self.active = now;
    }

    /// Hand every active band a voice (a free one: not active and faded out), and set each voice's target weight.
    fn assign(&mut self, levels: &[f32; 11]) -> [f32; VOICES] {
        for b in 0..11 {
            if self.active[b] && !self.voices.iter().any(|v| v.band == b as u8) {
                if let Some(v) = self.voices.iter_mut().find(|v| v.band == NO_BAND || (v.weight < 0.02 && !self.active[v.band as usize])) {
                    *v = Voice { band: b as u8, weight: 0.0 };
                }
            }
        }
        // the share of each active band: its level relative to the strongest, sharpened so a clear leader dominates
        let lead = (0..11).filter(|b| self.active[*b]).map(|b| levels[b]).fold(1.0e-6, f32::max);
        let mut target = [0.0f32; VOICES];
        let mut sum = 0.0;
        for (t, v) in target.iter_mut().zip(&self.voices) {
            if v.band != NO_BAND && self.active[v.band as usize] {
                *t = (levels[v.band as usize] / lead).clamp(0.0, 1.0).powf(2.0).max(0.05);
                sum += *t;
            }
        }
        if sum > 0.0 {
            target.iter_mut().for_each(|t| *t /= sum);
        }
        target
    }
}

impl Voices {
    fn new(plates: u32) -> Voices {
        let mk = |k: usize| PlateState::with_band(default_band(plates, k.min(plates as usize - 1)));
        Voices { n: plates, plates: [mk(0), mk(1), mk(2)] }
    }

    /// Advance the modes by `dt` seconds. While the signal gate is closed nothing changes at all.
    fn step(&mut self, levels: &[f32; 11], gate: f32, dt: f32) {
        if gate < GATE_MIN {
            return;
        }
        for (k, group) in groups(self.n).iter().enumerate() {
            let plate = &mut self.plates[k];
            let loudest = group.clone().map(|b| levels[b]).fold(0.0, f32::max);
            if loudest >= GROUP_MIN {
                plate.reselect(group, levels);
            }
            let target = plate.assign(levels);
            for (v, t) in plate.voices.iter_mut().zip(target) {
                let tau = if t > v.weight { TAU_UP } else { TAU_DOWN };
                v.weight += (t - v.weight) * (1.0 - (-dt / tau).exp());
                if v.band != NO_BAND && t == 0.0 && v.weight < 0.003 {
                    *v = Voice { band: NO_BAND, weight: 0.0 };
                }
            }
        }
    }

    /// The four modes of plate `k` as (m, n, s, weight) with the weights summing to 1.
    fn modes(&self, k: usize) -> [(f32, f32, f32, f32); VOICES] {
        let p = &self.plates[k.min(2)];
        let sum: f32 = p.voices.iter().filter(|v| v.band != NO_BAND).map(|v| v.weight).sum();
        let mut out = [(0.0, 0.0, 1.0, 0.0); VOICES];
        for (o, v) in out.iter_mut().zip(&p.voices) {
            if v.band != NO_BAND && sum > 1.0e-4 {
                let (m, n, sg) = MODES[v.band as usize];
                *o = (m, n, sg, v.weight / sum);
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------------- the drive

/// What a plate's vibration energy, shake and glow are this frame, from the signal. A plate's energy follows the
/// sustained level of its region; a hit in that region shakes it harder (Motion) and makes the sand glow (Flash).
fn plate_signals(plates: u32, k: usize, p: &SceneParams) -> (f32, f32, f32) {
    let (sus, kick) = if plates == 3 {
        (p.sus[k], p.fx[k])
    } else {
        (p.sus[..3].iter().cloned().fold(0.0, f32::max), p.fx[..3].iter().cloned().fold(0.0, f32::max))
    };
    let gate = p.gate.clamp(0.0, 1.0);
    let energy = gate * (0.10 + 0.90 * sus.clamp(0.0, 1.2).powf(0.8));
    let kick = kick.clamp(0.0, 1.5);
    (energy, 1.0 + 4.5 * p.motion.clamp(0.0, 1.5) * kick, kick * p.flash.clamp(0.0, 1.5))
}

/// The lamp's angle over the plates: a fixed place up and to the left, drifting slowly with Motion.
fn lamp_angle(time: f32, motion: f32) -> f32 {
    -2.3 + motion.clamp(0.0, 1.5) * 0.55 * (time * 0.11).sin() + motion.clamp(0.0, 1.5) * 0.25 * (time * 0.047 + 1.3).sin()
}

/// The grains to set up this frame: the next slice of the buffer, as (first, one past the last).
fn next_init(done: u32) -> (u32, u32) {
    let total = CAP_PER_PLATE * 3;
    (done.min(total), (done + INIT_CHUNK).min(total))
}

/// `grains` is how many grains move this frame, and `init` the slice of the buffer being set up.
fn drive(lay: &Layout, voices: &Voices, p: &SceneParams, dt: f32, tick: u32, seed: u32, grains: (u32, u32, u32)) -> SandU {
    let (active, init_lo, init_hi) = grains;
    let mut u = SandU::zeroed();
    let side = lay.side;
    u.view = [lay.size[0], lay.size[1], lay.plates as f32, 0.0];
    u.step = [dt, p.time, 0.0, 0.0];
    u.ids = [seed ^ tick.wrapping_mul(0x9e37_79b9), init_lo, active, init_hi];
    u.lamp = [lamp_angle(p.time, p.motion), 1.2, 0.0, grain_energy(side)];
    for k in 0..lay.plates as usize {
        let (e, shake, glow) = plate_signals(lay.plates, k, p);
        u.plate[k] = [lay.origin[k][0], lay.origin[k][1], side, e];
        u.hit[k] = [shake, glow, 0.0, 0.0];
        for (j, (m, n, sg, w)) in voices.modes(k).into_iter().enumerate() {
            u.mm[k][j] = m;
            u.nn[k][j] = n;
            u.sg[k][j] = sg;
            u.ww[k][j] = w;
        }
    }
    u
}

// ------------------------------------------------------------------------------------ the scene

impl Sand {
    pub fn new(ctx: &Ctx) -> Sand {
        let dev = ctx.device;
        let common = format!("{}\n", include_str!("sand_common.wgsl"));
        let sim_sh = ctx.module("sand_sim", &format!("{common}{}", include_str!("sand_sim.wgsl")));
        let draw_sh = ctx.module("sand", &format!("{common}{}", include_str!("sand.wgsl")));

        let entry = |binding, visibility, storage: Option<bool>| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: match storage {
                Some(read_only) => wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None },
                None => wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            },
            count: None,
        };
        let comp_bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sand_comp_bgl"),
            entries: &[entry(0, wgpu::ShaderStages::COMPUTE, Some(false)), entry(1, wgpu::ShaderStages::COMPUTE, None), entry(2, wgpu::ShaderStages::COMPUTE, Some(false))],
        });
        let draw_bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sand_draw_bgl"),
            entries: &[entry(1, wgpu::ShaderStages::VERTEX_FRAGMENT, None), entry(2, wgpu::ShaderStages::FRAGMENT, Some(true))],
        });
        let comp_layout = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(ctx.bgl), Some(&comp_bgl)], immediate_size: 0 });
        let draw_layout = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(ctx.bgl), Some(&draw_bgl)], immediate_size: 0 });
        let compute = |label: &str, entry: &str| {
            dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&comp_layout),
                module: &sim_sh,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };

        let buf = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sand_grains"),
            size: CAP_PER_PLATE as u64 * 3 * GRAIN_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let ubuf = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sand_u"),
            size: std::mem::size_of::<SandU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let flat = Pipe { depth_write: false, always: true, blend: None, msaa: false };
        Sand {
            init: compute("sand_init", "cs_init"),
            step: compute("sand_step", "cs_step"),
            plates: ctx.pipeline_with(&draw_layout, &draw_sh, "sand_plates", "vs_plate", "fs_plate", flat),
            comp_bgl, draw_bgl, buf, ubuf,
            dens: None,
            voices: Voices::new(3),
            lay: layout([5120, 1440]),
            active: 0,
            tick: 0,
            seed: 1,
            last_frame: 0,
            last_time: 0.0,
            init_done: 0,
        }
    }
}

impl Sand {
    /// The density buffer for plates of this size (made again when the size or the number of plates changes).
    fn ensure_dens(&mut self, device: &wgpu::Device, side: u32, plates: u32) {
        if self.dens.as_ref().map_or(false, |d| d.side == side && d.plates == plates) {
            return;
        }
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sand_density"),
            size: (side as u64 * side as u64 * plates as u64 * 8).max(8),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = |layout: &wgpu::BindGroupLayout, with_grains: bool| {
            let mut entries = vec![
                wgpu::BindGroupEntry { binding: 1, resource: self.ubuf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: buf.as_entire_binding() },
            ];
            if with_grains {
                entries.push(wgpu::BindGroupEntry { binding: 0, resource: self.buf.as_entire_binding() });
            }
            device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("sand_bg"), layout, entries: &entries })
        };
        let (comp_bg, draw_bg) = (bind(&self.comp_bgl, true), bind(&self.draw_bgl, false));
        self.dens = Some(Dens { side, plates, buf, comp_bg, draw_bg });
    }
}

impl Scene for Sand {
    fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, f: &Frame) {
        if f.frame_no == self.last_frame {
            return; // already advanced for this frame (a crossfade between two looks of this scene)
        }
        let continuous = self.last_frame != 0 && f.frame_no <= self.last_frame + STALE_FRAMES;
        let dt = if continuous { (f.p.time - self.last_time).clamp(0.0, 0.05) } else { 1.0 / 60.0 };
        self.last_frame = f.frame_no;
        self.last_time = f.p.time;

        // a change in the number of plates gives every grain a different plate and a different figure: start the sand over
        let lay = layout(f.size);
        if lay.plates != self.voices.n {
            self.voices = Voices::new(lay.plates);
            self.init_done = 0;
        }
        self.lay = lay;
        self.voices.step(&f.p.levels, f.p.gate, dt);
        // development aid (GEQ_SANDDEBUG=1): once a second, the levels and which modes each plate vibrates in
        if std::env::var_os("GEQ_SANDDEBUG").is_some() && self.tick % 60 == 0 {
            let lv: Vec<String> = f.p.levels.iter().map(|l| format!("{l:.2}")).collect();
            let st: Vec<String> = (0..lay.plates as usize).map(|k| self.voices.plates[k].voices.iter().filter(|v| v.band != NO_BAND).map(|v| format!("{}:{:.2}", v.band, v.weight)).collect::<Vec<_>>().join(",")).collect();
            eprintln!("t={:.1} gate={:.2} sus={:?} levels=[{}] voices {}", f.p.time, f.p.gate, f.p.sus, lv.join(" "), st.join(" | "));
        }

        let per_plate = grains_per_plate(lay.side, f.p.grains);
        self.active = per_plate * lay.plates;
        self.tick = self.tick.wrapping_add(1);
        let (lo, hi) = next_init(self.init_done);
        self.init_done = hi;
        let stepped = self.active.min(hi); // grains that are not set up yet neither move nor show
        let u = drive(&lay, &self.voices, f.p, dt, self.tick, self.seed, (stepped, lo, hi));
        queue.write_buffer(&self.ubuf, 0, bytemuck::bytes_of(&u));

        self.ensure_dens(device, lay.side as u32, lay.plates);
        let Some(dens) = &self.dens else { return };
        enc.clear_buffer(&dens.buf, 0, None);

        if hi > lo {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("sand_init"), timestamp_writes: None });
            pass.set_pipeline(&self.init);
            pass.set_bind_group(0, f.bg, &[]);
            pass.set_bind_group(1, &dens.comp_bg, &[]);
            pass.dispatch_workgroups((hi - lo).div_ceil(64), 1, 1);
        }
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("sand_step"), timestamp_writes: f.ts_compute.clone() });
        pass.set_pipeline(&self.step);
        pass.set_bind_group(0, f.bg, &[]);
        pass.set_bind_group(1, &dens.comp_bg, &[]);
        pass.dispatch_workgroups(stepped.div_ceil(64), 1, 1);
    }

    fn uses_update(&self, _p: &SceneParams) -> bool {
        true
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, _f: &Frame) {
        let Some(dens) = &self.dens else { return };
        if self.active == 0 {
            return;
        }
        pass.set_bind_group(1, &dens.draw_bg, &[]);
        pass.set_pipeline(&self.plates);
        pass.draw(0..6, 0..self.lay.plates);
    }

    fn wants_msaa(&self) -> bool {
        false // flat plates, and sand that is resolved per pixel
    }

    fn camera(&self, _aspect: f32, _p: &SceneParams) -> (Mat4, Vec3) {
        (Mat4::IDENTITY, Vec3::ZERO) // a flat picture
    }

    fn resize(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        // the sand lies in plate coordinates, so a new size keeps it; only the per-pixel sums depend on the size
        let lay = layout(size);
        self.ensure_dens(device, lay.side as u32, lay.plates);
    }

    fn reset(&mut self, seed: u64) {
        self.init_done = 0;
        self.seed = (seed as u32).wrapping_mul(2654435761).wrapping_add(1);
        self.voices = Voices::new(self.lay.plates);
        self.last_frame = 0;
        self.tick = 0;
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
            spectrum: [0.0; crate::analyzer::CURVE_POINTS],
            rise: [0.0; crate::analyzer::CURVE_POINTS],
        }
    }

    /// The plate's pattern for one mode and its gradient, as the shaders compute them.
    fn mode(m: f32, n: f32, s: f32, x: f32, y: f32) -> (f32, [f32; 2]) {
        use std::f32::consts::PI;
        let a = (n * PI * x).cos() * (m * PI * y).cos() + s * (m * PI * x).cos() * (n * PI * y).cos();
        let gx = PI * (-n * (n * PI * x).sin() * (m * PI * y).cos() - s * m * (m * PI * x).sin() * (n * PI * y).cos());
        let gy = PI * (-m * (n * PI * x).cos() * (m * PI * y).sin() - s * n * (m * PI * x).cos() * (n * PI * y).sin());
        (a, [gx, gy])
    }

    fn run(v: &mut Voices, levels: &[f32; 11], secs: f32) {
        for _ in 0..(secs * 120.0) as usize {
            v.step(levels, 1.0, 1.0 / 120.0);
        }
    }

    fn weight_of(v: &Voices, plate: usize, band: usize) -> f32 {
        v.plates[plate].voices.iter().find(|x| x.band == band as u8).map_or(0.0, |x| x.weight)
    }

    #[test]
    fn the_gradient_in_the_shader_is_the_derivative_of_the_pattern() {
        for (m, n, s) in MODES {
            for &(x, y) in &[(0.13, 0.71), (0.5, 0.2), (0.93, 0.37)] {
                let (_, g) = mode(m, n, s, x, y);
                let h = 1.0e-3;
                let dx = (mode(m, n, s, x + h, y).0 - mode(m, n, s, x - h, y).0) / (2.0 * h);
                let dy = (mode(m, n, s, x, y + h).0 - mode(m, n, s, x, y - h).0) / (2.0 * h);
                assert!((g[0] - dx).abs() < 0.02 * (1.0 + dx.abs()) && (g[1] - dy).abs() < 0.02 * (1.0 + dy.abs()), "({m},{n}) at ({x},{y}): {g:?} vs ({dx},{dy})");
            }
        }
    }

    #[test]
    fn modes_are_well_formed_and_no_line_is_still_in_every_figure() {
        for (m, n, s) in MODES {
            assert!(m < n, "a mode with m = n is flat");
            assert!(s == 1.0 || s == -1.0);
            // an antisymmetric mode with m + n even collapses into a blob at the centre
            assert!(s == 1.0 || (m + n) % 2.0 == 1.0, "({m},{n}) with s = -1 would be a blob");
        }
        // the diagonal x = y is still only for antisymmetric modes: not for every figure of a plate
        let still = |m, n, s| mode(m, n, s, 0.37, 0.37).0.abs() < 1.0e-5;
        assert!(MODES.iter().any(|(m, n, s)| still(*m, *n, *s)) && MODES.iter().any(|(m, n, s)| !still(*m, *n, *s)));
        for plate in groups(3) {
            let modes: Vec<_> = plate.clone().map(|b| MODES[b]).collect();
            assert!(modes.iter().any(|(m, n, s)| !still(*m, *n, *s)), "plate {plate:?} would always keep the diagonal still");
        }
    }

    #[test]
    fn higher_bands_vibrate_in_higher_modes() {
        for b in 1..11 {
            let (m0, n0, _) = MODES[b - 1];
            let (m1, n1, _) = MODES[b];
            assert!(m1 + n1 >= m0 + n0, "band {b} must not be simpler than band {}", b - 1);
            assert!((m0, n0) != (m1, n1), "bands {} and {b} need different figures", b - 1);
        }
    }

    #[test]
    fn three_plates_on_a_wide_display_and_one_on_a_narrow_window() {
        let l = layout([5120, 1440]);
        assert_eq!(l.plates, 3);
        assert!(l.side * 3.0 < 5120.0 && l.side <= 0.84 * 1440.0 + 1e-3);
        let gaps = [l.origin[0][0], l.origin[1][0] - (l.origin[0][0] + l.side), l.origin[2][0] - (l.origin[1][0] + l.side), 5120.0 - (l.origin[2][0] + l.side)];
        assert!(gaps.iter().all(|g| (g - gaps[0]).abs() <= 1.5 && *g > 0.1 * l.side), "evenly spaced (to a pixel) with room between: {gaps:?}");
        assert!(l.side.fract() == 0.0 && l.origin.iter().all(|o| o[0].fract() == 0.0 && o[1].fract() == 0.0), "whole pixels");
        assert_eq!(layout([1600, 450]).plates, 3, "the thumbnails show all three");
        let narrow = layout([1920, 1080]);
        assert_eq!(narrow.plates, 1);
        assert!((narrow.origin[0][0] * 2.0 + narrow.side - 1920.0).abs() <= 1.5, "centred");
        assert_eq!(layout([2400, 1080]).plates, 1, "21:9 is too narrow for three");
    }

    #[test]
    fn a_small_window_gets_proportionally_fewer_grains() {
        let big = grains_per_plate(1210.0, 1.0);
        let small = grains_per_plate(378.0, 1.0);
        assert!(big > 150_000 && big <= CAP_PER_PLATE);
        assert!(small < big / 4, "{small} vs {big}");
        assert!(grains_per_plate(1210.0, 0.5) < big && grains_per_plate(1210.0, 5.0) <= CAP_PER_PLATE);
    }

    #[test]
    fn the_strongest_bands_take_over_gradually_and_the_weights_sum_to_one() {
        let mut v = Voices::new(3);
        // the highs plate starts on band 9; a loud band 8 and a quieter 10 take over
        let mut lv = [0.0f32; 11];
        lv[8] = 0.8;
        lv[10] = 0.5;
        run(&mut v, &lv, 0.1);
        assert!(weight_of(&v, 2, 8) < 0.5, "the new mode has barely begun to rise after 0.1 s");
        run(&mut v, &lv, 4.0);
        assert!(weight_of(&v, 2, 8) > 0.5 && weight_of(&v, 2, 10) > 0.2 && weight_of(&v, 2, 9) < 0.01, "after a few seconds: {:?}", v.plates[2].voices);
        let modes = v.modes(2);
        assert!((modes.iter().map(|m| m.3).sum::<f32>() - 1.0).abs() < 1e-4);
        assert!(modes.iter().filter(|m| m.3 > 0.0).count() <= MAX_ACTIVE);
        // a band outside the plate's group never takes part
        assert!(v.plates[2].voices.iter().all(|x| x.band == NO_BAND || (8..11).contains(&(x.band as usize))));
    }

    #[test]
    fn a_small_wobble_in_the_levels_does_not_change_the_figure() {
        let mut v = Voices::new(3);
        let mut lv = [0.0f32; 11];
        lv[1] = 0.7;
        lv[2] = 0.6;
        run(&mut v, &lv, 4.0);
        let before: Vec<(u8, f32)> = v.plates[0].voices.iter().map(|x| (x.band, x.weight)).collect();
        // the two bands trade places by a few percent, back and forth, for a while
        for k in 0..600 {
            let t = if k % 40 < 20 { 0.05 } else { -0.05 };
            lv[1] = 0.65 + t;
            lv[2] = 0.65 - t;
            v.step(&lv, 1.0, 1.0 / 120.0);
        }
        let bands = |v: &Voices| {
            let mut b: Vec<u8> = v.plates[0].voices.iter().filter(|x| x.band != NO_BAND && x.weight > 0.01).map(|x| x.band).collect();
            b.sort();
            b
        };
        assert_eq!(bands(&v), vec![1, 2], "still the same two modes: {before:?} -> {:?}", v.plates[0].voices);
        // the weights only wander a little
        assert!((weight_of(&v, 0, 1) - 0.5).abs() < 0.25);
    }

    #[test]
    fn a_challenger_must_clearly_beat_the_incumbent() {
        let mut v = Voices::new(3);
        let mut lv = [0.0f32; 11];
        lv[1] = 0.6;
        run(&mut v, &lv, 3.0);
        lv[1] = 0.6;
        lv[3] = 0.65; // a hair louder: not enough to push band 1 aside, but within reach to join
        run(&mut v, &lv, 3.0);
        assert!(weight_of(&v, 0, 1) > 0.3, "the incumbent keeps its place");
        lv[3] = 1.0;
        lv[1] = 0.3;
        run(&mut v, &lv, 4.0);
        assert!(weight_of(&v, 0, 3) > 0.5 && weight_of(&v, 0, 1) < 0.2, "a clear challenger takes over: {:?}", v.plates[0].voices);
    }

    #[test]
    fn silence_leaves_the_last_figure_standing() {
        let mut v = Voices::new(3);
        let mut lv = [0.0f32; 11];
        lv[0] = 0.9;
        lv[3] = 0.7;
        run(&mut v, &lv, 4.0);
        let frozen: Vec<(f32, f32, f32, f32)> = v.modes(0).to_vec();
        // the gate closes and the levels fall to nothing
        for _ in 0..1200 {
            v.step(&[0.0; 11], 0.0, 1.0 / 120.0);
        }
        assert_eq!(v.modes(0).to_vec(), frozen);
        // quiet noise with the gate shut changes nothing either
        for _ in 0..600 {
            v.step(&[0.05; 11], 0.1, 1.0 / 120.0);
        }
        assert_eq!(v.modes(0).to_vec(), frozen);
    }

    #[test]
    fn a_plate_with_nothing_loud_in_its_region_keeps_its_figure() {
        let mut v = Voices::new(3);
        let mut lv = [0.0f32; 11];
        lv[0] = 0.9; // only the lows play
        run(&mut v, &lv, 5.0);
        assert_eq!(v.plates[2].voices[0].band, 9, "the highs plate has not been reshaped by noise");
        assert!((v.modes(2)[0].3 - 1.0).abs() < 1e-4);
    }

    #[test]
    fn a_lone_plate_chooses_from_all_the_bands() {
        let mut v = Voices::new(1);
        let mut lv = [0.0f32; 11];
        lv[9] = 0.9;
        lv[2] = 0.8;
        run(&mut v, &lv, 4.0);
        assert!(weight_of(&v, 0, 9) > 0.3 && weight_of(&v, 0, 2) > 0.3);
        assert_eq!(groups(1), &[0..11]);
    }

    #[test]
    fn energy_follows_the_region_and_a_hit_shakes_and_lights_the_sand() {
        let mut p = params();
        p.sus = [0.9, 0.2, 0.0, 0.0];
        let (e_low, shake, glow) = plate_signals(3, 0, &p);
        let (e_mid, _, _) = plate_signals(3, 1, &p);
        let (e_high, _, _) = plate_signals(3, 2, &p);
        assert!(e_low > e_mid && e_mid > e_high && e_high > 0.0, "{e_low} {e_mid} {e_high}");
        assert_eq!((shake, glow), (1.0, 0.0), "no hit, no shake or glow");
        p.fx = [1.0, 0.0, 0.0, 0.0];
        let (_, shake, glow) = plate_signals(3, 0, &p);
        let (_, other, other_glow) = plate_signals(3, 1, &p);
        assert!(shake > 2.0 && glow > 0.5 && other == 1.0 && other_glow == 0.0, "only the lows plate feels a kick");
        // Motion 0 stops the shaking, Flash 0 stops the glow
        p.motion = 0.0;
        p.flash = 0.0;
        let (_, shake, glow) = plate_signals(3, 0, &p);
        assert_eq!((shake, glow), (1.0, 0.0));
        // a lone plate feels every region
        p.motion = 0.6;
        p.flash = 0.8;
        p.fx = [0.0, 0.0, 1.0, 0.0];
        assert!(plate_signals(1, 0, &p).1 > 2.0);
        // a closed gate stills everything
        p.gate = 0.0;
        assert_eq!(plate_signals(3, 0, &p).0, 0.0);
    }

    #[test]
    fn the_setup_pass_covers_every_grain_once_over_four_frames() {
        let total = CAP_PER_PLATE * 3;
        let mut done = 0;
        let mut slices = Vec::new();
        loop {
            let (lo, hi) = next_init(done);
            if hi == lo {
                break;
            }
            assert_eq!(lo, done, "no gap and no overlap");
            slices.push((lo, hi));
            done = hi;
        }
        assert_eq!(slices.len(), 4);
        assert_eq!(done, total);
        assert_eq!(next_init(total), (total, total), "nothing more to set up afterwards");
    }

    #[test]
    fn the_uniform_matches_the_shader_struct() {
        assert_eq!(std::mem::size_of::<SandU>(), 16 * (4 + 18));
    }

    #[test]
    fn the_drive_carries_the_layout_and_the_modes() {
        let lay = layout([5120, 1440]);
        let v = Voices::new(3);
        let u = drive(&lay, &v, &params(), 1.0 / 120.0, 7, 3, (100, 10, 20));
        assert_eq!(u.view[2], 3.0);
        assert_eq!(u.ids[1..], [10, 100, 20]);
        assert_eq!(u.plate[2][0], lay.origin[2][0]);
        assert_eq!((u.mm[1][0], u.nn[1][0], u.sg[1][0], u.ww[1][0]), (MODES[6].0, MODES[6].1, MODES[6].2, 1.0));
    }
}
