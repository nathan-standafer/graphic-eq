// The compute side of a particle system. A scene's simulation shader includes this and supplies
// `integrate` and `spawn`, and a one-line entry point that calls `run_particles`.
@group(1) @binding(0) var<storage, read_write> parts: array<Particle>;
@group(1) @binding(1) var<uniform> pu: PartU;

// Which burst a spawn slot belongs to (the slot's offset after the continuous spawns), and the
// particle's index within it. Returns 6 when the slot is past every burst.
struct BurstSlot { k: u32, n: u32 }
fn find_burst(offset: u32) -> BurstSlot {
  var rem = offset;
  for (var k = 0u; k < 6u; k++) {
    let c = u32(pu.bursts[k].w);
    if (rem < c) { return BurstSlot(k, rem); }
    rem -= c;
  }
  return BurstSlot(6u, 0u);
}

const WG: u32 = 64u;

// The scene supplies:
//   fn spawn_particle(serial: u32, burst: u32, n: u32) -> Particle   (burst = 6 for a continuous spawn)
//   fn integrate(p: Particle, dt: f32) -> Particle
@compute @workgroup_size(64)
fn cs_main(@builtin(global_invocation_id) gid: vec3<u32>) {
  if (gid.x >= pu.ring.w) { return; }
  let n = pu.ring.x;
  let i = (pu.ring.z + gid.x) % n;
  let slot = (i + n - pu.ring.y) % n;
  var p = parts[i];
  if (slot < pu.seed.w) {
    var b = BurstSlot(6u, 0u);
    if (slot >= pu.seed.z) { b = find_burst(slot - pu.seed.z); }
    let serial = (pu.seed.y + slot) * 2654435761u ^ pu.seed.x;
    p = spawn_particle(serial, b.k, b.n);
  } else if (p.life > 0.0) {
    p = integrate(p, pu.step.x);
  } else {
    return;
  }
  parts[i] = p;
}
