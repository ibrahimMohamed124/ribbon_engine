//! Uploads CPU-side glTF models to the GPU as meshes and materials.

use crate::mesh::Vertex;
use crate::renderer::{MaterialId, MeshId, MeshRenderer, PbrMaterialDesc, TextureData};
use glam::Mat4;
use ribbon_plugin_gltf::ModelAsset;
use std::collections::HashSet;

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

/// Upload every placed primitive and the materials referenced by those parts.
pub fn upload_model(
    renderer: &mut MeshRenderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    model: &ModelAsset,
) -> Vec<GpuModelPart> {
    let used_materials: HashSet<usize> = model
        .nodes
        .iter()
        .filter_map(|node| model.primitives.get(node.primitive)?.material)
        .collect();
    // Don't allocate GPU resources for file materials that are not referenced by any placed
    // primitive in the loaded scene. These IDs would otherwise never appear in GpuModelPart and
    // therefore could not be released with the rest of that model's cache entry.
    let materials: Vec<Option<MaterialId>> = model
        .materials
        .iter()
        .enumerate()
        .map(|(index, material)| {
            if !used_materials.contains(&index) {
                return None;
            }
            let texture = material
                .base_color_texture
                .and_then(|index| model.images.get(index))
                .map(|image| TextureData {
                    width: image.width,
                    height: image.height,
                    rgba8: &image.rgba8,
                });
            let metallic_roughness_texture = material
                .metallic_roughness_texture
                .and_then(|index| model.images.get(index))
                .map(|image| TextureData {
                    width: image.width,
                    height: image.height,
                    rgba8: &image.rgba8,
                });
            let normal_texture = material
                .normal_texture
                .and_then(|index| model.images.get(index))
                .map(|image| TextureData {
                    width: image.width,
                    height: image.height,
                    rgba8: &image.rgba8,
                });
            let occlusion_texture = material
                .occlusion_texture
                .and_then(|index| model.images.get(index))
                .map(|image| TextureData {
                    width: image.width,
                    height: image.height,
                    rgba8: &image.rgba8,
                });
            let emissive_texture = material
                .emissive_texture
                .and_then(|index| model.images.get(index))
                .map(|image| TextureData {
                    width: image.width,
                    height: image.height,
                    rgba8: &image.rgba8,
                });
            Some(renderer.upload_pbr_material_with_maps(
                device,
                queue,
                PbrMaterialDesc {
                    base_color: material.base_color,
                    metallic: material.metallic_factor,
                    roughness: material.roughness_factor,
                    base_color_texture: texture,
                    metallic_roughness_texture,
                    normal_texture,
                    occlusion_texture,
                    occlusion_strength: material.occlusion_strength,
                    emissive_texture,
                    emissive_factor: material.emissive_factor,
                },
            ))
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
                .and_then(|index| materials.get(index).copied().flatten())
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
    use glam::{Mat4, Vec3};
    use ribbon_render::gpu::GpuContext;
    use ribbon_render::gpu_window::clear_view;

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
        let model = ribbon_plugin_gltf::parse(CUBE).expect("model parses");
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
                camera_position: eye.translation,
                sun: &SunLight::default(),
                spot_lights: &[],
                point_lights: &[],
                fog: crate::FogSettings::default(),
                draws: &draws,
            },
        );
        let pixels = context.readback().expect("readback").pixels;
        let (mut orange, mut blue) = (0, 0);
        // The renderer writes linear PBR values; this low-level offscreen test does
        // not run the production HDR tone-mapping/sRGB presentation stage. Check
        // color dominance at linear intensity, not a display-space byte threshold.
        for p in pixels {
            let (r, b) = ((p >> 16) & 0xff, p & 0xff);
            if r > 2 * b && r > 5 {
                orange += 1;
            } else if b > 2 * r && b > 5 {
                blue += 1;
            }
        }
        // A 64x64 offscreen face can cover only a modest number of pixels for
        // each UV checker cell. Require a visible cluster of both colors while
        // leaving headroom for differences between GPU adapters and drivers.
        assert!(
            orange > 20 && blue > 20,
            "expected both checker colors, orange={orange} blue={blue}"
        );
    }
}
