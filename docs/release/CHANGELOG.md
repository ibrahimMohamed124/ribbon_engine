# Changelog

Notable user-facing changes are recorded here. Unreleased entries must be reviewed before creating a
version tag. The workspace is currently pre-1.0; this file does not imply a released version.

## Unreleased

### Save/load

- Added versioned save sections, CRC-checked RON slot files, atomic replacement with backup recovery, section migrations, and ATHAR autosave/continue support. Save I/O remains synchronous; village restoration is limited.

### Engine particles

- Added renderer-neutral particle batch contracts, deterministic CPU emitters and presets, and replaced ATHAR's rain meshes with `ParticleEmitter::rain()`. The weather plugin was removed. GPU visual output remains unverified here.

### Gamepad input

- Added four-pad button/axis state processing, radial stick and trigger deadzones, trigger hysteresis, gamepad binding sources, threaded gilrs polling, event output, rumble requests, and a virtual raw-event injection path. Native device behavior remains unverified without hardware.

### Application states

- Added typed `State<S>` / `NextState<S>` resources, deferred transitions, enter/exit hooks, transition events, and `in_state` run conditions. State requests made from hooks are applied on the following frame.

### Audio / ATHAR runtime and packaging

- The modern audio ECS transform path now reads `GlobalTransform3D` and derives listener orientation from the world quaternion using Y-up / local -Z-forward. Legacy `AudioListener` remains a separate compatibility path; `ribbon_math` and STUDS coordinate conventions are unchanged.
- Added regression tests for front/back/left/right/above/below spatial direction, listener quaternion rotation, invalid transforms, transformed acoustic zones, and listener removal. Parent hierarchy and device-backed playback still require the Rust test suite and runtime validation on a supported host.
- ATHAR now registers `AudioPlugin`, enables Kira output through its default `audio` feature (while retaining a no-default-features headless build), attempts device initialization without failing the game when no device is available, and emits procedural ambience, footstep, door, key, and save-point audio events. Save-point persistence, monster behavior, and door-close gameplay remain unimplemented.
- ATHAR resolves assets from command-line/environment overrides or beside the executable, with a debug-only source fallback. Added an ATHAR package builder and a separate CI artifact path while retaining the STUDS artifact. CI's package script smoke-tests relocated asset resolution; CI execution remains the verification authority.

### 3D lighting and directional shadow stability

- Directional shadow projection now derives coverage from the active camera frustum, uses a rotation-invariant light-space extent, snaps its light-space XY center to the shadow texel grid, and fits depth coverage to the frustum. Cascades and configurable shadow distance/resolution remain deferred.
- Point-light uniform selection now ranks valid lights by estimated camera-space contribution (distance, range, intensity, and color luminance) and uses a stable value tie-breaker instead of ECS extraction order. The shader capacity remains eight and point lights remain unshadowed.
- Added CPU regression coverage for frustum coverage, sub-texel camera translation stability, invalid camera matrices, importance ranking, camera movement, deterministic tie-breaking, invalid lights, and shader-capacity truncation. GPU-backed visual validation and Cargo execution remain required on a supported host.

### HDR post-processing controls

- Added a public `PostProcessingSettings` resource with bounded runtime controls for GPU auto exposure, exposure compensation, contrast, saturation, temperature/tint, vignette, animated film grain and chromatic aberration. Defaults retain the existing fixed-exposure image; a restrained cinematic preset exercises the complete feature set in `graphics_showcase`.
- Auto exposure uses a GPU compute pass to sample a deterministic 16x16 HDR luminance grid, calculate average log luminance and adapt a persistent GPU exposure value with separate brighten/darken rates. The path performs no pixel readback to the CPU and is ordered before tone mapping in the same command buffer.
- Unified FXAA and post-processing in one fullscreen display shader. Runtime setting changes update uniform buffers rather than rebuilding pipelines; FXAA can be toggled without resetting the auto-exposure history. Added CPU sanitization tests, WGSL parse/validation coverage and extended the ignored GPU test to execute the cinematic post stack on hardware. Temporal AO reprojection/denoising, G-buffer normal AO, IBL, probes, TAA and cascaded/soft shadow improvements remain deferred.
- Added optional depth-reconstructed GTAO-style ambient occlusion with deterministic Low/Medium/High horizon sampling. The pass reads the main `Depth32Float` attachment, supports both single-sample and MSAA depth textures, writes a visibility target on GPU, and composites it into HDR scene lighting before bloom/tone mapping. It is disabled by default and intentionally does not claim temporal reprojection, history denoising or a G-buffer normal path.

