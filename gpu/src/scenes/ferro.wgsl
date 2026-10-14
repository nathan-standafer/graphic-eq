// Ferrofluid Orb, drawing: a glossy black sphere floating over a reflective floor, displaced by the height map
// (see ferro_height.wgsl). Nothing on it is lit by a surface light: it is a black mirror, and what it shows is
// a procedural studio of a few strip lights in theme colours (a long one overhead, two tall ones at the sides,
// two behind it for the rim, a thin one low in front) that drifts slowly around it. The floor shows the same
// lights, the orb's mirror image, and the ring of eleven LEDs that holds the orb up.
@group(1) @binding(0) var<uniform> o: OrbU;
@group(1) @binding(1) var hmap: texture_2d<f32>;
@group(1) @binding(2) var hsamp: sampler;

const MESH_X: u32 = 384u;
const MESH_Y: u32 = 192u;
const FLOOR_EXTENT: f32 = 600.0;
const FLOOR_CLEAR: f32 = 0.05;      // the sphere never dips below this height
const F0: f32 = 0.07;               // reflectance of the fluid looking straight at it

// ---- the mesh. `stride` 2 draws every second vertex (the reflection is soft anyway).
struct OrbOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) p: vec3<f32>,        // where the point really is (the mirror image moves it below the floor)
  @location(1) n: vec3<f32>,        // unnormalised normal
  @location(2) hd: vec4<f32>,       // the undisplaced direction, and the height
};

fn orb_vertex(vi: u32, stride: u32, mirror: bool) -> OrbOut {
  // counter-clockwise seen from outside: u runs to the right and v downward
  var cs = array<vec2<u32>, 6>(
    vec2<u32>(0u, 0u), vec2<u32>(0u, 1u), vec2<u32>(1u, 0u),
    vec2<u32>(1u, 0u), vec2<u32>(0u, 1u), vec2<u32>(1u, 1u));
  let across = MESH_X / stride;
  let cell = vi / 6u;
  let c = cs[vi % 6u];
  let ix = (cell % across + c.x) * stride;
  let iy = (cell / across + c.y) * stride;
  let uv = vec2<f32>(f32(ix) / f32(MESH_X), f32(iy) / f32(MESH_Y));
  let dir = map_dir(uv);
  let t = textureSampleLevel(hmap, hsamp, uv, 0.0);
  let rb = o.a.z + t.w;                                          // radius, as a fraction of R
  var p = vec3<f32>(0.0, o.a.y, 0.0) + dir * (o.a.x * rb);
  p.y = max(p.y, FLOOR_CLEAR);
  var out: OrbOut;
  let q = select(p, vec3<f32>(p.x, -p.y, p.z), mirror);
  out.pos = u.vp * vec4<f32>(q, 1.0);
  out.p = p;
  out.n = rb * dir - t.xyz;                                      // normal of the surface r = R (b + h(dir))
  out.hd = vec4<f32>(dir, t.w);
  return out;
}
@vertex fn vs_orb(@builtin(vertex_index) vi: u32) -> OrbOut { return orb_vertex(vi, 1u, false); }
@vertex fn vs_orb_refl(@builtin(vertex_index) vi: u32) -> OrbOut { return orb_vertex(vi, 2u, true); }

// ---- the studio
fn rot(v: vec3<f32>, sc: vec2<f32>) -> vec3<f32> {
  return vec3<f32>(sc.y * v.x + sc.x * v.z, v.y, -sc.x * v.x + sc.y * v.z);
}

// A rectangular light of half-sizes (hw, hh) along the unit vectors ax, ay, centred on c. The ray from p along r
// meets its plane at distance t; `soft` blurs the edges by a fixed length and `foot` by a length that grows with t
// (the pixel's footprint on that plane, plus the surface's roughness), so a thin strip seen in a surface that curves
// fast fades instead of shimmering, and a rough floor smears a far light more than a near one.
fn strip(c: vec3<f32>, ax: vec3<f32>, ay: vec3<f32>, hw: f32, hh: f32, p: vec3<f32>, r: vec3<f32>, foot: f32, soft: f32) -> f32 {
  let nrm = cross(ax, ay);
  let dn = dot(r, nrm);
  let t = dot(c - p, nrm) / select(dn, 1e-4, abs(dn) < 1e-4);
  let tt = max(t, 0.0);
  let hit = p + r * tt - c;
  let s = soft + foot * tt;
  let mx = 1.0 - smoothstep(hw - s, hw + s, abs(dot(hit, ax)));
  let my = 1.0 - smoothstep(hh - s, hh + s, abs(dot(hit, ay)));
  return mx * my * select(0.0, 1.0, t > 0.0);
}

