//! Integer-only 2D canvas placement and host pixel projection.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanvasPlacement {
    /// Integer magnification. Cropped canvases always use scale 1.
    pub scale: u32,
    /// Logical source rectangle sampled from the Java canvas.
    pub source: Rect,
    /// Physical output rectangle centered in the viewport.
    pub destination: Rect,
}

impl CanvasPlacement {
    pub fn centered(
        logical_width: u32,
        logical_height: u32,
        viewport_width: u32,
        viewport_height: u32,
    ) -> Result<Self, PresentationError> {
        if logical_width == 0 || logical_height == 0 || viewport_width == 0 || viewport_height == 0
        {
            return Err(PresentationError::ZeroDimension);
        }
        if viewport_width >= logical_width && viewport_height >= logical_height {
            let scale = (viewport_width / logical_width)
                .min(viewport_height / logical_height)
                .max(1);
            let width = logical_width * scale;
            let height = logical_height * scale;
            Ok(Self {
                scale,
                source: Rect {
                    x: 0,
                    y: 0,
                    width: logical_width,
                    height: logical_height,
                },
                destination: Rect {
                    x: (viewport_width - width) / 2,
                    y: (viewport_height - height) / 2,
                    width,
                    height,
                },
            })
        } else {
            let width = logical_width.min(viewport_width);
            let height = logical_height.min(viewport_height);
            Ok(Self {
                scale: 1,
                source: Rect {
                    x: (logical_width - width) / 2,
                    y: (logical_height - height) / 2,
                    width,
                    height,
                },
                destination: Rect {
                    x: (viewport_width - width) / 2,
                    y: (viewport_height - height) / 2,
                    width,
                    height,
                },
            })
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresentationError {
    ZeroDimension,
    PixelCountMismatch { expected: usize, actual: usize },
    CropOutsideImage,
    SizeOverflow,
}

impl std::fmt::Display for PresentationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "presentation error: {self:?}")
    }
}

impl std::error::Error for PresentationError {}

/// Crop a Java ARGB canvas and convert it to byte-packed RGBA for host APIs.
pub fn argb_to_rgba_cropped(
    pixels: &[u32],
    image_width: u32,
    image_height: u32,
    crop: Rect,
) -> Result<Vec<u8>, PresentationError> {
    let expected = usize::try_from(image_width)
        .ok()
        .and_then(|width| {
            usize::try_from(image_height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .ok_or(PresentationError::SizeOverflow)?;
    if pixels.len() != expected {
        return Err(PresentationError::PixelCountMismatch {
            expected,
            actual: pixels.len(),
        });
    }
    let end_x = crop
        .x
        .checked_add(crop.width)
        .ok_or(PresentationError::SizeOverflow)?;
    let end_y = crop
        .y
        .checked_add(crop.height)
        .ok_or(PresentationError::SizeOverflow)?;
    if crop.width == 0 || crop.height == 0 || end_x > image_width || end_y > image_height {
        return Err(PresentationError::CropOutsideImage);
    }
    let output_len = usize::try_from(crop.width)
        .ok()
        .and_then(|width| {
            usize::try_from(crop.height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(PresentationError::SizeOverflow)?;
    let mut output = Vec::with_capacity(output_len);
    for y in crop.y..end_y {
        for x in crop.x..end_x {
            let index = (y as usize) * (image_width as usize) + (x as usize);
            let argb = pixels[index];
            output.extend_from_slice(&[
                ((argb >> 16) & 0xff) as u8,
                ((argb >> 8) & 0xff) as u8,
                (argb & 0xff) as u8,
                ((argb >> 24) & 0xff) as u8,
            ]);
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_is_integer_centered_or_center_cropped() {
        let scaled = CanvasPlacement::centered(240, 320, 1000, 800).unwrap();
        assert_eq!(scaled.scale, 2);
        assert_eq!(
            scaled.destination,
            Rect {
                x: 260,
                y: 80,
                width: 480,
                height: 640
            }
        );
        assert_eq!(
            scaled.source,
            Rect {
                x: 0,
                y: 0,
                width: 240,
                height: 320
            }
        );

        let cropped = CanvasPlacement::centered(240, 320, 200, 300).unwrap();
        assert_eq!(cropped.scale, 1);
        assert_eq!(
            cropped.source,
            Rect {
                x: 20,
                y: 10,
                width: 200,
                height: 300
            }
        );
        assert_eq!(
            cropped.destination,
            Rect {
                x: 0,
                y: 0,
                width: 200,
                height: 300
            }
        );
    }

    #[test]
    fn argb_crop_converts_channel_order() {
        let rgba = argb_to_rgba_cropped(
            &[0xff010203, 0x80405060, 0x00112233, 0xffffffff],
            2,
            2,
            Rect {
                x: 1,
                y: 0,
                width: 1,
                height: 2,
            },
        )
        .unwrap();
        assert_eq!(rgba, vec![0x40, 0x50, 0x60, 0x80, 0xff, 0xff, 0xff, 0xff]);
    }
}
