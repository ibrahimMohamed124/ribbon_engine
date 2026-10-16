use raycast_blocks::enemy::{animation_state, EnemyAnimation, CATCH_DIST};

#[test]
fn enemy_animation_tracks_sleeping_walking_running_freezing_and_attack_states() {
    assert_eq!(
        animation_state(false, false, 10.0, 0.0, false),
        EnemyAnimation::Idle
    );
    assert_eq!(
        animation_state(true, false, 8.0, 3.6, true),
        EnemyAnimation::Walk
    );
    assert_eq!(
        animation_state(true, false, 2.0, 4.0, true),
        EnemyAnimation::RunLunge
    );
    assert_eq!(
        animation_state(true, true, 2.0, 0.4, false),
        EnemyAnimation::Frozen
    );
    assert_eq!(
        animation_state(true, false, CATCH_DIST * 0.9, 5.0, true),
        EnemyAnimation::Attack
    );
}

#[test]
fn monster_proximity_effects_are_monotonic_clamped_and_finite() {
    use raycast_blocks::enemy::{flashlight_pulse_factor, footstep_crossings, proximity_intensity};

    let levels: Vec<_> = (0..=20)
        .map(|step| proximity_intensity(step as f32, true))
        .collect();
    assert!(levels
        .iter()
        .all(|value| value.is_finite() && (0.0..=1.0).contains(value)));
    assert!(levels.windows(2).all(|pair| pair[0] >= pair[1]));
    assert_eq!(proximity_intensity(0.0, false), 0.0);
    assert_eq!(proximity_intensity(f32::NAN, true), 0.0);
    let pulse_levels: Vec<_> = (0..=20)
        .map(|step| flashlight_pulse_factor(step as f32 / 20.0, true, false))
        .collect();
    assert!(pulse_levels.windows(2).all(|pair| pair[0] >= pair[1]));
    assert!(pulse_levels.iter().all(|value| *value >= 0.45));
    assert!(flashlight_pulse_factor(1.0, true, true) >= 0.88);
    assert_eq!(footstep_crossings(0.2, 2.0), 0);
    assert_eq!(footstep_crossings(0.2, 3.3), 1);
    assert_eq!(footstep_crossings(0.2, 7.0), 2);
    assert_eq!(footstep_crossings(f32::NAN, 7.0), 0);
}
