# Steam Audio Windows bindgen prerequisite

The standard audio configuration already passed on the reported Windows run: 71 unit tests, 5 decoder integration tests, default-feature `cargo check`, formatting and Clippy. The optional Steam Audio feature failed before compiling Ribbon code because AudioNimbus' `audionimbus-sys` build script invoked bindgen/libclang and Clang could not resolve the standard C header `stddef.h`.

This is a native build-environment issue, not a Rust type error in `ribbon_plugin_audio`. AudioNimbus documents Clang 9.0+ as a build requirement. Bindgen supports forwarding additional include arguments through `BINDGEN_EXTRA_CLANG_ARGS`.

`tools/build_audio_with_steam_audio.ps1` discovers the latest MSVC x86/x64 include tree and Windows 10/11 SDK include tree using Visual Studio Installer's `vswhere.exe`. It adds the MSVC, UCRT, shared, UM, WinRT, C++/WinRT and available Clang resource-header paths to `BINDGEN_EXTRA_CLANG_ARGS`, sets `LIBCLANG_PATH` when a standard LLVM `libclang.dll` location is found, then runs Steam Audio `cargo check`, tests and Clippy.

Prerequisite: Visual Studio Build Tools / Visual Studio with **Desktop development with C++**, MSVC x86/x64 tools and a Windows 10/11 SDK installed. The helper does not install those components. If it reports that `vswhere.exe`, MSVC headers, SDK headers or `stddef.h` are missing, use Visual Studio Installer to add/repair those components and rerun from a new PowerShell session.

No Rust build is claimed by this overlay: the editing environment used to produce it does not have the Windows MSVC toolchain or Cargo. The user must run the helper on the Windows development machine.
