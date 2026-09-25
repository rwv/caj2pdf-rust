// SPDX-License-Identifier: MIT

//! Bounded T.88 JBIG2 segment framing and arithmetic model primitives.
//!
//! Standalone JBIG2 file headers, HN/C8 containers, and complete page
//! composition remain outside this module.

mod directory;
pub use directory::{
    DirectoryError, DirectoryErrorKind, DirectoryLimits, SegmentDirectory, read_embedded_directory,
};

pub mod dictionary;
pub mod generic;
pub mod iaid;
pub mod integer;
pub mod mq;
pub mod refinement;
pub mod text;

use crate::fallible::{len_u64, reserve_exact};
use crate::{Cancellation, Error, Limits, RangedSource};
use std::{error, fmt, mem};

/// An exact range containing one segment header immediately followed by its data.
/// Coordinates are in the supplied `RangedSource`, which may be a logical span.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SegmentSpan {
    pub offset: u64,
    pub length: u64,
}

/// Per-segment bounds, independent of any enclosing document limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeaderLimits {
    pub max_header_bytes: u64,
    pub max_references: u32,
    pub max_data_bytes: u64,
}

impl Default for HeaderLimits {
    fn default() -> Self {
        Self {
            max_header_bytes: 64 * 1024,
            max_references: 4096,
            max_data_bytes: 64 * 1024 * 1024,
        }
    }
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

/// A located failure while reading a JBIG2 segment header.
#[derive(Debug)]
pub struct HeaderError {
    pub offset: u64,
    pub segment: Option<u32>,
    pub kind: HeaderErrorKind,
}

/// Distinct malformed, unsupported, resource, cancellation, and source errors.
#[derive(Debug)]
pub enum HeaderErrorKind {
    InvalidSpan(&'static str),
    Truncated(&'static str),
    Malformed(&'static str),
    Unsupported {
        feature: &'static str,
        value: u64,
    },
    LimitExceeded {
        resource: &'static str,
        limit: u64,
        attempted: u64,
    },
    AllocationFailed,
    Cancelled,
    Source(Error),
}

pub type HeaderResult<T> = std::result::Result<T, HeaderError>;

#[derive(Clone, Copy)]
pub(super) struct PrefixBudget {
    pub metadata_used: u64,
    pub metadata_limit: u64,
    pub references_used: u64,
    pub references_limit: u64,
}

impl fmt::Display for HeaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "JBIG2 segment header at source byte {}", self.offset)?;
        if let Some(number) = self.segment {
            write!(f, ", segment {number}")?;
        }
        f.write_str(": ")?;
        match &self.kind {
            HeaderErrorKind::InvalidSpan(reason) => write!(f, "invalid span: {reason}"),
            HeaderErrorKind::Truncated(field) => write!(f, "truncated {field}"),
            HeaderErrorKind::Malformed(reason) => write!(f, "malformed {reason}"),
            HeaderErrorKind::Unsupported { feature, value } => {
                write!(f, "unsupported {feature} ({value})")
            }
            HeaderErrorKind::LimitExceeded {
                resource,
                limit,
                attempted,
            } => write!(f, "{resource} limit {limit} exceeded by {attempted}"),
            HeaderErrorKind::AllocationFailed => f.write_str("header allocation failed"),
            HeaderErrorKind::Cancelled => f.write_str("cancelled"),
            HeaderErrorKind::Source(source) => write!(f, "source: {source}"),
        }
    }
}

impl error::Error for HeaderError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.kind {
            HeaderErrorKind::Source(source) => Some(source),
            _ => None,
        }
    }
}

struct HeaderCursor {
    start: u64,
    at: u64,
    end: u64,
    max_header_bytes: u64,
    segment: Option<u32>,
}

impl HeaderCursor {
    fn error(&self, kind: HeaderErrorKind) -> HeaderError {
        HeaderError {
            offset: self.at,
            segment: self.segment,
            kind,
        }
    }

    fn invalid_span(&self, reason: &'static str) -> HeaderError {
        self.error(HeaderErrorKind::InvalidSpan(reason))
    }

