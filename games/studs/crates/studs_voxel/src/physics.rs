//! STUDS-specific voxel collision queries and axis-separated movement.
use crate::VoxelWorld;
use ribbon_math::Vect;

/// Collision footprint and sample heights, measured relative to the body origin.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyShape {
    pub radius: f32,
    pub sample_dz: Vec<f32>,
}

impl BodyShape {
    pub fn new(radius: f32, sample_dz: impl Into<Vec<f32>>) -> Self {
        Self {
            radius,
            sample_dz: sample_dz.into(),
        }
    }
}

/// Does a body overlap any solid block? `pos` is the body origin.
pub fn collides(world: &VoxelWorld, empty: u8, pos: Vect, shape: &BodyShape) -> bool {
    for &(dx, dy) in &[
        (-shape.radius, -shape.radius),
        (shape.radius, -shape.radius),
        (-shape.radius, shape.radius),
        (shape.radius, shape.radius),
    ] {
        for &dz in &shape.sample_dz {
            if world.is_solid_at(pos.x + dx, pos.y + dy, pos.z + dz, empty) {
                return true;
            }
        }
    }
    false
}

/// Move in x and then y, retaining the old position on each blocked axis.
pub fn slide_xy(
    world: &VoxelWorld,
    empty: u8,
    mut pos: Vect,
    delta_xy: (f32, f32),
    shape: &BodyShape,
) -> Vect {
    let x_pos = Vect::new(pos.x + delta_xy.0, pos.y, pos.z);
    if !collides(world, empty, x_pos, shape) {
        pos = x_pos;
    }
    let y_pos = Vect::new(pos.x, pos.y + delta_xy.1, pos.z);
    if !collides(world, empty, y_pos, shape) {
        pos = y_pos;
    }
    pos
}

#[cfg(test)]
mod tests {
    use super::{collides, slide_xy, BodyShape};
    use crate::VoxelWorld;
    use ribbon_math::Vect;

    const EMPTY: u8 = 0;
    const SOLID: u8 = 1;
    const DIM: usize = 8;

    fn shape() -> BodyShape {
        BodyShape::new(0.3, vec![-1.5, -0.6, 0.1])
    }

    fn world(solids: &[(usize, usize, usize)]) -> VoxelWorld {
        let mut blocks = vec![EMPTY; DIM * DIM * DIM];
        for y in 0..DIM {
            for x in 0..DIM {
                blocks[VoxelWorld::idx(DIM, DIM, 0, y, x)] = SOLID;
            }
        }
        for &(x, y, z) in solids {
            blocks[VoxelWorld::idx(DIM, DIM, z, y, x)] = SOLID;
        }
        VoxelWorld::new(DIM, DIM, DIM, blocks)
    }

    #[test]
    fn open_floor_space_does_not_collide_and_wall_does() {
        let open = world(&[]);
        assert!(!collides(&open, EMPTY, Vect::new(3.5, 3.5, 2.5), &shape()));
        let wall = world(&[(3, 3, 2)]);
        assert!(collides(&wall, EMPTY, Vect::new(3.5, 3.5, 2.5), &shape()));
    }

    #[test]
    fn all_four_footprint_corners_are_checked() {
        for &(dx, dy) in &[(-1i32, -1i32), (1, -1), (-1, 1), (1, 1)] {
            let x = if dx > 0 { 4 } else { 3 };
            let y = if dy > 0 { 4 } else { 3 };
            let world = world(&[(x, y, 2)]);
            assert!(collides(&world, EMPTY, Vect::new(3.8, 3.8, 2.5), &shape()));
        }
    }

    #[test]
    fn head_sample_detects_a_low_ceiling() {
        let world = world(&[(3, 3, 2)]);
        let head_only = BodyShape::new(0.3, vec![0.1]);
        assert!(collides(
            &world,
            EMPTY,
            Vect::new(3.8, 3.8, 1.9),
            &head_only
        ));
    }

    #[test]
    fn wall_sliding_and_corner_stop_match_axis_separated_movement() {
        let wall = world(&[(4, 3, 2)]);
        let pos = Vect::new(3.5, 3.5, 2.5);
        let slid = slide_xy(&wall, EMPTY, pos, (0.4, 0.5), &shape());
        assert_eq!(slid.x, pos.x);
        assert!((slid.y - 4.0).abs() < 1e-6);

        let corner = world(&[(4, 3, 2), (3, 4, 2)]);
        assert_eq!(slide_xy(&corner, EMPTY, pos, (0.4, 0.4), &shape()), pos);
        assert_eq!(slide_xy(&corner, EMPTY, pos, (0.0, 0.0), &shape()), pos);
    }
}

#[cfg(test)]
mod movement_integration_tests {
    use super::{slide_xy, BodyShape};
    use crate::VoxelWorld;
    use ribbon_math::Vect;

    #[test]
    fn fixed_step_consumes_horizontal_intent_and_preserves_height() {
        // Voxel collision is game-owned now: retain the source test's key invariant while
        // exercising the game-owned movement helper rather than restoring an engine voxel system.
        let world = VoxelWorld::new(4, 4, 4, vec![0; 64]);
        let collider = BodyShape::new(0.1, [0.0]);
        let start = Vect::new(1.0, 1.0, 2.0);
        let moved = slide_xy(&world, 0, start, (1.0, 0.5), &collider);
        assert_eq!(moved, Vect::new(2.0, 1.5, 2.0));
    }
}
