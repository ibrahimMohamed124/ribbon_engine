# Ribbon Engine — Engine-Wide Upgrade Audit

**Audit date:** 2026-11-04
**Source:** `Ribbon_Engine(5).zip`  
**Scope:** Verified source audit, completed audio/ATHAR work, rendering shadow/light selection, glTF alpha materials, MSAA/FXAA, and HDR post-processing controls. This report distinguishes code changes from features that remain deferred.

## Repository and compatibility baseline

The uploaded archive contains the source tree but no `.git` directory. A Git status/diff therefore cannot be recovered from this copy. The original upload was retained as the comparison baseline; the changes in this overlay are limited to the files listed in the delivery report. The archive already contained unrelated validation logs under `test-results/`; they were preserved.

The `crates/math` source tree and its manifest are unchanged. `ribbon_math` remains a workspace crate and its existing public APIs and dependencies remain available. The only new direct `glam` dependency is in `ribbon_plugin_audio`, at the integration boundary that reads `GlobalTransform3D`. No workspace-wide migration of `Vect`, `Angles`, or the legacy transform system was performed. STUDS source and its Z-up gameplay conventions are unchanged.

## Completed source changes

### 1. Modern 3D audio transforms

- Transform-backed audio listeners, emitters, and zones now read `ribbon_scene::GlobalTransform3D`.
- Listener orientation is derived from the normalized world quaternion: local `-Z` forward, `+X` right, `+Y` up.
- Parent composition is consumed through the scene-propagated global transform. Listener/emitter point positions ignore scale; oriented zone half-extents use world scale mapped to the zone's right/forward/up axes.
- Non-finite listener translation/quaternion data is rejected; malformed velocity is sanitized. Invalid or removed transformed listeners cause managed voices to fade and retry rather than continuing with a stale pose.
- The legacy `AudioListener` (+Z-up) adapter remains separate. An explicitly authored component-only `SpatialAudioListener` pose remains a compatibility fallback when no transform is attached.
- Regression tests were added for front/back/left/right/above/below, listener rotation, parented listener/emitter, oriented/scaled zones, invalid transforms, large finite coordinates, and listener removal. A mock sink verifies event-consumer pan direction.

**Not verified here:** Rust tests and actual Kira playback, because this execution environment has no Cargo/Rust toolchain or audio device.

### 2. ATHAR audio integration

- ATHAR now has a direct `ribbon_plugin_audio` dependency and registers `AudioPlugin` through the existing app plugin API.
- The `audio` Cargo feature is enabled by default and forwards to `ribbon_plugin_audio/audio`. A no-default-features build keeps a headless configuration without importing Kira's `ThreadedAudioSink`.
- With the default feature, ATHAR attempts to initialize the existing Kira `ThreadedAudioSink`. Initialization failure is logged to stderr and the engine log buffer; the game continues without audio.
- Ten original procedural WAV assets are included: indoor/rain ambience, two door cues, locked-door cue, walking/sprinting footsteps, key pickup, save-point chime, and a future monster cue. Their origin is documented in `games/athar/assets/CREDITS.md`.
- Ambience changes are emitted only on room/village phase changes and use the existing Kira crossfade behavior. Footsteps are distance-cadenced and capped to two catch-up events per update. Key pickup, locked door, unlocking/opening, and save-point interactions emit one-shot spatial events. The monster and door-close assets are packaged but are not triggered by current gameplay; the monster remains a static placeholder and the door is one-way.
- ATHAR's camera carries `SpatialAudioListener`. The indoor acoustic zone uses `AudioZoneVolume`.
- Headless/mockable event tests and a gameplay-audio cadence test were added; they are not executed in this environment.

### 3. ATHAR runtime assets and package path

- Runtime asset resolution is `--assets PATH`, then `RIBBON_ATHAR_ASSETS`, then `assets/` beside the executable, then a debug-build-only checkout fallback.
- Release runtime paths no longer depend on a compile-time checkout location. Remaining `CARGO_MANIFEST_DIR` uses are debug-only fallback or test-only paths.
- Startup prints the selected asset root. `--validate-assets` checks the room manifest, all room and streamed-village models, ten WAV files, manifest, and credits without opening a window/audio device.
- `tools/package_athar.py` stages the executable and full ATHAR assets directory, adds a run guide, optionally bundles MSVC x64 runtime DLLs on Windows, and invokes the staged executable's `--validate-assets` from a separate working directory before publishing the package directory.
- CI now uploads a distinct ATHAR artifact while preserving the existing STUDS artifact.

**Verification boundary:** package staging/copy/relocated-working-directory behavior was exercised with a small stub executable, not a Rust-built ATHAR binary. The real executable's relocated smoke test is wired into CI but CI was not run here.

