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
