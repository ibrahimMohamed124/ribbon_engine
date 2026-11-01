# Ribbon Audio Plugin (`ribbon_plugin_audio`)

`ribbon_plugin_audio` is Ribbon's game-facing audio subsystem. Kira 0.12.5 is the preferred output, mixing and voice backend; the previous Rodio adapter remains opt-in through `rodio-compat`. Backend-owned types stay inside this crate. `ThreadedAudioSink` confines Kira managers, playback handles, streaming decoders and track handles to a dedicated worker thread.

## Features and device behavior

- `audio` (default): enables the Kira backend, CPAL output, Symphonia decoding, and the declared WAV, AIFF, FLAC, MP3, Ogg/Vorbis and PCM codec features.
- `rodio-compat`: enables `rodio_backend::RodioBackend` for applications explicitly retaining that API. It is not initialized by the Kira backend and is not started automatically by STUDS.
- `--no-default-features`: excludes Kira/CPAL from this crate and retains the backend-independent APIs, spatial math, registry, ECS adapters and tests for headless configurations.

Codec capabilities depend on enabled Kira/Symphonia codec features and actual input files. Kira 0.12.5 does not expose an `opus` feature, so `.opus` files are intentionally rejected instead of being advertised as decodable. The included automated fixture is WAV; other codec families have not been independently fixture-tested in this environment. No HRTF or Steam Audio SDK feature is exposed: the present baseline uses world-space distance/direction, stereo panning, Doppler ratio, low-pass occlusion and zones, and does not pretend to provide binaural rendering or ray-traced reflections.

If Kira cannot initialize an output device, `ThreadedAudioSink::new` returns an actionable error. Games such as STUDS can log this once and continue with silent output. The application can retry by constructing a new sink and replacing `AudioOutput`; **automatic in-place device-loss detection/recovery is not implemented** by this version. One-shot events consumed while there is no output are intentionally dropped rather than replayed late; the latest `SetAmbience` value is retained and applied when an output is installed. The command queue is bounded at 256 commands; streaming byte commands have a bounded 64 MiB queued-source budget. Prefer file-backed streaming for long tracks.

## Minimal usage

```rust,no_run
use ribbon_plugin_audio::{
    AmbienceAsset, AmbienceId, PlaybackOptions, SoundAsset, SoundBank, SoundId,
    ThreadedAudioSink,
};

static CLICK: &[u8] = include_bytes!("click.wav");
let bank = SoundBank::try_new(
    vec![SoundAsset { id: SoundId(1), data: CLICK, gain: 1.0, speed: 1.0, spatial: false }],
    Vec::<AmbienceAsset>::new(),
)?;
let sink = ThreadedAudioSink::new(bank)?;
let voice = sink.play_voice(SoundId(1), PlaybackOptions::default())?;
sink.set_voice_gain(voice, 0.5, 0.1)?;
sink.stop_voice(voice, 0.2)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

### Streaming

`play_streaming_file(path, options)` delegates to Kira's file-backed `StreamingSoundData`, which decodes gradually and avoids retaining the complete decoded PCM track. `play_streaming_bytes` is also chunk-decoded but intentionally retains the compressed/encoded input bytes; it is limited to 64 MiB per active retained byte source and the same total backend byte budget. Static sound-bank clips are decoded during backend initialization, deduplicated by encoded content, and bounded to 16 MiB per unique encoded asset / 64 MiB aggregate unique encoded source bytes plus 64 MiB per decoded clip / 128 MiB aggregate retained PCM frames. The PCM metric measures Kira's actual stereo frame allocation and is exposed as `AudioDiagnostics::decoded_static_pcm_bytes`. The PCM admission check occurs after Kira decoding, so a single oversized compressed clip can still create a temporary decode allocation before it is rejected; use file-backed streaming for untrusted or long-form content. A corrupt or over-budget static clip is skipped and exposed through bounded diagnostics; other clips can remain playable. Long music and ambience should use file-backed streaming.

`AudioPlugin` registers `AudioAssetLoader` on the existing `AssetServer` (including when `AssetPlugin` is installed later in plugin-build order). `AudioAssetHandle::load` and `AudioAssetHandle::request` provide synchronous and asynchronous typed loading with a strong `AssetRef` lease. Call `refresh(&AssetServer)` after asset polling to observe the current revision; successful hot reload replaces the immutable encoded snapshot, while a failed reload reports `ReloadFailed` and retains the last good bytes. `ThreadedAudioSink::play_audio_asset` submits a revision snapshot to Kira's chunked streaming decoder. A live voice therefore keeps the submitted source revision stable while later playbacks use the refreshed one. This path is streaming and copies the bounded encoded bytes into the worker command; use the embedded `SoundBank` path for frequently reused short clips that should be predecoded. The bank retains its embedded `&'static [u8]` API for compatibility.

### Voices and buses

