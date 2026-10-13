//! The maze, the voxel storage, and voxel ray queries.
//!
//! `World` knows nothing about rendering, players or enemies; everything else
//! asks it questions ("is this tile open?", "what does this ray hit?").

use crate::math::{Rng, Vect};

pub const CELLS: usize = 10; // maze is CELLS x CELLS cells, each 2x2 tiles with 1-tile walls
pub const X_BLOCKS: usize = CELLS * 3 + 1;
pub const Y_BLOCKS: usize = CELLS * 3 + 1;
pub const Z_BLOCKS: usize = 6; // z=0 floor, z=1..=4 walls, z=5 ceiling
pub const FLOOR_TOP: f32 = 1.0;

pub const AIR: u8 = b' ';
pub const FLOOR: u8 = b'f';
pub const WALL: u8 = b'#';
pub const CEIL: u8 = b'c';

pub struct World {
    blocks: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub cell: (i32, i32, i32), // x, y, z
    pub normal: [i32; 3],
    pub dist: f32,
    pub point: Vect,
}

/// First tile (x or y) of maze cell `c`.
pub fn origin(c: usize) -> usize {
    1 + 3 * c
}

/// World coordinate of the centre of maze cell `c` (same value for x and y).
pub fn cell_center(c: usize) -> f32 {
    origin(c) as f32 + 1.0
}

impl World {
    pub fn idx(z: usize, y: usize, x: usize) -> usize {
        (z * Y_BLOCKS + y) * X_BLOCKS + x
    }

    /// Raw block data, z-major. Mostly useful for tests and tooling.
    pub fn blocks(&self) -> &[u8] {
        &self.blocks
    }

    pub fn get_checked(&self, z: i32, y: i32, x: i32) -> Option<u8> {
        if z < 0 || y < 0 || x < 0 {
            return None;
        }
        let (z, y, x) = (z as usize, y as usize, x as usize);
        if z >= Z_BLOCKS || y >= Y_BLOCKS || x >= X_BLOCKS {
            return None;
        }
        Some(self.blocks[Self::idx(z, y, x)])
    }

    /// Is the floor tile (x, y) walkable?
    pub fn tile_open(&self, x: i32, y: i32) -> bool {
        self.get_checked(1, y, x) == Some(AIR)
    }

    /// Is the point inside a solid block? Outside the horizontal bounds counts as solid.
    pub fn is_solid_at(&self, x: f32, y: f32, z: f32) -> bool {
        let (cx, cy, cz) = (x.floor() as i32, y.floor() as i32, z.floor() as i32);
        if cx < 0 || cy < 0 || cx >= X_BLOCKS as i32 || cy >= Y_BLOCKS as i32 || cz < 0 {
            return true;
        }
        match self.get_checked(cz, cy, cx) {
            Some(c) => c != AIR,
            None => false,
        }
    }

    /// Amanatides & Woo voxel traversal.
    pub fn cast(&self, origin: Vect, dir: Vect, max_dist: f32) -> Option<Hit> {
        let o = [origin.x, origin.y, origin.z];
        let d = [dir.x, dir.y, dir.z];
        let mut cell = [o[0].floor() as i32, o[1].floor() as i32, o[2].floor() as i32];

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
            let c = self.get_checked(cell[2], cell[1], cell[0])?;
            if c != AIR {
                return Some(Hit {
                    cell: (cell[0], cell[1], cell[2]),
                    normal,
                    dist: t,
                    point: origin.add(dir.scale(t)),
                });
            }
            let axis = if t_max[0] < t_max[1] {
                if t_max[0] < t_max[2] { 0 } else { 2 }
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

    /// Generate a maze: recursive backtracker + extra loops + a few big rooms.
    pub fn generate(rng: &mut Rng) -> World {
        let mut open = vec![false; X_BLOCKS * Y_BLOCKS];
        for cy in 0..CELLS {
            for cx in 0..CELLS {
                for dy in 0..2 {
                    for dx in 0..2 {
                        open[(origin(cy) + dy) * X_BLOCKS + origin(cx) + dx] = true;
                    }
                }
            }
        }

        // recursive-backtracker maze
        let mut visited = [[false; CELLS]; CELLS];
        let mut stack = vec![(0usize, 0usize)];
        visited[0][0] = true;
        while let Some(&(cx, cy)) = stack.last() {
            let mut nbrs: Vec<(usize, usize)> = Vec::new();
            if cx + 1 < CELLS && !visited[cy][cx + 1] { nbrs.push((cx + 1, cy)); }
            if cx > 0 && !visited[cy][cx - 1] { nbrs.push((cx - 1, cy)); }
            if cy + 1 < CELLS && !visited[cy + 1][cx] { nbrs.push((cx, cy + 1)); }
            if cy > 0 && !visited[cy - 1][cx] { nbrs.push((cx, cy - 1)); }
            if nbrs.is_empty() {
                stack.pop();
                continue;
            }
            let n = nbrs[rng.below(nbrs.len())];
            carve_between(&mut open, (cx, cy), n);
            visited[n.1][n.0] = true;
            stack.push(n);
        }

        // extra openings so there are loops to run around
        for cy in 0..CELLS {
            for cx in 0..CELLS {
                if cx + 1 < CELLS && !open[origin(cy) * X_BLOCKS + origin(cx) + 2] && rng.chance(0.18) {
                    carve_east(&mut open, cx, cy);
                }
                if cy + 1 < CELLS && !open[(origin(cy) + 2) * X_BLOCKS + origin(cx)] && rng.chance(0.18) {
                    carve_south(&mut open, cx, cy);
                }
            }
        }

        // a few bigger rooms (2x2 cells) with a single pillar in the middle
        for _ in 0..3 {
            let rx = rng.below(CELLS - 1);
            let ry = rng.below(CELLS - 1);
            carve_east(&mut open, rx, ry);
            carve_east(&mut open, rx, ry + 1);
            carve_south(&mut open, rx, ry);
            carve_south(&mut open, rx + 1, ry);
        }

        let mut blocks = vec![AIR; X_BLOCKS * Y_BLOCKS * Z_BLOCKS];
        for y in 0..Y_BLOCKS {
            for x in 0..X_BLOCKS {
                blocks[World::idx(0, y, x)] = FLOOR;
                blocks[World::idx(Z_BLOCKS - 1, y, x)] = CEIL;
                if !open[y * X_BLOCKS + x] {
                    for z in 1..Z_BLOCKS - 1 {
                        blocks[World::idx(z, y, x)] = WALL;
                    }
                }
            }
        }
        World { blocks }
    }
}

fn carve_east(open: &mut [bool], cx: usize, cy: usize) {
    let x = origin(cx) + 2;
    for dy in 0..2 {
        open[(origin(cy) + dy) * X_BLOCKS + x] = true;
    }
}

fn carve_south(open: &mut [bool], cx: usize, cy: usize) {
    let y = origin(cy) + 2;
    for dx in 0..2 {
        open[y * X_BLOCKS + origin(cx) + dx] = true;
    }
}

fn carve_between(open: &mut [bool], a: (usize, usize), b: (usize, usize)) {
    if b.0 > a.0 {
        carve_east(open, a.0, a.1);
    } else if b.0 < a.0 {
        carve_east(open, b.0, b.1);
    } else if b.1 > a.1 {
        carve_south(open, a.0, a.1);
    } else {
        carve_south(open, b.0, b.1);
    }
}
