//! Small bounded linear color and scalar curves.
use crate::ParticleError;
#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub stops: Vec<(f32, [f32; 4])>,
}
impl Gradient {
    pub fn constant(color: [f32; 4]) -> Self {
        Self {
            stops: vec![(0.0, color), (1.0, color)],
        }
    }
    pub fn validate(&mut self) -> Result<(), ParticleError> {
        if self.stops.len() > 8 {
            return Err(ParticleError::TooManyStops);
        }
        if self
            .stops
            .iter()
            .any(|(t, c)| !t.is_finite() || c.iter().any(|v| !v.is_finite()))
        {
            return Err(ParticleError::NonFinite);
        }
        self.stops.sort_by(|a, b| a.0.total_cmp(&b.0));
        Ok(())
    }
    pub fn eval(&self, t: f32) -> [f32; 4] {
        let t = finite(t).clamp(0.0, 1.0);
        if self.stops.is_empty() {
            return [1.0; 4];
        }
        lerp_stops(&self.stops, t)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    pub points: Vec<(f32, f32)>,
}
impl Curve {
    pub fn constant(v: f32) -> Self {
        Self {
            points: vec![(0.0, v), (1.0, v)],
        }
    }
    pub fn validate(&mut self) -> Result<(), ParticleError> {
        if self.points.len() > 8 {
            return Err(ParticleError::TooManyStops);
        }
        if self
            .points
            .iter()
            .any(|(t, v)| !t.is_finite() || !v.is_finite())
        {
            return Err(ParticleError::NonFinite);
        }
        self.points.sort_by(|a, b| a.0.total_cmp(&b.0));
        Ok(())
    }
    pub fn eval(&self, t: f32) -> f32 {
        let t = finite(t).clamp(0.0, 1.0);
        if self.points.is_empty() {
            return 1.0;
        }
        let i = self.points.partition_point(|(x, _)| *x < t);
        if i == 0 {
            return self.points[0].1;
        }
        if i == self.points.len() {
            return self.points[i - 1].1;
        }
        let (a, av) = self.points[i - 1];
        let (b, bv) = self.points[i];
        let f = if b > a { (t - a) / (b - a) } else { 1.0 };
        av + (bv - av) * f
    }
}
fn lerp_stops(s: &[(f32, [f32; 4])], t: f32) -> [f32; 4] {
    let i = s.partition_point(|(x, _)| *x < t);
    let (a, av, b, bv) = if i == 0 {
        (s[0].0, s[0].1, s[0].0, s[0].1)
    } else if i == s.len() {
        (s[i - 1].0, s[i - 1].1, s[i - 1].0, s[i - 1].1)
    } else {
        (s[i - 1].0, s[i - 1].1, s[i].0, s[i].1)
    };
    let f = if b > a { (t - a) / (b - a) } else { 1.0 };
    std::array::from_fn(|j| av[j] + (bv[j] - av[j]) * f)
}
fn finite(v: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}
