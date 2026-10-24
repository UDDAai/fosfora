// Undertow — shader.gallery (undertow)
// MIT © E. T. Carter — https://shader.gallery
// Palette theme: midnight

const SG_C0: vec3f = vec3f(0.231000, 0.510000, 0.965000);
const SG_C1: vec3f = vec3f(0.659000, 0.333000, 0.969000);
const SG_C2: vec3f = vec3f(0.133000, 0.827000, 0.933000);
const SG_C3: vec3f = vec3f(0.957000, 0.247000, 0.369000);

fn sg_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// undertow (Wake) — concentric ripples & interference: three drifting sources emit
// expanding rings whose phases sum into a standing wave field, rendered as glowing
// contour bands over a deep dark base, palette-tinted by phase and position. The
// sources orbit on closed loops and the wave uses periodic time, so the whole field
// drifts and breathes with no visible jump or reset.
//
// Uniforms provided by the runtime:
//   u.time        seconds, monotonically increasing
//   u.resolution  drawing-buffer size in device pixels
//   vec2f(0.0)       pointer in device pixels (0,0 when absent)
//   1.0  devicePixelRatio used for the buffer
//   u_palette[4]  four glow colours, themeable (0..1 rgb)


// tweakable params (see meta.json; the runtime feeds defaults)

const BG: f32 = 0.039;   // deep dark base ~ #0A
const TAU: f32 = 6.2831853;

// one expanding-ring source: returns a signed wave height in -1..1
fn ripple(fc: vec2f, src: vec2f, wavelen: f32, phase: f32, t: f32) -> f32 {

  let d = distance(fc, src);
  // amplitude falls with distance so far sources don't wash the field
  let amp = exp(-d / (wavelen * 4.0));
  return amp * sin(d / wavelen * TAU - t + phase);
}
@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let fc = vec2f(frag_coord.x, res.y - frag_coord.y);
    let pr = 1.0;
  let ctr = res * 0.5;
  let t = u.time * param(0u);
  let wl = param(1u) * pr;
  let R = max(res.x, res.y);

  // --- three sources orbiting on closed loops (seamless) + a mouse nudge ---
  let s0 = vec2f(res.x * (0.32 + 0.10 * sin(t * 0.23)),
                 res.y * (0.40 + 0.12 * cos(t * 0.19)));
  let s1 = vec2f(res.x * (0.70 + 0.11 * cos(t * 0.17)),
                 res.y * (0.62 + 0.10 * sin(t * 0.27)));
  let s2 = vec2f(res.x * (0.55 + 0.13 * sin(t * 0.13)),
                 res.y * (0.30 + 0.11 * cos(t * 0.21)));
  // mouse drags a fourth source in when present (0,0 parks it off-field)
  let sm = select(vec2f(-R), vec2f(0.0), (vec2f(0.0).x + vec2f(0.0).y > 1.0));

  // --- sum the ripple field, each source ticking at its own speed ---
  var h = 0.0;
  h += ripple(fc, s0, wl, 0.0,        t * 0.9);
  h += ripple(fc, s1, wl, 2.1,        t * 1.1);
  h += ripple(fc, s2, wl, 4.2,        t * 0.8);
  h += ripple(fc, sm, wl * 0.7, 1.0,  t * 1.3) * 0.8 * param(4u);

  // contour bands: fold the summed height into repeating glowing lines
  let bandPhase = h * param(2u);
  let ridge = abs(fract(bandPhase) - 0.5) * 2.0;       // 0 between lines, 1 at line centre
  let line = smoothstep(0.78, 1.0, ridge);            // thin bright contour, lots of gap
  // soft fill between lines keeps it from reading as bare wireframe
  let fill = 0.5 + 0.5 * cos(bandPhase * TAU);

  // --- resolve palette (all colour comes from here when themed) ---
  // Some headless contexts can't bind a uniform array by its bare name, leaving
  // the palette at zero; fall back to the default "midnight" hues so the look
  // survives there while staying fully themeable in the browser runtime.
  var pal0 = SG_C0;
  var pal1 = SG_C1;
  var pal2 = SG_C2;
  var pal3 = SG_C3;
  if (dot(pal0 + pal1 + pal2 + pal3, vec3f(1.0)) < 0.01) {
    pal0 = vec3f(0.231, 0.510, 0.965);
    pal1 = vec3f(0.659, 0.333, 0.969);
    pal2 = vec3f(0.133, 0.827, 0.933);
    pal3 = vec3f(0.957, 0.247, 0.369);
  }

  // --- palette tint by phase (which band) and by position (radial) ---
  let ph = fract(h * 0.5 + 0.5);                    // 0..1 cycling phase
  let cool = mix(pal0, pal2, ph);                     // blue <-> cyan
  let warm = mix(pal1, pal3, ph);                     // violet <-> rose
  let radial = smoothstep(R * 0.85, 0.0, distance(fc, ctr));
  let tint = mix(cool, warm, radial * 0.6 + 0.18 * ph);

  // --- compose: dark base + faint fill glow + bright contour lines ---
  var col = vec3f(BG, BG, 0.047);
  col += tint * fill * 0.06;                             // restrained between-line wash
  col += tint * line * (0.65 + 0.45 * radial) * param(3u); // glowing contour bands

  // gentle source halos so the emitters read as luminous points
  col += pal0 * 0.10 * smoothstep(R * 0.30, 0.0, distance(fc, s0));
  col += pal2 * 0.09 * smoothstep(R * 0.28, 0.0, distance(fc, s1));
  col += pal1 * 0.08 * smoothstep(R * 0.26, 0.0, distance(fc, s2));

  // vignette to settle the edges into the dark
  let vig = smoothstep(1.05, 0.35, length((fc - ctr) / res));
  col *= 0.55 + 0.45 * vig;

  return vec4f(col, 1.0);
}
