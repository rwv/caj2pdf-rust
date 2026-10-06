// SPDX-License-Identifier: MIT

//! Adversarial tests of the public refinement host and its caller-owned I/O.

mod common;

use caj2pdf_core::{
    Cancellation, Error, Limits, MAX_BUDGET_COUNT, NeverCancel, RangedSource, SequentialSink,
    jbig2::{
        dictionary::SymbolDescriptor,
        integer::BITMAP_BASE,
        mq::{ArithmeticErrorKind, CodedSpan, ContextBank, MqBudget, MqDecoder, MqTable},
        refinement::{
            RefinementBudget, RefinementDecoder, RefinementError, RefinementErrorKind,
            RefinementReference, RefinementRequest,
        },
    },
};
use std::{cell::Cell, io, rc::Rc};

#[derive(Default)]
struct Source {
    bytes: Vec<u8>,
    advertised: Option<u64>,
    max_read: usize,
    zero_at: Option<u64>,
    error_at: Option<u64>,
    overreport_at: Option<u64>,
    cancel_after_read: Option<Rc<Cell<bool>>>,
    calls: u64,
    max_request: usize,
}

impl Source {
    fn new(bytes: &[u8]) -> Self {
        Self {
            bytes: bytes.to_vec(),
            max_read: usize::MAX,
            ..Self::default()
        }
    }
}

impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.advertised.unwrap_or(self.bytes.len() as u64)
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> caj2pdf_core::Result<usize> {
        self.calls += 1;
        self.max_request = self.max_request.max(destination.len());
        if self.error_at == Some(offset) {
            return Err(Error::Io(io::Error::other(
                "injected reference read failure",
            )));
        }
        if self.overreport_at == Some(offset) {
            return Ok(destination.len() + 1);
        }
        if self.zero_at == Some(offset) {
            return Ok(0);
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
            if let Some(flag) = &self.cancel_after_read {
                flag.set(true);
            }
        }
        Ok(count)
    }
}

#[derive(Default)]
struct Sink {
    bytes: Vec<u8>,
    max_write: usize,
    zero_on_call: Option<u64>,
    error_on_call: Option<u64>,
    overreport_on_call: Option<u64>,
    cancel_after_write: Option<Rc<Cell<bool>>>,
    calls: u64,
    max_request: usize,
    flush_calls: u64,
    flush_error: bool,
    cancel_after_flush: Option<Rc<Cell<bool>>>,
}

impl Sink {
    fn new() -> Self {
        Self {
            max_write: usize::MAX,
            ..Self::default()
        }
    }
}

impl SequentialSink for Sink {
    fn write(&mut self, bytes: &[u8]) -> caj2pdf_core::Result<usize> {
        self.calls += 1;
        self.max_request = self.max_request.max(bytes.len());
        if self.error_on_call == Some(self.calls) {
            return Err(Error::Io(io::Error::other(
                "injected refinement write failure",
            )));
        }
        if self.overreport_on_call == Some(self.calls) {
            return Ok(bytes.len() + 1);
        }
        if self.zero_on_call == Some(self.calls) {
            return Ok(0);
        }
        let count = bytes.len().min(self.max_write);
        self.bytes.extend_from_slice(&bytes[..count]);
        if let Some(flag) = &self.cancel_after_write {
            flag.set(true);
        }
        Ok(count)
    }

    fn flush(&mut self) -> caj2pdf_core::Result<()> {
        self.flush_calls += 1;
        if self.flush_error {
            return Err(Error::Io(io::Error::other(
                "injected refinement flush failure",
            )));
        }
        if let Some(flag) = &self.cancel_after_flush {
            flag.set(true);
        }
        Ok(())
    }
}

struct Flag(Rc<Cell<bool>>);

impl Cancellation for Flag {
    fn is_cancelled(&self) -> bool {
        self.0.get()
    }
}

fn table() -> MqTable {
    MqTable::standard()
}

/// A coding unit's contexts with the bitmap range at
/// [`BITMAP_BASE`]; refinement needs no IAID contexts.
fn contexts(limits: &Limits, budget: &MqBudget) -> ContextBank {
    budget.context_bank(BITMAP_BASE + 1024, limits).unwrap()
}

