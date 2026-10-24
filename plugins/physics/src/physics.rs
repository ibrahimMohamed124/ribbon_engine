//! Platform-independent voxel body collision and horizontal wall sliding.

use ribbon_math::Vect;
use ribbon_plugin_voxel::VoxelWorld;

/// Axis-aligned box collider in world-space XYZ coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb3 {
    pub center: [f32; 3],
    pub half_extents: [f32; 3],
}

impl Aabb3 {
    /// Create a world-space AABB from its center and non-negative half-extents.
    pub fn new(center: [f32; 3], half_extents: [f32; 3]) -> Self {
        Self {
            center,
            half_extents: half_extents.map(|value| value.abs()),
        }
    }
}

/// Slide a vertical capsule-shaped player through static AABBs on the XZ plane.
///
/// `eye_position` is the camera/eye position. `eye_height` is the distance from
/// the capsule bottom to the eye, and `body_height` is the total capsule height.
/// X then Z are resolved independently so blocked movement can slide along walls.
/// This helper is independent of the renderer and can be reused by native 3D games.
pub fn slide_capsule_xz(
    eye_position: [f32; 3],
    delta_xz: [f32; 2],
    radius: f32,
    body_height: f32,
    eye_height: f32,
    colliders: &[Aabb3],
) -> [f32; 3] {
    let radius = radius.max(0.0);
    let body_height = body_height.max(0.0);
    let eye_height = eye_height.clamp(0.0, body_height);
    let mut position = eye_position;

    let mut candidate = position;
    candidate[0] += delta_xz[0];
    if !capsule_overlaps_aabb(candidate, radius, body_height, eye_height, colliders) {
        position[0] = candidate[0];
    }

    candidate = position;
    candidate[2] += delta_xz[1];
    if !capsule_overlaps_aabb(candidate, radius, body_height, eye_height, colliders) {
        position[2] = candidate[2];
    }
    position
}

fn capsule_overlaps_aabb(
    eye_position: [f32; 3],
    radius: f32,
    body_height: f32,
    eye_height: f32,
    colliders: &[Aabb3],
) -> bool {
    let bottom = eye_position[1] - eye_height;
    let top = bottom + body_height;
    let radius_squared = radius * radius;
    const EPSILON: f32 = 1.0e-4;

    colliders.iter().any(|collider| {
        let box_bottom = collider.center[1] - collider.half_extents[1];
        let box_top = collider.center[1] + collider.half_extents[1];
        if top <= box_bottom + EPSILON || bottom >= box_top - EPSILON {
            return false;
        }

        let min_x = collider.center[0] - collider.half_extents[0];
        let max_x = collider.center[0] + collider.half_extents[0];
        let min_z = collider.center[2] - collider.half_extents[2];
        let max_z = collider.center[2] + collider.half_extents[2];
        let closest_x = eye_position[0].clamp(min_x, max_x);
        let closest_z = eye_position[2].clamp(min_z, max_z);
        let dx = eye_position[0] - closest_x;
        let dz = eye_position[2] - closest_z;
        dx * dx + dz * dz < radius_squared - EPSILON
    })
}

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
    use ribbon_math::Vect;
    use ribbon_plugin_voxel::VoxelWorld;

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

    #[test]
    fn capsule_xz_movement_stops_at_walls_and_slides_along_them() {
        use super::{slide_capsule_xz, Aabb3};
        let wall = Aabb3::new([2.0, 1.5, 0.0], [0.1, 1.5, 4.0]);
        let colliders = [wall];
        let start = [1.4, 1.65, 0.0];
        let moved = slide_capsule_xz(start, [0.8, 0.8], 0.28, 1.72, 1.62, &colliders);
        assert!(
            (moved[0] - start[0]).abs() < 1.0e-5,
            "capsule crossed the wall: {moved:?}"
        );
        assert!(
            (moved[2] - 0.8).abs() < 1.0e-5,
            "capsule did not slide: {moved:?}"
        );
    }

    #[test]
    fn capsule_xz_ignores_boxes_outside_the_body_height_and_floor_contact() {
        use super::{slide_capsule_xz, Aabb3};
        let floor = Aabb3::new([0.0, -0.09, 0.0], [5.0, 0.09, 5.0]);
        let ceiling = Aabb3::new([0.0, 2.99, 0.0], [5.0, 0.09, 5.0]);
        let colliders = [floor, ceiling];
        let start = [0.0, 1.65, 0.0];
        assert_eq!(
            slide_capsule_xz(start, [1.0, 0.5], 0.28, 1.72, 1.62, &colliders),
            [1.0, 1.65, 0.5]
        );
    }
}
