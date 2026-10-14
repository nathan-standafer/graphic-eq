// Fireworks (compute). A burst is a shell of stars thrown from the point where a rocket detonated; the CPU
// (`fireworks.rs`) decides when and where, and sends each burst's line position, strength, seed and star count
// in `pu.bursts[k]` and its centre and radius in `pu.extra[k]`. The first slot of a burst is the detonation
// flash, the rest are stars: directions from a Fibonacci spiral over the star's index, so the shell is round
// whatever its size, and each star picks a frequency from the spectrum at the moment of detonation (the sampler
// the fountain uses on the rise, here on the level), which sets its colour and a little of its speed.
//
// Where the hit landed on the line sets the type: lows throw a willow (a big, slow, heavy-dragged sphere whose
// long trails droop to the floor), mids a peony (an even, quicker sphere), highs a crackle (small, fast, bright,
// short-lived stars that twinkle). The type rides in the star's `look.w`: 2 * kind + a splash flash.
//
// look = (frequency 0..1, world radius, brightness, 2 * kind + splash)
const BOUNCE: f32 = 0.38;

fn dead_particle() -> Particle {
  return Particle(vec3<f32>(0.0), 0.0, vec3<f32>(0.0), 1.0, vec4<f32>(0.0));
}

// Keep in step with `kind_of` in fireworks.rs: 0 willow, 1 peony, 2 crackle (3 is the detonation flash).
fn kind_of(line: f32) -> f32 {
  if (line < 0.30) { return 0.0; }
  if (line < 0.66) { return 1.0; }
  return 2.0;
}

fn drag_of(kind: f32) -> f32 {
  if (kind < 0.5) { return 1.7; }
  if (kind < 1.5) { return 1.05; }
  if (kind < 2.5) { return 2.4; }
  return 0.0;
}

fn gravity_of(kind: f32) -> f32 {
  if (kind < 0.5) { return 3.6; }
  if (kind < 1.5) { return 2.4; }
  return 2.0;
}

fn spawn_star(s: u32, k: u32, n: u32) -> Particle {
  let bd = pu.bursts[k];
  let ex = pu.extra[k];
  let line = bd.x;
  let st = clamp(bd.y, 0.0, 1.0);
  let seed = bd.z;
  let cnt = bd.w;
  let kind = kind_of(line);

  var p: Particle;
  p.pos = ex.xyz;
  if (n == 0u) {                                          // the detonation flash: a short, bright, motionless dot
    p.vel = vec3<f32>(0.0);
    p.max_life = 0.30 + 0.12 * st;
    p.life = p.max_life;
    p.look = vec4<f32>(line, 0.16 + 0.22 * st, 1.4 + 2.2 * st, 6.0);
    return p;
  }

  let m = max(cnt - 1.0, 1.0);
  let i = f32(n - 1u);
  let y = 1.0 - 2.0 * (i + 0.5) / m;
  let rr = sqrt(max(1.0 - y * y, 0.0));
  let az = i * 2.3999632 + 6.2831853 * seed;
  var dir = vec3<f32>(rr * cos(az), y, rr * sin(az));
  dir = normalize(dir + 0.14 * (vec3<f32>(rnd(s, 1u), rnd(s, 2u), rnd(s, 3u)) - vec3<f32>(0.5)));

  // the star's frequency, picked by rejection sampling on the spectrum as it is now; a silent spectrum falls
  // back to the place the hit landed
  var f = clamp(line + 0.3 * (rnd(s, 4u) - 0.5), 0.0, 1.0);
  var lev = 0.4;
  for (var t = 0u; t < 12u; t++) {
    let a = min(u32(rnd(s, 8u + 2u * t) * f32(NX - 1u) + 0.5), NX - 1u);
    let l = hist_m(a, 0u);
    let tilt = 0.8 + 1.2 * f32(a) / f32(NX - 1u);         // the highs are quieter on real music
    if (rnd(s, 9u + 2u * t) < clamp(l * tilt, 0.0, 1.0)) {
      f = f32(a) / f32(NX - 1u);
      lev = l;
      break;
    }
  }

  // a star coasts to a stop against the drag, travelling `radius` in all, so the speed that gets it there
  let rad = ex.w;
  let drag = drag_of(kind);
  let spread = select(0.10, 0.26, kind < 0.5);            // a willow's stars vary more than a peony's
  let v0 = rad * drag / 0.93 * (0.84 + 0.30 * lev) * (1.0 + spread * (2.0 * rnd(s, 5u) - 1.0));
  p.vel = dir * v0;

  var life = 0.0;
  var size = 0.0;
  if (kind < 0.5) {
    life = 2.8 + 1.0 * rnd(s, 6u);
    size = 0.044;
  } else if (kind < 1.5) {
    life = 1.9 + 1.0 * rnd(s, 6u);
    size = 0.036;
  } else {
    life = 0.8 + 0.7 * rnd(s, 6u);
    size = 0.027;
  }
  p.max_life = life * (0.92 + 0.18 * st);
  p.life = p.max_life;
  size = size * (0.75 + 0.55 * st) * (0.85 + 0.30 * rnd(s, 7u));
  p.look = vec4<f32>(f, size, 0.30 + 0.32 * st, 2.0 * kind);
  return p;
}

fn spawn_particle(s: u32, burst: u32, n: u32) -> Particle {
  if (burst >= 6u) { return dead_particle(); }
  return spawn_star(s, burst, n);
}

fn integrate(p: Particle, dt: f32) -> Particle {
  var q = p;
  q.life -= dt;
  let kind = floor(q.look.w * 0.5);
  if (kind > 2.5) { return q; }                           // the flash only fades
  q.vel.y -= gravity_of(kind) * dt;
  q.vel += curl_wind(q.pos * 0.28, pu.step.y * 0.22) * (1.1 * dt * u.ctl.x);   // Motion scales the drift
  q.vel *= exp(-drag_of(kind) * dt);
  q.pos += q.vel * dt;
  let splash = (q.look.w - 2.0 * kind) * exp(-12.0 * dt);   // flash from the last bounce
  q.look.w = 2.0 * kind + splash;
  if (q.pos.y < 0.0 && q.vel.y < 0.0) {
    q.pos.y = abs(q.pos.y);                               // height is never negative
    q.vel = vec3<f32>(q.vel.x * 0.6, -q.vel.y * BOUNCE, q.vel.z * 0.6);
    q.look.w = 2.0 * kind + 1.0;
    q.life = min(q.life, 0.5);                            // a star that lands burns out
  }
  return q;
}
