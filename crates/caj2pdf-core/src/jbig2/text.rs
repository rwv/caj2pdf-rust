// SPDX-License-Identifier: MIT

//! Bounded T.88 text-region segment data headers (§§7.4.1, 7.4.3.1).
//!
//! This parser validates segment framing, the region information field, the
//! Figure 36 flags, optional Huffman and refinement AT fields, and
//! `SBNUMINSTANCES`, then exposes the remaining body as an exact source range.
//! It never reads the body, decodes symbol instances, or composes pixels.

use super::{FieldCursor, FieldFault, SegmentHeader, SegmentSpan, unsupported};
use crate::{Cancellation, Context, Error, Limits, RangedSource, Result};

/// Immediate text region segment type (T.88 §7.3).
pub const IMMEDIATE_TEXT_REGION: u8 = 6;
const INTERMEDIATE_TEXT_REGION: u8 = 4;
const IMMEDIATE_LOSSLESS_TEXT_REGION: u8 = 7;
const REGION_INFO_BYTES: usize = 17;
const FLAGS_BYTES: usize = 2;
/// The fixed prefix read before optional fields: region information and flags.
const PREFIX_BYTES: usize = REGION_INFO_BYTES + FLAGS_BYTES;

/// External combination operator of a region with its page (§7.4.1.5).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegionCombination {
    Or,
    And,
    Xor,
    Xnor,
    Replace,
}

/// Region segment information field (§7.4.1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegionInfo {
    pub width: u32,
    pub height: u32,
    pub x: u32,
    pub y: u32,
    pub combination: RegionCombination,
}

/// Symbol instance reference corner (`REFCORNER`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReferenceCorner {
    BottomLeft,
    TopLeft,
    BottomRight,
    TopRight,
}

/// Symbol combination operator inside the region (`SBCOMBOP`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SymbolCombination {
    Or,
    And,
    Xor,
    Xnor,
}

/// Decoded Figure 36 text region flags. `raw` keeps the encoded value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextRegionFlags {
    pub raw: u16,
    /// `SBHUFF`: Huffman rather than arithmetic coding.
    pub huffman: bool,
    /// `SBREFINE`: symbol instances may be refined.
    pub refine: bool,
    /// `LOGSBSTRIPS`; `SBSTRIPS` is `1 << log_strips`.
    pub log_strips: u8,
    pub reference_corner: ReferenceCorner,
    pub transposed: bool,
    pub combination: SymbolCombination,
    /// `SBDEFPIXEL`: initial value of every region pixel.
    pub default_pixel: bool,
    /// Signed five-bit `SBDSOFFSET`, in `-16..=15`.
    pub ds_offset: i8,
    /// `SBRTEMPLATE`; zero whenever `refine` is false.
    pub refinement_template: u8,
}

impl TextRegionFlags {
    /// `SBSTRIPS`: 1, 2, 4, or 8.
    pub fn strips(self) -> u8 {
        1 << self.log_strips
    }
}

/// Caller-selected validation for a documented HN/C8 header defect.
/// The default parser always uses `Strict`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TextHeaderPolicy {
    #[default]
    Strict,
    /// Accept only raw flags `0xa40c`: an unused `SBRTEMPLATE` bit with
    /// `SBREFINE=0`. All other header validation remains strict.
    HnC8UnusedRefinementTemplate,
}

/// A nonconforming header accepted under an explicit caller policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextHeaderAnomaly {
    UnusedRefinementTemplate,
}

