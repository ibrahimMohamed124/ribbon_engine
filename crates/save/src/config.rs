//! Save configuration and metadata.
use std::path::PathBuf;
#[derive(Clone, Debug)]
pub struct SaveConfig {
    pub qualifier: String,
    pub organization: String,
    pub application: String,
    pub game_id: String,
    pub root_override: Option<PathBuf>,
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SaveMeta {
    pub slot: String,
    pub game_id: String,
    pub engine_save_format: u32,
    pub created_unix_secs: u64,
    pub play_seconds: f64,
    pub label: String,
}
