// Fullscreen triangle and a simple exposure + Reinhard tone map.
@group(0) @binding(0) var hdr_scene: texture_2d<f32>;
@group(0) @binding(1) var hdr_sampler: sampler;

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

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let hdr = max(textureSample(hdr_scene, hdr_sampler, in.uv).rgb * 1.15, vec3<f32>(0.0));
    let mapped = hdr / (vec3<f32>(1.0) + hdr);
    return vec4<f32>(mapped, 1.0);
}
