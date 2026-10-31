use super::*;

pub(super) fn emit_audio_events(world: &mut World) {
    let frame = world.resource_scope::<RunState, _>(|world, run| {
        let player = world
            .query::<&Player>()
            .iter()
            .next()
            .map(|(_, player)| (player.pos, player.view.psi))
            .unwrap_or((Vect::new(0.0, 0.0, 0.0), 0.0));
        let enemy_pos = world
            .query::<&Enemy>()
            .iter()
            .next()
            .map(|(_, enemy)| enemy.pos)
            .unwrap_or(player.0);
        let listener = AudioListener {
            position: player.0,
            yaw: player.1,
        };
        if let Some((_, current)) = world.query_mut::<&mut AudioListener>().into_iter().next() {
            *current = listener;
        }
        let enemy_awake = run.enemy_awake();
        let distance = enemy_pos.dist_xy(player.0);
        let ambience = match world
            .try_resource::<StudsState>()
            .map(|state| state.ui.screen())
            .unwrap_or(Screen::MainMenu)
        {
            Screen::Playing if run.state == State::Playing && enemy_awake => Some((
                ambiences::THREAT,
                0.025 + proximity_intensity(distance, true) * 0.11,
            )),
            Screen::MainMenu
            | Screen::Pause
            | Screen::Difficulty
            | Screen::Settings
            | Screen::Controls
            | Screen::ConfirmMainMenu => Some((ambiences::MENU, 0.12)),
            _ => None,
        };
        AudioFrame {
            events: std::mem::take(&mut run.events),
            spatial_events: std::mem::take(&mut run.spatial_events),
            ambience,
            effects_gain: world
                .try_resource::<StudsState>()
                .map(|state| {
                    if state.settings.mute_all {
                        0.0
                    } else {
                        state.settings.master_volume * state.settings.effects_volume
                    }
                })
                .unwrap_or(0.0),
            ambient_gain: world
                .try_resource::<StudsState>()
                .map(|state| {
                    if state.settings.mute_all {
                        0.0
                    } else {
                        state.settings.master_volume * state.settings.ambient_volume
                    }
                })
                .unwrap_or(0.0),
            listener,
        }
    });

    for event in frame.events {
        if frame.effects_gain > 0.0 {
            world.send_event(PlaySound {
                id: sound_id(event),
                position: None,
                gain: frame.effects_gain,
            });
        }
    }
    for event in frame.spatial_events {
        if frame.effects_gain <= 0.0 {
            continue;
        }
        let pan = event.pan.clamp(-1.0, 1.0);
        let forward_amount = (1.0 - pan * pan).max(0.0).sqrt();
        let right = Vect::new(-frame.listener.yaw.sin(), frame.listener.yaw.cos(), 0.0);
        let forward = Vect::new(-frame.listener.yaw.cos(), -frame.listener.yaw.sin(), 0.0);
        let position = Vect::new(
            frame.listener.position.x + right.x * pan + forward.x * forward_amount,
            frame.listener.position.y + right.y * pan + forward.y * forward_amount,
            frame.listener.position.z,
        );
        world.send_event(PlaySound {
            id: sound_id(event.event),
            position: Some(position),
            gain: frame.effects_gain,
        });
    }
    let (id, gain) = frame
        .ambience
        .map(|(id, volume)| (Some(id), frame.ambient_gain * volume))
        .unwrap_or((None, 0.0));
    world.send_event(SetAmbience { id, gain });
}

struct AudioFrame {
    events: Vec<AudioEvent>,
    spatial_events: Vec<crate::audio::SpatialAudioEvent>,
    ambience: Option<(ribbon_plugin_audio::AmbienceId, f32)>,
    effects_gain: f32,
    ambient_gain: f32,
    listener: AudioListener,
}

pub(super) fn sound_id(event: AudioEvent) -> SoundId {
    match event {
        AudioEvent::Footstep => sounds::FOOTSTEP,
        AudioEvent::Jump => sounds::JUMP,
        AudioEvent::Land => sounds::LAND,
        AudioEvent::PageCollected => sounds::COLLECT,
        AudioEvent::EnemySting => sounds::STING,
        AudioEvent::EnemyFootstep => sounds::CHASE_FOOTSTEP,
        AudioEvent::Caught => sounds::CAUGHT,
        AudioEvent::Won => sounds::VICTORY,
    }
}
