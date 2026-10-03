// SPDX-License-Identifier: MIT

//! Incremental composition of the independently controlled C8 native subset.

use super::{
    C8GlyphClass, EmpiricalPageGeometry, ErrorKind, Hnc8Reader, Location, NativeRecord,
    NativeRecordVisitor, Result, TextBudget, Variant, decode_native_character,
    decode_native_character_for_mode, decode_native_image_coordinate,
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
    /// Semantic symbols and spaces in the controlled HN-B mode-0 profile.
    /// Required only when a page uses that resource; never an implicit fallback.
    pub symbols: Option<usize>,
    /// Explicit HN-B Latin resource selected by `801d/3`.
    pub latin_state3: Option<usize>,
    /// Explicit C8 Latin resources selected by `801d/28` and `801d/31`.
    pub latin_state28: Option<usize>,
    pub latin_state31: Option<usize>,
}

/// Write and finish the current C8 or text/vector HN-B native page using already embedded resources.
///
/// Call `next_page` first. Images must be supplied in descriptor order, decoded
/// through the existing codecs. `top_first` identifies each emitted image's row
/// representation: existing JPEG/type-0 emitters use false; type-3 uses true.
/// Fonts are embedded once per document and may share resource indices.
///
/// Records are consumed one at a time; no glyph or page-content vector is kept.
/// This translation admits measured CJK/ASCII and symbol glyphs, segments,
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
    if !matches!(header.variant, Variant::C8 | Variant::HnB) {
        return Err(loc.error(ErrorKind::Unsupported {
            field: "native page composition variant",
            value: 0,
        }));
    }
    // Character mapping, font selection and placement are mode-specific.
    // Only HN-B has independently controlled mode-0 rendering records.
    if header.native_mode != Some(2)
        && !(header.variant == Variant::HnB && header.native_mode == Some(0))
    {
        return Err(loc.error(ErrorKind::Unsupported {
            field: "native page rendering mode",
            value: u64::from(header.native_mode.unwrap_or(u32::MAX)),
        }));
    }
    if header.variant == Variant::HnB && header.native_mode == Some(0) && !images.is_empty() {
        return Err(source_error(invalid(
            "unverified HN-B mode-0 image composition",
        )));
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
    let size = header
        .page_size
        .ok_or_else(|| source_error(invalid("native page size is missing")))?;
    let mut geometry = super::placement::source_page_geometry(size).map_err(source_error)?;
    let legacy = header.native_mode == Some(0);
    if legacy {
        // Add in source units before conversion, without overflowing u16 extents.
        let unit = super::EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
        geometry.size.width_points = (f64::from(size[0]) + 100.0) * unit;
        geometry.size.height_points = (f64::from(size[1]) + 100.0) * unit;
    }
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
        non_image_painted: false,
        style: None,
        axes: [None; 2],
        latin: roles.latin,
        skew: 0.0,
        gray: 68,
        cjk_mode: false,
        variant: header.variant,
        legacy,
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
    non_image_painted: bool,
    style: Option<u16>,
    axes: [Option<u16>; 2],
    latin: usize,
    skew: f64,
    gray: u8,
    cjk_mode: bool,
    variant: Variant,
    legacy: bool,
}

