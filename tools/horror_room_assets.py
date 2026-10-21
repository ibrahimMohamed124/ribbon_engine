"""Generate deterministic, explicitly marked placeholder geometry/textures for horror_slice.

Run via `python tools/make_test_models.py`. If 1K Poly Haven maps exist in
examples/horror_slice/assets/textures/source, they are embedded in the generated GLBs.
This file uses only Python's standard library.
"""
from __future__ import annotations

import hashlib
import json
import math
import struct
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ASSET_ROOT = ROOT / "examples" / "horror_slice" / "assets"
TEXTURE_ROOT = ASSET_ROOT / "textures"
PLACEHOLDER_ROOT = TEXTURE_ROOT / "placeholders"
SOURCE_ROOT = TEXTURE_ROOT / "source"
MODEL_ROOT = ASSET_ROOT / "models"
WIDTH = HEIGHT = 256

POLYHAVEN = {
    "concrete_wall_003": {
        "title": "Concrete Wall 003", "url": "https://polyhaven.com/a/concrete_wall_003",
        "creator": "Dimitrios Savva (photography), Rico Cilliers (processing)",
        "license": "CC0 1.0 Universal", "base": (139, 144, 136), "kind": "wall",
    },
    "concrete_floor_damaged_01": {
        "title": "Concrete Floor Damaged 01", "url": "https://polyhaven.com/a/concrete_floor_damaged_01",
        "creator": "Rob Tuytel", "license": "CC0 1.0 Universal", "base": (105, 109, 102), "kind": "floor",
    },
    "rusty_metal_02": {
        "title": "Rusty Metal 02", "url": "https://polyhaven.com/a/rusty_metal_02",
        "creator": "Rob Tuytel", "license": "CC0 1.0 Universal", "base": (106, 87, 64), "kind": "metal",
    },
}


def _hash_noise(x: int, y: int, salt: int = 0) -> float:
    n = (x * 374761393 + y * 668265263 + salt * 1442695041) & 0xFFFFFFFF
    n = ((n ^ (n >> 13)) * 1274126177) & 0xFFFFFFFF
    n ^= n >> 16
    return (n & 0xFFFFFF) / 16777215.0


