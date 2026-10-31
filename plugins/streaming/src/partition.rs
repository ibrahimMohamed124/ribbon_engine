use glam::Vec3;
use std::collections::{HashMap, HashSet};

use crate::RegionId;

/// Authored world-space, axis-aligned bounds for a streamable region.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegionBounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl RegionBounds {
    pub fn new(min: Vec3, max: Vec3) -> Result<Self, &'static str> {
        if !min.is_finite() || !max.is_finite() {
            return Err("region bounds must be finite");
        }
        if min.x > max.x || min.y > max.y || min.z > max.z {
            return Err("region bounds minimum must not exceed maximum");
        }
        Ok(Self { min, max })
    }

    /// Euclidean distance from a point to the AABB; points inside the bounds have distance zero.
    pub fn distance_to_point(self, point: Vec3) -> f32 {
        if !point.is_finite() {
            return f32::INFINITY;
        }
        let nearest = point.clamp(self.min, self.max);
        point.distance(nearest)
    }

    pub fn intersects(self, other: Self) -> bool {
        self.min.cmple(other.max).all() && self.max.cmpge(other.min).all()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Cell(i32, i32, i32);

/// Spatial hash used by the streaming coordinator to avoid testing every authored region against
/// every observer. Large regions that cover too many cells are kept in a small overflow set.
#[derive(Clone, Debug)]
pub struct WorldPartitionIndex {
    cell_size: f32,
    cells: HashMap<Cell, Vec<RegionId>>,
    region_cells: HashMap<RegionId, Vec<Cell>>,
    overflow: HashSet<RegionId>,
    all_regions: HashSet<RegionId>,
    max_cells_per_region: u64,
}

impl Default for WorldPartitionIndex {
    fn default() -> Self {
        Self::new(256.0)
    }
}

impl WorldPartitionIndex {
    pub fn new(cell_size: f32) -> Self {
        Self {
            cell_size: if cell_size.is_finite() && cell_size > 0.0 {
                cell_size
            } else {
                256.0
            },
            cells: HashMap::new(),
            region_cells: HashMap::new(),
            overflow: HashSet::new(),
            all_regions: HashSet::new(),
            max_cells_per_region: 4096,
        }
    }

    pub fn cell_size(&self) -> f32 {
        self.cell_size
    }

    pub fn insert(&mut self, id: RegionId, bounds: RegionBounds) -> Result<(), &'static str> {
        if !bounds.min.is_finite()
            || !bounds.max.is_finite()
            || bounds.min.x > bounds.max.x
            || bounds.min.y > bounds.max.y
            || bounds.min.z > bounds.max.z
        {
            return Err("partition bounds must be finite and ordered minimum-to-maximum");
        }
        self.remove(&id);
        self.all_regions.insert(id.clone());
        let (min, max) = self.cell_range(bounds);
        let count = cell_count(min, max);
        if count > self.max_cells_per_region {
            self.overflow.insert(id);
            return Ok(());
        }
        let mut occupied = Vec::with_capacity(count as usize);
        for x in min.0..=max.0 {
            for y in min.1..=max.1 {
                for z in min.2..=max.2 {
                    let cell = Cell(x, y, z);
                    self.cells.entry(cell).or_default().push(id.clone());
                    occupied.push(cell);
                }
            }
        }
        self.region_cells.insert(id, occupied);
        Ok(())
    }

    pub fn remove(&mut self, id: &RegionId) -> bool {
        let existed = self.all_regions.remove(id);
        self.overflow.remove(id);
        if let Some(cells) = self.region_cells.remove(id) {
            for cell in cells {
                if let Some(regions) = self.cells.get_mut(&cell) {
                    regions.retain(|candidate| candidate != id);
                    if regions.is_empty() {
                        self.cells.remove(&cell);
                    }
                }
            }
        }
        existed
    }

    /// Return region IDs whose indexed cells intersect the query AABB. This is a broad phase;
    /// callers still perform exact AABB distance checks before making lifecycle decisions.
    pub fn query(&self, bounds: RegionBounds) -> HashSet<RegionId> {
        let (min, max) = self.cell_range(bounds);
        if cell_count(min, max) > 50_000 {
            return self.all_regions.clone();
        }
        let mut result = self.overflow.clone();
        for x in min.0..=max.0 {
            for y in min.1..=max.1 {
                for z in min.2..=max.2 {
                    if let Some(regions) = self.cells.get(&Cell(x, y, z)) {
                        result.extend(regions.iter().cloned());
                    }
                }
            }
        }
        result
    }

    pub fn len(&self) -> usize {
        self.all_regions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.all_regions.is_empty()
    }

    fn cell_range(&self, bounds: RegionBounds) -> (Cell, Cell) {
        let cell = |value: f32| {
            let scaled = (value / self.cell_size).floor();
            scaled.clamp(i32::MIN as f32, i32::MAX as f32) as i32
        };
        (
            Cell(cell(bounds.min.x), cell(bounds.min.y), cell(bounds.min.z)),
            Cell(cell(bounds.max.x), cell(bounds.max.y), cell(bounds.max.z)),
        )
    }
}

