// Waterfall: spectrum history as a scrolling wireframe terrain, inside glass walls at time 0 and along both sides.
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
  let fres = 0.14 + 0.86 * pow(1.0 - clamp(abs(v.y), 0.0, 1.0), 3.0);
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
  let out = e * glass_reflect(in.wp) * u.exposure;
  if (max(out.x, max(out.y, out.z)) < 0.002) { discard; }     // between the lines: nothing to add
  return vec4<f32>(out, 1.0);
}

@fragment fn fs_ter(in: TerOut) -> @location(0) vec4<f32> {
  let depth = clamp(in.cell.y / f32(NZ - 1u), 0.0, 1.0);
  let fade = pow(1.0 - depth, 1.6);
  let aa = 1.0 - smoothstep(0.45, 0.9, max(fwidth(in.cell.y), fwidth(in.cell.x)));  // fade lines that get sub-pixel
  let wire = max(line(in.cell.y), 0.35 * line(in.cell.x * 0.5)) * aa;
  if (u.mode > 0.5) {
    // stereo overlay: additive wireframe, left = cyan, right = magenta. Pixels between the lines add nothing.
    if (wire * fade < 0.01) { discard; }
    let base = select(STEREO_L, STEREO_R, in.ch == 1u);
    return vec4<f32>(base * wire * (0.10 + 0.50 * in.h) * fade * u.exposure, 1.0);
  }
  let n = normalize(cross(dpdx(in.wp), dpdy(in.wp)));
  let shade = 0.5 + 0.5 * abs(dot(n, normalize(vec3<f32>(0.3, 1.0, 0.5))));
  let flash = 1.0 + u.flash * 0.5 * u.fx.x;
  let col = palette(clamp(in.h * 1.15, 0.0, 1.0), in.cell.x / f32(NX - 1u));
  var c = vec3<f32>(0.006, 0.008, 0.016) + col * 0.05 * shade * in.h;
  c += col * wire * (0.55 + 2.6 * in.h) * fade * flash;
  // the far end fades into what the floor beside it looks like at that distance: dark nearby, the
  // horizon's fog only far away
  let haze = smoothstep(20.0, 130.0, length(in.wp - u.cam.xyz));
  c = mix(mix(BG, sky_fog(), haze), c, fade);
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

// A long light bar overhead, seen as a straight band across the glass. `t` is the tangent of the
// reflected ray's elevation toward the far end of the floor, which is constant along a line of
// constant depth; `half_t` is the bar's half width in that measure and `half_x` how far it extends sideways.
fn lightbar(r: vec3<f32>, t0: f32, half_t: f32, half_x: f32) -> f32 {
  let fz = max(-r.z, 0.05);
  let t = r.y / fz;
  let lx = abs(r.x) / fz;
  return smoothstep(half_t * 2.6, half_t * 0.4, abs(t - t0)) * smoothstep(half_x + 0.8, half_x, lx);
}

@fragment fn fs_wfloor(in: FloorOut) -> @location(0) vec4<f32> {
  let to_cam = u.cam.xyz - in.wp;
  let dist = length(to_cam);
  let v = to_cam / dist;
  let cosv = clamp(v.y, 0.0, 1.0);                          // 1 looking straight down onto the glass
  let fres = 0.07 + 0.93 * pow(1.0 - cosv, 5.0);            // glass: dim reflection from above, strong when grazing
  let dom = palette(0.25, 0.5);
  let fade = 1.0 - smoothstep(20.0, 130.0, dist);
  let r = vec3<f32>(-v.x, v.y, -v.z);                       // where a ray mirrored by the glass goes: up into the sky
  let pa = length(fwidth(r)) * 0.58;

  // The grid is etched into the surface, at zero level where the waterfall stands, so the terrain and its
  // reflection meet it at the same place.
  let gz = -in.wp.z / (0.11 * 5.0);
  let aa = line_aa(max(fwidth(in.wp.x), fwidth(gz)));
  let g = max(gridline(in.wp.x), gridline(gz)) * aa;
  var c = vec3<f32>(0.004, 0.006, 0.013);
  c += dom * g * 0.30 * (1.0 - 0.5 * fres);

  // light cast onto the glass by the lit wall: strongest right in front of it
  if (in.wp.z > 0.0 && in.wp.z < 5.0 && abs(in.wp.x) < 13.0) {
    let xi = u32(clamp((in.wp.x / 19.0 + 0.5) * f32(NX - 1u) + 0.5, 0.0, f32(NX - 1u)));
    let hn = wf_h2(xi, 0u);
    let spill = exp(-in.wp.z * 0.9) * smoothstep(13.0, 9.0, abs(in.wp.x));
    var pool = palette(clamp(hn.x * 1.15, 0.0, 1.0), clamp(in.wp.x / 19.0 + 0.5, 0.0, 1.0)) * hn.x * hn.x;
    if (u.mode > 0.5) { pool = STEREO_L * hn.x * hn.x + STEREO_R * hn.y * hn.y; }
    c += pool * spill * 0.14;
  }

  // and by the side and back walls, whose edge lines light the glass beside them
  let zr = max(-in.wp.z / ROW_DZ - u.res.z, 0.0);
  let ex = abs(in.wp.x) - SIDE_X;
  if (in.wp.z > -SIDE_LEN && in.wp.z < 0.5 && abs(ex) < 4.0) {
    let side = select(0u, 1u, in.wp.x > 0.0);
    let he = edge_h(side, zr);
    var lit = palette(clamp(he.x * 1.15, 0.0, 1.0), 0.5) * he.x * he.x;
    if (u.mode > 0.5) { lit = STEREO_L * he.x * he.x + STEREO_R * he.y * he.y; }
    c += lit * exp(-abs(ex) * 1.1) * wall_fade(zr) * (1.0 - smoothstep(0.0, 0.5, in.wp.z)) * 0.12;
  }
  let ez = in.wp.z + SIDE_LEN;
  if (abs(ez) < 4.0 && abs(in.wp.x) < SIDE_X + 3.0) {
    let hb = wf_h2(u32(clamp((in.wp.x / 19.0 + 0.5) * f32(NX - 1u) + 0.5, 0.0, f32(NX - 1u))), NZ - 1u);
    var lit = palette(clamp(hb.x * 1.15, 0.0, 1.0), 0.5) * hb.x * hb.x;
    if (u.mode > 0.5) { lit = STEREO_L * hb.x * hb.x + STEREO_R * hb.y * hb.y; }
    c += lit * exp(-abs(ez) * 1.1) * wall_fade(f32(NZ - 1u)) * (1.0 - smoothstep(SIDE_X, SIDE_X + 3.0, abs(in.wp.x))) * 0.12;
  }

  // what the glass mirrors of the sky: three long light bars (they slide as the view moves) and a
  // faint glow toward the horizon
  let boxes = 2.4 * lightbar(r, 1.35, 0.07, 3.0) + 1.8 * lightbar(r, 0.95, 0.025, 3.5) + 1.3 * lightbar(r, 0.62, 0.03, 2.5);
  c += mix(dom, vec3<f32>(1.0), 0.3) * boxes * fres;
  c += sky_color(r, pa, false) * (0.14 + 0.86 * pow(1.0 - cosv, 3.0));   // the sky, mirrored in the glass

  return vec4<f32>(mix(sky_fog(), c, fade) * u.exposure, 1.0);
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

// ---- glass side and back walls along the terrain's left, right and far edges. Like the front wall,
// each carries the signal line, here the terrain's edge, so the line follows the intersection of the
// waterfall with the glass. The side walls follow the history rows as they slide away; the back wall
// shows the oldest row.
const SIDE_X: f32 = 9.5;
const ROW_DZ: f32 = 0.11;                    // world depth of one history row
const SIDE_LEN: f32 = 192.0 * 0.11;          // the terrain's depth, plus the row sliding off its end
const SIDE_TICK: f32 = 2.2;                  // spacing of the fixed vertical lines (four floor-grid cells)
struct SideOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) wp: vec3<f32>,
  @location(1) @interpolate(flat) side: u32,
};
// The terrain's edge lies in each of these planes: the glass is pulled a hair toward the camera so the
// line wins the depth tie.
fn glass_pos(w: vec3<f32>, mirrored: bool) -> vec4<f32> {
  var p = u.vp * vec4<f32>(w.x, select(w.y, -w.y, mirrored), w.z, 1.0);
  p.z -= 2.0e-5 * p.w;
  return p;
}
fn side_vertex(vi: u32, side: u32, mirrored: bool) -> SideOut {
  var q = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0));
  let c = q[vi];
  let w = vec3<f32>(select(-SIDE_X, SIDE_X, side == 1u), c.y * WALL_H, mix(0.03, -SIDE_LEN, c.x));
  return SideOut(glass_pos(w, mirrored), w, side);
}
@vertex fn vs_side(@builtin(vertex_index) vi: u32, @builtin(instance_index) side: u32) -> SideOut {
  return side_vertex(vi, side, false);
}
@vertex fn vs_side_refl(@builtin(vertex_index) vi: u32, @builtin(instance_index) side: u32) -> SideOut {
  return side_vertex(vi, side, true);
}
fn back_vertex(vi: u32, mirrored: bool) -> WallOut {
  var q = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0));
  let c = q[vi];
  let w = vec3<f32>(mix(-SIDE_X, SIDE_X, c.x), c.y * WALL_H, -SIDE_LEN);
  return WallOut(glass_pos(w, mirrored), w);
}
@vertex fn vs_back(@builtin(vertex_index) vi: u32) -> WallOut { return back_vertex(vi, false); }
@vertex fn vs_back_refl(@builtin(vertex_index) vi: u32) -> WallOut { return back_vertex(vi, true); }

