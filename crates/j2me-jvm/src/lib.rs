//! Game-neutral JVM primitive semantics for strict Java-to-Rust translation.
//!
//! Game bodies should route operations through this layer whenever ordinary
//! Rust has different observable behavior: overflow, division, shifts, signed
//! array indices, exceptions, or time. The layer is ordinary `std` Rust; only
//! recovered serialization codecs are required to remain `no_std`.
//!
//! ## Two spellings of the same JVM semantics
//!
//! The upper block is the const-fn arithmetic surface (`i32_*` / `i64_*`,
//! `narrow_*`, `array_ref`/`array_mut`, the `Clock` family, `JavaError`). The
//! lower block carries the bytecode-mnemonic spelling of the same semantics — the
//! `ishl`/`java_div` helpers, the `jget!`/`jset!` checked-access macros, and
//! slice-typed `java_array_*` accessors — for transliterations written directly
//! against the JVM opcode names. Both are canonical; pick whichever spelling
//! reads closest to the code being ported.
//!
//! ## `java.util.Random`
//!
//! [`random`] adds the JVM's 48-bit LCG ([`Random`]) so every port shares one
//! bit-identical `java.util.Random` instead of duplicating it.
//!
//! ## `java.lang.Math` and numeric conversions
//!
//! [`math`] names the `java.lang.Math` helpers and JVM narrowing casts whose
//! Java edge semantics differ from naive Rust — `Math.abs(MIN) == MIN`, the
//! saturating `(int)`-on-`float` narrowing, and the widen/narrow `(float)
//! Math.sqrt` — so every port shares one copy instead of re-deriving them.
//!
//! ## `java.io.DataInputStream` / `DataOutputStream`
//!
//! [`io`] provides an owned [`ByteArrayInputStream`] with exact Java
//! `buf`/`pos`/`mark`/`count` state, plus the big-endian, signed
//! [`DataInputStream`] / [`DataOutputStream`] codec including modified-UTF-8
//! `readUTF`/`writeUTF`. It is distinct from `j2me-codec`'s `no_std` bounded
//! reader: this layer is `std`, returns [`JavaError`]/[`JavaResult`], and
//! preserves Java stream cursor mutation at failure boundaries.
//!
//! ## `java.lang.Integer.parseInt`
//!
//! [`parse`] parses a decimal `int` with the JVM's full Unicode digit table
//! (`Character.digit` across every script, not just ASCII) and its
//! signed-overflow limits, so every port shares one copy.
//!
//! ## `java.lang.Thread` / `Runnable`
//!
//! [`thread`] provides runtime-owned thread identities and a deterministic,
//! cooperative start queue. Hosts dispatch game-owned Runnable callbacks
//! explicitly, with exact `Thread.currentThread()` identity and one-shot start
//! semantics, rather than introducing nondeterministic native threads.

pub mod io;
pub mod math;
pub mod parse;
pub mod random;
pub mod thread;

pub use io::{ByteArrayInputStream, DataInputStream, DataOutputStream};
pub use math::{d2f, f2i, f2l, fsqrt, i2f, iabs, imax, imin, ineg};
pub use parse::{parse_int, parse_int_opt};
pub use random::Random;
pub use thread::{HostThreadOp, RunnableId, ThreadId, ThreadRuntime, ThreadState};

use std::cell::Cell;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JavaError {
    NullPointer,
    IndexOutOfBounds,
    ArrayIndexOutOfBounds { index: i32, length: i32 },
    NegativeArraySize { length: i32 },
    Arithmetic,
    IllegalArgument(&'static str),
    IllegalState(&'static str),
    IllegalThreadState,
    ClassCast,
    Io(String),
    ConnectionNotFound(String),
    RecordStore(String),
    Media(String),
}

impl fmt::Display for JavaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NullPointer => write!(formatter, "NullPointerException"),
            Self::IndexOutOfBounds => write!(formatter, "IndexOutOfBoundsException"),
            Self::ArrayIndexOutOfBounds { index, length } => write!(
                formatter,
                "ArrayIndexOutOfBoundsException: index {index}, length {length}"
            ),
            Self::NegativeArraySize { length } => {
                write!(formatter, "NegativeArraySizeException: length {length}")
            }
            Self::Arithmetic => write!(formatter, "ArithmeticException"),
            Self::IllegalArgument(message) => {
                write!(formatter, "IllegalArgumentException: {message}")
            }
            Self::IllegalState(message) => write!(formatter, "IllegalStateException: {message}"),
            Self::IllegalThreadState => write!(formatter, "IllegalThreadStateException"),
            Self::ClassCast => write!(formatter, "ClassCastException"),
            Self::Io(message) => write!(formatter, "IOException: {message}"),
            Self::ConnectionNotFound(message) => {
                write!(formatter, "ConnectionNotFoundException: {message}")
            }
            Self::RecordStore(message) => {
                write!(formatter, "RecordStoreException: {message}")
            }
            Self::Media(message) => write!(formatter, "MediaException: {message}"),
        }
    }
}

