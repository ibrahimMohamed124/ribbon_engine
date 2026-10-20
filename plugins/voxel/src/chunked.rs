//! Mutable voxel-world chunking, dirty-region tracking, and incremental CPU remeshing.
//!
//! This module deliberately has no renderer or game dependency. Applications edit voxels
//! through [`ChunkedVoxelWorld::set_block`]; only the owning chunk and face-adjacent chunks
//! are invalidated, and consumers can pull remeshed chunk updates when they are ready.

use crate::meshing::{greedy_mesh_chunk, VoxelChunkMesh};
use crate::voxel::VoxelWorld;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Deref;
use std::sync::Arc;

/// Integer chunk coordinate in the voxel grid's original (Z-up) coordinate system.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkCoord {
    pub x: usize,
    pub y: usize,
    pub z: usize,
}

impl ChunkCoord {
    pub const fn new(x: usize, y: usize, z: usize) -> Self {
        Self { x, y, z }
    }

    pub const fn to_array(self) -> [usize; 3] {
        [self.x, self.y, self.z]
    }
}

/// A completed CPU remesh for one chunk. An empty `meshes` vector means that the chunk
/// became empty and consumers must discard any previously uploaded geometry for it.
#[derive(Clone, Debug)]
pub struct ChunkMeshUpdate {
    pub coord: ChunkCoord,
    pub revision: u64,
    pub meshes: Vec<Arc<VoxelChunkMesh>>,
}

/// A mutable voxel grid with chunk-size configuration, edge-aware dirty tracking, and a
/// retained CPU mesh cache. Rendering backends consume [`Self::rebuild_dirty_chunks`]
/// updates and own the corresponding GPU resources separately.
pub struct ChunkedVoxelWorld {
    world: VoxelWorld,
    chunk_size: [usize; 3],
    empty_block: u8,
    revision: u64,
    dirty: BTreeSet<ChunkCoord>,
    meshes: BTreeMap<ChunkCoord, Vec<Arc<VoxelChunkMesh>>>,
}

impl ChunkedVoxelWorld {
    /// Wrap a voxel grid. Every non-empty and empty chunk starts dirty so the first consumer
    /// receives a complete snapshot, including empty chunk updates.
    pub fn new(world: VoxelWorld, chunk_size: [usize; 3], empty_block: u8) -> Self {
        assert!(
            chunk_size.iter().all(|size| *size > 0),
            "chunk dimensions must be non-zero"
        );
        let dimensions = world.dimensions();
        let grid = [dimensions.0, dimensions.1, dimensions.2];
        let counts = [
            grid[0].div_ceil(chunk_size[0]),
            grid[1].div_ceil(chunk_size[1]),
            grid[2].div_ceil(chunk_size[2]),
        ];
        let mut dirty = BTreeSet::new();
        for z in 0..counts[2] {
            for y in 0..counts[1] {
                for x in 0..counts[0] {
                    dirty.insert(ChunkCoord::new(x, y, z));
                }
            }
        }
        Self {
            world,
            chunk_size,
            empty_block,
            revision: 0,
            dirty,
            meshes: BTreeMap::new(),
        }
    }

    pub fn world(&self) -> &VoxelWorld {
        &self.world
    }

    pub fn dimensions(&self) -> (usize, usize, usize) {
        self.world.dimensions()
    }

    pub fn chunk_size(&self) -> [usize; 3] {
        self.chunk_size
    }

    pub fn empty_block(&self) -> u8 {
        self.empty_block
    }

    /// Monotonically increasing edit revision. A no-op write does not increment it.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Current number of chunks waiting to be remeshed.
    pub fn dirty_chunk_count(&self) -> usize {
        self.dirty.len()
    }

    /// Chunk grid dimensions after clipping the final chunks at world boundaries.
    pub fn chunk_counts(&self) -> [usize; 3] {
        let world = self.world.dimensions();
        [
            world.0.div_ceil(self.chunk_size[0]),
            world.1.div_ceil(self.chunk_size[1]),
            world.2.div_ceil(self.chunk_size[2]),
        ]
    }

