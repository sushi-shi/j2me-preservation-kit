#![no_std]
//! Bounded, allocation-free primitives for game-specific wire decoders.
//!
//! This is the layer that deliberately remains `no_std`. Filesystem access,
//! archive traversal, image/audio decoding, the device runtime, and the strict
//! game transliteration do not inherit that constraint.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeError {
    UnexpectedEof { offset: usize, needed: usize },
    LengthOverflow,
    TrailingData { offset: usize, remaining: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    pub const fn position(&self) -> usize {
        self.offset
    }

    pub const fn remaining(&self) -> usize {
        self.bytes.len() - self.offset
    }

    pub fn read_exact(&mut self, length: usize) -> Result<&'a [u8], DecodeError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(DecodeError::LengthOverflow)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(DecodeError::UnexpectedEof {
                offset: self.offset,
                needed: length,
            })?;
        self.offset = end;
        Ok(value)
    }

    pub fn read_u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.read_exact(1)?[0])
    }

    pub fn read_i8(&mut self) -> Result<i8, DecodeError> {
        Ok(self.read_u8()? as i8)
    }

    pub fn read_u16_be(&mut self) -> Result<u16, DecodeError> {
        let bytes = self.read_exact(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    pub fn read_i16_be(&mut self) -> Result<i16, DecodeError> {
        Ok(self.read_u16_be()? as i16)
    }

    pub fn read_i32_be(&mut self) -> Result<i32, DecodeError> {
        let bytes = self.read_exact(4)?;
        Ok(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub fn read_u32_be(&mut self) -> Result<u32, DecodeError> {
        let bytes = self.read_exact(4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Little-endian `u16`. Some handset serializers hand-roll little-endian
    /// headers (e.g. `low + (high << 8)`) even though `DataInputStream` is
    /// big-endian, so both orders are needed across the corpus.
    pub fn read_u16_le(&mut self) -> Result<u16, DecodeError> {
        let bytes = self.read_exact(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    pub fn read_i16_le(&mut self) -> Result<i16, DecodeError> {
        Ok(self.read_u16_le()? as i16)
    }

    /// Advance the cursor by `count` bytes, failing (without moving) if fewer
    /// remain — useful for walking a directory-addressed archive by skipping
    /// prior entries.
    pub fn skip(&mut self, count: usize) -> Result<(), DecodeError> {
        self.read_exact(count).map(|_| ())
    }

    /// Move the cursor to an absolute `offset`, failing (without moving) if it
    /// is past the end of the input.
    pub fn seek(&mut self, offset: usize) -> Result<(), DecodeError> {
        if offset > self.bytes.len() {
            return Err(DecodeError::UnexpectedEof {
                offset: self.offset,
                needed: offset - self.bytes.len(),
            });
        }
        self.offset = offset;
        Ok(())
    }

    pub fn finish(self) -> Result<(), DecodeError> {
        if self.remaining() == 0 {
            Ok(())
        } else {
            Err(DecodeError::TrailingData {
                offset: self.offset,
                remaining: self.remaining(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_big_endian_values_and_rejects_truncation() {
        let mut reader = Reader::new(&[0x7f, 0x80, 0x12, 0x34, 0xaa]);
        assert_eq!(reader.read_i8(), Ok(127));
        assert_eq!(reader.read_i8(), Ok(-128));
        assert_eq!(reader.read_u16_be(), Ok(0x1234));
        assert_eq!(
            reader.finish(),
            Err(DecodeError::TrailingData {
                offset: 4,
                remaining: 1
            })
        );

        let mut short = Reader::new(&[1]);
        assert_eq!(
            short.read_u16_be(),
            Err(DecodeError::UnexpectedEof {
                offset: 0,
                needed: 2
            })
        );
        assert_eq!(short.position(), 0);
    }

    #[test]
    fn little_endian_reads_and_cursor_navigation() {
        let mut reader = Reader::new(&[0x34, 0x12, 0x78, 0x56, 0x00, 0x00, 0x00, 0x2a]);
        assert_eq!(reader.read_u16_le(), Ok(0x1234));
        assert_eq!(reader.read_i16_le(), Ok(0x5678));
        assert_eq!(reader.read_u32_be(), Ok(0x0000_002a));
        assert_eq!(reader.finish(), Ok(()));

        let mut nav = Reader::new(&[1, 2, 3, 4, 5]);
        assert_eq!(nav.skip(2), Ok(()));
        assert_eq!(nav.position(), 2);
        assert_eq!(nav.read_u8(), Ok(3));
        assert_eq!(nav.seek(1), Ok(()));
        assert_eq!(nav.read_u8(), Ok(2));
        // A seek past the end fails and leaves the cursor unmoved.
        assert_eq!(
            nav.seek(6),
            Err(DecodeError::UnexpectedEof {
                offset: 2,
                needed: 1
            })
        );
        assert_eq!(nav.position(), 2);
    }
}
