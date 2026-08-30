//! `java.io.DataInputStream` / `DataOutputStream`, big-endian and signed.
//!
//! This is the JVM-tier stream codec a strict transliteration reads and writes
//! every resource and RMS save record through. It reproduces `DataInputStream`
//! over a `ByteArrayInputStream` and `DataOutputStream` over a
//! `ByteArrayOutputStream`: big-endian throughout (matching the JVM), and
//! signed where Java is signed — `readByte` returns `i8`, and games routinely
//! compare glyph and sprite bytes against negative sentinels, so the signedness
//! is behavior, not cosmetics.
//!
//! ## Owned input-stream state
//!
//! [`ByteArrayInputStream`] owns the Java stream's exact `buf` / `pos` / `mark`
//! / `count` state. [`DataInputStream`] owns that stream and delegates to its
//! one cursor; it does not maintain a second position. This matters at EOF:
//! Java primitive reads consume the available prefix before throwing
//! `EOFException`, and a ranged byte-array stream may end before `buf.len()`.
//! `DataInputStream::new` remains a convenient slice-copying constructor, while
//! [`DataInputStream::from_stream`] and [`DataInputStream::into_inner`] preserve
//! an already-owned stream and make its final cursor state observable.
//!
//! Distinct from `j2me-codec`'s `no_std` bounded `Reader`: this layer is `std`,
//! returns [`JavaError`]/[`JavaResult`] to model Java's `EOFException` /
//! `UTFDataFormatException`, and mirrors the full `DataInput`/`DataOutput`
//! method surface including `readUTF`. `readUTF` is seeded with the modified
//! UTF-8 decoder proven on the silent-hill port (`orphan-formats`), including
//! its accept-a-raw-`0` quirk.
//!
//! The output stream owns a `ByteArrayOutputStream`-compatible memory sink.
//! Closing that exact pairing flushes and closes a sink whose `close()` is a
//! no-op: accumulated bytes and earlier `toByteArray()` copies remain valid,
//! and the memory sink remains writable just as Java's byte-array stream does.

use crate::{JavaError, JavaResult};

fn eof() -> JavaError {
    JavaError::Io("EOFException".to_string())
}

/// An owned `java.io.ByteArrayInputStream`.
///
/// The Java class retains the supplied array without copying it and carries
/// four fields: `buf`, `pos`, `mark`, and `count`. The positions stay as `i32`
/// rather than `usize` because the ranged Java constructor performs no bounds
/// validation and computes `offset + length` with wrapping JVM `int`
/// arithmetic. Ordinary callers create valid states, but preserving malformed
/// states lets hostile differential tests observe the same later failures.
#[derive(Debug, Clone)]
pub struct ByteArrayInputStream {
    buf: Vec<u8>,
    pos: i32,
    mark: i32,
    count: i32,
}

impl ByteArrayInputStream {
    /// `new ByteArrayInputStream(buf)`; takes ownership without copying.
    pub fn new(buf: Vec<u8>) -> Self {
        let count = i32::try_from(buf.len()).expect("Java byte[] length exceeds i32::MAX");
        Self {
            buf,
            pos: 0,
            mark: 0,
            count,
        }
    }

    /// Slice-copying convenience constructor for host-provided bytes.
    pub fn from_slice(buf: &[u8]) -> Self {
        Self::new(buf.to_vec())
    }

    /// `new ByteArrayInputStream(buf, offset, length)`.
    ///
    /// Java deliberately does not validate `offset` or `length` here. `pos`
    /// and `mark` become `offset`, while `count` is
    /// `min(offset + length, buf.length)` after wrapping `int` addition.
    pub fn new_range(buf: Vec<u8>, offset: i32, length: i32) -> Self {
        let buf_length = i32::try_from(buf.len()).expect("Java byte[] length exceeds i32::MAX");
        Self {
            buf,
            pos: offset,
            mark: offset,
            count: offset.wrapping_add(length).min(buf_length),
        }
    }

