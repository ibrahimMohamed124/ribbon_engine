//! Runtime-sized voxel storage and grid queries.

use engine_math::Vect;

#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub cell: (i32, i32, i32),
    pub normal: [i32; 3],
    pub dist: f32,
    pub point: Vect,
}

/// Repeating texture coordinates on the plane of the hit face.
///
/// The normal axis is omitted, so neighboring voxel faces meet at matching
/// integer texture boundaries. Procedural textures should tile at those edges.
pub fn face_uv(hit: &Hit) -> (f32, f32) {
    let fract = |value: f32| {
        let value = value - value.floor();
        if value >= 1.0 {
            0.0
        } else {
            value
        }
    };
    let (u, v) = if hit.normal[0] != 0 {
        (hit.point.y, hit.point.z)
    } else if hit.normal[1] != 0 {
        (hit.point.x, hit.point.z)
    } else {
        (hit.point.x, hit.point.y)
    };
    (fract(u), fract(v))
}

/// Sample classic voxel ambient occlusion around the four corners of a face.
///
/// The outside face cell is `hit.cell + hit.normal`. Three neighboring cells
/// are sampled per corner; out-of-bounds space is treated as solid.
pub fn face_ao(world: &VoxelWorld, hit: &Hit, empty: u8) -> f32 {
    let normal_axis = if hit.normal[0] != 0 {
        0
    } else if hit.normal[1] != 0 {
        1
    } else if hit.normal[2] != 0 {
        2
    } else {
        return 1.0;
    };
    let axes = match normal_axis {
        0 => [1, 2],
        1 => [0, 2],
        _ => [0, 1],
    };
    let mut outside = [hit.cell.0, hit.cell.1, hit.cell.2];
    outside[normal_axis] += hit.normal[normal_axis];

    let (u, v) = face_uv(hit);
    let mut corners = [[0.0; 2]; 2];
    for (iu, su) in [-1, 1].into_iter().enumerate() {
        for (iv, sv) in [-1, 1].into_iter().enumerate() {
            let mut side_u = outside;
            let mut side_v = outside;
            let mut corner = outside;
            side_u[axes[0]] += su;
            side_v[axes[1]] += sv;
            corner[axes[0]] += su;
            corner[axes[1]] += sv;
            let solid = |cell: [i32; 3]| {
                world
                    .get(cell[0], cell[1], cell[2])
                    .is_none_or(|block| block != empty)
            };
            let a = solid(side_u);
            let b = solid(side_v);
            let c = solid(corner);
            corners[iu][iv] = if a && b {
                0.0
            } else {
                (3 - i32::from(a) - i32::from(b) - i32::from(c)) as f32 / 3.0
            };
        }
    }

    let top = corners[0][0] * (1.0 - u) + corners[1][0] * u;
    let bottom = corners[0][1] * (1.0 - u) + corners[1][1] * u;
    (top * (1.0 - v) + bottom * v).clamp(0.0, 1.0)
}

pub struct VoxelWorld {
    width: usize,
    height: usize,
    depth: usize,
    blocks: Vec<u8>,
}

/// Shared collision/raycast grid resource with its empty-block identifier.
pub struct VoxelWorldRes(pub VoxelWorld, pub u8);

impl VoxelWorld {
    pub fn new(width: usize, height: usize, depth: usize, blocks: Vec<u8>) -> Self {
        assert_eq!(blocks.len(), width * height * depth);
        Self {
            width,
            height,
            depth,
            blocks,
        }
    }

    pub fn dimensions(&self) -> (usize, usize, usize) {
        (self.width, self.height, self.depth)
    }

    pub fn blocks(&self) -> &[u8] {
        &self.blocks
    }

    /// Get a voxel by world coordinates `(x, y, z)`, returning `None` outside the grid.
    pub fn get(&self, x: i32, y: i32, z: i32) -> Option<u8> {
        if x < 0 || y < 0 || z < 0 {
            return None;
        }
        let (x, y, z) = (x as usize, y as usize, z as usize);
        if x >= self.width || y >= self.height || z >= self.depth {
            return None;
        }
        Some(self.blocks[Self::idx(self.width, self.height, z, y, x)])
    }

    pub fn tile_open(&self, x: i32, y: i32, z: i32, empty: u8) -> bool {
        self.get(x, y, z) == Some(empty)
    }

