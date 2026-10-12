use super::*;
use std::collections::VecDeque;
use std::ops::Range;

/// Bytes per particle: position and life, velocity and total life, and four scene-defined values.
const PARTICLE_BYTES: u64 = 48;
/// A spawn plan never asks for more than this fraction of the ring in one frame.
const MAX_SPAWN_FRACTION: u32 = 4;
/// A scene that was not updated for this many frames starts again from an empty sky.
const STALE_FRAMES: u64 = 20;

/// Mirrors `PartU` in `particles_common.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PartU {
    step: [f32; 4],
    ring: [u32; 4],
    seed: [u32; 4],
    bursts: [[f32; 4]; 6],
    extra: [[f32; 4]; 4],
}

/// What to emit this frame: how many continuous spawns, and up to six bursts as (where, strength, kind, count).
#[derive(Default)]
pub struct SpawnPlan {
    pub cont: u32,
    pub bursts: Vec<[f32; 4]>,
}

/// A GPU particle system: one storage buffer used as a ring of particles, a compute pass that
/// integrates them and (re)spawns the slots the CPU says to, and instanced streaks to draw them.
///
/// Emission is bounded by the CPU: each frame it hands out the next `n` ring slots, and a particle born
/// in a slot lives at most `max_life` seconds, so everything that can be alive sits in a contiguous
/// stretch of the ring behind the head. Both the compute pass and the draw cover only that stretch.
pub struct Particles {
    count: u32,
    max_life: f32,
    buf: wgpu::Buffer,
    ubuf: wgpu::Buffer,
    pipe: wgpu::ComputePipeline,
    comp_bg: wgpu::BindGroup,
    draw_bg: wgpu::BindGroup,
    /// Pipeline layout for the draw pipelines: group 0 the scene uniforms, group 1 the particles.
    pub draw_layout: wgpu::PipelineLayout,
    head: u32,
    serial: u32,
    seed: u32,
    spawned: VecDeque<(f32, u32)>,
    span: u32,
    last_frame: u64,
    last_time: f32,
    clear: bool,
    /// Instance ranges (start, length) of the particles that may be alive; the second is the wrapped part.
    ranges: [(u32, u32); 2],
}

/// The stretch of the ring behind `head` that can hold live particles, as (start, length) with the
/// wrapped part second.
fn live_ranges(count: u32, head: u32, span: u32) -> [(u32, u32); 2] {
    let start = (head + count - span) % count;
    let first = span.min(count - start);
    [(start, first), (0, span - first)]
}

