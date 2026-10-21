// Quarter-resolution bright-pass blur used for a compact, deterministic bloom glow.
@group(0) @binding(0) var hdr_scene: texture_2d<f32>;
@group(0) @binding(1) var hdr_sampler: sampler;

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOut {
    let positions = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    let position = positions[vertex_index];
    var out: VertexOut;
    out.clip = vec4<f32>(position, 0.0, 1.0);
    out.uv = vec2<f32>(position.x * 0.5 + 0.5, 0.5 - position.y * 0.5);
    return out;
}

fn bright_pass(sample_color: vec3<f32>) -> vec3<f32> {
    let brightness = max(sample_color.r, max(sample_color.g, sample_color.b));
    return max(sample_color - vec3<f32>(0.8), vec3<f32>(0.0)) * smoothstep(0.65, 1.8, brightness);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let texel = 4.0 / vec2<f32>(textureDimensions(hdr_scene));
    var glow = vec3<f32>(0.0);
    glow += bright_pass(textureSample(hdr_scene, hdr_sampler, in.uv + texel * vec2<f32>(-1.0, -1.0)).rgb) * 1.0;
    glow += bright_pass(textureSample(hdr_scene, hdr_sampler, in.uv + texel * vec2<f32>( 0.0, -1.0)).rgb) * 2.0;
    glow += bright_pass(textureSample(hdr_scene, hdr_sampler, in.uv + texel * vec2<f32>( 1.0, -1.0)).rgb) * 1.0;
    glow += bright_pass(textureSample(hdr_scene, hdr_sampler, in.uv + texel * vec2<f32>(-1.0,  0.0)).rgb) * 2.0;
    glow += bright_pass(textureSample(hdr_scene, hdr_sampler, in.uv).rgb) * 4.0;
    glow += bright_pass(textureSample(hdr_scene, hdr_sampler, in.uv + texel * vec2<f32>( 1.0,  0.0)).rgb) * 2.0;
    glow += bright_pass(textureSample(hdr_scene, hdr_sampler, in.uv + texel * vec2<f32>(-1.0,  1.0)).rgb) * 1.0;
    glow += bright_pass(textureSample(hdr_scene, hdr_sampler, in.uv + texel * vec2<f32>( 0.0,  1.0)).rgb) * 2.0;
    glow += bright_pass(textureSample(hdr_scene, hdr_sampler, in.uv + texel * vec2<f32>( 1.0,  1.0)).rgb) * 1.0;
    return vec4<f32>(glow / 16.0, 1.0);
}
