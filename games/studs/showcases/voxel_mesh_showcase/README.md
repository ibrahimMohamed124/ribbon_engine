# Dynamic voxel mesh showcase

This STUDS showcase demonstrates STUDS-owned dynamic voxel APIs without putting voxel logic in the engine. The world is held in `studs_voxel::ChunkedVoxelWorld`; the example consumes `ChunkMeshUpdate` values, replaces only ECS entities belonging to dirty chunks, and attaches the resulting local meshes at each chunk's `origin_y_up`. The native `Render3dPlugin` extracts those generic mesh entities and owns their ordinary GPU upload/eviction; chunk invalidation and voxel remeshing stay in `studs_voxel`.

Run from the workspace root:

```powershell
cargo run -p voxel_mesh_showcase
# Optional finite run; the demo removes a floor voxel at frame 120 and restores it at frame 180.
cargo run -p voxel_mesh_showcase -- --frames 240
```

The edit is deliberately placed on an intersection of horizontal chunk boundaries so neighboring chunk invalidation and seam handling can be inspected. Console messages identify the removal and restoration frames.

## Render profiling capture

Run the example in release mode with a finite frame count to write a CSV report after shutdown.
The default warmup is 120 successfully rendered frames; set `--profile-warmup N` to change it.
For example:

```powershell
cargo run --release -p voxel_mesh_showcase -- --frames 1800 --profile-csv target/profiles/voxel-mesh-showcase.csv
python tools/perf/render_profile_report.py target/profiles/voxel-mesh-showcase.csv --output target/profiles/voxel-mesh-showcase.md
```

Compare runs only on the same machine, GPU adapter, resolution, build profile, scene setup, and
frame-count/warmup settings. GPU timestamps are asynchronous and the current GPU interval covers
shadow plus mesh rendering, not the whole frame. Resource byte counts are estimates, not driver VRAM readings.
