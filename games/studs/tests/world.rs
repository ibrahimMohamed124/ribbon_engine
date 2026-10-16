use studs::enemy::bfs_next;
use studs::world::{World, AIR, FLOOR, X_BLOCKS, Y_BLOCKS, Z_BLOCKS};
use voxelworks_core::math::{Rng, Vect};

#[test]
fn ray_down_hits_floor_top() {
    let w = World::generate(&mut Rng::new(1234));
    let hit = w
        .cast(Vect::new(2.0, 2.0, 2.5), Vect::new(0.0, 0.0, -1.0), 10.0)
        .unwrap();
    assert_eq!(hit.cell.2, 0);
    assert_eq!(hit.normal, [0, 0, 1]);
    assert!((hit.dist - 1.5).abs() < 1e-4);
}

#[test]
fn maze_is_fully_connected() {
    let w = World::generate(&mut Rng::new(98765));
    let goal = (2, 2);
    for y in 0..Y_BLOCKS as i32 {
        for x in 0..X_BLOCKS as i32 {
            if w.tile_open(x, y) && (x, y) != goal {
                assert!(
                    bfs_next(&w, (x, y), goal).is_some(),
                    "tile {x},{y} unreachable"
                );
            }
        }
    }
}

#[test]
fn same_seed_same_world() {
    let a = World::generate(&mut Rng::new(7));
    let b = World::generate(&mut Rng::new(7));
    assert_eq!(a.blocks(), b.blocks());
}

#[test]
fn floor_everywhere_and_start_cell_open() {
    let w = World::generate(&mut Rng::new(5));
    assert_eq!(w.blocks().len(), X_BLOCKS * Y_BLOCKS * Z_BLOCKS);
    assert_eq!(w.get(10, 10, 0), Some(FLOOR));
    assert_eq!(w.get(1, 1, 1), Some(AIR));
    assert_eq!(w.get(-1, 0, 0), None);
}

#[test]
fn outside_the_maze_is_solid() {
    let w = World::generate(&mut Rng::new(5));
    assert!(w.is_solid_at(-0.5, 2.0, 2.0));
    assert!(!w.is_solid_at(2.0, 2.0, 2.0));
}
