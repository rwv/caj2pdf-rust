// SPDX-License-Identifier: MIT

//! Bounded T.88 text-region segment data headers (§§7.4.1, 7.4.3.1).
//!
//! This parser validates segment framing, the region information field, the
//! Figure 36 flags, optional Huffman and refinement AT fields, and
//! `SBNUMINSTANCES`, then exposes the remaining body as an exact source range.
//! It never reads the body, decodes symbol instances, or composes pixels.

use super::{SegmentHeader, SegmentSpan};
use crate::{Cancellation, Error, Limits, RangedSource};
use std::{error, fmt};

/// Immediate text region segment type (T.88 §7.3).
pub const IMMEDIATE_TEXT_REGION: u8 = 6;
const INTERMEDIATE_TEXT_REGION: u8 = 4;
const IMMEDIATE_LOSSLESS_TEXT_REGION: u8 = 7;
const REGION_INFO_BYTES: usize = 17;
const FLAGS_BYTES: usize = 2;
/// The fixed prefix read before optional fields: region information and flags.
const PREFIX_BYTES: usize = REGION_INFO_BYTES + FLAGS_BYTES;

/// Resource bounds for one text-region header, in addition to `Limits`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextRegionBudget {
    /// Region information, flags, optional fields, and `SBNUMINSTANCES`.
    pub max_data_header_bytes: u64,
    pub max_body_bytes: u64,
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels: u64,
    pub max_instances: u32,
    pub max_source_request_bytes: usize,
}

impl Default for TextRegionBudget {
    fn default() -> Self {
        Self {
            max_data_header_bytes: 64,
            max_body_bytes: 64 * 1024 * 1024,
            max_width: 65_536,
            max_height: 65_536,
            max_pixels: 256 * 1024 * 1024,
            max_instances: 1_000_000,
            max_source_request_bytes: 256,
        }
    }
}

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

/// Parsed text region data header. `body` is an exact absolute source range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextRegionHeader {
    pub region: RegionInfo,
    pub flags: TextRegionFlags,
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

/// A located text-region header failure. `offset` is the semantic field
/// position; `bytes_fetched` counts physical source bytes read so far.
#[derive(Debug)]
pub struct TextRegionError {
    pub segment: u32,
    pub offset: u64,
    pub bytes_fetched: u64,
    pub kind: TextRegionErrorKind,
}

#[derive(Debug)]
pub enum TextRegionErrorKind {
    InvalidSpan(&'static str),
    Truncated(&'static str),
    Malformed(&'static str),
    /// A flags field violates a T.88 constraint; the raw value is retained.
    MalformedFlags {
        field: &'static str,
        raw: u16,
    },
    Unsupported {
        feature: &'static str,
        value: u64,
    },
    LimitExceeded {
        resource: &'static str,
        limit: u64,
        attempted: u64,
    },
    Cancelled,
    Source(Error),
}

pub type TextRegionResult<T> = Result<T, TextRegionError>;

impl fmt::Display for TextRegionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "JBIG2 text region segment {} at source byte {}: ",
            self.segment, self.offset
        )?;
        match &self.kind {
            TextRegionErrorKind::InvalidSpan(reason) => write!(f, "invalid span: {reason}"),
            TextRegionErrorKind::Truncated(field) => write!(f, "truncated {field}"),
            TextRegionErrorKind::Malformed(reason) => write!(f, "malformed {reason}"),
            TextRegionErrorKind::MalformedFlags { field, raw } => {
                write!(f, "malformed {field} (flags {raw:#06x})")
            }
            TextRegionErrorKind::Unsupported { feature, value } => {
                write!(f, "unsupported {feature} ({value})")
            }
            TextRegionErrorKind::LimitExceeded {
                resource,
                limit,
                attempted,
            } => write!(f, "{resource} limit {limit} exceeded by {attempted}"),
            TextRegionErrorKind::Cancelled => f.write_str("cancelled"),
            TextRegionErrorKind::Source(source) => write!(f, "source: {source}"),
        }
    }
}

impl error::Error for TextRegionError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.kind {
            TextRegionErrorKind::Source(source) => Some(source),
            _ => None,
        }
    }
}

struct Cursor<'a, S, C> {
    source: &'a mut S,
    cancellation: &'a C,
    segment: u32,
    start: u64,
    at: u64,
    end: u64,
    fetched: u64,
    request_bytes: usize,
    max_header_bytes: u64,
}

