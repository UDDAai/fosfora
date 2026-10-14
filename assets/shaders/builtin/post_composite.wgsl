// Post-processing composite shader.
// Combines: scene + bloom, chromatic aberration, ACES tonemap, vignette, film grain.

@group(0) @binding(0) var scene_texture: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
@group(0) @binding(2) var bloom_texture: texture_2d<f32>;
@group(0) @binding(3) var bloom_sampler: sampler;

struct PostParams {
    bloom_intensity: f32,
    ca_intensity: f32,     // chromatic aberration (onset-driven)
    vignette_strength: f32,
    grain_intensity: f32,  // film grain (flatness-driven)
    time: f32,
    rms: f32,
    alpha_mode: f32,       // 0 = opaque, 1 = luma-derived, 2 = scene-alpha passthrough
    tonemap_mode: f32,     // 0 = ACES, 1 = linear passthrough (SuperSplat-faithful)
    grain_rate: f32,       // grain updates per second; <= 0 = every frame
    // Flat f32s. A vec3f would align to 16 and disagree with the Rust layout.
    // 24 floats = 96 bytes. color-1 shadows, color-2 mids, color-3 highlights,
    // color-4 accent. Strength 0 leaves the picture unchanged.
    grade_strength: f32,
    shadow_r: f32,
    shadow_g: f32,
    shadow_b: f32,
    mid_r: f32,
    mid_g: f32,
    mid_b: f32,
    high_r: f32,
    high_g: f32,
    high_b: f32,
    accent_r: f32,
    accent_g: f32,
    accent_b: f32,
    _end0: f32,
    _end1: f32,
}
@group(0) @binding(4) var<uniform> post: PostParams;

// ACES filmic tonemapping
fn aces_tonemap(x: vec3f) -> vec3f {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3f(0.0), vec3f(1.0));
}

// Hash for film grain
fn hash_grain(p: vec2f) -> f32 {
    var p3 = fract(vec3f(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

@fragment
fn fs_main(@location(0) uv: vec2f) -> @location(0) vec4f {
    let ca = post.ca_intensity;

    // Center tap: RGB for the no-CA path / green channel, and the scene's coverage
    // alpha — the one value the passthrough alpha mode exists to preserve.
    let scene_px = textureSample(scene_texture, scene_sampler, uv);

    // Chromatic aberration: offset R and B channels
    var color: vec3f;
    if ca > 0.001 {
        let ca_offset = (uv - 0.5) * ca;
        color.r = textureSample(scene_texture, scene_sampler, uv + ca_offset).r;
        color.g = scene_px.g;
        color.b = textureSample(scene_texture, scene_sampler, uv - ca_offset).b;
    } else {
        color = scene_px.rgb;
    }

    // Bloom mix (RMS modulates intensity)
    let bloom = textureSample(bloom_texture, bloom_sampler, uv).rgb;
    let bloom_mix = post.bloom_intensity * (0.7 + post.rms * 0.6);
    color += bloom * bloom_mix;

    // Tonemap: ACES (house look) or linear passthrough (SuperSplat-faithful,
    // preserves raw sRGB contrast — no highlight compression / dark lift).
    if post.tonemap_mode < 0.5 {
        color = aces_tonemap(color);
    } else {
        color = clamp(color, vec3f(0.0), vec3f(1.0));
    }

    // Palette grade. Luma of the footage is kept; the swatch supplies the hue
    // for that brightness band, so a video stays a video.
    let grade_strength = clamp(post.grade_strength, 0.0, 1.0);
    if grade_strength > 0.001 {
        let luma_w = vec3f(0.2126, 0.7152, 0.0722);
        let luma = dot(color, luma_w);
        let shadow_w = 1.0 - smoothstep(0.05, 0.55, luma);
        let high_w = smoothstep(0.45, 0.92, luma);
        let mid_w = clamp(1.0 - shadow_w - high_w, 0.0, 1.0);
        let mapped = vec3f(post.shadow_r, post.shadow_g, post.shadow_b) * shadow_w
            + vec3f(post.mid_r, post.mid_g, post.mid_b) * mid_w
            + vec3f(post.high_r, post.high_g, post.high_b) * high_w;
        let mapped_l = max(dot(mapped, luma_w), 0.001);
        var recolored = (mapped / mapped_l) * luma;
        let accent = vec3f(post.accent_r, post.accent_g, post.accent_b);
        recolored = mix(recolored, accent * luma, high_w * 0.35);
        color = mix(color, recolored, grade_strength);
    }

    // Vignette
    let vignette_dist = length(uv - 0.5) * 1.414; // normalize to 0-1 at corners
    let vignette = 1.0 - post.vignette_strength * vignette_dist * vignette_dist;
    color *= vignette;

    // Film grain (flatness-driven: more grain when audio is flat/quiet).
    // The grain advances on its own cadence, not the display's. Continuous
    // full-rate noise integrates perceptually to smooth grey, so any repeated
    // frame freezes it into a sharp static field that reads as a flash — and
    // the compositor repeats frames on every window click, outside our control
    // (#1983). Holding each pattern for a few refreshes by design makes one
    // extra held refresh unremarkable. grain_rate <= 0 = the old every-frame
    // behaviour, bit for bit.
    let grain_t = select(
        post.time,
        floor(post.time * post.grain_rate) / post.grain_rate,
        post.grain_rate > 0.0,
    );
    let grain = (hash_grain(uv * 1000.0 + grain_t * 100.0) - 0.5) * post.grain_intensity;
    color += vec3f(grain);

    let final_color = clamp(color, vec3f(0.0), vec3f(1.0));
    // Output alpha (docs/alpha.md): opaque is the historical behavior, luma the legacy
    // NDI key, passthrough the overlay path — the scene's premultiplied coverage
    // survives to the surface/capture. RGB above is deliberately NOT masked by it:
    // bloom/CA spill over a=0 regions is additive premultiplied light.
    var alpha = 1.0;
    if post.alpha_mode > 1.5 {
        alpha = clamp(scene_px.a, 0.0, 1.0);
    } else if post.alpha_mode > 0.5 {
        let brightness = max(final_color.r, max(final_color.g, final_color.b));
        alpha = clamp(brightness * 2.0, 0.0, 1.0);
    }
    return vec4f(final_color, alpha);
}
