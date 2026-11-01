# Graphics Showcase

A live windowed scene demonstrating the features integrated into the current
`ribbon_plugin_render3d` pipeline.

Run from the repository root:

```sh
cargo run -p graphics_showcase
cargo run -p graphics_showcase -- --frames 600
```

## Controls

- **Space**: pause or resume animated objects, moving directional light, and camera orbit.
- **Left / Right**: reduce or increase orbit speed; continuing left can reverse the orbit.
- **Up / Down**: raise or lower the camera.
- **R**: reset the orbit, camera height, and animation state.

## What it demonstrates

- Native `wgpu` mesh rasterization, indexed geometry, instancing, and depth buffering.
- Built-in cube instances with linear RGB tints.
- ECS extraction of camera, global transforms, models and sunlight.
- glTF/GLB loading, cached imported models, base-color textures and mip chains.
- Cook-Torrance/GGX metallic-roughness material shading.
- A single directional shadow map, an `Rgba16Float` HDR scene target, and Reinhard tone mapping.
- CPU frustum culling for the main color pass while retaining off-screen objects for shadows.
- Per-frame transforms, camera orbit, and frame-limit shutdown for capture workflows.

`ribbon_plugin_render3d` also accepts greedy-meshed voxel chunks; run
`cargo run -p voxel_mesh_showcase` to see that path.

This is a first PBR/rendering vertical slice, not a complete modern renderer. Normal,
occlusion and emissive maps, alpha modes, cascaded/soft shadows, point/spot lights,
ambient occlusion, bloom, fog, IBL, GPU culling and STUDS migration remain future
work. See [`../../docs/architecture/RENDERER_PLAN.md`](../../docs/architecture/RENDERER_PLAN.md).

## Render profiling capture

Run the example in release mode with a finite frame count to write a CSV report after shutdown.
The default warmup is 120 successfully rendered frames; set `--profile-warmup N` to change it.
For example:

```powershell
cargo run --release -p graphics_showcase -- --frames 1800 --profile-csv target/profiles/graphics-showcase.csv
python tools/perf/render_profile_report.py target/profiles/graphics-showcase.csv --output target/profiles/graphics-showcase.md
```

Compare runs only on the same machine, GPU adapter, resolution, build profile, scene setup, and
frame-count/warmup settings. GPU timestamps are asynchronous and the current GPU interval covers
shadow plus mesh rendering, not the whole frame. Resource byte counts are estimates, not driver VRAM readings.

## Large-scene and asset-lifetime stress

The normal showcase remains unchanged. For a dense scene, `--stress-grid SIDE` spawns `SIDE x SIDE`
placements of the same bundled glTF model, so the render pass should upload one model resource set and
instance it many times. The side is capped at 128 (up to 16,384 model placements) to keep accidental
stress requests bounded.

```powershell
cargo run --release -p graphics_showcase -- --stress-grid 32 --frames 1800 --profile-warmup 60 --profile-csv target/profiles/large-scene.csv
python tools/perf/render_profile_report.py target/profiles/large-scene.csv --output target/profiles/large-scene.md
```

For lifecycle churn, `--stress-cycle-frames N` despawns the stress placements every `N` engine frames,
drops the strong `AssetRef<ModelAsset>` lease, explicitly evicts unpinned CPU assets, and asynchronously
requests/pins the model again before respawning the grid. This exercises generation changes, GPU model
cache release/re-upload, weak texture-cache behavior, and the renderer's instance-buffer shrink policy.

```powershell
cargo run --release -p graphics_showcase -- --stress-grid 24 --stress-cycle-frames 300 --frames 1800 --profile-warmup 60 --profile-csv target/profiles/asset-lifecycle.csv
python tools/perf/render_profile_report.py target/profiles/asset-lifecycle.csv --output target/profiles/asset-lifecycle.md
```

The profile records scene placement counts, live GPU model/voxel cache entries, cumulative upload/release
counts, and current instance-buffer capacity. Expect cache-entry counts to fall during inactive cycles;
expect the instance buffer to shrink after 120 consecutive frames at one-quarter capacity or less. The
captured memory values remain renderer estimates, and capture-time upload/reload bursts are intentional in
cycle mode rather than steady-state frame cost. The report includes both the latest and peak values for placement,
cache-entry and instance-capacity metrics so a capture that ends while the grid is inactive still shows the
largest workload it contained. No stress mode is enabled unless its CLI options are passed.
