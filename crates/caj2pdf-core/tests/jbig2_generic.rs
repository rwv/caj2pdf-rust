// SPDX-License-Identifier: MIT

//! Invented MQ probabilities and bytes test bounded image-model behavior only.
//! They do not establish T.88 Table E.1 or CAJ/HN pixel compatibility.

mod common;

use caj2pdf_core::{
    Cancellation, Limits, NeverCancel, RangedSource, SequentialSink,
    jbig2::{
        HeaderLimits, SegmentHeader, SegmentSpan,
        generic::{GenericBudget, GenericError, GenericErrorKind, GenericRegionDecoder},
        mq::{MQ_STATE_COUNT, MqBudget, MqContexts, MqErrorKind, MqState, MqTable},
        read_segment_header,
    },
};
use common::CancelAfter;
use std::{
    cell::Cell,
    future::Future,
    io,
    pin::pin,
    rc::Rc,
    task::{Context, Poll, Waker},
};

fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("unexpected pending test I/O"),
    }
}

struct Source {
    bytes: Vec<u8>,
    advertised: u64,
    max_read: usize,
    overreport: bool,
    stop_at: Option<u64>,
    error_at: Option<u64>,
    cancel_at: Option<u64>,
    read_calls: usize,
}
impl Source {
    fn new(bytes: Vec<u8>) -> Self {
        let advertised = bytes.len() as u64;
        Self {
            bytes,
            advertised,
            max_read: usize::MAX,
            overreport: false,
            stop_at: None,
            error_at: None,
            cancel_at: None,
            read_calls: 0,
        }
    }
}
impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.advertised
    }
    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        self.read_calls += 1;
        if self.overreport {
            return Ok(destination.len() + 1);
        }
        if self.error_at == Some(offset) {
            return Err(caj2pdf_core::Error::Io(io::Error::other(
                "test source failure",
            )));
        }
        if self.cancel_at == Some(offset) {
            return Err(caj2pdf_core::Error::Cancelled);
        }
        if self.stop_at.is_some_and(|end| offset >= end) {
            return Ok(0);
        }
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let count = self
            .bytes
            .len()
            .saturating_sub(start)
            .min(destination.len())
            .min(self.max_read);
        if count > 0 {
            destination[..count].copy_from_slice(&self.bytes[start..start + count]);
        }
        Ok(count)
    }
}
struct Sink {
    bytes: Vec<u8>,
    max_write: usize,
    zero: bool,
    overreport: bool,
    pending: bool,
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
            overreport: false,
            pending: false,
            cancel: None,
            flushed: false,
            flush_fail: false,
            cancel_on_flush: None,
        }
    }
}
impl SequentialSink for Sink {
    async fn write(&mut self, bytes: &[u8]) -> caj2pdf_core::Result<usize> {
        if self.pending {
            std::future::pending::<()>().await;
        }
        if self.overreport {
            return Ok(bytes.len() + 1);
        }
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
    async fn flush(&mut self) -> caj2pdf_core::Result<()> {
        if self.flush_fail {
            return Err(caj2pdf_core::Error::Io(io::Error::other(
                "test flush failure",
            )));
        }
        if let Some(flag) = &self.cancel_on_flush {
            flag.set(true);
        }
        self.flushed = true;
        Ok(())
    }
}
struct Flag(Rc<Cell<bool>>);
impl Cancellation for Flag {
    fn is_cancelled(&self) -> bool {
        self.0.get()
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
    ready(read_segment_header(
        source,
        SegmentSpan {
            offset: 0,
            length: source.advertised,
        },
        &Limits::default(),
        HeaderLimits::default(),
        &NeverCancel,
    ))
    .unwrap()
}
fn table() -> MqTable {
    // With C initially zero and Qe=0x4000, the invented state machine emits
    // LPS=1 for each early symbol; every renormalization restores A=0x8000.
    let mut states = vec![
        MqState {
            qe: 0x4000,
            next_mps: 0,
            next_lps: 0,
            switch_mps: false
        };
        MQ_STATE_COUNT
    ];
    states[0].next_lps = 1;
    states[1].next_lps = 1;
    states[1].next_mps = 1;
    MqTable::new(states, &Limits::default()).unwrap()
}
fn contexts(limits: &Limits, budget: &MqBudget) -> MqContexts {
    MqContexts::new(1024, limits, budget).unwrap()
}
const SHORT_STREAM: &[u8] = &[0, 0, 0, 0xff, 0xac];

#[test]
fn streams_packed_rows_and_distinguishes_semantic_from_physical_input() {
    let limits = Limits {
        io_chunk_bytes: 2,
        ..Limits::default()
    };
    let mq_budget = MqBudget::default();
    let mut source = record(3, 2, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    source.max_read = 1;
    let table = table();
    let mut bank = contexts(&limits, &mq_budget);
    bank.set(
        0,
        caj2pdf_core::jbig2::mq::MqContext {
            state_index: 1,
            mps: true,
        },
    )
    .unwrap();
    let mut sink = Sink {
        max_write: 1,
        ..Sink::default()
    };
    let mut decoder = ready(GenericRegionDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &NeverCancel,
        mq_budget,
        GenericBudget::default(),
    ))
    .unwrap();
    assert_eq!(decoder.progress().info.row_stride, 1);
    assert!(ready(decoder.decode_next_row()).unwrap());
    assert_eq!(
        (
            decoder.progress().rows_written,
            decoder.progress().pixels_decoded
        ),
        (1, 3)
    );
    assert!(ready(decoder.decode_next_row()).unwrap());
    assert!(!ready(decoder.decode_next_row()).unwrap());
    let report = ready(decoder.finish()).unwrap();
    assert_eq!(sink.bytes, [0xe0, 0xe0]); // three one-bits, then five zero padding bits per row
    assert!(sink.flushed);
    assert_eq!(report.progress.output_bytes_written, 2);
    assert_eq!(report.progress.pixels_decoded, 6);
    assert_eq!(report.progress.rows_written, 2);
    assert!(report.progress.mq.source_bytes_fetched >= 4);
    assert!(
        report.progress.mq.current_input_offset < report.mq_span.offset + report.mq_span.length
    );
    assert!(!report.progress.poisoned);
}

#[test]
fn third_row_uses_both_prior_rows_after_rotation() {
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let mut source = record(3, 3, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    let table = table();
    let mut bank = contexts(&limits, &mq_budget);
    let mut sink = Sink::default();
    let mut decoder = ready(GenericRegionDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &NeverCancel,
        mq_budget,
        GenericBudget::default(),
    ))
    .unwrap();
    for _ in 0..3 {
        assert!(ready(decoder.decode_next_row()).unwrap());
    }
    ready(decoder.finish()).unwrap();
    assert_eq!(sink.bytes, [0xe0, 0xe0, 0xe0]);
    // At (x=0,y=2): prior-two bits are 011 and prior-one bits are
    // 00111, with current-left 00: 0b011_00111_00 = 412.
    assert_eq!(bank.get(412).unwrap().state_index, 1);
}

#[test]
fn rejects_header_modes_at_placement_and_truncation_before_mq() {
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
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
        (3, 2, 0, 4, (1, -1), SHORT_STREAM, "unsupported adaptive"),
        (3, 2, 0, 4, (2, -1), &[0][..], "terminal"),
    ] {
        let mut source = record(w, h, region, flags, at, payload);
        let hdr = header(&mut source);
        let mut bank = contexts(&limits, &mq_budget);
        let mut sink = Sink::default();
        let err = match ready(GenericRegionDecoder::new(
            &mut source,
            &hdr,
            &table,
            &mut bank,
            &mut sink,
            &limits,
            &NeverCancel,
            mq_budget,
            GenericBudget::default(),
        )) {
            Ok(_) => panic!("accepted invalid {expected}"),
            Err(e) => e,
        };
        assert!(err.to_string().contains(expected), "{err}");
        assert!(sink.bytes.is_empty());
    }
    let mut source = record(3, 2, 0, 4, (2, -1), SHORT_STREAM);
    let mut hdr = header(&mut source);
    hdr.segment_type = 4;
    let mut bank = contexts(&limits, &mq_budget);
    let mut sink = Sink::default();
    let err = match ready(GenericRegionDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &NeverCancel,
        mq_budget,
        GenericBudget::default(),
    )) {
        Ok(_) => panic!("accepted type 4"),
        Err(e) => e,
    };
    assert!(matches!(
        err.kind,
        GenericErrorKind::Unsupported {
            feature: "segment type",
            value: 4
        }
    ));
    let mut source = record(3, 2, 0, 4, (2, -1), SHORT_STREAM);
    source.bytes[19..23].copy_from_slice(&u32::MAX.to_be_bytes());
    let hdr = header(&mut source);
    let mut bank = contexts(&limits, &mq_budget);
    let mut sink = Sink::default();
    let err = match ready(GenericRegionDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &NeverCancel,
        mq_budget,
        GenericBudget::default(),
    )) {
        Ok(_) => panic!("accepted overflowing region x"),
        Err(e) => e,
    };
    assert!(matches!(
        err.kind,
        GenericErrorKind::Malformed("region x plus width overflows")
    ));

    let mut source = record(3, 2, 0, 4, (2, -1), SHORT_STREAM);
    source.bytes.truncate(11 + 19);
    source.bytes[7..11].copy_from_slice(&19u32.to_be_bytes());
    source.advertised = source.bytes.len() as u64;
    let hdr = header(&mut source);
    let mut bank = contexts(&limits, &mq_budget);
    let mut sink = Sink::default();
    let err = match ready(GenericRegionDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &NeverCancel,
        mq_budget,
        GenericBudget::default(),
    )) {
        Ok(_) => panic!("accepted missing adaptive coordinate"),
        Err(e) => e,
    };
    assert!(matches!(
        err.kind,
        GenericErrorKind::Truncated("template-2 adaptive pixel")
    ));
}

