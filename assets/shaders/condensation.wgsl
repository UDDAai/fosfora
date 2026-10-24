// Condensation — shader.gallery (condensation)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: dusk

const SG_C0: vec3f = vec3f(0.420000, 0.475000, 0.671000);
const SG_C1: vec3f = vec3f(0.690000, 0.522000, 0.580000);
const SG_C2: vec3f = vec3f(0.871000, 0.667000, 0.498000);
const SG_C3: vec3f = vec3f(0.290000, 0.318000, 0.475000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// condensation (Veil) - frost growing across a cold pane at night. Behind the
// glass sits a dark street of out-of-focus lights, soft bokeh discs drifting
// almost imperceptibly. From every edge of the pane frost advances inward as
// fern dendrites built from explicit geometry: stems at jittered spacing point
// in from the edge, each with its own tilt and length, and barbs leave every
// stem forward at sixty degrees (the branching angle of hexagonal ice) on a
// regular pitch, longest at the base and shortest at the tip, so each stem is
// a leaf. Two scales interleave, ferns from neighbouring stems overlap into a
// solid margin, and a granular band closes the ice at the very edge. Between
// the barbs a faint crystalline haze fills the leaf, and the clear glass shows
// through wherever no crystal has reached. Where ice sits in front of a light
// the light blurs and scatters into a milky glow (the bokeh discs are analytic,
// so their softness simply grows under the ice), facets glint on and off as
// they turn to the light, and the stems creep inward and thaw back on a long
// cycle. Every so often a breath fogs a soft round patch of the clear glass
// with a grey droplet haze that clears again.


// tweakable params (see meta.json; the runtime feeds defaults)

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
  let a = hash21(i);
  let b = hash21(i + vec2f(1.0, 0.0));
  var c = hash21(i + vec2f(0.0, 1.0));
  var d = hash21(i + vec2f(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// polynomial smooth minimum: rounds the corners of the edge-distance field
fn smin(a: f32, b: f32, k: f32) -> f32 {

  let h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
  return mix(b, a, h) - k * h * (1.0 - h);
}

fn rot(p: vec2f, a: f32) -> vec2f {

  var c = cos(a);
  let s = sin(a);
  return vec2f(c * p.x - s * p.y, s * p.x + c * p.y);
}

// one family of fern dendrites growing in from one edge. x runs along the
// edge, v is the distance in from it, W the stem pitch, growth the length
// budget. Returns the crisp crystal mask and the soft haze inside each leaf.
fn ferns(x_in: f32, v: f32, W: f32, growth: f32, seed: f32, px: f32) -> vec2f {
  var x = x_in;

  var crisp = 0.0;
  var haze = 0.0;
  let kc = floor(x / W);
  for (var i: i32 = -1; i <= 1; i += 1) {
    var k = kc + f32(i);
    var h0 = hash21(vec2f(k * 1.3, seed));
    var h1 = hash21(vec2f(k * 2.1, seed + 3.7));
    var h2 = hash21(vec2f(k * 0.7, seed + 9.1));
    let sx = (k + 0.5 + (h0 - 0.5) * 0.55) * W;          // stem foot
    let L = growth * (0.55 + 0.9 * h2);                 // stem length
    // local frame: u across the stem, w along it, tilted about the foot
    var l = rot(vec2f(x - sx, v), (h1 - 0.5) * 0.6);
    // stems bow gently to one side as they grow
    l.x -= (h2 - 0.5) * 0.9 * l.y * l.y;
    let sgn = sign(l.x);
    var u = abs(l.x);
    let w = l.y;
    let on = step(0.0, w) * step(w, L);
    let taper = 1.0 - w / max(L, 1e-3);
    // the stem, thick at the foot and needle-thin at the tip
    let ws = W * 0.022 * (0.35 + 0.65 * taper);
    let stem = smoothstep(ws + px, ws - px * 0.5, u) * on;
    // barbs: forward-leaning lines at sixty degrees off the stem on a
    // regular pitch, each shortening toward the tip and a little ragged
    let sp = W * 0.11;
    // barbs curve forward as they run out, and the two sides alternate
    let bw = w - u * 0.577 - u * u * 1.6 + sgn * sp * 0.25 * step(0.5, h0);
    let j = floor(bw / sp + 0.5);
    let wj = j * sp;
    let jh = hash21(vec2f(j * 1.7 + sgn, k + seed));
    let Lb = W * 0.95 * smoothstep(L * 1.02, L * 0.12, wj) * (0.45 + 0.55 * jh) * step(0.12, jh);
    let along = u * 1.155;
    let bd = abs(bw - wj) * 0.866;
    let bwid = ws * 0.55 * (1.0 - along / max(Lb, 1e-3)) + px * 0.35;
    let barb = smoothstep(bwid + px, bwid - px * 0.5, bd) * step(along, Lb) * step(0.0, wj) * step(wj, L);
    // barbules: a finer pitch of short spurs off the barbs
    let sp2 = sp * 0.33;
    let bw2 = w + u * 0.577;
    let j2 = floor(bw2 / sp2 + 0.5);
    let bd2 = abs(bw2 - j2 * sp2) * 0.866;
    let spur = smoothstep(px * 1.2, px * 0.2, bd2) * step(along, Lb * 0.8) * on * 0.40;
    crisp = max(crisp, max(stem, max(barb, spur)));
    // the leaf silhouette: a soft haze of crystals too fine to draw
    let Lw = W * 0.95 * smoothstep(L * 1.02, L * 0.12, w);
    let env = smoothstep(Lw, Lw * 0.3, along) * on;
    haze = max(haze, env);
  }
  return vec2f(crisp, haze);
}

// the blurred light field behind the glass: a dark street of bokeh discs.
// soft is the disc edge softness (0.2 sharp focus, 1 fully scattered)
fn lights(p: vec2f, aspect: f32, t: f32, soft: f32, c0: vec3f, c1: vec3f, c2: vec3f, c3: vec3f) -> vec3f {

  var acc = vec3f(0.0);
  for (var i: i32 = 0; i < 12; i += 1) {
    var k = f32(i);
    var h0 = hash21(vec2f(k * 3.1, 7.0));
    var h1 = hash21(vec2f(k * 1.7, 19.0));
    var h2 = hash21(vec2f(k * 5.3, 31.0));
    let h3 = hash21(vec2f(k * 2.9, 43.0));
    var c = vec2f((h0 - 0.5) * (aspect + 0.3), (h1 - 0.5) * 1.15);
    c += 0.02 * vec2f(sin(t * 0.11 + k * 1.3), cos(t * 0.09 + k * 2.1));
    let rad = 0.06 + 0.16 * h2 * h2;
    var d = length(p - c);
    // a bokeh disc: bright rim when in focus, melting to a blob when scattered
    let disc = smoothstep(rad + rad * soft, rad - rad * (0.3 + 0.7 * (1.0 - soft)) * 0.6, d);
    let rim = smoothstep(rad * 0.55, rad * 0.98, d) * disc * (1.0 - soft) * 0.5;
    let glow = exp(-d * d / (rad * rad * (2.2 + 3.5 * soft))) * 0.20 / (1.0 + 1.5 * soft);
    var col = select(select(select(c3, c2, (h3 < 0.75)), c1, (h3 < 0.5)), c0, (h3 < 0.25));
    col = mix(col, vec3f(1.0), 0.22);
    // scattering spreads the energy: dimmer core, wider skirt
    let amp = (0.70 + 0.50 * h2) * mix(1.0, 0.6, soft);
    acc += col * amp * (disc * (0.7 + rim) + glow);
  }
  return acc;
}
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let fc = vec2f(frag_coord.x, res.y - frag_coord.y);
    let c0 = SG_C0;
  let c1 = SG_C1;
  let c2 = SG_C2;
  let c3 = SG_C3;
  let uv = fc / u.resolution.xy;
  let aspect = u.resolution.x / u.resolution.y;
  let p = vec2f((uv.x - 0.5) * aspect, uv.y - 0.5);
  let pr = max(1.0, 1.0);
  let q = fc / pr;                 // css px, for grain and glints
  let px = 1.0 / u.resolution.y;                 // frame units per device px
  let tp = u.time * clamp(param(1u), 0.0, 3.0);
  let reach = clamp(param(0u), 0.0, 1.0);
  let feath = clamp(param(2u), 0.0, 2.0);
  let spark = clamp(param(3u), 0.0, 2.0);
  let breath = clamp(param(4u), 0.0, 1.0);

  // ---- edge-distance field of the pane, rounded at the corners
  let ex = min(uv.x, 1.0 - uv.x) * aspect;
  let ey = min(uv.y, 1.0 - uv.y);
  let e = smin(ex, ey, 0.22);
  // which edge is nearest decides the finger direction: fingers grow inward,
  // so from the top and bottom they run vertically, from the sides across
  let wV = smoothstep(-0.12, 0.12, ex - ey);

  // ---- the growth cycle: stems creep inward, then slowly thaw back
  let cyc = 0.5 + 0.5 * sin(tp * TAU / 56.0 - 0.37);
  let growth = reach * (0.15 + 0.31 * cyc) * (0.6 + 0.4 * feath);
  let unev = vnoise(p * 3.5 + 71.0) * 0.6 + vnoise(p * 9.0 + 83.0) * 0.4;

  // ---- fern dendrites from each edge, two scales, top/bottom and sides
  let sideV = step(0.5, uv.y) * 10.0;
  let sideH = step(0.5, uv.x) * 10.0;
  let fV = ferns(p.x, ey, 0.17, growth,        1.0 + sideV, px);
  let fV2 = ferns(p.x, ey, 0.075, growth * 0.55, 21.0 + sideV, px);
  let fH = ferns(p.y, ex, 0.17, growth,        41.0 + sideH, px);
  let fH2 = ferns(p.y, ex, 0.075, growth * 0.55, 61.0 + sideH, px);
  let fernV = max(fV, fV2);
  let fernH = max(fH, fH2);
  let fern = mix(fernH, fernV, wV);
  let crisp = fern.x;
  let haze = fern.y;

  // ---- the granular margin: solid ice at the very edge, patchy
  let G = e - growth * 0.30 - 0.03 * (unev - 0.5);
  let band = smoothstep(0.006, -0.006, G);
  var thick = clamp(-G / 0.20, 0.0, 1.0);
  thick = sqrt(thick) * (0.55 + 0.6 * unev);
  // fresh crystal at the outer fringe of the growth
  let fresh = smoothstep(growth * 0.30, growth * 1.1, e);

  // total ice coverage and how solid it is
  let frost = clamp(band * (0.35 + 0.5 * thick) + crisp * 0.95 + haze * 0.30, 0.0, 1.0);
  let dend = crisp;

  // ---- breath: a soft round fog patch that blooms and clears
  let bph = fract(tp / 15.0 + 0.05);
  let bAmt = smoothstep(0.0, 0.10, bph) * (1.0 - smoothstep(0.22, 1.0, bph)) * breath;
  let bc = vec2f(0.10, -0.14);
  let bd = length((p - bc) * vec2f(1.0, 1.25));
  let bR = 0.26 + 0.10 * bAmt;
  var mist = smoothstep(bR, bR * 0.35, bd) * (0.8 + 0.4 * vnoise(p * 6.0 + tp * 0.1));
  mist = clamp(mist * bAmt * 0.7, 0.0, 1.0) * (1.0 - band * 0.6);

  // ---- the night behind the glass
  let soft = 0.22 + 0.7 * max(clamp(band * (0.4 + 0.6 * thick) + haze * 0.5 + crisp * 0.4, 0.0, 1.0), mist * 0.9);
  var night = mix(vec3f(0.030, 0.036, 0.050), c3 * 0.14 + c0 * 0.02, 0.6);
  night *= 0.85 + 0.35 * (0.5 - uv.y);                // a little city glow low down
  var L = lights(p, aspect, tp, soft, c0, c1, c2, c3);
  // soft-clip so two lights stacking never blow to flat white
  L = 1.0 - exp(-L * 1.15);
  var glass = night + L;
  // clear glass carries a faint cold reflection gradient
  glass += c0 * 0.02 * (1.0 - uv.y);

  // ---- ice: dim blue-grey where nothing lights it from behind, milky where
  // it scatters a light, brightest on the crystal ribs and the fresh fringe
  let iceTint = mix(vec3f(0.78, 0.83, 0.92), normalize(c0 + c2 + 0.2) * 0.8, 0.22);
  let lum = dot(L, vec3f(0.299, 0.587, 0.114));
  let solid = clamp(band * thick + haze * 0.5, 0.0, 1.0);
  var ice = iceTint * (0.08 + 0.10 * solid)
                + iceTint * dend * (0.16 + 0.16 * spark) * (0.6 + 0.6 * fresh)
                + L * (0.55 + 0.20 * solid) + iceTint * lum * 0.5;
  ice *= 0.85 + 0.3 * unev;
  // fine grain in the granular ice
  let grain = hash21(floor(q * 0.7)) - 0.5;
  ice += grain * 0.07 * band * thick;
  // glints: small facets turning to the light, twinkling on and off
  let gc = floor(q * 0.12);
  let gh = hash21(gc);
  let gh2 = hash21(gc + 7.7);
  let gf = fract(q * 0.12) - 0.5 + (vec2f(gh, gh2) - 0.5) * 0.5;
  let gdot = smoothstep(0.22, 0.03, length(gf));
  let tw = 0.5 + 0.5 * sin(tp * (0.5 + 1.2 * gh2) + gh * TAU);
  let gl = smoothstep(0.86, 1.0, gh * 0.5 + tw * 0.5) * gdot * step(0.72, gh2);
  ice += mix(vec3f(1.0), c2, 0.3) * gl * 1.1 * spark * frost;

  // ---- breath fog: grey warm droplet haze over the clear glass
  var fog = mix(vec3f(0.52, 0.50, 0.50), c1, 0.15) * (0.28 + 0.5 * lum) + L * 0.45;
  fog += (hash21(floor(q * 0.5) + 3.0) - 0.5) * 0.10;

  var col = glass;
  col = mix(col, fog, mist);
  col = mix(col, ice, frost * (0.75 + 0.25 * solid));

  return vec4f(col, 1.0);
}