// How strongly the far walls show: the terrain's own lines fade out with depth, but the glass keeps
// enough to close the box.
fn wall_fade(zr: f32) -> f32 {
  return mix(0.3, 1.0, pow(1.0 - clamp(zr / f32(NZ - 1u), 0.0, 1.0), 1.6));
}
// The terrain's height along one side edge at a fractional row position, as (left or mixed, right).
fn edge_h(side: u32, zr: f32) -> vec2<f32> {
  let zc = min(zr, f32(NZ - 1u));
  let i0 = min(u32(zc), NZ - 2u);
  let xi = select(0u, NX - 1u, side == 1u);
  return mix(wf_h2(xi, i0), wf_h2(xi, i0 + 1u), zc - f32(i0));
}

// A pane of glass at height `y` carrying the signal line(s) at `hh` (left or mixed, right) in world
// units; `vgrid` is the fixed vertical grid line and `post` a vertical frame post.
fn pane_color(y: f32, hh: vec2<f32>, vgrid: f32, post: f32, fade: f32) -> vec4<f32> {
  let stereo = u.mode > 0.5;
  let col = palette(0.3, 0.5);
  let px = fwidth(y);
  // distance to each signal line in screen pixels, so steep stretches stay as crisp as flat ones
  let fl = y - hh.x;
  let fr = y - hh.y;
  let gl = max(fwidth(fl), 0.35 * px);
  let gr = max(fwidth(fr), 0.35 * px);
  let curve_l = 1.0 - smoothstep(0.0, 2.2, abs(fl) / gl);
  let curve_r = select(0.0, 1.0 - smoothstep(0.0, 2.2, abs(fr) / gr), stereo);
  let below_l = 1.0 - smoothstep(-1.0, 1.0, fl / gl);
  let below_r = select(0.0, 1.0 - smoothstep(-1.0, 1.0, fr / gr), stereo);
  let lv = gridline(y / (WALL_H / 6.0));
  let frame = max(max(1.0 - smoothstep(0.0, 2.0 * px, y), 1.0 - smoothstep(0.0, 2.0 * px, WALL_H - y)), post);
  let cl = select(col, STEREO_L, stereo);
  let kb = select(1.0, 0.5, stereo);   // two fills overlap in stereo, so each is lighter
  let rgb = col * (0.25 * lv + 0.14 * vgrid + 0.9 * frame)
          + cl * (3.0 * curve_l + 0.7 * kb * below_l)
          + STEREO_R * (3.0 * curve_r + 0.7 * kb * below_r);
  let a = clamp(0.045 + 0.32 * max(below_l, below_r) + 0.25 * lv + 0.12 * vgrid + 0.7 * frame + 0.95 * max(curve_l, curve_r), 0.0, 1.0);
  return vec4<f32>(rgb, a * fade);
}

