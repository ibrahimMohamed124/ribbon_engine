//! Platform-independent game and utility primitives.

pub mod audio;
pub use engine_app::time;
pub use engine_platform::input;

// Temporary compatibility shims. Remove in phase 9.
pub use engine_math as math;
pub use plugin_physics as physics;
pub use plugin_voxel::voxel;
