use engine_app::{App, Time};
use engine_editor::{EditorPlugin, EditorShell};
use engine_platform::PlatformPlugin;
use engine_plugins::AppPluginExt;
use engine_scene::ScenePlugin;
use plugin_editor_tools::EditorToolsPlugin;
use plugin_rendering::ecs::{RenderBackend, RenderSettings, RenderingPlugin};
use std::error::Error;
use std::time::Duration;

fn main() -> Result<(), Box<dyn Error>> {
    let mut app = App::new();
    app.insert_resource(RenderSettings {
        backend: RenderBackend::Cpu,
        width: 640,
        height: 360,
    });
    app.add_plugin(PlatformPlugin)?;
    app.add_plugin(ScenePlugin)?;
    app.add_plugin(RenderingPlugin)?;
    app.add_plugin(EditorPlugin)?;
    app.add_plugin(EditorToolsPlugin)?;
    app.world.resource_mut::<EditorShell>().visible = true;
    let mut app = app.build()?;
    app.world.resource_mut::<Time>().delta = Duration::from_secs_f64(1.0 / 60.0);
    app.update();
    let draw_commands = app
        .world
        .try_resource::<engine_render::DrawList>()
        .map_or(0, |list| list.commands.len());
    println!("editor_demo UI commands: {draw_commands}");
    Ok(())
}
