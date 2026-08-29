//! `javax.microedition.lcdui.Image` factory paths that turn ENCODED bytes into
//! pixels: `createImage(byte[], int, int)` and `createImage(String)`.
//!
//! The neutral [`j2me_canvas::Image`] owns the ARGB buffer and stays codec-free,
//! so the mutable/from-pixels constructors (`createImage(w, h)` =
//! [`j2me_canvas::Image::create_mutable`], `from_argb`) live there and are
//! re-exported by [`crate`]. The PNG *decode* belongs to the device runtime: the
//! game reads a packed byte buffer (a sprite atlas, a merged PNG, a font sheet)
//! and expects the MIDP contract — an immutable image, or an exception on
//! malformed data.
//!
//! Two decode factories are modeled:
//! - `Image.createImage(byte[] data, int off, int len)` — the sprite / merged-PNG
//!   / bitmap-font loaders ([`create_image_region`]);
//! - `Image.createImage(String name)` — a resource loaded by name
//!   ([`create_image_named`], resolved through the host [`ImageResources`]
//!   seam; a missing resource is an `IOException`, matching the real API).
//!
//! Decoding folds paletted 1/2/4/8-bit PNGs (some with a `tRNS` chunk) into
//! straight 8-bit channels, packed `0xAARRGGBB` (`EXPAND | STRIP_16`).
//! Malformed or unsupported input returns a typed [`JavaError`] — never a panic
//! (rulebook R10): MIDP throws `IllegalArgumentException` when the bytes are not a
//! decodable image, and `ArrayIndexOutOfBoundsException` when the region bounds
//! are invalid.

use j2me_canvas::Image;
use j2me_jvm::JavaError;
use png::{ColorType, Transformations};

/// The host resource seam behind `Image.createImage(String name)`. A real MIDP
/// runtime resolves the name against the JAR classpath (`getResourceAsStream`);
/// here the host supplies the bytes for a name, or `None` when absent (the
/// `IOException` case). The bytes are the game's Java `byte[]` (signed octets).
pub trait ImageResources {
    /// Resolve a resource name (e.g. `"/en/font.png"`) to its raw bytes, or
    /// `None` if no such resource exists.
    fn load(&self, name: &str) -> Option<Vec<i8>>;
}

/// `Image.createImage(byte[] imageData, int imageOffset, int imageLength)` — the
/// overload the sprite / merged-PNG / bitmap-font loaders use (e.g.
/// `Image.createImage(png, 0, pngLength)`): decode the `[offset, offset+length)`
/// slice of the buffer. Bounds are checked with Java's semantics — a negative
/// offset/length or a slice past the end throws `ArrayIndexOutOfBoundsException`
/// (MIDP), reported here as a typed error.
pub fn create_image_region(
    image_data: &[i8],
    image_offset: i32,
    image_length: i32,
) -> Result<Image, JavaError> {
    let len = image_data.len() as i64;
    let off = image_offset as i64;
    let n = image_length as i64;
    // MIDP: negative offset/length or offset+length past the array is an AIOOBE.
    if image_offset < 0 || image_length < 0 || off + n > len {
        return Err(JavaError::ArrayIndexOutOfBounds {
            index: if image_offset < 0 {
                image_offset
            } else {
                image_offset.wrapping_add(image_length)
            },
            length: image_data.len() as i32,
        });
    }
    let start = image_offset as usize;
    let end = start + image_length as usize;
    let bytes: Vec<u8> = image_data[start..end].iter().map(|&b| b as u8).collect();
    decode_png(&bytes)
}

/// `Image.createImage(String name)` — load a named resource (e.g. a localized
/// font sheet) and decode it. The resource is resolved through the host
/// [`ImageResources`] seam; an absent resource is an `IOException` (the real API
/// declares `throws IOException`), and undecodable bytes are an
/// `IllegalArgumentException` — never a panic.
pub fn create_image_named(name: &str, resources: &dyn ImageResources) -> Result<Image, JavaError> {
    let data = resources
        .load(name)
        .ok_or_else(|| JavaError::Io(format!("createImage: resource not found: {name}")))?;
    let bytes: Vec<u8> = data.iter().map(|&b| b as u8).collect();
    decode_png(&bytes)
}

