# Graphics Showcase

A live windowed scene demonstrating the features integrated into the current
`ribbon_plugin_render3d` pipeline.

Run from the repository root:

```sh
cargo run -p graphics_showcase
cargo run -p graphics_showcase -- --frames 600
cargo run -p graphics_showcase -- --aa none
cargo run -p graphics_showcase -- --aa msaa4
cargo run -p graphics_showcase -- --aa fxaa
cargo run -p graphics_showcase -- --post-preset cinematic
cargo run -p graphics_showcase -- --ao low
cargo run -p graphics_showcase -- --ao medium
cargo run -p graphics_showcase -- --aa msaa4 --ao high --post-preset cinematic
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
- A single directional shadow map, an `Rgba16Float` HDR scene target, Reinhard tone mapping and basic bloom.
- GPU average-log-luminance auto exposure with asymmetric temporal adaptation, plus configurable exposure compensation, contrast, saturation, temperature and tint.
- Optional vignette, animated film grain and pixel-scaled chromatic aberration in the final display pass.
- Optional depth-reconstructed GTAO-style ambient occlusion with Low/Medium/High sample presets; the same feature runs against single-sample and MSAA depth buffers.
- Thin grid lines and continuously moving/rotating objects for visual comparison of no AA, MSAA, and FXAA.
- CPU frustum culling for the main color pass while retaining off-screen objects for shadows.
- Per-frame transforms, camera orbit, and frame-limit shutdown for capture workflows.

`ribbon_plugin_render3d` also accepts greedy-meshed voxel chunks; run
`cargo run -p voxel_mesh_showcase` to see that path.

This is a first PBR/rendering vertical slice, not a complete modern renderer. The renderer now
supports normal/occlusion/emissive maps, glTF OPAQUE/MASK/BLEND, per-material double-sided
pipelines, one frustum-fitted directional shadow map, one selected spotlight shadow map, up to
eight unshadowed point lights, basic bloom and fog. Cascaded/soft shadows, point-light shadowing, temporal AO reprojection/denoising,
IBL, GPU culling and order-independent transparency remain future work. See
[`../../docs/architecture/RENDERER_PLAN.md`](../../docs/architecture/RENDERER_PLAN.md).

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


## glTF alpha-material showcase

The default scene also loads `assets/models/material_alpha_showcase.glb`: a generated five-primitive display wall containing an OPAQUE panel with a deliberately low alpha factor (OPAQUE must ignore it), a textured MASK cutout panel, two overlapping BLEND sheets, and a reversed-winding double-sided panel. The fixture is generated specifically for Ribbon regression/visual inspection and contains no third-party texture or model content. View it in the default scene; it is not loaded in asset-stress mode.

## Anti-aliasing comparison

Pass `--aa none`, `--aa msaa2`, `--aa msaa4`, `--aa msaa8`, or `--aa fxaa` before the renderer creates its first GPU resources. The showcase inserts `AntiAliasingSettings` before `Render3dPlugin` and prints the requested mode at startup. Unsupported MSAA counts fall back to the highest supported lower count; the renderer logs the actual fallback. MSAA sample count is fixed for the renderer lifetime, so restart the showcase when comparing sample counts. The default scene includes sub-pixel-thin floor grid lines and animated/rotating cubes; compare captures from the same camera/frame state. FXAA is a post-tone-map spatial filter, not TAA.

An adapter-backed smoke test is available through:

```powershell
cargo test --locked -p ribbon_plugin_render3d multisampled_hdr_resolves_then_fxaa_tonemaps_on_gpu -- --ignored --nocapture
```

That ignored GPU test requires an adapter supporting a multisampled, filterable `Rgba16Float` HDR color target with resolve plus the matching `Depth32Float` sample count. If the adapter does not expose that pair, the engine falls back to 1x and the GPU test should be run on another supported adapter.

## HDR post-processing controls

The `PostProcessingSettings` resource is installed by `Render3dPlugin` and can be replaced or mutated at runtime. Its defaults preserve the existing fixed-exposure look. `PostProcessingSettings::cinematic()` enables GPU auto exposure, restrained color grading, vignette, film grain and chromatic aberration. Auto exposure computes a 16x16 average-log-luminance sample grid directly from the HDR texture in a compute pass, then adapts a persistent GPU exposure value using separate brighten/darken rates; it performs no CPU readback. All user-facing floats are sanitized before upload.

```powershell
cargo run --locked -p graphics_showcase -- --post-preset neutral
cargo run --locked -p graphics_showcase -- --post-preset cinematic
cargo run --locked -p graphics_showcase -- --aa msaa4 --post-preset cinematic
```

The example preset is intended for visual verification, not as a mandatory game look. The effects are independently configurable through `AutoExposureSettings`, `ColorGradingSettings`, `VignetteSettings`, `FilmGrainSettings` and `ChromaticAberrationSettings`.

## Screen-space ambient occlusion

`--ao off|low|medium|high` selects the optional depth-based ambient-occlusion pass. The default is `off`; Medium is a balanced six-direction/three-step kernel, while High uses eight directions/four steps. The pass reconstructs world positions and approximate normals from the active camera inverse view-projection and main depth texture, then applies visibility to the scene HDR contribution before bloom and tone mapping. It is a spatial screen-space approximation: without a G-buffer it cannot isolate ambient illumination from direct lighting, and it does not use temporal reprojection or history denoising. Its full-resolution Rgba8 visibility target is retained with the tone mapper even when disabled; compute dispatch is skipped while off. Combine it with MSAA to exercise the multisampled depth-load path.
