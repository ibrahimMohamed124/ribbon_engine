#!/usr/bin/env python3
"""Stage a relocatable ATHAR release package and smoke-test its runtime asset lookup."""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import uuid

REPO_ROOT = Path(__file__).resolve().parents[1]
ATHAR_ROOT = REPO_ROOT / "games" / "athar"
VILLAGE_MODELS = (
    "village_house.glb",
    "village_hall.glb",
    "village_tree.glb",
    "village_ground.glb",
    "village_road.glb",
    "village_lamp.glb",
    "village_well.glb",
)
AUDIO_ASSETS = (
    "ambience_indoor.wav",
    "ambience_rain.wav",
    "door_creak_close.wav",
    "door_creak_open.wav",
    "door_locked.wav",
    "footstep_sprint.wav",
    "footstep_walk.wav",
    "key_pickup.wav",
    "monster_rustle.wav",
    "save_point.wav",
)
WINDOWS_CRT_REQUIRED = ("vcruntime140.dll", "msvcp140.dll")
WINDOWS_CRT_PATTERN = re.compile(r"Microsoft\.VC(?P<generation>\d+)\.CRT$", re.IGNORECASE)


def require_file(path: Path, description: str) -> None:
    if not path.is_file():
        raise RuntimeError(f"missing {description}: {path}")


def _version_key(value: str) -> tuple[int, ...]:
    """Sort MSVC toolset versions numerically (14.9 before 14.10)."""
    parts = re.findall(r"\d+", value)
    return tuple(int(part) for part in parts) or (0,)


def _vswhere_installations() -> list[Path]:
    """Return Visual Studio roots reported by the official vswhere utility."""
    program_files_x86 = os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")
    program_files = os.environ.get("ProgramFiles", r"C:\Program Files")
    possible = (
        Path(program_files_x86) / "Microsoft Visual Studio" / "Installer" / "vswhere.exe",
        Path(program_files) / "Microsoft Visual Studio" / "Installer" / "vswhere.exe",
    )
    for executable in possible:
        if not executable.is_file():
            continue
        try:
            result = subprocess.run(
                [str(executable), "-all", "-products", "*", "-property", "installationPath"],
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL,
                check=False,
            )
        except OSError:
            continue
        if result.returncode == 0:
            return [Path(line.strip()) for line in result.stdout.splitlines() if line.strip()]
    return []