/// An MQ stream that refines `request` to an all-set target over the
/// reference packed in `reference` (absent bytes read as zero).
fn set_stream(request: &RefinementRequest, reference: &[u8]) -> Vec<u8> {
    let symbol = request.reference.symbol;
    // Oversized requests are refused before any decision.
    if u64::from(request.width) * u64::from(request.height) > 1 << 16
        || u64::from(symbol.width) * u64::from(symbol.height) > 1 << 16
    {
        return common::mq_encoder().finish();
    }
    let rows: Vec<Vec<bool>> = (0..u64::from(symbol.height))
        .map(|y| {
            (0..u64::from(symbol.width))
                .map(|x| {
                    let at =
                        symbol.relative_store_offset + y * u64::from(symbol.row_stride) + x / 8;
                    reference
                        .get(at as usize)
                        .is_some_and(|byte| byte & (0x80 >> (x % 8)) != 0)
                })
                .collect()
        })
        .collect();
    let target = vec![vec![true; request.width as usize]; request.height as usize];
    let mut encoder = common::mq_encoder();
    encoder.template1(
        0,
        &target,
        &rows,
        (
            i64::from(request.reference_dx),
            i64::from(request.reference_dy),
        ),
    );
    encoder.finish()
}

fn reference(width: u32, height: u32) -> RefinementReference {
    let stride = width.div_ceil(8);
    RefinementReference {
        store_base: 0,
        symbol: SymbolDescriptor {
            width,
            height,
            row_stride: stride,
            relative_store_offset: 0,
            stored_bytes: u64::from(stride) * u64::from(height),
        },
    }
}

fn request(width: u32, height: u32, reference: RefinementReference) -> RefinementRequest {
    RefinementRequest {
        width,
        height,
        template: 1,
        typical_prediction: false,
        reference_dx: 0,
        reference_dy: 0,
        reference,
    }
}

fn observe_error<C: Cancellation>(
    mut reference_source: Source,
    mut sink: Sink,
    request: RefinementRequest,
    budget: RefinementBudget,
    limits: Limits,
    cancellation: &C,
) -> (RefinementError, Source, Sink) {
    let mq_budget = MqBudget::default();
    let table = table();
    let mut contexts = contexts(&limits, &mq_budget);
    let bytes = set_stream(&request, &reference_source.bytes);
    let mut mq_source = Source::new(&bytes);
    let mut mq = MqDecoder::new(
        &mut mq_source,
        CodedSpan {
            offset: 0,
            length: bytes.len() as u64,
        },
        &table,
        &mut contexts,
        &limits,
        cancellation,
        mq_budget,
    )
    .unwrap();
    let mut host =
        RefinementDecoder::new(&mut mq, &mut sink, &limits, cancellation, budget).unwrap();
    let error = host
        .decode_bitmap(&mut reference_source, request)
        .unwrap_err();
    // Diagnostics must identify a failed bitmap without exposing an unrelated
    // source namespace; nested I/O errors remain available via Error::source.
    assert!(error.to_string().contains("JBIG2 refinement bitmap 0"));
    if matches!(
        error.kind,
        RefinementErrorKind::ReferenceSource(_)
            | RefinementErrorKind::Sink(_)
            | RefinementErrorKind::Mq(_)
    ) {
        assert!(std::error::Error::source(&error).is_some());
    } else {
        assert!(std::error::Error::source(&error).is_none());
    }
    assert!(error.progress.poisoned);
    assert!(host.progress().poisoned);
    assert!(matches!(
        host.mq_mut(),
        Err(RefinementError {
            kind: RefinementErrorKind::Poisoned,
            ..
        })
    ));
    let retry = host
        .decode_bitmap(&mut reference_source, request)
        .unwrap_err();
    assert!(matches!(retry.kind, RefinementErrorKind::Poisoned));
    assert!(retry.to_string().contains("poisoned"));
    drop(host);
    assert!(matches!(
        mq.decode_bit(BITMAP_BASE).unwrap_err().kind,
        ArithmeticErrorKind::Poisoned
    ));
    (error, reference_source, sink)
}

#[test]
fn unsupported_modes_and_zero_dimensions_fail_before_reference_or_sink_io() {
    let valid = request(1, 1, reference(1, 1));
    let variants = [
        RefinementRequest {
            template: 0,
            ..valid
        },
        RefinementRequest {
            typical_prediction: true,
            ..valid
        },
        RefinementRequest { width: 0, ..valid },
        RefinementRequest { height: 0, ..valid },
        RefinementRequest {
            reference: reference(0, 1),
            ..valid
        },
    ];
    for request in variants {
        let (error, source, sink) = observe_error(
            Source::new(&[0x80]),
            Sink::new(),
            request,
            RefinementBudget::default(),
            Limits::default(),
            &NeverCancel,
        );
        assert!(matches!(
            error.kind,
            RefinementErrorKind::Unsupported { .. }
        ));
        assert_eq!(source.calls, 0);
        assert_eq!(sink.calls, 0);
        assert_eq!(error.progress.pixels_decoded, 0);
    }
}

