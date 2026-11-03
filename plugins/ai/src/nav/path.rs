//! A* pathfinding and greedy visibility smoothing.
use super::NavGrid;
use glam::Vec3;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavError {
    NoGrid,
    StartBlocked,
    GoalBlocked,
    NoPath,
    OutOfBounds,
}
#[derive(Clone, Debug, PartialEq)]
pub struct NavPath {
    pub waypoints: Vec<Vec3>,
    pub length: f32,
}
impl NavGrid {
    pub fn find_path(&self, start: Vec3, goal: Vec3) -> Result<NavPath, NavError> {
        self.find_path_with_limit(start, goal, 200_000)
    }
    pub fn find_path_with_limit(
        &self,
        start: Vec3,
        goal: Vec3,
        max_expansions: usize,
    ) -> Result<NavPath, NavError> {
        if self.width == 0 || self.height == 0 {
            return Err(NavError::NoGrid);
        }
        let s = self
            .nearest_walkable(start, 3)
            .ok_or(NavError::StartBlocked)?;
        let g = self
            .nearest_walkable(goal, 3)
            .ok_or(NavError::GoalBlocked)?;
        let mut expanded = 0usize;
        let result = pathfinding::directed::astar::astar(
            &s,
            |&(x, y)| {
                expanded = expanded.saturating_add(1);
                if expanded > max_expansions {
                    return Vec::new();
                }
                let mut out = Vec::new();
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        let nx = x as i32 + dx;
                        let ny = y as i32 + dy;
                        if nx < 0 || ny < 0 {
                            continue;
                        }
                        let (nx, ny) = (nx as u32, ny as u32);
                        let Some(c) = self.cell(nx, ny) else { continue };
                        if !c.walkable {
                            continue;
                        }
                        if dx != 0
                            && dy != 0
                            && (!self
                                .cell((x as i32 + dx) as u32, y)
                                .is_some_and(|c| c.walkable)
                                || !self
                                    .cell(x, (y as i32 + dy) as u32)
                                    .is_some_and(|c| c.walkable))
                        {
                            continue;
                        }
                        let cost = if dx == 0 || dy == 0 { 10 } else { 14 };
                        out.push(((nx, ny), cost * c.cost as u32));
                    }
                }
                out
            },
            |&(x, y)| {
                let dx = x.abs_diff(g.0);
                let dy = y.abs_diff(g.1);
                10 * (dx + dy) - 6 * dx.min(dy)
            },
            |&p| p == g,
        );
        let (nodes, _) = result.ok_or(NavError::NoPath)?;
        let mut smooth = Vec::new();
        let mut i = 0;
        while i < nodes.len() {
            smooth.push(nodes[i]);
            if i + 1 >= nodes.len() {
                break;
            }
            let mut far = i + 1;
            for j in (i + 1..nodes.len()).rev() {
                if self.line_of_walk(
                    self.cell_center(nodes[i].0, nodes[i].1),
                    self.cell_center(nodes[j].0, nodes[j].1),
                ) {
                    far = j;
                    break;
                }
            }
            i = far;
        }
        let waypoints = smooth
            .into_iter()
            .map(|(x, y)| self.cell_center(x, y))
            .collect::<Vec<_>>();
        let length = waypoints.windows(2).map(|w| w[0].distance(w[1])).sum();
        Ok(NavPath { waypoints, length })
    }
}
