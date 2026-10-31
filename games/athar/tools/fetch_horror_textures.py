#!/usr/bin/env python3
"""Download 1K CC0 PBR maps from Poly Haven for Athar room and optional village assets.

Only requests official Poly Haven file metadata and map URLs. If this fails (offline,
proxy blocked, API changes), generated PLACEHOLDER maps remain in place; no fake asset
credits are written. After a successful download run `python games/athar/tools/horror_room_assets.py`;
for the optional village expansion, also run `python games/athar/tools/horror_village_assets.py`.
"""
from __future__ import annotations
import json
import sys
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
GAME_ROOT = ROOT / "games" / "athar"
DEST = GAME_ROOT / "assets" / "textures" / "source"
ASSETS = {
    # Keys are our local map suffixes; values are exact/case-sensitive aliases seen in
    # Poly Haven's /files payloads. We read only URLs returned by that official API.
    "concrete_wall_003": {
        "diff": ("Diffuse", "diff", "diffuse"),
        "arm": ("arm", "AO/Rough/Metal"),
        "nor_gl": ("nor_gl", "Normal (GL)"),
    },
    "concrete_floor_damaged_01": {
        "diff": ("Diffuse", "diff", "diffuse"),
        "arm": ("arm", "AO/Rough/Metal"),
        "nor_gl": ("nor_gl", "Normal (GL)"),
    },
    "rusty_metal_02": {
        "diff": ("Diffuse", "diff", "diffuse"),
        "arm": ("arm", "AO/Rough/Metal"),
        "nor_gl": ("nor_gl", "Normal (GL)"),
    },
    # Village materials use dedicated CC0 road, roof and timber maps when available.
    "cobblestone_floor_001": {
        "diff": ("Diffuse", "diff", "diffuse"),
        "arm": ("arm", "AO/Rough/Metal"),
        "nor_gl": ("nor_gl", "Normal (GL)"),
    },
    "roof_tiles": {
        "diff": ("Diffuse", "diff", "diffuse"),
        "arm": ("arm", "AO/Rough/Metal"),
        "nor_gl": ("nor_gl", "Normal (GL)"),
    },
    "wood_planks": {
        "diff": ("Diffuse", "diff", "diffuse"),
        "arm": ("arm", "AO/Rough/Metal"),
        "nor_gl": ("nor_gl", "Normal (GL)"),
    },
}
USER_AGENT = "Ribbon-Athar-AssetSetup/1.0 (Poly Haven CC0 textures)"


def get(url: str, timeout: int = 45) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return response.read()


def _nested(mapping: object, *keys: str):
    current = mapping
    for key in keys:
        if not isinstance(current, dict):
            return None
        current = current.get(key)
    return current


def _file_entry(metadata: dict, *map_names: str) -> dict:
    # The official /files/{asset} response is a nested tree (map -> resolution -> format).
    # Map labels vary between materials/API responses, so try documented aliases exactly.
    for map_name in map_names:
        candidates = [
            _nested(metadata, map_name, "1k", "jpg"),
            # Tolerate alternate nesting patterns without constructing/guessing file URLs.
            _nested(metadata, "1k", map_name, "jpg"),
            _nested(metadata, map_name, "jpg", "1k"),
            _nested(metadata, "jpg", "1k", map_name),
        ]
        for item in candidates:
            if isinstance(item, dict) and isinstance(item.get("url"), str):
                return item
    raise RuntimeError(f"Poly Haven did not return 1K JPG map under any alias: {map_names}")


def main() -> int:
    DEST.mkdir(parents=True, exist_ok=True)
    written = []
    try:
        # Download/validate the entire set in memory before touching any local source files.
        for asset, maps in ASSETS.items():
            api = f"https://api.polyhaven.com/files/{asset}"
            metadata = json.loads(get(api).decode("utf-8"))
            for suffix, aliases in maps.items():
                item = _file_entry(metadata, *aliases)
                data = get(item["url"])
                if len(data) < 1024 or not data.startswith(b"\xff\xd8"):
                    raise RuntimeError(f"Downloaded map `{asset}_{suffix}` is not a valid JPEG")
                path = DEST / f"{asset}_{suffix}_1k.jpg"
                written.append((path, data))
        for path, data in written:
            path.write_bytes(data)
            print(f"downloaded {path.relative_to(ROOT)} ({len(data):,} bytes)")
    except (OSError, urllib.error.URLError, ValueError, KeyError, RuntimeError) as error:
        print(f"ERROR: could not download the complete Poly Haven texture set: {error}", file=sys.stderr)
        print("No attribution claim was changed and no new source map was written; bundled CC0 fallback maps and explicitly marked PLACEHOLDER maps remain usable.", file=sys.stderr)
        return 1
    print(f"Downloaded {len(written)} 1K maps. Next run: python games/athar/tools/horror_room_assets.py; for the optional village, also run python games/athar/tools/horror_village_assets.py")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