    pub fn buffer(&self) -> &[u8] {
        &self.buf
    }

    pub fn position(&self) -> i32 {
        self.pos
    }

    pub fn mark_position(&self) -> i32 {
        self.mark
    }

    pub fn count(&self) -> i32 {
        self.count
    }

    /// `InputStream.read()` -- `0..=255`, or `-1` at EOF.
    pub fn read(&mut self) -> JavaResult<i32> {
        if self.pos >= self.count {
            return Ok(-1);
        }
        // `buf[pos++]` increments `pos` before the JVM array-bounds check.
        let index = self.pos;
        self.pos = self.pos.wrapping_add(1);
        let byte = usize::try_from(index)
            .ok()
            .and_then(|index| self.buf.get(index))
            .ok_or(JavaError::ArrayIndexOutOfBounds {
                index,
                length: self.buf.len() as i32,
            })?;
        Ok(i32::from(*byte))
    }

    /// `InputStream.read(byte[], int, int)`.
    ///
    /// Destination bounds are checked before EOF. The Java implementation then
    /// checks EOF before the zero-length special case, so a zero-length read at
    /// EOF returns `-1`, while the same read before EOF returns `0`.
    pub fn read_range(&mut self, buffer: &mut [i8], offset: i32, length: i32) -> JavaResult<i32> {
        let buffer_length =
            i32::try_from(buffer.len()).expect("Java byte[] length exceeds i32::MAX");
        if offset < 0 || length < 0 || length > buffer_length.wrapping_sub(offset) {
            return Err(JavaError::IndexOutOfBounds);
        }
        if self.pos >= self.count {
            return Ok(-1);
        }
        let available = self.count.wrapping_sub(self.pos);
        let copied = length.min(available);
        if copied <= 0 {
            return Ok(0);
        }

        let source_offset = usize::try_from(self.pos).map_err(|_| JavaError::IndexOutOfBounds)?;
        let destination_offset =
            usize::try_from(offset).map_err(|_| JavaError::IndexOutOfBounds)?;
        let copied = usize::try_from(copied).map_err(|_| JavaError::IndexOutOfBounds)?;
        let source_end = source_offset
            .checked_add(copied)
            .ok_or(JavaError::IndexOutOfBounds)?;
        let destination_end = destination_offset
            .checked_add(copied)
            .ok_or(JavaError::IndexOutOfBounds)?;
        let source = self
            .buf
            .get(source_offset..source_end)
            .ok_or(JavaError::IndexOutOfBounds)?;
        let destination = buffer
            .get_mut(destination_offset..destination_end)
            .ok_or(JavaError::IndexOutOfBounds)?;
        for (slot, byte) in destination.iter_mut().zip(source) {
            *slot = *byte as i8;
        }
        self.pos = self.pos.wrapping_add(copied as i32);
        Ok(copied as i32)
    }

    /// `InputStream.skip(long)` with Java's exact `int` field arithmetic.
    pub fn skip(&mut self, count: i64) -> i64 {
        let mut skipped = i64::from(self.count.wrapping_sub(self.pos));
        if count < skipped {
            skipped = if count < 0 { 0 } else { count };
        }
        self.pos = i64::from(self.pos).wrapping_add(skipped) as i32;
        skipped
    }

    /// `available()` -- the raw wrapping JVM `count - pos` expression.
    pub fn available(&self) -> i32 {
        self.count.wrapping_sub(self.pos)
    }

    pub fn is_empty(&self) -> bool {
        self.pos >= self.count
    }

    /// `mark(readAheadLimit)`; the limit is ignored by this Java class.
    pub fn mark(&mut self, _read_ahead_limit: i32) {
        self.mark = self.pos;
    }

    /// `reset()`.
    pub fn reset(&mut self) {
        self.pos = self.mark;
    }

