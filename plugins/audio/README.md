# Ribbon Audio Plugin (`ribbon_plugin_audio`)

`ribbon_plugin_audio` is Ribbon's game-facing audio subsystem. Kira 0.12.5 owns the preferred mixer, sound playback and CPAL output; `ThreadedAudioSink` confines Kira managers, track handles and streaming decoders to a dedicated worker. The compatibility Rodio adapter remains opt-in.

## Features and supported decoders

- `audio` (default): Kira + CPAL, Symphonia, and the enabled AIFF, FLAC, MP3, Ogg, PCM, Vorbis and WAV codecs.
- `steam-audio`: optional native Steam Audio integration through AudioNimbus. It implies `audio` and is not enabled by default.
- `rodio-compat`: optional legacy Rodio adapter.
- `--no-default-features`: backend-independent APIs and headless tests without Kira/CPAL.

The embedded decoder integration test uses real fixture files for WAV/PCM, AIFF/PCM, FLAC, MP3 and Ogg Vorbis. These are the codec/container families enabled by this crate. Opus is not enabled by Kira 0.12.5 here and is intentionally not advertised as supported.

## Hierarchical mixer, custom buses and sends

The mixer is a real Kira hierarchy: `main output -> Master -> Music/Ambience/Sfx/Ui/Dialogue/Voice -> per-voice track`. Runtime custom buses are real child tracks and may be nested beneath a built-in or custom bus. The graph validates names, IDs, cycles, parent existence, route IDs, duplicate routes, gains and resource limits before mirroring the graph into Kira.

Current caps are 16 custom buses, 16 send tracks and 8 send routes per custom bus. A custom bus declares its send topology at creation because Kira only allows `set_send` to change the gain of an already-declared route; it cannot add a new route to an existing live track. Live send gains can be automated. To change a bus's parent or add/remove one of its routes, stop its voices, remove and recreate the bus (child buses must be removed first). A send cannot be removed while a custom bus still references it.

```rust,no_run
use ribbon_plugin_audio::{
    AudioBus, AudioSendConfig, AudioSendEffect, AudioSendId, CustomBusConfig,
    CustomBusId, MixerBusParent, MixerSendRoute, PlaybackOptions, SoundId,
    ThreadedAudioSink,
};

fn configure_cave_audio(sink: &ThreadedAudioSink, sound_id: SoundId) -> Result<(), String> {
    let send_id = AudioSendId(1);
    sink.create_audio_send(AudioSendConfig {
        id: send_id,
        name: "shared-delay".into(),
        effect: AudioSendEffect::Delay {
            delay_seconds: 0.35, feedback_db: -12.0, wet: 0.25,
        },
        gain: 1.0,
    })?;
    let bus_id = CustomBusId(100);
    sink.create_custom_bus(CustomBusConfig {
        id: bus_id,
        name: "cave-sfx".into(),
        parent: MixerBusParent::BuiltIn(AudioBus::Sfx),
        gain: 0.8, muted: false,
        sends: vec![MixerSendRoute { send: send_id, gain: 0.4 }],
    })?;
    let _voice = sink.play_voice(sound_id, PlaybackOptions {
        custom_bus: Some(bus_id), ..PlaybackOptions::default()
    })?;
    sink.set_custom_bus_send_gain(bus_id, send_id, 0.2, 0.15)?;
    sink.set_custom_bus_gain(bus_id, 0.6, 0.2)?;
    Ok(())
}

# Ok::<(), Box<dyn std::error::Error>>(())
```

Custom bus gain, mute, effects, pause, resume and stop operations are sent through the bounded worker queue. Parent bus controls apply to nested descendants, including voices created while a parent is paused. Bus-level pause state is restored during device recovery independently from per-voice pause state. Built-in buses also retain their separate user gain, mute and acoustic-zone gain controls.

## DSP effects

Each built-in and custom bus exposes these real Kira track stages: low-pass, high-pass, four independent bell EQ bands, reverb, delay, distortion, convolution and compressor. The effect-chain validator rejects non-finite/out-of-range settings, duplicate single-instance stages and more than four EQ bands. EQ, filters, reverb, distortion and compressor controls use Kira tweens; delay and convolution wet/feedback/time controls are smoothed in the custom realtime processor. Delay is bounded to 1 second.

