use ribbon_app::{App, Stage, Time};
use ribbon_plugins::Plugin;
use ribbon_reflect::TypeRegistry;
use ribbon_scene::PrefabCodecRegistry;

use crate::config::PhysicsConfig;
use crate::events::{CollisionStarted, CollisionStopped, SensorEntered, SensorExited};
use crate::runtime::{PhysicsContext, PhysicsDebugData, PhysicsDiagnostics};

/// Installs a Rapier-backed 3D physics world.
///
/// Add [`ribbon_scene::ScenePlugin`] before this plugin. The engine's `FixedTime` accumulator
/// drives `Stage::FixedUpdate`; this plugin does not maintain a second clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct PhysicsPlugin;

impl Plugin for PhysicsPlugin {
    fn name(&self) -> &'static str {
        "PhysicsPlugin"
    }

    fn dependencies(&self) -> &'static [&'static str] {
        &["ScenePlugin"]
    }

    fn build(&self, app: &mut App) {
        // Physics codecs share the scene registry with render3d codecs. Preserve any registry
        // already installed by another plugin so plugin registration order cannot erase codecs.
        let mut prefab_codecs = app
            .world
            .remove_resource::<PrefabCodecRegistry>()
            .unwrap_or_default();
        let registration = crate::prefab::register_prefab_components(
            app.world.resource_mut::<TypeRegistry>(),
            &mut prefab_codecs,
        );
        match registration {
            Ok(()) => {
                app.insert_resource(prefab_codecs);
            }
            Err(error) => {
                app.insert_resource(prefab_codecs);
                app.world.resource_mut::<ribbon_app::LogBuffer>().push(
                    ribbon_app::LogLevel::Error,
                    format!("physics prefab reflection registration failed: {error}"),
                );
            }
        }
        if app.world.try_resource::<PhysicsConfig>().is_none() {
            app.world.insert_resource(PhysicsConfig::default());
        }
        if app.world.try_resource::<PhysicsDiagnostics>().is_none() {
            app.world.insert_resource(PhysicsDiagnostics::default());
        }
        if app.world.try_resource::<PhysicsDebugData>().is_none() {
            app.world.insert_resource(PhysicsDebugData::default());
        }
        if app.world.try_resource::<PhysicsContext>().is_none() {
            app.world.insert_resource(PhysicsContext::default());
        }
        app.add_event::<CollisionStarted>();
        app.add_event::<CollisionStopped>();
        app.add_event::<SensorEntered>();
        app.add_event::<SensorExited>();

        app.add_system(Stage::PreUpdate, |world| {
            world.resource_scope::<PhysicsContext, _>(|world, context| {
                context.reconcile(world);
            });
        });
        app.add_system(Stage::FixedUpdate, |world| {
            let delta_seconds = world.resource::<Time>().delta.as_secs_f32();
            world.resource_scope::<PhysicsContext, _>(|world, context| {
                context.step(world, delta_seconds);
            });
        });
    }
}
