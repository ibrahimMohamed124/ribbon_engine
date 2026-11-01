[CmdletBinding()]
param(
    [switch]$SkipTests,
    [switch]$SkipClippy
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot

function Get-LatestVersionDirectory {
    param([Parameter(Mandatory = $true)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Container)) { return $null }
    $directories = @(Get-ChildItem -LiteralPath $Path -Directory | Sort-Object {
        try { [version]$_.Name } catch { [version]'0.0' }
    } -Descending)
    if ($directories.Count -eq 0) { return $null }
    return $directories[0].FullName
}

function Get-ReadyDriveRoots {
    $roots = @()
    foreach ($drive in [System.IO.DriveInfo]::GetDrives()) {
        try {
            if ($drive.IsReady -and $drive.DriveType -eq [System.IO.DriveType]::Fixed) {
                $roots += $drive.RootDirectory.FullName
            }
        } catch { }
    }
    return @($roots | Select-Object -Unique)
}

function Add-UniqueExistingPath {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][System.Collections.Generic.List[string]]$List,
        [AllowNull()][string]$Path,
        [ValidateSet('Container', 'Leaf')][string]$Kind = 'Container'
    )
    if ([string]::IsNullOrWhiteSpace($Path)) { return }
    if ($Kind -eq 'Leaf') {
        if (Test-Path -LiteralPath $Path -PathType Leaf) {
            if (-not $List.Contains($Path)) { $List.Add($Path) }
        }
    } elseif (Test-Path -LiteralPath $Path -PathType Container) {
        if (-not $List.Contains($Path)) { $List.Add($Path) }
    }
}

function Get-VisualStudioWhere {
    $candidates = [System.Collections.Generic.List[string]]::new()
    $command = Get-Command 'vswhere.exe' -ErrorAction SilentlyContinue
    if ($command -and $command.Source) { Add-UniqueExistingPath -List $candidates -Path $command.Source -Kind Leaf }

    foreach ($programFiles in @(${env:ProgramFiles(x86)}, $env:ProgramFiles)) {
        if ($programFiles) {
            Add-UniqueExistingPath -List $candidates -Path (Join-Path $programFiles 'Microsoft Visual Studio\Installer\vswhere.exe') -Kind Leaf
        }
    }

    # Visual Studio can be installed on a non-system drive. vswhere is usually under
    # Program Files, but custom layouts are common, so inspect conventional locations
    # on every ready fixed drive instead of assuming C:.
    foreach ($driveRoot in (Get-ReadyDriveRoots)) {
        foreach ($relative in @(
            'Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe',
            'Program Files\Microsoft Visual Studio\Installer\vswhere.exe',
            'Microsoft Visual Studio\Installer\vswhere.exe',
            'Visual Studio\Installer\vswhere.exe',
            'VS\Installer\vswhere.exe',
            'BuildTools\Installer\vswhere.exe'
        )) {
            Add-UniqueExistingPath -List $candidates -Path (Join-Path $driveRoot $relative) -Kind Leaf
        }
    }
    if ($candidates.Count -eq 0) { return $null }
    return $candidates[0]
}

