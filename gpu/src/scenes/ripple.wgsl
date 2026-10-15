// Ripple Pond's drawing: a dark glossy pool shaded from the slope texture the wave simulation leaves (see
// ripple_sim.wgsl), a haze at the horizon, four light towers standing beyond the far shore, and the droplets in
// flight. The water is a mirror: what it shows is the horizon haze and the towers, so a passing ring tilts the
// surface and sweeps their reflections across it, and the towers' brightness follows the bands. Droplets are the
// CPU's closed-form arcs (`ripple.rs` keeps the table and knows where each is at any moment), drawn as streaks
// and mirrored in the water, displaced by the slope under them.
const FLOOR_EXTENT: f32 = 600.0;
const TOWERS: u32 = 4u;
const TOWER_Z: f32 = -25.0;
const SKY_Z: f32 = -70.0;

@group(1) @binding(2) var<uniform> r: RpU;
@group(1) @binding(3) var wsmp: sampler;
@group(1) @binding(4) var slope: texture_2d<f32>;

fn pond_uv(wp: vec3<f32>) -> vec2<f32> {
  return clamp(vec2<f32>((wp.x - r.pond.x) / r.pond.z, (wp.z - r.pond.y) / r.pond.w), vec2<f32>(0.0), vec2<f32>(1.0));
}

// The haze along the horizon, by the elevation of the ray: faint everywhere, brighter toward the horizon.
fn horizon(el: f32) -> vec3<f32> {
  return palette(0.35, 0.2) * (0.004 + 0.050 * exp(-max(el, 0.0) * 30.0)) * (0.7 + 0.5 * u.sus.w);
}

// ---- the towers: a skyline of four strips of light. Bass stands at the left, the highs at the right; each burns
// with its region's sustained level and flares on its onsets, and all stay lit in silence and drift very slowly.
fn tower_x(k: u32) -> f32 {
  var f = array<f32, 4>(-0.30, -0.10, 0.12, 0.32);
  let drift = sin(u.t * (0.045 + 0.011 * f32(k)) + 1.7 * f32(k)) * 0.035 * u.ctl.x;
  return (f[k] + drift) * r.pond.z;
}
fn tower_hw(k: u32) -> f32 {
  var a = array<f32, 4>(0.42, 0.30, 0.34, 0.26);
  return a[k];
}
fn tower_h(k: u32) -> f32 {
  var a = array<f32, 4>(7.0, 5.2, 6.0, 8.0);
  return a[k];
}
fn tower_role(k: u32) -> u32 {
  var a = array<u32, 4>(0u, 1u, 1u, 2u);
  return a[k];
}
fn tower_gain(k: u32) -> f32 {
  let role = tower_role(k);
  return (0.40 + 0.80 * u.sus[role] + 0.8 * u.flash * u.fx[role]) * (0.85 + 0.30 * u.sus.w);
}
fn tower_tint(k: u32) -> vec3<f32> {
  var t = array<f32, 4>(0.0, 0.6, 0.6, 1.0);
  var x = array<f32, 4>(0.1, 0.4, 0.6, 0.9);
  return mix(palette(t[k], x[k]), vec3<f32>(1.0), 0.22);
}

struct TowerOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) uv: vec2<f32>,                       // across in half widths, height in world units
  @location(1) @interpolate(flat) k: u32,
};

@vertex fn vs_tower(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> TowerOut {
  var q = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0));
  let c = q[vi];
  let hw = tower_hw(ii);
  let h = tower_h(ii);
  let dx = mix(-2.4 * hw, 2.4 * hw, c.x);
  let y = mix(0.0, h + 1.0, c.y);
  var o: TowerOut;
  o.pos = u.vp * vec4<f32>(tower_x(ii) + dx, y, TOWER_Z, 1.0);
  o.uv = vec2<f32>(dx / hw, y);
  o.k = ii;
  return o;
}

@fragment fn fs_tower(in: TowerOut) -> @location(0) vec4<f32> {
  let h = tower_h(in.k);
  let dx = abs(in.uv.x);
  let core = (1.0 - smoothstep(0.80, 1.0, dx)) * (1.0 - smoothstep(h - 0.8, h, in.uv.y)) * (0.65 + 0.35 * (1.0 - in.uv.y / h));
  // the glow dies away before the quad's edges and top, so the strip has no visible box around it
  let halo = exp(-0.5 * dx * dx) * 0.10 * (1.0 - smoothstep(1.2, 2.4, dx)) * (1.0 - smoothstep(h - 0.6, h + 1.0, in.uv.y));
  return vec4<f32>(tower_tint(in.k) * tower_gain(in.k) * (1.8 * core + halo) * u.exposure, 1.0);
}

// The water's own body: a soft sheen on the faces of the ripples turned toward the towers, so a ring shows wherever it
// is and not only where it catches a reflection. Flat water has a little of it everywhere (about 0.012), which the sky
// painted below the horizon has to match or the two meet in a step.
const SHEEN_DIR: vec3<f32> = vec3<f32>(0.0, 0.30, -0.95);
fn sheen(n: vec3<f32>) -> vec3<f32> {
  return palette(0.3, 0.5) * (16.0 * pow(max(dot(n, normalize(SHEEN_DIR)), 0.0), 6.0));
}

