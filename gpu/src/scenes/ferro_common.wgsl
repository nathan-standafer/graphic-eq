// Ferrofluid Orb: the block of numbers the CPU hands the shaders (the same struct in the height pass and in
// the drawing) and the map between the height texture and directions on the sphere. Heights are fractions of
// the sphere's radius, so the whole shape scales with it.
struct OrbU {
  a: vec4<f32>,   // sphere radius, centre height, breathing (radius scale), light-rig angle
  b: vec4<f32>,   // height map width and height, spike half-width (radians), lobe falloff
  c: vec4<f32>,   // ring radius, height of the tallest spike, unused, unused
  sites: array<vec4<f32>, 192>,   // the spikes: direction of the site, current height (>= 0)
  lobes: array<vec4<f32>, 11>,    // the lobes: direction of the site, current height (signed)
};

const OPI: f32 = 3.14159265;
const OTAU: f32 = 6.28318531;

// The height map is a longitude / latitude image of the sphere: u runs from -pi to pi around it (0.5 faces +z,
// the camera's side), v from the top pole (0) to the bottom one (1). The mesh and the height pass both use this.
fn map_dir(uv: vec2<f32>) -> vec3<f32> {
  let phi = uv.x * OTAU - OPI;
  let th = 0.5 * OPI - uv.y * OPI;
  let ct = cos(th);
  return vec3<f32>(ct * sin(phi), sin(th), ct * cos(phi));
}
