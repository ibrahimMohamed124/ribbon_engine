//! Uploads CPU-side glTF models to the GPU as meshes and materials.

use crate::mesh::Vertex;
use crate::renderer::{
    MaterialId, MeshId, MeshRenderer, PbrMaterialDesc, PbrMaterialMipmaps,
    PbrMaterialRenderOptions, TextureData,
};
use glam::Mat4;
use ribbon_plugin_gltf::ModelAsset;
use std::collections::HashSet;
use std::sync::Arc;

/// One uploaded primitive and where it sits in model space.
#[derive(Clone, Debug)]
pub struct GpuModelPart {
    /// Uploaded geometry.
    pub mesh: MeshId,
    /// Uploaded material ([`MeshRenderer::WHITE`] when the primitive has none).
    pub material: MaterialId,
    /// Model-space placement from the node hierarchy.
    pub transform: Mat4,
    /// Index into `ModelAsset::nodes`, used to find this placement's evaluated skin palette.
    pub placement_index: usize,
    /// Original glTF node index for animated placement transforms.
    pub node_index: usize,
    /// Skin index used by this placement, if any.
    pub skin: Option<usize>,
    /// Immutable fallback palette evaluated from the imported bind pose.
    pub bind_pose_palette: Option<Arc<[Mat4]>>,
}

/// Upload every placed primitive and the materials referenced by those parts.
pub fn upload_model(
    renderer: &mut MeshRenderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    model: &ModelAsset,
) -> Vec<GpuModelPart> {
    // Keep the original infallible API for compatibility, but do not hide invalid model data.
    // New code that needs a readiness contract should use `try_upload_model` and propagate the
    // validation error to its caller.
    match try_upload_model(renderer, device, queue, model) {
        Ok(parts) => parts,
        Err(error) => {
            eprintln!("render3d model upload rejected: {error}");
            Vec::new()
        }
    }
}

/// Validate and upload all resources referenced by a glTF model.
///
/// Validation happens before any GPU objects are created, so malformed or empty models cannot
/// partially populate the renderer cache and can be surfaced as a terminal warmup error.
pub fn try_upload_model(
    renderer: &mut MeshRenderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    model: &ModelAsset,
) -> Result<Vec<GpuModelPart>, String> {
    validate_model_for_upload(model)?;
    Ok(upload_validated_model(renderer, device, queue, model))
}

