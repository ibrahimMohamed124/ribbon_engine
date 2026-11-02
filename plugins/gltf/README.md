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
| `alphaMode` / `alphaCutoff` | Not represented in the current material contract |
| `doubleSided` | Not represented in the current material contract |
| `TANGENT` / `TEXCOORD_1` | Not retained by current primitive vertex data |

Consequently, the current importer/renderer should be treated as an opaque PBR foundation. It does not claim working glTF MASK/BLEND behavior or double-sided pipeline variants. These are explicit follow-up implementation items, not silently emulated by writing base-color alpha.
