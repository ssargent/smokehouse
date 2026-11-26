//! A tiny deterministic pseudo-random source (xorshift64), so the crate needs
//! no `rand` dependency.

/// xorshift64 generator.
#[derive(Debug)]
pub struct Rng(u64);

impl Rng {
    /// Seed the generator (the seed is forced non-zero).
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    /// Next 32-bit value.
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x >> 32) as u32
    }

    /// Uniform value in `[-amplitude, amplitude]`.
    pub fn uniform(&mut self, amplitude: f32) -> f32 {
        let unit = self.next_u32() as f32 / u32::MAX as f32;
        (unit * 2.0 - 1.0) * amplitude
    }
}

/// Four random-ish hex digits, e.g. for a first-run ntfy topic.
#[must_use]
pub fn random_hex4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::from(d.subsec_nanos()))
        .unwrap_or(0);
    let mut rng = Rng::new(nanos ^ (u64::from(std::process::id()) << 32) ^ 0x9E37_79B9_7F4A_7C15);
    format!("{:04x}", (rng.next_u32() & 0xFFFF) as u16)
}
