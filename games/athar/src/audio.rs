//! ATHAR's small game-owned sound bank and deterministic gameplay audio events.
//!
//! All WAVs in `assets/audio` are original procedural syntheses created for this sample; they
//! contain no third-party recordings. Hardware output is optional at runtime and the gameplay
//! event layer remains testable without opening an audio device.

use ribbon_app::{LogBuffer, LogLevel, World};
use ribbon_events::WorldEventsExt;
use ribbon_plugin_audio::{
    AmbienceAsset, AmbienceId, AudioVec3, Emitter3d, PlaySpatialSound, SetAmbience, SoundAsset,
    SoundBank, SoundId,
};
#[cfg(feature = "audio")]
use ribbon_plugin_audio::{AudioOutput, ThreadedAudioSink};
use ribbon_plugin_render3d::glam::Vec3;

use crate::scripts::GamePhase;

pub const SOUND_DOOR_LOCKED: SoundId = SoundId(1);
pub const SOUND_KEY_PICKUP: SoundId = SoundId(2);
pub const SOUND_DOOR_OPEN: SoundId = SoundId(3);
pub const SOUND_DOOR_CLOSE: SoundId = SoundId(4);
pub const SOUND_FOOTSTEP_WALK: SoundId = SoundId(5);
pub const SOUND_FOOTSTEP_SPRINT: SoundId = SoundId(6);
pub const SOUND_SAVE_POINT: SoundId = SoundId(7);
pub const SOUND_MONSTER_RUSTLE: SoundId = SoundId(8);
pub const AMBIENCE_INDOOR: AmbienceId = AmbienceId(1);
pub const AMBIENCE_RAIN: AmbienceId = AmbienceId(2);

pub const ATHAR_AUDIO_ASSETS: [&str; 10] = [
    "ambience_indoor.wav",
    "ambience_rain.wav",
    "door_creak_close.wav",
    "door_creak_open.wav",
    "door_locked.wav",
    "footstep_sprint.wav",
    "footstep_walk.wav",
    "key_pickup.wav",
    "monster_rustle.wav",
    "save_point.wav",
];

/// State used to produce cadence-based footsteps and one ambience transition per game phase.
#[derive(Default)]
pub struct GameplayAudioState {
    last_phase: Option<GamePhase>,
    last_player_position: Option<Vec3>,
    footstep_distance: f32,
}

pub fn sound_bank() -> SoundBank {
    SoundBank::new(
        vec![
            SoundAsset {
                id: SOUND_DOOR_LOCKED,
                data: include_bytes!("../assets/audio/door_locked.wav"),
                gain: 0.72,
                speed: 1.0,
                spatial: true,
            },
            SoundAsset {
                id: SOUND_KEY_PICKUP,
                data: include_bytes!("../assets/audio/key_pickup.wav"),
                gain: 0.78,
                speed: 1.0,
                spatial: true,
            },
            SoundAsset {
                id: SOUND_DOOR_OPEN,
                data: include_bytes!("../assets/audio/door_creak_open.wav"),
                gain: 0.82,
                speed: 1.0,
                spatial: true,
            },
            SoundAsset {
                id: SOUND_DOOR_CLOSE,
                data: include_bytes!("../assets/audio/door_creak_close.wav"),
                gain: 0.82,
                speed: 1.0,
                spatial: true,
            },
            SoundAsset {
                id: SOUND_FOOTSTEP_WALK,
                data: include_bytes!("../assets/audio/footstep_walk.wav"),
                gain: 0.45,
                speed: 1.0,
                spatial: true,
            },
            SoundAsset {
                id: SOUND_FOOTSTEP_SPRINT,
                data: include_bytes!("../assets/audio/footstep_sprint.wav"),
                gain: 0.50,
                speed: 1.0,
                spatial: true,
            },
            SoundAsset {
                id: SOUND_SAVE_POINT,
                data: include_bytes!("../assets/audio/save_point.wav"),
                gain: 0.80,
                speed: 1.0,
                spatial: true,
            },
            SoundAsset {
                id: SOUND_MONSTER_RUSTLE,
                data: include_bytes!("../assets/audio/monster_rustle.wav"),
                gain: 0.62,
                speed: 1.0,
                spatial: true,
            },
        ],
        vec![
            AmbienceAsset {
                id: AMBIENCE_INDOOR,
                data: include_bytes!("../assets/audio/ambience_indoor.wav"),
                gain: 0.36,
            },
            AmbienceAsset {
                id: AMBIENCE_RAIN,
                data: include_bytes!("../assets/audio/ambience_rain.wav"),
                gain: 0.34,
            },
        ],
    )
}

