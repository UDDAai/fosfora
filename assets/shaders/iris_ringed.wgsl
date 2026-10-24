// Iris Ringed — shader.gallery (iris-ringed)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: tide

const SG_C0: vec3f = vec3f(0.024000, 0.714000, 0.690000);
const SG_C1: vec3f = vec3f(0.298000, 0.851000, 0.706000);
const SG_C2: vec3f = vec3f(0.659000, 0.953000, 0.835000);
const SG_C3: vec3f = vec3f(0.075000, 0.435000, 0.518000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// ringed (Rosette) — a banded iris filling the frame: the radial fibers of the
// base iris are crossed by a set of glowing concentric annular rings, the way a
// real iris is scored by furrows and a collarette. The fibers give the fine
// radial grain; the rings are bright travelling bands that ripple slowly
// outward from the dark pupil, so the field reads as a woven radial-and-annular
// lattice rather than plain spokes. Colour blends cyclically through the palette
// by angle; the rings catch a brighter tint where they cross a fiber. The pupil
// stays a dark protagonist with a luminous rim; the lattice fills past the
// corners with a soft far-field fade.
//
// Uniforms provided by the runtime:
//   u.time        seconds, monotonically increasing
//   u.resolution  drawing-buffer size in device pixels
//   vec2f(0.0)       pointer in device pixels (0,0 when absent)
//   1.0  devicePixelRatio used for the buffer
//   u_palette[4]  four glow colours, themeable (0..1 rgb)


// tweakable params (see meta.json; the runtime feeds defaults)

const BG: vec3f = vec3f(0.035, 0.035, 0.043); // near-black base ~#09090B
const TAU: f32 = 6.28318530718;

fn wheelW(s: f32, c: f32) -> f32 {

  let d = abs(s - c);
  return max(0.0, 1.0 - min(d, 4.0 - d));
}

fn hash11(n: f32) -> f32 {

  return fract(sin(n * 12.9898) * 43758.5453);
}
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let fc = vec2f(frag_coord.x, res.y - frag_coord.y);
    let pr = 1.0;
  let ctr = res * 0.5;
  let t = u.time;

  // guard params against the unfed-uniform = 0.0 case so extremes/headless stay sane
  let fiberCount = max(param(0u), 16.0);
  let ringCount = max(param(1u), 2.0);
  let ripple = param(2u);
  let pupilR = max(param(3u), 20.0) * pr;

  // --- polar coordinates centred on the pupil ---
  let d = fc - ctr;
  let ang = atan2(d.y, d.x);          // -PI..PI
  let rad = length(d);
  let maxR = length(res) * 0.5;       // half-diagonal: outward==1 at the corner

  // radial coordinate from the pupil edge outward, 0..1 across the half-diagonal
  let rFromPupil = rad - pupilR;
  let outward = clamp(rFromPupil / max(maxR - pupilR, 1.0), 0.0, 1.0);

  let spin = t * 0.012;

  // ====================================================================
  // FIBER FIELD (fine radial grain — two nearest strands kept crisp)
  // ====================================================================
  let fa = (ang / TAU + 0.5 + spin) * fiberCount;
  let fi0 = floor(fa);
  let fi1 = fi0 + 1.0;

  var fiberLum = 0.0;
  var colAngle = ang;
  for (var k: i32 = 0; k < 2; k += 1) {
    let fi = select(fi1, fi0, (k == 0));
    let fiw = sg_mod(fi, fiberCount);
    let h = hash11(fiw + 1.0);
    let wig = 0.12 * sin(rFromPupil / (pupilR * 0.9) * 2.0 + h * TAU + t * 0.18);
    let fd = fa - (fi + 0.5 + wig);
    let strand = 1.0 - smoothstep(0.0, 0.5, abs(fd));
    let fb = 0.5 + 0.5 * hash11(fiw + 7.0);
    let lum = strand * fb;
    if (lum > fiberLum) {
      fiberLum = lum;
      colAngle = (fiw / fiberCount - 0.5) * TAU;
    }
  }

  // ====================================================================
  // CONCENTRIC RINGS
  // Bright annular bands spaced across the field, rippling slowly outward.
  // A continuous ring phase keeps the bands crisp without a hard reset; the
  // band profile is a periodic gaussian so each ring is a thin glow.
  // ====================================================================
  let ringPhase = outward * ringCount - t * ripple; // travels outward over time
  let ringFrac = fract(ringPhase);
  let ringD = min(ringFrac, 1.0 - ringFrac);    // distance to nearest ring centre
  var ring = exp(-pow(ringD / 0.085, 2.0));    // thin bright annulus
  // fade rings out toward the very far field so corners settle
  ring *= 1.0 - smoothstep(0.78, 1.0, outward);

  // ====================================================================
  // PALETTE: colour by fiber angle, blending the four hues cyclically
  // ====================================================================
  let hue = (colAngle / TAU + 0.5) + 0.12 * outward + t * 0.01;
  let s = fract(hue) * 4.0;
  let w0 = wheelW(s, 0.0);
  let w1 = wheelW(s, 1.0);
  let w2 = wheelW(s, 2.0);
  let w3 = wheelW(s, 3.0);

  let c0 = SG_C0;
  let c1 = SG_C1;
  let c2 = SG_C2;
  let c3 = SG_C3;
  let irisCol = (c0 * w0 + c1 * w1 + c2 * w2 + c3 * w3) / max(w0 + w1 + w2 + w3, 0.001);

  // ====================================================================
  // RADIAL SHADING & COMPOSE
  // ====================================================================
  let pupilMask = smoothstep(pupilR - 2.0 * pr, pupilR + 6.0 * pr, rad);
  let farFade = 1.0 - smoothstep(0.8, 1.0, outward);
  let radialFall = mix(1.15, 0.4, smoothstep(0.0, 0.7, outward));

  fiberLum *= pupilMask * farFade;

  var col = BG;
  // fibers: the fine radial grain
  col += irisCol * fiberLum * radialFall * 0.7;
  // rings: bright annular bands, brighter where a lit fiber crosses them
  let cross = ring * pupilMask * (0.5 + 0.9 * fiberLum);
  col += irisCol * cross * radialFall * 1.0;
  // a faint diffuse wash so the lattice never reads as black gaps
  col += irisCol * pupilMask * farFade * 0.04 * (0.6 + 0.4 * radialFall);

  // ====================================================================
  // PUPIL: dark negative space with a luminous rim (the protagonist)
  // ====================================================================
  let pupilInner = 1.0 - smoothstep(pupilR - 10.0 * pr, pupilR, rad);
  col *= mix(1.0, 0.05, pupilInner);
  let rim = exp(-pow((rad - pupilR) / (6.0 * pr), 2.0));
  col += irisCol * rim * 0.5;

  // settle the very corners so framing stays composed
  let vign = 1.0 - 0.45 * smoothstep(0.55, 1.15, rad / maxR);
  col *= vign;

  // gentle tone shaping
  col = col / (1.0 + col * 0.34);

  return vec4f(col, 1.0);
}
