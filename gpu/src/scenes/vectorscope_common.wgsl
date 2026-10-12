// Shared by the vectorscope's beam and composite shaders.
struct ScopeU {
  area: vec4<f32>,    // scope centre x, y and radius (px), gain applied to X and Y
  left: vec4<f32>,    // left-channel trace box: x0, y0, x1, y1 (px); empty when x1 <= x0
  right: vec4<f32>,   // right-channel trace box
  beam: vec4<f32>,    // core sigma and halo sigma (px), per-sample weight into the fast and the slow channel
  seg: vec4<u32>,     // first ring texel, segment count, ring size, unused
  trace: vec4<f32>,   // trace gain (box half heights per unit of amplitude), trace brightness, brightness floor on silence, unused
};
