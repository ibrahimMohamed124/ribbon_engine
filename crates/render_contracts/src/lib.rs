//! Renderer-neutral component and extraction contracts shared by render plugins and systems.
//!
//! This crate contains renderer-neutral component data and reflection metadata; GPU pipelines and
//! glTF asset instance types remain in the renderer plugin.

use glam::{Mat4, Vec3};

pub mod particles;
mod reflection;
pub use particles::{
    ParticleBatch, ParticleBillboard, ParticleBlendMode, ParticleInstance, ParticleRenderData,
    ParticleShape,
};
pub use ribbon_scene::{GlobalTransform3D, Transform3D};

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

impl Default for MeshInstance {
    fn default() -> Self {
        Self {
            mesh: BuiltinMesh::Cube,
            color: [1.0, 1.0, 1.0],
        }
    }
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

/// A world-space cone light. The direction points from the light into the scene.
/// At most one light with `casts_shadows` is rendered with a perspective shadow map.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpotLight {
    pub position: Vec3,
    pub direction: Vec3,
    pub color: Vec3,
    /// Radiometric intensity multiplier in linear space.
    pub intensity: f32,
    /// Maximum useful distance in world units.
    pub range: f32,
    /// Full-strength cone half-angle, in radians.
    pub inner_cone_angle: f32,
    /// Maximum cone half-angle, in radians.
    pub outer_cone_angle: f32,
    pub casts_shadows: bool,
}

impl Default for SpotLight {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            direction: Vec3::NEG_Z,
            color: Vec3::splat(1.0),
            intensity: 18.0,
            range: 12.0,
            inner_cone_angle: 22.0_f32.to_radians(),
            outer_cone_angle: 32.0_f32.to_radians(),
            casts_shadows: true,
        }
    }
}

/// A non-shadowed point light. The native renderer deterministically ranks extracted lights and
/// evaluates at most eight records in its current shader uniform layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointLight {
    pub position: Vec3,
    pub color: Vec3,
    pub intensity: f32,
    pub range: f32,
}

impl Default for PointLight {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            color: Vec3::splat(1.0),
            intensity: 2.0,
            range: 5.0,
        }
    }
}

/// Exponential distance/height fog parameters. `density = 0` disables fog.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FogSettings {
    pub color: Vec3,
    pub density: f32,
    pub base_height: f32,
    pub height_falloff: f32,
}

impl Default for FogSettings {
    fn default() -> Self {
        Self {
            color: Vec3::new(0.025, 0.035, 0.032),
            density: 0.0,
            base_height: 0.0,
            height_falloff: 0.0,
        }
    }
}

/// Camera and resolved world pose selected for one render frame.
#[derive(Clone, Copy, Debug)]
pub struct ExtractedCamera3D {
    /// Resolved camera world transform (including any parent transforms).
    pub transform: GlobalTransform3D,
    /// Projection settings for the camera.
    pub camera: Camera3D,
}

/// One built-in mesh draw extracted from an ECS entity.
#[derive(Clone, Copy, Debug)]
pub struct ExtractedMesh3D {
    /// Resolved object-to-world matrix.
    pub transform: Mat4,
    /// Linear RGB instance tint.
    pub color: [f32; 3],
}
