//! ECS-to-render extraction for the 3D mesh renderer.
//!
//! Extraction runs after `ScenePlugin` resolves `GlobalTransform3D` and before the
//! `GpuWindowPlugin` renders its frame. GPU passes consume this compact snapshot
//! instead of querying gameplay ECS data while recording GPU commands.

use crate::components::{
    Camera3D, FogSettings, MeshInstance, ModelInstance, PointLight, SpotLight, SunLight,
    VoxelMeshInstance3D,
};
use engine_app::{App, Stage, World};
use engine_assets::Handle;
use engine_plugins::Plugin;
use engine_scene::GlobalTransform3D;
use glam::Mat4;
use plugin_gltf::ModelAsset;
use plugin_voxel::VoxelChunkMesh;
use std::sync::Arc;

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

/// One glTF model placement extracted from an ECS entity.
#[derive(Clone, Copy, Debug)]
pub struct ExtractedModel3D {
    /// Resolved object-to-world matrix.
    pub transform: Mat4,
    /// Weak handle resolved against `AssetServer` during rendering.
    pub model: Handle<ModelAsset>,
}

/// One immutable greedy-meshed voxel chunk ready for GPU upload.
#[derive(Clone, Debug)]
pub struct ExtractedVoxelMesh3D {
    /// Resolved chunk-to-world transform including the voxel-grid origin.
    pub transform: Mat4,
    /// Shared CPU mesh; GPU upload is cached by `Render3dPlugin`.
    pub mesh: Arc<VoxelChunkMesh>,
    /// Linear RGB tint for the block id represented by this mesh.
    pub color: [f32; 3],
}

/// Compact render-facing scene snapshot produced from the ECS each frame.
///
/// This contains copied transforms and asset handles rather than references into
/// the ECS. The vectors are reused between frames to avoid repeated allocations.
#[derive(Default)]
pub struct ExtractedScene3D {
    /// First camera with a resolved global transform, if any.
    pub camera: Option<ExtractedCamera3D>,
    /// Built-in mesh instances.
    pub meshes: Vec<ExtractedMesh3D>,
    /// Loaded or pending glTF model instances.
    pub models: Vec<ExtractedModel3D>,
    /// Greedy-meshed voxel instances rendered by the same native GPU mesh pipeline.
    pub voxels: Vec<ExtractedVoxelMesh3D>,
    /// Directional sunlight and ambient contribution for this frame.
    pub sun: SunLight,
    /// World-space spotlight components for the native mesh renderer.
    pub spot_lights: Vec<SpotLight>,
    /// World-space point lights (only eight are consumed by the renderer).
    pub point_lights: Vec<PointLight>,
    /// Exponential atmosphere settings.
    pub fog: FogSettings,
    /// App frame counter used by tools to correlate diagnostics with a frame.
    pub frame: u64,
}

/// Extract camera, transform, mesh, model and light data from the live ECS world.
pub fn extract_scene_3d(world: &mut World) {
    let frame = world.resource::<engine_app::Time>().frame;
    let sun = world
        .try_resource::<SunLight>()
        .copied()
        .unwrap_or_default();

    world.resource_scope::<ExtractedScene3D, _>(|world, extracted| {
        extracted.camera = world
            .query::<(&GlobalTransform3D, &Camera3D)>()
            .iter()
            .next()
            .map(|(_, (transform, camera))| ExtractedCamera3D {
                transform: *transform,
                camera: *camera,
            });

        extracted.meshes.clear();
        for (_, (transform, mesh)) in world.query::<(&GlobalTransform3D, &MeshInstance)>().iter() {
            extracted.meshes.push(ExtractedMesh3D {
                transform: transform.matrix(),
                color: mesh.color,
            });
        }

        extracted.models.clear();
        for (_, (transform, model)) in world.query::<(&GlobalTransform3D, &ModelInstance)>().iter()
        {
            extracted.models.push(ExtractedModel3D {
                transform: transform.matrix(),
                model: model.model,
            });
        }

        extracted.voxels.clear();
        for (_, (transform, voxel)) in world
            .query::<(&GlobalTransform3D, &VoxelMeshInstance3D)>()
            .iter()
        {
            let origin = glam::Vec3::from(voxel.mesh.origin_y_up);
            extracted.voxels.push(ExtractedVoxelMesh3D {
                transform: transform.matrix() * Mat4::from_translation(origin),
                mesh: Arc::clone(&voxel.mesh),
                color: voxel.color,
            });
        }

        extracted.spot_lights.clear();
        extracted
            .spot_lights
            .extend(world.query::<&SpotLight>().iter().map(|(_, light)| *light));
        extracted.point_lights.clear();
        extracted
            .point_lights
            .extend(world.query::<&PointLight>().iter().map(|(_, light)| *light));
        extracted.sun = sun;
        extracted.fog = world
            .try_resource::<FogSettings>()
            .copied()
            .unwrap_or_default();
        extracted.frame = frame;
    });
}

