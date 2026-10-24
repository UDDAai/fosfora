// Dendrite — shader.gallery (dendrite)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: nocturne

const SG_C0: vec3f = vec3f(0.176000, 0.318000, 0.671000);
const SG_C1: vec3f = vec3f(0.255000, 0.518000, 0.835000);
const SG_C2: vec3f = vec3f(0.451000, 0.682000, 0.918000);
const SG_C3: vec3f = vec3f(0.110000, 0.176000, 0.392000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// dendrite (Strand) - crystal growing across a cold dark pane. Three seeds,
// each on its own slow cycle, throw out a star of straight trunks; every
// trunk grows side branches at a fixed angle and spacing, and every side
// branch grows its own smaller branchlets by the same rule. Nothing is a
// cell: the whole structure is open segments, and a pixel measures its
// distance to a handful of them. Which handful is found by projection: the
// two trunks that bracket the pixel in angle, the three side branches whose
// roots sit just up-trunk of where a branch at that angle would have to
// start to pass under the pixel, and the two branchlets found the same way
// one level down. Growth is a law, not a picture: a branch begins only when
// its parent tip has passed its root and then lengthens at a fixed rate, so
// the fern envelope (long branches near the seed, short ones near the tip)
// simply falls out of time. The advancing tips are the growth front and glow;
// the settled crystal behind them cools to a frost white with a faint haze of
// fine ice around dense growth. Behind the glass a blurred cold light field
// fills the frame. Each seed melts back and re-forms elsewhere.


// tweakable params (see meta.json; the runtime feeds defaults)

const PI: f32 = 3.14159265359;
const TAU: f32 = 6.28318530718;

fn hash21(p_in: vec2f) -> f32 {
  var p = p_in;

  p = fract(p * vec2f(234.34, 435.345));
  p += dot(p, p + 34.23);
  return fract(p.x * p.y);
}

fn vnoise(p: vec2f) -> f32 {

  var i = floor(p);
  let f = fract(p);
  var u = f * f * (3.0 - 2.0 * f);
  var a = hash21(i);
  let b = hash21(i + vec2f(1.0, 0.0));
  let c = hash21(i + vec2f(0.0, 1.0));
  var d = hash21(i + vec2f(1.0, 1.0));
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

// ---- accumulators
var<private> gCore: f32 = 0.0;   // settled crystal, screen-blended
var<private> gHaze: f32 = 0.0;   // fine ice haze around growth
var<private> gTip: f32 = 0.0;   // growth-front glow
var<private> gPr: f32 = 2.0;   // device px per css px
var<private> gPx: f32 = 400.0; // device px per frame unit
var<private> gT: f32 = 0.0;
var<private> gFade: f32 = 1.0;   // melt fade of the seed being drawn

// growth rates in frame units per second, one per level
const V0: f32 = 0.085;
const V1: f32 = 0.050;
const V2: f32 = 0.030;

// one crystal segment from A along unit dir, current length L of a possible
// Lmax; wpx is the root width in css px, id seeds the glitter
fn seg(p: vec2f, A: vec2f, dir: vec2f, L: f32, Lmax: f32, wpx: f32, id: f32, tipK: f32) {

  if (L <= 0.0005) { return; }
  let pa = p - A;
  let h = clamp(dot(pa, dir), 0.0, L);
  let v = pa - dir * h;
  var d = length(v) * gPx;                 // device px off the segment
  var u = h / max(L, 1e-4);
  let w = max(wpx * gPr * (1.0 - 0.55 * u), 0.5);
  var core = exp(-d * d / (2.0 * w * w)) * min(1.0, wpx * gPr / 0.7);
  // ice is faceted: brightness glitters along the length, slowly shifting
  let gl = vnoise(vec2f(h * gPx / (7.0 * gPr) + id * 3.0, id + gT * 0.15));
  core *= 0.55 + 0.75 * smoothstep(0.25, 0.95, gl);
  // kept tight so it dies well before the candidate-branch handover distance
  let haze = exp(-d / (4.5 * gPr)) * min(1.0, wpx);
  // the advancing tip glows while the segment is still growing
  let dt = length(pa - dir * L) * gPx;
  let growing = 1.0 - smoothstep(0.80, 1.0, L / max(Lmax, 1e-4));
  let tr = 2.2 * gPr * (0.5 + 0.5 * wpx);
  let tip = exp(-dt * dt / (2.0 * tr * tr)) * growing * tipK;
  gCore = 1.0 - (1.0 - gCore) * (1.0 - core * gFade);
  gHaze += haze * gFade;
  gTip  = max(gTip, tip * gFade);
}

// one trunk and everything that grows off it. th is the trunk angle, age the
// seconds since the seed formed, sid a per-trunk seed. ang/ca/sa are the
// side-branch angle and its cos/sin; sp1/sp2 the root spacings per level.
fn trunk(p: vec2f, seed: vec2f, th: f32, age: f32, sid: f32, ang: f32, ca: f32, sa: f32, sp1: f32, sp2: f32) {

  let dir = vec2f(cos(th), sin(th));
  let nrm = vec2f(-dir.y, dir.x);
  var d = p - seed;
  let along = dot(d, dir);
  let perp = dot(d, nrm);
  let side =select(1.0, -1.0, perp < 0.0);
  let ap = abs(perp);
  let cot = ca / max(sa, 0.05);

  let L0max = 0.80 + 0.40 * hash21(vec2f(sid, 1.7));
  let L0 = min(V0 * age, L0max);
  seg(p, seed, dir, L0, L0max, 1.25, sid, 1.0);

  // side branches on the pixel side of the trunk. Root spacing sp1, the two
  // sides staggered by a quarter step so they alternate like a fern.
  let nOff = 0.5 + 0.25 * side;
  let n0 = floor((along - ap * cot) / sp1 - nOff);
  for (var j: i32 = 0; j < 3; j += 1) {
    let n = n0 + f32(j) - 1.0;
    if (n < 0.0) { continue; }
    let hn = hash21(vec2f(n + sid * 3.1, sid + 7.3));
    let hn2 = hash21(vec2f(n + sid * 5.7, sid + 2.9));
    let s1 = (n + nOff + (hn - 0.5) * 0.25) * sp1;
    if (s1 > L0) { continue; }                       // trunk has not reached it
    let age1 = age - s1 / V0;
    // a small angular jitter keeps the fern from looking machined
    let a1 = th + side * (ang + (hn2 - 0.5) * 0.14);
    let b1 = vec2f(cos(a1), sin(a1));
    let P1 = seed + dir * s1;
    // branches near the root of a trunk may grow long; toward the tip they
    // get less room, and every branch has its own final size
    let L1max = (0.13 + 0.22 * hn) * (1.0 - 0.55 * s1 / L0max);
    let L1 = clamp(V1 * age1, 0.0, L1max);
    if (L1 <= 0.0) { continue; }
    seg(p, P1, b1, L1, L1max, 0.75, sid + n * 0.37, 0.7);

    // branchlets off this side branch, found by the same projection
    let n1 = vec2f(-b1.y, b1.x);
    let e = p - P1;
    let al2 = dot(e, b1);
    let pe2 = dot(e, n1);
    let sd2 =select(1.0, -1.0, pe2 < 0.0);
    let mOff = 0.5 + 0.25 * sd2;
    let m0 = floor((al2 - abs(pe2) * cot) / sp2 - mOff);
    for (var k: i32 = 0; k < 2; k += 1) {
      let m = m0 + f32(k);
      if (m < 0.0) { continue; }
      let hm = hash21(vec2f(m + n * 1.3, sid * 2.1 + 4.4));
      let s2 = (m + mOff) * sp2;
      if (s2 > L1) { continue; }
      let age2 = age1 - s2 / V1;
      let a2 = a1 + sd2 * ang;
      let b2 = vec2f(cos(a2), sin(a2));
      let L2max = (0.03 + 0.045 * hm) * (1.0 - 0.5 * s2 / max(L1max, 1e-3));
      let L2 = clamp(V2 * age2, 0.0, L2max);
      seg(p, P1 + b1 * s2, b2, L2, L2max, 0.5, sid + n * 0.53 + m * 0.11, 0.25);
    }
  }
}
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let fc = vec2f(frag_coord.x, res.y - frag_coord.y);
    let c0 = SG_C0;
  let c1 = SG_C1;
  let c2 = SG_C2;
  let c3 = SG_C3;
  gPr = max(1.0, 0.5);
  let uv = fc / u.resolution.xy;
  let aspect = u.resolution.x / u.resolution.y;
  let p = (uv - 0.5) * vec2f(aspect, 1.0) * 2.0;   // y in -1..1
  gPx          = u.resolution.y * 0.5;
  let speed = clamp(param(3u), 0.0, 3.0);
  gT           = u.time * speed;
  let M = floor(clamp(param(0u), 3.0, 8.0));
  let sw = TAU / M;
  let ang = radians(clamp(param(1u), 25.0, 85.0));
  let ca = cos(ang);
  let sa = sin(ang);
  let dens = clamp(param(2u), 0.4, 2.5);
  let sp1 = 0.085 / dens;
  let sp2 = 0.032 / dens;
  let glow = clamp(param(4u), 0.0, 2.0);

  // ---- three seeds on staggered form / melt cycles
  const PERIOD: f32 = 44.0;
  for (var i: i32 = 0; i < 3; i += 1) {
    let fi = f32(i);
    let birth = select((select(2.5, -5.0, (i == 1))), -16.0, (i == 0));
    let tt = gT - birth + PERIOD * 4.0;
    let cyc = floor(tt / PERIOD);
    let age = tt - cyc * PERIOD;
    gFade = 1.0 - smoothstep(PERIOD - 5.0, PERIOD - 0.5, age);
    if (gFade < 0.002) { continue; }
    let sd = 2.3 + fi * 4.7 + cyc * 0.61;
    var home = select((select(vec2f(0.05, 0.62), vec2f(0.72, -0.30), (i == 1))), vec2f(-0.85, 0.22), (i == 0));
    home.x *= aspect / 1.6;
    let jit = vec2f(hash21(vec2f(cyc + fi, 3.3)), hash21(vec2f(cyc + fi, 9.1))) - 0.5;
    let seed = home + jit * 0.45;
    let rot = hash21(vec2f(sd, 0.7)) * TAU;
    let dv = p - seed;
    let phi = atan2(dv.y, dv.x);
    let k0 = floor((phi - rot) / sw - 0.5);
    // the two trunks bracketing this pixel in angle
    for (var a: i32 = 0; a < 2; a += 1) {
      let k = k0 + f32(a);
      let km = sg_mod(k, M);
      let th = rot + (k + 0.5) * sw;
      trunk(p, seed, th, age, sd + km * 1.31, ang, ca, sa, sp1, sp2);
    }
  }

  // ---- the pane: cold dark glass with a blurred light field behind it
  var deep = mix(c3, c0, 0.4);
  let lumD = dot(deep, vec3f(0.299, 0.587, 0.114));
  deep = mix(vec3f(lumD), deep, 0.6);
  let lf = fbm(p * 0.55 + vec2f(3.0, 1.0) + gT * 0.012);
  let blob = smoothstep(0.42, 0.85, lf);
  let wash = fbm(p * 1.3 + vec2f(7.0, 2.0) - gT * 0.008);
  // a second, softer light source low in the frame so no corner goes dead
  let lf2 = fbm(p * 0.4 + vec2f(11.0, 6.0) - gT * 0.009);
  let blob2 = smoothstep(0.50, 0.90, lf2);
  var glass = deep * 0.14 + vec3f(0.028);
  glass += mix(c0, c1, smoothstep(-1.0, 1.0, p.y)) * (0.30 * blob);
  glass += mix(c1, c2, 0.5) * (0.16 * blob2);
  glass += mix(c2, c0, 0.5) * (0.08 * wash);
  glass *= 0.85 + 0.25 * uv.y;

  // ---- the crystal
  let frost = mix(c2, vec3f(1.0), 0.62);
  let haze = clamp(gHaze, 0.0, 4.0);
  var col = glass;
  // fine ice haze around dense growth, tinted by the cool pole
  col += mix(c2, c0, 0.35) * haze * 0.07 * (0.6 + 0.4 * glow);
  // settled crystal: frost white, slightly translucent over the light field
  col = mix(col, frost * (0.72 + 0.25 * blob), gCore * 0.82);
  // the growth front
  col += mix(vec3f(1.0), c1, 0.3) * gTip * 1.1 * glow;

  return vec4f(col, 1.0);
}
