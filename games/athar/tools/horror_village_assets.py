#!/usr/bin/env python3
"""Build self-contained CC0-textured village models for the Horror Slice.

Uses the same standard-library GLB writer as the room assets. When the more specific
wood/roof/cobblestone downloads are missing, models use the already bundled Poly Haven
concrete and rusty-metal maps, so the optional village models remain self-contained.
Run `python games/athar/tools/fetch_horror_textures.py`, then
`python games/athar/tools/horror_room_assets.py` and
`python games/athar/tools/horror_village_assets.py` to regenerate them.
"""
from __future__ import annotations

import json
import math
from pathlib import Path

from horror_room_assets import (
    ASSET_ROOT,
    MODEL_ROOT,
    POLYHAVEN,
    SOURCE_ROOT,
    Part,
    _make_material,
    _write_glb,
)

ROOT = Path(__file__).resolve().parents[3]
MANIFEST = ASSET_ROOT / "asset_manifest.json"


def _has_downloaded_maps(asset: str) -> bool:
    return all(
        (SOURCE_ROOT / f"{asset}_{suffix}_1k.jpg").is_file()
        for suffix in ("diff", "arm", "nor_gl")
    )


def _asset(preferred: str, fallback: str) -> str:
    return preferred if _has_downloaded_maps(preferred) else fallback


def _rotate_part_z(part: Part, angle: float, offset: tuple[float, float, float]) -> None:
    c, s = math.cos(angle), math.sin(angle)
    ox, oy, oz = offset
    part.positions = [(x * c - y * s + ox, x * s + y * c + oy, z + oz) for x, y, z in part.positions]
    part.normals = [(x * c - y * s, x * s + y * c, z) for x, y, z in part.normals]


def _quad(part: Part, points, uv_scale: float = 1.0, reverse: bool = False) -> None:
    """Add an arbitrarily sloped quad with computed normals and tiled UVs."""
    points = list(points)
    if reverse:
        points.reverse()
    a, b, c, _d = points
    ab = (b[0] - a[0], b[1] - a[1], b[2] - a[2])
    ac = (c[0] - a[0], c[1] - a[1], c[2] - a[2])
    normal = (
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    )
    length = math.sqrt(sum(v * v for v in normal)) or 1.0
    normal = tuple(v / length for v in normal)
    xs = [p[0] for p in points]
    ys = [p[1] for p in points]
    width = max(math.sqrt((points[1][0] - points[0][0]) ** 2 + (points[1][1] - points[0][1]) ** 2 + (points[1][2] - points[0][2]) ** 2), 0.01)
    depth = max(math.sqrt((points[3][0] - points[0][0]) ** 2 + (points[3][1] - points[0][1]) ** 2 + (points[3][2] - points[0][2]) ** 2), 0.01)
    uvs = [(0.0, 0.0), (width * uv_scale, 0.0), (width * uv_scale, depth * uv_scale), (0.0, depth * uv_scale)]
    base = len(part.positions)
    for point, uv in zip(points, uvs):
        part.positions.append(tuple(float(v) for v in point))
        part.normals.append(normal)
        part.uvs.append(uv)
    part.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3])


def _triangle(part: Part, points, uv_scale: float = 1.0) -> None:
    a, b, c = points
    ab = (b[0] - a[0], b[1] - a[1], b[2] - a[2])
    ac = (c[0] - a[0], c[1] - a[1], c[2] - a[2])
    normal = (
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    )
    length = math.sqrt(sum(v * v for v in normal)) or 1.0
    normal = tuple(v / length for v in normal)
    base = len(part.positions)
    uv = [(0.0, 0.0), (uv_scale, 0.0), (uv_scale * 0.5, uv_scale)]
    for point, texcoord in zip(points, uv):
        part.positions.append(tuple(float(v) for v in point))
        part.normals.append(normal)
        part.uvs.append(texcoord)
    part.indices.extend([base, base + 1, base + 2])


