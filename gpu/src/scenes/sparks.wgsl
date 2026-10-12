// Draw side of the sparks: additive streaks tested against the columns' depth (they pass behind and
// through the glass), plus their mirror in the floor.
fn spark_colour(p: Particle, ii: u32) -> vec3<f32> {
  let age = 1.0 - p.life / p.max_life;
  let spark = p.look.w;
  let base = palette(clamp(p.look.z, 0.0, 1.0), p.look.x);
  let hot = mix(vec3<f32>(1.0), base, smoothstep(0.0, mix(0.35, 0.16, spark), age));   // white-hot, then the column's colour
  let fade = smoothstep(0.0, 0.03, age) * clamp(p.life / 0.5, 0.0, 1.0);
  let flicker = 0.72 + 0.28 * sin(u.t * (16.0 + 9.0 * fract(f32(ii) * 0.381)) + f32(ii));
  return hot * mix(1.6, 3.2, spark) * fade * mix(flicker, 1.0, spark);
}

@vertex fn vs_spark(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> SpriteOut {
  let p = parts[ii];
  if (p.life <= 0.0) { return sprite_dead(); }
  return sprite_make(p.pos, p.vel, p.look.y, vi, spark_colour(p, ii), false);
}

@vertex fn vs_spark_refl(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> SpriteOut {
  let p = parts[ii];
  if (p.life <= 0.0) { return sprite_dead(); }
  let v = normalize(u.cam.xyz - p.pos);
  let fres = 0.06 + 0.94 * pow(1.0 - clamp(abs(v.y), 0.0, 1.0), 4.0);
  return sprite_make(p.pos, p.vel, p.look.y, vi, spark_colour(p, ii) * (fres * exp(-0.5 * p.pos.y)), true);
}
