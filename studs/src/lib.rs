//! STUDS - a monochrome, Roblox-flavoured horror game rendered by a software raycaster.
//!
//! Module map (dependencies flow downwards only):
//!
//! ```text
//! main.rs      window + frame loop (the only place that touches the OS)
//!   game.rs    game state machine, orchestrates everything below
//!     input.rs      keyboard / mouse -> Controls, key bindings
//!     player.rs     movement + physics
//!     enemy.rs      AI (pathfinding, "weeping angel" speed rule)
//!     world.rs      maze generation, voxel storage, voxel ray queries
//!     renderer/     raycaster, lighting, post-processing, entities, HUD
//!     audio.rs      backend-agnostic sound events
//!     ui/           menus, controls, settings, and pixel UI theme
//!     settings.rs   persisted user settings
//!   math.rs    Vect / Angles / Rng / hash (no dependencies)
//! ```

pub mod app;
pub mod audio;
pub mod enemy;
pub mod entities;
pub mod game;
pub mod input;
pub mod level;
pub mod math;
pub mod player;
pub mod renderer;
pub mod settings;
pub mod surfaces;
pub mod ui;
pub mod world;
