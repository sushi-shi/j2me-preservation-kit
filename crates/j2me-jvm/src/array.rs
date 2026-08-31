//! Same-component-type `java.lang.System.arraycopy` operations.
//!
//! Java permits both distinct-array copies and overlapping copies within one
//! array. Rust's borrow rules make that identity decision explicit, so the two
//! cases have separate entry points. Both retain Java's null and bounds cuts;
//! reference-component assignability remains host-owned and is deliberately
//! outside this statically typed helper.

use std::ops::Range;

use crate::{i32_add, JavaError, JavaResult};

fn copy_range(array_length: i32, position: i32, length: i32) -> JavaResult<Range<usize>> {
    if position < 0 || length < 0 || position > array_length || length > array_length - position {
        return Err(JavaError::ArrayIndexOutOfBounds {
            index: i32_add(position, length),
            length: array_length,
        });
    }
    let end = position + length;
    Ok(position as usize..end as usize)
}

/// `System.arraycopy(source, sourcePosition, destination,
/// destinationPosition, length)` for two distinct arrays with the same Rust
/// component type.
///
/// Both null checks precede range validation; the source range is then checked
/// before the destination range. No destination element changes unless both
/// complete ranges are valid.
pub fn arraycopy<T: Clone>(
    source: Option<&[T]>,
    source_position: i32,
    destination: Option<&mut [T]>,
    destination_position: i32,
    length: i32,
) -> JavaResult<()> {
    let source = source.ok_or(JavaError::NullPointer)?;
    let destination = destination.ok_or(JavaError::NullPointer)?;
    let source_range = copy_range(source.len() as i32, source_position, length)?;
    let destination_range = copy_range(destination.len() as i32, destination_position, length)?;
    destination[destination_range].clone_from_slice(&source[source_range]);
    Ok(())
}

/// The same `System.arraycopy` operation when source and destination are the
/// identical Java array. A temporary preserves Java's memmove-style overlap
/// semantics for every `Clone` component representation.
pub fn arraycopy_within<T: Clone>(
    array: Option<&mut [T]>,
    source_position: i32,
    destination_position: i32,
    length: i32,
) -> JavaResult<()> {
    let array = array.ok_or(JavaError::NullPointer)?;
    let source_range = copy_range(array.len() as i32, source_position, length)?;
    let destination_range = copy_range(array.len() as i32, destination_position, length)?;
    let source_copy = array[source_range].to_vec();
    array[destination_range].clone_from_slice(&source_copy);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinct_copy_validates_both_ranges_before_mutation() {
        let source = [10, 20, 30, 40];
        let mut destination = [1, 2, 3, 4, 5];
        arraycopy(Some(&source), 1, Some(&mut destination), 2, 2).unwrap();
        assert_eq!(destination, [1, 2, 20, 30, 5]);

        let before = destination;
        assert_eq!(
            arraycopy(Some(&source), 0, Some(&mut destination), 4, 2),
            Err(JavaError::ArrayIndexOutOfBounds {
                index: 6,
                length: 5,
            })
        );
        assert_eq!(destination, before);
    }

    #[test]
    fn null_and_negative_inputs_retain_java_failures() {
        let source = [1_i8, 2];
        let mut destination = [0_i8; 2];
        assert_eq!(
            arraycopy::<i8>(None, 0, Some(&mut destination), 0, 0),
            Err(JavaError::NullPointer)
        );
        assert_eq!(
            arraycopy(Some(&source), 0, None, 0, 0),
            Err(JavaError::NullPointer)
        );
        assert_eq!(
            arraycopy(Some(&source), 0, Some(&mut destination), 0, -1),
            Err(JavaError::ArrayIndexOutOfBounds {
                index: -1,
                length: 2,
            })
        );
    }

    #[test]
    fn zero_length_accepts_the_one_past_end_positions() {
        let source = [1_i16, 2];
        let mut destination = [3_i16, 4];
        arraycopy(Some(&source), 2, Some(&mut destination), 2, 0).unwrap();
        assert_eq!(destination, [3, 4]);
    }

    #[test]
    fn overlapping_identity_copy_uses_java_memmove_semantics() {
        let mut right = [1, 2, 3, 4, 5];
        arraycopy_within(Some(&mut right), 0, 1, 4).unwrap();
        assert_eq!(right, [1, 1, 2, 3, 4]);

        let mut left = [1, 2, 3, 4, 5];
        arraycopy_within(Some(&mut left), 1, 0, 4).unwrap();
        assert_eq!(left, [2, 3, 4, 5, 5]);
    }
}