    /// Test a point against the world. Out-of-range x/y and negative z are solid;
    /// z at or above the world depth is open space.
    pub fn is_solid_at(&self, x: f32, y: f32, z: f32, empty: u8) -> bool {
        let (cx, cy, cz) = (x.floor() as i32, y.floor() as i32, z.floor() as i32);
        if cx < 0 || cy < 0 || cx >= self.width as i32 || cy >= self.height as i32 || cz < 0 {
            return true;
        }
        match self.get(cx, cy, cz) {
            Some(block) => block != empty,
            None => false,
        }
    }

    /// Cast a ray through the grid, returning the first non-empty voxel up to `max_dist`.
    /// Rays whose origin cell lies outside the grid return `None`.
    pub fn cast(&self, origin: Vect, dir: Vect, max_dist: f32, empty: u8) -> Option<Hit> {
        let o = [origin.x, origin.y, origin.z];
        let d = [dir.x, dir.y, dir.z];
        let mut cell = [
            o[0].floor() as i32,
            o[1].floor() as i32,
            o[2].floor() as i32,
        ];

        let mut step = [0i32; 3];
        let mut t_delta = [f32::INFINITY; 3];
        let mut t_max = [f32::INFINITY; 3];
        for i in 0..3 {
            if d[i] > 0.0 {
                step[i] = 1;
                t_delta[i] = 1.0 / d[i];
                t_max[i] = ((cell[i] + 1) as f32 - o[i]) * t_delta[i];
            } else if d[i] < 0.0 {
                step[i] = -1;
                t_delta[i] = -1.0 / d[i];
                t_max[i] = (o[i] - cell[i] as f32) * t_delta[i];
            }
        }

        let mut t = 0.0f32;
        let mut normal = [0i32; 3];
        loop {
            let block = self.get(cell[0], cell[1], cell[2])?;
            if block != empty {
                return Some(Hit {
                    cell: (cell[0], cell[1], cell[2]),
                    normal,
                    dist: t,
                    point: origin + dir.scale(t),
                });
            }
            let axis = if t_max[0] < t_max[1] {
                if t_max[0] < t_max[2] {
                    0
                } else {
                    2
                }
            } else if t_max[1] < t_max[2] {
                1
            } else {
                2
            };
            if t_max[axis] > max_dist {
                return None;
            }
            t = t_max[axis];
            cell[axis] += step[axis];
            t_max[axis] += t_delta[axis];
            normal = [0; 3];
            normal[axis] = -step[axis];
        }
    }

    /// Convert `(z, y, x)` storage coordinates into the z-major flat buffer index.
    pub fn idx(width: usize, height: usize, z: usize, y: usize, x: usize) -> usize {
        (z * height + y) * width + x
    }
}

#[cfg(test)]
mod tests {
    use super::{face_ao, face_uv, Hit, VoxelWorld};
    use engine_math::Vect;

    const EMPTY: u8 = 0;
    const SOLID: u8 = 1;

    fn world_with_solids(cells: &[(i32, i32, i32)]) -> VoxelWorld {
        let (width, height, depth) = (8, 8, 8);
        let mut blocks = vec![EMPTY; width * height * depth];
        for &(x, y, z) in cells {
            blocks[VoxelWorld::idx(width, height, z as usize, y as usize, x as usize)] = SOLID;
        }
        VoxelWorld::new(width, height, depth, blocks)
    }

    fn x_face(point: Vect) -> Hit {
        Hit {
            cell: (3, 3, 3),
            normal: [1, 0, 0],
            dist: 1.0,
            point,
        }
    }

    #[test]
    fn face_uv_uses_the_two_axes_of_each_face() {
        let point = Vect::new(-2.25, 3.5, 4.75);
        let cases = [
            ([1, 0, 0], (0.5, 0.75)),
            ([-1, 0, 0], (0.5, 0.75)),
            ([0, 1, 0], (0.75, 0.75)),
            ([0, -1, 0], (0.75, 0.75)),
            ([0, 0, 1], (0.75, 0.5)),
            ([0, 0, -1], (0.75, 0.5)),
        ];
        for (normal, expected) in cases {
            let hit = Hit {
                cell: (0, 0, 0),
                normal,
                dist: 0.0,
                point,
            };
            let uv = face_uv(&hit);
            assert_eq!(uv, expected);
            assert!((0.0..1.0).contains(&uv.0));
            assert!((0.0..1.0).contains(&uv.1));
        }
    }

