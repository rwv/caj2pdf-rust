// SPDX-License-Identifier: MIT

use caj2pdf_core::{
    Cancellation, Limits, NeverCancel, RangedSource,
    jbig2::{
        DirectoryError, DirectoryErrorKind, DirectoryLimits, HeaderErrorKind, HeaderLimits,
        SegmentDirectory, SegmentSpan, read_embedded_directory, read_segment_header,
    },
};
use std::{
    cell::Cell,
    future::Future,
    pin::pin,
    rc::Rc,
    task::{Context, Poll, Waker},
};

fn run<F: Future>(future: F) -> F::Output {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = pin!(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("in-memory source unexpectedly yielded"),
    }
}

struct SpySource {
    bytes: Vec<u8>,
    size: u64,
    max_read: usize,
    reads: Rc<Cell<usize>>,
    ranges: Vec<(u64, usize)>,
}

impl SpySource {
    fn new(bytes: &[u8]) -> Self {
        Self {
            bytes: bytes.to_vec(),
            size: bytes.len() as u64,
            max_read: usize::MAX,
            reads: Rc::new(Cell::new(0)),
            ranges: Vec::new(),
        }
    }
}

impl RangedSource for SpySource {
    fn size(&self) -> u64 {
        self.size
    }

    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        self.reads.set(self.reads.get() + 1);
        self.ranges.push((offset, destination.len()));
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let count = self
            .bytes
            .len()
            .saturating_sub(start)
            .min(destination.len())
            .min(self.max_read);
        if count != 0 {
            destination[..count].copy_from_slice(&self.bytes[start..start + count]);
        }
        Ok(count)
    }
}

fn parse(bytes: &[u8]) -> Result<SegmentDirectory, caj2pdf_core::jbig2::DirectoryError> {
    let mut source = SpySource::new(bytes);
    run(read_embedded_directory(
        &mut source,
        SegmentSpan {
            offset: 0,
            length: bytes.len() as u64,
        },
        &Limits::default(),
        HeaderLimits::default(),
        DirectoryLimits::default(),
        &NeverCancel,
    ))
}

// Original synthetic headers. Their one- and two-byte payloads are invented.
// These fixed arrays keep the expected offsets independent of the parser.
const PAGE: &[u8] = &[0, 0, 0, 0, 48, 0, 1, 0, 0, 0, 2, 0x11, 0x22];
const DICTIONARY: &[u8] = &[0, 0, 0, 1, 0, 1, 1, 0, 0, 0, 1, 0x33];
const REFERRED_DICTIONARY: &[u8] = &[0, 0, 0, 2, 0, 0x21, 1, 1, 0, 0, 0, 0];
const TEXT: &[u8] = &[0, 0, 0, 3, 6, 0x20, 2, 1, 0, 0, 0, 1, 0x44];
const GENERIC: &[u8] = &[0, 0, 0, 4, 38, 0, 1, 0, 0, 0, 0];

fn observed_shape() -> Vec<u8> {
    [PAGE, DICTIONARY, REFERRED_DICTIONARY, TEXT, GENERIC].concat()
}

// Build only short-form, one-byte-reference synthetic variants. The primary
// observed-shape test above uses fixed bytes and independently listed fields.
fn segment(number: u8, kind: u8, page: u8, refs: &[u8], keep_self: bool, data: &[u8]) -> Vec<u8> {
    assert!(refs.len() <= 4);
    let retention = u8::from(keep_self) | ((1_u8 << refs.len()) - 1) << 1;
    let mut bytes = vec![0, 0, 0, number, kind, ((refs.len() as u8) << 5) | retention];
    bytes.extend_from_slice(refs);
    bytes.push(page);
    bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
    bytes.extend_from_slice(data);
    bytes
}

fn join(parts: &[Vec<u8>]) -> Vec<u8> {
    parts.concat()
}

