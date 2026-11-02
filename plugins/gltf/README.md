# `ribbon_plugin_gltf`

`ribbon_plugin_gltf` decodes glTF 2.0 scene geometry, node transforms, skins/animations, images, and metallic-roughness material inputs into `ModelAsset` data for the engine asset pipeline. The current importer reads base-color, metallic-roughness, normal, occlusion, and emissive textures and `TEXCOORD_0`; it does not yet preserve glTF tangent attributes or `TEXCOORD_1`.

## Material compatibility status

| glTF property | Current behavior |
| --- | --- |
| `baseColorFactor` / base-color texture | Imported |
| Metallic / roughness factors and texture | Imported |
| Normal texture | Imported; render3d currently reconstructs the tangent frame from derivatives |
| Occlusion texture and strength | Imported as material AO input; this is not SSAO |
| Emissive texture and factor | Imported |
| `alphaMode` / `alphaCutoff` | Imported, including glTF defaults (`OPAQUE`, cutoff `0.5`) and passed to render3d |
| `doubleSided` | Imported and passed to render3d pipeline selection |
| `TANGENT` / `TEXCOORD_1` | Not retained by current primitive vertex data |

## Alpha and sidedness behavior

The importer preserves glTF `OPAQUE`, `MASK`, and `BLEND` as `AlphaMode`, plus the alpha cutoff and `doubleSided` flag. The renderer implements the following runtime behavior:

- `OPAQUE`: normal depth testing and depth writes; both base-color factor alpha and texture alpha are ignored as required by glTF.
- `MASK`: samples base-color texture alpha multiplied by `baseColorFactor.a`, discards fragments below `alphaCutoff`, and applies the same discard in directional and spotlight shadow passes.
- `BLEND`: uses `baseColorFactor.a × baseColorTexture.a` with the standard straight-alpha blend state, depth testing remains enabled, depth writes are disabled, and objects are sorted back-to-front by transformed mesh-bounds center in camera space. Blended materials are omitted from shadow depth passes.
- `doubleSided`: uses no-cull color pipeline variants only for materials that request it; back-face normals are flipped for lighting. The directional shadow pass also uses a no-cull variant for these materials. Ordinary materials retain back-face culling.

Transparency is conventional object-center sorting, not order-independent transparency. Intersecting transparent surfaces and self-overlapping meshes can still sort imperfectly. The renderer does not claim weighted OIT, refraction, decals, or a general particle system. `TANGENT` and `TEXCOORD_1` are still not retained by the current vertex format.
