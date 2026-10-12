// The fountain's floor and particles. The floor is a glossy dark plane with a faint grid; the spectrum
// line glows along it where the jets start, and the particles are mirrored in it.
const FLOOR_EXTENT: f32 = 600.0;

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
  var c = vec3<f32>(0.007, 0.008, 0.015) + palette(0.35, 0.5) * g * 0.10;
  // the spectrum line the jets start from, and the light it throws on the glass
  let half_w = 0.5 * u.ctl.y;
  if (abs(in.wp.z) < 1.6 && abs(in.wp.x) < half_w + 1.0) {
    let t = clamp(in.wp.x / u.ctl.y + 0.5, 0.0, 1.0);
    let l = hist_m(u32(t * f32(NX - 1u) + 0.5), 0u);
    let col = palette(l * 0.85, t);
    let bar = exp(-in.wp.z * in.wp.z / (2.0 * 0.04 * 0.04));
    let glow = exp(-in.wp.z * in.wp.z / (2.0 * 0.45 * 0.45)) * l * l;
    let edge = 1.0 - smoothstep(half_w, half_w + 0.8, abs(in.wp.x));
    c += col * (bar * (0.10 + 2.0 * l) + 0.22 * glow) * edge;
  }
  return vec4<f32>(mix(BG, c, fade) * u.exposure, 1.0);
}

// ---- particles
fn jet_colour(p: Particle) -> vec3<f32> {
  let age = 1.0 - p.life / p.max_life;
  let base = palette(p.look.z, p.look.x);
  let hot = mix(vec3<f32>(1.0), base, smoothstep(0.0, 0.12, age));          // born white-hot, cools to its colour
  let fade = smoothstep(0.0, 0.03, age) * clamp(p.life / 0.4, 0.0, 1.0);
  return hot * (0.5 + 0.8 * p.look.z + 1.6 * p.look.w) * fade;   // a steeper rise glows brighter
}

@vertex fn vs_part(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> SpriteOut {
  let p = parts[ii];
  if (p.life <= 0.0) { return sprite_dead(); }
  return sprite_make(p.pos, p.vel, p.look.y, vi, jet_colour(p), false);
}

// mirrored below the glass: weak from above, stronger at grazing angles, fading with depth
@vertex fn vs_part_refl(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> SpriteOut {
  let p = parts[ii];
  if (p.life <= 0.0) { return sprite_dead(); }
  let v = normalize(u.cam.xyz - p.pos);
  let fres = 0.10 + 0.90 * pow(1.0 - clamp(abs(v.y), 0.0, 1.0), 3.0);
  let d = p.pos.y;
  return sprite_make(p.pos, p.vel, p.look.y, vi, jet_colour(p) * (fres * exp(-0.35 * d) * 0.9), true);
}
