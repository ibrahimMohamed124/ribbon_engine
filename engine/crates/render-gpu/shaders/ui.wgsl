
struct Screen { size: vec2<f32>, mode: u32, frame: u32, time: f32, amount: f32, pad: vec2<f32> };
@group(0) @binding(0) var<uniform> screen: Screen;
struct QuadInput { @location(0) rect: vec4<f32>, @location(1) color: vec4<f32> };
struct QuadOutput { @builtin(position) position: vec4<f32>, @location(0) color: vec4<f32> };
@vertex
fn vs_quad(@builtin(vertex_index) index: u32, input: QuadInput) -> QuadOutput {
    var corners = array<vec2<f32>, 6>(vec2<f32>(0.0,0.0), vec2<f32>(1.0,0.0), vec2<f32>(0.0,1.0), vec2<f32>(0.0,1.0), vec2<f32>(1.0,0.0), vec2<f32>(1.0,1.0));
    let pixel = input.rect.xy + corners[index] * input.rect.zw;
    let clip = vec2<f32>(pixel.x / screen.size.x * 2.0 - 1.0, 1.0 - pixel.y / screen.size.y * 2.0);
    var output: QuadOutput;
    output.position = vec4<f32>(clip, 0.0, 1.0);
    output.color = input.color;
    return output;
}
@fragment
fn fs_quad(input: QuadOutput) -> @location(0) vec4<f32> { return input.color; }
