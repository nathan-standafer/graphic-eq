// The fireworks' floor, rockets and stars. The floor is the fountain's glossy dark glass with the spectrum line
// the pads stand on; each live burst throws a radial pool of light on it. Rockets are thin rising streaks drawn
// from a small table the CPU keeps (they are closed-form arcs, so it knows where each is at any moment). Stars
// are the compute pass's particles, drawn as streaks whose length depends on the type of burst, and mirrored in
// the glass.
const FLOOR_EXTENT: f32 = 600.0;
const EMBER: vec3<f32> = vec3<f32>(1.0, 0.30, 0.05);

// 16 rockets then 8 pools, two vec4 each:
//   rocket: (position, brightness) (velocity, frequency)
//   pool:   (x, z, age, amplitude) (frequency, radius, life, unused)
struct FwU {
  shells: array<vec4<f32>, 32>,
  pools: array<vec4<f32>, 16>,
};
@group(1) @binding(2) var<uniform> fw: FwU;

struct FloorOut { @builtin(position) pos: vec4<f32>, @location(0) wp: vec3<f32> };
@vertex fn vs_floor(@builtin(vertex_index) vi: u32) -> FloorOut {
  var q = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0));
  let c = q[vi];
  let w = vec3<f32>(mix(-FLOOR_EXTENT, FLOOR_EXTENT, c.x), 0.0, mix(-FLOOR_EXTENT, FLOOR_EXTENT, c.y));
  return FloorOut(u.vp * vec4<f32>(w, 1.0), w);
}

@fragment fn fs_floor(in: FloorOut) -> @location(0) vec4<f32> {
  let dist = length(in.wp - u.cam.xyz);
  let fade = 1.0 - smoothstep(18.0, 120.0, dist);
  let aa = line_aa(max(fwidth(in.wp.x), fwidth(in.wp.z)));
  let g = max(gridline(in.wp.x), gridline(in.wp.z)) * aa;
  var c = vec3<f32>(0.007, 0.008, 0.015) + palette(0.35, 0.5) * g * 0.05;

  // the pools of light under the live bursts: a quick flash when the shell goes off, then a slower glow that
  // rises while the stars fall toward the glass and fades with them. Flash scales all of it.
  var lit = vec3<f32>(0.0);
  for (var k = 0u; k < 8u; k++) {
    let a = fw.pools[2u * k];
    let b = fw.pools[2u * k + 1u];
    if (a.w <= 0.0) { continue; }
    let sg = max(0.55 * b.y, 0.6);
    let d = in.wp.xz - a.xy;
    let d2 = dot(d, d);
    if (d2 > 14.0 * sg * sg) { continue; }
    let burst = exp(-a.z / 0.10);
    let glow = smoothstep(0.0, 0.6, a.z) * (1.0 - smoothstep(0.45 * b.z, b.z, a.z));
    let env = a.w * (0.65 * burst + 0.45 * glow);
    lit += palette(0.62 * b.x, b.x) * (env * exp(-d2 / (2.0 * sg * sg)));
  }
  c += lit * (0.55 * u.flash) * (0.7 + 0.8 * g);                 // the grid shows more where the glass is lit

  // the spectrum line the pads stand on (bass left, treble right), and the light it throws on the glass
  let half_w = 0.5 * u.ctl.y;
  if (abs(in.wp.z) < 1.6 && abs(in.wp.x) < half_w + 1.0) {
    let t = clamp(in.wp.x / u.ctl.y + 0.5, 0.0, 1.0);
    let l = hist_m(u32(t * f32(NX - 1u) + 0.5), 0u);
    let col = palette(l * 0.85, t);
    let bar = exp(-in.wp.z * in.wp.z / (2.0 * 0.04 * 0.04));
    let glow = exp(-in.wp.z * in.wp.z / (2.0 * 0.45 * 0.45)) * l * l;
    let edge = 1.0 - smoothstep(half_w, half_w + 0.8, abs(in.wp.x));
    c += col * (bar * (0.08 + 1.5 * l) + 0.18 * glow) * edge;
  }
  return vec4<f32>(mix(BG, c, fade) * u.exposure, 1.0);
}

