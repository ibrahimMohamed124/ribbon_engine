# Ribbon Engine — Engine-Wide Gap Closure Plan

**Baseline:** `Ribbon_Engine_Phase4_Full_Recovery_GAP_CLOSURE_CANDIDATE_V4.zip`
**Audit date:** 2026-11-03
**Scope:** Remaining requirements in the comprehensive engine-wide upgrade task.
**Compatibility invariant:** `crates/math`, `ribbon_math` APIs, the legacy Scene Transform system, and STUDS Z-up gameplay remain outside the change surface.

## 1. Source baseline and verification boundary

The V4 archive contains 691 workspace files and no `.git` directory. Consequently this delivery cannot establish the user's current working-tree status or create phase commits. Work is applied to the supplied V4 source snapshot and delivered as a relative-path overlay. The user's local changes must be preserved by applying only the listed overlay files after taking a backup.

V4 already contains substantial implementation: modern `GlobalTransform3D` audio spatialization, ATHAR AudioPlugin integration and procedural WAV assets, relocatable ATHAR packaging/CI artifact, glTF OPAQUE/MASK/BLEND and `doubleSided`, negotiated MSAA and FXAA, configurable HDR post effects, depth-reconstructed GTAO, deterministic point-light ranking, radial/deadzone gamepad processing with a gilrs thread, Unicode/IME platform events, and runtime window-mode/resolution/VSync settings. The state of these systems is recorded in `ENGINE_WIDE_UPGRADE_AUDIT.md`; this plan does not propose replacing them.

Source-level gaps verified in V4:

- One spotlight shadow map is allocated; the selected caster was previously the first `casts_shadows` record in extraction order. Patch 01 now ranks the single supported caster deterministically by camera-aware contribution.
- Directional shadows now use four fixed-resolution frustum-slice maps in one depth texture array; shadow distance follows the camera far plane and per-cascade caster culling remains deferred.
- Point lights are unshadowed and capped at eight in the Rust/WGSL uniform contract.
- The original renderer had one 80-byte interleaved CPU `Vertex` format with eight joint indices and weights on every vertex. Patch 02 introduced 56-byte static and 80-byte compatibility GPU paths; Patch 03 expands the explicit GPU layouts to 56-byte static and 104-byte skinned to carry tangent/UV1 while preserving the 80-byte legacy CPU input.
- Patch 03 now imports `TANGENT`, `TEXCOORD_0`, `TEXCOORD_1`, and per-texture `texCoord` selectors. Missing/invalid tangents use an area-weighted triangle/UV0 fallback; exact MikkTSpace seam matching and `KHR_texture_transform` remain unsupported.
- Bloom is one extraction/blur pass; GTAO has no temporal reprojection/history denoising.
- IBL, reflection probes, and runtime lightmap sampling are absent.
- Display settings are runtime-configurable but are not persisted through a settings resource/format; no explicit GPU compatibility profile negotiates a documented minimum feature/limit tier.
- Gamepad device polling exists, but persisted binding serialization and complete per-action pressed/released edge queries remain absent. Native controller hardware has not been verified here.
- ATHAR packages monster and door-close cues, but no current gameplay trigger emits those sounds.

## 2. Work started in this overlay

### Gap 4A — deterministic spotlight selection

**Status:** source change and regression tests added; Rust execution still required on Windows.

- Replaced first-in-query selection with a camera-aware score using range-limited inverse-square falloff, linear luminance, intensity, range, and cone solid-angle coverage.
- Invalid/non-finite/non-contributing records are ignored.
- Equal scores use a lexicographic value tie-breaker; selection no longer depends on ECS extraction order.
- The renderer still supports one spotlight shadow map. This change is a correctness foundation, not a claim of multiple spotlight shadow support.
- Added CPU regression tests for strongest caster selection, extraction-order independence, stable tie-breaking, best unshadowed fallback, and invalid values.

