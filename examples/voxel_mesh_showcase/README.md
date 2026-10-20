# Dynamic voxel mesh showcase

This engine-only example demonstrates the reusable dynamic voxel APIs without putting engine logic in a game. The world is held in `plugin_voxel::ChunkedVoxelWorld`; the example consumes `ChunkMeshUpdate` values, replaces only ECS entities belonging to dirty chunks, and attaches the resulting local meshes at each chunk's `origin_y_up`. The native `Render3dPlugin` extracts those entities and owns GPU upload/eviction.

Run from the workspace root:

```powershell
cargo run -p voxel_mesh_showcase
# Optional finite run; the demo removes a floor voxel at frame 120 and restores it at frame 180.
cargo run -p voxel_mesh_showcase -- --frames 240
```

The edit is deliberately placed on an intersection of horizontal chunk boundaries so neighboring chunk invalidation and seam handling can be inspected. Console messages identify the removal and restoration frames.
