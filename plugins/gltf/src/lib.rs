//! glTF 2.0 (`.glb` and self-contained `.gltf`) loading into CPU-side model data.
//!
//! A [`ModelAsset`] holds deduplicated primitives, materials and decoded images,
//! plus the default scene flattened into [`ModelNode`] placements. GPU upload is
//! the renderer's job. External buffer/image files are not resolved: use `.glb`
//! or embed the data.

use glam::{Mat4, Vec3};
use ribbon_assets::{AssetLoader, AssetMemoryBreakdown, LoadContext, LoadError, LoadErrorKind};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

/// Indexed triangle geometry with per-vertex normals and first UV set.
#[derive(Clone, Debug, PartialEq)]
pub struct PrimitiveData {
    /// Object-space positions.
    pub positions: Vec<[f32; 3]>,
    /// Unit normals (computed when the file has none).
    pub normals: Vec<[f32; 3]>,
    /// Texture coordinates, zeros when the file has none.
    pub uvs: Vec<[f32; 2]>,
    /// Triangle list indices.
    pub indices: Vec<u32>,
    /// Index into [`ModelAsset::materials`]; `None` selects the default material.
    pub material: Option<usize>,
}

/// Metallic-roughness material reduced to what the renderer currently uses.
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
    /// Index into [`ModelAsset::images`] for the linear G=roughness/B=metallic texture.
    pub metallic_roughness_texture: Option<usize>,
    /// Index into images for tangent-space normal data.
    pub normal_texture: Option<usize>,
    /// Index into images for the red-channel ambient-occlusion texture.
    pub occlusion_texture: Option<usize>,
    /// glTF occlusion strength.
    pub occlusion_strength: f32,
    /// Index into images for emissive color (sRGB).
    pub emissive_texture: Option<usize>,
    /// Linear emissive multiplier.
    pub emissive_factor: [f32; 3],
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
    /// Placements of primitives in model space.
    pub nodes: Vec<ModelNode>,
}

