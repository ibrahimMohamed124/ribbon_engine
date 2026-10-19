# Greedy Voxel Mesh Showcase

This app builds a small Z-up `VoxelWorld`, greedily meshes it in 8×6 chunks,
converts vertices and chunk origins to the engine's right-handed Y-up convention,
and renders the resulting `VoxelMeshInstance3D` entities through the native
`plugin_render3d` pipeline. The path shares the depth buffer, PBR lighting,
directional shadow map, HDR target and tone mapper used by GLB meshes.

Run from the repository root:

```powershell
cargo run -p voxel_mesh_showcase
```

For an automatic capture/shutdown:

```powershell
cargo run -p voxel_mesh_showcase -- --frames 240
```

The sample demonstrates greedy face merging, removal of internal faces,
block-material grouping, cross-chunk neighbor culling, and native GPU mesh
rendering. It deliberately does not claim that STUDS has migrated: the game still
uses its existing voxel rendering path, and live chunk edit/remesh scheduling is
future work.
