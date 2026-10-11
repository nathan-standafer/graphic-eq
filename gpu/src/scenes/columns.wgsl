// Columns: segmented LED cores (emissive) inside glass housings (reflective, mostly dark) on
// plinths, standing on a glossy floor. Instances: 0..10 core, 11..21 glass, 22..32 plinth,
// 33..43 core reflection.
const FLOOR_EXTENT: f32 = 600.0;
const COL_MAX_H: f32 = 6.0;     // LED strip length
const PLINTH: f32 = 0.16;       // height of the base the strip stands on
const CORE_W: f32 = 0.56;       // LED strip width and depth
const GLASS_W: f32 = 0.98;
const GLASS_TOP: f32 = 6.5;     // top of the housing above the floor
const SEG: f32 = 0.2;           // LED segment pitch
const GAP: f32 = 0.20;          // dark fraction of each segment

fn spacing() -> f32 { return u.ctl.y; }

/// Onset envelope for the region a band belongs to.
fn band_env(i: u32) -> f32 {
  if (i < 3u) { return u.fx.x; }
  if (i < 7u) { return u.fx.y; }
  return u.fx.z;
}

struct ColOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) wp: vec3<f32>,
  @location(1) n: vec3<f32>,
  @location(2) uv: vec2<f32>,
  @location(3) @interpolate(flat) inst: u32,
};

@vertex fn vs_col(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> ColOut {
  let kind = ii / 11u;
  let i = ii % 11u;
  let face = vi / 6u;
  let corner = vi % 6u;
  var q = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0));
  let uvq = q[corner];
  let axis = face / 2u;
  let sgn = select(-1.0, 1.0, (face % 2u) == 1u);
  let a = uvq.x - 0.5;
  let b = uvq.y - 0.5;
  var p = vec3<f32>(0.0);
  var n = vec3<f32>(0.0);
  if (axis == 0u) { p = vec3<f32>(0.5 * sgn, b, a); n = vec3<f32>(sgn, 0.0, 0.0); }
  else if (axis == 1u) { p = vec3<f32>(a, 0.5 * sgn, b); n = vec3<f32>(0.0, sgn, 0.0); }
  else { p = vec3<f32>(a, b, 0.5 * sgn); n = vec3<f32>(0.0, 0.0, sgn); }

  let cx = (f32(i) - 5.0) * spacing();
  var size = vec3<f32>(CORE_W, COL_MAX_H, CORE_W);
  var centre = vec3<f32>(cx, PLINTH + COL_MAX_H * 0.5, 0.0);
  if (kind == 1u) {
    size = vec3<f32>(GLASS_W, GLASS_TOP - PLINTH, GLASS_W);
    centre.y = PLINTH + size.y * 0.5;
  } else if (kind == 2u) {
    size = vec3<f32>(GLASS_W + 0.2, PLINTH, GLASS_W + 0.2);
    centre.y = PLINTH * 0.5;
  }
  var world = centre + p * size;
  if (kind == 3u) { world.y = -world.y; n.y = -n.y; }   // mirror below the floor
  var o: ColOut;
  o.pos = u.vp * vec4<f32>(world, 1.0);
  o.wp = world;
  o.n = n;
  o.uv = uvq;
  o.inst = ii;
  return o;
}

