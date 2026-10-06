// SPDX-License-Identifier: MIT

//! Synthetic MQ bytes for the standard states test bounded image-model
//! behavior only.
//! They do not establish T.88 Table E.1 or CAJ/HN pixel compatibility.

mod common;

use caj2pdf_core::{
    Error, ErrorKind, Limits, Payload, RangedSource,
    jbig2::{
        SegmentHeader, SegmentSpan,
        generic::{GenericRegionDecoder, read_generic_region_header},
        mq::{ContextBank, MqTable},
        read_segment_header,
    },
};
use common::CancelAfter;
use std::io::Write;
use std::{cell::Cell, io, rc::Rc};

struct Source {
    bytes: Vec<u8>,
}
impl Source {
    fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }
}
impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> caj2pdf_core::Result<usize> {
        let mut bytes = &self.bytes[..];
        bytes.read_at(offset, destination)
    }
}
struct Sink {
    bytes: Vec<u8>,
    max_write: usize,
    zero: bool,
    cancel: Option<Rc<Cell<bool>>>,
    flushed: bool,
    flush_fail: bool,
    cancel_on_flush: Option<Rc<Cell<bool>>>,
}
impl Default for Sink {
    fn default() -> Self {
        Self {
            bytes: Vec::new(),
            max_write: usize::MAX,
            zero: false,
            cancel: None,
            flushed: false,
            flush_fail: false,
            cancel_on_flush: None,
        }
    }
}
impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.zero {
            return Ok(0);
        }
        let count = bytes.len().min(self.max_write);
        self.bytes.extend_from_slice(&bytes[..count]);
        if let Some(flag) = &self.cancel {
            flag.set(true);
        }
        Ok(count)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        if self.flush_fail {
            return Err(io::Error::other("test flush failure"));
        }
        if let Some(flag) = &self.cancel_on_flush {
            flag.set(true);
        }
        self.flushed = true;
        Ok(())
    }
}

fn record(
    width: u32,
    height: u32,
    region_flags: u8,
    generic_flags: u8,
    at: (i8, i8),
    payload: &[u8],
) -> Source {
    let mut data = Vec::new();
    data.extend_from_slice(&width.to_be_bytes());
    data.extend_from_slice(&height.to_be_bytes());
    data.extend_from_slice(&0u32.to_be_bytes());
    data.extend_from_slice(&0u32.to_be_bytes());
    data.extend_from_slice(&[region_flags, generic_flags, at.0 as u8, at.1 as u8]);
    data.extend_from_slice(payload);
    let mut bytes = vec![0, 0, 0, 1, 38, 0, 1];
    bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&data);
    Source::new(bytes)
}
fn header(source: &mut Source) -> SegmentHeader {
    read_segment_header(
        source,
        SegmentSpan {
            offset: 0,
            length: source.bytes.len() as u64,
        },
        &Limits::default(),
        &CancelAfter::Never,
    )
    .unwrap()
}
fn table() -> MqTable {
    MqTable::standard()
}
fn contexts(limits: &Limits) -> ContextBank {
    ContextBank::new(1024, limits).unwrap()
}
const SHORT_STREAM: &[u8] = &[0xfc, 0xaf, 0xff, 0xac];

