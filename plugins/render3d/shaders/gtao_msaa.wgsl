// Multisampled-depth variant of the same deterministic screen-space horizon AO kernel.
struct GtaoParams {
    inverse_view_projection: mat4x4<f32>,
    camera_and_radius: vec4<f32>, // camera world position, world-space radius
    settings: vec4<f32>,          // intensity, bias, power, projection Y scale
    dimensions: vec4<u32>,        // width, height, depth sample count, reserved
    quality: vec4<u32>,           // direction count, steps per direction, reserved, reserved
}

@group(0) @binding(0) var depth_texture: texture_depth_multisampled_2d;
@group(0) @binding(1) var ao_output: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(2) var<uniform> params: GtaoParams;



fn load_depth(pixel: vec2<i32>) -> f32 {
    var nearest_depth = 1.0;
    let sample_count = clamp(params.dimensions.z, 1u, 8u);
    for (var sample_index = 0u; sample_index < sample_count; sample_index += 1u) {
        nearest_depth = min(nearest_depth, textureLoad(depth_texture, pixel, i32(sample_index)));
    }
    return nearest_depth;
}

fn reconstruct_world(pixel: vec2<i32>, depth: f32) -> vec3<f32> {
    let dimensions = vec2<f32>(params.dimensions.xy);
    let uv = (vec2<f32>(pixel) + vec2<f32>(0.5)) / dimensions;
    let clip = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let world = params.inverse_view_projection * clip;
    return world.xyz / max(abs(world.w), 0.000001) * sign(world.w);
}

fn clamped_pixel(pixel: vec2<i32>) -> vec2<i32> {
    let maximum = vec2<i32>(params.dimensions.xy) - vec2<i32>(1, 1);
    return clamp(pixel, vec2<i32>(0, 0), maximum);
}

@compute @workgroup_size(8, 8, 1)
fn cs_main(@builtin(global_invocation_id) invocation: vec3<u32>) {
    if invocation.x >= params.dimensions.x || invocation.y >= params.dimensions.y {
        return;
    }

    let pixel = vec2<i32>(invocation.xy);
    let depth = load_depth(pixel);
    if depth >= 0.999999 {
        textureStore(ao_output, pixel, vec4<f32>(1.0, 1.0, 1.0, 1.0));
        return;
    }

    let center = reconstruct_world(pixel, depth);
    let left_pixel = clamped_pixel(pixel + vec2<i32>(-1, 0));
    let right_pixel = clamped_pixel(pixel + vec2<i32>(1, 0));
    let up_pixel = clamped_pixel(pixel + vec2<i32>(0, -1));
    let down_pixel = clamped_pixel(pixel + vec2<i32>(0, 1));
    let left_depth = load_depth(left_pixel);
    let right_depth = load_depth(right_pixel);
    let up_depth = load_depth(up_pixel);
    let down_depth = load_depth(down_pixel);
    let left = reconstruct_world(left_pixel, left_depth);
    let right = reconstruct_world(right_pixel, right_depth);
    let up = reconstruct_world(up_pixel, up_depth);
    let down = reconstruct_world(down_pixel, down_depth);
    // Ignore clear-depth and discontinuous neighbors when reconstructing the local normal.
    // If one axis has no reliable derivative, use a view-facing fallback rather than a far-plane
    // position; this prevents unstable normals and dark halos at silhouettes and screen edges.
    let depth_tolerance = max(0.0005, depth * 0.002);
    let left_valid = left_depth < 0.999999 && abs(left_depth - depth) <= depth_tolerance;
    let right_valid = right_depth < 0.999999 && abs(right_depth - depth) <= depth_tolerance;
    let up_valid = up_depth < 0.999999 && abs(up_depth - depth) <= depth_tolerance;
    let down_valid = down_depth < 0.999999 && abs(down_depth - depth) <= depth_tolerance;
    var tangent_x = vec3<f32>(0.0);
    var tangent_y = vec3<f32>(0.0);
    if left_valid && right_valid {
        tangent_x = right - left;
    } else if right_valid {
        tangent_x = right - center;
    } else if left_valid {
        tangent_x = center - left;
    }
    if up_valid && down_valid {
        tangent_y = down - up;
    } else if down_valid {
        tangent_y = down - center;
    } else if up_valid {
        tangent_y = center - up;
    }
    let to_camera = normalize(params.camera_and_radius.xyz - center);
    var normal = to_camera;
    if length(tangent_x) > 0.000001 && length(tangent_y) > 0.000001 {
        let candidate = cross(tangent_y, tangent_x);
        if length(candidate) > 0.000001 {
            normal = normalize(candidate);
        }
    }
    if dot(normal, to_camera) < 0.0 {
        normal = -normal;
    }

    let view_distance = max(distance(center, params.camera_and_radius.xyz), 0.1);
    let focal_pixels = params.settings.w * f32(params.dimensions.y) * 0.5;
    let radius_pixels = clamp(params.camera_and_radius.w * focal_pixels / view_distance, 2.0, 64.0);
    let direction_count = clamp(params.quality.x, 1u, 8u);
    let step_count = clamp(params.quality.y, 1u, 4u);
    var accumulated_occlusion = 0.0;

    for (var direction_index = 0u; direction_index < direction_count; direction_index += 1u) {
        let angle = 6.28318530718 * f32(direction_index) / f32(direction_count);
        let direction = vec2<f32>(cos(angle), sin(angle));
        var horizon = 0.0;
        for (var step_index = 0u; step_index < step_count; step_index += 1u) {
            let step_fraction = (f32(step_index) + 1.0) / f32(step_count);
            let offset = vec2<i32>(round(direction * radius_pixels * step_fraction));
            let sample_pixel = pixel + offset;
            if sample_pixel.x < 0 || sample_pixel.y < 0 || sample_pixel.x >= i32(params.dimensions.x) || sample_pixel.y >= i32(params.dimensions.y) {
                continue;
            }
            let sample_depth = load_depth(sample_pixel);
            if sample_depth >= 0.999999 {
                continue;
            }
            let sample_position = reconstruct_world(sample_pixel, sample_depth);
            let delta = sample_position - center;
            let sample_distance = length(delta);
            if sample_distance <= params.settings.y || sample_distance >= params.camera_and_radius.w {
                continue;
            }
            let toward_camera = dot(delta, to_camera);
            if toward_camera <= params.settings.y {
                continue;
            }
            let alignment = max(dot(normal, delta / max(sample_distance, 0.000001)), 0.0);
            let radial_weight = 1.0 - sample_distance / params.camera_and_radius.w;
            let depth_weight = clamp(toward_camera / max(params.camera_and_radius.w * 0.2, 0.001), 0.0, 1.0);
            horizon = max(horizon, alignment * radial_weight * depth_weight);
        }
        accumulated_occlusion += horizon;
    }

    let average_occlusion = accumulated_occlusion / f32(direction_count);
    let visibility = pow(clamp(1.0 - average_occlusion * params.settings.x, 0.0, 1.0), params.settings.z);
    textureStore(ao_output, pixel, vec4<f32>(visibility, visibility, visibility, 1.0));
}
