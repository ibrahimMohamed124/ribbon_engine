//! glTF 2.0 (`.glb` and self-contained `.gltf`) loading into CPU-side model data.
//!
//! A [`ModelAsset`] holds deduplicated primitives, materials and decoded images,
//! plus the default scene flattened into [`ModelNode`] placements. GPU upload is
//! the renderer's job. External buffer/image files are not resolved: use `.glb`
//! or embed the data.

use glam::{Mat4, Quat, Vec2, Vec3};
use ribbon_assets::{AssetLoader, AssetMemoryBreakdown, LoadContext, LoadError, LoadErrorKind};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};

/// Indexed triangle geometry with normals, tangent frames, and two texture-coordinate sets.
#[derive(Clone, Debug, PartialEq)]
pub struct PrimitiveData {
    /// Object-space positions.
    pub positions: Vec<[f32; 3]>,
    /// Unit normals (computed when the file has none).
    pub normals: Vec<[f32; 3]>,
    /// First texture-coordinate set; zeros when the file has none.
    pub uvs: Vec<[f32; 2]>,
    /// Second texture-coordinate set; falls back to UV0 when the file has no TEXCOORD_1.
    pub uv1s: Vec<[f32; 2]>,
    /// Object-space tangent xyz and handedness sign in w. Imported from TANGENT or generated
    /// from triangle geometry and UV0 when missing/invalid.
    pub tangents: Vec<[f32; 4]>,
    /// Triangle list indices.
    pub indices: Vec<u32>,
    /// Index into [`ModelAsset::materials`]; `None` selects the default material.
    pub material: Option<usize>,
    /// Up to eight glTF joint indices per vertex (JOINTS_0 and JOINTS_1).
    pub joints: Vec<[u16; 8]>,
    /// Normalized linear-blend skinning weights (WEIGHTS_0 and WEIGHTS_1).
    pub weights: Vec<[f32; 8]>,
}

/// glTF material alpha interpretation used by the renderer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AlphaMode {
    /// Fully opaque; base-color alpha is ignored.
    #[default]
    Opaque,
    /// Alpha-tested coverage; fragments below `alpha_cutoff` are discarded.
    Mask,
    /// Alpha blended in back-to-front order.
    Blend,
}

/// Metallic-roughness material data retained from glTF.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialData {
    /// Linear base color multiplier (RGBA).
    pub base_color: [f32; 4],
    /// Metallic factor in the glTF metallic-roughness workflow.
    pub metallic_factor: f32,
    /// Perceptual roughness factor in the glTF metallic-roughness workflow.
    pub roughness_factor: f32,
    /// Index into [`ModelAsset::images`] for the sRGB base color texture.
    pub base_color_texture: Option<usize>,
    /// TEXCOORD_n selected by the base-color texture (0 or 1; higher sets fall back to 0).
    pub base_color_tex_coord: u32,
    /// Index into [`ModelAsset::images`] for the linear G=roughness/B=metallic texture.
    pub metallic_roughness_texture: Option<usize>,
    /// TEXCOORD_n selected by the metallic-roughness texture.
    pub metallic_roughness_tex_coord: u32,
    /// Index into images for tangent-space normal data.
    pub normal_texture: Option<usize>,
    /// TEXCOORD_n selected by the normal texture.
    pub normal_tex_coord: u32,
    /// Index into images for the red-channel ambient-occlusion texture.
    pub occlusion_texture: Option<usize>,
    /// TEXCOORD_n selected by the occlusion texture.
    pub occlusion_tex_coord: u32,
    /// glTF occlusion strength.
    pub occlusion_strength: f32,
    /// Index into images for emissive color (sRGB).
    pub emissive_texture: Option<usize>,
    /// TEXCOORD_n selected by the emissive texture.
    pub emissive_tex_coord: u32,
    /// Linear emissive multiplier.
    pub emissive_factor: [f32; 3],
    /// glTF alpha compositing mode. Missing values default to OPAQUE.
    pub alpha_mode: AlphaMode,
    /// Alpha-test threshold used only by MASK materials (glTF default 0.5).
    pub alpha_cutoff: f32,
    /// Render both triangle orientations and flip back-face normals for lighting.
    pub double_sided: bool,
}

/// Decoded 8-bit RGBA image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageData {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Tightly packed RGBA8 pixels, row-major from the top-left.
    pub rgba8: Vec<u8>,
    /// Mip levels prepared during AssetServer decoding for sRGB material roles.
    /// Level zero is omitted because it is already stored in `rgba8`.
    pub srgb_mips: Option<Vec<(u32, u32, Vec<u8>)>>,
    /// Mip levels prepared during AssetServer decoding for linear-data material roles.
    /// Level zero is omitted because it is already stored in `rgba8`.
    pub linear_mips: Option<Vec<(u32, u32, Vec<u8>)>>,
}

/// One primitive placed in model space by the node hierarchy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelNode {
    /// Index into [`ModelAsset::primitives`].
    pub primitive: usize,
    /// Model-space matrix accumulated from the node hierarchy.
    pub transform: Mat4,
    /// Original glTF node index. Used to resolve animated node transforms.
    pub node: usize,
    /// Optional skin index from the node.
    pub skin: Option<usize>,
}

/// A node in the imported glTF transform hierarchy, retained for skinning and animation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkeletonNode {
    pub parent: Option<usize>,
    pub translation: [f32; 3],
    /// glTF quaternion component order (x, y, z, w).
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

/// Joint list and inverse-bind matrices for one glTF skin.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkinData {
    /// glTF node indices, in the exact order expected by JOINTS vertex attributes.
    pub joints: Vec<usize>,
    /// Inverse bind matrix for each joint, in the same order as `joints`.
    pub inverse_bind_matrices: Vec<[[f32; 4]; 4]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimationProperty {
    Translation,
    Rotation,
    Scale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimationInterpolation {
    Linear,
    Step,
    CubicSpline,
}

/// One transform channel targeting a single glTF node.
/// Values are vec4: translation/scale use xyz and rotation uses xyzw.
/// For CUBICSPLINE, tangents are retained separately as required by glTF Hermite evaluation.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimationTrack {
    pub node: usize,
    pub property: AnimationProperty,
    pub interpolation: AnimationInterpolation,
    pub times: Vec<f32>,
    pub values: Vec<[f32; 4]>,
    pub in_tangents: Vec<[f32; 4]>,
    pub out_tangents: Vec<[f32; 4]>,
}

/// CPU-side skeletal transform clip imported from glTF animation channels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SkeletalAnimationClip {
    pub name: String,
    pub duration_seconds: f32,
    pub tracks: Vec<AnimationTrack>,
}

/// A loaded glTF scene.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelAsset {
    /// Unique primitives.
    pub primitives: Vec<PrimitiveData>,
    /// Materials in file order.
    pub materials: Vec<MaterialData>,
    /// Images in file order.
    pub images: Vec<ImageData>,
    /// Placements of primitives in model space. Each placement keeps its source node and skin.
    pub nodes: Vec<ModelNode>,
    /// All document nodes, indexed by original glTF node index.
    pub skeleton_nodes: Vec<SkeletonNode>,
    /// Imported skins and inverse-bind matrices.
    pub skins: Vec<SkinData>,
    /// Imported node TRS animation clips. Morph-target animation is not included here.
    pub skeletal_animations: Vec<SkeletalAnimationClip>,
}

impl ModelAsset {
    /// Resolve every glTF node's bind-pose global transform, indexed by original node index.
    pub fn bind_pose_node_transforms(&self) -> Result<Vec<Mat4>, String> {
        let local: Vec<Mat4> = self
            .skeleton_nodes
            .iter()
            .enumerate()
            .map(|(index, node)| {
                let translation = Vec3::from_array(node.translation);
                let scale = Vec3::from_array(node.scale);
                let rotation = Quat::from_xyzw(
                    node.rotation[0],
                    node.rotation[1],
                    node.rotation[2],
                    node.rotation[3],
                );
                if !translation.is_finite()
                    || !scale.is_finite()
                    || !rotation.is_finite()
                    || rotation.length_squared() <= 1.0e-12
                {
                    return Err(format!(
                        "skeleton node {index} has invalid bind-pose TRS data"
                    ));
                }
                Ok(Mat4::from_scale_rotation_translation(
                    scale,
                    rotation.normalize(),
                    translation,
                ))
            })
            .collect::<Result<_, _>>()?;

        fn resolve(
            index: usize,
            nodes: &[SkeletonNode],
            local: &[Mat4],
            state: &mut [u8],
            global: &mut [Mat4],
        ) -> Result<Mat4, String> {
            match state[index] {
                2 => return Ok(global[index]),
                1 => {
                    return Err(format!(
                        "cycle detected in bind-pose hierarchy at node {index}"
                    ))
                }
                _ => {}
            }
            state[index] = 1;
            let matrix = if let Some(parent) = nodes[index].parent {
                if parent >= nodes.len() {
                    return Err(format!(
                        "skeleton node {index} references missing parent {parent}"
                    ));
                }
                resolve(parent, nodes, local, state, global)? * local[index]
            } else {
                local[index]
            };
            if !matrix.to_cols_array().iter().all(|value| value.is_finite()) {
                return Err(format!(
                    "skeleton node {index} produced a non-finite bind transform"
                ));
            }
            global[index] = matrix;
            state[index] = 2;
            Ok(matrix)
        }

        let mut state = vec![0_u8; local.len()];
        let mut global = vec![Mat4::IDENTITY; local.len()];
        for index in 0..local.len() {
            let _ = resolve(index, &self.skeleton_nodes, &local, &mut state, &mut global)?;
        }
        Ok(global)
    }

