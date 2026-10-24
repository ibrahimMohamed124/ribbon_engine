# Model Viewer

Small native-window example that loads a glTF model and orbits a camera around its bounds.

## Streaming behavior

The model is requested with `AssetServer::request_ref`, so source IO and CPU glTF decode happen on the asset worker pool. The sample keeps the strong `AssetRef<ModelAsset>` in a resource while the model is displayed. The camera waits until the model becomes resident and then fits to its bounds. GPU buffers and textures are still uploaded by `ribbon_plugin_render3d` on the renderer thread.

The viewer accepts `.glb` files and `.gltf` files with embedded data only.

## Run

```powershell
cargo run -p model_viewer
cargo run -p model_viewer -- --frames 120
cargo run -p model_viewer -- path\to\model.glb
```

A failed request is reported once on the console. This example intentionally retains the last requested lease for its lifetime; it does not enable automatic cache eviction.

## Render profiling capture

Run the example in release mode with a finite frame count to write a CSV report after shutdown.
The default warmup is 120 successfully rendered frames; set `--profile-warmup N` to change it.
For example:

```powershell
cargo run --release -p model_viewer -- --frames 1800 --profile-csv target/profiles/model-viewer.csv
python tools/render_profile_report.py target/profiles/model-viewer.csv --output target/profiles/model-viewer.md
```

Compare runs only on the same machine, GPU adapter, resolution, build profile, scene setup, and
frame-count/warmup settings. GPU timestamps are asynchronous and the current GPU interval covers
shadow plus mesh rendering, not the whole frame. Resource byte counts are estimates, not driver VRAM readings.
