// Frost — shader.gallery (frost)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: glacier

const SG_C0: vec3f = vec3f(0.282000, 0.604000, 0.918000);
const SG_C1: vec3f = vec3f(0.180000, 0.831000, 0.890000);
const SG_C2: vec3f = vec3f(0.682000, 0.886000, 0.973000);
const SG_C3: vec3f = vec3f(0.235000, 0.404000, 0.745000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}


// tweakable params (see meta.json; the runtime feeds defaults)

var<private> c0: vec3f;
var<private> c1: vec3f;
var<private> c2: vec3f;
var<private> c3: vec3f;
var<private> gTime: f32;
var<private> gSpread: f32;
var<private> gMerge: f32;
fn hash(p_in: vec2f) -> f32 {
  var p = p_in;

  p = fract(p * vec2f(123.34, 456.21));
  p += dot(p, p + 45.32);
  return fract(p.x * p.y);
}
fn vnoise(p: vec3f) -> f32 {

  // cheap 3d value noise from 2d hashes on two slices
  var i = floor(p);
  var f = fract(p);
  f = f * f * (3.0 - 2.0 * f);
  let n0 = mix(mix(hash(i.xy + i.z * 37.0), hash(i.xy + vec2f(1.0,0.0) + i.z*37.0), f.x),
                 mix(hash(i.xy + vec2f(0.0,1.0) + i.z*37.0), hash(i.xy + vec2f(1.0,1.0) + i.z*37.0), f.x), f.y);
  let n1 = mix(mix(hash(i.xy + (i.z+1.0)*37.0), hash(i.xy + vec2f(1.0,0.0) + (i.z+1.0)*37.0), f.x),
                 mix(hash(i.xy + vec2f(0.0,1.0) + (i.z+1.0)*37.0), hash(i.xy + vec2f(1.0,1.0) + (i.z+1.0)*37.0), f.x), f.y);
  return mix(n0, n1, f.z);
}

fn smin(a: f32, b: f32, k: f32) -> f32 {

  let h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
  return mix(b, a, h) - k * h * (1.0 - h);
}

fn map(p: vec3f) -> f32 {

  var d = 1e5;
  let k = 0.20 + 0.5 * gMerge;
  for (var i: i32 = 0; i < 6; i += 1){
    let fi = f32(i);
    let c = vec3f(
      sin(gTime * (0.6 + fi * 0.05) + fi * 2.1),
      cos(gTime * (0.7 + fi * 0.06) + fi * 1.3),
      sin(gTime * (0.5 + fi * 0.04) + fi * 3.0)
    ) * gSpread;
    d = smin(d, length(p - c) - 0.72, k);
  }
  return d;
}

fn calcNormal(p: vec3f) -> vec3f {

  let e = vec2f(0.0016, 0.0);
  return normalize(vec3f(
    map(p + e.xyy) - map(p - e.xyy),
    map(p + e.yxy) - map(p - e.yxy),
    map(p + e.yyx) - map(p - e.yyx)));
}

// soft, bright environment the glass transmits and reflects
fn env(rd: vec3f) -> vec3f {

  let y = rd.y * 0.5 + 0.5;
  var col = mix(c3 * 0.45, c0 * 1.15, smoothstep(0.0, 0.65, y));
  col = mix(col, c2 * 1.4, smoothstep(0.5, 1.0, y));
  col += c1 * 0.5 * pow(smoothstep(0.6, 1.0, y), 2.0);
  return col;
}
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let fc = vec2f(frag_coord.x, res.y - frag_coord.y);
    c0 = SG_C0; c1 = SG_C1; c2 = SG_C2; c3 = SG_C3;
  let uv = (fc - 0.5 * u.resolution.xy) / u.resolution.y;

  gTime = u.time * param(0u);
  gSpread = 0.5 + param(3u);
  gMerge = param(1u);

  var ro = vec3f(0.0, 0.15, 3.5);
  let mo = (vec2f(0.0) / u.resolution - 0.5);
  let mAmt = param(4u) * step(0.5, dot(vec2f(0.0), vec2f(0.0)));
  let _swA = mo * mAmt * 1.0; ro = vec3f(ro.x + _swA.x, ro.y + _swA.y, ro.z);
  let ww = normalize(vec3f(0.0) - ro);
  let uu = normalize(cross(ww, vec3f(0.0, 1.0, 0.0)));
  let vv = cross(uu, ww);
  let rd = normalize(uv.x * uu + uv.y * vv + 1.5 * ww);

  var tt = 0.0;
  var hit = 0.0;
  for (var i: i32 = 0; i < 78; i += 1){
    var p = ro + rd * tt;
    let d = map(p);
    if (d < 0.0016){ hit = 1.0; break; }
    if (tt > 9.0) { break; }
    tt += d * 0.9;
  }
var col: vec3f;
  if (hit > 0.5){
    var p = ro + rd * tt;
    var n = calcNormal(p);
    // frosted micro-roughness: jitter the normal by a little 3d noise so the
    // surface scatters rather than mirrors
    let jit = vec3f(vnoise(p * 9.0), vnoise(p * 9.0 + 21.0), vnoise(p * 9.0 + 47.0)) - 0.5;
    n = normalize(n + jit * (0.45 * param(2u)));

    let fres = 0.06 + 0.94 * pow(1.0 - max(dot(-rd, n), 0.0), 5.0);

    // refraction: bend the ray through the glass (IOR ~1.45) and read the env
    let rdir = refract(rd, n, 0.69);
    let transm = env(rdir);
    // reflection of the bright environment
    let refl = env(reflect(rd, n));

    // frosted glass: transmitted colour goes milky with roughness, glass tints
    // it toward the cool palette; fresnel blends in the surface reflection
    let glassTint = mix(vec3f(1.0), c2 * 1.1 + 0.1, 0.5);
    let milky = mix(transm * glassTint, c2 * 1.2 + 0.12, 0.30 * param(2u));
    col = mix(milky, refl, fres * 0.7);

    // a soft frosted sheen highlight (broad, not a sharp chrome spec)
    let L = normalize(vec3f(0.4, 0.9, 0.4));
    let sheen = pow(max(dot(reflect(rd, n), L), 0.0), mix(8.0, 40.0, 1.0 - param(2u)));
    col += vec3f(1.0) * sheen * 0.4 * (0.4 + 0.6 * (1.0 - param(2u)));

    // interior depth — let the form turn into shadow so it reads as a solid
    // glass body, not a flat panel
    col *= 0.66 + 0.40 * (n.y * 0.5 + 0.5);
    col *= 0.86 + 0.18 * max(dot(n, normalize(vec3f(0.4, 0.9, 0.4))), 0.0);
  } else {
    col = env(rd);
  }

  return vec4f(col, 1.0);
}
