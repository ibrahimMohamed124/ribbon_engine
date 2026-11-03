# New systems verification

## Previously recorded baseline verification

The uploaded project verification log records successful `cargo fmt --all`, workspace Clippy with `-D warnings`, and `cargo test --locked --workspace`. It records 28 ATHAR tests passing, 67 render3d tests passing with 13 ignored, and the other listed package suites passing. The earlier particle shadow/color-pass regression ran on an NVIDIA GeForce RTX 2050. The user subsequently confirmed that a fresh `CARGO_TARGET_DIR` build of ATHAR launched successfully and entered the village without the reported WGPU validation error.

The baseline run did not establish physical gamepad behavior, a visual inspection of the rain, a 4x MSAA particle readback, or a complete UI visual/IME session. Its UI notes predate the current follow-up glyph-atlas and interaction work and must not be used as verification of this updated source state.

## Follow-up gap-closure patch status

The following code changes have been made in this candidate source tree: local-space particle acceleration and emitter-state lifecycle fixes; duration/loop boundary and burst handling; recycled particle batch capacity through extraction; additional particle simulation tests (steady-state rate, exact bursts, per-emitter cap, local/world transforms, disabled draining, prewarm, duration boundaries, curve/gradient bounds, and emitter removal); backup-only save recovery, corrupt-primary listing fallback, backup-aware slot existence/deletion, successful migration/missing-section/load-event tests; continuous-endpoint supercover DDA (including corner, boundary-aligned, and endpoint touches); per-pair round-robin sight budgeting with snapshot invalidation; FSM event assertions and additional physics-bake tests; UI vertical/horizontal/cross-axis/panel-fit/scroll layouts, ordering-preserving draw commands, rounded-rectangle SDF, keyboard/gamepad focus repeat, checkbox/slider actions, UTF-8 caret editing, IME composition/candidate placement, R8 glyph rasterization and dirty-region GPU uploads, logical-pixel projection, panel-fit measurement, slider focus outline, showcase persistent settings, and UI regression tests. The render3d patch also supports particle-only scenes and derives billboard axes from camera world rotation; the one required STUDS offscreen-renderer API construction was updated to supply that new camera-rotation field. The platform backend also reports rumble failures once rather than silently discarding them.

Static source review and non-Rust checks are recorded separately below. **These follow-up changes have not yet been compiled or tested in this execution environment.** The container does not contain `cargo`, `rustc`, `rustfmt`, or `rust-analyzer`; `python tools/check_deps.py` was attempted and exited with `dependency check could not read workspace metadata: [Errno 2] No such file or directory: 'cargo'`. TOML parsing, Python syntax compilation, file-preservation comparison, weather-reference audit, and archive integrity checks can be run here, but they do not replace Rust compilation or tests. Do not treat this report as a passed final checklist. No Git commits were created because the supplied workspace has no `.git` directory.

| Phase | Follow-up checks still required on a Rust-enabled host | Not yet verified for this patch |
| --- | --- | --- |
| 1. App states | `cargo clippy -p ribbon_app --all-targets -- -D warnings`; `cargo test -p ribbon_app`; dependency check | The existing state implementation is retained; no new state-machine behavior was changed in this patch. |
| 2. Gamepad | Clippy/tests for `ribbon_input` and `ribbon_platform` with default and `--no-default-features`; dependency check | No physical gamepad, hot-plug, or rumble device check. |
| 3. Particles | `cargo clippy -p ribbon_plugin_particles -p ribbon_render_contracts -p ribbon_plugin_render3d --all-targets -- -D warnings`; `cargo test -p ribbon_plugin_particles -p ribbon_render_contracts -p ribbon_plugin_render3d`; `cargo build -p athar`; run the ignored particle/shadow regression and `multisampled_hdr_resolves_then_fxaa_tonemaps_on_gpu`; dependency check | Actual MSAA 4x particle readback and rain appearance on the target Windows adapter. |
| 4. Save/load | `cargo clippy -p ribbon_save -p athar --all-targets -- -D warnings`; `cargo test -p ribbon_save -p athar`; dependency check | Real power-loss durability and platform-specific rename semantics beyond filesystem tests. |
| 5. AI | `cargo clippy -p ribbon_plugin_ai --all-targets -- -D warnings`; `cargo test -p ribbon_plugin_ai`; dependency check | Large-world performance and game-specific perception/navigation tuning. |
| 6. UI | `cargo clippy -p ribbon_ui -p ui_showcase --all-targets -- -D warnings`; `cargo test -p ribbon_ui`; run the ignored `ui_pass_renders_a_panel_and_text_into_an_offscreen_target`; `cargo build -p ui_showcase`; `cargo tree -d` | Native-window visual layout, actual gamepad device navigation, IME candidate placement on Windows, and offscreen glyph GPU readback. |

### Latest Windows host test feedback (2026-11-03)

The user ran the phase commands from the extracted gap-closure candidate on Windows. The confirmed results were:

