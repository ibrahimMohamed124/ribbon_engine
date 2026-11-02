//! Runtime priority admission, voice stealing and sustained HRTF stress test.
//!
//! Run:
//! cargo run --release -p ribbon_plugin_audio --features steam-audio --example audio_priority_stress

#[cfg(not(feature = "steam-audio"))]
compile_error!("This example requires steam-audio.");

use std::collections::HashSet;
use std::f32::consts::TAU;
use std::thread;
use std::time::{Duration, Instant};

use ribbon_plugin_audio::{
    AudioBus, AudioDiagnostics, ManagedEmitterAcoustics, PlaybackOptions, PlaybackStatus,
    SoundAsset, SoundBank, SoundId, ThreadedAudioSink, VoiceCompletions, VoiceId,
};

const TONE: &[u8] = include_bytes!("assets/tone.wav");

const VOICE_CAPACITY: usize = 128;
const BASE_PRIORITY: u8 = 80;
const FIRST_STEAL_PRIORITY: u8 = 81;
const CHURN_PRIORITY: u8 = 200;
const EXTRA_STEALS: usize = 32;
const UPDATE_ROUNDS: usize = 8;
const VOICE_GAIN: f32 = 0.0005;

fn ensure(condition: bool, message: impl Into<String>) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

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

fn options(priority: u8, angle: f32) -> PlaybackOptions {
    PlaybackOptions {
        gain: VOICE_GAIN,
        playback_rate: 1.0,
        pan: 0.0,
        looping: true,
        priority,
        bus: AudioBus::Sfx,
        fade_in_seconds: 0.02,
        binaural_direction: Some(direction(angle)),
        ..PlaybackOptions::default()
    }
}

fn start_voice(sink: &ThreadedAudioSink, priority: u8, index: usize) -> Result<VoiceId, String> {
    let angle = index as f32 * 0.173;
    sink.play_voice(SoundId(1), options(priority, angle))
}

fn snapshot(sink: &ThreadedAudioSink) -> Result<AudioDiagnostics, String> {
    sink.request_diagnostics()?
        .recv_timeout(Duration::from_secs(15))
        .map_err(|error| format!("Diagnostics response timed out: {error}"))
}

fn voice_state(sink: &ThreadedAudioSink, voice: VoiceId) -> Result<Option<PlaybackStatus>, String> {
    sink.request_voice_state(voice)?
        .recv_timeout(Duration::from_secs(15))
        .map_err(|error| format!("Voice-state response timed out: {error}"))
}

fn take_completions(
    sink: &ThreadedAudioSink,
    seen: &mut HashSet<VoiceId>,
    dropped_max: &mut u64,
) -> Result<VoiceCompletions, String> {
    let batch = sink
        .request_completed_voices()?
        .recv_timeout(Duration::from_secs(15))
        .map_err(|error| format!("Completion response timed out: {error}"))?;

    seen.extend(batch.voices.iter().copied());

    *dropped_max = (*dropped_max).max(batch.dropped_notifications_total);

    Ok(batch)
}

fn verify_failure_count(
    sink: &ThreadedAudioSink,
    expected: u64,
    context: &str,
) -> Result<(), String> {
    let actual = sink.backend_failures();

    ensure(
        actual == expected,
        format!(
            "{context}: expected {expected} backend failures, got {actual}; last error: {:?}",
            sink.last_backend_error()
        ),
    )
}

