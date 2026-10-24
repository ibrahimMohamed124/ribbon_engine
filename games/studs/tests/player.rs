use ribbon_math::{Angles, Rng};
use studs::input::Controls;
use studs::player::{Player, EYE_HEIGHT};
use studs::world::{World, FLOOR_TOP, X_BLOCKS, Y_BLOCKS};

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
    assert!(p.view.psi <= studs::player::PSI_LIMIT + 1e-6);
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

#[test]
fn footsteps_are_reported_by_distance_not_every_frame() {
    let world = World::generate(&mut Rng::new(19));
    let mut path = None;
    for y in 1..Y_BLOCKS as i32 - 1 {
        for x in 1..X_BLOCKS as i32 - 1 {
            if world.tile_open(x, y) && world.tile_open(x + 1, y) {
                path = Some((x, y, 0.0));
                break;
            }
            if world.tile_open(x, y) && world.tile_open(x, y + 1) {
                path = Some((x, y, std::f32::consts::FRAC_PI_2));
                break;
            }
        }
        if path.is_some() {
            break;
        }
    }
    let (x, y, phi) = path.expect("generated maze has an open path");
    let mut player = Player::new(x as f32 + 0.5, y as f32 + 0.5, Angles { psi: 0.0, phi });
    let walk = Controls {
        forward: true,
        ..Controls::default()
    };
    let events: Vec<_> = (0..30)
        .map(|_| player.update(&world, &walk, 0.02).footstep)
        .collect();
    assert!(events.iter().any(|&footstep| footstep));
    assert!(events.iter().filter(|&&footstep| footstep).count() < events.len());
}
