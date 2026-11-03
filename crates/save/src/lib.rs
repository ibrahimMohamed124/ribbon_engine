//! Versioned, section-based save files with CRC validation and atomic replacement.
mod config;
mod envelope;
mod error;
mod plugin;
mod section;
mod store;
#[cfg(test)]
mod tests;

pub use config::{SaveConfig, SaveMeta};
pub use error::SaveError;
pub use plugin::{PlayTime, SavePlugin};
pub use section::{AppSaveExt, SaveSection};
pub use store::{delete_slot, list_slots, load_from_slot, save_to_slot, slot_exists};

#[derive(Clone, Debug)]
pub struct SaveRequest {
    pub slot: String,
    pub label: String,
}
#[derive(Clone, Debug)]
pub struct LoadRequest {
    pub slot: String,
}
#[derive(Clone, Debug)]
pub struct SaveFinished {
    pub slot: String,
    pub result: Result<SaveMeta, SaveError>,
}
#[derive(Clone, Debug)]
pub struct LoadFinished {
    pub slot: String,
    pub result: Result<SaveMeta, SaveError>,
}
