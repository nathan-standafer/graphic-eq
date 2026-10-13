// Shared by the Smoke Jets simulation and screen shaders. All lengths are in simulation cells (square,
// about four screen pixels) and all speeds in cells per second, so the picture does not depend on the
// window size. Velocity is stored y-up (positive = up the screen) although texture rows run downward.
struct SimU {
  vel: vec4<f32>,    // velocity grid width, height, 1/width, 1/height
  dye: vec4<f32>,    // dye grid width, height, 1/width, 1/height (texels)
  scr: vec4<f32>,    // screen width, height (px), px per cell, floor height (px)
  step: vec4<f32>,   // dt, velocity drag (1/s), density decay (1/s), heat decay (1/s)
  force: vec4<f32>,  // vorticity strength (cells/s), buoyancy (cells/s^2 per unit heat), pressure keep, wobble (share of jet speed)
  misc: vec4<f32>,   // simulation time, seed, accent decay (1/s), thin-smoke loss (density/s)
  extra: vec4<f32>,  // max speed (cells/s), nozzle grip (1/s), spare, curl limit (1/s)
  diff: vec4<f32>,   // velocity smoothing per step, dye smoothing per step, spare, spare
  look: vec4<f32>,   // extinction, bump strength, ambient, haze gain
  noz: array<vec4<f32>, 11>,   // x (cells), sigma (cells), speed (cells/s), lamp level
  inj: array<vec4<f32>, 11>,   // density rate, heat rate, accent rate, band position (0..1)
};
