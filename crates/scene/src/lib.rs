//! 2D and 3D transforms, shared hierarchy, and serializable prefab scene documents.
//!
//! Render-specific cameras/materials remain in rendering plugins; game-specific data remains in games.

mod hierarchy;
mod hierarchy_components;
mod prefab;
mod transform;

pub use hierarchy::{
    propagate_transforms, propagate_transforms_3d, HierarchyError, HierarchyExt,
    TransformPropagationDiagnostics,
};
pub use hierarchy_components::{Children, Parent};
pub use prefab::{
    capture, capture_with_context, instantiate, instantiate_lenient, instantiate_with_context,
    Prefab, PrefabCodecRegistry, PrefabEntity, PrefabLoadContext, SceneDoc, SceneError,
};
pub use transform::{GlobalTransform, GlobalTransform3D, Name, Transform, Transform3D};

use ribbon_app::{App, Stage};
use ribbon_plugins::Plugin;
use ribbon_reflect::TypeRegistry;

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
            .and_then(|_| registry.register::<Transform3D>())
            .and_then(|_| registry.register::<GlobalTransform3D>())
            .and_then(|_| registry.register::<Parent>())
            .and_then(|_| registry.register::<Children>());
        if let Err(error) = registration {
            app.world
                .resource_mut::<ribbon_app::LogBuffer>()
                .push(ribbon_app::LogLevel::Error, error.to_string());
            return;
        }
        app.world.insert_resource(registry);
        app.world
            .insert_resource(TransformPropagationDiagnostics::default());
        app.add_system(Stage::PostUpdate, propagate_transforms);
        app.add_system(Stage::PostUpdate, propagate_transforms_3d);
    }
}
