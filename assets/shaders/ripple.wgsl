// Ripple — shader.gallery (ripple)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: tide

const SG_C0: vec3f = vec3f(0.024000, 0.714000, 0.690000);
const SG_C1: vec3f = vec3f(0.298000, 0.851000, 0.706000);
const SG_C2: vec3f = vec3f(0.659000, 0.953000, 0.835000);
const SG_C3: vec3f = vec3f(0.075000, 0.435000, 0.518000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}


// tweakable params (see meta.json; the runtime feeds defaults)
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

  let t = u.time;
  let N = max(param(0u), 6.0);
  let cellId = floor(p * N) + 0.5;
  let cellCentre = cellId / N;          // grid cell centre, frame-relative
  let g = fract(p * N) - 0.5;

  // two ripple sources (rule-of-thirds offsets) + optional pointer source
  let k = 6.283185307 / max(param(2u), 0.15);
  let s1 = vec2f(-0.35, 0.22);
  let s2 = vec2f(0.38, -0.28);
  var w = sin(length(cellCentre - s1) * k - t * param(1u))
          + 0.8 * sin(length(cellCentre - s2) * k - t * param(1u) * 1.1);

  let m = (vec2f(0.0) / u.resolution - 0.5) * vec2f(aspect, 1.0);
  let mAmt = param(4u) * step(0.5, dot(vec2f(0.0), vec2f(0.0)));
  w += mAmt * 1.2 * sin(length(cellCentre - m) * k - t * param(1u) * 1.3);

  w *= 0.5;   // ~ -1..1

  // dot size pulses with the wave height at this cell
  let base = 0.30;
  let radius = clamp(base + param(3u) * 0.26 * w, 0.04, 0.49);
  let d = length(g);
  let aa = clamp(1.5 * N / u.resolution.y, 0.004, 0.2);
  let dotm = smoothstep(radius + aa, radius - aa, d);

  // colour the dots by wave height (crest bright, trough deep) through palette
  let h = w * 0.5 + 0.5;
  var dotCol = mix(c3, c0, smoothstep(0.2, 0.6, h));
  dotCol = mix(dotCol, c2 * 1.15, smoothstep(0.6, 1.0, h));
  let bg = c3 * 0.09;
  let col = mix(bg, dotCol, dotm);

  return vec4f(col, 1.0);
}
