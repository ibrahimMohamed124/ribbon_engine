use std::error::Error;

use engine_render::DrawList;
use plugin_rendering::material::MaterialTable;
use studs::game::Game;
use studs::surfaces::{studs_material_table, StudsSurface};
use voxelworks_api::{App, FramePlan};
use voxelworks_api::{Ctx, Flow, WindowConfig};
use voxelworks_render_cpu::renderer::CpuRenderer;

struct Preview(Game);

impl App for Preview {
    fn update(&mut self, _ctx: &mut Ctx<'_>) -> Flow {
        Flow::Continue
    }

    fn frame_plan(&self, ctx: &Ctx<'_>) -> FramePlan<'_> {
        let (width, height) = (320, 180);
        FramePlan {
            scene: self
                .0
                .scene_styled(ctx.frame, 1.1, studs::styles::realistic()),
            world_layer: DrawList::new(width, height),
            brightness: 1.0,
            ui_layer: DrawList::new(width, height),
        }
    }

    fn materials(&self) -> &'static MaterialTable {
        studs_material_table()
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let config = WindowConfig {
        title: "STUDS GPU preview".into(),
        width: 320,
        height: 180,
        scale: 3,
        fixed_hz: 60,
        max_steps: 2,
    };
    let renderer = CpuRenderer::new(320, 180, StudsSurface);
    let mode = match std::env::args().nth(1).as_deref() {
        Some("cpu") => voxelworks_platform::RendererMode::Cpu,
        Some("gpu") | None => voxelworks_platform::RendererMode::Gpu,
        Some(other) => return Err(format!("expected cpu or gpu; got {other}").into()),
    };
    voxelworks_platform::run_with_mode(
        Preview(Game::new(42)),
        "STUDS Preview",
        config,
        renderer,
        mode,
    )
}