def _house_parts(_size: float = 1.0, hall: bool = False):
    wood = _asset("wood_planks", "concrete_wall_003")
    roof = _asset("roof_tiles", "rusty_metal_02")
    stone = _asset("cobblestone_floor_001", "concrete_floor_damaged_01")
    materials = [
        _make_material(wood, (0.72, 0.66, 0.55), roughness=0.92),
        _make_material(roof, (0.48, 0.42, 0.37), metallic=0.02, roughness=0.82),
        _make_material("rusty_metal_02", (0.12, 0.13, 0.13), metallic=0.12, roughness=0.67),
        _make_material(stone, (0.53, 0.55, 0.51), metallic=0.0, roughness=0.9),
    ]
    wall_h = 4.2 if hall else 3.25
    half_x = 5.2 if hall else 3.0
    half_z = 4.3 if hall else 3.0
    ridge_y = wall_h + (2.0 if hall else 1.45)
    eave_y = wall_h - 0.08
    parts = [Part(0), Part(1), Part(2), Part(3)]
    parts[3].box((0.0, 0.18, 0.0), (half_x * 2.10, 0.36, half_z * 2.08), 0.55)
    parts[0].box((0.0, wall_h / 2.0 + 0.32, 0.0), (half_x * 2.0, wall_h, half_z * 2.0), 0.50)

    # Pitched roof: two thick textured panels whose slope is baked into the vertices.
    roof_span = half_x + (0.85 if hall else 0.65)
    panel_width = roof_span
    panel_depth = half_z * 2.22
    panel_y = (ridge_y + eave_y) * 0.5
    panel_x = roof_span * 0.5
    left_panel = Part(1)
    left_panel.box((0.0, 0.0, 0.0), (panel_width, 0.20, panel_depth), 0.65)
    _rotate_part_z(left_panel, math.atan2(ridge_y - eave_y, roof_span), (-panel_x, panel_y, 0.0))
    # The panel's local center is transformed, and the rotation is centered at that origin.
    right_panel = Part(1)
    right_panel.box((0.0, 0.0, 0.0), (panel_width, 0.20, panel_depth), 0.65)
    _rotate_part_z(right_panel, -math.atan2(ridge_y - eave_y, roof_span), (panel_x, panel_y, 0.0))
    parts[1].positions.extend(left_panel.positions); parts[1].normals.extend(left_panel.normals); parts[1].uvs.extend(left_panel.uvs); parts[1].indices.extend([i for i in left_panel.indices])
    base = len(parts[1].positions) - len(right_panel.positions)
    parts[1].positions.extend(right_panel.positions); parts[1].normals.extend(right_panel.normals); parts[1].uvs.extend(right_panel.uvs); parts[1].indices.extend([base + i for i in right_panel.indices])

    # Gable ends use textured wood behind the roof panels.
    for z in (-half_z, half_z):
        _triangle(parts[0], [(-half_x, eave_y, z), (half_x, eave_y, z), (0.0, ridge_y, z)], 1.8 if hall else 1.0)
        # Add the rear-facing triangle too, so the gables are visible from both streets.
        _triangle(parts[0], [(half_x, eave_y, z + (0.035 if z < 0 else -0.035)), (-half_x, eave_y, z + (0.035 if z < 0 else -0.035)), (0.0, ridge_y, z + (0.035 if z < 0 else -0.035))], 1.8 if hall else 1.0)

    # Windows and frames on the two gable ends, with dark recesses and wet wooden trim.
    front_z = -half_z - 0.075
    window_y = wall_h * 0.68
    window_w = 0.85 if not hall else 1.25
    window_h = 1.0 if not hall else 1.35
    for x in (-half_x * 0.56, half_x * 0.56):
        parts[2].box((x, window_y, front_z), (window_w, window_h, 0.12), 0.9)
        frame_x = x - window_w * 0.5 - 0.07
        for fx in (frame_x, x + window_w * 0.5 + 0.07):
            parts[0].box((fx, window_y, front_z - 0.08), (0.12, window_h + 0.20, 0.13), 0.8)
        for fy in (window_y - window_h * 0.5 - 0.07, window_y + window_h * 0.5 + 0.07):
            parts[0].box((x, fy, front_z - 0.08), (window_w + 0.25, 0.12, 0.13), 0.8)
        parts[0].box((x, window_y, front_z - 0.09), (0.08, window_h, 0.14), 0.8)
        parts[0].box((x, window_y, front_z - 0.09), (window_w, 0.08, 0.14), 0.8)

    # Front door with a visible dark inset and brass-like handle.
    door_y = 1.10 if not hall else 1.35
    door_h = 2.20 if not hall else 2.70
    parts[2].box((0.0, door_y, front_z - 0.01), (1.05, door_h, 0.14), 1.0)
    parts[3].box((0.0, 0.22, front_z - 0.13), (1.30, 0.38, 0.20), 0.8)
    parts[2].box((0.36, door_y, front_z - 0.10), (0.055, 0.12, 0.055), 1.0)

    # Cross beams and corner timbers break up broad walls and read at walking distance.
    for x in (-half_x + 0.20, half_x - 0.20):
        parts[2].box((x, wall_h * 0.55, -half_z - 0.08), (0.20, wall_h + 0.15, 0.15), 0.8)
        parts[2].box((x, wall_h * 0.55, half_z + 0.08), (0.20, wall_h + 0.15, 0.15), 0.8)
    parts[2].box((0.0, wall_h + 0.05, -half_z - 0.08), (half_x * 1.95, 0.18, 0.14), 0.9)

    if hall:
        # Bell-tower-like square landmark and roof to break the repeated roof line.
        tower_y = ridge_y + 0.72
        parts[0].box((0.0, ridge_y + 0.48, 0.0), (1.7, 1.15, 1.7), 0.7)
        parts[2].box((0.0, ridge_y + 0.60, -0.90), (0.75, 0.52, 0.12), 0.8)
        tower_roof = Part(1)
        tower_roof.box((0.0, 0.0, 0.0), (2.3, 0.16, 2.3), 0.9)
        _rotate_part_z(tower_roof, 0.48, (0.0, tower_y, 0.0))
        base = len(parts[1].positions)
        parts[1].positions.extend(tower_roof.positions); parts[1].normals.extend(tower_roof.normals); parts[1].uvs.extend(tower_roof.uvs); parts[1].indices.extend([base + i for i in tower_roof.indices])
    return parts, materials