#[test]
fn forged_reference_descriptors_and_store_ranges_are_rejected_before_reads() {
    let mut malformed_stride = reference(9, 1);
    malformed_stride.symbol.row_stride = 1;
    let mut malformed_length = reference(1, 1);
    malformed_length.symbol.stored_bytes = 2;
    let mut overflow_base = reference(1, 1);
    overflow_base.store_base = u64::MAX;
    overflow_base.symbol.relative_store_offset = 1;
    let mut outside_source = reference(1, 1);
    outside_source.symbol.relative_store_offset = 1;
    let mut overflow_end = reference(1, 1);
    overflow_end.store_base = u64::MAX;
    for (reference, expect_malformed) in [
        (malformed_stride, true),
        (malformed_length, true),
        (overflow_base, false),
        (overflow_end, false),
        (outside_source, false),
    ] {
        let (error, source, sink) = observe_error(
            Source::new(&[0x80]),
            Sink::new(),
            request(1, 1, reference),
            RefinementBudget::default(),
            Limits::default(),
            &NeverCancel,
        );
        if expect_malformed {
            assert!(matches!(error.kind, RefinementErrorKind::Malformed(_)));
        } else {
            assert!(matches!(error.kind, RefinementErrorKind::InvalidSpan(_)));
        }
        assert_eq!(source.calls, 0);
        assert_eq!(sink.calls, 0);
    }
}

#[test]
fn constructor_rejects_invalid_limits_request_caps_and_layout() {
    for invalid in ["limits", "source cap", "sink cap", "short GR bank"] {
        let limits = Limits::default();
        let mq_budget = MqBudget::default();
        let table = table();
        let short = usize::from(invalid == "short GR bank");
        let mut contexts = mq_budget
            .context_bank(BITMAP_BASE + 1024 - short, &limits)
            .unwrap();
        let mut mq_source = Source::new(&[0x3f, 0xff, 0xac]);
        let mut mq = MqDecoder::new(
            &mut mq_source,
            CodedSpan {
                offset: 0,
                length: 3,
            },
            &table,
            &mut contexts,
            &limits,
            &NeverCancel,
            mq_budget,
        )
        .unwrap();
        let bad_limits = Limits {
            io_chunk_bytes: 0,
            ..limits
        };
        let host_limits = if invalid == "limits" {
            &bad_limits
        } else {
            &limits
        };
        let mut budget = RefinementBudget::default();
        if invalid == "source cap" {
            budget.max_source_request_bytes = 0;
        }
        if invalid == "sink cap" {
            budget.max_sink_request_bytes = 0;
        }
        let mut sink = Sink::new();
        let error =
            match RefinementDecoder::new(&mut mq, &mut sink, host_limits, &NeverCancel, budget) {
                Ok(_) => panic!("accepted invalid {invalid}"),
                Err(error) => error,
            };
        if invalid == "short GR bank" {
            assert!(matches!(error.kind, RefinementErrorKind::InvalidSpan(_)));
        } else {
            assert!(matches!(error.kind, RefinementErrorKind::Malformed(_)));
        }
        assert!(!error.progress.poisoned);
        assert!(error.to_string().contains("JBIG2 refinement bitmap"));
        // Rejected construction has not started a bitmap and leaves MQ usable.
        mq.decode_bit(BITMAP_BASE).unwrap();
    }
}

#[test]
fn constructor_rejects_an_already_poisoned_mq_coding_unit() {
    let limits = Limits::default();
    // Two prefetched bytes and the first byte-input consume three work units;
    // the first decision then fails before it can update any bitmap context.
    let mq_budget = MqBudget {
        max_work: 3,
        ..MqBudget::default()
    };
    let table = table();
    let mut contexts = contexts(&limits, &mq_budget);
    let mut mq_source = Source::new(&[0xff, 0xac]);
    let mut mq = MqDecoder::new(
        &mut mq_source,
        CodedSpan {
            offset: 0,
            length: 2,
        },
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        mq_budget,
    )
    .unwrap();
    assert!(matches!(
        mq.decode_bit(BITMAP_BASE).unwrap_err().kind,
        ArithmeticErrorKind::LimitExceeded {
            resource: "arithmetic work",
            ..
        }
    ));
    assert!(mq.snapshot().poisoned);
    let mut sink = Sink::new();
    let error = match RefinementDecoder::new(
        &mut mq,
        &mut sink,
        &limits,
        &NeverCancel,
        RefinementBudget::default(),
    ) {
        Ok(_) => panic!("accepted poisoned MQ stream"),
        Err(error) => error,
    };
    assert!(matches!(error.kind, RefinementErrorKind::Poisoned));
    assert_eq!(error.progress.completed_bitmaps, 0);
}