fn side_color(wp: vec3<f32>, side: u32) -> vec4<f32> {
  // history row under this point (rows slide away continuously)
  let zr = max(-wp.z / ROW_DZ - u.res.z, 0.0);
  let vz = gridline(wp.z / SIDE_TICK) * line_aa(fwidth(wp.z / SIDE_TICK));
  return pane_color(wp.y, edge_h(side, zr) * WALL_H, vz, 0.0, wall_fade(zr));
}
fn back_color(wp: vec3<f32>) -> vec4<f32> {
  // the oldest history row, interpolated across bins like the front wall's live line
  let fx = clamp((wp.x / 19.0 + 0.5) * f32(NX - 1u), 0.0, f32(NX - 1u) - 0.001);
  let i0 = u32(floor(fx));
  let hh = mix(wf_h2(i0, NZ - 1u), wf_h2(i0 + 1u, NZ - 1u), fract(fx)) * WALL_H;
  let vx = gridline(wp.x / 2.0) * line_aa(fwidth(wp.x / 2.0));
  let ex = min(wp.x + SIDE_X, SIDE_X - wp.x);
  return pane_color(wp.y, hh, vx, 1.0 - smoothstep(0.0, 0.05, ex), wall_fade(f32(NZ - 1u)));
}
@fragment fn fs_side(in: SideOut) -> @location(0) vec4<f32> {
  let c = side_color(in.wp, in.side);
  return vec4<f32>(c.rgb * u.exposure, c.a);
}
@fragment fn fs_side_refl(in: SideOut) -> @location(0) vec4<f32> {
  let c = side_color(in.wp, in.side);
  return vec4<f32>(c.rgb * c.a * glass_reflect(vec3<f32>(in.wp.x, -in.wp.y, in.wp.z)) * u.exposure, 1.0);
}
@fragment fn fs_back(in: WallOut) -> @location(0) vec4<f32> {
  let c = back_color(in.wp);
  return vec4<f32>(c.rgb * u.exposure, c.a);
}
@fragment fn fs_back_refl(in: WallOut) -> @location(0) vec4<f32> {
  let c = back_color(in.wp);
  return vec4<f32>(c.rgb * c.a * glass_reflect(vec3<f32>(in.wp.x, -in.wp.y, in.wp.z)) * u.exposure, 1.0);
}