impl<W: SequentialSink, C: Cancellation> NativeRecordVisitor for PageWriter<'_, '_, '_, '_, W, C> {
    async fn visit(&mut self, _: u64, record: NativeRecord) -> crate::Result<()> {
        if self.legacy {
            return self.visit_mode_zero(record).await;
        }
        if matches!(
            record,
            NativeRecord::Glyph { .. } | NativeRecord::Drawing { .. }
        ) {
            self.non_image_painted = true;
        }
        match record {
            NativeRecord::Control { tag: 0x8001, .. } => (), // y is carried by each glyph.
            // The parser validates the bounded ASCII payload. Original mixed
            // controls preserve font state, glyphs, drawings and images; the
            // encoded value is not a resource path to open.
            NativeRecord::EncodedString { .. } if self.variant == Variant::C8 => (),
            // Independent mixed controls preserve both ordinary and CJK mode
            // across source, extreme and marker-like atomic payload values.
            NativeRecord::ExtendedControl {
                tag: 0x80cc,
                value: 0x0204,
                ..
            } if self.variant == Variant::C8 => (),
            // Original mixed controls establish black glyphs for this exact
            // payload, retained across later style/resource selections.
            NativeRecord::ExtendedControl {
                tag: 0x81ff,
                value: 1..=3,
                words: [0, 200],
            } if self.variant == Variant::C8 => self.gray = 0,
            NativeRecord::Control {
                tag: 0x8021,
                value: 0x2000,
            }
            | NativeRecord::Control {
                tag: 0x80d0 | 0x80d2,
                value: 0,
            }
            | NativeRecord::Control {
                tag: 0x80d1,
                value: 1,
            } if self.variant == Variant::C8 => (),
            NativeRecord::Control {
                tag: 0x8024,
                value: 0x2800,
            } => self.skew = 0.0,
            NativeRecord::Control {
                tag: 0x8024,
                value: 0x281d,
            } => self.skew = 0.24,
            NativeRecord::Control {
                tag: 0x8024,
                value: 0x2815,
            } if self.variant == Variant::HnB => self.skew = 0.105,
            NativeRecord::Control { tag: 0x8002, value } => {
                self.style = Some(value);
                self.axes = [None; 2];
            }
            NativeRecord::Control {
                tag: 0x801c,
                value: 4,
            } if self.variant == Variant::HnB => (),
            NativeRecord::Control {
                tag: 0x8070,
                value: value @ (28 | 43),
            } if self.variant == Variant::HnB => self.axes[0] = Some(value),
            NativeRecord::Control {
                tag: 0x8071,
                value: value @ (28 | 43),
            } if self.variant == Variant::HnB => self.axes[1] = Some(value),
            NativeRecord::Control {
                tag: 0x8070,
                value: 36,
            } => self.axes[0] = Some(36),
            NativeRecord::Control {
                tag: 0x8071,
                value: 36,
            } => self.axes[1] = Some(36),
            NativeRecord::Control {
                tag: 0x801d,
                value: 0,
            } => self.latin = self.roles.latin,
            NativeRecord::Control {
                tag: 0x801d,
                value: 4,
            } => self.latin = self.roles.alternate_latin,
            NativeRecord::Control {
                tag: 0x801d,
                value: 3,
            } if self.variant == Variant::HnB => {
                self.latin = self
                    .roles
                    .latin_state3
                    .ok_or_else(|| invalid("missing HN-B state-3 Latin font resource"))?;
            }
            NativeRecord::Control {
                tag: 0x801d,
                value: state @ (28 | 31),
            } if self.variant == Variant::C8 => {
                self.latin = if state == 28 {
                    self.roles.latin_state28
                } else {
                    self.roles.latin_state31
                }
                .ok_or_else(|| invalid("missing C8 extended-state Latin font resource"))?;
            }
            // Independently controlled ordinary resource combinations.
            NativeRecord::Control {
                tag: 0x8067,
                value: 5 | 6 | 8 | 9,
            } => (),
            // C8 zero mode persists across style/resource selections; one
            // restores the ordinary per-code resource and placement rules.
            NativeRecord::Control {
                tag: 0x80ce,
                value: 0,
            } if self.variant == Variant::C8 => self.cjk_mode = true,
            NativeRecord::Control {
                tag: 0x80ce,
                value: 1,
            } => self.cjk_mode = false,
            NativeRecord::Control {
                tag: 0x8072,
                value: 0 | 0x1042 | 0xa3a8 | 0xa0f2,
            }
            | NativeRecord::Control {
                tag: 0x8073,
                value: 38..=42,
            }
            | NativeRecord::Control {
                tag: 0x8074,
                value: 0 | 0xb4a2 | 0xd4b4 | 0x24a7 | 0xa1a1 | 0xa3a9,
            }
            | NativeRecord::Control {
                tag: 0xc053 | 0xc054,
                ..
            } => (),
            NativeRecord::Control {
                tag: 0x8067,
                value: 7,
            }
            | NativeRecord::Control {
                tag: 0x8069,
                value: 0x1084,
            }
            | NativeRecord::Control {
                tag: 0x8072,
                value: 0x1084 | 0xa0f3 | 0xa0e7 | 0xc2db | 0xd2f2 | 0xcdc1,
            }
            | NativeRecord::Control {
                tag: 0x8073,
                value: 30..=32 | 79..=83,
            }
            | NativeRecord::Control {
                tag: 0x8074,
                value: 0xb7bd | 0xcfc8 | 0xc8cb | 0x2815 | 0xa0ec | 0xd3c9 | 0xb0d7 | 0xd1e9,
            }
            | NativeRecord::ExtendedControl {
                tag: 0xc052,
                value: 0xa385,
                ..
            } if self.variant == Variant::HnB => (),
            NativeRecord::Glyph { x, y, style, code } => {
                // Original CJK/Latin/symbol pairs establish these regular flags
                // across fields 2..=8, including a rectangular held-out control.
                let style = if self.variant == Variant::HnB
                    && style & 0xfc00 == 0x0400
                    && (2..=8).contains(&((style >> 5) & 31))
                    && (2..=8).contains(&(style & 31))
                {
                    (style & 0x03ff) | 0x1000
                } else {
                    style
                };
                let character = decode_native_character(code)
                    .ok_or_else(|| invalid("unsupported C8 native character"))?;
                if style == 0x114a && self.variant != Variant::HnB {
                    return Err(invalid("unverified C8 title style"));
                }
                if matches!(style, 0xe58c | 0x114a | 0x154a)
                    && self.axes == [None; 2]
                    && !('㐀'..='鿿').contains(&character)
                {
                    return Err(invalid("unverified large native glyph class"));
                }
                let axis_offset = if self.variant == Variant::HnB {
                    match (self.axes, code) {
                        ([Some(43), Some(43)], 0xa3a8) => Some((27.0, -4.0)),
                        ([Some(43), Some(43)], 0xa1b0 | 0xa1b1 | 0xa3a9) => Some((25.0, -4.0)),
                        ([Some(43), Some(43)], 0xa1b6) => Some((30.0, -4.0)),
                        ([Some(43), Some(43)], 0xa1b7) => Some((20.0, -4.0)),
                        ([Some(43), Some(43)], 0xa1b2 | 0xa1b3) => Some((25.0, 4.0)),
                        ([Some(28), Some(28)], 0xa1b2 | 0xa1b3) => Some((16.0, 8.0)),
                        _ => None,
                    }
                } else {
                    None
                };
                if matches!(code, 0xa1b2 | 0xa1b3 | 0xa1b6 | 0xa1b7)
                    && axis_offset.is_none()
                    && (self.variant != Variant::HnB
                        || !(style == 0x10a5
                            || (matches!(style, 0x08a5 | 0x0ca5 | 0x1084 | 0x0884)
                                && matches!(code, 0xa1b2 | 0xa1b3)))
                        || self.axes != [None; 2])
                {
                    return Err(invalid("unverified native bracket geometry"));
                }
                if self.axes != [None; 2]
                    && axis_offset.is_none()
                    && matches!(
                        code,
                        0xa1a4 | 0xa1af | 0xa1b0 | 0xa1b1 | 0xa3a8 | 0xa3a9 | 0xa3db | 0xa3dd
                    )
                {
                    return Err(invalid("unverified explicit-axis punctuation offsets"));
                }
                let latin = self.latin;
                // Select by raw code: Unicode alone does not establish the
                // resource or placement of the source's symbol variants.
                let (class, font, baseline_fraction) = match code {
                    _ if self.cjk_mode => {
                        if !character.is_ascii_alphanumeric()
                            && !('\u{3400}'..='\u{9fff}').contains(&character)
                            && !matches!(code, 0xa3c1..=0xa3da | 0xa3e1..=0xa3fa)
                        {
                            return Err(invalid("unverified C8 CJK-mode glyph placement"));
                        }
                        (C8GlyphClass::Cjk, self.roles.cjk, None)
                    }
                    0xa0a6
                    | 0xa0ae
                    | 0xa0af
                    | 0xa0ba
                    | 0xaab1
                    | 0xaab2
                    | 0xa1aa
                    | 0xa1ad
                    | 0xa1ae
                    | 0xa1af
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
                    0xa0ad | 0xa3c0 if self.variant == Variant::HnB => {
                        (C8GlyphClass::Cjk, latin, Some(0.0))
                    }
                    0xa1a4 | 0xa3ba => (C8GlyphClass::Cjk, latin, Some(1.0 / 8.0)),
                    0xa1b0 | 0xa1b1 | 0xa1b2 | 0xa1b3 | 0xa1b6 | 0xa1b7 | 0xa3a8 | 0xa3a9 => {
                        (C8GlyphClass::Cjk, latin, None)
                    }
                    0xa3db | 0xa3dd => (C8GlyphClass::Cjk, self.roles.latin, None),
                    0xa1a1 => (C8GlyphClass::Cjk, self.roles.cjk, None),
                    0xa3c1..=0xa3da | 0xa3e1..=0xa3fa if self.variant == Variant::C8 => {
                        (C8GlyphClass::Cjk, self.roles.cjk, None)
                    }
                    0xa1a2 => (C8GlyphClass::Latin, latin, None),
                    0xa1a3 if self.variant == Variant::HnB => (C8GlyphClass::Latin, latin, None),
                    _ if character.is_ascii_alphanumeric() => (C8GlyphClass::Latin, latin, None),
                    _ if ('\u{3400}'..='\u{9fff}').contains(&character) => {
                        (C8GlyphClass::Cjk, self.roles.cjk, None)
                    }
                    _ => return Err(invalid("unverified C8 glyph resource or placement class")),
                };
                let mut transform = super::placement::native_glyph_transform(
                    self.geometry,
                    self.origin,
                    [x, y],
                    style,
                    class,
                    self.axes,
                )?;
                if let Some(fraction) = baseline_fraction {
                    // Controlled symbol baselines use independent em height;
                    // resource choice does not imply ordinary Latin geometry.
                    transform[5] += transform[3] * fraction
                        - 15.0 * super::EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
                }
                if matches!(code, 0xa1a2 | 0xa1a3) {
                    // Verified ideographic punctuation shares the Latin baseline,
                    // but retains the CJK horizontal origin.
                    transform[4] -= transform[0] / 8.0;
                }
                if let Some((dx, dy)) = axis_offset {
                    let unit = super::EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
                    transform[4] += dx * unit;
                    transform[5] -= dy * unit;
                } else {
                    if matches!(code, 0xa1b2 | 0xa1b3) {
                        // Original pairs establish separate size-4/size-5 offsets;
                        // resource selection remains independent of placement.
                        let unit = super::EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
                        let (dx, dy) = if style & 0x03ff == 0x0084 {
                            (21.0, 6.0)
                        } else {
                            (25.0, 5.0)
                        };
                        transform[4] += dx * unit;
                        transform[5] -= dy * unit;
                    }
                    let offset_columns = match code {
                        0xa1b6 | 0xa1b7 | 0xa3a8 => Some((0, 2)),
                        0xa1b0 | 0xa1b1 | 0xa3a9 => Some((1, 2)),
                        0xa3db | 0xa3dd => Some((3, 4)),
                        _ => None,
                    };
                    if let Some((x_column, y_column)) = offset_columns {
                        // Raw offsets checked with independent width/height controls.
                        // Columns: opening-parenthesis x, closing-parenthesis x,
                        // parenthesis downward y, square-bracket x and downward y.
                        const OFFSETS: [[i16; 5]; 7] = [
                            [18, 16, 3, 24, 1],
                            [19, 18, 1, 27, -1],
                            [22, 21, 0, 30, -3],
                            [26, 25, -4, 36, -7],
                            [30, 28, -7, 41, -10],
                            [35, 33, -10, 48, -15],
                            [39, 37, -14, 54, -18],
                        ];
                        // The matrix evaluator above has validated both fields.
                        let width = usize::from((style >> 5) & 31) - 2;
                        let height = usize::from(style & 31) - 2;
                        let unit = super::EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
                        // Controlled HN-B style-5 book marks share parenthesis y;
                        // their x differs by +4/-6 source units from the opener.
                        let book_x = match code {
                            0xa1b6 => 4,
                            0xa1b7 => -6,
                            _ => 0,
                        };
                        transform[4] += f64::from(OFFSETS[width][x_column] + book_x) * unit;
                        transform[5] -= f64::from(OFFSETS[height][y_column]) * unit;
                    }
                    if matches!(code, 0xa1a4 | 0xa1af) {
                        // Middle dot and right single quote share a horizontal
                        // correction but keep their separately controlled baselines.
                        const X_OFFSETS: [i16; 7] = [7, 7, 8, 10, 11, 13, 15];
                        let width = usize::from((style >> 5) & 31) - 2;
                        transform[4] += f64::from(X_OFFSETS[width])
                            * super::EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
                    }
                }
                if self.skew != 0.0 {
                    // Measured width-relative shear; style changes retain it.
                    transform[2] = transform[0] * self.skew;
                }
                // Keep the verified current gray local to each glyph draw.
                self.page
                    .glyph_with_gray(font, character, transform, self.gray)
                    .await?;
            }
            NativeRecord::Drawing { .. } | NativeRecord::Image { .. } if self.skew != 0.0 => {
                return Err(invalid("unverified drawing or image in skewed text state"));
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
                if self.axes != [None; 2] {
                    return Err(invalid("unverified explicit-axis decoration"));
                }
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
                // HN-B controls establish opaque leading images. Images after
                // text can use a different raster operation and remain explicit.
                if self.variant == Variant::HnB && self.non_image_painted {
                    return Err(invalid("unverified HN-B image after text or drawing"));
                }
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
            | NativeRecord::End { .. } => (),
            _ => return Err(invalid("unverified C8 native rendering record")),
        }
        Ok(())
    }
}

impl<W: SequentialSink, C: Cancellation> PageWriter<'_, '_, '_, '_, W, C> {
    async fn visit_mode_zero(&mut self, record: NativeRecord) -> crate::Result<()> {
        match record {
            NativeRecord::Control { tag: 0x8001, .. }
            // Original paired rows preserve resources, geometry and explicit axes.
            | NativeRecord::Control { tag: 0x8072, value: 0 | 0xc2c7 }
            | NativeRecord::Control { tag: 0x8073, value: 41..=43 }
            | NativeRecord::Control { tag: 0x8074, value: 0xc8ce | 0xb5c8 | 0xb5c4 }
            | NativeRecord::Control { tag: 0xc053, .. }
            | NativeRecord::Control { tag: 0x80ce, value: 1 }
            | NativeRecord::Control {
                tag: 0x801d,
                value: 0 | 4,
            }
            | NativeRecord::Control {
                tag: 0x8067,
                value: 6,
            }
            | NativeRecord::Control {
                tag: 0xffff,
                value: 5,
            }
            | NativeRecord::End { .. } => (),
            NativeRecord::Control { tag: 0x8002, .. } => self.axes = [None; 2],
            NativeRecord::Control {
                tag: 0x8070,
                value: 36,
            } => self.axes[0] = Some(36),
            NativeRecord::Control {
                tag: 0x8071,
                value: 36,
            } => self.axes[1] = Some(36),
            NativeRecord::Drawing {
                tag: 0x8006,
                style: 0xa385,
                points,
            } => {
                // Both mode-0 endpoints can carry the high-bit marker. The
                // controlled y origin is five units above the mode-2 segment.
                let points =
                    points.map(|[x, y]| [if x & 0xc000 == 0xc000 { x & 0x3fff } else { x }, y]);
                let mut ends = empirical_c8_segment(self.geometry, self.origin, points, 0xa385)?;
                for end in &mut ends {
                    end[1] += 5.0 * super::EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
                }
                self.page.segment(ends[0], ends[1], 0.0).await?;
            }
            NativeRecord::Glyph { x, y, style, code } => {
                let character = decode_native_character_for_mode(0, code)
                    .ok_or_else(|| invalid("unsupported HN-B mode-0 character"))?;
                // The raw alphabet selects its resource independently of 801d.
                // Digits have a separate matrix; remaining symbols need another resource.
                let (class, font) = match code {
                    0x9ff5
                    | 0xa1a1..=0xa1a3
                    | 0xa1aa
                    | 0xa1ae..=0xa1b1
                    | 0xa3a7
                    | 0xa3ab..=0xa3ae
                    | 0xa3ba
                    | 0xa3bb
                    | 0xa3bf
                    | 0xa3db
                    | 0xa3dd
                    | 0xaab1
                    | 0xaab2 => {
                        let class = if matches!(code, 0xa3ba | 0xa3db | 0xa3dd) {
                            C8GlyphClass::Cjk
                        } else {
                            C8GlyphClass::Latin
                        };
                        (
                            class,
                            self.roles.symbols.ok_or_else(|| {
                                invalid("missing HN-B mode-0 symbol font resource")
                            })?,
                        )
                    }
                    0xa3a8 | 0xa3a9 => (C8GlyphClass::Cjk, self.roles.latin),
                    0xa3af => (C8GlyphClass::Cjk, self.roles.cjk),
                    0xa980..=0xa9b3 => (C8GlyphClass::Latin, self.roles.alternate_latin),
                    0xa3b0..=0xa3b9 | 0xa3c1..=0xa3da | 0xa3e1..=0xa3fa => {
                        (C8GlyphClass::Latin, self.roles.latin)
                    }
                    // The mode-0 decoder accepts only Han after the explicit
                    // alphabet and symbol classes handled above.
                    _ => (C8GlyphClass::Cjk, self.roles.cjk),
                };
                let mut transform = if (0xa3b0..=0xa3b9).contains(&code) {
                    super::placement::mode_zero_digit_transform(
                        self.geometry,
                        self.origin,
                        [x, y],
                        style,
                        self.axes,
                    )?
                } else {
                    super::placement::mode_zero_glyph_transform(
                        self.geometry,
                        self.origin,
                        [x, y],
                        style,
                        class,
                        self.axes,
                    )?
                };
                if code == 0xaab2 {
                    let left = match (style & 31, self.axes) {
                        (_, [Some(36), Some(36)]) | (4, [None, None]) => 33.0,
                        (0, [None, None]) => 31.0,
                        (5, [None, None]) => 34.0,
                        _ => return Err(invalid("unverified HN-B mode-0 hyphen geometry")),
                    };
                    transform[4] -= left * super::EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
                }
                self.page
                    .glyph_with_gray(font, character, transform, 68)
                    .await?;
            }
            _ => return Err(invalid("unverified HN-B mode-0 rendering record")),
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) use tests::mixed_page;
#[cfg(test)]
mod tests;
