# Ribbon Audio — Steam Audio Windows Fix 4

This overlay replaces `tools/build_audio_with_steam_audio.ps1` from Fix 3.

## Root cause corrected

Fix 3 incorrectly required `stddef.h` inside the MSVC include directory and the Windows SDK UCRT directory. The `stddef.h` reported missing by Bindgen is a Clang builtin header. Clang ships builtin headers under its resource directory, typically:

`<LLVM root>\lib\clang\<version>\include\stddef.h`

This is separate from the MSVC headers (`vcruntime.h`, `yvals_core.h`) and Windows SDK headers (`ucrt\corecrt.h`). Microsoft's MSVC toolset can therefore be present and usable even though the Clang builtin header is absent from the MSVC include directory.

Fix 4 now:

- Validates MSVC using `vcruntime.h`, `yvals_core.h`, `cl.exe`, and `libcmt.lib`, without requiring `stddef.h` there.
- Validates Windows SDK using UCRT's `corecrt.h`.
- Finds `clang.exe` and `libclang.dll` through PATH, environment overrides, Visual Studio LLVM, and conventional fixed-drive paths.
- Queries `clang.exe -print-resource-dir` and falls back to LLVM's versioned `lib\clang\<version>\include` layout.
- Adds Clang builtin headers, MSVC headers, and Windows SDK headers to `BINDGEN_EXTRA_CLANG_ARGS`, with the Clang builtin directory first.
- Emits a specific diagnostic if the LLVM resource headers are not installed.

## Apply

Extract this archive over the Ribbon Engine repository root, replacing:

`tools/build_audio_with_steam_audio.ps1`

Then run:

```powershell
.\tools\build_audio_with_steam_audio.ps1
```

If LLVM is installed in a nonstandard location, set its paths before running the script:

```powershell
$env:LLVM_PATH = 'D:\path\to\LLVM'
$env:LIBCLANG_PATH = 'D:\path\to\LLVM\bin'
.\tools\build_audio_with_steam_audio.ps1
```

The script intentionally does not fabricate or copy `stddef.h`. Install the full LLVM distribution (including its resource headers) if the diagnostic says they are missing.

## Validation caveat

The script was statically reviewed and the ZIP contents were verified in the packaging environment. PowerShell and Windows MSVC/LLVM are not available in that environment, so the Windows feature build must still be confirmed on the target machine. Steam Audio is only considered validated after the script's feature check, tests, and Clippy all pass.
