// SPDX-License-Identifier: MIT

//! Bounded T.88 JBIG2 segment framing and arithmetic model primitives.
//!
//! Standalone JBIG2 file headers, HN/C8 containers, and general page
//! composition remain outside this module. The narrow observed HN/C8
//! full-page OR profile has its own checked composition primitive.

mod directory;
pub use directory::{SegmentDirectory, read_embedded_directory};

pub mod dictionary;
pub mod generic;
pub mod iaid;
pub mod integer;
pub mod mq;
pub mod page_compose;
pub mod page_info;
pub mod page_profile;
pub mod refinement;
pub mod text;
pub mod text_composer;
pub mod text_instances;

use crate::fallible::{len_u64, reserve_exact, usize_from_u32};
use crate::{Cancellation, Context, Error, Limits, RangedSource, Result};
use std::mem;

/// An exact range containing one segment header immediately followed by its data.
/// Coordinates are in the supplied `RangedSource`, which may be a logical span.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SegmentSpan {
    pub offset: u64,
    pub length: u64,
}

/// Validated metadata for one segment. The data range is not read by this API.
#[derive(Debug, Eq, PartialEq)]
pub struct SegmentHeader {
    pub number: u32,
    /// Standard segment type code from T.88 §7.3; payload decoding is separate.
    pub segment_type: u8,
    pub deferred_non_retain: bool,
    pub page_association: u32,
    pub referred_to: Vec<u32>,
    pub data: SegmentSpan,
    /// Number of header bytes before `data`.
    pub header_length: u64,
    // Bit 0 is this segment; subsequent bits correspond to `referred_to`.
    retention: Vec<u8>,
}

impl SegmentHeader {
    pub fn retain_current(&self) -> bool {
        self.retained_bit(0)
    }

    pub fn retain_reference(&self, index: usize) -> Option<bool> {
        (index < self.referred_to.len()).then(|| self.retained_bit(index + 1))
    }

    fn retained_bit(&self, bit: usize) -> bool {
        (self.retention[bit / 8] >> (bit % 8)) & 1 != 0
    }

    pub(super) fn metadata_bytes(&self) -> Option<u64> {
        len_u64(self.referred_to.len())
            .checked_mul(mem::size_of::<u32>() as u64)?
            .checked_add(len_u64(self.retention.len()))
    }

    pub(super) fn header_offset(&self) -> u64 {
        self.data.offset - self.header_length
    }
}

mod cursor;
use cursor::{FieldCursor, FieldFault};

struct HeaderCursor {
    fields: FieldCursor,
    segment: Option<u32>,
}

impl HeaderCursor {
    fn new(start: u64, end: u64, request_bytes: usize) -> Self {
        Self {
            fields: FieldCursor {
                start,
                at: start,
                end,
                fetched: 0,
                request_bytes,
            },
            segment: None,
        }
    }

    /// Locate an error at the cursor in the segment read so far.
    fn locate(&self, error: Error) -> Error {
        error.or_at(
            self.fields.at,
            Context::Jbig2 {
                segment: self.segment,
            },
        )
    }

    fn malformed(&self, reason: &'static str) -> Error {
        self.locate(Error::malformed(self.fields.at, reason))
    }

    fn unsupported(&self, reason: &'static str) -> Error {
        self.locate(Error::unsupported(self.fields.at, reason))
    }

    fn check_cancelled<C: Cancellation>(&self, cancellation: &C) -> Result<()> {
        if cancellation.is_cancelled() {
            Err(self.locate(Error::cancelled()))
        } else {
            Ok(())
        }
    }

    /// A header that would end past the span is truncated at its end.
    fn fault(&self, fault: FieldFault, field: &'static str) -> Error {
        match fault {
            FieldFault::Overflow { .. } => self.malformed("header end overflows"),
            FieldFault::PastEnd {
                expected,
                available,
            } => self.locate(
                Error::truncated(self.fields.end, expected, available).because("segment header"),
            ),
            fault => self.locate(fault.error(self.fields.at, field)),
        }
    }

    fn check_future_header(&self, additional: u64) -> Result<()> {
        self.fields
            .check_room(additional)
            .map_err(|fault| self.fault(fault, "segment header"))
    }

    fn read_into<S: RangedSource, C: Cancellation>(
        &mut self,
        source: &mut S,
        destination: &mut [u8],
        field: &'static str,
        cancellation: &C,
    ) -> Result<()> {
        let result = self.fields.fill(source, cancellation, destination);
        result.map_err(|fault| self.fault(fault, field))
    }

    fn read<const N: usize, S: RangedSource, C: Cancellation>(
        &mut self,
        source: &mut S,
        field: &'static str,
        cancellation: &C,
    ) -> Result<[u8; N]> {
        let mut bytes = [0; N];
        self.read_into(source, &mut bytes, field, cancellation)?;
        Ok(bytes)
    }
}