/// Decode PNG bytes into an ARGB [`Image`], folding paletted/low-bit-depth/`tRNS`
/// input to 8-bit channels (`EXPAND | STRIP_16`). Any decode failure or
/// unsupported shape → `IllegalArgumentException` (the MIDP contract for
/// un-decodable image data), never a panic.
fn decode_png(bytes: &[u8]) -> Result<Image, JavaError> {
    let mut decoder = png::Decoder::new(bytes);
    decoder.set_transformations(Transformations::EXPAND | Transformations::STRIP_16);

    let mut reader = decoder
        .read_info()
        .map_err(|_| JavaError::IllegalArgument("createImage: not a decodable PNG (header)"))?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|_| JavaError::IllegalArgument("createImage: not a decodable PNG (frame)"))?;
    let data = &buf[..info.buffer_size()];

    let (w, h) = (info.width, info.height);
    if w == 0 || h == 0 {
        return Err(JavaError::IllegalArgument(
            "createImage: degenerate 0-size image",
        ));
    }
    // MIDP `Image` addresses pixels with `i32`; refuse anything that would not fit
    // rather than wrapping into a negative dimension.
    let px = (w as usize)
        .checked_mul(h as usize)
        .filter(|&n| n <= i32::MAX as usize && w <= i32::MAX as u32 && h <= i32::MAX as u32)
        .ok_or(JavaError::IllegalArgument(
            "createImage: image too large for i32",
        ))?;

    let mut argb = Vec::with_capacity(px);
    match info.color_type {
        ColorType::Rgba => {
            for c in data.chunks_exact(4) {
                argb.push(pack(c[3], c[0], c[1], c[2]));
            }
        }
        ColorType::Rgb => {
            for c in data.chunks_exact(3) {
                argb.push(pack(0xFF, c[0], c[1], c[2]));
            }
        }
        ColorType::GrayscaleAlpha => {
            for c in data.chunks_exact(2) {
                argb.push(pack(c[1], c[0], c[0], c[0]));
            }
        }
        ColorType::Grayscale => {
            for &g in data {
                argb.push(pack(0xFF, g, g, g));
            }
        }
        // EXPAND removes the paletted type; refuse rather than emit wrong pixels.
        ColorType::Indexed => {
            return Err(JavaError::IllegalArgument(
                "createImage: paletted PNG not expanded",
            ))
        }
    }

    if argb.len() != px {
        return Err(JavaError::IllegalArgument(
            "createImage: decoded pixel count mismatch",
        ));
    }
    // `w`/`h` fit `i32` and `argb.len() == w*h`, so the neutral buffer's fallible
    // `from_argb` cannot actually reject here; map any error to the MIDP
    // `IllegalArgumentException` contract rather than unwrapping (R10).
    Image::from_argb(w as i32, h as i32, argb)
        .map_err(|_| JavaError::IllegalArgument("createImage: pixel buffer construction failed"))
}

