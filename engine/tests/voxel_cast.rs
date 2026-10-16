use blocks_engine::math::{Rng, Vect};
use blocks_engine::voxel::{Hit, VoxelWorld};

const EMPTY: u8 = 0;
const SOLID: u8 = 1;
const STEP: f32 = 0.0005;

fn unit(rng: &mut Rng) -> f32 {
    (rng.next_u32() >> 8) as f32 / 16_777_216.0
}

fn random_world(width: usize, height: usize, depth: usize, rng: &mut Rng) -> VoxelWorld {
    let mut blocks = vec![EMPTY; width * height * depth];
    for z in 0..depth {
        for y in 0..height {
            for x in 0..width {
                if rng.chance(0.30) {
                    blocks[VoxelWorld::idx(width, height, z, y, x)] = SOLID;
                }
            }
        }
    }
    VoxelWorld::new(width, height, depth, blocks)
}

fn brute_force(world: &VoxelWorld, origin: Vect, dir: Vect, max_dist: f32) -> Option<Hit> {
    let (width, height, depth) = world.dimensions();
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
            || cell.0 >= width as i32
            || cell.1 >= height as i32
            || cell.2 >= depth as i32
        {
            return None;
        }
        if cell != previous && world.get(cell.0, cell.1, cell.2) != Some(EMPTY) {
            return Some(Hit {
                cell,
                normal: [0; 3],
                dist: t,
                point,
            });
        }
        previous = cell;
        t += STEP;
    }
    None
}

fn ambiguous_edge(hit: &Hit, normal: [i32; 3]) -> bool {
    [hit.point.x, hit.point.y, hit.point.z]
        .into_iter()
        .enumerate()
        .any(|(axis, v)| normal[axis] == 0 && v.fract().min(1.0 - v.fract()) <= 1e-3)
}

fn assert_random_world(width: usize, height: usize, seed: u32) -> usize {
    let mut rng = Rng::new(seed);
    let depth = 6;
    let world = random_world(width, height, depth, &mut rng);
    let mut skipped = 0;
    let rays = 10_000;
    for ray_index in 0..rays {
        let (x, y, z) = loop {
            let cell = (rng.below(width), rng.below(height), rng.below(depth));
            if world.get(cell.0 as i32, cell.1 as i32, cell.2 as i32) == Some(EMPTY) {
                break cell;
            }
        };
        let origin = Vect::new(
            x as f32 + unit(&mut rng),
            y as f32 + unit(&mut rng),
            z as f32 + unit(&mut rng),
        );
        let dir = Vect::new(
            unit(&mut rng) * 2.0 - 1.0,
            unit(&mut rng) * 2.0 - 1.0,
            unit(&mut rng) * 2.0 - 1.0,
        )
        .normalize();
        let max_dist = 32.0;
        let expected = brute_force(&world, origin, dir, max_dist);
        let actual = world.cast(origin, dir, max_dist, EMPTY);
        if expected.is_none()
            && actual
                .as_ref()
                .is_some_and(|hit| ambiguous_edge(hit, hit.normal))
        {
            skipped += 1;
            continue;
        }
        assert_eq!(
            actual.is_some(),
            expected.is_some(),
            "dimensions={width}x{height}x{depth}, ray={ray_index}, origin={origin:?}, dir={dir:?}, actual={actual:?}, expected={expected:?}"
        );
        let (Some(actual), Some(expected)) = (actual, expected) else {
            continue;
        };
        if ambiguous_edge(&expected, actual.normal) {
            skipped += 1;
            continue;
        }
        assert_eq!(actual.cell, expected.cell);
        assert!((actual.dist - expected.dist).abs() <= 0.01);
        let point_error = (actual.point.x - expected.point.x)
            .hypot(actual.point.y - expected.point.y)
            .hypot(actual.point.z - expected.point.z);
        assert!(point_error <= 1e-3, "point error {point_error}");
        assert_eq!(actual.normal.iter().map(|v| v.abs()).sum::<i32>(), 1);
        let axis = actual.normal.iter().position(|&v| v != 0).unwrap();
        assert!(actual.normal[axis] as f32 * dir.axis(axis) < 0.0);
    }
    assert!(skipped <= rays * 5 / 100, "skipped {skipped}/{rays}");
    skipped
}

#[test]
fn dda_matches_a_fine_step_marcher_on_random_worlds() {
    let skipped_small = assert_random_world(8, 8, 0x1234_5678);
    let skipped_large = assert_random_world(16, 16, 0x8765_4321);
    eprintln!("skipped ambiguous edge hits: {skipped_small} + {skipped_large}");
}

#[test]
fn directed_rays_cover_axes_edges_limits_and_world_exit() {
    let mut blocks = vec![EMPTY; 4 * 4 * 4];
    for (x, y, z) in [(3, 1, 1), (1, 3, 1), (1, 1, 3), (0, 1, 1)] {
        blocks[VoxelWorld::idx(4, 4, z, y, x)] = SOLID;
    }
    let world = VoxelWorld::new(4, 4, 4, blocks);
    for (origin, dir, cell, normal) in [
        (
            Vect::new(1.5, 1.5, 1.5),
            Vect::new(1.0, 0.0, 0.0),
            (3, 1, 1),
            [-1, 0, 0],
        ),
        (
            Vect::new(1.5, 1.5, 1.5),
            Vect::new(0.0, 1.0, 0.0),
            (1, 3, 1),
            [0, -1, 0],
        ),
        (
            Vect::new(1.5, 1.5, 1.5),
            Vect::new(0.0, 0.0, 1.0),
            (1, 1, 3),
            [0, 0, -1],
        ),
        (
            Vect::new(1.5, 1.5, 1.5),
            Vect::new(-1.0, 0.0, 0.0),
            (0, 1, 1),
            [1, 0, 0],
        ),
    ] {
        let hit = world.cast(origin, dir, 8.0, EMPTY).unwrap();
        assert_eq!(hit.cell, cell);
        assert_eq!(hit.normal, normal);
    }
    let from_edge = world
        .cast(
            Vect::new(2.0, 1.5, 1.5),
            Vect::new(-1.0, 0.0, 0.0),
            1.0,
            EMPTY,
        )
        .unwrap();
    assert_eq!(from_edge.cell, (0, 1, 1));
    assert_eq!(from_edge.dist, 1.0);
    assert!(world
        .cast(
            Vect::new(1.5, 1.5, 1.5),
            Vect::new(1.0, 0.0, 0.0),
            0.0,
            EMPTY
        )
        .is_none());
    assert!(world
        .cast(
            Vect::new(1.5, 1.5, 1.5),
            Vect::new(1.0, 0.0, 0.0),
            f32::MIN_POSITIVE,
            EMPTY
        )
        .is_none());
    assert!(world
        .cast(
            Vect::new(3.5, 2.5, 2.5),
            Vect::new(1.0, 0.0, 0.0),
            8.0,
            EMPTY
        )
        .is_none());
}
