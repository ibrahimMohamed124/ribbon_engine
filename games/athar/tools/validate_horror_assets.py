#!/usr/bin/env python3
"""Validate self-contained Athar GLBs and manifest hashes using only Python's stdlib."""
from __future__ import annotations

import hashlib
import json
import re
import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
GAME_ROOT = ROOT / "games" / "athar"
ASSET_ROOT = GAME_ROOT / "assets"
MODEL_ROOT = ASSET_ROOT / "models"
VILLAGE_TEXTURE_ASSETS = ("cobblestone_floor_001", "roof_tiles", "wood_planks")
ROOM_TEXTURE_ASSETS = ("concrete_wall_003", "concrete_floor_damaged_01", "rusty_metal_02")


def validate_glb(path: Path) -> tuple[dict, int]:
    data = path.read_bytes()
    if len(data) < 20:
        raise ValueError(f"{path.name}: file is too short to be GLB")
    magic, version, declared_length = struct.unpack_from("<4sII", data, 0)
    if magic != b"glTF" or version != 2 or declared_length != len(data):
        raise ValueError(f"{path.name}: invalid GLB header/length")

    cursor = 12
    json_length, json_kind = struct.unpack_from("<I4s", data, cursor)
    cursor += 8
    if json_kind != b"JSON" or cursor + json_length > len(data):
        raise ValueError(f"{path.name}: JSON chunk is missing or truncated")
    document = json.loads(data[cursor : cursor + json_length].decode("utf-8").rstrip(" \t\r\n\x00"))
    cursor += json_length
    if cursor + 8 > len(data):
        raise ValueError(f"{path.name}: BIN chunk header is missing")
    binary_length, binary_kind = struct.unpack_from("<I4s", data, cursor)
    cursor += 8
    if binary_kind != b"BIN\x00" or cursor + binary_length > len(data):
        raise ValueError(f"{path.name}: BIN chunk is missing or truncated")
    binary = data[cursor : cursor + binary_length]

    buffers = document.get("buffers", [])
    if len(buffers) != 1 or buffers[0].get("byteLength", binary_length) > binary_length:
        raise ValueError(f"{path.name}: expected one valid embedded buffer")
    views = document.get("bufferViews", [])
    for index, view in enumerate(views):
        if view.get("buffer", 0) != 0:
            raise ValueError(f"{path.name}: bufferView {index} references a non-embedded buffer")
        start = view.get("byteOffset", 0)
        if start < 0 or view.get("byteLength", 0) < 0 or start + view.get("byteLength", 0) > len(binary):
            raise ValueError(f"{path.name}: bufferView {index} is out of bounds")
    accessors = document.get("accessors", [])
    for index, accessor in enumerate(accessors):
        view_index = accessor.get("bufferView")
        if not isinstance(view_index, int) or not (0 <= view_index < len(views)):
            raise ValueError(f"{path.name}: accessor {index} has no valid bufferView")
        if accessor.get("count", 0) <= 0:
            raise ValueError(f"{path.name}: accessor {index} is empty")
    for index, image in enumerate(document.get("images", [])):
        if "uri" in image or not isinstance(image.get("bufferView"), int):
            raise ValueError(f"{path.name}: image {index} is not embedded in the GLB")
        if image["bufferView"] < 0 or image["bufferView"] >= len(views):
            raise ValueError(f"{path.name}: image {index} references an invalid bufferView")
        mime_type = image.get("mimeType")
        if mime_type not in ("image/png", "image/jpeg"):
            raise ValueError(f"{path.name}: image {index} has an unsupported MIME type")
        view = views[image["bufferView"]]
        start = view.get("byteOffset", 0)
        payload = binary[start : start + view.get("byteLength", 0)]
        if mime_type == "image/jpeg" and not payload.startswith(b"\xff\xd8\xff"):
            raise ValueError(f"{path.name}: image {index} declares JPEG but has invalid image bytes")
        if mime_type == "image/png" and not payload.startswith(b"\x89PNG\r\n\x1a\n"):
            raise ValueError(f"{path.name}: image {index} declares PNG but has invalid image bytes")
    primitives = [primitive for mesh in document.get("meshes", []) for primitive in mesh.get("primitives", [])]
    if not primitives:
        raise ValueError(f"{path.name}: no mesh primitives found")
    textures = document.get("textures", [])
    for index, texture in enumerate(textures):
        source = texture.get("source")
        if not isinstance(source, int) or not (0 <= source < len(document.get("images", []))):
            raise ValueError(f"{path.name}: texture {index} has no valid embedded image")
    for index, material in enumerate(document.get("materials", [])):
        pbr = material.get("pbrMetallicRoughness", {})
        texture_refs = [
            pbr.get("baseColorTexture"),
            pbr.get("metallicRoughnessTexture"),
            material.get("normalTexture"),
            material.get("occlusionTexture"),
            material.get("emissiveTexture"),
        ]
        for texture_info in filter(None, texture_refs):
            texture_index = texture_info.get("index")
            if not isinstance(texture_index, int) or not (0 <= texture_index < len(textures)):
                raise ValueError(f"{path.name}: material {index} references an invalid texture")
    for primitive in primitives:
        attributes = primitive.get("attributes", {})
        if not all(key in attributes for key in ("POSITION", "NORMAL", "TEXCOORD_0")):
            raise ValueError(f"{path.name}: primitive is missing required vertex attributes")
        if "indices" not in primitive:
            raise ValueError(f"{path.name}: primitive has no index accessor")
        index_accessor = primitive["indices"]
        if not isinstance(index_accessor, int) or not (0 <= index_accessor < len(accessors)):
            raise ValueError(f"{path.name}: primitive index accessor is invalid")
        for semantic, accessor_index in attributes.items():
            if not isinstance(accessor_index, int) or not (0 <= accessor_index < len(accessors)):
                raise ValueError(f"{path.name}: primitive attribute {semantic} has an invalid accessor")
    return document, len(primitives)


