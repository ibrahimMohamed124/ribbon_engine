# ADR 0006: Split large source files by responsibility

## Decision

Split the asset server into handle/storage, source helpers, loader/plugin glue, and server operation modules; split the render3d mesh renderer into setup, asset upload, shadow, and draw modules; split GPU transient texture pooling from device/context ownership; split render-profile data and CSV writing; split schedule planning/errors/stats; and split the large STUDS app, UI, game read view, and graphics showcase into focused submodules.

## Compatibility

Public types and methods retain their previous crate paths through re-exports. The refactor is intended to preserve behavior and keep non-test Rust source files at or below 600 lines.

## Validation boundary

The current environment does not contain `cargo`, `rustc`, or `rustfmt`; no Rust build or test result is claimed. Run the Windows validation commands in `docs/release/VALIDATION_WINDOWS.md` after extracting the delivered archive.
