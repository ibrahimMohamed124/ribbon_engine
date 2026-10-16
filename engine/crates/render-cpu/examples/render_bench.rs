#[path = "shared/mod.rs"]
mod shared;

use std::time::Instant;

use shared::{graded_style, Scene, SceneShader, HEIGHT, WIDTH};
use voxelworks_api::Framebuffer;
use voxelworks_render_cpu::render::{render, RenderStyle};

fn main() {
    let frames = std::env::args()
        .nth(1)
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|&value| value > 0)
        .unwrap_or(30);
    let scene = Scene::seeded(0x5eed_cafe);
    let shader = SceneShader::default();
    let viewpoint = scene.viewpoints()[0];
    let mut framebuffer = Framebuffer::new(WIDTH, HEIGHT);
    let baseline = measure(
        &scene,
        &shader,
        &viewpoint,
        RenderStyle::default(),
        frames,
        &mut framebuffer,
    );
    let graded = measure(
        &scene,
        &shader,
        &viewpoint,
        graded_style(),
        frames,
        &mut framebuffer,
    );
    println!(
        "scene: {}; resolution: {WIDTH}x{HEIGHT}; frames per style: {frames}",
        viewpoint.name
    );
    println!("baseline: {baseline:.3} ms/frame");
    println!("graded:   {graded:.3} ms/frame");
    println!("overhead: {:+.1}%", (graded / baseline - 1.0) * 100.0);
}

fn measure(
    scene: &Scene,
    shader: &SceneShader,
    viewpoint: &shared::Viewpoint,
    style: RenderStyle,
    frames: u32,
    framebuffer: &mut Framebuffer,
) -> f64 {
    for frame_no in 0..3 {
        let frame = scene.frame(viewpoint, style, frame_no);
        render(framebuffer, &frame, shader);
    }
    let start = Instant::now();
    for frame_no in 0..frames {
        let frame = scene.frame(viewpoint, style, frame_no);
        render(framebuffer, &frame, shader);
    }
    start.elapsed().as_secs_f64() * 1000.0 / frames as f64
}
