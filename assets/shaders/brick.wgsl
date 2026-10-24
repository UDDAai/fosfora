// Brick — shader.gallery (brick)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: rust

const SG_C0: vec3f = vec3f(0.831000, 0.380000, 0.180000);
const SG_C1: vec3f = vec3f(0.682000, 0.247000, 0.180000);
const SG_C2: vec3f = vec3f(0.949000, 0.620000, 0.353000);
const SG_C3: vec3f = vec3f(0.404000, 0.149000, 0.149000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// brick (Burnish) - a running-bond brick wall in raking light. The coursing
// is a plain offset-row lattice at css-pixel scale, every brick found by a
// per-cell hash: its own fired tone (dark headers, pale stretchers, a few
// flashed with the second pole), a slightly different size and lean, and
// its own mottle. The wall is a height field: brick faces sit proud with a
// rounded arris, the mortar bed lies recessed and gritty, and the edges are
// eaten by noise so corners chip and arrises crumble. Normals come from
// finite differences of that height, so a low sun sweeping slowly across
// the wall throws real relief: lit arrises on the sun side, a soft shadow
// dropped into the joint on the far side, and mortar that darkens where a
// brick face stands between it and the light. The camera drifts a little
// so the courses slide, and the sun angle turns over the minute.


// tweakable params (see meta.json; the runtime feeds defaults)

fn hash21(p_in: vec2f) -> f32 {
  var p = p_in;

  p = fract(p * vec2f(234.34, 435.345));
  p += dot(p, p + 34.23);
  return fract(p.x * p.y);
}

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

fn fbm3(p_in: vec2f) -> f32 {
  var p = p_in;

  var a = 0.5;
  var s = 0.0;
  for (var i: i32 = 0; i < 3; i += 1) {
    s += a * vnoise(p);
    p = M2 * p * 2.03 + vec2f(11.7, 5.3);
    a *= 0.5;
  }
  return s * 1.14;
}

fn luma(c: vec3f) -> f32 {
 return dot(c, vec3f(0.299, 0.587, 0.114)); }

// ---- the coursing. q in css px. Returns the brick cell id in .xy and the
// signed distance to the brick face edge (negative inside) in .z, with the
// per-brick size jitter and lean already applied.
fn course(q: vec2f, bh: f32, bw: f32, joint: f32) -> vec3f {

  let row = floor(q.y / bh);
  let off = sg_mod(row, 2.0) * 0.5 * bw;               // running bond
  let colf = (q.x + off) / bw;
  let col = floor(colf);
  var id = vec2f(col, row);
  var l = vec2f((colf - col) * bw - bw * 0.5, (q.y / bh - row) * bh - bh * 0.5);
  // per-brick jitter: size, lean, and a slight shift in the bed
  let h1 = hash21(id + 0.37);
  let h2 = hash21(id + 5.11);
  let h3 = hash21(id + 9.73);
  var hs = vec2f(bw, bh) * 0.5 - joint * 0.5;
  hs -= vec2f(h1, h2) * joint * 0.35;
  l += vec2f(h3 - 0.5, h1 - 0.5) * joint * 0.4;
  let lean = (h2 - 0.5) * 0.03;
  l = vec2f(l.x - l.y * lean, l.y + l.x * lean * 0.3);
  let dd = abs(l) - hs;
  var d = length(max(dd, vec2f(0.0))) + min(max(dd.x, dd.y), 0.0);
  return vec3f(id, d);
}

// ---- the height field, css px in and relief height out (0 = mortar bed).
// Brick faces stand proud with a rounded arris; wear eats the edge outward
// from the joint, more at the corners; both surfaces carry their own grain.
fn heightAt(q: vec2f, bh: f32, bw: f32, joint: f32, wear: f32, t: f32) -> f32 {

  var c = course(q, bh, bw, joint);
  var id = c.xy;
  var d = c.z;
  // chipped arrises: the edge wanders on a noise seeded per brick, and
  // larger bites come off where a second noise peaks near the edge
  let n1 = vnoise(q * 0.35 + id * 7.0) - 0.5;
  let n2 = vnoise(q * 0.09 + id * 3.0 + 31.0);
  let bite = smoothstep(0.72, 0.94, n2) * wear * joint * 1.4;
  d += n1 * wear * joint * 0.30 + bite;
  // face height with a rounded arris about half a joint wide
  let arris = joint * 0.55;
  var face = smoothstep(0.0, arris, -d);
  face = face * face * (3.0 - 2.0 * face);
  // brick face grain: sand mottle plus a few pits
  let mottle = fbm3(q * 0.16 + id * 1.7) - 0.5;
  let pit = smoothstep(0.70, 0.84, fbm3(q * 0.22 + id * 2.3 + 53.0));
  let brickH = 1.0 + mottle * 0.10 - pit * 0.16;
  // mortar bed: coarse gritty sand, slumped a little away from the brick
  let grit = (vnoise(q * 0.9 + 17.0) - 0.5) * 0.10 + (hash21(floor(q * 1.3)) - 0.5) * 0.05;
  let mortarH = 0.0 + grit;
  return mix(mortarH, brickH, face);
}
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let fc = vec2f(frag_coord.x, res.y - frag_coord.y);
    let c0 = SG_C0;
  let c1 = SG_C1;
  let c2 = SG_C2;
  let c3 = SG_C3;
  let pr = max(1.0, 0.5);
  let bh = clamp(param(0u), 10.0, 80.0);          // course height, css px
  let bw = bh * 2.15;                            // stretcher length
  let joint = max(bh * 0.14, 1.5);                  // mortar joint width
  let depth = clamp(param(1u), 0.0, 3.0);
  let wear = clamp(param(2u), 0.0, 2.0);
  let vari = clamp(param(3u), 0.0, 1.5);
  let t = u.time * clamp(param(4u), 0.0, 1.0);

  // css-px wall coordinates with a slow bounded camera drift
  var q = fc / pr;
  q += vec2f(sin(t * 0.55), 0.35 * cos(t * 0.41)) * bh * 2.0;

  // ---- relief: height and finite-difference normal
  let eps = 1.0;
  let h = heightAt(q, bh, bw, joint, wear, t);
  let hx = heightAt(q + vec2f(eps, 0.0), bh, bw, joint, wear, t);
  let hy = heightAt(q + vec2f(0.0, eps), bh, bw, joint, wear, t);
  let reliefPx = depth * joint * 1.2;               // relief height in css px
  let n = normalize(vec3f(-(hx - h) * reliefPx, -(hy - h) * reliefPx, eps));

  // ---- the raking sun: low, sweeping round the wall over the minute
  let sunA = 2.55 + 0.6 * sin(t * 0.9);
  let L = normalize(vec3f(cos(sunA), sin(sunA), 0.42));
  let diff = max(dot(n, L), 0.0);
  // cast shadow into the joint: a brick face standing sunward of a low
  // point blocks the sun. Probe the face mask a short way up the sun ray.
  let faceHere = smoothstep(0.35, 0.65, h);
  let probe = joint * (0.9 + 0.8 * depth);
  let cs = course(q + L.xy * probe, bh, bw, joint);
  let blocker = smoothstep(0.0, joint * 0.4, -cs.z);
  let shadow = 1.0 - 0.75 * blocker * (1.0 - faceHere) * min(depth, 1.5) / 1.5;
  // joints are also darker by occlusion alone
  let ao = 1.0 - 0.35 * (1.0 - faceHere) * min(depth, 1.5) / 1.5;

  // ---- materials, tinted by the palette but always fired clay and mortar
  let cell = course(q, bh, bw, joint);
  let id = cell.xy;
  let tone = hash21(id + 2.71);
  let flash = hash21(id + 8.13);
  let lumAll = luma(c0 + c1 + c2 + c3) * 0.25;
  // brick body: the first pole as an earthy fired tone
  let clayA = mix(vec3f(luma(c0)), c0, 0.62) * 0.98;
  let clayB = mix(vec3f(luma(c3)), c3, 0.55) * 0.62;         // dark burnt headers
  let clayC = mix(vec3f(luma(c1)), c1, 0.55) * 1.00;         // flashed bricks
  let toneMix = clamp(0.5 + (tone - 0.5) * (0.4 + 1.6 * vari), 0.0, 1.0);
  var brick = mix(clayB, clayA, toneMix);
  brick = mix(brick, clayC, smoothstep(0.86, 0.90, flash) * vari);
  // firing mottle across each face, and a sooty darkening in the pits
  let mottle = fbm3(q * 0.16 + id * 1.7) - 0.5;
  brick *= 1.0 + mottle * 0.22 * (0.5 + vari);
  brick *= 0.78 + 0.22 * smoothstep(0.55, 1.05, h);
  // mortar: a warm sandy grey built from the whole palette luminance
  var mortar = mix(vec3f(lumAll), (c2 + c0) * 0.5, 0.16) * 0.85 + vec3f(0.10);
  mortar *= 0.9 + 0.2 * (vnoise(q * 0.9 + 17.0) - 0.5) + 0.1 * (hash21(floor(q * 1.3)) - 0.5);
  // efflorescence: pale salts bloom on a few faces near the bed
  let salt = smoothstep(0.72, 0.95, fbm3(q * 0.05 + 99.0)) * 0.25;
  var albedo = mix(mortar, brick, faceHere);
  albedo = mix(albedo, albedo * 0.5 + vec3f(0.42), salt * (1.0 - 0.5 * faceHere));

  // ---- shade: matte clay, a hint of sheen where glazed faces catch the sun
  let V = vec3f(0.0, 0.0, 1.0);
  let H = normalize(L + V);
  var spec = max(dot(n, H), 0.0);
  spec = spec * spec; spec = spec * spec; spec = spec * spec;      // ^8
  let glaze = smoothstep(0.90, 0.96, flash) * faceHere * 0.18;
  let skyTint = mix(vec3f(1.0), c2, 0.25);
  var col = albedo * (0.20 * skyTint + 1.05 * (diff / L.z) * shadow) * ao;
  col += spec * glaze * mix(vec3f(1.0), c1, 0.3);
  // a low sun also grades the whole wall: brighter toward the light
  let uv = fc / u.resolution;
  col *= 0.72 + 0.55 * dot(uv - 0.5, L.xy);

  return vec4f(col, 1.0);
}