    /// Build the skin joint palettes for the imported bind pose. The result is indexed by
    /// `ModelAsset::nodes`; matrices are mesh-local: inverse(mesh global) * joint global * inverse-bind.
    pub fn bind_pose_palettes(&self) -> Result<Vec<Option<Arc<[Mat4]>>>, String> {
        if self.skeleton_nodes.is_empty() {
            if self.nodes.iter().any(|node| node.skin.is_some()) {
                return Err("skinned model has no retained node hierarchy".to_owned());
            }
            return Ok(vec![None; self.nodes.len()]);
        }
        let global = self.bind_pose_node_transforms()?;
        let mut palettes = vec![None; self.nodes.len()];
        for (placement_index, placement) in self.nodes.iter().enumerate() {
            let Some(skin_index) = placement.skin else {
                continue;
            };
            let skin = self.skins.get(skin_index).ok_or_else(|| {
                format!("placement {placement_index} references missing skin {skin_index}")
            })?;
            if skin.joints.is_empty() || skin.joints.len() != skin.inverse_bind_matrices.len() {
                return Err(format!(
                    "skin {skin_index} has inconsistent joint/inverse-bind data"
                ));
            }
            let mesh_global = *global.get(placement.node).ok_or_else(|| {
                format!(
                    "placement {placement_index} references missing mesh node {}",
                    placement.node
                )
            })?;
            let determinant = mesh_global.determinant();
            if !determinant.is_finite() || determinant.abs() < 1.0e-8 {
                return Err(format!(
                    "placement {placement_index} has a singular mesh transform"
                ));
            }
            let inverse_mesh = mesh_global.inverse();
            let mut palette = Vec::with_capacity(skin.joints.len());
            for (joint_node, inverse_bind) in skin.joints.iter().zip(&skin.inverse_bind_matrices) {
                let joint_global = *global.get(*joint_node).ok_or_else(|| {
                    format!("skin {skin_index} references missing joint node {joint_node}")
                })?;
                let matrix = inverse_mesh * joint_global * Mat4::from_cols_array_2d(inverse_bind);
                if !matrix.to_cols_array().iter().all(|value| value.is_finite()) {
                    return Err(format!(
                        "skin {skin_index} produced a non-finite bind palette"
                    ));
                }
                palette.push(matrix);
            }
            palettes[placement_index] = Some(Arc::<[Mat4]>::from(palette.into_boxed_slice()));
        }
        Ok(palettes)
    }

    /// Estimate currently allocated CPU bytes, including nested vector capacities.
    ///
    /// This is a cache-budget estimate rather than exact allocator telemetry; it excludes decoder
    /// scratch memory, the original compressed file bytes, and allocator bookkeeping.
    pub fn estimated_size_bytes(&self) -> u64 {
        let mut bytes = std::mem::size_of::<Self>() as u64;
        bytes = bytes.saturating_add(allocated_vec_bytes::<PrimitiveData>(
            self.primitives.capacity(),
        ));
        bytes = bytes.saturating_add(allocated_vec_bytes::<MaterialData>(
            self.materials.capacity(),
        ));
        bytes = bytes.saturating_add(allocated_vec_bytes::<ImageData>(self.images.capacity()));
        bytes = bytes.saturating_add(allocated_vec_bytes::<ModelNode>(self.nodes.capacity()));
        bytes = bytes.saturating_add(allocated_vec_bytes::<SkeletonNode>(
            self.skeleton_nodes.capacity(),
        ));
        bytes = bytes.saturating_add(allocated_vec_bytes::<SkinData>(self.skins.capacity()));
        bytes = bytes.saturating_add(allocated_vec_bytes::<SkeletalAnimationClip>(
            self.skeletal_animations.capacity(),
        ));
        for skin in &self.skins {
            bytes = bytes.saturating_add(allocated_vec_bytes::<usize>(skin.joints.capacity()));
            bytes = bytes.saturating_add(allocated_vec_bytes::<[[f32; 4]; 4]>(
                skin.inverse_bind_matrices.capacity(),
            ));
        }
        for clip in &self.skeletal_animations {
            bytes = bytes.saturating_add(clip.name.capacity() as u64);
            bytes = bytes.saturating_add(allocated_vec_bytes::<AnimationTrack>(
                clip.tracks.capacity(),
            ));
            for track in &clip.tracks {
                bytes = bytes.saturating_add(allocated_vec_bytes::<f32>(track.times.capacity()));
                bytes =
                    bytes.saturating_add(allocated_vec_bytes::<[f32; 4]>(track.values.capacity()));
                bytes = bytes.saturating_add(allocated_vec_bytes::<[f32; 4]>(
                    track.in_tangents.capacity(),
                ));
                bytes = bytes.saturating_add(allocated_vec_bytes::<[f32; 4]>(
                    track.out_tangents.capacity(),
                ));
            }
        }
        for primitive in &self.primitives {
            bytes = bytes.saturating_add(allocated_vec_bytes::<[f32; 3]>(
                primitive.positions.capacity(),
            ));
            bytes = bytes.saturating_add(allocated_vec_bytes::<[f32; 3]>(
                primitive.normals.capacity(),
            ));
            bytes = bytes
                .saturating_add(allocated_vec_bytes::<[f32; 2]>(primitive.uvs.capacity()))
                .saturating_add(allocated_vec_bytes::<[f32; 2]>(primitive.uv1s.capacity()))
                .saturating_add(allocated_vec_bytes::<[f32; 4]>(
                    primitive.tangents.capacity(),
                ));
            bytes = bytes.saturating_add(allocated_vec_bytes::<u32>(primitive.indices.capacity()));
            bytes =
                bytes.saturating_add(allocated_vec_bytes::<[u16; 8]>(primitive.joints.capacity()));
            bytes = bytes.saturating_add(allocated_vec_bytes::<[f32; 8]>(
                primitive.weights.capacity(),
            ));
        }
        for image in &self.images {
            bytes = bytes.saturating_add(image.rgba8.capacity() as u64);
            for levels in [&image.srgb_mips, &image.linear_mips].into_iter().flatten() {
                bytes = bytes.saturating_add(
                    (levels.capacity() as u64)
                        .saturating_mul(std::mem::size_of::<(u32, u32, Vec<u8>)>() as u64),
                );
                for (_, _, pixels) in levels {
                    bytes = bytes.saturating_add(pixels.capacity() as u64);
                }
            }
        }
        bytes
    }

