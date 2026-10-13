use raycast_blocks::input::Controls;
use raycast_blocks::math::{Angles, Rng};
use raycast_blocks::player::{Player, EYE_HEIGHT};
use raycast_blocks::world::{World, FLOOR_TOP};

fn setup() -> (World, Player) {
    let w = World::generate(&mut Rng::new(42));
    let p = Player::new(2.0, 2.0, Angles::default());
    (w, p)
}

#[test]
fn player_stands_on_floor() {
    let (w, mut p) = setup();
    p.pos.z = 3.5;
    p.on_ground = false;
    let c = Controls::default();
    for _ in 0..200 {
        p.update(&w, &c, 0.02);
    }
    assert!(p.on_ground);
    assert!((p.pos.z - (FLOOR_TOP + 0.001 + EYE_HEIGHT)).abs() < 0.01);
}

#[test]
fn jump_then_land_reports_events() {
    let (w, mut p) = setup();
    let jump = Controls {
        jump: true,
        ..Controls::default()
    };
    let idle = Controls::default();

    let ev = p.update(&w, &jump, 0.02);
    assert!(ev.jumped);
    assert!(!p.on_ground);

    let mut landed = false;
    for _ in 0..200 {
        if p.update(&w, &idle, 0.02).landed {
            landed = true;
            break;
        }
    }
    assert!(landed);
}

#[test]
fn sprinting_drains_stamina_until_exhausted() {
    let (w, mut p) = setup();
    let sprint = Controls {
        forward: true,
        sprint: true,
        ..Controls::default()
    };
    for _ in 0..100 {
        p.update(&w, &sprint, 0.02);
    }
    assert!(
        p.stamina > 0.4 && p.stamina < 0.6,
        "stamina = {}",
        p.stamina
    );
    for _ in 0..150 {
        p.update(&w, &sprint, 0.02);
    }
    assert!(p.exhausted);
}

#[test]
fn pitch_is_clamped() {
    let (w, mut p) = setup();
    let up = Controls {
        look_up: true,
        ..Controls::default()
    };
    for _ in 0..500 {
        p.update(&w, &up, 0.02);
    }
    assert!(p.view.psi <= raycast_blocks::player::PSI_LIMIT + 1e-6);
}

#[test]
fn walls_stop_the_player() {
    let (w, mut p) = setup();
    // walk "forward" along -x for a long time: the outer wall must stop us
    p.view.phi = std::f32::consts::PI;
    let walk = Controls {
        forward: true,
        ..Controls::default()
    };
    for _ in 0..300 {
        p.update(&w, &walk, 0.02);
    }
    assert!(p.pos.x > 1.0, "player escaped the maze: x = {}", p.pos.x);
}
