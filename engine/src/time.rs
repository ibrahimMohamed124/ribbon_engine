//! Fixed-rate simulation time accumulation.

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