impl std::error::Error for JavaError {}

pub type JavaResult<T> = Result<T, JavaError>;

#[inline]
pub const fn i32_add(left: i32, right: i32) -> i32 {
    left.wrapping_add(right)
}

#[inline]
pub const fn i32_sub(left: i32, right: i32) -> i32 {
    left.wrapping_sub(right)
}

#[inline]
pub const fn i32_mul(left: i32, right: i32) -> i32 {
    left.wrapping_mul(right)
}

#[inline]
pub const fn i32_div(left: i32, right: i32) -> JavaResult<i32> {
    if right == 0 {
        Err(JavaError::Arithmetic)
    } else if left == i32::MIN && right == -1 {
        Ok(i32::MIN)
    } else {
        Ok(left / right)
    }
}

#[inline]
pub const fn i32_rem(left: i32, right: i32) -> JavaResult<i32> {
    if right == 0 {
        Err(JavaError::Arithmetic)
    } else if left == i32::MIN && right == -1 {
        Ok(0)
    } else {
        Ok(left % right)
    }
}

#[inline]
pub const fn i64_add(left: i64, right: i64) -> i64 {
    left.wrapping_add(right)
}

#[inline]
pub const fn i64_sub(left: i64, right: i64) -> i64 {
    left.wrapping_sub(right)
}

#[inline]
pub const fn i64_mul(left: i64, right: i64) -> i64 {
    left.wrapping_mul(right)
}

#[inline]
pub const fn i64_div(left: i64, right: i64) -> JavaResult<i64> {
    if right == 0 {
        Err(JavaError::Arithmetic)
    } else if left == i64::MIN && right == -1 {
        Ok(i64::MIN)
    } else {
        Ok(left / right)
    }
}

#[inline]
pub const fn i64_rem(left: i64, right: i64) -> JavaResult<i64> {
    if right == 0 {
        Err(JavaError::Arithmetic)
    } else if left == i64::MIN && right == -1 {
        Ok(0)
    } else {
        Ok(left % right)
    }
}

#[inline]
pub const fn i32_shl(value: i32, distance: i32) -> i32 {
    value.wrapping_shl((distance & 31) as u32)
}

#[inline]
pub const fn i32_shr(value: i32, distance: i32) -> i32 {
    value.wrapping_shr((distance & 31) as u32)
}

#[inline]
pub const fn i32_ushr(value: i32, distance: i32) -> i32 {
    ((value as u32) >> ((distance & 31) as u32)) as i32
}

#[inline]
pub const fn i64_shl(value: i64, distance: i32) -> i64 {
    value.wrapping_shl((distance & 63) as u32)
}

#[inline]
pub const fn i64_shr(value: i64, distance: i32) -> i64 {
    value.wrapping_shr((distance & 63) as u32)
}

#[inline]
pub const fn i64_ushr(value: i64, distance: i32) -> i64 {
    ((value as u64) >> ((distance & 63) as u32)) as i64
}

#[inline]
pub const fn narrow_byte(value: i32) -> i8 {
    value as i8
}

#[inline]
pub const fn narrow_short(value: i32) -> i16 {
    value as i16
}

#[inline]
pub const fn narrow_char(value: i32) -> u16 {
    value as u16
}

pub fn new_i32_array(length: i32) -> JavaResult<Vec<i32>> {
    if length < 0 {
        Err(JavaError::NegativeArraySize { length })
    } else {
        Ok(vec![0; length as usize])
    }
}

