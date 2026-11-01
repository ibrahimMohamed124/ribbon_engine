//! Platform-independent AABB collision helpers for mesh-based worlds.

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

#[cfg(test)]
mod tests {
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
