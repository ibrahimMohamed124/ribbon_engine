use std::error::Error;

use render_cpu::renderer::CpuRenderer;
use studs::app::StudsApp;
use studs::surfaces::StudsSurface;

fn main() -> Result<(), Box<dyn Error>> {
    let app = StudsApp::new()?;
    let window_config = app.window_config();
    let renderer = CpuRenderer::new(
        window_config.width as u32,
        window_config.height as u32,
        StudsSurface,
    );
    let mut selected = std::env::var("STUDS_RENDERER").unwrap_or_else(|_| "auto".to_owned());
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--renderer" {
            selected = args.next().ok_or("--renderer expects auto, cpu, or gpu")?;
        }
    }
    let mode = match selected.as_str() {
        "auto" => platform::runner::RendererMode::Auto,
        "cpu" => platform::runner::RendererMode::Cpu,
        "gpu" => platform::runner::RendererMode::Gpu,
        _ => return Err(format!("unknown renderer '{selected}'; use auto, cpu, or gpu").into()),
    };
    platform::runner::run_with_mode(app, "STUDS", window_config, renderer, mode)
}