fn light_tint(t: f32, x: f32, white: f32) -> vec3<f32> {
  return mix(palette(t, x), vec3<f32>(1.0), white);
}

// What the lights add along the ray from p in direction r. Each light has its own audio role, and every one
// stays lit in silence, so the reflections keep sliding when the music stops. `flo` is 1 on the floor, which
// shows the overhead light but not the tall ones, which stay on the orb: three pillars of light
// on the floor would pull the eye from it.
fn strip_lights(p: vec3<f32>, r: vec3<f32>, soft: f32, foot: f32, flo: f32) -> vec3<f32> {
  let sc = vec2<f32>(sin(o.a.w), cos(o.a.w));
  let trend = 0.85 + 0.30 * u.sus.w;
  let key = (0.70 + 0.90 * u.sus.x + 1.0 * u.flash * u.fx.x) * trend;      // overhead: bass
  let side = (0.70 + 0.90 * u.sus.y + 0.5 * u.flash * u.fx.y) * trend;     // sides: mids
  let rim = (0.60 + 1.00 * u.sus.z + 1.2 * u.flash * u.fx.z) * trend;      // behind: highs
  let low = (0.40 + 0.80 * u.sus.w) * trend;
  let tall = 1.0 - flo;
  let ex = vec3<f32>(1.0, 0.0, 0.0);
  let ey = vec3<f32>(0.0, 1.0, 0.0);
  let ez = vec3<f32>(0.0, 0.0, 1.0);
  var e = vec3<f32>(0.0);
  e += light_tint(0.0, 0.50, 0.30) * (3.4 * key * mix(1.0, 0.35, flo)) * strip(rot(vec3<f32>(0.0, 5.4, 0.8), sc), rot(ex, sc), rot(ez, sc), 5.2, 0.55, p, r, foot, soft);
  e += light_tint(0.6, 0.10, 0.20) * (2.4 * side * tall) * strip(rot(vec3<f32>(-5.2, 2.7, 0.8), sc), rot(ey, sc), rot(ez, sc), 2.9, 0.34, p, r, foot, soft);
  e += light_tint(0.6, 0.90, 0.20) * (2.4 * side * tall) * strip(rot(vec3<f32>(5.2, 2.7, 0.8), sc), rot(ey, sc), rot(ez, sc), 2.9, 0.34, p, r, foot, soft);
  e += light_tint(0.8, 0.30, 0.15) * (3.0 * rim * tall) * strip(rot(vec3<f32>(-3.4, 2.4, -5.0), sc), rot(ey, sc), rot(ex, sc), 2.4, 0.24, p, r, foot, soft);
  e += light_tint(0.8, 0.70, 0.15) * (3.0 * rim * tall) * strip(rot(vec3<f32>(3.4, 2.4, -5.0), sc), rot(ey, sc), rot(ex, sc), 2.4, 0.24, p, r, foot, soft);
  e += light_tint(0.0, 0.50, 0.25) * (1.2 * low * (1.0 - flo)) * strip(rot(vec3<f32>(0.0, 0.45, 5.8), sc), rot(ex, sc), rot(ey, sc), 3.6, 0.10, p, r, foot, soft);
  return e;
}

// A faint haze at the horizon, so the sphere's silhouette reads against the dark even where no strip reflects.
fn dome(r: vec3<f32>) -> vec3<f32> {
  return palette(0.6, 0.5) * (0.008 + 0.034 * exp(-abs(r.y) * 3.0)) * (0.7 + 0.6 * u.sus.w);
}

fn band_env(i: u32) -> f32 {
  if (i < 3u) { return u.fx.x; }
  if (i < 7u) { return u.fx.y; }
  return u.fx.z;
}

// Light on the floor itself: the thin ring the orb floats over, a soft pool around it, and eleven LEDs along
// the front half of the ring, bass at the left, which fill with their bands' levels. `w` widens everything
// (the underside of the orb sees the floor blurred).
fn floor_light(q: vec3<f32>, w: f32) -> vec3<f32> {
  let rr = length(q.xz);
  let dr = rr - o.c.x;
  var c = vec3<f32>(0.0);
  if (abs(dr) < 3.0) {
    let sw = 0.014 + w;
    c += palette(0.45, 0.5) * exp(-0.5 * dr * dr / (sw * sw)) * (0.014 / sw) * (0.10 + 0.55 * u.sus.w);
    c += palette(0.45, 0.5) * exp(-dr * dr * 1.6) * (0.012 + 0.05 * u.sus.w);
    if (abs(dr) < 0.5) {
      let nw = 0.07 + w;
      let nk = (0.07 / nw) * (0.07 / nw);
      for (var i = 0u; i < 11u; i++) {
        let a = OPI * (1.0 - f32(i) / 10.0);
        let d = q.xz - vec2<f32>(cos(a), sin(a)) * o.c.x;
        let l = lvl(i);
        c += palette(clamp(l * 0.85, 0.0, 1.0), f32(i) / 10.0) * (0.03 + 2.4 * l * l) * (1.0 + u.flash * 0.9 * band_env(i)) * exp(-0.5 * dot(d, d) / (nw * nw)) * nk;
      }
    }
  }
  return c;
}