#[test]
fn preflights_area_output_allocation_and_input_budgets() {
    let mq_budget = MqBudget::default();
    let table = table();
    let mut source = record(9, 3, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    let cases = [
        (
            Limits::default(),
            GenericBudget {
                max_width: 8,
                ..GenericBudget::default()
            },
            mq_budget,
            "width",
        ),
        (
            Limits::default(),
            GenericBudget {
                max_pixels: 26,
                ..GenericBudget::default()
            },
            mq_budget,
            "pixels",
        ),
        (
            Limits::default(),
            GenericBudget {
                max_context_work: 269,
                ..GenericBudget::default()
            },
            mq_budget,
            "context work",
        ),
        (
            Limits {
                max_output_bytes: 5,
                ..Limits::default()
            },
            GenericBudget::default(),
            mq_budget,
            "output",
        ),
        (
            Limits {
                max_allocation_bytes: 4_000,
                ..Limits::default()
            },
            GenericBudget::default(),
            mq_budget,
            "allocation",
        ),
        (
            Limits {
                max_input_bytes: 24,
                ..Limits::default()
            },
            GenericBudget::default(),
            mq_budget,
            "input",
        ),
        (
            Limits::default(),
            GenericBudget::default(),
            MqBudget {
                max_symbols: 26,
                ..mq_budget
            },
            "symbols",
        ),
        (
            Limits::default(),
            GenericBudget::default(),
            MqBudget {
                max_span_bytes: 4,
                ..mq_budget
            },
            "span",
        ),
    ];
    for (limits, region_budget, mq_budget, expected) in cases {
        let mut bank = contexts(&Limits::default(), &MqBudget::default());
        let mut sink = Sink::default();
        let err = match ready(GenericRegionDecoder::new(
            &mut source,
            &hdr,
            &table,
            &mut bank,
            &mut sink,
            &limits,
            &NeverCancel,
            mq_budget,
            region_budget,
        )) {
            Ok(_) => panic!("accepted low {expected} budget"),
            Err(e) => e,
        };
        assert!(err.to_string().contains(expected), "{err}");
        assert!(sink.bytes.is_empty());
    }
}

#[test]
fn additional_constructor_bounds_and_located_source_errors() {
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let table = table();
    let mut source = record(3, 2, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    let cases = [
        (
            GenericBudget {
                max_height: 1,
                ..GenericBudget::default()
            },
            mq_budget,
            1024,
            "height",
        ),
        (
            GenericBudget::default(),
            MqBudget {
                max_contexts: 1023,
                ..mq_budget
            },
            1024,
            "MQ contexts",
        ),
        (
            GenericBudget::default(),
            mq_budget,
            1023,
            "1024 generic MQ contexts",
        ),
    ];
    for (region_budget, selected_mq_budget, count, expected) in cases {
        let mut bank = MqContexts::new(count, &limits, &mq_budget).unwrap();
        let mut sink = Sink::default();
        let err = match ready(GenericRegionDecoder::new(
            &mut source,
            &hdr,
            &table,
            &mut bank,
            &mut sink,
            &limits,
            &NeverCancel,
            selected_mq_budget,
            region_budget,
        )) {
            Ok(_) => panic!("accepted invalid {expected}"),
            Err(e) => e,
        };
        assert!(err.to_string().contains(expected), "{err}");
    }

    // Both width and height are u32, so their u64 product fits. Ten
    // context probes per pixel can still overflow u64 and must be rejected.
    let mut source = record(u32::MAX, u32::MAX, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    let mut bank = contexts(&limits, &mq_budget);
    let mut sink = Sink::default();
    let err = match ready(GenericRegionDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &NeverCancel,
        MqBudget {
            max_symbols: u64::MAX,
            ..mq_budget
        },
        GenericBudget {
            max_width: u32::MAX,
            max_height: u32::MAX,
            max_pixels: u64::MAX,
            max_context_work: u64::MAX,
        },
    )) {
        Ok(_) => panic!("accepted overflowing work"),
        Err(e) => e,
    };
    assert!(matches!(
        err.kind,
        GenericErrorKind::Malformed("context work overflows")
    ));

    let mut source = record(3, 2, 0, 4, (2, -1), SHORT_STREAM);
    source.bytes[23..27].copy_from_slice(&u32::MAX.to_be_bytes());
    let hdr = header(&mut source);
    let mut bank = contexts(&limits, &mq_budget);
    let mut sink = Sink::default();
    let err = match ready(GenericRegionDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &NeverCancel,
        mq_budget,
        GenericBudget::default(),
    )) {
        Ok(_) => panic!("accepted overflowing y"),
        Err(e) => e,
    };
    assert!(matches!(
        err.kind,
        GenericErrorKind::Malformed("region y plus height overflows")
    ));

    let mut source = record(3, 2, 0, 4, (2, -1), SHORT_STREAM);
    let mut hdr = header(&mut source);
    hdr.data.offset = u64::MAX - 1;
    let mut bank = contexts(&limits, &mq_budget);
    let mut sink = Sink::default();
    let err = match ready(GenericRegionDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &NeverCancel,
        mq_budget,
        GenericBudget::default(),
    )) {
        Ok(_) => panic!("accepted overflowing span"),
        Err(e) => e,
    };
    assert!(matches!(
        err.kind,
        GenericErrorKind::InvalidSpan("segment end overflows")
    ));

    let mut source = record(3, 2, 0, 4, (2, -1), SHORT_STREAM);
    let mut hdr = header(&mut source);
    source.advertised -= 1;
    let mut bank = contexts(&limits, &mq_budget);
    let mut sink = Sink::default();
    let err = match ready(GenericRegionDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &NeverCancel,
        mq_budget,
        GenericBudget::default(),
    )) {
        Ok(_) => panic!("accepted outside-source span"),
        Err(e) => e,
    };
    assert!(matches!(
        err.kind,
        GenericErrorKind::InvalidSpan("segment data outside source")
    ));
    source.advertised += 1;
    hdr.data.length = 17;
    let err = match ready(GenericRegionDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &NeverCancel,
        mq_budget,
        GenericBudget::default(),
    )) {
        Ok(_) => panic!("accepted short generic header"),
        Err(e) => e,
    };
    assert!(matches!(
        err.kind,
        GenericErrorKind::Truncated("generic flags")
    ));

    let mut source = record(3, 2, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    let flag = Rc::new(Cell::new(true));
    let cancellation = Flag(flag);
    let mut bank = contexts(&limits, &mq_budget);
    let mut sink = Sink::default();
    let err = match ready(GenericRegionDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &cancellation,
        mq_budget,
        GenericBudget::default(),
    )) {
        Ok(_) => panic!("accepted pre-cancelled region"),
        Err(e) => e,
    };
    assert!(matches!(err.kind, GenericErrorKind::Cancelled));

    for source_cancel in [false, true] {
        let mut source = record(3, 2, 0, 4, (2, -1), SHORT_STREAM);
        let hdr = header(&mut source);
        if source_cancel {
            source.cancel_at = Some(hdr.data.offset);
        } else {
            source.error_at = Some(hdr.data.offset);
        }
        let mut bank = contexts(&limits, &mq_budget);
        let mut sink = Sink::default();
        let err = match ready(GenericRegionDecoder::new(
            &mut source,
            &hdr,
            &table,
            &mut bank,
            &mut sink,
            &limits,
            &NeverCancel,
            mq_budget,
            GenericBudget::default(),
        )) {
            Ok(_) => panic!("accepted failing source"),
            Err(e) => e,
        };
        if source_cancel {
            assert!(matches!(err.kind, GenericErrorKind::Cancelled));
            assert!(err.to_string().contains("cancelled"));
        } else {
            assert!(matches!(err.kind, GenericErrorKind::Source(_)));
            assert!(err.to_string().contains("source"));
            assert!(std::error::Error::source(&err).is_some());
        }
    }
}

