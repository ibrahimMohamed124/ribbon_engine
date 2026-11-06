// Full-screen visualization for one layer of the directional Depth32Float texture array.
// Clear depth (1.0) is black; nearer casters are brighter (1.0 - depth).
struct DebugParams {
    selected_layer: vec4<u32>,
    target_size: vec4<u32>,
}

@group(0) @binding(0) var<uniform> debug_params: DebugParams;
@group(0) @binding(1) var directional_depth_array: texture_depth_2d_array;

struct FullscreenVertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index: u32) -> FullscreenVertexOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let position = positions[vertex_index];
    var output: FullscreenVertexOut;
    output.position = vec4<f32>(position, 0.0, 1.0);
    output.uv = vec2<f32>(position.x * 0.5 + 0.5, 0.5 - position.y * 0.5);
    return output;
}

@fragment
fn fs_depth_layer(input: FullscreenVertexOut) -> @location(0) vec4<f32> {
    let dimensions = textureDimensions(directional_depth_array, 0);
    let target_width = max(debug_params.target_size.x, 1u);
    let target_height = max(debug_params.target_size.y, 1u);
    let target_aspect = f32(target_width) / f32(target_height);
    let depth_aspect = f32(dimensions.x) / f32(dimensions.y);
    var uv = input.uv;

    // Preserve the square shadow-map aspect ratio; letterbox instead of stretching it to
    // the (usually widescreen) scene target.
    if target_aspect > depth_aspect {
        let displayed_width = depth_aspect / target_aspect;
        let left = (1.0 - displayed_width) * 0.5;
        if uv.x < left || uv.x > left + displayed_width {
            return vec4<f32>(0.0, 0.0, 0.0, 1.0);
        }
        uv.x = (uv.x - left) / displayed_width;
    } else {
        let displayed_height = target_aspect / depth_aspect;
        let top = (1.0 - displayed_height) * 0.5;
        if uv.y < top || uv.y > top + displayed_height {
            return vec4<f32>(0.0, 0.0, 0.0, 1.0);
        }
        uv.y = (uv.y - top) / displayed_height;
    }

    let safe_uv = clamp(uv, vec2<f32>(0.0), vec2<f32>(0.999999));
    let pixel = vec2<i32>(safe_uv * vec2<f32>(dimensions));
    let maximum = vec2<i32>(dimensions) - vec2<i32>(1, 1);
    let coordinate = clamp(pixel, vec2<i32>(0, 0), maximum);
    let layer = i32(min(debug_params.selected_layer.x, 3u));
    let depth = textureLoad(directional_depth_array, coordinate, layer, 0);
    let visible_caster = 1.0 - depth;
    return vec4<f32>(vec3<f32>(visible_caster), 1.0);
}