fn run_priority_test(
    sink: &ThreadedAudioSink,
    seen_completed: &mut HashSet<VoiceId>,
    expected_completed: &mut HashSet<VoiceId>,
    dropped_notifications: &mut u64,
    baseline_failures: u64,
) -> Result<(), String> {
    println!("\n========== STAGE 1: FILL VOICE BUDGET ==========");

    let mut original_voices = Vec::with_capacity(VOICE_CAPACITY);

    let start = Instant::now();

    for index in 0..VOICE_CAPACITY {
        let voice = start_voice(sink, BASE_PRIORITY, index)?;
        original_voices.push(voice);
    }

    let initial = snapshot(sink)?;

    println!("Submitted voices : {}", original_voices.len());
    println!("Active voices    : {}", initial.active_voices);
    println!("Voice capacity   : {}", initial.max_voices);
    println!(
        "Submission + barrier ms: {:.2}",
        start.elapsed().as_secs_f64() * 1000.0
    );

    ensure(
        initial.max_voices == VOICE_CAPACITY,
        "Backend voice capacity differs from the expected 128.",
    )?;

    ensure(
        initial.active_voices == VOICE_CAPACITY,
        "Could not establish 128 simultaneously active voices.",
    )?;

    ensure(
        initial.retiring_voices == 0,
        "Unexpected retiring voices before priority testing.",
    )?;

    ensure(
        voice_state(sink, original_voices[0])? == Some(PlaybackStatus::Playing),
        "Oldest baseline voice did not enter Playing state.",
    )?;

    ensure(
        voice_state(sink, original_voices[VOICE_CAPACITY - 1])? == Some(PlaybackStatus::Playing),
        "Newest baseline voice did not enter Playing state.",
    )?;

    println!("STAGE 1: PASS");

    // -----------------------------------------------------------
    // STAGE 2: Equal-priority admission must be rejected.
    // -----------------------------------------------------------

    println!("\n========== STAGE 2: EQUAL PRIORITY REJECTION ==========");

    let equal_rejected = start_voice(sink, BASE_PRIORITY, 1000)?;

    let equal_snapshot = snapshot(sink)?;

    verify_failure_count(sink, baseline_failures + 1, "Equal-priority rejection")?;

    ensure(
        equal_snapshot.active_voices == VOICE_CAPACITY,
        "Equal-priority rejection changed the active voice count.",
    )?;

    ensure(
        voice_state(sink, equal_rejected)?.is_none(),
        "Rejected equal-priority voice unexpectedly became active.",
    )?;

    ensure(
        voice_state(sink, original_voices[0])? == Some(PlaybackStatus::Playing),
        "Equal-priority rejection incorrectly displaced the oldest voice.",
    )?;

    expected_completed.insert(equal_rejected);

    let equal_batch = take_completions(sink, seen_completed, dropped_notifications)?;

    ensure(
        seen_completed.contains(&equal_rejected),
        "Rejected equal-priority voice did not generate a completion notification.",
    )?;

    ensure(
        equal_batch
            .active_voices
            .as_ref()
            .is_some_and(|ids| !ids.contains(&equal_rejected)),
        "Rejected voice appeared in the backend live-ID snapshot.",
    )?;

    println!("Equal priority rejected : PASS");
    println!("Active voices preserved : {}", equal_snapshot.active_voices);

    // -----------------------------------------------------------
    // STAGE 3: Lower-priority admission must be rejected.
    // -----------------------------------------------------------

    println!("\n========== STAGE 3: LOWER PRIORITY REJECTION ==========");

    let low_rejected = start_voice(sink, BASE_PRIORITY - 1, 1001)?;

    let low_snapshot = snapshot(sink)?;

    verify_failure_count(sink, baseline_failures + 2, "Lower-priority rejection")?;

    ensure(
        low_snapshot.active_voices == VOICE_CAPACITY,
        "Lower-priority rejection changed the active voice count.",
    )?;

    ensure(
        voice_state(sink, low_rejected)?.is_none(),
        "Rejected lower-priority voice unexpectedly became active.",
    )?;

    ensure(
        voice_state(sink, original_voices[0])? == Some(PlaybackStatus::Playing),
        "Lower-priority rejection displaced the oldest voice.",
    )?;

    expected_completed.insert(low_rejected);

    let low_batch = take_completions(sink, seen_completed, dropped_notifications)?;

    ensure(
        seen_completed.contains(&low_rejected),
        "Rejected lower-priority voice did not generate a completion notification.",
    )?;

    ensure(
        low_batch
            .active_voices
            .as_ref()
            .is_some_and(|ids| !ids.contains(&low_rejected)),
        "Rejected lower-priority voice appeared in the backend live-ID snapshot.",
    )?;

    println!("Lower priority rejected: PASS");
    println!("Both rejection notifications received: PASS");

    // -----------------------------------------------------------
    // STAGE 4: Higher-priority voice steals the oldest equal-priority
    // baseline voice.
    // -----------------------------------------------------------

    println!("\n========== STAGE 4: DETERMINISTIC PRIORITY STEAL ==========");

    let first_new = start_voice(sink, FIRST_STEAL_PRIORITY, 2000)?;

    let stolen_snapshot = snapshot(sink)?;

    verify_failure_count(sink, baseline_failures + 2, "First successful steal")?;

    ensure(
        stolen_snapshot.active_voices == VOICE_CAPACITY,
        "Voice count changed after priority stealing.",
    )?;

    let first_victim = original_voices[0];

    ensure(
        voice_state(sink, first_victim)?.is_none(),
        "The oldest baseline voice was not removed from the active registry.",
    )?;

    ensure(
        voice_state(sink, original_voices[1])? == Some(PlaybackStatus::Playing),
        "Priority stealing selected the wrong victim; second-oldest voice was affected.",
    )?;

    ensure(
        voice_state(sink, first_new)? == Some(PlaybackStatus::Playing),
        "Higher-priority replacement did not enter Playing state.",
    )?;

    expected_completed.insert(first_victim);

    let steal_batch = take_completions(sink, seen_completed, dropped_notifications)?;

    let victim_still_tracked = steal_batch
        .active_voices
        .as_ref()
        .is_some_and(|ids| ids.contains(&first_victim));

    let victim_already_completed = seen_completed.contains(&first_victim);

    ensure(
        victim_still_tracked || victim_already_completed,
        "Stolen voice was neither tracked as retiring nor reported as completed.",
    )?;

    println!("Victim ID       : {first_victim:?}");
    println!("Replacement ID  : {first_new:?}");
    println!("Active voices   : {}", stolen_snapshot.active_voices);
    println!("Retiring voices : {}", stolen_snapshot.retiring_voices);
    println!("Oldest-first victim selection: PASS");
    println!("Replacement admission: PASS");

    // Track exactly which voices should remain active.
    let mut active_ids = original_voices[1..].to_vec();
    active_ids.push(first_new);

    ensure(
        active_ids.len() == VOICE_CAPACITY,
        "Internal active-ID tracking is inconsistent after first steal.",
    )?;

    // -----------------------------------------------------------
    // STAGE 5: Repeated priority stealing.
    // -----------------------------------------------------------

    println!("\n========== STAGE 5: REPEATED PRIORITY CHURN ==========");

    let mut expected_victims = vec![first_victim];
    let churn_start = Instant::now();

    for iteration in 0..EXTRA_STEALS {
        // All initial voices have priority 80. The previously inserted
        // replacements have higher priority and cannot become victims
        // while baseline voices remain.
        let expected_victim = original_voices[iteration + 1];

        let replacement = start_voice(sink, CHURN_PRIORITY, 3000 + iteration)?;

        let current = snapshot(sink)?;

        verify_failure_count(sink, baseline_failures + 2, "Repeated priority churn")?;

        ensure(
            current.active_voices == VOICE_CAPACITY,
            format!("Active voice budget changed during churn iteration {iteration}."),
        )?;

        ensure(
            current.retiring_voices <= VOICE_CAPACITY,
            "Retiring voice budget exceeded its bound.",
        )?;

        ensure(
            voice_state(sink, expected_victim)?.is_none(),
            format!(
                "Expected oldest low-priority victim {:?} was not removed.",
                expected_victim
            ),
        )?;

        ensure(
            voice_state(sink, replacement)? == Some(PlaybackStatus::Playing),
            format!("Replacement voice {:?} did not become active.", replacement),
        )?;

        let previous_len = active_ids.len();

        active_ids.retain(|voice| *voice != expected_victim);

        ensure(
            active_ids.len() + 1 == previous_len,
            "Active-ID tracking did not contain the expected victim exactly once.",
        )?;

        active_ids.push(replacement);
        expected_victims.push(expected_victim);
        expected_completed.insert(expected_victim);

        if (iteration + 1) % 8 == 0 {
            let completed_batch = take_completions(sink, seen_completed, dropped_notifications)?;

            println!(
                "Completed {}/{} replacement operations; active={}, retiring={}, completion drops={}",
                iteration + 1,
                EXTRA_STEALS,
                current.active_voices,
                current.retiring_voices,
                completed_batch.dropped_notifications_total,
            );
        }
    }

    ensure(
        active_ids.len() == VOICE_CAPACITY,
        "Active voice tracking lost its 128-voice invariant.",
    )?;

    println!("Repeated steal operations: {}", EXTRA_STEALS + 1);
    println!(
        "Churn elapsed ms: {:.2}",
        churn_start.elapsed().as_secs_f64() * 1000.0
    );
    println!("STAGE 5: PASS");

    // -----------------------------------------------------------
    // STAGE 6: Sustained HRTF and acoustic updates.
    // -----------------------------------------------------------

    println!("\n========== STAGE 6: SUSTAINED HRTF LOAD ==========");

    let mut accepted_updates = 0usize;
    let sustained_start = Instant::now();

    for round in 0..UPDATE_ROUNDS {
        for (index, voice) in active_ids.iter().copied().enumerate() {
            let angle = index as f32 / VOICE_CAPACITY as f32 * TAU + round as f32 * 0.29;

            let occlusion = if (index + round) % 3 == 0 { 0.85 } else { 0.0 };

            let acoustics = ManagedEmitterAcoustics {
                gain: VOICE_GAIN,
                pan: 0.0,
                playback_rate: 1.0,
                occlusion,
                transmission_gain: 1.0,
                low_pass_cutoff_hz: if occlusion > 0.5 { 1_800.0 } else { 20_000.0 },
                binaural_direction: Some(direction(angle)),
            };

            sink.update_voice_spatial_acoustics(voice, acoustics)?;
            accepted_updates += 1;
        }

        // The diagnostics command acts as a worker-queue barrier.
        let current = snapshot(sink)?;

        ensure(
            current.active_voices == VOICE_CAPACITY,
            format!("Voice loss detected during HRTF round {round}."),
        )?;

        verify_failure_count(sink, baseline_failures + 2, "Sustained HRTF updates")?;

        let _ = take_completions(sink, seen_completed, dropped_notifications)?;

        println!(
            "HRTF round {:>2}/{}: active={}, retiring={}, accepted updates={}",
            round + 1,
            UPDATE_ROUNDS,
            current.active_voices,
            current.retiring_voices,
            accepted_updates,
        );
    }

    println!("Accepted HRTF updates : {accepted_updates}");
    println!(
        "Sustained load ms     : {:.2}",
        sustained_start.elapsed().as_secs_f64() * 1000.0
    );

    // Every retired voice should complete after its fade.
    println!("\n========== VERIFY RETIREMENT COMPLETIONS ==========");

    let deadline = Instant::now() + Duration::from_secs(4);

    while !expected_victims
        .iter()
        .all(|voice| seen_completed.contains(voice))
    {
        let _ = take_completions(sink, seen_completed, dropped_notifications)?;

        if expected_victims
            .iter()
            .all(|voice| seen_completed.contains(voice))
        {
            break;
        }

        if Instant::now() >= deadline {
            let missing: Vec<_> = expected_victims
                .iter()
                .filter(|voice| !seen_completed.contains(voice))
                .copied()
                .collect();

            return Err(format!(
                "Retired voice completion notifications timed out: {missing:?}"
            ));
        }

        thread::sleep(Duration::from_millis(25));
    }

    let final_load = snapshot(sink)?;

    ensure(
        final_load.active_voices == VOICE_CAPACITY,
        "Active voice count changed after sustained load.",
    )?;

    verify_failure_count(sink, baseline_failures + 2, "End of sustained test")?;

    println!("Expected victims completed: {}", expected_victims.len());
    println!("Retirement notifications : PASS");
    println!("Completion overflow      : {dropped_notifications}");
    println!("STAGE 6: PASS");

    Ok(())
}

