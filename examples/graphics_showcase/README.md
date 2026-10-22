# Graphics Showcase

A live windowed scene demonstrating the features integrated into the current
`plugin_render3d` pipeline.

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

`plugin_render3d` also accepts greedy-meshed voxel chunks; run
`cargo run -p voxel_mesh_showcase` to see that path.

This is a first PBR/rendering vertical slice, not a complete modern renderer. Normal,
occlusion and emissive maps, alpha modes, cascaded/soft shadows, point/spot lights,
ambient occlusion, bloom, fog, IBL, GPU culling and STUDS migration remain future
work. See [`../../docs/RENDERER_PLAN.md`](../../docs/RENDERER_PLAN.md).

## Render profiling capture

Run the example in release mode with a finite frame count to write a CSV report after shutdown.
The default warmup is 120 successfully rendered frames; set `--profile-warmup N` to change it.
For example:

```powershell
cargo run --release -p graphics_showcase -- --frames 1800 --profile-csv target/profiles/graphics-showcase.csv
python tools/render_profile_report.py target/profiles/graphics-showcase.csv --output target/profiles/graphics-showcase.md
```

Compare runs only on the same machine, GPU adapter, resolution, build profile, scene setup, and
frame-count/warmup settings. GPU timestamps are asynchronous and the current GPU interval covers
shadow plus mesh rendering, not the whole frame. Resource byte counts are estimates, not driver VRAM readings.