#[test]
fn page_preflight_detects_a_changed_generic_header_before_any_output() {
    let limits = Limits::default();
    let mut source = record(3, 2, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    let inspected = read_generic_region_header(&mut source, &hdr, &limits, &CancelAfter::Never)
        .expect("read-only header preflight");
    assert_eq!(inspected.info.width, 3);

    let table = table();
    let mut bank = contexts(&limits);
    let mut sink = Sink::default();
    let same = GenericRegionDecoder::new(
        Payload::from(&source.bytes[..]),
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &CancelAfter::Never,
    )
    .expect("same checked header");
    assert_eq!(same.checked_header(), inspected);
    drop(same);
    assert!(sink.bytes.is_empty());

    source.bytes[hdr.data.offset as usize + 3] = 4;
    let changed = GenericRegionDecoder::new(
        Payload::from(&source.bytes[..]),
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &CancelAfter::Never,
    )
    .expect("changed but still valid header");
    assert_ne!(changed.checked_header(), inspected);
    drop(changed);
    assert!(sink.bytes.is_empty());
}

#[test]
fn streams_packed_rows_and_distinguishes_semantic_from_physical_input() {
    let limits = Limits {
        io_chunk_bytes: 2,
        ..Limits::default()
    };
    let mut source = record(3, 2, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    let table = table();
    let mut bank = contexts(&limits);
    // An earlier region adapts the bank; this region must start from reset
    // contexts.
    let mut earlier = record(3, 2, 0, 4, (2, -1), SHORT_STREAM);
    let earlier_header = header(&mut earlier);
    let mut discard = Sink::default();
    let mut decoder = GenericRegionDecoder::new(
        Payload::from(&earlier.bytes[..]),
        &earlier_header,
        &table,
        &mut bank,
        &mut discard,
        &limits,
        &CancelAfter::Never,
    )
    .unwrap();
    while decoder.decode_next_row().unwrap() {}
    decoder.finish().unwrap();
    assert!((0..bank.len()).any(|index| bank.get(index).unwrap().state_index != 0));
    let mut sink = Sink {
        max_write: 1,
        ..Sink::default()
    };
    let mut decoder = GenericRegionDecoder::new(
        Payload::from(&source.bytes[..]),
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &CancelAfter::Never,
    )
    .unwrap();
    assert_eq!(decoder.progress().info.row_stride, 1);
    assert!(decoder.decode_next_row().unwrap());
    assert_eq!(
        (
            decoder.progress().rows_written,
            decoder.progress().pixels_decoded
        ),
        (1, 3)
    );
    assert!(decoder.decode_next_row().unwrap());
    assert!(!decoder.decode_next_row().unwrap());
    let report = decoder.finish().unwrap();
    assert_eq!(sink.bytes, [0xe0, 0xe0]); // three one-bits, then five zero padding bits per row
    assert!(sink.flushed);
    assert_eq!(report.progress.output_bytes_written, 2);
    assert_eq!(report.progress.pixels_decoded, 6);
    assert_eq!(report.progress.rows_written, 2);
    assert!(report.progress.mq.input_offset < report.mq_span.offset + report.mq_span.length);
}

#[test]
fn third_row_uses_both_prior_rows_after_rotation() {
    let limits = Limits::default();
    let mut source = record(3, 3, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    let table = table();
    let mut bank = contexts(&limits);
    let mut sink = Sink::default();
    let mut decoder = GenericRegionDecoder::new(
        Payload::from(&source.bytes[..]),
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &CancelAfter::Never,
    )
    .unwrap();
    for _ in 0..3 {
        assert!(decoder.decode_next_row().unwrap());
    }
    decoder.finish().unwrap();
    assert_eq!(sink.bytes, [0xe0, 0xe0, 0xe0]);
    // At (x=0,y=2): prior-two bits are 011 and prior-one bits are
    // 00111, with current-left 00: 0b011_00111_00 = 412.
    assert_eq!(bank.get(412).unwrap().state_index, 1);
}

#[test]
fn rejects_header_modes_at_placement_and_truncation_before_mq() {
    let limits = Limits::default();
    let table = table();
    for (w, h, region, flags, at, payload, expected) in [
        (0, 2, 0, 4, (2, -1), SHORT_STREAM, "dimension"),
        (3, 2, 0x80, 4, (2, -1), SHORT_STREAM, "reserved"),
        (3, 2, 7, 4, (2, -1), SHORT_STREAM, "combination"),
        (3, 2, 0, 0x84, (2, -1), SHORT_STREAM, "reserved"),
        (3, 2, 0, 5, (2, -1), SHORT_STREAM, "MMR"),
        (3, 2, 0, 0, (2, -1), SHORT_STREAM, "template"),
        (3, 2, 0, 12, (2, -1), SHORT_STREAM, "prediction"),
        (3, 2, 0, 4, (0, 0), SHORT_STREAM, "adaptive"),
        (
            3,
            2,
            0,
            4,
            (1, -1),
            SHORT_STREAM,
            "unsupported JBIG2 at byte 29, segment 1: adaptive",
        ),
        (3, 2, 0, 4, (2, -1), &[0][..], "terminal"),
    ] {
        let mut source = record(w, h, region, flags, at, payload);
        let hdr = header(&mut source);
        let mut bank = contexts(&limits);
        let mut sink = Sink::default();
        let err = match GenericRegionDecoder::new(
            Payload::from(&source.bytes[..]),
            &hdr,
            &table,
            &mut bank,
            &mut sink,
            &limits,
            &CancelAfter::Never,
        ) {
            Ok(_) => panic!("accepted invalid {expected}"),
            Err(e) => e,
        };
        assert!(err.to_string().contains(expected), "{err}");
        assert!(sink.bytes.is_empty());
    }
    let mut source = record(3, 2, 0, 4, (2, -1), SHORT_STREAM);
    let mut hdr = header(&mut source);
    hdr.segment_type = 4;
    let mut bank = contexts(&limits);
    let mut sink = Sink::default();
    let err = match GenericRegionDecoder::new(
        Payload::from(&source.bytes[..]),
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &CancelAfter::Never,
    ) {
        Ok(_) => panic!("accepted type 4"),
        Err(e) => e,
    };
    assert!(matches!(
        err,
        Error {
            kind: ErrorKind::UnsupportedFormat,
            reason: "segment type",
            ..
        }
    ));
    let mut source = record(3, 2, 0, 4, (2, -1), SHORT_STREAM);
    source.bytes[19..23].copy_from_slice(&u32::MAX.to_be_bytes());
    let hdr = header(&mut source);
    let mut bank = contexts(&limits);
    let mut sink = Sink::default();
    let err = match GenericRegionDecoder::new(
        Payload::from(&source.bytes[..]),
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &CancelAfter::Never,
    ) {
        Ok(_) => panic!("accepted overflowing region x"),
        Err(e) => e,
    };
    assert!(matches!(
        err,
        Error {
            kind: ErrorKind::Malformed,
            reason: "region x plus width overflows",
            ..
        }
    ));

    let mut source = record(3, 2, 0, 4, (2, -1), SHORT_STREAM);
    source.bytes.truncate(11 + 19);
    source.bytes[7..11].copy_from_slice(&19u32.to_be_bytes());
    let hdr = header(&mut source);
    let mut bank = contexts(&limits);
    let mut sink = Sink::default();
    let err = match GenericRegionDecoder::new(
        Payload::from(&source.bytes[..]),
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &CancelAfter::Never,
    ) {
        Ok(_) => panic!("accepted missing adaptive coordinate"),
        Err(e) => e,
    };
    assert!(matches!(
        err,
        Error {
            kind: ErrorKind::Truncated { .. },
            reason: "template-2 adaptive pixel",
            ..
        }
    ));
}

#[test]
fn preflights_area_output_allocation_and_input_limits() {
    let table = table();
    let mut source = record(9, 3, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    let cases = [
        (
            Limits {
                max_image_pixels: 26,
                ..Limits::default()
            },
            "maximum 26, attempted 27",
        ),
        (
            Limits {
                max_output_bytes: 5,
                ..Limits::default()
            },
            "output",
        ),
        (
            Limits {
                io_chunk_bytes: 1,
                max_allocation_bytes: 2_000,
                ..Limits::default()
            },
            "region working allocation bytes",
        ),
        (
            Limits {
                // One byte short of the 20 header bytes and the stream.
                max_input_bytes: 19 + SHORT_STREAM.len() as u64,
                ..Limits::default()
            },
            "input",
        ),
    ];
    for (limits, expected) in cases {
        let mut bank = contexts(&Limits::default());
        let mut sink = Sink::default();
        let err = match GenericRegionDecoder::new(
            Payload::from(&source.bytes[..]),
            &hdr,
            &table,
            &mut bank,
            &mut sink,
            &limits,
            &CancelAfter::Never,
        ) {
            Ok(_) => panic!("accepted low {expected} limit"),
            Err(e) => e,
        };
        assert!(err.to_string().contains(expected), "{err}");
        assert!(sink.bytes.is_empty());
    }
}

#[test]
fn additional_constructor_bounds_and_located_source_errors() {
    let limits = Limits::default();
    let table = table();
    let mut source = record(3, 2, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    let mut bank = ContextBank::new(1023, &limits).unwrap();
    let mut sink = Sink::default();
    let err = match GenericRegionDecoder::new(
        Payload::from(&source.bytes[..]),
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &CancelAfter::Never,
    ) {
        Ok(_) => panic!("accepted invalid MQ contexts"),
        Err(e) => e,
    };
    assert!(
        err.to_string().contains("1024 generic MQ contexts"),
        "{err}"
    );

    // Both width and height are u32, so their u64 product fits and is
    // refused by the pixel limit before any allocation.
    let mut source = record(u32::MAX, u32::MAX, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    let mut bank = contexts(&limits);
    let mut sink = Sink::default();
    let err = match GenericRegionDecoder::new(
        Payload::from(&source.bytes[..]),
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &CancelAfter::Never,
    ) {
        Ok(_) => panic!("accepted an oversized region"),
        Err(e) => e,
    };
    assert!(matches!(
        err,
        Error {
            kind: ErrorKind::LimitExceeded {
                resource: "region pixels",
                ..
            },
            ..
        }
    ));

    let mut source = record(3, 2, 0, 4, (2, -1), SHORT_STREAM);
    source.bytes[23..27].copy_from_slice(&u32::MAX.to_be_bytes());
    let hdr = header(&mut source);
    let mut bank = contexts(&limits);
    let mut sink = Sink::default();
    let err = match GenericRegionDecoder::new(
        Payload::from(&source.bytes[..]),
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &CancelAfter::Never,
    ) {
        Ok(_) => panic!("accepted overflowing y"),
        Err(e) => e,
    };
    assert!(matches!(
        err,
        Error {
            kind: ErrorKind::Malformed,
            reason: "region y plus height overflows",
            ..
        }
    ));

    let mut source = record(3, 2, 0, 4, (2, -1), SHORT_STREAM);
    let mut hdr = header(&mut source);
    hdr.data.offset = u64::MAX - 1;
    let mut bank = contexts(&limits);
    let mut sink = Sink::default();
    let err = match GenericRegionDecoder::new(
        Payload::from(&source.bytes[..]),
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &CancelAfter::Never,
    ) {
        Ok(_) => panic!("accepted overflowing span"),
        Err(e) => e,
    };
    assert!(matches!(
        err,
        Error {
            kind: ErrorKind::Malformed,
            ..
        }
    ));

    let mut source = record(3, 2, 0, 4, (2, -1), SHORT_STREAM);
    let mut hdr = header(&mut source);
    let last = source.bytes.pop().unwrap();
    let mut bank = contexts(&limits);
    let mut sink = Sink::default();
    let err = match GenericRegionDecoder::new(
        Payload::from(&source.bytes[..]),
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &CancelAfter::Never,
    ) {
        Ok(_) => panic!("accepted outside-source span"),
        Err(e) => e,
    };
    assert!(matches!(
        err,
        Error {
            kind: ErrorKind::Malformed,
            ..
        }
    ));
    source.bytes.push(last);
    hdr.data.length = 17;
    let err = match GenericRegionDecoder::new(
        Payload::from(&source.bytes[..]),
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &CancelAfter::Never,
    ) {
        Ok(_) => panic!("accepted short generic header"),
        Err(e) => e,
    };
    assert!(matches!(
        err,
        Error {
            kind: ErrorKind::Truncated { .. },
            reason: "generic flags",
            ..
        }
    ));

    let mut source = record(3, 2, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    let flag = Rc::new(Cell::new(true));
    let cancellation = CancelAfter::While(flag);
    let mut bank = contexts(&limits);
    let mut sink = Sink::default();
    let err = match GenericRegionDecoder::new(
        Payload::from(&source.bytes[..]),
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &cancellation,
    ) {
        Ok(_) => panic!("accepted pre-cancelled region"),
        Err(e) => e,
    };
    assert!(matches!(
        err,
        Error {
            kind: ErrorKind::Cancelled,
            ..
        }
    ));
}

#[test]
fn truncated_payload_and_sink_failure_are_typed() {
    let limits = Limits::default();
    let table = table();
    let mut source = record(3, 1, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    source.bytes.truncate(11);
    let mut bank = contexts(&limits);
    let mut sink = Sink::default();
    let err = match GenericRegionDecoder::new(
        Payload::from(&source.bytes[..]),
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &CancelAfter::Never,
    ) {
        Ok(_) => panic!("accepted a truncated payload"),
        Err(e) => e,
    };
    assert!(
        matches!(
            err,
            Error {
                kind: ErrorKind::Truncated { .. },
                ..
            } | Error {
                kind: ErrorKind::Malformed,
                ..
            }
        ),
        "{err}"
    );
    let mut source = record(3, 1, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    let mut bank = contexts(&limits);
    let mut sink = Sink {
        zero: true,
        ..Sink::default()
    };
    let mut decoder = GenericRegionDecoder::new(
        Payload::from(&source.bytes[..]),
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &CancelAfter::Never,
    )
    .unwrap();
    let err = decoder.decode_next_row().unwrap_err();
    assert!(matches!(err.kind, ErrorKind::Io(_)), "{err}");
    assert!(err.to_string().contains("segment 1"), "{err}");
    assert!(std::error::Error::source(&err).is_some());
}

#[test]
fn retries_partial_row_writes_and_reports_flush_failure() {
    let limits = Limits {
        io_chunk_bytes: 2,
        ..Limits::default()
    };
    let never = CancelAfter::Never;
    let table = table();
    let stream = [0xf9, 0xff, 0xac];
    let mut source = record(9, 1, 0, 4, (2, -1), &stream);
    let hdr = header(&mut source);
    let mut bank = contexts(&limits);
    let mut sink = Sink {
        max_write: 1,
        flush_fail: true,
        ..Sink::default()
    };
    let mut decoder = GenericRegionDecoder::new(
        Payload::from(&source.bytes[..]),
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &never,
    )
    .unwrap();
    assert!(decoder.decode_next_row().unwrap());
    let err = decoder.finish().unwrap_err();
    assert!(matches!(err.kind, ErrorKind::Io(_)), "{err}");
    assert!(err.to_string().contains("test flush failure"), "{err}");
    assert_eq!(sink.bytes, [0xff, 0x80]);
    assert!(!sink.flushed);
}

#[test]
fn cancellation_after_a_row_write_is_reported() {
    let limits = Limits::default();
    let table = table();
    let flag = Rc::new(Cell::new(false));
    let mut source = record(3, 1, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    let mut bank = contexts(&limits);
    let mut sink = Sink {
        cancel: Some(flag.clone()),
        ..Sink::default()
    };
    let cancellation = CancelAfter::While(flag);
    let mut decoder = GenericRegionDecoder::new(
        Payload::from(&source.bytes[..]),
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &cancellation,
    )
    .unwrap();
    let err = decoder.decode_next_row().unwrap_err();
    assert!(matches!(
        err,
        Error {
            kind: ErrorKind::Cancelled,
            ..
        }
    ));
    drop(decoder);
    assert_eq!(sink.bytes, [0xe0]);
}

#[test]
fn cancellation_before_next_row_and_during_flush_never_reports_success() {
    let limits = Limits::default();
    let table = table();
    let flag = Rc::new(Cell::new(false));
    let cancellation = CancelAfter::While(flag.clone());
    let mut source = record(3, 1, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    let mut bank = contexts(&limits);
    let mut sink = Sink::default();
    let mut decoder = GenericRegionDecoder::new(
        Payload::from(&source.bytes[..]),
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &cancellation,
    )
    .unwrap();
    flag.set(true);
    let err = decoder.decode_next_row().unwrap_err();
    assert!(matches!(
        err,
        Error {
            kind: ErrorKind::Cancelled,
            ..
        }
    ));
    drop(decoder);
    assert!(sink.bytes.is_empty());

    flag.set(false);
    let mut source = record(3, 1, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    let mut bank = contexts(&limits);
    let mut sink = Sink {
        cancel_on_flush: Some(flag),
        ..Sink::default()
    };
    let mut decoder = GenericRegionDecoder::new(
        Payload::from(&source.bytes[..]),
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &cancellation,
    )
    .unwrap();
    decoder.decode_next_row().unwrap();
    let err = decoder.finish().unwrap_err();
    assert!(matches!(
        err,
        Error {
            kind: ErrorKind::Cancelled,
            ..
        }
    ));
    assert_eq!(sink.bytes, [0xe0]);
    assert!(sink.flushed);
}

#[test]
fn rejects_terminal_errors_and_incomplete_finish() {
    let limits = Limits::default();
    let never = CancelAfter::Never;
    let table = table();
    let mut source = record(3, 1, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    let mut bank = contexts(&limits);
    let mut sink = Sink::default();
    let decoder = GenericRegionDecoder::new(
        Payload::from(&source.bytes[..]),
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &never,
    )
    .unwrap();
    let err = decoder.finish().unwrap_err();
    assert_eq!(err.reason, "not all generic rows were decoded");
    assert!(err.to_string().contains("segment 1"), "{err}");
    assert!(std::error::Error::source(&err).is_none());
    for bad_tail in [[0xff, 0xab], [0x00, 0xac]] {
        let mut bytes = SHORT_STREAM.to_vec();
        let tail = bytes.len() - 2;
        bytes[tail..].copy_from_slice(&bad_tail);
        let mut source = record(3, 1, 0, 4, (2, -1), &bytes);
        let hdr = header(&mut source);
        let mut bank = contexts(&limits);
        let mut sink = Sink::default();
        let mut decoder = GenericRegionDecoder::new(
            Payload::from(&source.bytes[..]),
            &hdr,
            &table,
            &mut bank,
            &mut sink,
            &limits,
            &never,
        )
        .unwrap();
        decoder.decode_next_row().unwrap();
        let err = decoder.finish().unwrap_err();
        assert!(err.reason.contains("MQ"), "{err}");
        assert!(std::error::Error::source(&err).is_none());
        assert!(!sink.flushed);
    }
}

#[test]
fn unexpected_internal_marker_keeps_the_mq_source_location() {
    let limits = Limits::default();
    let table = table();
    let mut source = record(128, 1, 0, 4, (2, -1), &[0, 0, 0xff, 0x90, 0xff, 0xac]);
    let hdr = header(&mut source);
    let mut bank = contexts(&limits);
    let mut sink = Sink::default();
    let mut decoder = GenericRegionDecoder::new(
        Payload::from(&source.bytes[..]),
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &CancelAfter::Never,
    )
    .unwrap();
    let err = decoder.decode_next_row().unwrap_err();
    assert_eq!(err.reason, "invalid MQ marker following 0xFF", "{err}");
    assert!(err.offset.is_some(), "{err}");
    assert!(sink.bytes.is_empty());
}

#[test]
fn malformed_short_mq_smoke_is_bounded() {
    let limits = Limits::default();
    let table = table();
    for seed in 0..128u8 {
        let stream = [seed, seed.rotate_left(1), 0xff, 0xac];
        let mut source = record(7, 2, 0, 4, (2, -1), &stream);
        let hdr = header(&mut source);
        let mut bank = contexts(&limits);
        let mut sink = Sink::default();
        if let Ok(mut decoder) = GenericRegionDecoder::new(
            Payload::from(&source.bytes[..]),
            &hdr,
            &table,
            &mut bank,
            &mut sink,
            &limits,
            &CancelAfter::Never,
        ) {
            for _ in 0..2 {
                if decoder.decode_next_row().is_err() {
                    break;
                }
            }
            // A malformed stream may fail during decisions or terminal check.
        }
        assert!(sink.bytes.len() <= 2);
    }
}

#[test]
fn cancellation_at_every_checkpoint_never_reports_success() {
    let limits = Limits::default();
    let table = table();
    let mut cancelled_runs = 0;
    for polls in 0..10_000 {
        let cancellation = CancelAfter::new(polls);
        let mut source = record(3, 2, 0, 4, (2, -1), SHORT_STREAM);
        let hdr = header(&mut source);
        let mut bank = contexts(&limits);
        let mut sink = Sink::default();
        let result = (|| {
            let mut decoder = GenericRegionDecoder::new(
                Payload::from(&source.bytes[..]),
                &hdr,
                &table,
                &mut bank,
                &mut sink,
                &limits,
                &cancellation,
            )?;
            while decoder.decode_next_row()? {}
            decoder.finish()
        })();
        match result {
            Ok(report) => {
                assert_eq!(sink.bytes, [0xe0, 0xe0]);
                assert!(sink.flushed);
                assert_eq!(report.progress.rows_written, 2);
                // Cancellation was observed at a checkpoint in every earlier run.
                assert!(cancelled_runs > 2, "{cancelled_runs}");
                return;
            }
            Err(err) => {
                assert!(
                    matches!(
                        err,
                        Error {
                            kind: ErrorKind::Cancelled,
                            ..
                        }
                    ),
                    "poll {polls}: {err}"
                );
                assert!(sink.bytes.len() <= 2);
                cancelled_runs += 1;
            }
        }
    }
    panic!("decode never completed without cancellation");
}

#[test]
fn working_allocation_cap_counts_three_rows_at_the_exact_boundary() {
    let table = table();
    let attempt = |width: u32, max_allocation_bytes: u64| {
        let limits = Limits {
            io_chunk_bytes: 16,
            max_allocation_bytes,
            ..Limits::default()
        };
        let mut source = record(width, 1, 0, 4, (2, -1), SHORT_STREAM);
        let hdr = header(&mut source);
        let mut bank = contexts(&Limits::default());
        let mut sink = Sink::default();
        let result = GenericRegionDecoder::new(
            Payload::from(&source.bytes[..]),
            &hdr,
            &table,
            &mut bank,
            &mut sink,
            &limits,
            &CancelAfter::Never,
        )
        .map(|_| ());
        (result, ())
    };
    let required = |width| {
        let Err(err) = attempt(width, 16).0 else {
            panic!("accepted a 16-byte working allocation");
        };
        let ErrorKind::LimitExceeded {
            resource: "region working allocation bytes",
            limit: 16,
            attempted,
        } = err.kind
        else {
            panic!("unexpected error kind: {:?}", err.kind);
        };
        assert_eq!(err.offset, Some(11));
        attempted
    };
    let one_byte_rows = required(8);
    // Width 9 needs two bytes per row; the cap covers all three row buffers.
    assert_eq!(required(9), one_byte_rows + 3);
    let (at_cap, ()) = attempt(8, one_byte_rows);
    assert!(at_cap.is_ok());
    let (below_cap, _) = attempt(8, one_byte_rows - 1);
    assert!(matches!(
        below_cap.unwrap_err(),
        Error {
            kind: ErrorKind::LimitExceeded {
                resource: "region working allocation bytes",
                ..
            },
            ..
        }
    ));
}

#[test]
fn span_errors_have_stable_messages() {
    let limits = Limits::default();
    let table = table();
    let mut source = record(3, 1, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    source.bytes.pop();
    let mut bank = contexts(&limits);
    let mut sink = Sink::default();
    let err = match GenericRegionDecoder::new(
        Payload::from(&source.bytes[..]),
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &CancelAfter::Never,
    ) {
        Ok(_) => panic!("accepted a segment beyond the source"),
        Err(err) => err,
    };
    assert_eq!(
        err.to_string(),
        "malformed JBIG2 at byte 11, segment 1: segment data outside source"
    );
    common::assert_display_propagates_fmt_error(&err);
}