#[test]
fn source_short_overreported_and_sink_failure_are_typed() {
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let table = table();
    for overreport in [false, true] {
        let mut source = record(3, 1, 0, 4, (2, -1), SHORT_STREAM);
        let hdr = header(&mut source);
        if overreport {
            source.overreport = true;
        } else {
            source.bytes.truncate(11);
        }
        let mut bank = contexts(&limits, &mq_budget);
        let mut sink = Sink::default();
        let err = match ready(GenericRegionDecoder::new(
            &mut source,
            &hdr,
            &table,
            &mut bank,
            &mut sink,
            &limits,
            &NeverCancel,
            mq_budget,
            GenericBudget::default(),
        )) {
            Ok(_) => panic!("accepted bad source"),
            Err(e) => e,
        };
        assert!(
            matches!(
                err.kind,
                GenericErrorKind::Truncated(_) | GenericErrorKind::Malformed(_)
            ),
            "{err}"
        );
    }
    let mut source = record(3, 1, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    source.max_read = 1;
    source.stop_at = Some(hdr.data.offset + 21);
    let mut bank = contexts(&limits, &mq_budget);
    let mut sink = Sink::default();
    let err = match ready(GenericRegionDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &NeverCancel,
        mq_budget,
        GenericBudget::default(),
    )) {
        Ok(_) => panic!("accepted short MQ source"),
        Err(e) => e,
    };
    assert!(matches!(err.kind, GenericErrorKind::Mq(_)), "{err}");
    assert!(std::error::Error::source(&err).is_some());
    for (zero, overreport) in [(true, false), (false, true)] {
        let mut source = record(3, 1, 0, 4, (2, -1), SHORT_STREAM);
        let hdr = header(&mut source);
        let mut bank = contexts(&limits, &mq_budget);
        let mut sink = Sink {
            zero,
            overreport,
            ..Sink::default()
        };
        let mut decoder = ready(GenericRegionDecoder::new(
            &mut source,
            &hdr,
            &table,
            &mut bank,
            &mut sink,
            &limits,
            &NeverCancel,
            mq_budget,
            GenericBudget::default(),
        ))
        .unwrap();
        let err = ready(decoder.decode_next_row()).unwrap_err();
        assert!(matches!(err.kind, GenericErrorKind::Sink(_)));
        assert!(err.to_string().contains("sink"));
        assert!(std::error::Error::source(&err).is_some());
        assert!(decoder.progress().poisoned);
        assert!(matches!(
            ready(decoder.decode_next_row()).unwrap_err().kind,
            GenericErrorKind::Poisoned
        ));
    }
}