    /// Categorize this model's decoded CPU residency using vector capacities. These are cache
    /// estimates rather than allocator telemetry. Imported skeleton and inverse-bind data is
    /// scene metadata; transform tracks are counted as animation memory.
    pub fn memory_breakdown(&self) -> AssetMemoryBreakdown {
        let mut breakdown = AssetMemoryBreakdown {
            scene_metadata_bytes: (std::mem::size_of::<Self>() as u64)
                .saturating_add(allocated_vec_bytes::<ModelNode>(self.nodes.capacity()))
                .saturating_add(allocated_vec_bytes::<SkeletonNode>(
                    self.skeleton_nodes.capacity(),
                ))
                .saturating_add(allocated_vec_bytes::<SkinData>(self.skins.capacity())),
            animation_bytes: allocated_vec_bytes::<SkeletalAnimationClip>(
                self.skeletal_animations.capacity(),
            ),
            ..AssetMemoryBreakdown::default()
        };
        breakdown.geometry_bytes = allocated_vec_bytes::<PrimitiveData>(self.primitives.capacity());
        for primitive in &self.primitives {
            breakdown.geometry_bytes = breakdown
                .geometry_bytes
                .saturating_add(allocated_vec_bytes::<[f32; 3]>(
                    primitive.positions.capacity(),
                ))
                .saturating_add(allocated_vec_bytes::<[f32; 3]>(
                    primitive.normals.capacity(),
                ))
                .saturating_add(allocated_vec_bytes::<[f32; 2]>(primitive.uvs.capacity()))
                .saturating_add(allocated_vec_bytes::<[f32; 2]>(primitive.uv1s.capacity()))
                .saturating_add(allocated_vec_bytes::<[f32; 4]>(
                    primitive.tangents.capacity(),
                ))
                .saturating_add(allocated_vec_bytes::<u32>(primitive.indices.capacity()))
                .saturating_add(allocated_vec_bytes::<[u16; 8]>(primitive.joints.capacity()))
                .saturating_add(allocated_vec_bytes::<[f32; 8]>(
                    primitive.weights.capacity(),
                ));
        }
        for skin in &self.skins {
            breakdown.scene_metadata_bytes = breakdown
                .scene_metadata_bytes
                .saturating_add(allocated_vec_bytes::<usize>(skin.joints.capacity()))
                .saturating_add(allocated_vec_bytes::<[[f32; 4]; 4]>(
                    skin.inverse_bind_matrices.capacity(),
                ));
        }
        for clip in &self.skeletal_animations {
            breakdown.animation_bytes = breakdown
                .animation_bytes
                .saturating_add(clip.name.capacity() as u64)
                .saturating_add(allocated_vec_bytes::<AnimationTrack>(
                    clip.tracks.capacity(),
                ));
            for track in &clip.tracks {
                breakdown.animation_bytes = breakdown
                    .animation_bytes
                    .saturating_add(allocated_vec_bytes::<f32>(track.times.capacity()))
                    .saturating_add(allocated_vec_bytes::<[f32; 4]>(track.values.capacity()))
                    .saturating_add(allocated_vec_bytes::<[f32; 4]>(
                        track.in_tangents.capacity(),
                    ))
                    .saturating_add(allocated_vec_bytes::<[f32; 4]>(
                        track.out_tangents.capacity(),
                    ));
            }
        }
        breakdown.material_bytes = allocated_vec_bytes::<MaterialData>(self.materials.capacity());
        breakdown.texture_bytes = allocated_vec_bytes::<ImageData>(self.images.capacity());
        for image in &self.images {
            breakdown.texture_bytes = breakdown
                .texture_bytes
                .saturating_add(image.rgba8.capacity() as u64);
            for levels in [&image.srgb_mips, &image.linear_mips].into_iter().flatten() {
                breakdown.texture_bytes = breakdown.texture_bytes.saturating_add(
                    (levels.capacity() as u64)
                        .saturating_mul(std::mem::size_of::<(u32, u32, Vec<u8>)>() as u64),
                );
                for (_, _, pixels) in levels {
                    breakdown.texture_bytes = breakdown
                        .texture_bytes
                        .saturating_add(pixels.capacity() as u64);
                }
            }
        }
        breakdown
    }

    /// Estimate bytes the current renderer upload path will submit for this model. This is a
    /// pacing estimate, not driver VRAM telemetry. It accounts for separate 56-byte static and
    /// 104-byte skinned vertex streams, duplicated only when a primitive is placed by both node
    /// kinds, plus the index buffers, referenced materials, and prepared image mip variants.
    pub fn estimated_gpu_upload_bytes(&self) -> u64 {
        let mut bytes = 0_u64;
        let mut static_primitive_used = vec![false; self.primitives.len()];
        let mut skinned_primitive_used = vec![false; self.primitives.len()];
        for node in &self.nodes {
            if let Some(used) = if node.skin.is_some() {
                skinned_primitive_used.get_mut(node.primitive)
            } else {
                static_primitive_used.get_mut(node.primitive)
            } {
                *used = true;
            }
        }
        for (index, primitive) in self.primitives.iter().enumerate() {
            let static_used = static_primitive_used[index];
            let skinned_used = skinned_primitive_used[index];
            if !static_used && !skinned_used {
                continue;
            }
            let vertices = primitive.positions.len() as u64;
            let mut vertex_stride = 0_u64;
            let mut layout_count = 0_u64;
            if static_used {
                vertex_stride = vertex_stride
                    .saturating_add(std::mem::size_of::<[f32; 3]>() as u64 * 2)
                    .saturating_add(std::mem::size_of::<[f32; 2]>() as u64 * 2)
                    .saturating_add(std::mem::size_of::<[f32; 4]>() as u64);
                layout_count += 1;
            }
            if skinned_used {
                vertex_stride = vertex_stride
                    .saturating_add(std::mem::size_of::<[f32; 3]>() as u64 * 2)
                    .saturating_add(std::mem::size_of::<[f32; 2]>() as u64 * 2)
                    .saturating_add(std::mem::size_of::<[f32; 4]>() as u64)
                    .saturating_add(std::mem::size_of::<[u16; 8]>() as u64)
                    .saturating_add(std::mem::size_of::<[f32; 8]>() as u64);
                layout_count += 1;
            }
            bytes = bytes.saturating_add(vertices.saturating_mul(vertex_stride));
            bytes = bytes.saturating_add(
                (primitive.indices.len() as u64)
                    .saturating_mul(std::mem::size_of::<u32>() as u64)
                    .saturating_mul(layout_count),
            );
        }

        let mut used_materials = std::collections::HashSet::new();
        let mut used_images: HashMap<usize, (bool, bool)> = HashMap::new();
        for node in &self.nodes {
            let Some(primitive) = self.primitives.get(node.primitive) else {
                continue;
            };
            let Some(material_index) = primitive.material else {
                continue;
            };
            if !used_materials.insert(material_index) {
                continue;
            }
            let Some(material) = self.materials.get(material_index) else {
                continue;
            };
            // The renderer creates one compact uniform per used material. Its texture cache key
            // includes the transfer function, so a source image used both as sRGB and linear data
            // may consume two GPU allocations, while repeated uses within one role are shared.
            bytes = bytes.saturating_add(256);
            for image_index in [material.base_color_texture, material.emissive_texture]
                .into_iter()
                .flatten()
            {
                used_images.entry(image_index).or_default().0 = true;
            }
            for image_index in [
                material.metallic_roughness_texture,
                material.normal_texture,
                material.occlusion_texture,
            ]
            .into_iter()
            .flatten()
            {
                used_images.entry(image_index).or_default().1 = true;
            }
        }

        for (index, (uses_srgb, uses_linear)) in used_images {
            let Some(image) = self.images.get(index) else {
                continue;
            };
            let base_bytes = image.rgba8.len() as u64;
            if uses_srgb {
                bytes = bytes.saturating_add(base_bytes);
                bytes = bytes.saturating_add(estimated_mip_upload_bytes(
                    image.width,
                    image.height,
                    image.srgb_mips.as_deref(),
                ));
            }
            if uses_linear {
                bytes = bytes.saturating_add(base_bytes);
                bytes = bytes.saturating_add(estimated_mip_upload_bytes(
                    image.width,
                    image.height,
                    image.linear_mips.as_deref(),
                ));
            }
        }
        bytes
    }

    /// Axis-aligned bounds of every placed vertex, or `None` for an empty model.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for node in &self.nodes {
            for position in &self.primitives[node.primitive].positions {
                let point = node.transform.transform_point3(Vec3::from(*position));
                min = min.min(point);
                max = max.max(point);
            }
        }
        (min.x <= max.x).then_some((min, max))
    }
}

fn estimated_mip_upload_bytes(
    base_width: u32,
    base_height: u32,
    levels: Option<&[(u32, u32, Vec<u8>)]>,
) -> u64 {
    if let Some(levels) = levels {
        return levels.iter().fold(0_u64, |total, (_, _, pixels)| {
            total.saturating_add(pixels.len() as u64)
        });
    }

    // A caller-created ModelAsset can omit precomputed levels; match the renderer compatibility
    // path exactly rather than charging a blanket one-third for 1x1 or odd-sized textures.
    let mut width = base_width;
    let mut height = base_height;
    let mut bytes = 0_u64;
    while width > 1 || height > 1 {
        width = (width / 2).max(1);
        height = (height / 2).max(1);
        bytes = bytes.saturating_add(
            u64::from(width)
                .saturating_mul(u64::from(height))
                .saturating_mul(4),
        );
    }
    bytes
}

fn allocated_vec_bytes<T>(capacity: usize) -> u64 {
    (capacity as u64).saturating_mul(std::mem::size_of::<T>() as u64)
}

/// Loader registered for `glb` and `gltf` files.
pub struct GltfLoader;

impl AssetLoader for GltfLoader {
    type Asset = ModelAsset;

    fn extensions(&self) -> &[&str] {
        &["glb", "gltf"]
    }

    fn load(&self, bytes: &[u8], context: &mut LoadContext) -> Result<ModelAsset, LoadError> {
        parse(bytes)
            .map_err(|message| LoadError::new(LoadErrorKind::Decode, &context.path, message))
    }

    fn estimated_size_bytes(&self, asset: &Self::Asset) -> u64 {
        asset.estimated_size_bytes()
    }

    fn memory_breakdown(&self, asset: &Self::Asset) -> AssetMemoryBreakdown {
        asset.memory_breakdown()
    }
}