function Get-RegisteredVisualStudioRoots {
    param([Parameter(Mandatory = $true)][AllowEmptyString()][string]$VsWhere)

    $roots = [System.Collections.Generic.List[string]]::new()
    if (-not [string]::IsNullOrWhiteSpace($VsWhere) -and (Test-Path -LiteralPath $VsWhere -PathType Leaf)) {
        # Prefer instances that explicitly advertise the x86/x64 MSVC component.
        $required = @(& $VsWhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>$null)
        if ($LASTEXITCODE -eq 0) {
            foreach ($path in $required) { Add-UniqueExistingPath -List $roots -Path ([string]$path) }
        }

        # Fallback to all registered VS instances. Some Build Tools/custom installations
        # have a usable VC tree even when their component metadata is incomplete.
        $all = @(& $VsWhere -all -products '*' -property installationPath 2>$null)
        if ($LASTEXITCODE -eq 0) {
            foreach ($path in $all) { Add-UniqueExistingPath -List $roots -Path ([string]$path) }
        }
    }

    # Registry fallback for installations registered by older/custom Visual Studio
    # installers, including instances located on D: or another fixed drive.
    foreach ($registryPath in @(
        'HKLM:\SOFTWARE\Microsoft\VisualStudio\SxS\VS7',
        'HKLM:\SOFTWARE\WOW6432Node\Microsoft\VisualStudio\SxS\VS7'
    )) {
        try {
            $item = Get-ItemProperty -LiteralPath $registryPath -ErrorAction Stop
            foreach ($property in $item.PSObject.Properties) {
                if ($property.Name -match '^\d+\.\d+$' -and $property.Value -is [string]) {
                    Add-UniqueExistingPath -List $roots -Path $property.Value
                }
            }
        } catch { }
    }

    # Last-resort conventional roots for nonstandard, unregistered installs.
    foreach ($driveRoot in (Get-ReadyDriveRoots)) {
        foreach ($relative in @(
            'Microsoft Visual Studio\2022\Community',
            'Microsoft Visual Studio\2022\Professional',
            'Microsoft Visual Studio\2022\Enterprise',
            'Microsoft Visual Studio\2022\BuildTools',
            'Visual Studio\2022\Community',
            'Visual Studio\2022\Professional',
            'Visual Studio\2022\Enterprise',
            'Visual Studio\2022\BuildTools',
            'VS\2022\Community',
            'VS\2022\Professional',
            'VS\2022\Enterprise',
            'VS\2022\BuildTools'
        )) {
            Add-UniqueExistingPath -List $roots -Path (Join-Path $driveRoot $relative)
        }
    }
    return @($roots)
}

function Get-MsvcIncludeDirectory {
    param([Parameter(Mandatory = $true)][string[]]$VisualStudioRoots)
    $valid = [System.Collections.Generic.List[object]]::new()
    foreach ($vsRoot in $VisualStudioRoots) {
        $msvcRoot = Join-Path $vsRoot 'VC\Tools\MSVC'
        if (-not (Test-Path -LiteralPath $msvcRoot -PathType Container)) { continue }
        foreach ($versionDirectory in (Get-ChildItem -LiteralPath $msvcRoot -Directory)) {
            $include = Join-Path $versionDirectory.FullName 'include'
            $compiler = Join-Path $versionDirectory.FullName 'bin\Hostx64\x64\cl.exe'
            $runtimeHeader = Join-Path $include 'vcruntime.h'
            $stlHeader = Join-Path $include 'yvals_core.h'
            $runtimeLibrary = Join-Path $versionDirectory.FullName 'lib\x64\libcmt.lib'

            # Do not require stddef.h here. It is a Clang builtin header and belongs
            # to LLVM's resource include directory, not necessarily the MSVC tree.
            if ((Test-Path -LiteralPath $runtimeHeader -PathType Leaf) -and
                (Test-Path -LiteralPath $stlHeader -PathType Leaf) -and
                (Test-Path -LiteralPath $compiler -PathType Leaf) -and
                (Test-Path -LiteralPath $runtimeLibrary -PathType Leaf)) {
                $version = [version]'0.0'
                try { $version = [version]$versionDirectory.Name } catch { }
                $valid.Add([pscustomobject]@{
                    Version = $version
                    Include = $include
                    Root = $vsRoot
                    Compiler = $compiler
                    RuntimeLibrary = $runtimeLibrary
                })
            }
        }
    }
    if ($valid.Count -eq 0) { return $null }
    return ($valid | Sort-Object Version -Descending | Select-Object -First 1)
}

