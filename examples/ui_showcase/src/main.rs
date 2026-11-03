use ribbon_app::{App, AppExit, NextState, Stage, State, Time, World};
use ribbon_events::WorldEventsExt;
use ribbon_plugin_render3d::{BuiltinMesh, Camera3D, MeshInstance, Transform3D};
use ribbon_plugins::AppPluginExt;
use ribbon_ui::{ui_scope, Anchor, PanelSize, UiPlugin};
use std::error::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Screen {
    Main,
    Settings,
}
struct Spin;

fn update_showcase(world: &mut World) {
    let settings = world.resource::<State<Screen>>().get() == Screen::Settings;
    let progress = (world.resource::<Time>().elapsed.as_secs_f32() * 0.12).fract();
    let mut go_settings = false;
    let mut go_main = false;
    let mut quit = false;
    ui_scope(world, |ui| {
        ui.panel("Showcase", Anchor::Center, PanelSize::Width(460.0), |ui| {
            if settings {
                ui.heading("Settings");
                ui.label("مرحبا بك في أثر");
                ui.label("Player settings | إعدادات اللاعب");
                let mut sound = true;
                ui.checkbox(&mut sound, "Sound enabled");
                let mut volume = 0.7;
                ui.slider(&mut volume, 0.0..=1.0, "Volume");
                let mut name = String::from("Player");
                ui.text_input(&mut name, "Player name");
                go_main = ui.button("Back").clicked;
            } else {
                ui.heading("Ribbon UI");
                ui.label("مرحبا بك في أثر");
                ui.button("Play");
                go_settings = ui.button("Settings").clicked;
                quit = ui.button("Quit").clicked;
                ui.progress_bar(progress, Some("Loading"));
            }
        });
    });
    if go_settings {
        world
            .resource_mut::<NextState<Screen>>()
            .set(Screen::Settings);
    }
    if go_main {
        world.resource_mut::<NextState<Screen>>().set(Screen::Main);
    }
    if quit {
        world.send_event(AppExit);
    }
}

fn spin(world: &mut World) {
    let dt = world.resource::<Time>().delta.as_secs_f32();
    for (_, (transform, _spin)) in world.query_mut::<(&mut Transform3D, &Spin)>() {
        transform.rotation =
            ribbon_plugin_render3d::glam::Quat::from_rotation_y(0.7 * dt) * transform.rotation;
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut app = App::new();
    app.add_plugin(ribbon_platform::PlatformPlugin)?;
    app.add_plugin(ribbon_scene::ScenePlugin)?;
    app.add_plugin(ribbon_render::gpu_window::GpuWindowPlugin)?;
    app.add_plugin(ribbon_plugin_render3d::Render3dPlugin)?;
    app.add_plugin(UiPlugin::new())?;
    app.init_state::<Screen>(Screen::Main);
    app.world.spawn((
        Transform3D::looking_at(
            ribbon_plugin_render3d::glam::Vec3::new(3.5, 2.5, 5.0),
            ribbon_plugin_render3d::glam::Vec3::new(0.0, 0.2, 0.0),
            ribbon_plugin_render3d::glam::Vec3::Y,
        ),
        Camera3D::default(),
    ));
    app.world.spawn((
        Transform3D::default(),
        MeshInstance {
            mesh: BuiltinMesh::Cube,
            color: [0.9, 0.45, 0.15],
        },
        Spin,
    ));
    app.add_system(Stage::Update, spin);
    app.add_system(Stage::Update, update_showcase);
    app.build()?.run();
    Ok(())
}