/// Decode glTF bytes into a [`ModelAsset`].
pub fn parse(bytes: &[u8]) -> Result<ModelAsset, String> {
    let (document, buffers, images) = gltf::import_slice(bytes).map_err(|e| e.to_string())?;
    let mut model = ModelAsset {
        skeleton_nodes: build_skeleton_nodes(&document),
        skins: build_skins(&document, &buffers)?,
        skeletal_animations: build_skeletal_animations(&document, &buffers)?,
        ..ModelAsset::default()
    };
    for image in &images {
        model.images.push(to_rgba8(image)?);
    }
    for material in document.materials() {
        let pbr = material.pbr_metallic_roughness();
        model.materials.push(MaterialData {
            base_color: pbr.base_color_factor(),
            metallic_factor: pbr.metallic_factor(),
            roughness_factor: pbr.roughness_factor(),
            base_color_texture: pbr
                .base_color_texture()
                .map(|info| info.texture().source().index()),
            base_color_tex_coord: pbr.base_color_texture().map_or(0, |info| info.tex_coord()),
            metallic_roughness_texture: pbr
                .metallic_roughness_texture()
                .map(|info| info.texture().source().index()),
            metallic_roughness_tex_coord: pbr
                .metallic_roughness_texture()
                .map_or(0, |info| info.tex_coord()),
            normal_texture: material
                .normal_texture()
                .map(|info| info.texture().source().index()),
            normal_tex_coord: material.normal_texture().map_or(0, |info| info.tex_coord()),
            occlusion_texture: material
                .occlusion_texture()
                .map(|info| info.texture().source().index()),
            occlusion_tex_coord: material
                .occlusion_texture()
                .map_or(0, |info| info.tex_coord()),
            occlusion_strength: material
                .occlusion_texture()
                .map_or(1.0, |info| info.strength()),
            emissive_texture: material
                .emissive_texture()
                .map(|info| info.texture().source().index()),
            emissive_tex_coord: material
                .emissive_texture()
                .map_or(0, |info| info.tex_coord()),
            emissive_factor: material.emissive_factor(),
            alpha_mode: match material.alpha_mode() {
                gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
                gltf::material::AlphaMode::Mask => AlphaMode::Mask,
                gltf::material::AlphaMode::Blend => AlphaMode::Blend,
            },
            alpha_cutoff: material.alpha_cutoff().unwrap_or(0.5),
            double_sided: material.double_sided(),
        });
    }
    let scene = document
        .default_scene()
        .or_else(|| document.scenes().next())
        .ok_or("file contains no scene")?;
    let mut cache = HashMap::new();
    for node in scene.nodes() {
        visit(&node, Mat4::IDENTITY, &buffers, &mut model, &mut cache)?;
    }
    prepare_material_image_mips(&mut model);
    Ok(model)
}

const IMAGE_MIP_LUT_STEPS: usize = 16_384;

struct ImageMipLookup {
    srgb_to_linear: [f32; 256],
    linear_to_srgb: Box<[u8]>,
}

fn image_mip_lookup() -> &'static ImageMipLookup {
    static LOOKUP: OnceLock<ImageMipLookup> = OnceLock::new();
    LOOKUP.get_or_init(|| {
        let srgb_to_linear = std::array::from_fn(|index| {
            let value = index as f32 / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        });
        let linear_to_srgb = (0..=IMAGE_MIP_LUT_STEPS)
            .map(|index| {
                let value = index as f32 / IMAGE_MIP_LUT_STEPS as f32;
                let encoded = if value <= 0.003_130_8 {
                    value * 12.92
                } else {
                    1.055 * value.powf(1.0 / 2.4) - 0.055
                };
                (encoded.clamp(0.0, 1.0) * 255.0).round() as u8
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        ImageMipLookup {
            srgb_to_linear,
            linear_to_srgb,
        }
    })
}

/// Prepare only image color-space variants referenced by the model's materials.
/// When called through `GltfLoader`, this runs in the AssetServer decode worker, not the renderer.
fn prepare_material_image_mips(model: &mut ModelAsset) {
    let used_materials: HashSet<usize> = model
        .nodes
        .iter()
        .filter_map(|node| model.primitives.get(node.primitive)?.material)
        .collect();
    let mut srgb_used = vec![false; model.images.len()];
    let mut linear_used = vec![false; model.images.len()];
    for material_index in used_materials {
        let Some(material) = model.materials.get(material_index) else {
            continue;
        };
        for index in [material.base_color_texture, material.emissive_texture]
            .into_iter()
            .flatten()
        {
            if let Some(used) = srgb_used.get_mut(index) {
                *used = true;
            }
        }
        for index in [
            material.metallic_roughness_texture,
            material.normal_texture,
            material.occlusion_texture,
        ]
        .into_iter()
        .flatten()
        {
            if let Some(used) = linear_used.get_mut(index) {
                *used = true;
            }
        }
    }

    for (index, image) in model.images.iter_mut().enumerate() {
        if srgb_used[index] && (image.width > 1 || image.height > 1) {
            image.srgb_mips = Some(build_image_mips(
                image.width,
                image.height,
                &image.rgba8,
                true,
            ));
        }
        if linear_used[index] && (image.width > 1 || image.height > 1) {
            image.linear_mips = Some(build_image_mips(
                image.width,
                image.height,
                &image.rgba8,
                false,
            ));
        }
    }
}

/// Return generated levels after level zero. Color textures are filtered in linear light;
/// data textures are averaged directly in their stored linear channel values.
fn build_image_mips(width: u32, height: u32, rgba8: &[u8], srgb: bool) -> Vec<(u32, u32, Vec<u8>)> {
    let lookup = srgb.then(image_mip_lookup);
    let mut previous_width = width;
    let mut previous_height = height;
    let mut levels: Vec<(u32, u32, Vec<u8>)> = Vec::new();

    while previous_width > 1 || previous_height > 1 {
        let previous = levels
            .last()
            .map_or(rgba8, |(_, _, pixels)| pixels.as_slice());
        let next_width = (previous_width / 2).max(1);
        let next_height = (previous_height / 2).max(1);
        let mut next = Vec::with_capacity((next_width * next_height * 4) as usize);
        for y in 0..next_height {
            for x in 0..next_width {
                let mut color = [0.0_f32; 3];
                let mut channels = [0_u32; 4];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let sx = (x * 2 + dx).min(previous_width - 1);
                    let sy = (y * 2 + dy).min(previous_height - 1);
                    let offset = ((sy * previous_width + sx) * 4) as usize;
                    if let Some(lookup) = lookup {
                        for channel in 0..3 {
                            color[channel] +=
                                lookup.srgb_to_linear[previous[offset + channel] as usize];
                        }
                    } else {
                        for channel in 0..3 {
                            channels[channel] += u32::from(previous[offset + channel]);
                        }
                    }
                    channels[3] += u32::from(previous[offset + 3]);
                }
                if let Some(lookup) = lookup {
                    for linear_sum in color {
                        let average = (linear_sum * 0.25).clamp(0.0, 1.0);
                        let lut_index = (average * IMAGE_MIP_LUT_STEPS as f32).round() as usize;
                        next.push(lookup.linear_to_srgb[lut_index.min(IMAGE_MIP_LUT_STEPS)]);
                    }
                } else {
                    for sum in channels.iter().take(3) {
                        next.push(((sum + 2) / 4) as u8);
                    }
                }
                next.push(((channels[3] + 2) / 4) as u8);
            }
        }
        levels.push((next_width, next_height, next));
        previous_width = next_width;
        previous_height = next_height;
    }
    levels
}

fn visit(
    node: &gltf::Node<'_>,
    parent: Mat4,
    buffers: &[gltf::buffer::Data],
    model: &mut ModelAsset,
    cache: &mut HashMap<(usize, usize), Option<usize>>,
) -> Result<(), String> {
    let world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
    if let Some(mesh) = node.mesh() {
        for primitive in mesh.primitives() {
            let key = (mesh.index(), primitive.index());
            let slot = match cache.get(&key) {
                Some(slot) => *slot,
                None => {
                    let built = build_primitive(&primitive, buffers)?;
                    let slot = built.map(|data| {
                        model.primitives.push(data);
                        model.primitives.len() - 1
                    });
                    cache.insert(key, slot);
                    slot
                }
            };
            if let Some(primitive) = slot {
                model.nodes.push(ModelNode {
                    primitive,
                    transform: world,
                    node: node.index(),
                    skin: node.skin().map(|skin| skin.index()),
                });
            }
        }
    }
    for child in node.children() {
        visit(&child, world, buffers, model, cache)?;
    }
    Ok(())
}

