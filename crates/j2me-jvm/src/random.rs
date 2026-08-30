//! `java.util.Random`, exactly.
//!
//! The algorithm is fully specified in the javadoc, so a correct implementation
//! reproduces a real JVM's sequence bit for bit. That matters for strict
//! transliteration: whenever a game drives combat rolls, enemy selection, or any
//! other decision through `java.util.Random`, a differential trace against the
//! JVM is worthless unless the two agree about the random stream down to the bit.
//!
//! This is the game-neutral 48-bit LCG shared by every port, so no game body
//! keeps its own duplicate copy. The method surface is the full JDK superset
//! (`next`, `next_int`, `next_int_bound`, `next_long`, `next_boolean`,
//! `next_float`, `next_double`); a game that only uses a few of them draws on a
//! subset of the same, bit-identical implementation.

const MULTIPLIER: i64 = 0x5_DEEC_E66D;
const ADDEND: i64 = 0xB;
const MASK: i64 = (1 << 48) - 1;

/// A 48-bit linear congruential generator matching `java.util.Random`.
#[derive(Debug, Clone)]
pub struct Random {
    seed: i64,
}

impl Random {
    /// Current 48-bit LCG state, for differential-trace diagnostics.
    pub fn state(&self) -> i64 {
        self.seed
    }

    /// `new Random(seed)`.
    pub fn new(seed: i64) -> Self {
        Self {
            seed: (seed ^ MULTIPLIER) & MASK,
        }
    }

    /// `setSeed(seed)`.
    pub fn set_seed(&mut self, seed: i64) {
        self.seed = (seed ^ MULTIPLIER) & MASK;
    }

    /// The protected `next(bits)` primitive every other method is built on.
    pub fn next(&mut self, bits: u32) -> i32 {
        self.seed = self.seed.wrapping_mul(MULTIPLIER).wrapping_add(ADDEND) & MASK;
        // Java: (int)(seed >>> (48 - bits)) -- an unsigned shift of the 48-bit
        // state, which is always non-negative, then a narrowing cast.
        ((self.seed as u64) >> (48 - bits)) as i32
    }

    /// `nextInt()`.
    pub fn next_int(&mut self) -> i32 {
        self.next(32)
    }

    /// `nextInt(bound)`, including the rejection loop that keeps the
    /// distribution uniform. Panics if `bound` is not positive, as Java throws.
    pub fn next_int_bound(&mut self, bound: i32) -> i32 {
        assert!(bound > 0, "bound must be positive");

        // Power of two: Java takes the high bits directly.
        if (bound & -bound) == bound {
            return (((bound as i64).wrapping_mul(self.next(31) as i64)) >> 31) as i32;
        }

        loop {
            let bits = self.next(31);
            let value = bits % bound;
            // Reject when the modulo would bias the result.
            if bits.wrapping_sub(value).wrapping_add(bound - 1) >= 0 {
                return value;
            }
        }
    }

    /// `nextLong()`.
    pub fn next_long(&mut self) -> i64 {
        ((self.next(32) as i64) << 32).wrapping_add(self.next(32) as i64)
    }

    /// `nextBoolean()`.
    pub fn next_boolean(&mut self) -> bool {
        self.next(1) != 0
    }

    /// `nextFloat()`.
    pub fn next_float(&mut self) -> f32 {
        self.next(24) as f32 / (1 << 24) as f32
    }

    /// `nextDouble()`.
    pub fn next_double(&mut self) -> f64 {
        let high = (self.next(26) as i64) << 27;
        let combined = high.wrapping_add(self.next(27) as i64);
        combined as f64 * (1.0f64 / (1i64 << 53) as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every expected value below was produced by a real JVM (OpenJDK 17), not
    // derived by hand. Regenerate with:
    //   var r = new java.util.Random(42);
    //   for (int i = 0; i < 5; i++) System.out.println(r.nextInt());
    #[test]
    fn matches_the_jvm_sequence_for_seed_42() {
        let mut random = Random::new(42);
        assert_eq!(
            [
                random.next_int(),
                random.next_int(),
                random.next_int(),
                random.next_int(),
                random.next_int(),
            ],
            [-1170105035, 234785527, -1360544799, 205897768, 1325939940]
        );
    }

    #[test]
    fn matches_the_jvm_sequence_for_seed_zero() {
        let mut random = Random::new(0);
        assert_eq!(
            [random.next_int(), random.next_int(), random.next_int()],
            [-1155484576, -723955400, 1033096058]
        );
    }

    #[test]
    fn bounded_draws_match_the_jvm() {
        let mut random = Random::new(12345);
        let drawn: Vec<i32> = (0..8).map(|_| random.next_int_bound(100)).collect();
        assert_eq!(drawn, vec![51, 80, 41, 28, 55, 84, 75, 2]);
    }

    #[test]
    fn power_of_two_bounds_take_the_fast_path() {
        let mut random = Random::new(7);
        let drawn: Vec<i32> = (0..6).map(|_| random.next_int_bound(16)).collect();
        assert!(drawn.iter().all(|value| (0..16).contains(value)));
    }

    #[test]
    fn long_float_and_double_match_the_jvm() {
        let mut random = Random::new(99);
        assert_eq!(random.next_long(), -5119754439980850796);

        let mut random = Random::new(99);
        assert_eq!(random.next_float(), 0.7224575);

        let mut random = Random::new(99);
        assert_eq!(random.next_double(), 0.7224575488195071);
    }

    #[test]
    fn boolean_and_set_seed_reset_the_stream() {
        // `nextBoolean()` is just the top bit of the same stream, and
        // `setSeed` scrambles identically to the constructor, so a reset
        // reproduces the seed-42 booleans exactly.
        let expected: Vec<bool> = {
            let mut random = Random::new(42);
            (0..4).map(|_| random.next_boolean()).collect()
        };

        let mut random = Random::new(0);
        random.set_seed(42);
        let drawn: Vec<bool> = (0..4).map(|_| random.next_boolean()).collect();
        assert_eq!(drawn, expected);
    }
}
