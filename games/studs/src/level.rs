//! The maze, the voxel storage, and voxel ray queries.
//!
//! `World` knows nothing about rendering, players or enemies; everything else
//! asks it questions ("is this tile open?", "what does this ray hit?").

use ribbon_math::Rng;
use std::ops::Deref;
use studs_voxel::{ChunkedVoxelWorld, VoxelWorld};

pub const CELLS: usize = 10; // maze is CELLS x CELLS cells, each 2x2 tiles with 1-tile walls
pub const X_BLOCKS: usize = CELLS * 3 + 1;
pub const Y_BLOCKS: usize = CELLS * 3 + 1;
pub const Z_BLOCKS: usize = 6; // z=0 floor, z=1..=4 walls, z=5 ceiling
pub const FLOOR_TOP: f32 = 1.0;

pub const AIR: u8 = b' ';
pub const FLOOR: u8 = b'f';
pub const WALL: u8 = b'#';
pub const CEIL: u8 = b'c';

/// STUDS maze data backed by the engine's mutable, chunked voxel world.
/// The maze generator owns block meanings and layout; chunk invalidation and meshing
/// remain in `studs_voxel` and the GPU upload path remains in `ribbon_plugin_render3d`.
pub struct World(ChunkedVoxelWorld);

/// First tile (x or y) of maze cell `c`.
pub fn origin(c: usize) -> usize {
    1 + 3 * c
}

/// World coordinate of the centre of maze cell `c` (same value for x and y).
pub fn cell_center(c: usize) -> f32 {
    origin(c) as f32 + 1.0
}

impl World {
    /// Convert `(z, y, x)` maze storage coordinates into the z-major flat index.
    pub fn idx(z: usize, y: usize, x: usize) -> usize {
        (z * Y_BLOCKS + y) * X_BLOCKS + x
    }

    pub fn blocks(&self) -> &[u8] {
        self.0.blocks()
    }

    /// Mutable access to the engine-owned chunked voxel data for edits made by game rules.
    pub fn chunked_world_mut(&mut self) -> &mut ChunkedVoxelWorld {
        &mut self.0
    }

    pub fn tile_open(&self, x: i32, y: i32) -> bool {
        self.0.tile_open(x, y, 1, AIR)
    }

    pub fn is_solid_at(&self, x: f32, y: f32, z: f32) -> bool {
        self.0.is_solid_at(x, y, z, AIR)
    }

    pub fn cast(
        &self,
        origin: ribbon_math::Vect,
        dir: ribbon_math::Vect,
        max_dist: f32,
    ) -> Option<studs_voxel::Hit> {
        self.0.cast(origin, dir, max_dist, AIR)
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
            if cx + 1 < CELLS && !visited[cy][cx + 1] {
                nbrs.push((cx + 1, cy));
            }
            if cx > 0 && !visited[cy][cx - 1] {
                nbrs.push((cx - 1, cy));
            }
            if cy + 1 < CELLS && !visited[cy + 1][cx] {
                nbrs.push((cx, cy + 1));
            }
            if cy > 0 && !visited[cy - 1][cx] {
                nbrs.push((cx, cy - 1));
            }
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
                if cx + 1 < CELLS
                    && !open[origin(cy) * X_BLOCKS + origin(cx) + 2]
                    && rng.chance(0.18)
                {
                    carve_east(&mut open, cx, cy);
                }
                if cy + 1 < CELLS
                    && !open[(origin(cy) + 2) * X_BLOCKS + origin(cx)]
                    && rng.chance(0.18)
                {
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
        World(ChunkedVoxelWorld::new(
            VoxelWorld::new(X_BLOCKS, Y_BLOCKS, Z_BLOCKS, blocks),
            [8, 8, Z_BLOCKS],
            AIR,
        ))
    }
}

impl Deref for World {
    type Target = VoxelWorld;

    fn deref(&self) -> &Self::Target {
        &self.0
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
