// Shared by the Ripple Pond simulation and drawing shaders. The pond is a rectangle on the floor; the wave
// equation runs on a grid of square cells over it (20 cells to a world unit), and every length the simulation
// uses is in cells. Heights are world units, so a slope (height per length) means the same in both.
struct RpU {
  grid: vec4<f32>,   // grid width, height (cells), 1/width, 1/height
  pond: vec4<f32>,   // x of the west edge, z of the north edge (world), width, depth (world)
  step: vec4<f32>,   // dt, c^2 (cells^2/s^2), impulse weight per step, impulses this frame
  damp: vec4<f32>,   // damping (1/s), sponge damping at the edge (1/s), sponge width (cells), height limit (world)
  look: vec4<f32>,   // restoring leak of the height (1/s), velocity limit (world/s), spare, spare
  imp: array<vec4<f32>, 16>,     // (cell x, cell y, sigma in cells, amplitude in world units); amplitude 0 = unused
  drops: array<vec4<f32>, 128>,  // 64 droplets, two vec4 each: (position, radius) (velocity, line); radius 0 = unused
};
