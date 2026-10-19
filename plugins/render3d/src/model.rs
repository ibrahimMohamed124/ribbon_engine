//! Uploads CPU-side glTF models to the GPU as meshes and materials.

use crate::mesh::Vertex;
use crate::renderer::{MaterialId, MeshId, MeshRenderer, TextureData};
use glam::Mat4;
use plugin_gltf::ModelAsset;

/// One uploaded primitive and where it sits in model space.
#[derive(Clone, Copy, Debug)]
pub struct GpuModelPart {
    /// Uploaded geometry.
    pub mesh: MeshId,
    /// Uploaded material ([`MeshRenderer::WHITE`] when the primitive has none).
    pub material: MaterialId,
    /// Model-space placement from the node hierarchy.
    pub transform: Mat4,
}

/// Upload every primitive and material of `model`; returns its placed parts.
pub fn upload_model(
    renderer: &mut MeshRenderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    model: &ModelAsset,
) -> Vec<GpuModelPart> {
    let materials: Vec<MaterialId> = model
        .materials
        .iter()
        .map(|material| {
            let texture = material
                .base_color_texture
                .and_then(|index| model.images.get(index))
                .map(|image| TextureData {
                    width: image.width,
                    height: image.height,
                    rgba8: &image.rgba8,
                });
            renderer.upload_material(device, queue, material.base_color, texture)
        })
        .collect();
    let meshes: Vec<MeshId> = model
        .primitives
        .iter()
        .map(|primitive| {
            let vertices: Vec<Vertex> = primitive
                .positions
                .iter()
                .zip(&primitive.normals)
                .zip(&primitive.uvs)
                .map(|((position, normal), uv)| Vertex {
                    position: *position,
                    normal: *normal,
                    uv: *uv,
                })
                .collect();
            renderer.upload_mesh(device, &vertices, &primitive.indices)
        })
        .collect();
    model
        .nodes
        .iter()
        .map(|node| GpuModelPart {
            mesh: meshes[node.primitive],
            material: model.primitives[node.primitive]
                .material
                .and_then(|index| materials.get(index).copied())
                .unwrap_or(MeshRenderer::WHITE),
            transform: node.transform,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::upload_model;
    use crate::components::{Camera3D, SunLight, Transform3D};
    use crate::renderer::{Draw, MeshRenderer, Scene};
    use engine_render::gpu::GpuContext;
    use engine_render::gpu_window::clear_view;
    use glam::{Mat4, Vec3};

    const CUBE: &[u8] = include_bytes!("../../../assets/models/checker_cube.glb");

    #[test]
    fn textured_gltf_cube_renders_with_its_checker_pattern() {
        let context = match pollster::block_on(GpuContext::new_offscreen(64, 64)) {
            Ok(context) => context,
            Err(error) => {
                eprintln!("skipping GPU test: {}", error.0);
                return;
            }
        };
        let (device, queue) = (context.device(), context.queue());
        clear_view(device, queue, context.target().view(), [0.0, 0.0, 0.0, 1.0]);
        let mut renderer = MeshRenderer::new(device, queue, wgpu::TextureFormat::Rgba8Unorm);
        let model = plugin_gltf::parse(CUBE).expect("model parses");
        let parts = upload_model(&mut renderer, device, queue, &model);
        assert_eq!(parts.len(), 1);
        assert_ne!(parts[0].material, MeshRenderer::WHITE);
        // The node sits at (1.5, 0.5, 0); shift it back to the origin for the fixed test camera.
        let recenter = Mat4::from_translation(-Vec3::new(1.5, 0.5, 0.0));
        let draws: Vec<Draw> = parts
            .iter()
            .map(|part| Draw {
                mesh: part.mesh,
                material: part.material,
                model: recenter * part.transform,
                tint: [1.0; 3],
            })
            .collect();
        let eye = Transform3D::looking_at(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO, Vec3::Y);
        let view = Mat4::from_rotation_translation(eye.rotation, eye.translation).inverse();
        renderer.render(
            device,
            queue,
            context.target().view(),
            (64, 64),
            &Scene {
                view_proj: Camera3D::default().projection(1.0) * view,
                sun: &SunLight::default(),
                draws: &draws,
            },
        );
        let pixels = context.readback().expect("readback").pixels;
        let (mut orange, mut blue) = (0, 0);
        for p in pixels {
            let (r, b) = ((p >> 16) & 0xff, p & 0xff);
            if r > 2 * b && r > 40 {
                orange += 1;
            } else if b > 2 * r && b > 40 {
                blue += 1;
            }
        }
        assert!(
            orange > 50 && blue > 50,
            "expected both checker colors, orange={orange} blue={blue}"
        );
    }
}
