//! Small math / noise / RNG helpers shared by every subsystem.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vect {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vect {
    pub fn new(x: f32, y: f32, z: f32) -> Self {
        Vect { x, y, z }
    }
    pub fn add(self, o: Vect) -> Vect {
        Vect::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
    pub fn scale(self, s: f32) -> Vect {
        Vect::new(self.x * s, self.y * s, self.z * s)
    }
    pub fn normalize(self) -> Vect {
        let len = (self.x * self.x + self.y * self.y + self.z * self.z).sqrt();
        if len == 0.0 { self } else { self.scale(1.0 / len) }
    }
    pub fn dot(self, o: Vect) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    pub fn axis(self, i: usize) -> f32 {
        match i {
            0 => self.x,
            1 => self.y,
            _ => self.z,
        }
    }
    /// Distance on the floor plane (ignores z).
    pub fn dist_xy(self, o: Vect) -> f32 {
        let dx = self.x - o.x;
        let dy = self.y - o.y;
        (dx * dx + dy * dy).sqrt()
    }
}

/// psi = pitch (up/down), phi = yaw (left/right)
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Angles {
    pub psi: f32,
    pub phi: f32,
}

pub fn forward_vec(a: Angles) -> Vect {
    Vect::new(a.psi.cos() * a.phi.cos(), a.psi.cos() * a.phi.sin(), a.psi.sin())
}

/// Deterministic noise in [-1, 1].
pub fn hash4(a: i32, b: i32, c: i32, d: i32) -> f32 {
    let mut h = (a as u32).wrapping_mul(73_856_093)
        ^ (b as u32).wrapping_mul(19_349_663)
        ^ (c as u32).wrapping_mul(83_492_791)
        ^ (d as u32).wrapping_mul(2_654_435_761);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0xffff) as f32 / 32767.5 - 1.0
}

pub fn pack_gray(g: f32) -> u32 {
    let v = (g.clamp(0.0, 1.0) * 255.0) as u32;
    (v << 16) | (v << 8) | v
}

/// xorshift32 - tiny, deterministic, good enough for level generation.
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Rng(seed | 1)
    }
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u32() as usize) % n
    }
    pub fn chance(&mut self, p: f32) -> bool {
        ((self.next_u32() >> 8) as f32 / 16_777_216.0) < p
    }
}