pub fn array_ref<T>(values: Option<&[T]>, index: i32) -> JavaResult<&T> {
    let values = values.ok_or(JavaError::NullPointer)?;
    let length = values.len() as i32;
    if index < 0 || index >= length {
        Err(JavaError::ArrayIndexOutOfBounds { index, length })
    } else {
        Ok(&values[index as usize])
    }
}

pub fn array_mut<T>(values: Option<&mut [T]>, index: i32) -> JavaResult<&mut T> {
    let values = values.ok_or(JavaError::NullPointer)?;
    let length = values.len() as i32;
    if index < 0 || index >= length {
        Err(JavaError::ArrayIndexOutOfBounds { index, length })
    } else {
        Ok(&mut values[index as usize])
    }
}

pub fn array_2d_mut<T>(
    values: Option<&mut [Vec<T>]>,
    first: i32,
    second: i32,
) -> JavaResult<&mut T> {
    let inner = array_mut(values, first)?;
    array_mut(Some(inner), second)
}

pub trait Clock {
    fn current_time_millis(&self) -> i64;

    /// Advance a deterministic clock by `delta_millis`. Real wall clocks keep
    /// the default no-op because time already moves independently. Keeping this
    /// on the trait lets headless hosts step time through `&dyn Clock` without
    /// exposing or downcasting the concrete clock implementation.
    fn advance(&self, _delta_millis: i64) {}
}

#[derive(Debug, Default, Clone, Copy)]
pub struct WallClock;

impl Clock for WallClock {
    fn current_time_millis(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis() as i64)
    }
}

#[derive(Debug, Default)]
pub struct VirtualClock {
    millis: Cell<i64>,
}

impl VirtualClock {
    pub const fn new(millis: i64) -> Self {
        Self {
            millis: Cell::new(millis),
        }
    }

    pub fn set(&self, millis: i64) {
        self.millis.set(millis);
    }

    pub fn advance(&self, delta_millis: i64) {
        self.millis
            .set(self.millis.get().wrapping_add(delta_millis));
    }
}

impl Clock for VirtualClock {
    fn current_time_millis(&self) -> i64 {
        self.millis.get()
    }

    fn advance(&self, delta_millis: i64) {
        VirtualClock::advance(self, delta_millis);
    }
}

// ===========================================================================
// JVM-primitive helpers in the bytecode-mnemonic spelling.
//
// These express exactly the same JVM semantics as the `i32_*`/`i64_*`/`array_*`
// surface above, under the opcode names (`ishl`, `iushr`, `java_div`, …) a
// transliteration written directly against the bytecode uses, plus the
// `jget!`/`jset!` checked-access macros and slice-typed `java_array_*`
// accessors. Use whichever spelling reads closest to the code being ported.
// ===========================================================================

/// Java `int` division. Differs from Rust only at `i32::MIN / -1`, where Java
/// yields `i32::MIN` and Rust panics. Division by zero raises
/// `ArithmeticException` (the caller decides whether the original guarded it).
#[inline]
pub fn java_div(a: i32, b: i32) -> JavaResult<i32> {
    if b == 0 {
        return Err(JavaError::Arithmetic);
    }
    Ok(a.wrapping_div(b))
}

/// Java `int` remainder. `i32::MIN % -1 == 0` in Java (Rust panics); sign
/// follows the dividend, which already matches Rust.
#[inline]
pub fn java_rem(a: i32, b: i32) -> JavaResult<i32> {
    if b == 0 {
        return Err(JavaError::Arithmetic);
    }
    Ok(a.wrapping_rem(b))
}

/// Java `long` division (same overflow case as [`java_div`]).
#[inline]
pub fn java_ldiv(a: i64, b: i64) -> JavaResult<i64> {
    if b == 0 {
        return Err(JavaError::Arithmetic);
    }
    Ok(a.wrapping_div(b))
}

/// Java `long` remainder.
#[inline]
pub fn java_lrem(a: i64, b: i64) -> JavaResult<i64> {
    if b == 0 {
        return Err(JavaError::Arithmetic);
    }
    Ok(a.wrapping_rem(b))
}