/// Start Kira on the engine's existing dedicated audio worker. Device errors are logged and the
/// game continues silently; no audio-device initialization occurs in the headless test setup.
#[cfg(feature = "audio")]
pub fn initialize_output(world: &mut World) {
    match ThreadedAudioSink::new(sound_bank()) {
        Ok(backend) => {
            world.resource_mut::<AudioOutput>().0 = Some(Box::new(backend));
            println!("[ATHAR audio] Kira output initialized");
        }
        Err(error) => {
            let message = format!(
                "ATHAR audio output unavailable; continuing silently: {error}"
            );
            eprintln!("[ATHAR audio] {message}");
            world
                .resource_mut::<LogBuffer>()
                .push(LogLevel::Warn, message);
        }
    }
}

/// Keep ATHAR's headless/no-default-features build usable without linking an audio device backend.
#[cfg(not(feature = "audio"))]
pub fn initialize_output(world: &mut World) {
    let message = "ATHAR audio output is disabled by the `audio` Cargo feature";
    eprintln!("[ATHAR audio] {message}");
    world
        .resource_mut::<LogBuffer>()
        .push(LogLevel::Info, message.to_owned());
}

pub fn emit_spatial(world: &mut World, sound: SoundId, position: Vec3, gain: f32) {
    if !position.is_finite() || !gain.is_finite() || gain <= 0.0 {
        return;
    }
    world.send_event(PlaySpatialSound {
        id: sound,
        emitter: Emitter3d {
            position: AudioVec3::new(position.x, position.y, position.z),
            velocity: AudioVec3::ZERO,
            min_distance: 0.55,
            max_distance: 22.0,
            rolloff: 1.15,
            gain: 1.0,
            occlusion: 0.0,
        },
        gain: gain.clamp(0.0, 1.0),
    });
}

