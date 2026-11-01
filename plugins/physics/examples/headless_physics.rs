use std::time::Duration;

use glam::Vec3;
use ribbon_app::{App, Time};
use ribbon_plugin_physics::{raycast, Collider, PhysicsPlugin, RaycastQuery, RigidBody};
use ribbon_plugins::AppPluginExt;
use ribbon_scene::{ScenePlugin, Transform3D};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = App::new();
    app.add_plugin(ScenePlugin)?;
    app.add_plugin(PhysicsPlugin)?;

    app.world.spawn((
        Transform3D::from_translation(Vec3::new(0.0, -0.5, 0.0)),
        Collider::cuboid(Vec3::new(5.0, 0.5, 5.0)),
    ));
    let ball = app.world.spawn((
        Transform3D::from_translation(Vec3::new(0.0, 3.0, 0.0)),
        RigidBody::dynamic(),
        Collider::ball(0.5),
    ));

    // Headless driving uses the same engine time resource and fixed-update lifecycle as a runner.
    for _ in 0..120 {
        app.world.resource_mut::<Time>().delta = Duration::from_secs_f64(1.0 / 60.0);
        app.update();
    }

    let query = RaycastQuery::new(Vec3::new(0.0, 4.0, 0.0), -Vec3::Y, 10.0);
    if let Some(hit) = raycast(&app.world, query) {
        println!(
            "ray hit {:?} at {:?}, distance {:.3}",
            hit.entity, hit.position, hit.distance
        );
    }
    println!("dynamic body entity: {:?}", ball);
    Ok(())
}
