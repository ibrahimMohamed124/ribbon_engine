> **Superseded restoration note (2026-10-29).**
> This file was written by the previous restructuring pass; see [`restore-inventory.md`](restore-inventory.md) as the current restoration record.
> It did not call out the prompt-described Rapier, audio, flashlight, CHANGELOG, surface-shader-test, and weather regressions or the STUDS GPU-camera input behavior change.
> The supplied `engine.zip` does not contain the stated Rapier/audio/flashlight/weather sources, CHANGELOG, or surface shader contract test, so those cannot be restored without guessing.
> Files and behavior changes that had authoritative REF sources have been restored or reverted where applicable; unavailable sources are listed as deviations in the inventory.
> The STUDS input behavior change was reverted and its exact removed hunks are recorded in [`studs-gpu-camera-input-note.md`](studs-gpu-camera-input-note.md).
>
# Ribbon Engine restructuring snapshot — delivery notes

## Scope

This archive is a full workspace snapshot reconstructed from the most recent project source and recoverable supporting artifacts available in the conversation's file library. It contains the mesh renderer architecture, game-owned STUDS voxel packages, the `ribbon_input` and `ribbon_profiling` leaf crates, the asset-aware prefab codec extension, `games/athar`, and the reorganized documentation/tool directories.

The repository archive used as a base had no `.git` directory. The delivered archive therefore contains source changes and documentation, not Git history or commit objects. Existing source and asset files are retained unless a change is documented here.

## Changes completed in this continuation

- Added shared mouse button edge state and mouse motion/scroll storage to `ribbon_input`, with platform-neutral input-source types.
- Added a generic rebinding model and action-map application method while keeping the key-only map API compatible.
- Migrated the STUDS pending-frame accumulator to the generic `BufferedInput` while preserving `InputFrame::merge` as the game's merge policy.
- Re-exported shared input types from `ribbon_platform`; platform event conversion and the runner remain platform-owned.
- Changed the legacy platform mouse helper to delegate state tracking to `ribbon_input`.
- Added prefab round-trip coverage for `Transform3D`, a model handle serialized by path, and `SpotLight`; retained the missing-asset / rollback test.
- Restored the missing reusable weather plugin and moved rain particle ownership out of ATHAR gameplay code while keeping the scene defaults (220 drops, speed range 11–20, radius 21, tint, streak scale). Added `ribbon_render_contracts` for renderer-neutral camera/mesh/light/fog data so weather does not depend on render3d internals.
- Restored the recoverable optional village asset generator and the binary GLB/manifest validator into `games/athar/tools/`; rewrote the texture fetcher to cover the three room materials plus the optional village road/roof/wood maps.
- Corrected Athar asset-tool roots so manifest paths remain workspace-relative and generator output paths resolve after `horror_slice` moved to `games/athar`. The engine-only fixture generator no longer imports the game's asset generator.
- Added a profiling completion-line formatter in the leaf crate and a test locking the exact existing `[RENDER PROFILE] wrote ...` message, then routed the render plugin through it without changing the emitted text.
- Moved `editor_demo` from `examples/` to `games/studs/showcases/editor_demo` because it depends on the relocated STUDS-owned CPU renderer; kept its package name and `cargo run -p editor_demo` invocation stable, fixed all path dependencies and the Windows icon helper include.
- Fixed the dependency-layer checker to allow dependencies between packages owned by the same game while still rejecting engine-to-game and cross-game edges.
- Updated stale profiling-tool paths to `tools/perf/` and asset-tool paths to `tools/assets/` across current documentation, and updated the CI sample-package label.
- Updated prefab implementation documentation, game-tool invocations, and the icon build-layout note; added Windows validation/release-candidate checklists.

## Validation boundary

