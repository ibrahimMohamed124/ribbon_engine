//! Compatibility module preserving the public renderer component paths.

use crate::mesh::MeshData;
use ribbon_assets::Handle;
use ribbon_plugin_gltf::ModelAsset;
pub use ribbon_render_contracts::{
    BuiltinMesh, Camera3D, FogSettings, MeshInstance, PointLight, SpotLight, SunLight, Transform3D,
};
use std::sync::Arc;

/// An instance of caller-owned indexed triangle geometry rendered by the native mesh pass.
///
/// Geometry is reference-counted so multiple entities can share one CPU mesh and the renderer
/// can retain its GPU upload until the last placement disappears. This type is renderer-generic;
/// it deliberately remains independent of game-specific mesh sources and asset formats.
#[derive(Clone)]
pub struct DynamicMeshInstance3D {
    /// Indexed triangle geometry in object-local coordinates.
    pub mesh: Arc<MeshData>,
    /// Linear RGB tint multiplier.
    pub color: [f32; 3],
}

/// Draw every primitive of a loaded glTF model at the entity's transform.
///
/// The model is drawn once its asset has finished loading in the `AssetServer`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModelInstance {
    /// Handle of the model in the asset server.
    pub model: Handle<ModelAsset>,
}
