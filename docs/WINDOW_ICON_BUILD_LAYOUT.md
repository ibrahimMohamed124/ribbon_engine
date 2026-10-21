# Windows icon build-helper layout

The PE resource embedding helper lives at `build_support/windows/icon.rs` because it is Windows-specific build tooling. Each executable package includes it from its own `build.rs`; this lets the icon resource be linked into that executable, rather than into a shared library.

Runtime window icons remain part of `crates/engine_platform`, where window creation sets both the small titlebar icon and the Windows taskbar icon. The resource helper does not own runtime window behavior.

All active executable build scripts include the single canonical implementation at `build_support/windows/icon.rs`. The former `build_support/windows_icon.rs` shim was removed when the last legacy package was retired, so future build changes have one supported path.

No Rust build or tests were run while creating this source overlay.
