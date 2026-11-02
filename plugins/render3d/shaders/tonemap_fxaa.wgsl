// Fullscreen tone mapping with FXAA applied to the final display-referred color.
@group(0) @binding(0) var hdr_scene: texture_2d<f32>;
@group(0) @binding(1) var hdr_sampler: sampler;
@group(0) @binding(2) var bloom_texture: texture_2d<f32>;

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOut {
    let positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let position = positions[vertex_index];
    var out: VertexOut;
    out.clip = vec4<f32>(position, 0.0, 1.0);
    out.uv = vec2<f32>(position.x * 0.5 + 0.5, 0.5 - position.y * 0.5);
    return out;
}

fn display_color(uv_in: vec2<f32>) -> vec3<f32> {
    let uv = clamp(uv_in, vec2<f32>(0.0), vec2<f32>(1.0));
    let bloom = textureSample(bloom_texture, hdr_sampler, uv).rgb * 0.42;
    let hdr = max(textureSample(hdr_scene, hdr_sampler, uv).rgb * 1.15 + bloom, vec3<f32>(0.0));
    return hdr / (vec3<f32>(1.0) + hdr);
}

fn luma(color: vec3<f32>) -> f32 {
    return dot(color, vec3<f32>(0.299, 0.587, 0.114));
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let dimensions = vec2<f32>(textureDimensions(hdr_scene, 0));
    let inverse_size = 1.0 / max(dimensions, vec2<f32>(1.0));
    let uv = in.uv;

    let rgb_m = display_color(uv);
    let luma_m = luma(rgb_m);
    let luma_nw = luma(display_color(uv + vec2<f32>(-1.0, -1.0) * inverse_size));
    let luma_ne = luma(display_color(uv + vec2<f32>( 1.0, -1.0) * inverse_size));
    let luma_sw = luma(display_color(uv + vec2<f32>(-1.0,  1.0) * inverse_size));
    let luma_se = luma(display_color(uv + vec2<f32>( 1.0,  1.0) * inverse_size));

    let luma_min = min(luma_m, min(min(luma_nw, luma_ne), min(luma_sw, luma_se)));
    let luma_max = max(luma_m, max(max(luma_nw, luma_ne), max(luma_sw, luma_se)));
    let edge_span = luma_max - luma_min;
    if edge_span < max(0.0312, luma_max * 0.125) {
        return vec4<f32>(rgb_m, 1.0);
    }

    var direction = vec2<f32>(
        -((luma_nw + luma_ne) - (luma_sw + luma_se)),
          (luma_nw + luma_sw) - (luma_ne + luma_se),
    );
    let reduce = max((luma_nw + luma_ne + luma_sw + luma_se) * 0.03125, 0.0078125);
    let reciprocal_min = 1.0 / (min(abs(direction.x), abs(direction.y)) + reduce);
    direction = clamp(direction * reciprocal_min, vec2<f32>(-8.0), vec2<f32>(8.0)) * inverse_size;

    let rgb_a = 0.5 * (
        display_color(uv + direction * (1.0 / 3.0 - 0.5)) +
        display_color(uv + direction * (2.0 / 3.0 - 0.5))
    );
    let rgb_b = rgb_a * 0.5 + 0.25 * (
        display_color(uv + direction * -0.5) +
        display_color(uv + direction * 0.5)
    );
    let luma_b = luma(rgb_b);
    var result = rgb_b;
    if luma_b < luma_min || luma_b > luma_max {
        result = rgb_a;
    }
    return vec4<f32>(result, 1.0);
}
