# STUDS Voxel World Crate (`studs_voxel`)

STUDS-owned voxel-grid storage, DDA ray casting, face queries, mutable chunk tracking and greedy
surface meshing. The crate depends on `ribbon_math` only and does not depend on the
app runtime or a rendering backend. It is intentionally under `games/studs/crates/`: these
voxel APIs belong to STUDS and are not engine APIs. Other games can build their own world model
on the public mesh renderer without inheriting voxel policy.

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

The STUDS GPU adapter can consume the `ChunkMeshUpdate` stream through
`studs_voxel_gpu::OffscreenVoxelRenderer`, which uploads changed meshes, releases superseded/empty
GPU meshes and reuses mesh slots. This adapter is owned by STUDS, not by the generic engine renderer. For the native-surface ECS path,
wrap a generated mesh in `Arc<VoxelChunkMesh>` and attach it with
`ribbon_plugin_render3d::VoxelMeshInstance3D`. See `games/studs/showcases/voxel_mesh_showcase` and
`games/studs/docs/DYNAMIC_VOXEL_WORLD_MIGRATION.md` for the current integration boundaries. Block ids
remain application-defined; palettes/material-atlas policy is deliberately not owned by
this crate.
