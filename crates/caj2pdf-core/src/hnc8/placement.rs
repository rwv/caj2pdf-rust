// SPDX-License-Identifier: MIT

//! Pure geometry for the explicitly empirical HN-A/C8 placement profile.
//!
//! The coordinate factor is measured, not an independently known physical
//! source unit. The caller supplies independently checked source dimensions
//! and raw text words; this module neither reads a document nor writes a PDF.
//! It does not enable production HN/C8 composition.

use super::RawTextCoordinate;
use crate::jbig1::Type0Info;
use crate::pdf::PageSpec;
use crate::{Error, Result};

/// Empirical PDF points per raw text-coordinate unit in the measured profile.
/// This does not assign an authoritative physical unit to the source word.
pub const EMPIRICAL_COORDINATE_POINTS_PER_UNIT: f64 = 240.0 / 2473.0;
/// Empirical PDF points per source image pixel in the measured profile.
/// Dimension evaluation uses the exact ratio `72 / 300`: the integer product
/// is exactly representable for every `u32`, before one floating division.
pub const EMPIRICAL_PIXEL_POINTS: f64 = 0.24;
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
            return Err(Error::InvalidInput {
                reason: "empirical PDF origin must be finite",
            });
        }
        if !width_points.is_finite()
            || !height_points.is_finite()
            || width_points <= 0.0
            || height_points <= 0.0
        {
            return Err(Error::InvalidInput {
                reason: "empirical page dimensions must be finite and positive",
            });
        }
        let right = left + width_points;
        let top = bottom + height_points;
        if !right.is_finite() || !top.is_finite() || right <= left || top <= bottom {
            return Err(Error::InvalidInput {
                reason: "empirical page rectangle overflows or collapses at the supplied origin",
            });
        }
        if ((right - left) - width_points).abs() > EMPIRICAL_PLACEMENT_TOLERANCE_POINTS
            || ((top - bottom) - height_points).abs() > EMPIRICAL_PLACEMENT_TOLERANCE_POINTS
        {
            return Err(Error::InvalidInput {
                reason: "empirical PDF origin cannot preserve the page dimension precision",
            });
        }
        Ok([left, bottom, right, top])
    }
}

/// Derive a page from the first image's checked pixel dimensions.
///
/// Use this for JPEG dimensions. Type-0 DIB pages require
/// [`empirical_page_from_type0`], because the observed profile includes the
/// padded row width in its page size. This helper has no dynamic allocations;
/// image-decoder limits remain the caller's responsibility.
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

/// Display width for checked type-0 metadata in the observed reference profile.
/// Whole padding bytes expand the displayed width; unused bits in the last
/// visible byte do not. Callers validate the DIB dimensions before using this.
pub(super) fn type0_display_width(info: Type0Info) -> u64 {
    if info.visible_bytes == info.dib_stride {
        u64::from(info.width)
    } else {
        info.dib_stride as u64 * 8
    }
}

/// Derive a first-type-0 page from consistent DIB dimensions and row storage.
///
/// The observed profile retains visible width if its byte count already
/// equals the DIB stride; otherwise whole padding bytes expand the width.
/// Although [`Type0Info`] normally comes from the checked decoder, its fields
/// are public, so both stride and visible-byte consistency are checked here.
pub fn empirical_page_from_type0(
    info: Type0Info,
    origin_points: [f64; 2],
) -> Result<EmpiricalPageGeometry> {
    if info.width == 0 || info.height == 0 {
        return Err(Error::InvalidInput {
            reason: "empirical type-0 dimensions must be positive",
        });
    }
    let stride = u64::from(info.width).div_ceil(32) * 4;
    let visible = u64::from(info.width).div_ceil(8);
    if info.dib_stride as u64 != stride || info.visible_bytes as u64 != visible {
        return Err(Error::InvalidInput {
            reason: "empirical type-0 stride or visible bytes differ from the one-bit DIB dimensions",
        });
    }
    let display_width =
        u32::try_from(type0_display_width(info)).map_err(|_| Error::InvalidInput {
            reason: "empirical type-0 display width exceeds the supported pixel range",
        })?;
    empirical_page_from_pixels(display_width, info.height, origin_points)
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
    let [left, bottom, _, top] = page.media_box()?;
    let size = pixel_size(pixel_width, pixel_height)?;
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
        return Err(Error::InvalidInput {
            reason: "empirical PDF origin cannot preserve the selected coordinate precision",
        });
    }
    Ok([size.width_points, 0.0, 0.0, -size.height_points, x, y])
}

fn pixel_size(pixel_width: u32, pixel_height: u32) -> Result<PageSpec> {
    if pixel_width == 0 || pixel_height == 0 {
        return Err(Error::InvalidInput {
            reason: "empirical image dimensions must be positive",
        });
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
