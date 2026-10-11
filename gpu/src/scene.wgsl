struct U {
  vp: mat4x4<f32>,
  cam: vec4<f32>,
  t: f32, aspect: f32, mode: f32, theme: f32,
  head: f32, rows: f32, bloom: f32, bass: f32,
  levels: array<vec4<f32>, 3>,
  peaks: array<vec4<f32>, 3>,
  res: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var hist: texture_2d<f32>;

const COL_SPACING: f32 = 1.35;
const COL_MAX_H: f32 = 6.0;
const NX: u32 = 96u;
const NZ: u32 = 192u;

fn lvl(i: u32) -> f32 { return u.levels[i / 4u][i % 4u]; }
fn pk(i: u32) -> f32 { return u.peaks[i / 4u][i % 4u]; }

fn hsv(h: f32, s: f32, v: f32) -> vec3<f32> {
  let k = vec3<f32>(1.0, 2.0 / 3.0, 1.0 / 3.0);
  let p = abs(fract(vec3<f32>(h) + k) * 6.0 - vec3<f32>(3.0));
  return v * mix(vec3<f32>(1.0), clamp(p - vec3<f32>(1.0), vec3<f32>(0.0), vec3<f32>(1.0)), s);
}
fn ramp(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, t: f32) -> vec3<f32> {
  let lo = mix(a, b, clamp(t / 0.6, 0.0, 1.0));
  let hi = mix(b, c, clamp((t - 0.6) / 0.4, 0.0, 1.0));
  return select(hi, lo, t < 0.6);
}
fn palette(t: f32, x: f32) -> vec3<f32> {
  let th = i32(u.theme + 0.5);
  if (th == 0) { return ramp(vec3<f32>(0.10, 1.0, 0.35), vec3<f32>(1.0, 0.9, 0.2), vec3<f32>(1.0, 0.15, 0.2), t); }
  if (th == 1) { return ramp(vec3<f32>(1.0, 0.65, 0.0), vec3<f32>(1.0, 0.4, 0.0), vec3<f32>(1.0, 0.12, 0.0), t); }
  if (th == 2) { return ramp(vec3<f32>(0.0, 0.9, 1.0), vec3<f32>(0.3, 0.5, 1.0), vec3<f32>(0.75, 0.3, 1.0), t); }
  if (th == 3) { return ramp(vec3<f32>(1.0, 0.15, 0.85), vec3<f32>(0.5, 0.35, 1.0), vec3<f32>(0.0, 0.9, 1.0), t); }
  return hsv(fract(0.85 - 0.8 * x), 0.85, 1.0);
}

// ---------------------------------------------------------------- background
struct BgOut { @builtin(position) pos: vec4<f32> };
@vertex fn vs_bg(@builtin(vertex_index) vi: u32) -> BgOut {
  var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
  return BgOut(vec4<f32>(p[vi], 1.0, 1.0));
}
@fragment fn fs_bg(in: BgOut) -> @location(0) vec4<f32> {
  let uv = in.pos.xy / u.res.xy;
  let r = length((uv - vec2<f32>(0.5, 0.62)) * vec2<f32>(u.aspect, 1.0));
  var c = mix(vec3<f32>(0.004, 0.005, 0.012), vec3<f32>(0.030, 0.026, 0.065), 1.0 - uv.y);
  c += palette(0.45, 0.5) * (0.035 + 0.22 * u.bass) * exp(-r * r * 4.0);
  return vec4<f32>(c, 1.0);
}

// ------------------------------------------------------------------- columns
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

  let cx = (f32(i) - 5.0) * COL_SPACING;
  var world = vec3<f32>(0.0);
  if (kind == 2u) {
    let ph = pk(i) * COL_MAX_H;
    let s = select(0.0, 1.0, ph > 0.06);
    world = vec3<f32>(cx, ph + 0.14, 0.0) + p * vec3<f32>(1.0, 0.10, 1.0) * s;
  } else {
    let h = max(lvl(i) * COL_MAX_H, 0.05);
    world = vec3<f32>(cx, h * 0.5, 0.0) + p * vec3<f32>(0.95, h, 0.95);
    if (kind == 1u) { world.y = -world.y; n.y = -n.y; }
  }
  var o: ColOut;
  o.pos = u.vp * vec4<f32>(world, 1.0);
  o.wp = world;
  o.n = n;
  o.uv = uvq;
  o.inst = ii;
  return o;
}

