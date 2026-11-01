# Windows validation follow-up — 2026-11-01

## Reported validation state

The user's second Windows run confirms the Render3D duplicate-import fix:

- `cargo fmt --all -- --check`: PASS
- `cargo check -p ribbon_plugin_streaming --all-targets`: PASS
- `cargo test -p ribbon_plugin_render3d --all-targets`: 40/40 PASS
- AssetServer: 22/22 PASS
- Scene/Prefab: 9/9 PASS
- Render profiling: 12/12 PASS
- glTF: 6/6 PASS
- Physics/Prefab: 24/24 PASS

The same run reported:

1. `plugins/editor_tools/src/lib.rs`: `ui.label(String)` did not match the UI API's `&str` argument.
2. `coordinator::tests::lifecycle_timing_records_each_stage_and_request_to_active_latency`: aggregate `snapshot().diagnostics.max_request_to_active_ns` remained zero until the next coordinator tick, although the per-region timing was already populated.
3. `coordinator::tests::dormant_reactivation_starts_a_new_activation_timing_attempt`: the test expected zero `physics_preparation_ns`, despite the second activation explicitly transitioning through `PreparingPhysics`.

## Changes in this hotfix

- `set_state` now immediately publishes a newly completed lifetime max latency and corresponding slowest region to aggregate diagnostics. Normal `refresh_diagnostics` continues to recompute the aggregate over all regions during a tick.
- The dormant reactivation test now uses explicit elapsed intervals and requires the reactivation attempt to record physics preparation, while confirming that CPU loading and GPU warmup remain zero because resident assets are reused.
- The editor label now passes `&format!(...)` to the UI API.

## Not yet verified

The hotfix still requires the user's Windows run. In particular, workspace check, the 48-test streaming suite, editor-tools tests, ATHAR tests, workspace tests, Clippy, and the complete validation script must be rerun. No GPU hardware/device-loss recovery claim is established by these unit-test changes.
