// The vectorscope's screen: the persistence buffer shown as phosphor, over a fixed graticule.
@group(1) @binding(0) var persist: texture_2d<f32>;
@group(1) @binding(1) var<uniform> sc: ScopeU;

@vertex fn vs_scope(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
  var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
  return vec4<f32>(p[vi], 0.0, 1.0);
}

fn in_box(p: vec2<f32>, b: vec4<f32>) -> bool {
  return b.z > b.x && p.x >= b.x - 40.0 && p.x <= b.z + 40.0 && p.y >= b.y - 40.0 && p.y <= b.w + 40.0;
}
// A 1 px line at distance `d` (px) from the pixel, with a soft edge.
fn hairline(d: f32, width: f32) -> f32 {
  return 1.0 - smoothstep(0.5 * width, 0.5 * width + 1.0, d);
}

@fragment fn fs_scope(@builtin(position) fp: vec4<f32>) -> @location(0) vec4<f32> {
  let p = fp.xy;
  let ph = textureLoad(persist, vec2<i32>(p), 0);
  let dom = palette(0.0, 0.5);
  let sup = palette(0.6, 0.5);
  let on_trace = in_box(p, sc.left) || in_box(p, sc.right);
  let tint = select(dom, sup, on_trace);
  // the flash of the beam burns toward white; the afterglow keeps the phosphor's own colour
  var c = mix(tint, vec3<f32>(1.0), clamp(ph.x * 0.16, 0.0, 0.65)) * ph.x + tint * ph.y;

  // ---- graticule: faint, fixed, and still there when the signal stops
  let pulse = 1.0 + u.flash * 1.6 * u.fx.x;                      // bass hits flick the outer ring
  let cen = sc.area.xy;
  let rad = sc.area.z;
  let d = p - cen;
  let r = length(d);
  var g = 0.0;
  g = max(g, hairline(abs(r - rad), 1.4) * 0.16 * pulse);            // the outer ring
  for (var k = 1; k < 4; k++) {
    g = max(g, hairline(abs(r - rad * 0.25 * f32(k)), 1.0) * 0.055);
  }
  let inside = r < rad * 1.02;
  // axes: mono is the vertical, anti-phase the horizontal; the diagonals are left-only and right-only
  g = max(g, select(0.0, hairline(abs(d.x), 1.0) * 0.07, inside));
  g = max(g, select(0.0, hairline(abs(d.y), 1.0) * 0.07, inside));
  let dash = step(0.5, fract(r / (rad * 0.05)));
  g = max(g, select(0.0, hairline(abs(d.x - d.y) * 0.70710678, 1.0) * 0.045 * dash, inside));
  g = max(g, select(0.0, hairline(abs(d.x + d.y) * 0.70710678, 1.0) * 0.045 * dash, inside));
  // ticks every eighth of the radius along both axes
  let tick_x = hairline(abs(fract(d.x / (rad * 0.125) + 0.5) - 0.5) * rad * 0.125, 1.0) * step(abs(d.y), 4.0);
  let tick_y = hairline(abs(fract(d.y / (rad * 0.125) + 0.5) - 0.5) * rad * 0.125, 1.0) * step(abs(d.x), 4.0);
  g = max(g, select(0.0, max(tick_x, tick_y) * 0.12, inside));
  // the face of the tube glows very slightly
  let face = 0.012 * (1.0 - smoothstep(0.0, rad * 1.05, r));
  var grat = dom * (g + face);

  // trace boxes: a frame, a centre line and quarter lines, and time ticks
  var tg = 0.0;
  for (var side = 0; side < 2; side++) {
    let b = select(sc.left, sc.right, side == 1);
    if (b.z > b.x && p.x >= b.x - 2.0 && p.x <= b.z + 2.0 && p.y >= b.y - 2.0 && p.y <= b.w + 2.0) {
      let w = b.z - b.x;
      let hh = b.w - b.y;
      let cy = b.y + 0.5 * hh;
      let edge = min(min(p.x - b.x, b.z - p.x), min(p.y - b.y, b.w - p.y));
      tg = max(tg, hairline(edge, 1.2) * 0.09);
      tg = max(tg, hairline(abs(p.y - cy), 1.0) * 0.08);
      tg = max(tg, hairline(abs(abs(p.y - cy) - 0.25 * hh), 1.0) * 0.035 * step(0.5, fract((p.x - b.x) / 14.0)));
      let tx = (p.x - b.x) / w * 6.0;
      tg = max(tg, hairline(abs(fract(tx + 0.5) - 0.5) * w / 6.0, 1.0) * 0.05 * step(abs(p.y - cy), 0.35 * hh));
    }
  }
  grat += sup * tg;

  return vec4<f32>((c + grat) * u.exposure, 1.0);
}
