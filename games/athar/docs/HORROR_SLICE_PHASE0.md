# Horror Slice — Phase 0 report

Date: 2026-10-20

> Historical snapshot of the initial Phase 0 proposal and checks. The room, renderer, assets,
> and collision implementation have moved on since this report. Current workspace validation
> was reported by the user as passing Cargo tests and Clippy without errors or warnings; use the
> current CI workflow for the authoritative GitHub matrix. The original Rapier proposal below
> was superseded by the axis-separated static AABB capsule helper documented in
> `docs/decisions/0000-legacy-decision-log.md`; Rapier is not currently a dependency.

## Scope

Phase 0 adds the `athar` example package and pins the architecture decision
for general room collision. The starter scene intentionally contains only a
camera and a placeholder floor and uses the native presentation path:
`PlatformPlugin` -> `ScenePlugin` -> `GpuWindowPlugin` -> `Render3dPlugin`.
It does not use STUDS or `OffscreenVoxelRenderer`.

## Baseline command matrix

The full requested matrix was attempted from the workspace root both before and
after the Phase 0 changes. This execution environment has no `cargo`, `rustc`, or
`rustfmt` binary, and outbound DNS/network access is unavailable, so none of the
Rust commands could actually run in either pass. These are blocked checks, **not
passing results**.

| Command | Result | Notes |
| --- | --- | --- |
| `cargo fmt --check` | Blocked, exit 127 | `cargo: command not found` |
| `cargo clippy --workspace --all-targets -- -D warnings` | Blocked, exit 127 | `cargo: command not found` |
| `cargo test --workspace` | Blocked, exit 127 | `cargo: command not found` |
| `cargo test --workspace --no-default-features` | Blocked, exit 127 | `cargo: command not found` |
| `python tools/check_deps.py` | Blocked, exit 2 | Before and after the change, the checker could not invoke `cargo metadata` because `cargo` is missing |

The original overlay did not contain `.git` metadata, so the proposed phase commit
could not be created from that overlay. That limitation was specific to the overlay
workflow and does not apply to this repository checkout.

## User-reported Windows follow-up (before the later visual/rendering pass)

The user subsequently ran the original Phase 0 checkout from PowerShell. After running `cargo fmt`, `cargo fmt --check` passed; `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace -j 1`, `cargo test --workspace --no-default-features -j 1`, and `python tools/check_deps.py` passed. The no-default-features test run emitted two `dead_code` warnings in STUDS (`backend` and `fall_back_to_cpu`). `cargo run -p athar` returned to the prompt without a visible error, but the available log did not establish whether the window was opened and closed or failed to present. Those results apply to the original Phase 0 pass; the later lighting/material pass must run the check matrix again.

## Decision

The initial proposal was to use `rapier3d` for a kinematic controller and collision/ray
queries. That proposal was superseded: the current room only needs static AABB collision,
so `ribbon_plugin_physics` provides an axis-separated capsule slide helper without adding Rapier.
See the current decision in `docs/decisions/0000-legacy-decision-log.md`.

## Changed files

- `Cargo.toml`: adds `games/athar` to the workspace.
- `games/athar/Cargo.toml`: declares only the engine and native renderer dependencies required by the starter.
- `games/athar/src/main.rs`: creates a native GPU window, camera, and temporary floor mesh; supports `--frames N` for a bounded smoke run.
- `games/athar/README.md`: records intended scope and run commands.
- `tools/allowed_deps.toml`: added the example's direct dependency policy and initially proposed allowing `rapier3d` for `ribbon_plugin_physics`; that proposal was later superseded as described above.
- `docs/decisions/0000-legacy-decision-log.md`: records the collision-backend choice and rationale.

## Verification status

### Verified by source inspection/static checks

- The starter uses `GpuWindowPlugin` and `Render3dPlugin`, not the offscreen/readback bridge.
- The example depends on engine crates/plugins only; no engine crate depends on the example or a game.
- No STUDS golden, expectation, or existing test file was changed, and no new tests were added in Phase 0.
- The workspace manifest, lockfile, new crate manifest, and dependency policy parse as TOML using Python's `tomllib`; a static script also confirmed workspace membership and that the new crate's direct dependencies are covered by its allowlist entry.

### Not verified

- Rust formatting, compilation, Clippy, workspace tests, and dependency-checker results remain unverified because the toolchain is unavailable in this environment.
- The native window has not been opened here; the placeholder floor, GPU adapter creation, and final image require manual verification on the Windows GPU.
- No screenshot has been produced, so no visual success is claimed.

## Local follow-up required before accepting Phase 0

From the repository root on a machine with the workspace's Rust toolchain:

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --workspace --no-default-features
python tools/check_deps.py
cargo run -p athar
```

If Windows rustc exits with OS error 1455, repeat the Rust checks serially with
`$env:CARGO_INCREMENTAL = '0'` and `cargo ... -j 1`. Keep Phase 0 open until the
four Rust checks and dependency check pass and the native window is visually
confirmed. Then commit with exactly:

```text
slice: phase 0 - baseline and native window
```