    fn check_cancelled<C: Cancellation>(&self, cancellation: &C) -> HeaderResult<()> {
        if cancellation.is_cancelled() {
            Err(self.error(HeaderErrorKind::Cancelled))
        } else {
            Ok(())
        }
    }

    fn check_future_header(&self, additional: u64) -> HeaderResult<()> {
        let attempted = (self.at - self.start)
            .checked_add(additional)
            .ok_or_else(|| self.invalid_span("header length overflows"))?;
        if attempted > self.max_header_bytes {
            return Err(self.error(HeaderErrorKind::LimitExceeded {
                resource: "JBIG2 header bytes",
                limit: self.max_header_bytes,
                attempted,
            }));
        }
        let future = self
            .at
            .checked_add(additional)
            .ok_or_else(|| self.invalid_span("header end overflows"))?;
        if future > self.end {
            return Err(HeaderError {
                offset: self.end,
                segment: self.segment,
                kind: HeaderErrorKind::Truncated("segment header"),
            });
        }
        Ok(())
    }

    async fn read_into<S: RangedSource, C: Cancellation>(
        &mut self,
        source: &mut S,
        destination: &mut [u8],
        field: &'static str,
        limits: &Limits,
        cancellation: &C,
    ) -> HeaderResult<()> {
        let requested = len_u64(destination.len());
        self.check_future_header(requested)?;
        let mut done = 0;
        while done < destination.len() {
            self.check_cancelled(cancellation)?;
            let chunk = (destination.len() - done).min(limits.io_chunk_bytes);
            let read = source
                .read_at(self.at, &mut destination[done..done + chunk])
                .await
                .map_err(|error| match error {
                    Error::Cancelled => self.error(HeaderErrorKind::Cancelled),
                    other => self.error(HeaderErrorKind::Source(other)),
                })?;
            if read > chunk {
                return Err(self.error(HeaderErrorKind::Malformed(
                    "source returned more bytes than requested",
                )));
            }
            done += read;
            self.at = self
                .at
                .checked_add(len_u64(read))
                .ok_or_else(|| self.invalid_span("read end overflows"))?;
            self.check_cancelled(cancellation)?;
            if read == 0 {
                return Err(self.error(HeaderErrorKind::Truncated(field)));
            }
        }
        self.check_cancelled(cancellation)
    }