#[test]
fn reference_byte_limit_uses_the_host_limits_before_any_reference_io() {
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let table = table();
    let mut contexts = contexts(&limits, &mq_budget);
    let mut mq_source = Source::new(&[0xff, 0xac]);
    let mut mq = MqDecoder::new(
        &mut mq_source,
        CodedSpan {
            offset: 0,
            length: 2,
        },
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        mq_budget,
    )
    .unwrap();
    let host_limits = Limits {
        max_input_bytes: 0,
        ..limits
    };
    let mut reference_source = Source::new(&[0x80]);
    let mut sink = Sink::new();
    let mut host = RefinementDecoder::new(
        &mut mq,
        &mut sink,
        &host_limits,
        &NeverCancel,
        RefinementBudget::default(),
    )
    .unwrap();
    let error = host
        .decode_bitmap(&mut reference_source, request(1, 1, reference(1, 1)))
        .unwrap_err();
    assert!(matches!(
        error.kind,
        RefinementErrorKind::LimitExceeded {
            resource: "reference stored bytes",
            ..
        }
    ));
    assert_eq!(reference_source.calls, 0);
    assert!(error.progress.poisoned);
    drop(host);
    assert_eq!(sink.calls, 0);
    assert!(matches!(
        mq.decode_bit(BITMAP_BASE).unwrap_err().kind,
        ArithmeticErrorKind::Poisoned
    ));
}

#[test]
fn target_and_reference_row_allocation_limits_are_distinct() {
    let limits = Limits {
        io_chunk_bytes: 1024,
        max_allocation_bytes: 20_000,
        ..Limits::default()
    };
    let budget = RefinementBudget {
        max_width: 240_000,
        ..RefinementBudget::default()
    };
    let (target_error, source, sink) = observe_error(
        Source::new(&[0x80]),
        Sink::new(),
        request(240_000, 1, reference(1, 1)),
        budget,
        limits,
        &NeverCancel,
    );
    assert!(matches!(
        target_error.kind,
        RefinementErrorKind::LimitExceeded {
            resource: "target row allocation",
            ..
        }
    ));
    assert_eq!(source.calls, 0);
    assert_eq!(sink.calls, 0);

    let budget = RefinementBudget {
        max_reference_width: 240_000,
        ..RefinementBudget::default()
    };
    let (reference_error, source, sink) = observe_error(
        Source::new(&vec![0; 30_000]),
        Sink::new(),
        request(1, 1, reference(240_000, 1)),
        budget,
        limits,
        &NeverCancel,
    );
    assert!(matches!(
        reference_error.kind,
        RefinementErrorKind::LimitExceeded {
            resource: "reference row allocation",
            ..
        }
    ));
    assert_eq!(source.calls, 0);
    assert_eq!(sink.calls, 0);
}

#[test]
fn mq_marker_failure_keeps_its_actual_byte_offset_and_poisoned_host() {
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let table = table();
    let mut contexts = contexts(&limits, &mq_budget);
    // Initial bytes are legal. FF followed by 90 is forbidden once byte-in
    // reaches that position; the final FF AC remains a separate tail.
    let mut mq_source = Source::new(&[0x00, 0x00, 0xff, 0x90, 0xff, 0xac]);
    let mut mq = MqDecoder::new(
        &mut mq_source,
        CodedSpan {
            offset: 0,
            length: 6,
        },
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        mq_budget,
    )
    .unwrap();
    let mut reference_source = Source::new(&[0x80]);
    let mut sink = Sink::new();
    let mut host = RefinementDecoder::new(
        &mut mq,
        &mut sink,
        &limits,
        &NeverCancel,
        RefinementBudget::default(),
    )
    .unwrap();
    let error = host
        .decode_bitmap(&mut reference_source, request(128, 1, reference(1, 1)))
        .unwrap_err();
    match &error.kind {
        RefinementErrorKind::Mq(inner) => {
            assert!(matches!(
                inner.kind,
                ArithmeticErrorKind::InvalidMarker(0x90)
            ));
            assert_eq!(inner.offset, Some(3));
            assert_eq!(error.offset, inner.offset);
        }
        other => panic!("expected MQ marker error, got {other:?}"),
    }
    assert!(error.to_string().contains("source byte 3"));
    assert!(std::error::Error::source(&error).is_some());
    assert!(error.progress.poisoned);
    drop(host);
    assert_eq!(sink.calls, 0);
    assert!(matches!(
        mq.decode_bit(BITMAP_BASE).unwrap_err().kind,
        ArithmeticErrorKind::Poisoned
    ));
}