fn cleanup_backend(
    sink: &ThreadedAudioSink,
    seen_completed: &mut HashSet<VoiceId>,
    expected_completed: &mut HashSet<VoiceId>,
    dropped_notifications: &mut u64,
) -> Result<AudioDiagnostics, String> {
    println!("\n========== CLEANUP ==========");

    // Retrieve the actual backend-owned set, including any voices
    // currently waiting for their retirement fade.
    let batch = take_completions(sink, seen_completed, dropped_notifications)?;

    let live_ids = batch
        .active_voices
        .ok_or_else(|| "Backend did not return its live voice IDs.".to_owned())?;

    expected_completed.extend(live_ids.iter().copied());

    let mut stop_errors = 0usize;

    for voice in live_ids.iter().copied() {
        if let Err(error) = sink.stop_voice(voice, 0.02) {
            stop_errors += 1;
            eprintln!("Stop submission failed for {voice:?}: {error}");
        }
    }

    ensure(
        stop_errors == 0,
        format!("Could not submit {stop_errors} stop commands."),
    )?;

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut current = snapshot(sink)?;

    while current.active_voices != 0 || current.retiring_voices != 0 {
        if Instant::now() >= deadline {
            return Err(format!(
                "Cleanup timed out: active={}, retiring={}",
                current.active_voices, current.retiring_voices
            ));
        }

        thread::sleep(Duration::from_millis(30));

        let _ = take_completions(sink, seen_completed, dropped_notifications)?;

        current = snapshot(sink)?;
    }

    // Drain the notifications which may have been produced by the
    // final diagnostics cleanup itself.
    let _ = take_completions(sink, seen_completed, dropped_notifications)?;

    let final_state = snapshot(sink)?;

    ensure(
        final_state.active_voices == 0 && final_state.retiring_voices == 0,
        "Audio backend still owns voices after cleanup.",
    )?;

    println!("Live IDs at cleanup : {}", live_ids.len());
    println!("Stop submission errors: {stop_errors}");
    println!("Remaining active    : {}", final_state.active_voices);
    println!("Remaining retiring  : {}", final_state.retiring_voices);

    Ok(final_state)
}

