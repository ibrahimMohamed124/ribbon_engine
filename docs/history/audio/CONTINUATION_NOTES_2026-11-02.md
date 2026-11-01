# Ribbon Audio Continuation — Seven Gap Pass

This overlay continues from the verified audio Fix 3 baseline and focuses on the seven requested gaps.

## Implemented in this pass

1. **Dynamic hierarchical mixer:** Kira master/built-in hierarchy plus up to 16 runtime custom child buses. Bus/send IDs and topology are validated in `mixer_graph.rs`; backend creates actual Kira child tracks.
2. **Custom buses and send routing:** up to 16 shared send tracks and 8 predeclared routes per custom bus. Kira routes are assigned in `TrackBuilder::with_send`; live gains are controlled through `TrackHandle::set_send`. Parent pause/stop/resume includes descendant buses; pause is stored at the mixer-node level, applies to voices created while paused, and survives device rebuild. A live route cannot be added after a Kira track is created, so route/parent topology changes require recreate after dependent voices/children are removed. Parallel reverb/delay sends return wet-only signal to avoid duplicating the dry path.
3. **DSP:** 4-band bell parametric EQ, Kira low/high pass, reverb, distortion and compressor, bounded 1-second delay, and a 48-tap synthetic stereo FIR convolution stage. Delay/wet/feedback controls are smoothed in the custom processor. Arbitrary recorded IR asset import and replacement is not implemented.
4. **Steam Audio:** optional `steam-audio` Cargo feature using AudioNimbus 0.16.0 (Steam Audio 4.8.1), with per-voice Steam Audio DirectEffect occlusion and optional binaural HRTF effects. ECS-managed source direction updates atomically; shared context/HRTF resources are reused. Steam Audio scene geometry, ray-traced simulation, reflections and pathing are not wired. It requires Clang >=9 and curl/wget and downloads roughly 180 MB on first auto-install build.
5. **Material propagation:** bounded acoustic zone/portal graph, material transmission and low-pass, dynamic door open fraction interpolation, best-transmission path query, and a helper to compose the path into `ManagedEmitterAcoustics`. Opaque portals are excluded from audible routes and non-finite direct material inputs are sanitized. The application supplies room/door topology and ray/physics material results; no general triangle-mesh raytracer is included.
6. **Device loss recovery:** worker monitors backend errors after successful device initialization; it retries output recreation with 1–30 second exponential backoff that is not reset by repeated queued errors, rebuilds buses/sends and restores bus pause state before replaying embedded SoundBank voices/ambience. Replayed clips restart from the beginning. Unowned external file/cursor streams are completed. Initial no-device construction still returns an error so the caller must retry creation.
7. **PCM peak management:** static decode metadata preflight for WAV, AIFF, FLAC, Ogg Vorbis and MP3; per clip/bank/peak guards; conservative estimate and streaming fallback; real fixture decode integration test for each enabled format. The peak metric estimates plugin-visible allocations plus a 512 KiB margin; it cannot bound codec-internal scratch allocations absolutely.

## Files

- `plugins/audio/Cargo.toml` — decoder features and optional `steam-audio` / AudioNimbus dependency.
- `plugins/audio/src/mixer_graph.rs` — graph, limits, send and subtree validation.
- `plugins/audio/src/kira_backend.rs` — real Kira tracks, sends, DSP, preflight, recovery, HRTF voice attachment.
- `plugins/audio/src/mixer.rs` — EQ, delay, distortion, convolution descriptors and validation.
- `plugins/audio/src/decode_budget.rs` — WAV/AIFF/FLAC/Ogg/MP3 metadata parsers.
- `plugins/audio/src/propagation.rs` — material and portal path solver with door response.
- `plugins/audio/src/steam_audio.rs` — optional Kira Steam Audio direct-occlusion and binaural effect adapters.
- `plugins/audio/src/threaded.rs` — bounded worker commands and output recovery loop.
- `plugins/audio/src/debug.rs` — resource, decode and recovery diagnostics.
- `plugins/audio/README.md` — updated usage, capabilities, limits, prerequisites and validation.
- `tools/allowed_deps.toml` — allow-list entry for optional AudioNimbus dependency.

## Known limitations / next production-hardening pass

- No arbitrary WAV/AIFF impulse-response import yet; convolution uses the built-in synthetic IR.
- Steam Audio direct occlusion and HRTF are integrated per voice; scene-based ray tracing, reflections, pathing and geometry baking remain unwired. The native feature still needs Windows compile/runtime validation with SDK and audio hardware.
- Portal acoustics requires an application-owned map from scene rooms/doors to graph IDs and an application collision/material query.
- Device recovery after a runtime disconnect is implemented, but startup with no audio endpoint still fails fast; SoundBank replay restarts from time zero, external streams complete.
- Static decode peak estimate does not control private temporary allocations inside third-party codecs.
- Kira effect graphs are assembled at track creation. New bus parent/send topology cannot mutate in place; create a replacement bus after removing voices and child buses.
- `Cargo.lock` needs regeneration by an ordinary Cargo resolution command in an environment with registry access because AudioNimbus was added as an optional pinned dependency.