fn env(p: vec3<f32>, r: vec3<f32>, soft: f32, foot: f32) -> vec3<f32> {
  var e = strip_lights(p, r, soft, foot, 0.0);
  if (r.y < 0.0) {
    // looking down: the floor, with its ring
    let q = p + r * (-p.y / min(r.y, -1e-3));
    e += floor_light(q, 0.04 + foot * length(q - p)) + vec3<f32>(0.003, 0.004, 0.008);
  } else {
    e += dome(r);
  }
  return e;
}

// The fluid as seen from `eye` (the mirror image uses the camera's reflection in the floor).
fn orb_color(p: vec3<f32>, nrm: vec3<f32>, hd: vec4<f32>, eye: vec3<f32>, soft: f32, rough: f32) -> vec3<f32> {
  let n = normalize(nrm);
  let v = normalize(eye - p);
  let r = reflect(-v, n);
  let foot = length(fwidth(r)) * 0.8 + rough;
  let ndv = clamp(dot(n, v), 0.0, 1.0);
  let fres = F0 + (1.0 - F0) * pow(1.0 - ndv, 5.0);
  var c = env(p, r, soft, foot) * fres + vec3<f32>(0.0015, 0.002, 0.004);
  // the tallest spikes glow at their tips in the colour of their frequency
  let hn = clamp((hd.w - 0.22) / 0.30, 0.0, 1.0);
  c += palette(0.10 + 0.85 * hn, 0.5 + 0.5 * hd.y) * (hn * hn) * (0.55 + 1.3 * u.flash * u.fx.z);
  return c;
}

@fragment fn fs_orb(in: OrbOut) -> @location(0) vec4<f32> {
  return vec4<f32>(orb_color(in.p, in.n, in.hd, u.cam.xyz, 0.03, 0.012) * u.exposure, 1.0);
}

// The mirror image: the same fluid seen from the camera's reflection, faint from above and strong at grazing
// angles, and fading with the depth below the floor.
@fragment fn fs_orb_refl(in: OrbOut) -> @location(0) vec4<f32> {
  let eye_m = vec3<f32>(u.cam.x, -u.cam.y, u.cam.z);
  let col = orb_color(in.p, in.n, in.hd, eye_m, 0.10, 0.12);
  let q = vec3<f32>(in.p.x, -in.p.y, in.p.z);
  let v = normalize(u.cam.xyz - q);
  let fres = 0.10 + 0.90 * pow(1.0 - clamp(v.y, 0.0, 1.0), 3.0);
  let fade = exp(-in.p.y * 0.30);
  return vec4<f32>(col * fres * fade * 0.9 * u.exposure, 1.0);
}

// ---- the floor
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
  let to_cam = u.cam.xyz - in.wp;
  let dist = length(to_cam);
  let v = to_cam / dist;
  let rr = length(in.wp.xz);
  let grid = line(rr) * line_aa(fwidth(rr)) * (1.0 - smoothstep(5.0, 24.0, dist));
  let r = vec3<f32>(-v.x, v.y, -v.z);                     // the view ray reflected in the floor
  let foot = length(fwidth(r)) * 0.8 + 0.11;
  let fade = 1.0 - smoothstep(16.0, 80.0, dist);

  var c = vec3<f32>(0.0035, 0.0045, 0.0090) + palette(0.35, 0.5) * grid * 0.035;
  c += floor_light(in.wp, 0.0);
  // low-frequency hits send a ripple out from the ring
  for (var k = 0u; k < 8u; k++) {
    let rg = u.rings[k];
    if (rg.y <= 0.0) { break; }                           // the CPU packs live rings at the front
    if (rg.z < 0.5) {
      let d = (rr - (o.c.x + rg.x * 4.2)) / (0.22 + 0.18 * rg.x);
      if (abs(d) < 3.5) {
        c += palette(0.8, 0.5) * exp(-d * d) * rg.y * exp(-rg.x * 1.8) * 0.07 * u.flash;
      }
    }
  }
  // the polished floor mirrors the lights, strongly at grazing angles
  let fres = 0.05 + 0.95 * pow(1.0 - clamp(v.y, 0.0, 1.0), 3.0);
  c += strip_lights(in.wp, r, 0.10, foot, 1.0) * fres * 0.45;
  return vec4<f32>(mix(BG, c, fade) * u.exposure, 1.0);
}
