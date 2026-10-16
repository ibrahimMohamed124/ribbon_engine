#!/usr/bin/env python3
"""Fetch Poly Haven 1K PBR maps and package compact 256px PNG texture sets."""

from __future__ import annotations

import argparse
import hashlib
import io
import re
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

    output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for name, image in (("albedo.png", albedo), ("normal.png", normal), ("roughness_ao_metallic.png", packed)):
            buffer = io.BytesIO()
            image.save(buffer, format="PNG", optimize=True)
            archive.writestr(name, buffer.getvalue())
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


def write_credits(rows: list[dict[str, str]]) -> None:
    lines = ["# Asset credits", "", "All listed assets are CC0-1.0. Source downloads are not bundled; processed files are created by `python tools/fetch_assets.py`.", ""]
    for row in rows:
        lines.extend([
            f"## {row['name']}",
            "",
            f"- Source: [{row['source_url']}]({row['source_url']})",
            f"- Author: {row['author']}",
            f"- License: {row['license']}",
            f"- Use: {row['usage']}",
            f"- Processed file: `{row['local_path']}`",
            f"- SHA-256: `{row['sha256']}`",
            "",
        ])
    (ROOT / "assets" / "CREDITS.md").write_text("\n".join(lines), encoding="utf-8", newline="\n")


def check(rows: list[dict[str, str]]) -> bool:
    good = True
    for row in rows:
        path = ROOT / row["local_path"]
        expected = row["sha256"]
        if expected == "pending-download" or not path.is_file():
            print(f"missing processed asset/hash: {row['asset_id']} ({path})")
            good = False
            continue
        actual = hashlib.sha256(path.read_bytes()).hexdigest()
        if actual != expected:
            print(f"hash mismatch: {row['asset_id']} expected {expected}, got {actual}")
            good = False
    return good


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="verify local files and manifest hashes without downloading")
    args = parser.parse_args()
    rows = tomllib.loads(MANIFEST.read_text(encoding="utf-8"))["assets"]
    write_credits(rows)
    if args.check:
        return 0 if check(rows) else 1

    hashes: dict[str, str] = {}
    try:
        for row in rows:
            output = ROOT / row["local_path"]
            print(f"Fetching and processing {row['asset_id']}...")
            hashes[row["asset_id"]] = process_set(row["asset_id"], output)
        update_hashes(rows, hashes)
        rows = tomllib.loads(MANIFEST.read_text(encoding="utf-8"))["assets"]
        write_credits(rows)
    except Exception as error:
        print(f"Asset fetch failed: {error}", file=sys.stderr)
        print("No placeholder or fabricated image files were created.", file=sys.stderr)
        return 1
    write_credits(rows)
    print("Processed texture sets and manifest hashes updated.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