    pub const fn mark_supported(&self) -> bool {
        true
    }

    /// `close()`; intentionally leaves every field untouched.
    pub fn close(&mut self) {}

    pub fn into_buffer(self) -> Vec<u8> {
        self.buf
    }
}

/// `java.io.DataInputStream` wrapping a `ByteArrayInputStream`.
#[derive(Debug, Clone)]
pub struct DataInputStream {
    input: ByteArrayInputStream,
}

impl DataInputStream {
    /// Convenience constructor that copies a host slice into an owned stream.
    pub fn new(data: &[u8]) -> Self {
        Self::from_stream(ByteArrayInputStream::from_slice(data))
    }

    /// Takes ownership of a byte vector without copying it.
    pub fn from_bytes(data: Vec<u8>) -> Self {
        Self::from_stream(ByteArrayInputStream::new(data))
    }

    /// Mirrors `new DataInputStream(input)` without introducing another cursor.
    pub fn from_stream(input: ByteArrayInputStream) -> Self {
        Self { input }
    }

    pub fn inner(&self) -> &ByteArrayInputStream {
        &self.input
    }

    pub fn inner_mut(&mut self) -> &mut ByteArrayInputStream {
        &mut self.input
    }

    pub fn into_inner(self) -> ByteArrayInputStream {
        self.input
    }

    /// Bytes consumed so far. Useful for reporting where a parse failed.
    pub fn position(&self) -> i32 {
        self.input.position()
    }

    /// `available()`.
    pub fn available(&self) -> i32 {
        self.input.available()
    }

    pub fn is_empty(&self) -> bool {
        self.input.is_empty()
    }

    fn read_required(&mut self) -> JavaResult<u8> {
        let value = self.input.read()?;
        if value < 0 {
            Err(eof())
        } else {
            Ok(value as u8)
        }
    }

    /// `readByte()` -- signed.
    pub fn read_byte(&mut self) -> JavaResult<i8> {
        Ok(self.read_required()? as i8)
    }

    /// `readUnsignedByte()`.
    pub fn read_unsigned_byte(&mut self) -> JavaResult<i32> {
        Ok(i32::from(self.read_required()?))
    }

    /// `readBoolean()` -- any non-zero byte is true.
    pub fn read_boolean(&mut self) -> JavaResult<bool> {
        Ok(self.read_required()? != 0)
    }

    /// `readShort()` -- signed, big-endian.
    pub fn read_short(&mut self) -> JavaResult<i16> {
        let first = self.read_required()?;
        let second = self.read_required()?;
        Ok(i16::from_be_bytes([first, second]))
    }

    /// `readUnsignedShort()`.
    pub fn read_unsigned_short(&mut self) -> JavaResult<i32> {
        let first = self.read_required()?;
        let second = self.read_required()?;
        Ok(i32::from(u16::from_be_bytes([first, second])))
    }

    /// `readChar()` -- UTF-16 code unit.
    pub fn read_char(&mut self) -> JavaResult<u16> {
        let first = self.read_required()?;
        let second = self.read_required()?;
        Ok(u16::from_be_bytes([first, second]))
    }

    /// `readInt()` -- signed, big-endian.
    pub fn read_int(&mut self) -> JavaResult<i32> {
        let bytes = [
            self.read_required()?,
            self.read_required()?,
            self.read_required()?,
            self.read_required()?,
        ];
        Ok(i32::from_be_bytes(bytes))
    }

    /// `readLong()` -- signed, big-endian.
    pub fn read_long(&mut self) -> JavaResult<i64> {
        let mut bytes = [0_i8; 8];
        self.read_fully(&mut bytes)?;
        Ok(i64::from_be_bytes(bytes.map(|byte| byte as u8)))
    }

    /// `readFloat()` -- IEEE 754 bits, big-endian.
    pub fn read_float(&mut self) -> JavaResult<f32> {
        Ok(f32::from_bits(self.read_int()? as u32))
    }

