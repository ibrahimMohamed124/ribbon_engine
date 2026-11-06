
## Unreleased — Patch 06.2 CSM transition diagnostic correction

- Corrected the camera-sweep test to solve for the measured receiver-pixel view depth. It now samples
  immediately before and after each real `SplitFar` and asserts that GPU CascadeIndex enters the
  following cascade after the boundary.
- Added `ShadowVisibilityComponents` to both static and skinned color shaders. RGB exposes current
  PCF visibility, next-cascade PCF visibility, and the final blended result; the normal grayscale
  visibility mode shares the same implementation.
- The GPU test scans overlap pixels for a non-degenerate shadow-edge probe, verifies the final
  visibility interpolation against the sampled inputs within 3/255 quantization tolerance, and
  checks that the grayscale final output matches the component output. It no longer treats a
  permanently fully-shadowed center pixel as evidence of correct blending.
- No cascade split, transition-width, bias, PCF-kernel, or shadow-resolution parameters changed.
  Arbitrary continuous camera motion and texel stability remain separate validation tasks.

# Changelog

Notable user-facing changes are recorded here. Unreleased entries must be reviewed before creating a
version tag. The workspace is currently pre-1.0; this file does not imply a released version.

## Unreleased

### CSM texel-radius stabilization follow-up

- Windows validation exposed small f32 variation in the reconstructed cascade sphere radius. Since the light-space snap grid is anchored in world space, that tiny scale variation could shift the grid by a visible fraction of a texel. The fitted radius is now quantized to the nearest 1/16 world-unit step with an added 1/32 world-unit conservative guard before deriving the 1024² texel size. The final radius never shrinks the fitted sphere and adds less than 1/16 world unit; cascade splits, transition width, bias, PCF, and texture resolution are unchanged.
- Added a direct regression for the observed radius pair, normalized test camera quaternions, and changed the far-plane assertion to allow a small f32 unprojection tolerance while continuing to check every far-slice corner against the light clip volume.
- These are source changes only; the three CPU texel-stability tests and GPU depth-layer sweep must be rerun on the Windows host before this stabilization is accepted.

### CSM light-space depth-fit stabilization

- Windows sub-texel validation showed that the snapped XY origin and world-space texel size could remain fixed while small f32 inverse-frustum reconstruction differences changed the fitted light-space Z interval and raw-radius-derived depth padding, perturbing the full shadow matrix.
- The fitted sphere radius is now shared by the XY extent and depth-padding calculation. The fitted light-space depth minimum is rounded down and maximum rounded up to 1/16 world-unit increments before applying the existing depth padding. This is conservative: it preserves or expands caster depth coverage and stabilizes the orthographic depth mapping for small reconstruction noise within the same XY snap cell.
- Added CPU coverage for conservative, stable depth-range quantization and for reuse of the same quantized radius in XY extent and depth padding. The per-cascade sub-texel regression now checks stable XY snap origin, texel size, quantized Z bounds, depth padding, and the complete matrix; one-texel crossing assertions remain strict.
- This changes only fitted light-space Z range/padding stability; cascade splits, transition width, depth/normal bias, PCF kernel, shadow-map resolution, and spotlight shadows are unchanged. GPU depth-layer comparison remains required on the Windows host.

### CSM texel-snapping and stability validation

- Extracted the directional light basis, world-space shadow extent/texel-size calculation, and texel-center snapping into shared helpers used by runtime fitting and test-only diagnostics. The initial Patch 07 version left the formulas unchanged; this follow-up quantizes the fitted radius as described above.
- Added CPU tests for all four cascades that verify identical shadow matrices within a snap cell, a one-shadow-texel projection step after crossing the rounding boundary, and stable snapped XY origins/texel scale under small yaw and pitch rotations.
- Added far-cascade frustum coverage checks through the camera far plane under five rotated camera orientations.
- Added an ignored 1024² GPU depth-layer regression that compares all four depth-array layers at sub-texel and one-texel camera offsets, emitting per-cascade PNGs and `texel_stability.csv`. This GPU test still requires execution on a supported host; it is a discrete capture, not a continuous temporal recording.
- No cascade split, transition width, depth/normal bias, PCF kernel, or shadow resolution values were changed. Per-cascade caster culling and configurable shadow distance/resolution remain deferred.

### Renderer mesh upload layouts and glTF tangent/UV support