#[test]
fn per_bitmap_and_cumulative_caps_are_checked_before_io() {
    let cases: &[(RefinementBudget, &str)] = &[
        (
            RefinementBudget {
                max_width: 0,
                ..RefinementBudget::default()
            },
            "target width",
        ),
        (
            RefinementBudget {
                max_height: 0,
                ..RefinementBudget::default()
            },
            "target height",
        ),
        (
            RefinementBudget {
                max_reference_width: 0,
                ..RefinementBudget::default()
            },
            "reference width",
        ),
        (
            RefinementBudget {
                max_reference_height: 0,
                ..RefinementBudget::default()
            },
            "reference height",
        ),
        (
            RefinementBudget {
                max_reference_pixels_per_bitmap: 0,
                ..RefinementBudget::default()
            },
            "reference pixels per bitmap",
        ),
        (
            RefinementBudget {
                max_reference_bytes_per_bitmap: 0,
                ..RefinementBudget::default()
            },
            "reference bytes per bitmap",
        ),
        (
            RefinementBudget {
                max_pixels_per_bitmap: 0,
                ..RefinementBudget::default()
            },
            "pixels per bitmap",
        ),
        (
            RefinementBudget {
                max_bytes_per_bitmap: 0,
                ..RefinementBudget::default()
            },
            "bytes per bitmap",
        ),
        (
            RefinementBudget {
                max_total_pixels: 0,
                ..RefinementBudget::default()
            },
            "total pixels",
        ),
        (
            RefinementBudget {
                max_total_output_bytes: 0,
                ..RefinementBudget::default()
            },
            "total output bytes",
        ),
        (
            RefinementBudget {
                max_mq_decisions: 0,
                ..RefinementBudget::default()
            },
            "MQ decisions",
        ),
        (
            RefinementBudget {
                max_context_work: 0,
                ..RefinementBudget::default()
            },
            "context work",
        ),
        (
            RefinementBudget {
                max_working_bytes: 0,
                ..RefinementBudget::default()
            },
            "working bytes",
        ),
    ];
    for &(budget, expected) in cases {
        let (error, source, sink) = observe_error(
            Source::new(&[0x80]),
            Sink::new(),
            request(1, 1, reference(1, 1)),
            budget,
            Limits::default(),
            &NeverCancel,
        );
        assert!(
            matches!(error.kind, RefinementErrorKind::LimitExceeded { resource, .. } if resource == expected),
            "{error}"
        );
        assert_eq!(source.calls, 0);
        assert_eq!(sink.calls, 0);
    }
}

#[test]
fn reference_read_and_output_write_caps_stop_before_the_next_io_call() {
    for cap_reference_bytes in [false, true] {
        let mut source = Source::new(&[0x80, 0x80]);
        source.max_read = 1;
        let mut budget = RefinementBudget {
            max_source_request_bytes: 1,
            ..RefinementBudget::default()
        };
        if cap_reference_bytes {
            budget.max_reference_bytes_fetched = 1;
        } else {
            budget.max_reference_reads = 1;
        }
        let (error, source, sink) = observe_error(
            source,
            Sink::new(),
            request(1, 1, reference(9, 1)),
            budget,
            Limits::default(),
            &NeverCancel,
        );
        assert!(
            matches!(error.kind, RefinementErrorKind::LimitExceeded { resource, .. }
            if resource == if cap_reference_bytes { "reference bytes" } else { "reference reads" })
        );
        assert_eq!(source.calls, 1);
        assert_eq!(error.progress.reference_reads, 1);
        assert_eq!(error.progress.reference_bytes_fetched, 1);
        assert_eq!(sink.calls, 0);
    }

    let mut sink = Sink::new();
    sink.max_write = 1;
    let budget = RefinementBudget {
        max_sink_writes: 1,
        max_sink_request_bytes: 1,
        ..RefinementBudget::default()
    };
    let (error, _, sink) = observe_error(
        Source::new(&[0x80]),
        sink,
        request(9, 1, reference(1, 1)),
        budget,
        Limits::default(),
        &NeverCancel,
    );
    assert!(matches!(
        error.kind,
        RefinementErrorKind::LimitExceeded {
            resource: "sink writes",
            ..
        }
    ));
    assert_eq!(error.progress.pixels_decoded, 9);
    assert_eq!(error.progress.sink_writes, 1);
    assert_eq!(error.progress.output_bytes_written, 1);
    assert_eq!(sink.calls, 1);
    assert_eq!(sink.bytes, [0xff]);
}

#[test]
fn cumulative_pixel_and_output_caps_apply_to_the_next_bitmap() {
    for cap_pixels in [false, true] {
        let limits = Limits::default();
        let mq_budget = MqBudget::default();
        let table = table();
        let mut contexts = contexts(&limits, &mq_budget);
        let mut mq_source = Source::new(&[0xbf, 0xff, 0xac]);
        let mut mq = MqDecoder::new(
            &mut mq_source,
            CodedSpan {
                offset: 0,
                length: 3,
            },
            &table,
            &mut contexts,
            &limits,
            &NeverCancel,
            mq_budget,
        )
        .unwrap();
        let mut reference_source = Source::new(&[0x80]);
        let mut sink = Sink::new();
        let mut budget = RefinementBudget::default();
        if cap_pixels {
            budget.max_total_pixels = 1;
        } else {
            budget.max_total_output_bytes = 1;
        }
        let mut host =
            RefinementDecoder::new(&mut mq, &mut sink, &limits, &NeverCancel, budget).unwrap();
        host.decode_bitmap(&mut reference_source, request(1, 1, reference(1, 1)))
            .unwrap();
        let error = host
            .decode_bitmap(&mut reference_source, request(1, 1, reference(1, 1)))
            .unwrap_err();
        assert!(
            matches!(error.kind, RefinementErrorKind::LimitExceeded { resource, .. }
            if resource == if cap_pixels { "total pixels" } else { "total output bytes" })
        );
        assert_eq!(error.progress.completed_bitmaps, 1);
        assert_eq!(error.progress.pixels_decoded, 1);
        assert_eq!(error.progress.output_bytes_written, 1);
        assert!(error.progress.poisoned);
        drop(host);
        assert_eq!(sink.bytes, [0x80]);
        assert!(matches!(
            mq.decode_bit(BITMAP_BASE).unwrap_err().kind,
            ArithmeticErrorKind::Poisoned
        ));
    }
}

