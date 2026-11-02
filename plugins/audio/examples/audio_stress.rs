//! Runtime stress test for Ribbon Audio + Steam Audio.
//! Run with:
//! cargo run --release -p ribbon_plugin_audio --features steam-audio --example audio_stress

#[cfg(not(feature = "steam-audio"))]
compile_error!("audio_stress requires the steam-audio feature");

use std::f32::consts::TAU;
use std::thread;
use std::time::{Duration, Instant};

use ribbon_plugin_audio::{
    AudioBus, AudioDiagnostics, ManagedEmitterAcoustics, PlaybackOptions, SoundAsset, SoundBank,
    SoundId, ThreadedAudioSink, VoiceId,
};

const TONE: &[u8] = include_bytes!("assets/tone.wav");
const VOICE_GAIN: f32 = 0.0005;
const QUEUE_RETRY_LIMIT: usize = 500;

fn bank() -> Result<SoundBank, String> {
    SoundBank::try_new(
        vec![SoundAsset {
            id: SoundId(1),
            data: TONE,
            gain: 1.0,
            speed: 1.0,
            spatial: true,
        }],
        vec![],
    )
}

fn direction(angle: f32) -> [f32; 3] {
    [angle.sin(), 0.0, -angle.cos()]
}

fn queue_is_full(error: &str) -> bool {
    error.contains("queue is full")
}

fn take_snapshot(sink: &ThreadedAudioSink, retries: &mut u64) -> Result<AudioDiagnostics, String> {
    let deadline = Instant::now() + Duration::from_secs(15);

    loop {
        match sink.request_diagnostics() {
            Ok(receiver) => {
                return receiver
                    .recv_timeout(Duration::from_secs(15))
                    .map_err(|error| format!("Diagnostics worker response failed: {error}"));
            }
            Err(error) if queue_is_full(&error) && Instant::now() < deadline => {
                *retries += 1;
                thread::sleep(Duration::from_millis(2));
            }
            Err(error) => return Err(error),
        }
    }
}

fn submit_voice(
    sink: &ThreadedAudioSink,
    angle: f32,
    retries: &mut u64,
) -> Result<VoiceId, String> {
    for attempt in 0..=QUEUE_RETRY_LIMIT {
        let options = PlaybackOptions {
            gain: VOICE_GAIN,
            pan: 0.0,
            looping: true,
            priority: 128,
            bus: AudioBus::Sfx,
            fade_in_seconds: 0.02,
            binaural_direction: Some(direction(angle)),
            ..PlaybackOptions::default()
        };

        match sink.play_voice(SoundId(1), options) {
            Ok(voice) => return Ok(voice),
            Err(error) if queue_is_full(&error) && attempt < QUEUE_RETRY_LIMIT => {
                *retries += 1;
                thread::sleep(Duration::from_millis(2));
            }
            Err(error) => return Err(error),
        }
    }

    Err("Voice submission retry limit reached".to_owned())
}

fn submit_update(
    sink: &ThreadedAudioSink,
    voice: VoiceId,
    index: usize,
    round: usize,
    total: usize,
    retries: &mut u64,
) -> Result<(), String> {
    let angle = (index as f32 / total as f32) * TAU + round as f32 * 0.31;

    let occlusion = if (index + round).is_multiple_of(3) {
        0.85
    } else {
        0.0
    };

    let acoustics = ManagedEmitterAcoustics {
        gain: VOICE_GAIN,
        pan: 0.0,
        playback_rate: 1.0,
        occlusion,
        transmission_gain: 1.0,
        low_pass_cutoff_hz: if occlusion > 0.5 { 1_800.0 } else { 20_000.0 },
        binaural_direction: Some(direction(angle)),
    };

    for attempt in 0..=QUEUE_RETRY_LIMIT {
        match sink.update_voice_spatial_acoustics(voice, acoustics) {
            Ok(()) => return Ok(()),
            Err(error) if queue_is_full(&error) && attempt < QUEUE_RETRY_LIMIT => {
                *retries += 1;
                thread::sleep(Duration::from_millis(2));
            }
            Err(error) => return Err(error),
        }
    }

    Err("HRTF update retry limit reached".to_owned())
}

