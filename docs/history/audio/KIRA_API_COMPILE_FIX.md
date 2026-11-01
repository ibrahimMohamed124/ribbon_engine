# Kira 0.12.5 API compile fix

This overlay corrects the compile errors reported while validating `ribbon_plugin_audio` against Kira 0.12.5.

Changes:

- Uses `StreamingSoundHandle<kira::sound::FromFileError>` for the decoder error type returned by Kira's streaming constructors.
- Passes stereo pan as `f32`, matching Kira's `Panning` conversion.
- Passes `Mix` values as `f32`.
- Calls `FilterHandle::set_mode` only at effect construction time; in Kira 0.12.5 it does not accept a tween. The mode is fixed by each filter builder.
- Tracks command-driven state for streaming handles, which do not expose `state()` in Kira 0.12.5. The per-voice `TrackHandle::num_sounds()` signal is used to recognize natural stream completion and finish stop fades without retaining dead voice records.
- Converts event cooldown seconds from `f32` to `f64` for comparison with the deterministic event clock.
- Removes unused imports and rewrites builder construction to avoid overwritten-initial-value warnings. Also removes a duplicate effect-application call.
- Adds a device-independent mapping test for all Kira playback states.

## Applying

Extract the accompanying ZIP into the project root, allowing it to overwrite these project-relative files. No copy script is needed. If the overlay was extracted directly into the project root, the files are already in place; do not copy a file onto itself.

## Validation

Run `cargo fmt --all --check`, then `cargo check -p ribbon_plugin_audio --all-targets`, `cargo test -p ribbon_plugin_audio --all-targets --all-features`, `cargo test -p ribbon_plugin_audio --no-default-features`, and `cargo test -p studs --test audio`. This environment did not have Cargo/rustc installed, so these commands are not represented as having passed here.
