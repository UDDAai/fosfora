// Boids — shader.gallery (boids)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: dusk

const SG_C0: vec3f = vec3f(0.420000, 0.475000, 0.671000);
const SG_C1: vec3f = vec3f(0.690000, 0.522000, 0.580000);
const SG_C2: vec3f = vec3f(0.871000, 0.667000, 0.498000);
const SG_C3: vec3f = vec3f(0.290000, 0.318000, 0.475000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// boids (Shoal) - a murmuration streaming across the whole frame at dusk.
// There is no feedback buffer, so the flock is not simulated step by step;
// it is written as a closed-form motion that has the three boid rules baked
// in. Alignment: every bird shares one swarm path with an analytic velocity,
// plus coherent turning waves that roll through the flock, so neighbours
// bank together. Cohesion: a slow travelling density field decides where
// the flock is thick and where it is empty, and each bird is pulled up the
// gradient of that field, so clumps pile up and gaps open between them.
// Separation: every bird keeps its own slot in a hashed lattice with its own
// jostle, so no two ever coincide. Each bird is a small swept-wing mark
// oriented along its own finite-difference velocity, flapping on its own
// clock, dragging a fading motion trail. Three parallax layers give depth:
// far birds small, dim and fogged; near ones large and bright.


// tweakable params (see meta.json; the runtime feeds defaults)

var<private> c0: vec3f;
var<private> c1: vec3f;
var<private> c2: vec3f;
var<private> c3: vec3f;
fn hash21(p_in: vec2f) -> f32 {
  var p = p_in;

  p = fract(p * vec2f(234.34, 435.345));
  p += dot(p, p + 34.23);
  return fract(p.x * p.y);
}
fn hash22(p: vec2f) -> vec2f {
 return vec2f(hash21(p), hash21(p + vec2f(41.3, 17.9))); }

fn vnoise(p: vec2f) -> f32 {

  var i = floor(p);
  let f = fract(p);
  var u = f * f * (3.0 - 2.0 * f);
  var a = hash21(i);
  let b = hash21(i + vec2f(1.0, 0.0));
  let c = hash21(i + vec2f(0.0, 1.0));
  let d = hash21(i + vec2f(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}
const M2: mat2x2<f32> = mat2x2<f32>(vec2f(0.80, 0.60), vec2f(-0.60, 0.80));
fn fbm(p_in: vec2f) -> f32 {
  var p = p_in;

  var a = 0.5;
  var s = 0.0;
  for (var i: i32 = 0; i < 4; i += 1) {
    s += a * vnoise(p);
    p = M2 * p * 2.03 + vec2f(11.7, 5.3);
    a *= 0.5;
  }
  return s * 1.07;
}

// the swarm path in frame units and its analytic velocity: a stream along
// x with slow banking S-curves, so the whole flock crosses and re-crosses
fn swarmPos(t: f32) -> vec2f {

  return vec2f(0.30 * t + 0.45 * sin(t * 0.21),
              0.70 * sin(t * 0.17 + 1.0) + 0.25 * sin(t * 0.41));
}
fn swarmVel(t: f32) -> vec2f {

  return vec2f(0.30 + 0.0945 * cos(t * 0.21),
              0.119 * cos(t * 0.17 + 1.0) + 0.1025 * cos(t * 0.41));
}

// cohesion field: three travelling plane waves in swarm space. Returns the
// value and its analytic gradient so birds can be pulled toward the thick.
fn cohesion(x: vec2f, t: f32) -> vec3f {

  let k1 = vec2f(3.1, 1.2);
  let k2 = vec2f(-1.4, 2.7);
  let k3 = vec2f(2.0, -2.4);
  let k4 = vec2f(0.7, 0.5);
  let a1 = dot(x, k1) + t * 0.31;
  let a2 = dot(x, k2) - t * 0.23;
  let a3 = dot(x, k3) + t * 0.19 + 2.0;
  let a4 = dot(x, k4) - t * 0.11 + 1.0;      // the broad wave that opens big gaps
  var v = sin(a1) + 0.8 * sin(a2) + 0.6 * sin(a3) + 1.1 * sin(a4);
  let g = k1 * cos(a1) + 0.8 * k2 * cos(a2) + 0.6 * k3 * cos(a3) + 1.1 * k4 * cos(a4);
  return vec3f(v, g);
}

// per-bird displacement inside its slot at time t: cohesion pull, turning
// waves, individual jostle. In units of one cell.
fn jostle(wc: vec2f, h: vec2f, t: f32, turn: f32) -> vec2f {

  var co = cohesion(wc, t);
  let pull = co.yz / (1.0 + length(co.yz)) * 0.38;
  // coherent turning waves rolling through the flock
  let wph = dot(wc, vec2f(2.6, 1.4)) - t * 1.3;
  let wph2 = dot(wc, vec2f(-1.2, 2.9)) + t * 0.9;
  let wave = turn * 0.12 * vec2f(sin(wph) + 0.6 * cos(wph2), cos(wph) * 0.7 + 0.6 * sin(wph2));
  // the individual: a small jitter on its own clock
  let jit = 0.05 * vec2f(sin(t * (1.6 + 1.5 * h.x) + h.y * 6.2832),
                         cos(t * (1.3 + 1.5 * h.y) + h.x * 6.2832));
  return pull + wave + jit;
}

// one parallax layer of the flock. p frame coords, cell in frame units,
// par parallax factor, near 0 (far) .. 1 (close)
fn flock(p: vec2f, cell: f32, par: f32, seed: f32, near: f32, t: f32, px: f32, acc: ptr<function, vec3f>) {

  let O = swarmPos(t) * par;
  let Ov = swarmVel(t) * par;
  let w = p - O;                    // swarm-space coords
  let q = w / cell;
  let ci = floor(q);

  let bodyLen = (0.20 + 0.06 * (1.0 - near)) * param(3u) * cell;
  let trailLen = 0.55 * param(2u) * cell;
  let dens = clamp(param(1u), 0.0, 1.0);
  let turn = clamp(param(4u), 0.0, 2.0);
  let dt = 0.06;

  var birdCol = mix(c2, c1, 0.3) * (0.6 + 0.7 * near);
  let fog = mix(c3, c0, 0.4) * 0.35;
  birdCol = mix(fog + c2 * 0.15, birdCol, 0.35 + 0.65 * near);

  for (var y: i32 = -1; y <= 1; y += 1) {
    for (var x: i32 = -1; x <= 1; x += 1) {
      let id = ci + vec2f(f32(x), f32(y));
      let h = hash22(id + seed * 13.1);
      let h3 = hash21(id * 1.71 + seed * 3.7);
      // the slot centre in swarm space, jittered
      let slot = (id + 0.5 + (h - 0.5) * 0.6) * cell;
      // cohesion decides whether this slot holds a bird
      var co = cohesion(slot, t).x;
      let density = smoothstep(0.5 - 2.0 * dens, 2.0 - 1.6 * dens, co + 0.2);
      if (h3 > density) { continue; }

      let d0 = jostle(slot, h, t, turn);
      let d1 = jostle(slot, h, t - dt, turn);
      let pos = slot + d0 * cell;
      // heading: the swarm velocity plus a damped share of the local jostle,
      // so neighbours bank together instead of scattering
      let vel = Ov + 0.45 * (d0 - d1) * cell / dt;
      let spd = length(vel);
      var u = vel / max(spd, 1e-4);
      var v = vec2f(-u.y, u.x);

      let r = w - pos;
      let loc = vec2f(dot(r, u), dot(r, v));

      // the boid glyph: a dart, nose forward, with a notched tail, the
      // classic flocking mark. The flap on this bird alone narrows and
      // widens it, so the flock shimmers instead of stamping one shape.
      let flap = sin(t * (9.0 + 4.0 * h.x) + h.y * 6.2832);
      let wid = 0.42 * (0.65 + 0.35 * abs(flap));
      let ay = vec2f(loc.x / bodyLen, abs(loc.y) / bodyLen);   // fold, in body units
      let taper = ay.y - wid * (0.62 - ay.x) / 1.0;           // side edge to the nose
      let notch = (-0.38 + 0.45 * ay.y) - ay.x;               // concave tail
      let nose = ay.x - 0.62;
      let mark = max(max(taper * 0.85, notch * 0.8), nose) * bodyLen;

      // soft focus on the near layer, crisp specks far away
      let aa = px * (0.8 + 2.2 * near);
      let alpha = 1.0 - smoothstep(-aa * 0.5, aa, mark);
      // motion trail: a fading streak behind the tail, longer when faster
      let tl = trailLen * clamp(spd * 2.2, 0.6, 2.4);
      let s = clamp(-loc.x / tl, 0.0, 1.0);
      let onTrail = step(loc.x, 0.0);
      let tw = bodyLen * 0.05 * (1.0 - s) + px * 0.4;
      var trail = (1.0 - smoothstep(tw - px, tw + px * 1.5, abs(loc.y))) * onTrail
                  * (1.0 - s) * (1.0 - s) * step(-loc.x, tl) * step(bodyLen * 0.25, -loc.x);
      trail *= 0.42;

      let col = birdCol * (0.85 + 0.3 * h3);
      // a faint glow around the near birds
      let glowR = max(mark, 0.0) / (bodyLen * 0.9);
      let glow = exp(-glowR * glowR * 2.0) * 0.10 * near;
      *acc = mix(*acc, col, max(alpha, trail));
      *acc += col * glow;
    }
  }
}
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let fc = vec2f(frag_coord.x, res.y - frag_coord.y);
    c0 = SG_C0; c1 = SG_C1; c2 = SG_C2; c3 = SG_C3;
  let uv = fc / u.resolution.xy;
  let aspect = u.resolution.x / u.resolution.y;
  let p = vec2f((uv.x - 0.5) * aspect, uv.y - 0.5) * 2.0;
  let px = 2.0 / u.resolution.y;
  let t = u.time * clamp(param(0u), 0.0, 3.0) + 24.0;

  // ---- dusk sky: dark zenith tinted by the deep pole, a haze band low
  // down lit by the pale pole, slow cloud in between
  let zenith = mix(vec3f(0.02, 0.025, 0.04), c3, 0.18);
  let haze = mix(vec3f(0.16, 0.15, 0.17), c2, 0.28);
  let dusk = mix(vec3f(0.10, 0.08, 0.10), c1, 0.30);
  let hz = smoothstep(0.9, -0.85, uv.y * 2.0 - 1.0);
  var sky = mix(zenith, dusk, hz * hz * 0.9);
  sky = mix(sky, haze, smoothstep(-0.3, -1.0, p.y) * 0.7);
  // long dusk cloud bands, stretched along the horizon, drifting slowly
  let cloud = fbm(vec2f(p.x * 0.45 - u.time * 0.012, p.y * 2.6) + 3.0);
  let bands = smoothstep(0.35, 0.75, cloud);
  sky = mix(sky, sky * 0.6 + dusk * 0.25, bands * 0.55 * (0.4 + 0.6 * hz));
  sky += (cloud - 0.5) * 0.05 * mix(c1, c2, 0.5);
  // a low sun glow off to one side, mostly below the frame
  let sunP = vec2f(-0.9 * aspect * 0.6, -1.25);
  let sd = length((p - sunP) * vec2f(0.6, 1.0));
  sky += mix(c2, c1, 0.4) * exp(-sd * sd * 0.9) * 0.28;

  var acc = sky;
  // far to near: cell in frame units (height 2), parallax, seed, nearness
  flock(p, 0.075, 0.55, 1.0, 0.0, t, px, &acc);
  flock(p, 0.125, 0.80, 2.0, 0.5, t, px, &acc);
  flock(p, 0.230, 1.10, 3.0, 1.0, t, px, &acc);

  return vec4f(acc, 1.0);
}
