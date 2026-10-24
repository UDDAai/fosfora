// Suffuse — shader.gallery (suffuse)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: opal

const SG_C0: vec3f = vec3f(0.580000, 0.776000, 0.949000);
const SG_C1: vec3f = vec3f(0.769000, 0.690000, 0.949000);
const SG_C2: vec3f = vec3f(0.949000, 0.745000, 0.835000);
const SG_C3: vec3f = vec3f(0.682000, 0.918000, 0.871000);

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

  var i = floor(p);
  let f = fract(p);
  let u = f * f * (3.0 - 2.0 * f);
  var a = hash(i);
  let b = hash(i + vec2f(1.0, 0.0));
  let c = hash(i + vec2f(0.0, 1.0));
  var d = hash(i + vec2f(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}
fn fbm(p_in: vec2f) -> f32 {
  var p = p_in;

  var s = 0.0;
  var a = 0.55;
  let rot = mat2x2<f32>(vec2f(0.80, 0.60), vec2f(-0.60, 0.80));
  for (var i: i32 = 0; i < 4; i += 1){
    s += a * vnoise(p);
    p = rot * p * 2.03 + vec2f(11.3, 7.1);
    a *= 0.5;
  }
  return s;
}
fn poleW(pos: vec2f, c: vec2f, r: f32) -> f32 {

  var d = pos - c;
  return exp(-dot(d, d) / max(r * r, 1e-3));
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
  var pos = vec2f((uv.x - 0.5) * aspect, uv.y - 0.5);

  let m = (vec2f(0.0) / u.resolution - 0.5) * vec2f(aspect, 1.0);
  let mAmt = param(4u) * step(0.5, dot(vec2f(0.0), vec2f(0.0)));
  pos += m * mAmt * 0.4;

  let t = u.time * param(0u);

  // very gentle warp — keeps everything dreamy and soft-edged
  let wv = vec2f(fbm(pos * 1.3 + t * 0.3), fbm(pos * 1.3 + vec2f(5.0, 2.0) - t * 0.25)) - 0.5;
  pos += wv * (0.30 * param(3u));

  // four wide pastel poles drifting slowly — large radius so they bleed into
  // one another into a single suffused wash
  let P0 = vec2f(-0.45, -0.25) + vec2f(sin(t * 0.61 + 0.0), cos(t * 0.53 + 1.0)) * 0.30;
  let P1 = vec2f( 0.42, -0.38) + vec2f(sin(t * 0.47 + 2.1), cos(t * 0.66 + 0.4)) * 0.32;
  let P2 = vec2f( 0.38,  0.40) + vec2f(sin(t * 0.58 + 4.0), cos(t * 0.44 + 3.0)) * 0.30;
  let P3 = vec2f(-0.42,  0.42) + vec2f(sin(t * 0.71 + 5.5), cos(t * 0.60 + 2.4)) * 0.34;

  let r = 0.62 * max(param(1u), 0.3);
  let w0 = poleW(pos, P0, r);
  let w1 = poleW(pos, P1, r);
  let w2 = poleW(pos, P2, r);
  let w3 = poleW(pos, P3, r);
  let wsum = w0 + w1 + w2 + w3 + 1e-3;
  var col = (w0 * c0 + w1 * c1 + w2 * c2 + w3 * c3) / wsum;

  // iridescent thin-film shimmer: a slow soap-bubble sheen riding the blend
  let sheen = 0.5 + 0.5 * sin(6.283185307 * (length(pos) * 0.8 + wv.x + wv.y) + t * 1.5);
  col = mix(col, mix(c2, c0, sheen), param(2u) * 0.18 * sheen);

  // airy and high-key
  col = col * 0.93 + 0.07;

  return vec4f(col, 1.0);
}