**Windows verification required:**

```powershell
cargo fmt --all -- --check
cargo test --locked -p ribbon_plugin_render3d spotlight_selection -- --nocapture
cargo clippy --locked -p ribbon_plugin_render3d --all-targets -- -D warnings
```

## 3. Remaining implementation milestones

### Gap 10A — vertex data contract (Patch 02 + Patch 03 source complete)

- Added explicit `StaticVertex` (56 bytes) and `SkinnedVertex` (104 bytes) GPU formats; static vertices carry no joint/weight stream and skinned vertices retain all eight influences.
- Kept the public legacy CPU `Vertex`/`MeshData` shape (80 bytes); zero-weight legacy uploads now select the compact static GPU stream, while weighted or malformed inputs retain all skinning fields in the skinned stream. Both paths generate tangents and use UV0 as the UV1 fallback.
- Updated main color and directional/spot shadow shader layouts for both vertex formats.
- Imported glTF `TANGENT`, `TEXCOORD_0`, `TEXCOORD_1`, and the per-texture `texCoord` selector for base color, metallic-roughness, normal, occlusion, and emissive maps.
- Added an embedded GLB fixture with explicit UV1, authored tangent handedness, and a base-color UV1 selector; added fallback tangent/UV tests for the existing cube fixture.
- Missing/invalid tangent frames currently use an area-weighted indexed-triangle/UV0 fallback, not full MikkTSpace. `KHR_texture_transform` is not applied; UV sets above 1 fall back to UV0. Exact baker-compatible mirrored/degenerate seam handling remains a limitation.
- Static/skinned upload byte estimates now account for both GPU layouts and duplicate a primitive only when placements require both formats.
- No representative GPU-memory/time benchmark or interactive UV1 normal-map visual comparison has been run yet.

**Acceptance status:** source structures, import/extraction data, shader layouts, and CPU regression fixtures are present. Rust formatting, package tests, Naga validation, Clippy, and the static/skinned off-frustum caster GPU regressions pass on the RTX 2050/Vulkan host. A dedicated UV1 visual comparison and representative GPU-memory/time benchmark remain outstanding.

### Gap 5A — directional cascades

**Status:** four-cascade source implementation, CPU coverage tests, and targeted RTX 2050/Vulkan GPU checks pass, including static/skinned off-frustum casters, sampled transition blending, sub-texel depth-layer stability, debug captures, and headless rendering. Continuous arbitrary camera-motion review remains separate.

- A four-layer `Depth32Float` texture array stores camera-frustum cascades; practical logarithmic/linear split blending uses lambda 0.65.
- Each cascade fits its own overlapped frustum slice using a rotation-invariant light-space sphere, world-space texel snapping, and a fitted depth range.
- WGSL selects a cascade from positive camera-space depth and blends 3x3 PCF visibility through the final 10% of each non-final split.
- Static and skinned vertex layouts are explicitly selected in color, directional-shadow, and spotlight-shadow passes. Spotlight resources remain independent.
- CPU tests cover monotonic practical splits, each overlapped slice's clip-space coverage, full-frustum coverage, sub-texel stability, invalid-camera fallback, and the numeric 10% transition contract.
- Optional GPU diagnostics expose cascade index, blend weight, individual depth layers, and actual blended visibility. A fixed-direction camera sweep compares center-pixel GPU results against the numeric selection reference and writes a CSV/contact sheets for visual review.
- Four 1024² directional layers plus one 1024² spotlight map are tracked in resource estimates. Shadow distance follows the camera far plane; per-cascade caster culling and configurable resolution/distance remain deferred.

**Acceptance:** all CPU tests and Naga validation pass; the user's Vulkan adapter renders static/skinned casters into all layers with no shader validation errors; dynamic transition captures and numeric readbacks agree at sampled depths. Arbitrary camera rotation, full-scene transition quality, and texel stability require their own acceptance checks.