def _tree_parts():
    trunk_asset = _asset("wood_planks", "rusty_metal_02")
    foliage_asset = _asset("wood_planks", "concrete_wall_003")
    materials = [
        _make_material(trunk_asset, (0.48, 0.31, 0.19), metallic=0.0, roughness=0.94),
        _make_material(foliage_asset, (0.19, 0.28, 0.16), metallic=0.0, roughness=0.97),
    ]
    trunk, leaves = Part(0), Part(1)
    trunk.cylinder((0.0, 1.4, 0.0), 0.24, 2.8, 12)
    leaves.ellipsoid((0.0, 3.3, 0.0), (1.25, 1.40, 1.1), 14, 9)
    leaves.ellipsoid((-0.68, 3.05, 0.04), (0.82, 0.95, 0.88), 12, 8)
    leaves.ellipsoid((0.62, 3.05, -0.06), (0.84, 0.97, 0.91), 12, 8)
    return [trunk, leaves], materials


def _ground_parts():
    ground_asset = _asset("concrete_floor_damaged_01", "concrete_floor_damaged_01")
    materials = [_make_material(ground_asset, (0.52, 0.56, 0.43), metallic=0.0, roughness=0.93)]
    part = Part(0)
    part.box((0.0, -0.13, 0.0), (260.0, 0.26, 260.0), 0.14)
    return [part], materials