/// An unlocated feature outside the supported profile.
fn unsupported(feature: &'static str) -> Error {
    Error::from(crate::ErrorKind::UnsupportedFormat).because(feature)
}

/// Where in a segment a dictionary or text-region check before decoding
/// failed.
#[derive(Clone, Copy)]
struct Site {
    segment: u32,
    offset: u64,
}

impl Site {
    fn context(self) -> Context {
        Context::Jbig2 {
            segment: Some(self.segment),
        }
    }

    /// Locate an unlocated error here.
    fn locate(self, error: Error) -> Error {
        error.or_at(self.offset, self.context())
    }

    fn malformed(self, reason: &'static str) -> Error {
        self.locate(Error::invalid(reason))
    }

    fn unsupported(self, reason: &'static str) -> Error {
        self.locate(unsupported(reason))
    }

    fn limit(self, resource: &'static str, limit: u64, attempted: u64) -> Error {
        self.locate(Error::limit(resource, limit, attempted))
    }

    /// Fail with a limit error when `attempted` exceeds `limit`.
    fn cap(self, resource: &'static str, limit: u64, attempted: u64) -> Result<()> {
        if attempted > limit {
            Err(self.limit(resource, limit, attempted))
        } else {
            Ok(())
        }
    }
}

fn allowed_type(kind: u8) -> bool {
    matches!(
        kind,
        0 | 4
            | 6
            | 7
            | 16
            | 20
            | 22
            | 23
            | 36
            | 38
            | 39
            | 40
            | 42
            | 43
            | 48
            | 49
            | 50
            | 51
            | 52
            | 53
            | 62
    )
}

fn valid_reference_count(kind: u8, count: u32) -> bool {
    match kind {
        16 | 36 | 38 | 39 | 48 | 49 | 50 | 51 | 52 | 53 => count == 0,
        20 | 22 | 23 | 40 => count == 1,
        42 | 43 => count <= 1,
        _ => true,
    }
}

fn valid_page_association(kind: u8, page: u32) -> bool {
    match kind {
        4 | 6 | 7 | 20 | 22 | 23 | 36 | 38 | 39 | 40 | 42 | 43 | 48 | 49 | 50 => page != 0,
        51 => page == 0,
        _ => true,
    }
}

pub(super) fn validate_enclosing_span<S: RangedSource>(
    source: &S,
    span: SegmentSpan,
    limits: &Limits,
) -> Result<u64> {
    let cursor = HeaderCursor::new(span.offset, span.offset, limits.io_chunk_bytes);
    limits
        .check_input_size(span.length)
        .map_err(|error| cursor.locate(error))?;
    let end = span
        .offset
        .checked_add(span.length)
        .ok_or_else(|| cursor.malformed("segment span end overflows 64 bits"))?;
    if end > source.size() {
        return Err(cursor.malformed("segment span extends beyond source size"));
    }
    Ok(end)
}

