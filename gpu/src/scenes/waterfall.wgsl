// Waterfall: spectrum history as a scrolling wireframe terrain, with a glass wall at time 0.
const FLOOR_EXTENT: f32 = 600.0;   // effectively infinite; fogged out long before the edge
const TER_H: f32 = 4.4;            // terrain and glass-wall height at full scale

struct TerOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) cell: vec2<f32>,
  @location(1) h: f32,
  @location(2) wp: vec3<f32>,
  @location(3) @interpolate(flat) ch: u32,
};
// Mixed mode reads the mono sum; stereo overlay reads the channels separately.
fn wf_h(xi: u32, zi: u32, ch: u32) -> f32 {
  if (u.mode > 0.5) { return select(hist_h2(xi, zi).x, hist_h2(xi, zi).y, ch == 1u); }
  return hist_m(xi, zi);
}
fn wf_h2(xi: u32, zi: u32) -> vec2<f32> {
  if (u.mode > 0.5) { return hist_h2(xi, zi); }
  let m = hist_m(xi, zi);
  return vec2<f32>(m, m);
}
fn ter_vertex(xi: u32, zi: u32, ch: u32) -> TerOut {
  let h = wf_h(xi, zi, ch);
  let w = vec3<f32>((f32(xi) / f32(NX - 1u) - 0.5) * 19.0, h * TER_H, -(f32(zi) + u.res.z) * 0.11);
  var out: TerOut;
  out.pos = u.vp * vec4<f32>(w, 1.0);
  out.cell = vec2<f32>(f32(xi), f32(zi));
  out.h = h;
  out.wp = w;
  out.ch = ch;
  return out;
}
@vertex fn vs_ter(@builtin(vertex_index) vi: u32, @builtin(instance_index) ch: u32) -> TerOut {
  var cs = array<vec2<u32>, 6>(
    vec2<u32>(0u, 0u), vec2<u32>(1u, 0u), vec2<u32>(0u, 1u),
    vec2<u32>(1u, 0u), vec2<u32>(1u, 1u), vec2<u32>(0u, 1u));
  let cell = vi / 6u;
  let o = cs[vi % 6u];
  return ter_vertex(cell % (NX - 1u) + o.x, cell / (NX - 1u) + o.y, ch);
}

// ---- the terrain mirrored below the glass floor. The reflection is soft, so it uses a mesh with
// every second bin and row (a quarter of the vertices); the lines still come from the cell coordinates.
const RCELLS: u32 = 96u;
@vertex fn vs_ter_refl(@builtin(vertex_index) vi: u32, @builtin(instance_index) ch: u32) -> TerOut {
  var cs = array<vec2<u32>, 6>(
    vec2<u32>(0u, 0u), vec2<u32>(1u, 0u), vec2<u32>(0u, 1u),
    vec2<u32>(1u, 0u), vec2<u32>(1u, 1u), vec2<u32>(0u, 1u));
  let cell = vi / 6u;
  let o = cs[vi % 6u];
  let xi = min((cell % RCELLS + o.x) * 2u, NX - 1u);
  let zi = min((cell / RCELLS + o.y) * 2u, NZ - 1u);
  var out = ter_vertex(xi, zi, ch);
  out.wp.y = -out.wp.y;
  out.pos = u.vp * vec4<f32>(out.wp, 1.0);
  return out;
}

