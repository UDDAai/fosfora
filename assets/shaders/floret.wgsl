// Floret — shader.gallery (floret)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: viridian

const SG_C0: vec3f = vec3f(0.133000, 0.686000, 0.443000);
const SG_C1: vec3f = vec3f(0.345000, 0.847000, 0.522000);
const SG_C2: vec3f = vec3f(0.722000, 0.953000, 0.667000);
const SG_C3: vec3f = vec3f(0.067000, 0.404000, 0.353000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}


// tweakable params (see meta.json; the runtime feeds defaults)

const N: i32 = 160;
const GA: f32 = 2.399963;   // golden angle in radians (137.5 deg)

fn ramp(c0: vec3f, c1: vec3f, c2: vec3f, c3: vec3f, x_in: f32) -> vec3f {
  var x = x_in;

  x = clamp(x, 0.0, 1.0);
  var a = mix(c3, c0, smoothstep(0.0, 0.34, x));
  a = mix(a, c1, smoothstep(0.34, 0.67, x));
  a = mix(a, c2, smoothstep(0.67, 1.0, x));
  return a;
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
  var p = vec2f((uv.x - 0.5) * aspect, uv.y - 0.5);

  let m = (vec2f(0.0) / u.resolution - 0.5) * vec2f(aspect, 1.0);
  let mAmt = param(4u) * step(0.5, dot(vec2f(0.0), vec2f(0.0)));
  p -= m * mAmt * 0.3;

  let t = u.time;
  let sp = max(param(1u), 0.02);
  let aa = clamp(2.0 / u.resolution.y, 0.001, 0.05);

  var col = c3 * 0.08;     // dark ground
  // place phyllotaxis seeds; keep only the nearest-contributing dot per pixel
  for (var i: i32 = 0; i < N; i += 1){
    let fi = f32(i);
    let ang = fi * GA + t * param(0u);
    let rad = sqrt(fi) * sp;
    let s = rad * vec2f(cos(ang), sin(ang));
    let d = length(p - s);
    let dotR = sp * param(2u) * (0.72 + 0.28 * sqrt(fi / f32(N)));  // grow outward
    let mask = smoothstep(dotR + aa, dotR - aa, d);
    // colour by radius; a brightness pulse rides outward through the seeds
    let idx = fi / f32(N);
    let pulse = 0.78 + 0.45 * sin(rad * 10.0 - t * 3.0) * param(3u);
    let seedCol = ramp(c0, c1, c2, c3, idx) * (1.18 * pulse);
    col = mix(col, seedCol, mask);
  }

  return vec4f(col, 1.0);
}
