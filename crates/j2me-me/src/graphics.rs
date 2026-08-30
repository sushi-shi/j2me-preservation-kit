//! Deterministic MIDP Graphics operations over a neutral ARGB image:
//! `setColor`/`setFont`/clip/`translate`/`fillRect`/`drawRect`/`drawLine`/`fillTriangle`/
//! rounded rectangles/`drawImage`/`drawRegion` (with `GraphicsError` and
//! `SpriteTransform`), the MIDP `drawArc` / `fillArc` ellipse-sector rasteriser, and a public
//! [`anchor_top_left`] anchor resolver.

use crate::font::FontSpec;
use j2me_canvas::Image;

pub const HCENTER: i32 = 1;
pub const VCENTER: i32 = 2;
pub const LEFT: i32 = 4;
pub const RIGHT: i32 = 8;
pub const TOP: i32 = 16;
pub const BOTTOM: i32 = 32;
pub const BASELINE: i32 = 64;
pub const TOP_LEFT: i32 = TOP | LEFT;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GraphicsError {
    InvalidAnchor(i32),
    InvalidSourceRegion,
    InvalidTransform(i32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i32)]
pub enum SpriteTransform {
    None = 0,
    MirrorRotate180 = 1,
    Mirror = 2,
    Rotate180 = 3,
    MirrorRotate270 = 4,
    Rotate90 = 5,
    Rotate270 = 6,
    MirrorRotate90 = 7,
}

impl TryFrom<i32> for SpriteTransform {
    type Error = GraphicsError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::MirrorRotate180),
            2 => Ok(Self::Mirror),
            3 => Ok(Self::Rotate180),
            4 => Ok(Self::MirrorRotate270),
            5 => Ok(Self::Rotate90),
            6 => Ok(Self::Rotate270),
            7 => Ok(Self::MirrorRotate90),
            _ => Err(GraphicsError::InvalidTransform(value)),
        }
    }
}

impl SpriteTransform {
    const fn output_size(self, width: i32, height: i32) -> (i32, i32) {
        match self {
            Self::MirrorRotate270 | Self::Rotate90 | Self::Rotate270 | Self::MirrorRotate90 => {
                (height, width)
            }
            _ => (width, height),
        }
    }

