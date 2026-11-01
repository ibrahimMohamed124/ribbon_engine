//! Compatibility re-exports for the game level and the engine voxel hit type.
pub use crate::level::{
    cell_center, origin, World, AIR, CEIL, CELLS, FLOOR, FLOOR_TOP, WALL, X_BLOCKS, Y_BLOCKS,
    Z_BLOCKS,
};
pub use studs_voxel::Hit;