    /// Convert a world voxel coordinate to its owner chunk, if it lies inside the world.
    pub fn chunk_for_voxel(&self, x: usize, y: usize, z: usize) -> Option<ChunkCoord> {
        let dimensions = self.world.dimensions();
        if x >= dimensions.0 || y >= dimensions.1 || z >= dimensions.2 {
            return None;
        }
        Some(ChunkCoord::new(
            x / self.chunk_size[0],
            y / self.chunk_size[1],
            z / self.chunk_size[2],
        ))
    }

    /// Edit a voxel. Returns the previous block id, or `None` when out of bounds.
    /// Rewriting the same block is a no-op. Edits on chunk faces also invalidate the
    /// neighboring chunk because its boundary face visibility depends on this voxel.
    pub fn set_block(&mut self, x: i32, y: i32, z: i32, block: u8) -> Option<u8> {
        let old = self.world.get(x, y, z)?;
        if old == block {
            return Some(old);
        }
        self.world.set(x, y, z, block)?;
        self.revision = self.revision.wrapping_add(1);
        self.invalidate_voxel(x as usize, y as usize, z as usize);
        Some(old)
    }

    /// Read-only access to the last meshed chunk snapshot. It may lag behind edits until
    /// [`Self::rebuild_dirty_chunks`] is called.
    pub fn chunk_meshes(&self, coord: ChunkCoord) -> Option<&[Arc<VoxelChunkMesh>]> {
        self.meshes.get(&coord).map(Vec::as_slice)
    }

    /// Rebuild only dirty chunks and return their latest meshes. The full world is sampled
    /// while meshing each chunk, so inter-chunk hidden faces are always culled consistently.
    pub fn rebuild_dirty_chunks(&mut self) -> Vec<ChunkMeshUpdate> {
        let dirty = std::mem::take(&mut self.dirty);
        let mut updates = Vec::with_capacity(dirty.len());
        let world_dimensions = self.world.dimensions();
        for coord in dirty {
            let origin = [
                coord.x * self.chunk_size[0],
                coord.y * self.chunk_size[1],
                coord.z * self.chunk_size[2],
            ];
            let chunk_meshes: Vec<Arc<VoxelChunkMesh>> =
                greedy_mesh_chunk(&self.world, origin, self.chunk_size, self.empty_block)
                    .into_iter()
                    .filter(|mesh| !mesh.indices.is_empty())
                    .map(Arc::new)
                    .collect();

            if chunk_meshes.is_empty() {
                self.meshes.remove(&coord);
            } else {
                self.meshes.insert(coord, chunk_meshes.clone());
            }
            // `greedy_mesh_chunk` clips at the real world extent. The bounds check here
            // also makes updates resilient if a future implementation supports resizing.
            let still_valid = origin[0] < world_dimensions.0
                && origin[1] < world_dimensions.1
                && origin[2] < world_dimensions.2;
            updates.push(ChunkMeshUpdate {
                coord,
                revision: self.revision,
                meshes: if still_valid {
                    chunk_meshes
                } else {
                    Vec::new()
                },
            });
        }
        updates
    }

    /// Mark every known chunk dirty, for example after a global material or meshing policy change.
    pub fn mark_all_dirty(&mut self) {
        let counts = self.chunk_counts();
        for z in 0..counts[2] {
            for y in 0..counts[1] {
                for x in 0..counts[0] {
                    self.dirty.insert(ChunkCoord::new(x, y, z));
                }
            }
        }
    }

    fn invalidate_voxel(&mut self, x: usize, y: usize, z: usize) {
        let coord = ChunkCoord::new(
            x / self.chunk_size[0],
            y / self.chunk_size[1],
            z / self.chunk_size[2],
        );
        self.dirty.insert(coord);
        let local = [
            x % self.chunk_size[0],
            y % self.chunk_size[1],
            z % self.chunk_size[2],
        ];
        let counts = self.chunk_counts();
        let mut neighbours = Vec::with_capacity(6);
        for axis in 0..3 {
            if local[axis] == 0 && coord.to_array()[axis] > 0 {
                let mut neighbour = coord.to_array();
                neighbour[axis] -= 1;
                neighbours.push(ChunkCoord::new(neighbour[0], neighbour[1], neighbour[2]));
            }
            if local[axis] + 1 == self.chunk_size[axis] && coord.to_array()[axis] + 1 < counts[axis]
            {
                let mut neighbour = coord.to_array();
                neighbour[axis] += 1;
                neighbours.push(ChunkCoord::new(neighbour[0], neighbour[1], neighbour[2]));
            }
        }
        self.dirty.extend(neighbours);
    }
}

