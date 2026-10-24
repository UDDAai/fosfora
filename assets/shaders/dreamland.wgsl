// Dreamland — Kali, "Fractal Land" / "Fractal Cartoon" (2013)
// https://www.shadertoy.com/view/XsBXWt
// CC BY-NC-SA 3.0. The Nyan Cat sprite and the sound pass are omitted.
//
// param(0) = speed. 0.5 matches the original `t = iTime * 0.5`.
// param(1) = integral of speed, written by the engine (`"rates"` in the .pfx).
// Bass stands in for the original music-texture sample that sized the sun.

const RAY_STEPS: i32 = 150;
const DETAIL: f32 = 0.001;
const GAMMA: f32 = 1.4;
const BRIGHTNESS: f32 = 1.2;
const SATURATION: f32 = 0.65;
const ORIGIN: vec3f = vec3f(-1.0, 0.7, 0.0);

var<private> march_time: f32;
var<private> det: f32;
var<private> edge: f32;

struct Flight {
    go: vec3f,
    dir: vec3f,
}

// GLSL mod: `x - y * floor(x / y)`. WGSL `%` keeps the sign of x.
fn gmod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// GLSL `p * mat2(cos(a), sin(a), -sin(a), cos(a))` (row vector, column-major).
fn rot2(p: vec2f, a: f32) -> vec2f {
    let c = cos(a);
    let s = sin(a);
    return vec2f(p.x * c + p.y * s, -p.x * s + p.y * c);
}

// "Amazing Surface" fractal.
fn formula(p_in: vec4f) -> vec4f {
    var p = p_in;
    let xz = abs(p.xz + 1.0) - abs(p.xz - 1.0) - p.xz;
    p.x = xz.x;
    p.z = xz.y;
    p.y -= 0.25;
    let xy = rot2(p.xy, radians(35.0));
    p.x = xy.x;
    p.y = xy.y;
    p *= 2.0 / clamp(dot(p.xyz, p.xyz), 0.2, 1.0);
    return p;
}

fn de(pos_in: vec3f) -> f32 {
    var pos = pos_in;
    pos.y += sin(pos.z - march_time * 6.0) * 0.15;
    var tpos = pos;
    tpos.z = abs(3.0 - gmod(tpos.z, 6.0));
    var p = vec4f(tpos, 1.0);
    for (var i = 0; i < 4; i++) {
        p = formula(p);
    }
    let fr = (length(max(vec2f(0.0), p.yz - 1.5)) - 1.0) / p.w;
    var ro = max(abs(pos.x + 1.0) - 0.3, pos.y - 0.35);
    ro = max(ro, -max(abs(pos.x + 1.0) - 0.1, pos.y - 0.5));
    pos.z = abs(0.25 - gmod(pos.z, 0.5));
    ro = max(ro, -max(abs(pos.z) - 0.2, pos.y - 0.3));
    ro = max(ro, -max(abs(pos.z) - 0.01, -pos.y + 0.32));
    return min(fr, ro);
}

fn path(ti: f32) -> vec3f {
    let t = ti * 1.5;
    return vec3f(sin(t), (1.0 - sin(t * 2.0)) * 0.5, -t * 5.0) * 0.5;
}

fn surface_normal(p: vec3f) -> vec3f {
    let e = vec3f(0.0, det * 5.0, 0.0);
    let d1 = de(p - e.yxx);
    let d2 = de(p + e.yxx);
    let d3 = de(p - e.xyx);
    let d4 = de(p + e.xyx);
    let d5 = de(p - e.xxy);
    let d6 = de(p + e.xxy);
    let d = de(p);
    edge = abs(d - 0.5 * (d2 + d1)) + abs(d - 0.5 * (d4 + d3)) + abs(d - 0.5 * (d6 + d5));
    edge = min(1.0, pow(max(edge, 0.0), 0.55) * 15.0);
    let n = vec3f(d1 - d2, d3 - d4, d5 - d6);
    return n / max(length(n), 1e-6);
}

