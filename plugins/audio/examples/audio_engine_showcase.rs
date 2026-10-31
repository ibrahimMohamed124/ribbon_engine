//! Audible, device-optional Kira showcase. Run with:
//! `cargo run -p ribbon_plugin_audio --example audio_engine_showcase`
//! Set `RIBBON_AUDIO_STREAM_FILE` to a long supported file to exercise file-backed streaming.

use std::time::Duration;

use ribbon_plugin_audio::mixer::Audio;
use ribbon_plugin_audio::{
    AcousticZone, AmbienceAsset, AmbienceId, AudioBus, AudioEffect, AudioEventDescriptor,
    AudioEventRegistry, AudioEventRuntime, AudioParameter, AudioVec3, Emitter3d, Listener3d,
    MixerSnapshot, PlaybackOptions, SoundAsset, SoundBank, SoundId, ThreadedAudioSink,
};

const TONE: &[u8] = include_bytes!("assets/tone.wav");

fn bank() -> SoundBank {
    SoundBank::new(
        vec![
            SoundAsset {
                id: SoundId(1),
                data: TONE,
                gain: 1.0,
                speed: 1.0,
                spatial: true,
            },
            // A second stable ID backed by the same bytes demonstrates shared decoding.
            SoundAsset {
                id: SoundId(2),
                data: TONE,
                gain: 1.0,
                speed: 1.0,
                spatial: true,
            },
        ],
        vec![AmbienceAsset {
            id: AmbienceId(1),
            data: TONE,
            gain: 0.25,
        }],
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // These pure models are useful in headless builds, automated tests, and editor previews.
    let listener = Listener3d::default();
    let emitter = Emitter3d {
        position: AudioVec3::new(-3.0, 4.0, 1.5),
        min_distance: 1.0,
        max_distance: 30.0,
        rolloff: 1.1,
        gain: 0.9,
        ..Emitter3d::default()
    };
    let spatial = ribbon_plugin_audio::spatial::calculate(listener, emitter, 343.0);
    let zone = AcousticZone {
        id: 2,
        wet_mix: 0.3,
        low_pass_hz: 2_200.0,
        reverb_seconds: 1.8,
        ambience_gain: 0.8,
    }
    .sanitized();
    let stress = AudioParameter::new(0.4, 0.0, 1.0)?;
    println!(
        "spatial preview: pan={:.2}, distance={:.2}, gain={:.2}, stress={:.2}, zone wet={:.2}",
        spatial.pan, spatial.distance, spatial.gain, stress.value, zone.wet_mix
    );

    let sink = match ThreadedAudioSink::new(bank()) {
        Ok(sink) => sink,
        Err(error) => {
            // Device failures should not block a headless game or CI runner.
            eprintln!("Kira output unavailable ({error}); demonstrating the silent mixer instead.");
            let mut silent = Audio::null(1.0);
            silent.set_bus_gain(AudioBus::Sfx, 0.8);
            let mut snapshot = MixerSnapshot::default();
            snapshot.set_gain(AudioBus::Music, 0.35);
            snapshot.set_effects(
                AudioBus::Ambience,
                vec![AudioEffect::LowPass { cutoff_hz: 2_500.0 }],
            );
            silent.transition_to_snapshot(&snapshot, 0.5);
            silent.advance_transitions(0.25);
            assert!(silent.transition_active());
            silent.advance_transitions(0.25);
            assert!(!silent.transition_active());
            return Ok(());
        }
    };

    // Register and deterministically resolve an event with two non-repeating alternatives.
    let known_sounds = [SoundId(1), SoundId(2)];
    let mut event_registry = AudioEventRegistry::default();
    event_registry.register_variants(
        "player.footstep",
        AudioEventDescriptor {
            variants: known_sounds.to_vec(),
            gain_min: 0.55,
            gain_max: 0.75,
            playback_rate_min: 0.95,
            playback_rate_max: 1.05,
            priority: 90,
            max_instances: 4,
            cooldown_seconds: 0.03,
            bus: AudioBus::Sfx,
            looping: false,
        },
        &known_sounds,
    )?;
    let mut event_runtime = AudioEventRuntime::new(0xA0D1_0206_u64);
    let resolved = event_runtime.trigger(&event_registry, "player.footstep", 0.0)?;

    // Static/preloaded one-shot: source bytes are decoded once during backend initialization.
    let one_shot = sink.play_resolved_event(&resolved)?;

    // Change the actual bus track and its live DSP chain smoothly.
    sink.set_bus_gain(AudioBus::Sfx, 0.75, 0.25)?;
    sink.set_bus_effects(
        AudioBus::Sfx,
        vec![
            AudioEffect::LowPass {
                cutoff_hz: 12_000.0,
            },
            AudioEffect::Compressor {
                threshold_db: -14.0,
                ratio: 3.0,
            },
        ],
    )?;
    let mut tension = MixerSnapshot::default();
    tension.set_gain(AudioBus::Music, 0.25);
    tension.set_gain(AudioBus::Ambience, 0.85);
    tension.set_effects(
        AudioBus::Ambience,
        vec![
            AudioEffect::LowPass {
                cutoff_hz: zone.low_pass_hz,
            },
            AudioEffect::Reverb {
                wet: zone.wet_mix,
                decay_seconds: zone.reverb_seconds,
            },
        ],
    );
    sink.transition_to_snapshot(&tension, 0.6)?;

    // Independent looped ambience and a looping voice with live pause/resume/rate/stop controls.
    sink.set_ambience_layer(AmbienceId(1), 0.16, 0.2)?;
    let loop_voice = sink.play_voice(
        SoundId(1),
        PlaybackOptions {
            gain: 0.22,
            pan: -0.35,
            looping: true,
            priority: 180,
            bus: AudioBus::Ambience,
            fade_in_seconds: 0.2,
            ..PlaybackOptions::default()
        },
    )?;
    std::thread::sleep(Duration::from_millis(250));
    sink.set_voice_pan(loop_voice, 0.4, 0.15)?;
    sink.set_voice_rate(loop_voice, 0.85, 0.15)?;
    sink.pause_voice(loop_voice)?;
    std::thread::sleep(Duration::from_millis(100));
    sink.resume_voice(loop_voice)?;

    // Genuine file-backed streaming when a long file path is provided; encoded bytes otherwise
    // use Kira's streaming decoder as a self-contained demo fixture.
    let stream = if let Ok(path) = std::env::var("RIBBON_AUDIO_STREAM_FILE") {
        sink.play_streaming_file(
            path,
            PlaybackOptions {
                bus: AudioBus::Music,
                looping: true,
                gain: 0.12,
                ..PlaybackOptions::default()
            },
        )?
    } else {
        sink.play_streaming_bytes(
            TONE.to_vec(),
            PlaybackOptions {
                bus: AudioBus::Music,
                gain: 0.12,
                looping: true,
                ..PlaybackOptions::default()
            },
        )?
    };

    std::thread::sleep(Duration::from_millis(700));
    sink.stop_voice(one_shot, 0.05)?;
    event_runtime.finish(&resolved.name);
    sink.stop_voice(loop_voice, 0.12)?;
    sink.stop_voice(stream, 0.12)?;
    sink.stop_ambience_layer(AmbienceId(1), 0.12)?;
    let diagnostics = sink
        .request_diagnostics()?
        .recv_timeout(Duration::from_secs(1))?;
    println!(
        "Kira active voices={}, streaming voices={}, voice budget={}",
        diagnostics.active_voices, diagnostics.active_streaming_voices, diagnostics.max_voices
    );
    std::thread::sleep(Duration::from_millis(150));
    Ok(())
}