def _road_parts():
    road_asset = _asset("cobblestone_floor_001", "concrete_floor_damaged_01")
    materials = [_make_material(road_asset, (0.43, 0.44, 0.43), metallic=0.0, roughness=0.42)]
    part = Part(0)
    part.box((0.0, -0.035, 0.0), (230.0, 0.07, 10.0), 0.22)
    return [part], materials


def _lamp_parts():
    metal_asset = "rusty_metal_02"
    dark = _make_material(metal_asset, (0.075, 0.085, 0.09), metallic=0.25, roughness=0.62)
    glow = _make_material(metal_asset, (0.88, 0.56, 0.23), metallic=0.0, roughness=0.3, emissive=(1.8, 0.82, 0.25))
    post, light = Part(0), Part(1)
    post.cylinder((0.0, 1.55, 0.0), 0.09, 3.1, 10)
    post.box((0.0, 3.02, 0.0), (0.48, 0.10, 0.48), 0.7)
    post.box((0.0, 3.45, 0.0), (0.38, 0.08, 0.38), 0.7)
    for x in (-0.17, 0.17):
        for z in (-0.17, 0.17):
            post.box((x, 3.23, z), (0.045, 0.36, 0.045), 0.7)
    light.box((0.0, 3.23, 0.0), (0.22, 0.28, 0.22), 0.7)
    return [post, light], [dark, glow]


def _well_parts():
    stone_asset = _asset("cobblestone_floor_001", "concrete_floor_damaged_01")
    materials = [_make_material(stone_asset, (0.58, 0.60, 0.57), roughness=0.88), _make_material("rusty_metal_02", (0.12, 0.12, 0.11), metallic=0.15, roughness=0.9)]
    stone, wood = Part(0), Part(1)
    for i in range(12):
        angle = math.tau * i / 12
        x, z = math.cos(angle) * 1.0, math.sin(angle) * 1.0
        stone.box((x, 0.40, z), (0.64, 0.72, 0.62), 0.8)
    wood.box((0.0, 1.65, 0.0), (0.14, 2.1, 0.14), 0.9)
    wood.box((0.0, 2.55, 0.0), (2.5, 0.14, 0.14), 0.9)
    return [stone, wood], materials


def _append_credits_and_manifest(manifest: dict) -> None:
    # Keep the three dedicated village materials represented in the manifest even when their
    # downloaded source maps are not present on the machine running this generator.
    sources = {item.get("url") for item in manifest.get("sources", [])}
    for name in ("cobblestone_floor_001", "roof_tiles", "wood_planks"):
        if POLYHAVEN[name]["url"] not in sources:
            meta = POLYHAVEN[name]
            manifest.setdefault("sources", []).append({
                "asset": name,
                "title": meta["title"],
                "url": meta["url"],
                "creator": meta["creator"],
                "license": meta["license"],
                "maps": ["diff", "arm", "nor_gl"],
                "status": "DOWNLOAD_OPTIONAL_FALLBACK_USED" if not _has_downloaded_maps(name) else "DOWNLOADED",
            })
    out = MANIFEST

    credits_path = ASSET_ROOT / "CREDITS.md"
    text = credits_path.read_text(encoding="utf-8") if credits_path.exists() else "# Horror Slice asset credits\n\n"
    village_sources = ["cobblestone_floor_001", "roof_tiles", "wood_planks"]
    for name in village_sources:
        meta = POLYHAVEN[name]
        entry = next((item for item in manifest.get("sources", []) if item.get("url") == meta["url"]), None)
        if entry is not None:
            present_maps = sum(
                (SOURCE_ROOT / f"{name}_{suffix}_1k.jpg").is_file()
                for suffix in ("diff", "arm", "nor_gl")
            )
            entry["maps"] = ["diff", "arm", "nor_gl"]
            entry["maps_present"] = present_maps
            entry["status"] = "DOWNLOADED" if present_maps == 3 else "OPTIONAL_MAPS_UNAVAILABLE"
        heading = f"## {meta['title']}"
        if heading not in text:
            text += f"{heading}\n- Source: {meta['url']}\n- Creator: {meta['creator']}\n- License: {meta['license']}\n- Intended use: village road/stone, roof, or timber texture when the dedicated source maps are available.\n\n"
    text += "## Village model geometry\n- Generated by `games/athar/tools/horror_village_assets.py` from deterministic procedural geometry.\n- Textures are embedded in the GLB files. If dedicated village maps are unavailable, the generator reuses already bundled CC0 Poly Haven maps; any procedural fallback maps remain explicitly marked `PLACEHOLDER` in `asset_manifest.json`.\n"
    credits_path.write_text(text, encoding="utf-8")
    out.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


