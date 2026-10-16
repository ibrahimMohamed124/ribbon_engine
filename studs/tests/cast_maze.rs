use engine_core::math::{Rng, Vect};
use studs::world::{World, AIR, X_BLOCKS, Y_BLOCKS, Z_BLOCKS};

fn unit(rng: &mut Rng) -> f32 {
    (rng.next_u32() >> 8) as f32 / 16_777_216.0
}

fn brute_force(
    world: &World,
    origin: Vect,
    dir: Vect,
    max_dist: f32,
) -> Option<(i32, i32, i32, f32, Vect)> {
    let mut previous = (-1, -1, -1);
    let mut t = 0.0;
    while t <= max_dist {
        let point = origin + dir.scale(t);
        let cell = (
            point.x.floor() as i32,
            point.y.floor() as i32,
            point.z.floor() as i32,
        );
        if cell.0 < 0
            || cell.1 < 0
            || cell.2 < 0
            || cell.0 >= X_BLOCKS as i32
            || cell.1 >= Y_BLOCKS as i32
            || cell.2 >= Z_BLOCKS as i32
        {
            return None;
        }
        if cell != previous && world.get(cell.0, cell.1, cell.2) != Some(AIR) {
            return Some((cell.0, cell.1, cell.2, t, point));
        }
        previous = cell;
        t += 0.0005;
    }
    None
}

#[test]
fn generated_mazes_match_the_brute_force_cast() {
    for seed in 0..20 {
        let mut rng = Rng::new(seed + 1);
        let world = World::generate(&mut rng);
        for _ in 0..100 {
            let cell = loop {
                let x = rng.below(X_BLOCKS);
                let y = rng.below(Y_BLOCKS);
                if world.get(x as i32, y as i32, 2) == Some(AIR) {
                    break (x, y);
                }
            };
            let origin = Vect::new(
                cell.0 as f32 + 0.1 + unit(&mut rng) * 0.8,
                cell.1 as f32 + 0.1 + unit(&mut rng) * 0.8,
                2.2 + unit(&mut rng) * 0.6,
            );
            let dir = Vect::new(
                unit(&mut rng) * 2.0 - 1.0,
                unit(&mut rng) * 2.0 - 1.0,
                unit(&mut rng) * 0.5 - 0.25,
            )
            .normalize();
            let expected = brute_force(&world, origin, dir, 32.0);
            let actual = world.cast(origin, dir, 32.0);
            assert_eq!(actual.is_some(), expected.is_some(), "seed {seed}");
            if let (Some(actual), Some((x, y, z, dist, point))) = (actual, expected) {
                assert_eq!(actual.cell, (x, y, z), "seed {seed}");
                assert!((actual.dist - dist).abs() <= 0.01, "seed {seed}");
                let error = (actual.point.x - point.x)
                    .hypot(actual.point.y - point.y)
                    .hypot(actual.point.z - point.z);
                assert!(error <= 1e-3, "seed {seed}, point error {error}");
            }
        }
    }
}
