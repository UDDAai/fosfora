// Ingot — shader.gallery (ingot)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: foundry

const SG_C0: vec3f = vec3f(0.976000, 0.514000, 0.149000);
const SG_C1: vec3f = vec3f(0.965000, 0.690000, 0.180000);
const SG_C2: vec3f = vec3f(0.996000, 0.835000, 0.420000);
const SG_C3: vec3f = vec3f(0.620000, 0.235000, 0.110000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}


// tweakable params (see meta.json; the runtime feeds defaults)

fn hash(p_in: vec2f) -> f32 {
  var p = p_in;

  p = fract(p * vec2f(123.34, 456.21));
  p += dot(p, p + 45.32);
  return fract(p.x * p.y);
}
fn vnoise(p: vec2f) -> f32 {

  let i = floor(p);
  let f = fract(p);
  let u = f * f * (3.0 - 2.0 * f);
  let a = hash(i);
  let b = hash(i + vec2f(1.0, 0.0));
  let c = hash(i + vec2f(0.0, 1.0));
  let d = hash(i + vec2f(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
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
  let q = vec2f((uv.x - 0.5) * aspect, uv.y - 0.5);

  let t = u.time;

  // optional pointer tilt — shifts where the sheet "faces" the light
  let mo = (vec2f(0.0) / u.resolution - 0.5) * vec2f(aspect, 1.0);
  let mAmt = param(4u) * step(0.5, dot(vec2f(0.0), vec2f(0.0)));
  let tilt = mo * mAmt * 0.5;

  // brushed grooves run horizontally: brightness varies fast across y (between
  // grooves) and slowly along x — the anisotropic signature of brushed metal
  let bf = mix(60.0, 340.0, param(1u));
  let groove = vnoise(vec2f(q.x * 5.0, q.y * bf)) * 0.6
               + vnoise(vec2f(q.x * 11.0, q.y * bf * 2.3)) * 0.4;
  // micro-glint along the grooves
  let glint = vnoise(vec2f(q.x * 220.0, q.y * bf)) ;

  // surface "normal" tilt from the grooves (mostly in the cross-brush axis)
  let ny = (groove - 0.5);

  // a couple of broad highlight bands sweeping vertically across the sheet —
  // these are the studio lights the metal catches. Anisotropy stretches them
  // wide in x and narrow in y.
  let sweep = 0.30 * sin(t * param(0u)) + tilt.y;
  let yc = q.y + ny * 0.12 - sweep;
  let bandW = mix(0.05, 0.26, param(3u));
  let band1 = exp(-(yc * yc) / (bandW * bandW));
  let yc2 = q.y + ny * 0.12 - (sweep * 0.6 - 0.28);
  let band2 = exp(-(yc2 * yc2) / (bandW * bandW * 0.5)) * 0.6;

  // base metal: vertical gradient between palette tones, brush-modulated so the
  // grooves read as fine reflectance variation
  let vgrad = smoothstep(-0.55, 0.55, q.y + tilt.y);
  var metal = mix(c3 * 0.55, c0 * 1.0, vgrad);
  metal = mix(metal, c1 * 1.05, vgrad * vgrad * 0.5);
  metal *= 0.78 + 0.34 * groove;          // brushed reflectance streaks

  // highlights — bright, palette-warm, sharpened by the brush glint
  let hl = (band1 + band2) * (0.6 + 0.8 * glint);
  var col = metal + (c2 * 1.2 + 0.2) * hl * param(2u);
  // a hot core in the brightest band reads as polished metal catching the light
  col += vec3f(1.0) * pow(band1, 3.0) * glint * param(2u) * 0.6;

  return vec4f(col, 1.0);
}
