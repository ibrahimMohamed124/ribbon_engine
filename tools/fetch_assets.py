#!/usr/bin/env python3
"""Fetch hash-verified source assets and validate compact runtime assets."""

from __future__ import annotations

import argparse
import hashlib
import io
import re
import subprocess
import sys
import tomllib
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "assets" / "manifest.toml"
SOURCES = ROOT / "assets" / "source"  # ignored authoring cache
MAX_BYTES = 40 * 1024 * 1024
MAX_TRACKED_FILE = 5 * 1024 * 1024
ALLOWED_LICENSES = {"CC0-1.0", "CC-BY-3.0", "CC-BY-4.0"}
RUNTIME_EXTENSIONS = {".rgba", ".png", ".wav", ".txt"}


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def fetch(url: str, expected_sha256: str, destination: Path) -> bytes:
    if destination.is_file():
        data = destination.read_bytes()
    else:
        try:
            request = urllib.request.Request(url, headers={"User-Agent": "Voxelworks-asset-fetch/1.0"})
            with urllib.request.urlopen(request, timeout=45) as response:
                data = response.read(MAX_BYTES + 1)
        except (OSError, urllib.error.URLError) as error:
            raise RuntimeError(f"could not fetch {url}: {error}") from error
        if len(data) > MAX_BYTES:
            raise RuntimeError(f"download exceeded {MAX_BYTES} bytes: {url}")
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(data)
    actual = sha256(data)
    if actual != expected_sha256:
        raise RuntimeError(f"SHA-256 mismatch for {url}: expected {expected_sha256}, got {actual}")
    return data


def process_set(row: dict) -> None:
    try:
        from PIL import Image
    except ImportError as error:
        raise RuntimeError("Pillow is required for processing. Install it with `python -m pip install Pillow`.") from error

    downloaded: dict[str, bytes] = {}
    for source in row["source_maps"]:
        name = source["name"]
        downloaded[name] = fetch(
            source["url"], source["sha256"], SOURCES / row["asset_id"] / f"{name}.png"
        )

    images = {name: Image.open(io.BytesIO(data)).convert("RGB") for name, data in downloaded.items()}
    size = (256, 256)
    albedo = images["albedo"].resize(size, Image.Resampling.LANCZOS).convert("RGB")
    normal = images["normal"].resize(size, Image.Resampling.LANCZOS).convert("RGB")
    arm = images["arm"].resize(size, Image.Resampling.LANCZOS)
    runtime_dir = ROOT / row["runtime_path"]
    runtime_dir.mkdir(parents=True, exist_ok=True)
    runtime_maps = {
        "albedo": albedo.convert("RGBA"),
        "normal": normal.convert("RGBA"),
        "roughness": Image.merge("RGB", (arm.getchannel("G"),) * 3).convert("RGBA"),
        "ao": Image.merge("RGB", (arm.getchannel("R"),) * 3).convert("RGBA"),
    }
    for name, image in runtime_maps.items():
        path = runtime_dir / f"{name}.rgba"
        data = image.tobytes()
        path.write_bytes(data)
        expected = row["runtime_sha256"][name]
        actual = sha256(data)
        if actual != expected:
            raise RuntimeError(
                f"runtime hash mismatch for {row['asset_id']}/{name}: expected {expected}, got {actual}"
            )


def write_credits(rows: list[dict], monsters: list[dict]) -> None:
    lines = [
        "# Asset credits",
        "",
        "Raw third-party source files and archives are fetched on demand and are excluded from version control. Only compact runtime maps and baked monster sprites are shipped.",
        "",
    ]
    for row in rows:
        lines.extend([
            f"## {row['name']}",
            "",
            f"- Source: [{row['source_url']}]({row['source_url']})",
            f"- Author: {row['author']}",
            f"- License: {row['license']}",
            f"- Use: {row['usage']}",
            f"- Runtime maps: `{row['runtime_path']}/` (256x256 raw RGBA8)",
            "- Source map URLs and SHA-256:",
            *[
                f"  - {source['name']}: [{source['url']}]({source['url']}) (`{source['sha256']}`)"
                for source in row["source_maps"]
            ],
            "",
        ])
    for row in monsters:
        lines.extend([
            f"## {row['name']}",
            "",
            f"- Source: [{row['source_url']}]({row['source_url']})",
            f"- Download: [{row['download_url']}]({row['download_url']})",
            f"- Author and provenance: {row['author']}",
            f"- License: {row['license']} (the source page displays CC0)",
            f"- Use: {row['usage']}",
            f"- Raw archive (ignored): `{row['local_path']}`",
            f"- Selected model: `{row['selected_model']}`",
            f"- SHA-256: `{row['sha256']}`",
            f"- Baked runtime maps: `{row['albedo_path']}`, `{row['normal_path']}`, and `{row['emissive_path']}` (digests are recorded in the manifest)",
            "",
        ])
    (ROOT / "assets" / "CREDITS.md").write_text("\n".join(lines), encoding="utf-8", newline="\n")


