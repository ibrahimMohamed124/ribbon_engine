//! ECS components and extraction for the voxel renderer.

use crate::cpu::render::lighting::SurfaceShader;
use crate::cpu::renderer::CpuRenderer;
use crate::types::entities::{EntityInstance, EntityKind, EntityRegistry};
use crate::types::{
    Camera as SceneCamera, LightingParams, MaterialTable, RenderScene, RenderStyle,
};
use engine_app::{App, LogLevel};
use engine_ecs::World;
use engine_math::{Angles, Vect};
use engine_platform::PlatformPlugin;
use engine_plugins::{Plugin, PluginError};
use engine_render::{DrawList, FramePlan, Framebuffer, Renderer};
use engine_scene::{GlobalTransform, ScenePlugin};
use plugin_voxel::VoxelWorldRes;

/// ECS camera settings. Its pose comes from the entity's `GlobalTransform`.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    /// Horizontal field of view in radians.
    pub hfov: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self { hfov: 1.1 }
    }
}

/// Marks the camera selected for the current frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct ActiveCamera;

/// Renderer-facing data attached to an ECS entity.
#[derive(Clone, Copy, Debug)]
pub struct RenderEntity {
    pub kind: EntityKind,
    pub params: [f32; 4],
}

/// Current voxel material table.
#[derive(Clone, Copy)]
pub struct MaterialTableRes(pub &'static MaterialTable);

/// Entity mesh/animation definitions used by render extraction.
#[derive(Clone, Copy)]
pub struct EntityRegistryRes(pub &'static EntityRegistry);

/// Lighting settings passed to the selected render backend.
#[derive(Clone, Copy)]
pub struct LightingRes(pub LightingParams);

/// Style applied to the world layer.
#[derive(Clone, Copy)]
pub struct RenderStyleRes(pub RenderStyle);

/// CPU rendering output retained for platform presentation or headless capture.
pub struct CpuBackend<S> {
    pub renderer: CpuRenderer<S>,
    pub framebuffer: Framebuffer,
}

impl<S> CpuBackend<S> {
    pub fn new(width: u32, height: u32, shader: S) -> Self {
        Self {
            renderer: CpuRenderer::new(width, height, shader),
            framebuffer: Framebuffer::new(width as usize, height as usize),
        }
    }
}

/// Backend selected by an application.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderBackend {
    #[default]
    Cpu,
    Gpu,
}

/// Backend choice and internal framebuffer dimensions.
#[derive(Clone, Copy, Debug)]
pub struct RenderSettings {
    pub backend: RenderBackend,
    pub width: u32,
    pub height: u32,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            backend: RenderBackend::Cpu,
            width: 320,
            height: 200,
        }
    }
}

/// A small default shader for engine examples; games can install their own backend.
pub struct DefaultShader;

impl SurfaceShader for DefaultShader {
    fn albedo(
        &self,
        block: u8,
        _hit: &plugin_voxel::Hit,
        _direction: Vect,
        _style: &RenderStyle,
    ) -> f32 {
        f32::from(block) / f32::from(u8::MAX)
    }
}

/// Errors while installing required rendering plugins.
#[derive(Debug)]
pub enum RenderingPluginError {
    Plugin(PluginError),
}

/// Rendering plugin requiring platform and scene foundations.
pub struct RenderingPlugin;

impl Plugin for RenderingPlugin {
    fn name(&self) -> &'static str {
        "RenderingPlugin"
    }
    fn dependencies(&self) -> &'static [&'static str] {
        &["PlatformPlugin", "ScenePlugin"]
    }
    fn build(&self, app: &mut App) {
        if app.world.try_resource::<RenderSettings>().is_none() {
            app.world.insert_resource(RenderSettings::default());
        }
        let settings = *app.world.resource::<RenderSettings>();
        match settings.backend {
            RenderBackend::Cpu => {
                if app.world.try_resource::<CpuBackend<DefaultShader>>().is_none() {
                    app.world.insert_resource(CpuBackend::new(
                        settings.width,
                        settings.height,
                        DefaultShader,
                    ));
                }
            }
            RenderBackend::Gpu => app.world.resource_mut::<engine_app::LogBuffer>().push(
                LogLevel::Warn,
                "voxel GPU backend remains available through the legacy renderer; ECS GPU selection is pending",
            ),
        }
        app.add_system(
            engine_app::Stage::Render,
            render_frame_system::<DefaultShader>,
        );
    }
}