impl<S: RangedSource, C: Cancellation> Cursor<'_, S, C> {
    fn error_at(&self, offset: u64, kind: TextRegionErrorKind) -> TextRegionError {
        TextRegionError {
            segment: self.segment,
            offset,
            bytes_fetched: self.fetched,
            kind,
        }
    }

    fn error(&self, kind: TextRegionErrorKind) -> TextRegionError {
        self.error_at(self.at, kind)
    }

    fn check_cancelled(&self) -> TextRegionResult<()> {
        if self.cancellation.is_cancelled() {
            Err(self.error(TextRegionErrorKind::Cancelled))
        } else {
            Ok(())
        }
    }

    /// Read exactly `N` header bytes with bounded requests.
    async fn read<const N: usize>(&mut self, field: &'static str) -> TextRegionResult<[u8; N]> {
        // `at <= end` and `end` fits u64, so neither sum can overflow.
        let attempted = self.at - self.start + N as u64;
        if attempted > self.max_header_bytes {
            return Err(self.error(TextRegionErrorKind::LimitExceeded {
                resource: "text region header bytes",
                limit: self.max_header_bytes,
                attempted,
            }));
        }
        if self.at + N as u64 > self.end {
            return Err(self.error(TextRegionErrorKind::Truncated(field)));
        }
        let mut bytes = [0u8; N];
        let mut done = 0;
        while done < N {
            self.check_cancelled()?;
            let request = (N - done).min(self.request_bytes);
            let got = match self
                .source
                .read_at(self.at, &mut bytes[done..done + request])
                .await
            {
                Ok(got) => got,
                Err(Error::Cancelled) => return Err(self.error(TextRegionErrorKind::Cancelled)),
                Err(error) => return Err(self.error(TextRegionErrorKind::Source(error))),
            };
            if got > request {
                return Err(self.error(TextRegionErrorKind::Malformed("source read length")));
            }
            if got == 0 {
                return Err(self.error(TextRegionErrorKind::Truncated(field)));
            }
            self.at += got as u64;
            self.fetched += got as u64;
            done += got;
        }
        self.check_cancelled()?;
        Ok(bytes)
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
pub async fn read_text_region_header<S: RangedSource, C: Cancellation>(
    source: &mut S,
    header: &SegmentHeader,
    dictionary: &SegmentHeader,
    limits: &Limits,
    budget: TextRegionBudget,
    cancellation: &C,
) -> TextRegionResult<TextRegionHeader> {
    let fail = |kind| TextRegionError {
        segment: header.number,
        offset: header.data.offset,
        bytes_fetched: 0,
        kind,
    };
    limits
        .validate()
        .map_err(|e| fail(TextRegionErrorKind::Source(e)))?;
    if budget.max_source_request_bytes == 0 {
        return Err(fail(TextRegionErrorKind::Malformed(
            "zero I/O request bound",
        )));
    }
    if cancellation.is_cancelled() {
        return Err(fail(TextRegionErrorKind::Cancelled));
    }
    match header.segment_type {
        IMMEDIATE_TEXT_REGION => {}
        INTERMEDIATE_TEXT_REGION | IMMEDIATE_LOSSLESS_TEXT_REGION => {
            return Err(fail(TextRegionErrorKind::Unsupported {
                feature: "text region segment type",
                value: u64::from(header.segment_type),
            }));
        }
        _ => {
            return Err(fail(TextRegionErrorKind::Malformed(
                "segment type is not a text region",
            )));
        }
    }
    if header.page_association == 0 {
        return Err(fail(TextRegionErrorKind::Malformed(
            "immediate region without a page",
        )));
    }
    if header.referred_to.len() != 1 {
        return Err(fail(TextRegionErrorKind::Unsupported {
            feature: "text region reference count",
            value: header.referred_to.len() as u64,
        }));
    }
    if header.referred_to[0] != dictionary.number {
        return Err(fail(TextRegionErrorKind::Malformed(
            "reference differs from the supplied dictionary",
        )));
    }
    if dictionary.segment_type != 0 {
        return Err(fail(TextRegionErrorKind::Malformed(
            "referred segment is not a symbol dictionary",
        )));
    }
    if dictionary.number >= header.number {
        return Err(fail(TextRegionErrorKind::Malformed(
            "dictionary does not precede region",
        )));
    }
    if dictionary.page_association != 0 && dictionary.page_association != header.page_association {
        return Err(fail(TextRegionErrorKind::Malformed(
            "dictionary page association differs",
        )));
    }
    if header.data.length > limits.max_input_bytes {
        return Err(fail(TextRegionErrorKind::LimitExceeded {
            resource: "text region data bytes",
            limit: limits.max_input_bytes,
            attempted: header.data.length,
        }));
    }
    if header.header_length > header.data.offset {
        return Err(fail(TextRegionErrorKind::InvalidSpan(
            "segment header start underflow",
        )));
    }
    let end = header
        .data
        .offset
        .checked_add(header.data.length)
        .ok_or_else(|| fail(TextRegionErrorKind::InvalidSpan("data end overflow")))?;
    if end > source.size() {
        return Err(fail(TextRegionErrorKind::InvalidSpan(
            "data outside source",
        )));
    }

    let mut cursor = Cursor {
        source,
        cancellation,
        segment: header.number,
        start: header.data.offset,
        at: header.data.offset,
        end,
        fetched: 0,
        request_bytes: budget.max_source_request_bytes.min(limits.io_chunk_bytes),
        max_header_bytes: budget.max_data_header_bytes,
    };
    let start = header.data.offset;
    let prefix: [u8; PREFIX_BYTES] = cursor.read("text region header").await?;
    let region = parse_region(&prefix, start, &cursor, budget)?;
    let flags_offset = start + REGION_INFO_BYTES as u64;
    let flags = parse_flags(u16::from_be_bytes([prefix[17], prefix[18]]))
        .map_err(|kind| cursor.error_at(flags_offset, kind))?;

    let huffman_flags = if flags.huffman {
        let offset = cursor.at;
        let raw = u16::from_be_bytes(cursor.read("text region Huffman flags").await?);
        check_huffman_flags(raw, flags.refine).map_err(|kind| cursor.error_at(offset, kind))?;
        Some(raw)
    } else {
        None
    };
    let refinement_at = if flags.refine && flags.refinement_template == 0 {
        let [x1, y1, x2, y2] = cursor.read("text region refinement AT").await?;
        Some([(x1 as i8, y1 as i8), (x2 as i8, y2 as i8)])
    } else {
        None
    };
    let instances_offset = cursor.at;
    let instances = u32::from_be_bytes(cursor.read("SBNUMINSTANCES").await?);
    if instances > budget.max_instances {
        return Err(cursor.error_at(
            instances_offset,
            TextRegionErrorKind::LimitExceeded {
                resource: "text region symbol instances",
                limit: u64::from(budget.max_instances),
                attempted: u64::from(instances),
            },
        ));
    }
    let body_length = end - cursor.at;
    if body_length > budget.max_body_bytes {
        return Err(cursor.error(TextRegionErrorKind::LimitExceeded {
            resource: "text region body bytes",
            limit: budget.max_body_bytes,
            attempted: body_length,
        }));
    }
    if !flags.huffman && body_length < 2 {
        return Err(cursor.error(TextRegionErrorKind::Truncated("MQ body terminal pair")));
    }
    Ok(TextRegionHeader {
        region,
        flags,
        huffman_flags,
        refinement_at,
        instances,
        header_bytes: cursor.at - start,
        body: SegmentSpan {
            offset: cursor.at,
            length: body_length,
        },
    })
}

fn parse_region<S: RangedSource, C: Cancellation>(
    bytes: &[u8; PREFIX_BYTES],
    start: u64,
    cursor: &Cursor<'_, S, C>,
    budget: TextRegionBudget,
) -> TextRegionResult<RegionInfo> {
    let (width, height) = (be32(&bytes[0..4]), be32(&bytes[4..8]));
    let (x, y) = (be32(&bytes[8..12]), be32(&bytes[12..16]));
    let region_flags = bytes[16];
    let flags_offset = start + 16;
    if region_flags & 0xf8 != 0 {
        return Err(cursor.error_at(
            flags_offset,
            TextRegionErrorKind::Malformed("reserved region segment flags"),
        ));
    }
    let combination = match region_flags & 7 {
        0 => RegionCombination::Or,
        1 => RegionCombination::And,
        2 => RegionCombination::Xor,
        3 => RegionCombination::Xnor,
        4 => RegionCombination::Replace,
        _ => {
            return Err(cursor.error_at(
                flags_offset,
                TextRegionErrorKind::Malformed("region combination operator"),
            ));
        }
    };
    if width == 0 || height == 0 {
        return Err(cursor.error_at(
            start,
            TextRegionErrorKind::Unsupported {
                feature: "empty text region",
                value: u64::from(width) * u64::from(height),
            },
        ));
    }
    for (resource, limit, attempted, offset) in [
        ("text region width", budget.max_width, width, start),
        ("text region height", budget.max_height, height, start + 4),
    ] {
        if attempted > limit {
            return Err(cursor.error_at(
                offset,
                TextRegionErrorKind::LimitExceeded {
                    resource,
                    limit: u64::from(limit),
                    attempted: u64::from(attempted),
                },
            ));
        }
    }
    let pixels = u64::from(width) * u64::from(height);
    if pixels > budget.max_pixels {
        return Err(cursor.error_at(
            start,
            TextRegionErrorKind::LimitExceeded {
                resource: "text region pixels",
                limit: budget.max_pixels,
                attempted: pixels,
            },
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

fn parse_flags(raw: u16) -> Result<TextRegionFlags, TextRegionErrorKind> {
    let refine = raw & 2 != 0;
    let refinement_template = (raw >> 15) as u8;
    if !refine && refinement_template != 0 {
        return Err(TextRegionErrorKind::MalformedFlags {
            field: "SBRTEMPLATE without SBREFINE",
            raw,
        });
    }
    // Sign-extend the five-bit SBDSOFFSET in bits 10-14.
    let ds_offset = (((raw >> 10) & 0x1f) as i8) << 3 >> 3;
    Ok(TextRegionFlags {
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
    })
}

/// Figure 37 constraints: bit 15 reserved, selector value 2 forbidden for
/// `SBHUFFFS` and the refinement tables, which must be zero without refinement.
fn check_huffman_flags(raw: u16, refine: bool) -> Result<(), TextRegionErrorKind> {
    let malformed = |field| Err(TextRegionErrorKind::MalformedFlags { field, raw });
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
