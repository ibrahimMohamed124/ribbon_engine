//! Small deterministic PCG32 random number generator.
use glam::Vec3;
/// PCG-XSH-RR 32-bit generator.
pub struct Pcg32 {
    state: u64,
    inc: u64,
}
impl Pcg32 {
    pub fn new(seed: u64, stream: u64) -> Self {
        let mut s = Self {
            state: 0,
            inc: (stream << 1) | 1,
        };
        s.next_u32();
        s.state = s.state.wrapping_add(seed);
        s.next_u32();
        s
    }
    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(6364136223846793005).wrapping_add(self.inc);
        let x = (((old >> 18) ^ old) >> 27) as u32;
        let r = (old >> 59) as u32;
        x.rotate_right(r)
    }
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (1.0 / 16_777_216.0)
    }
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        let (a, b) = if lo <= hi { (lo, hi) } else { (hi, lo) };
        a + (b - a) * self.next_f32()
    }
    pub fn unit_vec3(&mut self) -> Vec3 {
        let z = self.range(-1.0, 1.0);
        let phi = self.range(0.0, std::f32::consts::TAU);
        let r = (1.0 - z * z).max(0.0).sqrt();
        Vec3::new(r * phi.cos(), z, r * phi.sin())
    }
    pub fn in_unit_sphere(&mut self) -> Vec3 {
        self.unit_vec3() * self.next_f32().cbrt()
    }
}
