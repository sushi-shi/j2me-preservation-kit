//! Game-neutral ARGB image storage shared by the MIDP runtime and host tools.
//!
//! Encoded PNG/JPEG handling is deliberately outside this crate. A host or
//! game-specific resource adapter decodes bytes and calls [`Image::from_argb`].

pub type Argb = u32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageError {
    InvalidDimensions,
    PixelCountOverflow,
    PixelCountMismatch { expected: usize, actual: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    width: i32,
    height: i32,
    pixels: Vec<Argb>,
    mutable: bool,
}

impl Image {
    pub fn create_mutable(width: i32, height: i32) -> Result<Self, ImageError> {
        let length = pixel_count(width, height)?;
        Ok(Self {
            width,
            height,
            pixels: vec![0xffff_ffff; length],
            mutable: true,
        })
    }

    pub fn from_argb(width: i32, height: i32, pixels: Vec<Argb>) -> Result<Self, ImageError> {
        let expected = pixel_count(width, height)?;
        if pixels.len() != expected {
            return Err(ImageError::PixelCountMismatch {
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            pixels,
            mutable: false,
        })
    }

    pub const fn width(&self) -> i32 {
        self.width
    }

    pub const fn height(&self) -> i32 {
        self.height
    }

    pub const fn is_mutable(&self) -> bool {
        self.mutable
    }

    pub fn pixels(&self) -> &[Argb] {
        &self.pixels
    }

    pub fn pixels_mut(&mut self) -> &mut [Argb] {
        &mut self.pixels
    }

    pub fn get(&self, x: i32, y: i32) -> Option<Argb> {
        self.index(x, y).map(|index| self.pixels[index])
    }

    #[inline]
    pub fn set(&mut self, x: i32, y: i32, value: Argb) {
        if let Some(index) = self.index(x, y) {
            self.pixels[index] = value;
        }
    }

    #[inline]
    pub fn blend(&mut self, x: i32, y: i32, source: Argb) {
        if let Some(index) = self.index(x, y) {
            self.pixels[index] = source_over(source, self.pixels[index]);
        }
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            None
        } else {
            Some(y as usize * self.width as usize + x as usize)
        }
    }
}

fn pixel_count(width: i32, height: i32) -> Result<usize, ImageError> {
    if width <= 0 || height <= 0 {
        return Err(ImageError::InvalidDimensions);
    }
    (width as usize)
        .checked_mul(height as usize)
        .ok_or(ImageError::PixelCountOverflow)
}

/// Straight-alpha source-over compositing for packed `0xAARRGGBB` pixels.
#[inline]
pub fn source_over(source: Argb, destination: Argb) -> Argb {
    let source_alpha = (source >> 24) & 0xff;
    if source_alpha == 0xff {
        return source;
    }
    if source_alpha == 0 {
        return destination;
    }

    let destination_alpha = (destination >> 24) & 0xff;
    let inverse = 0xff - source_alpha;
    let output_alpha = source_alpha + (destination_alpha * inverse + 127) / 255;
    if output_alpha == 0 {
        return 0;
    }

    let channel = |shift: u32| {
        let source_channel = (source >> shift) & 0xff;
        let destination_channel = (destination >> shift) & 0xff;
        let premultiplied = source_channel * source_alpha
            + (destination_channel * destination_alpha * inverse + 127) / 255;
        (premultiplied + output_alpha / 2) / output_alpha
    };

    (output_alpha << 24) | (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimensions_and_pixel_counts_are_bounded() {
        assert_eq!(
            Image::create_mutable(0, 1),
            Err(ImageError::InvalidDimensions)
        );
        assert_eq!(
            Image::from_argb(2, 2, vec![0; 3]),
            Err(ImageError::PixelCountMismatch {
                expected: 4,
                actual: 3,
            })
        );
    }

    #[test]
    fn writes_clip_and_alpha_blends() {
        let mut image = Image::create_mutable(2, 2).unwrap();
        image.set(-1, 0, 0xff00_0000);
        assert_eq!(image.get(0, 0), Some(0xffff_ffff));
        image.set(0, 0, 0xff00_00ff);
        image.blend(0, 0, 0x80ff_0000);
        let output = image.get(0, 0).unwrap();
        assert_eq!(output >> 24, 0xff);
        assert!((output >> 16) & 0xff >= 0x7f);
        assert!(output & 0xff >= 0x7f);
    }

    #[test]
    fn transparent_destination_keeps_straight_source_color() {
        assert_eq!(source_over(0x80ff_0000, 0), 0x80ff_0000);
    }
}
