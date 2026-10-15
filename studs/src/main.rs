use std::error::Error;

use raycast_blocks::app::StudsApp;
use raycast_blocks::surfaces::StudsSurface;
use render_cpu::renderer::CpuRenderer;

fn main() -> Result<(), Box<dyn Error>> {
    let app = StudsApp::new()?;
    let window_config = app.window_config();
    let renderer = CpuRenderer::new(
        window_config.width as u32,
        window_config.height as u32,
        StudsSurface,
    );
    platform::run(app, window_config, renderer)
}