    /// `readDouble()`.
    pub fn read_double(&mut self) -> JavaResult<f64> {
        Ok(f64::from_bits(self.read_long()? as u64))
    }

    /// `readUTF()`.
    ///
    /// Reads a two-byte unsigned length, then that many bytes of *modified*
    /// UTF-8, returning the decoded UTF-16 code units.
    ///
    /// Returns `Vec<u16>` rather than a Rust `String` because modified UTF-8 is
    /// a UTF-16 codec: it can carry an embedded NUL and lone surrogates, neither
    /// of which a `String` can hold. The NUL appears two ways and both decode to
    /// `U+0000` here — the `C0 80` two-byte form `writeUTF` emits, and a raw
    /// `0x00` byte, which a real `DataInputStream.readUTF` also accepts even
    /// though `writeUTF` never produces it (the accept-raw-`0` quirk proven on
    /// the silent-hill corpus). Malformed input raises `UTFDataFormatException`.
    pub fn read_utf(&mut self) -> JavaResult<Vec<u16>> {
        let length = self.read_unsigned_short()? as usize;
        let mut bytes = vec![0_i8; length];
        self.read_fully(&mut bytes)?;
        let bytes: Vec<u8> = bytes.into_iter().map(|byte| byte as u8).collect();
        decode_modified_utf8(&bytes)
    }

    /// `read(byte[])`: fills the buffer and returns how many bytes were read.
    ///
    /// Unlike `readFully`, a short read is not an error. Because the underlying
    /// stream is a `ByteArrayInputStream`, even a zero-length read returns `-1`
    /// at EOF; before EOF the same zero-length read returns `0`.
    pub fn read(&mut self, buffer: &mut [i8]) -> JavaResult<i32> {
        let length = i32::try_from(buffer.len()).expect("Java byte[] length exceeds i32::MAX");
        self.input.read_range(buffer, 0, length)
    }

    /// `readFully(byte[])`; a short input consumes its available prefix before
    /// raising `EOFException`.
    pub fn read_fully(&mut self, buffer: &mut [i8]) -> JavaResult<()> {
        let length = i32::try_from(buffer.len()).expect("Java byte[] length exceeds i32::MAX");
        let mut consumed = 0_i32;
        while consumed < length {
            let read = self
                .input
                .read_range(buffer, consumed, length.wrapping_sub(consumed))?;
            if read < 0 {
                return Err(eof());
            }
            consumed = consumed.wrapping_add(read);
        }
        Ok(())
    }

    /// Reads `count` signed bytes.
    pub fn read_bytes(&mut self, count: usize) -> JavaResult<Vec<i8>> {
        let mut bytes = vec![0_i8; count];
        self.read_fully(&mut bytes)?;
        Ok(bytes)
    }

    /// `skip(n)`.
    pub fn skip(&mut self, count: i64) -> JavaResult<i64> {
        Ok(self.input.skip(count))
    }
}