#[test]
fn retries_partial_row_writes_and_reports_flush_failure() {
    let limits = Limits {
        io_chunk_bytes: 2,
        ..Limits::default()
    };
    let mq_budget = MqBudget::default();
    let table = table();
    let stream = [0, 0, 0, 0, 0xff, 0xac];
    let mut source = record(9, 1, 0, 4, (2, -1), &stream);
    let hdr = header(&mut source);
    let mut bank = contexts(&limits, &mq_budget);
    let mut sink = Sink {
        max_write: 1,
        flush_fail: true,
        ..Sink::default()
    };
    let mut decoder = ready(GenericRegionDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &NeverCancel,
        mq_budget,
        GenericBudget::default(),
    ))
    .unwrap();
    assert!(ready(decoder.decode_next_row()).unwrap());
    let err = ready(decoder.finish()).unwrap_err();
    assert!(matches!(err.kind, GenericErrorKind::Sink(_)), "{err}");
    assert_eq!(err.output_bytes_written, 2);
    assert_eq!(sink.bytes, [0xff, 0x80]);
    assert!(!sink.flushed);
}

#[test]
fn cancellation_and_dropped_pending_row_poison_decoder() {
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let table = table();
    let flag = Rc::new(Cell::new(false));
    let mut source = record(3, 1, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    let mut bank = contexts(&limits, &mq_budget);
    let mut sink = Sink {
        cancel: Some(flag.clone()),
        ..Sink::default()
    };
    let cancellation = Flag(flag);
    let mut decoder = ready(GenericRegionDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &cancellation,
        mq_budget,
        GenericBudget::default(),
    ))
    .unwrap();
    let err = ready(decoder.decode_next_row()).unwrap_err();
    assert!(matches!(err.kind, GenericErrorKind::Cancelled));
    assert_eq!(err.output_bytes_written, 1);
    assert!(decoder.progress().poisoned);
    drop(decoder);
    assert_eq!(sink.bytes, [0xe0]);

    let mut source = record(3, 1, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    let mut bank = contexts(&limits, &mq_budget);
    let mut sink = Sink {
        pending: true,
        ..Sink::default()
    };
    let mut decoder = ready(GenericRegionDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &NeverCancel,
        mq_budget,
        GenericBudget::default(),
    ))
    .unwrap();
    {
        let mut future = pin!(decoder.decode_next_row());
        assert!(matches!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
    }
    assert!(decoder.progress().poisoned);
    assert!(matches!(
        ready(decoder.decode_next_row()).unwrap_err().kind,
        GenericErrorKind::Poisoned
    ));
}

#[test]
fn cancellation_before_next_row_and_during_flush_never_reports_success() {
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let table = table();
    let flag = Rc::new(Cell::new(false));
    let cancellation = Flag(flag.clone());
    let mut source = record(3, 1, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    let mut bank = contexts(&limits, &mq_budget);
    let mut sink = Sink::default();
    let mut decoder = ready(GenericRegionDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &cancellation,
        mq_budget,
        GenericBudget::default(),
    ))
    .unwrap();
    flag.set(true);
    let err = ready(decoder.decode_next_row()).unwrap_err();
    assert!(matches!(err.kind, GenericErrorKind::Cancelled));
    assert_eq!(err.pixels_decoded, 0);
    drop(decoder);
    assert!(sink.bytes.is_empty());

    flag.set(false);
    let mut source = record(3, 1, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    let mut bank = contexts(&limits, &mq_budget);
    let mut sink = Sink {
        cancel_on_flush: Some(flag),
        ..Sink::default()
    };
    let mut decoder = ready(GenericRegionDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &cancellation,
        mq_budget,
        GenericBudget::default(),
    ))
    .unwrap();
    ready(decoder.decode_next_row()).unwrap();
    let err = ready(decoder.finish()).unwrap_err();
    assert!(matches!(err.kind, GenericErrorKind::Cancelled));
    assert_eq!(
        (
            err.rows_written,
            err.pixels_decoded,
            err.output_bytes_written
        ),
        (1, 3, 1)
    );
    assert_eq!(sink.bytes, [0xe0]);
    assert!(sink.flushed);
}

#[test]
fn rejects_terminal_errors_and_incomplete_finish() {
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let table = table();
    let mut source = record(3, 1, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    let mut bank = contexts(&limits, &mq_budget);
    let mut sink = Sink::default();
    let decoder = ready(GenericRegionDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &NeverCancel,
        mq_budget,
        GenericBudget::default(),
    ))
    .unwrap();
    let err = ready(decoder.finish()).unwrap_err();
    assert!(matches!(err.kind, GenericErrorKind::Incomplete));
    assert!(err.to_string().contains("not all rows"));
    assert!(std::error::Error::source(&err).is_none());
    for bad_tail in [[0xff, 0xab], [0x00, 0xac]] {
        let mut bytes = SHORT_STREAM.to_vec();
        bytes[3..].copy_from_slice(&bad_tail);
        let mut source = record(3, 1, 0, 4, (2, -1), &bytes);
        let hdr = header(&mut source);
        let mut bank = contexts(&limits, &mq_budget);
        let mut sink = Sink::default();
        let mut decoder = ready(GenericRegionDecoder::new(
            &mut source,
            &hdr,
            &table,
            &mut bank,
            &mut sink,
            &limits,
            &NeverCancel,
            mq_budget,
            GenericBudget::default(),
        ))
        .unwrap();
        ready(decoder.decode_next_row()).unwrap();
        let err = ready(decoder.finish()).unwrap_err();
        assert!(matches!(err.kind, GenericErrorKind::Mq(_)), "{err}");
        assert!(err.to_string().contains("MQ: "), "{err}");
        assert!(std::error::Error::source(&err).is_some());
        assert!(!sink.flushed);
    }
}

#[test]
fn unexpected_internal_marker_keeps_the_mq_source_location() {
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let table = table();
    let mut source = record(128, 1, 0, 4, (2, -1), &[0, 0, 0xff, 0x90, 0xff, 0xac]);
    let hdr = header(&mut source);
    let mut bank = contexts(&limits, &mq_budget);
    let mut sink = Sink::default();
    let mut decoder = ready(GenericRegionDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &NeverCancel,
        mq_budget,
        GenericBudget::default(),
    ))
    .unwrap();
    let err = ready(decoder.decode_next_row()).unwrap_err();
    match &err.kind {
        GenericErrorKind::Mq(inner) => {
            assert!(matches!(inner.kind, MqErrorKind::InvalidMarker(0x90)));
            assert_eq!(Some(err.offset), inner.offset);
        }
        _ => panic!("expected located MQ marker error: {err}"),
    }
    assert!(decoder.progress().poisoned);
    assert!(sink.bytes.is_empty());
}

