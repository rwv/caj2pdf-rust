// SPDX-License-Identifier: MIT

use caj2pdf_core::{
    Cancellation, Error, ErrorKind, Limits, NeverCancel, RangedSource,
    jbig2::{SegmentHeader, SegmentSpan, page_info::read_page_info, read_segment_header},
};
use std::{cell::Cell, io, rc::Rc};

#[derive(Clone, Copy)]
enum SourceFailure {
    Io,
    Cancelled,
    Truncated,
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

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> caj2pdf_core::Result<usize> {
        self.reads.push((offset, destination.len()));
        if let Some((failed_offset, failure)) = self.fail_at
            && failed_offset == offset
        {
            return Err(match failure {
                SourceFailure::Io => {
                    Error::from(ErrorKind::Io(io::Error::other("synthetic I/O failure")))
                }
                SourceFailure::Cancelled => Error::cancelled(),
                SourceFailure::Truncated => Error::truncated(offset, destination.len() as u64, 0),
            });
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
        if let Some((read_number, flag)) = &self.cancel_after_read
            && self.reads.len() == *read_number
        {
            flag.set(true);
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
    let header = read_segment_header(&mut source, span, &Limits::default(), &NeverCancel).unwrap();
    source.reads.clear();
    (source, header)
}

fn prepared(bytes: &[u8]) -> (Source, SegmentHeader) {
    prepared_with_fields(bytes, 0, 48, 1)
}

#[test]
fn parses_invented_odd_width_page_and_keeps_nonzero_resolutions() {
    let (mut source, header) = prepared(&body(13, 7, 3_779, 4_000));
    let info = read_page_info(&mut source, &header, &Limits::default(), &NeverCancel).unwrap();
    assert_eq!((info.width, info.height), (13, 7));
    assert_eq!(info.data, header.data);
    assert_eq!((info.x_resolution, info.y_resolution), (3_779, 4_000));
    assert_eq!(info.flags_raw, 0x01);
    assert_eq!(info.striping_raw, 0);
    assert_eq!((info.row_stride, info.packed_bytes), (2, 14));
    assert_eq!(source.reads, [(header.data.offset, 19)]);
}

#[test]
fn one_pixel_and_exact_dimension_limits_are_inclusive() {
    let (mut source, header) = prepared(&body(1, 1, 0, 0));
    let limits = Limits {
        io_chunk_bytes: 1,
        max_allocation_bytes: 1,
        max_image_pixels: 1,
        ..Limits::default()
    };
    let info = read_page_info(&mut source, &header, &limits, &NeverCancel).unwrap();
    assert_eq!((info.row_stride, info.packed_bytes), (1, 1));
}

#[test]
fn positioned_short_reads_are_bounded_and_counted() {
    let (mut source, header) = prepared(&body(9, 2, 0, 0));
    source.max_read = 2;
    let limits = Limits {
        io_chunk_bytes: 4,
        ..Limits::default()
    };
    let info = read_page_info(&mut source, &header, &limits, &NeverCancel).unwrap();
    assert_eq!((info.row_stride, info.packed_bytes), (2, 4));
    assert_eq!(source.reads.len(), 10);
    assert_eq!(source.reads.first(), Some(&(header.data.offset, 4)));
    assert_eq!(source.reads.last(), Some(&(header.data.offset + 18, 1)));
    assert!(source.reads.iter().all(|(_, requested)| *requested <= 4));
}

#[test]
fn rejects_short_and_extra_declared_bodies_before_reading() {
    let (mut short_source, short_header) = prepared(&body(2, 2, 0, 0)[..18]);
    let error = read_page_info(
        &mut short_source,
        &short_header,
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Truncated { .. },
            ..
        }
    ));
    assert!(error.to_string().contains(": page information body"));
    assert!(short_source.reads.is_empty());

    let mut extra = body(2, 2, 0, 0).to_vec();
    extra.push(0x55);
    let (mut extra_source, extra_header) = prepared(&extra);
    let error = read_page_info(
        &mut extra_source,
        &extra_header,
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Malformed,
            ..
        }
    ));
    assert!(error.to_string().contains(": extra page information bytes"));
    assert!(extra_source.reads.is_empty());
}

#[test]
fn rejects_geometry_over_the_pixel_and_allocation_limits() {
    let (mut source, header) = prepared(&body(9, 9, 0, 0));
    for (limits, resource) in [
        (
            Limits {
                max_image_pixels: 80,
                ..Limits::default()
            },
            "page pixels",
        ),
        (
            Limits {
                io_chunk_bytes: 1,
                max_allocation_bytes: 17,
                ..Limits::default()
            },
            "packed page bytes",
        ),
    ] {
        let error = read_page_info(&mut source, &header, &limits, &NeverCancel).unwrap_err();
        assert!(
            matches!(error, Error { kind: ErrorKind::LimitExceeded { resource: actual, .. }, .. } if actual == resource),
            "{error}"
        );
    }
}

