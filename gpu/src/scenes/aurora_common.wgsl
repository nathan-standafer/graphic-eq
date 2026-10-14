// Aurora: the uniform block and helpers shared by the ribbon pass (half resolution, into an offscreen
// target) and the screen pass (stars, treeline, lake, and the target shown through its own composite).
//
// The ribbon pass is theme-free: it stores how much of each palette role (dominant, supporting, accent)
// the light at a pixel carries, plus how much it dims what lies behind it. The screen pass turns those
// weights into colour, so two looks of the scene crossfade with their own palettes from one target.
struct AuroraU {
  a: vec4<f32>,   // horizon (uv y, 0 = top), half width of the view at distance 1, ribbon count, fold clock
  b: vec4<f32>,   // ray clock, lake clock, glow (the signal gate), spread (the slow energy trend)
  c: vec4<f32>,   // fold depth (mids), ripple depth (bass), lift (bass), unused
  rib: array<vec4<f32>, 32>,   // four per ribbon, farthest first
};
@group(1) @binding(0) var<uniform> au: AuroraU;

const EYE_Y: f32 = 1.6;        // eye height over the lake (world units)
const NCOL: u32 = 256u;        // columns across a ribbon
const XS: f32 = 1.4;           // a ribbon reaches this far past the screen edge (screen half widths)
const V_LO: f32 = -0.16;       // the quad reaches this far below its base: the shader draws that edge itself
const V_HI: f32 = 1.30;        // and this far above its column's height, where the light has faded out

fn ihash(n: u32) -> u32 {
  var v = n * 747796405u + 2891336453u;
  v = ((v >> ((v >> 28u) + 4u)) ^ v) * 277803737u;
  return (v >> 22u) ^ v;
}
// Hash of an integer-valued float to 0..1.
fn hash11(x: f32) -> f32 {
  return f32(ihash(bitcast<u32>(i32(x))) & 0xffffffu) * (1.0 / 16777216.0);
}
fn vnoise(x: f32) -> f32 {
  let i = floor(x);
  let f = x - i;
  let w = f * f * (3.0 - 2.0 * f);
  return mix(hash11(i), hash11(i + 1.0), w);
}

// How much of (dominant, supporting, accent) the palette ramp blends at t: the weights `palette` applies.
fn role_w(t: f32) -> vec3<f32> {
  let lo = clamp(t / 0.6, 0.0, 1.0);
  let hi = clamp((t - 0.6) / 0.4, 0.0, 1.0);
  return select(vec3<f32>(0.0, 1.0 - hi, hi), vec3<f32>(1.0 - lo, lo, 0.0), t < 0.6);
}
