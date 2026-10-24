use ribbon_math::Vect;
use studs::audio::AudioEvent;
use studs::enemy::ENEMY_AWAKE_AFTER;
use studs::game::{Difficulty, Game, State, PAGES_TOTAL};
use studs::input::InputFrame;
use studs::world::FLOOR_TOP;

const DT: f32 = 0.016;

fn idle() -> InputFrame {
    InputFrame::default()
}

#[test]
fn new_game_starts_clean() {
    let g = Game::new(1);
    assert_eq!(g.state, State::Playing);
    assert_eq!(g.collected, 0);
    assert_eq!(g.pages.len(), PAGES_TOTAL);
    assert!(g.got.iter().all(|&x| !x));
    assert!(g.flashlight_on);
    assert!(g.events.is_empty());
}

#[test]
fn same_seed_gives_same_layout() {
    let a = Game::new(99);
    let b = Game::new(99);
    assert_eq!(a.pages, b.pages);
    assert_eq!(a.world.blocks(), b.world.blocks());
}

#[test]
fn difficulties_change_enemy_grace_period_and_chase_pressure() {
    let easy = Game::new_with_difficulty(17, Difficulty::Easy);
    let normal = Game::new(17);
    let hard = Game::new_with_difficulty(17, Difficulty::Hard);

    assert!(!easy.enemy_awake());
    assert!(!normal.enemy_awake());
    assert!(!hard.enemy_awake());
    assert!(Difficulty::Easy.enemy_wake_time() > Difficulty::Normal.enemy_wake_time());
    assert!(Difficulty::Hard.enemy_wake_time() < Difficulty::Normal.enemy_wake_time());
    assert!(
        Difficulty::Easy.enemy_speed_multiplier() < Difficulty::Normal.enemy_speed_multiplier()
    );
    assert!(
        Difficulty::Hard.enemy_speed_multiplier() > Difficulty::Normal.enemy_speed_multiplier()
    );
    assert!(Difficulty::Easy.seen_speed_factor() < Difficulty::Normal.seen_speed_factor());
    assert!(Difficulty::Hard.seen_speed_factor() > Difficulty::Normal.seen_speed_factor());

    let mut easy = easy;
    let mut hard = hard;
    easy.time = Difficulty::Easy.enemy_wake_time() + 0.1;
    hard.time = Difficulty::Hard.enemy_wake_time() + 0.1;
    assert!(easy.enemy_awake());
    assert!(hard.enemy_awake());
}

#[test]
fn pages_are_distinct() {
    let g = Game::new(3);
    for i in 0..g.pages.len() {
        for j in (i + 1)..g.pages.len() {
            assert!(g.pages[i] != g.pages[j]);
        }
    }
}

#[test]
fn enemy_sleeps_at_first() {
    let mut g = Game::new(1);
    let start = g.enemy.pos;
    for _ in 0..100 {
        g.update(&idle(), 0.05); // 5 simulated seconds
    }
    assert!(g.time < ENEMY_AWAKE_AFTER);
    assert_eq!(g.enemy.pos, start);
    assert_eq!(g.proximity(), 0.0);
}

#[test]
fn flashlight_toggle_comes_from_input() {
    let mut g = Game::new(1);
    let toggle = InputFrame {
        toggle_flashlight: true,
        ..InputFrame::default()
    };
    g.update(&toggle, DT);
    assert!(!g.flashlight_on);
    g.update(&toggle, DT);
    assert!(g.flashlight_on);
}

#[test]
fn collecting_every_page_wins() {
    let mut g = Game::new(5);
    let pages = g.pages.clone();
    for p in pages {
        g.player.pos.x = p.x;
        g.player.pos.y = p.y;
        g.update(&idle(), DT);
    }
    assert_eq!(g.collected, PAGES_TOTAL);
    assert_eq!(g.state, State::Won);
    assert!(g.events.contains(&AudioEvent::PageCollected));
    assert!(g.events.contains(&AudioEvent::EnemySting));
    assert!(g.events.contains(&AudioEvent::Won));
}

#[test]
fn enemy_touching_the_player_catches_them() {
    let mut g = Game::new(5);
    g.time = ENEMY_AWAKE_AFTER + 1.0;
    g.enemy.pos = Vect::new(g.player.pos.x, g.player.pos.y, FLOOR_TOP);
    g.update(&idle(), DT);
    assert_eq!(g.state, State::Caught);
    assert!(g.events.contains(&AudioEvent::Caught));
}

#[test]
fn restart_needs_a_delay_and_keeps_the_flashlight_setting() {
    let mut g = Game::new_with_difficulty(5, Difficulty::Hard);
    g.flashlight_on = false;
    g.time = ENEMY_AWAKE_AFTER + 1.0;
    g.enemy.pos = Vect::new(g.player.pos.x, g.player.pos.y, FLOOR_TOP);
    g.update(&idle(), DT);
    assert_eq!(g.state, State::Caught);
    assert!(!g.can_restart()); // the game-over screen was only just shown

    for _ in 0..100 {
        g.update(&idle(), DT);
    }
    assert!(g.can_restart());

    g.restart(77);
    assert_eq!(g.state, State::Playing);
    assert_eq!(g.collected, 0);
    assert!(!g.flashlight_on);
    assert_eq!(g.difficulty, Difficulty::Hard);
}

#[test]
fn window_title_reflects_state() {
    let mut g = Game::new(1);
    assert!(g.window_title(60.0).contains("pages 0/6"));
    g.state = State::Caught;
    assert!(g.window_title(60.0).contains("IT GOT YOU"));
    g.state = State::Won;
    assert!(g.window_title(60.0).contains("press R"));
}
