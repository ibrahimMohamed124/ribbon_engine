//! ECS components and extraction for the voxel renderer.

use crate::cpu::render::lighting::SurfaceShader;
use crate::cpu::renderer::CpuRenderer;
use crate::types::entities::{EntityInstance, EntityKind, EntityRegistry};
use crate::types::{
    Camera as SceneCamera, LightingParams, MaterialTable, RenderScene, RenderStyle,
};
use ribbon_app::{App, LogLevel};
use ribbon_ecs::World;
use ribbon_math::{Angles, Vect};
use ribbon_platform::{PlatformPlugin, WindowFrame};
use ribbon_plugin_voxel::VoxelWorldRes;
use ribbon_plugins::{Plugin, PluginError};
use ribbon_render::{DrawList, FramePlan, Framebuffer, Renderer};
use ribbon_scene::{GlobalTransform, ScenePlugin};

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

/// Offscreen GPU voxel renderer used by ECS applications selecting [`RenderBackend::Gpu`].
/// The completed frame is copied to the platform-neutral `WindowFrame`; a native-surface
/// presentation path can replace this readback bridge without changing scene extraction.
#[cfg(feature = "gpu")]
pub struct GpuBackend {
    pub renderer: Option<crate::gpu::voxel::GpuRenderer>,
    pub framebuffer: Framebuffer,
    /// Set after an initialization failure so the scheduled GPU system can keep
    /// forwarding frames to the CPU fallback on subsequent updates.
    pub failed: bool,
}

#[cfg(feature = "gpu")]
impl GpuBackend {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            renderer: None,
            framebuffer: Framebuffer::new(width as usize, height as usize),
            failed: false,
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
        _hit: &ribbon_plugin_voxel::Hit,
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
            app.insert_resource(RenderSettings::default());
        }
        let settings = *app.world.resource::<RenderSettings>();
        match settings.backend {
            RenderBackend::Cpu => {
                if app
                    .world
                    .try_resource::<CpuBackend<DefaultShader>>()
                    .is_none()
                {
                    app.world.insert_resource(CpuBackend::new(
                        settings.width.max(1),
                        settings.height.max(1),
                        DefaultShader,
                    ));
                }
                app.add_system(
                    ribbon_app::Stage::Render,
                    render_frame_system::<DefaultShader>,
                );
            }
            RenderBackend::Gpu => {
                #[cfg(feature = "gpu")]
                {
                    if app.world.try_resource::<GpuBackend>().is_none() {
                        app.world.insert_resource(GpuBackend::new(
                            settings.width.max(1),
                            settings.height.max(1),
                        ));
                    }
                    app.add_system(ribbon_app::Stage::Render, render_gpu_frame_system);
                }
                #[cfg(not(feature = "gpu"))]
                {
                    app.world.resource_mut::<ribbon_app::LogBuffer>().push(
                        LogLevel::Warn,
                        "GPU rendering was requested but ribbon_plugin_rendering was built without its `gpu` feature; using CPU",
                    );
                    app.insert_resource(RenderSettings {
                        backend: RenderBackend::Cpu,
                        width: settings.width.max(1),
                        height: settings.height.max(1),
                    });
                    app.world.insert_resource(CpuBackend::new(
                        settings.width.max(1),
                        settings.height.max(1),
                        DefaultShader,
                    ));
                    app.add_system(
                        ribbon_app::Stage::Render,
                        render_frame_system::<DefaultShader>,
                    );
                }
            }
        }
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
        if world.try_resource::<VoxelWorldRes>().is_none()
            || world.try_resource::<MaterialTableRes>().is_none()
            || world.try_resource::<EntityRegistryRes>().is_none()
        {
            return;
        }
        let mut ui_layer = world.remove_resource::<DrawList>().unwrap_or_else(|| {
            DrawList::new(backend.framebuffer.width, backend.framebuffer.height)
        });
        ui_layer.width = backend.framebuffer.width;
        ui_layer.height = backend.framebuffer.height;
        let voxels = world.resource::<VoxelWorldRes>();
        let materials = world.resource::<MaterialTableRes>();
        let registry = world.resource::<EntityRegistryRes>();
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
            world.insert_resource(ui_layer);
            world
                .resource_mut::<ribbon_app::LogBuffer>()
                .push(LogLevel::Warn, error);
            return;
        }
        let settings = world
            .try_resource::<RenderSettings>()
            .copied()
            .unwrap_or_default();
        let width = settings.width.max(1) as usize;
        let height = settings.height.max(1) as usize;
        if backend.framebuffer.width != width || backend.framebuffer.height != height {
            backend.renderer.resize(width as u32, height as u32);
            backend.framebuffer = Framebuffer::new(width, height);
        }
        ui_layer.width = width;
        ui_layer.height = height;
        let plan = FramePlan {
            scene: Some(scene),
            world_layer: DrawList::new(width, height),
            brightness: 1.0,
            ui_layer,
        };
        backend.renderer.render_frame(&plan, materials.0);
        backend.renderer.present(&mut backend.framebuffer);
        drop(plan);
        if world.try_resource::<WindowFrame>().is_some() {
            let width = backend.framebuffer.width;
            let height = backend.framebuffer.height;
            let pixels = backend.framebuffer.pixels.clone();
            world
                .resource_mut::<WindowFrame>()
                .set_pixels(width, height, &pixels);
        }
        let _ = _camera_entity;
    });
}

