// The fountain throws sparks where the volume is rising. The newest history row carries, for each of the 192
// frequency bins, how steeply that frequency's level is climbing right now (see `analyzer::Levels::rise`): a
// kick is a steep rise in the lows, a snare in the highs, and steady sound is no rise at all. A spawn picks a
// frequency by rejection sampling on that rise, above an adjustable threshold, and launches a spark from
// that frequency's place on the line; the steeper the rise, the more sparks, the bigger, brighter and faster.
// Sparks arc under gravity, drift on a gentle wind and bounce on the floor.
const G: f32 = 14.0;
const V_MAX: f32 = 18.0;       // launch speed of the steepest rise: an apex ~11 units up, off the top of the screen
const BOUNCE: f32 = 0.5;
// The rise is relative to the music's own hardest recent hits (about 1 at those; see `modulation::RiseNorm`),
// weighted up toward the treble. A frequency starts throwing sparks at RISE_START and throws its strongest at
// RISE_FULL_SPARK, both divided by the sensitivity slider. (The Rust side keeps these two numbers in step;
// they are calibrated in `modulation.rs`.)
const RISE_START: f32 = 0.42;
const RISE_FULL_SPARK: f32 = 1.0;

fn dead_particle() -> Particle {
  return Particle(vec3<f32>(0.0), 0.0, vec3<f32>(0.0), 1.0, vec4<f32>(0.0));
}

fn life_for(speed: f32, r: f32) -> f32 {
  return clamp(2.2 * speed / G + 0.9 + 0.5 * r, 0.8, 4.0);
}

fn spawn_spark(s: u32) -> Particle {
  let sens = pu.extra[0].x;
  var xi = 0u;
  var pw = 0.0;
  var ok = false;
  for (var k = 0u; k < 16u; k++) {
    let a = min(u32(rnd(s, 2u * k) * f32(NX - 1u) + 0.5), NX - 1u);
    let g = hist_rise(a, 0u) * sens;
    let q = clamp((g - RISE_START) / (RISE_FULL_SPARK - RISE_START), 0.0, 1.0);
    if (rnd(s, 2u * k + 1u) < q) { xi = a; pw = q; ok = true; break; }
  }
  if (!ok) { return dead_particle(); }
  let t = f32(xi) / f32(NX - 1u);                         // 0 bass .. 1 treble
  let x01 = (f32(xi) + 3.0 * (rnd(s, 40u) - 0.5)) / f32(NX - 1u);   // jittered across a few bins so no bin is a lone column
  // every spark takes its own angle and speed, so a hit throws a fan of arcs rather than a curtain of lines
  let elev = mix(0.45, 0.95, t) * pow(rnd(s, 42u), 0.6);  // from vertical; treble scatters wider than bass
  let th = 6.2831853 * rnd(s, 43u);
  let speed = V_MAX * (0.20 + 0.80 * pw) * (0.40 + 0.60 * rnd(s, 41u));
  var p: Particle;
  p.pos = vec3<f32>((x01 - 0.5) * u.ctl.y + 0.5 * (rnd(s, 47u) - 0.5), 0.03, 0.6 * (rnd(s, 44u) - 0.5));
    p.vel = vec3<f32>(sin(elev) * cos(th), cos(elev), 0.6 * sin(elev) * sin(th)) * speed;
  p.max_life = life_for(speed, rnd(s, 45u));
  p.life = p.max_life;
  let size = mix(0.055, 0.020, pow(t, 0.7)) * (0.45 + 1.1 * pw) * (0.8 + 0.4 * rnd(s, 46u));   // a harder hit throws bigger sparks
  p.look = vec4<f32>(t, size, pw * 0.85, 0.0);            // z: how steep the rise was, which sets colour and glow
  return p;
}

fn spawn_particle(s: u32, burst: u32, n: u32) -> Particle {
  return spawn_spark(s);
}

fn integrate(p: Particle, dt: f32) -> Particle {
  var q = p;
  q.vel.y -= G * dt;
  q.vel += curl_wind(q.pos * 0.35, pu.step.y * 0.3) * (1.4 * dt);
  q.vel *= exp(-0.2 * dt);
  q.pos += q.vel * dt;
  q.life -= dt;
  q.look.w *= exp(-14.0 * dt);                             // splash flash from the last bounce
  if (q.pos.y < 0.0 && q.vel.y < 0.0) {
    q.pos.y = abs(q.pos.y);                                // height is never negative: a spark that dipped below the floor is put back above it
    q.vel = vec3<f32>(q.vel.x * 0.75, -q.vel.y * BOUNCE, q.vel.z * 0.75);
    q.look.w = 1.0;
    if (q.vel.y < 1.2) { q.life = min(q.life, 0.2); }      // settled: let it fade
  }
  return q;
}