#[test]
fn malformed_short_mq_smoke_is_bounded() {
    let limits = Limits::default();
    let mq_budget = MqBudget {
        max_work: 2048,
        ..MqBudget::default()
    };
    let table = table();
    for seed in 0..128u8 {
        let stream = [seed, seed.rotate_left(1), 0xff, 0xac];
        let mut source = record(7, 2, 0, 4, (2, -1), &stream);
        let hdr = header(&mut source);
        let mut bank = contexts(&limits, &mq_budget);
        let mut sink = Sink::default();
        if let Ok(mut decoder) = ready(GenericRegionDecoder::new(
            &mut source,
            &hdr,
            &table,
            &mut bank,
            &mut sink,
            &limits,
            &NeverCancel,
            mq_budget,
            GenericBudget::default(),
        )) {
            for _ in 0..2 {
                if ready(decoder.decode_next_row()).is_err() {
                    break;
                }
            }
            // A malformed stream may fail during decisions or terminal check.
        }
        assert!(sink.bytes.len() <= 2);
        assert!(source.read_calls <= 64);
    }
}

#[test]
fn cancellation_at_every_checkpoint_never_reports_success() {
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let table = table();
    let mut cancelled_runs = 0;
    for polls in 0..10_000 {
        let cancellation = CancelAfter::new(polls);
        let mut source = record(3, 2, 0, 4, (2, -1), SHORT_STREAM);
        let hdr = header(&mut source);
        let mut bank = contexts(&limits, &mq_budget);
        let mut sink = Sink::default();
        let result = ready(async {
            let mut decoder = GenericRegionDecoder::new(
                &mut source,
                &hdr,
                &table,
                &mut bank,
                &mut sink,
                &limits,
                &cancellation,
                mq_budget,
                GenericBudget::default(),
            )
            .await?;
            while decoder.decode_next_row().await? {}
            decoder.finish().await
        });
        match result {
            Ok(report) => {
                assert_eq!(sink.bytes, [0xe0, 0xe0]);
                assert!(sink.flushed);
                assert_eq!(report.progress.rows_written, 2);
                // Cancellation was observed at a checkpoint in every earlier run.
                assert!(cancelled_runs > 10, "{cancelled_runs}");
                return;
            }
            Err(err) => {
                let cancelled = match &err.kind {
                    GenericErrorKind::Cancelled => true,
                    GenericErrorKind::Mq(inner) => matches!(inner.kind, MqErrorKind::Cancelled),
                    _ => false,
                };
                assert!(cancelled, "poll {polls}: {err}");
                assert!(err.output_bytes_written <= 2);
                assert_eq!(sink.bytes.len() as u64, err.output_bytes_written);
                cancelled_runs += 1;
            }
        }
    }
    panic!("decode never completed without cancellation");
}

