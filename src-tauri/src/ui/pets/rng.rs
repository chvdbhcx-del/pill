//! Small seedable generator (xorshift64*). The web version used Math.random(); here every random choice goes through this so a replay with the
//! same seed and the same calls gives the same pet (tests, screenshots).

#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// [0, 1)
    pub fn f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_in_range() {
        let (mut a, mut b) = (Rng::new(7), Rng::new(7));
        for _ in 0..1000 {
            let (x, y) = (a.f32(), b.f32());
            assert_eq!(x, y);
            assert!((0.0..1.0).contains(&x));
        }
        assert_ne!(Rng::new(1).next_u64(), Rng::new(2).next_u64());
    }

    #[test]
    fn roughly_uniform() {
        let mut r = Rng::new(42);
        let mean = (0..10_000).map(|_| r.f32()).sum::<f32>() / 10_000.0;
        assert!((mean - 0.5).abs() < 0.02, "{mean}");
    }
}
