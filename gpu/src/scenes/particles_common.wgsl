// Shared by every particle simulation and every particle draw. One storage buffer holds the particles;
// a ring of spawn slots decides which ones are (re)born each frame, so emission is bounded by the CPU and
// randomness comes from a hash of each spawn's serial number (deterministic for a given sequence of frames).
struct Particle {
  pos: vec3<f32>,
  life: f32,            // seconds left; a particle with life <= 0 is dead
  vel: vec3<f32>,
  max_life: f32,
  look: vec4<f32>,      // scene-defined: colour position, size, level, and a scene-specific channel
};

struct PartU {
  step: vec4<f32>,      // dt, time, unused, unused
  ring: vec4<u32>,      // ring size, ring head before this frame's spawns, first live slot, live span
  seed: vec4<u32>,      // seed, serial of the first spawn this frame, continuous spawn count, total spawn count
  bursts: array<vec4<f32>, 6>,   // per burst: where (scene-defined), strength, kind, particle count
  extra: array<vec4<f32>, 4>,    // scene-defined
};

fn pcg(v: u32) -> u32 {
  let s = v * 747796405u + 2891336453u;
  let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
  return (w >> 22u) ^ w;
}
// Uniform 0..1 from a spawn serial and a stream index.
fn rnd(serial: u32, k: u32) -> f32 {
  return f32(pcg(serial * 16u + k + 0x9e3779b9u) >> 8u) * (1.0 / 16777216.0);
}

// A gentle divergence-free wind: the curl of a vector potential made of sines.
fn curl_wind(p: vec3<f32>, t: f32) -> vec3<f32> {
  let a = sin(1.7 * p.y + 1.1 * p.z + t);
  let b = sin(1.3 * p.z + 0.7 * p.x + 2.0 + 0.8 * t);
  let c = sin(1.9 * p.x + 0.9 * p.y + 4.0 + 0.6 * t);
  let da_dz = 1.1 * cos(1.7 * p.y + 1.1 * p.z + t);
  let db_dz = 1.3 * cos(1.3 * p.z + 0.7 * p.x + 2.0 + 0.8 * t);
  let dc_dy = 0.9 * cos(1.9 * p.x + 0.9 * p.y + 4.0 + 0.6 * t);
  let da_dy = 1.7 * cos(1.7 * p.y + 1.1 * p.z + t);
  let db_dx = 0.7 * cos(1.3 * p.z + 0.7 * p.x + 2.0 + 0.8 * t);
  let dc_dx = 1.9 * cos(1.9 * p.x + 0.9 * p.y + 4.0 + 0.6 * t);
  let _unused = a + b + c;
  return vec3<f32>(dc_dy - db_dz, da_dz - dc_dx, db_dx - da_dy);
}
