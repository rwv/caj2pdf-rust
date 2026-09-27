// SPDX-License-Identifier: MIT

use caj2pdf_core::{
    Cancellation, Error, Limits, NeverCancel, RangedSource,
    jbig2::{
        HeaderLimits, SegmentHeader, SegmentSpan,
        page_info::{PageInfoBudget, PageInfoErrorKind, read_page_info},
        read_segment_header,
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

#[derive(Clone, Copy)]
enum SourceFailure {
    Io,
    Cancelled,
    Truncated,
}

fn run<F: Future>(future: F) -> F::Output {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = pin!(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("in-memory source unexpectedly yielded"),
    }
}

struct Source {
    bytes: Vec<u8>,
    size: u64,
    max_read: usize,
    reads: Vec<(u64, usize)>,
    overreport: bool,
    fail_at: Option<(u64, SourceFailure)>,
    cancel_after_read: Option<(usize, Rc<Cell<bool>>)>,
}

impl Source {
    fn new(bytes: Vec<u8>) -> Self {
        let size = bytes.len() as u64;
        Self {
            bytes,
            size,
            max_read: usize::MAX,
            reads: Vec::new(),
            overreport: false,
            fail_at: None,
            cancel_after_read: None,
        }
    }
}

impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.size
    }

    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        self.reads.push((offset, destination.len()));
        if let Some((failed_offset, failure)) = self.fail_at {
            if failed_offset == offset {
                return Err(match failure {
                    SourceFailure::Io => Error::Io(io::Error::other("synthetic I/O failure")),
                    SourceFailure::Cancelled => Error::Cancelled,
                    SourceFailure::Truncated => Error::TruncatedInput {
                        offset,
                        expected: destination.len() as u64,
                        available: 0,
                    },
                });
            }
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
        if let Some((read_number, flag)) = &self.cancel_after_read {
            if self.reads.len() == *read_number {
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

struct CancelOnCheck {
    checks: Cell<u32>,
    from: u32,
}

impl Cancellation for CancelOnCheck {
    fn is_cancelled(&self) -> bool {
        let checks = self.checks.get() + 1;
        self.checks.set(checks);
        checks >= self.from
    }
}

fn body(width: u32, height: u32, x_resolution: u32, y_resolution: u32) -> [u8; 19] {
    let mut bytes = [0_u8; 19];
    bytes[0..4].copy_from_slice(&width.to_be_bytes());
    bytes[4..8].copy_from_slice(&height.to_be_bytes());
    bytes[8..12].copy_from_slice(&x_resolution.to_be_bytes());
    bytes[12..16].copy_from_slice(&y_resolution.to_be_bytes());
    bytes[16] = 0x01;
    bytes
}

fn prepared_with_fields(
    bytes: &[u8],
    number: u32,
    kind: u8,
    association: u8,
) -> (Source, SegmentHeader) {
    let mut segment = Vec::new();
    segment.extend(number.to_be_bytes());
    segment.extend([kind, 0x01, association]);
    segment.extend((bytes.len() as u32).to_be_bytes());
    segment.extend(bytes);
    let mut source = Source::new(segment);
    let span = SegmentSpan {
        offset: 0,
        length: source.size,
    };
    let header = run(read_segment_header(
        &mut source,
        span,
        &Limits::default(),
        HeaderLimits::default(),
        &NeverCancel,
    ))
    .unwrap();
    source.reads.clear();
    (source, header)
}

fn prepared(bytes: &[u8]) -> (Source, SegmentHeader) {
    prepared_with_fields(bytes, 0, 48, 1)
}

#[test]
fn parses_invented_odd_width_page_and_keeps_nonzero_resolutions() {
    let (mut source, header) = prepared(&body(13, 7, 3_779, 4_000));
    let info = run(read_page_info(
        &mut source,
        &header,
        &Limits::default(),
        PageInfoBudget::default(),
        &NeverCancel,
    ))
    .unwrap();
    assert_eq!((info.width, info.height), (13, 7));
    assert_eq!(info.data, header.data);
    assert_eq!((info.x_resolution, info.y_resolution), (3_779, 4_000));
    assert_eq!(info.flags_raw, 0x01);
    assert_eq!((info.default_pixel, info.combination_operator), (0, 0));
    assert_eq!(info.striping_raw, 0);
    assert_eq!((info.row_stride, info.packed_bytes), (2, 14));
    assert_eq!((info.source_bytes_fetched, info.source_read_calls), (19, 1));
    assert_eq!(info.max_source_request_bytes, 19);
    assert_eq!(source.reads, [(header.data.offset, 19)]);
}

#[test]
fn one_pixel_and_exact_dimension_limits_are_inclusive() {
    let (mut source, header) = prepared(&body(1, 1, 0, 0));
    let budget = PageInfoBudget {
        max_width: 1,
        max_height: 1,
        max_pixels: 1,
        max_packed_bytes: 1,
        ..PageInfoBudget::default()
    };
    let info = run(read_page_info(
        &mut source,
        &header,
        &Limits::default(),
        budget,
        &NeverCancel,
    ))
    .unwrap();
    assert_eq!((info.row_stride, info.packed_bytes), (1, 1));
}

#[test]
fn positioned_short_reads_are_bounded_and_counted() {
    let (mut source, header) = prepared(&body(9, 2, 0, 0));
    source.max_read = 2;
    let budget = PageInfoBudget {
        max_source_request_bytes: 4,
        ..PageInfoBudget::default()
    };
    let info = run(read_page_info(
        &mut source,
        &header,
        &Limits::default(),
        budget,
        &NeverCancel,
    ))
    .unwrap();
    assert_eq!((info.row_stride, info.packed_bytes), (2, 4));
    assert_eq!(
        (info.source_bytes_fetched, info.source_read_calls),
        (19, 10)
    );
    assert_eq!(info.max_source_request_bytes, 4);
    assert_eq!(source.reads.first(), Some(&(header.data.offset, 4)));
    assert_eq!(source.reads.last(), Some(&(header.data.offset + 18, 1)));
    assert!(source.reads.iter().all(|(_, requested)| *requested <= 4));
}

#[test]
fn rejects_short_and_extra_declared_bodies_before_reading() {
    let (mut short_source, short_header) = prepared(&body(2, 2, 0, 0)[..18]);
    let error = run(read_page_info(
        &mut short_source,
        &short_header,
        &Limits::default(),
        PageInfoBudget::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(matches!(error.kind, PageInfoErrorKind::Truncated(_)));
    assert!(
        error
            .to_string()
            .contains("truncated page information body")
    );
    assert!(short_source.reads.is_empty());

    let mut extra = body(2, 2, 0, 0).to_vec();
    extra.push(0x55);
    let (mut extra_source, extra_header) = prepared(&extra);
    let error = run(read_page_info(
        &mut extra_source,
        &extra_header,
        &Limits::default(),
        PageInfoBudget::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(matches!(error.kind, PageInfoErrorKind::Malformed(_)));
    assert!(
        error
            .to_string()
            .contains("malformed extra page information bytes")
    );
    assert!(extra_source.reads.is_empty());
}

#[test]
fn rejects_zero_unknown_and_over_budget_geometry() {
    for (width, height, expected) in [
        (0, 2, "malformed"),
        (2, 0, "malformed"),
        (2, u32::MAX, "unsupported"),
        (10, 2, "limit"),
        (2, 10, "limit"),
    ] {
        let (mut source, header) = prepared(&body(width, height, 0, 0));
        let budget = PageInfoBudget {
            max_width: 9,
            max_height: 9,
            ..PageInfoBudget::default()
        };
        let error = run(read_page_info(
            &mut source,
            &header,
            &Limits::default(),
            budget,
            &NeverCancel,
        ))
        .unwrap_err();
        assert_eq!(
            match error.kind {
                PageInfoErrorKind::Malformed(_) => "malformed",
                PageInfoErrorKind::Unsupported { .. } => "unsupported",
                PageInfoErrorKind::LimitExceeded { .. } => "limit",
                _ => "other",
            },
            expected,
            "{width}x{height}"
        );
        assert!(error.to_string().contains(expected));
    }
    let (mut source, header) = prepared(&body(9, 9, 0, 0));
    for budget in [
        PageInfoBudget {
            max_pixels: 80,
            ..PageInfoBudget::default()
        },
        PageInfoBudget {
            max_packed_bytes: 17,
            ..PageInfoBudget::default()
        },
    ] {
        let error = run(read_page_info(
            &mut source,
            &header,
            &Limits::default(),
            budget,
            &NeverCancel,
        ))
        .unwrap_err();
        assert!(matches!(
            error.kind,
            PageInfoErrorKind::LimitExceeded { .. }
        ));
    }
}

#[test]
fn rejects_every_unimplemented_flag_and_striping_variant() {
    for (flags, expected) in [
        (0x80, "malformed"),
        (0x00, "unsupported"),
        (0x03, "unsupported"),
        (0x05, "unsupported"),
        (0x09, "unsupported"),
        (0x11, "unsupported"),
        (0x21, "unsupported"),
        (0x41, "unsupported"),
    ] {
        let mut bytes = body(2, 2, 0, 0);
        bytes[16] = flags;
        let (mut source, header) = prepared(&bytes);
        let error = run(read_page_info(
            &mut source,
            &header,
            &Limits::default(),
            PageInfoBudget::default(),
            &NeverCancel,
        ))
        .unwrap_err();
        assert_eq!(
            match error.kind {
                PageInfoErrorKind::Malformed(_) => "malformed",
                PageInfoErrorKind::Unsupported { .. } => "unsupported",
                _ => "other",
            },
            expected,
            "flags {flags:#04x}"
        );
        assert_eq!(error.offset, header.data.offset + 16);
    }
    for striping in [1_u16, 0x8000, 0x8001] {
        let mut bytes = body(2, 2, 0, 0);
        bytes[17..19].copy_from_slice(&striping.to_be_bytes());
        let (mut source, header) = prepared(&bytes);
        let error = run(read_page_info(
            &mut source,
            &header,
            &Limits::default(),
            PageInfoBudget::default(),
            &NeverCancel,
        ))
        .unwrap_err();
        assert!(matches!(error.kind, PageInfoErrorKind::Unsupported { .. }));
        assert_eq!(error.offset, header.data.offset + 17);
    }
}

#[test]
fn rejects_wrong_segment_number_kind_and_page() {
    for (number, kind, page) in [(1, 48, 1), (0, 38, 1), (0, 48, 2)] {
        let (mut source, header) = prepared_with_fields(&body(2, 2, 0, 0), number, kind, page);
        let error = run(read_page_info(
            &mut source,
            &header,
            &Limits::default(),
            PageInfoBudget::default(),
            &NeverCancel,
        ))
        .unwrap_err();
        assert!(matches!(error.kind, PageInfoErrorKind::Unsupported { .. }));
        assert!(source.reads.is_empty());
    }
}

#[test]
fn rejects_malformed_page_association_references_and_span_metadata() {
    let (mut source, mut header) = prepared(&body(2, 2, 0, 0));
    header.page_association = 0;
    let error = run(read_page_info(
        &mut source,
        &header,
        &Limits::default(),
        PageInfoBudget::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(matches!(error.kind, PageInfoErrorKind::Malformed(_)));

    header.page_association = 1;
    header.referred_to.push(0);
    let error = run(read_page_info(
        &mut source,
        &header,
        &Limits::default(),
        PageInfoBudget::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(matches!(error.kind, PageInfoErrorKind::Malformed(_)));

    header.referred_to.clear();
    header.header_length = header.data.offset + 1;
    let error = run(read_page_info(
        &mut source,
        &header,
        &Limits::default(),
        PageInfoBudget::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(matches!(error.kind, PageInfoErrorKind::InvalidSpan(_)));
    assert!(error.to_string().contains("invalid span"));

    header.header_length = 11;
    header.data.offset = u64::MAX - 10;
    source.size = u64::MAX;
    let error = run(read_page_info(
        &mut source,
        &header,
        &Limits::default(),
        PageInfoBudget::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(matches!(error.kind, PageInfoErrorKind::InvalidSpan(_)));
    assert!(source.reads.is_empty());
}

#[test]
fn rejects_physical_truncation_zero_progress_overreport_and_source_failure() {
    let (mut source, header) = prepared(&body(2, 2, 0, 0));
    source.size -= 1;
    let error = run(read_page_info(
        &mut source,
        &header,
        &Limits::default(),
        PageInfoBudget::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(matches!(error.kind, PageInfoErrorKind::Truncated(_)));
    assert!(source.reads.is_empty());

    let (mut source, header) = prepared(&body(2, 2, 0, 0));
    source.max_read = 0;
    let error = run(read_page_info(
        &mut source,
        &header,
        &Limits::default(),
        PageInfoBudget::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(matches!(error.kind, PageInfoErrorKind::Truncated(_)));
    assert_eq!(
        (error.source_bytes_fetched, error.source_read_calls),
        (0, 1)
    );

    let (mut source, header) = prepared(&body(2, 2, 0, 0));
    source.overreport = true;
    let error = run(read_page_info(
        &mut source,
        &header,
        &Limits::default(),
        PageInfoBudget::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(matches!(error.kind, PageInfoErrorKind::Malformed(_)));
    assert_eq!(
        (error.source_bytes_fetched, error.source_read_calls),
        (0, 1)
    );

    let (mut source, header) = prepared(&body(2, 2, 0, 0));
    source.fail_at = Some((header.data.offset, SourceFailure::Io));
    let error = run(read_page_info(
        &mut source,
        &header,
        &Limits::default(),
        PageInfoBudget::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(matches!(error.kind, PageInfoErrorKind::Source(_)));
    assert!(error.to_string().contains("source: I/O error"));
    assert!(std::error::Error::source(&error).is_some());

    for (failure, expected) in [
        (SourceFailure::Cancelled, "cancelled"),
        (SourceFailure::Truncated, "truncated page information body"),
    ] {
        let (mut source, header) = prepared(&body(2, 2, 0, 0));
        source.fail_at = Some((header.data.offset, failure));
        let error = run(read_page_info(
            &mut source,
            &header,
            &Limits::default(),
            PageInfoBudget::default(),
            &NeverCancel,
        ))
        .unwrap_err();
        assert!(error.to_string().contains(expected));
        assert_eq!(
            (error.source_bytes_fetched, error.source_read_calls),
            (0, 1)
        );
    }
}

#[test]
fn cancellation_before_and_after_a_body_read_is_located() {
    let (mut source, header) = prepared(&body(2, 2, 0, 0));
    let state = Rc::new(Cell::new(true));
    let error = run(read_page_info(
        &mut source,
        &header,
        &Limits::default(),
        PageInfoBudget::default(),
        &Flag(state.clone()),
    ))
    .unwrap_err();
    assert!(matches!(error.kind, PageInfoErrorKind::Cancelled));
    assert!(error.to_string().contains("cancelled"));
    assert!(std::error::Error::source(&error).is_none());
    assert!(source.reads.is_empty());

    state.set(false);
    source.cancel_after_read = Some((1, state.clone()));
    source.max_read = 1;
    let error = run(read_page_info(
        &mut source,
        &header,
        &Limits::default(),
        PageInfoBudget::default(),
        &Flag(state.clone()),
    ))
    .unwrap_err();
    assert!(matches!(error.kind, PageInfoErrorKind::Cancelled));
    assert_eq!(
        (error.source_bytes_fetched, error.source_read_calls),
        (1, 1)
    );

    let (mut source, header) = prepared(&body(2, 2, 0, 0));
    let cancellation = CancelOnCheck {
        checks: Cell::new(0),
        from: 2,
    };
    let error = run(read_page_info(
        &mut source,
        &header,
        &Limits::default(),
        PageInfoBudget::default(),
        &cancellation,
    ))
    .unwrap_err();
    assert!(matches!(error.kind, PageInfoErrorKind::Cancelled));
    assert_eq!(cancellation.checks.get(), 2);
    assert!(source.reads.is_empty());
}

#[test]
fn explicit_source_and_output_budgets_stop_work() {
    let (mut source, header) = prepared(&body(9, 2, 0, 0));
    let budgets = [
        PageInfoBudget {
            max_source_io_bytes: 18,
            ..PageInfoBudget::default()
        },
        PageInfoBudget {
            max_source_request_bytes: 1,
            max_source_io_calls: 18,
            ..PageInfoBudget::default()
        },
    ];
    for budget in budgets {
        let error = run(read_page_info(
            &mut source,
            &header,
            &Limits::default(),
            budget,
            &NeverCancel,
        ))
        .unwrap_err();
        assert!(matches!(
            error.kind,
            PageInfoErrorKind::LimitExceeded { .. }
        ));
    }
    let error = run(read_page_info(
        &mut source,
        &header,
        &Limits::default(),
        PageInfoBudget {
            max_source_request_bytes: 0,
            ..PageInfoBudget::default()
        },
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(matches!(error.kind, PageInfoErrorKind::Malformed(_)));

    let limits = Limits {
        max_input_bytes: 18,
        ..Limits::default()
    };
    let error = run(read_page_info(
        &mut source,
        &header,
        &limits,
        PageInfoBudget::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(matches!(
        error.kind,
        PageInfoErrorKind::LimitExceeded { .. }
    ));
    assert!(error.to_string().contains("limit 18 exceeded by 19"));

    let limits = Limits {
        max_output_bytes: 3,
        ..Limits::default()
    };
    let error = run(read_page_info(
        &mut source,
        &header,
        &limits,
        PageInfoBudget::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(matches!(
        error.kind,
        PageInfoErrorKind::LimitExceeded { .. }
    ));
}
