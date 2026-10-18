//! Mesh rendering for the engine: ECS components, an instanced wgpu mesh
//! pipeline with a depth buffer, and a plugin that draws into the window.
//!
//! World space is right-handed and Y-up. Transforms here are world-space; the
//! scene hierarchy is not consulted yet.

pub mod components;
pub mod mesh;
pub mod renderer;

pub use components::{BuiltinMesh, Camera3D, MeshInstance, SunLight, Transform3D};
pub use glam;
pub use renderer::{InstanceData, MeshRenderer, Scene};

use engine_app::{App, World};
use engine_plugins::Plugin;
use engine_render::gpu_window::{FramePass, FrameTarget, GpuWindow};
use glam::Mat4;

#[derive(Default)]
struct MeshPass {
    renderer: Option<(wgpu::TextureFormat, MeshRenderer)>,
}

impl FramePass for MeshPass {
    fn render(&mut self, world: &mut World, target: &FrameTarget<'_>) {
        let (width, height) = target.size;
        if width == 0 || height == 0 {
            return;
        }
        let Some((pose, camera)) = world
            .query::<(&Transform3D, &Camera3D)>()
            .iter()
            .next()
            .map(|(_, (pose, camera))| (*pose, *camera))
        else {
            return;
        };
        let instances: Vec<InstanceData> = world
            .query::<(&Transform3D, &MeshInstance)>()
            .iter()
            .map(|(_, (transform, mesh))| InstanceData {
                model: transform.matrix(),
                color: mesh.color,
            })
            .collect();
        let sun = world
            .try_resource::<SunLight>()
            .copied()
            .unwrap_or_default();
        let view = Mat4::from_rotation_translation(pose.rotation, pose.translation).inverse();
        let view_proj = camera.projection(width as f32 / height as f32) * view;
        if self
            .renderer
            .as_ref()
            .is_none_or(|(format, _)| *format != target.format)
        {
            self.renderer = Some((
                target.format,
                MeshRenderer::new(target.device, target.format),
            ));
        }
        if let Some((_, renderer)) = self.renderer.as_mut() {
            renderer.render(
                target.device,
                target.queue,
                target.view,
                target.size,
                &Scene {
                    view_proj,
                    sun: &sun,
                    instances: &instances,
                },
            );
        }
    }
}

/// Registers the mesh pass with the window presenter and installs a default [`SunLight`].
pub struct Render3dPlugin;

impl Plugin for Render3dPlugin {
    fn name(&self) -> &'static str {
        "Render3dPlugin"
    }

    fn dependencies(&self) -> &'static [&'static str] {
        &["GpuWindowPlugin"]
    }

    fn build(&self, app: &mut App) {
        if app.world.try_resource::<SunLight>().is_none() {
            app.insert_resource(SunLight::default());
        }
        app.world
            .resource_mut::<GpuWindow>()
            .add_pass(Box::new(MeshPass::default()));
    }
}
