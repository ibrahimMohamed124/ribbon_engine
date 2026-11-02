
& {
    $ErrorActionPreference = "Stop"

    $vsRoot = "C:\Program Files\Microsoft Visual Studio\18\Community"
    $msvcRoot = Join-Path $vsRoot "VC\Tools\MSVC"
    $kitsRoot = Join-Path `
        ([Environment]::GetFolderPath("ProgramFilesX86")) `
        "Windows Kits\10\Include"

    $toolset = Get-ChildItem $msvcRoot -Directory |
        Sort-Object {
            try { [version]$_.Name } catch { [version]"0.0" }
        } -Descending |
        Select-Object -First 1

    $sdk = Get-ChildItem $kitsRoot -Directory |
        Where-Object {
            Test-Path (Join-Path $_.FullName "ucrt\stddef.h")
        } |
        Sort-Object {
            try { [version]$_.Name } catch { [version]"0.0" }
        } -Descending |
        Select-Object -First 1

    if (-not $toolset -or -not $sdk) {
        throw "Could not identify a valid MSVC toolset and Windows SDK."
    }

    $vcInclude = Join-Path $toolset.FullName "include"
    $ucrt = Join-Path $sdk.FullName "ucrt"

    $includeDirs = @(
        $ucrt
        $vcInclude
        (Join-Path $sdk.FullName "shared")
        (Join-Path $sdk.FullName "um")
        (Join-Path $sdk.FullName "winrt")
        (Join-Path $sdk.FullName "cppwinrt")
    ) | Where-Object { Test-Path $_ } | Select-Object -Unique

    $requiredHeaders = @(
        (Join-Path $ucrt "stddef.h")
        (Join-Path $ucrt "stdlib.h")
        (Join-Path $vcInclude "vcruntime.h")
        (Join-Path $vcInclude "intrin.h")
    )

    foreach ($header in $requiredHeaders) {
        if (-not (Test-Path $header)) {
            throw "Required header is missing: $header"
        }
        Write-Host "FOUND: $header"
    }

    # Pass include directories explicitly to bindgen/libclang.
    $argParts = @()

    foreach ($dir in $includeDirs) {
        $argParts += "-I"
        $argParts += ('"' + $dir + '"')
    }

    $clangArgs = $argParts -join " "

    $env:BINDGEN_EXTRA_CLANG_ARGS = $clangArgs
    $env:BINDGEN_EXTRA_CLANG_ARGS_x86_64_pc_windows_msvc = $clangArgs
    $env:CPATH = $includeDirs -join ";"
    $env:INCLUDE = $includeDirs -join ";"

    Write-Host "`n========== CONFIGURATION =========="
    Write-Host "MSVC: $($toolset.Name)"
    Write-Host "SDK : $($sdk.Name)"
    Write-Host "Clang arguments: $clangArgs"

    Write-Host "`n========== STEAM AUDIO BUILD =========="

    cargo run --release -p ribbon_plugin_audio --features steam-audio --example audio_stress

    if ($LASTEXITCODE -ne 0) {
        throw "Steam Audio build/run failed. Preserve the complete compiler output."
    }

    Write-Host "`nSUCCESS: Steam Audio stress test completed."
}
