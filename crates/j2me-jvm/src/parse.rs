//! `java.lang.Integer.parseInt`, exactly — Unicode digits and all.
//!
//! `Integer.parseInt(String)` is *not* ASCII-only. It parses each character with
//! `Character.digit`, which recognises the decimal digit `0-9` of every Unicode
//! script — Arabic-Indic, Devanagari, fullwidth, and dozens more — by testing
//! it against a table of each script's "digit zero" code point. A port that
//! only accepts `b'0'..=b'9'` silently rejects input a real JVM would parse, so
//! this carries the full digit-zero table.
//!
//! Overflow follows the JVM's `Integer.parseInt`: magnitude is accumulated
//! against the positive limit `2147483647` or, for a leading `-`, the negative
//! limit `2147483648`, so `"-2147483648"` parses to `Integer.MIN_VALUE` while
//! `"2147483648"` overflows.
//!
//! Seeded from the silent-hill port's `parse_java_i32` so every port shares one
//! copy. Input is `&[u16]` (UTF-16 code units, how a Java `String` is stored and
//! how `charAt`/`Character.digit` see it); the recognised digit zeroes are all
//! in the BMP, so a single `u16` per digit is faithful.

use crate::{JavaError, JavaResult};

/// The "digit zero" code point of every Unicode script whose decimal digits
/// `Character.digit(c, 10)` recognises. A code unit in `zero..=zero+9` is the
/// digit `unit - zero`. Kept identical to the JDK's table (and the silent-hill
/// source it was hoisted from).
const DIGIT_ZEROES: [u16; 37] = [
    0x0030, 0x0660, 0x06f0, 0x07c0, 0x0966, 0x09e6, 0x0a66, 0x0ae6, 0x0b66, 0x0be6, 0x0c66, 0x0ce6,
    0x0d66, 0x0de6, 0x0e50, 0x0ed0, 0x0f20, 0x1040, 0x1090, 0x17e0, 0x1810, 0x1946, 0x19d0, 0x1a80,
    0x1a90, 0x1b50, 0x1bb0, 0x1c40, 0x1c50, 0xa620, 0xa8d0, 0xa900, 0xa9d0, 0xa9f0, 0xaa50, 0xabf0,
    0xff10,
];

/// `Integer.parseInt(String)` (radix 10).
///
/// Returns the parsed value, or [`JavaError::IllegalArgument`] carrying
/// `"NumberFormatException"` — the `IllegalArgumentException` subclass Java
/// throws — for empty input, a sign with no digits, a non-digit character, or a
/// value outside the `int` range. An optional leading `+`/`-` is accepted, as by
/// the JVM.
pub fn parse_int(text: &[u16]) -> JavaResult<i32> {
    parse_int_opt(text).ok_or(JavaError::IllegalArgument("NumberFormatException"))
}

/// The parse as an `Option`, for callers that prefer the source's shape over the
/// [`JavaError`] model. `None` is Java's `NumberFormatException`.
pub fn parse_int_opt(text: &[u16]) -> Option<i32> {
    if text.is_empty() {
        return None;
    }
    let (negative, digits) = match text[0] {
        0x002d => (true, &text[1..]),  // '-'
        0x002b => (false, &text[1..]), // '+'
        _ => (false, text),
    };
    if digits.is_empty() {
        return None;
    }
    let limit = if negative {
        2_147_483_648_u64
    } else {
        2_147_483_647_u64
    };
    let mut magnitude = 0_u64;
    for unit in digits {
        let digit = DIGIT_ZEROES.iter().find_map(|zero| {
            if *unit >= *zero && *unit <= *zero + 9 {
                Some(u64::from(*unit - *zero))
            } else {
                None
            }
        })?;
        if magnitude > (limit - digit) / 10 {
            return None;
        }
        magnitude = magnitude * 10 + digit;
    }
    if negative {
        Some((0_i64 - magnitude as i64) as i32)
    } else {
        Some(magnitude as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    #[test]
    fn parses_plain_decimal_strings() {
        assert_eq!(parse_int(&utf16("123")).unwrap(), 123);
        assert_eq!(parse_int(&utf16("0")).unwrap(), 0);
        assert_eq!(parse_int(&utf16("+42")).unwrap(), 42);
        assert_eq!(parse_int(&utf16("-7")).unwrap(), -7);
    }

    #[test]
    fn parses_the_int_range_boundaries() {
        assert_eq!(parse_int(&utf16("2147483647")).unwrap(), i32::MAX);
        assert_eq!(parse_int(&utf16("-2147483648")).unwrap(), i32::MIN);
    }

    #[test]
    fn overflow_on_either_side_is_a_number_format_exception() {
        assert_eq!(
            parse_int(&utf16("2147483648")),
            Err(JavaError::IllegalArgument("NumberFormatException"))
        );
        assert!(parse_int(&utf16("-2147483649")).is_err());
        assert!(parse_int(&utf16("9999999999")).is_err());
    }

    #[test]
    fn malformed_input_is_rejected() {
        assert!(parse_int(&utf16("")).is_err());
        assert!(parse_int(&utf16("+")).is_err());
        assert!(parse_int(&utf16("-")).is_err());
        assert!(parse_int(&utf16("12x")).is_err());
        assert!(parse_int(&utf16("1 2")).is_err());
    }

    #[test]
    fn non_ascii_unicode_digits_parse_like_the_jvm() {
        // Arabic-Indic digits U+0661 U+0662 U+0663 == "123".
        assert_eq!(parse_int(&[0x0661, 0x0662, 0x0663]).unwrap(), 123);
        // Fullwidth digits U+FF11 U+FF10 == "10".
        assert_eq!(parse_int(&[0xff11, 0xff10]).unwrap(), 10);
        // Character.digit runs per character, so scripts may even be mixed:
        // ASCII '1' followed by Arabic-Indic '2' parses to 12, as the JVM does.
        assert_eq!(parse_int(&[0x0031, 0x0662]).unwrap(), 12);
    }

    #[test]
    fn opt_form_mirrors_the_result_form() {
        assert_eq!(parse_int_opt(&utf16("55")), Some(55));
        assert_eq!(parse_int_opt(&utf16("nope")), None);
    }
}