/// Parsed text region data header. `body` is an exact absolute source range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextRegionHeader {
    /// The immediate text segment and its single dictionary reference.
    pub segment: u32,
    pub page_association: u32,
    pub dictionary_segment: u32,
    pub region: RegionInfo,
    pub flags: TextRegionFlags,
    /// `None` for strictly valid headers; retains an accepted deviation.
    pub anomaly: Option<TextHeaderAnomaly>,
    /// Figure 37 Huffman table selections, present only when `SBHUFF` is 1.
    pub huffman_flags: Option<u16>,
    /// `SBRATX1, SBRATY1, SBRATX2, SBRATY2`, present only when `SBREFINE` is 1
    /// and `SBRTEMPLATE` is 0.
    pub refinement_at: Option<[(i8, i8); 2]>,
    /// `SBNUMINSTANCES`.
    pub instances: u32,
    pub header_bytes: u64,
    pub body: SegmentSpan,
}

impl TextRegionHeader {
    /// The first legal configuration outside the arithmetic, refinement
    /// template 1 (or no refinement) profile that later decoders target.
    pub fn unsupported_feature(&self) -> Option<(&'static str, u64)> {
        if let Some(flags) = self.huffman_flags {
            Some(("Huffman text region", u64::from(flags)))
        } else if self.refinement_at.is_some() {
            Some(("refinement template 0 with adaptive pixels", 0))
        } else {
            None
        }
    }
}

struct Cursor<'a, S, C> {
    source: &'a mut S,
    cancellation: &'a C,
    segment: u32,
    fields: FieldCursor,
}

impl<S: RangedSource, C: Cancellation> Cursor<'_, S, C> {
    fn error_at(&self, offset: u64, error: Error) -> Error {
        error.or_at(
            offset,
            Context::Jbig2 {
                segment: Some(self.segment),
            },
        )
    }

    fn error(&self, error: Error) -> Error {
        self.error_at(self.fields.at, error)
    }

    /// Read exactly `N` header bytes with bounded requests.
    fn read<const N: usize>(&mut self, field: &'static str) -> Result<[u8; N]> {
        let mut bytes = [0u8; N];
        let result = self.fields.fill(self.source, self.cancellation, &mut bytes);
        result.map_err(|fault| self.fault(fault, field))?;
        Ok(bytes)
    }

    /// A field whose end overflows is truncated, as one past the end is.
    fn fault(&self, fault: FieldFault, field: &'static str) -> Error {
        let at = self.fields.at;
        self.error(match fault {
            FieldFault::Overflow { expected } => {
                Error::truncated(at, expected, self.fields.end.saturating_sub(at)).because(field)
            }
            fault => fault.error(at, field),
        })
    }
}

fn be32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// Parse one text region segment data header within `header.data`.
///
/// `dictionary` is the validated header of the single symbol dictionary this
/// region refers to. Framing, reference, page, and span checks complete before
/// any source read; the returned `body` range is never read or allocated.
/// The region's pixels and its `SBNUMINSTANCES` are each bounded by
/// `Limits::max_image_pixels`.
pub fn read_text_region_header<S: RangedSource, C: Cancellation>(
    source: &mut S,
    header: &SegmentHeader,
    dictionary: &SegmentHeader,
    limits: &Limits,
    cancellation: &C,
) -> Result<TextRegionHeader> {
    read_text_region_header_with_policy(
        source,
        header,
        dictionary,
        limits,
        cancellation,
        TextHeaderPolicy::Strict,
    )
}

