//! The 3GPP fixed-point basic operators the AMR-NB decoder is defined in.
//!
//! AMR-NB is specified as *bit-exact fixed-point arithmetic*: TS 26.073 is a
//! C program, and a decoder is conformant when it reproduces that program's
//! integer output sample for sample. Every operator below is therefore a
//! literal transliteration of one reference operator, saturation quirks
//! included — they are the specification, not an implementation choice.
//!
//! Three quirks matter enough to name, because each one is a plausible
//! "cleanup" that would silently break exactness:
//!
//! - [`l_shr`] returns `0` for shifts of 31 or more, *including for negative
//!   inputs*, where an arithmetic shift would give `-1`.
//! - [`l_mult`] special-cases the single product `0x4000_0000` (that is,
//!   `-32768 * -32768`) to `i32::MAX` instead of wrapping.
//! - [`mult`] saturates only on the positive side, because a 16x16 product
//!   shifted right by 15 can exceed `i16::MAX` but never falls below
//!   `i16::MIN`.
//!
//! The reference threads a mutable `Overflow` flag through every operator.
//! The decoder in this crate does not carry one: the only consumer of that
//! flag in the reference decoder is an excitation-rescaling branch guarded by
//! a flag that the reference's own `Syn_filt` never sets, so the branch is
//! unreachable and the flag is write-only. Saturation behaviour, which does
//! affect the samples, is preserved exactly.
//!
//! Every operator here is total: no shift, cast, or arithmetic step can panic
//! for any input, which is what lets the decoder meet the repository's
//! "reject malformed input without panicking" rule by construction.

/// `add`: 16-bit addition saturating to the 16-bit range.
pub fn add(var1: i16, var2: i16) -> i16 {
    (i32::from(var1) + i32::from(var2)).clamp(-32768, 32767) as i16
}

/// `sub`: 16-bit subtraction saturating to the 16-bit range.
pub fn sub(var1: i16, var2: i16) -> i16 {
    (i32::from(var1) - i32::from(var2)).clamp(-32768, 32767) as i16
}

/// `negate`: negation with `i16::MIN` mapped to `i16::MAX`.
pub fn negate(var1: i16) -> i16 {
    if var1 == i16::MIN {
        i16::MAX
    } else {
        -var1
    }
}

/// `mult`: `(var1 * var2) >> 15`, saturating high only.
///
/// The reference clamps solely against `0x7fff`; the product of two 16-bit
/// values shifted right by 15 cannot underflow, so no low clamp exists to
/// transliterate.
pub fn mult(var1: i16, var2: i16) -> i16 {
    let product = (i32::from(var1) * i32::from(var2)) >> 15;
    if product > 0x7fff {
        i16::MAX
    } else {
        product as i16
    }
}

/// `mult_r`: rounded `(var1 * var2 + 0x4000) >> 15`, saturating both ways.
pub fn mult_r(var1: i16, var2: i16) -> i16 {
    let mut product = (i32::from(var1) * i32::from(var2) + 0x4000) >> 15;
    // The reference sign-extends bit 16 before clamping.
    product |= -(product & 0x0001_0000);
    product.clamp(-32768, 32767) as i16
}

/// `shl`: 16-bit left shift saturating on lost bits; negative counts shift
/// right.
pub fn shl(var1: i16, var2: i16) -> i16 {
    if var2 < 0 {
        let count = (-i32::from(var2)).min(15);
        // The reference yields 0 for counts of 15 or more via its own guard;
        // an arithmetic shift by 15 of an i16 is equivalent here.
        return (i32::from(var1) >> count) as i16;
    }
    let count = i32::from(var2).min(31) as u32;
    let wide = i32::from(var1).wrapping_shl(count);
    let narrow = wide as i16;
    if i32::from(narrow) >> count != i32::from(var1) {
        if var1 > 0 {
            i16::MAX
        } else {
            i16::MIN
        }
    } else {
        narrow
    }
}

/// `shr`: 16-bit arithmetic right shift; negative counts shift left with
/// saturation.
pub fn shr(var1: i16, var2: i16) -> i16 {
    if var2 == 0 {
        return var1;
    }
    if var2 > 0 {
        let count = i32::from(var2).min(15);
        return (i32::from(var1) >> count) as i16;
    }
    shl(var1, -var2.max(-15))
}

/// `L_add`: 32-bit addition saturating to the 32-bit range.
pub fn l_add(var1: i32, var2: i32) -> i32 {
    match var1.checked_add(var2) {
        Some(sum) => sum,
        None if var1 < 0 => i32::MIN,
        None => i32::MAX,
    }
}

/// `L_sub`: 32-bit subtraction saturating to the 32-bit range.
pub fn l_sub(var1: i32, var2: i32) -> i32 {
    match var1.checked_sub(var2) {
        Some(difference) => difference,
        None if var1 < 0 => i32::MIN,
        None => i32::MAX,
    }
}