    #[test]
    fn adjacent_cells_share_uv_for_the_same_world_point() {
        let point = Vect::new(4.375, 8.625, 2.125);
        let left = Hit {
            cell: (1, 2, 3),
            normal: [0, 0, 1],
            dist: 0.0,
            point,
        };
        let right = Hit {
            cell: (2, 2, 3),
            ..left
        };
        assert_eq!(face_uv(&left), face_uv(&right));
    }

    #[test]
    fn flat_face_center_has_no_occlusion() {
        let world = world_with_solids(&[(3, 3, 3)]);
        assert_eq!(
            face_ao(&world, &x_face(Vect::new(4.0, 3.5, 3.5)), EMPTY),
            1.0
        );
    }

    #[test]
    fn concave_corner_is_darker_and_ao_is_bounded() {
        let world = world_with_solids(&[(3, 3, 3), (4, 2, 3), (4, 3, 2), (4, 2, 2)]);
        let corner = face_ao(&world, &x_face(Vect::new(4.0, 3.01, 3.01)), EMPTY);
        assert!(corner < 1.0);
        for u in [0.0, 0.1, 0.5, 0.9, 0.999] {
            for v in [0.0, 0.1, 0.5, 0.9, 0.999] {
                let ao = face_ao(&world, &x_face(Vect::new(4.0, 3.0 + u, 3.0 + v)), EMPTY);
                assert!((0.0..=1.0).contains(&ao));
            }
        }
    }

    #[test]
    fn symmetric_concave_corner_has_symmetric_interpolation() {
        let world = world_with_solids(&[(3, 3, 3), (4, 2, 3), (4, 3, 2), (4, 2, 2)]);
        let a = face_ao(&world, &x_face(Vect::new(4.0, 3.25, 3.75)), EMPTY);
        let b = face_ao(&world, &x_face(Vect::new(4.0, 3.75, 3.25)), EMPTY);
        assert!((a - b).abs() < 1e-6);
    }

    #[test]
    fn canonical_get_uses_xyz_order_and_returns_none_outside() {
        let world = world_with_solids(&[(3, 4, 5)]);
        assert_eq!(world.get(3, 4, 5), Some(SOLID));
        assert_eq!(world.get(5, 4, 3), Some(EMPTY));
        assert_eq!(world.get(-1, 4, 5), None);
        assert_eq!(world.get(3, 8, 5), None);
        assert_eq!(world.get(3, 4, 8), None);
    }

    #[test]
    fn point_query_keeps_its_asymmetric_vertical_bounds() {
        let world = world_with_solids(&[]);
        assert!(world.is_solid_at(-0.1, 1.0, 2.0, EMPTY));
        assert!(world.is_solid_at(1.0, -0.1, 2.0, EMPTY));
        assert!(world.is_solid_at(1.0, 1.0, -0.1, EMPTY));
        assert!(!world.is_solid_at(1.0, 1.0, 8.0, EMPTY));
    }

    #[test]
    fn cast_documents_origin_hit_and_distance_limit() {
        let world = world_with_solids(&[(3, 3, 3), (5, 3, 3)]);
        let origin_hit = world
            .cast(
                Vect::new(3.5, 3.5, 3.5),
                Vect::new(1.0, 0.0, 0.0),
                8.0,
                EMPTY,
            )
            .unwrap();
        assert_eq!(origin_hit.dist, 0.0);
        assert_eq!(origin_hit.normal, [0, 0, 0]);
        assert!(world
            .cast(
                Vect::new(-0.5, 3.5, 3.5),
                Vect::new(1.0, 0.0, 0.0),
                8.0,
                EMPTY
            )
            .is_none());
        assert!(world
            .cast(
                Vect::new(4.5, 3.5, 3.5),
                Vect::new(1.0, 0.0, 0.0),
                0.49,
                EMPTY
            )
            .is_none());
        assert!(world
            .cast(
                Vect::new(4.5, 3.5, 3.5),
                Vect::new(1.0, 0.0, 0.0),
                0.5,
                EMPTY
            )
            .is_some());
    }
}
