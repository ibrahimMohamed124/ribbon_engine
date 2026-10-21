# Ribbon Engine window and launcher icon

The engine uses `assets/engine_icon.jpg` as the original supplied logo. Three derived files are checked in alongside it:

- `assets/engine_icon.png`: square emblem crop used as the editable, compact raster artwork.
- `assets/engine_icon.rgba`: 64 x 64 tightly packed RGBA pixels embedded directly into `engine_platform`, so window creation requires no image-decoder dependency.
- `assets/engine_icon.ico`: multi-resolution Windows icon used for the executable's PE icon resource.

## Runtime windows

`crates/engine_platform/src/window.rs` applies the icon through a shared `with_engine_icons` helper. The generic Winit `with_window_icon` sets the small Windows icon (`ICON_SMALL`) used in the titlebar. On Windows, the helper also uses Winit's `WindowAttributesExtWindows::with_taskbar_icon` to set the large icon (`ICON_BIG`) used by the taskbar. All active windowed applications use this helper through the shared `WinitRunner`.

## Launcher / executable icon on Windows

The workspace's windowed app packages include a small local `build.rs` that includes `build_support/windows/icon.rs`. On Windows, the helper tries `rc.exe` on `PATH`, discovers `rc.exe` under installed Windows SDK version directories (including common `Program Files (x86)\Windows Kits\10\bin\<version>\x64` locations), then tries `llvm-rc.exe` and `windres.exe`. The selected compiler converts `assets/engine_icon.ico` into a PE resource linked into the package's binary targets. The helper adds no Rust crate dependency. If no compiler works, Cargo prints a clear warning; runtime window icons still work, but Explorer/launcher executable icons will not be embedded for that build. Installing the Windows 10/11 SDK or adding `rc.exe` to `PATH` resolves that fallback case.

## Verification

From the workspace root on Windows:

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p horror_slice
```

For another windowed target, substitute its package name (for example `gpu_window`, `graphics_showcase`, `voxel_mesh_showcase`, `model_viewer`, `cube_spin`, `scene_hierarchy_3d`, `editor_demo`, `voxel_room`, or `studs`). Look for the icon in the titlebar and taskbar. The runtime taskbar icon is configured independently from the executable's embedded Explorer icon; the latter still requires a working Windows resource compiler. After rebuilding, the package executable should also carry the icon resource when the Windows resource compiler is available.

### Build-script lint compatibility

The shared helper normalizes Windows SDK version path separators using `trim_end_matches(['\\', '/'])`, avoiding Clippy's `manual_pattern_char_comparison` warning under `-D warnings`.
