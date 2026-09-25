// SPDX-License-Identifier: MIT

//! A bounded index of contiguous embedded JBIG2 segments.

use super::{
    HeaderError, HeaderLimits, PrefixBudget, SegmentHeader, SegmentSpan, read_header_prefix,
    validate_enclosing_span,
};
use crate::fallible::{len_u64, reserve_exact};
use crate::{Cancellation, Limits, RangedSource};
use std::{error, fmt, mem};

/// Per-image bounds in addition to [`HeaderLimits`] and the shared [`Limits`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectoryLimits {
    pub max_span_bytes: u64,
    pub max_segments: u32,
    pub max_total_references: u32,
    pub max_metadata_bytes: u64,
}

impl Default for DirectoryLimits {
    fn default() -> Self {
        Self {
            max_span_bytes: 64 * 1024 * 1024,
            max_segments: 4096,
            max_total_references: 16_384,
            max_metadata_bytes: 1024 * 1024,
        }
    }
}

/// Headers in physical source order. Segment numbers need not be in that order.
#[derive(Debug)]
pub struct SegmentDirectory {
    pub span: SegmentSpan,
    pub segments: Vec<SegmentHeader>,
}

/// A located failure while indexing or validating an embedded directory.
#[derive(Debug)]
pub struct DirectoryError {
    pub offset: u64,
    pub segment: Option<u32>,
    pub kind: DirectoryErrorKind,
}

#[derive(Debug)]
pub enum DirectoryErrorKind {
    Header(HeaderError),
    InvalidSpan(&'static str),
    LimitExceeded {
        resource: &'static str,
        limit: u64,
        attempted: u64,
    },
    AllocationFailed,
    Cancelled,
    DuplicateNumber(u32),
    DuplicateReference(u32),
    MissingReference(u32),
    PageMismatch {
        reference: u32,
        referenced_page: u32,
    },
    ReferenceType {
        reference: u32,
        referenced_type: u8,
    },
    TooManyTables {
        limit: u8,
        attempted: u8,
    },
    IntermediateReused(u32),
    RetentionViolation(u32),
}

type Result<T> = std::result::Result<T, DirectoryError>;

impl From<HeaderError> for DirectoryError {
    fn from(error: HeaderError) -> Self {
        Self {
            offset: error.offset,
            segment: error.segment,
            kind: DirectoryErrorKind::Header(error),
        }
    }
}

impl fmt::Display for DirectoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let DirectoryErrorKind::Header(error) = &self.kind {
            return write!(f, "{error}");
        }
        write!(f, "JBIG2 directory at source byte {}", self.offset)?;
        if let Some(number) = self.segment {
            write!(f, ", segment {number}")?;
        }
        f.write_str(": ")?;
        match &self.kind {
            DirectoryErrorKind::Header(_) => unreachable!(),
            DirectoryErrorKind::InvalidSpan(reason) => write!(f, "invalid span: {reason}"),
            DirectoryErrorKind::LimitExceeded {
                resource,
                limit,
                attempted,
            } => write!(f, "{resource} limit {limit} exceeded by {attempted}"),
            DirectoryErrorKind::AllocationFailed => f.write_str("directory allocation failed"),
            DirectoryErrorKind::Cancelled => f.write_str("cancelled"),
            DirectoryErrorKind::DuplicateNumber(number) => {
                write!(f, "duplicate segment number {number}")
            }
            DirectoryErrorKind::DuplicateReference(reference) => {
                write!(f, "duplicate reference to segment {reference}")
            }
            DirectoryErrorKind::MissingReference(reference) => {
                write!(f, "missing reference to segment {reference}")
            }
            DirectoryErrorKind::PageMismatch {
                reference,
                referenced_page,
            } => write!(
                f,
                "reference to segment {reference} on disallowed page {referenced_page}"
            ),
            DirectoryErrorKind::ReferenceType {
                reference,
                referenced_type,
            } => write!(
                f,
                "reference to segment {reference} has disallowed type {referenced_type}"
            ),
            DirectoryErrorKind::TooManyTables { limit, attempted } => {
                write!(f, "tables reference limit {limit} exceeded by {attempted}")
            }
            DirectoryErrorKind::IntermediateReused(reference) => write!(
                f,
                "intermediate segment {reference} has multiple non-extension users"
            ),
            DirectoryErrorKind::RetentionViolation(reference) => {
                write!(f, "segment {reference} was referenced after non-retention")
            }
        }
    }
}

