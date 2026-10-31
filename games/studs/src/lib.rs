//! STUDS - a first-person horror game built on the Ribbon Engine.
//!
//! Module map (dependencies flow downwards only):
//!
//! ```text
//! main.rs      executable entry point; assembles and runs the ECS App
//!   app.rs       StudsPlugin and input/update/audio/render schedule systems
//!   game.rs      game state machine, orchestrates the domain logic below
//!     input.rs      keyboard / mouse -> Controls, key bindings
//!     player.rs     movement + physics
//!     enemy.rs      AI (pathfinding, "weeping angel" speed rule)
//!     world.rs      maze generation, voxel storage, voxel ray queries
//!     surfaces.rs   game materials, procedural albedo, and sprite loading
//!     audio.rs      backend-agnostic sound events
//!     ui/           menus, controls, settings, and pixel UI theme
//!     settings.rs   persisted user settings
//!   ribbon_math, studs_voxel   shared math and voxel storage
//! ```

pub mod app;
pub mod audio;
pub mod config;
pub mod enemy;
pub mod entities;
pub mod game;
pub mod game_ecs;
pub mod input;
pub mod level;
pub mod player;
pub mod settings;
pub mod styles;
pub mod surfaces;
pub mod ui;
pub mod world;
