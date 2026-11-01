#!/usr/bin/env python3
"""Regenerate engine test glTF fixtures under assets/models/ (standard library only).

Game-specific asset generation belongs beside the game that owns those assets.
For Athar, use ``python games/athar/tools/horror_room_assets.py`` instead.
"""

import json
import struct
import zlib
from pathlib import Path

OUT = Path(__file__).resolve().parents[2] / "assets" / "models"


def png(width, height, pixel):
    rows = b"".join(
        b"\x00" + b"".join(bytes(pixel(x, y)) for x in range(width)) for y in range(height)
    )

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(rows))
        + chunk(b"IEND", b"")
    )


def pad(data, fill):
    return data + fill * (-len(data) % 4)


def glb(document, binary):
    json_chunk = pad(json.dumps(document, separators=(",", ":")).encode(), b" ")
    bin_chunk = pad(binary, b"\x00")
    total = 12 + 8 + len(json_chunk) + 8 + len(bin_chunk)
    return (
        struct.pack("<4sII", b"glTF", 2, total)
        + struct.pack("<I4s", len(json_chunk), b"JSON")
        + json_chunk
        + struct.pack("<I4s", len(bin_chunk), b"BIN\x00")
        + bin_chunk
    )


def cube():
    faces = [
        ((1, 0, 0), (0, 1, 0), (0, 0, 1)),
        ((-1, 0, 0), (0, 0, 1), (0, 1, 0)),
        ((0, 1, 0), (0, 0, 1), (1, 0, 0)),
        ((0, -1, 0), (1, 0, 0), (0, 0, 1)),
        ((0, 0, 1), (1, 0, 0), (0, 1, 0)),
        ((0, 0, -1), (0, 1, 0), (1, 0, 0)),
    ]
    corners = [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)]
    positions, normals, uvs, indices = [], [], [], []
    for normal, u, v in faces:
        base = len(positions)
        for cu, cv in corners:
            positions.append(tuple(normal[i] * 0.5 + u[i] * cu + v[i] * cv for i in range(3)))
            normals.append(normal)
            uvs.append((cu + 0.5, 0.5 - cv))
        indices += [base, base + 1, base + 2, base, base + 2, base + 3]
    return positions, normals, uvs, indices


def checker_cube():
    positions, normals, uvs, indices = cube()
    texture = png(8, 8, lambda x, y: (230, 120, 30) if (x // 2 + y // 2) % 2 == 0 else (30, 60, 160))
    chunks, views = [], []

    def add(data, target=None):
        offset = sum(len(c) for c in chunks)
        entry = {"buffer": 0, "byteOffset": offset, "byteLength": len(data)}
        if target:
            entry["target"] = target
        views.append(entry)
        chunks.append(pad(data, b"\x00"))

    add(b"".join(struct.pack("<3f", *p) for p in positions), 34962)
    add(b"".join(struct.pack("<3f", *n) for n in normals), 34962)
    add(b"".join(struct.pack("<2f", *t) for t in uvs), 34962)
    add(b"".join(struct.pack("<H", i) for i in indices), 34963)
    add(texture)
    document = {
        "asset": {"version": "2.0", "generator": "tools/assets/make_test_models.py"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [
            {"children": [1], "translation": [0, 0.5, 0]},
            {"mesh": 0, "translation": [1.5, 0, 0], "scale": [0.8, 0.8, 0.8]},
        ],
        "meshes": [
            {"primitives": [{"attributes": {"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2}, "indices": 3, "material": 0}]}
        ],
        "materials": [
            {"pbrMetallicRoughness": {"baseColorFactor": [1, 1, 1, 1], "baseColorTexture": {"index": 0}}}
        ],
        "textures": [{"source": 0}],
        "images": [{"bufferView": 4, "mimeType": "image/png"}],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 24, "type": "VEC3", "min": [-0.5] * 3, "max": [0.5] * 3},
            {"bufferView": 1, "componentType": 5126, "count": 24, "type": "VEC3"},
            {"bufferView": 2, "componentType": 5126, "count": 24, "type": "VEC2"},
            {"bufferView": 3, "componentType": 5123, "count": 36, "type": "SCALAR"},
        ],
        "bufferViews": views,
        "buffers": [{"byteLength": sum(len(c) for c in chunks)}],
    }
    return glb(document, b"".join(chunks))


def bare_triangle():
    data = b"".join(struct.pack("<3f", *p) for p in [(0, 0, 0), (1, 0, 0), (0, 1, 0)])
    document = {
        "asset": {"version": "2.0", "generator": "tools/assets/make_test_models.py"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0]}
        ],
        "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": len(data)}],
        "buffers": [{"byteLength": len(data)}],
    }
    return glb(document, data)


if __name__ == "__main__":
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "checker_cube.glb").write_bytes(checker_cube())
    (OUT / "bare_triangle.glb").write_bytes(bare_triangle())
    print("wrote", sorted(p.name for p in OUT.iterdir()))
