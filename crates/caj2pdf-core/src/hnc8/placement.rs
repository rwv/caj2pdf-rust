// SPDX-License-Identifier: MIT

//! Pure geometry for the explicitly empirical HN-A/C8 placement profile.
//!
//! The coordinate factor is measured, not an independently known physical
//! source unit. The caller supplies independently checked source dimensions
//! and raw text words; this module neither reads a document nor writes a PDF.
//! It does not enable production HN/C8 composition.

use super::RawTextCoordinate;
use crate::pdf::PageSpec;
use crate::{Error, Result};

/// Empirical PDF points per raw text-coordinate unit in the measured profile.
/// This does not assign an authoritative physical unit to the source word.
pub const EMPIRICAL_COORDINATE_POINTS_PER_UNIT: f64 = 240.0 / 2473.0;
/// Four-decimal reference output gives this absolute comparison tolerance.
/// Evaluation returns full `f64` precision rather than rounding early.
pub const EMPIRICAL_PLACEMENT_TOLERANCE_POINTS: f64 = 0.00005;

/// Caller-supplied PDF origin and a source-derived page size.
///
/// An origin shift is a caller choice, not an observed HN/C8 source field.
/// Public fields permit construction by callers; every evaluator revalidates
/// them. No image pixels or coordinates are retained by this value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EmpiricalPageGeometry {
    pub origin_points: [f64; 2],
    pub size: PageSpec,
}

impl EmpiricalPageGeometry {
    /// Return a finite, noncollapsed `[left, bottom, right, top]` PDF rectangle.
    /// The supplied dimensions must survive the origin addition within the
    /// documented empirical comparison tolerance.
    pub fn media_box(self) -> Result<[f64; 4]> {
        let [left, bottom] = self.origin_points;
        let PageSpec {
            width_points,
            height_points,
        } = self.size;
        if !left.is_finite() || !bottom.is_finite() {
            return Err(Error::invalid("empirical PDF origin must be finite"));
        }
        if !width_points.is_finite()
            || !height_points.is_finite()
            || width_points <= 0.0
            || height_points <= 0.0
        {
            return Err(Error::invalid(
                "empirical page dimensions must be finite and positive",
            ));
        }
        let right = left + width_points;
        let top = bottom + height_points;
        if !right.is_finite() || !top.is_finite() || right <= left || top <= bottom {
            return Err(Error::invalid(
                "empirical page rectangle overflows or collapses at the supplied origin",
            ));
        }
        if ((right - left) - width_points).abs() > EMPIRICAL_PLACEMENT_TOLERANCE_POINTS
            || ((top - bottom) - height_points).abs() > EMPIRICAL_PLACEMENT_TOLERANCE_POINTS
        {
            return Err(Error::invalid(
                "empirical PDF origin cannot preserve the page dimension precision",
            ));
        }
        Ok([left, bottom, right, top])
    }
}

/// Derive a page from the first image's checked pixel dimensions.
///
/// Points per pixel use the exact ratio `72 / 300`: the integer product is
/// exactly representable for every `u32`, before one floating division.
/// This helper has no dynamic allocations; image-decoder limits remain the
/// caller's responsibility.
pub fn empirical_page_from_pixels(
    pixel_width: u32,
    pixel_height: u32,
    origin_points: [f64; 2],
) -> Result<EmpiricalPageGeometry> {
    let size = pixel_size(pixel_width, pixel_height)?;
    let page = EmpiricalPageGeometry {
        origin_points,
        size,
    };
    page.media_box()?;
    Ok(page)
}

