// SPDX-License-Identifier: MIT

//! A bounded index of contiguous embedded JBIG2 segments.

use super::{SegmentHeader, SegmentSpan, read_header_prefix, validate_enclosing_span};
use crate::fallible::{len_u64, reserve_exact};
use crate::{Cancellation, Context, Error, Limits, RangedSource, Result};
use std::mem;

/// Headers in physical source order. Segment numbers need not be in that order.
#[derive(Debug)]
pub struct SegmentDirectory {
    pub span: SegmentSpan,
    pub segments: Vec<SegmentHeader>,
}

fn at(header: &SegmentHeader, error: Error) -> Error {
    error.or_at(
        header.header_offset(),
        Context::Jbig2 {
            segment: Some(header.number),
        },
    )
}

fn malformed(header: &SegmentHeader, reason: &'static str) -> Error {
    at(header, Error::invalid(reason))
}

fn unassigned(offset: u64, error: Error) -> Error {
    error.or_at(offset, Context::Jbig2 { segment: None })
}

fn check_cancelled<C: Cancellation>(cancellation: &C, offset: u64) -> Result<()> {
    if cancellation.is_cancelled() {
        Err(unassigned(offset, Error::cancelled()))
    } else {
        Ok(())
    }
}

fn checked_total(current: u64, additional: u64, offset: u64) -> Result<u64> {
    current
        .checked_add(additional)
        .ok_or_else(|| unassigned(offset, Error::invalid("metadata total overflows")))
}

#[derive(Clone, Copy)]
struct IndexEntry {
    number: u32,
    position: usize,
    non_extension_uses: u8,
    expired: bool,
}

fn intermediate(kind: u8) -> bool {
    matches!(kind, 4 | 20 | 36 | 40)
}

fn allowed_target(source: u8, target: u8) -> bool {
    match source {
        0 | 4 | 6 | 7 => matches!(target, 0 | 53),
        20 | 22 | 23 => target == 16,
        40 | 42 | 43 => intermediate(target),
        62 => true,
        _ => false,
    }
}

fn validate_graph<C: Cancellation>(
    segments: &[SegmentHeader],
    metadata_bytes: u64,
    limits: &Limits,
    cancellation: &C,
) -> Result<()> {
    let mut index = Vec::new();
    let refused = unassigned(
        0,
        limits.allocation_refused(
            "JBIG2 directory index bytes",
            len_u64(segments.len()).saturating_mul(mem::size_of::<IndexEntry>() as u64),
        ),
    );
    reserve_exact(&mut index, segments.len(), refused)?;
    for (position, segment) in segments.iter().enumerate() {
        check_cancelled(cancellation, segment.header_offset())?;
        index.push(IndexEntry {
            number: segment.number,
            position,
            non_extension_uses: 0,
            expired: false,
        });
    }
    index.sort_unstable_by_key(|entry| entry.number);
    for pair in index.windows(2) {
        let later = &segments[pair[0].position.max(pair[1].position)];
        check_cancelled(cancellation, later.header_offset())?;
        if pair[0].number == pair[1].number {
            return Err(malformed(later, "duplicate segment number"));
        }
    }

    // One reusable scratch buffer finds repeated references without changing
    // their order, which is needed for the corresponding retention bits.
    let mut sorted_references = Vec::new();
    for source_index in 0..index.len() {
        let source = &segments[index[source_index].position];
        check_cancelled(cancellation, source.header_offset())?;
        sorted_references.clear();
        if source.referred_to.len() > 1 {
            let scratch_bytes = len_u64(source.referred_to.len())
                .checked_mul(mem::size_of::<u32>() as u64)
                .ok_or_else(|| malformed(source, "scratch size overflows"))?;
            let attempted = checked_total(metadata_bytes, scratch_bytes, source.header_offset())?;
            if attempted > limits.max_allocation_bytes {
                return Err(at(
                    source,
                    Error::limit(
                        "JBIG2 directory metadata bytes",
                        limits.max_allocation_bytes,
                        attempted,
                    ),
                ));
            }
            if sorted_references.capacity() < source.referred_to.len() {
                let refused = at(
                    source,
                    limits.allocation_refused("JBIG2 directory metadata bytes", scratch_bytes),
                );
                reserve_exact(&mut sorted_references, source.referred_to.len(), refused)?;
            }
            sorted_references.extend_from_slice(&source.referred_to);
            sorted_references.sort_unstable();
            for pair in sorted_references.windows(2) {
                check_cancelled(cancellation, source.header_offset())?;
                if pair[0] == pair[1] {
                    return Err(malformed(source, "duplicate reference to a segment"));
                }
            }
        }

        let mut tables = 0_u8;
        for (reference_position, reference) in source.referred_to.iter().copied().enumerate() {
            check_cancelled(cancellation, source.header_offset())?;
            let target_index = index
                .binary_search_by_key(&reference, |entry| entry.number)
                .map_err(|_| malformed(source, "reference to a missing segment"))?;
            let target = &segments[index[target_index].position];
            if target.page_association != 0 && target.page_association != source.page_association {
                return Err(malformed(
                    source,
                    "reference to a segment on a disallowed page",
                ));
            }
            if !allowed_target(source.segment_type, target.segment_type) {
                return Err(malformed(
                    source,
                    "reference to a segment of a disallowed type",
                ));
            }
            if target.segment_type == 53 {
                tables = tables
                    .checked_add(1)
                    .ok_or_else(|| malformed(source, "table count overflows"))?;
                let limit = if source.segment_type == 0 { 4 } else { 8 };
                if tables > limit {
                    return Err(at(
                        source,
                        Error::limit(
                            "JBIG2 table references",
                            u64::from(limit),
                            u64::from(tables),
                        ),
                    ));
                }
            }
            if intermediate(target.segment_type) && source.segment_type != 62 {
                let entry = &mut index[target_index];
                if entry.non_extension_uses != 0 {
                    return Err(malformed(
                        source,
                        "intermediate segment has multiple non-extension users",
                    ));
                }
                entry.non_extension_uses = 1;
            }
            if !target.retain_current() || index[target_index].expired {
                return Err(malformed(
                    source,
                    "segment was referenced after non-retention",
                ));
            }
            if source.retain_reference(reference_position) == Some(false) {
                index[target_index].expired = true;
            }
        }
    }
    Ok(())
}

