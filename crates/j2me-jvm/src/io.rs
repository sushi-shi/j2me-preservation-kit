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
//! ## `&[u8]` borrow convention
//!
//! [`DataInputStream`] **borrows** the input as `&'a [u8]` and tracks a cursor,
//! rather than owning a buffer or wrapping a `Read`. That is stalker's
//! convention (a `ByteArrayInputStream` is already an in-memory slice, so the
//! reader is zero-copy and a failed read consumes nothing), and it is the one
//! adopted here. Reads that borrow from the input — `read_bytes`, `read_utf` —
//! return owned `Vec`s so the cursor can keep advancing.
//!
//! Distinct from `j2me-codec`'s `no_std` bounded `Reader`: this layer is `std`,
//! returns [`JavaError`]/[`JavaResult`] to model Java's `EOFException` /
//! `UTFDataFormatException`, and mirrors the full `DataInput`/`DataOutput`
//! method surface including `readUTF`. `readUTF` is seeded with the modified
//! UTF-8 decoder proven on the silent-hill port (`orphan-formats`), including
//! its accept-a-raw-`0` quirk.

use crate::{JavaError, JavaResult};

/// `java.io.DataInputStream` wrapping a `ByteArrayInputStream`.
#[derive(Debug, Clone)]
pub struct DataInputStream<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> DataInputStream<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }

    /// Bytes consumed so far. Useful for reporting where a parse failed.
    pub fn position(&self) -> usize {
        self.position
    }

    /// `available()`.
    pub fn available(&self) -> usize {
        self.data.len().saturating_sub(self.position)
    }

    pub fn is_empty(&self) -> bool {
        self.available() == 0
    }

    fn take(&mut self, count: usize) -> JavaResult<&'a [u8]> {
        let end = self
            .position
            .checked_add(count)
            .ok_or_else(|| JavaError::Io("stream position overflow".to_string()))?;
        if end > self.data.len() {
            return Err(JavaError::Io("EOFException".to_string()));
        }
        let slice = &self.data[self.position..end];
        self.position = end;
        Ok(slice)
    }

    /// `readByte()` -- signed.
    pub fn read_byte(&mut self) -> JavaResult<i8> {
        Ok(self.take(1)?[0] as i8)
    }

    /// `readUnsignedByte()`.
    pub fn read_unsigned_byte(&mut self) -> JavaResult<i32> {
        Ok(self.take(1)?[0] as i32)
    }

    /// `readBoolean()` -- any non-zero byte is true.
    pub fn read_boolean(&mut self) -> JavaResult<bool> {
        Ok(self.take(1)?[0] != 0)
    }

    /// `readShort()` -- signed, big-endian.
    pub fn read_short(&mut self) -> JavaResult<i16> {
        let bytes = self.take(2)?;
        Ok(i16::from_be_bytes([bytes[0], bytes[1]]))
    }

    /// `readUnsignedShort()`.
    pub fn read_unsigned_short(&mut self) -> JavaResult<i32> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]) as i32)
    }

    /// `readChar()` -- UTF-16 code unit.
    pub fn read_char(&mut self) -> JavaResult<u16> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    /// `readInt()` -- signed, big-endian.
    pub fn read_int(&mut self) -> JavaResult<i32> {
        let bytes = self.take(4)?;
        Ok(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// `readLong()` -- signed, big-endian.
    pub fn read_long(&mut self) -> JavaResult<i64> {
        let bytes = self.take(8)?;
        Ok(i64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
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
        let bytes = self.take(length)?;
        decode_modified_utf8(bytes)
    }

    /// `read(byte[])`: fills the buffer and returns how many bytes were read.
    ///
    /// Unlike `readFully`, a short read is not an error. A non-empty read at EOF
    /// returns `-1`; a zero-length read returns `0` even at EOF.
    pub fn read(&mut self, buffer: &mut [i8]) -> JavaResult<i32> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let count = buffer.len().min(self.available());
        if count == 0 {
            return Ok(-1);
        }
        let bytes = self.take(count)?;
        for (slot, byte) in buffer.iter_mut().zip(bytes) {
            *slot = *byte as i8;
        }
        Ok(count as i32)
    }

    /// `readFully(byte[])`: errors unless the buffer can be filled completely.
    pub fn read_fully(&mut self, buffer: &mut [i8]) -> JavaResult<()> {
        let bytes = self.take(buffer.len())?;
        for (slot, byte) in buffer.iter_mut().zip(bytes) {
            *slot = *byte as i8;
        }
        Ok(())
    }

    /// Reads `count` signed bytes.
    pub fn read_bytes(&mut self, count: usize) -> JavaResult<Vec<i8>> {
        Ok(self.take(count)?.iter().map(|byte| *byte as i8).collect())
    }

    /// `skip(n)`.
    pub fn skip(&mut self, count: usize) -> JavaResult<usize> {
        let skipped = count.min(self.available());
        self.position += skipped;
        Ok(skipped)
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
        // The failed read must not consume anything.
        assert_eq!(input.available(), 1);
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
        let mut buffer = [0i8; 4];
        assert!(input.read_fully(&mut buffer).is_err());
    }

    #[test]
    fn write_short_truncates_like_java() {
        let mut output = DataOutputStream::new();
        output.write_short(0x1_2345);
        assert_eq!(output.as_bytes(), &[0x23, 0x45]);
    }
}