// One parser serves both an exact segment span and a bounded embedded scan.
// The caller validates the enclosing span before calling this function.
pub(super) fn read_header_prefix<S: RangedSource, C: Cancellation>(
    source: &mut S,
    start: u64,
    end: u64,
    limits: &Limits,
    cancellation: &C,
) -> Result<(SegmentHeader, u64)> {
    let mut cursor = HeaderCursor::new(start, end, limits.io_chunk_bytes);
    cursor.check_cancelled(cancellation)?;

    let number = u32::from_be_bytes(cursor.read(source, "segment number", cancellation)?);
    cursor.segment = Some(number);
    let flags = cursor.read::<1, _, _>(source, "segment flags", cancellation)?[0];
    let segment_type = flags & 0x3f;
    if !allowed_type(segment_type) {
        return Err(cursor.unsupported("reserved segment type"));
    }
    let first = cursor.read::<1, _, _>(source, "reference count and retention", cancellation)?[0];
    let count_tag = first >> 5;
    let (reference_count, retention_bytes, short_retention) = match count_tag {
        0..=4 => (u32::from(count_tag), 1_u64, Some(first & 0x1f)),
        7 => {
            let tail = cursor.read::<3, _, _>(source, "long reference count", cancellation)?;
            let count = u32::from_be_bytes([first, tail[0], tail[1], tail[2]]) & 0x1fff_ffff;
            if count <= 4 {
                return Err(cursor.malformed("noncanonical long reference count"));
            }
            (count, (u64::from(count) + 1).div_ceil(8), None)
        }
        _ => {
            return Err(cursor.unsupported("reserved reference-count form"));
        }
    };
    if !valid_reference_count(segment_type, reference_count) {
        return Err(cursor.malformed("reference count for segment type"));
    }
    let reference_width = if number <= 256 {
        1_u64
    } else if number <= 65_536 {
        2
    } else {
        4
    };
    let association_width = if flags & 0x40 == 0 { 1_u64 } else { 4 };
    let reference_bytes = u64::from(reference_count)
        .checked_mul(reference_width)
        .ok_or_else(|| cursor.malformed("reference size overflows"))?;
    let remaining_header = retention_bytes
        .checked_sub(u64::from(short_retention.is_some()))
        .and_then(|value| value.checked_add(reference_bytes))
        .and_then(|value| value.checked_add(association_width + 4))
        .ok_or_else(|| cursor.malformed("header size overflows"))?;
    cursor.check_future_header(remaining_header)?;
    let allocation_bytes = u64::from(reference_count)
        .checked_mul(mem::size_of::<u32>() as u64)
        .and_then(|value| value.checked_add(retention_bytes))
        .ok_or_else(|| cursor.malformed("allocation size overflows"))?;
    if allocation_bytes > limits.max_allocation_bytes {
        return Err(cursor.locate(Error::limit(
            "JBIG2 header metadata bytes",
            limits.max_allocation_bytes,
            allocation_bytes,
        )));
    }

    let mut retention = Vec::new();
    // At most `(2^29 + 1).div_ceil(8)` bytes for a 29-bit reference count.
    let retention_length = retention_bytes as usize;
    let failed = cursor.locate(limits.allocation_refused("JBIG2 retention bytes", retention_bytes));
    reserve_exact(&mut retention, retention_length, failed)?;
    if let Some(short) = short_retention {
        retention.push(short);
        let used = (1_u8 << (reference_count + 1)) - 1;
        if short & !used != 0 {
            return Err(cursor.malformed("unused short retention bits"));
        }
    } else {
        retention.resize(retention_length, 0);
        cursor.read_into(source, &mut retention, "long retention flags", cancellation)?;
        let used = ((reference_count + 1) % 8) as u8;
        if used != 0 && retention[retention_length - 1] & !((1_u8 << used) - 1) != 0 {
            return Err(cursor.malformed("unused long retention bits"));
        }
    }

    let mut referred_to = Vec::new();
    let count = usize_from_u32(reference_count);
    let failed =
        cursor.locate(limits.allocation_refused("JBIG2 reference bytes", allocation_bytes));
    reserve_exact(&mut referred_to, count, failed)?;
    for _ in 0..count {
        let reference = match reference_width {
            1 => u32::from(cursor.read::<1, _, _>(source, "reference number", cancellation)?[0]),
            2 => u32::from(u16::from_be_bytes(cursor.read(
                source,
                "reference number",
                cancellation,
            )?)),
            _ => u32::from_be_bytes(cursor.read(source, "reference number", cancellation)?),
        };
        if reference >= number {
            return Err(cursor.malformed("reference is not lower than segment number"));
        }
        referred_to.push(reference);
    }
    let page_association = if association_width == 1 {
        u32::from(cursor.read::<1, _, _>(source, "page association", cancellation)?[0])
    } else {
        u32::from_be_bytes(cursor.read(source, "page association", cancellation)?)
    };
    if !valid_page_association(segment_type, page_association) {
        return Err(cursor.malformed("page association for segment type"));
    }
    let data_length =
        u32::from_be_bytes(cursor.read(source, "segment data length", cancellation)?);
    if data_length == u32::MAX {
        return Err(cursor.unsupported("unknown segment data length"));
    }
    let data_end = cursor
        .fields
        .at
        .checked_add(u64::from(data_length))
        .ok_or_else(|| cursor.malformed("data end overflows"))?;
    if data_end > cursor.fields.end {
        return Err(cursor.locate(
            Error::truncated(
                cursor.fields.at,
                u64::from(data_length),
                cursor.fields.end - cursor.fields.at,
            )
            .because("segment data"),
        ));
    }
    cursor.check_cancelled(cancellation)?;
    Ok((
        SegmentHeader {
            number,
            segment_type,
            deferred_non_retain: flags & 0x80 != 0,
            page_association,
            referred_to,
            data: SegmentSpan {
                offset: cursor.fields.at,
                length: u64::from(data_length),
            },
            header_length: cursor.fields.at - cursor.fields.start,
            retention,
        },
        data_end,
    ))
}

/// Read only the header of one exact, contiguous header-plus-data span.
///
/// Source reads never pass the supplied span. Returned data bytes are not read
/// or decoded. Cross-segment rules are checked by [`read_embedded_directory`].
pub fn read_segment_header<S: RangedSource, C: Cancellation>(
    source: &mut S,
    span: SegmentSpan,
    limits: &Limits,
    cancellation: &C,
) -> Result<SegmentHeader> {
    let end = validate_enclosing_span(source, span, limits)?;
    let (header, next) = read_header_prefix(source, span.offset, end, limits, cancellation)?;
    if next != end {
        return Err(
            Error::malformed(header.data.offset, "bytes follow declared segment data")
                .in_jbig2(Some(header.number)),
        );
    }
    Ok(header)
}
