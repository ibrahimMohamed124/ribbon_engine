//! Compatibility module preserving the public renderer component paths.

use crate::gpu_streaming::{
    GpuResourceHandle, GpuResourceLease, GpuResourceWarmup, GpuResourceWarmupStatus,
};
use crate::mesh::MeshData;
use crate::renderer::{MaterialId, MeshId};
use glam::Mat4;
use ribbon_assets::Handle;
use ribbon_plugin_gltf::ModelAsset;
pub use ribbon_render_contracts::{
    BuiltinMesh, Camera3D, FogSettings, MeshInstance, PointLight, SpotLight, SunLight, Transform3D,
};
use std::sync::Arc;

/// Marker for render entities that have been spawned as part of a streaming activation but have
/// not reached their region's visibility commit yet. The render extraction pass omits marked
/// entities; physics and other ECS systems continue to observe them normally.
///
/// Streaming removes this marker from every region-owned visual entity in one Update once all
/// required visuals and colliders are ready. Applications may also use it to stage their own
/// renderer-facing entity groups.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StagedRender;

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

/// Instance of independently warmed GPU mesh/material resources.
///
/// The component owns both warmup leases. Despawning the entity releases those claims, and the
/// renderer drops the GPU objects on its owning thread after the final extracted-frame lease is
/// gone. `Transform3D` remains a separate ECS component, just like other 3D render instances.
#[derive(Clone, Debug)]
pub struct GpuMeshInstance3D {
    /// Renderer-owned mesh handle.
    pub mesh: MeshId,
    /// Renderer-owned PBR material handle.
    pub material: MaterialId,
    /// Linear RGB tint multiplier.
    pub color: [f32; 3],
    pub(crate) _mesh_lease: GpuResourceLease,
    pub(crate) _material_lease: GpuResourceLease,
}

impl GpuMeshInstance3D {
    /// Build an ECS instance only after both generic GPU warmup requests passed asynchronous
    /// renderer validation. The instance clones the leases so caller-side handles may be dropped.
    pub fn from_ready_leases(
        warmup: &GpuResourceWarmup,
        mesh_lease: &GpuResourceLease,
        material_lease: &GpuResourceLease,
        color: [f32; 3],
    ) -> Result<Self, String> {
        let mesh = match warmup.status(mesh_lease) {
            Some(GpuResourceWarmupStatus::Ready(GpuResourceHandle::Mesh(mesh))) => mesh,
            Some(GpuResourceWarmupStatus::Failed(error)) => {
                return Err(format!("mesh GPU warmup failed: {error}"));
            }
            Some(status) => return Err(format!("mesh GPU resource is not ready: {status:?}")),
            None => return Err("mesh GPU warmup lease is no longer registered".to_owned()),
        };
        let material = match warmup.status(material_lease) {
            Some(GpuResourceWarmupStatus::Ready(GpuResourceHandle::Material(material))) => material,
            Some(GpuResourceWarmupStatus::Failed(error)) => {
                return Err(format!("material GPU warmup failed: {error}"));
            }
            Some(status) => return Err(format!("material GPU resource is not ready: {status:?}")),
            None => return Err("material GPU warmup lease is no longer registered".to_owned()),
        };
        Ok(Self {
            mesh,
            material,
            color,
            _mesh_lease: mesh_lease.clone(),
            _material_lease: material_lease.clone(),
        })
    }
}

/// Draw every primitive of a loaded glTF model at the entity's transform.
///
/// The model is drawn once its asset has finished loading in the `AssetServer`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModelInstance {
    /// Handle of the model in the asset server.
    pub model: Handle<ModelAsset>,
}

/// Evaluated per-model-node skin palettes produced by `AnimationPlugin`.
///
/// `node_palettes[i]` corresponds to `ModelAsset::nodes[i]`; a skinned placement contains
/// model-local joint matrices (mesh-node inverse * joint-global * inverse-bind). Palettes are
/// immutable and shared with extracted render snapshots so the GPU upload sees a stable pose.
#[derive(Clone, Debug)]
pub struct SkeletalPose {
    pub model_asset_id: u64,
    pub model_generation: u32,
    /// Animated model-space global transforms indexed by original glTF node index.
    pub node_global_transforms: Arc<[Mat4]>,
    /// Mesh-local joint palettes indexed by `ModelAsset::nodes` placement index.
    pub node_palettes: Arc<[Option<Arc<[Mat4]>>]>,
}

impl Default for SkeletalPose {
    fn default() -> Self {
        Self {
            model_asset_id: 0,
            model_generation: 0,
            node_global_transforms: Arc::<[Mat4]>::from(Vec::new().into_boxed_slice()),
            node_palettes: Arc::<[Option<Arc<[Mat4]>>]>::from(Vec::new().into_boxed_slice()),
        }
    }
}
