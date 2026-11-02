// GPU-only average log-luminance adaptation. A 64-lane workgroup samples a deterministic 16x16
// grid from the HDR scene, reduces it in workgroup memory and updates persistent GPU exposure.
// No pixel data is mapped or read back to CPU memory.
@group(0) @binding(0) var hdr_scene: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> exposure_state: f32;

struct ExposureParams {
    limits: vec4<f32>,      // min exposure, max exposure, key value, frame delta seconds
    adaptation: vec4<f32>,  // speed-up, speed-down, reserved, reserved
}

@group(0) @binding(2) var<uniform> params: ExposureParams;
var<workgroup> partial_log_luminance: array<f32, 64>;

@compute @workgroup_size(8, 8, 1)
fn cs_main(@builtin(local_invocation_index) local_index: u32) {
    let dimensions = textureDimensions(hdr_scene, 0);
    let safe_dimensions = max(dimensions, vec2<u32>(1u, 1u));
    var local_log_sum = 0.0;
    for (var sample_offset = 0u; sample_offset < 4u; sample_offset = sample_offset + 1u) {
        let sample_index = local_index + sample_offset * 64u;
        let sample_x = sample_index % 16u;
        let sample_y = sample_index / 16u;
        let normalized = (vec2<f32>(f32(sample_x), f32(sample_y)) + vec2<f32>(0.5)) / 16.0;
        let pixel = min(
            vec2<u32>(normalized * vec2<f32>(safe_dimensions)),
            safe_dimensions - vec2<u32>(1u),
        );
        let color = max(textureLoad(hdr_scene, vec2<i32>(pixel), 0).rgb, vec3<f32>(0.0));
        let luminance = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
        local_log_sum += log(max(luminance, 0.0001));
    }
    partial_log_luminance[local_index] = local_log_sum;
    workgroupBarrier();

    for (var stride = 32u; stride > 0u; stride = stride / 2u) {
        if local_index < stride {
            partial_log_luminance[local_index] += partial_log_luminance[local_index + stride];
        }
        workgroupBarrier();
    }

    if local_index == 0u {
        let average_luminance = exp(partial_log_luminance[0] / 256.0);
        let target_exposure = clamp(
            params.limits.z / max(average_luminance, 0.0001),
            params.limits.x,
            params.limits.y,
        );
        let previous_exposure = clamp(exposure_state, params.limits.x, params.limits.y);
        let adaptation_speed = select(
            params.adaptation.y,
            params.adaptation.x,
            target_exposure > previous_exposure,
        );
        let adaptation = 1.0 - exp(-max(params.limits.w, 0.0) * adaptation_speed);
        exposure_state = mix(previous_exposure, target_exposure, adaptation);
    }
}
