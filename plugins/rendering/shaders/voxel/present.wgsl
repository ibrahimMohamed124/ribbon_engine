
struct Screen { size: vec2<f32>, mode: u32, frame: u32, time: f32, amount: f32, pad: vec2<f32> };
@group(0) @binding(0) var source_image: texture_2d<f32>;
@group(0) @binding(1) var<uniform> screen: Screen;
@group(1) @binding(0) var<uniform> quad_screen: Screen;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    return vec4<f32>(positions[index], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let source_size = vec2<f32>(textureDimensions(source_image));
    let scale = min(screen.size.x / source_size.x, screen.size.y / source_size.y);
    let offset = (screen.size - source_size * scale) * 0.5;
    let local = position.xy - offset;
    if (local.x < 0.0 || local.y < 0.0 || local.x >= source_size.x * scale || local.y >= source_size.y * scale) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let pixel = vec2<i32>(floor(local / scale));
    return vec4<f32>(textureLoad(source_image, pixel, 0).rgb, 1.0);
}

fn hash4(x: i32, y: i32, z: i32, w: i32) -> f32 {
    var h = bitcast<u32>(x) * 73856093u;
    h = h ^ (bitcast<u32>(y) * 19349663u);
    h = h ^ (bitcast<u32>(z) * 83492791u);
    h = h ^ (bitcast<u32>(w) * 2654435761u);
    h = h ^ (h >> 13u);
    h = h * 0x5bd1e995u;
    h = h ^ (h >> 15u);
    return f32(h & 0xffffu) / 32767.5 - 1.0;
}

fn gray(value: f32) -> vec3<f32> { return vec3<f32>(clamp(value, 0.0, 1.0)); }

@fragment
fn fs_internal(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(position.xy);
    let dims = vec2<i32>(textureDimensions(source_image));
    let x = f32(p.x);
    let y = f32(p.y);
    let width = f32(dims.x);
    let height = f32(dims.y);
    var color = textureLoad(source_image, p, 0).rgb;
    if (screen.mode == 1u) {
        color = color * clamp(1.0 - screen.amount, 0.0, 1.0);
    } else if (screen.mode == 2u) {
        let vertical = y / height;
        let nx = (x / width - 0.5) * 1.25;
        let ny = (vertical - 0.46) * 1.4;
        let vignette = clamp(1.0 - sqrt(nx * nx + ny * ny), 0.0, 1.0);
        let grain = hash4(p.x / 3, p.y / 3, i32(screen.frame / 8u), 71) * 0.025;
        let glow = max(1.0 - abs(x / width - 0.72) * 4.0, 0.0) * max(1.0 - vertical, 0.0) * 0.035;
        let g = clamp(0.018 + vignette * 0.045 + grain + glow, 0.0, 0.13);
        color = vec3<f32>(clamp(g + glow * 0.52, 0.0, 0.14), g, g);
        let pulse = i32(hash4(i32(screen.frame / 7u), 0, 0, 4) * 4.0);
        if (p.y >= dims.y - 12 && p.y < dims.y - 11) { color = vec3<f32>(20.0, 24.0, 27.0) / 255.0; }
        if (p.y >= dims.y / 5 && p.y < dims.y / 5 + 2 + pulse && p.x >= dims.x * 3 / 4 - 18 && p.x < dims.x * 3 / 4 + 18) {
            color = vec3<f32>(57.0, 41.0, 41.0) / 255.0;
        }
    } else if (screen.mode == 3u) {
        let n = (hash4(p.x, p.y, i32(screen.frame), 9) + 1.0) * 0.5;
        color = gray(0.04 + 0.22 * n);
        if (screen.time < 0.12) {
            color = vec3<f32>(1.0);
        } else if (hash4(i32(screen.frame / 2u), 1, 2, 3) <= 0.85) {
            let hw = i32(min(14.0 + screen.time * 40.0, 64.0));
            let hh = i32(min(8.0 + screen.time * 22.0, 34.0));
            let jx = i32(hash4(i32(screen.frame), 4, 4, 4) * 3.0);
            let cx = dims.x / 2;
            let cy = dims.y / 2 - 22;
            let left_eye = p.x >= cx - 62 - hw / 2 + jx && p.x < cx - 62 - hw / 2 + jx + hw && p.y >= cy - hh / 2 && p.y < cy - hh / 2 + hh;
            let right_eye = p.x >= cx + 62 - hw / 2 + jx && p.x < cx + 62 - hw / 2 + jx + hw && p.y >= cy - hh / 2 && p.y < cy - hh / 2 + hh;
            let mouth = p.x >= cx - 58 + jx && p.x < cx + 58 + jx && p.y >= cy + 62 && p.y < cy + 66 + i32(screen.time * 3.0);
            if (left_eye || right_eye || mouth) { color = vec3<f32>(1.0); }
        }
    } else if (screen.mode == 4u) {
        color = gray(min(screen.time / 3.0, 1.0) * 0.9 + hash4(p.x, p.y, i32(screen.frame), 11) * 0.04);
    } else if (screen.mode == 5u) {
        color = vec3<f32>(1.0);
    } else if (screen.mode == 6u) {
        color = clamp(color * clamp(screen.amount, 0.5, 1.5), vec3<f32>(0.0), vec3<f32>(1.0));
    }
    return vec4<f32>(color, 1.0);
}

struct QuadInput { @location(0) rect: vec4<f32>, @location(1) color: vec4<f32> };
struct QuadOutput { @builtin(position) position: vec4<f32>, @location(0) color: vec4<f32> };
@vertex
fn vs_quad(@builtin(vertex_index) index: u32, input: QuadInput) -> QuadOutput {
    var corners = array<vec2<f32>, 6>(vec2<f32>(0.0,0.0), vec2<f32>(1.0,0.0), vec2<f32>(0.0,1.0), vec2<f32>(0.0,1.0), vec2<f32>(1.0,0.0), vec2<f32>(1.0,1.0));
    let pixel = input.rect.xy + corners[index] * input.rect.zw;
    let clip = vec2<f32>(pixel.x / quad_screen.size.x * 2.0 - 1.0, 1.0 - pixel.y / quad_screen.size.y * 2.0);
    var output: QuadOutput;
    output.position = vec4<f32>(clip, 0.0, 1.0);
    output.color = input.color;
    return output;
}
@fragment
fn fs_quad(input: QuadOutput) -> @location(0) vec4<f32> { return input.color; }
