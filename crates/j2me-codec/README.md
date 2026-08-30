# j2me-codec

`no_std`, allocation-free bounded readers for the ad-hoc serialization formats
recovered from J2ME (Java ME / MIDP) game builds.

`Reader` walks a `&[u8]` with big-endian, Java-`DataInput`-shaped primitive
reads (`read_u8`, `read_i8`, `read_u16_be`, `read_exact`, …). Every read is
bounds- and overflow-checked and returns a typed [`DecodeError`] instead of
panicking, so a truncated or malformed asset fails cleanly rather than reading
out of bounds.

## Where it sits

This is the one deliberately `no_std` layer of the J2ME preservation stack.
Filesystem access, archive traversal, image/audio decoding, the device runtime
(`j2me-me`), and the strict game transliteration do **not** inherit that
constraint — they build their game-specific wire decoders on top of `Reader`.
It has no dependencies on the other crates.

## Usage

```rust
use j2me_codec::{Reader, DecodeError};

let mut r = Reader::new(&[0x00, 0x2a, 0xff]);
assert_eq!(r.read_u16_be(), Ok(42));
assert_eq!(r.read_i8(), Ok(-1));
assert_eq!(r.remaining(), 0);
assert!(matches!(r.read_u8(), Err(DecodeError::UnexpectedEof { .. })));
```

An optional `std` feature is reserved for future host-side conveniences; the
default build is `no_std`.

## License

Dedicated to the public domain under **CC0 1.0 Universal**. See the workspace
`LICENSE`.