/// Upload a model after the caller has verified its CPU representation.
///
/// This is crate-visible so the renderer can surface malformed models without scanning their
/// full vertex/image payload twice before every successful upload.
pub(crate) fn upload_validated_model(
    renderer: &mut MeshRenderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    model: &ModelAsset,
) -> Vec<GpuModelPart> {
    let bind_pose_palettes = if model.nodes.iter().any(|node| node.skin.is_some()) {
        model
            .bind_pose_palettes()
            .expect("validated skin hierarchy must produce a valid bind pose")
    } else {
        vec![None; model.nodes.len()]
    };
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
            let base_image = material
                .base_color_texture
                .and_then(|index| model.images.get(index));
            let surface_image = material
                .metallic_roughness_texture
                .and_then(|index| model.images.get(index));
            let normal_image = material
                .normal_texture
                .and_then(|index| model.images.get(index));
            let occlusion_image = material
                .occlusion_texture
                .and_then(|index| model.images.get(index));
            let emissive_image = material
                .emissive_texture
                .and_then(|index| model.images.get(index));
            let texture = base_image.map(|image| TextureData {
                width: image.width,
                height: image.height,
                rgba8: &image.rgba8,
            });
            let metallic_roughness_texture = surface_image.map(|image| TextureData {
                width: image.width,
                height: image.height,
                rgba8: &image.rgba8,
            });
            let normal_texture = normal_image.map(|image| TextureData {
                width: image.width,
                height: image.height,
                rgba8: &image.rgba8,
            });
            let occlusion_texture = occlusion_image.map(|image| TextureData {
                width: image.width,
                height: image.height,
                rgba8: &image.rgba8,
            });
            let emissive_texture = emissive_image.map(|image| TextureData {
                width: image.width,
                height: image.height,
                rgba8: &image.rgba8,
            });
            Some(
                renderer.upload_pbr_material_with_precomputed_mips_and_options(
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
                    PbrMaterialMipmaps {
                        base_color: base_image.and_then(|image| image.srgb_mips.as_deref()),
                        metallic_roughness: surface_image
                            .and_then(|image| image.linear_mips.as_deref()),
                        normal: normal_image.and_then(|image| image.linear_mips.as_deref()),
                        occlusion: occlusion_image.and_then(|image| image.linear_mips.as_deref()),
                        emissive: emissive_image.and_then(|image| image.srgb_mips.as_deref()),
                    },
                    PbrMaterialRenderOptions {
                        alpha_mode: material.alpha_mode,
                        alpha_cutoff: material.alpha_cutoff,
                        double_sided: material.double_sided,
                    },
                ),
            )
        })
        .collect();
    // Only upload primitive geometry referenced by a placed node. Keeping unused primitive
    // uploads here would create MeshIds that never enter the returned cache entry and therefore
    // could not be released when a warmup or active scene use ends.
    let used_primitives: HashSet<usize> = model.nodes.iter().map(|node| node.primitive).collect();
    let meshes: Vec<Option<MeshId>> = model
        .primitives
        .iter()
        .enumerate()
        .map(|(index, primitive)| {
            if !used_primitives.contains(&index) {
                return None;
            }
            let vertices: Vec<Vertex> = primitive
                .positions
                .iter()
                .zip(&primitive.normals)
                .zip(&primitive.uvs)
                .enumerate()
                .map(|(vertex_index, ((position, normal), uv))| Vertex {
                    position: *position,
                    normal: *normal,
                    uv: *uv,
                    joints: primitive.joints[vertex_index],
                    weights: primitive.weights[vertex_index],
                })
                .collect();
            Some(renderer.upload_mesh(device, &vertices, &primitive.indices))
        })
        .collect();
    model
        .nodes
        .iter()
        .enumerate()
        .map(|(placement_index, node)| GpuModelPart {
            mesh: meshes[node.primitive]
                .expect("validated node references a primitive that was uploaded"),
            material: model.primitives[node.primitive]
                .material
                .and_then(|index| materials.get(index).copied().flatten())
                .unwrap_or(MeshRenderer::WHITE),
            transform: node.transform,
            placement_index,
            node_index: node.node,
            skin: node.skin,
            bind_pose_palette: bind_pose_palettes[placement_index].clone(),
        })
        .collect()
}

