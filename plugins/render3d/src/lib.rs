//! Mesh rendering for the engine: ECS components, an instanced wgpu mesh
//! pipeline with a depth buffer, and a plugin that draws into the window.
//!
//! World space is right-handed and Y-up. Transforms here are world-space; the
//! scene hierarchy is not consulted yet.

pub mod components;
pub mod mesh;
pub mod model;
pub mod renderer;

pub use components::{BuiltinMesh, Camera3D, MeshInstance, ModelInstance, SunLight, Transform3D};
pub use glam;
pub use model::{upload_model, GpuModelPart};
pub use renderer::{Draw, MaterialId, MeshId, MeshRenderer, Scene, TextureData};

use engine_app::{App, World};
use engine_assets::AssetServer;
use engine_plugins::Plugin;
use engine_render::gpu_window::{FramePass, FrameTarget, GpuWindow};
use glam::Mat4;
use std::collections::hash_map::Entry;
use std::collections::HashMap;

#[derive(Default)]
struct MeshPass {
    renderer: Option<(wgpu::TextureFormat, MeshRenderer)>,
    /// Uploaded models keyed by asset handle id and generation.
    models: HashMap<(u64, u32), Vec<GpuModelPart>>,
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
        let mut draws: Vec<Draw> = world
            .query::<(&Transform3D, &MeshInstance)>()
            .iter()
            .map(|(_, (transform, mesh))| Draw {
                mesh: MeshRenderer::CUBE,
                material: MeshRenderer::WHITE,
                model: transform.matrix(),
                tint: mesh.color,
            })
            .collect();
        let placements: Vec<_> = world
            .query::<(&Transform3D, &ModelInstance)>()
            .iter()
            .map(|(_, (transform, instance))| (transform.matrix(), instance.model))
            .collect();
        if self
            .renderer
            .as_ref()
            .is_none_or(|(format, _)| *format != target.format)
        {
            self.models.clear();
            self.renderer = Some((
                target.format,
                MeshRenderer::new(target.device, target.queue, target.format),
            ));
        }
        let Some((_, renderer)) = self.renderer.as_mut() else {
            return;
        };
        if !placements.is_empty() {
            if let Some(server) = world.try_resource::<AssetServer>() {
                for (matrix, handle) in placements {
                    let key = (handle.id(), handle.generation());
                    let parts = match self.models.entry(key) {
                        Entry::Occupied(entry) => entry.into_mut(),
                        Entry::Vacant(entry) => {
                            let Some(model) = server.get(handle) else {
                                continue;
                            };
                            entry.insert(upload_model(renderer, target.device, target.queue, model))
                        }
                    };
                    draws.extend(parts.iter().map(|part| Draw {
                        mesh: part.mesh,
                        material: part.material,
                        model: matrix * part.transform,
                        tint: [1.0; 3],
                    }));
                }
            }
        }
        let sun = world
            .try_resource::<SunLight>()
            .copied()
            .unwrap_or_default();
        let view = Mat4::from_rotation_translation(pose.rotation, pose.translation).inverse();
        let view_proj = camera.projection(width as f32 / height as f32) * view;
        renderer.render(
            target.device,
            target.queue,
            target.view,
            target.size,
            &Scene {
                view_proj,
                sun: &sun,
                draws: &draws,
            },
        );
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