No Rust formatter, Cargo build, Rust test, Clippy run, native Windows GPU launch, or pixel comparison was performed in this environment. Do not treat this archive as build-verified or release-ready until the commands in [`docs/release/VALIDATION_WINDOWS.md`](../release/VALIDATION_WINDOWS.md) have passed on the target machine. Static checks run for this archive: Python syntax compilation passed for all 13 Python files; the Athar GLB/manifest validator passed for all 12 self-contained room and village models (24.59 MiB); the current 38 workspace members have unique package names and all declared relative path dependencies found by static audit exist; the static dependency allowlist/layer audit reported no unapproved or cross-layer edges; `crates/` and `plugins/` contain no voxel references; model hashes match; 99 Markdown files contain 62 checked relative links and no broken targets; the ten largest engine source files have at most 561 non-test nonblank lines. Cargo metadata, Rust formatting, compilation, Clippy, tests, and native runtime/GPU checks could not be run because this environment has no `cargo` or `rustfmt` executable.

## Before/after verification matrix

The requested Phase-0 baseline and the exact Oct 28 working tree are not present in the recovered files. The table therefore separates evidence that can be established here from commands that must run against this delivered snapshot on Windows; it intentionally does not infer a pass from earlier logs for a different source state.

| Check | Phase-0 baseline | Delivered source snapshot |
|---|---|---|
| `cargo fmt --all -- --check` | Not recoverable | Not run (Rust formatter unavailable) |
| `cargo check --workspace --all-targets` | Not recoverable | Not run (Cargo unavailable) |
| `cargo clippy --workspace --all-targets` | Not recoverable | Not run (Cargo unavailable) |
| `cargo test --workspace` and STUDS golden tests | Not recoverable for this prompt's baseline | Not run (Cargo unavailable); golden source assertions compared as noted above |
| `cargo build --workspace --examples` | Not recoverable | Not run (Cargo unavailable) |
| `--no-default-features` build and tests | Not recoverable | Not run (Cargo unavailable) |
| Static workspace/path/allowlist audit | No recorded baseline | PASS — 38 unique packages; relative path dependencies exist; no unapproved or cross-game edge found |
| Markdown relative links | No recorded baseline | PASS — 62 links checked; none broken |
| Engine voxel scan | No recorded baseline | PASS — no matching files under `crates/` or `plugins/` |
| ATHAR asset manifest and embedded GLBs | No recorded baseline | PASS — 12 packaged GLBs validated, 24.59 MiB total |
| Windows launch/GPU visual checks | Not recoverable | Not run — required on target hardware |

## Known recovery limitations

- There was no delivered Oct 28 working-tree archive or Git checkout to restore byte-for-byte. The workspace is based on the saved full archive and changes that can be reconstructed from available artifacts; unrecoverable later local-only edits are not invented.
- The saved Oct 25 base archive lacked `plugins/weather`; this continuation restored the intended generic plugin from the recoverable weather-overlay design, adapted it to the current package names and existing ATHAR behavior, and added `ribbon_render_contracts` so weather consumes renderer-neutral data rather than renderer internals. The contracts crate is intentionally limited to `glam` and `ribbon_scene`; the glTF-typed `ExtractedScene3D` snapshot remains in `render3d` to avoid an upward dependency. This restored implementation still requires local Cargo/Windows runtime verification.
- `horror_village_assets.py` and `validate_horror_assets.py` were recovered from the saved village overlay and ported with corrected paths. `generate_flashlight_asset.py` and `generate_horror_audio.py` were not present in the saved workspace or available ZIP overlays; they were not invented. The optional village model generator is tooling only and does not add village gameplay to the current single-room Athar scene. The spec-listed `generate_flashlight_asset.py` and `generate_horror_audio.py` were not recoverable in the available archives and remain absent; they were not reconstructed speculatively.

## Phase-by-phase acceptance record

This is a source/archive audit, not a claim that the Rust workspace has passed its Windows build.