def main() -> int:
    manifest_path = ASSET_ROOT / "asset_manifest.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    model_records = manifest.get("models", [])
    model_hashes = {record.get("path"): record.get("sha256") for record in model_records}
    total_bytes = 0
    try:
        if not model_records:
            raise ValueError("manifest has no model records")
        room_text = (ASSET_ROOT / "room.ron").read_text(encoding="utf-8")
        room_assets = set(re.findall(r'asset\s*:\s*"([^"]+\.glb)"', room_text))
        for filename in room_assets:
            if not (MODEL_ROOT / filename).is_file():
                raise ValueError(f"room.ron references a missing model: {filename}")

        for record in model_records:
            relative = record.get("path")
            path = ROOT / relative if isinstance(relative, str) else None
            if path is None or path.parent != MODEL_ROOT or path.suffix.lower() != ".glb":
                raise ValueError(f"manifest has an invalid model path: {relative!r}")
            if not path.is_file():
                raise ValueError(f"missing model: {relative}")
            digest = hashlib.sha256(path.read_bytes()).hexdigest()
            expected = model_hashes.get(path.relative_to(ROOT).as_posix())
            if not expected or expected != digest:
                raise ValueError(f"{path.name}: asset_manifest.json hash does not match the GLB")
            document, primitive_count = validate_glb(path)
            total_bytes += path.stat().st_size
            print(
                f"OK {path.name}: {primitive_count} primitives, "
                f"{len(document.get('materials', []))} materials, "
                f"{len(document.get('images', []))} embedded images, sha256={digest[:12]}"
            )

        placeholder_records = manifest.get("textures", [])
        if not placeholder_records:
            raise ValueError("manifest has no explicit placeholder texture records")
        for record in placeholder_records:
            relative = record.get("path")
            path = ROOT / relative if isinstance(relative, str) else None
            if path is None or path.parent != (ASSET_ROOT / "textures" / "placeholders"):
                raise ValueError(f"manifest has an invalid placeholder texture path: {relative!r}")
            data = path.read_bytes()
            if record.get("status") != "PLACEHOLDER" or record.get("sha256") != hashlib.sha256(data).hexdigest():
                raise ValueError(f"placeholder metadata/hash mismatch: {relative}")

        source_records = manifest.get("source_maps", [])
        source_by_path = {item.get("path"): item for item in source_records}
        if len(source_by_path) != len(source_records):
            raise ValueError("manifest contains duplicate source-map path records")
        # Validate every indexed source map and any on-disk 1K JPEG, without requiring optional maps.
        for relative, record in source_by_path.items():
            path = ROOT / relative if isinstance(relative, str) else None
            if path is None or path.parent != (ASSET_ROOT / "textures" / "source"):
                raise ValueError(f"manifest has an invalid source-map path: {relative!r}")
            data = path.read_bytes()
            digest = hashlib.sha256(data).hexdigest()
            if not data.startswith(b"\xff\xd8") or record.get("sha256") != digest:
                raise ValueError(f"source map is not a valid matching JPEG: {relative}")
        source_root = ASSET_ROOT / "textures" / "source"
        for path in source_root.glob("*_1k.jpg"):
            data = path.read_bytes()
            if not data.startswith(b"\xff\xd8\xff"):
                raise ValueError(f"source map is not a valid JPEG: {path.relative_to(ROOT).as_posix()}")
        village_textures = manifest.get("village_textures", {})
        actual_present = sum(
            (source_root / f"{asset}_{kind}_1k.jpg").is_file()
            for asset in VILLAGE_TEXTURE_ASSETS
            for kind in ("diff", "arm", "nor_gl")
        )
        expected_maps = village_textures.get("dedicated_source_maps_expected")
        present_maps = village_textures.get("dedicated_source_maps_present")
        if village_textures and (expected_maps != 9 or present_maps != actual_present):
            raise ValueError(
                "village texture availability metadata is stale: "
                f"manifest says {present_maps}/{expected_maps}, files are {actual_present}/9"
            )
    except (OSError, ValueError, KeyError, json.JSONDecodeError, struct.error) as error:
        print(f"Horror asset validation failed: {error}", file=sys.stderr)
        return 1
    print(f"Validated {len(model_records)} self-contained Athar GLBs ({total_bytes / (1024 * 1024):.2f} MiB total).")
    print(f"Dedicated optional village maps: {actual_present}/9; existing room maps and embedded GLBs remain self-contained.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
