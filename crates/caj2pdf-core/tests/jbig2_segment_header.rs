// SPDX-License-Identifier: MIT

use caj2pdf_core::{
    Cancellation, Error, Limits, NeverCancel, RangedSource,
    jbig2::{
        DirectoryError, DirectoryErrorKind, DirectoryLimits, HeaderError, HeaderErrorKind,
        HeaderLimits, SegmentDirectory, SegmentSpan, read_embedded_directory, read_segment_header,
    },
};
use std::{
    cell::Cell,
    future::Future,
    io,
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

struct TestSource {
    bytes: Vec<u8>,
    size: u64,
    max_read: usize,
    reads: Vec<(u64, usize)>,
    cancel_after: Option<(usize, Rc<Cell<bool>>)>,
    source_cancel_at: Option<u64>,
    fail_at: Option<u64>,
    overreport: bool,
}

impl TestSource {
    fn new(bytes: &[u8]) -> Self {
        Self {
            bytes: bytes.to_vec(),
            size: bytes.len() as u64,
            max_read: usize::MAX,
            reads: Vec::new(),
            cancel_after: None,
            source_cancel_at: None,
            fail_at: None,
            overreport: false,
        }
    }
}

impl RangedSource for TestSource {
    fn size(&self) -> u64 {
        self.size
    }

    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        self.reads.push((offset, destination.len()));
        if self.source_cancel_at == Some(offset) {
            return Err(Error::Cancelled);
        }
        if self.fail_at == Some(offset) {
            return Err(Error::Io(io::Error::other("test I/O failure")));
        }
        if self.overreport {
            return Ok(destination.len() + 1);
        }
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
        if let Some((after, flag)) = &self.cancel_after {
            if self.reads.len() == *after {
                flag.set(true);
            }
        }
        Ok(count)
    }
}

struct Flag(Rc<Cell<bool>>);

impl Cancellation for Flag {
    fn is_cancelled(&self) -> bool {
        self.0.get()
    }
}

fn parse(
    bytes: &[u8],
) -> Result<caj2pdf_core::jbig2::SegmentHeader, caj2pdf_core::jbig2::HeaderError> {
    let mut source = TestSource::new(bytes);
    run(read_segment_header(
        &mut source,
        SegmentSpan {
            offset: 0,
            length: bytes.len() as u64,
        },
        &Limits::default(),
        HeaderLimits::default(),
        &NeverCancel,
    ))
}

// Original synthetic fields: global tables segment 0, no references or data.
const GLOBAL_EMPTY: &[u8] = &[0, 0, 0, 0, 53, 1, 0, 0, 0, 0, 0];
// Segment 256 uses one-byte references, a short count of four, page 255,
// and two uninterpreted data bytes. These are invented test bytes.
const SHORT_FOUR: &[u8] = &[
    0, 0, 1, 0, 0x80, 0x95, 0, 1, 127, 255, 255, 0, 0, 0, 2, 0xab, 0xcd,
];
// Segment 257 uses two-byte references, five long-form references, a
// four-byte page association, and zero data bytes.
const LONG_FIVE: &[u8] = &[
    0, 0, 1, 1, 0x40, 0xe0, 0, 0, 5, 0x2b, 0, 0, 0, 1, 0, 255, 1, 0, 0, 12, 1, 2, 3, 4, 0, 0, 0, 0,
];