/// Internal marker so applications can install the public extraction plugin together
/// with `Render3dPlugin` without registering the same system twice.
struct ExtractionInstalled;

/// Registers the extraction system after the scene plugin's transform propagation.
pub(crate) fn install_extraction(app: &mut App) {
    if app.world.try_resource::<ExtractionInstalled>().is_some() {
        return;
    }
    if app.world.try_resource::<ExtractedScene3D>().is_none() {
        app.insert_resource(ExtractedScene3D::default());
    }
    app.insert_resource(ExtractionInstalled);
    app.add_system(Stage::PostUpdate, extract_scene_3d);
}

/// Plugin that can be used by applications wanting only the data extraction API.
/// Most applications should add [`crate::Render3dPlugin`], which installs this system.
pub struct RenderExtraction3DPlugin;

impl Plugin for RenderExtraction3DPlugin {
    fn name(&self) -> &'static str {
        "RenderExtraction3DPlugin"
    }

    fn dependencies(&self) -> &'static [&'static str] {
        &["ScenePlugin"]
    }

    fn build(&self, app: &mut App) {
        install_extraction(app);
    }
}

#[cfg(test)]
mod tests {
    use super::{extract_scene_3d, ExtractedScene3D};
    use crate::{Camera3D, FogSettings, MeshInstance, PointLight, SpotLight};
    use engine_app::App;
    use engine_scene::GlobalTransform3D;
    use glam::Vec3;

    #[test]
    fn extracts_spot_and_point_lights_and_fog_settings() {
        let mut app = App::new();
        app.world.insert_resource(ExtractedScene3D::default());
        let spot = SpotLight {
            position: Vec3::new(1.0, 2.0, 3.0),
            direction: Vec3::new(0.0, -0.2, -1.0),
            intensity: 42.0,
            casts_shadows: true,
            ..SpotLight::default()
        };
        let point = PointLight {
            position: Vec3::new(-1.0, 0.5, 2.0),
            intensity: 4.5,
            ..PointLight::default()
        };
        app.world.spawn((spot,));
        app.world.spawn((point,));
        app.world.insert_resource(FogSettings {
            color: Vec3::new(0.03, 0.04, 0.035),
            density: 0.08,
            base_height: -1.0,
            height_falloff: 0.2,
        });

        extract_scene_3d(&mut app.world);
        let extracted = app.world.resource::<ExtractedScene3D>();
        assert_eq!(extracted.spot_lights, vec![spot]);
        assert_eq!(extracted.point_lights, vec![point]);
        assert_eq!(extracted.fog.density, 0.08);
        assert_eq!(extracted.fog.base_height, -1.0);
    }

    #[test]
    fn extracts_world_space_camera_and_mesh_transforms() {
        let mut app = App::new();
        assert!(app
            .world
            .insert_resource(ExtractedScene3D::default())
            .is_none());
        let camera_transform = GlobalTransform3D {
            translation: Vec3::new(1.0, 2.0, 3.0),
            ..GlobalTransform3D::default()
        };
        app.world.spawn((camera_transform, Camera3D::default()));
        app.world.spawn((
            GlobalTransform3D {
                translation: Vec3::new(4.0, 5.0, 6.0),
                ..GlobalTransform3D::default()
            },
            MeshInstance {
                mesh: crate::BuiltinMesh::Cube,
                color: [0.25, 0.5, 0.75],
            },
        ));

        extract_scene_3d(&mut app.world);
        let extracted = app.world.resource::<ExtractedScene3D>();
        assert_eq!(
            extracted.camera.unwrap().transform.translation,
            Vec3::new(1.0, 2.0, 3.0)
        );
        assert_eq!(extracted.meshes.len(), 1);
        assert_eq!(extracted.meshes[0].color, [0.25, 0.5, 0.75]);
        assert_eq!(
            extracted.meshes[0].transform.w_axis.truncate(),
            Vec3::new(4.0, 5.0, 6.0)
        );
    }
}