/// Decodes Java modified UTF-8 to UTF-16 code units.
///
/// The three widths follow `DataInputStream.readUTF`: a leading byte with the
/// high bit clear (`0x00..=0x7F`, which includes the raw `0`) is one code unit;
/// `0xC0..=0xDF` is two bytes; `0xE0..=0xEF` is three. Any other leading byte,
/// a truncated sequence, or a continuation byte without the `10` prefix raises
/// `UTFDataFormatException`.
fn decode_modified_utf8(bytes: &[u8]) -> JavaResult<Vec<u16>> {
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut offset = 0usize;
    while offset < bytes.len() {
        let first = bytes[offset];
        match first >> 4 {
            // 0x00..=0x7F: single byte. Covers the raw-0 quirk.
            0..=7 => {
                decoded.push(u16::from(first));
                offset += 1;
            }
            // 0xC0..=0xDF: two bytes.
            12 | 13 => {
                if offset + 1 >= bytes.len() {
                    return Err(utf_data_format());
                }
                let second = bytes[offset + 1];
                if second & 0xc0 != 0x80 {
                    return Err(utf_data_format());
                }
                decoded.push((u16::from(first & 0x1f) << 6) | u16::from(second & 0x3f));
                offset += 2;
            }
            // 0xE0..=0xEF: three bytes.
            14 => {
                if offset + 2 >= bytes.len() {
                    return Err(utf_data_format());
                }
                let second = bytes[offset + 1];
                let third = bytes[offset + 2];
                if second & 0xc0 != 0x80 || third & 0xc0 != 0x80 {
                    return Err(utf_data_format());
                }
                decoded.push(
                    (u16::from(first & 0x0f) << 12)
                        | (u16::from(second & 0x3f) << 6)
                        | u16::from(third & 0x3f),
                );
                offset += 3;
            }
            _ => return Err(utf_data_format()),
        }
    }
    Ok(decoded)
}

fn utf_data_format() -> JavaError {
    JavaError::Io("UTFDataFormatException".to_string())
}

/// The number of bytes the modified UTF-8 encoding of `chars` occupies (`0` as
/// `C0 80`, `0x0001..=0x007F` as one byte, `0x0080..=0x07FF` as two, the rest as
/// three), matching `DataOutputStream.writeUTF`'s length computation.
fn modified_utf8_len(chars: &[u16]) -> usize {
    chars
        .iter()
        .map(|&unit| match unit {
            0 => 2,
            0x0001..=0x007f => 1,
            0x0080..=0x07ff => 2,
            _ => 3,
        })
        .sum()
}

/// `java.io.DataOutputStream` over a `ByteArrayOutputStream`.
#[derive(Debug, Clone, Default)]
pub struct DataOutputStream {
    data: Vec<u8>,
}

impl DataOutputStream {
    pub fn new() -> Self {
        Self::default()
    }

