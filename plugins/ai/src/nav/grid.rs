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
    pub fn line_of_walk(&self, a: Vec3, b: Vec3) -> bool {
        let Some((mut x0, mut y0)) = self.world_to_cell(a) else {
            return false;
        };
        let Some((x1, y1)) = self.world_to_cell(b) else {
            return false;
        };
        let dx = (x1 as i32 - x0 as i32).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let dy = -(y1 as i32 - y0 as i32).abs();
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            if !self.cell(x0, y0).is_some_and(|c| c.walkable) {
                return false;
            }
            if x0 == x1 && y0 == y1 {
                return true;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 = (x0 as i32 + sx) as u32;
            }
            if e2 <= dx {
                err += dx;
                y0 = (y0 as i32 + sy) as u32;
            }
        }
    }
    pub fn nearest_walkable(&self, p: Vec3, max_radius_cells: u32) -> Option<(u32, u32)> {
        let (sx, sy) = self.world_to_cell(p)?;
        if self.cell(sx, sy).is_some_and(|c| c.walkable) {
            return Some((sx, sy));
        }
        for r in 1..=max_radius_cells {
            let mut best = None;
            for y in sy.saturating_sub(r)..=(sy + r).min(self.height.saturating_sub(1)) {
                for x in sx.saturating_sub(r)..=(sx + r).min(self.width.saturating_sub(1)) {
                    if self.cell(x, y).is_some_and(|c| c.walkable) {
                        let d = (x as i64 - sx as i64).pow(2) + (y as i64 - sy as i64).pow(2);
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