## Required validation on Windows

```powershell
cargo fmt --all -- --check
cargo check -p ribbon_plugin_audio --all-targets
cargo test -p ribbon_plugin_audio --all-targets --no-fail-fast
cargo clippy -p ribbon_plugin_audio --all-targets -- -D warnings
cargo test -p ribbon_plugin_audio --test decoder_integration -- --nocapture
cargo test -p ribbon_plugin_audio --no-default-features
cargo check -p ribbon_plugin_audio --features steam-audio --all-targets
cargo test -p ribbon_plugin_audio --features steam-audio --all-targets
py -3 tools/check_deps.py
```

Workspace validation should follow after the package-level suite passes. The editing container does not have `cargo`, `rustc` or `rustfmt`; no Rust build/test result is claimed for this overlay.

## Additional hardening performed during continuation

- Device recovery now reuses the decoded static-sound cache instead of decoding the complete SoundBank again. The cache clone only clones `StaticSoundData` handles; Kira stores their PCM as `Arc<[Frame]>`, so the decoded sample allocation is shared between old/new managers. A unit test checks `Arc::ptr_eq` across the recovery cache clone.
- Recovery-time cache reuse preserves existing decode diagnostics and memory counters without rescanning/redecoding the bank.
- WAV preflight now rejects zero sample rates and block-align values inconsistent with channels and bits per sample; the new regression test mutates both fields on a real WAV fixture and verifies rejection.
- FLAC STREAMINFO sample counts use checked conversion to `usize`, avoiding truncation on narrower targets.
- After each successful full decode, static admission recalculates the peak estimate with actual decoded PCM bytes and defers the sound to streaming if the measured plugin-visible peak exceeds the configured guard.
- Added direct Kira `Effect` unit tests using `MockInfoBuilder`: the delay test verifies a wet-only impulse at the expected 10-frame offset with no dry duplicate, and the convolution test verifies a finite non-zero decaying FIR response.

The recovery cache-sharing behavior relies on Kira's documented `StaticSoundData` layout (`frames: Arc<[Frame]>`) and cheap-clone semantics. Cargo build/test must still be run on the target Windows toolchain; this environment does not contain Rust tooling.


## Latest Steam Audio direct-path work

- Added a per-voice stereo-preserving `DirectEffect` under the optional native feature. The worker-driven occlusion value is converted to Steam Audio's clear-path convention; Ribbon's material transmission gain is still applied in its normal gain path, avoiding duplicate transmission attenuation. Binaural processing alone performs the mono point-source conversion.
- ECS-managed emitters now attach binaural direction at voice creation and update it as the listener-relative direction changes.
- Shared Steam Audio `Context`, `AudioSettings` and default `Hrtf` are retained process-wide to avoid reconstructing native contexts per voice.
- Added `plugins/audio/tests/decoder_integration.rs`, which decodes actual WAV, AIFF, FLAC, MP3 and Ogg Vorbis fixtures through Kira and verifies finite, non-silent PCM.
- Fixture container metadata was checked with `ffprobe`, and each file was decoded independently with `ffmpeg -f null` in this environment. This verifies fixture integrity only; it is not a substitute for running the Rust/Kira integration test.


## Windows validation fixes after first application

The first Windows build found Kira API type mismatches in the EQ and distortion gain setters. Kira 0.12.5 accepts `f32`/`Decibels` for those decibel-valued setters (while EQ frequency and Q remain `f64`); the call sites now pass `f32`. Three unused custom-bus tuple bindings were marked intentionally unused because the stable `options.custom_bus` identifier is stored on the active voice. The event rollback helper is now compiled only with the `audio` feature, removing a dead-code warning in `--no-default-features` builds.

The decoder integration coverage is split into five independently reported tests (WAV/PCM, AIFF/PCM, FLAC, MP3 and Ogg Vorbis). The integration test is intentionally gated by the `audio` feature; therefore `cargo test -p ribbon_plugin_audio --no-default-features` reports zero decoder integration tests, while the default-feature command `cargo test -p ribbon_plugin_audio --test decoder_integration -- --nocapture` is the command that must run all five.

## Fix 2 — mutable EQ handle access

The next Windows build found that Kira 0.12.5's EQ handle setters require mutable access. The neutral-reset loop now iterates with `&mut self.eq`, and the per-band update uses `get_mut(eq_band)`. The earlier `f32` Decibels corrections remain intact; EQ frequency and Q stay `f64`.

No decoder fixture or test-gating change is needed: `decoder_integration.rs` is intentionally guarded by `#![cfg(feature = "audio")]`, and the crate default enables `audio`. Thus the explicit default-feature integration command should report five tests; with `--no-default-features`, zero decoder integration tests is expected because Kira is not compiled.

This follow-up was inspected statically. The editing container has no Cargo/Rust compiler, so it is not represented as a compiled or tested patch. Re-run the Windows commands in `OVERLAY_NOTES.md` after applying it.