function Get-WindowsSdkIncludeRoot {
    $includeRoots = [System.Collections.Generic.List[string]]::new()
    foreach ($registryPath in @(
        'HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots',
        'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows Kits\Installed Roots'
    )) {
        try {
            $item = Get-ItemProperty -LiteralPath $registryPath -ErrorAction Stop
            if ($item.KitsRoot10) {
                Add-UniqueExistingPath -List $includeRoots -Path (Join-Path $item.KitsRoot10 'Include')
            }
        } catch { }
    }
    foreach ($programFiles in @(${env:ProgramFiles(x86)}, $env:ProgramFiles)) {
        if ($programFiles) {
            Add-UniqueExistingPath -List $includeRoots -Path (Join-Path $programFiles 'Windows Kits\10\Include')
        }
    }
    foreach ($driveRoot in (Get-ReadyDriveRoots)) {
        foreach ($relative in @(
            'Program Files (x86)\Windows Kits\10\Include',
            'Program Files\Windows Kits\10\Include',
            'Windows Kits\10\Include'
        )) {
            Add-UniqueExistingPath -List $includeRoots -Path (Join-Path $driveRoot $relative)
        }
    }
    foreach ($root in $includeRoots) {
        $versionRoot = Get-LatestVersionDirectory -Path $root
        # UCRT's corecrt.h is a reliable Windows SDK marker. stddef.h is not
        # required to be supplied by the Windows SDK; Clang ships its own copy.
        if ($versionRoot -and (Test-Path -LiteralPath (Join-Path $versionRoot 'ucrt\corecrt.h') -PathType Leaf)) {
            return $versionRoot
        }
    }
    return $null
}

Write-Host 'Discovering Visual Studio, MSVC and Windows SDK (any fixed drive)...' -ForegroundColor Cyan

$vswhere = Get-VisualStudioWhere
$vsRoots = @()
if ($vswhere) {
    Write-Host ("vswhere: {0}" -f $vswhere) -ForegroundColor DarkGray
    $vsRoots = @(Get-RegisteredVisualStudioRoots -VsWhere $vswhere)
} else {
    Write-Warning 'vswhere.exe was not found. Trying conventional Visual Studio paths and registry entries.'
    $vsRoots = @(Get-RegisteredVisualStudioRoots -VsWhere '')
}

$msvc = $null
if ($vsRoots.Count -gt 0) { $msvc = Get-MsvcIncludeDirectory -VisualStudioRoots $vsRoots }
if (-not $msvc) {
    $discovered = if ($vsRoots.Count -gt 0) { $vsRoots -join "`n  " } else { '(no registered Visual Studio roots found)' }
    throw @"
A Visual Studio root was not found with an installed MSVC C++ header tree.
Discovered Visual Studio roots:
  $discovered

The Visual Studio IDE being installed on D: is not sufficient by itself. The MSVC x86/x64 C++ workload must also be installed. In Visual Studio Installer > Modify, enable:
  - Desktop development with C++
  - MSVC v143 (or the toolset matching your VS version), x86/x64
  - Windows 10/11 SDK
Then rerun this script. The discovery logic checks every ready fixed drive and does not require Visual Studio to be on C:.
"@
}

$sdkVersionRoot = Get-WindowsSdkIncludeRoot
if (-not $sdkVersionRoot) {
    throw 'Windows SDK UCRT headers were not found. Install a Windows 10/11 SDK through Visual Studio Installer, then rerun this script.'
}