/// Index one caller-delimited, contiguous embedded segment span.
///
/// This reads only segment headers. Each declared data span is skipped by
/// checked offset arithmetic. Physical segment order may differ from number
/// order, as allowed by T.88 Annex D.3. The caller must identify the enclosing
/// embedded span; this API does not discover HN/C8 records or standalone files.
/// The directory's headers and index together are bounded by
/// `Limits::max_allocation_bytes`.
pub fn read_embedded_directory<S: RangedSource, C: Cancellation>(
    source: &mut S,
    span: SegmentSpan,
    limits: &Limits,
    cancellation: &C,
) -> Result<SegmentDirectory> {
    let end = validate_enclosing_span(source, span, limits)?;
    let entry_bytes = (mem::size_of::<SegmentHeader>() as u64)
        .checked_add(mem::size_of::<IndexEntry>() as u64)
        .ok_or_else(|| unassigned(span.offset, Error::invalid("entry size overflows")))?;
    let metadata_limit = |attempted: u64, offset: u64| {
        if attempted > limits.max_allocation_bytes {
            Err(unassigned(
                offset,
                Error::limit(
                    "JBIG2 directory metadata bytes",
                    limits.max_allocation_bytes,
                    attempted,
                ),
            ))
        } else {
            Ok(())
        }
    };
    let mut segments = Vec::new();
    let mut next = span.offset;
    let mut metadata_used = 0_u64;
    while next < end {
        check_cancelled(cancellation, next)?;
        let entry_total = checked_total(metadata_used, entry_bytes, next)?;
        metadata_limit(entry_total, next)?;
        let refused = unassigned(
            next,
            limits.allocation_refused("JBIG2 directory metadata bytes", entry_total),
        );
        reserve_exact(&mut segments, 1, refused)?;
        let (header, after) = read_header_prefix(source, next, end, limits, cancellation)?;
        // A parsed header consumes at least its fixed number, flag, count,
        // page, and length fields, and its data end is not before them, so
        // this loop ends.
        debug_assert!(after > next);
        let header_metadata = header
            .metadata_bytes()
            .ok_or_else(|| malformed(&header, "metadata size overflows"))?;
        metadata_used = checked_total(entry_total, header_metadata, next)?;
        metadata_limit(metadata_used, next)?;
        segments.push(header);
        next = after;
        check_cancelled(cancellation, next)?;
    }
    validate_graph(&segments, metadata_used, limits, cancellation)?;
    check_cancelled(cancellation, end)?;
    Ok(SegmentDirectory { span, segments })
}

#[cfg(test)]
mod tests;