    async fn read<const N: usize, S: RangedSource, C: Cancellation>(
        &mut self,
        source: &mut S,
        field: &'static str,
        limits: &Limits,
        cancellation: &C,
    ) -> HeaderResult<[u8; N]> {
        let mut bytes = [0; N];
        self.read_into(source, &mut bytes, field, limits, cancellation)
            .await?;
        Ok(bytes)
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
) -> HeaderResult<u64> {
    let cursor = HeaderCursor {
        start: span.offset,
        at: span.offset,
        end: span.offset,
        max_header_bytes: 0,
        segment: None,
    };
    limits
        .validate()
        .and_then(|()| limits.check_input_size(span.length))
        .map_err(|error| cursor.error(HeaderErrorKind::Source(error)))?;
    let end = span
        .offset
        .checked_add(span.length)
        .ok_or_else(|| cursor.invalid_span("end overflows 64 bits"))?;
    if end > source.size() {
        return Err(cursor.invalid_span("range extends beyond source size"));
    }
    Ok(end)
}

// One parser serves both an exact segment span and a bounded embedded scan.
// The caller validates the enclosing span before calling this function.
pub(super) async fn read_header_prefix<S: RangedSource, C: Cancellation>(
    source: &mut S,
    start: u64,
    end: u64,
    limits: &Limits,
    header_limits: HeaderLimits,
    budget: Option<PrefixBudget>,
    cancellation: &C,
) -> HeaderResult<(SegmentHeader, u64)> {
    let mut cursor = HeaderCursor {
        start,
        at: start,
        end,
        max_header_bytes: header_limits.max_header_bytes,
        segment: None,
    };
    cursor.check_cancelled(cancellation)?;

    let number = u32::from_be_bytes(
        cursor
            .read(source, "segment number", limits, cancellation)
            .await?,
    );
    cursor.segment = Some(number);
    let flags = cursor
        .read::<1, _, _>(source, "segment flags", limits, cancellation)
        .await?[0];
    let segment_type = flags & 0x3f;
    if !allowed_type(segment_type) {
        return Err(cursor.error(HeaderErrorKind::Unsupported {
            feature: "reserved segment type",
            value: u64::from(segment_type),
        }));
    }
    let first = cursor
        .read::<1, _, _>(
            source,
            "reference count and retention",
            limits,
            cancellation,
        )
        .await?[0];
    let count_tag = first >> 5;
    let (reference_count, retention_bytes, short_retention) = match count_tag {
        0..=4 => (u32::from(count_tag), 1_u64, Some(first & 0x1f)),
        7 => {
            let tail = cursor
                .read::<3, _, _>(source, "long reference count", limits, cancellation)
                .await?;
            let count = u32::from_be_bytes([first, tail[0], tail[1], tail[2]]) & 0x1fff_ffff;
            if count <= 4 {
                return Err(cursor.error(HeaderErrorKind::Malformed(
                    "noncanonical long reference count",
                )));
            }
            (count, (u64::from(count) + 1).div_ceil(8), None)
        }
        _ => {
            return Err(cursor.error(HeaderErrorKind::Unsupported {
                feature: "reserved reference-count form",
                value: u64::from(count_tag),
            }));
        }
    };
    if !valid_reference_count(segment_type, reference_count) {
        return Err(cursor.error(HeaderErrorKind::Malformed(
            "reference count for segment type",
        )));
    }
    if reference_count > header_limits.max_references {
        return Err(cursor.error(HeaderErrorKind::LimitExceeded {
            resource: "JBIG2 references",
            limit: u64::from(header_limits.max_references),
            attempted: u64::from(reference_count),
        }));
    }
    if let Some(budget) = budget {
        let attempted = budget
            .references_used
            .checked_add(u64::from(reference_count))
            .ok_or_else(|| cursor.invalid_span("reference total overflows"))?;
        if attempted > budget.references_limit {
            return Err(cursor.error(HeaderErrorKind::LimitExceeded {
                resource: "JBIG2 directory references",
                limit: budget.references_limit,
                attempted,
            }));
        }
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
        .ok_or_else(|| cursor.invalid_span("reference size overflows"))?;
    let remaining_header = retention_bytes
        .checked_sub(u64::from(short_retention.is_some()))
        .and_then(|value| value.checked_add(reference_bytes))
        .and_then(|value| value.checked_add(association_width + 4))
        .ok_or_else(|| cursor.invalid_span("header size overflows"))?;
    cursor.check_future_header(remaining_header)?;
    let allocation_bytes = u64::from(reference_count)
        .checked_mul(mem::size_of::<u32>() as u64)
        .and_then(|value| value.checked_add(retention_bytes))
        .ok_or_else(|| cursor.invalid_span("allocation size overflows"))?;
    if allocation_bytes > limits.max_allocation_bytes {
        return Err(cursor.error(HeaderErrorKind::LimitExceeded {
            resource: "JBIG2 header metadata bytes",
            limit: limits.max_allocation_bytes,
            attempted: allocation_bytes,
        }));
    }
    if let Some(budget) = budget {
        let attempted = budget
            .metadata_used
            .checked_add(allocation_bytes)
            .ok_or_else(|| cursor.invalid_span("metadata total overflows"))?;
        if attempted > budget.metadata_limit {
            return Err(cursor.error(HeaderErrorKind::LimitExceeded {
                resource: "JBIG2 directory metadata bytes",
                limit: budget.metadata_limit,
                attempted,
            }));
        }
    }

    let mut retention = Vec::new();
    let retention_length = usize::try_from(retention_bytes)
        .map_err(|_| cursor.invalid_span("retention size overflows"))?;
    reserve_exact(
        &mut retention,
        retention_length,
        cursor.error(HeaderErrorKind::AllocationFailed),
    )?;
    if let Some(short) = short_retention {
        retention.push(short);
        let used = (1_u8 << (reference_count + 1)) - 1;
        if short & !used != 0 {
            return Err(cursor.error(HeaderErrorKind::Malformed("unused short retention bits")));
        }
    } else {
        retention.resize(retention_length, 0);
        cursor
            .read_into(
                source,
                &mut retention,
                "long retention flags",
                limits,
                cancellation,
            )
            .await?;
        let used = ((reference_count + 1) % 8) as u8;
        if used != 0 && retention[retention_length - 1] & !((1_u8 << used) - 1) != 0 {
            return Err(cursor.error(HeaderErrorKind::Malformed("unused long retention bits")));
        }
    }

    let mut referred_to = Vec::new();
    let count = usize::try_from(reference_count)
        .map_err(|_| cursor.invalid_span("reference count overflows"))?;
    reserve_exact(
        &mut referred_to,
        count,
        cursor.error(HeaderErrorKind::AllocationFailed),
    )?;
    for _ in 0..count {
        let reference = match reference_width {
            1 => u32::from(
                cursor
                    .read::<1, _, _>(source, "reference number", limits, cancellation)
                    .await?[0],
            ),
            2 => u32::from(u16::from_be_bytes(
                cursor
                    .read(source, "reference number", limits, cancellation)
                    .await?,
            )),
            _ => u32::from_be_bytes(
                cursor
                    .read(source, "reference number", limits, cancellation)
                    .await?,
            ),
        };
        if reference >= number {
            return Err(cursor.error(HeaderErrorKind::Malformed(
                "reference is not lower than segment number",
            )));
        }
        referred_to.push(reference);
    }
    let page_association = if association_width == 1 {
        u32::from(
            cursor
                .read::<1, _, _>(source, "page association", limits, cancellation)
                .await?[0],
        )
    } else {
        u32::from_be_bytes(
            cursor
                .read(source, "page association", limits, cancellation)
                .await?,
        )
    };
    if !valid_page_association(segment_type, page_association) {
        return Err(cursor.error(HeaderErrorKind::Malformed(
            "page association for segment type",
        )));
    }
    let data_length = u32::from_be_bytes(
        cursor
            .read(source, "segment data length", limits, cancellation)
            .await?,
    );
    if data_length == u32::MAX {
        return Err(cursor.error(HeaderErrorKind::Unsupported {
            feature: "unknown segment data length",
            value: u64::from(data_length),
        }));
    }
    if u64::from(data_length) > header_limits.max_data_bytes {
        return Err(cursor.error(HeaderErrorKind::LimitExceeded {
            resource: "JBIG2 segment data bytes",
            limit: header_limits.max_data_bytes,
            attempted: u64::from(data_length),
        }));
    }
    let data_end = cursor
        .at
        .checked_add(u64::from(data_length))
        .ok_or_else(|| cursor.invalid_span("data end overflows"))?;
    if data_end > cursor.end {
        return Err(cursor.error(HeaderErrorKind::Truncated("segment data")));
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
                offset: cursor.at,
                length: u64::from(data_length),
            },
            header_length: cursor.at - cursor.start,
            retention,
        },
        data_end,
    ))
}

/// Read only the header of one exact, contiguous header-plus-data span.
///
/// Source reads never pass the supplied span. Returned data bytes are not read
/// or decoded. Cross-segment rules are checked by [`read_embedded_directory`].
pub async fn read_segment_header<S: RangedSource, C: Cancellation>(
    source: &mut S,
    span: SegmentSpan,
    limits: &Limits,
    header_limits: HeaderLimits,
    cancellation: &C,
) -> HeaderResult<SegmentHeader> {
    let end = validate_enclosing_span(source, span, limits)?;
    let (header, next) = read_header_prefix(
        source,
        span.offset,
        end,
        limits,
        header_limits,
        None,
        cancellation,
    )
    .await?;
    if next != end {
        return Err(HeaderError {
            offset: header.data.offset,
            segment: Some(header.number),
            kind: HeaderErrorKind::Malformed("bytes follow declared segment data"),
        });
    }
    Ok(header)
}
