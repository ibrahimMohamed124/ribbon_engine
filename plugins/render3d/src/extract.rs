//! ECS-to-render extraction for the 3D mesh renderer.
//!
//! Extraction runs after `ScenePlugin` resolves `GlobalTransform3D` and before the
//! `GpuWindowPlugin` renders its frame. GPU passes consume this compact snapshot
//! instead of querying gameplay ECS data while recording GPU commands.

use crate::components::{
    Camera3D, DynamicMeshInstance3D, FogSettings, GpuMeshInstance3D, MeshInstance, ModelInstance,
    PointLight, SpotLight, SunLight,
};
use crate::gpu_streaming::GpuResourceLease;
use crate::mesh::MeshData;
use crate::renderer::{MaterialId, MeshId};
use glam::Mat4;
use ribbon_app::{App, Stage, World};
use ribbon_assets::Handle;
use ribbon_plugin_gltf::ModelAsset;
use ribbon_plugins::Plugin;
use ribbon_render::profiling::RenderProfileRecorder;
use ribbon_scene::GlobalTransform3D;
use std::sync::Arc;
use std::time::Instant;

pub use ribbon_render_contracts::{ExtractedCamera3D, ExtractedMesh3D};

/// One glTF model placement extracted from an ECS entity.
#[derive(Clone, Copy, Debug)]
pub struct ExtractedModel3D {
    /// Resolved object-to-world matrix.
    pub transform: Mat4,
    /// Weak handle resolved against `AssetServer` during rendering.
    pub model: Handle<ModelAsset>,
}

/// One placement of an arbitrary indexed mesh extracted from the ECS.
#[derive(Clone)]
pub struct ExtractedDynamicMesh3D {
    /// Resolved object-to-world matrix.
    pub transform: Mat4,
    /// Shared CPU geometry retained while its GPU cache entry is live.
    pub mesh: Arc<MeshData>,
    /// Linear RGB tint multiplier.
    pub color: [f32; 3],
}

/// Standalone GPU mesh/material placement. The extracted leases keep both resources alive until
/// this frame has finished consuming the snapshot.
#[derive(Clone)]
pub struct ExtractedGpuMesh3D {
    pub transform: Mat4,
    pub mesh: MeshId,
    pub material: MaterialId,
    pub color: [f32; 3],
    _mesh_lease: GpuResourceLease,
    _material_lease: GpuResourceLease,
}

/// Compact render-facing scene snapshot produced from the ECS each frame.
///
/// This contains copied transforms, shared mesh geometry, and asset handles rather than ECS
/// component references. Model handle records stay here because they are specific to the glTF API.
#[derive(Default)]
pub struct ExtractedScene3D {
    /// First camera with a resolved global transform, if any.
    pub camera: Option<ExtractedCamera3D>,
    /// Built-in mesh instances.
    pub meshes: Vec<ExtractedMesh3D>,
    /// Arbitrary indexed mesh instances with shared CPU geometry.
    pub dynamic_meshes: Vec<ExtractedDynamicMesh3D>,
    /// Independently warmed GPU mesh/material instances.
    pub gpu_meshes: Vec<ExtractedGpuMesh3D>,
    /// Loaded or pending glTF model instances.
    pub models: Vec<ExtractedModel3D>,
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
    /// CPU extraction time, populated only while render-profile capture is enabled.
    pub extraction_cpu_time_ns: Option<u64>,
}

/// Extract camera, transform, mesh, model and light data from the live ECS world.
pub fn extract_scene_3d(world: &mut World) {
    let extraction_started = world
        .try_resource::<RenderProfileRecorder>()
        .is_some()
        .then(Instant::now);
    let frame = world.resource::<ribbon_app::Time>().frame;
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

        extracted.dynamic_meshes.clear();
        for (_, (transform, instance)) in world
            .query::<(&GlobalTransform3D, &DynamicMeshInstance3D)>()
            .iter()
        {
            extracted.dynamic_meshes.push(ExtractedDynamicMesh3D {
                transform: transform.matrix(),
                mesh: Arc::clone(&instance.mesh),
                color: instance.color,
            });
        }

        extracted.gpu_meshes.clear();
        for (_, (transform, instance)) in world
            .query::<(&GlobalTransform3D, &GpuMeshInstance3D)>()
            .iter()
        {
            extracted.gpu_meshes.push(ExtractedGpuMesh3D {
                transform: transform.matrix(),
                mesh: instance.mesh,
                material: instance.material,
                color: instance.color,
                _mesh_lease: instance._mesh_lease.clone(),
                _material_lease: instance._material_lease.clone(),
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
    if let Some(started) = extraction_started {
        world
            .resource_mut::<ExtractedScene3D>()
            .extraction_cpu_time_ns =
            Some(started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64);
    } else {
        world
            .resource_mut::<ExtractedScene3D>()
            .extraction_cpu_time_ns = None;
    }
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
    use glam::Vec3;
    use ribbon_app::App;
    use ribbon_scene::GlobalTransform3D;

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
