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
cargo test -p plugin_render3d spotlight_only_corridor_writes_a_headless_png -- --nocapture
```

If the focused GPU test passes, then run both workspace test configurations and
`python tools/check_deps.py`. If it still reports zero changed pixels, keep the assertion intact and
share the new `target/spotlight_corridor.png` plus the complete test output for the next diagnosis.