# Bindgen loads libclang.dll, while Clang's builtin headers (including stddef.h)
# live under <LLVM root>\lib\clang\<version>\include. These are separate paths.
# Search PATH, environment overrides, Visual Studio's LLVM, and fixed drives.
$clangBinaryDirectories = [System.Collections.Generic.List[string]]::new()
if ($env:LIBCLANG_PATH) {
    if (Test-Path -LiteralPath $env:LIBCLANG_PATH -PathType Leaf) {
        Add-UniqueExistingPath -List $clangBinaryDirectories -Path (Split-Path -Parent $env:LIBCLANG_PATH)
    } else {
        Add-UniqueExistingPath -List $clangBinaryDirectories -Path $env:LIBCLANG_PATH
    }
}
if ($env:LLVM_PATH) {
    Add-UniqueExistingPath -List $clangBinaryDirectories -Path (Join-Path $env:LLVM_PATH 'bin')
    Add-UniqueExistingPath -List $clangBinaryDirectories -Path $env:LLVM_PATH
}
$clangCommand = Get-Command 'clang.exe' -ErrorAction SilentlyContinue
if ($clangCommand -and $clangCommand.Source) {
    Add-UniqueExistingPath -List $clangBinaryDirectories -Path (Split-Path -Parent $clangCommand.Source)
}
foreach ($driveRoot in (Get-ReadyDriveRoots)) {
    foreach ($relative in @(
        'Program Files\LLVM\bin',
        'Program Files (x86)\LLVM\bin',
        'LLVM\bin',
        'Tools\LLVM\bin'
    )) {
        Add-UniqueExistingPath -List $clangBinaryDirectories -Path (Join-Path $driveRoot $relative)
    }
}
foreach ($vsRoot in $vsRoots) {
    Add-UniqueExistingPath -List $clangBinaryDirectories -Path (Join-Path $vsRoot 'VC\Tools\Llvm\bin')
}

$clangBinaryDirectory = $clangBinaryDirectories |
    Where-Object { Test-Path -LiteralPath (Join-Path $_ 'libclang.dll') -PathType Leaf } |
    Select-Object -First 1
if ($clangBinaryDirectory) {
    $env:LIBCLANG_PATH = $clangBinaryDirectory
}

$resourceIncludeDirectories = [System.Collections.Generic.List[string]]::new()
if ($env:CLANG_RESOURCE_DIR) {
    $resourceCandidate = $env:CLANG_RESOURCE_DIR
    if ((Split-Path -Leaf $resourceCandidate) -ne 'include') {
        $resourceCandidate = Join-Path $resourceCandidate 'include'
    }
    Add-UniqueExistingPath -List $resourceIncludeDirectories -Path $resourceCandidate
}

# Query clang itself when available. This is the authoritative way to locate
# its resource directory and avoids assuming LLVM's versioned directory layout.
foreach ($binaryDirectory in $clangBinaryDirectories) {
    $clangExe = Join-Path $binaryDirectory 'clang.exe'
    if (Test-Path -LiteralPath $clangExe -PathType Leaf) {
        try {
            $resourceOutput = @(& $clangExe '-print-resource-dir' 2>$null)
            if ($LASTEXITCODE -eq 0 -and $resourceOutput.Count -gt 0) {
                $resourceRoot = ([string]$resourceOutput[0]).Trim()
                if (-not [string]::IsNullOrWhiteSpace($resourceRoot)) {
                    Add-UniqueExistingPath -List $resourceIncludeDirectories -Path (Join-Path $resourceRoot 'include')
                }
            }
        } catch { }
    }

    # Fallback for installations that provide libclang.dll and resource headers
    # but omit clang.exe from PATH.
    $llvmRoot = Split-Path -Parent $binaryDirectory
    $versionedResourceRoot = Join-Path $llvmRoot 'lib\clang'
    $resourceVersion = Get-LatestVersionDirectory -Path $versionedResourceRoot
    if ($resourceVersion) {
        Add-UniqueExistingPath -List $resourceIncludeDirectories -Path (Join-Path $resourceVersion 'include')
    }
}

$clangBuiltinInclude = $resourceIncludeDirectories |
    Where-Object { Test-Path -LiteralPath (Join-Path $_ 'stddef.h') -PathType Leaf } |
    Select-Object -First 1
if (-not $clangBuiltinInclude) {
    $searched = if ($resourceIncludeDirectories.Count -gt 0) {
        $resourceIncludeDirectories -join "`n  "
    } else {
        '(no Clang resource include directories discovered)'
    }
    throw @"
Clang's builtin stddef.h was not found. This is not an MSVC workload check:
stddef.h is shipped with Clang under lib\clang\<version>\include.

Discovered MSVC toolset: $($msvc.Version)
MSVC compiler: $($msvc.Compiler)
MSVC include: $($msvc.Include)
Clang binary directories:
  $($clangBinaryDirectories -join "`n  ")
Clang resource include directories searched:
  $searched

Install the full LLVM distribution (including Clang resource headers), or set:
  `$env:LLVM_PATH = 'D:\path\to\LLVM'
  `$env:LIBCLANG_PATH = 'D:\path\to\LLVM\bin'
Then rerun this script. Do not copy or fabricate stddef.h into the MSVC directory.
"@
}

