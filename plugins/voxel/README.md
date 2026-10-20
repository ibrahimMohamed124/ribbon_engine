# plugin_voxel

Voxel-grid storage, DDA ray casting, face queries, mutable chunk tracking and greedy
surface meshing. The crate depends on `engine_math` only and does not depend on the
app runtime or a rendering backend. Chunk scheduling and meshing remain reusable engine
capabilities; consumers decide what block ids mean.

## Greedy mesh API

- `greedy_mesh_chunk(world, origin, chunk_size, empty)` meshes a world-space
  chunk. Neighbor reads use the full world, so shared boundary faces are culled.
- `greedy_mesh_world(world, empty)` meshes the whole grid as one chunk.
- `VoxelChunkMesh` groups faces by block id and stores Y-up local vertices,
  chunk origin, indices, normals, and repeating UV coordinates.
- `z_up_to_y_up` converts legacy voxel coordinates to the native mesh convention.

## Dynamic worlds

`ChunkedVoxelWorld::new(world, chunk_size, empty_block)` wraps mutable voxel storage. Use
`set_block(x, y, z, block)` for edits; it returns the prior block id, ignores no-op writes,
rejects out-of-bounds cells and advances the world's revision only for real edits. Edits
invalidate their owning chunk, plus face-adjacent chunks when the edited voxel lies on a
chunk boundary. Call `rebuild_dirty_chunks()` to receive only the changed chunk snapshots.
An update with an empty `meshes` vector explicitly means the chunk became empty and the
rendering backend should evict prior uploaded geometry.

The greedy mesher samples the complete world when building a chunk. This hides faces shared
by adjacent non-empty voxels even when those voxels live in separate chunks, and preserves
visible faces on the outside of the world. Boundary clipping also supports dimensions that
are not multiples of the chunk size.

A renderer can either consume the `ChunkMeshUpdate` stream itself, or use
`plugin_render3d::OffscreenVoxelRenderer`, which consumes dirty updates, uploads new meshes,
releases superseded/empty GPU meshes and reuses mesh slots. For the native-surface ECS path,
wrap a generated mesh in `Arc<VoxelChunkMesh>` and attach it with
`plugin_render3d::VoxelMeshInstance3D`. See `examples/voxel_mesh_showcase` and
`docs/DYNAMIC_VOXEL_WORLD_MIGRATION.md` for the current integration boundaries. Block ids
remain application-defined; palettes/material-atlas policy is deliberately not owned by
this crate.