def discover_windows_crt_dir(explicit: Path | None = None) -> tuple[Path | None, list[Path]]:
    r"""Find a supported x64 VC CRT directory without requiring a Developer PowerShell.

    Visual Studio environment variables are often unset in ordinary PowerShell.
    In that case, discover installed VS instances via vswhere and inspect their
    VC\Redist\MSVC trees. VCToolsInstallDir is also supported when present.
    """
    if explicit is not None:
        candidate = explicit.expanduser().resolve()
        if not WINDOWS_CRT_PATTERN.fullmatch(candidate.name) or candidate.parent.name.lower() != "x64":
            raise RuntimeError(
                "--crt-dir must point directly to an x64\\Microsoft.VCxxx.CRT directory: "
                f"{candidate}"
            )
        return candidate, [candidate]

    search_roots: list[Path] = []

    def add(path: Path | None) -> None:
        if path is None:
            return
        try:
            resolved = path.expanduser().resolve()
        except OSError:
            return
        if resolved not in search_roots and resolved.is_dir():
            search_roots.append(resolved)

    # These roots are most specific and should be checked first.
    redist_env = os.environ.get("VCToolsRedistDir")
    if redist_env:
        add(Path(redist_env))

    vc_install = os.environ.get("VCINSTALLDIR")
    if vc_install:
        add(Path(vc_install) / "Redist" / "MSVC")

    tools_install = os.environ.get("VCToolsInstallDir")
    if tools_install:
        tools_path = Path(tools_install)
        # ...\VC\Tools\MSVC\<toolset>\ -> ...\VC\Redist\MSVC\
        if len(tools_path.parents) >= 3:
            add(tools_path.parents[2] / "Redist" / "MSVC")

    vs_install = os.environ.get("VSINSTALLDIR")
    if vs_install:
        add(Path(vs_install) / "VC" / "Redist" / "MSVC")

    installations = _vswhere_installations()
    for installation in installations:
        add(installation / "VC" / "Redist" / "MSVC")

    # Fallback for standard VS 2022/18 layouts when vswhere is unavailable.
    for variable in ("ProgramFiles", "ProgramFiles(x86)"):
        base = os.environ.get(variable)
        if base:
            for generation in ("18", "2022"):
                for edition in ("Community", "Professional", "Enterprise", "BuildTools"):
                    add(
                        Path(base)
                        / "Microsoft Visual Studio"
                        / generation
                        / edition
                        / "VC"
                        / "Redist"
                        / "MSVC"
                    )

    discovered: list[Path] = []
    for root in search_roots:
        # The caller may supply the CRT directory itself through an environment
        # variable; otherwise search only under known redistributable roots.
        if WINDOWS_CRT_PATTERN.fullmatch(root.name) and root.parent.name.lower() == "x64":
            candidates = [root]
        else:
            try:
                candidates = list(root.glob("**/x64/Microsoft.VC*.CRT"))
            except OSError:
                candidates = []
        for candidate in candidates:
            if (
                candidate.is_dir()
                and candidate.parent.name.lower() == "x64"
                and WINDOWS_CRT_PATTERN.fullmatch(candidate.name)
                and candidate not in discovered
            ):
                discovered.append(candidate)

    def crt_priority(directory: Path) -> tuple[int, tuple[int, ...]]:
        match = WINDOWS_CRT_PATTERN.fullmatch(directory.name)
        generation = int(match.group("generation")) if match else 0
        toolset_version = _version_key(directory.parents[1].name)
        return generation, toolset_version

    discovered.sort(key=crt_priority, reverse=True)
    return (discovered[0] if discovered else None), search_roots


def copy_windows_crt(
    destination: Path,
    required: bool,
    explicit_crt_dir: Path | None = None,
) -> list[str]:
    if os.name != "nt":
        return []

    crt_dir, searched = discover_windows_crt_dir(explicit_crt_dir)
    if crt_dir is None:
        if required:
            searched_text = "\n".join(f"  - {path}" for path in searched) or "  (no Visual Studio redist roots discovered)"
            raise RuntimeError(
                "MSVC x64 runtime directory was not found. The packaging script now probes "
                "VS installations via vswhere, but no x64 Microsoft.VCxxx.CRT directory was discovered. "
                "Install the MSVC x64 redistributable files or pass --crt-dir pointing to "
                "...\\x64\\Microsoft.VC145.CRT (or the matching installed toolset). Searched roots:\n"
                + searched_text
            )
        print(
            "warning: MSVC runtime DLLs were not found; the packaged game may require the "
            "Microsoft Visual C++ 2015-2022 x64 Redistributable.",
            file=sys.stderr,
        )
        return []

    dlls = sorted(path for path in crt_dir.glob("*.dll") if path.is_file())
    names = {path.name.lower() for path in dlls}
    missing = [name for name in WINDOWS_CRT_REQUIRED if name not in names]
    if missing:
        raise RuntimeError(
            f"The discovered x64 CRT directory is incomplete ({crt_dir}); "
            f"missing required DLLs: {', '.join(missing)}"
        )
    if not dlls:
        raise RuntimeError(f"The discovered x64 CRT directory contains no DLLs: {crt_dir}")

    # Copy every DLL from the same Microsoft CRT component so dependent MSVC
    # libraries used by Rust and native plugins are not accidentally omitted.
    for item in dlls:
        shutil.copy2(item, destination / item.name)
    print(f"Bundled {len(dlls)} MSVC CRT DLL(s) from {crt_dir}")
    return [item.name for item in dlls]


def validate_staged_assets(assets: Path) -> None:
    require_file(assets / "room.ron", "room definition")
    require_file(assets / "CREDITS.md", "asset credits")
    require_file(assets / "asset_manifest.json", "asset manifest")
    for name in VILLAGE_MODELS:
        require_file(assets / "models" / name, f"village model {name}")
    for name in AUDIO_ASSETS:
        require_file(assets / "audio" / name, f"original audio asset {name}")


