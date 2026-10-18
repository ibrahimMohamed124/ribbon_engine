//! ECS components for the 3D renderer. World space is right-handed and Y-up;
//! a camera looks down its local -Z axis (the glTF convention).

use engine_assets::Handle;
use glam::{Mat4, Quat, Vec3};
use plugin_gltf::ModelAsset;

/// Position, orientation and scale of an entity in world space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform3D {
    /// World-space position.
    pub translation: Vec3,
    /// World-space orientation.
    pub rotation: Quat,
    /// Per-axis scale. Keep it uniform until lighting gets a normal matrix.
    pub scale: Vec3,
}

impl Default for Transform3D {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }
}

impl Transform3D {
    /// Identity transform moved to `translation`.
    pub fn from_translation(translation: Vec3) -> Self {
        Self {
            translation,
            ..Self::default()
        }
    }

    /// A transform at `eye` whose -Z axis points at `target`.
    pub fn looking_at(eye: Vec3, target: Vec3, up: Vec3) -> Self {
        let (_, rotation, translation) = Mat4::look_at_rh(eye, target, up)
            .inverse()
            .to_scale_rotation_translation();
        Self {
            translation,
            rotation,
            scale: Vec3::ONE,
        }
    }

    /// Local-to-world matrix.
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}

/// Perspective camera. The first entity with a `Camera3D` and a [`Transform3D`] is used.
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
