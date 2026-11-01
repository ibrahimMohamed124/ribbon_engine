# Ribbon Engine release-candidate checklist

This checklist is a validation gate, not a declaration that the current snapshot is release-ready. Mark an item complete only after capturing its actual output from the target Windows environment.

## Architecture and packaging

- [ ] No voxel references remain in `crates/` or `plugins/` source, manifests, shaders, docs, or scripts.
- [ ] `ribbon_plugin_render3d` and `ribbon_plugin_physics` dependency trees contain no STUDS/voxel package.
- [ ] `ribbon_input` and `ribbon_profiling` remain leaf crates with no workspace, `wgpu`, or `winit` dependencies.
- [ ] Public re-exports still preserve existing crate paths after module splits.
- [ ] No engine source file exceeds roughly 600 non-test lines without an ADR explaining why.
- [ ] `games/athar` and `games/studs` remain game-owned; no engine crate points back to either game.
- [ ] Directory renames under `crates/` change paths only; package names and dependency edges remain unchanged.
- [ ] The dependency-policy checker passes and the allowlist contains no wildcard allowances.

## Code correctness

- [ ] `cargo fmt --all -- --check` passes.
- [ ] Locked workspace check, Clippy with warnings denied, and example build pass.
- [ ] Default-feature and no-default-feature workspace tests pass.
- [ ] STUDS golden tests remain bit-identical to the baseline.
- [ ] Prefab RON round-trip restores `Transform3D`, `ModelInstance` by asset path, and a light component.
- [ ] A missing prefab asset returns an error and rolls back all entities spawned during that instantiation.
- [ ] Input unit coverage confirms key/mouse edge states, focus-loss release, binding-source resolution, and buffered-frame accumulation.
- [ ] Profile output-format tests assert an exact stable sample line.

## Platform and visual acceptance

- [ ] `cargo run -p athar` opens the native window on Windows.
- [ ] Mouse movement to the right rotates the camera to the right; WASD moves relative to camera heading.
- [ ] Cursor release/capture and focus loss do not leave stuck keys or buttons.
- [ ] Floor/wall/door/key/save-point/creature assets remain visible under the actual GPU renderer.
- [ ] Shadows, material textures, fog, tone mapping, and normal/emissive maps are inspected in a saved screenshot.
- [ ] GPU shutdown completes without an access violation or leaked renderer-owned resources.
- [ ] Old captures are preserved and any new benchmark compares the same machine, adapter, resolution, workload and build settings.

## Documentation

- [ ] Relative Markdown links resolve after the final moves.
- [ ] Architecture, performance, release, history, and decisions docs remain in their designated folders.
- [ ] Game-specific tools and commands are referenced using `games/athar/...` paths.
- [ ] Remaining limitations and all deviations from the task are recorded without claiming unperformed validation.
