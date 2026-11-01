# Ribbon Audio continuation overlay — snapshot, borrow-check, emitter lifecycle, and Clippy fixes

This is a cumulative audio-only overlay relative to the uploaded `Ribbon_Engine(4).zip` baseline. It includes the prior audio continuation work, the snapshot DSP transition fix, the follow-up compiler fixes, and the latest moving-emitter lifecycle / Clippy corrections. No unrelated engine files are included.

## Latest test correction from the Windows run

- `ecs.rs`: clarified `moving_emitter_reuses_one_voice_and_stops_when_despawned` to assert the lifecycle in two stages. Creating a managed voice intentionally applies its initial acoustic state once; moving that existing voice applies a second update. The prior assertion incorrectly expected only one update total, even though the implementation correctly performed both initialization and movement updates. The test now asserts one start and one initial update after frame one, then still one start and two total updates after movement, followed by one stop on despawn.

## Latest fixes from the Windows test output

- `ecs.rs`: changed stale-emitter collection to `filter_map` so the `HashSet::contains` call receives an `&Entity` directly. This prevents a live emitter's runtime voice from being incorrectly classified as stale and restarted during the next ECS update. The existing `moving_emitter_reuses_one_voice_and_stops_when_despawned` regression test continues to assert one start, one update, and one stop on despawn; its fake sink now explicitly advertises managed-voice support.
- `ecs.rs`: removed the unused non-oriented `AudioZoneVolume::weight` wrapper. The zone-weight test now exercises `weight_oriented` with the default orientation, preserving coverage without a dead-code warning.
- `lib.rs`: introduced `ManagedEmitterAcoustics`, a named value type grouping the live voice's gain, pan, playback rate, occlusion, material transmission gain, and low-pass cutoff.
- `kira_backend.rs`, `threaded.rs`, and `lib.rs`: routed the spatial-acoustics update path through `ManagedEmitterAcoustics`. This removes the four `too_many_arguments` Clippy diagnostics while keeping validation, clamping, gain composition, and worker-queue semantics explicit. The threaded command now carries the typed value as one payload.

## Earlier compiler fixes retained

- `automation.rs`: replaced the `&mut self` helper call made while iterating `self.bindings` with a field-scoped helper that mutably borrows only `self.bus_effects`. This resolves the reported E0502 aliasing conflict without cloning bindings or changing automation order/state behavior.
- `ecs.rs`: explicitly typed the test's `material_cutoff` as `f32` to resolve E0689, and removed an unnecessary `mut` binding.
- `threaded.rs`: changed the deprecated `AtomicU64::fetch_update` call to `try_update`, as recommended by the compiler diagnostic. The checked, non-wrapping voice-ID allocation behavior is otherwise unchanged.

## Snapshot transition fix

`Audio::try_transition_to_snapshot` previously applied snapshot effect chains through the immediate `AudioBackend::set_effects` path, so the requested snapshot duration was ignored for DSP parameters. The backend contract now has `set_effects_faded` with an immediate default for compatibility; the Kira backend forwards the requested duration to `set_bus_effects_faded`. Snapshot validation still happens before any backend mutation. A regression test records and asserts the exact effect transition duration.

## Apply

Extract this ZIP over the repository root, preserving the paths. This is cumulative and replaces the audio files from the earlier continuation overlay.

## Validation status

Performed in this environment: source-scope review, targeted call-site searches, delimiter-balance checks, whitespace checks, and ZIP/content integrity checks.

Not performed here: `cargo fmt`, `cargo check`, Rust tests, Clippy, or audio-device runtime testing. This environment has no `cargo`, `rustc`, or `rustfmt` executable. The previous Windows log confirms that the earlier borrow-check fix built and 49 of 50 audio tests passed; the new emitter-lifecycle and Clippy changes in this overlay still require verification on the Windows checkout.

Run the following from the repository root after applying the overlay:

```powershell
cargo fmt --all -- --check
cargo check -p ribbon_plugin_audio --all-targets
cargo test -p ribbon_plugin_audio --all-targets --no-fail-fast
cargo clippy -p ribbon_plugin_audio --all-targets -- -D warnings
cargo test -p ribbon_plugin_audio snapshot_transition_duration_is_applied_to_effect_chains -- --nocapture
```

If those pass, continue with the workspace check/test/Clippy commands and `py -3 tools/check_deps.py`.

## Remaining known audio gaps

The previous overlay's documented limitations remain: no runtime-created arbitrary bus hierarchy/sends, no EQ/delay/distortion/convolution/true-peak limiter, no HRTF or Steam Audio, no built-in scene-material ray tracing/portal propagation, and no automatic in-place output-device loss recovery. Static decode peak memory can still exceed the retained PCM cache limit.
