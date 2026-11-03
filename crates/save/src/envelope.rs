//! On-disk envelope format.
use crate::SaveMeta;
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Envelope {
    pub format: u32,
    pub meta: SaveMeta,
    pub sections: Vec<SectionRecord>,
}
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct SectionRecord {
    pub key: String,
    pub version: u32,
    pub data: String,
}