// How much of an object shows in the glass at a point mirrored `d` below the floor: weak when
// looking down onto the glass, strong at grazing angles, and fading with depth.
fn glass_reflect(mirrored: vec3<f32>) -> f32 {
  let d = max(-mirrored.y, 0.0);
  let v = normalize(u.cam.xyz - vec3<f32>(mirrored.x, -mirrored.y, mirrored.z));
  let fres = 0.06 + 0.94 * pow(1.0 - clamp(abs(v.y), 0.0, 1.0), 4.0);
  return fres * exp(-d * 0.45) * 1.45;
}
// A line that widens and dims with depth below the floor, like a blurred reflection.
fn soft_line(v: f32, s: f32) -> f32 {
  let d = min(fract(v), 1.0 - fract(v));
  let w = fwidth(v);
  return 1.0 - smoothstep(0.0, (1.3 + 3.5 * s) * w + 1e-4 + 0.05 * s, d);
}
@fragment fn fs_ter_refl(in: TerOut) -> @location(0) vec4<f32> {
  let soft = clamp(max(-in.wp.y, 0.0) * 0.45, 0.0, 1.0);
  let fade = pow(1.0 - clamp(in.cell.y / f32(NZ - 1u), 0.0, 1.0), 1.6);
  // thin lines shimmer and band when mirrored at a grazing angle, so they fade out sooner than in the real terrain
  let aa = 1.0 - smoothstep(0.22, 0.55, max(fwidth(in.cell.y), fwidth(in.cell.x)));
  // near-vertical ridge faces pack their lines so densely that a soft reflection turns solid white
  let n = normalize(cross(dpdx(in.wp), dpdy(in.wp)));
  let face = 0.3 + 0.7 * abs(n.y);
  let wire = max(soft_line(in.cell.y, soft), 0.35 * soft_line(in.cell.x * 0.5, soft)) * aa * face * (1.0 - 0.5 * soft);
  let h = clamp(in.h, 0.0, 1.0);
  var e = vec3<f32>(0.0);
  if (u.mode > 0.5) {
    e = select(STEREO_L, STEREO_R, in.ch == 1u) * wire * (0.10 + 0.50 * h) * fade * 0.95;   // two meshes add up, so each is quieter
  } else {
    e = palette(clamp(h * 1.15, 0.0, 1.0), in.cell.x / f32(NX - 1u)) * wire * (0.55 + 2.6 * h) * fade;
  }
  return vec4<f32>(e * glass_reflect(in.wp) * u.exposure, 1.0);
}

@fragment fn fs_ter(in: TerOut) -> @location(0) vec4<f32> {
  let n = normalize(cross(dpdx(in.wp), dpdy(in.wp)));
  let shade = 0.5 + 0.5 * abs(dot(n, normalize(vec3<f32>(0.3, 1.0, 0.5))));
  let depth = clamp(in.cell.y / f32(NZ - 1u), 0.0, 1.0);
  let fade = pow(1.0 - depth, 1.6);
  let aa = 1.0 - smoothstep(0.45, 0.9, max(fwidth(in.cell.y), fwidth(in.cell.x)));  // fade lines that get sub-pixel
  let wire = max(line(in.cell.y), 0.35 * line(in.cell.x * 0.5)) * aa;
  let flash = 1.0 + u.flash * 0.5 * u.fx.x;
  if (u.mode > 0.5) {
    // stereo overlay: additive wireframe, left = cyan, right = magenta
    let base = select(STEREO_L, STEREO_R, in.ch == 1u);
    return vec4<f32>(base * wire * (0.10 + 0.50 * in.h) * fade * u.exposure, 1.0);
  }
  let col = palette(clamp(in.h * 1.15, 0.0, 1.0), in.cell.x / f32(NX - 1u));
  var c = vec3<f32>(0.006, 0.008, 0.016) + col * 0.05 * shade * in.h;
  c += col * wire * (0.55 + 2.6 * in.h) * fade * flash;
  c = mix(BG, c, fade);
  return vec4<f32>(c * u.exposure, 1.0);
}

// ---- glass floor at zero level, with a fixed grid
struct FloorOut { @builtin(position) pos: vec4<f32>, @location(0) wp: vec3<f32> };
@vertex fn vs_wfloor(@builtin(vertex_index) vi: u32) -> FloorOut {
  var q = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0));
  let c = q[vi];
  let w = vec3<f32>(mix(-FLOOR_EXTENT, FLOOR_EXTENT, c.x), -0.004, mix(-FLOOR_EXTENT, FLOOR_EXTENT, c.y));
  return FloorOut(u.vp * vec4<f32>(w, 1.0), w);
}
@fragment fn fs_wfloor(in: FloorOut) -> @location(0) vec4<f32> {
  // The floor stands still; the terrain scrolls over it. Lines every 1.0 in x and every 5 history
  // rows (0.55) in z, the same pitch the terrain uses.
  let zc = -in.wp.z / (0.11 * 5.0);
  let gx = in.wp.x / 1.0;
  let aa = line_aa(max(fwidth(gx), fwidth(zc)));
  let g = max(gridline(gx), gridline(zc)) * aa;
  let dist = length(in.wp - u.cam.xyz);
  let fade = 1.0 - smoothstep(20.0, 130.0, dist);
  var c = vec3<f32>(0.005, 0.007, 0.014) + palette(0.25, 0.5) * g * 0.40;
  return vec4<f32>(mix(BG, c, fade) * u.exposure, 1.0);
}