### Gap 5B/6B — bounded local-light shadows

- Add deterministic multi-spotlight caster ranking and a bounded depth atlas or texture-array path, with per-light matrix/atlas metadata.
- Add opt-in point-light shadows using cubemap/array depth resources and correct world-space visibility sampling; retain unshadowed point lights when capacity is exhausted.
- Add configurable caster capacities, adapter-limit fallback, resource reuse, and diagnostics.
- Test deterministic selection and capacity on CPU; verify occlusion with actual GPU readback and an interactive scene.

**Acceptance:** lights without allocated shadow slots remain illuminated by attenuation only; no unbounded allocations or false claims of point-light occlusion.

### Gap 9A — environment and post quality

- Replace single-pass bloom with bounded downsample/blur/upsample quality tiers and explicit resolution scaling.
- Add temporal GTAO only with history validation, reprojection, disocclusion handling, and camera-cut reset; otherwise retain the current spatial implementation and document the deferral.
- Implement actual IBL: environment cubemap, diffuse irradiance, prefiltered specular mip chain, BRDF integration, intensity and rotation.
- Add a minimal runtime reflection-probe representation and blending only if the renderer can consume it correctly; otherwise explicitly defer it.
- Add runtime lightmap texture/UV-set sampling only after Gap 10A provides UV1 selection. Do not advertise lightmap baking unless a real baker is supplied.

### Gap 11A — display/GPU compatibility

- Add a serialized display preference resource that uses the existing display API; do not introduce a second settings framework.
- Query adapter features and per-format usages/sample counts and define a documented compatibility tier based on actual shader/storage/attachment needs.
- Add fallbacks for optional features rather than blindly requesting default limits as a universal profile.
- Test persisted defaults, invalid settings, unsupported present modes, sample-count fallback, and swapchain/resource recreation.

### Gap 12A — action binding persistence and edge semantics

- Add a versioned serde-compatible binding representation only if it can preserve existing binding variants and stable key names.
- Add per-action just-pressed/just-released queries derived from previous/current evaluated states; do not change existing held/active semantics.
- Add round-trip, migration/default, rebinding, axis sign, trigger hysteresis, and focus-loss tests.
- Keep real controller testing separate from deterministic injection tests.

### Gap 3A — remaining ATHAR sound triggers

- Add door-close audio only when a real closing transition exists; current door behavior is one-way, so do not fake a close event.
- Add monster proximity/movement cues only when monster gameplay state exists; otherwise keep the asset documented as a future cue.
- Recheck duplicate ambience and one-shot event behavior through the mock backend and manually through Kira on Windows.

### Final integration and docs

- Reconcile stale `crates/input/README.md`, renderer capability docs, the upgrade audit, architecture matrix, and release verification document with actual source.
- Run package-level tests, full workspace tests, Clippy, formatting, dependency policy, and packaging smoke tests.
- GPU tests requiring an adapter remain explicitly `#[ignore]` in the normal workspace run and must be invoked separately on supported hardware.
- Record hardware checks separately for RTX 2050/Vulkan, audio device, gamepad, fullscreen transitions, ATHAR package relocation, and STUDS gameplay regression.

## 4. Protected invariants

- No edits to `crates/math` or the existing `ribbon_math` public API.
- No workspace-wide conversion from `Vect`/`Angles` to glam.
- No migration of legacy Scene transforms.
- No changes to STUDS coordinate conventions, procedural generation, or game-owned voxel systems.
- No weakening, deleting, or skipping existing tests to turn a failure green.
- No claim of GPU/audio/controller/visual validation unless that test was actually performed.

## 5. Delivery policy

Each milestone is a separate relative-path patch with its own source changes, tests, and documentation. The supplied archive has no Git metadata, so commits and clean-tree guarantees must be performed in the user's repository after reviewing `git diff`. For each patch, keep a pre-apply copy and run the exact Windows verification commands listed with that milestone.
