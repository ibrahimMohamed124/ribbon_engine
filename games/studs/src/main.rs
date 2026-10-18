use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    studs::app::build_app(None, false)?.run();
    Ok(())
}