$includeDirectories = [System.Collections.Generic.List[string]]::new()
# Put Clang's own builtin headers first, then MSVC and Windows SDK system headers.
Add-UniqueExistingPath -List $includeDirectories -Path $clangBuiltinInclude
Add-UniqueExistingPath -List $includeDirectories -Path $msvc.Include
foreach ($sdkSubdirectory in @('ucrt', 'shared', 'um', 'winrt', 'cppwinrt')) {
    Add-UniqueExistingPath -List $includeDirectories -Path (Join-Path $sdkVersionRoot $sdkSubdirectory)
}
$includeDirectories = @($includeDirectories | Select-Object -Unique | Where-Object { Test-Path -LiteralPath $_ -PathType Container })

$clangArguments = ($includeDirectories | ForEach-Object { '-I"{0}"' -f $_ }) -join ' '
if ($env:BINDGEN_EXTRA_CLANG_ARGS) {
    $clangArguments = ($clangArguments + ' ' + $env:BINDGEN_EXTRA_CLANG_ARGS).Trim()
}
$env:BINDGEN_EXTRA_CLANG_ARGS = $clangArguments

Write-Host ("Visual Studio: {0}" -f $msvc.Root) -ForegroundColor Green
Write-Host ("MSVC include : {0}" -f $msvc.Include) -ForegroundColor Green
Write-Host ("Windows SDK  : {0}" -f $sdkVersionRoot) -ForegroundColor Green
Write-Host ("Clang builtins: {0}" -f $clangBuiltinInclude) -ForegroundColor Green
Write-Host ("Clang stddef : {0}" -f (Join-Path $clangBuiltinInclude 'stddef.h')) -ForegroundColor Green
if ($clangBinaryDirectory) {
    Write-Host ("libclang     : {0}" -f $clangBinaryDirectory) -ForegroundColor Green
} else {
    Write-Warning 'libclang.dll was not found in common LLVM locations. Bindgen must be able to discover libclang through its existing configuration.'
}
Write-Host 'BINDGEN_EXTRA_CLANG_ARGS configured for this PowerShell process.' -ForegroundColor Green

Push-Location $repoRoot
try {
    $checks = @(
        @{ Name = 'Steam Audio feature check'; Args = @('check', '-p', 'ribbon_plugin_audio', '--features', 'steam-audio', '--all-targets') }
    )
    if (-not $SkipTests) {
        $checks += @{ Name = 'Steam Audio feature tests'; Args = @('test', '-p', 'ribbon_plugin_audio', '--features', 'steam-audio', '--all-targets', '--no-fail-fast') }
    }
    if (-not $SkipClippy) {
        $checks += @{ Name = 'Steam Audio feature Clippy'; Args = @('clippy', '-p', 'ribbon_plugin_audio', '--features', 'steam-audio', '--all-targets', '--', '-D', 'warnings') }
    }

    foreach ($check in $checks) {
        Write-Host "`n============================================================" -ForegroundColor Cyan
        Write-Host ("RUNNING: {0}" -f $check.Name) -ForegroundColor Cyan
        Write-Host ("cargo {0}" -f ($check.Args -join ' '))
        $cargoArgs = $check.Args
        & cargo @cargoArgs
        $exitCode = $LASTEXITCODE
        if ($exitCode -ne 0) {
            throw ("{0} failed with exit code {1}. See the first native/Clang diagnostic above." -f $check.Name, $exitCode)
        }
        Write-Host ("PASS: {0}" -f $check.Name) -ForegroundColor Green
    }
} finally {
    Pop-Location
}

Write-Host "`nSteam Audio feature validation completed successfully." -ForegroundColor Green
