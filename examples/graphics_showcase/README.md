# Graphics Showcase

A live windowed scene demonstrating the graphics features that are integrated
into the current `plugin_render3d` pipeline.

Run from the repository root:

```sh
cargo run -p graphics_showcase
```

To close automatically after a known number of frames:

```sh
cargo run -p graphics_showcase -- --frames 600
```

## Controls

- **Space**: pause or resume the animated objects, moving directional light, and camera orbit.
- **Left / Right**: reduce or increase orbit speed; continuing left can reverse the orbit.
- **Up / Down**: raise or lower the camera.
- **R**: reset the orbit, camera height, and animation state.

## What it demonstrates

- The active `wgpu` mesh rasterization pipeline and depth buffer.
- Instanced built-in cube geometry with per-instance color tints.
- Directional sunlight and ambient illumination, with the sun direction animated.
- glTF/GLB loading through `AssetServer` and `GltfLoader`.
- Base-color texture sampling and the renderer's generated mip chain for the checker cube.
- Multiple instances of a cached imported model, transformed around its actual model-space bounds.
- A separate imported triangle whose normals are computed by the glTF loader when absent.
- Per-frame transforms, camera orbit, and frame-limit shutdown for capture workflows.

The example uses `GpuWindowPlugin` for direct native-surface presentation and
`ExtractedScene3D` for the ECS-to-render boundary. Camera and entity world transforms
are resolved by `ScenePlugin` and extracted during `PostUpdate`; GPU passes consume the
resulting snapshot. On the first rendered frame, the terminal prints `[RENDER EXTRACT] PASS`
with the camera, built-in mesh, and glTF placement counts (or `FAIL` if extraction is empty).

The example intentionally uses only capabilities already integrated into the
current windowed mesh renderer. It is not a claim that mesh ray-picking, shadow
maps, metallic/roughness PBR, normal/ORM maps, bloom, ambient occlusion, or
voxel-to-chunk-mesh rendering are implemented in this pipeline. Those gaps are
tracked in [`../../docs/RENDERER_PLAN.md`](../../docs/RENDERER_PLAN.md).
