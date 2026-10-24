// OrganicLiquid — recolor the chemical field. The original image pass samples
// an unconnected iChannel2 for a second gradient, so that term is absent here.
// With the mouse up, the spotlight sits off-screen and the filaments stand alone.

@fragment
fn fs_main(@builtin(position) frag_coord: vec4f) -> @location(0) vec4f {
    let res = max(u.resolution, vec2f(1.0));
    let uv = frag_coord.xy / res;
    let pixel = 1.0 / res;
    let aspect = vec2f(1.0, res.y / res.x);
    let d = pixel;

    let dx = input0(uv + vec2f(d.x, 0.0)) - input0(uv - vec2f(d.x, 0.0));
    let dy = input0(uv + vec2f(0.0, d.y)) - input0(uv - vec2f(0.0, d.y));
    let light_size = 4.0;
    let displacement = vec2f(dx.x, dy.x) * light_size;
    // iMouse is zero until the button is down, which parks the light off-screen.
    let mouse = vec2f(0.0);
    let light = pow(max(1.0 - distance(
        0.5 + (uv - 0.5) * aspect * light_size + displacement,
        0.5 + (mouse - 0.5) * aspect * light_size,
    ), 0.0), 4.0);

    let shifted = uv + vec2f(dx.x, dy.x) * pixel * 8.0;
    let sample = input0(shifted).x;
    let rd = vec4f(sample) * vec4f(0.7, 1.5, 2.0, 1.0) - vec4f(0.3, 1.0, 1.0, 1.0);
    let lit = mix(rd, vec4f(8.0, 6.0, 2.0, 1.0), light * 0.75 * vec4f(1.0 - sample));
    return vec4f(max(lit.rgb, vec3f(0.0)), 1.0);
}
