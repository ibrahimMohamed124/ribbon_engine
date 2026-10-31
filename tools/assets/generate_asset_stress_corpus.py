#!/usr/bin/env python3
"""Generate deterministic, distinct self-contained glTF models for asset-cache stress runs.

Uses only the Python standard library. Outputs are written under target/ by default, never to the
checked-in assets directory, and can be regenerated safely before a profiling capture.
"""

from __future__ import annotations

import argparse
import json
import struct
import zlib
from pathlib import Path

MAX_MODELS = 512


def png_chunk(kind: bytes, payload: bytes) -> bytes:
    body = kind + payload
    return (
        struct.pack(">I", len(payload))
        + body
        + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)
    )


def encode_png(width: int, height: int, index: int) -> bytes:
    # Odd multipliers modulo 256 keep the first channel unique across the full 0..255 corpus.
    base = (
        (31 + index * 71) % 256,
        (19 + index * 43) % 256,
        (53 + index * 97) % 256,
    )
    accent = tuple(255 - value for value in base)
    rows = bytearray()
    for y in range(height):
        rows.append(0)  # PNG filter: None.
        for x in range(width):
            checker = ((x // max(1, width // 8)) + (y // max(1, height // 8))) % 2
            color = base if checker == 0 else accent
            # Add a small deterministic diagonal stripe to keep every generated texture distinct.
            stripe = (x * 3 + y * 5 + index * 11) % max(8, width // 2) < 2
            if stripe:
                color = (color[1], color[2], color[0])
            rows.extend(color)
    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + png_chunk(b"IHDR", header)
        + png_chunk(b"IDAT", zlib.compress(bytes(rows), level=6))
        + png_chunk(b"IEND", b"")
    )


def pad4(data: bytes, fill: bytes = b"\x00") -> bytes:
    return data + fill * (-len(data) % 4)


def pack_glb(document: dict[str, object], binary: bytes) -> bytes:
    json_chunk = pad4(json.dumps(document, separators=(",", ":")).encode("utf-8"), b" ")
    binary_chunk = pad4(binary)
    total_length = 12 + 8 + len(json_chunk) + 8 + len(binary_chunk)
    return (
        struct.pack("<4sII", b"glTF", 2, total_length)
        + struct.pack("<I4s", len(json_chunk), b"JSON")
        + json_chunk
        + struct.pack("<I4s", len(binary_chunk), b"BIN\x00")
        + binary_chunk
    )


def cube_geometry(index: int) -> tuple[list[tuple[float, float, float]], list[tuple[float, float, float]], list[tuple[float, float]], list[int]]:
    faces = [
        ((1, 0, 0), (0, 1, 0), (0, 0, 1)),
        ((-1, 0, 0), (0, 0, 1), (0, 1, 0)),
        ((0, 1, 0), (0, 0, 1), (1, 0, 0)),
        ((0, -1, 0), (1, 0, 0), (0, 0, 1)),
        ((0, 0, 1), (1, 0, 0), (0, 1, 0)),
        ((0, 0, -1), (0, 1, 0), (1, 0, 0)),
    ]
    corners = [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)]
    axis_scale = (
        0.62 + (index % 7) * 0.105,
        0.68 + ((index * 3) % 9) * 0.075,
        0.64 + ((index * 5) % 8) * 0.09,
    )
    positions: list[tuple[float, float, float]] = []
    normals: list[tuple[float, float, float]] = []
    uvs: list[tuple[float, float]] = []
    indices: list[int] = []
    for normal, u_axis, v_axis in faces:
        base = len(positions)
        for corner_u, corner_v in corners:
            point = tuple(
                (normal[axis] * 0.5 + u_axis[axis] * corner_u + v_axis[axis] * corner_v)
                * axis_scale[axis]
                for axis in range(3)
            )
            positions.append(point)
            normals.append(tuple(float(component) for component in normal))
            uvs.append((corner_u + 0.5, 0.5 - corner_v))
        indices.extend((base, base + 1, base + 2, base, base + 2, base + 3))
    return positions, normals, uvs, indices


def make_model(index: int, texture_size: int) -> bytes:
    positions, normals, uvs, indices = cube_geometry(index)
    texture = encode_png(texture_size, texture_size, index)
    chunks: list[bytes] = []
    buffer_views: list[dict[str, int]] = []

    def add_view(raw: bytes, target: int | None = None) -> int:
        current = pad4(b"".join(chunks))
        if current:
            previous_length = sum(len(chunk) for chunk in chunks)
            padding = len(current) - previous_length
            if padding:
                chunks.append(b"\x00" * padding)
        offset = sum(len(chunk) for chunk in chunks)
        chunks.append(raw)
        view: dict[str, int] = {
            "buffer": 0,
            "byteOffset": offset,
            "byteLength": len(raw),
        }
        if target is not None:
            view["target"] = target
        buffer_views.append(view)
        return len(buffer_views) - 1

    position_view = add_view(b"".join(struct.pack("<3f", *point) for point in positions), 34962)
    normal_view = add_view(b"".join(struct.pack("<3f", *normal) for normal in normals), 34962)
    uv_view = add_view(b"".join(struct.pack("<2f", *uv) for uv in uvs), 34962)
    index_view = add_view(b"".join(struct.pack("<H", value) for value in indices), 34963)
    image_view = add_view(texture)
    binary = b"".join(chunks)

    color = [
        ((37 + index * 41) % 180 + 60) / 255.0,
        ((53 + index * 73) % 180 + 60) / 255.0,
        ((71 + index * 29) % 180 + 60) / 255.0,
        1.0,
    ]
    minimum = [min(point[axis] for point in positions) for axis in range(3)]
    maximum = [max(point[axis] for point in positions) for axis in range(3)]
    document: dict[str, object] = {
        "asset": {"version": "2.0", "generator": "tools/assets/generate_asset_stress_corpus.py"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0, "translation": [0.0, 0.5, 0.0]}],
        "meshes": [{"primitives": [{
            "attributes": {"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2},
            "indices": 3,
            "material": 0,
        }]}],
        "materials": [{"pbrMetallicRoughness": {
            "baseColorFactor": color,
            "metallicFactor": ((index * 13) % 100) / 100.0,
            "roughnessFactor": 0.12 + ((index * 17) % 80) / 100.0,
            "baseColorTexture": {"index": 0},
        }}],
        "textures": [{"source": 0}],
        "images": [{"bufferView": image_view, "mimeType": "image/png"}],
        "accessors": [
            {"bufferView": position_view, "componentType": 5126, "count": len(positions), "type": "VEC3", "min": minimum, "max": maximum},
            {"bufferView": normal_view, "componentType": 5126, "count": len(normals), "type": "VEC3"},
            {"bufferView": uv_view, "componentType": 5126, "count": len(uvs), "type": "VEC2"},
            {"bufferView": index_view, "componentType": 5123, "count": len(indices), "type": "SCALAR", "min": [min(indices)], "max": [max(indices)]},
        ],
        "bufferViews": buffer_views,
        "buffers": [{"byteLength": len(binary)}],
    }
    return pack_glb(document, binary)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--count", type=int, required=True, help=f"number of distinct assets (1..{MAX_MODELS})")
    parser.add_argument("--texture-size", type=int, default=128, help="square RGBA-converted source texture size (8..512, power of two)")
    parser.add_argument("--output-root", default="target/asset-stress-corpus", help="output root relative to the repository root unless absolute")
    args = parser.parse_args()
    if not 1 <= args.count <= MAX_MODELS:
        parser.error(f"--count must be between 1 and {MAX_MODELS}")
    if not 8 <= args.texture_size <= 512 or args.texture_size & (args.texture_size - 1):
        parser.error("--texture-size must be a power of two between 8 and 512")

    root = Path(__file__).resolve().parents[2]
    output_root = Path(args.output_root)
    if not output_root.is_absolute():
        output_root = root / output_root
    models_dir = output_root / "models"
    models_dir.mkdir(parents=True, exist_ok=True)
    for old in models_dir.glob("stress_[0-9][0-9][0-9][0-9].glb"):
        old.unlink()

    source_bytes = 0
    for index in range(args.count):
        output = models_dir / f"stress_{index:04}.glb"
        content = make_model(index, args.texture_size)
        output.write_bytes(content)
        source_bytes += len(content)

    manifest = [
        "# Generated Multi-Asset Stress Corpus",
        "",
        f"- Distinct model count: {args.count}",
        f"- Source texture size: {args.texture_size} x {args.texture_size}",
        f"- GLB source bytes: {source_bytes:,}",
        f"- Approximate decoded texture payload alone: {args.count * args.texture_size * args.texture_size * 4:,} bytes, before geometry/material/container overhead",
        "- Files are generated under target/ and are not checked-in game assets.",
        "- Each model has distinct vertex dimensions, material factors, and texture content.",
    ]
    (output_root / "asset-corpus-manifest.md").write_text("\n".join(manifest) + "\n", encoding="utf-8")
    print(f"generated {args.count} distinct GLB assets in {models_dir}")
    print(f"compressed source bytes: {source_bytes:,}; decoded texture payload floor: {args.count * args.texture_size * args.texture_size * 4:,} bytes")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