#[test]
fn indexes_observed_five_type_shape_with_exact_data_offsets() {
    let bytes = observed_shape();
    let directory = parse(&bytes).unwrap();
    assert_eq!(directory.span.length, 61);
    assert_eq!(directory.segments.len(), 5);
    assert_eq!(
        directory
            .segments
            .iter()
            .map(|segment| (
                segment.number,
                segment.segment_type,
                segment.page_association
            ))
            .collect::<Vec<_>>(),
        [(0, 48, 1), (1, 0, 1), (2, 0, 1), (3, 6, 1), (4, 38, 1)]
    );
    assert_eq!(
        directory
            .segments
            .iter()
            .map(|segment| segment.referred_to.as_slice())
            .collect::<Vec<_>>(),
        [&[][..], &[], &[1], &[2], &[]]
    );
    assert_eq!(
        directory
            .segments
            .iter()
            .map(|segment| segment.data)
            .collect::<Vec<_>>(),
        [
            SegmentSpan {
                offset: 11,
                length: 2,
            },
            SegmentSpan {
                offset: 24,
                length: 1,
            },
            SegmentSpan {
                offset: 37,
                length: 0,
            },
            SegmentSpan {
                offset: 49,
                length: 1,
            },
            SegmentSpan {
                offset: 61,
                length: 0,
            },
        ]
    );
}

#[test]
fn accepts_embedded_physical_reordering_and_zero_data() {
    let bytes = [TEXT, DICTIONARY, GENERIC, PAGE, REFERRED_DICTIONARY].concat();
    let directory = parse(&bytes).unwrap();
    assert_eq!(
        directory
            .segments
            .iter()
            .map(|segment| segment.number)
            .collect::<Vec<_>>(),
        [3, 1, 4, 0, 2]
    );
    assert_eq!(directory.segments[2].data.length, 0);
    assert_eq!(directory.segments[2].data.offset, 36);
    assert_eq!(directory.segments[4].data.offset, bytes.len() as u64);
}

#[test]
fn an_empty_embedded_span_has_no_segments_or_source_reads() {
    let mut source = SpySource::new(&[0xaa, 0xbb]);
    let directory = run(read_embedded_directory(
        &mut source,
        SegmentSpan {
            offset: 1,
            length: 0,
        },
        &Limits::default(),
        HeaderLimits::default(),
        DirectoryLimits::default(),
        &NeverCancel,
    ))
    .unwrap();
    assert_eq!(directory.span.offset, 1);
    assert!(directory.segments.is_empty());
    assert!(source.ranges.is_empty());
}

#[test]
fn general_directory_does_not_require_an_observed_profile_page_header() {
    let region_only = segment(0, 38, 1, &[], false, &[]);
    let directory = parse(&region_only).unwrap();
    assert_eq!(directory.segments.len(), 1);
    assert_eq!(directory.segments[0].segment_type, 38);
}

#[test]
fn skips_payload_bytes_and_never_reads_outside_the_given_span() {
    let mut bytes = vec![0xaa; 7];
    bytes.extend(observed_shape());
    bytes.extend([0xbb; 4]);
    let mut source = SpySource::new(&bytes);
    source.max_read = 1;
    let directory = run(read_embedded_directory(
        &mut source,
        SegmentSpan {
            offset: 7,
            length: 61,
        },
        &Limits {
            io_chunk_bytes: 1,
            ..Limits::default()
        },
        HeaderLimits::default(),
        DirectoryLimits::default(),
        &NeverCancel,
    ))
    .unwrap();
    assert_eq!(directory.segments[0].data.offset, 18);
    assert_eq!(directory.segments[4].data.offset, 68);
    assert_eq!(source.ranges.len(), 57);
    assert!(source.ranges.iter().all(|(offset, size)| {
        *size == 1
            && directory.segments.iter().any(|segment| {
                let start = segment.data.offset - segment.header_length;
                *offset >= start && *offset < segment.data.offset
            })
    }));
}

