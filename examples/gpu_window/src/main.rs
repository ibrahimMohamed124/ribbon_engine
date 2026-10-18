//! Opens a window and presents an animated clear color through wgpu.
//!
//! Usage: `cargo run -p gpu_window [-- --frames N]`. Without `--frames` the
//! window stays open until it is closed.

use engine_app::{App, AppExit, Stage, World};
use engine_events::WorldEventsExt;
use engine_platform::{PlatformConfig, PlatformPlugin};
use engine_plugins::AppPluginExt;
use engine_render::gpu_window::{ClearColor, GpuWindowPlugin};
use std::error::Error;

struct FrameCounter {
    frames: u64,
    limit: Option<u64>,
}

fn animate(world: &mut World) {
    let (frame, limit) = {
        let counter = world.resource_mut::<FrameCounter>();
        counter.frames += 1;
        (counter.frames, counter.limit)
    };
    let phase = frame as f64 * 0.02;
    world.resource_mut::<ClearColor>().0 = [
        0.05 + 0.04 * phase.sin().abs(),
        0.05 + 0.04 * (phase * 0.7).sin().abs(),
        0.08 + 0.06 * (phase * 0.4).sin().abs(),
        1.0,
    ];
    if limit.is_some_and(|limit| frame >= limit) {
        world.send_event(AppExit);
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut limit = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--frames" => {
                limit = Some(
                    args.next()
                        .ok_or("--frames expects a number")?
                        .parse::<u64>()?,
                );
            }
            other => return Err(format!("unknown argument `{other}`").into()),
        }
    }
    let mut app = App::new();
    app.insert_resource(PlatformConfig {
        title: "Voxelworks GPU window".to_owned(),
        width: 960,
        height: 540,
    });
    app.insert_resource(FrameCounter { frames: 0, limit });
    app.add_plugin(PlatformPlugin)?;
    app.add_plugin(GpuWindowPlugin)?;
    app.add_system(Stage::Update, animate);
    app.build()?.run();
    Ok(())
}
