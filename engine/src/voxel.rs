//! Runtime-sized voxel storage and grid queries.

use crate::math::Vect;

#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub cell: (i32, i32, i32),
    pub normal: [i32; 3],
    pub dist: f32,
    pub point: Vect,
}

pub struct VoxelWorld {
    width: usize,
    height: usize,
    depth: usize,
    blocks: Vec<u8>,
}

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

    pub fn get_checked(&self, z: i32, y: i32, x: i32) -> Option<u8> {
        if z < 0 || y < 0 || x < 0 {
            return None;
        }
        let (z, y, x) = (z as usize, y as usize, x as usize);
        if z >= self.depth || y >= self.height || x >= self.width {
            return None;
        }
        Some(self.blocks[Self::idx(self.width, self.height, z, y, x)])
    }

    pub fn tile_open(&self, x: i32, y: i32, z: i32, empty: u8) -> bool {
        self.get_checked(z, y, x) == Some(empty)
    }

    pub fn is_solid_at(&self, x: f32, y: f32, z: f32, empty: u8) -> bool {
        let (cx, cy, cz) = (x.floor() as i32, y.floor() as i32, z.floor() as i32);
        if cx < 0 || cy < 0 || cx >= self.width as i32 || cy >= self.height as i32 || cz < 0 {
            return true;
        }
        match self.get_checked(cz, cy, cx) {
            Some(block) => block != empty,
            None => false,
        }
    }

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
            let block = self.get_checked(cell[2], cell[1], cell[0])?;
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

    pub fn idx(width: usize, height: usize, z: usize, y: usize, x: usize) -> usize {
        (z * height + y) * width + x
    }
}
