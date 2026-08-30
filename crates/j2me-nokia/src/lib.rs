//! `com.nokia.mid.ui.DirectGraphics` / `DirectUtils` — the Nokia UI vendor
//! blitter, as an OPT-IN extension over [`j2me_me`]. Games that render through a
//! Nokia `FullCanvas` with `drawPixels` (rather than `Graphics.drawImage`) depend
//! on this crate; games that use only standard MIDP do not.
//!
//! It wraps a *live* [`j2me_me::Graphics`] via [`get_direct_graphics`], sharing
//! that `Graphics`' current translate + clip through its
//! [`plot`](j2me_me::Graphics::plot) / [`read`](j2me_me::Graphics::read)
//! primitives — so a `setClip` on the `Graphics` after `getDirectGraphics` is
//! honored, exactly as on a handset.
//!
//! **The pixel-format conversions are the contract.** 4444→8888 replicates each
//! 4-bit nibble into both halves of the 8-bit channel (`n * 0x11`, so `0xF →
//! 0xFF`); 8888→4444 truncates to the high nibble (`c >> 4`). This pair is exact
//! on a round trip (`0xNN >> 4 == n`), which the sprite-bake pattern (decode PNG →
//! mutable buffer → `getPixels` back as `short[]` 4444) relies on: quantization
//! happens once and the baked sprite then redraws bit-stably every frame. A
//! rounding read-back would break that stability.
//!
//! **Closed sets (R10).** Only the formats/manipulations implemented here are
//! accepted — pixel formats {`TYPE_USHORT_4444_ARGB`, `TYPE_INT_8888_ARGB`} and
//! manipulations {`0`, `FLIP_HORIZONTAL`}. The other Nokia formats (`444`, `565`,
//! `1555`, `888`) and manipulations (vertical flip, rotations) are rejected with a
//! typed error rather than silently misinterpreted; add them here when a game
//! needs them.
//!
//! Array indexing follows Java: `drawPixels`/`getPixels` are unguarded in the
//! typical caller, so an out-of-range access panics (the MIDlet would have died
//! of the `ArrayIndexOutOfBoundsException` too).

use j2me_canvas::Image;
use j2me_jvm::JavaError;
use j2me_me::graphics::anchor_top_left;
use j2me_me::Graphics;

/// `DirectGraphics.TYPE_USHORT_4444_ARGB` — 16-bit `short[]` pixels.
pub const TYPE_USHORT_4444_ARGB: i32 = 4444;
/// `DirectGraphics.TYPE_INT_8888_ARGB` — 32-bit `int[]` pixels.
pub const TYPE_INT_8888_ARGB: i32 = 8888;
/// `DirectGraphics.FLIP_HORIZONTAL` — mirror left↔right.
pub const FLIP_HORIZONTAL: i32 = 8192;

/// The pixel formats this crate implements (closed set, R10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Ushort4444Argb,
    Int8888Argb,
}

impl PixelFormat {
    /// Resolve a raw Nokia format constant, rejecting everything outside the
    /// implemented set so a misread format can never be silently misinterpreted.
    pub fn from_raw(value: i32) -> Result<Self, JavaError> {
        match value {
            TYPE_USHORT_4444_ARGB => Ok(Self::Ushort4444Argb),
            TYPE_INT_8888_ARGB => Ok(Self::Int8888Argb),
            _ => Err(JavaError::IllegalArgument(
                "DirectGraphics pixel format not implemented (only 4444 and 8888)",
            )),
        }
    }
}

/// The manipulations this crate implements (closed set, R10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Manipulation {
    None,
    FlipHorizontal,
}

impl Manipulation {
    /// Resolve the raw manipulation argument; rotations and vertical flips are
    /// not implemented and are rejected.
    pub fn from_raw(value: i32) -> Result<Self, JavaError> {
        match value {
            0 => Ok(Self::None),
            FLIP_HORIZONTAL => Ok(Self::FlipHorizontal),
            _ => Err(JavaError::IllegalArgument(
                "DirectGraphics manipulation not implemented (only 0 and FLIP_HORIZONTAL)",
            )),
        }
    }

