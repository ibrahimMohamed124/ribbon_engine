# Dynamic voxel mesh showcase

This engine-only example demonstrates the reusable dynamic voxel APIs without putting engine logic in a game. The world is held in `ribbon_plugin_voxel::ChunkedVoxelWorld`; the example consumes `ChunkMeshUpdate` values, replaces only ECS entities belonging to dirty chunks, and attaches the resulting local meshes at each chunk's `origin_y_up`. The native `Render3dPlugin` extracts those entities and owns GPU upload/eviction.

Run from the workspace root:

```powershell
cargo run -p voxel_mesh_showcase
# Optional finite run for a bounded capture or smoke check:
cargo run -p voxel_mesh_showcase -- --frames 240
```

The world is no longer edited by a timed script. You control edits directly so you can observe invalidation and mesh replacement at your own pace.


## Controls and interactive editing

The control guide and live diagnostics are drawn inside the window.

- **D / A**: increase/decrease X. **W / S**: increase/decrease Y in the source grid.
- **R / F**: increase/decrease the selected voxel coordinate along Z.
- **E**: place the selected block type at the target coordinate.
- **Q**: remove the block at the target coordinate.
- **C**: cycle through block types 1–4.
- **T**: reset the voxel world, target coordinate, camera, and edit counter.
- **Left / Right**: orbit the camera; **Up / Down** adjust camera elevation.
- **Hold right mouse button and drag** to orbit; the **mouse wheel** zooms.

A small marker floats above the selected cell. The overlay reports the target and owning chunk, block id, world revision, dirty chunks, the last non-empty chunk-remesh count, edit count, last action, and renderer-published GPU mesh cache/upload/release counters. Those renderer counters are from the latest published render diagnostics and can lag an edit by one frame. Byte counts are estimates where applicable, not driver VRAM readings.

Use the initial selected coordinate `(7, 5, 0)` to start: it lies on the X and Y chunk-face boundary intersection, so removing/replacing its floor voxel invalidates the owner and both adjacent chunks. Selection is clamped to the actual world bounds, edits are one-shot per key press, and reset rebuilds a deterministic initial world.

## Render profiling capture

Run the example in release mode with a finite frame count to write a CSV report after shutdown.
The default warmup is 120 successfully rendered frames; set `--profile-warmup N` to change it.
For example:

```powershell
cargo run --release -p voxel_mesh_showcase -- --frames 1800 --profile-csv target/profiles/voxel-mesh-showcase.csv
python tools/render_profile_report.py target/profiles/voxel-mesh-showcase.csv --output target/profiles/voxel-mesh-showcase.md
```

Compare runs only on the same machine, GPU adapter, resolution, build profile, scene setup, and
frame-count/warmup settings. GPU timestamps are asynchronous and the current GPU interval covers
shadow plus mesh rendering, not the whole frame. Resource byte counts are estimates, not driver VRAM readings.