#[test]
fn short_reference_read_then_zero_reports_exact_physical_progress() {
    let mut source = Source::new(&[0x80]);
    source.advertised = Some(2);
    source.max_read = 1;
    source.zero_at = Some(1);
    let (error, source, sink) = observe_error(
        source,
        Sink::new(),
        request(1, 1, reference(9, 1)),
        RefinementBudget::default(),
        Limits::default(),
        &NeverCancel,
    );
    assert!(matches!(
        error.kind,
        RefinementErrorKind::TruncatedReference
    ));
    assert_eq!(error.offset, Some(1));
    assert_eq!(error.progress.reference_reads, 2);
    assert_eq!(error.progress.reference_bytes_fetched, 1);
    assert_eq!(source.calls, 2);
    assert_eq!(sink.calls, 0);
}

#[test]
fn reference_error_and_overreport_preserve_read_location_and_progress() {
    for overreport in [false, true] {
        let mut source = Source::new(&[0x80, 0x80]);
        source.max_read = 1;
        if overreport {
            source.overreport_at = Some(1);
        } else {
            source.error_at = Some(1);
        }
        let (error, source, sink) = observe_error(
            source,
            Sink::new(),
            request(1, 1, reference(9, 1)),
            RefinementBudget::default(),
            Limits::default(),
            &NeverCancel,
        );
        assert!(matches!(
            error.kind,
            RefinementErrorKind::ReferenceSource(_)
        ));
        assert_eq!(error.offset, Some(1));
        assert_eq!(error.progress.reference_reads, 2);
        assert_eq!(error.progress.reference_bytes_fetched, 1);
        assert_eq!(source.calls, 2);
        assert_eq!(sink.calls, 0);
    }
}

#[test]
fn partial_output_before_sink_failure_is_counted_and_poisoned() {
    for failure in ["zero", "error", "overreport"] {
        let mut sink = Sink::new();
        sink.max_write = 1;
        match failure {
            "zero" => sink.zero_on_call = Some(2),
            "error" => sink.error_on_call = Some(2),
            "overreport" => sink.overreport_on_call = Some(2),
            _ => unreachable!(),
        }
        let (error, source, sink) = observe_error(
            Source::new(&[0x80]),
            sink,
            request(9, 1, reference(1, 1)),
            RefinementBudget::default(),
            Limits::default(),
            &NeverCancel,
        );
        assert!(matches!(error.kind, RefinementErrorKind::Sink(_)));
        assert_eq!(error.progress.pixels_decoded, 9);
        assert_eq!(error.progress.rows_written, 0);
        assert_eq!(error.progress.output_bytes_written, 1);
        assert_eq!(error.progress.sink_writes, 2);
        assert_eq!(sink.bytes, [0xff]);
        assert_eq!(source.calls, 1);
    }
}

#[test]
fn cancellation_after_a_reference_read_counts_that_read() {
    let cancelled = Rc::new(Cell::new(false));
    let signal = Flag(cancelled.clone());
    let mut source = Source::new(&[0x80, 0x80]);
    source.max_read = 1;
    source.cancel_after_read = Some(cancelled);
    let (error, source, sink) = observe_error(
        source,
        Sink::new(),
        request(1, 1, reference(9, 1)),
        RefinementBudget::default(),
        Limits::default(),
        &signal,
    );
    assert!(matches!(error.kind, RefinementErrorKind::Cancelled));
    assert_eq!(error.progress.reference_reads, 1);
    assert_eq!(error.progress.reference_bytes_fetched, 1);
    assert_eq!(source.calls, 1);
    assert_eq!(sink.calls, 0);
}