fn build_primitive(
    primitive: &gltf::Primitive<'_>,
    buffers: &[gltf::buffer::Data],
) -> Result<Option<PrimitiveData>, String> {
    if primitive.mode() != gltf::mesh::Mode::Triangles {
        return Ok(None);
    }
    let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));
    let positions: Vec<[f32; 3]> = reader
        .read_positions()
        .ok_or("primitive has no POSITION attribute")?
        .collect();
    let indices: Vec<u32> = match reader.read_indices() {
        Some(indices) => indices.into_u32().collect(),
        None => (0..positions.len() as u32).collect(),
    };
    if !indices.len().is_multiple_of(3) || indices.iter().any(|&i| i as usize >= positions.len()) {
        return Err("primitive indices are out of range".to_owned());
    }
    let normals = match reader.read_normals() {
        Some(normals) => normals.collect(),
        None => smooth_normals(&positions, &indices),
    };
    let uvs = match reader.read_tex_coords(0) {
        Some(coords) => coords.into_f32().collect(),
        None => vec![[0.0, 0.0]; positions.len()],
    };
    let uv1s = match reader.read_tex_coords(1) {
        Some(coords) => coords.into_f32().collect(),
        // Missing UV1 safely aliases UV0, preserving existing material behavior for assets that
        // request TEXCOORD_1 without actually providing it.
        None => uvs.clone(),
    };
    if normals.len() != positions.len()
        || uvs.len() != positions.len()
        || uv1s.len() != positions.len()
    {
        return Err("vertex attribute counts do not match".to_owned());
    }
    let tangents = match reader.read_tangents() {
        Some(source) => {
            let imported: Vec<[f32; 4]> = source.collect();
            if imported.len() == positions.len()
                && imported.iter().all(|tangent| {
                    tangent.iter().all(|value| value.is_finite())
                        && Vec3::new(tangent[0], tangent[1], tangent[2]).length_squared() > 1.0e-12
                        && tangent[3].abs() > 1.0e-6
                })
            {
                imported
                    .into_iter()
                    .map(|tangent| {
                        let direction = Vec3::new(tangent[0], tangent[1], tangent[2]).normalize();
                        [
                            direction.x,
                            direction.y,
                            direction.z,
                            if tangent[3] < 0.0 { -1.0 } else { 1.0 },
                        ]
                    })
                    .collect()
            } else {
                generate_tangents(&positions, &normals, &uvs, &indices)
            }
        }
        None => generate_tangents(&positions, &normals, &uvs, &indices),
    };
    let (joints, weights) = read_skinning_attributes(&reader, positions.len())?;
    Ok(Some(PrimitiveData {
        positions,
        normals,
        uvs,
        uv1s,
        tangents,
        indices,
        material: primitive.material().index(),
        joints,
        weights,
    }))
}

fn generate_tangents(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    indices: &[u32],
) -> Vec<[f32; 4]> {
    let mut tangent_sum = vec![Vec3::ZERO; positions.len()];
    let mut bitangent_sum = vec![Vec3::ZERO; positions.len()];
    for triangle in indices.as_chunks::<3>().0 {
        let [i0, i1, i2] = [
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize,
        ];
        if [i0, i1, i2]
            .iter()
            .any(|&index| index >= positions.len() || index >= normals.len() || index >= uvs.len())
        {
            continue;
        }
        let p0 = Vec3::from_array(positions[i0]);
        let p1 = Vec3::from_array(positions[i1]);
        let p2 = Vec3::from_array(positions[i2]);
        let uv0 = Vec2::from_array(uvs[i0]);
        let uv1 = Vec2::from_array(uvs[i1]);
        let uv2 = Vec2::from_array(uvs[i2]);
        let edge1 = p1 - p0;
        let edge2 = p2 - p0;
        let delta1 = uv1 - uv0;
        let delta2 = uv2 - uv0;
        let determinant = delta1.x * delta2.y - delta1.y * delta2.x;
        if !determinant.is_finite() || determinant.abs() <= 1.0e-8 {
            continue;
        }
        let inverse = determinant.recip();
        let tangent = (edge1 * delta2.y - edge2 * delta1.y) * inverse;
        let bitangent = (edge2 * delta1.x - edge1 * delta2.x) * inverse;
        if !tangent.is_finite() || !bitangent.is_finite() {
            continue;
        }
        let area = edge1.cross(edge2).length().max(1.0e-8);
        for index in [i0, i1, i2] {
            tangent_sum[index] += tangent * area;
            bitangent_sum[index] += bitangent * area;
        }
    }
    (0..positions.len())
        .map(|index| {
            let normal = normals
                .get(index)
                .copied()
                .map(Vec3::from_array)
                .filter(|normal| normal.is_finite() && normal.length_squared() > 1.0e-12)
                .map(Vec3::normalize)
                .unwrap_or(Vec3::Z);
            let projected = tangent_sum[index] - normal * normal.dot(tangent_sum[index]);
            let tangent = if projected.is_finite() && projected.length_squared() > 1.0e-12 {
                projected.normalize()
            } else {
                let axis = if normal.y.abs() < 0.9 {
                    Vec3::Y
                } else {
                    Vec3::X
                };
                axis.cross(normal).normalize_or_zero()
            };
            let tangent = if tangent.length_squared() > 1.0e-12 {
                tangent
            } else {
                Vec3::X
            };
            let handedness = if bitangent_sum[index].is_finite()
                && normal.cross(tangent).dot(bitangent_sum[index]) < 0.0
            {
                -1.0
            } else {
                1.0
            };
            [tangent.x, tangent.y, tangent.z, handedness]
        })
        .collect()
}

fn build_skeleton_nodes(document: &gltf::Document) -> Vec<SkeletonNode> {
    let nodes: Vec<_> = document.nodes().collect();
    let mut parents = vec![None; nodes.len()];
    for node in &nodes {
        for child in node.children() {
            if let Some(parent) = parents.get_mut(child.index()) {
                *parent = Some(node.index());
            }
        }
    }
    nodes
        .into_iter()
        .map(|node| {
            let (translation, rotation, scale) = node.transform().decomposed();
            SkeletonNode {
                parent: parents.get(node.index()).copied().flatten(),
                translation,
                rotation,
                scale,
            }
        })
        .collect()
}

fn build_skins(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
) -> Result<Vec<SkinData>, String> {
    document
        .skins()
        .map(|skin| {
            let joints: Vec<usize> = skin.joints().map(|joint| joint.index()).collect();
            if joints.is_empty() {
                return Err(format!("skin {} has no joints", skin.index()));
            }
            let inverse_bind_matrices: Vec<[[f32; 4]; 4]> = skin
                .reader(|buffer| Some(&buffers[buffer.index()]))
                .read_inverse_bind_matrices()
                .map(|matrices| matrices.collect())
                .unwrap_or_else(|| vec![Mat4::IDENTITY.to_cols_array_2d(); joints.len()]);
            if inverse_bind_matrices.len() != joints.len() {
                return Err(format!(
                    "skin {} has {} joints but {} inverse-bind matrices",
                    skin.index(),
                    joints.len(),
                    inverse_bind_matrices.len()
                ));
            }
            if inverse_bind_matrices
                .iter()
                .flatten()
                .flatten()
                .any(|value| !value.is_finite())
            {
                return Err(format!(
                    "skin {} contains non-finite inverse-bind matrices",
                    skin.index()
                ));
            }
            Ok(SkinData {
                joints,
                inverse_bind_matrices,
            })
        })
        .collect()
}

fn build_skeletal_animations(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
) -> Result<Vec<SkeletalAnimationClip>, String> {
    use gltf::animation::util::ReadOutputs;
    use gltf::animation::{Interpolation, Property};

    document
        .animations()
        .map(|animation| {
            let mut clip = SkeletalAnimationClip {
                name: animation
                    .name()
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("animation_{}", animation.index())),
                ..SkeletalAnimationClip::default()
            };
            for channel in animation.channels() {
                let target = channel.target();
                let property = match target.property() {
                    Property::Translation => AnimationProperty::Translation,
                    Property::Rotation => AnimationProperty::Rotation,
                    Property::Scale => AnimationProperty::Scale,
                    // Morph-target weights require a separate mesh morph pipeline and are not
                    // misrepresented as skeletal joint channels.
                    Property::MorphTargetWeights => continue,
                };
                let reader = channel.reader(|buffer| Some(&buffers[buffer.index()]));
                let times: Vec<f32> = reader
                    .read_inputs()
                    .ok_or_else(|| {
                        format!(
                            "animation {} channel has no input accessor",
                            animation.index()
                        )
                    })?
                    .collect();
                if times.is_empty()
                    || times[0] < 0.0
                    || times.iter().any(|v| !v.is_finite())
                    || times.windows(2).any(|pair| pair[0] >= pair[1])
                {
                    return Err(format!(
                        "animation {} channel has invalid keyframe times",
                        animation.index()
                    ));
                }
                let outputs = reader.read_outputs().ok_or_else(|| {
                    format!(
                        "animation {} channel has no output accessor",
                        animation.index()
                    )
                })?;
                let raw_values: Vec<[f32; 4]> = match outputs {
                    ReadOutputs::Translations(values) => {
                        values.map(|v| [v[0], v[1], v[2], 0.0]).collect()
                    }
                    ReadOutputs::Scales(values) => {
                        values.map(|v| [v[0], v[1], v[2], 0.0]).collect()
                    }
                    ReadOutputs::Rotations(values) => values
                        .into_f32()
                        .map(|v| [v[0], v[1], v[2], v[3]])
                        .collect(),
                    ReadOutputs::MorphTargetWeights(_) => continue,
                };
                if raw_values.iter().flatten().any(|value| !value.is_finite()) {
                    return Err(format!(
                        "animation {} channel contains non-finite output values",
                        animation.index()
                    ));
                }
                let interpolation = match channel.sampler().interpolation() {
                    Interpolation::Linear => AnimationInterpolation::Linear,
                    Interpolation::Step => AnimationInterpolation::Step,
                    Interpolation::CubicSpline => AnimationInterpolation::CubicSpline,
                };
                let (values, in_tangents, out_tangents) =
                    if interpolation == AnimationInterpolation::CubicSpline {
                        if raw_values.len() != times.len().saturating_mul(3) {
                            return Err(format!(
                                "animation {} cubic spline output count is invalid",
                                animation.index()
                            ));
                        }
                        let mut values = Vec::with_capacity(times.len());
                        let mut ins = Vec::with_capacity(times.len());
                        let mut outs = Vec::with_capacity(times.len());
                        for triple in raw_values.as_chunks::<3>().0 {
                            ins.push(triple[0]);
                            values.push(triple[1]);
                            outs.push(triple[2]);
                        }
                        (values, ins, outs)
                    } else {
                        if raw_values.len() != times.len() {
                            return Err(format!(
                                "animation {} output count does not match input count",
                                animation.index()
                            ));
                        }
                        (raw_values, Vec::new(), Vec::new())
                    };
                clip.duration_seconds = clip.duration_seconds.max(*times.last().unwrap_or(&0.0));
                clip.tracks.push(AnimationTrack {
                    node: target.node().index(),
                    property,
                    interpolation,
                    times,
                    values,
                    in_tangents,
                    out_tangents,
                });
            }
            Ok(clip)
        })
        .collect()
}

