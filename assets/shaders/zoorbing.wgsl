// Zoorbing — whisky_shusuky, "Octagrams" (2020)
// https://www.shadertoy.com/view/tlVGDt
// CC BY-NC-SA 3.0.
//
// param(0) = speed. 0.5 matches the original iTime.
// param(1) = integral of speed, written by the engine (`"rates"` in the .pfx).

var<private> g_time: f32;

fn gmod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

fn gmod3(p: vec3f, y: f32) -> vec3f {
    return vec3f(gmod(p.x, y), gmod(p.y, y), gmod(p.z, y));
}

// GLSL `p * mat2(cos(a), sin(a), -sin(a), cos(a))`.
fn rot2(p: vec2f, a: f32) -> vec2f {
    let c = cos(a);
    let s = sin(a);
    return vec2f(p.x * c + p.y * s, -p.x * s + p.y * c);
}

fn spin_xy(p: vec3f, a: f32) -> vec3f {
    let xy = rot2(p.xy, a);
    return vec3f(xy.x, xy.y, p.z);
}

fn sd_box(p: vec3f, b: vec3f) -> f32 {
    let q = abs(p) - b;
    return length(max(q, vec3f(0.0))) + min(max(q.x, max(q.y, q.z)), 0.0);
}

fn slab(pos_in: vec3f, scale: f32) -> f32 {
    let pos = pos_in * scale;
    return -sd_box(pos, vec3f(0.4, 0.4, 0.1)) / 1.5;
}

fn box_set(pos_in: vec3f) -> f32 {
    let pulse = abs(sin(g_time * 0.4));
    let scale = 2.0 - pulse * 1.5;
    let sway = sin(g_time * 0.4) * 2.5;

    var pos = pos_in;
    pos.y += sway;
    let box1 = slab(spin_xy(pos, 0.8), scale);

    pos = pos_in;
    pos.y -= sway;
    let box2 = slab(spin_xy(pos, 0.8), scale);

    pos = pos_in;
    pos.x += sway;
    let box3 = slab(spin_xy(pos, 0.8), scale);

    pos = pos_in;
    pos.x -= sway;
    let box4 = slab(spin_xy(pos, 0.8), scale);

    let box5 = slab(spin_xy(pos_in, 0.8), 0.5) * 6.0;
    let box6 = slab(pos_in, 0.5) * 6.0;
    return max(max(max(max(max(box1, box2), box3), box4), box5), box6);
}

@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    // Shadertoy's fragCoord origin is the bottom left.
    let fc = vec2f(frag_coord.x, res.y - frag_coord.y);
    let p = (fc * 2.0 - res) / min(res.x, res.y);
    let clock = param(1u) * 2.0;

    let ro = vec3f(0.0, -0.2, clock * 4.0);
    var ray = normalize(vec3f(p, 1.5));
    let xy = rot2(ray.xy, sin(clock * 0.03) * 5.0);
    ray.x = xy.x;
    ray.y = xy.y;
    let yz = rot2(ray.yz, sin(clock * 0.05) * 0.2);
    ray.y = yz.x;
    ray.z = yz.y;

    var t = 0.1;
    var ac = 0.0;
    for (var i = 0; i < 99; i++) {
        var pos = ro + ray * t;
        pos = gmod3(pos - 2.0, 4.0) - 2.0;
        g_time = clock - f32(i) * 0.01;
        let d = max(abs(box_set(pos)), 0.01);
        ac += exp(-d * 23.0);
        t += d * 0.55;
    }

    var col = vec3f(ac * 0.02 * (1.0 + u.bass * 0.8));
    col += vec3f(0.0, 0.2 * abs(sin(clock)), 0.5 + sin(clock) * 0.2);
    return vec4f(max(col, vec3f(0.0)), 1.0);
}