#[test]
fn cancellation_after_a_partial_sink_write_counts_that_write() {
    let cancelled = Rc::new(Cell::new(false));
    let signal = Flag(cancelled.clone());
    let mut sink = Sink::new();
    sink.max_write = 1;
    sink.cancel_after_write = Some(cancelled);
    let (error, _, sink) = observe_error(
        Source::new(&[0x80]),
        sink,
        request(9, 1, reference(1, 1)),
        RefinementBudget::default(),
        Limits::default(),
        &signal,
    );
    assert!(matches!(error.kind, RefinementErrorKind::Cancelled));
    assert_eq!(error.progress.output_bytes_written, 1);
    assert_eq!(error.progress.sink_writes, 1);
    assert_eq!(sink.bytes, [0xff]);
}

#[test]
fn failed_or_capped_flush_poisoned_the_session_without_losing_output_progress() {
    for capped in [false, true] {
        let limits = Limits::default();
        let mq_budget = MqBudget::default();
        let table = table();
        let mut contexts = contexts(&limits, &mq_budget);
        let mut mq_source = Source::new(&[0xbf, 0xff, 0xac]);
        let mut mq = MqDecoder::new(
            &mut mq_source,
            CodedSpan {
                offset: 0,
                length: 3,
            },
            &table,
            &mut contexts,
            &limits,
            &NeverCancel,
            mq_budget,
        )
        .unwrap();
        let mut reference_source = Source::new(&[0x80]);
        let mut sink = Sink::new();
        sink.flush_error = !capped;
        let budget = RefinementBudget {
            max_flushes: 1,
            ..RefinementBudget::default()
        };
        let mut host =
            RefinementDecoder::new(&mut mq, &mut sink, &limits, &NeverCancel, budget).unwrap();
        host.decode_bitmap(&mut reference_source, request(1, 1, reference(1, 1)))
            .unwrap();
        if capped {
            host.flush_store().unwrap();
            assert_eq!(host.progress().flushes, 1);
        }
        let error = host.flush_store().unwrap_err();
        if capped {
            assert!(matches!(
                error.kind,
                RefinementErrorKind::LimitExceeded {
                    resource: "store flushes",
                    ..
                }
            ));
            assert!(
                error
                    .to_string()
                    .contains("store flushes limit 1 exceeded by 2")
            );
            assert_eq!(error.progress.flushes, 1);
        } else {
            assert!(matches!(error.kind, RefinementErrorKind::Sink(_)));
            assert_eq!(error.progress.flushes, 1);
            assert!(std::error::Error::source(&error).is_some());
        }
        assert_eq!(error.progress.completed_bitmaps, 1);
        assert_eq!(error.progress.output_bytes_written, 1);
        assert!(error.progress.poisoned);
        // A failed or over-budget flush cannot be repeated on this session.
        assert!(matches!(
            host.flush_store().unwrap_err().kind,
            RefinementErrorKind::Poisoned
        ));
        assert!(matches!(
            host.mq_mut(),
            Err(RefinementError {
                kind: RefinementErrorKind::Poisoned,
                ..
            })
        ));
        drop(host);
        assert_eq!(sink.bytes, [0x80]);
        assert_eq!(sink.flush_calls, 1);
        assert!(matches!(
            mq.decode_bit(BITMAP_BASE).unwrap_err().kind,
            ArithmeticErrorKind::Poisoned
        ));
    }
}

#[test]
fn cancellation_after_completed_flush_poisoned_the_coding_unit() {
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let table = table();
    let mut contexts = contexts(&limits, &mq_budget);
    let signal = Rc::new(Cell::new(false));
    let cancellation = Flag(signal.clone());
    let mut mq_source = Source::new(&[0xff, 0xac]);
    let mut mq = MqDecoder::new(
        &mut mq_source,
        CodedSpan {
            offset: 0,
            length: 2,
        },
        &table,
        &mut contexts,
        &limits,
        &cancellation,
        mq_budget,
    )
    .unwrap();
    let mut sink = Sink::new();
    sink.cancel_after_flush = Some(signal);
    let mut host = RefinementDecoder::new(
        &mut mq,
        &mut sink,
        &limits,
        &cancellation,
        RefinementBudget::default(),
    )
    .unwrap();
    let error = host.flush_store().unwrap_err();
    assert!(matches!(error.kind, RefinementErrorKind::Cancelled));
    assert_eq!(error.progress.flushes, 1);
    assert!(error.progress.poisoned);
    drop(host);
    assert_eq!(sink.flush_calls, 1);
    assert!(matches!(
        mq.decode_bit(BITMAP_BASE).unwrap_err().kind,
        ArithmeticErrorKind::Poisoned
    ));
}

