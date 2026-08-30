//! `java.lang.Math` and the JVM's numeric conversions, exactly.
//!
//! Rust's arithmetic operators and `as` casts are *mostly* Java's, but the edges
//! differ, and those edges are where a strict transliteration silently drifts.
//! This module names the handful of `java.lang.Math` helpers and JVM narrowing
//! conversions whose Java semantics a port must not get wrong:
//!
//! * `Math.abs(Integer.MIN_VALUE)` is `Integer.MIN_VALUE`, not a positive value:
//!   the negation overflows and wraps (JLS 15.15.4). [`iabs`] and [`ineg`] use
//!   `wrapping_neg` so this holds.
//! * `(int)` on a `float`/`double` truncates toward zero, maps `NaN` to `0`, and
//!   *saturates* out-of-range magnitudes to `Integer.MIN_VALUE`/`MAX_VALUE`
//!   (JLS 5.1.3). Rust's `as` matches on all three — unlike C — so [`f2i`] and
//!   [`f2l`] are just the cast, named to make the narrowing explicit.
//! * `(float) Math.sqrt(x)` widens to `double`, takes the root, then narrows
//!   ([`fsqrt`]); computing the root in `f32` throughout can differ in the last
//!   bit.
//!
//! Shared here so every port uses one reviewed implementation
//! instead of re-deriving these edges. The names match the JVM conversion
//! mnemonics (`i2f`, `f2i`, `f2l`, `d2f`) and the `Math` method (`imax`, `imin`,
//! `iabs`) they stand in for.

/// `Math.max(int, int)`.
#[inline]
pub fn imax(left: i32, right: i32) -> i32 {
    if left >= right {
        left
    } else {
        right
    }
}

/// `Math.min(int, int)`.
#[inline]
pub fn imin(left: i32, right: i32) -> i32 {
    if left <= right {
        left
    } else {
        right
    }
}

/// `Math.abs(int)`. `Math.abs(Integer.MIN_VALUE)` is `Integer.MIN_VALUE`,
/// because the negation overflows and wraps rather than trapping.
#[inline]
pub fn iabs(value: i32) -> i32 {
    if value >= 0 {
        value
    } else {
        value.wrapping_neg()
    }
}

/// Unary `-` on an `int` (`ineg`). `-(Integer.MIN_VALUE)` wraps back to
/// `Integer.MIN_VALUE`; Rust's `-` would panic on overflow.
#[inline]
pub fn ineg(value: i32) -> i32 {
    value.wrapping_neg()
}

/// `(float)` on an `int` (`i2f`). Widening; may lose precision for magnitudes
/// past 2^24, exactly as the JVM rounds.
#[inline]
pub fn i2f(value: i32) -> f32 {
    value as f32
}

/// `(int)` on a `float` (`f2i`): truncates toward zero, `NaN` becomes `0`, and
/// out-of-range values saturate to `Integer.MIN_VALUE`/`MAX_VALUE`. Rust's `as`
/// matches on all three.
#[inline]
pub fn f2i(value: f32) -> i32 {
    value as i32
}

/// `(long)` on a `float` (`f2l`): same truncate-toward-zero, `NaN`-to-`0`,
/// saturating narrowing as [`f2i`], to 64 bits.
#[inline]
pub fn f2l(value: f32) -> i64 {
    value as i64
}

/// `(float)` on a `double` (`d2f`): narrows with round-to-nearest, overflow to
/// the signed infinities — the IEEE 754 rounding the JVM performs.
#[inline]
pub fn d2f(value: f64) -> f32 {
    value as f32
}

/// `(float) Math.sqrt(x)`. Java has no `float` square root: it widens the
/// argument to `double`, takes the root, and narrows the result back to
/// `float`. Reproducing that widen/narrow matters because a straight `f32`
/// square root can differ in the last bit.
#[inline]
pub fn fsqrt(value: f32) -> f32 {
    (value as f64).sqrt() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_max_pick_the_expected_operand() {
        assert_eq!(imax(3, 7), 7);
        assert_eq!(imax(-2, -9), -2);
        assert_eq!(imin(3, 7), 3);
        assert_eq!(imin(-2, -9), -9);
        // Ties return the (equal) operand.
        assert_eq!(imax(5, 5), 5);
        assert_eq!(imin(5, 5), 5);
    }

    #[test]
    fn abs_and_neg_wrap_at_min_value_like_java() {
        // The whole reason this module exists: Math.abs(MIN) == MIN.
        assert_eq!(iabs(i32::MIN), i32::MIN);
        assert_eq!(ineg(i32::MIN), i32::MIN);
        assert_eq!(iabs(-5), 5);
        assert_eq!(iabs(5), 5);
        assert_eq!(ineg(7), -7);
        assert_eq!(ineg(-7), 7);
    }

    #[test]
    fn float_to_int_truncates_saturates_and_maps_nan() {
        assert_eq!(f2i(2.9), 2);
        assert_eq!(f2i(-2.9), -2);
        assert_eq!(f2i(f32::NAN), 0);
        assert_eq!(f2i(f32::INFINITY), i32::MAX);
        assert_eq!(f2i(f32::NEG_INFINITY), i32::MIN);
        // A magnitude past i32::MAX saturates rather than wrapping.
        assert_eq!(f2i(1.0e30), i32::MAX);
    }

    #[test]
    fn float_to_long_matches_the_same_narrowing() {
        assert_eq!(f2l(2.9), 2);
        assert_eq!(f2l(-2.9), -2);
        assert_eq!(f2l(f32::NAN), 0);
        assert_eq!(f2l(f32::INFINITY), i64::MAX);
        assert_eq!(f2l(f32::NEG_INFINITY), i64::MIN);
    }

    #[test]
    fn int_to_float_and_double_to_float_round_like_the_jvm() {
        assert_eq!(i2f(176), 176.0);
        // 2^24 + 1 is not representable in f32; it rounds to 2^24.
        assert_eq!(i2f(16_777_217), 16_777_216.0);
        assert_eq!(d2f(0.846_153_846_153_846), 0.846_153_86);
    }

    #[test]
    fn float_sqrt_goes_through_double() {
        assert_eq!(fsqrt(4.0), 2.0);
        assert_eq!(fsqrt(0.0), 0.0);
        assert!((fsqrt(2.0) - std::f32::consts::SQRT_2).abs() < 1e-6);
    }
}
