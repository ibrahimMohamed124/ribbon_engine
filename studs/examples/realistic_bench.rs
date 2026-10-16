use std::time::Instant;

use engine_core::gfx::Framebuffer;
use raycast_blocks::game::Game;
use raycast_blocks::renderer::{RenderStyle, DEFAULT_HFOV};
use raycast_blocks::surfaces::{studs_material_table, StudsSurface};
use render_cpu::render::render_with_materials;

fn main() {
    let frames = std::env::args()
        .nth(1)
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|&count| count > 0)
        .unwrap_or(30);
    let game = Game::new(42);
    let surface = StudsSurface;
    let materials = studs_material_table();
    let mut framebuffer = Framebuffer::new(320, 180);
    for (name, style) in [
        ("classic", RenderStyle::classic()),
        ("moody", RenderStyle::moody()),
        ("realistic", RenderStyle::realistic()),
    ] {
        for frame in 0..3 {
            let scene = game.scene_styled(frame, DEFAULT_HFOV, style).unwrap();
            render_with_materials(&mut framebuffer, &scene, &surface, materials);
        }
        let start = Instant::now();
        for frame in 0..frames {
            let scene = game.scene_styled(frame, DEFAULT_HFOV, style).unwrap();
            render_with_materials(&mut framebuffer, &scene, &surface, materials);
        }
        println!(
            "{name}: {:.3} ms/frame at 320x180",
            start.elapsed().as_secs_f64() * 1000.0 / frames as f64
        );
    }
}
