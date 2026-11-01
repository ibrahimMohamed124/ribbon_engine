# Ribbon Audio Plugin (`ribbon_plugin_audio`)

`ribbon_plugin_audio` is Ribbon's game-facing audio subsystem. Kira 0.12.5 is the preferred output, mixing and voice backend; the previous Rodio adapter remains opt-in through `rodio-compat`. Backend-owned types stay inside this crate. `ThreadedAudioSink` confines Kira managers, playback handles, streaming decoders and track handles to a dedicated worker thread.

## Features and device behavior

- `audio` (default): enables the Kira backend, CPAL output, Symphonia decoding, and the declared WAV, AIFF, FLAC, MP3, Ogg/Vorbis and PCM codec features.
- `rodio-compat`: enables `rodio_backend::RodioBackend` for applications explicitly retaining that API. It is not initialized by the Kira backend and is not started automatically by STUDS.
- `--no-default-features`: excludes Kira/CPAL from this crate and retains the backend-independent APIs, spatial math, registry, ECS adapters and tests for headless configurations.

Codec capabilities depend on enabled Kira/Symphonia codec features and actual input files. Kira 0.12.5 does not expose an `opus` feature, so `.opus` files are intentionally rejected instead of being advertised as decodable. The included automated fixture is WAV; other codec families have not been independently fixture-tested in this environment. No HRTF or Steam Audio SDK feature is exposed: the present baseline uses world-space distance/direction, stereo panning, Doppler ratio, low-pass occlusion and zones, and does not pretend to provide binaural rendering or ray-traced reflections.

If Kira cannot initialize an output device, `ThreadedAudioSink::new` returns an actionable error. Games such as STUDS can log this once and continue with silent output. The command queue is bounded at 256 commands; streaming byte commands have a bounded 64 MiB queued-source budget. Prefer file-backed streaming for long tracks.

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

`play_streaming_file(path, options)` delegates to Kira's file-backed `StreamingSoundData`, which decodes gradually and avoids retaining the complete decoded PCM track. `play_streaming_bytes` is also chunk-decoded but intentionally retains the compressed/encoded input bytes; it is limited to 64 MiB per active retained byte source and the same total backend byte budget. Static sound-bank clips are decoded during backend initialization, deduplicated by encoded content, and bounded to 16 MiB per unique asset / 64 MiB aggregate unique encoded source bytes. A corrupt or over-budget static clip is skipped and exposed through bounded diagnostics; other clips can remain playable. Long music and ambience should use file-backed streaming.

`AudioAssetLoader` integrates encoded bytes with the existing `ribbon_assets` loader mechanism. It does not yet automatically turn a dynamic Ribbon asset handle into a live voice: callers currently submit registered `SoundId`/`SoundBank` assets or use streaming paths directly. The bank still uses embedded `&'static [u8]` data for compatibility.

### Voices and buses

Playback IDs are opaque `VoiceId` values. `ThreadedAudioSink` offers pause/resume/stop, gain, rate, pan, looping, bus-wide stop/pause/resume, asynchronous state queries, diagnostics (including a bounded latest decode error and count), independent ambience layers, and interruptible snapshot transitions. Unknown or stale controls are recorded as backend errors; stopping an already stopped voice is idempotent. The Kira bus graph is `main output -> Master -> Music/Ambience/Sfx/Ui/Dialogue/Voice -> per-voice track`. The root includes a peak-protection compressor. Bus gains and mute states are separate from the acoustic-zone multiplier.

Available DSP descriptors are `LowPass`, `HighPass`, `Reverb` and `Compressor`. They are applied on real Kira tracks and transition over a tween. The current public effect set does **not** yet expose Kira's EQ, delay or distortion builders, and the implementation does not claim per-voice convolution or a physically accurate limiter. The root compressor is a practical peak-protection stage, not a guarantee against every possible inter-sample peak.

### Spatial audio and ECS

`AudioEmitter` synchronizes with `ribbon_scene::GlobalTransform`; moving voices update gain, stereo pan, rate/Doppler and an occlusion low-pass filter rather than spawning a new player every frame. `SpatialAudioListener` takes its world pose from `GlobalTransform`. If more than one listener exists, the entity with the lowest stable entity bits is used; if none exists, spatial events are ignored and non-spatial events continue. Coordinates use Ribbon's world convention with +Z up, and the listener forward vector follows `ribbon_math::forward_vec`.

`AudioOcclusionProvider` is an adapter to an engine/game collision query. Queries are staggered by entity and frame, cached for a bounded age, and smoothed before application. `AudioZoneVolume` is currently an axis-aligned box centered on an entity's global translation. Zone overlap uses highest priority, then highest blend weight, then lowest stable zone ID. The winning zone fades its low-pass, shared bus reverb and ambience level; rotated geometry, portals, transmission materials and ray-traced reflections are not implemented here.

### Sound events and parameters

The legacy `AudioEventDef` registry remains available. `AudioEventDescriptor` adds validated variant lists, gain/rate ranges, bus, priority, looping, cooldown and per-event concurrency limits. `AudioEventRuntime` uses a caller-supplied seed and game-clock time for repeatable variant selection; call `ThreadedAudioSink::play_resolved_event`, and then `AudioEventRuntime::finish(event.name)` when that instance ends (or immediately if playback submission fails). `AudioParameters` provides bounded finite parameters, but parameter-to-snapshot/effect automation curves are not currently automatic; game code drives the relevant controls.

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
