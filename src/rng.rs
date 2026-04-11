/// Mulberry32 — fast, seedable PRNG. No heap, no imports.
pub struct Rng(u32);

impl Rng {
    #[inline]
    pub fn new(seed: u32) -> Self {
        Self(seed)
    }

    /// Returns a pseudo-random u32.
    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_add(0x6D2B79F5);
        let mut z = self.0;
        z = (z ^ (z >> 15)).wrapping_mul(z | 1);
        z ^= z.wrapping_add((z ^ (z >> 7)).wrapping_mul(z | 61));
        z ^ (z >> 14)
    }

    /// Returns a value in [0, n).
    #[inline]
    pub fn next_usize(&mut self, n: usize) -> usize {
        (self.next_u32() as usize) % n
    }
}