def package_readme(binary_name: str, crt_files: list[str]) -> str:
    windows_runtime = (
        "The package includes the required MSVC runtime DLLs: " + ", ".join(crt_files) + "."
        if crt_files
        else "On Windows, install the Microsoft Visual C++ 2015-2022 Redistributable (x64) when it is not already present."
    )
    return f"""# Ribbon ATHAR — packaged build

This directory is a relocatable ATHAR runtime package. Keep the `assets/` directory beside `{binary_name}`.

## Run

- Windows PowerShell: `./{binary_name}`
- Linux/macOS: `./{binary_name}`
- Explicit assets override: `./{binary_name} --assets ./assets`

The game also accepts `RIBBON_ATHAR_ASSETS` as an asset-root override. At startup it resolves assets in this order: `--assets`, `RIBBON_ATHAR_ASSETS`, an `assets/` folder next to the executable, and (debug builds only) the source checkout fallback.

## Validate the package

Run `{binary_name} --validate-assets` from any working directory. This checks the room definition, all room and streamed-village models, packaged audio WAVs, manifest, and credits without creating a window or opening an audio device.

## Runtime notes

{windows_runtime}
Linux and macOS builds still require the platform's normal window-system, graphics-driver, and audio-system libraries. If an audio device is unavailable, ATHAR logs a warning and continues without sound.

## Assets and licensing

See `assets/CREDITS.md` for model/texture attributions and licensing. ATHAR's ten WAV files in `assets/audio/` are original procedural syntheses created for this sample; they contain no third-party recordings.
"""


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True, help="built ATHAR executable")
    parser.add_argument("--output", type=Path, required=True, help="package directory")
    parser.add_argument(
        "--require-windows-crt",
        action="store_true",
        help="fail on Windows if the MSVC runtime DLLs cannot be bundled",
    )
    parser.add_argument(
        "--crt-dir",
        type=Path,
        help=r"explicit x64\Microsoft.VCxxx.CRT directory (optional; otherwise auto-discovered)",
    )
    args = parser.parse_args()

    binary = args.binary.resolve()
    output = args.output.resolve()
    require_file(binary, "ATHAR executable")
    if output == output.parent or output == REPO_ROOT:
        raise RuntimeError(f"refusing unsafe package output directory: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)
    staging = output.parent / f".{output.name}.staging-{uuid.uuid4().hex}"
    backup = output.parent / f".{output.name}.previous-{uuid.uuid4().hex}"

    try:
        staging.mkdir()
        binary_name = binary.name
        shutil.copy2(binary, staging / binary_name)
        shutil.copytree(ATHAR_ROOT / "assets", staging / "assets")
        validate_staged_assets(staging / "assets")
        crt_files = copy_windows_crt(staging, args.require_windows_crt, args.crt_dir)
        (staging / "README.txt").write_text(
            package_readme(binary_name, crt_files), encoding="utf-8"
        )

        # The binary must resolve the adjacent assets even when launched from an unrelated cwd.
        with tempfile.TemporaryDirectory(prefix="athar-package-cwd-") as cwd:
            completed = subprocess.run(
                [str(staging / binary_name), "--validate-assets"],
                cwd=cwd,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                check=False,
            )
        if completed.returncode != 0:
            raise RuntimeError(
                "packaged executable failed its relocated asset smoke test:\n" + completed.stdout
            )
        print(completed.stdout, end="" if completed.stdout.endswith("\n") else "\n")

        if output.exists():
            output.rename(backup)
        staging.rename(output)
        if backup.exists():
            shutil.rmtree(backup)
        print(f"ATHAR package staged at {output}")
        return 0
    except Exception:
        if staging.exists():
            shutil.rmtree(staging, ignore_errors=True)
        if backup.exists() and not output.exists():
            backup.rename(output)
        raise


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:  # produce a concise CI diagnostic
        print(f"ATHAR packaging failed: {error}", file=sys.stderr)
        raise SystemExit(1)