/// Check that every model resource the current renderer upload path indexes is valid.
pub fn validate_model_for_upload(model: &ModelAsset) -> Result<(), String> {
    if model.nodes.is_empty() {
        return Err("model has no placed renderable nodes".to_owned());
    }
    if model.primitives.is_empty() {
        return Err("model has no indexed primitives".to_owned());
    }
    for (index, primitive) in model.primitives.iter().enumerate() {
        if primitive.positions.is_empty() || primitive.indices.is_empty() {
            return Err(format!("primitive {index} has empty geometry"));
        }
        if primitive.positions.len() != primitive.normals.len()
            || primitive.positions.len() != primitive.uvs.len()
            || primitive.positions.len() != primitive.joints.len()
            || primitive.positions.len() != primitive.weights.len()
        {
            return Err(format!(
                "primitive {index} has mismatched position/normal/UV/skinning attribute counts"
            ));
        }
        for (vertex, weights) in primitive.weights.iter().enumerate() {
            if weights
                .iter()
                .any(|weight| !weight.is_finite() || *weight < 0.0)
            {
                return Err(format!(
                    "primitive {index} vertex {vertex} has invalid skin weights"
                ));
            }
            let sum: f32 = weights.iter().sum();
            if sum > 1.0e-6 && (sum - 1.0).abs() > 1.0e-3 {
                return Err(format!(
                    "primitive {index} vertex {vertex} skin weights are not normalized"
                ));
            }
        }
        if primitive.indices.len() % 3 != 0 {
            return Err(format!("primitive {index} is not a triangle list"));
        }
        if primitive
            .indices
            .iter()
            .any(|&index| index as usize >= primitive.positions.len())
        {
            return Err(format!("primitive {index} contains an out-of-range index"));
        }
        if primitive
            .material
            .is_some_and(|material| material >= model.materials.len())
        {
            return Err(format!("primitive {index} references a missing material"));
        }
        if primitive
            .positions
            .iter()
            .flatten()
            .any(|value| !value.is_finite())
            || primitive
                .normals
                .iter()
                .flatten()
                .any(|value| !value.is_finite())
            || primitive
                .uvs
                .iter()
                .flatten()
                .any(|value| !value.is_finite())
        {
            return Err(format!("primitive {index} contains non-finite vertex data"));
        }
    }
    for (index, node) in model.nodes.iter().enumerate() {
        if node.primitive >= model.primitives.len() {
            return Err(format!("node {index} references a missing primitive"));
        }
        if !node
            .transform
            .to_cols_array()
            .iter()
            .all(|value| value.is_finite())
        {
            return Err(format!("node {index} contains a non-finite transform"));
        }
    }
    for (placement_index, node) in model.nodes.iter().enumerate() {
        if node.node >= model.skeleton_nodes.len()
            && (!model.skins.is_empty() || node.skin.is_some())
        {
            return Err(format!(
                "model node placement {placement_index} references missing skeleton node {}",
                node.node
            ));
        }
        if let Some(skin_index) = node.skin {
            let skin = model.skins.get(skin_index).ok_or_else(|| {
                format!(
                    "model node placement {placement_index} references missing skin {skin_index}"
                )
            })?;
            if skin.joints.is_empty() || skin.joints.len() != skin.inverse_bind_matrices.len() {
                return Err(format!(
                    "skin {skin_index} has inconsistent joint and inverse-bind data"
                ));
            }
            if skin
                .joints
                .iter()
                .any(|joint| *joint >= model.skeleton_nodes.len())
            {
                return Err(format!(
                    "skin {skin_index} references a missing skeleton node"
                ));
            }
            let primitive = &model.primitives[node.primitive];
            for (vertex, (joints, weights)) in
                primitive.joints.iter().zip(&primitive.weights).enumerate()
            {
                for influence in 0..8 {
                    if weights[influence] > 0.0
                        && usize::from(joints[influence]) >= skin.joints.len()
                    {
                        return Err(format!("placement {placement_index} vertex {vertex} joint index {} exceeds skin {skin_index} joint count {}", joints[influence], skin.joints.len()));
                    }
                }
            }
        } else {
            let primitive = &model.primitives[node.primitive];
            if primitive
                .weights
                .iter()
                .any(|weights| weights.iter().any(|weight| *weight > 0.0))
            {
                return Err(format!(
                    "model node placement {placement_index} has weighted vertices but no skin"
                ));
            }
        }
    }
    for (index, image) in model.images.iter().enumerate() {
        let expected = u64::from(image.width)
            .saturating_mul(u64::from(image.height))
            .saturating_mul(4);
        if image.width == 0 || image.height == 0 || expected != image.rgba8.len() as u64 {
            return Err(format!(
                "image {index} has invalid RGBA8 dimensions/data length"
            ));
        }
        validate_prepared_mip_chain(
            index,
            "sRGB",
            image.width,
            image.height,
            image.srgb_mips.as_deref(),
        )?;
        validate_prepared_mip_chain(
            index,
            "linear",
            image.width,
            image.height,
            image.linear_mips.as_deref(),
        )?;
    }
    // A skinned primitive must have a valid fallback bind pose even when the game does not
    // install AnimationPlugin or has not attached a SkeletalAnimationPlayer yet.
    if model.nodes.iter().any(|node| node.skin.is_some()) {
        let _ = model.bind_pose_palettes()?;
    }
    for (index, material) in model.materials.iter().enumerate() {
        if material.base_color.iter().any(|value| !value.is_finite())
            || !material.metallic_factor.is_finite()
            || !material.roughness_factor.is_finite()
            || !material.occlusion_strength.is_finite()
            || material
                .emissive_factor
                .iter()
                .any(|value| !value.is_finite())
        {
            return Err(format!("material {index} contains non-finite factors"));
        }
        if !material.alpha_cutoff.is_finite() {
            return Err(format!("material {index} has a non-finite alpha cutoff"));
        }
        if !(0.0..=1.0).contains(&material.alpha_cutoff) {
            return Err(format!("material {index} alpha cutoff is outside 0..=1"));
        }
        for texture in [
            material.base_color_texture,
            material.metallic_roughness_texture,
            material.normal_texture,
            material.occlusion_texture,
            material.emissive_texture,
        ]
        .into_iter()
        .flatten()
        {
            if texture >= model.images.len() {
                return Err(format!(
                    "material {index} references missing image {texture}"
                ));
            }
        }
    }
    Ok(())
}