#[test]
fn reads_short_and_long_forms_with_independent_expected_fields() {
    let global = parse(GLOBAL_EMPTY).unwrap();
    assert_eq!(global.number, 0);
    assert_eq!(global.segment_type, 53);
    assert_eq!(global.page_association, 0);
    assert!(global.referred_to.is_empty());
    assert!(global.retain_current());
    assert_eq!(global.retain_reference(0), None);
    assert_eq!(global.header_length, 11);
    assert_eq!(
        global.data,
        SegmentSpan {
            offset: 11,
            length: 0
        }
    );

    let short = parse(SHORT_FOUR).unwrap();
    assert_eq!(short.number, 256);
    assert_eq!(short.segment_type, 0);
    assert!(short.deferred_non_retain);
    assert_eq!(short.referred_to, [0, 1, 127, 255]);
    assert_eq!(short.page_association, 255);
    assert!(short.retain_current());
    assert_eq!(short.retain_reference(0), Some(false));
    assert_eq!(short.retain_reference(1), Some(true));
    assert_eq!(short.retain_reference(2), Some(false));
    assert_eq!(short.retain_reference(3), Some(true));
    assert_eq!(short.retain_reference(4), None);
    assert_eq!(short.header_length, 15);
    assert_eq!(
        short.data,
        SegmentSpan {
            offset: 15,
            length: 2
        }
    );

    let long = parse(LONG_FIVE).unwrap();
    assert_eq!(long.number, 257);
    assert_eq!(long.segment_type, 0);
    assert_eq!(long.referred_to, [0, 1, 255, 256, 12]);
    assert_eq!(long.page_association, 0x0102_0304);
    assert_eq!(long.header_length, 28);
    assert_eq!(
        long.data,
        SegmentSpan {
            offset: 28,
            length: 0
        }
    );
    assert!(long.retain_current());
    assert_eq!(long.retain_reference(0), Some(true));
    assert_eq!(long.retain_reference(1), Some(false));
    assert_eq!(long.retain_reference(2), Some(true));
    assert_eq!(long.retain_reference(3), Some(false));
    assert_eq!(long.retain_reference(4), Some(true));
}

