//! Hierarchy, transforms, and serializable prefab scene documents.
//!
//! Render-specific cameras/materials remain in `plugin_rendering`; game-specific data remains in games.

mod hierarchy;
mod prefab;
mod transform;

pub use hierarchy::{propagate_transforms, HierarchyError, HierarchyExt};
pub use prefab::{
    capture, instantiate, instantiate_lenient, Prefab, PrefabEntity, SceneDoc, SceneError,
};
pub use transform::{Children, GlobalTransform, Name, Parent, Transform};

use engine_app::{App, Stage};
use engine_plugins::Plugin;
use engine_reflect::TypeRegistry;

/// Registers scene components and their post-update transform propagation.
#[derive(Default)]
pub struct ScenePlugin;

impl Plugin for ScenePlugin {
    fn name(&self) -> &'static str {
        "ScenePlugin"
    }

    fn build(&self, app: &mut App) {
        let mut registry = TypeRegistry::default();
        let registration = registry
            .register::<Name>()
            .and_then(|_| registry.register::<Transform>())
            .and_then(|_| registry.register::<GlobalTransform>())
            .and_then(|_| registry.register::<Parent>())
            .and_then(|_| registry.register::<Children>());
        if let Err(error) = registration {
            app.world
                .resource_mut::<engine_app::LogBuffer>()
                .push(engine_app::LogLevel::Error, error.to_string());
            return;
        }
        app.world.insert_resource(registry);
        app.add_system(Stage::PostUpdate, propagate_transforms);
    }
}