fn validate_prepared_mip_chain(
    image_index: usize,
    role: &str,
    base_width: u32,
    base_height: u32,
    levels: Option<&[(u32, u32, Vec<u8>)]>,
) -> Result<(), String> {
    let Some(levels) = levels else {
        // User-created ModelAssets can still use the renderer's compatibility mip generator.
        return Ok(());
    };

    let mut previous_width = base_width;
    let mut previous_height = base_height;
    for (level_index, (width, height, pixels)) in levels.iter().enumerate() {
        if previous_width == 1 && previous_height == 1 {
            return Err(format!(
                "image {image_index} {role} mip chain contains levels after reaching 1x1"
            ));
        }
        let expected_width = (previous_width / 2).max(1);
        let expected_height = (previous_height / 2).max(1);
        let expected_bytes = u64::from(expected_width)
            .saturating_mul(u64::from(expected_height))
            .saturating_mul(4);
        if *width != expected_width || *height != expected_height {
            return Err(format!(
                "image {image_index} {role} mip {level_index} has dimensions {width}x{height}; expected {expected_width}x{expected_height}"
            ));
        }
        if pixels.len() as u64 != expected_bytes {
            return Err(format!(
                "image {image_index} {role} mip {level_index} has {} bytes; expected {expected_bytes}",
                pixels.len()
            ));
        }
        previous_width = *width;
        previous_height = *height;
    }

    if (base_width > 1 || base_height > 1) && (previous_width != 1 || previous_height != 1) {
        return Err(format!(
            "image {image_index} {role} mip chain is incomplete; it ends at {previous_width}x{previous_height} instead of 1x1"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{upload_model, validate_model_for_upload};
    use crate::components::{Camera3D, SunLight, Transform3D};
    use crate::renderer::{Draw, MeshRenderer, Scene};
    use glam::{Mat4, Vec3};
    use ribbon_plugin_gltf::{ModelAsset, ModelNode, PrimitiveData, SkeletonNode, SkinData};
    use ribbon_render::gpu::GpuContext;
    use ribbon_render::gpu_window::clear_view;

    const CUBE: &[u8] = include_bytes!("../../../assets/models/checker_cube.glb");

    #[test]
    fn model_upload_validation_rejects_empty_and_accepts_packaged_models() {
        let empty = ribbon_plugin_gltf::ModelAsset::default();
        assert!(validate_model_for_upload(&empty).is_err());
        let model = ribbon_plugin_gltf::parse(CUBE).expect("packaged model parses");
        validate_model_for_upload(&model).expect("packaged model is uploadable");
    }

    fn minimal_skinned_model(joint_index: u16, weight: f32) -> ModelAsset {
        ModelAsset {
            primitives: vec![PrimitiveData {
                positions: vec![[0.0, 0.0, 0.0]; 3],
                normals: vec![[0.0, 0.0, 1.0]; 3],
                uvs: vec![[0.0, 0.0]; 3],
                indices: vec![0, 1, 2],
                material: None,
                joints: vec![[joint_index, 0, 0, 0, 0, 0, 0, 0]; 3],
                weights: vec![[weight, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]; 3],
            }],
            nodes: vec![ModelNode {
                primitive: 0,
                transform: Mat4::IDENTITY,
                node: 0,
                skin: Some(0),
            }],
            skeleton_nodes: vec![
                SkeletonNode {
                    parent: None,
                    translation: [0.0; 3],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0; 3],
                },
                SkeletonNode {
                    parent: Some(0),
                    translation: [0.0; 3],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0; 3],
                },
            ],
            skins: vec![SkinData {
                joints: vec![1],
                inverse_bind_matrices: vec![Mat4::IDENTITY.to_cols_array_2d()],
            }],
            ..ModelAsset::default()
        }
    }

    #[test]
    fn model_upload_validation_rejects_invalid_alpha_cutoff() {
        let mut model = ribbon_plugin_gltf::parse(CUBE).expect("packaged model parses");
        model.materials[0].alpha_cutoff = f32::NAN;
        let error = validate_model_for_upload(&model)
            .expect_err("non-finite alpha cutoff must be rejected before GPU upload");
        assert!(
            error.contains("material 0 has a non-finite alpha cutoff"),
            "{error}"
        );

        model.materials[0].alpha_cutoff = 1.25;
        let error = validate_model_for_upload(&model)
            .expect_err("alpha cutoff outside glTF range must be rejected");
        assert!(
            error.contains("material 0 alpha cutoff is outside 0..=1"),
            "{error}"
        );
    }

    #[test]
    fn skin_validation_rejects_positive_weights_with_out_of_range_joint_indices() {
        let model = minimal_skinned_model(0, 1.0);
        validate_model_for_upload(&model).expect("valid single-joint skin");

        let invalid = minimal_skinned_model(1, 1.0);
        let error = validate_model_for_upload(&invalid).expect_err("invalid joint must fail");
        assert!(error.contains("exceeds skin 0 joint count 1"), "{error}");
    }

    #[test]
    fn skin_validation_rejects_weighted_vertices_without_a_skin() {
        let mut model = minimal_skinned_model(0, 1.0);
        model.nodes[0].skin = None;
        let error =
            validate_model_for_upload(&model).expect_err("weighted mesh must require a skin");
        assert!(
            error.contains("has weighted vertices but no skin"),
            "{error}"
        );
    }

    #[test]
    fn skin_validation_rejects_tiny_nonzero_unnormalized_weights() {
        // The WGSL path treats every positive weight above 1e-6 total as active. Such a sum must
        // be normalized; it cannot be accepted as an unskinned vertex and then collapse it.
        let invalid = minimal_skinned_model(2, 5.0e-6);
        let error = validate_model_for_upload(&invalid).expect_err("tiny weights must fail");
        assert!(error.contains("skin weights are not normalized"), "{error}");
    }

    #[test]
    fn worker_prepared_color_mips_match_the_renderer_compatibility_filter() {
        let model = ribbon_plugin_gltf::parse(CUBE).expect("packaged model parses");
        let image = &model.images[0];
        let renderer_levels = crate::renderer::build_mips(image.width, image.height, &image.rgba8);
        assert_eq!(
            image.srgb_mips.as_deref(),
            Some(&renderer_levels[1..]),
            "worker-prepared levels must preserve renderer color filtering",
        );
    }

    #[test]
    fn model_upload_validation_rejects_malformed_prepared_mips() {
        let mut model = ribbon_plugin_gltf::parse(CUBE).expect("packaged model parses");
        let levels = model.images[0]
            .srgb_mips
            .as_mut()
            .expect("parser prepares mipmaps");
        levels[0].0 = 3;
        let error = validate_model_for_upload(&model).expect_err("bad mip width must fail");
        assert!(error.contains("sRGB mip 0 has dimensions 3x4"), "{error}");
    }

    #[test]
    #[ignore = "requires a working wgpu adapter; validates model GPU upload and rendering"]
    fn model_upload_does_not_allocate_gpu_meshes_for_unplaced_primitives() {
        let context = match pollster::block_on(GpuContext::new_offscreen(8, 8)) {
            Ok(context) => context,
            Err(error) => {
                eprintln!("skipping GPU resource-lifetime test: {}", error.0);
                return;
            }
        };
        let (device, queue) = (context.device(), context.queue());
        let mut renderer = MeshRenderer::new(device, queue, wgpu::TextureFormat::Rgba8Unorm);
        let before = renderer.resource_stats().live_meshes;
        let mut model = ribbon_plugin_gltf::parse(CUBE).expect("packaged model parses");
        let unused_primitive = model.primitives[0].clone();
        model.primitives.push(unused_primitive);
        assert_eq!(model.nodes.len(), 1);

        let parts = upload_model(&mut renderer, device, queue, &model);
        let after = renderer.resource_stats().live_meshes;
        assert_eq!(parts.len(), 1);
        assert_eq!(after.saturating_sub(before), 1);
    }

    #[test]
    #[ignore = "requires a working wgpu adapter; validates textured glTF rendering"]
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
                camera_rotation: glam::Quat::IDENTITY,
                sun: &SunLight::default(),
                spot_lights: &[],
                point_lights: &[],
                fog: crate::FogSettings::default(),
                draws: &draws,
                particles: &[],
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