impl ModelAsset {
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
        for primitive in &self.primitives {
            bytes = bytes.saturating_add(allocated_vec_bytes::<[f32; 3]>(
                primitive.positions.capacity(),
            ));
            bytes = bytes.saturating_add(allocated_vec_bytes::<[f32; 3]>(
                primitive.normals.capacity(),
            ));
            bytes = bytes.saturating_add(allocated_vec_bytes::<[f32; 2]>(primitive.uvs.capacity()));
            bytes = bytes.saturating_add(allocated_vec_bytes::<u32>(primitive.indices.capacity()));
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
    /// estimates rather than allocator telemetry. The loader currently does not retain animation
    /// clips, so animation residency is zero for this asset type.
    pub fn memory_breakdown(&self) -> AssetMemoryBreakdown {
        let mut breakdown = AssetMemoryBreakdown {
            scene_metadata_bytes: (std::mem::size_of::<Self>() as u64)
                .saturating_add(allocated_vec_bytes::<ModelNode>(self.nodes.capacity())),
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
                .saturating_add(allocated_vec_bytes::<u32>(primitive.indices.capacity()));
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
    /// pacing estimate, not driver VRAM telemetry. It includes unique placed primitive vertex/
    /// index buffers, referenced material records, and each unique image transfer-function
    /// variant with its actual prepared mip byte count when available.
    pub fn estimated_gpu_upload_bytes(&self) -> u64 {
        let mut bytes = 0_u64;
        let used_primitives: std::collections::HashSet<usize> =
            self.nodes.iter().map(|node| node.primitive).collect();
        for index in used_primitives {
            let Some(primitive) = self.primitives.get(index) else {
                continue;
            };
            let vertices = primitive.positions.len() as u64;
            bytes = bytes.saturating_add(vertices.saturating_mul(
                (std::mem::size_of::<[f32; 3]>() * 2 + std::mem::size_of::<[f32; 2]>()) as u64,
            ));
            bytes = bytes.saturating_add(
                (primitive.indices.len() as u64).saturating_mul(std::mem::size_of::<u32>() as u64),
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
    let mut model = ModelAsset::default();
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
            metallic_roughness_texture: pbr
                .metallic_roughness_texture()
                .map(|info| info.texture().source().index()),
            normal_texture: material
                .normal_texture()
                .map(|info| info.texture().source().index()),
            occlusion_texture: material
                .occlusion_texture()
                .map(|info| info.texture().source().index()),
            occlusion_strength: material
                .occlusion_texture()
                .map_or(1.0, |info| info.strength()),
            emissive_texture: material
                .emissive_texture()
                .map(|info| info.texture().source().index()),
            emissive_factor: material.emissive_factor(),
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
    if normals.len() != positions.len() || uvs.len() != positions.len() {
        return Err("vertex attribute counts do not match".to_owned());
    }
    Ok(Some(PrimitiveData {
        positions,
        normals,
        uvs,
        indices,
        material: primitive.material().index(),
    }))
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
    use super::{parse, GltfLoader, ModelAsset};
    use glam::{Mat4, Vec3};
    use ribbon_assets::{AssetServer, LoadErrorKind, MemorySource};

    const CUBE: &[u8] = include_bytes!("../../../assets/models/checker_cube.glb");
    const TRIANGLE: &[u8] = include_bytes!("../../../assets/models/bare_triangle.glb");

    #[test]
    fn textured_cube_loads_geometry_material_image_and_node_transform() {
        let model = parse(CUBE).expect("cube parses");
        assert_eq!(model.primitives.len(), 1);
        let cube = &model.primitives[0];
        assert_eq!((cube.positions.len(), cube.indices.len()), (24, 36));
        assert_eq!(cube.material, Some(0));
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
                    metallic_roughness_texture: None,
                    normal_texture: Some(1),
                    occlusion_texture: None,
                    occlusion_strength: 1.0,
                    emissive_texture: Some(2),
                    emissive_factor: [0.0; 3],
                },
                MaterialData {
                    base_color: [1.0; 4],
                    metallic_factor: 0.0,
                    roughness_factor: 1.0,
                    base_color_texture: None,
                    metallic_roughness_texture: Some(2),
                    normal_texture: None,
                    occlusion_texture: None,
                    occlusion_strength: 1.0,
                    emissive_texture: None,
                    emissive_factor: [0.0; 3],
                },
                MaterialData {
                    base_color: [1.0; 4],
                    metallic_factor: 0.0,
                    roughness_factor: 1.0,
                    base_color_texture: Some(3),
                    metallic_roughness_texture: None,
                    normal_texture: None,
                    occlusion_texture: None,
                    occlusion_strength: 1.0,
                    emissive_texture: None,
                    emissive_factor: [0.0; 3],
                },
            ],
            primitives: vec![
                PrimitiveData {
                    positions: vec![[0.0; 3]; 3],
                    normals: vec![[0.0, 0.0, 1.0]; 3],
                    uvs: vec![[0.0; 2]; 3],
                    indices: vec![0, 1, 2],
                    material: Some(0),
                },
                PrimitiveData {
                    positions: vec![[0.0; 3]; 3],
                    normals: vec![[0.0, 0.0, 1.0]; 3],
                    uvs: vec![[0.0; 2]; 3],
                    indices: vec![0, 1, 2],
                    material: Some(1),
                },
            ],
            nodes: vec![
                ModelNode {
                    primitive: 0,
                    transform: Mat4::IDENTITY,
                },
                ModelNode {
                    primitive: 1,
                    transform: Mat4::IDENTITY,
                },
            ],
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
                indices: vec![0, 1, 2],
                material: Some(0),
            }],
            materials: vec![MaterialData {
                base_color: [1.0; 4],
                metallic_factor: 0.0,
                roughness_factor: 1.0,
                base_color_texture: Some(0),
                metallic_roughness_texture: None,
                normal_texture: Some(0),
                occlusion_texture: None,
                occlusion_strength: 1.0,
                emissive_texture: None,
                emissive_factor: [0.0; 3],
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
            }],
        };

        // 3 vertices * 32 bytes + 3 indices * 4 bytes + one 256-byte material uniform,
        // plus two transfer-function-specific texture allocations (base + 1x1 mip each).
        assert_eq!(
            model.estimated_gpu_upload_bytes(),
            96 + 12 + 256 + (16 + 4) * 2
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
