// Chladni Sand, compute side. A grain is kicked about at random by an amount that follows the plate's
// vibration there, |a(x, y)| times the plate's energy (a velocity impulse with damping, so a grain hops in short
// arcs instead of flickering), and is drawn toward the nearest still line a = 0. Where the plate is still
// nothing moves, so sand collects on the nodal lines; when the weights of the modes change the old lines
// start to shake and the sand runs to the new ones. All rates are per second (the noise scales with sqrt(dt)),
// so the picture is the same at 60 and 120 fps.
@group(1) @binding(0) var<storage, read_write> grains: array<Grain>;
@group(1) @binding(1) var<uniform> su: SandU;
@group(1) @binding(2) var<storage, read_write> dens: array<atomic<u32>>;

const GAMMA: f32 = 30.0;        // velocity damping, 1/s
const NOISE: f32 = 4.4;         // impulse strength: gives a speed of about 0.3 plates a second at |a| = 1, energy 1
const FLOOR: f32 = 0.03;        // a little jitter even on a still line, so a settled line has some width
const KAPPA: f32 = 3.2;         // how fast a grain is drawn to the nearest still line at energy 1, 1/s
const DRIFT_CAP: f32 = 0.15;    // a grain never aims further than this (plate lengths) in one go
const BASE: f32 = 0.12;         // a resting grain's brightness (times its energy gain)

// Add a grain to the four pixels around it, in proportion to how much of it each one covers.
fn splat(plate: u32, p: vec2<f32>, body: f32, glint: f32) {
  let side = i32(su.plate[plate].z);
  let q = p * f32(side) - vec2<f32>(0.5);
  let i0 = vec2<i32>(floor(q));
  let f = q - vec2<f32>(i0);
  for (var k = 0u; k < 4u; k++) {
    let dx = i32(k & 1u);
    let dy = i32(k >> 1u);
    let x = i0.x + dx;
    let y = i0.y + dy;
    if (x < 0 || y < 0 || x >= side || y >= side) { continue; }
    let w = select(1.0 - f.x, f.x, dx == 1) * select(1.0 - f.y, f.y, dy == 1);
    let idx = (u32(side) * u32(side) * plate + u32(y) * u32(side) + u32(x)) * 2u;
    let b = u32(w * body * DENS_SCALE + 0.5);
    let g = u32(w * glint * DENS_SCALE + 0.5);
    if (b > 0u) { atomicAdd(&dens[idx], b); }
    if (g > 0u) { atomicAdd(&dens[idx + 1u], g); }
  }
}

// Start from a figure that is already formed: each grain tries random places and keeps the first one that
// lies near a nodal line, or stays where the last try landed (a loose grain) if none did.
@compute @workgroup_size(64)
fn cs_init(@builtin(global_invocation_id) gid: vec3<u32>) {
  let i = su.ids.y + gid.x;
  if (i >= su.ids.w) { return; }
  let plate = i % u32(su.view.z);
  let seed = su.ids.x;
  var p = vec2<f32>(sand_hash(seed, i, 0u), sand_hash(seed, i, 1u));
  for (var k = 0u; k < 32u; k++) {
    let q = vec2<f32>(sand_hash(seed, i, 2u + 3u * k), sand_hash(seed, i, 3u + 3u * k));
    let a = wave(plate, q).a;
    p = q;
    if (sand_hash(seed, i, 4u + 3u * k) < exp(-a * a / (2.0 * 0.06 * 0.06))) { break; }
  }
  grains[i] = Grain(p, vec2<f32>(0.0), sand_hash(seed, i, 200u), sand_hash(seed, i, 201u));
}

@compute @workgroup_size(64)
fn cs_step(@builtin(global_invocation_id) gid: vec3<u32>) {
  let i = gid.x;
  if (i >= su.ids.z) { return; }
  let plate = i % u32(su.view.z);
  let dt = su.step.x;
  let e = su.plate[plate].w;
  let shake = su.hit[plate].x;
  let seed = su.ids.x;
  var g = grains[i];
  let w = wave(plate, g.pos);

  let r = vec2<f32>(sand_hash(seed, i, 0u), sand_hash(seed, i, 1u)) * 2.0 - 1.0;
  g.vel = g.vel * exp(-GAMMA * dt) + r * (NOISE * e * shake * (FLOOR + abs(w.a)) * sqrt(dt));

  // the step that would take the grain to the nearest zero of a, followed at a rate that grows with the energy
  // (a zero that lies beyond the plate's edge does not draw: sand would pile up against the wall)
  var aim = w.a * w.grad / (dot(w.grad, w.grad) + 1.0);
  let goal = g.pos - aim;
  if (any(goal < vec2<f32>(0.0)) || any(goal > vec2<f32>(1.0))) { aim = vec2<f32>(0.0); }
  let len = length(aim);
  if (len > DRIFT_CAP) { aim *= DRIFT_CAP / len; }
  g.pos += g.vel * dt - aim * (KAPPA * e * dt / sqrt(shake));   // sand thrown up by a hit is not drawn to the lines until it lands

  // the plate's edges bounce a grain back
  if (g.pos.x < 0.0) { g.pos.x = -g.pos.x; g.vel.x = -g.vel.x; }
  if (g.pos.x > 1.0) { g.pos.x = 2.0 - g.pos.x; g.vel.x = -g.vel.x; }
  if (g.pos.y < 0.0) { g.pos.y = -g.pos.y; g.vel.y = -g.vel.y; }
  if (g.pos.y > 1.0) { g.pos.y = 2.0 - g.pos.y; g.vel.y = -g.vel.y; }
  g.pos = clamp(g.pos, vec2<f32>(0.0), vec2<f32>(1.0));

  // a grain that is moving tumbles, so its facet catches the lamp for a moment and lets go
  let speed = length(g.vel);
  g.face = fract(g.face + (sand_hash(seed, i, 2u) - 0.5) * min(speed, 1.0) * 8.0 * dt);
  grains[i] = g;

  // what the grain adds to its pixels: its own tone, a glint when the facet it lies on faces the lamp, and a flare
  // while it is thrown about and while the plate's hit glows
  let spec = pow(max(cos(g.face * 6.2831853 - su.lamp.x), 0.0), 40.0);
  let moving = clamp(speed * 2.0, 0.0, 1.0);
  let lit = (1.0 + su.hit[plate].y * (0.5 + 1.5 * moving)) * su.lamp.w;
  splat(plate, g.pos, (BASE * (0.55 + 0.9 * g.tone) + 0.14 * moving) * lit, spec * su.lamp.y * lit);
}