def check(rows: list[dict], monsters: list[dict]) -> bool:
    good = True
    tracked = subprocess.run(["git", "ls-files", "-z"], cwd=ROOT, capture_output=True, check=False)
    if tracked.returncode != 0:
        print("could not inspect tracked files for raw-source and size checks")
        good = False
        names: list[str] = []
    else:
        names = tracked.stdout.decode("utf-8", errors="replace").split("\0")

    for name in names:
        if not name:
            continue
        path = ROOT / name
        if not path.exists():
            # Deleted-but-not-yet-staged paths are no longer part of the tree.
            continue
        lowered = name.lower()
        if any(lowered.endswith(ext) for ext in (".zip", ".7z", ".rar", ".tar", ".gz")):
            print(f"archive must not be committed: {name}")
            good = False
        if any(part in {"source", "raw", "unpacked"} for part in Path(name).parts):
            print(f"raw authoring data must not be committed: {name}")
            good = False
        if path.is_file() and path.stat().st_size > MAX_TRACKED_FILE:
            runtime_asset = "assets" in Path(name).parts and path.suffix.lower() in RUNTIME_EXTENSIONS
            if not runtime_asset:
                print(f"tracked file exceeds 5 MiB and is not a runtime asset: {name}")
                good = False

    total_size = 0
    for row in rows:
        runtime_dir = ROOT / row["runtime_path"]
        for name in ("albedo", "normal", "roughness", "ao"):
            path = runtime_dir / f"{name}.rgba"
            if not path.is_file():
                print(f"missing processed runtime map: {path}")
                good = False
                continue
            actual = sha256(path.read_bytes())
            expected = row["runtime_sha256"][name]
            if actual != expected:
                print(f"runtime hash mismatch for {row['asset_id']}/{name}: expected {expected}, got {actual}")
                good = False
            total_size += path.stat().st_size
        for source in row["source_maps"]:
            path = SOURCES / row["asset_id"] / f"{source['name']}.png"
            if path.is_file() and sha256(path.read_bytes()) != source["sha256"]:
                print(f"cached source hash mismatch: {path}")
                good = False

    for row in monsters:
        source = ROOT / row["local_path"]
        if source.is_file() and sha256(source.read_bytes()) != row["sha256"]:
            print(f"monster source hash mismatch: {row['name']}")
            good = False
        for name in ("albedo", "normal", "emissive"):
            baked = ROOT / row[f"{name}_path"]
            if not baked.is_file():
                print(f"missing baked monster {name}: {baked}")
                good = False
                continue
            actual = sha256(baked.read_bytes())
            expected = row[f"{name}_sha256"]
            if actual != expected:
                print(f"baked monster {name} hash mismatch: expected {expected}, got {actual}")
                good = False
            total_size += baked.stat().st_size
    if total_size > MAX_BYTES:
        print(f"processed runtime assets exceed the 40 MiB cap: {total_size} bytes")
        good = False
    return good


def validate_manifest(rows: list[dict], require_hashes: bool) -> None:
    required = {"name", "asset_id", "source_url", "author", "license", "source_maps", "runtime_path", "runtime_sha256", "usage"}
    for row in rows:
        missing = required - row.keys()
        if missing:
            raise RuntimeError(f"manifest entry {row.get('asset_id', '<unnamed>')} is missing: {', '.join(sorted(missing))}")
        if row["license"] not in ALLOWED_LICENSES:
            raise RuntimeError(f"license is not approved for redistribution: {row['asset_id']} ({row['license']})")
        if {source["name"] for source in row["source_maps"]} != {"albedo", "normal", "arm"}:
            raise RuntimeError(f"source maps are incomplete for {row['asset_id']}")
        for source in row["source_maps"]:
            if not source["url"].startswith("https://"):
                raise RuntimeError(f"invalid source URL for {row['asset_id']}/{source['name']}")
            if require_hashes and not re.fullmatch(r"[0-9a-f]{64}", source["sha256"]):
                raise RuntimeError(f"missing or invalid source SHA-256 for {row['asset_id']}/{source['name']}")
        for name in ("albedo", "normal", "roughness", "ao"):
            if require_hashes and not re.fullmatch(r"[0-9a-f]{64}", row["runtime_sha256"].get(name, "")):
                raise RuntimeError(f"missing or invalid runtime SHA-256 for {row['asset_id']}/{name}")


def validate_monster_manifest(rows: list[dict]) -> None:
    required = {"name", "source_url", "download_url", "author", "license", "sha256", "local_path", "usage", "selected_model", "processed_path", "albedo_path", "albedo_sha256", "normal_path", "normal_sha256", "emissive_path", "emissive_sha256"}
    for row in rows:
        missing = required - row.keys()
        if missing:
            raise RuntimeError(f"monster manifest entry is missing: {', '.join(sorted(missing))}")
        if row["license"] not in ALLOWED_LICENSES:
            raise RuntimeError(f"monster license is not approved: {row['license']}")
        for key in ("sha256", "albedo_sha256", "normal_sha256", "emissive_sha256"):
            if not re.fullmatch(r"[0-9a-f]{64}", row[key]):
                raise RuntimeError(f"monster {key} is missing or invalid for {row['name']}")


def fetch_monster_sources(rows: list[dict]) -> None:
    for row in rows:
        fetch(row["download_url"], row["sha256"], ROOT / row["local_path"])


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="verify manifest and runtime hashes without downloading")
    args = parser.parse_args()
    manifest = tomllib.loads(MANIFEST.read_text(encoding="utf-8"))
    rows = manifest["assets"]
    monsters = manifest.get("monsters", [])
    validate_manifest(rows, require_hashes=True)
    validate_monster_manifest(monsters)
    if args.check:
        write_credits(rows, monsters)
        return 0 if check(rows, monsters) else 1

    try:
        for row in rows:
            print(f"Fetching and processing {row['asset_id']}...")
            process_set(row)
        fetch_monster_sources(monsters)
        write_credits(rows, monsters)
    except Exception as error:
        print(f"Asset fetch failed: {error}", file=sys.stderr)
        print("No placeholder or fabricated asset files were created.", file=sys.stderr)
        return 1
    print("Hash-verified sources processed into compact runtime maps.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