/// Update room/rain ambience only when the phase changes, and emit footsteps by travelled distance
/// rather than per-frame key polling. The backend crossfades a replacement ambience over 180 ms.
pub fn update_gameplay_audio(
    world: &mut World,
    phase: GamePhase,
    player_position: Option<Vec3>,
    dt: f32,
) {
    if world.try_resource::<GameplayAudioState>().is_none() {
        return;
    }
    let mut ambience_event = None;
    let mut footsteps = Vec::new();
    world.resource_scope::<GameplayAudioState, _>(|_, state| {
        let phase_changed = state.last_phase != Some(phase);
        if phase_changed {
            ambience_event = Some(match phase {
                GamePhase::Room => SetAmbience {
                    id: Some(AMBIENCE_INDOOR),
                    gain: 0.72,
                },
                GamePhase::Village => SetAmbience {
                    id: Some(AMBIENCE_RAIN),
                    gain: 0.82,
                },
            });
            state.last_phase = Some(phase);
            state.footstep_distance = 0.0;
        }

        let current_position = player_position.filter(|position| position.is_finite());
        if let (Some(previous), Some(current)) = (state.last_player_position, current_position) {
            if !phase_changed && dt.is_finite() && dt > f32::EPSILON {
                let delta = Vec3::new(current.x - previous.x, 0.0, current.z - previous.z);
                let distance = delta.length();
                let speed = distance / dt;
                if speed >= 0.35 && distance.is_finite() {
                    let sprinting = speed >= 3.7;
                    let cadence = if sprinting { 1.02 } else { 0.82 };
                    state.footstep_distance += distance;
                    // A single rendered update can never flood the audio queue with catch-up
                    // footsteps; emit at most two events even after a long frame.
                    let mut emitted = 0;
                    while state.footstep_distance >= cadence && emitted < 2 {
                        state.footstep_distance -= cadence;
                        footsteps.push((
                            if sprinting {
                                SOUND_FOOTSTEP_SPRINT
                            } else {
                                SOUND_FOOTSTEP_WALK
                            },
                            current - Vec3::Y * 1.55,
                            if sprinting { 0.70 } else { 0.62 },
                        ));
                        emitted += 1;
                    }
                } else if speed < 0.15 {
                    state.footstep_distance = 0.0;
                }
            }
        } else {
            state.footstep_distance = 0.0;
        }
        state.last_player_position = current_position;
    });

    if let Some(event) = ambience_event {
        world.send_event(event);
    }
    for (sound, position, gain) in footsteps {
        emit_spatial(world, sound, position, gain);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ribbon_events::{EventCursor, WorldEventsExt};
    use ribbon_plugin_audio::{AudioPlugin, PlaySpatialSound};
    use ribbon_plugins::AppPluginExt;

    #[test]
    fn athar_sound_bank_contains_decodable_non_empty_original_assets() {
        let bank = sound_bank();
        bank.validate().expect("ATHAR's packaged audio bank is valid");
        assert_eq!(bank.sound_count(), 8);
        assert_eq!(bank.ambience_count(), 2);
        assert!(bank.estimated_encoded_bytes() > 100_000);
    }

    #[test]
    fn gameplay_event_helper_emits_spatial_sound_without_hardware() {
        let mut app = ribbon_app::App::new();
        app.add_plugin(AudioPlugin).expect("audio plugin is headless-safe");
        emit_spatial(
            &mut app.world,
            SOUND_KEY_PICKUP,
            Vec3::new(1.0, 2.0, 3.0),
            0.8,
        );
        let mut cursor = EventCursor::<PlaySpatialSound>::default();
        let events = cursor
            .read(app.world.events::<PlaySpatialSound>())
            .copied()
            .collect::<Vec<_>>();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, SOUND_KEY_PICKUP);
        assert_eq!(events[0].emitter.position, AudioVec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn walking_footsteps_are_distance_cadenced_not_frame_triggered() {
        let mut app = ribbon_app::App::new();
        app.add_plugin(AudioPlugin).expect("audio plugin is headless-safe");
        app.world.insert_resource(GameplayAudioState::default());
        let mut cursor = EventCursor::<PlaySpatialSound>::default();

        update_gameplay_audio(
            &mut app.world,
            GamePhase::Room,
            Some(Vec3::ZERO),
            1.0 / 60.0,
        );
        // One metre over half a second is walking speed and crosses one 0.82 m cadence.
        update_gameplay_audio(
            &mut app.world,
            GamePhase::Room,
            Some(Vec3::new(1.0, 0.0, 0.0)),
            0.5,
        );
        // A small movement increment does not produce an event every update.
        update_gameplay_audio(
            &mut app.world,
            GamePhase::Room,
            Some(Vec3::new(1.1, 0.0, 0.0)),
            0.1,
        );
        // Sprinting uses its own cue and caps catch-up events to two for this long frame.
        update_gameplay_audio(
            &mut app.world,
            GamePhase::Room,
            Some(Vec3::new(5.1, 0.0, 0.0)),
            0.5,
        );

        let sounds = cursor
            .read(app.world.events::<PlaySpatialSound>())
            .copied()
            .collect::<Vec<_>>();
        assert_eq!(sounds.len(), 3, "a long frame must not flood the audio queue");
        assert_eq!(sounds[0].id, SOUND_FOOTSTEP_WALK);
        assert_eq!(sounds[1].id, SOUND_FOOTSTEP_SPRINT);
        assert_eq!(sounds[2].id, SOUND_FOOTSTEP_SPRINT);
    }

    #[test]
    fn ambience_is_started_once_and_only_crossfades_on_phase_change() {
        let mut app = ribbon_app::App::new();
        app.add_plugin(AudioPlugin).expect("audio plugin is headless-safe");
        app.world.insert_resource(GameplayAudioState::default());
        update_gameplay_audio(
            &mut app.world,
            GamePhase::Room,
            Some(Vec3::ZERO),
            1.0 / 60.0,
        );
        update_gameplay_audio(
            &mut app.world,
            GamePhase::Room,
            Some(Vec3::ZERO),
            1.0 / 60.0,
        );
        update_gameplay_audio(
            &mut app.world,
            GamePhase::Village,
            Some(Vec3::new(8.0, 0.0, 0.0)),
            1.0 / 60.0,
        );
        let mut cursor = EventCursor::<SetAmbience>::default();
        let events = cursor
            .read(app.world.events::<SetAmbience>())
            .copied()
            .collect::<Vec<_>>();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].id, Some(AMBIENCE_INDOOR));
        assert_eq!(events[1].id, Some(AMBIENCE_RAIN));
    }
}