/// Parse a text-region header with an explicit interoperability policy.
/// `HnC8UnusedRefinementTemplate` accepts only raw `0xa40c`; it does not
/// normalize the returned flags or skip any framing, size, or body checks.
pub fn read_text_region_header_with_policy<S: RangedSource, C: Cancellation>(
    source: &mut S,
    header: &SegmentHeader,
    dictionary: &SegmentHeader,
    limits: &Limits,
    cancellation: &C,
    policy: TextHeaderPolicy,
) -> Result<TextRegionHeader> {
    let fail = |error: Error| {
        error.or_at(
            header.data.offset,
            Context::Jbig2 {
                segment: Some(header.number),
            },
        )
    };
    if cancellation.is_cancelled() {
        return Err(fail(Error::cancelled()));
    }
    match header.segment_type {
        IMMEDIATE_TEXT_REGION => {}
        INTERMEDIATE_TEXT_REGION | IMMEDIATE_LOSSLESS_TEXT_REGION => {
            return Err(fail(unsupported("text region segment type")));
        }
        _ => {
            return Err(fail(Error::invalid("segment type is not a text region")));
        }
    }
    if header.page_association == 0 {
        return Err(fail(Error::invalid("immediate region without a page")));
    }
    if header.referred_to.len() != 1 {
        return Err(fail(unsupported("text region reference count")));
    }
    if header.referred_to[0] != dictionary.number {
        return Err(fail(Error::invalid(
            "reference differs from the supplied dictionary",
        )));
    }
    if dictionary.segment_type != 0 {
        return Err(fail(Error::invalid(
            "referred segment is not a symbol dictionary",
        )));
    }
    if dictionary.number >= header.number {
        return Err(fail(Error::invalid("dictionary does not precede region")));
    }
    if dictionary.page_association != 0 && dictionary.page_association != header.page_association {
        return Err(fail(Error::invalid("dictionary page association differs")));
    }
    if header.data.length > limits.max_input_bytes {
        return Err(fail(Error::limit(
            "text region data bytes",
            limits.max_input_bytes,
            header.data.length,
        )));
    }
    if header.header_length > header.data.offset {
        return Err(fail(Error::invalid("segment header start underflow")));
    }
    let end = header
        .data
        .offset
        .checked_add(header.data.length)
        .ok_or_else(|| fail(Error::invalid("data end overflow")))?;
    if end > source.size() {
        return Err(fail(Error::invalid("data outside source")));
    }

    let mut cursor = Cursor {
        source,
        cancellation,
        segment: header.number,
        fields: FieldCursor {
            start: header.data.offset,
            at: header.data.offset,
            end,
            fetched: 0,
            request_bytes: limits.io_chunk_bytes.max(1),
        },
    };
    let start = header.data.offset;
    let prefix: [u8; PREFIX_BYTES] = cursor.read("text region header")?;
    let region = parse_region(&prefix, &cursor, limits)?;
    let (flags, anomaly) = parse_flags(
        u16::from_be_bytes([prefix[REGION_INFO_BYTES], prefix[REGION_INFO_BYTES + 1]]),
        policy,
    )
    .map_err(|error| cursor.error_at(start + REGION_INFO_BYTES as u64, error))?;

    let huffman_flags = if flags.huffman {
        let offset = cursor.fields.at;
        let raw = u16::from_be_bytes(cursor.read("text region Huffman flags")?);
        check_huffman_flags(raw, flags.refine).map_err(|error| cursor.error_at(offset, error))?;
        Some(raw)
    } else {
        None
    };
    let refinement_at = if flags.refine && flags.refinement_template == 0 {
        let [x1, y1, x2, y2] = cursor.read("text region refinement AT")?;
        Some([(x1 as i8, y1 as i8), (x2 as i8, y2 as i8)])
    } else {
        None
    };
    let instances_offset = cursor.fields.at;
    let instances = u32::from_be_bytes(cursor.read("SBNUMINSTANCES")?);
    if u64::from(instances) > limits.max_image_pixels {
        return Err(cursor.error_at(
            instances_offset,
            Error::limit(
                "text region symbol instances",
                limits.max_image_pixels,
                u64::from(instances),
            ),
        ));
    }
    let body_length = end - cursor.fields.at;
    if !flags.huffman && body_length < 2 {
        return Err(cursor.error(
            Error::truncated(cursor.fields.at, 2, body_length).because("MQ body terminal pair"),
        ));
    }
    Ok(TextRegionHeader {
        segment: header.number,
        page_association: header.page_association,
        dictionary_segment: header.referred_to[0],
        region,
        flags,
        anomaly,
        huffman_flags,
        refinement_at,
        instances,
        header_bytes: cursor.fields.at - start,
        body: SegmentSpan {
            offset: cursor.fields.at,
            length: body_length,
        },
    })
}