/// Extract the active camera and render entities, then render one CPU frame.
/// The system is inert until all scene and backend resources have been installed.
pub fn render_frame_system<S: SurfaceShader + Send + Sync + 'static>(world: &mut World) {
    let Some(backend) = world.try_resource::<CpuBackend<S>>() else {
        return;
    };
    let _ = backend;
    world.resource_scope::<CpuBackend<S>, _>(|world, backend| {
        let camera = world
            .query::<(&Camera, &ActiveCamera, &GlobalTransform)>()
            .iter()
            .next()
            .map(|(entity, (camera, _, transform))| (entity, *camera, *transform));
        let Some((_camera_entity, camera, pose)) = camera else {
            return;
        };
        let entities: Vec<_> = world
            .query::<(&RenderEntity, &GlobalTransform)>()
            .iter()
            .map(|(_, (render, transform))| EntityInstance {
                kind: render.kind,
                pos: transform.translation,
                yaw: transform.rotation.phi,
                params: render.params,
            })
            .collect();
        let mut ui_layer = world.remove_resource::<DrawList>().unwrap_or_else(|| {
            DrawList::new(backend.framebuffer.width, backend.framebuffer.height)
        });
        ui_layer.width = backend.framebuffer.width;
        ui_layer.height = backend.framebuffer.height;
        let Some(voxels) = world.try_resource::<VoxelWorldRes>() else {
            return;
        };
        let Some(materials) = world.try_resource::<MaterialTableRes>() else {
            return;
        };
        let Some(registry) = world.try_resource::<EntityRegistryRes>() else {
            return;
        };
        let mut lighting =
            world
                .try_resource::<LightingRes>()
                .map(|r| r.0)
                .unwrap_or(LightingParams {
                    light_level: 0.0,
                    intensity: 1.0,
                    frame: 0,
                    style: RenderStyle::default(),
                });
        lighting.style = world
            .try_resource::<RenderStyleRes>()
            .map_or_else(RenderStyle::default, |style| style.0);
        let scene = RenderScene {
            world: &voxels.0,
            empty_voxel: voxels.1,
            camera: SceneCamera {
                eye: pose.translation,
                view: Angles {
                    psi: pose.rotation.psi,
                    phi: pose.rotation.phi,
                },
                hfov: camera.hfov,
            },
            lighting,
            entities,
            entity_registry: registry.0,
        };
        if let Err(error) = scene.validate_entity_limits() {
            world
                .resource_mut::<engine_app::LogBuffer>()
                .push(LogLevel::Warn, error);
            return;
        }
        let width = backend.framebuffer.width;
        let height = backend.framebuffer.height;
        let plan = FramePlan {
            scene: Some(scene),
            world_layer: DrawList::new(width, height),
            brightness: 1.0,
            ui_layer,
        };
        backend.renderer.render_frame(&plan, materials.0);
        backend.renderer.present(&mut backend.framebuffer);
        let _ = _camera_entity;
    });
}

/// Register `RenderingPlugin`, returning dependency errors without panicking.
pub fn add_rendering_plugin(app: &mut App) -> Result<(), PluginError> {
    use engine_plugins::AppPluginExt;
    app.add_plugin(PlatformPlugin)?;
    app.add_plugin(ScenePlugin)?;
    app.add_plugin(RenderingPlugin)?;
    Ok(())
}