#[test]
fn working_allocation_cap_counts_three_rows_at_the_exact_boundary() {
    let mq_budget = MqBudget::default();
    let table = table();
    let attempt = |width: u32, max_allocation_bytes: u64| {
        let limits = Limits {
            io_chunk_bytes: 16,
            max_allocation_bytes,
            ..Limits::default()
        };
        let mut source = record(width, 1, 0, 4, (2, -1), SHORT_STREAM);
        let hdr = header(&mut source);
        let mut bank = contexts(&Limits::default(), &mq_budget);
        let mut sink = Sink::default();
        let result = ready(GenericRegionDecoder::new(
            &mut source,
            &hdr,
            &table,
            &mut bank,
            &mut sink,
            &limits,
            &NeverCancel,
            mq_budget,
            GenericBudget::default(),
        ))
        .map(|_| ());
        (result, source.read_calls)
    };
    let required = |width| {
        let Err(err) = attempt(width, 16).0 else {
            panic!("accepted a 16-byte working allocation");
        };
        let GenericErrorKind::LimitExceeded {
            resource: "region working allocation bytes",
            limit: 16,
            attempted,
        } = err.kind
        else {
            panic!("unexpected error kind: {:?}", err.kind);
        };
        assert_eq!(err.offset, 11);
        attempted
    };
    let one_byte_rows = required(8);
    // Width 9 needs two bytes per row; the cap covers all three row buffers.
    assert_eq!(required(9), one_byte_rows + 3);
    let (at_cap, reads) = attempt(8, one_byte_rows);
    assert!(at_cap.is_ok());
    assert!(reads > 0);
    let (below_cap, _) = attempt(8, one_byte_rows - 1);
    assert!(matches!(
        below_cap.unwrap_err().kind,
        GenericErrorKind::LimitExceeded {
            resource: "region working allocation bytes",
            ..
        }
    ));
}