/// Predict one image CTM from checked source pixels and raw coordinate words.
///
/// The raw source origin is top-left, x increases rightward, and y increases
/// downward. The result is `[width, 0, 0, -height, x, y]`; negative PDF
/// positions and images outside the page are preserved without clipping.
/// Calls preserve source order and repeated images without a lookup table.
/// For type-0 rasters, use visible width when `visible_bytes == dib_stride`,
/// otherwise `dib_stride * 8`. This matches the page helper above.
///
/// All raw `u16` values, including bit 15, are evaluated as unsigned without
/// clipping. This is the evaluator's mathematical domain, not proof that
/// every value or document layout occurs in the source format. A successful
/// evaluation is only an empirical prediction; it does not validate framing
/// or format applicability. Pixel ranges come from the source image decoder.
/// A PDF origin whose magnitude loses the selected coordinate offset beyond
/// [`EMPIRICAL_PLACEMENT_TOLERANCE_POINTS`] is rejected, even if its page
/// rectangle is finite and noncollapsed.
pub fn empirical_image_transform(
    page: EmpiricalPageGeometry,
    pixel_width: u32,
    pixel_height: u32,
    coordinate: RawTextCoordinate,
) -> Result<[f64; 6]> {
    image_transform(page, pixel_size(pixel_width, pixel_height)?, coordinate)
}

/// Source page/image extents share the measured coordinate unit. Their
/// physical unit remains empirical; pixel dimensions do not determine layout.
pub(super) fn source_page_geometry(size: [u16; 2]) -> Result<EmpiricalPageGeometry> {
    Ok(EmpiricalPageGeometry {
        origin_points: [0.0, 0.0],
        size: source_size(size)?,
    })
}

pub(super) fn source_image_transform(
    page: EmpiricalPageGeometry,
    coordinate: RawTextCoordinate,
) -> Result<[f64; 6]> {
    image_transform(
        page,
        source_size([coordinate.width, coordinate.height])?,
        coordinate,
    )
}

fn source_size([width, height]: [u16; 2]) -> Result<PageSpec> {
    if width == 0 || height == 0 {
        return Err(Error::invalid(
            "declared source dimensions must be positive",
        ));
    }
    Ok(PageSpec {
        width_points: f64::from(width) * EMPIRICAL_COORDINATE_POINTS_PER_UNIT,
        height_points: f64::from(height) * EMPIRICAL_COORDINATE_POINTS_PER_UNIT,
    })
}

fn image_transform(
    page: EmpiricalPageGeometry,
    size: PageSpec,
    coordinate: RawTextCoordinate,
) -> Result<[f64; 6]> {
    let [left, bottom, _, top] = page.media_box()?;
    let x_offset = f64::from(coordinate.x) * EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
    let y_offset = f64::from(coordinate.y) * EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
    let x = left + x_offset;
    let y = top - y_offset;
    // The page bounds are finite. A bounded u16 offset is far too small to
    // overflow any finite f64 endpoint, even at its largest magnitude.
    if ((x - left) - x_offset).abs() > EMPIRICAL_PLACEMENT_TOLERANCE_POINTS
        || ((top - y) - y_offset).abs() > EMPIRICAL_PLACEMENT_TOLERANCE_POINTS
        // Page-height and selected-offset rounding can each pass separately
        // while their combined translation exceeds the same tolerance.
        || ((y - bottom) - (page.size.height_points - y_offset)).abs()
            > EMPIRICAL_PLACEMENT_TOLERANCE_POINTS
    {
        return Err(Error::invalid(
            "empirical PDF origin cannot preserve the selected coordinate precision",
        ));
    }
    Ok([size.width_points, 0.0, 0.0, -size.height_points, x, y])
}

/// Glyph classes independently measured with original C8 font controls.
/// This class does not select a font or decode a source character.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum C8GlyphClass {
    Cjk,
    Latin,
}