fn main() -> Result<(), String> {
    println!("==================================================");
    println!("RIBBON AUDIO — PRIORITY AND SUSTAINED LOAD TEST");
    println!("==================================================");

    let sink = ThreadedAudioSink::new(bank()?)
        .map_err(|error| format!("Audio initialization failed: {error}"))?;

    let baseline_failures = sink.backend_failures();
    let baseline_dropped_commands = sink.dropped_commands();

    let mut seen_completed = HashSet::new();
    let mut expected_completed = HashSet::new();
    let mut dropped_notifications = 0u64;

    let stress_result = run_priority_test(
        &sink,
        &mut seen_completed,
        &mut expected_completed,
        &mut dropped_notifications,
        baseline_failures,
    );

    if let Err(error) = &stress_result {
        eprintln!("\nSTRESS TEST ERROR: {error}");
    }

    // Cleanup is attempted even if a test stage fails.
    let cleanup_result = cleanup_backend(
        &sink,
        &mut seen_completed,
        &mut expected_completed,
        &mut dropped_notifications,
    );

    let cleanup_passed = match &cleanup_result {
        Ok(state) => {
            println!("Cleanup result: PASS");
            state.active_voices == 0 && state.retiring_voices == 0
        }
        Err(error) => {
            eprintln!("Cleanup result: FAIL — {error}");
            false
        }
    };

    let final_failures = sink.backend_failures();
    let final_dropped_commands = sink.dropped_commands();

    let missing_completions: Vec<_> = expected_completed
        .iter()
        .filter(|voice| !seen_completed.contains(voice))
        .copied()
        .collect();

    let expected_failures = baseline_failures + 2;

    println!("\n========== FINAL REPORT ==========");
    println!("Intentional admission rejections: 2");
    println!("Backend failures expected/actual: {expected_failures}/{final_failures}");
    println!("Dropped commands before/after: {baseline_dropped_commands}/{final_dropped_commands}");
    println!("Completion notifications dropped: {dropped_notifications}");
    println!("Expected completion IDs: {}", expected_completed.len());
    println!("Received completion IDs: {}", seen_completed.len());
    println!("Missing completion IDs: {}", missing_completions.len());

    if !missing_completions.is_empty() {
        println!("Missing IDs: {missing_completions:?}");
    }

    if let Some(error) = sink.last_backend_error() {
        println!("Last backend error: {error}");
    }

    let passed = stress_result.is_ok()
        && cleanup_passed
        && final_failures == expected_failures
        && final_dropped_commands == baseline_dropped_commands
        && dropped_notifications == 0
        && missing_completions.is_empty();

    println!("\n==================================================");
    println!("OVERALL RESULT: {}", if passed { "PASS" } else { "FAIL" });
    println!("==================================================");

    if passed {
        Ok(())
    } else {
        Err("Priority and sustained-load validation failed.".to_owned())
    }
}
