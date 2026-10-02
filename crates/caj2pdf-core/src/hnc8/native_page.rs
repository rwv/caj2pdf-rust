// SPDX-License-Identifier: MIT

//! Incremental composition of the independently controlled C8 native subset.

use super::{
    C8GlyphClass, EmpiricalPageGeometry, ErrorKind, Hnc8Reader, Location, NativeRecord,
    NativeRecordVisitor, Result, TextBudget, Variant, decode_native_character,
    decode_native_image_coordinate, empirical_c8_glyph_transform,
    empirical_c8_horizontal_decoration, empirical_c8_segment,
};
use crate::pdf::{ContentPageWriter, FontObject, ImageObject, PdfDocument};
use crate::{Cancellation, Error, RangedSource, SequentialSink};

/// Indices into the font handles supplied to [`write_c8_native_page`].
/// Resources are explicit: no system lookup or implicit glyph substitution.
#[derive(Clone, Copy, Debug)]
pub struct C8PageFonts {
    pub cjk: usize,
    pub latin: usize,
    /// Resource selected by the observed ordinary Latin `801d/4` state.
    /// This is not a universal bold flag.
    pub alternate_latin: usize,
    /// Font index and nonsemantic character-map alias for horizontal decoration.
    pub decoration: Option<(usize, char)>,
}

/// Write and finish the current C8 native page using already embedded resources.
///
/// Call `next_page` first. Images must be supplied in descriptor order, decoded
/// through the existing codecs. `top_first` identifies each emitted image's row
/// representation: existing JPEG/type-0 emitters use false; type-3 uses true.
/// Fonts are embedded once per document and may share resource indices.
///
/// Records are consumed one at a time; no glyph or page-content vector is kept.
/// This initial translation admits measured ordinary CJK/ASCII glyphs, segments,
/// horizontal decoration and the controlled image form. Required symbols and
/// other unresolved controls fail explicitly rather than being skipped. It does
/// not establish whole-document C8 support or provide the adapter font transport.
/// Returns the zero-based PDF page index. Any failure after opening the page
/// leaves the PDF unfinished and unusable.
pub async fn write_c8_native_page<S, W, C>(
    reader: &mut Hnc8Reader<'_, S, C>,
    document: &mut PdfDocument<'_, W, C>,
    fonts: &[&FontObject],
    roles: C8PageFonts,
    images: &[ImageObject],
    top_first: &[bool],
    budget: TextBudget,
) -> Result<u32>
where
    S: RangedSource,
    W: SequentialSink,
    C: Cancellation,
{
    let header = reader.header();
    let loc = Location {
        variant: Some(header.variant),
        page: reader.current.map(|p| p.page.page_number),
        image: None,
        offset: reader.current.map_or(0, |p| p.page.text.offset),
    };
    let source_error = |source| {
        loc.error(ErrorKind::Source {
            field: "native page composition",
            source,
        })
    };
    if header.variant != Variant::C8 {
        return Err(loc.error(ErrorKind::Unsupported {
            field: "native page composition variant",
            value: 0,
        }));
    }
    let current = reader
        .current
        .ok_or_else(|| loc.error(ErrorKind::NoCurrentPage))?;
    if images.len() as u64 != u64::from(current.page.image_count) || top_first.len() != images.len()
    {
        return Err(source_error(invalid(
            "native page image resources differ from declared count",
        )));
    }
    let geometry = super::placement::source_page_geometry(
        header
            .page_size
            .ok_or_else(|| source_error(invalid("native page size is missing")))?,
    )
    .map_err(source_error)?;
    let origin = header
        .native_origin
        .ok_or_else(|| source_error(invalid("native page origin is missing")))?;
    let mut page = document
        .begin_content_page(geometry.size, fonts, images)
        .await
        .map_err(source_error)?;
    let mut writer = PageWriter {
        page: &mut page,
        roles,
        geometry,
        origin,
        top_first,
        image: 0,
        style: None,
        alternate: false,
    };
    reader.visit_native_records(budget, &mut writer).await?;
    page.finish().await.map_err(source_error)
}

fn invalid(reason: &'static str) -> Error {
    Error::InvalidInput { reason }
}

struct PageWriter<'p, 'd, 'a, 'r, W: SequentialSink, C: Cancellation> {
    page: &'p mut ContentPageWriter<'d, 'a, 'r, W, C>,
    roles: C8PageFonts,
    geometry: EmpiricalPageGeometry,
    origin: [u16; 2],
    top_first: &'r [bool],
    image: usize,
    style: Option<u16>,
    alternate: bool,
}