/// Evaluate the empirical C8 text matrix for the measured native style subset.
///
/// Size fields 2 through 8 with observed high bits `0x0800`, `0x0c00` or
/// `0x1000` share the measured glyph geometry. Independently controlled
/// `0x04e7` and `0x14e7` also share field-7 geometry; `0x0484`, `0x1484` and `0x9c84`
/// share field-4 geometry. `0x04c6` and `0x14c6` share field-6 geometry;
/// `0x14a5` shares field-5 geometry. The observed `0xe58c` and `0x154a` CJK forms use
/// measured sizes 109 and 84 respectively; their Latin baselines are unknown.
/// The point-size
/// model is calibrated from original font controls, including held-out field 7;
/// it is not an authoritative physical-unit definition. See the recorded
/// geometry and rasterization limits in `docs/research/c8-native-records.md`.
///
/// `position` and `source_origin` are raw x/y words. Subtraction is signed;
/// off-page glyphs remain off-page. Font selection, character decoding, color
/// and page-content admission are the caller's separate responsibilities.
/// `axes` holds explicit glyph axes when the record supplies them.
pub(super) fn native_glyph_transform(
    page: EmpiricalPageGeometry,
    source_origin: [u16; 2],
    position: [u16; 2],
    style: u16,
    class: C8GlyphClass,
    axes: [Option<u16>; 2],
) -> Result<[f64; 6]> {
    let [left, _, _, top] = page.media_box()?;
    let (width, height, latin_offset) = if axes != [None; 2] {
        let (width, height, baseline) = match axes {
            [Some(4), Some(4)] => (4.0, 4.0, 15.0),
            [Some(36), Some(36)] => (36.0, 36.0, 8.0),
            [Some(width @ (28 | 43)), Some(height @ (28 | 43))] => (
                f64::from(width),
                f64::from(height),
                if height == 28 { 9.0 } else { 6.0 },
            ),
            _ => {
                return Err(Error::invalid("unverified native explicit glyph axes"));
            }
        };
        (width * 75.0 / 301.0, height * 75.0 / 301.0, baseline)
    } else if matches!(style, 0x1000 | 0x1001 | 0x1020 | 0x1021 | 0x1022 | 0x1041) {
        // Independent small-field/unequal-axis controls. Decoration and
        // other flag/field combinations remain outside this glyph evidence.
        let width = match (style >> 5) & 31 {
            0 => 21.0,
            1 => 24.0,
            _ => 28.0,
        };
        let (height, baseline) = match style & 31 {
            0 => (21.0, 11.0),
            1 => (24.0, 10.0),
            _ => (28.0, 9.0),
        };
        (width * 75.0 / 301.0, height * 75.0 / 301.0, baseline)
    } else if matches!(style, 0xe58c | 0x114a | 0x154a) && class == C8GlyphClass::Cjk {
        // Original controls distinguish explicit 109 and 84 from adjacent
        // sizes. Latin baselines and other size-field flags remain unverified.
        let size = if style == 0xe58c { 109.0 } else { 84.0 };
        let em = size * 75.0 / 301.0;
        (em, em, 0.0)
    } else {
        if !matches!(style & 0xfc00, 0x0800 | 0x0c00 | 0x1000)
            && !matches!(
                style,
                0x04e7 | 0x14e7 | 0x0484 | 0x1484 | 0x9c84 | 0x04c6 | 0x14c6 | 0x14a5
            )
        {
            return Err(Error::invalid("unverified C8 glyph style flags"));
        }
        c8_style_metrics((style & 0x03ff) | 0x1000)?
    };
    let unit = EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
    let mut x = left + (f64::from(position[0]) - f64::from(source_origin[0]) + 20.0) * unit;
    let mut y = top - (f64::from(position[1]) - f64::from(source_origin[1]) - 15.0) * unit - height;
    if class == C8GlyphClass::Latin {
        x += width / 8.0;
        y -= latin_offset * unit;
    }
    Ok([width, 0.0, 0.0, height, x, y])
}