    fn flips_horizontally(self) -> bool {
        self == Self::FlipHorizontal
    }
}

/// `DirectUtils.getDirectGraphics(Graphics)` — the Nokia view over an existing
/// [`Graphics`], sharing its live translate, clip, and target.
pub fn get_direct_graphics<'g, 'a>(g: &'g mut Graphics<'a>) -> DirectGraphics<'g, 'a> {
    DirectGraphics { g }
}

/// The Nokia `DirectGraphics` interface over one wrapped [`Graphics`].
pub struct DirectGraphics<'g, 'a> {
    g: &'g mut Graphics<'a>,
}

impl DirectGraphics<'_, '_> {
    /// `drawPixels(short[] pixels, boolean transparency, int offset, int
    /// scanlength, int x, int y, int width, int height, int manipulation, int
    /// format)` — the ARGB4444 blit. `transparency = true` source-over composites
    /// the expanded pixel; `false` ignores the source alpha and draws opaque.
    /// Source pixel `(col, row)` is read at `offset + row * scanlength + col`;
    /// `FLIP_HORIZONTAL` reads the columns from the far end while the destination
    /// box stays put.
    #[allow(clippy::too_many_arguments)] // faithful to the 10-arg Nokia signature
    pub fn draw_pixels_4444(
        &mut self,
        pixels: &[i16],
        transparency: bool,
        offset: i32,
        scanlength: i32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        manipulation: i32,
        format: i32,
    ) -> Result<(), JavaError> {
        if PixelFormat::from_raw(format)? != PixelFormat::Ushort4444Argb {
            return Err(JavaError::IllegalArgument(
                "drawPixels(short[]): format must be TYPE_USHORT_4444_ARGB",
            ));
        }
        let flip_h = Manipulation::from_raw(manipulation)?.flips_horizontally();
        for row in 0..height {
            for col in 0..width {
                let s_col = if flip_h { width - 1 - col } else { col };
                let idx = offset + row * scanlength + s_col;
                let argb = argb4444_to_argb8888(pixels[idx as usize]);
                self.blit(x + col, y + row, argb, transparency);
            }
        }
        Ok(())
    }

    /// `drawPixels(int[] pixels, …)` — the ARGB8888 blit (full-screen fade /
    /// darkness overlays). Pixels are the raw `0xAARRGGBB` bit pattern.
    #[allow(clippy::too_many_arguments)] // faithful to the 10-arg Nokia signature
    pub fn draw_pixels_8888(
        &mut self,
        pixels: &[i32],
        transparency: bool,
        offset: i32,
        scanlength: i32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        manipulation: i32,
        format: i32,
    ) -> Result<(), JavaError> {
        if PixelFormat::from_raw(format)? != PixelFormat::Int8888Argb {
            return Err(JavaError::IllegalArgument(
                "drawPixels(int[]): format must be TYPE_INT_8888_ARGB",
            ));
        }
        let flip_h = Manipulation::from_raw(manipulation)?.flips_horizontally();
        for row in 0..height {
            for col in 0..width {
                let s_col = if flip_h { width - 1 - col } else { col };
                let idx = offset + row * scanlength + s_col;
                let argb = pixels[idx as usize] as u32;
                self.blit(x + col, y + row, argb, transparency);
            }
        }
        Ok(())
    }

    /// `getPixels(short[] pixels, int offset, int scanlength, int x, int y, int
    /// width, int height, int format)` — read a target region back as ARGB4444
    /// (truncating each channel to its high nibble). Reading outside the target
    /// panics (unguarded in the typical caller).
    #[allow(clippy::too_many_arguments)] // faithful to the 8-arg Nokia signature
    pub fn get_pixels_4444(
        &mut self,
        pixels: &mut [i16],
        offset: i32,
        scanlength: i32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        format: i32,
    ) -> Result<(), JavaError> {
        if PixelFormat::from_raw(format)? != PixelFormat::Ushort4444Argb {
            return Err(JavaError::IllegalArgument(
                "getPixels(short[]): format must be TYPE_USHORT_4444_ARGB",
            ));
        }
        for row in 0..height {
            for col in 0..width {
                let argb = self
                    .g
                    .read(x + col, y + row)
                    .expect("getPixels read outside the Graphics target (unguarded AIOOBE)");
                let idx = offset + row * scanlength + col;
                pixels[idx as usize] = argb8888_to_argb4444(argb);
            }
        }
        Ok(())
    }