def _noise_pixels(asset: str, map_kind: str, width: int = WIDTH, height: int = HEIGHT):
    meta = POLYHAVEN[asset]
    base = meta["base"]
    style = meta["kind"]
    salt = len(asset)

    lattice_grids: dict[tuple[int, int, int], list[list[float]]] = {}

    def value_noise(u: float, v: float, cells_u: int, cells_v: int, seed: int) -> float:
        # Smooth periodic value noise: the tile edges meet without visible seams.
        # Cache seeded lattices once per octave; computing a hash for every texel was
        # needlessly slow when regenerating all nine 256x256 fallback maps.
        key = (cells_u, cells_v, seed)
        grid = lattice_grids.get(key)
        if grid is None:
            grid = [
                [_hash_noise(x, y, seed) for x in range(cells_u)]
                for y in range(cells_v)
            ]
            lattice_grids[key] = grid
        px, py = (u % 1.0) * cells_u, (v % 1.0) * cells_v
        ix, iy = math.floor(px), math.floor(py)
        fx, fy = px - ix, py - iy
        fx = fx * fx * (3.0 - 2.0 * fx)
        fy = fy * fy * (3.0 - 2.0 * fy)
        a = grid[iy][ix] * (1.0 - fx) + grid[iy][(ix + 1) % cells_u] * fx
        next_y = (iy + 1) % cells_v
        b = grid[next_y][ix] * (1.0 - fx) + grid[next_y][(ix + 1) % cells_u] * fx
        return (a * (1.0 - fy) + b * fy) - 0.5

    # Tileable polygon cracks are a better floor placeholder than regular sine-wave loops.
    floor_sites = []
    site_count = 18
    for index in range(site_count):
        floor_sites.append((
            _hash_noise(index, 11, salt * 13),
            _hash_noise(index, 23, salt * 19),
        ))

    def floor_edge_gap(u: float, v: float) -> float:
        nearest, second = 2.0, 2.0
        for sx, sy in floor_sites:
            du = abs(u - sx)
            dv = abs(v - sy)
            du = min(du, 1.0 - du)
            dv = min(dv, 1.0 - dv)
            distance = math.sqrt(du * du + dv * dv)
            if distance < nearest:
                second, nearest = nearest, distance
            elif distance < second:
                second = distance
        return second - nearest

    def pixel(x: int, y: int):
        u, v = x / width, y / height
        macro = value_noise(u, v, 7, 6, salt)
        mid = value_noise(u, v, 17, 13, salt * 7)
        fine = value_noise(u, v, 47, 39, salt * 17)
        damp = max(0.0, value_noise(u, v, 8, 5, salt * 29) + 0.20)

        if map_kind == "diff":
            # Keep albedo variation low-frequency and low contrast. Tiny high-contrast
            # flecks become unnaturally bright under the flashlight, especially on floors.
            shade = 0.94 + 0.12 * macro + 0.065 * mid + 0.018 * fine
            if style == "wall":
                # Broad damp patches and flaking plaster: avoid confetti-like white spots.
                peel = value_noise(u, v, 9, 13, salt * 31) + 0.18 * value_noise(u, v, 17, 21, salt * 37)
                if peel > 0.25:
                    shade *= 1.045
                elif peel < -0.34:
                    shade *= 0.91
                if damp > 0.32:
                    shade *= 0.95 - min(0.055, damp * 0.055)
            elif style == "floor":
                gap = floor_edge_gap(u, v)
                if gap < 0.0045:
                    shade *= 0.63
                elif gap < 0.010:
                    shade *= 0.84
                shade *= 0.95 + 0.05 * damp
            else:
                rust = max(0.0, value_noise(u, v, 12, 18, salt * 41) + 0.22)
                scratch = abs(value_noise(u, v, 49, 15, salt * 43))
                shade *= 0.86 + rust * 0.22
                if scratch < 0.012:
                    shade *= 0.93
            return tuple(max(0, min(255, int(c * shade))) for c in base)

        if map_kind == "normal":
            # Smooth, near-flat tangent-space normals. The earlier per-pixel hash
            # normal gave concrete an over-bumpy metallic sparkle in the flashlight.
            nx = value_noise(u, v, 43, 37, salt * 47)
            ny = value_noise(u, v, 41, 35, salt * 53)
            # Placeholder normal maps stay almost flat to avoid glittering highlights
            # from high-intensity small spotlights. Real downloaded maps retain full detail.
            strength = 6 if style == "metal" else (4 if style == "floor" else 3)
            return (
                int(128 + nx * strength),
                int(128 + ny * strength),
                253,
            )

        if map_kind == "arm":
            ao = 232 - int(damp * 17)
            roughness = 224 + int(mid * 22)
            if style == "metal":
                roughness = 202 + int(mid * 26)
                metal = 104 if mid > 0.15 else 30
            else:
                metal = 0
            return (
                max(0, min(255, ao)),
                max(0, min(255, roughness)),
                max(0, min(255, metal)),
            )

        if map_kind == "emissive":
            return (255, 255, 255)
        return (0, 0, 0)
    return pixel


def _png(width: int, height: int, pixel) -> bytes:
    def chunk(kind: bytes, data: bytes) -> bytes:
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))
    rows = b"".join(b"\x00" + b"".join(bytes(pixel(x, y)) for x in range(width)) for y in range(height))
    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(rows, 6)) + chunk(b"IEND", b"")


def _pad(data: bytes, fill: bytes) -> bytes:
    return data + fill * (-len(data) % 4)


def _add_blob(buffer: bytearray, views: list, data: bytes, target: int | None = None) -> int:
    buffer.extend(b"\0" * (-len(buffer) % 4))
    index = len(views)
    item = {"buffer": 0, "byteOffset": len(buffer), "byteLength": len(data)}
    if target is not None:
        item["target"] = target
    views.append(item)
    buffer.extend(data)
    return index