/// Java `int` left shift: the count is masked to 5 bits.
#[inline]
pub fn ishl(x: i32, n: i32) -> i32 {
    x.wrapping_shl(n as u32)
}

/// Java `int` arithmetic right shift: the count is masked to 5 bits.
#[inline]
pub fn ishr(x: i32, n: i32) -> i32 {
    x.wrapping_shr(n as u32)
}

/// Java `int` unsigned right shift `>>>`: mask the count, shift as unsigned.
#[inline]
pub fn iushr(x: i32, n: i32) -> i32 {
    ((x as u32) >> (n & 31)) as i32
}

/// Java `long` left shift: the count is masked to 6 bits.
#[inline]
pub fn lshl(x: i64, n: i32) -> i64 {
    x.wrapping_shl(n as u32)
}

/// Java `long` arithmetic right shift: the count is masked to 6 bits.
#[inline]
pub fn lshr(x: i64, n: i32) -> i64 {
    x.wrapping_shr(n as u32)
}

/// Java `long` unsigned right shift `>>>`: mask the count, shift as unsigned.
#[inline]
pub fn lushr(x: i64, n: i32) -> i64 {
    ((x as u64) >> (n & 63)) as i64
}

/// Checked array read reproducing Java's `ArrayIndexOutOfBoundsException`.
/// Use inside a region the original guarded with `try/catch`; outside one,
/// index directly (a panic is then faithful).
#[macro_export]
macro_rules! jget {
    ($arr:expr, $idx:expr) => {{
        let idx: i32 = $idx;
        let len = $arr.len();
        if idx < 0 || (idx as usize) >= len {
            Err($crate::JavaError::ArrayIndexOutOfBounds {
                index: idx,
                length: len as i32,
            })
        } else {
            Ok($arr[idx as usize])
        }
    }};
}

/// Checked array write counterpart to [`jget!`].
#[macro_export]
macro_rules! jset {
    ($arr:expr, $idx:expr, $val:expr) => {{
        let idx: i32 = $idx;
        let len = $arr.len();
        if idx < 0 || (idx as usize) >= len {
            Err($crate::JavaError::ArrayIndexOutOfBounds {
                index: idx,
                length: len as i32,
            })
        } else {
            $arr[idx as usize] = $val;
            Ok(())
        }
    }};
}

/// Borrows one array element with Java's signed-index exception semantics.
/// A negative index (including one produced by wrapping arithmetic) and an
/// index at or past the length both raise `ArrayIndexOutOfBoundsException`.
pub fn java_array_ref<T>(values: &[T], index: i32) -> JavaResult<&T> {
    let length = values.len() as i32;
    if index < 0 || index >= length {
        Err(JavaError::ArrayIndexOutOfBounds { index, length })
    } else {
        Ok(&values[index as usize])
    }
}

/// Mutably borrows one array element with Java's signed-index semantics.
pub fn java_array_mut<T>(values: &mut [T], index: i32) -> JavaResult<&mut T> {
    let length = values.len() as i32;
    if index < 0 || index >= length {
        Err(JavaError::ArrayIndexOutOfBounds { index, length })
    } else {
        Ok(&mut values[index as usize])
    }
}

/// Mutably borrows one element from a two-dimensional Java array, checking the
/// outer dimension before the inner one (the order the JVM does).
pub fn java_array_2d_mut<T>(values: &mut [Vec<T>], first: i32, second: i32) -> JavaResult<&mut T> {
    java_array_mut(java_array_mut(values, first)?, second)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_and_shifts_match_java_edges() {
        assert_eq!(i32_add(i32::MAX, 1), i32::MIN);
        assert_eq!(i32_div(i32::MIN, -1), Ok(i32::MIN));
        assert_eq!(i32_rem(i32::MIN, -1), Ok(0));
        assert_eq!(i32_div(1, 0), Err(JavaError::Arithmetic));
        assert_eq!(i32_shl(1, 32), 1);
        assert_eq!(i32_ushr(-1, 1), i32::MAX);
        assert_eq!(i64_ushr(-1, 60), 15);
    }

    #[test]
    fn arrays_check_null_then_signed_bounds() {
        let mut values = [10, 20];
        assert_eq!(array_ref(Some(&values), 1), Ok(&20));
        assert_eq!(array_ref::<i32>(None, 0), Err(JavaError::NullPointer));
        assert_eq!(
            array_mut(Some(&mut values), -1),
            Err(JavaError::ArrayIndexOutOfBounds {
                index: -1,
                length: 2,
            })
        );
    }

    #[test]
    fn virtual_clock_wraps_like_java_long() {
        let clock = VirtualClock::new(i64::MAX);
        clock.advance(1);
        assert_eq!(clock.current_time_millis(), i64::MIN);
    }
}

