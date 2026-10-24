// Quench — shader.gallery (quench)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: ember

const SG_C0: vec3f = vec3f(0.984000, 0.451000, 0.133000);
const SG_C1: vec3f = vec3f(0.957000, 0.247000, 0.369000);
const SG_C2: vec3f = vec3f(0.965000, 0.620000, 0.043000);
const SG_C3: vec3f = vec3f(0.561000, 0.149000, 0.305000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}


// tweakable params (see meta.json; the runtime feeds defaults)

var<private> c0: vec3f;
var<private> c1: vec3f;
var<private> c2: vec3f;
var<private> c3: vec3f;
var<private> gTime: f32;
var<private> gAmp: f32;
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
  let d = hash(i + vec2f(1.0, 1.0));
  return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}
fn fbm(p_in: vec2f) -> f32 {
  var p = p_in;

  var s = 0.0;
  var a = 0.55;
  let rot = mat2x2<f32>(vec2f(0.80, 0.60), vec2f(-0.60, 0.80));
  for (var i: i32 = 0; i < 5; i += 1){ s += a * vnoise(p); p = rot * p * 2.02 + vec2f(11.3, 7.1); a *= 0.5; }
  return s;
}

// molten surface height at world xz
fn waves(xz: vec2f) -> f32 {

  let p = xz * 0.55;
  var h = 0.55 * fbm(p + vec2f(gTime * 0.35, gTime * 0.12));
  h += 0.28 * fbm(p * 2.1 - vec2f(gTime * 0.25, 0.0));
  return (h - 0.5) * gAmp;
}

fn env(rd: vec3f) -> vec3f {

  // hot forge sky: deep at the horizon, bright above
  let y = clamp(rd.y * 0.5 + 0.5, 0.0, 1.0);
  var col = mix(c3 * 0.4, c0 * 1.1, smoothstep(0.0, 0.55, y));
  col = mix(col, c2 * 1.4, smoothstep(0.5, 1.0, y));
  col += c1 * 0.7 * pow(smoothstep(0.55, 1.0, y), 3.0);
  return col;
}
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let fc = vec2f(frag_coord.x, res.y - frag_coord.y);
    c0 = SG_C0; c1 = SG_C1; c2 = SG_C2; c3 = SG_C3;
  let uv = (fc - 0.5 * u.resolution.xy) / u.resolution.y;
  gTime = u.time * param(0u);
  gAmp = 0.25 + 0.7 * clamp(param(1u), 0.0, 1.0);

  // camera looking low across the molten sea so it recedes to a horizon
  var ro = vec3f(0.0, 1.05, 3.0);
  let mo = (vec2f(0.0) / u.resolution - 0.5);
  let mAmt = param(4u) * step(0.5, dot(vec2f(0.0), vec2f(0.0)));
  let _swA = mo * mAmt * 1.2; ro = vec3f(ro.x + _swA.x, ro.y, ro.z + _swA.y);
  let ta = vec3f(0.0, 0.0, -3.0);
  let ww = normalize(ta - ro);
  let uu = normalize(cross(ww, vec3f(0.0, 1.0, 0.0)));
  let vv = cross(uu, ww);
  let rd = normalize(uv.x * uu + uv.y * vv + 1.5 * ww);

  // heightfield march: walk forward, find where the ray drops below the
  // surface, refine by linear interpolation between the last two samples
  var t = 0.0;
  var lastDiff = ro.y - waves(ro.xz);
  var lastT = 0.0;
  var hitT = -1.0;
  for (var i: i32 = 0; i < 110; i += 1){
    var p = ro + rd * t;
    let diff = p.y - waves(p.xz);
    if (diff < 0.0){
      hitT = mix(lastT, t, lastDiff / max(lastDiff - diff, 1e-4));
      break;
    }
    lastDiff = diff; lastT = t;
    t += 0.05 + t * 0.025;
    if (t > 32.0) { break; }
  }
var col: vec3f;
  if (hitT > 0.0){
    var p = ro + rd * hitT;
    // normal from the height gradient (finite differences)
    let e = 0.04;
    let hL = waves(p.xz - vec2f(e, 0.0));
    let hR = waves(p.xz + vec2f(e, 0.0));
    let hD = waves(p.xz - vec2f(0.0, e));
    let hU = waves(p.xz + vec2f(0.0, e));
    let n = normalize(vec3f(hL - hR, 2.0 * e, hD - hU));

    let refl = reflect(rd, n);
    let fres = pow(1.0 - max(dot(-rd, n), 0.0), 5.0);
    let reflCol = env(refl);

    // molten metal: high reflectance + a hot emissive glow that pools in the
    // troughs (low height) like lava between cooling crests
    let height = waves(p.xz) / max(gAmp, 0.1);     // ~ -0.5..0.5
    let trough = smoothstep(0.15, -0.35, height);  // 1 in the low valleys
    let emissive = (c0 * 1.2 + c1 * 0.5) * trough * param(2u);

    let tint = mix(vec3f(0.92), c0 * 1.3, 0.3);
    col = reflCol * tint;
    col += reflCol * fres * 0.7;
    col += emissive;

    // sharp sun glint off the crests
    let L = normalize(vec3f(0.3, 0.8, -0.5));
    let spec = pow(max(dot(refl, L), 0.0), mix(30.0, 260.0, param(3u)));
    col += vec3f(1.0) * spec * (0.5 + 0.5 * param(3u));

    // distance haze toward the horizon so it fills smoothly
    col = mix(col, env(vec3f(0.0, 0.02, -1.0)), smoothstep(12.0, 30.0, hitT));
  } else {
    col = env(rd);   // hot sky above the horizon fills the top of the frame
  }

  return vec4f(col, 1.0);
}