    /// `getPixels(int[] pixels, …)` — read back as ARGB8888.
    #[allow(clippy::too_many_arguments)] // faithful to the 8-arg Nokia signature
    pub fn get_pixels_8888(
        &mut self,
        pixels: &mut [i32],
        offset: i32,
        scanlength: i32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        format: i32,
    ) -> Result<(), JavaError> {
        if PixelFormat::from_raw(format)? != PixelFormat::Int8888Argb {
            return Err(JavaError::IllegalArgument(
                "getPixels(int[]): format must be TYPE_INT_8888_ARGB",
            ));
        }
        for row in 0..height {
            for col in 0..width {
                let argb = self
                    .g
                    .read(x + col, y + row)
                    .expect("getPixels read outside the Graphics target (unguarded AIOOBE)");
                let idx = offset + row * scanlength + col;
                pixels[idx as usize] = argb as i32;
            }
        }
        Ok(())
    }

    /// `DirectGraphics.drawImage(Image img, int x, int y, int anchor, int
    /// manipulation)` — the Nokia anchored image draw. A horizontal flip leaves
    /// the `w×h` bounding box unchanged, so the anchor resolves exactly as
    /// `Graphics.drawImage`; only the source column order reverses.
    pub fn draw_image(
        &mut self,
        img: &Image,
        x: i32,
        y: i32,
        anchor: i32,
        manipulation: i32,
    ) -> Result<(), JavaError> {
        let flip_h = Manipulation::from_raw(manipulation)?.flips_horizontally();
        let (tlx, tly) = anchor_top_left(x, y, img.width(), img.height(), anchor)
            .map_err(|_| JavaError::IllegalArgument("DirectGraphics.drawImage: invalid anchor"))?;
        let (w, h) = (img.width(), img.height());
        for row in 0..h {
            for col in 0..w {
                let s_col = if flip_h { w - 1 - col } else { col };
                if let Some(px) = img.get(s_col, row) {
                    self.g.plot(tlx + col, tly + row, px, true);
                }
            }
        }
        Ok(())
    }

    /// One pixel through the wrapped `Graphics`' translate + clip. The Nokia
    /// `transparency` flag decides the mode: `true` alpha-composites; `false`
    /// ignores the source alpha and writes the pixel opaque.
    #[inline]
    fn blit(&mut self, x: i32, y: i32, argb: u32, transparency: bool) {
        if transparency {
            self.g.plot(x, y, argb, true);
        } else {
            self.g.plot(x, y, argb | 0xFF00_0000, false);
        }
    }
}

/// Expand one ARGB4444 pixel (Java `short` bit pattern) to `0xAARRGGBB`: each
/// nibble `n` becomes `n * 0x11` (both halves replicated, so `0xF → 0xFF`).
#[inline]
pub fn argb4444_to_argb8888(p: i16) -> u32 {
    let p = p as u16 as u32;
    let a = (p >> 12) & 0xF;
    let r = (p >> 8) & 0xF;
    let g = (p >> 4) & 0xF;
    let b = p & 0xF;
    ((a * 0x11) << 24) | ((r * 0x11) << 16) | ((g * 0x11) << 8) | (b * 0x11)
}