impl error::Error for DirectoryError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.kind {
            DirectoryErrorKind::Header(error) => Some(error),
            _ => None,
        }
    }
}

fn at(header: &SegmentHeader, kind: DirectoryErrorKind) -> DirectoryError {
    DirectoryError {
        offset: header.header_offset(),
        segment: Some(header.number),
        kind,
    }
}

fn invalid_span(header: &SegmentHeader, reason: &'static str) -> DirectoryError {
    at(header, DirectoryErrorKind::InvalidSpan(reason))
}

fn unassigned(offset: u64, kind: DirectoryErrorKind) -> DirectoryError {
    DirectoryError {
        offset,
        segment: None,
        kind,
    }
}

fn check_cancelled<C: Cancellation>(cancellation: &C, offset: u64) -> Result<()> {
    if cancellation.is_cancelled() {
        Err(DirectoryError {
            offset,
            segment: None,
            kind: DirectoryErrorKind::Cancelled,
        })
    } else {
        Ok(())
    }
}

fn checked_total(current: u64, additional: u64, offset: u64) -> Result<u64> {
    current.checked_add(additional).ok_or(DirectoryError {
        offset,
        segment: None,
        kind: DirectoryErrorKind::InvalidSpan("metadata total overflows"),
    })
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
    limits: DirectoryLimits,
    cancellation: &C,
) -> Result<()> {
    let mut index = Vec::new();
    let refused = unassigned(0, DirectoryErrorKind::AllocationFailed);
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
            return Err(at(later, DirectoryErrorKind::DuplicateNumber(later.number)));
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
                .ok_or(invalid_span(source, "scratch size overflows"))?;
            let attempted = checked_total(metadata_bytes, scratch_bytes, source.header_offset())?;
            if attempted > limits.max_metadata_bytes {
                return Err(at(
                    source,
                    DirectoryErrorKind::LimitExceeded {
                        resource: "JBIG2 directory metadata bytes",
                        limit: limits.max_metadata_bytes,
                        attempted,
                    },
                ));
            }
            if sorted_references.capacity() < source.referred_to.len() {
                let refused = at(source, DirectoryErrorKind::AllocationFailed);
                reserve_exact(&mut sorted_references, source.referred_to.len(), refused)?;
            }
            sorted_references.extend_from_slice(&source.referred_to);
            sorted_references.sort_unstable();
            for pair in sorted_references.windows(2) {
                check_cancelled(cancellation, source.header_offset())?;
                if pair[0] == pair[1] {
                    return Err(at(source, DirectoryErrorKind::DuplicateReference(pair[0])));
                }
            }
        }

        let mut tables = 0_u8;
        for (reference_position, reference) in source.referred_to.iter().copied().enumerate() {
            check_cancelled(cancellation, source.header_offset())?;
            let target_index = index
                .binary_search_by_key(&reference, |entry| entry.number)
                .map_err(|_| at(source, DirectoryErrorKind::MissingReference(reference)))?;
            let target = &segments[index[target_index].position];
            if target.page_association != 0 && target.page_association != source.page_association {
                return Err(at(
                    source,
                    DirectoryErrorKind::PageMismatch {
                        reference,
                        referenced_page: target.page_association,
                    },
                ));
            }
            if !allowed_target(source.segment_type, target.segment_type) {
                return Err(at(
                    source,
                    DirectoryErrorKind::ReferenceType {
                        reference,
                        referenced_type: target.segment_type,
                    },
                ));
            }
            if target.segment_type == 53 {
                tables = tables
                    .checked_add(1)
                    .ok_or(invalid_span(source, "table count overflows"))?;
                let limit = if source.segment_type == 0 { 4 } else { 8 };
                if tables > limit {
                    return Err(at(
                        source,
                        DirectoryErrorKind::TooManyTables {
                            limit,
                            attempted: tables,
                        },
                    ));
                }
            }
            if intermediate(target.segment_type) && source.segment_type != 62 {
                let entry = &mut index[target_index];
                if entry.non_extension_uses != 0 {
                    return Err(at(
                        source,
                        DirectoryErrorKind::IntermediateReused(reference),
                    ));
                }
                entry.non_extension_uses = 1;
            }
            if !target.retain_current() || index[target_index].expired {
                return Err(at(
                    source,
                    DirectoryErrorKind::RetentionViolation(reference),
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
pub async fn read_embedded_directory<S: RangedSource, C: Cancellation>(
    source: &mut S,
    span: SegmentSpan,
    limits: &Limits,
    header_limits: HeaderLimits,
    directory_limits: DirectoryLimits,
    cancellation: &C,
) -> Result<SegmentDirectory> {
    let end = validate_enclosing_span(source, span, limits)?;
    let directory_limits = DirectoryLimits {
        max_metadata_bytes: directory_limits
            .max_metadata_bytes
            .min(limits.max_allocation_bytes),
        ..directory_limits
    };
    if span.length > directory_limits.max_span_bytes {
        return Err(DirectoryError {
            offset: span.offset,
            segment: None,
            kind: DirectoryErrorKind::LimitExceeded {
                resource: "JBIG2 directory span bytes",
                limit: directory_limits.max_span_bytes,
                attempted: span.length,
            },
        });
    }
    let entry_bytes = (mem::size_of::<SegmentHeader>() as u64)
        .checked_add(mem::size_of::<IndexEntry>() as u64)
        .ok_or(DirectoryError {
            offset: span.offset,
            segment: None,
            kind: DirectoryErrorKind::InvalidSpan("entry size overflows"),
        })?;
    let mut segments = Vec::new();
    let mut next = span.offset;
    let mut references_used = 0_u64;
    let mut metadata_used = 0_u64;
    while next < end {
        check_cancelled(cancellation, next)?;
        let failure = unassigned(
            next,
            DirectoryErrorKind::InvalidSpan("segment count overflows"),
        );
        let count = len_u64(segments.len()).checked_add(1).ok_or(failure)?;
        if count > u64::from(directory_limits.max_segments) {
            return Err(DirectoryError {
                offset: next,
                segment: None,
                kind: DirectoryErrorKind::LimitExceeded {
                    resource: "JBIG2 directory segments",
                    limit: u64::from(directory_limits.max_segments),
                    attempted: count,
                },
            });
        }
        let entry_total = checked_total(metadata_used, entry_bytes, next)?;
        if entry_total > directory_limits.max_metadata_bytes {
            return Err(DirectoryError {
                offset: next,
                segment: None,
                kind: DirectoryErrorKind::LimitExceeded {
                    resource: "JBIG2 directory metadata bytes",
                    limit: directory_limits.max_metadata_bytes,
                    attempted: entry_total,
                },
            });
        }
        let refused = unassigned(next, DirectoryErrorKind::AllocationFailed);
        reserve_exact(&mut segments, 1, refused)?;
        let budget = PrefixBudget {
            metadata_used: entry_total,
            metadata_limit: directory_limits.max_metadata_bytes,
            references_used,
            references_limit: u64::from(directory_limits.max_total_references),
        };
        let (header, after) = read_header_prefix(
            source,
            next,
            end,
            limits,
            header_limits,
            Some(budget),
            cancellation,
        )
        .await?;
        if after <= next {
            return Err(invalid_span(&header, "segment did not advance"));
        }
        let header_metadata = header
            .metadata_bytes()
            .ok_or(invalid_span(&header, "metadata size overflows"))?;
        metadata_used = checked_total(entry_total, header_metadata, next)?;
        references_used = checked_total(references_used, len_u64(header.referred_to.len()), next)?;
        segments.push(header);
        next = after;
        check_cancelled(cancellation, next)?;
    }
    validate_graph(&segments, metadata_used, directory_limits, cancellation)?;
    check_cancelled(cancellation, end)?;
    Ok(SegmentDirectory { span, segments })
}
