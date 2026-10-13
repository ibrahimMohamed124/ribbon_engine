use std::error::Error;

use raycast_blocks::app::StudsApp;

fn main() -> Result<(), Box<dyn Error>> {
    let app = StudsApp::new()?;
    let window_config = app.window_config();
    blocks_engine_desktop::run(app, window_config)
}