/// Independently controlled HN-B mode-0 CJK and Latin placement.
/// Resource selection and the mode-0 page extent addition are handled by the
/// page writer. Unobserved styles/classes are not inferred from mode 2.
pub(super) fn mode_zero_glyph_transform(
    page: EmpiricalPageGeometry,
    source_origin: [u16; 2],
    position: [u16; 2],
    style: u16,
    class: C8GlyphClass,
    axes: [Option<u16>; 2],
) -> Result<[f64; 6]> {
    if axes == [None; 2] && matches!(style, 0 | 0x1000) {
        let [left, _, _, top] = page.media_box()?;
        let unit = EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
        let em = 21.0 * 75.0 / 301.0;
        let baseline = if class == C8GlyphClass::Latin {
            10.0
        } else {
            0.0
        };
        return Ok([
            em,
            0.0,
            0.0,
            em,
            left + (f64::from(position[0]) - f64::from(source_origin[0]) + 20.0) * unit,
            top - (f64::from(position[1]) - f64::from(source_origin[1]) + baseline) * unit - em,
        ]);
    }
    if !matches!(
        style,
        0 | 0x0484 | 0x0884 | 0x1084 | 0x9c84 | 0x0ca4 | 0x10a4 | 0x10a5 | 0x04e7 | 0x0ce7
    ) && !(class == C8GlyphClass::Cjk && style == 0x154a)
    {
        return Err(Error::invalid("unverified HN-B mode-0 glyph style"));
    }
    let mut transform = native_glyph_transform(
        page,
        source_origin,
        position,
        style,
        C8GlyphClass::Cjk,
        axes,
    )?;
    let baseline = if class == C8GlyphClass::Latin {
        10.0
    } else {
        0.0
    };
    transform[5] -= (15.0 + baseline) * EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
    Ok(transform)
}

/// Required mode-0 digits share the ordinary Latin font, but not alphabet
/// positioning. Offsets are empirical source units from isolated controls;
/// only the observed height fields and explicit-36 pair are admitted.
pub(super) fn mode_zero_digit_transform(
    page: EmpiricalPageGeometry,
    source_origin: [u16; 2],
    position: [u16; 2],
    style: u16,
    axes: [Option<u16>; 2],
) -> Result<[f64; 6]> {
    let (left, down) = match (style, axes) {
        (_, [Some(36), Some(36)]) => (22.0, 18.0),
        (0 | 0x1000, [None, None]) => (21.0, 15.0),
        (0x0484 | 0x0884 | 0x1084 | 0x9c84 | 0x0ca4 | 0x10a4, [None, None]) => (22.0, 18.0),
        (0x10a5, [None, None]) => (22.0, 20.0),
        _ => {
            return Err(Error::invalid("unverified HN-B mode-0 digit geometry"));
        }
    };
    let mut transform = mode_zero_glyph_transform(
        page,
        source_origin,
        position,
        style,
        C8GlyphClass::Cjk,
        axes,
    )?;
    transform[4] -= left * EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
    transform[5] -= down * EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
    Ok(transform)
}

/// Constant-size description of a forward horizontal C8 decoration.
/// Emit `glyph_count` marks with the first matrix's x incremented by
/// `index * first_glyph[0]`, using the same clip for each decorative glyph.
/// No glyph list, font data or page content is retained.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EmpiricalC8HorizontalDecoration {
    pub first_glyph: [f64; 6],
    /// `[left, bottom, width, height]` in PDF points.
    pub clip: [f64; 4],
    pub glyph_count: u16,
}

/// Nominal placement for the observed forward horizontal `8010/1` form.
/// The caller must establish record identity and provide the active text style.
/// Reversed, zero-length, vertical and diagonal spans are not admitted here.
///
/// Repetition uses one nominal em, not font advance or integer screen pixels.
/// Final marks are clipped at the source endpoint. Source/PDF raster residuals
/// and the empirical physical-unit model are documented in c8-native-records.
/// Font choice and alias mapping remain caller responsibilities; use decorative
/// glyph output rather than treating the alias as semantic Unicode text.
/// This evaluator does not enable complete native-page conversion.
pub fn empirical_c8_horizontal_decoration(
    page: EmpiricalPageGeometry,
    source_origin: [u16; 2],
    points: [[u16; 2]; 2],
    style: u16,
) -> Result<EmpiricalC8HorizontalDecoration> {
    let [left, bottom, _, top] = page.media_box()?;
    let (width, height, _) = c8_style_metrics(style)?;
    let [[x1, y1], [x2, y2]] = points;
    if x2 <= x1 || y1 != y2 {
        return Err(Error::invalid(
            "unverified C8 decoration direction or empty span",
        ));
    }
    let unit = EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
    let x = left + (f64::from(x1) - f64::from(source_origin[0])) * unit;
    let y = top - (f64::from(y1) - f64::from(source_origin[1])) * unit - height / 2.0;
    let span = f64::from(x2 - x1) * unit;
    // u16 coordinates and the minimum admitted em bound this count below 1000.
    let glyph_count = (span / width).ceil() as u16;
    Ok(EmpiricalC8HorizontalDecoration {
        first_glyph: [width, 0.0, 0.0, height, x, y],
        clip: [x, bottom, span, page.size.height_points],
        glyph_count,
    })
}

