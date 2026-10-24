// Hyperspeed — shader.gallery (hyperspeed)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: midnight

const SG_C0: vec3f = vec3f(0.231000, 0.510000, 0.965000);
const SG_C1: vec3f = vec3f(0.659000, 0.333000, 0.969000);
const SG_C2: vec3f = vec3f(0.133000, 0.827000, 0.933000);
const SG_C3: vec3f = vec3f(0.957000, 0.247000, 0.369000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// hyperspeed (Abyss) - the jump to lightspeed, seen from the cockpit. Stars
// pour out of a dim glowing core and stretch into motion-blurred streaks that
// lengthen as they reach the frame edges. The trick is the coordinate system:
// the star field is a lattice in (angle, log radius) space that scrolls
// outward in log radius at constant speed. A trail of fixed length in log
// space is a screen streak whose length is proportional to radius, so stars
// are short sparks near the core and long blades at the rim, exactly the
// perspective of moving down a tunnel of stars. Three lattices at different
// angular density, speed and width give parallax: the near layer is fast,
// fat and bright, the far layer is a dense fine drizzle. Every streak has a
// hot white head that trails off into a palette tint, its brightness tapering
// along the blur. Under the stars the haze itself is smeared: fbm sampled in
// the same (angle, log r) space and scrolled outward, so the ground is a
// tunnel of radial wisps, not a flat vignette. The core is a soft lamp, dim
// but never black, and the whole tunnel rolls slowly about its axis.


// tweakable params (see meta.json; the runtime feeds defaults)

const TAU: f32 = 6.28318530718;

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

  return vec2f(hash21(p), hash21(p + vec2f(41.3, 17.9)));
}
fn vnoise(p: vec2f) -> f32 {

  var i = floor(p);
  let f = fract(p);
  let u = f * f * (3.0 - 2.0 * f);
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

// palette pick by a hash, so each star owns one pole
fn pickCol(k: f32) -> vec3f {

  let x = floor(fract(k) * 4.0);
  var a = c0;
  a = mix(a, c1, step(0.5, x));
  a = mix(a, c2, step(1.5, x));
  a = mix(a, c3, step(2.5, x));
  return a;
}

// one parallax lattice of streaks.
//   ang   angle of this pixel, 0..TAU (already rolled for this layer)
//   s     log radius of this pixel
//   r     radius in frame units (half height = 1)
//   M     angular slots around the ring
//   dS    radial cell height in log units
//   v     scroll speed in cells per second
//   w     streak half width in frame units at r = 1
//   len   trail length in cells
//   gain  layer brightness
fn lattice(ang: f32, s: f32, r: f32, M: f32, dS: f32, v: f32, w: f32, len: f32, gain: f32, seed: f32, t: f32, dens: f32, tint: f32) -> vec3f {

  let cellA = TAU / M;
  let ai = floor(ang / cellA);
  let sc = s / dS - t * v;          // scrolling cell coordinate
  let si = floor(sc);
  var acc = vec3f(0.0);
  // the head of a trail that covers us may sit in our cell or the two ahead
  for (var j: i32 = 0; j < 3; j += 1) {
    let id = vec2f(ai, si + f32(j)) + seed;
    let h = hash22(id);
    let h3 = hash21(id * 1.31 + 7.7);
    let h4 = hash21(id * 0.71 + 3.1);
    let on = step(h4, dens);
    // head position in cell coords, and the trail length behind it
    let sHead = si + f32(j) + h.y;
    // lengths spread wide: most trails modest, a few long blades
    let L = len * (0.35 + 1.4 * h3 * h3);
    let back = sHead - sc;              // how far behind the head we are
    let along = clamp(back / L, 0.0, 1.0);
    // the head is not a hard cap: it rolls off softly a little way ahead,
    // the way a blurred point does, and the tail fades to nothing
    let inTrail = smoothstep(-0.06, 0.02, back) * step(back, L);
    var taper = 1.0 - along;
    taper = taper * sqrt(taper);
    let head = exp(-abs(back) * 22.0 / max(len, 0.2));
    // perpendicular distance in frame units: angular offset times radius
    let aStar = (ai + 0.18 + 0.64 * h.x) * cellA;
    let perp = (ang - aStar) * r;
    let ww = w * (0.45 + 0.9 * r) * (0.7 + 0.6 * h3);
    let q = perp / ww;
    let line = exp(-q * q * 2.0);
    // wider faint blur sheath around the core of the streak
    let sheath = exp(-q * q * 0.25) * 0.18;
    var I = on * inTrail * (taper * 0.7 + head * 1.4) * (line + sheath);
    // stars emerge from the core haze and are brightest near the rim
    I *= smoothstep(0.03, 0.30, r) * (0.55 + 0.45 * min(r, 1.4)) * (0.6 + 0.8 * h3);
    // colour: white-hot head, palette-tinted tail
    let pole = pickCol(h3 * 3.7 + h.x);
    let col = mix(vec3f(1.0), pole, tint * clamp(along * 1.3 + 0.3, 0.0, 1.0));
    acc += col * I * gain;
  }
  return acc;
}
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let fc = vec2f(frag_coord.x, res.y - frag_coord.y);
    c0 = SG_C0; c1 = SG_C1; c2 = SG_C2; c3 = SG_C3;
  let uv = fc / u.resolution.xy;
  let aspect = u.resolution.x / u.resolution.y;
  let speed = clamp(param(0u), 0.0, 4.0);
  let dens = clamp(param(1u), 0.0, 1.0) * 0.9;
  let len = clamp(param(2u), 0.1, 3.0);
  let core = clamp(param(3u), 0.0, 2.0);
  let tint = clamp(param(4u), 0.0, 1.0);
  let t = u.time;

  // frame coords, half height = 1. The vanishing point sits a touch above
  // centre and leans toward the pointer when there is one (zero at rest).
  var p = vec2f((uv.x - 0.5) * aspect, uv.y - 0.5) * 2.0;
  let m = (vec2f(0.0) / u.resolution - 0.5) * vec2f(aspect, 1.0) * 2.0;
  let mAmt = param(5u) * step(0.5, dot(vec2f(0.0), vec2f(0.0)));
  let vp = vec2f(0.0, 0.06) + m * 0.18 * mAmt;
  p -= vp;

  let r = length(p);
  let s = log(max(r, 1e-4));
  let ang = atan2(p.y, p.x);
  // slow roll of the whole tunnel about its axis
  let roll = t * 0.035 * (0.4 + 0.6 * speed);

  // ---- the ground: a deep tinted dark, lifted by radially smeared haze.
  // The haze is fbm in (angle, log r) space, stretched along log r and
  // scrolled outward, so the wisps stream with the stars.
  let ground = c3 * 0.045 + c0 * 0.025;
  let hz = vec2f(ang * 2.2 + roll * 2.0, s * 0.55 - t * 0.28 * speed);
  // the angle wraps at +-pi, so a second sample is taken on an angle that
  // wraps at 0 instead, and the two are cross-faded so each seam is hidden
  // under the other sample
  let ang2 = sg_mod(ang, TAU);
  let hzB = vec2f(ang2 * 2.2 + roll * 2.0, hz.y) + 13.0;
  let hz1 = fbm(hz);
  let hz2 = fbm(hzB);
  let seam = 0.5 - 0.5 * cos(ang);     // 1 at +-pi, 0 at 0
  var haze = mix(hz1, hz2, seam) - 0.28;
  haze = max(haze, 0.0);
  // haze brightens toward the core lamp and fades a little at the rim
  let hazeF = (0.45 + 0.9 * exp(-r * 1.1)) * core;
  let hazeCol = mix(c0, c2, 0.5 + 0.5 * sin(ang + roll * 3.0)) * 0.6 + c1 * 0.2;
  var col = ground + hazeCol * haze * hazeF;

  // ---- the core lamp: dim, warm, never black at the centre
  let lamp = exp(-r * r * 9.0);
  let lamp2 = exp(-r * 3.2);
  let lampCol = mix(c1, c0, 0.5) * 0.7 + vec3f(0.22);
  col += lampCol * (lamp * 0.7 + lamp2 * 0.22) * core;

  // ---- three parallax lattices of streaks, far to near
  let a0 = sg_mod(ang + roll * 0.8 + 0.7, TAU);
  let a1 = sg_mod(ang + roll * 1.0 + 2.1, TAU);
  let a2 = sg_mod(ang + roll * 1.25 + 4.4, TAU);
  col += lattice(a0, s, r, 72.0, 0.36, 0.55 * speed, 0.0018, len * 0.85, 0.55, 11.0, t, dens, tint);
  col += lattice(a1, s, r, 44.0, 0.50, 0.95 * speed, 0.0030, len * 1.20, 0.85, 23.0, t, dens, tint);
  col += lattice(a2, s, r, 26.0, 0.70, 1.55 * speed, 0.0055, len * 1.60, 1.15, 37.0, t, dens * 0.8, tint);

  // ---- rim: the tunnel walls brighten faintly at the far edges where the
  // streaks pile up, a soft blue-shifted wash from the fastest layer
  col += mix(c2, c0, 0.5) * 0.06 * smoothstep(0.6, 1.6, r) * core;

  // gentle tone roll-off so stacked streaks bloom instead of clipping
  col = col / (1.0 + col * 0.35);

  return vec4f(col, 1.0);
}