fn parse_region<S: RangedSource, C: Cancellation>(
    bytes: &[u8; PREFIX_BYTES],
    cursor: &Cursor<'_, S, C>,
    limits: &Limits,
) -> Result<RegionInfo> {
    let start = cursor.fields.start;
    let (width, height) = (be32(&bytes[0..4]), be32(&bytes[4..8]));
    let (x, y) = (be32(&bytes[8..12]), be32(&bytes[12..16]));
    let flags_offset = start + 16;
    if bytes[16] & 0xf8 != 0 {
        return Err(cursor.error_at(
            flags_offset,
            Error::invalid("reserved region segment flags"),
        ));
    }
    let combination = match bytes[16] & 7 {
        0 => RegionCombination::Or,
        1 => RegionCombination::And,
        2 => RegionCombination::Xor,
        3 => RegionCombination::Xnor,
        4 => RegionCombination::Replace,
        _ => {
            return Err(
                cursor.error_at(flags_offset, Error::invalid("region combination operator"))
            );
        }
    };
    for (attempted, offset) in [(width, start), (height, start + 4)] {
        if attempted == 0 {
            return Err(cursor.error_at(offset, unsupported("empty text region")));
        }
    }
    let pixels = u64::from(width) * u64::from(height);
    if pixels > limits.max_image_pixels {
        return Err(cursor.error_at(
            start,
            Error::limit("text region pixels", limits.max_image_pixels, pixels),
        ));
    }
    Ok(RegionInfo {
        width,
        height,
        x,
        y,
        combination,
    })
}

fn parse_flags(
    raw: u16,
    policy: TextHeaderPolicy,
) -> Result<(TextRegionFlags, Option<TextHeaderAnomaly>)> {
    let refine = raw & 2 != 0;
    let refinement_template = (raw >> 15) as u8;
    let anomaly = if !refine && refinement_template != 0 {
        if policy == TextHeaderPolicy::HnC8UnusedRefinementTemplate && raw == 0xa40c {
            Some(TextHeaderAnomaly::UnusedRefinementTemplate)
        } else {
            return Err(Error::invalid("SBRTEMPLATE without SBREFINE"));
        }
    } else {
        None
    };
    // Sign-extend the five-bit SBDSOFFSET in bits 10-14.
    let ds_offset = (((raw >> 10) & 0x1f) as i8) << 3 >> 3;
    Ok((
        TextRegionFlags {
            raw,
            huffman: raw & 1 != 0,
            refine,
            log_strips: ((raw >> 2) & 3) as u8,
            reference_corner: match (raw >> 4) & 3 {
                0 => ReferenceCorner::BottomLeft,
                1 => ReferenceCorner::TopLeft,
                2 => ReferenceCorner::BottomRight,
                _ => ReferenceCorner::TopRight,
            },
            transposed: raw & 0x40 != 0,
            combination: match (raw >> 7) & 3 {
                0 => SymbolCombination::Or,
                1 => SymbolCombination::And,
                2 => SymbolCombination::Xor,
                _ => SymbolCombination::Xnor,
            },
            default_pixel: raw & 0x200 != 0,
            ds_offset,
            refinement_template,
        },
        anomaly,
    ))
}

/// Figure 37 constraints: bit 15 reserved, selector value 2 forbidden for
/// `SBHUFFFS` and the refinement tables, which must be zero without refinement.
fn check_huffman_flags(raw: u16, refine: bool) -> Result<()> {
    let malformed = |field| Err(Error::invalid(field));
    if raw & 0x8000 != 0 {
        return malformed("reserved Huffman flag");
    }
    if [0, 6, 8, 10, 12]
        .iter()
        .any(|shift| (raw >> shift) & 3 == 2)
    {
        return malformed("reserved Huffman table selector");
    }
    if !refine && raw & 0x7fc0 != 0 {
        return malformed("refinement Huffman tables without SBREFINE");
    }
    Ok(())
}