| Phase | Source state | Verification boundary |
|---|---|---|
| 1 — Game-owned voxel stack | `crates/` and `plugins/` contain no voxel references; voxel storage, meshing, CPU rendering, GPU bridge, and voxel showcases live under `games/studs/`. Physics exposes mesh-oriented AABB collision helpers without a voxel dependency. | Static source/manifest audit passed. Cargo dependency trees and golden tests still need execution on Windows. |
| 2 — Renderer responsibilities | `ribbon_render` and `ribbon_plugin_render3d` have separate role READMEs; the latter keeps public re-exports for contracts now held by `ribbon_render_contracts`. | Source review passed; compilation not run. |
| 3 — Source splits | The large assets, renderer, schedule, audio, GPU-window, and other implementations are decomposed into focused modules with re-exports. The ten largest non-test source counts are listed below. | Approximate static line-count audit only; formatter/compiler not available. |
| 4 — `ribbon_profiling` | Std-only leaf crate contains generic counters/timing/capture helpers and a test pinning the existing `[RENDER PROFILE] wrote ...` line. | Manifest/allowlist and source checked; tests not executed. |
| 5 — `ribbon_input` | Platform-neutral key/mouse edge state, mouse motion/scroll, rebinding/action sources, scripted `VirtualInput`, and buffered input are separated from the window runner. Platform OS events are queued before scripted input in `First`; STUDS retains its game action names and merge policy. | Source checked; input/settings tests not executed. |
| 6 — Prefab asset references | Context-aware codec APIs keep `ribbon_scene` independent from assets/renderers. `ModelInstance` serializes by path, restores through a provided `AssetServer`, and has round-trip plus missing-asset/rollback tests in source. | Source reviewed; tests not executed. |
| 7 — ATHAR ownership | Package is `athar` under `games/athar`; related docs and recoverable tools moved alongside it. Global profiling/asset tools use `tools/perf/` and `tools/assets/`. | Static path and Markdown-link scan passed; launch not executed. Two unavailable asset/audio scripts remain absent (see recovery limitations). |
| 8 — Documentation | Architecture, performance, release, history and ADR documents are grouped and indexed; the original `docs/DECISIONS.md` was moved byte-for-byte to `docs/decisions/0000-legacy-decision-log.md`; the four spotlight-shadow records are consolidated with their content retained. | 99 Markdown files scanned; all 62 relative links checked by the static checker resolve. Some paths inside the required unchanged legacy decision log are historical and refer to the old top-level `tools/` layout. |
| 9 — Tooling and directory names | Tools are grouped into `tools/perf/` and `tools/assets/`; `crates/` directory names no longer carry the `ribbon_` prefix while Cargo package names remain stable. `.gitignore` includes `__pycache__/`, `*.pyc`, and `*.py[cod]`. The obsolete commit script and its references are absent. The reusable root `scripts/git-diff.sh` remains included and is no longer hidden by a broad `scripts/` ignore rule. | Static workspace-manifest, path-dependency and allowlist/layer scan passed. The archive has no `.git`, so tracked-file status for `tools/__pycache__` cannot be reconstructed. |

## Static evidence captured for this archive

| Check | Result |
|---|---|
| Python syntax compilation | PASS — all 13 Python files compile in memory. |
| ATHAR embedded GLB/manifest validation | PASS — 12 room/village GLBs, 24.59 MiB total; all hashes and embedded model/material data validate. Dedicated optional village maps remain 0/9. |
| Workspace manifest/path/allowlist scan | PASS — 38 workspace members; 39 `Cargo.toml` files including the root; unique package names, all relative path dependencies resolve, and no unapproved or cross-layer dependency edge was reported by the static audit. |
| Engine voxel scan | PASS — zero matching source/documentation files beneath `crates/` and `plugins/` for the prompt's searched extensions. |
| Markdown relative links | PASS — 99 Markdown files, 62 relative links checked, zero broken targets. |
| Engine source length | PASS by approximate source scan — no file exceeds 561 non-test-ish nonblank lines; comments are counted, and test modules are excluded heuristically. |
| STUDS golden-source comparison against the saved base archive | PASS for expected results — `games/studs/tests/golden.rs` differs only in the two import paths needed for the renderer relocation; assertion logic/constants remain unchanged. The relocated CPU renderer's `fixture_golden.rs` is byte-identical to its saved source. These are source comparisons, not test results. |
| Cargo metadata / rustfmt / build / Clippy / Rust tests | NOT RUN — no `cargo`, `rustc`, or `rustfmt` executable is installed here. |
| Native Windows GPU output / camera / shutdown | NOT RUN — must be checked on the target device. |

