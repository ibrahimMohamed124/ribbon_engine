# Spotlight shadow fixes — consolidated history

---

## Original record: SPOTLIGHT_SHADOW_BIAS_FIX.md

# Spotlight shadow depth-bias isolation

## Symptom

The `spotlight_only_corridor_writes_a_headless_png` GPU regression still reports
`changed pixels=0, difference energy=0` with the normal (non-diagnostic) mesh shader.
The game can render its room and flashlight while this separate shadow regression fails.

## Change

`MeshRenderer` now owns separate depth-only pipelines for directional and spotlight shadow maps.
The directional map retains its existing caster bias (`constant = 2`, `slope_scale = 2.0`).
The perspective spotlight pipeline uses `DepthBiasState::default()`; spotlight PCF already applies
its own small receiver-side comparison bias in `mesh.wgsl`.

The spotlight render pass uses the new spotlight pipeline. This avoids applying the directional
pipeline's slope-scaled caster bias to perspective depth, without changing the directional shadow
pipeline or weakening the regression assertion.

## Verification status

This is a targeted source-level fix hypothesis, not a confirmed runtime fix. The project and GPU
tests were not run while preparing this patch. From the workspace root on Windows, run:

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p ribbon_plugin_render3d spotlight_only_corridor_writes_a_headless_png -- --nocapture
```

If the focused GPU test passes, then run both workspace test configurations and
`python tools/check_deps.py`. If it still reports zero changed pixels, keep the assertion intact and
share the new `target/spotlight_corridor.png` plus the complete test output for the next diagnosis.


---

## Original record: SPOTLIGHT_SHADOW_COORDINATES_FIX.md

# Shadow coordinate interpolation fix

## Symptom

On the Windows GPU, `spotlight_only_corridor_writes_a_headless_png` rendered the corridor but reported
`changed pixels=0, difference energy=0` when comparing the exact same frame with the spotlight shadow
map enabled and disabled. The PNG was saved before the failing assertion so the frame remains inspectable.

## Root cause in the shader path

The mesh vertex shader emitted directional and spotlight **light clip-space coordinates** as ordinary
varyings. Rasterization perspective-corrects ordinary varyings using the *camera's* clip-space `w`,
which is not the projection used by the light. The resulting interpolated coordinates can be wrong
for shadow-map lookup, even when the light projection matrix and shadow render pass are present.

## Change

The fragment shader now computes the directional and spotlight shadow coordinates from the
perspective-correct interpolated `world_position` using the matching light view-projection uniform.
This keeps projection consistent with the fragment's actual surface position. The shadow comparison,
PCF kernel, depth bias, and regression assertions have not been weakened.

## Verification status

The source change is made, but this document does not claim the regression test passes yet. On Windows,
run:

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p ribbon_plugin_render3d spotlight_only_corridor_writes_a_headless_png -- --nocapture
```

The test is expected to save `target/spotlight_corridor.png` before checking the shadow difference. If
it still fails, inspect that PNG and the reported pixel-difference measurements before making further
changes. Then run both full workspace test configurations.


---

## Original record: SPOTLIGHT_SHADOW_PIPELINE_FIX.md

# Spotlight Shadow Pipeline Repair

## Why this change

The renderer previously reused the directional shadow pipeline for spotlight depth rendering.
That pipeline culls back faces and carries a bias intended for directional shadowing. A
spotlight placed inside a closed room needs the inward-facing surfaces of the room to be
able to populate its perspective depth map; it also benefits from an independent depth-bias
configuration.

## Changes

- Keep the existing directional shadow pipeline and its state unchanged.
- Add a dedicated spotlight shadow pipeline with back-face culling disabled and the default
  depth bias, then use it only for the spotlight shadow pass.
- Adjust the existing headless corridor fixture's light placement and intensity so the
  floor region behind the blocker produces a measurable signal in its `Rgba8Unorm` target.
- Preserve the existing assertions and their thresholds. No new tests are added.

## Verification

This archive was checked for source consistency and ZIP integrity only. Rust formatting,
Clippy, WGSL validation, and the GPU regression test have not been run in the artifact-building
environment. Validate on the development machine:

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p ribbon_plugin_render3d spotlight_only_corridor_writes_a_headless_png -- --nocapture
```

If the GPU test still reports zero changed pixels, inspect the newly generated
`target/spotlight_corridor.png` and attach it with the complete test output. Do not lower or
remove the regression assertion.


---

## Original record: SPOTLIGHT_SHADOW_REGRESSION_FIX_V4.md

# Spotlight shadow regression fixture adjustment (v4)

## Observed result

After forcing a clean rebuild, the spotlight-only corridor regression reported 12 changed pixels and difference energy 751. This establishes that the shadow path is active, but the current scene does not produce a large enough visible shadow for the existing assertion thresholds.

## Change

The regression fixture's spotlight direction now aims through the off-center blocker (`(-0.31, -0.32, -0.91)`) rather than down the corridor centerline. The blocker is deliberately off the camera centerline so the resulting floor shadow remains visible to the camera. Existing assertion thresholds and non-test engine behavior are unchanged.

## Verification status

This source package has not been compiled or GPU-tested in the packaging environment. Run the focused test on a machine with a usable wgpu adapter. If it still fails, inspect the newly generated `target/spotlight_corridor.png`; do not lower the assertion thresholds.
