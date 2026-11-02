ATHAR MSVC CRT packaging fix
============================

Root cause
----------
The previous packager searched only for Microsoft.VC143.CRT and depended on
VCINSTALLDIR or VCToolsRedistDir being populated. The supplied build log shows
Visual Studio 18 / MSVC 14.51.36231, whose current Visual Studio 2026 runtime
uses the VC145 CRT family. An ordinary PowerShell window may also have none of
the Developer Command Prompt environment variables set.

What changed
------------
- Discovers Visual Studio installations using vswhere.
- Uses VCINSTALLDIR, VSINSTALLDIR, VCToolsInstallDir and VCToolsRedistDir when set.
- Supports Microsoft.VC145.CRT and older Microsoft.VC143.CRT-style folders.
- Prefers the highest CRT generation and then the newest toolset folder.
- Copies all DLLs from the one selected x64 CRT directory, not a fixed three-file list.
- Adds --crt-dir for installations with a non-standard layout.
- Still fails when --require-windows-crt is specified and no valid CRT directory
  with vcruntime140.dll and msvcp140.dll is present. It does not silently skip
  runtime bundling.

Apply from the repository root
------------------------------
    git apply --check .\athar-vc-runtime-discovery.patch
    git apply .\athar-vc-runtime-discovery.patch

Then run the original packaging command:
    py -3 tools/package_athar.py --binary .\target\release\athar.exe --output .\dist\athar --require-windows-crt

Optional explicit path
----------------------
Find the installed VC145 CRT directory:
    $vcRoot = 'C:\Program Files\Microsoft Visual Studio\18\Community\VC'
    $crt = Get-ChildItem (Join-Path $vcRoot 'Redist\MSVC') -Directory -Recurse -Filter 'Microsoft.VC145.CRT' -ErrorAction SilentlyContinue | Where-Object { $_.Parent.Name -eq 'x64' } | Select-Object -First 1
    if (-not $crt) { throw 'VC145 x64 CRT folder not found; check the Visual Studio C++ redistributable components.' }

Then run:
    py -3 tools/package_athar.py --binary .\target\release\athar.exe --output .\dist\athar --require-windows-crt --crt-dir $crt.FullName

Validation performed for this fix
---------------------------------
- Python compile with SyntaxWarnings treated as errors: passed.
- CLI help invocation: passed.
- Synthetic directory test: auto-discovery preferred VC145 over VC143; explicit
  VC143 path was accepted for compatibility; all DLLs from the selected folder
  were copied.

Not performed here
------------------
- The user's Windows Visual Studio installation was not accessible here.
- The real ATHAR executable was not staged/launched against the user's installed
  VC145 CRT. Run the packaging command on Windows to verify the actual package.

Redistribution note
-------------------
Only package unmodified runtime DLLs from an appropriately licensed Visual
Studio installation and a redistributable (non-preview) toolset, following the
applicable Microsoft Visual Studio license terms.