// ---- glass wall at time 0 (front of the waterfall)
const WALL_H: f32 = TER_H;
struct WallOut { @builtin(position) pos: vec4<f32>, @location(0) wp: vec3<f32> };
@vertex fn vs_wall(@builtin(vertex_index) vi: u32) -> WallOut {
  var q = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0));
  let c = q[vi];
  let w = vec3<f32>(mix(-9.5, 9.5, c.x), c.y * WALL_H, 0.03);
  return WallOut(u.vp * vec4<f32>(w, 1.0), w);
}
fn wall_color(wp: vec3<f32>) -> vec4<f32> {
  let stereo = u.mode > 0.5;
  let col = palette(0.3, 0.5);
  // live spectrum at time 0 (newest history row), interpolated across bins; R = left/mixed, G = right
  let fx = clamp((wp.x / 19.0 + 0.5) * f32(NX - 1u), 0.0, f32(NX - 1u) - 0.001);
  let i0 = u32(floor(fx));
  let hh = mix(wf_h2(i0, 0u), wf_h2(i0 + 1u, 0u), fract(fx)) * WALL_H;
  let y = wp.y;
  let px = fwidth(y);
  let curve_l = 1.0 - smoothstep(0.0, 2.2 * px, abs(y - hh.x));
  let curve_r = select(0.0, 1.0 - smoothstep(0.0, 2.2 * px, abs(y - hh.y)), stereo);
  let below_l = select(0.0, 1.0, y < hh.x);
  let below_r = select(0.0, select(0.0, 1.0, y < hh.y), stereo);
  let lv = gridline(y / (WALL_H / 6.0));
  let ex = min(wp.x + 9.5, 9.5 - wp.x);
  let frame = max(1.0 - smoothstep(0.0, 0.05, ex), max(1.0 - smoothstep(0.0, 2.0 * px, y), 1.0 - smoothstep(0.0, 2.0 * px, WALL_H - y)));
  let cl = select(col, STEREO_L, stereo);
  let kb = select(1.0, 0.5, stereo);   // two fills overlap in stereo, so each is lighter
  var rgb = col * (0.25 * lv + 0.9 * frame)
          + cl * (3.0 * curve_l + 0.7 * kb * below_l)
          + STEREO_R * (3.0 * curve_r + 0.7 * kb * below_r);
  let a = clamp(0.045 + 0.32 * max(below_l, below_r) + 0.25 * lv + 0.7 * frame + 0.95 * max(curve_l, curve_r), 0.0, 1.0);
  return vec4<f32>(rgb, a);
}
@fragment fn fs_wall(in: WallOut) -> @location(0) vec4<f32> {
  let c = wall_color(in.wp);
  return vec4<f32>(c.rgb * u.exposure, c.a);
}

// ---- the wall mirrored below the floor; `wp` stays the real position so the colour is the same
@vertex fn vs_wall_refl(@builtin(vertex_index) vi: u32) -> WallOut {
  var q = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0));
  let c = q[vi];
  let w = vec3<f32>(mix(-9.5, 9.5, c.x), c.y * WALL_H, 0.03);
  return WallOut(u.vp * vec4<f32>(w.x, -w.y, w.z, 1.0), w);
}
@fragment fn fs_wall_refl(in: WallOut) -> @location(0) vec4<f32> {
  let c = wall_color(in.wp);
  return vec4<f32>(c.rgb * c.a * glass_reflect(vec3<f32>(in.wp.x, -in.wp.y, in.wp.z)) * u.exposure, 1.0);
}