impl<W: SequentialSink, C: Cancellation> NativeRecordVisitor for PageWriter<'_, '_, '_, '_, W, C> {
    async fn visit(&mut self, _: u64, record: NativeRecord) -> crate::Result<()> {
        match record {
            NativeRecord::Control { tag: 0x8001, .. } => (), // y is carried by each glyph.
            NativeRecord::Control { tag: 0x8002, value } => self.style = Some(value),
            NativeRecord::Control {
                tag: 0x801d,
                value: 0,
            } => self.alternate = false,
            NativeRecord::Control {
                tag: 0x801d,
                value: 4,
            } => self.alternate = true,
            // Independently controlled ordinary resource combinations.
            NativeRecord::Control {
                tag: 0x8067,
                value: 5 | 6 | 8 | 9,
            } => (),
            NativeRecord::Glyph { x, y, style, code } => {
                let character = decode_native_character(code)
                    .ok_or_else(|| invalid("unsupported C8 native character"))?;
                let latin = if self.alternate {
                    self.roles.alternate_latin
                } else {
                    self.roles.latin
                };
                // Select by raw code: Unicode alone does not establish the
                // resource or placement of the source's symbol variants.
                let (class, font, baseline_fraction) = match code {
                    0xa0a6
                    | 0xa1aa
                    | 0xa1ad
                    | 0xa1ae
                    | 0xa2d9..=0xa2df
                    | 0xa3a3
                    | 0xa3a5
                    | 0xa3ab..=0xa3b9
                    | 0xa3bb..=0xa3bf
                    | 0xa3dc
                    | 0xa3fb
                    | 0xa3fd => (C8GlyphClass::Cjk, latin, Some(0.0)),
                    // These symbols retain the ordinary Latin resource even
                    // under the alternate-resource state.
                    0xa1c6 | 0xa1c8 | 0xa9aa | 0xaab3 | 0xaca3 => {
                        (C8GlyphClass::Cjk, self.roles.latin, Some(0.0))
                    }
                    0xa3ba => (C8GlyphClass::Cjk, latin, Some(1.0 / 8.0)),
                    _ if character.is_ascii_alphanumeric() => (C8GlyphClass::Latin, latin, None),
                    _ if ('\u{3400}'..='\u{9fff}').contains(&character) => {
                        (C8GlyphClass::Cjk, self.roles.cjk, None)
                    }
                    _ => return Err(invalid("unverified C8 glyph resource or placement class")),
                };
                let mut transform =
                    empirical_c8_glyph_transform(self.geometry, self.origin, [x, y], style, class)?;
                if let Some(fraction) = baseline_fraction {
                    // Controlled symbol baselines use independent em height;
                    // resource choice does not imply ordinary Latin geometry.
                    transform[5] += transform[3] * fraction
                        - 15.0 * super::EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
                }
                // Original source controls establish this gray for the admitted
                // ordinary text profile; keep it local to each glyph draw.
                self.page
                    .glyph_with_gray(font, character, transform, 68)
                    .await?;
            }
            NativeRecord::Drawing {
                tag: 0x8006,
                style,
                points,
            } => {
                let [from, to] = empirical_c8_segment(self.geometry, self.origin, points, style)?;
                self.page.segment(from, to, 0.0).await?;
            }
            NativeRecord::Drawing {
                tag: 0x8010,
                style: 1,
                points,
            } => {
                let (font, alias) = self
                    .roles
                    .decoration
                    .ok_or_else(|| invalid("missing C8 decoration font resource"))?;
                let style = self
                    .style
                    .ok_or_else(|| invalid("missing C8 decoration style"))?;
                let decoration =
                    empirical_c8_horizontal_decoration(self.geometry, self.origin, points, style)?;
                for index in 0..decoration.glyph_count {
                    let mut transform = decoration.first_glyph;
                    transform[4] += f64::from(index) * transform[0];
                    self.page
                        .decoration_glyph(font, alias, transform, decoration.clip)
                        .await?;
                }
            }
            NativeRecord::Image { words } => {
                let coordinate = decode_native_image_coordinate(&words)
                    .ok_or_else(|| invalid("unsupported C8 native image coordinates"))?;
                // Independent tail controls vary all eight low bytes without
                // changing the embedded image or its placement. Other high
                // prefixes remain unverified; never interpret this as a path.
                if words[5..].iter().any(|word| word & 0xff00 != 0xc000) {
                    return Err(invalid("unverified C8 native image payload"));
                }
                let unit = super::EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
                let x = (f64::from(coordinate.x) - f64::from(self.origin[0])) * unit;
                let y = self.geometry.size.height_points
                    - (f64::from(coordinate.y) - f64::from(self.origin[1])) * unit;
                let width = f64::from(coordinate.width) * unit;
                let height = f64::from(coordinate.height) * unit;
                let mut transform = [width, 0.0, 0.0, -height, x, y];
                if self.top_first[self.image] {
                    transform[3] = height;
                    transform[5] -= height;
                }
                self.page.image(self.image, transform).await?;
                self.image += 1;
            }
            NativeRecord::Control {
                tag: 0xffff,
                value: 5,
            }
            | NativeRecord::End { value: 1 } => (),
            _ => return Err(invalid("unverified C8 native rendering record")),
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) use tests::mixed_page;
#[cfg(test)]
mod tests;