@fragment fn fs_col(in: ColOut) -> @location(0) vec4<f32> {
  let kind = in.inst / 11u;
  let i = in.inst % 11u;
  let wy = abs(in.wp.y);
  let col = palette(clamp(wy / COL_MAX_H, 0.0, 1.0), f32(i) / 10.0);
  let light = normalize(vec3<f32>(0.4, 0.8, 0.6));
  let shade = 0.45 + 0.55 * max(dot(in.n, light), 0.0);
  let edge = min(min(in.uv.x, 1.0 - in.uv.x), min(in.uv.y, 1.0 - in.uv.y));
  let rim = 1.0 + 1.3 * (1.0 - smoothstep(0.0, 0.07, edge));
  var emissive = col * shade * 0.85 * (1.0 + 0.6 * (rim - 1.0));
  if (abs(in.n.y) < 0.5) {
    let s = fract(wy * 5.0);
    emissive *= 1.0 - 0.92 * step(0.80, s);   // LED segment gaps
  } else {
    emissive *= 1.35;
  }
  var alpha = 1.0;
  if (kind == 2u) { emissive = col * 1.6 + vec3<f32>(0.15); }
  if (kind == 1u) { alpha = 0.32 * exp(-wy * 0.45); }
  return vec4<f32>(emissive, alpha);
}

// --------------------------------------------------------------------- floor
struct FloorOut { @builtin(position) pos: vec4<f32>, @location(0) wp: vec3<f32> };
@vertex fn vs_floor(@builtin(vertex_index) vi: u32) -> FloorOut {
  var q = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0));
  let c = q[vi];
  let w = vec3<f32>(mix(-18.0, 18.0, c.x), -0.002, mix(-12.0, 8.0, c.y));
  return FloorOut(u.vp * vec4<f32>(w, 1.0), w);
}
fn gridline(v: f32) -> f32 {
  let d = abs(fract(v + 0.5) - 0.5);
  let w = fwidth(v);
  return 1.0 - smoothstep(0.0, 1.5 * w + 1e-4, d);
}
@fragment fn fs_floor(in: FloorOut) -> @location(0) vec4<f32> {
  var c = vec3<f32>(0.012, 0.014, 0.026);
  let fade = 1.0 - smoothstep(5.0, 17.0, length(in.wp.xz * vec2<f32>(0.55, 1.0)));
  let g = max(gridline(in.wp.x / COL_SPACING), gridline(in.wp.z));
  c += palette(0.35, 0.5) * g * 0.22 * fade;
  var glow = vec3<f32>(0.0);
  for (var i = 0u; i < 11u; i++) {
    let cx = (f32(i) - 5.0) * COL_SPACING;
    let d = length(in.wp.xz - vec2<f32>(cx, 0.0));
    glow += palette(lvl(i) * 0.8, f32(i) / 10.0) * lvl(i) * exp(-d * d * 0.30);
  }
  c += glow * 0.9 * fade;
  return vec4<f32>(c, 0.80);
}

// ------------------------------------------------------------------- terrain
struct TerOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) cell: vec2<f32>,
  @location(1) h: f32,
  @location(2) wp: vec3<f32>,
};
fn hist_h(xi: u32, zi: u32) -> f32 {
  let rows = i32(u.rows);
  let row = (i32(u.head) - i32(zi) + rows) % rows;
  return textureLoad(hist, vec2<i32>(i32(xi), row), 0).r;
}
@vertex fn vs_ter(@builtin(vertex_index) vi: u32) -> TerOut {
  var cs = array<vec2<u32>, 6>(
    vec2<u32>(0u, 0u), vec2<u32>(1u, 0u), vec2<u32>(0u, 1u),
    vec2<u32>(1u, 0u), vec2<u32>(1u, 1u), vec2<u32>(0u, 1u));
  let cell = vi / 6u;
  let o = cs[vi % 6u];
  let cx = cell % (NX - 1u);
  let cz = cell / (NX - 1u);
  let xi = cx + o.x;
  let zi = cz + o.y;
  let h = hist_h(xi, zi);
  let fz = f32(zi) / f32(NZ - 1u);
  let w = vec3<f32>((f32(xi) / f32(NX - 1u) - 0.5) * 19.0, h * 5.5, -(f32(zi) + u.res.z) * 0.11);
  var out: TerOut;
  out.pos = u.vp * vec4<f32>(w, 1.0);
  out.cell = vec2<f32>(f32(xi), f32(zi));
  out.h = h;
  out.wp = w;
  return out;
}
fn line(v: f32) -> f32 {
  let d = min(fract(v), 1.0 - fract(v));
  let w = fwidth(v);
  return 1.0 - smoothstep(0.0, 1.3 * w + 1e-4, d);
}
@fragment fn fs_ter(in: TerOut) -> @location(0) vec4<f32> {
  let n = normalize(cross(dpdx(in.wp), dpdy(in.wp)));
  let shade = 0.5 + 0.5 * abs(dot(n, normalize(vec3<f32>(0.3, 1.0, 0.5))));
  let depth = clamp(in.cell.y / f32(NZ - 1u), 0.0, 1.0);
  let fade = pow(1.0 - depth, 1.6);
  let aa = 1.0 - smoothstep(0.45, 0.9, max(fwidth(in.cell.y), fwidth(in.cell.x)));  // fade lines that get sub-pixel
  let wire = max(line(in.cell.y), 0.35 * line(in.cell.x)) * aa;
  let col = palette(clamp(in.h * 1.15, 0.0, 1.0), in.cell.x / f32(NX - 1u));
  var c = vec3<f32>(0.006, 0.008, 0.016) + col * 0.05 * shade * in.h;
  c += col * wire * (0.55 + 2.6 * in.h) * fade;
  c = mix(vec3<f32>(0.004, 0.005, 0.012), c, fade);
  return vec4<f32>(c, 1.0);
}

