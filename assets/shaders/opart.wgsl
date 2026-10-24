// Opart — shader.gallery (opart)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: mercury

const SG_C0: vec3f = vec3f(0.612000, 0.682000, 0.769000);
const SG_C1: vec3f = vec3f(0.776000, 0.831000, 0.890000);
const SG_C2: vec3f = vec3f(0.918000, 0.945000, 0.973000);
const SG_C3: vec3f = vec3f(0.408000, 0.471000, 0.561000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// opart (Manifold) - an op-art band illusion in the Bridget Riley and Vasarely
// register. A field of hard black bands on near-white paper is pushed through
// a set of invisible lenses: six domes and dents of different sizes, one huge
// and mostly off the frame. Each lens magnifies the bands at its centre and
// crushes them at its rim, and because the band phase depends on the lensed x
// alone, a straight band bows outward around a dome and pinches into a dent,
// which the eye reads as a solid form pressing through the print. The lenses
// are applied one after another as radial maps, so the sheet never folds
// back on itself where two lenses meet. The bands also carry a shading trick
// borrowed from engraving: their black weight thickens on the side of each
// dome that faces away from an implied upper-left lamp and thins on the lit
// side, so the false relief gets a false light as well. Where a rim crushes
// the bands finer than a pixel they resolve to flat grey, the way the eye
// loses a Riley print at its tightest folds. The domes breathe and drift
// slowly and a long slow wave travels through the field, so the print never
// sits still but never jumps. Palette is a tint only: a whisper of the pale
// pole in the paper and the dark pole in the ink.


// tweakable params (see meta.json; the runtime feeds defaults)

const NLENS: i32 = 6;

fn hash21(p_in: vec2f) -> f32 {
  var p = p_in;

  p = fract(p * vec2f(234.34, 435.345));
  p += dot(p, p + 34.23);
  return fract(p.x * p.y);
}

fn luma(c: vec3f) -> f32 {
 return dot(c, vec3f(0.299, 0.587, 0.114)); }

// lens i: centre (frame units, x already scaled by aspect), radius, strength.
// Strength breathes on its own harmonic of the base clock so the whole set
// loops together; positive is a dome, negative a dent.
fn lens(i: i32, a: f32, t: f32, c: ptr<function, vec2f>, R: ptr<function, f32>, m: ptr<function, f32>) {

  *c = vec2f(0.0); *R = 1.0; *m = 0.0;
  if (i == 0) { *c = vec2f(-0.32 * a,  0.08); *R = 0.74; *m =  0.95 * (0.85 + 0.15 * sin(t)); }
  if (i == 1) { *c = vec2f( 0.50 * a, -0.38); *R = 0.46; *m = -0.60 * (0.85 + 0.15 * sin(t * 0.5 + 2.1)); }
  if (i == 2) { *c = vec2f( 0.56 * a,  0.60); *R = 0.28; *m =  0.90 * (0.85 + 0.15 * sin(t * 2.0 + 1.0)); }
  if (i == 3) { *c = vec2f(-0.74 * a, -0.68); *R = 0.30; *m =  0.85 * (0.85 + 0.15 * sin(t * 1.5 + 4.2)); }
  if (i == 4) { *c = vec2f( 0.02 * a, -1.02); *R = 0.52; *m = -0.55 * (0.85 + 0.15 * sin(t * 1.0 + 3.3)); }
  if (i == 5) { *c = vec2f( 1.00 * a,  1.20); *R = 1.90; *m =  0.70 * (0.85 + 0.15 * sin(t * 0.5 + 5.0)); }
  // slow drift of each centre, small so the composition holds
  *c += 0.03 * vec2f(sin(t * 0.5 + f32(i) * 1.7), cos(t * 0.5 + f32(i) * 2.3));
}

// dome map: bands laid on a hemisphere and seen from above. Arc length on
// the sphere is asin(r), so equal bands spread widest at the pole and crush
// toward the limb without limit; m blends from flat sheet (0) to full sphere
// (1). Returns the new radius for a point at normalised radius r inside 1.
fn domeMap(r_in: f32, m: f32) -> f32 {
  var r = r_in;

  var s = asin(min(r, 1.0)) * 0.63662;   // 2/pi: continuous with the flat sheet at r = 1
  return mix(r, s, m);
}

// swell map: a broad soft lens with no rim, C1 at the edge, for the huge
// off-frame lens that bends the whole field without cutting a silhouette
fn swellMap(r: f32, m: f32) -> f32 {

  var s = max(1.0 - r * r, 0.0);
  return r * (1.0 - m * s * s);
}

// dent map: the sheet sucked toward a point, bands crowd at the centre and
// relax toward the rim. Monotonic for m below about 0.8.
fn dentMap(r: f32, m: f32) -> f32 {

  var s = max(1.0 - r * r, 0.0);
  return r * (1.0 + m * s * s);
}

// push frame point p through every lens in turn. Returns the lensed point
// (its x drives the band phase) and the false-light shade in -1..1, built
// from each lens as a sphere normal against the lamp.
fn warpField(p_in: vec2f, a: f32, t: f32, strength: f32, shade: ptr<function, f32>) -> vec2f {
  var p = p_in;

  let L = normalize(vec2f(-0.7, 0.6));
  *shade = 0.0;
  for (var i: i32 = 0; i < NLENS; i += 1) {
    var c: vec2f;
    var R: f32;
    var m: f32;
    lens(i, a, t, &c, &R, &m);
    let q = p - c;
    let r = length(q) / R;
    let mm = min(abs(m) * strength, 0.98);
    var rn = select(dentMap(r, mm * 0.8), domeMap(r, mm), (m > 0.0));
    if (i == NLENS - 1) { rn = swellMap(r, mm * 0.7); }
    rn = select(r, rn, (r < 1.0));
    p = c + q * (rn / max(r, 1e-4));
    // sphere shading: the lit flank faces the lamp, dents run the other way
    let cov = select(0.0, sqrt(max(1.0 - r * r, 0.0)) * 0.35 + 0.65, (r < 1.0));
    *shade += sign(m) * min(mm * 1.4, 1.0) * cov * dot(q / R, L);
  }
  // optional pointer dome, zero at rest
  let mres = vec2f(0.0) / u.resolution;
  let mp = vec2f((mres.x - 0.5) * a, mres.y - 0.5) * 2.0;
  let mAmt = param(5u) * step(0.5, dot(vec2f(0.0), vec2f(0.0)));
  let mq = p - mp;
  let mr = length(mq) / 0.4;
  let mrn = select(mr, domeMap(mr, 0.6 * mAmt), (mr < 1.0));
  p = mp + mq * (mrn / max(mr, 1e-4));
  *shade += mAmt * (select(0.0, 0.8, (mr < 1.0))) * dot(mq / 0.4, L);
  return p;
}
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let fc = vec2f(frag_coord.x, res.y - frag_coord.y);
    let c0 = SG_C0;
  let c1 = SG_C1;
  let c2 = SG_C2;
  let c3 = SG_C3;
  let aspect = res.x / res.y;
  let uv = fc / res;
  let t = u.time * clamp(param(2u), 0.0, 2.0);
  let dens = max(param(0u), 4.0);
  let str = clamp(param(1u), 0.0, 2.0);
  let tint = clamp(param(4u), 0.0, 1.0);

  // frame coords: y in -1..1, x in -aspect..aspect
  let p0 = vec2f((uv.x - 0.5) * aspect, uv.y - 0.5) * 2.0;
  let px = 2.0 / res.y;                 // one device pixel in frame units

  // lens the sheet at this pixel and one pixel over in x and y (finite
  // differences for the filter footprint, no derivative extension needed)
var shade: f32;
var shX: f32;
var shY: f32;
  let w0 = warpField(p0, aspect, t, str, &shade);
  let wX = warpField(p0 + vec2f(px, 0.0), aspect, t, str, &shX);
  let wY = warpField(p0 + vec2f(0.0, px), aspect, t, str, &shY);

  // tilt the band field on the lensed sheet, then let a long slow wave
  // travel through it
  let ang = radians(param(3u));
  let rotM = mat2x2<f32>(cos(ang), -sin(ang), sin(ang), cos(ang));
  let p = rotM * w0;
  let pX = rotM * wX;
  let pY = rotM * wY;
  let xw = p.x  + 0.03 * sin(p.y  * 1.3 - t * 0.5);
  let xwX = pX.x + 0.03 * sin(pX.y * 1.3 - t * 0.5);
  let xwY = pY.x + 0.03 * sin(pY.y * 1.3 - t * 0.5);

  // band phase in cycles, and how many cycles this pixel spans
  let k = dens / (2.0 * aspect);
  let phi = xw * k;
  let fpx = length(vec2f(xwX - xw, xwY - xw)) * k;

  // false light sets the ink weight: the lit flank thins to fine lines, the
  // far flank thickens toward solid black
  shade = clamp(shade, -1.0, 1.0);
  let duty = 0.5 - 0.32 * shade;        // fraction of the cycle that is ink

  // the band: a triangle wave thresholded at the duty, filtered by pixel
  // footprint so sub-pixel bands resolve to their mean grey instead of moire
  let tri = abs(fract(phi) - 0.5) * 2.0;
  let w = max(fpx * 1.6, 0.015);
  var ink = 1.0 - smoothstep(duty - w, duty + w, tri);
  let fine = smoothstep(0.35, 0.8, fpx);
  ink = mix(ink, duty, fine);

  // ---- paper and ink, tinted by palette luminance mixes
  var pale = c2;
  var lp = luma(c2);
  if (luma(c1) > lp) { pale = c1; lp = luma(c1); }
  if (luma(c0) > lp) { pale = c0; lp = luma(c0); }
  if (luma(c3) > lp) { pale = c3; lp = luma(c3); }
  var dark = c3;
  var ld = luma(c3);
  if (luma(c0) < ld) { dark = c0; ld = luma(c0); }
  if (luma(c1) < ld) { dark = c1; ld = luma(c1); }
  if (luma(c2) < ld) { dark = c2; ld = luma(c2); }
  let paleTint = pale / max(lp, 0.05) * 0.92;
  let darkTint = dark / max(ld, 0.05) * 0.10;
  var paper = mix(vec3f(0.94), clamp(paleTint, vec3f(0.0), vec3f(1.0)), tint * 0.45);
  let inkC = mix(vec3f(0.045), clamp(darkTint, vec3f(0.0), vec3f(1.0)), tint * 0.7);

  // a very faint sheet: tooth speckle at css-pixel scale, and a soft top-lit
  // gradient across the paper so the print reads as a lit object
  let q = fc / max(1.0, 0.5);
  let tooth = (hash21(floor(q * 0.7)) - 0.5) * 0.03;
  let sheet = 1.0 + tooth + 0.03 * (uv.y - 0.5);
  paper *= sheet;

  var col = mix(paper, inkC, ink);

  // the lit flanks also catch a breath of light on the paper itself
  col *= 1.0 + 0.05 * shade;

  return vec4f(col, 1.0);
}
