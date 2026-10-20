//! Greedy voxel surface meshing for GPU mesh renderers.
//!
//! The voxel grid is stored in Z-up coordinates. Generated vertices/normals and
//! chunk origins are converted to the engine's right-handed Y-up convention.
//! Faces between any two non-empty voxels are culled, while equal block ids are
//! merged into larger quads to reduce vertex and triangle counts.

use crate::voxel::VoxelWorld;
use std::collections::BTreeMap;

/// One GPU-friendly vertex generated from a voxel surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoxelVertex {
    /// Position in chunk-local right-handed Y-up coordinates.
    pub position: [f32; 3],
    /// Unit normal in right-handed Y-up coordinates.
    pub normal: [f32; 3],
    /// Repeating UV coordinates; merged quads tile instead of stretching.
    pub uv: [f32; 2],
}

/// Greedy surface mesh for one non-empty block id within a chunk.
#[derive(Clone, Debug, PartialEq)]
pub struct VoxelChunkMesh {
    /// Block id represented by this mesh. Use it to select the GPU material.
    pub block: u8,
    /// Chunk origin converted to right-handed Y-up coordinates.
    pub origin_y_up: [f32; 3],
    /// Chunk-local vertices in the engine's Y-up coordinate convention.
    pub vertices: Vec<VoxelVertex>,
    /// Counter-clockwise triangle indices.
    pub indices: Vec<u32>,
}

impl VoxelChunkMesh {
    /// Number of triangles in this mesh.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

/// Convert a point/vector from the legacy voxel system's Z-up axes to engine Y-up.
/// This is a -90 degree rotation around X: `(x, y, z) -> (x, z, -y)`.
pub fn z_up_to_y_up(value: [f32; 3]) -> [f32; 3] {
    [value[0], value[2], -value[1]]
}

/// Greedily mesh the requested world-space chunk.
///
/// `origin` is expressed in voxel-grid coordinates and `chunk_size` in voxels.
/// Chunks extending past the world are clipped. Neighbor reads are done against
/// the complete world, so faces between neighboring chunks are culled correctly.
/// Out-of-world space is treated as empty so outer boundary faces remain visible.
pub fn greedy_mesh_chunk(
    world: &VoxelWorld,
    origin: [usize; 3],
    chunk_size: [usize; 3],
    empty: u8,
) -> Vec<VoxelChunkMesh> {
    let world_size = world.dimensions();
    let bounds = [world_size.0, world_size.1, world_size.2];
    if (0..3).any(|axis| origin[axis] >= bounds[axis] || chunk_size[axis] == 0) {
        return Vec::new();
    }
    let size = [
        chunk_size[0].min(bounds[0] - origin[0]),
        chunk_size[1].min(bounds[1] - origin[1]),
        chunk_size[2].min(bounds[2] - origin[2]),
    ];
    let origin_y_up = z_up_to_y_up([origin[0] as f32, origin[1] as f32, origin[2] as f32]);
    let mut meshes = BTreeMap::<u8, VoxelChunkMesh>::new();

    // For each axis, sweep every grid plane and greedily merge same-material faces.
    for axis in 0..3 {
        let u_axis = (axis + 1) % 3;
        let v_axis = (axis + 2) % 3;
        let width = size[u_axis];
        let height = size[v_axis];
        for plane in 0..=size[axis] {
            let mut mask = vec![None::<(u8, i8)>; width * height];
            for v in 0..height {
                for u in 0..width {
                    let mut a = [0_i32; 3];
                    let mut b = [0_i32; 3];
                    a[axis] = plane as i32 - 1;
                    b[axis] = plane as i32;
                    a[u_axis] = u as i32;
                    b[u_axis] = u as i32;
                    a[v_axis] = v as i32;
                    b[v_axis] = v as i32;
                    let a_block = sample(world, origin, a, empty);
                    let b_block = sample(world, origin, b, empty);
                    let face = if a_block == empty && b_block != empty {
                        Some((b_block, -1))
                    } else if a_block != empty && b_block == empty {
                        Some((a_block, 1))
                    } else {
                        None
                    };
                    mask[v * width + u] = face;
                }
            }

            let mut v = 0;
            while v < height {
                let mut u = 0;
                while u < width {
                    let index = v * width + u;
                    let Some(face) = mask[index] else {
                        u += 1;
                        continue;
                    };

                    let mut quad_width = 1;
                    while u + quad_width < width && mask[v * width + u + quad_width] == Some(face) {
                        quad_width += 1;
                    }
                    let mut quad_height = 1;
                    'height: while v + quad_height < height {
                        for dx in 0..quad_width {
                            if mask[(v + quad_height) * width + u + dx] != Some(face) {
                                break 'height;
                            }
                        }
                        quad_height += 1;
                    }

                    emit_quad(
                        meshes.entry(face.0).or_insert_with(|| VoxelChunkMesh {
                            block: face.0,
                            origin_y_up,
                            vertices: Vec::new(),
                            indices: Vec::new(),
                        }),
                        axis,
                        u_axis,
                        v_axis,
                        plane,
                        u,
                        v,
                        quad_width,
                        quad_height,
                        face.1,
                    );

                    for dy in 0..quad_height {
                        for dx in 0..quad_width {
                            mask[(v + dy) * width + u + dx] = None;
                        }
                    }
                    u += quad_width;
                }
                v += 1;
            }
        }
    }

