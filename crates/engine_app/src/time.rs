//! Fixed-rate simulation time accumulation.

use std::collections::VecDeque;
use std::time::Duration;

/// Per-frame virtual or wall-clock time exposed to systems.
#[derive(Clone, Copy, Debug, Default)]
pub struct Time {
    /// Duration of the most recent frame (or fixed step while FixedUpdate runs).
    pub delta: Duration,
    /// Time accumulated since startup.
    pub elapsed: Duration,
    /// Number of frames advanced.
    pub frame: u64,
}

/// FixedUpdate accumulator with the engine's eight-step frame cap.
#[derive(Clone, Copy, Debug)]
pub struct FixedTime {
    /// Fixed simulation step.
    pub step: Duration,
    accumulator: Duration,
    /// Whether the most recent call discarded excess time.
    pub dropped_excess: bool,
}

impl Default for FixedTime {
    fn default() -> Self {
        Self {
            step: Duration::from_secs_f64(1.0 / 60.0),
            accumulator: Duration::ZERO,
            dropped_excess: false,
        }
    }
}

impl FixedTime {
    /// Add a frame delta, returning fixed steps due and dropping excess above eight.
    pub fn advance(&mut self, delta: Duration) -> u32 {
        self.dropped_excess = false;
        if self.step.is_zero() {
            self.accumulator = Duration::ZERO;
            return 0;
        }
        self.accumulator = self.accumulator.saturating_add(delta);
        let due = self.accumulator.as_nanos() / self.step.as_nanos();
        if due > 8 {
            let remainder = self.accumulator.as_nanos() % self.step.as_nanos();
            self.accumulator = Duration::new(
                (remainder / 1_000_000_000) as u64,
                (remainder % 1_000_000_000) as u32,
            );
            self.dropped_excess = true;
            return 8;
        }
        let due = due as u32;
        self.accumulator = self
            .accumulator
            .saturating_sub(self.step.saturating_mul(due));
        due
    }
}

/// Severity for a buffered engine log message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    /// Diagnostic message.
    Info,
    /// Recoverable issue or degraded behavior.
    Warn,
    /// Operation failed.
    Error,
}

/// A bounded ring buffer of recent messages, capped at 512 entries.
#[derive(Default)]
pub struct LogBuffer {
    entries: VecDeque<(LogLevel, String)>,
}

impl LogBuffer {
    /// Append a message, evicting the oldest when the capacity is full.
    pub fn push(&mut self, level: LogLevel, message: impl Into<String>) {
        if self.entries.len() == 512 {
            self.entries.pop_front();
        }
        self.entries.push_back((level, message.into()));
    }

    /// Iterate through buffered messages from oldest to newest.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &(LogLevel, String)> {
        self.entries.iter()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FixedStepper {
    step: f32,
    max_steps: u32,
    accumulator: f32,
}

impl FixedStepper {
    pub fn new(hz: u32, max_steps: u32) -> Self {
        Self {
            step: 1.0 / hz.max(1) as f32,
            max_steps: max_steps.max(1),
            accumulator: 0.0,
        }
    }

    /// Accumulate real time and return the fixed simulation steps due.
    /// Excess accumulated time is discarded when the step limit is exceeded.
    pub fn advance(&mut self, real_dt: f32) -> u32 {
        if real_dt.is_finite() && real_dt > 0.0 {
            self.accumulator += real_dt;
        }
        let epsilon = self.step * 1e-6;
        let due = ((self.accumulator + epsilon) / self.step).floor() as u32;
        if due > self.max_steps {
            self.accumulator = 0.0;
            return self.max_steps;
        }
        self.accumulator = (self.accumulator - due as f32 * self.step).max(0.0);
        if self.accumulator >= self.step {
            // Avoid an alpha of 1.0 from rounding after subtracting exact multiples.
            self.accumulator %= self.step;
        }
        due
    }

    pub fn step(&self) -> f32 {
        self.step
    }

    pub fn alpha(&self) -> f32 {
        (self.accumulator / self.step).clamp(0.0, 1.0 - f32::EPSILON)
    }
}

#[cfg(test)]
mod tests {
    use super::FixedStepper;

    #[test]
    fn exact_multiples_and_remainders_are_counted() {
        let mut stepper = FixedStepper::new(10, 10);
        assert_eq!(stepper.advance(0.3), 3);
        assert_eq!(stepper.alpha(), 0.0);
        assert_eq!(stepper.advance(0.15), 1);
        assert!((stepper.alpha() - 0.5).abs() < 1e-5);
        assert_eq!(stepper.advance(0.05), 1);
        assert!(stepper.alpha() < 1.0);
    }

    #[test]
    fn max_steps_drop_excess_remainder() {
        let mut stepper = FixedStepper::new(10, 3);
        assert_eq!(stepper.advance(1.0), 3);
        assert_eq!(stepper.alpha(), 0.0);
        assert_eq!(stepper.advance(0.05), 0);
        assert!((stepper.alpha() - 0.5).abs() < 1e-5);
    }

    #[test]
    fn invalid_time_counts_as_zero() {
        let mut stepper = FixedStepper::new(10, 10);
        assert_eq!(stepper.advance(f32::NAN), 0);
        assert_eq!(stepper.advance(f32::INFINITY), 0);
        assert_eq!(stepper.advance(-1.0), 0);
        assert_eq!(stepper.alpha(), 0.0);
    }

    #[test]
    fn alpha_stays_in_unit_interval() {
        let mut stepper = FixedStepper::new(120, 10);
        for _ in 0..1000 {
            stepper.advance(1.0 / 144.0);
            assert!((0.0..1.0).contains(&stepper.alpha()));
        }
    }
}
