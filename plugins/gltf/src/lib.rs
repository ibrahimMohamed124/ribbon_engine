//! glTF 2.0 (`.glb` and self-contained `.gltf`) loading into CPU-side model data.
//!
//! A [`ModelAsset`] holds deduplicated primitives, materials and decoded images,
//! plus the default scene flattened into [`ModelNode`] placements. GPU upload is
//! the renderer's job. External buffer/image files are not resolved: use `.glb`
//! or embed the data.

use engine_assets::{AssetLoader, LoadContext, LoadError, LoadErrorKind};
use glam::{Mat4, Vec3};
use std::collections::HashMap;

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
    Ok(model)
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
    })
}

#[cfg(test)]
mod tests {
    use super::{parse, GltfLoader, ModelAsset};
    use engine_assets::{AssetServer, LoadErrorKind, MemorySource};
    use glam::{Mat4, Vec3};

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