- Added separate 56-byte static and 104-byte skinned GPU vertex streams. The static stream carries position, normal, UV0, tangent/handedness and UV1; the skinned stream additionally retains all eight joint influences. The legacy 80-byte CPU `Vertex` and `MeshData` input remains source-compatible; zero-weight data is adapted to the 56-byte static stream and weighted data to the 104-byte skinned stream.
- The glTF importer now preserves valid `TANGENT`, `TEXCOORD_0`, `TEXCOORD_1`, and each supported material texture's `texCoord` selection. Missing/invalid tangent frames use an area-weighted indexed-triangle/UV0 fallback; absent UV1 falls back to UV0. UV sets above 1 and `KHR_texture_transform` remain unsupported, and UV1 normal maps derive their tangent frame from screen-space derivatives. The fallback is not full MikkTSpace, and GPU-backed UV1/tangent visual validation is still required.

### Immediate-mode UI

- Added `ribbon_ui` with vertical/horizontal layouts, cross-axis alignment, anchored panels, clipped scroll areas, keyboard/gamepad focus and repeat, single-line caret editing, IME pre-edit/candidate positioning, cosmic-text advanced shaping, an R8 glyph atlas with dirty-region uploads, and an ordered GPU overlay pass. The `ui_showcase` example uses typed app states. Selection/clipboard/undo, multiline input, grapheme-aware caret movement, color emoji and full RTL layout mirroring remain out of scope; GPU visual verification is still required on a compatible host.

### Gameplay AI

- Added generic ECS state machines, resumable behavior trees, grid A* navigation with smoothing and physics baking, steering output, and budgeted line-of-sight perception. Navmesh and multi-floor navigation remain out of scope.

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
- ATHAR now registers `AudioPlugin`, enables Kira output through its default `audio` feature (while retaining a no-default-features headless build), attempts device initialization without failing the game when no device is available, and emits procedural ambience, footstep, door, key, and save-point audio events. Save-point autosave and `--continue` persistence are implemented; village restoration remains limited to the room-start fallback, while monster movement and door-close gameplay remain unimplemented.
- ATHAR resolves assets from command-line/environment overrides or beside the executable, with a debug-only source fallback. Added an ATHAR package builder and a separate CI artifact path while retaining the STUDS artifact. CI's package script smoke-tests relocated asset resolution; CI execution remains the verification authority.

### CSM texel-stability test calibration

- Corrected the radius-quantization regression to account for both the conservative 1/32-world-unit
  quantization guard and the existing 1.5-texel projection guard. The test now checks the helper's
  full returned extent against the explicit expected formula rather than applying the quantization
  bound to an extent that already includes the projection guard.
- Replaced one-shot camera alignment in the sub-texel matrix test with iterative feedback against
  the measured runtime snap cell. Test positions retain a deliberate margin from the half-texel
  rounding boundary, while the one-texel crossing assertion remains unchanged in strength.
- No runtime cascade fitting, split, blend, bias, PCF, resolution, or spotlight behavior changed.

### CSM diagnostic visualization

- Added runtime `DirectionalShadowDebugOptions` to `Render3dPlugin`, disabled by default and configurable as a world resource without rebuilding renderer pipelines.
- Added `CascadeIndex` (red/green/blue/yellow), `BlendWeight` (green-to-red transition weight), `DepthLayer` (letterboxed grayscale view of an individual `Depth32Float` array layer), and later `ShadowVisibility` (the actual blended 3×3-PCF result) diagnostic modes. Static and skinned color shaders share the cascade-selection and blend-weight calculation used by the normal shadow visibility path.
- Added an ignored GPU capture test that writes six 1280×720 PNGs: cascade index, blend weight, and each of the four depth-array layers. These captures support visual review but do not prove temporal stability; camera translation/rotation and shadow shimmering checks remain separate work.
- Diagnostic modes are disabled by default, preserve the regular shadow-generation passes, and suppress particles in mesh-color views so they do not obscure the diagnostics.

### Cascaded directional shadows and vertex-layout routing

- Replaced the single directional depth map with four 1024² `Depth32Float` layers in a texture array. Practical splits use a 65% logarithmic / 35% linear blend; each overlapped camera-frustum slice gets its own rotation-invariant, texel-snapped light matrix. The shader performs 3x3 comparison PCF and blends into the next cascade over the final 10% of each non-final split.
- Kept the selected spotlight on its independent perspective depth map. Added separate per-cascade shadow uniform buffers so all four matrices remain distinct within one frame, and updated GPU resource estimates for the extra depth layers and uniforms.
- Routed static and skinned meshes to matching color, directional-shadow and spotlight-shadow pipelines.
- Added CPU tests for practical split ordering, clip-space coverage of each overlapped slice, transition-window coverage, and invalid-camera fallback. The base CSM GPU smoke test was subsequently confirmed on NVIDIA RTX 2050/Vulkan; visual transition and temporal-stability diagnostics are tracked separately. Per-cascade caster culling and configurable shadow distance/resolution remain deferred.

