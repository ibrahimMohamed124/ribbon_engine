//! Bounded diagnostic snapshots for the audio runtime.

/// Point-in-time counters collected on the backend owner thread.
///
/// `embedded_encoded_bytes` measures the source data retained by the sound bank, not the decoded
/// PCM cache. Kira does not expose portable callback CPU/underrun counters through its public API.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioDiagnostics {
    pub active_voices: usize,
    pub retiring_voices: usize,
    pub max_voices: usize,
    pub registered_sounds: usize,
    pub registered_ambiences: usize,
    pub embedded_encoded_bytes: usize,
    pub active_streaming_voices: usize,
    pub active_stream_source_bytes: usize,
    pub active_ambience_layers: usize,
    /// Number of embedded clips that failed decoding or exceeded static-cache budgets.
    pub decode_failures: usize,
    /// Most recent decode/cache-admission diagnostic, bounded to 512 characters.
    pub last_decode_error: Option<String>,
}