fn run_stages(
    sink: &ThreadedAudioSink,
    voices: &mut Vec<VoiceId>,
    retries: &mut u64,
) -> Result<bool, String> {
    for target in [32usize, 64, 128] {
        println!("\n========== TARGET: {target} VOICES ==========");

        let stage_start = Instant::now();
        let failures_before = sink.backend_failures();
        let dropped_before = sink.dropped_commands();

        let play_start = Instant::now();

        while voices.len() < target {
            let index = voices.len();
            let angle = index as f32 / target as f32 * TAU;

            match submit_voice(sink, angle, retries) {
                Ok(voice) => voices.push(voice),
                Err(error) => {
                    eprintln!("Voice submission failed: {error}");
                    return Ok(false);
                }
            }
        }

        let play_submission_ms = play_start.elapsed().as_secs_f64() * 1000.0;

        // Diagnostics is queued after all Play commands. Its response
        // therefore observes the backend after processing those commands.
        let barrier_start = Instant::now();
        let started = take_snapshot(sink, retries)?;
        let play_barrier_ms = barrier_start.elapsed().as_secs_f64() * 1000.0;

        println!("Requested voices       : {target}");
        println!("Submitted voice IDs    : {}", voices.len());
        println!("Active after Play      : {}", started.active_voices);
        println!("Backend voice capacity : {}", started.max_voices);
        println!("Play submission ms    : {play_submission_ms:.2}");
        println!("Play queue barrier ms : {play_barrier_ms:.2}");

        if started.active_voices != target {
            eprintln!(
                "Voice admission mismatch: expected {target}, got {}",
                started.active_voices
            );
            return Ok(false);
        }

        // Increase update frequency with the number of live voices.
        let rounds = target / 4;
        let update_start = Instant::now();
        let mut accepted_updates = 0usize;
        let mut update_snapshot = started.clone();

        for round in 0..rounds {
            for (index, voice) in voices.iter().copied().enumerate() {
                submit_update(sink, voice, index, round, target, retries)?;
                accepted_updates += 1;
            }

            // Barrier after each update wave, so commands are processed
            // while the audio output continues rendering.
            update_snapshot = take_snapshot(sink, retries)?;

            if update_snapshot.active_voices != target {
                eprintln!(
                    "Active voice count changed during HRTF updates: {}",
                    update_snapshot.active_voices
                );
                return Ok(false);
            }
        }

        let update_submission_ms = update_start.elapsed().as_secs_f64() * 1000.0;

        // Keep all voices alive during an additional rendering window.
        thread::sleep(Duration::from_millis(300));
        let rendered = take_snapshot(sink, retries)?;

        let backend_failure_delta = sink.backend_failures().saturating_sub(failures_before);

        let dropped_delta = sink.dropped_commands().saturating_sub(dropped_before);

        let expected_updates = target * rounds;

        let stage_passed = started.active_voices == target
            && update_snapshot.active_voices == target
            && rendered.active_voices == target
            && accepted_updates == expected_updates
            && backend_failure_delta == 0
            && rendered.decode_failures == 0;

        println!("HRTF update rounds     : {rounds}");
        println!("Accepted HRTF updates  : {accepted_updates}");
        println!("Expected HRTF updates  : {expected_updates}");
        println!("Active after updates   : {}", update_snapshot.active_voices);
        println!("Active after rendering : {}", rendered.active_voices);
        println!("Update elapsed ms      : {update_submission_ms:.2}");
        println!("Backend failure delta  : {backend_failure_delta}");
        println!("Queue rejection delta  : {dropped_delta}");
        println!("Queue retries total    : {}", *retries);
        println!(
            "Decoded static PCM MiB: {:.3}",
            rendered.decoded_static_pcm_bytes as f64 / 1_048_576.0
        );
        println!(
            "Peak decode MiB        : {:.3}",
            rendered.peak_static_decode_bytes as f64 / 1_048_576.0
        );
        println!("Device recoveries      : {}", rendered.device_recoveries);
        println!(
            "Stage elapsed ms       : {:.2}",
            stage_start.elapsed().as_secs_f64() * 1000.0
        );

        if let Some(error) = sink.last_backend_error() {
            println!("Last backend error     : {error}");
        }

        println!(
            "STAGE RESULT           : {}",
            if stage_passed { "PASS" } else { "FAIL" }
        );

        if !stage_passed {
            return Ok(false);
        }
    }

    Ok(true)
}

