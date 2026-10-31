//! Renderer-agnostic profiling primitives shared by engine subsystems.
use std::collections::BTreeMap;

/// A monotonically accumulated named counter.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counters(BTreeMap<String, u64>);
impl Counters {
    pub fn increment(&mut self, name: impl Into<String>, amount: u64) {
        let entry = self.0.entry(name.into()).or_default();
        *entry = entry.saturating_add(amount);
    }
    pub fn get(&self, name: &str) -> u64 {
        self.0.get(name).copied().unwrap_or(0)
    }
    pub fn iter(&self) -> impl Iterator<Item = (&str, u64)> {
        self.0.iter().map(|(k, v)| (k.as_str(), *v))
    }
}

/// Bounded timing samples for renderer-independent frame statistics.
#[derive(Clone, Debug)]
pub struct TimingSamples {
    capacity: usize,
    values: Vec<u64>,
}
impl TimingSamples {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            values: Vec::new(),
        }
    }
    pub fn push(&mut self, value_ns: u64) {
        if self.values.len() == self.capacity {
            self.values.remove(0);
        }
        self.values.push(value_ns);
    }
    pub fn values(&self) -> &[u64] {
        &self.values
    }
    pub fn percentile(&self, percentile: f64) -> Option<u64> {
        if self.values.is_empty() {
            return None;
        }
        let mut sorted = self.values.clone();
        sorted.sort_unstable();
        let p = percentile.clamp(0.0, 100.0) / 100.0;
        let index = ((sorted.len() - 1) as f64 * p).ceil() as usize;
        sorted.get(index).copied()
    }
}

/// Stable key/value writer for profile metadata; renderer-specific formats remain owned by their callers.
pub fn write_key_value_line<'a>(pairs: impl IntoIterator<Item = (&'a str, String)>) -> String {
    pairs
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Format the established one-line capture completion message without tying profiling to a renderer.
/// Callers pass the already-rendered path so platform-specific path formatting remains unchanged.
pub fn format_capture_written_line(sample_count: usize, output_path: &str) -> String {
    format!("[RENDER PROFILE] wrote {sample_count} frame samples to {output_path}")
}

#[cfg(test)]
mod tests {
    use super::{format_capture_written_line, write_key_value_line, TimingSamples};
    #[test]
    fn percentile_uses_nearest_rank_ceiling() {
        let mut samples = TimingSamples::new(8);
        samples.push(10);
        samples.push(20);
        samples.push(30);
        assert_eq!(samples.percentile(50.0), Some(20));
        assert_eq!(samples.percentile(95.0), Some(30));
    }
    #[test]
    fn key_value_line_format_is_stable() {
        assert_eq!(
            write_key_value_line([("frame", "7".to_owned()), ("cpu_ns", "120".to_owned())]),
            "frame=7 cpu_ns=120"
        );
    }

    #[test]
    fn render_capture_completion_log_format_is_stable() {
        assert_eq!(
            format_capture_written_line(7, "target/profiles/athar.csv"),
            "[RENDER PROFILE] wrote 7 frame samples to target/profiles/athar.csv"
        );
    }
}

/// Renderer-agnostic distribution summary for nanosecond samples. Percentiles use nearest-rank selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DurationSummary {
    pub sample_count: usize,
    pub mean_ns: u64,
    pub p50_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub peak_ns: u64,
}
impl DurationSummary {
    pub fn from_values(values: impl IntoIterator<Item = u64>) -> Self {
        let mut values: Vec<u64> = values.into_iter().collect();
        if values.is_empty() {
            return Self::default();
        }
        values.sort_unstable();
        let sum = values
            .iter()
            .fold(0_u128, |acc, value| acc.saturating_add(u128::from(*value)));
        let percentile = |percentage: usize| {
            let rank = (percentage * values.len()).div_ceil(100).max(1);
            values[rank.saturating_sub(1).min(values.len() - 1)]
        };
        Self {
            sample_count: values.len(),
            mean_ns: (sum / values.len() as u128).min(u128::from(u64::MAX)) as u64,
            p50_ns: percentile(50),
            p95_ns: percentile(95),
            p99_ns: percentile(99),
            peak_ns: *values.last().unwrap_or(&0),
        }
    }
}

/// Generic capture settings shared by non-rendering and rendering profilers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureConfig {
    pub enabled: bool,
    pub output_path: String,
    pub warmup_frames: u64,
    pub sample_capacity: usize,
}
impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            output_path: "target/profiles/capture.csv".into(),
            warmup_frames: 120,
            sample_capacity: 600,
        }
    }
}

impl CaptureConfig {
    /// Read generic capture settings without depending on a renderer or application crate.
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Ok(value) = std::env::var("RIBBON_PROFILE_ENABLED") {
            config.enabled = matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            );
        }
        if let Ok(value) = std::env::var("RIBBON_PROFILE_PATH") {
            if !value.trim().is_empty() {
                config.output_path = value;
            }
        }
        if let Ok(value) = std::env::var("RIBBON_PROFILE_WARMUP_FRAMES") {
            if let Ok(frames) = value.parse() {
                config.warmup_frames = frames;
            }
        }
        if let Ok(value) = std::env::var("RIBBON_PROFILE_CAPACITY") {
            if let Ok(capacity) = value.parse::<usize>() {
                config.sample_capacity = capacity;
            }
        }
        config
    }
}

/// Simple bounded capture lifecycle independent of a renderer backend.
#[derive(Clone, Debug)]
pub struct CaptureSession {
    config: CaptureConfig,
    seen_frames: u64,
    retained_samples: usize,
}
impl CaptureSession {
    pub fn new(config: CaptureConfig) -> Self {
        Self {
            config,
            seen_frames: 0,
            retained_samples: 0,
        }
    }
    pub fn config(&self) -> &CaptureConfig {
        &self.config
    }
    pub fn observe_frame(&mut self) -> bool {
        self.seen_frames = self.seen_frames.saturating_add(1);
        if !self.config.enabled
            || self.seen_frames <= self.config.warmup_frames
            || self.config.sample_capacity == 0
        {
            return false;
        }
        self.retained_samples = self
            .retained_samples
            .saturating_add(1)
            .min(self.config.sample_capacity);
        true
    }
    pub fn seen_frames(&self) -> u64 {
        self.seen_frames
    }
    pub fn retained_samples(&self) -> usize {
        self.retained_samples
    }
}

#[cfg(test)]
mod summary_tests {
    use super::{CaptureConfig, CaptureSession, DurationSummary};
    #[test]
    fn duration_summary_uses_nearest_rank_percentiles() {
        let summary = DurationSummary::from_values([10, 20, 30, 40]);
        assert_eq!(summary.sample_count, 4);
        assert_eq!(summary.mean_ns, 25);
        assert_eq!(summary.p50_ns, 20);
        assert_eq!(summary.p95_ns, 40);
        assert_eq!(summary.p99_ns, 40);
    }
    #[test]
    fn capture_session_respects_enablement_warmup_and_capacity() {
        let config = CaptureConfig {
            enabled: true,
            warmup_frames: 2,
            sample_capacity: 2,
            ..CaptureConfig::default()
        };
        let mut session = CaptureSession::new(config);
        assert!(!session.observe_frame());
        assert!(!session.observe_frame());
        assert!(session.observe_frame());
        assert!(session.observe_frame());
        assert!(session.observe_frame());
        assert_eq!(session.retained_samples(), 2);
    }
}
