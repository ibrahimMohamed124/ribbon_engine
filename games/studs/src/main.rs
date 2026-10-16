use std::error::Error;

use voxelworks_render_cpu::renderer::CpuRenderer;
extern crate studs as studs_lib;

use studs_lib::app::StudsApp;
use studs_lib::surfaces::StudsSurface;

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
        "auto" => voxelworks_platform::runner::RendererMode::Auto,
        "cpu" => voxelworks_platform::runner::RendererMode::Cpu,
        "gpu" => voxelworks_platform::runner::RendererMode::Gpu,
        _ => return Err(format!("unknown renderer '{selected}'; use auto, cpu, or gpu").into()),
    };
    voxelworks_platform::runner::run_with_mode(app, "STUDS", window_config, renderer, mode)
}
