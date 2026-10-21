# Model Viewer

Small native-window example that loads a glTF model and orbits a camera around its bounds.

## Streaming behavior

The model is requested with `AssetServer::request_ref`, so source IO and CPU glTF decode happen on the asset worker pool. The sample keeps the strong `AssetRef<ModelAsset>` in a resource while the model is displayed. The camera waits until the model becomes resident and then fits to its bounds. GPU buffers and textures are still uploaded by `plugin_render3d` on the renderer thread.

The viewer accepts `.glb` files and `.gltf` files with embedded data only.

## Run

```powershell
cargo run -p model_viewer
cargo run -p model_viewer -- --frames 120
cargo run -p model_viewer -- path\to\model.glb
```

A failed request is reported once on the console. This example intentionally retains the last requested lease for its lifetime; it does not enable automatic cache eviction.
