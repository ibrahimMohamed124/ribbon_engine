//! Mesh rendering for the engine: ECS components, an instanced wgpu mesh
//! pipeline with a depth buffer, and a plugin that draws into the window.
//!
//! World space is right-handed and Y-up. Three-dimensional local transforms are
//! owned by `engine_scene`; this renderer consumes their resolved global transforms.

pub mod components;
pub mod extract;
pub mod mesh;
pub mod model;
pub mod renderer;

pub use components::{BuiltinMesh, Camera3D, MeshInstance, ModelInstance, SunLight, Transform3D};
pub use engine_scene::GlobalTransform3D;
pub use extract::{
    extract_scene_3d, ExtractedCamera3D, ExtractedMesh3D, ExtractedModel3D, ExtractedScene3D,
    RenderExtraction3DPlugin,
};
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
        // Temporarily borrow the snapshot so GPU submission can read AssetServer without
        // cloning the extracted vectors. `resource_scope` restores the snapshot even on unwind.
        if world.try_resource::<ExtractedScene3D>().is_none() {
            return;
        }
        world.resource_scope::<ExtractedScene3D, _>(|world, extracted| {
            self.render_extracted(world, extracted, target);
        });
    }
}

impl MeshPass {
    fn render_extracted(
        &mut self,
        world: &mut World,
        extracted: &mut ExtractedScene3D,
        target: &FrameTarget<'_>,
    ) {
        let (width, height) = target.size;
        if width == 0 || height == 0 {
            return;
        }
        let Some(camera) = extracted.camera else {
            return;
        };

        let mut draws: Vec<Draw> = extracted
            .meshes
            .iter()
            .map(|mesh| Draw {
                mesh: MeshRenderer::CUBE,
                material: MeshRenderer::WHITE,
                model: mesh.transform,
                tint: mesh.color,
            })
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
        if !extracted.models.is_empty() {
            if let Some(server) = world.try_resource::<AssetServer>() {
                for placement in &extracted.models {
                    let key = (placement.model.id(), placement.model.generation());
                    let parts = match self.models.entry(key) {
                        Entry::Occupied(entry) => entry.into_mut(),
                        Entry::Vacant(entry) => {
                            let Some(model) = server.get(placement.model) else {
                                continue;
                            };
                            entry.insert(upload_model(renderer, target.device, target.queue, model))
                        }
                    };
                    draws.extend(parts.iter().map(|part| Draw {
                        mesh: part.mesh,
                        material: part.material,
                        model: placement.transform * part.transform,
                        tint: [1.0; 3],
                    }));
                }
            }
        }

        let view = Mat4::from_rotation_translation(
            camera.transform.rotation,
            camera.transform.translation,
        )
        .inverse();
        let view_proj = camera.camera.projection(width as f32 / height as f32) * view;
        renderer.render(
            target.device,
            target.queue,
            target.view,
            target.size,
            &Scene {
                view_proj,
                sun: &extracted.sun,
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
        &["GpuWindowPlugin", "ScenePlugin"]
    }

    fn build(&self, app: &mut App) {
        if app.world.try_resource::<SunLight>().is_none() {
            app.insert_resource(SunLight::default());
        }
        // ScenePlugin has already registered PostUpdate transform propagation because it is
        // an explicit dependency. Install extraction after it so all snapshots are world-space.
        extract::install_extraction(app);
        app.world
            .resource_mut::<GpuWindow>()
            .add_pass(Box::new(MeshPass::default()));
    }
}