- Phase 1: `cargo clippy -p ribbon_app --all-targets -- -D warnings` passed; all 11 `ribbon_app` tests passed.
- Phase 2: all 13 `ribbon_input` and 10 `ribbon_platform` tests passed with default features and again with `--no-default-features`. Clippy found a test-only `field_reassign_with_default` warning in the invalid-trigger-settings fixture; the fixture now uses a struct initializer.
- Phase 3: all 18 particle tests, the render3d suite (70 passed, 14 ignored), and the render-contract layout test passed; `cargo build -p athar` passed. Clippy found redundant parentheses in one renderer test assertion; those parentheses have been removed. The 14 ignored GPU tests were not run by the normal test command.
- Phase 4: compilation was blocked by a missing test import for crate-private `SaveRegistry` and two unnecessary `mut` bindings. The test import and the two read-only fixture bindings have been corrected in this follow-up source update. Save/load tests and Clippy must be rerun.
- Phase 5: compilation was blocked by a mutable-borrow overlap when inserting a missing `Perceived` component in the no-target path. The borrow is now ended before insertion; AI tests and Clippy must be rerun.
- Phase 6: compilation was blocked by importing `TextDraw` from the crate root instead of `crate::draw`, plus an unused `PackedGlyph` re-export. Both imports have been corrected; UI tests, Clippy, and the showcase build must be rerun.
- `cargo tree -d` showed one `wgpu` version (`25.0.2`). Other duplicated crates in that output do not violate the UI acceptance criterion.
- `python tools/check_deps.py` was not run because the Windows shell could not resolve the `python` command. Try `py -3 tools/check_deps.py` (or `python3 tools/check_deps.py`) on that host.

These are targeted fixes for the exact compiler diagnostics in the supplied log, not a claim that the rerun has passed. The full workspace gate remains open until the corrected Phase 4–6 commands and final workspace commands complete successfully.

## Required final workspace gate

The follow-up candidate must still pass all of these on the target host before it can be called complete. Also run Phase 2 both with default features and with `--no-default-features`, and run each ignored GPU test explicitly on a compatible adapter:

```powershell
cargo fmt --all
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo build --locked --workspace --all-targets
py -3 tools/check_deps.py  # or python3 tools/check_deps.py
cargo tree -d
```

The weather-reference grep must return hits only under `docs/history/**` and the two superseded weather ADR notes. The ignored GPU tests and manual examples must be exercised separately where hardware is available.

### Latest Windows host test feedback — follow-up 2 (2026-11-03)

The next Windows run confirms Phase 4 is now green: `cargo clippy --locked -p ribbon_save -p athar --all-targets -- -D warnings` passed, along with all 13 `ribbon_save` tests and all 28 ATHAR tests. Phase 5's 18 AI tests also passed; Clippy reported one `extend_with_drain` lint for the pending visibility vector. The implementation now uses `Vec::append` for that transfer and must be rerun through Clippy and tests.

Phase 6 exposed one architectural compile error and five additional Clippy findings. The compile error came from constructing `UiContext` with a struct-update expression in `plugin.rs` even though the type owns private internal state. A `UiContext::with_theme` constructor now initializes those internals inside their defining module, and `UiPlugin` uses that constructor. A regression test covers custom-theme preservation and the default frame state. The other UI lint corrections use `try_fold` for optional clip intersections, restrict the test-only atlas packing helper to test builds, use `is_multiple_of` for cache maintenance intervals, and use `as_chunks::<4>()` for RGBA alpha extraction. These changes preserve the old clipping, atlas packing, cache interval, and alpha-channel behavior.

The current container still has no Rust toolchain, so the new Phase 5/6 corrections have not been compiled here. Rerun the Phase 5 and Phase 6 commands plus the final workspace gate below on Windows; do not treat this source archive as a passed final checklist until those commands succeed. Phase 4 is confirmed passed by the latest user-provided Windows output.


### Latest Windows host test feedback — follow-up 3 (2026-11-03)

The latest Windows run confirms Phase 5 is green: `cargo clippy --locked -p ribbon_plugin_ai --all-targets -- -D warnings` passed and all 18 AI tests passed. Phase 6 Clippy and `cargo build --locked -p ui_showcase` passed, but `cargo test --locked -p ribbon_ui` exposed three behavioral-test failures: directional gamepad repeat did not wrap at the edge of a focus row; end-aligned content could exceed the right edge by a floating-point rounding unit; and the draw-order test incorrectly expected a plain rectangle to have the rounded rectangle's radius. The UI suite had 38 passing, 3 failing, and 1 GPU test ignored.

The workspace `cargo clippy --locked --workspace --all-targets -- -D warnings` reached the particle test suite and found one `field_reassign_with_default` lint in the non-finite-emitter fixture. It is now initialized with a struct update expression. The full workspace test run otherwise compiled all packages and reported all displayed suites passing except for the same three `ribbon_ui` failures; the 14 ignored render3d GPU tests and the ignored UI GPU test remain unexecuted. `cargo build --locked --workspace --all-targets` passed. `py -3 tools/check_deps.py` passed for 44 workspace packages. `cargo tree -d` was started, but the supplied output ends before any result, so its outcome is not recorded.

The source update after this feedback makes directional focus wrap to the opposite edge only when no candidate exists in the requested direction, computes end-aligned width from the exact right edge used for placement, corrects the shape-radius test to distinguish normal and rounded vertices, and fixes the particle fixture lint. These edits still require a fresh Windows format/Clippy/test/build run; this environment has no Rust toolchain and cannot certify the changes.
