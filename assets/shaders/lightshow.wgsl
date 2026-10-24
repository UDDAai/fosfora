// Lightshow — shader.gallery (lightshow)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: witchlight

const SG_C0: vec3f = vec3f(0.545000, 0.361000, 0.965000);
const SG_C1: vec3f = vec3f(0.925000, 0.282000, 0.600000);
const SG_C2: vec3f = vec3f(0.298000, 0.733000, 0.910000);
const SG_C3: vec3f = vec3f(0.984000, 0.749000, 0.141000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// lightshow (Bloom) - a 1960s liquid light show: coloured oil and dyed water
// pressed between two glass plates on an overhead projector, thrown onto the
// wall. The oil rides the plate as blooming translucent lenses that drift,
// swell, touch and fuse; where two dyes overlap the light passes through both
// films and goes to a deeper mixed colour; where a lens edge bends the light
// there is a bright meniscus rim and a soft halo of thrown colour.
// Built without any cell gather: four dyes, each a smooth-min union of four
// discs on slow Lissajous orbits, so same-dye blobs merge into amoebas with a
// single soft outline while different dyes simply overlap. Each blob is given
// a rounded dome thickness; the projector light is attenuated in log space
// through the base-dyed water and through every film it crosses (oil displaces
// the dark water, so a lens is brighter than the ground around it), and the
// thickness gradient by finite differences lights the rim like a lens edge.
// The plate itself rotates slowly on the wheel, the dyes convect in fbm
// mottle, a scatter of tiny air bubbles rides the water, and the lamp breathes.


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
fn hash11(n: f32) -> f32 {
 return fract(sin(n * 127.1) * 43758.5453); }
fn vnoise(p: vec2f) -> f32 {

  var i = floor(p);
  let f = fract(p);
  let u = f * f * (3.0 - 2.0 * f);
  var a = hash21(i);
  let b = hash21(i + vec2f(1.0, 0.0));
  var c = hash21(i + vec2f(0.0, 1.0));
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
fn rot(a: f32) -> mat2x2<f32> {
 var c = cos(a);
  var s = sin(a);
  return mat2x2<f32>(c, -s, s, c); }

// polynomial smooth minimum: two discs within k of each other grow a neck
fn smin(a: f32, b: f32, k: f32) -> f32 {

  let h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
  return mix(b, a, h) - k * h * (1.0 - h);
}

// signed distance to the fused union of the four discs of dye group g, at
// plate point p. Disc n of 16 starts in its own stratified cell of a 4x4 grid
// (the index permuted so one dye is spread across the plate, not clumped)
// and rides a slow Lissajous orbit from there, its radius breathing.
fn dyeField(p: vec2f, g: f32, t: f32, aspect: f32, sizeS: f32, k: f32) -> f32 {

  var d = 1e3;
  for (var i: i32 = 0; i < 4; i += 1) {
    let n = g * 4.0 + f32(i);
    let m = sg_mod(n * 5.0 + 3.0, 16.0);          // permuted grid slot
    let cell = vec2f(sg_mod(m, 4.0), floor(m / 4.0));
    let h0 = hash11(n + 1.0);
    let h1 = hash11(n + 9.0);
    let h2 = hash11(n + 17.0);
    let h3 = hash11(n + 25.0);
    let base = ((cell + 0.5 + (vec2f(h0, h1) - 0.5) * 0.6) / 4.0 * 2.0 - 1.0)
               * vec2f(aspect * 1.05, 1.0);
    let amp = vec2f(0.30 + 0.30 * h2, 0.25 + 0.25 * h3);
    let orb = vec2f(sin(t * (0.21 + 0.13 * h0) + h1 * TAU),
                      cos(t * (0.17 + 0.11 * h1) + h2 * TAU)) * amp;
    var r = (0.22 + 0.26 * h3) * sizeS;
    r *= 1.0 + 0.16 * sin(t * (0.3 + 0.2 * h2) + h0 * TAU);
    let di = length(p - base - orb) - r;
    d = smin(d, di, k);
  }
  return d;
}

// lens dome thickness from the signed distance: a circular meniscus profile
// that rises steeply at the outline and flattens across the body
fn dome(d: f32, edge: f32) -> f32 {

  let inside = clamp(-d / edge, 0.0, 1.0);
  return sqrt(inside * (2.0 - inside));
}
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let fc = vec2f(frag_coord.x, res.y - frag_coord.y);
    c0 = SG_C0; c1 = SG_C1; c2 = SG_C2; c3 = SG_C3;
  let uv = fc / u.resolution.xy;
  let aspect = u.resolution.x / u.resolution.y;
  let t = u.time * clamp(param(0u), 0.0, 2.0);
  let sizeS = clamp(param(1u), 0.3, 2.0);
  let k = 0.04 + 0.42 * clamp(param(2u), 0.0, 1.0);
  let dyeK = clamp(param(3u), 0.2, 2.0);
  let lampK = clamp(param(4u), 0.0, 2.0);

  // wall coords, half height = 1
  let q = vec2f((uv.x - 0.5) * aspect, uv.y - 0.5) * 2.0;

  // the pointer, when present, nudges the plate a little (zero at rest)
  let m = (vec2f(0.0) / u.resolution - 0.5) * vec2f(aspect, 1.0) * 2.0;
  let mAmt = param(5u) * step(0.5, dot(vec2f(0.0), vec2f(0.0)));
  let pPlate = q - m * 0.12 * mAmt;

  // the wheel turns: the whole plate rotates slowly about a point off centre
  let pivot = vec2f(0.12 * aspect, -0.08);
  var p = rot(t * 0.045) * (pPlate - pivot) + pivot;

  // convection: a soft warp so the lens outlines are never true circles
  let wq = vec2f(fbm(p * 0.9 + vec2f(t * 0.05, -t * 0.03)),
                 fbm(p * 0.9 + vec2f(4.7, 2.3) - vec2f(t * 0.04, t * 0.06))) - 0.5;
  p += wq * 0.18;

  // ---- the projector: a soft hotspot on the wall, breathing lamp
  let breathe = 1.0 + 0.04 * sin(u.time * 0.7) + 0.02 * sin(u.time * 2.3);
  let hs = q - vec2f(0.15, 0.1);
  let hotspot = exp(-dot(hs, hs) * 0.35);
  let lampCol = vec3f(1.0, 0.975, 0.94);                    // incandescent
  let light = lampCol * (0.55 + 0.75 * hotspot) * breathe * lampK;

  // ---- the base-dyed water: a deep tint built from the first pole with a
  // little of the fourth, kept dark so the oil lenses read as light. Mottle
  // from convection.
  var waterDye = mix(c0, c3, 0.22);
  let lumaW = dot(waterDye, vec3f(0.299, 0.587, 0.114));
  waterDye = mix(vec3f(lumaW), waterDye, 0.88);                // desaturate a touch
  let mottle = fbm(p * 1.6 + vec2f(-t * 0.07, t * 0.05)) - 0.5;
  let waterT = clamp(waterDye * (0.24 + 0.16 * mottle) + 0.02, vec3f(0.01), vec3f(1.0));
  let logT = log(waterT);                                   // water attenuation

  // ---- four dyes: field, dome thickness, gradient, then attenuate in log
  // space. Oil displaces the dark water, so where a lens sits the water term
  // is removed and the film term takes over.
  let e = 0.012;
  var coverAll = 0.0;       // 1 - product(1 - h_g): how much water is displaced
  var filmLog = vec3f(0.0);
  var rim = 0.0;
  var rimCol = vec3f(0.0);
  var halo = vec3f(0.0);
  var inner = 0.0;
  let edgeW = 0.19 * sizeS;

  for (var g: i32 = 0; g < 4; g += 1) {
    let gf = f32(g);
    var dye = c0;
    dye = mix(dye, c1, step(0.5, gf));
    dye = mix(dye, c2, step(1.5, gf));
    dye = mix(dye, c3, step(2.5, gf));
    // gels are a touch more saturated than the swatch
    let lum = dot(dye, vec3f(0.299, 0.587, 0.114));
    let gel = clamp(mix(vec3f(lum), dye, 1.05), vec3f(0.03), vec3f(1.0));

    // each dye has its own slight plate wobble so outlines do not share edges
    let pg = p + 0.05 * vec2f(sin(p.y * 2.3 + gf * 1.7 + t * 0.2),
                              cos(p.x * 2.1 - gf * 2.9 + t * 0.17));
    let d = dyeField(pg, gf, t, aspect, sizeS, k);
    let dx = dyeField(pg + vec2f(e, 0.0), gf, t, aspect, sizeS, k);
    let dy = dyeField(pg + vec2f(0.0, e), gf, t, aspect, sizeS, k);
    let h = dome(d, edgeW);
    let hx = dome(dx, edgeW);
    let hy = dome(dy, edgeW);
    let slope = length(vec2f(hx - h, hy - h)) / e;

    // the oil is not a uniform slab: convection thins and pools it, so the
    // body of every lens carries paler and deeper zones that slowly move
    let thkVar = vnoise(pg * 1.3 + gf * 7.1 + vec2f(t * 0.05, -t * 0.03)) * 0.7
                 + vnoise(pg * 3.1 + gf * 3.3 - vec2f(t * 0.04, t * 0.02)) * 0.3;
    // film attenuation: thicker oil, deeper colour. log(gel) is negative.
    let thick = h * (0.35 + 0.65 * h) * dyeK * (0.5 + 1.1 * thkVar);
    filmLog += log(gel) * thick * 1.1;
    coverAll = 1.0 - (1.0 - coverAll) * (1.0 - h);

    // meniscus: the steep outer skirt of the dome bends light into a bright
    // rim; the shoulder just inside it goes a little dark (the lens shadow)
    let skirt = smoothstep(0.0, 1.0, h) * (1.0 - smoothstep(0.55, 1.0, h));
    let rimG = clamp(slope * 0.17, 0.0, 1.0) * skirt;
    rim   += rimG;
    rimCol += mix(gel, vec3f(1.0), 0.55) * rimG;
    inner += smoothstep(0.45, 0.95, h) * (1.0 - smoothstep(0.85, 1.0, h)) * 0.5;

    // thrown halo outside the outline: soft colour bloom on the wall
    let outD = max(d, 0.0);
    halo += gel * exp(-outD * 4.5) * (1.0 - h) * 0.6;
  }

  // water is only where there is no oil; films stack multiplicatively
  let T = exp(logT * (1.0 - coverAll) + filmLog);

  // ---- tiny air bubbles riding the water: sparse, bright ring, dark centre
  let pr = max(1.0, 0.5);
  let bq = (rot(t * 0.045) * (pPlate - pivot)) * 90.0 + vec2f(t * 1.5, t * 0.6);
  let bi = floor(bq);
  let bf = fract(bq) - 0.5;
  let bh = hash21(bi + 7.0);
  let bc = (vec2f(hash21(bi + 3.0), hash21(bi + 11.0)) - 0.5) * 0.5;
  let br = 0.07 + 0.10 * hash21(bi + 19.0);
  let bd = length(bf - bc) - br;
  let bubble = step(bh, 0.10) * smoothstep(0.03, 0.0, abs(bd)) ;
  let bubbleIn = step(bh, 0.10) * smoothstep(0.0, -0.04, bd);

  // ---- compose: projector light through the plate, plus rim and halo
  var col = T * light;
  col *= 1.0 - 0.22 * clamp(inner, 0.0, 1.0);               // lens shoulder shade
  col += rimCol * light * 0.55 * clamp(rim, 0.0, 1.5);       // meniscus rim
  col += halo * light * 0.26 * (1.0 - coverAll);             // thrown bloom
  col = mix(col, col * 0.7, bubbleIn * 0.6);
  col += vec3f(1.0) * bubble * 0.25 * light;

  // the dyes are translucent so the wall keeps a whisper of texture
  col *= 0.94 + 0.06 * vnoise(fc / pr * 0.9);

  // soft roll-off so stacked rims bloom rather than clip
  col = col / (1.0 + col * 0.25);

  return vec4f(col, 1.0);
}
