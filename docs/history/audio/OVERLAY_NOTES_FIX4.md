# Overlay notes — Fix 4

Replaces only `tools/build_audio_with_steam_audio.ps1` and adds this README.

Fix 3 had a false-negative MSVC test: it required `stddef.h` under MSVC and UCRT. Bindgen's diagnostic is for Clang's builtin header, supplied by LLVM's resource include tree. Fix 4 separates the MSVC, Windows SDK, libclang, and Clang resource-header discovery paths and adds the correct resource directory to Bindgen's include arguments.

The user's reported installation was checked against the file list: `vcruntime.h`, `yvals_core.h`, `cl.exe`, and `libcmt.lib` exist in MSVC 14.51.36231; only `MSVC\include\stddef.h` is absent. That is no longer treated as proof that MSVC is missing.
