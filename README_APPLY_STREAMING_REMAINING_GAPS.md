# Ribbon Engine — Remaining Streaming Gaps Continuation Overlay

Apply this incremental overlay **only** to the Ribbon checkout that passed
`tools/validate_streaming.ps1` with 15/15 checks after the duplicate-import and streaming-validation hotfixes.
It is not a complete source archive and is not intended for the original unmodified project ZIP.

From the repository root, extract this archive with paths preserved, overwriting the listed files.

Then run:

```powershell
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test -p ribbon_plugin_render3d --all-targets
cargo test -p ribbon_plugin_streaming --all-targets
cargo test --workspace --no-fail-fast
cargo clippy --workspace --all-targets -- -D warnings
py -3 tools/check_deps.py
```

The ignored real-adapter device-recreation test is intentionally separate from the regular suite:

```powershell
.\tools\test_shader_pipeline_device_recovery.ps1
```

The new source files and changes in this overlay have **not** been compiled or tested in the artifact-preparation environment. The previous 15/15 result belongs to the preceding checkout and must not be reused as evidence for this overlay.

See `README_STREAMING_REMAINING_GAPS.md` for exact implementation scope and known limitations.