Playback IDs are opaque `VoiceId` values. `ThreadedAudioSink` offers pause/resume/stop, gain, rate, pan, looping, bus-wide stop/pause/resume, asynchronous state queries, diagnostics (including a bounded latest decode error and count), independent ambience layers, and interruptible snapshot transitions. `AudioPlugin` polls completions without blocking the game thread and publishes `AudioVoiceCompleted`; when an `AudioEventRuntime` is inserted as a world resource, it releases matching bound event slots automatically. Threaded Kira completion batches include an authoritative bounded active-voice snapshot (at most the configured active and retiring voice budgets), so event reservations are reconciled even if the bounded completion notification queue overflowed. Standalone integrations can use `poll_completed_voices` and `finish_completions` directly. Legacy sinks may omit the active snapshot. Unknown or stale controls are recorded as backend errors; stopping an already stopped voice is idempotent. The Kira bus graph is `main output -> Master -> Music/Ambience/Sfx/Ui/Dialogue/Voice -> per-voice track`. This is a real but currently fixed six-child graph; arbitrary named child buses and user-defined parent routing are not yet exposed. The root includes a peak-protection compressor. Bus gains and mute states are separate from the acoustic-zone multiplier. Snapshots carry gain, mute, zone-gain and effect overrides in one queue command; omitted buses retain their current state.

Available DSP descriptors are `LowPass`, `HighPass`, `Reverb` and `Compressor`. They are applied on real Kira tracks and transition over a tween. The current public effect set does **not** yet expose Kira's EQ, delay or distortion builders, and the implementation does not claim per-voice convolution or a physically accurate limiter. The root compressor is a practical peak-protection stage, not a guarantee against every possible inter-sample peak. The six built-in child buses form a real fixed hierarchy, but runtime-created named sub-buses and independently configurable send levels are still outstanding.

### Spatial audio and ECS

`AudioEmitter` synchronizes with `ribbon_scene::GlobalTransform`; moving voices update gain, stereo pan, rate/Doppler and an occlusion low-pass filter rather than spawning a new player every frame. `SpatialAudioListener` takes its world pose from `GlobalTransform`. If more than one listener exists, the entity with the lowest stable entity bits is used; if none exists, spatial events are ignored and non-spatial events continue. Coordinates use Ribbon's scene convention with +Z up; `ribbon_math::Angles::default()` faces +X (`psi` is pitch and `phi` is yaw), and scene listeners follow `ribbon_math::forward_vec`. The legacy `AudioListener` yaw adapter keeps its historical +Y-at-zero convention. `calculate_with_model` exposes listener-local right/forward/up direction, azimuth and elevation values plus inverse, linear and exponential attenuation curves, with a silent cutoff beyond maximum distance. The Kira backend remains stereo panning rather than HRTF/binaural spatial rendering; the extra direction fields are available to future or external spatial renderers.

`AudioOcclusionProvider` is an adapter to an engine/game collision query. Queries are staggered by entity and frame, cached for a bounded age, and smoothed before application. The material adapter's filter cutoff is preserved as its own response; only the emitter's explicit baseline occlusion is composed as an additional low-pass constraint. Existing adapters can keep implementing `query_occlusion`; material-aware adapters may additionally override `query_acoustics` to return transmission gain and an explicit low-pass cutoff. Those values reach the live Kira voice filter and gain path. The audio plugin does not infer wall materials itself: the game/physics adapter supplies material transmission. `AudioZoneVolume` is an oriented box centered on an entity's global translation and rotated by its scene transform. Overlapping zones at the highest active priority blend their acoustic values by influence weight; lower-priority zones are ignored while that tier is active. The blended zone fades its low-pass, shared bus reverb and ambience level. The audio adapter does not own geometry or materials: applications must implement `query_acoustics` using their collision/material system. Door/portal graphs, ray-traced reflections, a physically modelled propagation solver and HRTF rendering are not implemented here.

### Sound events and parameters

The legacy `AudioEventDef` registry remains available. `AudioEventDescriptor` adds validated variant lists, gain/rate ranges, bus, priority, looping, cooldown and per-event concurrency limits. `AudioEventRuntime` uses a caller-supplied seed and game-clock time for repeatable variant selection. Prefer `trigger_and_play` to resolve, submit and bind a voice atomically from game code; use `finish_completions` when consuming completion batches. The legacy `trigger` + `bind_voice` + `finish` sequence remains available for custom backends. `AudioParameters` provides bounded finite parameters. `AudioAutomation` maps registered parameters to bus gain, voice gain/rate/pan, low-pass/high-pass cutoff and reverb wet controls with deterministic delta-time smoothing. `AudioAutomation::submit` creates one bounded command batch per update; if the command queue is full, the batch is rejected as one unit.

## Validation

Run from the repository root in a Rust environment. Kira is a new dependency and the lockfile must be resolved by Cargo in the target environment before using `--locked` builds.

```powershell
cargo fmt --all --check
cargo check -p ribbon_plugin_audio --all-targets
cargo check -p ribbon_plugin_audio --examples
cargo test -p ribbon_plugin_audio --all-targets --all-features
cargo test -p ribbon_plugin_audio --no-default-features
cargo test -p studs --test audio
cargo check --workspace --all-targets
cargo test --workspace --all-targets
cargo run -p ribbon_plugin_audio --example audio_engine_showcase
```

The editing container used to prepare this continuation has no Rust toolchain (`cargo`, `rustc`, `rustfmt`) and cannot access the package registry from shell. Therefore those Rust commands, the Windows audio-device path, and the full workspace/STUDS test matrix have **not** been run here. Treat them as required validation, not as passed checks. In particular, run ordinary `cargo check` once online to resolve Kira 0.12.5 and update `Cargo.lock`, then review that generated lockfile into source control.
