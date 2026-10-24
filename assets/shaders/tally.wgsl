// Tally — shader.gallery (tally)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: sodium

const SG_C0: vec3f = vec3f(0.965000, 0.620000, 0.149000);
const SG_C1: vec3f = vec3f(0.984000, 0.749000, 0.286000);
const SG_C2: vec3f = vec3f(0.996000, 0.871000, 0.553000);
const SG_C3: vec3f = vec3f(0.706000, 0.380000, 0.078000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}


// tweakable params (see meta.json; the runtime feeds defaults)

fn hash(n: f32) -> f32 {
 return fract(sin(n * 43758.5453) * 1.0 + n * 0.0001); }

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

  let t = u.time * param(1u);
  // LED grid: square cells; columns set by param(0u), rows follow aspect so dots
  // stay round and the board fills the frame
  let NX = max(param(0u), 6.0);
  let NY = floor(NX / aspect + 0.5);
  let cellId = floor(uv * vec2f(NX, NY));
  var g = fract(uv * vec2f(NX, NY)) - 0.5;
  g.x *= aspect / (select(1.0, 1.0, NY / NX < 1.0));     // keep round (square cells)

  let colX = cellId.x;
  let rowFrac = (cellId.y + 0.5) / NY;           // 0 bottom .. 1 top

  // per-column bouncing bar height (layered sines keyed by a column hash)
  let seed = hash(colX + 1.0);
  var h = 0.5
          + 0.32 * sin(t * (1.3 + seed) + seed * 30.0)
          + 0.18 * sin(t * (2.1 + seed * 1.7) + seed * 12.0);
  // pointer pushes the nearest column to full
  let m = vec2f(0.0) / u.resolution;
  let mAmt = param(4u) * step(0.5, dot(vec2f(0.0), vec2f(0.0)));
  let mCol = floor(m.x * NX);
  h = mix(h, 1.05, mAmt * step(abs(colX - mCol), 0.5));
  h = clamp(h, 0.05, 1.0);

  let lit = step(rowFrac, h);                    // dots below the bar top are on
  let peak = step(abs(rowFrac - h), 0.6 / NY);   // a peak-hold cap dot

  // round LED dot
  let d = length(g);
  let radius = param(2u) * 0.5;
  let aa = clamp(1.5 * NX / u.resolution.x, 0.004, 0.2);
  let dotm = smoothstep(radius + aa, radius - aa, d);

  // VU colour by height: cool/low -> warm/high through the palette
  let onCol = ramp(c0, c1, c2, c3, rowFrac * 0.9 + 0.05);
  let ledOff = c3 * param(3u);
  let led = mix(ledOff, onCol, max(lit, peak));
  let bg = c3 * 0.05;
  let col = mix(bg, led, dotm);

  return vec4f(col, 1.0);
}