/// `L_mult`: `var1 * var2 << 1`, with the reference's `0x4000_0000`
/// special case.
///
/// Only one input pair reaches that product — `-32768 * -32768` — and the
/// reference returns `i32::MAX` for it rather than the wrapped shift.
pub fn l_mult(var1: i16, var2: i16) -> i32 {
    let product = i32::from(var1) * i32::from(var2);
    if product != 0x4000_0000 {
        product << 1
    } else {
        i32::MAX
    }
}

/// `L_mac`: `acc + (var1 * var2 << 1)`, saturating.
pub fn l_mac(acc: i32, var1: i16, var2: i16) -> i32 {
    l_add(acc, l_mult(var1, var2))
}

/// `L_msu`: `acc - (var1 * var2 << 1)`, saturating.
pub fn l_msu(acc: i32, var1: i16, var2: i16) -> i32 {
    l_sub(acc, l_mult(var1, var2))
}

/// `L_shl`: 32-bit left shift saturating on lost bits; negative counts shift
/// right.
pub fn l_shl(var1: i32, var2: i16) -> i32 {
    if var2 > 0 {
        let count = (var2 as u32).min(63);
        if count > 31 {
            return if var1 == 0 {
                0
            } else if var1 < 0 {
                i32::MIN
            } else {
                i32::MAX
            };
        }
        let shifted = var1.wrapping_shl(count);
        if shifted >> count != var1 {
            if var1 < 0 {
                i32::MIN
            } else {
                i32::MAX
            }
        } else {
            shifted
        }
    } else {
        let count = -i32::from(var2);
        if count < 31 {
            var1 >> count
        } else {
            // The reference leaves its output variable at its zero
            // initialiser for counts of 31 or more.
            0
        }
    }
}

/// `L_shr`: 32-bit arithmetic right shift; negative counts shift left with
/// saturation.
///
/// Note the asymmetry with an ordinary arithmetic shift: a count of 31 or
/// more yields `0` even for a negative input, because the reference guards
/// the shift with `var2 < 31` and returns its zero-initialised output
/// otherwise.
pub fn l_shr(var1: i32, var2: i16) -> i32 {
    if var2 > 0 {
        if i32::from(var2) < 31 {
            var1 >> var2
        } else {
            0
        }
    } else {
        l_shl(var1, -var2.max(-31))
    }
}

/// `L_shr_r`: [`l_shr`] with rounding from the last bit shifted out.
pub fn l_shr_r(var1: i32, var2: i16) -> i32 {
    if var2 > 31 {
        return 0;
    }
    let mut result = l_shr(var1, var2);
    if var2 > 0 && var1 & (1i32 << (var2 - 1)) != 0 {
        result = result.wrapping_add(1);
    }
    result
}

/// `pv_round`: round a 32-bit accumulator into 16 bits.
pub fn round16(var1: i32) -> i16 {
    (l_add(var1, 0x8000) >> 16) as i16
}

/// `norm_l`: left shifts needed to normalise a 32-bit value, `0` for zero.
pub fn norm_l(var1: i32) -> i16 {
    if var1 == 0 {
        return 0;
    }
    let y = var1 - i32::from(var1 < 0);
    let magnitude = y ^ (y >> 31);
    (magnitude.leading_zeros() as i16) - 1
}

/// `norm_s`: left shifts needed to normalise a 16-bit value, `0` for zero.
pub fn norm_s(var1: i16) -> i16 {
    if var1 == 0 {
        return 0;
    }
    let y = var1 - i16::from(var1 < 0);
    let magnitude = y ^ (y >> 15);
    ((magnitude as u16).leading_zeros() as i16) - 1
}

/// `div_s`: fractional division of two non-negative 16-bit values.
///
/// Returns `0` when the numerator is negative or exceeds the denominator,
/// and `i16::MAX` when they are equal — both are the reference's own
/// guards, not error signalling.
pub fn div_s(var1: i16, var2: i16) -> i16 {
    if var1 > var2 || var1 < 0 {
        return 0;
    }
    if var1 == 0 {
        return 0;
    }
    if var1 == var2 {
        return i16::MAX;
    }
    let mut var_out: i16 = 0;
    let mut numerator = i32::from(var1);
    let denominator = i32::from(var2);
    let denominator_by_2 = denominator << 1;
    let denominator_by_4 = denominator << 2;
    for _ in 0..5 {
        var_out <<= 3;
        numerator <<= 3;
        if numerator >= denominator_by_4 {
            numerator -= denominator_by_4;
            var_out |= 4;
        }
        if numerator >= denominator_by_2 {
            numerator -= denominator_by_2;
            var_out |= 2;
        }
        if numerator >= denominator {
            numerator -= denominator;
            var_out |= 1;
        }
    }
    var_out
}

/// `Log2_norm`: base-2 logarithm of an already normalised 32-bit value.
pub fn log2_norm(var1: i32, exp: i16) -> (i16, i16) {
    if var1 <= 0 {
        return (0, 0);
    }
    let exponent = 30 - exp;
    let shifted = var1 >> 10;
    let index = ((shifted >> 15) - 32) as usize;
    let offset = shifted & 0x7fff;
    let table = &super::tables::LOG2_TBL;
    let mut accumulator = i32::from(table[index]) << 16;
    let step = table[index] - table[index + 1];
    accumulator -= (i32::from(step) * offset) << 1;
    (exponent, (accumulator >> 16) as i16)
}