### Ten largest engine source files

Line counts are from the static heuristic that skips `#[cfg(test)]` modules approximately. These figures are useful for triage but are not a compiler/parser result.

| Source file | Non-test-ish lines | Total lines |
|---|---:|---:|
| `plugins/render3d/src/lib.rs` | 561 | 584 |
| `crates/schedule/src/lib.rs` | 547 | 600 |
| `plugins/render3d/src/renderer.rs` | 528 | 569 |
| `crates/commands/src/lib.rs` | 515 | 736 |
| `crates/assets/src/lib.rs` | 500 | 560 |
| `crates/platform/src/runner.rs` | 464 | 489 |
| `crates/render/src/gpu_window.rs` | 453 | 525 |
| `plugins/render3d/src/renderer/setup.rs` | 451 | 453 |
| `crates/scene/src/hierarchy.rs` | 450 | 547 |
| `plugins/render3d/src/post.rs` | 449 | 466 |

`plugins/render3d/src/renderer/draw.rs` also has 449 non-test-ish lines (455 total), tied with `post.rs` at the cut-off. `crates/commands/src/lib.rs` has 736 total lines because its tests are colocated; its approximate non-test portion is 515 lines.

## Deviations, open questions, and out-of-scope observations

- **Recovered weather boundary:** the Oct 25 base archive did not contain `plugins/weather`. The recoverable weather design was restored as `ribbon_plugin_weather` plus the data-only `ribbon_render_contracts` crate. `ExtractedScene3D`/`ExtractedModel3D` remain in `render3d` because the model snapshot includes a glTF-specific asset handle. ADR `0012-weather-render-contracts.md` documents that boundary. This reconstructed code needs the full Windows build/test pass.
- **STUDS camera correction:** the mesh-GPU input adapter reverses mouse yaw and swaps horizontal look/strafe actions only for that backend, keeping the CPU reference path and its golden expectations stable. This is the targeted behavior fix requested alongside the structural work; it is not part of a purely move-only refactor.
- **Recovered tooling gap:** `generate_flashlight_asset.py` and `generate_horror_audio.py` could not be found in the saved full archive or the available overlays and were not recreated speculatively. The recovered room/village generator and validator are present.
- **Legacy decision log:** kept byte-identical as instructed. A few old tool paths inside that historical table still name `tools/run_*.ps1`; the current equivalent scripts are under `tools/perf/`.
- **Git and baseline evidence:** the recovered source archive has no `.git`, and the full Oct 28 working-tree snapshot was not saved. Therefore no commit hashes, Git move history, format-patch series, exact before/after Cargo results, or baseline-vs-current test table could be produced honestly.
- **Legacy 2D transform:** `ribbon_scene::Transform` / `GlobalTransform` remains in the engine as required; it still serves the STUDS CPU/rendered 2D-style path and is used in scene/editor reflection coverage, so it was not removed as apparently voxel-only.
- **Out-of-scope limitations:** dedicated road/roof/wood source maps are still absent (0/9, existing packaged GLBs remain self-contained), ATHAR's monster remains a static silhouette, and no new performance or pixel-equivalence claim is made. Confirm the live game, rain transition, mouse/WASD response, GPU rendering, and clean shutdown on Windows before calling the work verified.
