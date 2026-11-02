# Ribbon Engine — Engine-Wide Upgrade Audit

**Audit date:** 2026-11-02
**Source:** `Ribbon_Engine(5).zip`
**Scope:** Verified source audit plus the completed audio/ATHAR work in this overlay. This report distinguishes code changes from features that remain deferred.

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

## Audited but deferred rendering and platform issues

The following problems were checked against source and are not represented as fixed by this overlay:

| Area | Verified source state | Status in this overlay |
|---|---|---|
| Directional shadows | Fixed camera-following orthographic bounds, fixed-size map; no frustum fitting, texel snapping, or cascades | Deferred |
| Spot-light shadows | One selected shadow-casting spotlight at a time | Deferred; no multi-caster atlas/array |
| Point-light selection | Renderer uploads the first eight extracted point lights; ordering can depend on extraction/query order | Deferred; no importance ranking or priority API |
| Point-light shadows | Point lights use attenuation; no cubemap/array visibility test | Deferred |
| glTF alpha modes | OPAQUE/MASK/BLEND are not represented as distinct GPU material paths | Deferred |
| glTF double-sided | `doubleSided` is not imported into pipeline/culling variants | Deferred |
| MSAA / FXAA | Render targets use sample count 1; no FXAA path | Deferred |
| Post-processing | Existing HDR/tone-map, basic bloom and fog remain; no independent renderer-level quality/settings resource | Deferred |
| Auto exposure / grading / vignette / grain / chromatic aberration | No corresponding implemented runtime effects | Deferred |
| SSAO | No depth/normal-aware screen-space occlusion implementation | Deferred; material AO maps are not SSAO |
| IBL | No environment-cubemap irradiance/prefilter/BRDF integration | Deferred |
| Reflection probes / lightmaps | No complete runtime probe/lightmap data path or baking system | Deferred |
| Vertex formats | One shared `Vertex` carries 8 joint indices and 8 weights for all vertices (80 bytes before any alignment effects) | Deferred; no static/skinned split |
| Tangents / UV1 | glTF importer currently retains UV0; mesh format does not store tangents or UV1; normal mapping uses derivative TBN reconstruction | Deferred |
| Display settings | Basic configured width/height path exists; no complete runtime window-mode/present-mode/resolution settings API | Deferred |
| Keyboard / gamepad / text / IME | Physical key coverage is incomplete; gamepad backend and full IME/text lifecycle are not complete | Deferred |

`docs/architecture/ARCHITECTURE.md`, `plugins/render3d/README.md`, and the new `plugins/gltf/README.md` were aligned with those observed limitations. The status language intentionally avoids describing deferred capabilities as implemented.

## Validation actually performed

- Python compilation: `python -m py_compile tools/package_athar.py` — passed.
- ATHAR asset validator: `python games/athar/tools/validate_horror_assets.py` — passed for 12 self-contained GLBs (24.59 MiB reported by the validator).
- WAV file validation with Python's standard `wave` module — passed for all 10 files (mono/stereo PCM, 22,050 Hz, non-empty frames).
- TOML parsing for workspace manifest, lockfile, affected manifests, and dependency allowlist — passed.
- YAML parsing for `.github/workflows/ci.yml` — passed.
- Relative Markdown link check — passed after correcting the two stale documentation destinations.
- Packaging staging/copy/relocated-working-directory harness — passed with a stub executable only.
- `python tools/check_deps.py` was attempted but stopped because it shells out to `cargo metadata` and `cargo` is absent. This is not a passing dependency-policy result.

The following were **not run**: `cargo fmt`, any `cargo check/test/clippy` command, full workspace tests, actual ATHAR or STUDS launch, Windows packaging against a real executable, graphics-backed validation, and audio-device playback. `cargo`, `rustc`, and `rustfmt` are not installed in the execution environment. No hardware verification is claimed.

## Known limitations

- Kira output is attempted during normal ATHAR startup, but audio-device initialization and actual sound behavior must still be verified on a supported Windows host.
- The save-point interaction currently presents a chime/checkpoint message only; it does not serialize or persist save state.
- Door closing and monster movement/audio triggers are future gameplay hooks, not implemented gameplay.
- The Windows package requires the CI runner to expose the expected MSVC x64 runtime redistributable directory; the packaging step fails rather than silently emitting a Windows package without DLLs when `--require-windows-crt` is used.
- Rendering, display, and input items in the table above remain separate engineering phases.