// ---- LED strip: segments light from the bottom up to the level; the segment holding the peak
// is bright. `soft` (0..1) blurs the segment pattern, which is how the floor reflection softens.
fn core_emission(y_world: f32, i: u32, across: f32, side: f32, soft: f32) -> vec3<f32> {
  let y = y_world - PLINTH;
  let s = y / SEG;
  let si = floor(s);
  let fy = fract(s);
  let hs = lvl(i) * COL_MAX_H / SEG;
  let lit = clamp(hs - si + 0.5 * soft, 0.0, 1.0);
  let col = palette(clamp(si * SEG / COL_MAX_H, 0.0, 1.0), f32(i) / 10.0);
  // block shape inside the segment: dark gap, bevelled edges
  let v = fy / (1.0 - GAP);
  let inside = select(0.0, 1.0, v <= 1.0);
  let bevel_y = clamp(min(v, 1.0 - v) / 0.20, 0.0, 1.0);
  let bevel_x = clamp(min(across, 1.0 - across) / 0.16, 0.0, 1.0);
  let block = mix(inside * (0.50 + 0.50 * smoothstep(0.0, 1.0, min(bevel_x, bevel_y))), 0.62, soft);
  let env = band_env(i);
  var e = col * lit * block * (1.55 + u.flash * 1.1 * env);
  e += col * block * 0.020;                              // unlit LEDs stay faintly visible
  // peak hold: one bright segment
  let pks = pk(i) * COL_MAX_H / SEG;
  if (pk(i) > 0.01 && si == floor(pks)) {
    e = mix(e, mix(col, vec3<f32>(1.0), 0.5) * block * 2.3, 1.0 - soft * 0.9);
  }
  return e * side;
}

@fragment fn fs_core(in: ColOut) -> @location(0) vec4<f32> {
  let i = in.inst % 11u;
  let across = in.uv.x;
  var side = select(0.5, 1.0, in.n.z > 0.5);
  if (abs(in.n.y) > 0.5) { side = 0.25; }
  let e = core_emission(in.wp.y, i, across, side, 0.0);
  return vec4<f32>(e * u.exposure, 1.0);
}

// ---- mirrored strip, added onto the floor: fades and loses its segment pattern with depth
@fragment fn fs_refl(in: ColOut) -> @location(0) vec4<f32> {
  let i = in.inst % 11u;
  let d = max(-in.wp.y, 0.0);                       // how far below the floor this point mirrors
  let soft = clamp(d * 0.35, 0.0, 1.0);
  let across = in.uv.x;
  let side = select(0.45, 1.0, abs(in.n.z) > 0.5);
  let e = core_emission(-in.wp.y, i, across, side, soft);
  let v = normalize(u.cam.xyz - vec3<f32>(in.wp.x, -in.wp.y, in.wp.z));
  let fres = 0.06 + 0.94 * pow(1.0 - clamp(abs(v.y), 0.0, 1.0), 4.0);
  let fade = exp(-d * 0.5);
  return vec4<f32>(e * fres * fade * 1.0 * u.exposure, 1.0);
}

// ---- dark metal plinth with a small level indicator on the front
@fragment fn fs_plinth(in: ColOut) -> @location(0) vec4<f32> {
  let i = in.inst % 11u;
  let key = normalize(vec3<f32>(-0.4, 0.8, 0.6));
  let shade = 0.35 + 0.65 * max(dot(in.n, key), 0.0);
  var c = vec3<f32>(0.012, 0.014, 0.020) * shade;
  let ed = min(min(in.uv.x, 1.0 - in.uv.x), min(in.uv.y, 1.0 - in.uv.y));
  c += vec3<f32>(0.10, 0.12, 0.17) * (1.0 - smoothstep(0.0, 0.05, ed)) * 0.35 * shade;
  if (in.n.z > 0.5) {
    let strip = (1.0 - smoothstep(0.0, 0.12, abs(in.uv.y - 0.5) - 0.10)) * (1.0 - smoothstep(0.0, 0.06, abs(in.uv.x - 0.5) - 0.30));
    let l = lvl(i);
    c += palette(l * 0.8, f32(i) / 10.0) * strip * (0.10 + 1.4 * l);
  }
  return vec4<f32>(c * u.exposure, 1.0);
}