/// Every cap at its most permissive accepted value: dimensions and
/// non-counter fields unbounded, counter fields at `MAX_BUDGET_COUNT`.
fn ceiling_budget() -> RefinementBudget {
    RefinementBudget {
        max_width: u32::MAX,
        max_height: u32::MAX,
        max_reference_width: u32::MAX,
        max_reference_height: u32::MAX,
        max_reference_pixels_per_bitmap: u64::MAX,
        max_reference_bytes_per_bitmap: u64::MAX,
        max_pixels_per_bitmap: MAX_BUDGET_COUNT,
        max_total_pixels: MAX_BUDGET_COUNT,
        max_bytes_per_bitmap: u64::MAX,
        max_total_output_bytes: MAX_BUDGET_COUNT,
        max_reference_reads: MAX_BUDGET_COUNT,
        max_reference_bytes_fetched: MAX_BUDGET_COUNT,
        max_sink_writes: MAX_BUDGET_COUNT,
        max_flushes: MAX_BUDGET_COUNT,
        max_source_request_bytes: usize::MAX,
        max_sink_request_bytes: usize::MAX,
        max_mq_decisions: u64::MAX,
        max_context_work: u64::MAX,
        max_working_bytes: u64::MAX,
    }
}

#[test]
fn constructor_rejects_counter_budgets_above_the_ceiling() {
    type Field = fn(&mut RefinementBudget) -> &mut u64;
    let fields: [(&str, Field); 7] = [
        ("pixels per bitmap budget", |b| &mut b.max_pixels_per_bitmap),
        ("total pixels budget", |b| &mut b.max_total_pixels),
        ("total output bytes budget", |b| {
            &mut b.max_total_output_bytes
        }),
        ("reference reads budget", |b| &mut b.max_reference_reads),
        ("reference bytes budget", |b| {
            &mut b.max_reference_bytes_fetched
        }),
        ("sink writes budget", |b| &mut b.max_sink_writes),
        ("store flushes budget", |b| &mut b.max_flushes),
    ];
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let table = table();
    for (name, field) in fields {
        let mut contexts = contexts(&limits, &mq_budget);
        let mut mq_source = Source::new(&[0xbf, 0xff, 0xac]);
        let mut mq = MqDecoder::new(
            &mut mq_source,
            CodedSpan {
                offset: 0,
                length: 3,
            },
            &table,
            &mut contexts,
            &limits,
            &NeverCancel,
            mq_budget,
        )
        .unwrap();
        let mut sink = Sink::new();
        let mut budget = ceiling_budget();
        *field(&mut budget) = MAX_BUDGET_COUNT + 1;
        let error = match RefinementDecoder::new(&mut mq, &mut sink, &limits, &NeverCancel, budget)
        {
            Ok(_) => panic!("accepted {name} above the ceiling"),
            Err(error) => error,
        };
        assert!(
            matches!(
                error.kind,
                RefinementErrorKind::LimitExceeded {
                    resource,
                    limit: MAX_BUDGET_COUNT,
                    attempted,
                } if resource == name && attempted == MAX_BUDGET_COUNT + 1
            ),
            "{error}"
        );
        assert_eq!((error.offset, error.row, error.x), (None, 0, 0));
        assert!(!error.progress.poisoned);
        assert_eq!(sink.calls, 0);
        // Rejected construction leaves MQ usable.
        mq.decode_bit(BITMAP_BASE).unwrap();
    }
}

#[test]
fn maximal_geometry_under_ceiling_caps_is_refused_before_io() {
    // (2^32 - 1)^2 pixels exceed the per-bitmap pixel ceiling, which is the
    // cap that now keeps ten context probes per pixel within u64.
    let limits = Limits {
        max_output_bytes: u64::MAX,
        ..Limits::default()
    };
    let (error, source, sink) = observe_error(
        Source::new(&[0x80]),
        Sink::new(),
        request(u32::MAX, u32::MAX, reference(1, 1)),
        ceiling_budget(),
        limits,
        &NeverCancel,
    );
    assert!(
        matches!(
            error.kind,
            RefinementErrorKind::LimitExceeded {
                resource: "pixels per bitmap",
                limit: MAX_BUDGET_COUNT,
                attempted,
            } if attempted == u64::from(u32::MAX) * u64::from(u32::MAX)
        ),
        "{error}"
    );
    assert_eq!((error.row, error.x, error.offset), (0, 0, None));
    assert_eq!(source.calls, 0);
    assert_eq!(sink.calls, 0);
    assert_eq!(error.progress.pixels_decoded, 0);
}

#[test]
fn allocation_failure_message_and_formatter_errors_are_reported() {
    // Row reservation failure needs a real allocator failure; the message is
    // still part of the public error contract.
    let error = RefinementError {
        offset: Some(7),
        bitmap_index: 2,
        row: 3,
        x: 4,
        progress: Box::default(),
        kind: RefinementErrorKind::AllocationFailed,
    };
    assert_eq!(
        error.to_string(),
        "JBIG2 refinement bitmap 2 row 3 x 4 at source byte 7: row allocation failed"
    );
    assert!(std::error::Error::source(&error).is_none());
    common::assert_display_propagates_fmt_error(&error);
}
