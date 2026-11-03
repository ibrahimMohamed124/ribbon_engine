//! 2.5D navigation grid data and cell-space helpers.
use glam::{Vec2, Vec3};
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavCell {
    pub walkable: bool,
    pub ground_y: f32,
    pub cost: u8,
}
#[derive(Clone, Debug)]
pub struct NavGrid {
    pub origin: Vec2,
    pub cell_size: f32,
    pub width: u32,
    pub height: u32,
    pub(crate) cells: Vec<NavCell>,
}
#[derive(Clone, Debug)]
pub struct NavWorld {
    pub grid: NavGrid,
}
impl NavGrid {
    pub fn new(origin: Vec2, cell_size: f32, width: u32, height: u32) -> Self {
        let n = (width as usize).saturating_mul(height as usize);
        Self {
            origin,
            cell_size: if cell_size.is_finite() && cell_size > 0.0 {
                cell_size
            } else {
                1.0
            },
            width,
            height,
            cells: vec![
                NavCell {
                    walkable: true,
                    ground_y: 0.0,
                    cost: 1
                };
                n
            ],
        }
    }
    fn index(&self, x: u32, y: u32) -> Option<usize> {
        (x < self.width && y < self.height).then_some(y as usize * self.width as usize + x as usize)
    }
    pub fn world_to_cell(&self, p: Vec3) -> Option<(u32, u32)> {
        if !p.is_finite() {
            return None;
        }
        let q = (glam::Vec2::new(p.x, p.z) - self.origin) / self.cell_size;
        if q.x < 0.0 || q.y < 0.0 || q.x >= self.width as f32 || q.y >= self.height as f32 {
            None
        } else {
            Some((q.x.floor() as u32, q.y.floor() as u32))
        }
    }
    pub fn cell_center(&self, x: u32, y: u32) -> Vec3 {
        let ground = self.cell(x, y).map_or(0.0, |c| c.ground_y);
        Vec3::new(
            self.origin.x + (x as f32 + 0.5) * self.cell_size,
            ground,
            self.origin.y + (y as f32 + 0.5) * self.cell_size,
        )
    }
    pub fn cell(&self, x: u32, y: u32) -> Option<&NavCell> {
        self.index(x, y).and_then(|i| self.cells.get(i))
    }
    pub fn set_walkable(&mut self, x: u32, y: u32, walkable: bool) {
        if let Some(i) = self.index(x, y) {
            self.cells[i].walkable = walkable;
        }
    }
    pub fn set_cost(&mut self, x: u32, y: u32, cost: u8) {
        if let Some(i) = self.index(x, y) {
            self.cells[i].cost = cost.max(1);
        }
    }
    pub(crate) fn set_ground(&mut self, x: u32, y: u32, ground: f32) {
        if let Some(i) = self.index(x, y) {
            self.cells[i].ground_y = ground;
        }
    }
    /// Returns true only when every grid cell touched by the segment is walkable.
    /// Continuous supercover DDA uses the actual segment endpoints (not just their
    /// containing cell indices), including both side cells at exact corner crossings
    /// and both strips for segments that lie on a grid boundary.
    pub fn line_of_walk(&self, a: Vec3, b: Vec3) -> bool {
        if !a.is_finite() || !b.is_finite() || self.width == 0 || self.height == 0 {
            return false;
        }
        let qa = (Vec2::new(a.x, a.z) - self.origin) / self.cell_size;
        let qb = (Vec2::new(b.x, b.z) - self.origin) / self.cell_size;
        if !qa.is_finite()
            || !qb.is_finite()
            || qa.x < 0.0
            || qa.y < 0.0
            || qb.x < 0.0
            || qb.y < 0.0
            || qa.x >= self.width as f32
            || qb.x >= self.width as f32
            || qa.y >= self.height as f32
            || qb.y >= self.height as f32
        {
            return false;
        }
        let walkable = |cx: i64, cy: i64| -> bool {
            cx >= 0
                && cy >= 0
                && cx < self.width as i64
                && cy < self.height as i64
                && self
                    .cell(cx as u32, cy as u32)
                    .is_some_and(|cell| cell.walkable)
        };
        let on_boundary = |value: f64| (value - value.round()).abs() <= 1.0e-6;
        let qa = (qa.x as f64, qa.y as f64);
        let qb = (qb.x as f64, qb.y as f64);
        let mut x = qa.0.floor() as i64;
        let mut y = qa.1.floor() as i64;
        let end_x = qb.0.floor() as i64;
        let end_y = qb.1.floor() as i64;
        let dx = qb.0 - qa.0;
        let dy = qb.1 - qa.1;

        // A segment endpoint on a grid line touches cells on both sides. Checking
        // the full neighborhood also handles endpoints exactly on a grid corner.
        let check_endpoint = |point: (f64, f64)| -> bool {
            let x = point.0.floor() as i64;
            let y = point.1.floor() as i64;
            let mut xs = [x, x];
            let mut ys = [y, y];
            let x_count = if on_boundary(point.0) {
                xs[0] = x - 1;
                2
            } else {
                1
            };
            let y_count = if on_boundary(point.1) {
                ys[0] = y - 1;
                2
            } else {
                1
            };
            (0..x_count).all(|ix| {
                (0..y_count).all(|iy| {
                    let cx = xs[ix];
                    let cy = ys[iy];
                    // The grid's outer border has no neighboring cell outside
                    // the authored world. Internal boundaries must check both
                    // adjacent cells, while an outer boundary checks its one
                    // in-grid side only.
                    (cx < 0 || cy < 0 || cx >= self.width as i64 || cy >= self.height as i64)
                        || walkable(cx, cy)
                })
            })
        };
        if !check_endpoint(qa) || !check_endpoint(qb) {
            return false;
        }

        let step_x = if dx > 0.0 {
            1_i64
        } else if dx < 0.0 {
            -1_i64
        } else {
            0_i64
        };
        let step_y = if dy > 0.0 {
            1_i64
        } else if dy < 0.0 {
            -1_i64
        } else {
            0_i64
        };
        let infinity = f64::INFINITY;
        let delta_x = if step_x == 0 {
            infinity
        } else {
            1.0 / dx.abs()
        };
        let delta_y = if step_y == 0 {
            infinity
        } else {
            1.0 / dy.abs()
        };
        let mut max_x = if step_x > 0 {
            (x as f64 + 1.0 - qa.0) / dx
        } else if step_x < 0 {
            (qa.0 - x as f64) / -dx
        } else {
            infinity
        };
        let mut max_y = if step_y > 0 {
            (y as f64 + 1.0 - qa.1) / dy
        } else if step_y < 0 {
            (qa.1 - y as f64) / -dy
        } else {
            infinity
        };

        // A line parallel to a grid boundary touches both adjacent cell strips
        // for its entire length, not just the cell chosen by floor().
        if step_x == 0 && on_boundary(qa.0) {
            let boundary = qa.0.round() as i64;
            let min_y = y.min(end_y);
            let max_y = y.max(end_y);
            for cy in min_y..=max_y {
                let left = boundary - 1;
                let right = boundary;
                if (left >= 0 && !walkable(left, cy))
                    || (right < self.width as i64 && !walkable(right, cy))
                {
                    return false;
                }
            }
        }
        if step_y == 0 && on_boundary(qa.1) {
            let boundary = qa.1.round() as i64;
            let min_x = x.min(end_x);
            let max_x = x.max(end_x);
            for cx in min_x..=max_x {
                let top = boundary - 1;
                let bottom = boundary;
                if (top >= 0 && !walkable(cx, top))
                    || (bottom < self.height as i64 && !walkable(cx, bottom))
                {
                    return false;
                }
            }
        }

        if !walkable(x, y) {
            return false;
        }
        while x != end_x || y != end_y {
            let scale = max_x.abs().max(max_y.abs()).max(1.0);
            let corner =
                max_x.is_finite() && max_y.is_finite() && (max_x - max_y).abs() <= 1.0e-7 * scale;
            if corner {
                // At a corner crossing the mathematical segment touches both
                // orthogonal neighbors as well as the diagonal destination.
                if !walkable(x + step_x, y) || !walkable(x, y + step_y) {
                    return false;
                }
                x += step_x;
                y += step_y;
                max_x += delta_x;
                max_y += delta_y;
            } else if max_x < max_y {
                x += step_x;
                max_x += delta_x;
            } else {
                y += step_y;
                max_y += delta_y;
            }
            if !walkable(x, y) {
                return false;
            }
        }
        true
    }
    pub fn nearest_walkable(&self, p: Vec3, max_radius_cells: u32) -> Option<(u32, u32)> {
        let (sx, sy) = self.world_to_cell(p)?;
        if self.cell(sx, sy).is_some_and(|c| c.walkable) {
            return Some((sx, sy));
        }
        for r in 1..=max_radius_cells {
            let mut best = None;
            for y in sy.saturating_sub(r)..=sy.saturating_add(r).min(self.height.saturating_sub(1))
            {
                for x in
                    sx.saturating_sub(r)..=sx.saturating_add(r).min(self.width.saturating_sub(1))
                {
                    if self.cell(x, y).is_some_and(|c| c.walkable) {
                        let dx = i128::from(x) - i128::from(sx);
                        let dy = i128::from(y) - i128::from(sy);
                        let d = dx * dx + dy * dy;
                        if best.is_none_or(|(_, _, bd)| d < bd) {
                            best = Some((x, y, d));
                        }
                    }
                }
            }
            if let Some((x, y, _)) = best {
                return Some((x, y));
            }
        }
        None
    }
}