fn raymarch(ro: vec3f, dir_in: vec3f) -> vec3f {
    edge = 0.0;
    var dir = dir_in;
    var p = ro;
    var d = 100.0;
    var totdist = 0.0;
    for (var i = 0; i < RAY_STEPS; i++) {
        if (d <= det || totdist >= 25.0) {
            break;
        }
        p = ro + totdist * dir;
        d = de(p);
        det = DETAIL * exp(0.13 * totdist);
        totdist += d;
    }

    // Missed rays become sky. The normal is unused there, so skip it.
    var col = vec3f(0.0);
    if (totdist <= 25.0) {
        p -= (det - d) * dir;
        let norm = surface_normal(p);
        col = (1.0 - abs(norm)) * max(0.0, 1.0 - edge * 0.8);
    }

    totdist = clamp(totdist, 0.0, 26.0);
    dir.y -= 0.02;
    let sunsize = 7.0 - max(0.0, u.bass) * 5.0;
    // Original sun angle is `atan(dir.x, dir.y) + iTime * 1.5`, and iTime = t / 0.5.
    let an = atan2(dir.x, dir.y) + march_time * 3.0;
    let spoke = abs(0.2 - gmod(an, 0.4));
    let s = pow(clamp(1.0 - length(dir.xy) * sunsize - spoke, 0.0, 1.0), 0.1);
    let sb = pow(clamp(1.0 - length(dir.xy) * (sunsize - 0.2) - spoke, 0.0, 1.0), 0.1);
    let sg = pow(clamp(1.0 - length(dir.xy) * (sunsize - 4.5) - 0.5 * spoke, 0.0, 1.0), 3.0);
    let y = mix(0.45, 1.2, pow(smoothstep(0.0, 1.0, 0.75 - dir.y), 2.0)) * (1.0 - sb * 0.5);

    var backg = vec3f(0.5, 0.0, 1.0) * ((1.0 - s) * (1.0 - sg) * y + (1.0 - sb) * sg * vec3f(1.0, 0.8, 0.15) * 3.0);
    backg += vec3f(1.0, 0.9, 0.1) * s;
    backg = max(backg, sg * vec3f(1.0, 0.9, 0.5));

    if (totdist > 25.0) {
        col = backg;
    } else {
        col = mix(vec3f(1.0, 0.9, 0.3), col, exp(-0.004 * totdist * totdist));
    }
    col = pow(max(col, vec3f(0.0)), vec3f(GAMMA)) * BRIGHTNESS;
    col = mix(vec3f(length(col)), col, SATURATION);
    col *= vec3f(1.0, 0.9, 0.85);
    return col;
}

fn move_cam(dir_in: vec3f) -> Flight {
    var dir = dir_in;
    let go = path(march_time);
    let adv = path(march_time + 0.7);
    let advec = normalize(adv - go);
    var an = adv.x - go.x;
    an *= min(1.0, abs(adv.z - go.z)) * sign(adv.z - go.z) * 0.7;
    let xy = rot2(dir.xy, an);
    dir.x = xy.x;
    dir.y = xy.y;
    an = advec.y * 1.7;
    let yz = rot2(dir.yz, an);
    dir.y = yz.x;
    dir.z = yz.y;
    an = atan2(advec.x, advec.z);
    let xz = rot2(dir.xz, an);
    dir.x = xz.x;
    dir.z = xz.y;
    return Flight(go, dir);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    // Shadertoy's fragCoord origin is the bottom left. WGSL's is the top left.
    let fc = vec2f(frag_coord.x, res.y - frag_coord.y);
    var uv = (fc / res) * 2.0 - 1.0;
    let oriuv = uv;
    uv.y *= res.y / res.x;

    march_time = param(1u);
    det = 0.0;

    // Original mouse path when the button is up.
    let mouse = vec2f(0.0, -0.05);
    // Original fov uses iTime, which is march_time / 0.5 while speed stays at 0.5.
    let fov = 0.9 - max(0.0, 0.7 - march_time * 0.6);
    var dir = normalize(vec3f(uv * fov, 1.0));
    let yz = rot2(dir.yz, mouse.y);
    dir.y = yz.x;
    dir.z = yz.y;
    let xz = rot2(dir.xz, mouse.x);
    dir.x = xz.x;
    dir.z = xz.y;

    let flight = move_cam(dir);
    var color = raymarch(ORIGIN + flight.go, flight.dir);
    let vig = pow(
        max(0.0, 0.95 - length(oriuv * oriuv * oriuv * vec2f(1.05, 1.1))),
        0.3,
    );
    color = mix(vec3f(0.0), color, vig);
    return vec4f(max(color, vec3f(0.0)), 1.0);
}