def generate_village_assets() -> None:
    MODEL_ROOT.mkdir(parents=True, exist_ok=True)
    if not MANIFEST.exists():
        raise RuntimeError("asset_manifest.json is missing; run python games/athar/tools/horror_room_assets.py first")
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    for name, builder in (
        ("village_house.glb", lambda: _house_parts()),
        ("village_hall.glb", lambda: _house_parts(1.0, hall=True)),
        ("village_tree.glb", _tree_parts),
        ("village_ground.glb", _ground_parts),
        ("village_road.glb", _road_parts),
        ("village_lamp.glb", _lamp_parts),
        ("village_well.glb", _well_parts),
    ):
        path = MODEL_ROOT / name
        # Regenerate each village model deterministically so updated downloaded maps are embedded.
        parts, materials = builder()
        _write_glb(path, parts, materials, manifest)
    # Deduplicate map entries produced by each model writer while retaining model fingerprints.
    records = {}
    for record in manifest.get("asset_records", []):
        key = (record.get("path"), record.get("map"), record.get("asset"), record.get("sha256"))
        records[key] = record
    manifest["asset_records"] = list(records.values())
    model_records = {}
    for record in manifest.get("models", []):
        model_records[record.get("path")] = record
    manifest["models"] = list(model_records.values())
    downloaded_village = [
        SOURCE_ROOT / f"{name}_{suffix}_1k.jpg"
        for name in ("cobblestone_floor_001", "roof_tiles", "wood_planks")
        for suffix in ("diff", "arm", "nor_gl")
    ]
    present = sum(path.is_file() for path in downloaded_village)
    manifest["village_textures"] = {
        "dedicated_source_maps_present": present,
        "dedicated_source_maps_expected": len(downloaded_village),
        "fallback_policy": "Reuse bundled downloaded CC0 concrete/floor/metal maps when dedicated village maps are unavailable; procedural maps are explicitly marked PLACEHOLDER.",
    }
    original_room_maps = sum(
        (SOURCE_ROOT / f"{name}_{suffix}_1k.jpg").is_file()
        for name in ("concrete_wall_003", "concrete_floor_damaged_01", "rusty_metal_02")
        for suffix in ("diff", "arm", "nor_gl")
    )
    if original_room_maps == 9 and present == 9:
        manifest["notice"] = "All 18 planned 1K source maps were downloaded from the official Poly Haven API and embedded in self-contained GLBs; generated PLACEHOLDER maps remain marked for fallback/testing."
    elif original_room_maps:
        manifest["notice"] = (f"Downloaded {original_room_maps}/9 original-room 1K source maps and {present}/9 dedicated "
                               "village maps from the official Poly Haven API. Village models reuse bundled CC0 room maps "
                               "when dedicated maps are unavailable; generated maps are marked PLACEHOLDER.")
    else:
        manifest["notice"] = (f"Downloaded {original_room_maps}/9 original-room 1K source maps and {present}/9 dedicated "
                               "village maps from the official Poly Haven API. Generated maps are marked PLACEHOLDER; "
                               "run games/athar/tools/fetch_horror_textures.py then games/athar/tools/horror_village_assets.py to embed CC0 maps.")
    _append_credits_and_manifest(manifest)
    print(f"generated 7 self-contained village GLBs; dedicated village maps available: {present}/9")


if __name__ == "__main__":
    generate_village_assets()