**Convolution scope:** the current convolution stage is a bounded 48-tap stereo FIR using a generated synthetic decaying impulse response (up to 1 second). This is a real convolution processor, but it does not yet load arbitrary recorded room impulse-response WAV/AIFF assets or expose per-bus custom IR file management. Kira's built-in reverb and the shared reverb send are available for longer diffuse tails.

The master route includes a high-ratio compressor as practical peak protection. It is not a guaranteed brick-wall true-peak limiter.

## HRTF / binaural and Steam Audio

When the `steam-audio` feature is enabled, Kira voices receive a Steam Audio direct-path effect for the live engine occlusion value. A voice can also be created with a listener-local `PlaybackOptions::binaural_direction` (`[right, up, behind]`); the Kira track then receives a BinauralEffect. `ThreadedAudioSink::set_voice_binaural_direction` updates its direction atomically without rebuilding the track. ECS-managed emitters populate this direction automatically and update it as the emitter/listener relationship changes. The direct-path stage preserves stereo for ordinary voices; only binaural HRTF voices are downmixed for point-source rendering. Both processors use shared process-level Steam Audio context/HRTF resources and bounded preallocated buffers. The current direct effect consumes occlusion; material transmission gain and low-pass filtering remain composed by Ribbon's material/portal path so those controls are not applied twice.

```rust,no_run
use ribbon_plugin_audio::{PlaybackOptions, SoundId, ThreadedAudioSink};

fn configure_binaural_voice(
    sink: &ThreadedAudioSink,
    sound_id: SoundId,
) -> Result<(), String> {
    let voice = sink.play_voice(sound_id, PlaybackOptions {
        binaural_direction: Some([1.0, 0.0, 0.0]), // source at listener-right
        ..PlaybackOptions::default()
    })?;
    sink.set_voice_binaural_direction(voice, [0.0, 0.0, 1.0])?;
    Ok(())
}

# Ok::<(), Box<dyn std::error::Error>>(())
```