// ---- rockets
fn shell_sprite(vi: u32, ii: u32, mirror: bool) -> SpriteOut {
  let a = fw.shells[2u * ii];
  let b = fw.shells[2u * ii + 1u];
  if (a.w <= 0.0) { return sprite_dead(); }
  var col = mix(vec3<f32>(1.0, 0.92, 0.75), palette(0.15, b.w), 0.30) * (1.5 * a.w);
  if (mirror) {
    let v = normalize(u.cam.xyz - a.xyz);
    col *= (0.10 + 0.90 * pow(1.0 - clamp(abs(v.y), 0.0, 1.0), 3.0)) * exp(-0.35 * a.y) * 0.9;
  }
  return sprite_make_trail(a.xyz, b.xyz, 0.04, 0.16, vi, col, mirror);
}

@vertex fn vs_shell(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> SpriteOut {
  return shell_sprite(vi, ii, false);
}

@vertex fn vs_shell_refl(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> SpriteOut {
  return shell_sprite(vi, ii, true);
}

// ---- stars
// Seconds of velocity drawn behind a star, by type: a willow's trails droop, a crackle's are a speck.
fn trail_of(kind: f32) -> f32 {
  if (kind < 0.5) { return 0.34; }
  if (kind < 1.5) { return 0.12; }
  if (kind < 2.5) { return 0.055; }
  return 0.0;
}

// A fast star would draw a line as long as the burst is wide (a hedgehog); its streak is also capped in world
// units, so the long droop is only reached by the slow stars of a willow falling.
fn trail_len(kind: f32, vel: vec3<f32>) -> f32 {
  var cap = 0.25;
  if (kind < 0.5) { cap = 1.2; } else if (kind < 1.5) { cap = 0.38; }
  return min(trail_of(kind), cap / max(length(vel), 0.01));
}

fn star_colour(p: Particle, ii: u32) -> vec3<f32> {
  let kind = floor(p.look.w * 0.5);
  let splash = p.look.w - 2.0 * kind;
  let age = 1.0 - p.life / p.max_life;
  let h = f32(pcg(ii * 2654435761u + 977u) >> 8u) * (1.0 / 16777216.0);
  if (kind > 2.5) {
    // the detonation flash: white-hot and the only thing allowed near the brightest values; Flash scales it
    let tint = mix(vec3<f32>(1.0), palette(1.0, p.look.x), 0.30);
    return tint * p.look.z * exp(-age * 3.2) * u.flash;
  }
  // the brightest stars take the theme's accent role, the rest their frequency's place on the ramp
  let base = palette(select(0.62 * p.look.x, 1.0, h > 0.93), p.look.x);
  // born white-hot, then its colour, then cooling toward ember orange as it burns down
  let hot = mix(vec3<f32>(1.0), base, smoothstep(0.0, 0.07, age));
  let col = mix(hot, EMBER, 0.75 * smoothstep(0.50, 1.0, age));
  // a star starts dim and lights up as the shell opens, so the detonation flash stays the brightest thing and
  // a thousand overlapping stars do not burn to a flat disc
  let opening = 0.30 + 0.70 * smoothstep(0.0, 0.22, age * p.max_life);
  let fade = opening * clamp(p.life / (0.40 * p.max_life), 0.0, 1.0);
  // a crackle's stars twinkle: a flicker by a per-star hash of the time, not child particles
  var tw = 1.0;
  if (kind > 1.5) {
    let slot = floor(u.t * 14.0 + h * 9.0);
    let r = f32(pcg(ii * 747796405u + u32(slot) * 2891336453u) >> 8u) * (1.0 / 16777216.0);
    tw = 0.25 + 1.5 * r * r;
  }
  return col * p.look.z * fade * tw * (1.0 + 1.4 * splash);
}

@vertex fn vs_star(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> SpriteOut {
  let p = parts[ii];
  if (p.life <= 0.0) { return sprite_dead(); }
  let kind = floor(p.look.w * 0.5);
  return sprite_make_trail(p.pos, p.vel, p.look.y, trail_len(kind, p.vel), vi, star_colour(p, ii), false);
}

// mirrored below the glass: weak from above, stronger at grazing angles, fading with depth
@vertex fn vs_star_refl(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> SpriteOut {
  let p = parts[ii];
  if (p.life <= 0.0) { return sprite_dead(); }
  let kind = floor(p.look.w * 0.5);
  let v = normalize(u.cam.xyz - p.pos);
  let fres = 0.10 + 0.90 * pow(1.0 - clamp(abs(v.y), 0.0, 1.0), 3.0);
  let col = star_colour(p, ii) * (fres * exp(-0.35 * p.pos.y) * 0.9);
  return sprite_make_trail(p.pos, p.vel, p.look.y, trail_len(kind, p.vel), vi, col, true);
}
