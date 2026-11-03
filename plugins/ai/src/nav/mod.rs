//! Single-layer grid navigation and physics baking.
pub mod agent;
pub use agent::{NavAgent, NavDestination, NavSettings, NavStatus, NavSteering};
mod bake;
mod grid;
mod path;
pub use bake::{bake_nav_grid, NavBakeConfig};
pub use grid::{NavCell, NavGrid, NavWorld};
pub use path::{NavError, NavPath};