### Rendering anti-aliasing

- Added `AntiAliasingSettings` and the `None`, 2x/4x/8x MSAA, and FXAA modes to `ribbon_plugin_render3d`. MSAA capability negotiation intersects HDR color resolve and depth-format sample support; unsupported requests fall back to the highest lower common sample count.
- The main 3D color pass now uses matching multisampled HDR color/depth attachments and resolves into the single-sample HDR scene target before bloom and tone mapping. Resize recreates size-dependent MSAA targets. MSAA sample count is latched when renderer GPU resources are first created; runtime sample-count reconfiguration is deferred to preserve live GPU handles. FXAA can be toggled at runtime in the fullscreen post-tone-map pass.
- Added sample-count negotiation/capability tests, FXAA WGSL validation, and an ignored GPU integration test for MSAA resolve followed by FXAA. The showcase's remaining-feature list no longer reports MSAA/FXAA as missing.
- `ribbon_render::gpu_window::FrameTarget` now exposes the selected `wgpu::Adapter` so render passes can query format/sample-count capabilities before allocating optional multisampled targets; downstream code constructing `FrameTarget` literals must provide this adapter reference.

### Platform display and input foundation

- Expanded physical keyboard mapping from the original small subset to all 194 standard `winit 0.30.13` `KeyCode` variants, including international/language, media/browser/system and F1-F35 keys. Added exhaustive unique key-name and mapping coverage tests; left/right modifiers remain distinct.
- Added frame-local Unicode text commits and IME pre-edit/lifecycle events, safe UTF-8 byte-boundary normalization for native composition cursor offsets, and a `WindowHandle` hook to position the operating-system IME candidate window. Text input remains separate from physical gameplay bindings. Unidentified native physical scan codes are now preserved for local rebinding (not portable serialized bindings).
- Added runtime `DisplaySettings` for Windowed/Borderless/Exclusive fullscreen, optional validated physical-resolution overrides, exact-size exclusive mode lookup with borderless fallback, and capability-negotiated VSync (`On`, `Adaptive`, `Off`). Clearing a resolution override follows `PlatformConfig`; clearing a VSync override restores the environment-selected present mode.
- The native gamepad backend, editor text-widget focus/surrounding-text integration and display-settings persistence/HDR metadata remain deferred.

### Rendering API / migration

- The legacy `ribbon_plugin_rendering::gpu::voxel::GpuRenderer` / `GpuPresenter`, the `gpu` feature,
  and its ECS backend selector were removed. New GPU 3D applications use `ribbon_plugin_render3d`;
  existing CPU legacy-scene applications use `ribbon_plugin_rendering::cpu` and its CPU-only ECS bridge.
- Documented `SurfaceShader::{albedo, albedo_with_materials, entity_surface}` as CPU-only extension
  callbacks. They are not translated into mesh GPU shader behavior. GPU materials use the data-driven
  `PbrMaterialDesc`/`TextureData` API; offscreen voxel chunks currently support per-block linear RGB
  tint through `VoxelBlockColor`, not per-block PBR texture assignment.
- Added behavioral tests asserting custom CPU surface callbacks affect CPU output and deterministic
  palette tests for the offscreen voxel tint/fallback contract.
- Added `docs/history/RENDERING_API_CONTRACTS.md` and `docs/release/RELEASE_CANDIDATE_CHECKLIST.md` to make current
  public API boundaries, intentional breaking changes, and outstanding release gates explicit.

### Interactive showcases

- Added a reusable alpha-blended `WindowTextOverlayPass` using the shared engine bitmap font for
  compact native-window diagnostics and control guides.
- Made `graphics_showcase` expose its controls/status in-window, support right-mouse camera orbit, mouse-wheel zoom, bounded camera movement, and a complete reset path.
- Made `voxel_mesh_showcase` interactive with bounded source-grid selection, block placement/removal/material cycling, deterministic reset, a floating target marker, and live chunk/mesh diagnostics.
- Expanded showcase input, selection, reset, overlay geometry, and chunk-boundary regression coverage.
- Extended CI definitions for no-default-feature Clippy, targeted showcase regressions,
  release-example builds, and a source audit guarding against reintroduction of the retired GPU
  voxel raycaster. These workflow definitions have not been run as part of this change.
