use raycast_blocks::enemy::{
    bfs_next, Enemy, CATCH_DIST, ENEMY_AWAKE_AFTER, ENEMY_BASE_SPEED, SEEN_SPEED_FACTOR,
};
use raycast_blocks::math::{Angles, Rng, Vect};
use raycast_blocks::world::{World, FLOOR_TOP};

fn world() -> World {
    World::generate(&mut Rng::new(42))
}

#[test]
fn sleeps_until_timer_or_first_page() {
    assert!(!Enemy::awake(0.0, 0));
    assert!(!Enemy::awake(ENEMY_AWAKE_AFTER - 0.1, 0));
    assert!(Enemy::awake(ENEMY_AWAKE_AFTER + 0.1, 0));
    assert!(Enemy::awake(0.0, 1));
}

#[test]
fn speed_rules() {
    assert_eq!(Enemy::speed(0, false), ENEMY_BASE_SPEED);
    assert!(Enemy::speed(3, false) > Enemy::speed(0, false));
    assert!((Enemy::speed(0, true) - ENEMY_BASE_SPEED * SEEN_SPEED_FACTOR).abs() < 1e-6);
}

#[test]
fn tuned_speed_scales_chase_and_watched_speed() {
    let easy = Enemy::speed_with_tuning(2, false, 0.70, 0.08);
    let normal = Enemy::speed(2, false);
    let hard = Enemy::speed_with_tuning(2, false, 1.35, 0.14);
    assert!(easy < normal);
    assert!(hard > normal);

    assert!(Enemy::speed_with_tuning(2, true, 0.70, 0.08) < easy);
    assert!(Enemy::speed_with_tuning(2, true, 1.35, 0.14) < hard);
}

#[test]
fn catch_radius() {
    let e = Enemy::new(2.0, 2.0);
    assert!(e.has_caught(Vect::new(2.0 + CATCH_DIST * 0.5, 2.0, 2.5)));
    assert!(!e.has_caught(Vect::new(2.0 + CATCH_DIST * 2.0, 2.0, 2.5)));
}

#[test]
fn bfs_steps_to_a_neighbour_tile() {
    let w = world();
    let next = bfs_next(&w, (1, 1), (2, 2)).expect("path inside the first cell");
    let manhattan = (next.0 - 1).abs() + (next.1 - 1).abs();
    assert_eq!(manhattan, 1);
    assert!(w.tile_open(next.0, next.1));
}

#[test]
fn bfs_refuses_walls() {
    let w = world();
    assert!(bfs_next(&w, (0, 0), (2, 2)).is_none());
}

#[test]
fn enemy_moves_toward_player() {
    let w = world();
    let mut e = Enemy::new(1.5, 1.5);
    let player = Vect::new(2.5, 2.5, 2.5);
    let before = e.pos;
    e.update(&w, player, 3.0, 0.1);
    assert!(e.moving);
    assert!(e.pos != before);
    assert_eq!(e.pos.z, FLOOR_TOP);
}

#[test]
fn enemy_with_zero_speed_stays_put() {
    let w = world();
    let mut e = Enemy::new(1.5, 1.5);
    let before = e.pos;
    e.update(&w, Vect::new(2.5, 2.5, 2.5), 0.0, 0.1);
    assert!(!e.moving);
    assert_eq!(e.pos, before);
}

#[test]
fn seen_only_when_facing_it() {
    let w = world();
    let enemy = Enemy::new(2.8, 2.0);
    let eye = Vect::new(1.2, 2.0, 2.5);
    let facing = Angles { psi: 0.0, phi: 0.0 };
    let away = Angles {
        psi: 0.0,
        phi: std::f32::consts::PI,
    };
    assert!(enemy.is_seen_from(eye, facing, &w, 18.0));
    assert!(!enemy.is_seen_from(eye, away, &w, 18.0));
    assert!(!enemy.is_seen_from(eye, facing, &w, 0.5)); // too far for the given range
}