fn cell_count(min: Cell, max: Cell) -> u64 {
    let axis = |a: i32, b: i32| (i64::from(b) - i64::from(a) + 1).max(0) as u64;
    axis(min.0, max.0)
        .saturating_mul(axis(min.1, max.1))
        .saturating_mul(axis(min.2, max.2))
}

#[cfg(test)]
mod tests {
    use super::{RegionBounds, WorldPartitionIndex};
    use crate::RegionId;
    use glam::Vec3;

    #[test]
    fn aabb_distance_is_zero_inside_and_euclidean_outside() {
        let bounds = RegionBounds::new(Vec3::ZERO, Vec3::splat(10.0)).unwrap();
        assert_eq!(bounds.distance_to_point(Vec3::splat(5.0)), 0.0);
        assert!((bounds.distance_to_point(Vec3::new(13.0, 4.0, 4.0)) - 3.0).abs() < 1.0e-5);
    }

    #[test]
    fn spatial_hash_returns_intersecting_regions_and_removes_stale_entries() {
        let mut index = WorldPartitionIndex::new(10.0);
        let near = RegionId::new("near");
        let far = RegionId::new("far");
        index
            .insert(
                near.clone(),
                RegionBounds::new(Vec3::ZERO, Vec3::splat(4.0)).unwrap(),
            )
            .unwrap();
        index
            .insert(
                far.clone(),
                RegionBounds::new(Vec3::splat(100.0), Vec3::splat(104.0)).unwrap(),
            )
            .unwrap();
        let query = RegionBounds::new(Vec3::splat(-2.0), Vec3::splat(6.0)).unwrap();
        let found = index.query(query);
        assert!(found.contains(&near));
        assert!(!found.contains(&far));
        assert!(index.remove(&near));
        assert!(!index.query(query).contains(&near));
    }

    #[test]
    fn index_rejects_non_finite_or_reversed_bounds() {
        let mut index = WorldPartitionIndex::default();
        let invalid = RegionBounds {
            min: Vec3::ONE,
            max: Vec3::ZERO,
        };
        assert!(index.insert(RegionId::new("invalid"), invalid).is_err());
        assert!(index.is_empty());
    }

    #[test]
    fn very_large_regions_use_the_overflow_list_without_cell_explosion() {
        let mut index = WorldPartitionIndex::new(1.0);
        let huge = RegionId::new("huge");
        index
            .insert(
                huge.clone(),
                RegionBounds::new(Vec3::splat(-10_000.0), Vec3::splat(10_000.0)).unwrap(),
            )
            .unwrap();
        assert!(index
            .query(RegionBounds::new(Vec3::ZERO, Vec3::ONE).unwrap())
            .contains(&huge));
    }
}
