#!/usr/bin/env python3
"""Fetch licensed source assets, build compact runtime textures, and check hashes."""

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
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "assets" / "manifest.toml"
SOURCES = ROOT / "assets" / "source"  # intentionally ignored by git
MAX_BYTES = 40 * 1024 * 1024
ALLOWED_LICENSES = {"CC0-1.0", "CC-BY-3.0", "CC-BY-4.0"}


def urls(asset_id: str) -> dict[str, list[str]]:
    base = f"https://dl.polyhaven.org/file/ph-assets/Textures/png/1k/{asset_id}/{asset_id}"
    # Poly Haven naming differs for a few legacy assets. Try official map names
    # in order, without querying or depending on a mutable third-party mirror.
    diffuse_suffix = "_diffuse_1k.png" if asset_id == "t_concrete_wall_002" else "_diff_1k.png"
    return {
        "albedo": [f"{base}{diffuse_suffix}"],
        "normal": [f"{base}_nor_gl_1k.png"],
        "arm": [f"{base}_arm_1k.png"],  # R=AO, G=roughness, B=metallic
    }


def fetch(url_candidates: list[str], destination: Path) -> bytes:
    if destination.is_file():
        return destination.read_bytes()
    last_error: Exception | None = None
    for url in url_candidates:
        try:
            request = urllib.request.Request(url, headers={"User-Agent": "STUDS-asset-fetch/1.0"})
            with urllib.request.urlopen(request, timeout=45) as response:
                data = response.read(MAX_BYTES + 1)
            if len(data) > MAX_BYTES:
                raise RuntimeError(f"download exceeded {MAX_BYTES} bytes: {url}")
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(data)
            return data
        except (OSError, urllib.error.URLError, RuntimeError) as error:
            last_error = error
    raise RuntimeError(f"could not fetch {url_candidates[0]}: {last_error}")


def process_set(asset_id: str, output: Path) -> str:
    try:
        from PIL import Image
    except ImportError as error:
        raise RuntimeError("Pillow is required for processing. Install it with `python -m pip install Pillow`.") from error

    downloaded: dict[str, bytes] = {}
    for map_name, candidates in urls(asset_id).items():
        downloaded[map_name] = fetch(candidates, SOURCES / asset_id / f"{map_name}.png")

    images = {name: Image.open(io.BytesIO(data)).convert("RGB") for name, data in downloaded.items()}
    size = (256, 256)
    albedo = images["albedo"].resize(size, Image.Resampling.LANCZOS).convert("RGB")
    normal = images["normal"].resize(size, Image.Resampling.LANCZOS).convert("RGB")
    arm = images["arm"].resize(size, Image.Resampling.LANCZOS)
    # Store roughness and AO in one compact two-channel-friendly RGB map. The
    # runtime can sample R/G; B remains metallic for future use.
    packed = Image.merge("RGB", (arm.getchannel("G"), arm.getchannel("R"), arm.getchannel("B")))

    # Raw RGBA sidecars are the runtime format. They need no image decoder in
    # engine_core/engine_scene and can be read on Windows with std::fs.
    runtime_dir = output.with_suffix("")
    runtime_dir.mkdir(parents=True, exist_ok=True)
    rgba_images = {
        "albedo.rgba": albedo.convert("RGBA"),
        "normal.rgba": normal.convert("RGBA"),
        "roughness.rgba": Image.merge("RGB", (arm.getchannel("G"),) * 3).convert("RGBA"),
        "ao.rgba": Image.merge("RGB", (arm.getchannel("R"),) * 3).convert("RGBA"),
    }
    for filename, image in rgba_images.items():
        (runtime_dir / filename).write_bytes(image.tobytes())

    output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for name, image in (("albedo.png", albedo), ("normal.png", normal), ("roughness_ao_metallic.png", packed)):
            buffer = io.BytesIO()
            image.save(buffer, format="PNG", optimize=True)
            archive.writestr(name, buffer.getvalue())
        for filename, image in rgba_images.items():
            archive.writestr(f"runtime/{filename}", image.tobytes())
    return hashlib.sha256(output.read_bytes()).hexdigest()


