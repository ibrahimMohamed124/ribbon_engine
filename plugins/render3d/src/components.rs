//! ECS render components. World space is right-handed and Y-up; a camera looks
//! down its local -Z axis (the glTF convention). `Transform3D` is re-exported
//! from `engine_scene` so rendering consumes the shared scene transform model.

use engine_assets::Handle;
pub use engine_scene::Transform3D;
use glam::{Mat4, Vec3};
use plugin_gltf::ModelAsset;

/// Perspective camera. The first entity with a `Camera3D` and local/global 3D transforms is used.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera3D {
    /// Vertical field of view in radians.
    pub fov_y: f32,
    /// Near clip distance.
    pub near: f32,
    /// Far clip distance.
    pub far: f32,
}

impl Default for Camera3D {
    fn default() -> Self {
        Self {
            fov_y: 60_f32.to_radians(),
            near: 0.1,
            far: 200.0,
        }
    }
}

impl Camera3D {
    /// Projection matrix with a 0..1 depth range.
    pub fn projection(&self, aspect: f32) -> Mat4 {
        Mat4::perspective_rh(self.fov_y, aspect, self.near, self.far)
    }
}

/// Meshes built into the renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinMesh {
    /// Unit cube centered on the origin.
    Cube,
}

/// Draw `mesh` with a flat base color (linear RGB) at the entity's transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshInstance {
    /// Mesh to draw.
    pub mesh: BuiltinMesh,
    /// Linear RGB base color.
    pub color: [f32; 3],
}

/// Draw every primitive of a loaded glTF model at the entity's transform.
///
/// The model is drawn once its asset has finished loading in the `AssetServer`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelInstance {
    /// Handle of the model in the asset server.
    pub model: Handle<ModelAsset>,
}

/// Directional light plus constant ambient term.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunLight {
    /// Direction the light travels (from the sun toward the scene).
    pub direction: Vec3,
    /// Linear RGB light color.
    pub color: Vec3,
    /// Linear RGB ambient light.
    pub ambient: Vec3,
}

impl Default for SunLight {
    fn default() -> Self {
        Self {
            direction: Vec3::new(-0.4, -0.8, -0.45).normalize(),
            color: Vec3::new(1.0, 0.96, 0.9),
            ambient: Vec3::splat(0.12),
        }
    }
}
