//! Errors returned by save operations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveError {
    Io(String),
    Ron(String),
    Corrupt(String),
    Crc,
    WrongGame {
        expected: String,
        found: String,
    },
    UnsupportedEnvelope(u32),
    UnsupportedSectionVersion {
        key: String,
        found: u32,
        expected: u32,
    },
    InvalidSlot(String),
    NoSuchSlot(String),
    MissingSection(String),
    Apply(String),
    NotConfigured,
}
impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SaveError {}
impl From<std::io::Error> for SaveError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}