class Part:
    def __init__(self, material: int):
        self.material = material
        self.positions: list[tuple[float, float, float]] = []
        self.normals: list[tuple[float, float, float]] = []
        self.uvs: list[tuple[float, float]] = []
        self.indices: list[int] = []

    def quad(self, center, normal, u_axis, v_axis, u_size, v_size, uv_scale=1.0):
        base = len(self.positions)
        for cu, cv in [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)]:
            self.positions.append(tuple(center[i] + u_axis[i] * cu * u_size + v_axis[i] * cv * v_size for i in range(3)))
            self.normals.append(tuple(float(n) for n in normal))
            self.uvs.append(((cu + 0.5) * u_size * uv_scale, (0.5 - cv) * v_size * uv_scale))
        self.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3])

    def box(self, center, size, uv_scale=0.5):
        sx, sy, sz = size
        # Every (u,v) pair is right-handed with the corresponding outward face normal.
        faces = [
            ((1,0,0),(0,1,0),(0,0,1),sy,sz), ((-1,0,0),(0,0,1),(0,1,0),sz,sy),
            ((0,1,0),(0,0,1),(1,0,0),sz,sx), ((0,-1,0),(1,0,0),(0,0,1),sx,sz),
            ((0,0,1),(1,0,0),(0,1,0),sx,sy), ((0,0,-1),(0,1,0),(1,0,0),sy,sx),
        ]
        for normal, u_axis, v_axis, u_size, v_size in faces:
            sign = 1 if normal[0] > 0 or normal[1] > 0 or normal[2] > 0 else -1
            face_center = tuple(center[i] + normal[i] * size[i] * 0.5 for i in range(3))
            self.quad(face_center, normal, u_axis, v_axis, u_size, v_size, uv_scale)

    def ellipsoid(self, center, radii, segments=12, rings=8):
        base = len(self.positions)
        for iy in range(rings + 1):
            theta = math.pi * iy / rings
            sin_theta, cos_theta = math.sin(theta), math.cos(theta)
            for ix in range(segments + 1):
                phi = 2 * math.pi * ix / segments
                sin_phi, cos_phi = math.sin(phi), math.cos(phi)
                # Y-up ellipsoid: poles point up/down and the creature's torso remains vertical.
                n = (sin_theta * cos_phi, cos_theta, sin_theta * sin_phi)
                self.positions.append((center[0] + radii[0] * n[0], center[1] + radii[1] * n[1], center[2] + radii[2] * n[2]))
                norm = (n[0] / max(radii[0], 1e-4), n[1] / max(radii[1], 1e-4), n[2] / max(radii[2], 1e-4))
                ll = math.sqrt(sum(c*c for c in norm)) or 1.0
                self.normals.append(tuple(c / ll for c in norm))
                self.uvs.append((ix / segments, iy / rings))
        for iy in range(rings):
            for ix in range(segments):
                a = base + iy * (segments + 1) + ix
                b, c, d = a + 1, a + segments + 2, a + segments + 1
                # Sphere's generated normal/winding are consistent for the given angle convention.
                self.indices.extend([a, b, c, a, c, d])

    def cylinder(self, center, radius, height, segments=12):
        # Cylinder axis is Y; two caps and a smooth-sided wall.
        y0, y1 = center[1] - height / 2, center[1] + height / 2
        start = len(self.positions)
        for y in (y0, y1):
            for i in range(segments + 1):
                a = math.tau * i / segments
                self.positions.append((center[0] + math.cos(a) * radius, y, center[2] + math.sin(a) * radius))
                self.normals.append((math.cos(a), 0.0, math.sin(a)))
                self.uvs.append((i / segments, 0.0 if y == y0 else 1.0))
        for i in range(segments):
            a = start + i
            b, c, d = a + 1, a + segments + 2, a + segments + 1
            self.indices.extend([a, c, b, a, d, c])
        for y, normal, reverse in [(y0, (0,-1,0), True), (y1, (0,1,0), False)]:
            center_index = len(self.positions)
            self.positions.append((center[0], y, center[2])); self.normals.append(normal); self.uvs.append((0.5,0.5))
            ring_start = start if y == y0 else start + segments + 1
            for i in range(segments):
                a, b = ring_start + i, ring_start + i + 1
                self.indices.extend([center_index, b, a] if reverse else [center_index, a, b])


