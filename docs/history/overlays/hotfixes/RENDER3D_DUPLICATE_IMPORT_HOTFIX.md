# Render3D duplicate-import hotfix

This ZIP is for a checkout that already has the Production Streaming Overlay applied.
Extract it into the Ribbon workspace root, preserving paths; it replaces only
`plugins/render3d/src/lib.rs`.

The fix removes the redundant private `ModelGpuResidencyBudget` import. The public
re-export remains unchanged.

Then run:

```powershell
cargo check -p ribbon_plugin_render3d --all-targets
cargo test -p ribbon_plugin_render3d --all-targets
cargo test -p ribbon_plugin_streaming --all-targets
cargo test -p ribbon_plugin_editor_tools --all-targets
cargo test -p athar --all-targets
cargo check --workspace --all-targets
cargo test --workspace --no-fail-fast
cargo clippy --workspace --all-targets -- -D warnings
.\tools\validate_streaming.ps1
```

The fix has not been compiled in this packaging environment. The rerun above is
required to confirm the compiler blocker is removed and uncover any subsequent errors.
