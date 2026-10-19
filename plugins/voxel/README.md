# plugin_voxel

Voxel-grid storage, DDA ray casting, face queries, and greedy surface meshing.
The crate depends on `engine_math` only and does not depend on the app runtime or
a rendering backend.

## Greedy mesh API

- `greedy_mesh_chunk(world, origin, chunk_size, empty)` meshes a world-space
  chunk. Neighbor reads use the full world, so shared boundary faces are culled.
- `greedy_mesh_world(world, empty)` meshes the whole grid as one chunk.
- `VoxelChunkMesh` groups faces by block id and stores Y-up local vertices,
  chunk origin, indices, normals, and repeating UV coordinates.
- `z_up_to_y_up` converts legacy voxel coordinates to the native mesh convention.

To draw generated output, wrap each mesh in `Arc<VoxelChunkMesh>` and attach it
with `plugin_render3d::VoxelMeshInstance3D`. See
`examples/voxel_mesh_showcase` for a full app. The mesher is currently a static
CPU operation; world edits do not yet automatically dirty/remesh chunks, and
block ids are exposed to the game as material/tint selection rather than a full
material atlas system.