type SkinningAttributes = (Vec<[u16; 8]>, Vec<[f32; 8]>);

fn read_skinning_attributes<'a, 's, F>(
    reader: &gltf::mesh::Reader<'a, 's, F>,
    vertex_count: usize,
) -> Result<SkinningAttributes, String>
where
    F: Clone + Fn(gltf::Buffer<'a>) -> Option<&'s [u8]>,
{
    let joints0 = reader
        .read_joints(0)
        .map(|values| values.into_u16().collect::<Vec<_>>());
    let weights0 = reader
        .read_weights(0)
        .map(|values| values.into_f32().collect::<Vec<_>>());
    let joints1 = reader
        .read_joints(1)
        .map(|values| values.into_u16().collect::<Vec<_>>());
    let weights1 = reader
        .read_weights(1)
        .map(|values| values.into_f32().collect::<Vec<_>>());
    if reader.read_joints(2).is_some() || reader.read_weights(2).is_some() {
        return Err("more than eight skinning influences per vertex are not supported".to_owned());
    }

    let (joints0, weights0) = match (joints0, weights0) {
        (Some(joints), Some(weights)) => (joints, weights),
        (None, None) if joints1.is_none() && weights1.is_none() => {
            return Ok((vec![[0; 8]; vertex_count], vec![[0.0; 8]; vertex_count]));
        }
        _ => return Err("skinning attributes must provide matching JOINTS_n / WEIGHTS_n pairs starting at set zero".to_owned()),
    };
    if joints0.len() != vertex_count || weights0.len() != vertex_count {
        return Err("JOINTS_0 / WEIGHTS_0 counts do not match vertex count".to_owned());
    }
    let (joints1, weights1) = match (joints1, weights1) {
        (Some(joints), Some(weights)) => (joints, weights),
        (None, None) => (vec![[0; 4]; vertex_count], vec![[0.0; 4]; vertex_count]),
        _ => return Err("JOINTS_1 and WEIGHTS_1 must be provided together".to_owned()),
    };
    if joints1.len() != vertex_count || weights1.len() != vertex_count {
        return Err("JOINTS_1 / WEIGHTS_1 counts do not match vertex count".to_owned());
    }
    let mut out_joints = Vec::with_capacity(vertex_count);
    let mut out_weights = Vec::with_capacity(vertex_count);
    for vertex in 0..vertex_count {
        let mut joints = [0_u16; 8];
        joints[..4].copy_from_slice(&joints0[vertex]);
        joints[4..].copy_from_slice(&joints1[vertex]);
        let mut weights = [0.0_f32; 8];
        weights[..4].copy_from_slice(&weights0[vertex]);
        weights[4..].copy_from_slice(&weights1[vertex]);
        if weights
            .iter()
            .any(|weight| !weight.is_finite() || *weight < 0.0)
        {
            return Err(format!("vertex {vertex} has invalid skin weights"));
        }
        let sum: f32 = weights.iter().sum();
        if !sum.is_finite() {
            return Err(format!(
                "vertex {vertex} skin weights overflow their normalized sum"
            ));
        }
        if sum > 1.0e-8 {
            for weight in &mut weights {
                *weight /= sum;
            }
        } else {
            joints = [0; 8];
            weights = [0.0; 8];
        }
        out_joints.push(joints);
        out_weights.push(weights);
    }
    Ok((out_joints, out_weights))
}

/// Area-weighted vertex normals for meshes that ship without any.
pub fn smooth_normals(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    let mut sums = vec![Vec3::ZERO; positions.len()];
    for &triangle in indices.as_chunks::<3>().0 {
        let [a, b, c] = triangle.map(|i| Vec3::from(positions[i as usize]));
        let face = (b - a).cross(c - a);
        for i in triangle {
            sums[i as usize] += face;
        }
    }
    sums.into_iter()
        .map(|sum| sum.try_normalize().unwrap_or(Vec3::Y).to_array())
        .collect()
}

fn to_rgba8(image: &gltf::image::Data) -> Result<ImageData, String> {
    use gltf::image::Format;
    let pixels = &image.pixels;
    let count = (image.width as usize) * (image.height as usize);
    let high = |chunk: &[u8]| chunk[1]; // little-endian 16-bit: keep the high byte
    let mut out = Vec::with_capacity(count * 4);
    match image.format {
        Format::R8G8B8A8 => out.extend_from_slice(pixels),
        Format::R8G8B8 => pixels
            .as_chunks::<3>()
            .0
            .iter()
            .for_each(|p| out.extend_from_slice(&[p[0], p[1], p[2], 255])),
        Format::R8G8 => pixels
            .as_chunks::<2>()
            .0
            .iter()
            .for_each(|p| out.extend_from_slice(&[p[0], p[0], p[0], p[1]])),
        Format::R8 => pixels
            .iter()
            .for_each(|&p| out.extend_from_slice(&[p, p, p, 255])),
        Format::R16G16B16A16 => pixels.as_chunks::<8>().0.iter().for_each(|p| {
            out.extend_from_slice(&[
                high(&p[0..2]),
                high(&p[2..4]),
                high(&p[4..6]),
                high(&p[6..8]),
            ])
        }),
        Format::R16G16B16 => pixels.as_chunks::<6>().0.iter().for_each(|p| {
            out.extend_from_slice(&[high(&p[0..2]), high(&p[2..4]), high(&p[4..6]), 255])
        }),
        other => return Err(format!("unsupported image format {other:?}")),
    }
    if out.len() != count * 4 {
        return Err("image pixel data has the wrong size".to_owned());
    }
    Ok(ImageData {
        width: image.width,
        height: image.height,
        rgba8: out,
        srgb_mips: None,
        linear_mips: None,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn gltf_alpha_modes_cutoff_and_double_sided_defaults_are_imported() {
        let source = br#"{
            "asset": { "version": "2.0" },
            "scene": 0,
            "scenes": [{ "nodes": [0] }],
            "nodes": [{}],
            "materials": [
                {},
                { "alphaMode": "MASK", "alphaCutoff": 0.37, "doubleSided": true },
                { "alphaMode": "BLEND", "doubleSided": true }
            ]
        }"#;
        let model = super::parse(source).expect("minimal material document should import");
        assert_eq!(model.materials.len(), 3);
        assert_eq!(model.materials[0].alpha_mode, super::AlphaMode::Opaque);
        assert_eq!(model.materials[0].alpha_cutoff, 0.5);
        assert!(!model.materials[0].double_sided);
        assert_eq!(model.materials[1].alpha_mode, super::AlphaMode::Mask);
        assert!((model.materials[1].alpha_cutoff - 0.37).abs() < 1.0e-6);
        assert!(model.materials[1].double_sided);
        assert_eq!(model.materials[2].alpha_mode, super::AlphaMode::Blend);
        assert_eq!(model.materials[2].alpha_cutoff, 0.5);
        assert!(model.materials[2].double_sided);
    }

    use super::{parse, AlphaMode, GltfLoader, ModelAsset};
    use glam::{Mat4, Vec3};
    use ribbon_assets::{AssetServer, LoadErrorKind, MemorySource};

    const CUBE: &[u8] = include_bytes!("../../../assets/models/checker_cube.glb");
    const TRIANGLE: &[u8] = include_bytes!("../../../assets/models/bare_triangle.glb");
    const UV1_TANGENT_FIXTURE: &[u8] =
        include_bytes!("../../../assets/models/uv1_tangent_fixture.glb");
    const SKINNED_TRIANGLE: &[u8] = br#"{"asset":{"version":"2.0","generator":"Ribbon skeletal animation fixture"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0,"skin":0,"children":[1]},{"translation":[0,1,0]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0,"JOINTS_0":1,"WEIGHTS_0":2,"JOINTS_1":3,"WEIGHTS_1":4},"indices":5}]}],"skins":[{"inverseBindMatrices":6,"joints":[1]}],"animations":[{"name":"move_joint","samplers":[{"input":7,"output":8,"interpolation":"LINEAR"}],"channels":[{"sampler":0,"target":{"node":1,"path":"translation"}}]}],"buffers":[{"byteLength":284,"uri":"data:application/octet-stream;base64,AAAAvwAAAAAAAAAAAAAAPwAAAAAAAAAAAAAAAAAAgD8AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAPgAAAD4AAAA+AAAAPgAAAD4AAAA+AAAAPgAAAD4AAAA+AAAAPgAAAD4AAAA+AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAPgAAAD4AAAA+AAAAPgAAAD4AAAA+AAAAPgAAAD4AAAA+AAAAPgAAAD4AAAA+AAABAAIAAAAAAIA/AAAAAAAAAAAAAAAAAAAAAAAAgD8AAAAAAAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAAAAAAIA/AAAAAAAAgD8AAAAAAACAPwAAAAAAAAAAAAAAQAAAAAA="}],"bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36,"target":34962},{"buffer":0,"byteOffset":36,"byteLength":24,"target":34962},{"buffer":0,"byteOffset":60,"byteLength":48,"target":34962},{"buffer":0,"byteOffset":108,"byteLength":24,"target":34962},{"buffer":0,"byteOffset":132,"byteLength":48,"target":34962},{"buffer":0,"byteOffset":180,"byteLength":6,"target":34963},{"buffer":0,"byteOffset":188,"byteLength":64},{"buffer":0,"byteOffset":252,"byteLength":8},{"buffer":0,"byteOffset":260,"byteLength":24}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[-0.5,0,0],"max":[0.5,1,0]},{"bufferView":1,"componentType":5123,"count":3,"type":"VEC4"},{"bufferView":2,"componentType":5126,"count":3,"type":"VEC4"},{"bufferView":3,"componentType":5123,"count":3,"type":"VEC4"},{"bufferView":4,"componentType":5126,"count":3,"type":"VEC4"},{"bufferView":5,"componentType":5123,"count":3,"type":"SCALAR"},{"bufferView":6,"componentType":5126,"count":1,"type":"MAT4"},{"bufferView":7,"componentType":5126,"count":2,"type":"SCALAR","min":[0],"max":[1]},{"bufferView":8,"componentType":5126,"count":2,"type":"VEC3"}]}"#;

    #[test]
    fn skinned_gltf_imports_eight_influences_inverse_bind_and_animation_tracks() {
        let model = parse(SKINNED_TRIANGLE).expect("skinned glTF parses");
        assert_eq!(model.skeleton_nodes.len(), 2);
        assert_eq!(model.skeleton_nodes[1].parent, Some(0));
        assert_eq!(model.skins.len(), 1);
        assert_eq!(model.skins[0].joints, vec![1]);
        assert_eq!(model.nodes.len(), 1);
        assert_eq!(model.nodes[0].skin, Some(0));
        assert_eq!(model.primitives[0].joints[0].len(), 8);
        assert_eq!(model.primitives[0].weights[0].len(), 8);
        assert!(model.primitives[0].weights[0]
            .iter()
            .all(|weight| (*weight - 0.125).abs() < 1.0e-6));
        assert!((model.primitives[0].weights[0].iter().sum::<f32>() - 1.0).abs() < 1.0e-6);
        assert_eq!(model.skeletal_animations.len(), 1);
        assert_eq!(model.skeletal_animations[0].name, "move_joint");
        assert_eq!(model.skeletal_animations[0].tracks.len(), 1);
        assert_eq!(model.skeletal_animations[0].tracks[0].node, 1);
        assert_eq!(model.skeletal_animations[0].tracks[0].times, vec![0.0, 1.0]);
        let bind_globals = model
            .bind_pose_node_transforms()
            .expect("valid bind pose node transforms");
        assert_eq!(bind_globals.len(), 2);
        assert!((bind_globals[1].w_axis.y - 1.0).abs() < 1.0e-6);
        let bind_palettes = model.bind_pose_palettes().expect("valid bind pose palette");
        assert_eq!(bind_palettes.len(), 1);
        assert_eq!(
            bind_palettes[0].as_ref().expect("skinned placement").len(),
            1
        );
        assert!(model.memory_breakdown().animation_bytes > 0);
        assert_eq!(
            model.memory_breakdown().total_bytes(),
            model.estimated_size_bytes()
        );
        assert!(model.estimated_gpu_upload_bytes() >= 3 * 104);
    }

    #[test]
    fn textured_cube_loads_geometry_material_image_and_node_transform() {
        let model = parse(CUBE).expect("cube parses");
        assert_eq!(model.primitives.len(), 1);
        let cube = &model.primitives[0];
        assert_eq!((cube.positions.len(), cube.indices.len()), (24, 36));
        assert_eq!(cube.material, Some(0));
        assert_eq!(cube.uv1s.len(), cube.positions.len());
        assert_eq!(cube.tangents.len(), cube.positions.len());
        assert_eq!(
            cube.uv1s, cube.uvs,
            "missing TEXCOORD_1 falls back to TEXCOORD_0"
        );
        assert!(cube.tangents.iter().all(|tangent| {
            tangent.iter().all(|value| value.is_finite())
                && (glam::Vec3::new(tangent[0], tangent[1], tangent[2]).length() - 1.0).abs()
                    < 1.0e-4
                && tangent[3].abs() == 1.0
        }));
        assert_eq!(model.materials[0].base_color_tex_coord, 0);
        assert_eq!(model.materials[0].metallic_roughness_tex_coord, 0);
        assert_eq!(model.materials[0].normal_tex_coord, 0);
        assert_eq!(model.materials[0].occlusion_tex_coord, 0);
        assert_eq!(model.materials[0].emissive_tex_coord, 0);
        assert_eq!(model.materials[0].base_color_texture, Some(0));
        assert_eq!(model.materials[0].base_color, [1.0; 4]);
        // The fixture omits metallicFactor; glTF 2.0 defines its default as 1.0.
        assert_eq!(model.materials[0].metallic_factor, 1.0);
        assert_eq!(model.materials[0].roughness_factor, 1.0);
        assert_eq!(model.materials[0].metallic_roughness_texture, None);
        assert_eq!((model.images[0].width, model.images[0].height), (8, 8));
        assert_eq!(model.images[0].rgba8.len(), 8 * 8 * 4);
        let prepared_mips = model.images[0]
            .srgb_mips
            .as_ref()
            .expect("worker-prepared sRGB mips");
        assert_eq!(
            prepared_mips
                .iter()
                .map(|(width, height, _)| (*width, *height))
                .collect::<Vec<_>>(),
            vec![(4, 4), (2, 2), (1, 1)],
        );
        assert!(model.images[0].linear_mips.is_none());
        // Parent translation (0, 0.5, 0) times child translation (1.5, 0, 0) and scale 0.8.
        let expected = Mat4::from_translation(Vec3::new(0.0, 0.5, 0.0))
            * Mat4::from_translation(Vec3::new(1.5, 0.0, 0.0))
            * Mat4::from_scale(Vec3::splat(0.8));
        assert_eq!(model.nodes.len(), 1);
        assert!(model.nodes[0].transform.abs_diff_eq(expected, 1e-6));
        let (min, max) = model.bounds().expect("bounds");
        assert!(min.abs_diff_eq(Vec3::new(1.1, 0.1, -0.4), 1e-5), "{min}");
        assert!(max.abs_diff_eq(Vec3::new(1.9, 0.9, 0.4), 1e-5), "{max}");
    }

    #[test]
    fn gltf_imports_uv1_tangents_and_per_texture_tex_coord_selection() {
        let model = parse(UV1_TANGENT_FIXTURE).expect("UV1/tangent fixture parses");
        let primitive = &model.primitives[0];
        assert_eq!(primitive.positions.len(), 3);
        assert_eq!(primitive.uvs, vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
        assert_eq!(
            primitive.uv1s,
            vec![[0.25, 0.25], [0.75, 0.25], [0.25, 0.75]]
        );
        assert_eq!(primitive.tangents, vec![[0.0, 1.0, 0.0, -1.0]; 3]);
        let material = &model.materials[0];
        assert_eq!(material.base_color_texture, Some(0));
        assert_eq!(material.base_color_tex_coord, 1);
        assert_eq!(material.metallic_roughness_tex_coord, 0);
        assert_eq!(material.normal_tex_coord, 0);
        assert_eq!(material.occlusion_tex_coord, 0);
        assert_eq!(material.emissive_tex_coord, 0);
    }

    #[test]
    fn model_cpu_memory_breakdown_matches_total_capacity_estimate() {
        let model = parse(CUBE).expect("cube parses");
        let breakdown = model.memory_breakdown();
        assert!(breakdown.geometry_bytes > 0);
        assert!(breakdown.texture_bytes > 0);
        assert!(breakdown.material_bytes > 0);
        assert!(breakdown.scene_metadata_bytes > 0);
        assert_eq!(breakdown.animation_bytes, 0);
        assert_eq!(breakdown.other_bytes, 0);
        assert_eq!(breakdown.total_bytes(), model.estimated_size_bytes());
    }

    #[test]
    fn material_images_prepare_only_required_color_space_mips_and_account_for_them() {
        use super::{
            prepare_material_image_mips, ImageData, MaterialData, ModelAsset, ModelNode,
            PrimitiveData,
        };

        let image = |value: u8| ImageData {
            width: 2,
            height: 2,
            rgba8: vec![value; 16],
            srgb_mips: None,
            linear_mips: None,
        };
        let mut model = ModelAsset {
            images: vec![
                image(96),
                image(128),
                image(180),
                ImageData {
                    width: 1,
                    height: 1,
                    rgba8: vec![255; 4],
                    srgb_mips: None,
                    linear_mips: None,
                },
            ],
            materials: vec![
                MaterialData {
                    base_color: [1.0; 4],
                    metallic_factor: 0.0,
                    roughness_factor: 1.0,
                    base_color_texture: Some(0),
                    base_color_tex_coord: 0,
                    metallic_roughness_texture: None,
                    metallic_roughness_tex_coord: 0,
                    normal_texture: Some(1),
                    normal_tex_coord: 0,
                    occlusion_texture: None,
                    occlusion_tex_coord: 0,
                    occlusion_strength: 1.0,
                    emissive_texture: Some(2),
                    emissive_tex_coord: 0,
                    emissive_factor: [0.0; 3],
                    alpha_mode: AlphaMode::Opaque,
                    alpha_cutoff: 0.5,
                    double_sided: false,
                },
                MaterialData {
                    base_color: [1.0; 4],
                    metallic_factor: 0.0,
                    roughness_factor: 1.0,
                    base_color_texture: None,
                    base_color_tex_coord: 0,
                    metallic_roughness_texture: Some(2),
                    metallic_roughness_tex_coord: 0,
                    normal_texture: None,
                    normal_tex_coord: 0,
                    occlusion_texture: None,
                    occlusion_tex_coord: 0,
                    occlusion_strength: 1.0,
                    emissive_texture: None,
                    emissive_tex_coord: 0,
                    emissive_factor: [0.0; 3],
                    alpha_mode: AlphaMode::Opaque,
                    alpha_cutoff: 0.5,
                    double_sided: false,
                },
                MaterialData {
                    base_color: [1.0; 4],
                    metallic_factor: 0.0,
                    roughness_factor: 1.0,
                    base_color_texture: Some(3),
                    base_color_tex_coord: 0,
                    metallic_roughness_texture: None,
                    metallic_roughness_tex_coord: 0,
                    normal_texture: None,
                    normal_tex_coord: 0,
                    occlusion_texture: None,
                    occlusion_tex_coord: 0,
                    occlusion_strength: 1.0,
                    emissive_texture: None,
                    emissive_tex_coord: 0,
                    emissive_factor: [0.0; 3],
                    alpha_mode: AlphaMode::Opaque,
                    alpha_cutoff: 0.5,
                    double_sided: false,
                },
            ],
            primitives: vec![
                PrimitiveData {
                    positions: vec![[0.0; 3]; 3],
                    normals: vec![[0.0, 0.0, 1.0]; 3],
                    uvs: vec![[0.0; 2]; 3],
                    uv1s: vec![[0.0; 2]; 3],
                    tangents: vec![[1.0, 0.0, 0.0, 1.0]; 3],
                    indices: vec![0, 1, 2],
                    material: Some(0),
                    joints: vec![[0; 8]; 3],
                    weights: vec![[0.0; 8]; 3],
                },
                PrimitiveData {
                    positions: vec![[0.0; 3]; 3],
                    normals: vec![[0.0, 0.0, 1.0]; 3],
                    uvs: vec![[0.0; 2]; 3],
                    uv1s: vec![[0.0; 2]; 3],
                    tangents: vec![[1.0, 0.0, 0.0, 1.0]; 3],
                    indices: vec![0, 1, 2],
                    material: Some(1),
                    joints: vec![[0; 8]; 3],
                    weights: vec![[0.0; 8]; 3],
                },
            ],
            nodes: vec![
                ModelNode {
                    primitive: 0,
                    transform: Mat4::IDENTITY,
                    node: 0,
                    skin: None,
                },
                ModelNode {
                    primitive: 1,
                    transform: Mat4::IDENTITY,
                    node: 1,
                    skin: None,
                },
            ],
            skeleton_nodes: Vec::new(),
            skins: Vec::new(),
            skeletal_animations: Vec::new(),
        };
        let decoded_bytes = model.estimated_size_bytes();
        prepare_material_image_mips(&mut model);

        assert_eq!(model.images[0].srgb_mips.as_ref().map(Vec::len), Some(1));
        assert!(model.images[0].linear_mips.is_none());
        assert_eq!(model.images[1].linear_mips.as_ref().map(Vec::len), Some(1));
        assert!(model.images[1].srgb_mips.is_none());
        assert_eq!(model.images[2].srgb_mips.as_ref().map(Vec::len), Some(1));
        assert_eq!(model.images[2].linear_mips.as_ref().map(Vec::len), Some(1));
        assert!(model.images[3].srgb_mips.is_none());
        assert!(model.images[3].linear_mips.is_none());
        assert!(model.estimated_size_bytes() > decoded_bytes);
    }

    #[test]
    fn gpu_upload_estimate_counts_shared_image_base_once_per_transfer_function() {
        use super::{ImageData, MaterialData, ModelAsset, ModelNode, PrimitiveData};

        let model = ModelAsset {
            primitives: vec![PrimitiveData {
                positions: vec![[0.0; 3]; 3],
                normals: vec![[0.0, 0.0, 1.0]; 3],
                uvs: vec![[0.0; 2]; 3],
                uv1s: vec![[0.0; 2]; 3],
                tangents: vec![[1.0, 0.0, 0.0, 1.0]; 3],
                indices: vec![0, 1, 2],
                material: Some(0),
                joints: vec![[0; 8]; 3],
                weights: vec![[0.0; 8]; 3],
            }],
            materials: vec![MaterialData {
                base_color: [1.0; 4],
                metallic_factor: 0.0,
                roughness_factor: 1.0,
                base_color_texture: Some(0),
                base_color_tex_coord: 0,
                metallic_roughness_texture: None,
                metallic_roughness_tex_coord: 0,
                normal_texture: Some(0),
                normal_tex_coord: 0,
                occlusion_texture: None,
                occlusion_tex_coord: 0,
                occlusion_strength: 1.0,
                emissive_texture: None,
                emissive_tex_coord: 0,
                emissive_factor: [0.0; 3],
                alpha_mode: AlphaMode::Opaque,
                alpha_cutoff: 0.5,
                double_sided: false,
            }],
            images: vec![ImageData {
                width: 2,
                height: 2,
                rgba8: vec![128; 16],
                srgb_mips: Some(vec![(1, 1, vec![128; 4])]),
                linear_mips: Some(vec![(1, 1, vec![128; 4])]),
            }],
            nodes: vec![ModelNode {
                primitive: 0,
                transform: Mat4::IDENTITY,
                node: 0,
                skin: None,
            }],
            skeleton_nodes: Vec::new(),
            skins: Vec::new(),
            skeletal_animations: Vec::new(),
        };

        // This unskinned placement uses 3 * 56-byte static vertices plus its index buffer,
        // one material uniform, and two transfer-function-specific image allocations.
        assert_eq!(
            model.estimated_gpu_upload_bytes(),
            168 + 12 + 256 + (16 + 4) * 2
        );
    }

    #[test]
    fn missing_normals_uvs_indices_and_material_get_defaults() {
        let model = parse(TRIANGLE).expect("triangle parses");
        let triangle = &model.primitives[0];
        assert_eq!(triangle.indices, vec![0, 1, 2]);
        assert_eq!(triangle.uvs, vec![[0.0, 0.0]; 3]);
        assert_eq!(triangle.material, None);
        for normal in &triangle.normals {
            assert!(Vec3::from(*normal).abs_diff_eq(Vec3::Z, 1e-6), "{normal:?}");
        }
    }

    #[test]
    fn garbage_is_a_decode_error_and_the_server_routes_glb_files() {
        let mut server = AssetServer::default();
        server.register_loader(GltfLoader);
        let source = MemorySource::default();
        source.insert("bad.glb", b"not a glb".to_vec());
        source.insert("cube.glb", CUBE.to_vec());
        server.add_source(0, source);
        let error = server.load::<ModelAsset>("bad.glb").unwrap_err();
        assert_eq!(error.kind, LoadErrorKind::Decode);
        let handle = server.load::<ModelAsset>("cube.glb").expect("cube loads");
        assert_eq!(server.get(handle).expect("stored").primitives.len(), 1);
    }
}
