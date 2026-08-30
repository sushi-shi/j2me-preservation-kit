//! Device-selected system-font service boundary.
//!
//! MIDP font metrics and glyphs varied by handset. The runtime therefore owns
//! only Java-facing descriptors and validation; a game supplies a reviewed
//! provider whose id is named by its device profile.

use j2me_jvm::JavaError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FontSpec {
    pub face: i32,
    pub style: i32,
    pub size: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FontMetrics {
    pub height: i32,
    pub baseline: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlyphBitmap {
    pub width: u32,
    pub height: u32,
    pub advance: i32,
    pub bearing_x: i32,
    pub bearing_y: i32,
    /// Row-major coverage values (0 transparent, 255 fully covered).
    pub alpha: Vec<u8>,
}

pub trait FontProvider {
    fn provider_id(&self) -> &str;
    fn metrics(&self, font: FontSpec) -> Result<FontMetrics, JavaError>;
    fn glyph(&self, font: FontSpec, character: char) -> Result<GlyphBitmap, JavaError>;
}

#[derive(Debug)]
pub struct FontRuntime<P> {
    provider: P,
}

impl<P: FontProvider> FontRuntime<P> {
    pub fn for_profile(
        provider: P,
        profile: &j2me_device::FontFragment,
    ) -> Result<Self, JavaError> {
        if provider.provider_id() != profile.provider {
            return Err(JavaError::IllegalState(
                "font provider does not match selected device profile",
            ));
        }
        Ok(Self { provider })
    }

    pub fn metrics(&self, font: FontSpec) -> Result<FontMetrics, JavaError> {
        self.provider.metrics(font)
    }

    pub fn glyph(&self, font: FontSpec, character: char) -> Result<GlyphBitmap, JavaError> {
        let glyph = self.provider.glyph(font, character)?;
        if glyph.alpha.len() != (glyph.width as usize).saturating_mul(glyph.height as usize) {
            return Err(JavaError::IllegalState(
                "font provider returned an invalid glyph bitmap",
            ));
        }
        Ok(glyph)
    }

    pub fn char_width(&self, font: FontSpec, character: char) -> Result<i32, JavaError> {
        Ok(self.glyph(font, character)?.advance)
    }

    pub fn string_width(&self, font: FontSpec, text: &str) -> Result<i32, JavaError> {
        text.chars().try_fold(0_i32, |width, character| {
            Ok(width.saturating_add(self.char_width(font, character)?))
        })
    }

    pub fn into_provider(self) -> P {
        self.provider
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture;

    impl FontProvider for Fixture {
        fn provider_id(&self) -> &str {
            "fixture-v1"
        }
        fn metrics(&self, _: FontSpec) -> Result<FontMetrics, JavaError> {
            Ok(FontMetrics {
                height: 9,
                baseline: 7,
            })
        }
        fn glyph(&self, _: FontSpec, _: char) -> Result<GlyphBitmap, JavaError> {
            Ok(GlyphBitmap {
                width: 1,
                height: 1,
                advance: 2,
                bearing_x: 0,
                bearing_y: 1,
                alpha: vec![255],
            })
        }
    }

    #[test]
    fn profile_must_name_the_exact_reviewed_provider() {
        let profile = j2me_device::FontFragment {
            provider: "fixture-v1".to_owned(),
        };
        let runtime = FontRuntime::for_profile(Fixture, &profile).unwrap();
        let spec = FontSpec {
            face: 0,
            style: 0,
            size: 8,
        };
        assert_eq!(runtime.metrics(spec).unwrap().baseline, 7);
        assert_eq!(runtime.glyph(spec, 'A').unwrap().alpha, vec![255]);
    }
}