def update_hashes(rows: list[dict[str, str]], hashes: dict[str, str]) -> None:
    text = MANIFEST.read_text(encoding="utf-8")
    for row in rows:
        asset_id = row["asset_id"]
        pattern = rf'(asset_id\s*=\s*"{re.escape(asset_id)}"[\s\S]*?sha256\s*=\s*")[^"]*(")'
        text, count = re.subn(pattern, rf"\g<1>{hashes[asset_id]}\g<2>", text, count=1)
        if count != 1:
            raise RuntimeError(f"could not write digest for {asset_id} to {MANIFEST}")
    MANIFEST.write_text(text, encoding="utf-8", newline="\n")


def write_credits(rows: list[dict[str, str]], monsters: list[dict[str, str]]) -> None:
    lines = ["# Asset credits", "", "Texture source downloads and raw monster FBX archives are excluded from version control. Compact processed texture sets and baked monster sprites are intended for game distribution.", ""]
    for row in rows:
        lines.extend([
            f"## {row['name']}",
            "",
            f"- Source: [{row['source_url']}]({row['source_url']})",
            f"- Author: {row['author']}",
            f"- License: {row['license']}",
            f"- Use: {row['usage']}",
            f"- Processed file: `{row['local_path']}`",
            f"- Runtime maps: `{row['runtime_path']}/` (256x256 raw RGBA8)",
            f"- SHA-256: `{row['sha256']}`",
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
            f"- Baked game maps: `{row['albedo_path']}`, `{row['normal_path']}`, and `{row['emissive_path']}` (their SHA-256 values are recorded in `assets/manifest.toml`)",
            "",
        ])
    (ROOT / "assets" / "CREDITS.md").write_text("\n".join(lines), encoding="utf-8", newline="\n")


def check(rows: list[dict[str, str]], monsters: list[dict[str, str]]) -> bool:
    good = True
    tracked = subprocess.run(
        ["git", "ls-files", "-z"], cwd=ROOT, capture_output=True, check=False
    )
    if tracked.returncode != 0:
        print("could not inspect tracked files for accidental monster source commits")
        good = False
    else:
        for name in tracked.stdout.decode("utf-8", errors="replace").split("\0"):
            if name.startswith("assets/monster/source/"):
                print(f"authoring source must not be committed: {name}")
                good = False
            path = ROOT / name
            if name and path.is_file() and path.stat().st_size > MAX_BYTES:
                print(f"tracked file exceeds the 40 MiB asset/source limit: {name}")
                good = False
    for row in rows:
        path = ROOT / row["local_path"]
        expected = row["sha256"]
        runtime_dir = ROOT / row["runtime_path"]
        maps_exist = all((runtime_dir / filename).is_file() for filename in ("albedo.rgba", "normal.rgba", "roughness.rgba", "ao.rgba"))
        if expected == "pending-download" or not path.is_file() or not maps_exist:
            print(f"missing processed asset/hash/runtime maps: {row['asset_id']} ({path})")
            good = False
            continue
        actual = hashlib.sha256(path.read_bytes()).hexdigest()
        if actual != expected:
            print(f"hash mismatch: {row['asset_id']} expected {expected}, got {actual}")
            good = False
        try:
            with zipfile.ZipFile(path) as archive:
                names = set(archive.namelist())
            required = {"albedo.png", "normal.png", "roughness_ao_metallic.png"} | {
                f"runtime/{filename}"
                for filename in ("albedo.rgba", "normal.rgba", "roughness.rgba", "ao.rgba")
            }
            if not required <= names:
                print(f"processed archive is incomplete: {row['asset_id']}")
                good = False
        except zipfile.BadZipFile:
            print(f"invalid processed archive: {row['asset_id']}")
            good = False
    total_size = sum(
        (ROOT / row["local_path"]).stat().st_size
        for row in rows
        if (ROOT / row["local_path"]).is_file()
    )
    total_size += sum(
        (ROOT / row["runtime_path"] / filename).stat().st_size
        for row in rows
        for filename in ("albedo.rgba", "normal.rgba", "roughness.rgba", "ao.rgba")
        if (ROOT / row["runtime_path"] / filename).is_file()
    )
    if total_size > MAX_BYTES:
        print(f"processed assets exceed the 40 MiB shipping limit: {total_size} bytes")
        good = False
    for row in monsters:
        source = ROOT / row["local_path"]
        if source.is_file():
            actual = hashlib.sha256(source.read_bytes()).hexdigest()
            if actual != row["sha256"]:
                print(f"monster source hash mismatch: {row['name']} expected {row['sha256']}, got {actual}")
                good = False
        # The compact baked maps are shipped; CI verifies every digest and
        # accounts for them in the common asset budget.
        for kind in ("albedo", "normal", "emissive"):
            baked = ROOT / row[f"{kind}_path"]
            if not baked.is_file():
                print(f"missing baked monster {kind}: {baked}")
                good = False
                continue
            actual = hashlib.sha256(baked.read_bytes()).hexdigest()
            if actual != row[f"{kind}_sha256"]:
                print(f"baked monster {kind} hash mismatch: expected {row[f'{kind}_sha256']}, got {actual}")
                good = False
            total_size += baked.stat().st_size
    if total_size > MAX_BYTES:
        print(f"processed assets including monster exceed the 40 MiB shipping limit: {total_size} bytes")
        good = False
    return good


