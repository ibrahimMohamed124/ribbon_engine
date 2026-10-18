//! In-game scene preview using the ECS application and rendering plugin.
//!
//! The former renderer-mode switch belonged to the legacy immediate-mode runner. This preview
//! now exercises the same plugin-based CPU rendering path as the migrated game application.

use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    studs::app::build_app(Some(42), true)?.run();
    Ok(())
}
