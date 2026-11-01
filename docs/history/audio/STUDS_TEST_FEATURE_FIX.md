# STUDS audio integration-test feature fix

The STUDS `audio` runtime feature intentionally enables Kira only. The integration test `games/studs/tests/audio.rs` also checks the legacy Rodio WAV decoder's decoded-memory footprint. That test-only call must enable the explicit `rodio-compat` feature without enabling Rodio in normal STUDS runtime builds.

`games/studs/Cargo.toml` therefore declares `ribbon_plugin_audio` as a dev-dependency with `default-features = false` and `features = ["rodio-compat"]`. Cargo unifies dependency features for the test target, exposing `rodio_backend` only in test builds. This does not instantiate a Rodio output backend and does not change the game's Kira runtime backend.

The Kira manager is retained in `KiraAudioBackend` solely to own the device/backend lifetime; it is now named `_manager` to express that ownership and eliminate dead-code warnings. The showcase sink binding no longer uses unnecessary `mut`.

## Validation to run on Windows

```powershell
cargo fmt --all --check
cargo test -p ribbon_plugin_audio --all-targets --all-features
cargo test -p ribbon_plugin_audio --no-default-features
cargo check -p ribbon_plugin_audio --no-default-features --all-targets
cargo test -p studs --test audio
cargo check --workspace --all-targets
cargo test --workspace --all-targets
```