def validate_manifest(rows: list[dict[str, str]], require_hashes: bool) -> None:
    required = {
        "name", "asset_id", "source_url", "author", "license", "sha256",
        "local_path", "runtime_path", "usage",
    }
    for row in rows:
        missing = required - row.keys()
        if missing:
            raise RuntimeError(f"manifest entry {row.get('asset_id', '<unnamed>')} is missing: {', '.join(sorted(missing))}")
        if row["license"] not in ALLOWED_LICENSES:
            raise RuntimeError(f"license is not approved for redistribution: {row['asset_id']} ({row['license']})")
        if require_hashes and not re.fullmatch(r"[0-9a-f]{64}", row["sha256"]):
            raise RuntimeError(f"manifest SHA-256 is missing or invalid for {row['asset_id']}")


def validate_monster_manifest(rows: list[dict[str, str]]) -> None:
    required = {"name", "source_url", "download_url", "author", "license", "sha256", "local_path", "usage", "selected_model", "processed_path", "albedo_path", "albedo_sha256", "normal_path", "normal_sha256", "emissive_path", "emissive_sha256"}
    for row in rows:
        missing = required - row.keys()
        if missing:
            raise RuntimeError(f"monster manifest entry is missing: {', '.join(sorted(missing))}")
        if row["license"] not in ALLOWED_LICENSES:
            raise RuntimeError(f"monster license is not approved: {row['license']}")
        if not re.fullmatch(r"[0-9a-f]{64}", row["sha256"]):
            raise RuntimeError(f"monster SHA-256 is missing or invalid for {row['name']}")
        for kind in ("albedo", "normal", "emissive"):
            if not re.fullmatch(r"[0-9a-f]{64}", row[f"{kind}_sha256"]):
                raise RuntimeError(f"monster {kind} SHA-256 is missing or invalid for {row['name']}")


def fetch_monster_sources(rows: list[dict[str, str]]) -> None:
    for row in rows:
        destination = ROOT / row["local_path"]
        if not destination.is_file():
            fetch([row["download_url"]], destination)
        actual = hashlib.sha256(destination.read_bytes()).hexdigest()
        if actual != row["sha256"]:
            raise RuntimeError(f"monster source hash mismatch for {row['name']}: expected {row['sha256']}, got {actual}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="verify local files and manifest hashes without downloading")
    args = parser.parse_args()
    manifest = tomllib.loads(MANIFEST.read_text(encoding="utf-8"))
    rows = manifest["assets"]
    monsters = manifest.get("monsters", [])
    write_credits(rows, monsters)
    validate_manifest(rows, require_hashes=args.check)
    validate_monster_manifest(monsters)
    if args.check:
        return 0 if check(rows, monsters) else 1

    hashes: dict[str, str] = {}
    try:
        for row in rows:
            output = ROOT / row["local_path"]
            print(f"Fetching and processing {row['asset_id']}...")
            hashes[row["asset_id"]] = process_set(row["asset_id"], output)
        fetch_monster_sources(monsters)
        update_hashes(rows, hashes)
        rows = tomllib.loads(MANIFEST.read_text(encoding="utf-8"))["assets"]
        write_credits(rows, monsters)
    except Exception as error:
        print(f"Asset fetch failed: {error}", file=sys.stderr)
        print("No placeholder or fabricated image files were created.", file=sys.stderr)
        return 1
    write_credits(rows, monsters)
    print("Processed texture sets and manifest hashes updated.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