def _placeholder_maps():
    PLACEHOLDER_ROOT.mkdir(parents=True, exist_ok=True)
    manifest = []
    for asset, meta in POLYHAVEN.items():
        for map_kind in ("diff", "arm", "normal"):
            name = f"{asset}_{map_kind}_PLACEHOLDER.png"
            path = PLACEHOLDER_ROOT / name
            # Regenerate deterministic placeholders on every run so material tuning is
            # reflected in both source PNGs and the corresponding packed GLB assets.
            path.write_bytes(_png(WIDTH, HEIGHT, _noise_pixels(asset, map_kind)))
            manifest.append({"path": str(path.relative_to(ROOT)).replace("\\", "/"), "sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "status": "PLACEHOLDER", "intended_replacement": meta["title"], "source": meta["url"], "creator": meta["creator"], "license": meta["license"]})
    emissive = PLACEHOLDER_ROOT / "emissive_white_PLACEHOLDER.png"
    if not emissive.exists():
        emissive.write_bytes(_png(WIDTH, HEIGHT, lambda _x, _y: (255, 255, 255)))
    manifest.append({"path": str(emissive.relative_to(ROOT)).replace("\\", "/"), "sha256": hashlib.sha256(emissive.read_bytes()).hexdigest(), "status": "PLACEHOLDER", "intended_replacement": "neutral white emissive texture (no external source)"})
    return manifest


def _get_map(asset: str, kind: str, map_manifest: dict):
    names = {"diff": "diff", "arm": "arm", "normal": "nor_gl"}
    actual = SOURCE_ROOT / f"{asset}_{names[kind]}_1k.jpg"
    if actual.exists():
        return actual.read_bytes(), "image/jpeg", (str(actual.relative_to(ROOT)).replace("\\", "/"), hashlib.sha256(actual.read_bytes()).hexdigest(), "DOWNLOADED")
    path = PLACEHOLDER_ROOT / f"{asset}_{kind}_PLACEHOLDER.png"
    data = path.read_bytes()
    return data, "image/png", (str(path.relative_to(ROOT)).replace("\\", "/"), hashlib.sha256(data).hexdigest(), "PLACEHOLDER")


def _make_material(asset: str, base_color, metallic=0.0, roughness=0.8, emissive=(0.0, 0.0, 0.0)):
    return {"asset": asset, "base_color": [*base_color, 1.0], "metallic": metallic, "roughness": roughness, "emissive": list(emissive)}


def _write_glb(path: Path, parts: list[Part], materials: list[dict], manifests: list[dict]):
    binary = bytearray()
    views: list[dict] = []
    accessors: list[dict] = []
    primitives = []
    for part in parts:
        if not part.indices:
            continue
        def accessor(data: bytes, component_type: int, count: int, ty: str, target: int, minmax=None):
            view = _add_blob(binary, views, data, target)
            item = {"bufferView": view, "componentType": component_type, "count": count, "type": ty}
            if minmax is not None:
                item.update(minmax)
            accessors.append(item)
            return len(accessors) - 1
        pos_bytes = b"".join(struct.pack("<3f", *p) for p in part.positions)
        norm_bytes = b"".join(struct.pack("<3f", *p) for p in part.normals)
        uv_bytes = b"".join(struct.pack("<2f", *p) for p in part.uvs)
        idx_bytes = b"".join(struct.pack("<I", i) for i in part.indices)
        xs, ys, zs = zip(*part.positions)
        pos_a = accessor(pos_bytes, 5126, len(part.positions), "VEC3", 34962, {"min": [min(xs), min(ys), min(zs)], "max": [max(xs), max(ys), max(zs)]})
        norm_a = accessor(norm_bytes, 5126, len(part.normals), "VEC3", 34962)
        uv_a = accessor(uv_bytes, 5126, len(part.uvs), "VEC2", 34962)
        idx_a = accessor(idx_bytes, 5125, len(part.indices), "SCALAR", 34963)
        primitives.append({"attributes": {"POSITION": pos_a, "NORMAL": norm_a, "TEXCOORD_0": uv_a}, "indices": idx_a, "material": part.material, "mode": 4})

    texture_indices: dict[tuple[str, str], int] = {}
    images: list[dict] = []
    textures: list[dict] = []
    image_payloads: list[tuple[str, str, bytes]] = []
    for material in materials:
        asset = material["asset"]
        for map_kind in ("diff", "arm", "normal"):
            key = (asset, map_kind)
            if key in texture_indices:
                continue
            data, mime_type, info = _get_map(asset, map_kind, manifests)
            texture_indices[key] = len(images)
            image_payloads.append((asset, map_kind, data))
            manifests.setdefault("asset_records", []).append({"path": info[0], "sha256": info[1], "status": info[2], "asset": POLYHAVEN[asset]["title"], "map": map_kind})
        if material["emissive"] != [0.0, 0.0, 0.0]:
            ep = PLACEHOLDER_ROOT / "emissive_white_PLACEHOLDER.png"
            key = ("emissive_white_PLACEHOLDER", "emissive")
            if key not in texture_indices:
                data = ep.read_bytes()
                texture_indices[key] = len(images)
                image_payloads.append((key[0], key[1], data))
                manifests.setdefault("asset_records", []).append({"path": str(ep.relative_to(ROOT)).replace("\\", "/"), "sha256": hashlib.sha256(data).hexdigest(), "status": "PLACEHOLDER", "asset": "white emissive multiplier"})
    for asset, map_kind, data in image_payloads:
        mime_type = "image/jpeg" if data.startswith(b"\xff\xd8") else "image/png"
        view = _add_blob(binary, views, data)
        images.append({"bufferView": view, "mimeType": mime_type, "name": f"{asset}_{map_kind}"})
        textures.append({"source": len(textures)})

    gltf_materials = []
    for material in materials:
        asset = material["asset"]
        item = {
            "pbrMetallicRoughness": {
                "baseColorFactor": material["base_color"],
                "baseColorTexture": {"index": texture_indices[(asset, "diff")]},
                "metallicFactor": material["metallic"],
                "roughnessFactor": material["roughness"],
                "metallicRoughnessTexture": {"index": texture_indices[(asset, "arm")]},
            },
            "normalTexture": {"index": texture_indices[(asset, "normal")]},
            "occlusionTexture": {"index": texture_indices[(asset, "arm")], "strength": 0.45},
        }
        if material["emissive"] != [0.0, 0.0, 0.0]:
            item["emissiveTexture"] = {"index": texture_indices[("emissive_white_PLACEHOLDER", "emissive")]}
            item["emissiveFactor"] = material["emissive"]
        gltf_materials.append(item)

    doc = {
        "asset": {"version": "2.0", "generator": "tools/make_test_models.py (horror_slice placeholders; replace with CC0 source maps)"},
        "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": primitives}], "materials": gltf_materials,
        "textures": textures, "images": images, "accessors": accessors, "bufferViews": views,
        "buffers": [{"byteLength": len(binary)}],
    }
    json_chunk = _pad(json.dumps(doc, separators=(",", ":")).encode(), b" ")
    bin_chunk = _pad(bytes(binary), b"\0")
    total = 12 + 8 + len(json_chunk) + 8 + len(bin_chunk)
    result = struct.pack("<4sII", b"glTF", 2, total) + struct.pack("<I4s", len(json_chunk), b"JSON") + json_chunk + struct.pack("<I4s", len(bin_chunk), b"BIN\0") + bin_chunk
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(result)
    manifests.setdefault("models", []).append({"path": str(path.relative_to(ROOT)).replace("\\", "/"), "sha256": hashlib.sha256(result).hexdigest(), "status": "PROCEDURAL_PLACEHOLDER_GEOMETRY"})