## 4. First rendering-completion phase

- Directional shadow projection now reconstructs the active camera frustum from the inverse 0..1-depth view-projection matrix. It computes a rotation-invariant bounding-sphere extent in light space, snaps the light-space XY center to the configured 1024² shadow texel grid, and fits the depth interval to the frustum corners with bounded padding. Invalid/singular camera matrices use a finite defensive fallback rather than uploading NaNs.
- Point lights are ranked before uniform packing by an estimated camera-space contribution using inverse-square distance, range, intensity, and linear color luminance. Invalid/non-positive lights are omitted. Ties use a stable lexicographic key made from light values, so ECS query/insertion order does not alter the selected list. The capacity remains eight to match the current Rust uniform and WGSL arrays.
- Patch 01 additionally changed the single spotlight-shadow caster selection from first-in-query to deterministic camera-aware importance ranking with stable value tie-breaking and fallback to the strongest unshadowed candidate if the selected shadow caster is invalid/non-contributing.
- Regression tests cover frustum-corner coverage, sub-texel movement stability, invalid view-projection fallback, camera-distance-sensitive ranking, deterministic insertion-order behavior, equal-score tie-breaking, invalid lights, capacity truncation, and the five spotlight selection cases.

**Not verified here:** These Rust tests were not compiled or executed because this container has no Cargo/Rust toolchain. Matrix behavior was independently checked with a small numerical reproduction, but that is not a substitute for running the Rust tests or a GPU-backed visual test on the user's Windows GPU.

## 5. Anti-aliasing phase

- Added `AntiAliasingSettings` with `None`, `Msaa2x`, `Msaa4x`, `Msaa8x`, and `Fxaa` modes.
- MSAA negotiates a common sample count across `Rgba16Float` filterable HDR resolve support/usages and `Depth32Float` render-attachment support. It renders into matching multisampled color/depth attachments and resolves into the single-sample HDR target before bloom and tone mapping. Unsupported requests fall back downward and ultimately to one sample.
- Resize recreates the size-dependent multisampled color target and the renderer recreates its matching depth target. The selected MSAA count is latched when renderer GPU resources are first created; runtime sample-count changes are deliberately not applied because the renderer owns generation-checked mesh/material handles and sample-dependent pipelines. FXAA can be toggled at runtime as a fullscreen post-tone-map filter; if selected after an MSAA renderer is initialized, it layers over the existing latched MSAA sample count.
- Added pure sample-count/capability intersection tests, FXAA WGSL parser/validator coverage, and an ignored GPU test for HDR MSAA resolve followed by FXAA. TAA is not claimed or exposed.

**Verification boundary:** Rust formatting, Cargo tests, Clippy, and GPU readback were not run in this packaging environment because it has no Cargo/Rust toolchain. The new GPU test must be explicitly run on a compatible adapter and visual output must be reviewed on the target GPU.

## Audited but deferred rendering and platform issues

The following problems were checked against source and are not represented as fixed by this overlay:

| Area | Verified source state | Status in this overlay |
|---|---|---|
| Directional shadows | Four 1024² `Depth32Float` array layers fitted to overlapping camera-frustum slices; practical logarithmic/linear splits, rotation-invariant light-space extent, snapped XY centers, 3x3 PCF and 10% adjacent-cascade blend | Source implementation added; user-side WGSL/GPU validation pending; fixed 1024² resolution and camera-far-plane distance, no per-cascade caster culling |
| Spot-light shadows | One selected shadow-casting spotlight at a time; selection is ranked by deterministic camera-aware contribution with value-based tie-breaking | Partially implemented; no multi-caster atlas/array, per-light capacity setting, or dedicated multi-spot GPU visual proof |
| Spotlight selection | Single shadow slot prefers the most important valid shadow caster and is independent of ECS order | Patch 01 user-verified: five selection tests and Render3D Clippy passed; broader multi-slot shadows remain deferred |
| Point-light selection | Eight lights are selected by deterministic estimated contribution using camera distance, range, intensity, and color; value-based tie breaking removes ECS iteration-order dependence | Implemented, limited; fixed shader capacity, no explicit per-light developer priority |
| Point-light shadows | Point lights use attenuation; no cubemap/array visibility test | Deferred |
| glTF alpha modes | OPAQUE ignores factor/texture alpha; MASK multiplies factor and texture alpha and discards below cutoff in color and shadow passes; BLEND uses straight-alpha blending, depth test/no depth write, and deterministic camera-space back-to-front bounds-center sorting | Implemented, limited; no OIT, blend objects do not cast current shadows |
| glTF double-sided | Imported; per-material no-cull color and directional-shadow variants, back-face normal flipping; ordinary materials keep back-face culling | Implemented |
| MSAA / FXAA | Adapter-negotiated 2x/4x/8x MSAA uses matching HDR color/depth targets with resolve before post-processing; FXAA is selected in the fullscreen post-tone-map pass | Implemented, limited; no TAA; MSAA sample count is fixed at renderer initialization, FXAA can toggle at runtime |
| Post-processing | Public `PostProcessingSettings` resource controls the retained HDR/bloom/tone-map pipeline; runtime updates use uniform buffers without rebuilding pipelines | Implemented, limited; one bloom level and Reinhard tone mapping remain |
| Auto exposure / grading / vignette / grain / chromatic aberration | GPU average-log-luminance compute adaptation, exposure compensation, contrast/saturation/temperature/tint, vignette, animated monochrome grain, and radial chromatic aberration | Implemented, limited; 16x16 sampling rather than histogram/local exposure; no physically calibrated camera model |
| Screen-space AO | Optional deterministic depth-reconstructed GTAO-style horizon pass; supports single-sample and MSAA depth | Implemented, spatial only; temporal reprojection, denoising and geometric-normal/G-buffer path deferred |
| IBL | No environment-cubemap irradiance/prefilter/BRDF integration | Deferred |
| Reflection probes / lightmaps | No complete runtime probe/lightmap data path or baking system | Deferred |
| Vertex formats | `StaticVertex` is 56 bytes (position, normal, UV0, tangent/handedness, UV1); `SkinnedVertex` is 104 bytes and preserves eight influences; the legacy 80-byte CPU `Vertex` remains as an upload adapter that selects static or skinned layout from weights | Implemented in source, Rust verification outstanding; static GPU path no longer carries skinning data |
| Tangents / UV1 | glTF import preserves authored `TANGENT`, UV0/UV1, and each supported material texture's `texCoord`; missing/invalid tangents use an area-weighted UV0 fallback; UV1 normal maps use derivative TBN | Partially implemented; not full MikkTSpace, no `KHR_texture_transform`, UV sets >1 fall back to UV0, visual verification outstanding |
| Display settings | `DisplaySettings` applies Windowed/Borderless/Exclusive modes, optional physical-resolution overrides, exact-size exclusive mode selection with borderless fallback, and VSync policy negotiation against surface capabilities. Clearing a resolution override follows `PlatformConfig`; clearing VSync restores the initial environment-selected present mode. | Runtime foundation implemented; no persisted settings format, HDR metadata/display calibration API, multi-monitor selector, or native runtime verification in this environment |
| Keyboard | All 194 standard `winit 0.30.13` `KeyCode` variants map to physical `Key` values; unidentified physical keys preserve platform-native scan-code identities for local rebinding; stable names round-trip for the portable catalogue; focus loss releases held keys. | Platform mapping implemented; native scan-code persistence remains application-specific and those codes are not portable |
| Text / IME | Unicode commits, pre-edit composition, enable/disable lifecycle, UTF-8-safe byte cursor range, and native candidate/caret rectangle hook are exposed separately from gameplay keys. | Implemented platform event foundation; editor/UI widget focus ownership, editing commands, surrounding-text replacement, and platform IME runtime matrix remain deferred |
| Gamepad | `GamepadSet` supports four slots, button edges, analog axes, radial stick/trigger deadzones and hysteresis; `ribbon_platform` polls gilrs on a dedicated thread and forwards rumble requests | Implemented in source; no physical controller/hot-plug/rumble hardware validation is claimed |

`docs/architecture/ARCHITECTURE.md`, `plugins/render3d/README.md`, and the new `plugins/gltf/README.md` were aligned with those observed limitations. The status language intentionally avoids describing deferred capabilities as implemented.

## Platform input and display checkpoint (source update)

- Expanded the platform-neutral physical `Key` catalogue and winit mapping to all 194 standard `KeyCode` variants documented by the workspace's pinned winit 0.30.13 line. Key-name serialization helpers round-trip every portable catalogue entry; left/right modifiers remain distinct. Native `PhysicalKey::Unidentified` scan-code identities are preserved by `Key::Native` for local rebinding, while portable name serialization intentionally excludes them. Mapping uniqueness/coverage and native-code state handling have regression tests.
- Added committed Unicode text and IME pre-edit/lifecycle events as a separate input stream, a frame-local `TextInput` resource, UTF-8 boundary clamping for byte-indexed composition cursor ranges, and a native IME candidate/caret rectangle hook on `WindowHandle`. This is a platform event foundation, not a full editor text-widget implementation.
- Added runtime `DisplaySettings` for Windowed, BorderlessFullscreen and ExclusiveFullscreen modes, an optional validated physical-resolution override, exact requested-size exclusive mode lookup, borderless fallback, and VSync selection from actual surface capabilities. Clearing the resolution override follows the legacy `PlatformConfig` dimensions; `vsync = None` preserves the original environment-selected present mode and restores it when a runtime override is cleared.
- Remaining platform work is explicit: persistable display preferences/HDR metadata, multi-monitor policy, and editor widget focus/surrounding-text lifecycle. Native gilrs polling, analog axes/triggers/deadzones and the rumble request path are implemented in source, but physical controller behavior has not been hardware-verified.