    meshes.into_values().collect()
}

/// Mesh the entire voxel grid as a single chunk.
pub fn greedy_mesh_world(world: &VoxelWorld, empty: u8) -> Vec<VoxelChunkMesh> {
    let size = world.dimensions();
    greedy_mesh_chunk(world, [0; 3], [size.0, size.1, size.2], empty)
}

fn sample(world: &VoxelWorld, origin: [usize; 3], local: [i32; 3], empty: u8) -> u8 {
    let x = origin[0] as i32 + local[0];
    let y = origin[1] as i32 + local[1];
    let z = origin[2] as i32 + local[2];
    world.get(x, y, z).unwrap_or(empty)
}

#[allow(clippy::too_many_arguments)]
fn emit_quad(
    mesh: &mut VoxelChunkMesh,
    axis: usize,
    u_axis: usize,
    v_axis: usize,
    plane: usize,
    u: usize,
    v: usize,
    width: usize,
    height: usize,
    sign: i8,
) {
    let point = |du: usize, dv: usize| {
        let mut result = [0.0_f32; 3];
        result[axis] = plane as f32;
        result[u_axis] = (u + du) as f32;
        result[v_axis] = (v + dv) as f32;
        result
    };
    let p0 = point(0, 0);
    let p1 = point(width, 0);
    let p2 = point(width, height);
    let p3 = point(0, height);
    let mut normal = [0.0_f32; 3];
    normal[axis] = f32::from(sign);
    let normal = z_up_to_y_up(normal);
    let base = mesh.vertices.len() as u32;
    for (position, uv) in [
        (p0, [0.0, 0.0]),
        (p1, [width as f32, 0.0]),
        (p2, [width as f32, height as f32]),
        (p3, [0.0, height as f32]),
    ] {
        mesh.vertices.push(VoxelVertex {
            position: z_up_to_y_up(position),
            normal,
            uv,
        });
    }
    if sign > 0 {
        mesh.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    } else {
        mesh.indices
            .extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
    }
}

#[cfg(test)]
mod tests {
    use super::{greedy_mesh_chunk, greedy_mesh_world, z_up_to_y_up};
    use crate::VoxelWorld;

    const EMPTY: u8 = 0;

    fn world(size: (usize, usize, usize), cells: &[(usize, usize, usize, u8)]) -> VoxelWorld {
        let (width, height, depth) = size;
        let mut blocks = vec![EMPTY; width * height * depth];
        for &(x, y, z, block) in cells {
            blocks[VoxelWorld::idx(width, height, z, y, x)] = block;
        }
        VoxelWorld::new(width, height, depth, blocks)
    }

    #[test]
    fn one_voxel_produces_six_outward_faces() {
        let world = world((1, 1, 1), &[(0, 0, 0, 1)]);
        let meshes = greedy_mesh_world(&world, EMPTY);
        assert_eq!(meshes.len(), 1);
        assert_eq!(meshes[0].vertices.len(), 24);
        assert_eq!(meshes[0].indices.len(), 36);
        assert_eq!(meshes[0].triangle_count(), 12);
    }

    #[test]
    fn adjacent_voxels_merge_coplanar_faces_and_hide_their_interface() {
        let world = world((2, 1, 1), &[(0, 0, 0, 1), (1, 0, 0, 1)]);
        let meshes = greedy_mesh_world(&world, EMPTY);
        assert_eq!(meshes.len(), 1);
        assert_eq!(meshes[0].vertices.len(), 24);
        assert_eq!(meshes[0].indices.len(), 36);
    }

    #[test]
    fn chunk_boundaries_cull_faces_using_neighboring_world_cells() {
        let world = world((2, 1, 1), &[(0, 0, 0, 1), (1, 0, 0, 1)]);
        let left = greedy_mesh_chunk(&world, [0, 0, 0], [1, 1, 1], EMPTY);
        let right = greedy_mesh_chunk(&world, [1, 0, 0], [1, 1, 1], EMPTY);
        assert_eq!(left[0].indices.len(), 30);
        assert_eq!(right[0].indices.len(), 30);
        assert_eq!(left[0].origin_y_up, [0.0, 0.0, 0.0]);
        assert_eq!(right[0].origin_y_up, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn different_solid_blocks_do_not_create_internal_faces() {
        let world = world((2, 1, 1), &[(0, 0, 0, 1), (1, 0, 0, 2)]);
        let meshes = greedy_mesh_world(&world, EMPTY);
        assert_eq!(meshes.len(), 2);
        assert!(meshes.iter().all(|mesh| mesh.indices.len() == 30));
    }

    #[test]
    fn coordinate_conversion_maps_legacy_z_up_to_engine_y_up() {
        assert_eq!(z_up_to_y_up([2.0, 3.0, 4.0]), [2.0, 4.0, -3.0]);
    }
}