def generate_assets():
    MODEL_ROOT.mkdir(parents=True, exist_ok=True)
    SOURCE_ROOT.mkdir(parents=True, exist_ok=True)
    manifest_records = _placeholder_maps()
    manifest: dict = {"schema_version": 1, "notice": "No external binary textures could be downloaded in the generation environment. Generated maps are marked PLACEHOLDER; run tools/fetch_horror_textures.py and then tools/make_test_models.py to embed CC0 maps.", "references": [], "sources": [{"asset": v["title"], "url": v["url"], "creator": v["creator"], "license": v["license"], "recommended_resolution": "1K", "replacements": [f"{k}_diff_1k.jpg", f"{k}_arm_1k.jpg", f"{k}_nor_gl_1k.jpg"]} for k, v in POLYHAVEN.items()], "textures": manifest_records, "asset_records": [], "models": []}

    # Architecture shell: floor, four walls, and low ceiling. The player starts facing away from the near wall.
    wall = _make_material("concrete_wall_003", (0.94, 0.96, 0.92), metallic=0.0, roughness=0.97)
    floor = _make_material("concrete_floor_damaged_01", (0.94, 0.95, 0.92), metallic=0.0, roughness=0.96)
    ceiling = _make_material("concrete_wall_003", (0.48, 0.50, 0.45), metallic=0.0, roughness=0.98)
    shell_parts = [Part(0), Part(1), Part(2)]
    shell_parts[1].box((0.0, -0.09, 0.0), (12.0, 0.18, 14.0), 0.48)
    shell_parts[0].box((-5.95, 1.50, 0.0), (0.18, 3.0, 14.0), 0.45)
    # Right wall split around the doorway.
    shell_parts[0].box((5.95, 1.50, 3.0), (0.18, 3.0, 8.0), 0.45)
    shell_parts[0].box((5.95, 1.50, -5.0), (0.18, 3.0, 4.0), 0.45)
    shell_parts[0].box((5.95, 2.695, -2.0), (0.18, 0.61, 2.0), 0.45)
    shell_parts[0].box((0.0, 1.50, -6.95), (12.0, 3.0, 0.18), 0.45)
    shell_parts[0].box((0.0, 1.50, 6.95), (12.0, 3.0, 0.18), 0.45)
    shell_parts[2].box((0.0, 2.99, 0.0), (12.0, 0.18, 14.0), 0.45)
    # Old unlit fluorescent fixture, built as an architectural detail, not an additional game mechanic.
    shell_parts[0].box((0.0, 2.79, 0.25), (1.8, 0.10, 0.22), 1.0)
    _write_glb(MODEL_ROOT / "room_shell.glb", shell_parts, [wall, floor, ceiling], manifest)

    metal = _make_material("rusty_metal_02", (0.82, 0.79, 0.71), metallic=0.08, roughness=0.91)
    dark_metal = _make_material("rusty_metal_02", (0.025, 0.035, 0.03), metallic=0.5, roughness=0.48)
    door_parts = [Part(0), Part(1)]
    door_parts[0].box((0, 1.20, 0), (0.12, 2.38, 1.92), 0.75)
    # Raised vertical steel ribs and lock hardware on the inside-facing -X surface.
    for z in (-0.76, -0.38, 0.0, 0.38, 0.76):
        door_parts[0].box((-0.069, 1.2, z), (0.015, 2.18, 0.035), 1.0)
    door_parts[1].box((-0.082, 1.05, 0.67), (0.045, 0.16, 0.11), 1.0)
    door_parts[1].ellipsoid((-0.115, 1.05, 0.67), (0.045, 0.055, 0.055), 12, 8)
    # Obvious dark keyhole inset below the handle, visible from inside the room.
    door_parts[1].ellipsoid((-0.114, 0.78, 0.67), (0.025, 0.065, 0.035), 10, 6)
    _write_glb(MODEL_ROOT / "door.glb", door_parts, [metal, dark_metal], manifest)

    gold = _make_material("rusty_metal_02", (1.0, 0.80, 0.36), metallic=0.08, roughness=0.72)
    key_parts = [Part(0)]
    # Key ring, long stem and two notches; laid flat for a clearly visible pickup silhouette.
    center, outer, inner, segs = ( -0.18, 0.045, 0.0), 0.13, 0.078, 20
    start = len(key_parts[0].positions)
    p = key_parts[0]
    for rad in (outer, inner):
        for i in range(segs):
            a = i * math.tau / segs
            p.positions.append((center[0] + math.cos(a) * rad, center[1], center[2] + math.sin(a) * rad))
            p.normals.append((0.0, 1.0, 0.0)); p.uvs.append((i / segs, 0.0))
    for i in range(segs):
        a, b = start + i, start + (i + 1) % segs
        c, d = start + segs + (i + 1) % segs, start + segs + i
        p.indices.extend([a, c, b, a, d, c])
    p.box((0.19, 0.045, 0.0), (0.53, 0.055, 0.075), 1.0)
    p.box((0.37, 0.045, -0.07), (0.08, 0.055, 0.12), 1.0)
    p.box((0.47, 0.045, -0.06), (0.075, 0.055, 0.10), 1.0)
    _write_glb(MODEL_ROOT / "key.glb", key_parts, [gold], manifest)

    lantern_dark = _make_material("rusty_metal_02", (0.025, 0.032, 0.029), metallic=0.18, roughness=0.84)
    lantern_glow = _make_material("rusty_metal_02", (0.8, 0.95, 0.75), metallic=0.0, roughness=0.25, emissive=(1.2, 1.7, 1.1))
    lantern_parts = [Part(0), Part(1)]
    lantern_parts[0].box((0, 0.22, 0), (0.72, 0.44, 0.70), 0.8)  # heavy save-point pedestal
    lantern_parts[0].box((0, 0.52, 0), (0.46, 0.08, 0.44), 1.0)
    lantern_parts[0].box((0, 1.20, 0), (0.48, 0.08, 0.46), 1.0)
    for x in (-0.20, 0.20):
        for z in (-0.19, 0.19):
            lantern_parts[0].box((x, 0.86, z), (0.045, 0.66, 0.045), 1.0)
    lantern_parts[0].box((0, 1.43, 0), (0.35, 0.06, 0.33), 1.0)
    lantern_parts[1].box((0, 0.86, 0), (0.28, 0.56, 0.27), 1.0)
    _write_glb(MODEL_ROOT / "save_point.glb", lantern_parts, [lantern_dark, lantern_glow], manifest)

    creature = _make_material("concrete_wall_003", (0.018, 0.024, 0.021), metallic=0.0, roughness=0.98)
    eyes = _make_material("rusty_metal_02", (0.6, 0.9, 0.6), metallic=0.0, roughness=0.2, emissive=(0.32, 0.75, 0.38))
    monster_parts = [Part(0), Part(1)]
    monster_parts[0].ellipsoid((0.0, 1.27, 0.0), (0.30, 0.67, 0.20), 14, 9)
    monster_parts[0].ellipsoid((0.0, 2.06, 0.015), (0.19, 0.26, 0.17), 14, 9)
    monster_parts[0].ellipsoid((-0.39, 1.37, 0.0), (0.12, 0.59, 0.12), 10, 7)
    monster_parts[0].ellipsoid((0.39, 1.36, 0.0), (0.12, 0.62, 0.12), 10, 7)
    monster_parts[0].ellipsoid((-0.16, 0.46, 0.0), (0.14, 0.47, 0.14), 10, 7)
    monster_parts[0].ellipsoid((0.16, 0.46, 0.0), (0.14, 0.47, 0.14), 10, 7)
    monster_parts[1].ellipsoid((-0.075, 2.09, 0.158), (0.022, 0.026, 0.012), 8, 5)
    monster_parts[1].ellipsoid((0.075, 2.09, 0.158), (0.022, 0.026, 0.012), 8, 5)
    _write_glb(MODEL_ROOT / "monster_placeholder.glb", monster_parts, [creature, eyes], manifest)

    for path in MODEL_ROOT.glob("*.glb"):
        if path.name in ("room_shell.glb", "door.glb", "key.glb", "save_point.glb", "monster_placeholder.glb"):
            continue
        # Preserve any other existing test-model binaries; they are not horror assets.
    for model_record in manifest["models"]:
        for asset in POLYHAVEN:
            if f"{asset}_" in model_record["path"]:
                model_record["reference_texture"] = POLYHAVEN[asset]["title"]

    # Add any actually downloaded files to the manifest once, with source hashes and attribution.
    for asset, meta in POLYHAVEN.items():
        for suffix, name in [("diff", "diff"), ("arm", "arm"), ("normal", "nor_gl")]:
            path = SOURCE_ROOT / f"{asset}_{name}_1k.jpg"
            if path.exists():
                manifest["textures"].append({"path": str(path.relative_to(ROOT)).replace("\\", "/"), "sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "status": "DOWNLOADED", "source": meta["url"], "creator": meta["creator"], "license": meta["license"], "map": suffix})
    downloaded_maps = [
        SOURCE_ROOT / f"{asset}_{map_name}_1k.jpg"
        for asset in POLYHAVEN
        for map_name in ("diff", "arm", "nor_gl")
    ]
    downloaded_count = sum(path.exists() for path in downloaded_maps)
    if downloaded_count == len(downloaded_maps):
        manifest["notice"] = "All nine 1K source maps were downloaded from the official Poly Haven API and embedded in the self-contained GLBs; procedural PLACEHOLDER source maps remain in the placeholder folder for fallback/testing."
    elif downloaded_count:
        manifest["notice"] = f"Downloaded {downloaded_count} of nine planned 1K source maps from the official Poly Haven API; missing material maps are generated and explicitly marked PLACEHOLDER."
    else:
        manifest["notice"] = "No external binary textures could be downloaded in the generation environment. Generated maps are marked PLACEHOLDER; run tools/fetch_horror_textures.py and then tools/make_test_models.py to embed CC0 maps."

    reference_image = ASSET_ROOT / "reference" / "horror_room_concept_reference.png"
    if reference_image.exists():
        manifest["references"] = [{
            "path": str(reference_image.relative_to(ROOT)).replace("\\", "/"),
            "sha256": hashlib.sha256(reference_image.read_bytes()).hexdigest(),
            "status": "GENERATED_CONCEPT_REFERENCE",
            "note": "Visual target reference only; not loaded by the game at runtime.",
        }]
    out_path = ASSET_ROOT / "asset_manifest.json"
    # Keep the model record list deterministic and deduplicate map records.
    unique = {}
    for record in manifest["asset_records"]:
        key = (record["path"], record.get("map"), record.get("asset"))
        unique[key] = record
    manifest["asset_records"] = list(unique.values())
    out_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")

    credits = ["# Horror Slice asset credits", "", "Downloaded maps are CC0 and retain the source creator attribution below. If maps could not be downloaded, the packaged maps are explicitly marked `PLACEHOLDER` and are procedural rather than copies of these works.", ""]
    for asset, meta in POLYHAVEN.items():
        credits.extend([f"## {meta['title']}", f"- Source: {meta['url']}", f"- Creator: {meta['creator']}", f"- License: {meta['license']}", "- Intended use: wall/floor/metal PBR replacement; albedo + AO/roughness/metalness + OpenGL normal map.", ""])
    credits.extend(["## Local placeholders", "- Generated by `tools/horror_room_assets.py` and invoked through `python tools/make_test_models.py`.", "- Status: `PLACEHOLDER`; not presented as external photographs or as works by the credited Poly Haven authors.", "- Replace by running `python tools/fetch_horror_textures.py` then `python tools/make_test_models.py`.", ""])
    (ASSET_ROOT / "CREDITS.md").write_text("\n".join(credits), encoding="utf-8")
    print(f"generated horror_slice models and manifest at {ASSET_ROOT}")