impl Deref for ChunkedVoxelWorld {
    type Target = VoxelWorld;

    fn deref(&self) -> &Self::Target {
        &self.world
    }
}

#[cfg(test)]
mod tests {
    use super::{ChunkCoord, ChunkedVoxelWorld};
    use crate::VoxelWorld;

    const AIR: u8 = 0;
    const STONE: u8 = 1;

    fn world(size: (usize, usize, usize), solids: &[(usize, usize, usize)]) -> VoxelWorld {
        let (width, height, depth) = size;
        let mut blocks = vec![AIR; width * height * depth];
        for &(x, y, z) in solids {
            blocks[VoxelWorld::idx(width, height, z, y, x)] = STONE;
        }
        VoxelWorld::new(width, height, depth, blocks)
    }

    #[test]
    fn initial_snapshot_remeshes_every_chunk_and_caches_non_empty_meshes() {
        let mut world = ChunkedVoxelWorld::new(world((4, 2, 1), &[(0, 0, 0)]), [2, 2, 1], AIR);
        let updates = world.rebuild_dirty_chunks();
        assert_eq!(updates.len(), 2);
        assert_eq!(world.dirty_chunk_count(), 0);
        assert_eq!(
            world.chunk_meshes(ChunkCoord::new(0, 0, 0)).unwrap().len(),
            1
        );
        assert!(world.chunk_meshes(ChunkCoord::new(1, 0, 0)).is_none());
    }

    #[test]
    fn interior_edit_only_invalidates_its_own_chunk_and_noop_writes_do_not_revision() {
        let mut world = ChunkedVoxelWorld::new(world((8, 8, 4), &[]), [4, 4, 4], AIR);
        world.rebuild_dirty_chunks();
        let revision = world.revision();
        assert_eq!(world.set_block(1, 1, 1, STONE), Some(AIR));
        assert_eq!(world.revision(), revision + 1);
        assert_eq!(world.dirty_chunk_count(), 1);
        assert_eq!(world.set_block(1, 1, 1, STONE), Some(STONE));
        assert_eq!(world.revision(), revision + 1);
        assert_eq!(world.dirty_chunk_count(), 1);
    }

    #[test]
    fn face_boundary_edit_invalidates_both_adjacent_chunks() {
        let mut world = ChunkedVoxelWorld::new(world((8, 4, 2), &[(4, 1, 1)]), [4, 4, 2], AIR);
        world.rebuild_dirty_chunks();
        assert_eq!(world.set_block(3, 1, 1, STONE), Some(AIR));
        let updates = world.rebuild_dirty_chunks();
        let coords: Vec<_> = updates.iter().map(|update| update.coord).collect();
        assert_eq!(
            coords,
            vec![ChunkCoord::new(0, 0, 0), ChunkCoord::new(1, 0, 0)]
        );
        assert_eq!(updates[0].meshes[0].indices.len(), 30);
        assert_eq!(updates[1].meshes[0].indices.len(), 30);
    }

    #[test]
    fn removing_last_voxel_returns_an_empty_update_for_gpu_eviction() {
        let mut world = ChunkedVoxelWorld::new(world((2, 2, 2), &[(1, 1, 1)]), [2, 2, 2], AIR);
        world.rebuild_dirty_chunks();
        assert!(world.chunk_meshes(ChunkCoord::new(0, 0, 0)).is_some());
        assert_eq!(world.set_block(1, 1, 1, AIR), Some(STONE));
        let updates = world.rebuild_dirty_chunks();
        assert_eq!(updates.len(), 1);
        assert!(updates[0].meshes.is_empty());
        assert!(world.chunk_meshes(ChunkCoord::new(0, 0, 0)).is_none());
    }

    #[test]
    fn out_of_bounds_edit_does_not_change_world_or_revision() {
        let mut world = ChunkedVoxelWorld::new(world((2, 2, 2), &[]), [1, 1, 1], AIR);
        world.rebuild_dirty_chunks();
        assert_eq!(world.set_block(-1, 0, 0, STONE), None);
        assert_eq!(world.set_block(2, 0, 0, STONE), None);
        assert_eq!(world.revision(), 0);
        assert_eq!(world.dirty_chunk_count(), 0);
    }
}