/// Evaluate endpoints for observed native `8006/a381`, `a383`, `a385` and `a38b` segments.
/// Callers must establish the record tag separately. Emit these endpoints with
/// the existing PDF segment writer's zero width (device-dependent hairline).
/// The empirical source margin is independent of text/decoration baselines.
/// Raster width and antialiasing differ across PDF renderers; this is not a
/// pixel-parity guarantee. For independently controlled `a385`, paired `c000`
/// bits in the first x word mark its low 14-bit coordinate. Other words retain
/// their raw values. Unknown styles are rejected.
///
/// Endpoints retain order and signed off-page positions. This allocation-free
/// evaluator performs no font selection or complete-page admission.
pub fn empirical_c8_segment(
    page: EmpiricalPageGeometry,
    source_origin: [u16; 2],
    points: [[u16; 2]; 2],
    style: u16,
) -> Result<[[f64; 2]; 2]> {
    let [left, _, _, top] = page.media_box()?;
    if !matches!(style, 0xa381 | 0xa383 | 0xa385 | 0xa38b) {
        return Err(Error::invalid("unverified C8 segment style"));
    }
    let unit = EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
    let mut points = points;
    if style == 0xa385 && points[0][0] & 0xc000 == 0xc000 {
        points[0][0] &= 0x3fff;
    }
    Ok(points.map(|[x, y]| {
        [
            left + (f64::from(x) - f64::from(source_origin[0]) + 20.0) * unit,
            top - (f64::from(y) - f64::from(source_origin[1]) + 20.0) * unit,
        ]
    }))
}

fn c8_style_metrics(style: u16) -> Result<(f64, f64, f64)> {
    if style & 0xfc00 != 0x1000 {
        return Err(Error::invalid("unverified C8 glyph style flags"));
    }
    let metrics = |field| -> Result<(f64, f64)> {
        let (step, latin_offset) = match field {
            2 => (28, 9),
            3 => (31, 9),
            4 => (35, 8),
            5 => (42, 6),
            6 => (48, 5),
            7 => (56, 3),
            8 => (63, 1),
            _ => {
                return Err(Error::invalid("unverified C8 glyph size field"));
            }
        };
        Ok((f64::from(step) * 75.0 / 301.0, f64::from(latin_offset)))
    };
    let (width, _) = metrics((style >> 5) & 31)?;
    let (height, latin_offset) = metrics(style & 31)?;
    Ok((width, height, latin_offset))
}

fn pixel_size(pixel_width: u32, pixel_height: u32) -> Result<PageSpec> {
    if pixel_width == 0 || pixel_height == 0 {
        return Err(Error::invalid(
            "empirical image dimensions must be positive",
        ));
    }
    Ok(PageSpec {
        // Multiplication by an already rounded binary 0.24 can move the
        // shortest PDF decimal across a renderer's device-pixel boundary.
        // These products are exact integers below 2^53; round only the ratio.
        width_points: f64::from(pixel_width) * 72.0 / 300.0,
        height_points: f64::from(pixel_height) * 72.0 / 300.0,
    })
}

#[cfg(test)]
mod tests;
