# Texture and material memory audit

## Scope

This pass addresses the high material allocation estimate observed in the Horror Slice profile:
`material_bytes_estimate = 224.00 MiB` across 11 reported live materials. The report is an engine-side allocation estimate, not a driver-reported VRAM measurement.

## Findings and changes

### 1. The generated glTF materials all pointed at texture index zero

`games/athar/tools/horror_room_assets.py::_write_glb` assigned `texture_indices[key] = len(images)` while `images` remained empty until a later serialization loop. Consequently, different diffuse, ARM, normal, and emissive map entries were all assigned index zero even though the GLB contained distinct embedded image and texture records. This made maps use the first embedded image instead of their intended source and could cause each material slot to upload the same 1K pixels into several incompatible texture formats.

The generator now assigns indices from `len(image_payloads)`, the ordered list that is populated as unique map payloads are discovered. The five checked-in Horror Slice GLBs have also had their material references corrected to their intended image maps, and `assets/asset_manifest.json` now contains the updated GLB SHA-256 values. Geometry and binary image payloads were preserved; only GLB JSON material texture references and the manifest hashes changed.

### 2. GPU materials previously uploaded duplicate texture objects

Each `GpuMaterial` uploaded separate GPU textures for base color, metallic/roughness, normal, occlusion, and emissive maps. In the Horror Slice assets, the occlusion map deliberately references the same ARM image as metallic/roughness, and the same embedded source maps are reused across several GLBs/materials. Those byte-identical maps do not need multiple GPU allocations when their dimensions and color-space interpretation match.

`MeshRenderer` now maintains a content-addressed weak cache keyed by width, height, sRGB-vs-linear format, and a dual 64-bit pixel fingerprint. Each live material holds strong `Arc` references to its GPU textures; the cache holds only `Weak` references. Therefore the cache itself does not pin GPU memory after the last material releases a texture. Dead weak entries are pruned when the cache grows beyond its maintenance threshold. sRGB and linear views remain separate formats even when the source pixels are identical.

### 3. Resource accounting now counts unique live textures once

`material_bytes_estimate` is now the sum of unique live material texture mip chains plus material uniform bytes. The profile CSV/report exposes this breakdown, the count of unique live material GPU textures, and cumulative texture-cache hits/misses. The texture and uniform breakdown rows are subsets of the aggregate material category and must not be added again to total memory.

## Static asset inventory

Across the five checked-in Horror Slice GLBs, the intended maps are nine unique 1024x1024 textures (diffuse, ARM, and normal for concrete wall, damaged concrete floor, and rusty metal) plus the 128x128 white emissive placeholder. After correcting texture references and sharing identical decoded pixels with the same format, the predicted material texture estimate is approximately 48.08 MiB including the small fallback textures, plus 528 bytes of material uniform data for the eleven material slots. This is a static estimate; the target-side profile must confirm the actual number and bytes of live GPU texture allocations.

Correcting the broken map references will also change the Horror Slice appearance: diffuse, ARM, normal and emissive slots will finally point to the map intended by the asset generator. Review the scene visually when validating the memory reduction.

## Validation boundary

- Static validation confirmed each material map reference resolves to the intended named image, each texture source index is in range, and the manifest hashes match the updated GLBs.
- The texture generator mapping bug and the weak, content-addressed GPU cache are implemented in source.
- No tests were added or changed, and no Cargo build, Clippy run, workspace test, or game runtime was executed while preparing this pass.
- The 48.1 MiB estimate is not an observed post-change profile result. Confirm it by running the existing validation matrix and capturing `athar` on the target Windows machine.

## Target-side verification

From a fresh extraction of the archive, run:

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace -j 1
cargo test --workspace --no-default-features -j 1
python tools/check_deps.py
cargo run --release -p athar -- --frames 1800 --profile-warmup 120 --profile-csv target/profiles/athar-memory-audit.csv
python tools/perf/render_profile_report.py target/profiles/athar-memory-audit.csv --output target/profiles/athar-memory-audit.md
```

Compare `Material textures (deduplicated)`, `Material uniform buffers`, `Unique live material GPU textures`, and cache hits/misses with the previous 224 MiB material estimate. Also inspect diffuse/normal/ARM response, especially the room shell, door, key, save-point lantern, and monster eyes, because correcting the glTF indices restores the intended texture maps.