    const fn destination(self, x: i32, y: i32, width: i32, height: i32) -> (i32, i32) {
        match self {
            Self::None => (x, y),
            Self::MirrorRotate180 => (x, height - 1 - y),
            Self::Mirror => (width - 1 - x, y),
            Self::Rotate180 => (width - 1 - x, height - 1 - y),
            Self::MirrorRotate270 => (y, x),
            Self::Rotate90 => (height - 1 - y, x),
            Self::Rotate270 => (y, width - 1 - x),
            Self::MirrorRotate90 => (height - 1 - y, width - 1 - x),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Rect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl Rect {
    fn intersect(self, other: Self) -> Self {
        let left = i64::from(self.x).max(i64::from(other.x));
        let top = i64::from(self.y).max(i64::from(other.y));
        let right = (i64::from(self.x) + i64::from(self.width))
            .min(i64::from(other.x) + i64::from(other.width));
        let bottom = (i64::from(self.y) + i64::from(self.height))
            .min(i64::from(other.y) + i64::from(other.height));
        Self {
            x: left.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            y: top.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            width: (right - left).max(0).min(i64::from(i32::MAX)) as i32,
            height: (bottom - top).max(0).min(i64::from(i32::MAX)) as i32,
        }
    }

    fn contains(self, x: i32, y: i32) -> bool {
        i64::from(x) >= i64::from(self.x)
            && i64::from(y) >= i64::from(self.y)
            && i64::from(x) < i64::from(self.x) + i64::from(self.width)
            && i64::from(y) < i64::from(self.y) + i64::from(self.height)
    }
}

pub struct Graphics<'a> {
    target: &'a mut Image,
    color: u32,
    font: FontSpec,
    translate_x: i32,
    translate_y: i32,
    clip: Rect,
}

impl<'a> Graphics<'a> {
    pub fn new(target: &'a mut Image) -> Self {
        let clip = Rect {
            x: 0,
            y: 0,
            width: target.width(),
            height: target.height(),
        };
        Self {
            target,
            color: 0xff00_0000,
            font: FontSpec::DEFAULT,
            translate_x: 0,
            translate_y: 0,
            clip,
        }
    }

    pub fn set_color(&mut self, rgb: i32) {
        self.color = 0xff00_0000 | (rgb as u32 & 0x00ff_ffff);
    }

    pub fn set_color_rgb(&mut self, red: i32, green: i32, blue: i32) {
        self.color = 0xff00_0000
            | (((red & 0xff) as u32) << 16)
            | (((green & 0xff) as u32) << 8)
            | ((blue & 0xff) as u32);
    }

    pub const fn color(&self) -> i32 {
        (self.color & 0x00ff_ffff) as i32
    }

    /// `setFont(Font)`. MIDP specifies that a null reference selects the
    /// implementation's default font. The state latch is the reusable part of
    /// the sibling Gothic port's proven `Graphics.setFont/getFont` surface;
    /// glyph drawing remains a separate device-provider operation.
    pub fn set_font(&mut self, font: Option<FontSpec>) {
        self.font = font.unwrap_or(FontSpec::DEFAULT);
    }

    /// `getFont()` -- the immutable descriptor used by later text operations.
    pub const fn font(&self) -> FontSpec {
        self.font
    }

    pub fn translate(&mut self, x: i32, y: i32) {
        self.translate_x = self.translate_x.wrapping_add(x);
        self.translate_y = self.translate_y.wrapping_add(y);
    }

    pub const fn translate_x(&self) -> i32 {
        self.translate_x
    }

    pub const fn translate_y(&self) -> i32 {
        self.translate_y
    }

    pub fn set_clip(&mut self, x: i32, y: i32, width: i32, height: i32) {
        self.clip = Rect {
            x: x.wrapping_add(self.translate_x),
            y: y.wrapping_add(self.translate_y),
            width,
            height,
        }
        .intersect(self.full_bounds());
    }

    pub fn clip_rect(&mut self, x: i32, y: i32, width: i32, height: i32) {
        self.clip = self.clip.intersect(Rect {
            x: x.wrapping_add(self.translate_x),
            y: y.wrapping_add(self.translate_y),
            width,
            height,
        });
    }

    pub const fn clip_x(&self) -> i32 {
        self.clip.x.wrapping_sub(self.translate_x)
    }

    pub const fn clip_y(&self) -> i32 {
        self.clip.y.wrapping_sub(self.translate_y)
    }

    pub const fn clip_width(&self) -> i32 {
        self.clip.width
    }

    pub const fn clip_height(&self) -> i32 {
        self.clip.height
    }

    pub fn fill_rect(&mut self, x: i32, y: i32, width: i32, height: i32) {
        let area = Rect {
            x: x.wrapping_add(self.translate_x),
            y: y.wrapping_add(self.translate_y),
            width,
            height,
        }
        .intersect(self.clip)
        .intersect(self.full_bounds());
        for target_y in area.y..area.y + area.height {
            for target_x in area.x..area.x + area.width {
                self.target.set(target_x, target_y, self.color);
            }
        }
    }

    pub fn draw_line(&mut self, x1: i32, y1: i32, x2: i32, y2: i32) {
        let mut x = i64::from(x1.wrapping_add(self.translate_x));
        let mut y = i64::from(y1.wrapping_add(self.translate_y));
        let end_x = i64::from(x2.wrapping_add(self.translate_x));
        let end_y = i64::from(y2.wrapping_add(self.translate_y));
        let delta_x = (end_x - x).abs();
        let delta_y = -(end_y - y).abs();
        let step_x = if x < end_x { 1 } else { -1 };
        let step_y = if y < end_y { 1 } else { -1 };
        let mut error = delta_x + delta_y;
        loop {
            if let (Ok(target_x), Ok(target_y)) = (i32::try_from(x), i32::try_from(y)) {
                if self.clip.contains(target_x, target_y) {
                    self.target.set(target_x, target_y, self.color);
                }
            }
            if x == end_x && y == end_y {
                break;
            }
            let doubled = error * 2;
            if doubled >= delta_y {
                error += delta_y;
                x += step_x;
            }
            if doubled <= delta_x {
                error += delta_x;
                y += step_y;
            }
        }
    }

    pub fn draw_rect(&mut self, x: i32, y: i32, width: i32, height: i32) {
        if width < 0 || height < 0 {
            return;
        }
        self.draw_line(x, y, x.wrapping_add(width), y);
        self.draw_line(
            x,
            y.wrapping_add(height),
            x.wrapping_add(width),
            y.wrapping_add(height),
        );
        self.draw_line(x, y, x, y.wrapping_add(height));
        self.draw_line(
            x.wrapping_add(width),
            y,
            x.wrapping_add(width),
            y.wrapping_add(height),
        );
    }

    /// `Graphics.fillTriangle`: fill the closed triangle using doubled integer
    /// pixel-centre tests. The bounding box is intersected with the live clip
    /// before iteration, so hostile coordinates cannot create an unbounded
    /// host loop.
    #[allow(clippy::too_many_arguments)]
    pub fn fill_triangle(&mut self, x1: i32, y1: i32, x2: i32, y2: i32, x3: i32, y3: i32) {
        let vertices = [
            (
                x1.wrapping_add(self.translate_x),
                y1.wrapping_add(self.translate_y),
            ),
            (
                x2.wrapping_add(self.translate_x),
                y2.wrapping_add(self.translate_y),
            ),
            (
                x3.wrapping_add(self.translate_x),
                y3.wrapping_add(self.translate_y),
            ),
        ];
        let min_x = vertices.iter().map(|point| point.0).min().unwrap();
        let max_x = vertices.iter().map(|point| point.0).max().unwrap();
        let min_y = vertices.iter().map(|point| point.1).min().unwrap();
        let max_y = vertices.iter().map(|point| point.1).max().unwrap();
        let bounds = Rect {
            x: min_x,
            y: min_y,
            width: max_x.saturating_sub(min_x).saturating_add(1),
            height: max_y.saturating_sub(min_y).saturating_add(1),
        }
        .intersect(self.clip)
        .intersect(self.full_bounds());

        let doubled = vertices.map(|(x, y)| (i128::from(x) * 2, i128::from(y) * 2));
        let edge = |a: (i128, i128), b: (i128, i128), p: (i128, i128)| {
            (p.0 - a.0) * (b.1 - a.1) - (p.1 - a.1) * (b.0 - a.0)
        };
        for y in bounds.y..bounds.y + bounds.height {
            for x in bounds.x..bounds.x + bounds.width {
                let point = (i128::from(x) * 2 + 1, i128::from(y) * 2 + 1);
                let edges = [
                    edge(doubled[0], doubled[1], point),
                    edge(doubled[1], doubled[2], point),
                    edge(doubled[2], doubled[0], point),
                ];
                if !edges.iter().any(|value| *value < 0) || !edges.iter().any(|value| *value > 0) {
                    self.target.set(x, y, self.color);
                }
            }
        }
    }

    pub fn fill_round_rect(
        &mut self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        arc_width: i32,
        arc_height: i32,
    ) {
        self.round_rect(x, y, width, height, arc_width, arc_height, true);
    }

    pub fn draw_round_rect(
        &mut self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        arc_width: i32,
        arc_height: i32,
    ) {
        self.round_rect(x, y, width, height, arc_width, arc_height, false);
    }

    #[allow(clippy::too_many_arguments)]
    fn round_rect(
        &mut self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        arc_width: i32,
        arc_height: i32,
        fill: bool,
    ) {
        if width <= 0 || height <= 0 {
            return;
        }
        let left = x.wrapping_add(self.translate_x);
        let top = y.wrapping_add(self.translate_y);
        let bounds = Rect {
            x: left,
            y: top,
            width,
            height,
        }
        .intersect(self.clip)
        .intersect(self.full_bounds());
        let arc_width = arc_width.saturating_abs().min(width);
        let arc_height = arc_height.saturating_abs().min(height);
        let inside = |px: i32, py: i32| {
            rounded_rect_contains(px, py, left, top, width, height, arc_width, arc_height)
        };
        for py in bounds.y..bounds.y + bounds.height {
            for px in bounds.x..bounds.x + bounds.width {
                if !inside(px, py) {
                    continue;
                }
                if !fill
                    && inside(px.wrapping_sub(1), py)
                    && inside(px.wrapping_add(1), py)
                    && inside(px, py.wrapping_sub(1))
                    && inside(px, py.wrapping_add(1))
                {
                    continue;
                }
                self.target.set(px, py, self.color);
            }
        }
    }

    /// `fillArc(x, y, w, h, startAngle, arcAngle)` — the filled ellipse sector
    /// inscribed in the `w×h` box, current opaque color, clipped. A full sweep
    /// (`|arc| >= 360`) fills the whole inscribed ellipse; a partial sweep is a
    /// well-defined angular sector.
    pub fn fill_arc(&mut self, x: i32, y: i32, w: i32, h: i32, start_angle: i32, arc_angle: i32) {
        self.arc(x, y, w, h, (start_angle, arc_angle), true);
    }

    /// `drawArc(x, y, w, h, startAngle, arcAngle)` — the 1px outline of that
    /// ellipse sector.
    pub fn draw_arc(&mut self, x: i32, y: i32, w: i32, h: i32, start_angle: i32, arc_angle: i32) {
        self.arc(x, y, w, h, (start_angle, arc_angle), false);
    }

    /// Shared ellipse rasteriser: a pixel is painted when its centre lies inside
    /// the inscribed ellipse (`fill`) or on its boundary (outline), and — for a
    /// partial sweep — its polar angle lies within `[start, start+arc)` (MIDP
    /// degrees: 0 at 3 o'clock, counter-clockwise positive). A full sweep
    /// (`|arc| >= 360`) skips the angle test. Deterministic and panic-free;
    /// clipped to the current clip and the target bounds.
    fn arc(&mut self, x: i32, y: i32, w: i32, h: i32, angle: (i32, i32), fill: bool) {
        if w <= 0 || h <= 0 {
            return;
        }
        let (start_angle, arc_angle) = angle;
        let bx = x.wrapping_add(self.translate_x);
        let by = y.wrapping_add(self.translate_y);
        let rw = w as f64 / 2.0;
        let rh = h as f64 / 2.0;
        let cx = bx as f64 + rw;
        let cy = by as f64 + rh;
        let full = arc_angle <= -360 || arc_angle >= 360;
        let (start, span) = normalize_arc(start_angle, arc_angle);

        // Pixel-centre membership in the inscribed ellipse.
        let inside = |px: i32, py: i32| -> bool {
            let nx = (px as f64 + 0.5 - cx) / rw;
            let ny = (py as f64 + 0.5 - cy) / rh;
            nx * nx + ny * ny <= 1.0
        };

        for py in by..by + h {
            for px in bx..bx + w {
                if !inside(px, py) {
                    continue;
                }
                if !fill {
                    // Outline: keep only pixels with a 4-neighbour outside.
                    let boundary = !inside(px - 1, py)
                        || !inside(px + 1, py)
                        || !inside(px, py - 1)
                        || !inside(px, py + 1);
                    if !boundary {
                        continue;
                    }
                }
                if !full {
                    let nx = (px as f64 + 0.5 - cx) / rw;
                    let ny = (py as f64 + 0.5 - cy) / rh;
                    if !angle_in(nx, ny, start, span) {
                        continue;
                    }
                }
                if self.clip.contains(px, py) {
                    self.target.set(px, py, self.color);
                }
            }
        }
    }

    pub fn draw_image(
        &mut self,
        source: &Image,
        x: i32,
        y: i32,
        anchor: i32,
    ) -> Result<(), GraphicsError> {
        self.draw_region(
            source,
            Rect {
                x: 0,
                y: 0,
                width: source.width(),
                height: source.height(),
            },
            SpriteTransform::None,
            x,
            y,
            anchor,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_region_raw(
        &mut self,
        source: &Image,
        source_x: i32,
        source_y: i32,
        width: i32,
        height: i32,
        transform: i32,
        destination_x: i32,
        destination_y: i32,
        anchor: i32,
    ) -> Result<(), GraphicsError> {
        self.draw_region(
            source,
            Rect {
                x: source_x,
                y: source_y,
                width,
                height,
            },
            SpriteTransform::try_from(transform)?,
            destination_x,
            destination_y,
            anchor,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_region(
        &mut self,
        source: &Image,
        region: Rect,
        transform: SpriteTransform,
        destination_x: i32,
        destination_y: i32,
        anchor: i32,
    ) -> Result<(), GraphicsError> {
        if region.width <= 0
            || region.height <= 0
            || region.x < 0
            || region.y < 0
            || i64::from(region.x) + i64::from(region.width) > i64::from(source.width())
            || i64::from(region.y) + i64::from(region.height) > i64::from(source.height())
        {
            return Err(GraphicsError::InvalidSourceRegion);
        }
        let (output_width, output_height) = transform.output_size(region.width, region.height);
        let (left, top) = anchor_top_left(
            destination_x,
            destination_y,
            output_width,
            output_height,
            anchor,
        )?;
        let left = left.wrapping_add(self.translate_x);
        let top = top.wrapping_add(self.translate_y);
        for source_y in 0..region.height {
            for source_x in 0..region.width {
                let (offset_x, offset_y) =
                    transform.destination(source_x, source_y, region.width, region.height);
                let target_x = left.wrapping_add(offset_x);
                let target_y = top.wrapping_add(offset_y);
                if !self.clip.contains(target_x, target_y) {
                    continue;
                }
                if let Some(pixel) = source.get(region.x + source_x, region.y + source_y) {
                    self.target.blend(target_x, target_y, pixel);
                }
            }
        }
        Ok(())
    }

    fn full_bounds(&self) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width: self.target.width(),
            height: self.target.height(),
        }
    }

    /// Plot one pixel in user coordinates through the current translate and
    /// clip. `blend = true` alpha-composites (source-over) using the source
    /// pixel's own alpha; `false` writes it through unchanged. This is the
    /// low-level primitive vendor blitters (e.g. Nokia `DirectGraphics.drawPixels`
    /// in the `j2me-nokia` crate) build on so they share this `Graphics`' *live*
    /// clip and translation rather than a snapshot.
    pub fn plot(&mut self, x: i32, y: i32, argb: u32, blend: bool) {
        let px = x.wrapping_add(self.translate_x);
        let py = y.wrapping_add(self.translate_y);
        if !self.clip.contains(px, py) {
            return;
        }
        if blend {
            self.target.blend(px, py, argb);
        } else {
            self.target.set(px, py, argb);
        }
    }

    /// Read one pixel in user coordinates through the current translate, or
    /// `None` outside the target bounds. The read-back primitive for vendor
    /// `getPixels`; unlike [`plot`](Self::plot) it is deliberately *not* clipped
    /// (a raw target read, matching Nokia `DirectGraphics.getPixels`).
    pub fn read(&self, x: i32, y: i32) -> Option<u32> {
        self.target.get(
            x.wrapping_add(self.translate_x),
            y.wrapping_add(self.translate_y),
        )
    }
}

/// Resolve a MIDP anchor to the top-left draw position, validating the anchor
/// bits (`IllegalArgumentException` on an unknown or contradictory combination).
pub fn anchor_top_left(
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    anchor: i32,
) -> Result<(i32, i32), GraphicsError> {
    let horizontal = anchor & (LEFT | HCENTER | RIGHT);
    let vertical = anchor & (TOP | VCENTER | BOTTOM | BASELINE);
    let known = LEFT | HCENTER | RIGHT | TOP | VCENTER | BOTTOM | BASELINE;
    if anchor & !known != 0
        || !matches!(horizontal, 0 | LEFT | HCENTER | RIGHT)
        || !matches!(vertical, 0 | TOP | VCENTER | BOTTOM)
    {
        return Err(GraphicsError::InvalidAnchor(anchor));
    }
    let left = match horizontal {
        RIGHT => x.wrapping_sub(width),
        HCENTER => x.wrapping_sub(width / 2),
        _ => x,
    };
    let top = match vertical {
        BOTTOM => y.wrapping_sub(height),
        VCENTER => y.wrapping_sub(height / 2),
        _ => y,
    };
    Ok((left, top))
}

/// Normalise a MIDP arc to `(startDeg in [0,360), spanDeg >= 0)`: a negative
/// `arcAngle` sweeps clockwise, i.e. from `start + arc` over `|arc|`.
fn normalize_arc(start_angle: i32, arc_angle: i32) -> (f64, f64) {
    let (mut start, span) = if arc_angle < 0 {
        ((start_angle + arc_angle) as f64, (-arc_angle) as f64)
    } else {
        (start_angle as f64, arc_angle as f64)
    };
    start = start.rem_euclid(360.0);
    (start, span)
}

/// Whether the polar angle of the normalised point `(nx, ny)` (screen y down)
/// lies within `[start, start + span)` in MIDP degrees (0 at 3 o'clock, CCW+).
fn angle_in(nx: f64, ny: f64, start: f64, span: f64) -> bool {
    if span >= 360.0 {
        return true;
    }
    let ang = (-ny).atan2(nx).to_degrees().rem_euclid(360.0);
    (ang - start).rem_euclid(360.0) <= span
}

#[allow(clippy::too_many_arguments)]
fn rounded_rect_contains(
    px: i32,
    py: i32,
    left: i32,
    top: i32,
    width: i32,
    height: i32,
    arc_width: i32,
    arc_height: i32,
) -> bool {
    let right = i64::from(left) + i64::from(width);
    let bottom = i64::from(top) + i64::from(height);
    if i64::from(px) < i64::from(left)
        || i64::from(py) < i64::from(top)
        || i64::from(px) >= right
        || i64::from(py) >= bottom
    {
        return false;
    }
    if arc_width <= 1 || arc_height <= 1 {
        return true;
    }

    let radius_x = f64::from(arc_width) / 2.0;
    let radius_y = f64::from(arc_height) / 2.0;
    let sample_x = f64::from(px) + 0.5;
    let sample_y = f64::from(py) + 0.5;
    let inner_left = f64::from(left) + radius_x;
    let inner_right = right as f64 - radius_x;
    let inner_top = f64::from(top) + radius_y;
    let inner_bottom = bottom as f64 - radius_y;
    if sample_x >= inner_left && sample_x < inner_right
        || sample_y >= inner_top && sample_y < inner_bottom
    {
        return true;
    }
    let center_x = if sample_x < inner_left {
        inner_left
    } else {
        inner_right
    };
    let center_y = if sample_y < inner_top {
        inner_top
    } else {
        inner_bottom
    };
    let dx = (sample_x - center_x) / radius_x;
    let dy = (sample_y - center_y) / radius_y;
    dx * dx + dy * dy <= 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_state_defaults_sets_and_resets_on_null() {
        let mut image = Image::create_mutable(2, 2).unwrap();
        let custom = FontSpec {
            face: 64,
            style: 1 | 2,
            size: 16,
        };
        {
            let mut graphics = Graphics::new(&mut image);
            assert_eq!(graphics.font(), FontSpec::DEFAULT);
            graphics.set_font(Some(custom));
            assert_eq!(graphics.font(), custom);
            graphics.set_font(None);
            assert_eq!(graphics.font(), FontSpec::DEFAULT);
        }

        // A second Graphics over the same target is a fresh Java context; font
        // state belongs to Graphics, not Image.
        let graphics = Graphics::new(&mut image);
        assert_eq!(graphics.font(), FontSpec::DEFAULT);
    }

    #[test]
    fn fill_and_line_respect_clip() {
        let mut image = Image::create_mutable(5, 4).unwrap();
        {
            let mut graphics = Graphics::new(&mut image);
            graphics.set_color(0x123456);
            graphics.set_clip(1, 1, 3, 2);
            graphics.fill_rect(-10, -10, 30, 30);
        }
        assert_eq!(image.get(1, 1), Some(0xff12_3456));
        assert_eq!(image.get(3, 2), Some(0xff12_3456));
        assert_eq!(image.get(0, 0), Some(0xffff_ffff));
    }

    #[test]
    fn image_anchor_and_transform_are_explicit() {
        let source = Image::from_argb(2, 1, vec![0xffff_0000, 0xff00_00ff]).unwrap();
        let mut target = Image::create_mutable(4, 3).unwrap();
        {
            let mut graphics = Graphics::new(&mut target);
            graphics.draw_image(&source, 3, 0, RIGHT | TOP).unwrap();
            graphics
                .draw_region_raw(&source, 0, 0, 2, 1, 5, 0, 1, LEFT | TOP)
                .unwrap();
        }
        assert_eq!(target.get(1, 0), Some(0xffff_0000));
        assert_eq!(target.get(2, 0), Some(0xff00_00ff));
        assert_eq!(target.get(0, 1), Some(0xffff_0000));
        assert_eq!(target.get(0, 2), Some(0xff00_00ff));
    }

    #[test]
    fn invalid_regions_and_transforms_are_rejected() {
        let source = Image::create_mutable(1, 1).unwrap();
        let mut target = Image::create_mutable(1, 1).unwrap();
        let mut graphics = Graphics::new(&mut target);
        assert_eq!(
            graphics.draw_region_raw(&source, 0, 0, 2, 1, 0, 0, 0, 0),
            Err(GraphicsError::InvalidSourceRegion)
        );
        assert_eq!(
            graphics.draw_region_raw(&source, 0, 0, 1, 1, 8, 0, 0, 0),
            Err(GraphicsError::InvalidTransform(8))
        );
    }

    #[test]
    fn postal_primitives_fill_triangles_and_round_corners() {
        let mut target = Image::create_mutable(9, 9).unwrap();
        {
            let mut graphics = Graphics::new(&mut target);
            graphics.set_color(0x00ff00);
            graphics.fill_triangle(1, 1, 7, 1, 4, 7);
            graphics.set_color(0xff0000);
            graphics.fill_round_rect(0, 0, 5, 5, 5, 5);
        }
        assert_eq!(target.get(5, 2), Some(0xff00_ff00));
        assert_eq!(target.get(4, 5), Some(0xff00_ff00));
        assert_eq!(target.get(2, 2), Some(0xffff_0000));
        assert_eq!(target.get(0, 0), Some(0xffff_ffff));
    }
}

// Behavioral tests for the primitives above (`fill_rect`/`draw_rect`/`draw_line`
// with translate + clip, anchor resolution, alpha blits) and the
// `draw_arc`/`fill_arc` ellipse-sector rasteriser.
#[cfg(test)]
mod behavior_tests {
    use super::*;

    #[test]
    fn fill_rect_respects_translate_and_clip() {
        let mut img = Image::create_mutable(6, 6).unwrap();
        let mut g = Graphics::new(&mut img);
        g.set_color(0x00FF_0000); // red
        g.translate(1, 1);
        g.set_clip(0, 0, 3, 3); // clip 3x3 at (1,1) in target space
        g.fill_rect(0, 0, 10, 10); // huge, gets clipped
        assert_eq!(img.get(1, 1), Some(0xFFFF_0000));
        assert_eq!(img.get(3, 3), Some(0xFFFF_0000));
        assert_eq!(img.get(4, 4), Some(0xFFFF_FFFF)); // outside clip stays white
        assert_eq!(img.get(0, 0), Some(0xFFFF_FFFF)); // before translate origin
    }

    #[test]
    fn get_color_round_trips_set_color() {
        let mut img = Image::create_mutable(2, 2).unwrap();
        let mut g = Graphics::new(&mut img);
        g.set_color(0x00AB_CDEF);
        assert_eq!(g.color(), 0x00AB_CDEF);
        g.set_color_rgb(255, 255, 255);
        assert_eq!(g.color(), 0x00FF_FFFF);
    }

    #[test]
    fn anchors_resolve_center_and_corners() {
        assert_eq!(anchor_top_left(10, 10, 4, 4, TOP_LEFT).unwrap(), (10, 10));
        assert_eq!(
            anchor_top_left(10, 10, 4, 4, RIGHT | BOTTOM).unwrap(),
            (6, 6)
        );
        assert_eq!(
            anchor_top_left(10, 10, 4, 4, HCENTER | VCENTER).unwrap(),
            (8, 8)
        );
    }

    #[test]
    fn draw_image_blits_with_alpha() {
        let mut img = Image::create_mutable(4, 4).unwrap(); // opaque white
                                                            // 2x2 source: opaque blue top-left, transparent elsewhere.
        let src = Image::from_argb(
            2,
            2,
            vec![0xFF00_00FF, 0x0000_0000, 0x0000_0000, 0x0000_0000],
        )
        .unwrap();
        let mut g = Graphics::new(&mut img);
        g.draw_image(&src, 1, 1, TOP_LEFT).unwrap();
        assert_eq!(img.get(1, 1), Some(0xFF00_00FF)); // opaque blue landed
        assert_eq!(img.get(2, 1), Some(0xFFFF_FFFF)); // transparent left white
    }

    #[test]
    fn draw_image_right_anchor_and_translate() {
        // A 2x2 sprite drawn with TOP|RIGHT anchor lands its right edge at x.
        let mut img = Image::create_mutable(6, 6).unwrap();
        let src = Image::from_argb(2, 2, vec![0xFF00_0000; 4]).unwrap();
        let mut g = Graphics::new(&mut img);
        g.translate(1, 0);
        g.draw_image(&src, 4, 0, TOP | RIGHT).unwrap(); // top-left = (4-2, 0) + tx(1,0) = (3,0)
        assert_eq!(img.get(3, 0), Some(0xFF00_0000));
        assert_eq!(img.get(4, 1), Some(0xFF00_0000));
        assert_eq!(img.get(2, 0), Some(0xFFFF_FFFF)); // nothing to the left
    }

    #[test]
    fn draw_line_is_clipped() {
        let mut img = Image::create_mutable(5, 5).unwrap();
        let mut g = Graphics::new(&mut img);
        g.set_color(0x0000_0000);
        g.set_clip(0, 0, 3, 5);
        g.draw_line(0, 0, 4, 0); // horizontal, clipped at x<3
        assert_eq!(img.get(0, 0), Some(0xFF00_0000));
        assert_eq!(img.get(2, 0), Some(0xFF00_0000));
        assert_eq!(img.get(3, 0), Some(0xFFFF_FFFF)); // clipped out
    }

    #[test]
    fn draw_rect_outlines_but_leaves_the_interior() {
        let mut img = Image::create_mutable(6, 6).unwrap();
        let mut g = Graphics::new(&mut img);
        g.set_color(0x0000_0000);
        g.draw_rect(1, 1, 3, 3); // corners (1,1)..(4,4)
        assert_eq!(img.get(1, 1), Some(0xFF00_0000)); // corner
        assert_eq!(img.get(4, 4), Some(0xFF00_0000)); // far corner (inclusive)
        assert_eq!(img.get(2, 2), Some(0xFFFF_FFFF)); // interior untouched
    }

    /// Count pixels equal to `argb` in the whole image.
    fn count_color(img: &Image, argb: u32) -> usize {
        let mut n = 0;
        for y in 0..img.height() {
            for x in 0..img.width() {
                if img.get(x, y) == Some(argb) {
                    n += 1;
                }
            }
        }
        n
    }

    #[test]
    fn fill_arc_full_sweep_fills_the_ellipse_interior_but_not_the_corners() {
        // A wide, short filled ellipse: fillArc(x, y, 22, 9, 0, 360). Its area is
        // well under the bounding box, and the box corners stay unpainted (R3:
        // proves it is an ellipse, not a rectangle).
        let mut img = Image::create_mutable(24, 12).unwrap();
        let black = 0xFF00_0000u32;
        {
            let mut g = Graphics::new(&mut img);
            g.set_color(0x0000_0000);
            g.fill_arc(1, 1, 22, 9, 0, 360);
        }
        let ink = count_color(&img, black);
        assert!(ink > 0, "the ellipse must paint interior pixels");
        assert!(ink < 22 * 9, "an ellipse fills less than its bounding box");
        // Centre is inside; the four bounding-box corners are outside the ellipse.
        assert_eq!(img.get(1 + 11, 1 + 4), Some(black));
        assert_eq!(img.get(1, 1), Some(0xFFFF_FFFF));
        assert_eq!(img.get(1 + 21, 1), Some(0xFFFF_FFFF));
        assert_eq!(img.get(1, 1 + 8), Some(0xFFFF_FFFF));
        assert_eq!(img.get(1 + 21, 1 + 8), Some(0xFFFF_FFFF));
    }

    #[test]
    fn draw_arc_outline_paints_a_ring_not_a_disc() {
        // A larger circle: the outline paints a hollow ring — some interior
        // pixel stays unpainted (R3: distinguishes drawArc from fillArc).
        let mut img = Image::create_mutable(12, 12).unwrap();
        let black = 0xFF00_0000u32;
        {
            let mut g = Graphics::new(&mut img);
            g.set_color(0x0000_0000);
            g.draw_arc(1, 1, 9, 9, 0, 360);
        }
        assert!(count_color(&img, black) > 0, "outline paints pixels");
        // The centre of a hollow outline is not painted.
        assert_eq!(img.get(1 + 4, 1 + 4), Some(0xFFFF_FFFF));
    }

    #[test]
    fn arc_honors_clip_and_color() {
        // fillArc respects the clip like every other primitive.
        let mut img = Image::create_mutable(12, 12).unwrap();
        {
            let mut g = Graphics::new(&mut img);
            g.set_color(0x0000_00FF);
            g.set_clip(0, 0, 6, 12); // left half only
            g.fill_arc(0, 0, 12, 12, 0, 360);
        }
        // No blue ink in the clipped-out right half.
        for y in 0..12 {
            for x in 6..12 {
                assert_ne!(img.get(x, y), Some(0xFF00_00FF), "arc escaped the clip");
            }
        }
    }
}