// ------------------------------------------------- waterfall floor (flat grid at 0)
@vertex fn vs_wfloor(@builtin(vertex_index) vi: u32) -> FloorOut {
  var q = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0));
  let c = q[vi];
  let w = vec3<f32>(mix(-9.5, 9.5, c.x), -0.004, mix(4.0, -21.1, c.y));
  return FloorOut(u.vp * vec4<f32>(w, 1.0), w);
}
@fragment fn fs_wfloor(in: FloorOut) -> @location(0) vec4<f32> {
  // same coordinates as the terrain: x spacing 19/95 per bin, z spacing 0.11 per history row,
  // lines every 5 bins / 5 rows, scrolling with the history
  let zc = -in.wp.z / 0.11 - u.res.z - u.res.w;
  let g = max(gridline(in.wp.x / 1.0), gridline(zc / 5.0));
  let depth = clamp(-in.wp.z / 21.1, 0.0, 1.0);
  let fade = pow(1.0 - depth, 1.4) * (1.0 - smoothstep(8.0, 9.5, abs(in.wp.x)));
  let col = palette(0.25, 0.5);
  var c = vec3<f32>(0.005, 0.007, 0.014) + col * g * 0.55 * fade;
  c = mix(vec3<f32>(0.004, 0.005, 0.012), c, fade);
  return vec4<f32>(c, 1.0);
}

// ------------------------------------------- glass wall at time 0 (front of the waterfall)
const WALL_H: f32 = 5.6;
struct WallOut { @builtin(position) pos: vec4<f32>, @location(0) wp: vec3<f32> };
@vertex fn vs_wall(@builtin(vertex_index) vi: u32) -> WallOut {
  var q = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0));
  let c = q[vi];
  let w = vec3<f32>(mix(-9.5, 9.5, c.x), c.y * WALL_H, 0.03);
  return WallOut(u.vp * vec4<f32>(w, 1.0), w);
}
@fragment fn fs_wall(in: WallOut) -> @location(0) vec4<f32> {
  let col = palette(0.3, 0.5);
  // live spectrum at time 0 (newest history row), interpolated across bins
  let fx = clamp((in.wp.x / 19.0 + 0.5) * f32(NX - 1u), 0.0, f32(NX - 1u) - 0.001);
  let i0 = u32(floor(fx));
  let h = mix(hist_h(i0, 0u), hist_h(i0 + 1u, 0u), fract(fx)) * WALL_H;
  let y = in.wp.y;
  let px = fwidth(y);
  let curve = 1.0 - smoothstep(0.0, 2.2 * px, abs(y - h));
  let below = select(0.0, 1.0, y < h);
  // level lines every 1/6 of the height
  let lv = gridline(y / (WALL_H / 6.0));
  // frame
  let ex = min(in.wp.x + 9.5, 9.5 - in.wp.x);
  let frame = max(1.0 - smoothstep(0.0, 0.05, ex), max(1.0 - smoothstep(0.0, 2.0 * px, y), 1.0 - smoothstep(0.0, 2.0 * px, WALL_H - y)));
  var rgb = col * (0.25 * lv + 0.9 * frame + 3.0 * curve + 0.35 * below);
  let a = clamp(0.045 + 0.10 * below + 0.25 * lv + 0.7 * frame + 0.95 * curve, 0.0, 1.0);
  return vec4<f32>(rgb, a);
}
