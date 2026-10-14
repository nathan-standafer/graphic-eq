// Chladni Sand: the uniform block and the plate's vibration pattern, shared by the simulation and the draw.
// Coordinates on a plate run 0..1 from its top-left corner (y down, like the screen); a plate's pattern is
//   a(x, y) = sum_i w_i * ( cos(n_i pi x) cos(m_i pi y) + s_i cos(m_i pi x) cos(n_i pi y) )
// with s_i = -1 (antisymmetric: the diagonal x = y stays still) or +1, and the weights normalised to sum to 1. Sand is thrown about wherever |a| is large and settles where it is
// zero, which draws the figure.
struct SandU {
  view: vec4<f32>,      // screen width, height (px), plates on screen, unused
  step: vec4<f32>,      // dt, time, unused, unused
  ids: vec4<u32>,       // seed for this frame's randomness, first and one past the last grain to initialise this frame, grains simulated
  lamp: vec4<f32>,      // angle of the lamp over the plates (rad), glint strength, unused, energy of one grain
  plate: array<vec4<f32>, 3>,   // per plate: left, top, side (px), vibration energy
  hit: array<vec4<f32>, 3>,     // per plate: shake multiplier from hits, glow from hits, unused, unused
  mm: array<vec4<f32>, 3>,      // per plate: the m of its four modes
  nn: array<vec4<f32>, 3>,      // ... and the n
  sg: array<vec4<f32>, 3>,      // ... and the sign s (-1 or +1)
  ww: array<vec4<f32>, 3>,      // ... and the weights (they sum to 1; a mode with weight 0 is skipped)
};

// One grain of sand: where it lies on its plate, how fast it is moving, the angle of the little facet that
// catches the lamp, and its own tone (brightness and size).
struct Grain {
  pos: vec2<f32>,
  vel: vec2<f32>,
  face: f32,
  tone: f32,
};

// The sand is accumulated per pixel of its plate in fixed point (two u32 per pixel: the body of the grains and the
// glints) by the compute pass, and the plate's fragment shader turns the sums into light.
const DENS_SCALE: f32 = 1024.0;

struct Wave { a: f32, grad: vec2<f32> }

fn wave(plate: u32, p: vec2<f32>) -> Wave {
  var a = 0.0;
  var g = vec2<f32>(0.0);
  for (var k = 0u; k < 4u; k++) {
    let w = su.ww[plate][k];
    if (w <= 1.0e-4) { continue; }
    let m = su.mm[plate][k];
    let n = su.nn[plate][k];
    let cnx = cos(n * PI * p.x);
    let snx = sin(n * PI * p.x);
    let cmx = cos(m * PI * p.x);
    let smx = sin(m * PI * p.x);
    let cny = cos(n * PI * p.y);
    let sny = sin(n * PI * p.y);
    let cmy = cos(m * PI * p.y);
    let smy = sin(m * PI * p.y);
    let s = su.sg[plate][k];
    a += w * (cnx * cmy + s * cmx * cny);
    g += w * PI * vec2<f32>(-n * snx * cmy - s * m * smx * cny, -m * cnx * smy - s * n * cmx * sny);
  }
  return Wave(a, g);
}

// Each plate's sand takes its own colour from the theme: lows first, highs last (a lone plate is in between).
fn plate_tint(plate: u32) -> vec3<f32> {
  if (su.view.z > 1.5) { return palette(0.36 * f32(plate), 0.5 * f32(plate)); }
  return palette(0.12, 0.5);
}

fn sand_pcg(v: u32) -> u32 {
  let s = v * 747796405u + 2891336453u;
  let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
  return (w >> 22u) ^ w;
}
// Uniform 0..1 from a frame seed, a grain and a stream index.
fn sand_hash(seed: u32, i: u32, k: u32) -> f32 {
  return f32(sand_pcg(sand_pcg(seed ^ (i * 2654435761u)) + k * 2891336453u) >> 8u) * (1.0 / 16777216.0);
}