#[test]
fn reports_truncation_extra_bytes_and_unknown_data_length() {
    let mut short = observed_shape();
    short.pop();
    let error = parse(&short).unwrap_err();
    assert_eq!(error.offset, 60);
    assert!(matches!(
        error.kind,
        DirectoryErrorKind::Header(caj2pdf_core::jbig2::HeaderError {
            kind: HeaderErrorKind::Truncated(_),
            ..
        })
    ));

    let mut extra = observed_shape();
    extra.push(0xee);
    let error = parse(&extra).unwrap_err();
    assert_eq!(error.offset, 62);
    assert!(matches!(
        error.kind,
        DirectoryErrorKind::Header(caj2pdf_core::jbig2::HeaderError {
            kind: HeaderErrorKind::Truncated(_),
            ..
        })
    ));

    let mut unknown = observed_shape();
    unknown[57..61].fill(0xff);
    let error = parse(&unknown).unwrap_err();
    assert_eq!(error.segment, Some(4));
    assert!(matches!(
        error.kind,
        DirectoryErrorKind::Header(caj2pdf_core::jbig2::HeaderError {
            kind: HeaderErrorKind::Unsupported {
                feature: "unknown segment data length",
                ..
            },
            ..
        })
    ));

    let mut source = SpySource::new(PAGE);
    let error = run(read_segment_header(
        &mut source,
        SegmentSpan {
            offset: 0,
            length: PAGE.len() as u64 + 1,
        },
        &Limits::default(),
        HeaderLimits::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(matches!(error.kind, HeaderErrorKind::InvalidSpan(_)));
    let mut source = SpySource::new(&[PAGE, &[0xee]].concat());
    let error = run(read_segment_header(
        &mut source,
        SegmentSpan {
            offset: 0,
            length: PAGE.len() as u64 + 1,
        },
        &Limits::default(),
        HeaderLimits::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(matches!(
        error.kind,
        HeaderErrorKind::Malformed("bytes follow declared segment data")
    ));
}

#[test]
fn rejects_duplicate_missing_self_forward_and_repeated_references() {
    let duplicate = join(&[
        segment(1, 0, 0, &[], true, &[]),
        segment(1, 0, 0, &[], true, &[]),
    ]);
    let error = parse(&duplicate).unwrap_err();
    assert_eq!(error.offset, 11);
    assert!(matches!(error.kind, DirectoryErrorKind::DuplicateNumber(1)));

    let missing = segment(1, 0, 0, &[0], true, &[]);
    let error = parse(&missing).unwrap_err();
    assert_eq!(error.segment, Some(1));
    assert!(matches!(
        error.kind,
        DirectoryErrorKind::MissingReference(0)
    ));

    for reference in [1, 2] {
        let error = parse(&segment(1, 0, 0, &[reference], true, &[])).unwrap_err();
        assert!(matches!(
            error.kind,
            DirectoryErrorKind::Header(caj2pdf_core::jbig2::HeaderError {
                kind: HeaderErrorKind::Malformed("reference is not lower than segment number"),
                ..
            })
        ));
    }

    let repeated = join(&[
        segment(0, 0, 0, &[], true, &[]),
        segment(1, 0, 0, &[0, 0], true, &[]),
    ]);
    let error = parse(&repeated).unwrap_err();
    assert_eq!(error.segment, Some(1));
    assert!(matches!(
        error.kind,
        DirectoryErrorKind::DuplicateReference(0)
    ));
}

#[test]
fn rejects_cross_page_and_global_to_paged_references() {
    for source_page in [0, 1] {
        let bytes = join(&[
            segment(0, 0, 2, &[], true, &[]),
            segment(1, 0, source_page, &[0], true, &[]),
        ]);
        let error = parse(&bytes).unwrap_err();
        assert_eq!(error.segment, Some(1));
        assert!(matches!(
            error.kind,
            DirectoryErrorKind::PageMismatch {
                reference: 0,
                referenced_page: 2
            }
        ));
    }
    let global = join(&[
        segment(0, 0, 0, &[], true, &[]),
        segment(1, 0, 1, &[0], true, &[]),
    ]);
    assert_eq!(parse(&global).unwrap().segments.len(), 2);
}

#[test]
fn checks_target_types_table_caps_and_intermediate_use() {
    for (source_type, target_type, page) in [(0, 38, 1), (6, 16, 1), (22, 0, 1), (42, 0, 1)] {
        let bytes = join(&[
            segment(0, target_type, page, &[], true, &[]),
            segment(1, source_type, page, &[0], true, &[]),
        ]);
        let error = parse(&bytes).unwrap_err();
        assert_eq!(error.segment, Some(1));
        assert!(matches!(
            error.kind,
            DirectoryErrorKind::ReferenceType { .. }
        ));
    }

    let mut tables = Vec::new();
    for number in 0..5 {
        tables.push(segment(number, 53, 0, &[], true, &[]));
    }
    // Five long-form references to table segments exceed a symbol
    // dictionary's T.88 maximum of four.
    tables.push(vec![
        0, 0, 0, 5, 0, 0xe0, 0, 0, 5, 0x3f, 0, 1, 2, 3, 4, 1, 0, 0, 0, 0,
    ]);
    let error = parse(&join(&tables)).unwrap_err();
    assert!(matches!(
        error.kind,
        DirectoryErrorKind::TooManyTables {
            limit: 4,
            attempted: 5
        }
    ));

    let reused = join(&[
        segment(0, 36, 1, &[], true, &[]),
        segment(1, 42, 1, &[0], true, &[]),
        segment(2, 42, 1, &[0], true, &[]),
    ]);
    let error = parse(&reused).unwrap_err();
    assert_eq!(error.segment, Some(2));
    assert!(matches!(
        error.kind,
        DirectoryErrorKind::IntermediateReused(0)
    ));

    let extension_then_region = join(&[
        segment(0, 36, 1, &[], true, &[]),
        segment(1, 62, 1, &[0], true, &[]),
        segment(2, 42, 1, &[0], true, &[]),
    ]);
    assert_eq!(parse(&extension_then_region).unwrap().segments.len(), 3);
}

#[test]
fn duplicate_reference_scratch_obeys_the_combined_metadata_cap() {
    let bytes = join(&[
        segment(0, 0, 0, &[], true, &[]),
        segment(1, 0, 0, &[], true, &[]),
        segment(2, 0, 0, &[0, 1], true, &[]),
    ]);
    // Find a boundary where owned metadata fits but the temporary duplicate
    // reference scratch does not. This does not depend on struct layout.
    let scratch_failure = (1..=4096).find_map(|cap| {
        let mut source = SpySource::new(&bytes);
        let result = run(read_embedded_directory(
            &mut source,
            SegmentSpan {
                offset: 0,
                length: bytes.len() as u64,
            },
            &Limits::default(),
            HeaderLimits::default(),
            DirectoryLimits {
                max_metadata_bytes: cap,
                ..DirectoryLimits::default()
            },
            &NeverCancel,
        ));
        match result {
            Err(error)
                if error.segment == Some(2)
                    && matches!(
                        error.kind,
                        DirectoryErrorKind::LimitExceeded {
                            resource: "JBIG2 directory metadata bytes",
                            ..
                        }
                    ) =>
            {
                Some(error)
            }
            _ => None,
        }
    });
    assert!(scratch_failure.is_some());
}

#[test]
fn checks_retention_across_number_order() {
    let never_retained = join(&[
        segment(0, 0, 0, &[], false, &[]),
        segment(1, 0, 0, &[0], true, &[]),
    ]);
    assert!(matches!(
        parse(&never_retained).unwrap_err().kind,
        DirectoryErrorKind::RetentionViolation(0)
    ));

    let mut last_use = segment(1, 0, 0, &[0], true, &[]);
    last_use[5] = 0x21; // the reference retain bit is clear
    let expired = join(&[
        segment(2, 0, 0, &[0], true, &[]),
        segment(0, 0, 0, &[], true, &[]),
        last_use,
    ]);
    let error = parse(&expired).unwrap_err();
    assert_eq!(error.segment, Some(2));
    assert!(matches!(
        error.kind,
        DirectoryErrorKind::RetentionViolation(0)
    ));
}

#[test]
fn preflights_span_count_reference_and_metadata_caps() {
    let bytes = observed_shape();
    let mut source = SpySource::new(&bytes);
    let error = run(read_embedded_directory(
        &mut source,
        SegmentSpan {
            offset: 0,
            length: bytes.len() as u64,
        },
        &Limits::default(),
        HeaderLimits::default(),
        DirectoryLimits {
            max_span_bytes: 60,
            ..DirectoryLimits::default()
        },
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(source.ranges.is_empty());
    assert!(matches!(
        error.kind,
        DirectoryErrorKind::LimitExceeded {
            resource: "JBIG2 directory span bytes",
            ..
        }
    ));

    let mut source = SpySource::new(&bytes);
    let error = run(read_embedded_directory(
        &mut source,
        SegmentSpan {
            offset: 0,
            length: bytes.len() as u64,
        },
        &Limits::default(),
        HeaderLimits::default(),
        DirectoryLimits {
            max_segments: 2,
            ..DirectoryLimits::default()
        },
        &NeverCancel,
    ))
    .unwrap_err();
    assert_eq!(error.offset, 25);
    assert!(source.ranges.iter().all(|(offset, _)| *offset < 25));
    assert!(matches!(
        error.kind,
        DirectoryErrorKind::LimitExceeded {
            resource: "JBIG2 directory segments",
            ..
        }
    ));

    let mut source = SpySource::new(&bytes);
    let error = run(read_embedded_directory(
        &mut source,
        SegmentSpan {
            offset: 0,
            length: bytes.len() as u64,
        },
        &Limits::default(),
        HeaderLimits::default(),
        DirectoryLimits {
            max_total_references: 1,
            ..DirectoryLimits::default()
        },
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(matches!(
        error.kind,
        DirectoryErrorKind::Header(caj2pdf_core::jbig2::HeaderError {
            kind: HeaderErrorKind::LimitExceeded {
                resource: "JBIG2 directory references",
                ..
            },
            ..
        })
    ));

    let mut source = SpySource::new(&bytes);
    let error = run(read_embedded_directory(
        &mut source,
        SegmentSpan {
            offset: 0,
            length: bytes.len() as u64,
        },
        &Limits::default(),
        HeaderLimits::default(),
        DirectoryLimits {
            max_metadata_bytes: 1,
            ..DirectoryLimits::default()
        },
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(source.ranges.is_empty());
    assert!(matches!(
        error.kind,
        DirectoryErrorKind::LimitExceeded {
            resource: "JBIG2 directory metadata bytes",
            ..
        }
    ));

    let mut source = SpySource::new(PAGE);
    source.size = u64::MAX;
    let error = run(read_embedded_directory(
        &mut source,
        SegmentSpan {
            offset: u64::MAX - 2,
            length: 10,
        },
        &Limits::default(),
        HeaderLimits::default(),
        DirectoryLimits::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(source.ranges.is_empty());
    assert!(matches!(
        error.kind,
        DirectoryErrorKind::Header(caj2pdf_core::jbig2::HeaderError {
            kind: HeaderErrorKind::InvalidSpan(_),
            ..
        })
    ));
}

struct CancelAtGraph {
    reads: Rc<Cell<usize>>,
    expected_reads: usize,
    checks_after_reads: Cell<usize>,
}

impl Cancellation for CancelAtGraph {
    fn is_cancelled(&self) -> bool {
        if self.reads.get() == self.expected_reads {
            let count = self.checks_after_reads.get() + 1;
            self.checks_after_reads.set(count);
            count >= 5
        } else {
            false
        }
    }
}

#[test]
fn cancellation_applies_during_graph_validation_and_one_byte_reads() {
    let bytes = observed_shape();
    let mut baseline = SpySource::new(&bytes);
    baseline.max_read = 1;
    run(read_embedded_directory(
        &mut baseline,
        SegmentSpan {
            offset: 0,
            length: bytes.len() as u64,
        },
        &Limits {
            io_chunk_bytes: 1,
            ..Limits::default()
        },
        HeaderLimits::default(),
        DirectoryLimits::default(),
        &NeverCancel,
    ))
    .unwrap();
    let mut source = SpySource::new(&bytes);
    source.max_read = 1;
    let cancellation = CancelAtGraph {
        reads: source.reads.clone(),
        expected_reads: baseline.ranges.len(),
        checks_after_reads: Cell::new(0),
    };
    let error = run(read_embedded_directory(
        &mut source,
        SegmentSpan {
            offset: 0,
            length: bytes.len() as u64,
        },
        &Limits {
            io_chunk_bytes: 1,
            ..Limits::default()
        },
        HeaderLimits::default(),
        DirectoryLimits::default(),
        &cancellation,
    ))
    .unwrap_err();
    assert_eq!(source.ranges.len(), baseline.ranges.len());
    assert!(cancellation.checks_after_reads.get() >= 5);
    assert!(matches!(error.kind, DirectoryErrorKind::Cancelled));
}

#[test]
fn public_directory_errors_keep_location_and_underlying_header_cause() {
    let cases = [
        (DirectoryErrorKind::InvalidSpan("end"), "invalid span: end"),
        (
            DirectoryErrorKind::LimitExceeded {
                resource: "segments",
                limit: 2,
                attempted: 3,
            },
            "segments limit 2 exceeded by 3",
        ),
        (
            DirectoryErrorKind::AllocationFailed,
            "directory allocation failed",
        ),
        (DirectoryErrorKind::Cancelled, "cancelled"),
        (
            DirectoryErrorKind::DuplicateNumber(8),
            "duplicate segment number 8",
        ),
        (
            DirectoryErrorKind::DuplicateReference(2),
            "duplicate reference to segment 2",
        ),
        (
            DirectoryErrorKind::MissingReference(2),
            "missing reference to segment 2",
        ),
        (
            DirectoryErrorKind::PageMismatch {
                reference: 2,
                referenced_page: 3,
            },
            "reference to segment 2 on disallowed page 3",
        ),
        (
            DirectoryErrorKind::ReferenceType {
                reference: 2,
                referenced_type: 38,
            },
            "reference to segment 2 has disallowed type 38",
        ),
        (
            DirectoryErrorKind::TooManyTables {
                limit: 4,
                attempted: 5,
            },
            "tables reference limit 4 exceeded by 5",
        ),
        (
            DirectoryErrorKind::IntermediateReused(2),
            "intermediate segment 2 has multiple non-extension users",
        ),
        (
            DirectoryErrorKind::RetentionViolation(2),
            "segment 2 was referenced after non-retention",
        ),
    ];
    for (kind, detail) in cases {
        let error = DirectoryError {
            offset: 17,
            segment: Some(8),
            kind,
        };
        assert_eq!(
            error.to_string(),
            format!("JBIG2 directory at source byte 17, segment 8: {detail}")
        );
        assert!(std::error::Error::source(&error).is_none());
    }
    let header_error = parse(&[0]).unwrap_err();
    assert!(header_error.to_string().contains("JBIG2 segment header"));
    assert!(std::error::Error::source(&header_error).is_some());
}

#[test]
fn directory_wide_errors_render_without_a_segment_number() {
    let bytes = join(&[
        segment(0, 0, 0, &[], true, &[]),
        segment(1, 0, 0, &[], true, &[]),
    ]);
    let mut source = SpySource::new(&bytes);
    let error = run(read_embedded_directory(
        &mut source,
        SegmentSpan {
            offset: 0,
            length: bytes.len() as u64,
        },
        &Limits::default(),
        HeaderLimits::default(),
        DirectoryLimits {
            max_segments: 1,
            ..DirectoryLimits::default()
        },
        &NeverCancel,
    ))
    .unwrap_err();
    assert_eq!((error.offset, error.segment), (11, None));
    assert_eq!(
        error.to_string(),
        "JBIG2 directory at source byte 11: JBIG2 directory segments limit 1 exceeded by 2"
    );
}

#[test]
fn reference_scratch_is_reused_and_cleared_between_segments() {
    let shared = join(&[
        segment(0, 0, 1, &[], true, &[]),
        segment(1, 0, 1, &[], true, &[]),
        segment(2, 6, 1, &[0, 1], true, &[]),
        segment(3, 6, 1, &[1, 0], true, &[]),
    ]);
    let directory = parse(&shared).unwrap();
    assert_eq!(
        directory
            .segments
            .iter()
            .map(|segment| segment.referred_to.as_slice())
            .collect::<Vec<_>>(),
        [&[][..], &[], &[0, 1], &[1, 0]]
    );

    // The second two-reference list fits the existing scratch capacity; a
    // repeat inside it must still be found and attributed to that segment.
    let repeated = join(&[
        segment(0, 0, 1, &[], true, &[]),
        segment(1, 0, 1, &[], true, &[]),
        segment(2, 6, 1, &[0, 1], true, &[]),
        segment(3, 6, 1, &[1, 1], true, &[]),
    ]);
    let error = parse(&repeated).unwrap_err();
    assert_eq!(error.segment, Some(3));
    assert!(matches!(
        error.kind,
        DirectoryErrorKind::DuplicateReference(1)
    ));
}