// ---- the sky behind everything: just the horizon haze, painted by the elevation of each ray
struct SkyOut { @builtin(position) pos: vec4<f32>, @location(0) wp: vec3<f32> };
@vertex fn vs_sky(@builtin(vertex_index) vi: u32) -> SkyOut {
  var q = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0));
  let c = q[vi];
  let w = vec3<f32>(mix(-400.0, 400.0, c.x), mix(-30.0, 90.0, c.y), SKY_Z);
  return SkyOut(u.vp * vec4<f32>(w, 1.0), w);
}
@fragment fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
  let d = normalize(in.wp - u.cam.xyz);
  var c = horizon(d.y);
  if (d.y < 0.0) {
    // between the horizon and where the water quad ends (its far clip) this is the water at its far end: the haze
    // reflected at a grazing angle, so the two meet without a seam
    let a = -d.y;
    c = horizon(a) * (0.09 + 0.91 * pow(1.0 - a, 5.0)) + vec3<f32>(0.002, 0.003, 0.0065) + sheen(vec3<f32>(0.0, 1.0, 0.0));
  }
  return vec4<f32>(c * u.exposure, 1.0);
}

// ---- the water
struct FloorOut { @builtin(position) pos: vec4<f32>, @location(0) wp: vec3<f32> };
@vertex fn vs_floor(@builtin(vertex_index) vi: u32) -> FloorOut {
  var q = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0));
  let c = q[vi];
  let w = vec3<f32>(mix(-FLOOR_EXTENT, FLOOR_EXTENT, c.x), 0.0, mix(-FLOOR_EXTENT, FLOOR_EXTENT, c.y));
  return FloorOut(u.vp * vec4<f32>(w, 1.0), w);
}

@fragment fn fs_water(in: FloorOut) -> @location(0) vec4<f32> {
  let to_cam = u.cam.xyz - in.wp;
  let v = to_cam / length(to_cam);
  // the slope texture is mipmapped and read anisotropically, so a thin ring far away is averaged over the pixel's
  // footprint instead of aliasing as it moves
  let s = textureSample(slope, wsmp, pond_uv(in.wp));
  let n = normalize(vec3<f32>(-s.x, 1.0, -s.y));
  var rr = reflect(-v, n);
  rr = normalize(vec3<f32>(rr.x, max(rr.y, 0.02), rr.z));
  let foot = length(fwidth(rr)) * 0.8 + 0.004;
  let ndv = clamp(dot(n, v), 0.0, 1.0);
  let fres = 0.09 + 0.91 * pow(1.0 - ndv, 5.0);

  var refl = horizon(rr.y);
  for (var k = 0u; k < TOWERS; k++) {
    let h = tower_h(k);
    let m = rect_light_at(vec3<f32>(tower_x(k), 0.5 * h, TOWER_Z), vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), tower_hw(k), 0.5 * h, in.wp, rr, foot, 0.04);
    refl += tower_tint(k) * (1.8 * tower_gain(k)) * m.x * (0.45 + 0.55 * (1.0 - clamp((m.z + 0.5 * h) / h, 0.0, 1.0)));
  }
  var c = vec3<f32>(0.002, 0.003, 0.0065) + refl * fres;

  c += sheen(n);

  // the spectrum line the drops start from (bass left, treble right), seen through the ripples
  let half_w = 0.5 * u.ctl.y;
  let zz = in.wp.z + s.y * 0.35;
  if (abs(zz) < 1.6 && abs(in.wp.x) < half_w + 1.0) {
    let t = clamp(in.wp.x / u.ctl.y + 0.5, 0.0, 1.0);
    let l = hist_m(u32(t * f32(NX - 1u) + 0.5), 0u);
    let col = palette(l * 0.85, t);
    let bar = exp(-zz * zz / (2.0 * 0.05 * 0.05));
    let glow = exp(-zz * zz / (2.0 * 0.45 * 0.45)) * l * l;
    let edge = 1.0 - smoothstep(half_w, half_w + 0.8, abs(in.wp.x));
    c += col * (bar * (0.02 + 0.7 * l) + 0.10 * glow) * edge;
  }
  return vec4<f32>(c * u.exposure, 1.0);
}

// ---- droplets in flight
fn drop_sprite(vi: u32, ii: u32, mirror: bool) -> SpriteOut {
  let a = r.drops[2u * ii];
  let b = r.drops[2u * ii + 1u];
  if (a.w <= 0.0) { return sprite_dead(); }
  var pos = a.xyz;
  var col = mix(vec3<f32>(0.85, 0.95, 1.0), palette(0.2, b.w), 0.45) * (0.9 + 0.8 * clamp(u.flash, 0.0, 1.5));
  if (mirror) {
    // seen in the water, bent by the slope under it, and weak from above
    let s = textureSampleLevel(slope, wsmp, pond_uv(pos), 0.0);
    pos = vec3<f32>(pos.x + s.x * pos.y * 1.5, pos.y, pos.z + s.y * pos.y * 1.5);
    let v = normalize(u.cam.xyz - a.xyz);
    col *= (0.10 + 0.90 * pow(1.0 - clamp(abs(v.y), 0.0, 1.0), 3.0)) * 0.9;
  }
  return sprite_make_trail(pos, b.xyz, a.w, 0.05, vi, col, mirror);
}

@vertex fn vs_drop(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> SpriteOut {
  return drop_sprite(vi, ii, false);
}
@vertex fn vs_drop_refl(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> SpriteOut {
  return drop_sprite(vi, ii, true);
}