/// `Log2`: base-2 logarithm as a separated exponent and fraction.
pub fn log2(var1: i32) -> (i16, i16) {
    let exp = norm_l(var1);
    log2_norm(var1 << exp, exp)
}

/// `Pow2`: `2^(exponent + fraction)` in 32-bit fixed point.
pub fn pow2(exponent: i16, fraction: i16) -> i32 {
    let scaled = l_mult(fraction, 32);
    let index = (((scaled >> 16) as i16) & 31) as usize;
    let offset = ((scaled >> 1) as i16) & 0x7fff;
    let table = &super::tables::POW2_TBL;
    let mut accumulator = i32::from(table[index]) << 16;
    let step = table[index] - table[index + 1];
    accumulator = l_msu(accumulator, step, offset);
    l_shr_r(accumulator, 30 - exponent)
}

/// `Inv_sqrt`: `1 / sqrt(var1)` in 32-bit fixed point.
pub fn inv_sqrt(var1: i32) -> i32 {
    if var1 <= 0 {
        return 0x3fff_ffff;
    }
    let mut value = var1;
    let mut exp = norm_l(value);
    value <<= exp;
    exp = 30 - exp;
    if exp & 1 == 0 {
        value >>= 1;
    }
    exp >>= 1;
    exp += 1;
    value >>= 9;
    let index = ((value >> 16) - 16) as usize;
    let offset = ((value >> 1) as i16) & 0x7fff;
    let table = &super::tables::INV_SQRT_TBL;
    let mut accumulator = i32::from(table[index]) << 16;
    let step = table[index] - table[index + 1];
    accumulator -= (i32::from(step) * i32::from(offset)) << 1;
    accumulator >> exp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saturation_follows_the_reference_and_never_panics() {
        assert_eq!(add(32767, 1), 32767);
        assert_eq!(add(-32768, -1), -32768);
        assert_eq!(sub(-32768, 1), -32768);
        assert_eq!(negate(-32768), 32767);
        // `mult` clamps high only; the low side cannot be reached.
        assert_eq!(mult(-32768, -32768), 32767);
        assert_eq!(mult(-32768, 32767), -32767);
        // `L_mult`'s one special-cased product.
        assert_eq!(l_mult(-32768, -32768), i32::MAX);
        // Every other product is simply doubled: 16384^2 << 1.
        assert_eq!(l_mult(16384, 16384), 0x2000_0000);
        assert_eq!(l_add(i32::MAX, 1), i32::MAX);
        assert_eq!(l_sub(i32::MIN, 1), i32::MIN);
    }

    #[test]
    fn the_shift_operators_keep_the_references_edge_cases() {
        // The quirk a "cleanup" would break: 31 or more shifts out to zero,
        // not to the sign.
        assert_eq!(l_shr(-1, 31), 0);
        assert_eq!(l_shr(-1, 30), -1);
        assert_eq!(l_shr(-4, 1), -2);
        assert_eq!(l_shl(1 << 30, 2), i32::MAX);
        assert_eq!(l_shl(-(1 << 30), 2), i32::MIN);
        assert_eq!(l_shl(8, -2), 2);
        assert_eq!(shl(16384, 2), i16::MAX);
        assert_eq!(shl(-16384, 2), i16::MIN);
        assert_eq!(shr(-8, 2), -2);
        assert_eq!(l_shr_r(3, 1), 2);
        assert_eq!(l_shr_r(1, 1), 1);
        assert_eq!(l_shr_r(1, 32), 0);
    }

    #[test]
    fn normalisation_and_division_match_the_reference_loops() {
        assert_eq!(norm_l(0), 0);
        assert_eq!(norm_l(1), 30);
        assert_eq!(norm_l(-1), 30);
        assert_eq!(norm_l(1 << 30), 0);
        assert_eq!(norm_s(0), 0);
        assert_eq!(norm_s(1), 14);
        assert_eq!(norm_s(1 << 14), 0);
        assert_eq!(div_s(0, 1), 0);
        assert_eq!(div_s(5, 5), i16::MAX);
        assert_eq!(div_s(6, 5), 0);
        assert_eq!(div_s(-1, 5), 0);
        // 1/2 in Q15.
        assert_eq!(div_s(1, 2), 16384);
    }

    #[test]
    fn the_transcendental_helpers_round_trip_within_their_resolution() {
        // Log2 of 2^30 is exactly 30 with a zero fraction.
        assert_eq!(log2(1 << 30), (30, 0));
        // Log2 separates the exponent from the fraction; 2^29 is exact.
        assert_eq!(log2(1 << 29), (29, 0));
        // Pow2 inverts it at full scale.
        assert_eq!(pow2(30, 0), 0x4000_0000);
        // Inv_sqrt's guard value for non-positive input.
        assert_eq!(inv_sqrt(0), 0x3fff_ffff);
        assert_eq!(inv_sqrt(-5), 0x3fff_ffff);
        assert!(inv_sqrt(1 << 30) > 0);
    }
}
