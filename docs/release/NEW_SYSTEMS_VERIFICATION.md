# New systems verification

Verification performed for the six system phases. GPU rendering, visual appearance, and physical devices were not available for manual acceptance.

| Phase | Commands / checks | Result | Not verified |
| --- | --- | --- | --- |
| 1. App states | `cargo clippy -p ribbon_app --all-targets -- -D warnings`; `cargo test -p ribbon_app`; `py -3 tools/check_deps.py` | Passed; transition ordering, run conditions, deferred hook requests, and initial hooks covered. | — |
| 2. Gamepad | Clippy/tests for `ribbon_input` and `ribbon_platform` with default and `--no-default-features`; dependency check | Passed on both feature configurations. | No physical gamepad, hot-plug, or rumble device check. |
| 3. Particles | `cargo test -p ribbon_plugin_particles -p ribbon_render_contracts -p ribbon_plugin_render3d`; `cargo build -p athar`; dependency check | Passed; WGSL validation ran. GPU tests are ignored. | GPU pixels/MSAA visual output and ATHAR rain appearance. |
| 4. Save/load | `cargo clippy -p ribbon_save -p athar --all-targets -- -D warnings`; `cargo test -p ribbon_save -p athar`; dependency check | Passed, including backup recovery and all-or-nothing section preparation. | Crash/power-loss behavior outside the exercised filesystem tests. |
| 5. AI | `cargo clippy -p ribbon_plugin_ai --all-targets -- -D warnings`; `cargo test -p ribbon_plugin_ai`; dependency check | Passed; grid paths, baking, steering, behavior trees, FSM, and sight tests ran. | Large-world performance and gameplay tuning. |
| 6. UI | `cargo clippy -p ribbon_ui -p ui_showcase --all-targets -- -D warnings`; `cargo test -p ribbon_ui`; `cargo build -p ui_showcase`; `cargo tree -d`; dependency check | Passed; bundled font shaping and WGSL validation ran; `wgpu` resolved as 25.0.2. | UI glyphs are not yet rendered by the GPU pass; no native-window/GPU visual check, OS IME caret check, or physical gamepad navigation check. |

Final workspace commands: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo build --workspace --all-targets`, and `py -3 tools/check_deps.py` completed successfully after the minimal `Scene.particles` fixture update in STUDS required by the Phase 3 API addition.