/// Extract an ECS scene and render it through the GPU voxel backend.
///
/// This first integration uses the established GPU raycaster offscreen target and
/// the generic frame bridge; the returned pixels are presented by `WinitRunner`.
#[cfg(feature = "gpu")]
pub fn render_gpu_frame_system(world: &mut World) {
    let Some(gpu_failed) = world
        .try_resource::<GpuBackend>()
        .map(|backend| backend.failed)
    else {
        return;
    };
    if gpu_failed {
        render_frame_system::<DefaultShader>(world);
        return;
    }
    let mut initialization_error = None;
    world.resource_scope::<GpuBackend, _>(|world, backend| {
        let camera = world
            .query::<(&Camera, &ActiveCamera, &GlobalTransform)>()
            .iter()
            .next()
            .map(|(entity, (camera, _, transform))| (entity, *camera, *transform));
        let Some((_camera_entity, camera, pose)) = camera else {
            return;
        };
        if world.try_resource::<VoxelWorldRes>().is_none()
            || world.try_resource::<MaterialTableRes>().is_none()
            || world.try_resource::<EntityRegistryRes>().is_none()
        {
            return;
        }
        let mut ui_layer = world.remove_resource::<DrawList>().unwrap_or_else(|| {
            DrawList::new(
                backend.framebuffer.width.max(1),
                backend.framebuffer.height.max(1),
            )
        });
        let voxels = world.resource::<VoxelWorldRes>();
        let materials = world.resource::<MaterialTableRes>();
        let registry = world.resource::<EntityRegistryRes>();
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
        let mut lighting = world
            .try_resource::<LightingRes>()
            .map(|resource| resource.0)
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
            world.insert_resource(ui_layer);
            world
                .resource_mut::<ribbon_app::LogBuffer>()
                .push(LogLevel::Warn, error);
            return;
        }

        let settings = world
            .try_resource::<RenderSettings>()
            .copied()
            .unwrap_or_default();
        let width = settings.width.max(1) as usize;
        let height = settings.height.max(1) as usize;
        if backend.framebuffer.width != width || backend.framebuffer.height != height {
            backend.framebuffer = Framebuffer::new(width, height);
        }
        if backend.renderer.is_none() {
            match crate::gpu::voxel::GpuRenderer::new(width as u32, height as u32) {
                Ok(renderer) => backend.renderer = Some(renderer),
                Err(error) => {
                    world.insert_resource(ui_layer);
                    initialization_error = Some(error);
                    return;
                }
            }
        }
        ui_layer.width = width;
        ui_layer.height = height;
        let plan = FramePlan {
            scene: Some(scene),
            world_layer: DrawList::new(width, height),
            brightness: 1.0,
            ui_layer,
        };
        let mut frame_error = None;
        if let Some(renderer) = backend.renderer.as_mut() {
            renderer.render_frame(&plan, materials.0);
            drop(plan);
            frame_error = renderer.take_error();
            renderer.present(&mut backend.framebuffer);
            if world.try_resource::<WindowFrame>().is_some() {
                world.resource_mut::<WindowFrame>().set_pixels(
                    backend.framebuffer.width,
                    backend.framebuffer.height,
                    &backend.framebuffer.pixels,
                );
            }
        } else {
            drop(plan);
        }
        if let Some(error) = frame_error {
            world
                .resource_mut::<ribbon_app::LogBuffer>()
                .push(LogLevel::Warn, format!("GPU voxel frame failed: {error}"));
        }
    });

    if let Some(error) = initialization_error {
        world.resource_mut::<ribbon_app::LogBuffer>().push(
            LogLevel::Warn,
            format!("GPU voxel renderer initialization failed; falling back to CPU: {error}"),
        );
        let settings = world
            .try_resource::<RenderSettings>()
            .copied()
            .unwrap_or_default();
        world.insert_resource(RenderSettings {
            backend: RenderBackend::Cpu,
            width: settings.width.max(1),
            height: settings.height.max(1),
        });
        world.resource_mut::<GpuBackend>().failed = true;
        world.insert_resource(CpuBackend::new(
            settings.width.max(1),
            settings.height.max(1),
            DefaultShader,
        ));
        render_frame_system::<DefaultShader>(world);
    }
}

/// Register `RenderingPlugin`, returning dependency errors without panicking.
pub fn add_rendering_plugin(app: &mut App) -> Result<(), PluginError> {
    use ribbon_plugins::AppPluginExt;
    app.add_plugin(PlatformPlugin)?;
    app.add_plugin(ScenePlugin)?;
    app.add_plugin(RenderingPlugin)?;
    Ok(())
}