**Validation boundary for this source update:** static source consistency and catalogue cardinality checks were performed in the packaging environment. Rust formatting, Cargo tests, Clippy, native fullscreen transitions, swapchain reconfiguration and real IME behavior were not executed here because Cargo/Rust are unavailable.

## Validation actually performed

- Python compilation: `python -m py_compile tools/package_athar.py` — passed.
- ATHAR asset validator: `python games/athar/tools/validate_horror_assets.py` — passed for 12 self-contained GLBs (24.59 MiB reported by the validator).
- WAV file validation with Python's standard `wave` module — passed for all 10 files (mono/stereo PCM, 22,050 Hz, non-empty frames).
- TOML parsing for workspace manifest, lockfile, affected manifests, and dependency allowlist — passed.
- YAML parsing for `.github/workflows/ci.yml` — passed.
- Relative Markdown link check — passed after correcting the two stale documentation destinations.
- Packaging staging/copy/relocated-working-directory harness in this environment passed with a stub executable only.
- The user subsequently reported on Windows that `cargo fmt --all -- --check`, `cargo test -p ribbon_scene` (9/9), both ATHAR test configurations (27/27 each), both Audio test configurations after the mock `Debug` fix, the Steam Audio stress script (32/64/128 voices), and Release ATHAR packaging passed. The user's packaging log confirms 10 VC145 CRT DLLs, five room entities, 12 room/village models, and 10 audio assets. These are user-reported host results, not commands executed in this container.
- `python tools/check_deps.py` was attempted here but stopped because it shells out to `cargo metadata` and `cargo` is absent. This is not a passing dependency-policy result.

The following were **not run here**: Rust formatting, any Cargo check/test/clippy command, full workspace tests, real ATHAR or STUDS launch, Windows packaging against a real executable, graphics-backed validation, and audio-device playback. The alpha-material GPU regression is included as an ignored test requiring an actual wgpu adapter; the default graphics showcase also loads the generated material-alpha GLB fixture, but neither was executed in this container. `cargo`, `rustc`, and `rustfmt` are not installed in this execution environment. The new directional-shadow regression tests are CPU-side matrix checks and have not been compiled or executed here. No GPU visual verification is claimed.

## Known limitations

- Kira output is attempted during normal ATHAR startup, but audio-device initialization and actual sound behavior must still be verified on a supported Windows host.
- The save-point interaction currently presents a chime/checkpoint message only; it does not serialize or persist save state.
- Door closing and monster movement/audio triggers are future gameplay hooks, not implemented gameplay.
- The Windows packaging helper now discovers VC145/VC143 CRT redist directories with `vswhere`/Visual Studio layout discovery, and supports an explicit `--crt-dir`; it still fails rather than silently omitting the runtime when `--require-windows-crt` is used. The user reported a successful package stage bundling 10 VC145 CRT DLLs.
- Phase 3B implements GPU average-log-luminance auto exposure, exposure compensation, contrast/saturation/temperature/tint grading, vignette, animated film grain and chromatic aberration on top of Reinhard/bloom/fog. Four directional shadow cascades are now source-implemented with practical splits, depth-array sampling, 3x3 PCF and transition blending; Windows Cargo/Naga and GPU visual validation remain required. Per-cascade caster culling, configurable shadow distance/resolution, multi-caster spotlight shadows and point-light shadows remain deferred. Temporal AO reprojection/denoising, IBL, probes/lightmaps, MikkTSpace-compatible tangent generation, `KHR_texture_transform`, higher UV sets, display-settings persistence/HDR metadata, editor widget text focus/surrounding-text management, and TAA also remain open. Static/skinned vertex routing is explicitly selected by GPU layout in color and shadow passes. glTF BLEND is conventional object-bounds-center sorting, not OIT; blend materials do not cast the current shadow maps and intersecting surfaces can sort imperfectly. The material showcase is a small regression fixture, not an artist-authing tool or OIT demo.