#[test]
fn finish_after_a_dropped_row_future_is_poisoned_without_flush() {
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let table = table();
    let mut source = record(3, 1, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    let mut bank = contexts(&limits, &mq_budget);
    let mut sink = Sink {
        pending: true,
        ..Sink::default()
    };
    let mut decoder = ready(GenericRegionDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &NeverCancel,
        mq_budget,
        GenericBudget::default(),
    ))
    .unwrap();
    {
        let mut future = pin!(decoder.decode_next_row());
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
    let err = ready(decoder.finish()).unwrap_err();
    assert!(matches!(err.kind, GenericErrorKind::Poisoned));
    assert!(err.to_string().contains("decoder state is poisoned"));
    assert_eq!((err.rows_written, err.pixels_decoded), (0, 3));
    assert!(!sink.flushed);
}

#[test]
fn span_errors_and_unreachable_allocation_failure_have_stable_messages() {
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let table = table();
    let mut source = record(3, 1, 0, 4, (2, -1), SHORT_STREAM);
    let hdr = header(&mut source);
    source.advertised -= 1;
    let mut bank = contexts(&limits, &mq_budget);
    let mut sink = Sink::default();
    let err = match ready(GenericRegionDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut bank,
        &mut sink,
        &limits,
        &NeverCancel,
        mq_budget,
        GenericBudget::default(),
    )) {
        Ok(_) => panic!("accepted a segment beyond the source"),
        Err(err) => err,
    };
    assert_eq!(
        err.to_string(),
        "JBIG2 generic region segment 1 at source byte 11: \
         invalid span: segment data outside source"
    );
    // Row reservation failure needs a real allocator failure; the message is
    // still part of the public error contract.
    let allocation = GenericError {
        offset: 11,
        segment: 1,
        rows_written: 0,
        pixels_decoded: 0,
        output_bytes_written: 0,
        kind: GenericErrorKind::AllocationFailed,
    };
    assert!(allocation.to_string().ends_with(": row allocation failed"));
    assert!(std::error::Error::source(&allocation).is_none());
    common::assert_display_propagates_fmt_error(&allocation);
}
