//! Deterministic, dependency-free RNG (SplitMix64). Integer-only so that generated
//! vectors are bit-identical on every platform and compiler version.

/// SplitMix64 generator.
#[derive(Clone, Debug)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Independent child stream (for per-case generators).
    pub fn fork(&mut self, salt: u64) -> Self {
        Self::new(self.next_u64() ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15))
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (n > 0), multiply-shift reduction.
    pub fn below(&mut self, n: u64) -> u64 {
        assert!(n > 0);
        ((u128::from(self.next_u64()) * u128::from(n)) >> 64) as u64
    }

    /// Uniform in `lo..=hi`.
    pub fn range_i64(&mut self, lo: i64, hi: i64) -> i64 {
        assert!(lo <= hi);
        let span = (hi - lo) as u64 + 1;
        lo + self.below(span) as i64
    }

    /// Uniform in `lo..=hi`.
    pub fn range_usize(&mut self, lo: usize, hi: usize) -> usize {
        self.range_i64(lo as i64, hi as i64) as usize
    }

    /// True with probability `num/den`.
    pub fn chance(&mut self, num: u64, den: u64) -> bool {
        self.below(den) < num
    }

    pub fn i8(&mut self) -> i8 {
        self.next_u64() as i8
    }

    pub fn i32(&mut self) -> i32 {
        self.next_u64() as i32
    }

    pub fn u16_in(&mut self, lo: u16, hi: u16) -> u16 {
        self.range_i64(i64::from(lo), i64::from(hi)) as u16
    }

    pub fn u8_in(&mut self, lo: u8, hi: u8) -> u8 {
        self.range_i64(i64::from(lo), i64::from(hi)) as u8
    }

    /// Approximately normal integer with mean 0 and standard deviation ≈ `std`
    /// (Irwin–Hall sum of 12 uniforms; integer arithmetic only).
    pub fn normalish(&mut self, std: i64) -> i64 {
        const ONE: i64 = 1 << 16;
        let mut s: i64 = 0;
        for _ in 0..12 {
            s += self.below(ONE as u64) as i64;
        }
        let z = s - 6 * ONE; // ~N(0,1) scaled by ONE
        (z * std).div_euclid(ONE)
    }

    pub fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len() as u64) as usize]
    }
}