/// Quantize one `0xAARRGGBB` pixel to ARGB4444 (Java `short` bit pattern): each
/// channel truncates to its high nibble (`c >> 4`). Exact inverse of
/// [`argb4444_to_argb8888`] on its range — the bake round trip is bit-stable.
#[inline]
pub fn argb8888_to_argb4444(p: u32) -> i16 {
    let a = (p >> 28) & 0xF;
    let r = (p >> 20) & 0xF;
    let g = (p >> 12) & 0xF;
    let b = (p >> 4) & 0xF;
    (((a << 12) | (r << 8) | (g << 4) | b) as u16) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: u32 = 0xFFFF_FFFF;

    #[test]
    fn argb4444_expansion_replicates_nibbles_not_shifts() {
        // A=0xF R=0x0 G=0x8 B=0xA → 0xFF, 0x00, 0x88, 0xAA.
        let p = 0xF08Au16 as i16;
        let correct = argb4444_to_argb8888(p);
        assert_eq!(correct, 0xFF00_88AA);
        assert_eq!(argb4444_to_argb8888(0xFFFFu16 as i16), 0xFFFF_FFFF);
        assert_eq!(argb4444_to_argb8888(0x0000), 0x0000_0000);

        // R3 control: the classic WRONG expansion (shift-only, low half zero)
        // produces a different value — this test discriminates against it.
        let wrong = {
            let q = 0xF08Au32;
            ((((q >> 12) & 0xF) << 4) << 24)
                | ((((q >> 8) & 0xF) << 4) << 16)
                | ((((q >> 4) & 0xF) << 4) << 8)
                | ((q & 0xF) << 4)
        };
        assert_eq!(wrong, 0xF000_80A0);
        assert_ne!(wrong, correct);
    }

    #[test]
    fn argb4444_round_trips_exactly_through_8888() {
        for p in [0x0000u16, 0xFFFF, 0x1234, 0xF08A, 0x8421, 0x0F0F] {
            let p = p as i16;
            assert_eq!(argb8888_to_argb4444(argb4444_to_argb8888(p)), p);
        }
    }

    #[test]
    fn argb8888_quantizes_by_truncating_to_the_high_nibble() {
        // 0x12345678: high nibbles A=1 R=3 G=5 B=7. A rounding conversion would
        // carry 0x78 → 0x8 (R3 control on the B channel).
        assert_eq!(argb8888_to_argb4444(0x1234_5678), 0x1357);
        assert_eq!(argb8888_to_argb4444(0xFFFF_FFFF), 0xFFFFu16 as i16);
    }

    #[test]
    fn draw_pixels_4444_places_converts_and_respects_transparency() {
        let mut img = Image::create_mutable(4, 4).unwrap(); // opaque white
        {
            let mut g = Graphics::new(&mut img);
            let mut dg = get_direct_graphics(&mut g);
            // 2x1: opaque red (A=F R=F G=0 B=0), fully transparent.
            let px: [i16; 2] = [0xFF00u16 as i16, 0x0000];
            dg.draw_pixels_4444(&px, true, 0, 2, 1, 1, 2, 1, 0, 4444)
                .unwrap();
        }
        assert_eq!(img.get(1, 1), Some(0xFFFF_0000)); // expanded opaque red
        assert_eq!(img.get(2, 1), Some(WHITE)); // transparent pixel skipped

        // transparency = false: the alpha data is IGNORED — the transparent
        // pixel draws opaque (R3 control against always-blending).
        let mut img2 = Image::create_mutable(4, 4).unwrap();
        {
            let mut g = Graphics::new(&mut img2);
            let mut dg = get_direct_graphics(&mut g);
            let px: [i16; 2] = [0xFF00u16 as i16, 0x0000];
            dg.draw_pixels_4444(&px, false, 0, 2, 1, 1, 2, 1, 0, 4444)
                .unwrap();
        }
        assert_eq!(img2.get(2, 1), Some(0xFF00_0000)); // opaque black, not skipped
    }

    #[test]
    fn flip_horizontal_mirrors_the_source_columns() {
        // 3x1 asymmetric sprite: red, green, blue.
        let px: [i16; 3] = [0xFF00u16 as i16, 0xF0F0u16 as i16, 0xF00Fu16 as i16];

        // Identity control first: red lands on the left.
        let mut plain = Image::create_mutable(5, 3).unwrap();
        {
            let mut g = Graphics::new(&mut plain);
            let mut dg = get_direct_graphics(&mut g);
            dg.draw_pixels_4444(&px, true, 0, 3, 1, 1, 3, 1, 0, 4444)
                .unwrap();
        }
        assert_eq!(plain.get(1, 1), Some(0xFFFF_0000)); // red left
        assert_eq!(plain.get(3, 1), Some(0xFF00_00FF)); // blue right

        // FLIP_HORIZONTAL: same destination box, columns read from the far end.
        let mut flipped = Image::create_mutable(5, 3).unwrap();
        {
            let mut g = Graphics::new(&mut flipped);
            let mut dg = get_direct_graphics(&mut g);
            dg.draw_pixels_4444(&px, true, 0, 3, 1, 1, 3, 1, FLIP_HORIZONTAL, 4444)
                .unwrap();
        }
        assert_eq!(flipped.get(1, 1), Some(0xFF00_00FF)); // blue left
        assert_eq!(flipped.get(2, 1), Some(0xFF00_FF00)); // green center
        assert_eq!(flipped.get(3, 1), Some(0xFFFF_0000)); // red right
    }

    #[test]
    fn draw_pixels_is_clipped_by_the_wrapped_graphics() {
        // The wrapped Graphics' clip binds the Nokia blit too.
        let mut img = Image::create_mutable(6, 2).unwrap();
        {
            let mut g = Graphics::new(&mut img);
            g.set_clip(0, 0, 3, 2);
            let mut dg = get_direct_graphics(&mut g);
            let px: [i16; 6] = [0xFF00u16 as i16; 6];
            dg.draw_pixels_4444(&px, true, 0, 6, 0, 0, 6, 1, 0, 4444)
                .unwrap();
        }
        assert_eq!(img.get(2, 0), Some(0xFFFF_0000)); // inside the clip
        assert_eq!(img.get(3, 0), Some(WHITE)); // suppressed outside
    }

    #[test]
    fn draw_pixels_8888_overlays_blend_over_the_target() {
        // The fade path: a half-alpha black overlay darkens an opaque red target
        // instead of replacing it.
        let mut img = Image::create_mutable(2, 1).unwrap();
        {
            let mut g = Graphics::new(&mut img);
            g.set_color(0x00FF_0000);
            g.fill_rect(0, 0, 2, 1);
            let mut dg = get_direct_graphics(&mut g);
            let px: [i32; 1] = [0x8000_0000u32 as i32]; // 50% black
            dg.draw_pixels_8888(&px, true, 0, 1, 0, 0, 1, 1, 0, 8888)
                .unwrap();
        }
        let darkened = img.get(0, 0).unwrap();
        assert_eq!(darkened >> 24, 0xFF); // still opaque
        let red = (darkened >> 16) & 0xFF;
        assert!(red < 0xFF && red > 0x40, "red was darkened, not replaced");
        assert_eq!(img.get(1, 0), Some(0xFFFF_0000)); // untouched control
    }

    #[test]
    fn get_pixels_4444_reads_back_what_the_bake_path_drew() {
        // The sprite bake: pixels drawn into a mutable buffer come back as 4444
        // shorts; the colors are 0x11-multiples, so quantization is lossless.
        let src: [i16; 4] = [
            0xF00Fu16 as i16, // opaque blue
            0xFF00u16 as i16, // opaque red
            0x0000,           // transparent
            0xF0F0u16 as i16, // opaque green
        ];
        let mut buffer = Image::create_mutable(2, 2).unwrap();
        {
            let mut g = Graphics::new(&mut buffer);
            let mut dg = get_direct_graphics(&mut g);
            dg.draw_pixels_4444(&src, false, 0, 2, 0, 0, 2, 2, 0, 4444)
                .unwrap();
        }
        let mut baked = [0i16; 4];
        {
            let mut g = Graphics::new(&mut buffer);
            let mut dg = get_direct_graphics(&mut g);
            dg.get_pixels_4444(&mut baked, 0, 2, 0, 0, 2, 2, 4444)
                .unwrap();
        }
        // transparency=false drew every pixel opaque, so the read-back has alpha
        // F everywhere; the RGB nibbles round-tripped exactly.
        assert_eq!(baked[0], 0xF00Fu16 as i16);
        assert_eq!(baked[1], 0xFF00u16 as i16);
        assert_eq!(baked[2], 0xF000u16 as i16); // transparent drew as opaque black
        assert_eq!(baked[3], 0xF0F0u16 as i16);
    }

    #[test]
    fn get_pixels_8888_reads_the_raw_argb() {
        let mut img = Image::create_mutable(1, 1).unwrap();
        {
            let mut g = Graphics::new(&mut img);
            g.set_color(0x0012_3456);
            g.fill_rect(0, 0, 1, 1);
            let mut dg = get_direct_graphics(&mut g);
            let mut out = [0i32; 1];
            dg.get_pixels_8888(&mut out, 0, 1, 0, 0, 1, 1, 8888)
                .unwrap();
            assert_eq!(out[0] as u32, 0xFF12_3456);
        }
    }

    #[test]
    fn formats_and_manipulations_outside_the_closed_sets_are_rejected() {
        // R10: the Nokia API defines 444/565/1555/888 and vertical flips /
        // rotations, but they are not implemented here — they must error.
        assert!(PixelFormat::from_raw(565).is_err());
        assert!(PixelFormat::from_raw(444).is_err());
        assert!(PixelFormat::from_raw(1555).is_err());
        assert!(PixelFormat::from_raw(888).is_err());
        assert_eq!(PixelFormat::from_raw(4444), Ok(PixelFormat::Ushort4444Argb));
        assert_eq!(PixelFormat::from_raw(8888), Ok(PixelFormat::Int8888Argb));

        assert!(Manipulation::from_raw(0x4000).is_err()); // FLIP_VERTICAL
        assert!(Manipulation::from_raw(90).is_err()); // ROTATE_90
        assert!(Manipulation::from_raw(FLIP_HORIZONTAL | 90).is_err());
        assert_eq!(Manipulation::from_raw(0), Ok(Manipulation::None));
        assert_eq!(
            Manipulation::from_raw(8192),
            Ok(Manipulation::FlipHorizontal)
        );

        // A drawPixels with a wrong format is refused before touching pixels.
        let mut img = Image::create_mutable(2, 2).unwrap();
        let mut g = Graphics::new(&mut img);
        let mut dg = get_direct_graphics(&mut g);
        let px: [i16; 1] = [0];
        assert!(dg
            .draw_pixels_4444(&px, true, 0, 1, 0, 0, 1, 1, 0, 565)
            .is_err());
        // And the short[] overload refuses the int[] format (cross-check).
        assert!(dg
            .draw_pixels_4444(&px, true, 0, 1, 0, 0, 1, 1, 0, 8888)
            .is_err());
    }

    #[test]
    fn direct_draw_image_flips_and_anchors() {
        // 2x1 image: red then blue.
        let src = Image::from_argb(2, 1, vec![0xFFFF_0000, 0xFF00_00FF]).unwrap();
        let mut img = Image::create_mutable(4, 2).unwrap();
        {
            let mut g = Graphics::new(&mut img);
            let mut dg = get_direct_graphics(&mut g);
            dg.draw_image(&src, 1, 0, 20, FLIP_HORIZONTAL).unwrap();
        }
        // TOP|LEFT at (1,0), mirrored: blue first.
        assert_eq!(img.get(1, 0), Some(0xFF00_00FF));
        assert_eq!(img.get(2, 0), Some(0xFFFF_0000));

        // Identity control: without the flip, red stays first.
        let mut plain = Image::create_mutable(4, 2).unwrap();
        {
            let mut g = Graphics::new(&mut plain);
            let mut dg = get_direct_graphics(&mut g);
            dg.draw_image(&src, 1, 0, 20, 0).unwrap();
        }
        assert_eq!(plain.get(1, 0), Some(0xFFFF_0000));
        assert_eq!(plain.get(2, 0), Some(0xFF00_00FF));
    }
}