Steam Audio is optional because it links a native SDK. AudioNimbus 0.16.0 tracks Steam Audio 4.8.1; its `auto-install` feature downloads the platform SDK during the first enabled build. The documented build requirements include `curl` or `wget` and Clang 9.0 or later; the first SDK download is approximately 180 MB and is cached by the build script. See the [AudioNimbus installation and API documentation](https://docs.rs/crate/audionimbus/0.16.0) before enabling it on a build agent.

**Windows bindgen headers:** the Steam Audio feature also needs Clang to resolve the host C/C++ headers. If the build reports `fatal error: 'stddef.h' file not found`, the native SDK download succeeded but `libclang` cannot see the MSVC/Windows SDK include directories. Install Visual Studio Build Tools with the **Desktop development with C++** workload, the latest MSVC x86/x64 toolset and a Windows 10/11 SDK. Then run `tools/build_audio_with_steam_audio.ps1`; it discovers the installed MSVC and SDK include paths, configures `BINDGEN_EXTRA_CLANG_ARGS` for the current PowerShell process, and runs the Steam Audio check, tests and Clippy. The script does not download or install Visual Studio components.

This integration wires per-voice Steam Audio direct occlusion and optional HRTF binaural rendering. It does not yet build Steam Audio scene geometry, ray-traced direct simulation, baked/reflected sound simulations, pathing or convolution IRs. Ribbon's room/portal graph and the application's physics/material adapter currently provide occlusion, transmission and filtering inputs. Steam Audio itself supports geometry-driven direct, reflection and pathing workflows, but those simulation systems still need a separate scene adapter and lifecycle integration.

## Material-aware propagation, doors and portals

`AudioOcclusionProvider::query_acoustics` can return material transmission gain and low-pass cutoff from the game's collision/material system. `AcousticPropagationGraph` adds a bounded room/zone and portal graph (256 zones, 1024 portals). Portal edges carry acoustic material response; doors interpolate between closed and open response using `open_fraction`. A Dijkstra query chooses the most transmissive path, composes transmission multiplicatively and chooses the most restrictive cutoff. Fully opaque portals are excluded from audible routes, so a low-gain alternate path is not hidden by an artificial epsilon cost. `AcousticPropagationResult::apply_to_managed_emitter` sanitizes direct material inputs and composes the route before `ThreadedAudioSink::update_voice_spatial_acoustics`.

The graph is deliberately geometry-agnostic: the game must map scene rooms and doors to zone/portal IDs and update open fractions. It does not ray-cast arbitrary meshes, infer material IDs or calculate diffraction/reflection from triangle geometry.

## Device-loss recovery

After successful initialization, the audio worker polls Kira/CPAL backend errors and attempts to recreate the output backend with exponential backoff from 1 to 30 seconds. Repeated queued errors do not reset the retry timer. A successful recovery rebuilds the standard/custom mixer graph, shared sends, bus gains/mutes/effects/pause state and replayable embedded sound-bank voices/ambience layers. Sound-bank voices restart from the beginning; their precise playback cursor is not restored. File-backed and cursor/byte streaming voices whose original rewindable source is not retained by the backend are reported as completed instead of being resumed with invalid data. Recovery attempts, successes and the latest device error are exposed through `AudioDiagnostics`.

If no output device is available at the initial `ThreadedAudioSink::new` call, construction still returns an error; the caller should retry sink creation when the operating system reports that an output device has returned. In-place recovery is for device loss after initialization.

## PCM decode peak-memory management

Static sound-bank admission parses duration/frame metadata before invoking Kira/Symphonia's full decoder for WAV, AIFF, FLAC, Ogg Vorbis and MP3. It estimates decoded stereo-f32 storage and temporary encoded cursor storage, adds a 512 KiB safety margin, and checks per-sound, bank-total and peak limits before decode. Current limits are 16 MiB encoded per static clip, 64 MiB aggregate unique encoded static assets, 64 MiB PCM per clip, 128 MiB aggregate retained PCM and 128 MiB estimated decode peak. Unique encoded assets share one decoded cache entry. If metadata is malformed, unsupported or cannot be estimated safely, or any limit would be exceeded, the embedded asset is left out of the static cache and is played through chunked streaming instead. File-backed streaming is preferred for large or untrusted content; byte-backed streaming retains compressed input and is separately bounded.

`AudioDiagnostics::decoded_static_pcm_bytes` reports actual retained frame storage. `peak_static_decode_bytes` reports the maximum conservative preflight estimate. The estimate bounds the allocations visible to this plugin but cannot guarantee a strict OS process peak because a codec may use implementation-specific temporary scratch memory.

## Voices, assets and events

`VoiceId` is opaque and stable. `ThreadedAudioSink` offers non-blocking play/stream commands, voice controls, bus controls, snapshots, automation, diagnostics, ambience layers, sends and custom buses. The command queue and streaming-byte budget are bounded. Asset hot reload preserves a stable encoded revision per voice. `AudioPlugin` polls completions and updates event-runtime concurrency reservations.

`AudioEmitter` synchronizes moving ECS sources without restarting the voice, and the material-aware occlusion provider can feed the same update path. `AudioZoneVolume` is an oriented box; highest-priority overlapping zones blend their environmental acoustics.

## Validation

Run from the repository root on a Rust toolchain. First run an ordinary Cargo command to resolve the newly added optional dependency and update `Cargo.lock`; do not use `--locked` until that updated lockfile is committed.

```powershell
cargo fmt --all -- --check
cargo check -p ribbon_plugin_audio --all-targets
cargo test -p ribbon_plugin_audio --all-targets --no-fail-fast
cargo clippy -p ribbon_plugin_audio --all-targets -- -D warnings
cargo test -p ribbon_plugin_audio --test decoder_integration -- --nocapture
cargo test -p ribbon_plugin_audio --no-default-features
```

After those pass, run `cargo check --workspace --all-targets`, `cargo test --workspace --no-fail-fast`, `cargo clippy --workspace --all-targets -- -D warnings` and `py -3 tools/check_deps.py`.

The optional native integration requires a separate feature build on a machine with the documented Steam Audio prerequisites:

```powershell
cargo check -p ribbon_plugin_audio --features steam-audio --all-targets
cargo test -p ribbon_plugin_audio --features steam-audio --all-targets
```

The editing container used for this overlay has no Rust toolchain, so Cargo formatting/build/test/clippy commands could not be executed here. These commands are validation instructions, not claimed successes.