fn cleanup(
    sink: &ThreadedAudioSink,
    voices: &[VoiceId],
    retries: &mut u64,
) -> Result<bool, String> {
    println!("\n========== CLEANUP ==========");

    let mut stop_errors = 0usize;

    for voice in voices.iter().copied() {
        let mut stopped = false;

        for attempt in 0..=QUEUE_RETRY_LIMIT {
            match sink.stop_voice(voice, 0.02) {
                Ok(()) => {
                    stopped = true;
                    break;
                }
                Err(error) if queue_is_full(&error) && attempt < QUEUE_RETRY_LIMIT => {
                    *retries += 1;
                    thread::sleep(Duration::from_millis(2));
                }
                Err(error) => {
                    eprintln!("Stop failed for {voice:?}: {error}");
                    break;
                }
            }
        }

        if !stopped {
            stop_errors += 1;
        }
    }

    let deadline = Instant::now() + Duration::from_secs(4);
    let mut final_state = take_snapshot(sink, retries)?;

    while (final_state.active_voices != 0 || final_state.retiring_voices != 0)
        && Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(50));
        final_state = take_snapshot(sink, retries)?;
    }

    println!("Voices submitted       : {}", voices.len());
    println!("Stop errors            : {stop_errors}");
    println!("Remaining active       : {}", final_state.active_voices);
    println!("Remaining retiring     : {}", final_state.retiring_voices);
    println!("Backend failures       : {}", sink.backend_failures());
    println!("Decode failures        : {}", final_state.decode_failures);
    println!("Total dropped commands : {}", sink.dropped_commands());
    println!("Total queue retries    : {}", *retries);

    Ok(stop_errors == 0
        && final_state.active_voices == 0
        && final_state.retiring_voices == 0
        && sink.backend_failures() == 0
        && final_state.decode_failures == 0)
}

fn main() -> Result<(), String> {
    println!("==================================================");
    println!("RIBBON AUDIO — STEAM AUDIO STRESS TEST");
    println!("==================================================");

    let bank = bank()?;

    let sink = ThreadedAudioSink::new(bank)
        .map_err(|error| format!("Audio initialization failed: {error}"))?;

    let mut voices = Vec::with_capacity(128);
    let mut retries = 0u64;

    let stages_passed = match run_stages(&sink, &mut voices, &mut retries) {
        Ok(passed) => passed,
        Err(error) => {
            eprintln!("Stress stage error: {error}");
            false
        }
    };

    // Always attempt cleanup, even when one of the stages fails.
    let cleanup_passed = match cleanup(&sink, &voices, &mut retries) {
        Ok(passed) => passed,
        Err(error) => {
            eprintln!("Cleanup error: {error}");
            false
        }
    };

    let passed = stages_passed && cleanup_passed;

    println!("\n==================================================");
    println!("OVERALL RESULT: {}", if passed { "PASS" } else { "FAIL" });
    println!("==================================================");

    if passed {
        Ok(())
    } else {
        Err("Audio stress test did not pass all stages".to_owned())
    }
}
