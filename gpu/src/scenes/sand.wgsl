// Chladni Sand, draw side: the dark metal plates with a level strip under each, and the sand on them. The compute
// pass has added every grain into per-pixel sums (see `sand_common.wgsl`); here each plate pixel turns its sums into
// light: a pale body tinted by the plate's colour and the lamp's glints, softly saturating where sand piles up.
// The colour comes from the theme, one hue per plate.
@group(1) @binding(1) var<uniform> su: SandU;
@group(1) @binding(2) var<storage, read> dens: array<vec2<u32>>;

const SAT_BODY: f32 = 1.6;      // where a pile of sand stops getting brighter
const SAT_GLINT: f32 = 1.2;

fn px_to_clip(p: vec2<f32>) -> vec4<f32> {
  return vec4<f32>(p.x / su.view.x * 2.0 - 1.0, 1.0 - p.y / su.view.y * 2.0, 0.0, 1.0);
}

fn quad_corner(vi: u32) -> vec2<f32> {
  var q = array<vec2<f32>, 6>(
    vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
    vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0));
  return q[vi];
}

// ------------------------------------------------------------------------------------------ plates
struct PlateOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) @interpolate(flat) plate: u32,
};

@vertex fn vs_plate(@builtin(vertex_index) vi: u32, @builtin(instance_index) pi: u32) -> PlateOut {
  let c = quad_corner(vi);
  let ph = su.plate[pi];
  let m = 0.10 * ph.z;
  let lo = ph.xy - vec2<f32>(m);
  let hi = ph.xy + vec2<f32>(ph.z + m, ph.z + 1.5 * m);   // room below for the level strip
  return PlateOut(px_to_clip(mix(lo, hi, c)), pi);
}

@fragment fn fs_plate(in: PlateOut) -> @location(0) vec4<f32> {
  let ph = su.plate[in.plate];
  let side = ph.z;
  let e = ph.w;
  let glow = su.hit[in.plate].y;
  let f = in.pos.xy - ph.xy;                  // px from the plate's corner
  let q = f / side;
  let tint = plate_tint(in.plate);
  let ldir = vec2<f32>(cos(su.lamp.x), sin(su.lamp.x));

  // signed distance to the plate's edge in px (negative inside)
  let hd = abs(f - vec2<f32>(0.5 * side)) - vec2<f32>(0.5 * side);
  let sd = length(max(hd, vec2<f32>(0.0))) + min(max(hd.x, hd.y), 0.0);
  let inside = clamp(0.5 - sd, 0.0, 1.0);

  // a little of the plate's colour spills onto the dark field around it
  var c = BG + tint * 0.0035 * exp(-max(sd, 0.0) / (0.025 * side)) * (0.5 + 0.5 * min(e, 1.0));

  if (inside > 0.0) {
    let row = hash21(vec2<f32>(floor(in.pos.x / 7.0), floor(in.pos.y)));   // brushed along x
    var body = vec3<f32>(0.0100, 0.0112, 0.0160) * (0.80 + 0.40 * row) * (1.0 + 0.45 * dot(q - vec2<f32>(0.5), ldir));
    let a = wave(in.plate, q).a;
    body += tint * (a * a) * (0.0025 + 0.006 * min(e, 1.2));       // the plate faintly shows where it moves
    // the edge catches the lamp
    let n = select(vec2<f32>(0.0, sign(f.y - 0.5 * side)), vec2<f32>(sign(f.x - 0.5 * side), 0.0), hd.x > hd.y);
    let rim = (1.0 - smoothstep(0.0, 2.2, -sd)) * (0.05 + 0.22 * (0.5 + 0.5 * dot(n, ldir)));
    c = mix(c, body + tint * rim, inside);

    // the sand: this pixel's sums
    let side_px = u32(side);
    let ip = vec2<u32>(clamp(floor(f), vec2<f32>(0.0), vec2<f32>(side - 1.0)));
    let v = dens[(in.plate * side_px + ip.y) * side_px + ip.x];
    let b = f32(v.x) * (1.0 / DENS_SCALE);
    let gl = f32(v.y) * (1.0 / DENS_SCALE);
    let pale = mix(tint, vec3<f32>(1.0), 0.16);
    let shine = mix(tint, vec3<f32>(1.0), 0.45);
    c += (pale * (SAT_BODY * (1.0 - exp(-b / SAT_BODY))) + shine * (SAT_GLINT * (1.0 - exp(-gl / SAT_GLINT)))) * inside;
  }

  // the level strip under the plate: thirty-two segments that fill with the plate's energy and flare on a hit
  let hh = max(0.007 * side, 1.5);
  let sy = f.y - 1.05 * side;
  if (abs(sy) < hh + 1.0 && q.x > 0.0 && q.x < 1.0) {
    let sx = q.x * 32.0;
    let within = fract(sx);
    let on_x = smoothstep(0.06, 0.14, within) * (1.0 - smoothstep(0.86, 0.94, within));
    let band = (1.0 - smoothstep(hh - 1.0, hh, abs(sy))) * on_x;
    let lit = select(0.0, 1.0, (floor(sx) + 0.5) / 32.0 < min(e, 1.0));
    c += tint * band * (0.012 + 0.50 * lit + 0.9 * glow);
  }
  return vec4<f32>(c * u.exposure, 1.0);
}
