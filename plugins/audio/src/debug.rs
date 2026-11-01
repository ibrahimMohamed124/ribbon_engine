//! Bounded diagnostic snapshots for the audio runtime.

/// Point-in-time counters collected on the backend owner thread.
///
/// `embedded_encoded_bytes` measures encoded source data retained by the sound bank, while
/// `decoded_static_pcm_bytes` measures the unique decoded frame storage retained by the Kira
/// static cache. Kira does not expose portable callback CPU/underrun counters through its public API.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioDiagnostics {
    pub active_voices: usize,
    pub retiring_voices: usize,
    pub max_voices: usize,
    pub registered_sounds: usize,
    pub registered_ambiences: usize,
    pub embedded_encoded_bytes: usize,
    /// Actual PCM frame storage retained by unique predecoded static clips.
    pub decoded_static_pcm_bytes: usize,
    /// Highest conservative startup decode peak estimate (retained PCM + encoded cursor + new PCM).
    pub peak_static_decode_bytes: usize,
    pub active_streaming_voices: usize,
    pub active_stream_source_bytes: usize,
    pub active_ambience_layers: usize,
    /// Live user-defined mixer tracks.
    pub custom_mixer_buses: usize,
    /// Live shared send-effect tracks.
    pub mixer_send_tracks: usize,
    /// Number of device-loss recovery attempts made by the audio worker.
    pub device_recovery_attempts: u64,
    /// Number of successful output-device recreations.
    pub device_recoveries: u64,
    /// Last output-device error or recovery failure.
    pub last_device_error: Option<String>,
    /// Number of embedded clips that failed decoding or exceeded static-cache budgets.
    pub decode_failures: usize,
    /// Most recent decode/cache-admission diagnostic, bounded to 512 characters.
    pub last_decode_error: Option<String>,
}