### 3D lighting and directional shadow stability

- The initial directional-shadow implementation derived coverage from the active camera frustum, used a rotation-invariant light-space extent, snapped its light-space XY center to the shadow texel grid, and fitted depth coverage to the frustum. At that milestone, cascades and configurable shadow distance/resolution were still deferred; the follow-up CSM milestone is recorded in the dedicated section above.
- Point-light uniform selection now ranks valid lights by estimated camera-space contribution (distance, range, intensity, and color luminance) and uses a stable value tie-breaker instead of ECS extraction order. The shader capacity remains eight and point lights remain unshadowed.
- Added CPU regression coverage for frustum coverage, sub-texel camera translation stability, invalid camera matrices, importance ranking, camera movement, deterministic tie-breaking, invalid lights, and shader-capacity truncation. GPU-backed visual validation and Cargo execution remain required on a supported host.

### HDR post-processing controls

- Added a public `PostProcessingSettings` resource with bounded runtime controls for GPU auto exposure, exposure compensation, contrast, saturation, temperature/tint, vignette, animated film grain and chromatic aberration. Defaults retain the existing fixed-exposure image; a restrained cinematic preset exercises the complete feature set in `graphics_showcase`.
- Auto exposure uses a GPU compute pass to sample a deterministic 16x16 HDR luminance grid, calculate average log luminance and adapt a persistent GPU exposure value with separate brighten/darken rates. The path performs no pixel readback to the CPU and is ordered before tone mapping in the same command buffer.
- Unified FXAA and post-processing in one fullscreen display shader. Runtime setting changes update uniform buffers rather than rebuilding pipelines; FXAA can be toggled without resetting the auto-exposure history. Added CPU sanitization tests, WGSL parse/validation coverage and extended the ignored GPU test to execute the cinematic post stack on hardware. Temporal AO reprojection/denoising, G-buffer normal AO, IBL, probes, TAA and additional soft-shadow filtering remain deferred.
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
- Native gamepad discovery, analog axes, bindings and rumble are implemented in `ribbon_platform`; editor text-widget focus/surrounding-text integration and display-settings persistence/HDR metadata remain deferred.

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

### CSM texel-stability probe calibration follow-up

- The sub-texel matrix regression now treats the final measured f32 snap-cell fractions as the actual baseline instead of requiring the reconstructed frustum center to land on an ideal 0.15 fraction to near-machine precision. It enforces a bounded alignment residual and performs all within-cell offsets relative to the measured baseline.
- The one-texel crossing probe now targets 0.65 texel from that measured baseline, leaving a deliberate margin beyond the 0.5 rounding threshold while retaining the exact one-texel snap and projected-pixel assertions. No production shadow-fitting or CSM parameters changed.

### CSM texel-cell probe calibration follow-up

- The test-only snap-cell alignment now uses a damped, bounded feedback correction and records the measured `f32` fractions as the baseline for subsequent camera offsets. It allows at most 0.02 texel-fraction residual from the target rather than requiring an idealized sub-pixel coordinate that the reconstructed frustum may not represent exactly.
- Same-cell probes are derived from the measured baseline and remain at 0.25/0.42 fractions; the snap-crossing probe targets 0.65 to stay clearly beyond the 0.5 rounding boundary. The exact unchanged-matrix and one-texel/one-pixel assertions remain intact. Production shadow fitting and CSM parameters are unchanged.

### CSM light-space depth-fit stability follow-up

- Replaced independently rounded light-space corner minimum/maximum bounds with a conservative interval centered on the fitted cascade sphere's light-space depth. The already-quantized sphere radius is used as the Z half-extent and for depth padding; because that sphere encloses every cascade corner, the new interval preserves coverage while avoiding a one-quantum min/max jump when an extreme corner lies on a quantization boundary.
- The regression continues to compare the complete runtime matrices inside a single XY snap cell and keeps the one-texel crossing assertions. It now checks the stable quantized depth span and padding separately, allowing only the tiny floating-point movement of the projected center to be judged by the strict full-matrix tolerance.
- This slightly more conservative sphere-based Z fit may use a wider depth interval than a tight corner min/max fit. Cascade splitting, transition blending, bias, PCF, shadow resolution, and spotlight fitting are unchanged.