impl Particles {
    /// `sim_src` is the scene's simulation shader (it follows `particles_common.wgsl` and
    /// `particles_sim.wgsl`) with a compute entry point `entry`.
    pub fn new(ctx: &Ctx, label: &str, sim_src: &str, entry: &str, count: u32, max_life: f32) -> Particles {
        let dev = ctx.device;
        let src = format!("{}\n{}\n{}", include_str!("particles_common.wgsl"), include_str!("particles_sim.wgsl"), sim_src);
        let sim = ctx.module(&format!("{label}_sim"), &src);

        let entry_of = |binding, visibility, access: Option<bool>| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: match access {
                Some(read_only) => wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None },
                None => wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            },
            count: None,
        };
        let comp_bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("part_comp_bgl"),
            entries: &[entry_of(0, wgpu::ShaderStages::COMPUTE, Some(false)), entry_of(1, wgpu::ShaderStages::COMPUTE, None)],
        });
        let draw_bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("part_draw_bgl"),
            entries: &[entry_of(0, wgpu::ShaderStages::VERTEX, Some(true)), entry_of(1, wgpu::ShaderStages::VERTEX, None)],
        });
        let comp_layout = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(ctx.bgl), Some(&comp_bgl)], immediate_size: 0 });
        let draw_layout = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(ctx.bgl), Some(&draw_bgl)], immediate_size: 0 });
        let pipe = dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(label),
            layout: Some(&comp_layout),
            module: &sim,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        });

        let buf = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particles"),
            size: count as u64 * PARTICLE_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let ubuf = dev.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particles_u"),
            size: std::mem::size_of::<PartU>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = |layout: &wgpu::BindGroupLayout| {
            dev.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("particles_bg"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: ubuf.as_entire_binding() },
                ],
            })
        };
        let (comp_bg, draw_bg) = (bind(&comp_bgl), bind(&draw_bgl));
        Particles {
            count, max_life, buf, ubuf, pipe, comp_bg, draw_bg, draw_layout,
            head: 0, serial: 0, seed: 1,
            spawned: VecDeque::new(), span: 0,
            last_frame: 0, last_time: 0.0,
            clear: true,
            ranges: [(0, 0); 2],
        }
    }

    /// Empty the sky and restart the random sequence.
    /// Development aid (GEQ_PARTDEBUG=1): read the particle buffer back, wait for it, and print where the live
    /// particles are relative to the floor. Slow; never on in normal use.
    pub fn debug_report(&self, device: &wgpu::Device, queue: &wgpu::Queue, label: &str) {
        let bytes = self.count as u64 * PARTICLE_BYTES;
        let stage = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("part_debug"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_buffer_to_buffer(&self.buf, 0, &stage, 0, bytes);
        queue.submit([enc.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        stage.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r.is_ok());
        });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        if rx.recv().unwrap_or(false) {
            let data = stage.slice(..).get_mapped_range().expect("mapped");
            let f: &[f32] = bytemuck::cast_slice(&data);
            let per = (PARTICLE_BYTES / 4) as usize;
            let (mut live, mut below, mut low, mut min_y, mut falling_low, mut rising_low) = (0u32, 0u32, 0u32, f32::MAX, 0u32, 0u32);
            let (mut z_sum, mut z_max) = (0.0f32, 0.0f32);
            for p in f.chunks_exact(per) {
                // pos (0..3), life (3), vel (4..7), max_life (7)
                if p[3] > 0.0 && p[7] > 0.0 {
                    live += 1;
                    z_sum += p[2].abs();
                    z_max = z_max.max(p[2].abs());
                    min_y = min_y.min(p[1]);
                    if p[1] < 0.0 {
                        below += 1;
                    }
                    if p[1] < 0.06 {
                        low += 1;
                        if p[5] < 0.0 { falling_low += 1 } else { rising_low += 1 }
                    }
                }
            }
            eprintln!("[{label}] live {live}, below the floor {below}, within 0.06 of it {low} ({falling_low} falling, {rising_low} rising), lowest y {min_y:.4}, depth from the emitter line: mean {:.2}, max {:.2}", z_sum / live.max(1) as f32, z_max);
        }
    }

    pub fn reset(&mut self, seed: u64) {
        self.clear = true;
        self.seed = (seed as u32).wrapping_mul(2654435761).wrapping_add(1);
        self.serial = 0;
        self.last_frame = 0;
    }

    /// Start a frame: the seconds since the previous update to integrate, or `None` when this frame was
    /// already advanced (a crossfade between two looks of one scene). After a gap in frames the sky is
    /// emptied, since nothing that was flying then would still be.
    pub fn begin(&mut self, f: &Frame) -> Option<f32> {
        if f.frame_no == self.last_frame {
            return None;
        }
        let gap = if self.last_frame == 0 { u64::MAX } else { f.frame_no - self.last_frame };
        if gap > STALE_FRAMES {
            self.clear = true;
        }
        let dt = if gap == 1 { (f.p.time - self.last_time).clamp(1.0e-4, 0.05) } else { 1.0 / 60.0 };
        self.last_frame = f.frame_no;
        self.last_time = f.p.time;
        Some(dt)
    }

    /// Record this frame's spawns and run the compute pass. `dt` is what `begin` returned.
    pub fn advance(&mut self, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, f: &Frame, dt: f32, plan: &SpawnPlan, extra: [[f32; 4]; 4]) {
        if std::mem::take(&mut self.clear) {
            enc.clear_buffer(&self.buf, 0, None);
            self.spawned.clear();
            self.span = 0;
        }

        // spawn plan, bounded
        let cap = self.count / MAX_SPAWN_FRACTION;
        let cont = plan.cont.min(cap);
        let mut bursts = [[0f32; 4]; 6];
        let mut total = cont;
        for (slot, b) in bursts.iter_mut().zip(&plan.bursts) {
            let n = (b[3].max(0.0) as u32).min(cap.saturating_sub(total));
            *slot = [b[0], b[1], b[2], n as f32];
            total += n;
        }
        let head_before = self.head;
        let serial = self.serial;
        self.head = (self.head + total) % self.count;
        self.serial = self.serial.wrapping_add(total);

        // what can be alive: everything spawned within the longest lifetime
        self.spawned.push_back((f.p.time, total));
        while self.spawned.front().map_or(false, |(t, _)| f.p.time - t > self.max_life + dt) {
            self.spawned.pop_front();
        }
        self.span = self.spawned.iter().map(|(_, n)| *n as u64).sum::<u64>().min(self.count as u64) as u32;
        self.ranges = live_ranges(self.count, self.head, self.span);

        let u = PartU {
            step: [dt, f.p.time, 0.0, 0.0],
            ring: [self.count, head_before, self.ranges[0].0, self.span],
            seed: [self.seed, serial, cont, total],
            bursts,
            extra,
        };
        queue.write_buffer(&self.ubuf, 0, bytemuck::bytes_of(&u));
        let ts = f.ts_compute.clone();
        if self.span > 0 {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("particles"), timestamp_writes: ts });
            pass.set_pipeline(&self.pipe);
            pass.set_bind_group(0, f.bg, &[]);
            pass.set_bind_group(1, &self.comp_bg, &[]);
            pass.dispatch_workgroups(self.span.div_ceil(64), 1, 1);
        } else if ts.is_some() {
            // an empty pass still records the timestamps, so the timing slot never reads stale data
            let _ = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("particles"), timestamp_writes: ts });
        }
    }

    /// Draw the particles that may be alive with the currently set pipeline. `vertices` is the quad
    /// (0..6); instance `i` is particle `i`.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, vertices: Range<u32>) {
        pass.set_bind_group(1, &self.draw_bg, &[]);
        for (start, n) in self.ranges {
            if n > 0 {
                pass.draw(vertices.clone(), start..start + n);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::live_ranges;

    #[test]
    fn live_stretch_wraps_around_the_ring() {
        assert_eq!(live_ranges(100, 50, 20), [(30, 20), (0, 0)]);
        assert_eq!(live_ranges(100, 10, 30), [(80, 20), (0, 10)]);
        assert_eq!(live_ranges(100, 0, 100), [(0, 100), (0, 0)]);
        assert_eq!(live_ranges(100, 7, 0), [(7, 0), (0, 0)]);
    }
}