    /// `size()`.
    pub fn size(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// `toByteArray()`.
    pub fn into_bytes(self) -> Vec<u8> {
        self.data
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// `close()` for this stream's owned `ByteArrayOutputStream`-compatible
    /// backing.
    ///
    /// `DataOutputStream.close()` delegates through `FilterOutputStream` to
    /// the backing stream after flushing it. `ByteArrayOutputStream.flush()`
    /// and `close()` are both no-ops, so the accumulated buffer is retained and
    /// remains writable. Any byte-array copy obtained before closing remains an
    /// independent snapshot.
    pub const fn close(&mut self) {}

    /// `writeByte(int)` -- writes the low eight bits.
    pub fn write_byte(&mut self, value: i32) {
        self.data.push(value as u8);
    }

    /// `writeBoolean(boolean)`.
    pub fn write_boolean(&mut self, value: bool) {
        self.data.push(u8::from(value));
    }

    /// `writeShort(int)` -- writes the low sixteen bits, big-endian.
    pub fn write_short(&mut self, value: i32) {
        self.data.extend_from_slice(&(value as i16).to_be_bytes());
    }

    /// `writeChar(int)`.
    pub fn write_char(&mut self, value: i32) {
        self.data.extend_from_slice(&(value as u16).to_be_bytes());
    }

    /// `writeInt(int)`.
    pub fn write_int(&mut self, value: i32) {
        self.data.extend_from_slice(&value.to_be_bytes());
    }

    /// `writeLong(long)`.
    pub fn write_long(&mut self, value: i64) {
        self.data.extend_from_slice(&value.to_be_bytes());
    }

    /// `writeFloat(float)`.
    pub fn write_float(&mut self, value: f32) {
        self.write_int(value.to_bits() as i32);
    }

    /// `writeDouble(double)`.
    pub fn write_double(&mut self, value: f64) {
        self.write_long(value.to_bits() as i64);
    }

    /// `writeUTF(String)`.
    ///
    /// Writes the two-byte unsigned modified-UTF-8 byte length followed by the
    /// encoding of the given UTF-16 code units, emitting `U+0000` as the `C0 80`
    /// two-byte form (never a raw `0`). Raises `UTFDataFormatException` if the
    /// encoded length exceeds the 65535 the length prefix can hold.
    pub fn write_utf(&mut self, chars: &[u16]) -> JavaResult<()> {
        let byte_len = modified_utf8_len(chars);
        if byte_len > 0xffff {
            return Err(JavaError::Io(
                "UTFDataFormatException: encoded string too long".to_string(),
            ));
        }
        self.write_short(byte_len as i32);
        for &unit in chars {
            match unit {
                0 => {
                    self.data.push(0xc0);
                    self.data.push(0x80);
                }
                0x0001..=0x007f => self.data.push(unit as u8),
                0x0080..=0x07ff => {
                    self.data.push(0xc0 | (unit >> 6) as u8);
                    self.data.push(0x80 | (unit & 0x3f) as u8);
                }
                _ => {
                    self.data.push(0xe0 | (unit >> 12) as u8);
                    self.data.push(0x80 | ((unit >> 6) & 0x3f) as u8);
                    self.data.push(0x80 | (unit & 0x3f) as u8);
                }
            }
        }
        Ok(())
    }

    /// `write(byte[])`.
    pub fn write(&mut self, buffer: &[i8]) {
        self.data.extend(buffer.iter().map(|byte| *byte as u8));
    }

    /// `write(byte[], off, len)`.
    pub fn write_range(&mut self, buffer: &[i8], offset: usize, length: usize) {
        self.data
            .extend(buffer[offset..offset + length].iter().map(|b| *b as u8));
    }

    pub fn write_raw(&mut self, buffer: &[u8]) {
        self.data.extend_from_slice(buffer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_big_endian_signed_values() {
        let bytes = [0xFF, 0xFF, 0x80, 0x00, 0x00, 0x00, 0x00, 0x01];
        let mut input = DataInputStream::new(&bytes);
        assert_eq!(input.read_short().unwrap(), -1);
        assert_eq!(input.read_short().unwrap(), -32768);
        assert_eq!(input.read_int().unwrap(), 1);
        assert!(input.is_empty());
    }

    #[test]
    fn signed_and_unsigned_byte_reads_differ() {
        let bytes = [0xFF, 0xFF];
        let mut input = DataInputStream::new(&bytes);
        assert_eq!(input.read_byte().unwrap(), -1);
        assert_eq!(input.read_unsigned_byte().unwrap(), 255);
    }

    #[test]
    fn reading_past_the_end_is_an_eof_not_a_panic() {
        let bytes = [0x01];
        let mut input = DataInputStream::new(&bytes);
        assert!(input.read_int().is_err());
        // DataInputStream consumes the available prefix before EOFException.
        assert_eq!(input.available(), 0);
    }

    #[test]
    fn int_and_long_round_trip_big_endian() {
        let mut output = DataOutputStream::new();
        output.write_int(-1_991_225_785);
        output.write_short(300);
        output.write_byte(-1);
        output.write_boolean(true);
        output.write_long(i64::MIN);
        output.write_float(0.846_153_86);
        output.write_double(-2.5);
        let bytes = output.into_bytes();

        let mut input = DataInputStream::new(&bytes);
        assert_eq!(input.read_int().unwrap(), -1_991_225_785);
        assert_eq!(input.read_short().unwrap(), 300);
        assert_eq!(input.read_byte().unwrap(), -1);
        assert!(input.read_boolean().unwrap());
        assert_eq!(input.read_long().unwrap(), i64::MIN);
        assert_eq!(input.read_float().unwrap(), 0.846_153_86);
        assert_eq!(input.read_double().unwrap(), -2.5);
        assert!(input.is_empty());
    }

    #[test]
    fn read_utf_decodes_modified_utf8_with_multibyte_chars() {
        // length 0x0006, then: 'A', 'ä' (U+00E4 = C3 A4), '€' (U+20AC = E2 82 AC).
        let bytes = [0x00, 0x06, 0x41, 0xC3, 0xA4, 0xE2, 0x82, 0xAC];
        let mut input = DataInputStream::new(&bytes);
        assert_eq!(input.read_utf().unwrap(), vec![0x0041, 0x00E4, 0x20AC]);
        assert!(input.is_empty());
    }

    #[test]
    fn read_utf_accepts_an_embedded_null_both_ways() {
        // The C0 80 two-byte NUL that writeUTF emits: "A\0B", length 4.
        let encoded = [0x00, 0x04, 0x41, 0xC0, 0x80, 0x42];
        let mut input = DataInputStream::new(&encoded);
        assert_eq!(input.read_utf().unwrap(), vec![0x0041, 0x0000, 0x0042]);

        // The raw-0 quirk: readUTF also accepts a literal 0x00 byte, length 3.
        let raw = [0x00, 0x03, 0x41, 0x00, 0x42];
        let mut input = DataInputStream::new(&raw);
        assert_eq!(input.read_utf().unwrap(), vec![0x0041, 0x0000, 0x0042]);
    }

    #[test]
    fn write_utf_then_read_utf_round_trips_including_the_null() {
        let value = [0x0041u16, 0x0000, 0x00E4, 0x20AC, 0x0042];
        let mut output = DataOutputStream::new();
        output.write_utf(&value).unwrap();
        // writeUTF emits the NUL as C0 80, so the byte length is 1+2+2+3+1 = 9.
        assert_eq!(output.as_bytes()[..2], [0x00, 0x09]);

        let bytes = output.into_bytes();
        let mut input = DataInputStream::new(&bytes);
        assert_eq!(input.read_utf().unwrap(), value.to_vec());
        assert!(input.is_empty());
    }

    #[test]
    fn closing_byte_array_output_preserves_buffer_copy_and_writability() {
        let mut output = DataOutputStream::new();
        output.write_int(0x1234_5678);

        // ByteArrayOutputStream.toByteArray() returns an independent snapshot.
        let snapshot = output.as_bytes().to_vec();
        output.close();
        assert_eq!(output.as_bytes(), snapshot);

        // Closing ByteArrayOutputStream has no effect; writes may continue, and
        // the earlier toByteArray snapshot cannot change with the backing sink.
        output.write_byte(0x9a);
        assert_eq!(snapshot, [0x12, 0x34, 0x56, 0x78]);
        assert_eq!(output.as_bytes(), [0x12, 0x34, 0x56, 0x78, 0x9a]);
    }

    #[test]
    fn read_utf_rejects_a_truncated_sequence() {
        // Claims length 2 but the two-byte lead 0xC3 has no continuation byte.
        let bytes = [0x00, 0x02, 0xC3, 0x41];
        let mut input = DataInputStream::new(&bytes);
        assert!(input.read_utf().is_err());
    }

    #[test]
    fn short_read_is_allowed_but_read_fully_is_not() {
        let bytes = [1u8, 2];
        let mut input = DataInputStream::new(&bytes);
        let mut buffer = [0i8; 4];
        assert_eq!(input.read(&mut buffer).unwrap(), 2);

        let mut input = DataInputStream::new(&bytes);
        let mut buffer = [-1i8; 4];
        assert!(input.read_fully(&mut buffer).is_err());
        assert_eq!(buffer, [1, 2, -1, -1]);
        assert_eq!(input.position(), 2);
    }

    #[test]
    fn byte_array_input_stream_retains_exact_full_and_ranged_state() {
        let bytes = vec![0, 1, 2, 3, 4];
        let pointer = bytes.as_ptr();
        let full = ByteArrayInputStream::new(bytes);
        assert_eq!(full.buffer().as_ptr(), pointer);
        assert_eq!(full.position(), 0);
        assert_eq!(full.mark_position(), 0);
        assert_eq!(full.count(), 5);
        assert_eq!(full.available(), 5);

        let mut ranged = ByteArrayInputStream::new_range(vec![0, 1, 2, 3, 4], 2, 2);
        assert_eq!(ranged.position(), 2);
        assert_eq!(ranged.mark_position(), 2);
        assert_eq!(ranged.count(), 4);
        assert_eq!(ranged.available(), 2);
        assert_eq!(ranged.read().unwrap(), 2);
        ranged.mark(123);
        assert_eq!(ranged.read().unwrap(), 3);
        assert_eq!(ranged.read().unwrap(), -1);
        ranged.close();
        ranged.reset();
        assert_eq!(ranged.position(), 3);
        assert_eq!(ranged.read().unwrap(), 3);
    }

    #[test]
    fn byte_array_input_stream_bulk_read_preserves_java_eof_and_bounds_order() {
        let mut before_eof = ByteArrayInputStream::new(vec![7]);
        let mut empty = [];
        assert_eq!(before_eof.read_range(&mut empty, 0, 0), Ok(0));

        let mut at_eof = ByteArrayInputStream::new(vec![]);
        assert_eq!(at_eof.read_range(&mut empty, 0, 0), Ok(-1));
        assert_eq!(
            at_eof.read_range(&mut empty, 1, 0),
            Err(JavaError::IndexOutOfBounds)
        );

        let mut stream = ByteArrayInputStream::new_range(vec![0, 0x80, 0xff, 4], 1, 2);
        let mut destination = [-7_i8; 5];
        assert_eq!(stream.read_range(&mut destination, 2, 3), Ok(2));
        assert_eq!(destination, [-7, -7, -128, -1, -7]);
        assert_eq!(stream.position(), 3);
        assert_eq!(stream.read_range(&mut destination, 0, 1), Ok(-1));
    }

    #[test]
    fn byte_array_input_stream_preserves_hostile_constructor_arithmetic() {
        let mut negative = ByteArrayInputStream::new_range(vec![10, 11, 12], -1, 2);
        assert_eq!(negative.position(), -1);
        assert_eq!(negative.mark_position(), -1);
        assert_eq!(negative.count(), 1);
        assert_eq!(negative.available(), 2);
        assert_eq!(
            negative.read(),
            Err(JavaError::ArrayIndexOutOfBounds {
                index: -1,
                length: 3,
            })
        );
        // `buf[pos++]` advances before the failing JVM bounds check.
        assert_eq!(negative.position(), 0);

        let mut past_end = ByteArrayInputStream::new_range(vec![1, 2, 3], 5, 0);
        assert_eq!(past_end.available(), -2);
        assert_eq!(past_end.read(), Ok(-1));
        assert_eq!(past_end.skip(1), -2);
        assert_eq!(past_end.position(), 3);
    }

    #[test]
    fn data_input_stream_owns_and_returns_the_same_cursor() {
        let bytes = vec![0x12, 0x34, 0x56];
        let pointer = bytes.as_ptr();
        let stream = ByteArrayInputStream::new(bytes);
        let mut input = DataInputStream::from_stream(stream);
        assert_eq!(input.inner().buffer().as_ptr(), pointer);
        assert_eq!(input.read_unsigned_short(), Ok(0x1234));
        let stream = input.into_inner();
        assert_eq!(stream.buffer().as_ptr(), pointer);
        assert_eq!(stream.position(), 2);
        assert_eq!(stream.available(), 1);
    }

    #[test]
    fn write_short_truncates_like_java() {
        let mut output = DataOutputStream::new();
        output.write_short(0x1_2345);
        assert_eq!(output.as_bytes(), &[0x23, 0x45]);
    }
}