// Unit tests for the bytecode-mnemonic helpers above.
#[cfg(test)]
mod mnemonic_tests {
    use super::*;

    #[test]
    fn division_overflow_matches_java() {
        // Java: i32::MIN / -1 == i32::MIN (no overflow trap); Rust would panic.
        assert_eq!(java_div(i32::MIN, -1), Ok(i32::MIN));
        assert_eq!(java_rem(i32::MIN, -1), Ok(0));
        assert_eq!(java_div(7, 2), Ok(3));
        assert_eq!(java_div(-7, 2), Ok(-3)); // truncates toward zero
        assert_eq!(java_rem(-7, 2), Ok(-1)); // sign follows dividend
        assert_eq!(java_div(1, 0), Err(JavaError::Arithmetic));
        assert_eq!(java_ldiv(i64::MIN, -1), Ok(i64::MIN));
        assert_eq!(java_lrem(i64::MIN, -1), Ok(0));
    }

    #[test]
    fn unsigned_shift_masks_and_zero_fills() {
        // >>> fills with zero and masks the count to 5 bits.
        assert_eq!(iushr(-1, 28), 0x0000_000F);
        assert_eq!(iushr(-1, 32), -1); // 32 & 31 == 0 -> no shift, Java semantics
        assert_eq!(ishl(1, 33), 2); // 33 & 31 == 1
        assert_eq!(ishr(-8, 1), -4);
        // long masks to 6 bits.
        assert_eq!(lshl(1, 65), 2);
        assert_eq!(lushr(-1, 60), 0xF);
        assert_eq!(lshr(-8, 1), -4);
    }

    #[test]
    fn virtual_clock_is_deterministic() {
        let c = VirtualClock::new(1000);
        assert_eq!(c.current_time_millis(), 1000);
        c.advance(250);
        assert_eq!(c.current_time_millis(), 1250);
    }

    #[test]
    fn checked_array_access_reports_bounds() {
        let mut a = [10i32, 20, 30];
        assert_eq!(jget!(a, 1), Ok(20));
        assert_eq!(
            jget!(a, 5),
            Err(JavaError::ArrayIndexOutOfBounds {
                index: 5,
                length: 3
            })
        );
        assert_eq!(
            jget!(a, -1),
            Err(JavaError::ArrayIndexOutOfBounds {
                index: -1,
                length: 3
            })
        );
        assert!(jset!(a, 0, 99).is_ok());
        assert_eq!(a[0], 99);
    }

    #[test]
    fn array_ref_helpers_report_bounds() {
        let values = [1i32, 2, 3];
        assert_eq!(java_array_ref(&values, 2), Ok(&3));
        assert_eq!(
            java_array_ref(&values, -1),
            Err(JavaError::ArrayIndexOutOfBounds {
                index: -1,
                length: 3
            })
        );
        // 2D helper checks the outer dimension before the inner one.
        let mut grid = vec![vec![0i32; 2], vec![0i32; 2]];
        *java_array_2d_mut(&mut grid, 1, 0).unwrap() = 9;
        assert_eq!(grid[1][0], 9);
        assert_eq!(
            java_array_2d_mut(&mut grid, 5, 0),
            Err(JavaError::ArrayIndexOutOfBounds {
                index: 5,
                length: 2
            })
        );
    }

    #[test]
    fn byte_arrays_are_signed() {
        // The convention: `byte[]` is `Vec<i8>`. A stored 0xFF reads back as -1,
        // exactly as `baload` sign-extends into an `int`.
        let bytes: Vec<i8> = vec![-1, 0, 127];
        assert_eq!(java_array_ref(&bytes, 0), Ok(&-1i8));
        assert_eq!(bytes[0] as i32 & 0xff, 255);
    }
}