#[test]
fn reference_width_boundaries_are_inclusive() {
    // Segment 65536 still uses two-byte reference IDs.
    let two = parse(&[0, 1, 0, 0, 0, 0x22, 255, 255, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!(two.referred_to, [65535]);
    assert_eq!(two.header_length, 13);
    // Segment 65537 uses four-byte reference IDs.
    let four = parse(&[0, 1, 0, 1, 0, 0x22, 0, 1, 0, 0, 0, 0, 0, 0, 0]).unwrap();
    assert_eq!(four.referred_to, [65536]);
    assert_eq!(four.header_length, 15);
    // A reference to segment 0 is valid for nonzero current numbers.
    assert_eq!(
        parse(&[0, 0, 0, 1, 0, 0x22, 0, 0, 0, 0, 0, 0])
            .unwrap()
            .referred_to,
        [0]
    );
}

#[test]
fn every_short_count_and_long_retention_byte_boundary() {
    for (count, bitfield) in [(0, 1), (1, 3), (2, 5), (3, 9), (4, 17)] {
        let mut bytes = vec![0, 0, 0, 20, 0, ((count as u8) << 5) | bitfield];
        bytes.extend((0..count).map(|value| value as u8));
        bytes.extend([0, 0, 0, 0, 0]);
        let header = parse(&bytes).unwrap();
        assert_eq!(header.referred_to.len(), count);
        assert!(header.retain_current());
    }

    // The retained bits straddle byte boundaries at counts 8 and 16.
    for (count, retention) in [
        (7_u8, vec![0b1000_0001]),
        (8, vec![0b1000_0001, 0b0000_0001]),
        (15, vec![0b1000_0001, 0b1000_0001]),
        (16, vec![0b1000_0001, 0b1000_0001, 0b0000_0001]),
    ] {
        let mut bytes = vec![0, 1, 0, 1, 0, 0xe0, 0, 0, count];
        bytes.extend(retention);
        for reference in 0..count {
            bytes.extend(u32::from(reference).to_be_bytes());
        }
        bytes.extend([0, 0, 0, 0, 0]);
        let header = parse(&bytes).unwrap();
        assert_eq!(header.referred_to.len(), usize::from(count));
        assert_eq!(header.referred_to[0], 0);
        assert_eq!(
            header.referred_to[usize::from(count) - 1],
            u32::from(count - 1)
        );
        assert!(header.retain_current());
        assert_eq!(header.retain_reference(6), Some(true));
        if count >= 8 {
            assert_eq!(header.retain_reference(7), Some(true));
        }
        if count == 16 {
            assert_eq!(header.retain_reference(15), Some(true));
        }
    }
}

#[test]
fn bounded_reads_use_absolute_offsets_and_ignore_data_bytes() {
    let mut bytes = vec![0xaa; 7];
    bytes.extend(SHORT_FOUR);
    bytes.extend([0xbb; 5]);
    let mut source = TestSource::new(&bytes);
    source.max_read = 1;
    let limits = Limits {
        io_chunk_bytes: 1,
        ..Limits::default()
    };
    let header = run(read_segment_header(
        &mut source,
        SegmentSpan {
            offset: 7,
            length: SHORT_FOUR.len() as u64,
        },
        &limits,
        HeaderLimits::default(),
        &NeverCancel,
    ))
    .unwrap();
    assert_eq!(
        header.data,
        SegmentSpan {
            offset: 22,
            length: 2
        }
    );
    assert_eq!(source.reads.len(), 15);
    assert!(
        source
            .reads
            .iter()
            .all(|(offset, count)| *offset >= 7 && *offset < 22 && *count == 1)
    );
}

#[test]
fn truncated_fields_never_read_beyond_declared_span() {
    let mut source = TestSource::new(LONG_FIVE);
    for length in 0..LONG_FIVE.len() {
        source.reads.clear();
        let error = run(read_segment_header(
            &mut source,
            SegmentSpan {
                offset: 0,
                length: length as u64,
            },
            &Limits::default(),
            HeaderLimits::default(),
            &NeverCancel,
        ))
        .unwrap_err();
        assert!(
            matches!(error.kind, HeaderErrorKind::Truncated(_)),
            "{length}: {error}"
        );
        assert_eq!(error.offset, length as u64);
        assert!(
            source
                .reads
                .iter()
                .all(|(offset, requested)| *offset + *requested as u64 <= length as u64)
        );
    }
    let mut physically_short = TestSource::new(&LONG_FIVE[..12]);
    physically_short.size = LONG_FIVE.len() as u64;
    let error = run(read_segment_header(
        &mut physically_short,
        SegmentSpan {
            offset: 0,
            length: LONG_FIVE.len() as u64,
        },
        &Limits::default(),
        HeaderLimits::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(matches!(
        error.kind,
        HeaderErrorKind::Truncated("reference number")
    ));
    assert_eq!(error.offset, 12);
}

#[test]
fn malformed_and_unsupported_forms_are_located() {
    let mut reserved_type = GLOBAL_EMPTY.to_vec();
    reserved_type[4] = 63;
    let error = parse(&reserved_type).unwrap_err();
    assert_eq!(error.offset, 5);
    assert_eq!(error.segment, Some(0));
    assert!(matches!(
        error.kind,
        HeaderErrorKind::Unsupported {
            feature: "reserved segment type",
            value: 63
        }
    ));

    for tag in [5_u8, 6] {
        let mut bytes = GLOBAL_EMPTY.to_vec();
        bytes[5] = tag << 5;
        let error = parse(&bytes).unwrap_err();
        assert!(matches!(
            error.kind,
            HeaderErrorKind::Unsupported {
                feature: "reserved reference-count form",
                ..
            }
        ));
    }
    let mut noncanonical = LONG_FIVE.to_vec();
    noncanonical[8] = 4;
    assert!(matches!(
        parse(&noncanonical).unwrap_err().kind,
        HeaderErrorKind::Malformed("noncanonical long reference count")
    ));

    let mut unused_short = GLOBAL_EMPTY.to_vec();
    unused_short[5] = 0b0001_0001;
    assert!(matches!(
        parse(&unused_short).unwrap_err().kind,
        HeaderErrorKind::Malformed("unused short retention bits")
    ));
    let mut unused_long = LONG_FIVE.to_vec();
    unused_long[9] = 0b1100_0000;
    assert!(matches!(
        parse(&unused_long).unwrap_err().kind,
        HeaderErrorKind::Malformed("unused long retention bits")
    ));

    // Unknown data length is allowed by T.88 only for an immediate generic
    // region, but this reader deliberately does not scan a terminator.
    let mut unknown = vec![0, 0, 0, 2, 38, 1, 1];
    unknown.extend(u32::MAX.to_be_bytes());
    let error = parse(&unknown).unwrap_err();
    assert_eq!(error.offset, 11);
    assert!(matches!(
        error.kind,
        HeaderErrorKind::Unsupported {
            feature: "unknown segment data length",
            ..
        }
    ));
}

#[test]
fn single_header_reference_and_page_rules() {
    let mut self_reference = vec![0, 0, 0, 5, 0, 0x22, 5, 0];
    self_reference.extend([0, 0, 0, 0]);
    assert!(matches!(
        parse(&self_reference).unwrap_err().kind,
        HeaderErrorKind::Malformed("reference is not lower than segment number")
    ));
    self_reference[6] = 6;
    assert!(matches!(
        parse(&self_reference).unwrap_err().kind,
        HeaderErrorKind::Malformed("reference is not lower than segment number")
    ));

    let mut pattern_with_reference = self_reference.clone();
    pattern_with_reference[4] = 16;
    assert!(matches!(
        parse(&pattern_with_reference).unwrap_err().kind,
        HeaderErrorKind::Malformed("reference count for segment type")
    ));
    let mut halftone_without_reference = GLOBAL_EMPTY.to_vec();
    halftone_without_reference[4] = 22;
    assert!(matches!(
        parse(&halftone_without_reference).unwrap_err().kind,
        HeaderErrorKind::Malformed("reference count for segment type")
    ));
    let mut refinement_with_two = vec![0, 0, 0, 4, 42, 0x40, 0, 1, 0];
    refinement_with_two.extend([0, 0, 0, 0]);
    assert!(matches!(
        parse(&refinement_with_two).unwrap_err().kind,
        HeaderErrorKind::Malformed("reference count for segment type")
    ));

    let mut global_region = GLOBAL_EMPTY.to_vec();
    global_region[4] = 38;
    assert!(matches!(
        parse(&global_region).unwrap_err().kind,
        HeaderErrorKind::Malformed("page association for segment type")
    ));
    let mut paged_eof = GLOBAL_EMPTY.to_vec();
    paged_eof[4] = 51;
    paged_eof[6] = 1;
    assert!(matches!(
        parse(&paged_eof).unwrap_err().kind,
        HeaderErrorKind::Malformed("page association for segment type")
    ));
    let mut valid_region = global_region;
    valid_region[6] = 1;
    assert_eq!(parse(&valid_region).unwrap().page_association, 1);
}

#[test]
fn span_data_and_allocation_limits_fail_before_unbounded_work() {
    let mut source = TestSource::new(GLOBAL_EMPTY);
    source.size = u64::MAX;
    let error = run(read_segment_header(
        &mut source,
        SegmentSpan {
            offset: u64::MAX - 2,
            length: 10,
        },
        &Limits::default(),
        HeaderLimits::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(matches!(error.kind, HeaderErrorKind::InvalidSpan(_)));
    assert!(source.reads.is_empty());

    let mut short_data = SHORT_FOUR.to_vec();
    short_data.pop();
    assert!(matches!(
        parse(&short_data).unwrap_err().kind,
        HeaderErrorKind::Truncated("segment data")
    ));
    let mut extra_data = SHORT_FOUR.to_vec();
    extra_data.push(0);
    assert!(matches!(
        parse(&extra_data).unwrap_err().kind,
        HeaderErrorKind::Malformed("bytes follow declared segment data")
    ));

    let mut huge = GLOBAL_EMPTY.to_vec();
    huge[4] = 0;
    huge[5..9].copy_from_slice(&[0xe0, 0xff, 0xff, 0xff]);
    let mut source = TestSource::new(&huge);
    let error = run(read_segment_header(
        &mut source,
        SegmentSpan {
            offset: 0,
            length: huge.len() as u64,
        },
        &Limits::default(),
        HeaderLimits::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(matches!(
        error.kind,
        HeaderErrorKind::LimitExceeded {
            resource: "JBIG2 references",
            ..
        }
    ));
    assert_eq!(source.reads.len(), 4);

    let mut source = TestSource::new(LONG_FIVE);
    let limits = Limits {
        io_chunk_bytes: 1,
        max_allocation_bytes: 20,
        ..Limits::default()
    };
    let error = run(read_segment_header(
        &mut source,
        SegmentSpan {
            offset: 0,
            length: LONG_FIVE.len() as u64,
        },
        &limits,
        HeaderLimits::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(matches!(
        error.kind,
        HeaderErrorKind::LimitExceeded {
            resource: "JBIG2 header metadata bytes",
            attempted: 21,
            ..
        }
    ));

    let mut source = TestSource::new(LONG_FIVE);
    let header_limits = HeaderLimits {
        max_header_bytes: 27,
        ..HeaderLimits::default()
    };
    assert!(matches!(
        run(read_segment_header(
            &mut source,
            SegmentSpan {
                offset: 0,
                length: LONG_FIVE.len() as u64
            },
            &Limits::default(),
            header_limits,
            &NeverCancel,
        ))
        .unwrap_err()
        .kind,
        HeaderErrorKind::LimitExceeded {
            resource: "JBIG2 header bytes",
            ..
        }
    ));

    let mut source = TestSource::new(SHORT_FOUR);
    let header_limits = HeaderLimits {
        max_data_bytes: 1,
        ..HeaderLimits::default()
    };
    assert!(matches!(
        run(read_segment_header(
            &mut source,
            SegmentSpan {
                offset: 0,
                length: SHORT_FOUR.len() as u64
            },
            &Limits::default(),
            header_limits,
            &NeverCancel,
        ))
        .unwrap_err()
        .kind,
        HeaderErrorKind::LimitExceeded {
            resource: "JBIG2 segment data bytes",
            ..
        }
    ));
}

#[test]
fn cancellation_and_source_failures_keep_locations() {
    let active = Rc::new(Cell::new(true));
    let mut source = TestSource::new(GLOBAL_EMPTY);
    let error = run(read_segment_header(
        &mut source,
        SegmentSpan {
            offset: 0,
            length: GLOBAL_EMPTY.len() as u64,
        },
        &Limits::default(),
        HeaderLimits::default(),
        &Flag(active),
    ))
    .unwrap_err();
    assert!(matches!(error.kind, HeaderErrorKind::Cancelled));
    assert!(source.reads.is_empty());

    let active = Rc::new(Cell::new(false));
    let mut source = TestSource::new(SHORT_FOUR);
    source.max_read = 1;
    source.cancel_after = Some((4, active.clone()));
    let error = run(read_segment_header(
        &mut source,
        SegmentSpan {
            offset: 0,
            length: SHORT_FOUR.len() as u64,
        },
        &Limits::default(),
        HeaderLimits::default(),
        &Flag(active),
    ))
    .unwrap_err();
    assert!(matches!(error.kind, HeaderErrorKind::Cancelled));
    assert_eq!(error.offset, 4);
    assert_eq!(source.reads.len(), 4);

    let active = Rc::new(Cell::new(false));
    let mut source = TestSource::new(LONG_FIVE);
    source.max_read = 1;
    source.cancel_after = Some((14, active.clone()));
    let error = run(read_segment_header(
        &mut source,
        SegmentSpan {
            offset: 0,
            length: LONG_FIVE.len() as u64,
        },
        &Limits::default(),
        HeaderLimits::default(),
        &Flag(active),
    ))
    .unwrap_err();
    assert!(matches!(error.kind, HeaderErrorKind::Cancelled));
    assert_eq!(error.segment, Some(257));
    assert_eq!(error.offset, 14);
    assert_eq!(source.reads.len(), 14);

    let active = Rc::new(Cell::new(false));
    let mut source = TestSource::new(GLOBAL_EMPTY);
    source.bytes.truncate(5);
    source.max_read = 1;
    source.cancel_after = Some((6, active.clone()));
    let error = run(read_segment_header(
        &mut source,
        SegmentSpan {
            offset: 0,
            length: GLOBAL_EMPTY.len() as u64,
        },
        &Limits::default(),
        HeaderLimits::default(),
        &Flag(active),
    ))
    .unwrap_err();
    assert!(matches!(error.kind, HeaderErrorKind::Cancelled));
    assert_eq!(error.offset, 5);
    assert_eq!(source.reads.len(), 6);

    let mut source = TestSource::new(GLOBAL_EMPTY);
    source.source_cancel_at = Some(5);
    let error = run(read_segment_header(
        &mut source,
        SegmentSpan {
            offset: 0,
            length: GLOBAL_EMPTY.len() as u64,
        },
        &Limits::default(),
        HeaderLimits::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert_eq!(error.offset, 5);
    assert!(matches!(error.kind, HeaderErrorKind::Cancelled));

    let mut source = TestSource::new(GLOBAL_EMPTY);
    source.fail_at = Some(5);
    let error = run(read_segment_header(
        &mut source,
        SegmentSpan {
            offset: 0,
            length: GLOBAL_EMPTY.len() as u64,
        },
        &Limits::default(),
        HeaderLimits::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert_eq!(error.offset, 5);
    assert!(matches!(error.kind, HeaderErrorKind::Source(Error::Io(_))));

    let mut source = TestSource::new(GLOBAL_EMPTY);
    source.overreport = true;
    assert!(matches!(
        run(read_segment_header(
            &mut source,
            SegmentSpan {
                offset: 0,
                length: GLOBAL_EMPTY.len() as u64
            },
            &Limits::default(),
            HeaderLimits::default(),
            &NeverCancel,
        ))
        .unwrap_err()
        .kind,
        HeaderErrorKind::Malformed("source returned more bytes than requested")
    ));
}

#[test]
fn public_errors_preserve_readable_locations_and_source_causes() {
    let cases = [
        (HeaderErrorKind::InvalidSpan("end"), "invalid span: end"),
        (HeaderErrorKind::Truncated("number"), "truncated number"),
        (
            HeaderErrorKind::Malformed("reference"),
            "malformed reference",
        ),
        (
            HeaderErrorKind::Unsupported {
                feature: "count form",
                value: 6,
            },
            "unsupported count form (6)",
        ),
        (
            HeaderErrorKind::LimitExceeded {
                resource: "header bytes",
                limit: 2,
                attempted: 3,
            },
            "header bytes limit 2 exceeded by 3",
        ),
        (
            HeaderErrorKind::AllocationFailed,
            "header allocation failed",
        ),
        (HeaderErrorKind::Cancelled, "cancelled"),
    ];
    for (kind, detail) in cases {
        let error = HeaderError {
            offset: 17,
            segment: Some(42),
            kind,
        };
        assert_eq!(
            error.to_string(),
            format!("JBIG2 segment header at source byte 17, segment 42: {detail}")
        );
        assert!(std::error::Error::source(&error).is_none());
    }

    let source_error = HeaderError {
        offset: 5,
        segment: None,
        kind: HeaderErrorKind::Source(Error::Io(io::Error::other("test I/O failure"))),
    };
    assert_eq!(
        source_error.to_string(),
        "JBIG2 segment header at source byte 5: source: I/O error: test I/O failure"
    );
    assert!(std::error::Error::source(&source_error).is_some());
}

/// Parses `bytes` as an embedded directory with this file's source type, so
/// directory-budgeted header parsing shares the header tests' instantiation.
fn parse_directory(
    bytes: &[u8],
    directory_limits: DirectoryLimits,
) -> Result<SegmentDirectory, DirectoryError> {
    let mut source = TestSource::new(bytes);
    run(read_embedded_directory(
        &mut source,
        SegmentSpan {
            offset: 0,
            length: bytes.len() as u64,
        },
        &Limits::default(),
        HeaderLimits::default(),
        directory_limits,
        &NeverCancel,
    ))
}

#[test]
fn directory_budgets_bound_each_header_before_its_references_are_read() {
    // Segment 0 is retained; segments 1 and 2 each refer to it once.
    let bytes = [
        [0, 0, 0, 0, 0, 0x01, 0, 0, 0, 0, 0].as_slice(),
        &[0, 0, 0, 1, 0, 0x23, 0, 0, 0, 0, 0, 0],
        &[0, 0, 0, 2, 0, 0x23, 0, 0, 0, 0, 0, 0],
    ]
    .concat();
    let references = DirectoryLimits {
        max_total_references: 1,
        ..DirectoryLimits::default()
    };
    assert!(matches!(
        parse_directory(&bytes, references).unwrap_err().kind,
        DirectoryErrorKind::Header(HeaderError {
            kind: HeaderErrorKind::LimitExceeded {
                resource: "JBIG2 directory references",
                limit: 1,
                attempted: 2,
            },
            ..
        })
    ));
    // Some metadata cap admits the directory preflight but not a header's
    // retention and reference storage. This does not depend on struct layout.
    let header_failure = (1..=4096).find(|&cap| {
        let metadata = DirectoryLimits {
            max_metadata_bytes: cap,
            ..DirectoryLimits::default()
        };
        matches!(
            parse_directory(&bytes, metadata),
            Err(DirectoryError {
                kind: DirectoryErrorKind::Header(HeaderError {
                    kind: HeaderErrorKind::LimitExceeded {
                        resource: "JBIG2 directory metadata bytes",
                        ..
                    },
                    ..
                }),
                ..
            })
        )
    });
    assert!(header_failure.is_some());
}