#[test]
fn rejects_a_body_span_beyond_the_address_space() {
    let (mut source, mut header) = prepared(&body(2, 2, 0, 0));
    header.data.offset = u64::MAX - 10;
    source.size = u64::MAX;
    let error = read_page_info(&mut source, &header, &Limits::default(), &NeverCancel).unwrap_err();
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Malformed,
            ..
        }
    ));
    assert!(error.to_string().contains("page information end overflows"));
    assert!(source.reads.is_empty());
}

#[test]
fn rejects_physical_truncation_zero_progress_overreport_and_source_failure() {
    let (mut source, header) = prepared(&body(2, 2, 0, 0));
    source.size -= 1;
    let error = read_page_info(&mut source, &header, &Limits::default(), &NeverCancel).unwrap_err();
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Truncated { .. },
            ..
        }
    ));
    assert!(source.reads.is_empty());

    let (mut source, header) = prepared(&body(2, 2, 0, 0));
    source.max_read = 0;
    let error = read_page_info(&mut source, &header, &Limits::default(), &NeverCancel).unwrap_err();
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Truncated { .. },
            ..
        }
    ));

    let (mut source, header) = prepared(&body(2, 2, 0, 0));
    source.overreport = true;
    let error = read_page_info(&mut source, &header, &Limits::default(), &NeverCancel).unwrap_err();
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Malformed,
            ..
        }
    ));

    let (mut source, header) = prepared(&body(2, 2, 0, 0));
    source.fail_at = Some((header.data.offset, SourceFailure::Io));
    let error = read_page_info(&mut source, &header, &Limits::default(), &NeverCancel).unwrap_err();
    assert!(matches!(error.kind, ErrorKind::Io(_)));
    assert!(
        error.to_string().contains("synthetic I/O failure"),
        "{error}"
    );
    assert!(std::error::Error::source(&error).is_some());

    for (failure, expected) in [
        (SourceFailure::Cancelled, "cancelled"),
        (SourceFailure::Truncated, ": page information body"),
    ] {
        let (mut source, header) = prepared(&body(2, 2, 0, 0));
        source.fail_at = Some((header.data.offset, failure));
        let error =
            read_page_info(&mut source, &header, &Limits::default(), &NeverCancel).unwrap_err();
        assert!(error.to_string().contains(expected));
    }
}

#[test]
fn cancellation_before_and_after_a_body_read_is_located() {
    let (mut source, header) = prepared(&body(2, 2, 0, 0));
    let state = Rc::new(Cell::new(true));
    let error = read_page_info(
        &mut source,
        &header,
        &Limits::default(),
        &Flag(state.clone()),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Cancelled,
            ..
        }
    ));
    assert!(error.to_string().contains("cancelled"));
    assert!(std::error::Error::source(&error).is_none());
    assert!(source.reads.is_empty());

    state.set(false);
    source.cancel_after_read = Some((1, state.clone()));
    source.max_read = 1;
    let error = read_page_info(
        &mut source,
        &header,
        &Limits::default(),
        &Flag(state.clone()),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Cancelled,
            ..
        }
    ));

    let (mut source, header) = prepared(&body(2, 2, 0, 0));
    let cancellation = CancelOnCheck {
        checks: Cell::new(0),
        from: 2,
    };
    let error =
        read_page_info(&mut source, &header, &Limits::default(), &cancellation).unwrap_err();
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Cancelled,
            ..
        }
    ));
    assert_eq!(cancellation.checks.get(), 2);
    assert!(source.reads.is_empty());
}

#[test]
fn input_and_output_limits_stop_work() {
    let (mut source, header) = prepared(&body(9, 2, 0, 0));
    let limits = Limits {
        max_input_bytes: 18,
        ..Limits::default()
    };
    let error = read_page_info(&mut source, &header, &limits, &NeverCancel).unwrap_err();
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::LimitExceeded { .. },
            ..
        }
    ));
    assert!(error.to_string().contains("maximum 18, attempted 19"));

    let limits = Limits {
        max_output_bytes: 3,
        ..Limits::default()
    };
    let error = read_page_info(&mut source, &header, &limits, &NeverCancel).unwrap_err();
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::LimitExceeded { .. },
            ..
        }
    ));
}