// ---- glass housing: almost clear, bright at the edges and at grazing angles
@fragment fn fs_glass(in: ColOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
  let i = in.inst % 11u;
  var n = normalize(in.n);
  if (!front) { n = -n; }
  let v = normalize(u.cam.xyz - in.wp);
  let ndv = clamp(dot(n, v), 0.0, 1.0);
  let fres = pow(1.0 - ndv, 3.0);
  // distance to the nearest edge of this face, in world units
  var ed = 0.0;
  if (abs(in.n.y) > 0.5) {
    ed = min(min(in.uv.x, 1.0 - in.uv.x), min(in.uv.y, 1.0 - in.uv.y)) * GLASS_W;
  } else {
    ed = min(GLASS_W * 0.5 - abs((in.uv.x - 0.5) * GLASS_W), min(in.wp.y - PLINTH, GLASS_TOP - in.wp.y));
  }
  let edge = 1.0 - smoothstep(0.0, 0.03 + 1.5 * fwidth(ed), ed);

  // two soft strip lights reflected in the glass
  let r = reflect(-v, n);
  let l1 = smoothstep(0.88, 0.99, dot(r, normalize(vec3<f32>(-0.55, 0.45, 0.70))));
  let l2 = smoothstep(0.92, 0.995, dot(r, normalize(vec3<f32>(0.70, 0.25, 0.65))));
  let env = 0.55 * l1 + 0.30 * l2;

  // light scattered by the lit LEDs
  let lit_h = lvl(i) * COL_MAX_H + PLINTH;
  let scatter = smoothstep(lit_h + 0.6, lit_h - 0.6, in.wp.y) * lvl(i);
  let tint = palette(clamp(in.wp.y / COL_MAX_H, 0.0, 1.0), f32(i) / 10.0);

  var rgb = vec3<f32>(0.010, 0.014, 0.024)
          + vec3<f32>(0.42, 0.50, 0.66) * (0.32 * edge + 0.25 * env) * (0.5 + 0.5 * n.y + 0.4)
          + tint * scatter * 0.035 * (1.0 + u.flash * band_env(i));
  var a = 0.06 + 0.40 * fres + 0.70 * edge + 0.50 * env;
  // cap and base trim are solid
  if (in.wp.y > GLASS_TOP - 0.28 || in.wp.y < PLINTH + 0.12 || n.y > 0.5) {
    rgb = vec3<f32>(0.016, 0.018, 0.026) + vec3<f32>(0.30, 0.36, 0.48) * edge * 0.5;
    a = 1.0;
  }
  return vec4<f32>(rgb * u.exposure, clamp(a, 0.0, 1.0));
}

// --------------------------------------------------------------------------- floor
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
  let sp = spacing();
  let gx = in.wp.x / sp;
  let aa = line_aa(max(fwidth(gx), fwidth(in.wp.z)));
  let g = max(gridline(gx), gridline(in.wp.z / sp)) * aa;
  var c = vec3<f32>(0.007, 0.008, 0.015) + palette(0.35, 0.5) * g * 0.10;
  // pools of light under each column
  var pool = vec3<f32>(0.0);
  let s2 = (sp * sp) / 1.8225;
  for (var i = 0u; i < 11u; i++) {
    let cx = (f32(i) - 5.0) * sp;
    let dx = in.wp.x - cx;
    let dz = in.wp.z * 0.8;
    let l = lvl(i);
    pool += palette(l * 0.8, f32(i) / 10.0) * l * l * exp(-(dx * dx + dz * dz) * 0.34 / s2) * (1.0 + u.flash * 0.9 * band_env(i));
  }
  c += pool * 0.55;
  // low-frequency onsets send a ripple across the floor
  for (var k = 0u; k < 8u; k++) {
    let rg = u.rings[k];
    if (rg.y <= 0.0) { break; }                 // the CPU packs live rings at the front
    if (rg.z < 0.5) {
      let rr = length(in.wp.xz * vec2<f32>(0.8, 1.0));
      let front = rg.x * 7.0 * sp;
      let w = 0.35 + 0.25 * rg.x;
      let dr = (rr - front) / w;
      if (abs(dr) < 3.5) {
        c += palette(0.8, 0.5) * exp(-dr * dr) * rg.y * exp(-rg.x * 1.8) * 0.16 * u.flash;
      }
    }
  }
  return vec4<f32>(mix(BG, c, fade) * u.exposure, 1.0);
}