/// Pack straight (non-premultiplied) 8-bit channels into `0xAARRGGBB`.
#[inline]
fn pack(a: u8, r: u8, g: u8, b: u8) -> u32 {
    ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode a tiny PNG in-test (authored bytes, never game data). `png`'s own
    /// encoder guarantees a decodable blob.
    fn rgba_png(w: u32, h: u32, rgba: &[u8]) -> Vec<i8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut writer = enc.write_header().unwrap();
            writer.write_image_data(rgba).unwrap();
        }
        out.into_iter().map(|b| b as i8).collect()
    }

    #[test]
    fn create_image_region_decodes_rgba_into_argb() {
        // 2x1: opaque red, half-alpha green.
        let png = rgba_png(2, 1, &[255, 0, 0, 255, 0, 255, 0, 128]);
        let img = create_image_region(&png, 0, png.len() as i32).unwrap();
        assert_eq!((img.width(), img.height()), (2, 1));
        assert!(!img.is_mutable(), "a decoded image is immutable");
        assert_eq!(img.get(0, 0), Some(0xFFFF_0000));
        assert_eq!(img.get(1, 0), Some(0x8000_FF00));
    }

    #[test]
    fn create_image_decodes_paletted_with_trns_via_expand() {
        // A common real-world shape: an indexed PNG with a tRNS chunk. EXPAND
        // must turn it into RGBA before packing.
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, 2, 2);
            enc.set_color(ColorType::Indexed);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_palette(vec![255, 0, 0, 0, 0, 255]); // idx0 red, idx1 blue
            enc.set_trns(vec![0x00, 0xFF]); // idx0 transparent, idx1 opaque
            let mut writer = enc.write_header().unwrap();
            writer.write_image_data(&[0, 1, 1, 0]).unwrap();
        }
        let bytes: Vec<i8> = out.into_iter().map(|b| b as i8).collect();
        let img = create_image_region(&bytes, 0, bytes.len() as i32).unwrap();
        assert_eq!((img.width(), img.height()), (2, 2));
        assert_eq!(img.get(0, 0), Some(0x00FF_0000)); // transparent red
        assert_eq!(img.get(1, 0), Some(0xFF00_00FF)); // opaque blue
    }

    #[test]
    fn create_image_rejects_non_png_without_panicking() {
        // R10: malformed input is a typed IllegalArgumentException, never a panic.
        let bytes: Vec<i8> = b"definitely not a png".iter().map(|&b| b as i8).collect();
        assert_eq!(
            create_image_region(&bytes, 0, bytes.len() as i32),
            Err(JavaError::IllegalArgument(
                "createImage: not a decodable PNG (header)"
            ))
        );
    }

    #[test]
    fn create_image_region_decodes_an_embedded_slice() {
        // The sprite loader passes a buffer whose PNG is a prefix of a longer
        // allocation: createImage(buf, 0, realLength) must decode only [0, len).
        let png = rgba_png(1, 1, &[10, 20, 30, 255]);
        let real_len = png.len() as i32;
        let mut padded = png.clone();
        padded.extend(std::iter::repeat_n(0i8, 37)); // trailing garbage
        let img = create_image_region(&padded, 0, real_len).unwrap();
        assert_eq!((img.width(), img.height()), (1, 1));
        assert_eq!(img.get(0, 0), Some(0xFF0A_141E));
    }

    #[test]
    fn create_image_region_offset_slice_matches_whole() {
        // A non-zero offset: the PNG sits after a header the loader skips.
        let png = rgba_png(1, 1, &[9, 8, 7, 255]);
        let mut buf: Vec<i8> = vec![-1, -2, -3, -4]; // 4-byte lead-in
        let off = buf.len() as i32;
        let n = png.len() as i32;
        buf.extend_from_slice(&png);
        let img = create_image_region(&buf, off, n).unwrap();
        assert_eq!(img.get(0, 0), Some(0xFF09_0807));
    }

    #[test]
    fn create_image_region_rejects_out_of_range_bounds() {
        // R10: invalid region bounds are AIOOBE, never a slice panic. Controls:
        // negative offset, negative length, and a length past the end.
        let png = rgba_png(1, 1, &[0, 0, 0, 255]);
        assert!(matches!(
            create_image_region(&png, -1, 4),
            Err(JavaError::ArrayIndexOutOfBounds { .. })
        ));
        assert!(matches!(
            create_image_region(&png, 0, -1),
            Err(JavaError::ArrayIndexOutOfBounds { .. })
        ));
        assert!(matches!(
            create_image_region(&png, 2, png.len() as i32),
            Err(JavaError::ArrayIndexOutOfBounds { .. })
        ));
    }

    #[test]
    fn create_image_named_decodes_present_and_errors_on_missing() {
        // The host resource seam: a present name decodes, an absent name is an
        // IOException (R10 — never a panic), proving the seam actually gates.
        struct Res(Vec<i8>);
        impl ImageResources for Res {
            fn load(&self, name: &str) -> Option<Vec<i8>> {
                (name == "/en/font.png").then(|| self.0.clone())
            }
        }
        let png = rgba_png(1, 1, &[1, 2, 3, 255]);
        let res = Res(png);
        let img = create_image_named("/en/font.png", &res).unwrap();
        assert_eq!(img.get(0, 0), Some(0xFF01_0203));
        assert!(matches!(
            create_image_named("/de/missing.png", &res),
            Err(JavaError::Io(_))
        ));
    }
}
