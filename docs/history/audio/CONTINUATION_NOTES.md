# Ribbon audio engine continuation — implementation and validation notes

This repository update evolves `plugins/audio` rather than adding a second audio subsystem.

## Implemented in this update

- Kira `0.12.5` is the preferred backend behind the `audio` feature. The Kira manager, tracks, decoders and playback handles live on a dedicated owner thread. `ThreadedAudioSink` exposes a bounded command queue. The old Rodio backend remains optional under `rodio-compat`; the two backends are not initialized together automatically.
- Real Kira-backed playback IDs, live gain/rate/pan/looping controls, pause/resume/stop, fades, asynchronous state/diagnostic requests, deterministic priority-based voice replacement, and a bounded retiring-voice collection.
- File-backed streaming through Kira's streaming decoder. Encoded-byte streaming is distinct from file streaming and retains the compressed source under explicit per-source/aggregate byte budgets. Embedded static sounds are decoded in the backend worker and content-deduplicated; static encoded data is bounded by per-asset and bank limits.
- A live hierarchical mixer graph (`Master` → `Music`, `Ambience`, `Sfx`, `Ui`, `Dialogue`, `Voice`), bus gain/mute/zone gain, effect chains for low-pass, high-pass, reverb and compression, root compression-based peak protection, and interruptible/tweened snapshot changes.
- Variant-aware sound events with validated references and ranges, deterministic seeded variation, no immediate repeated variants when alternatives exist, game-clock cooldowns, per-event instance counts and bus/priority routing.
- ECS `AudioEmitter` synchronization with `GlobalTransform`; a deterministic listener choice; per-frame live voice updates; a physics-provider occlusion hook with staggered/cached/smoothed updates; and priority/blend-weight acoustic AABB zones that update filtering, reverb and ambience gain.
- Typed encoded-audio loader integration with Ribbon's `AssetLoader`, diagnostics, duplicate sound-bank ID rejection, safer numeric sanitation, and tests for mixer transitions, event cursors, registry behavior, voice limits, emitter synchronization, occlusion and zone blending.
- A `audio_engine_showcase` example with a valid embedded WAV, real Kira commands when a device is available, and a silent fallback for headless machines. The example is explicitly gated by the `audio` feature.
- `tools/validate_audio.ps1` to run the principal checks and print individual exit codes. The `effective_master_volume` dead-code warning is fixed by using it in the effective bus-gain calculation rather than suppressing the lint.

## Real remaining limitations

This is materially more complete than the previous Rodio-only version, but the original request's entire production scope is **not** fully satisfied:

- `AudioAssetLoader` returns typed encoded assets, but dynamic `ribbon_assets` handles are not yet automatically promoted into active voices with a full hot-reload/lifetime policy. The legacy sound bank still stores `&'static [u8]` for source compatibility.
- The live effect set currently exposes low-pass, high-pass, reverb and compressor. Kira's EQ, delay and distortion APIs are not yet represented by `AudioEffect`. The root compressor is not a hard true-peak limiter.
- Spatialization currently calculates world-space attenuation, stereo pan, Doppler ratio, and occlusion filtering. It is not HRTF/binaural rendering, and front/back/elevation cues are not independently rendered. Steam Audio is not integrated.
- Occlusion is a game/physics-provided query adapter, not a built-in geometry/material ray caster. Acoustic zones are axis-aligned boxes; portals, transmission materials and ray-traced reflections are not implemented.
- Audio parameters are validated and deterministic sound-event variation works, but arbitrary parameter-to-effect automation curves are still driven by game code rather than a built-in curve binding system.
- Kira feature/codec flags are enabled in Cargo metadata, but the only checked-in generated fixture is WAV. Other formats and device-specific behavior have not been independently run through tests here.
- The `Cargo.lock` from the provided archive predates Kira. Run the included script online / in a Rust-enabled Windows environment so Cargo can resolve Kira `0.12.5` and update the lockfile before any `--locked` build.

## Validation truth

The editing container has no `cargo`, `rustc`, or `rustfmt` executable, and package registry access from shell is unavailable. Consequently, no Rust compile/test/fmt command is claimed to have passed in this environment. The checks performed here were TOML parsing, verification of the generated WAV fixture, selected static source assertions, and ZIP integrity checks. Execute `powershell -ExecutionPolicy Bypass -File tools/validate_audio.ps1` in the Windows checkout to run the build/test matrix and resolve the lockfile; investigate any compiler or regression failure rather than bypassing it.
