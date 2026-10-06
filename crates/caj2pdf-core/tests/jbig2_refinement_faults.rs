// SPDX-License-Identifier: MIT

//! Adversarial tests of the public refinement host over in-memory stores.

mod common;

use caj2pdf_core::{
    Cancellation, Limits, NeverCancel, Payload,
    jbig2::{
        dictionary::SymbolDescriptor,
        integer::BITMAP_BASE,
        mq::{ArithmeticErrorKind, CodedSpan, ContextBank, MqDecoder, MqTable},
        refinement::{
            ReferenceStore, RefinementDecoder, RefinementError, RefinementErrorKind,
            RefinementReference, RefinementRequest,
        },
    },
};
use std::{cell::Cell, rc::Rc};

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
fn contexts(limits: &Limits) -> ContextBank {
    caj2pdf_core::jbig2::mq::context_bank(BITMAP_BASE + 1024, limits).unwrap()
}

fn span(bytes: &[u8]) -> CodedSpan {
    CodedSpan {
        offset: 0,
        length: bytes.len() as u64,
    }
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

/// Decode one refinement of `request` over `reference` and return its error
/// and the output store.
fn observe_error<C: Cancellation>(
    reference: &[u8],
    request: RefinementRequest,
    limits: Limits,
    cancellation: &C,
) -> (RefinementError, Vec<u8>) {
    let table = table();
    let mut contexts = contexts(&Limits::default());
    let bytes = set_stream(&request, reference);
    let mut mq = MqDecoder::new(
        Payload::from(&bytes[..]),
        span(&bytes),
        &table,
        &mut contexts,
        &Limits::default(),
    )
    .unwrap();
    let mut output = Vec::new();
    let mut host = RefinementDecoder::new(&mut mq, &mut output, &limits, cancellation).unwrap();
    let error = host
        .decode_bitmap(ReferenceStore::Other(reference), request)
        .unwrap_err();
    // Diagnostics identify the failed bitmap; nested MQ errors remain
    // available via Error::source.
    assert!(error.to_string().contains("JBIG2 refinement bitmap 0"));
    assert_eq!(
        std::error::Error::source(&error).is_some(),
        matches!(error.kind, RefinementErrorKind::Mq(_))
    );
    (error, output)
}

#[test]
fn unsupported_modes_and_zero_dimensions_fail_before_decoding() {
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
        let (error, output) = observe_error(&[0x80], request, Limits::default(), &NeverCancel);
        assert!(matches!(
            error.kind,
            RefinementErrorKind::Unsupported { .. }
        ));
        assert!(output.is_empty());
        assert_eq!(error.progress.pixels_decoded, 0);
    }
}

#[test]
fn forged_reference_descriptors_and_store_ranges_are_rejected_before_decoding() {
    let mut malformed_stride = reference(9, 1);
    malformed_stride.symbol.row_stride = 1;
    let mut malformed_length = reference(1, 1);
    malformed_length.symbol.stored_bytes = 2;
    let mut overflow_base = reference(1, 1);
    overflow_base.store_base = u64::MAX;
    overflow_base.symbol.relative_store_offset = 1;
    let mut outside_store = reference(1, 1);
    outside_store.symbol.relative_store_offset = 1;
    let mut overflow_end = reference(1, 1);
    overflow_end.store_base = u64::MAX;
    for (reference, expect_malformed) in [
        (malformed_stride, true),
        (malformed_length, true),
        (overflow_base, false),
        (overflow_end, false),
        (outside_store, false),
    ] {
        let (error, output) = observe_error(
            &[0x80],
            request(1, 1, reference),
            Limits::default(),
            &NeverCancel,
        );
        if expect_malformed {
            assert!(matches!(error.kind, RefinementErrorKind::Malformed(_)));
        } else {
            assert!(matches!(error.kind, RefinementErrorKind::InvalidSpan(_)));
        }
        assert!(output.is_empty());
        assert_eq!(error.progress.pixels_decoded, 0);
    }
}

#[test]
fn constructor_rejects_a_bank_without_the_gr_range() {
    let limits = Limits::default();
    let table = table();
    let mut contexts = caj2pdf_core::jbig2::mq::context_bank(BITMAP_BASE + 1023, &limits).unwrap();
    let bytes = [0x3f, 0xff, 0xac];
    let mut mq = MqDecoder::new(
        Payload::from(&bytes[..]),
        span(&bytes),
        &table,
        &mut contexts,
        &limits,
    )
    .unwrap();
    let mut output = Vec::new();
    let error = match RefinementDecoder::new(&mut mq, &mut output, &limits, &NeverCancel) {
        Ok(_) => panic!("accepted a short GR bank"),
        Err(error) => error,
    };
    assert!(matches!(error.kind, RefinementErrorKind::InvalidSpan(_)));
    assert!(error.to_string().contains("JBIG2 refinement bitmap"));
    // Rejected construction has not started a bitmap and leaves MQ usable.
    mq.decode_bit(BITMAP_BASE).unwrap();
}

#[test]
fn target_and_reference_row_allocation_limits_are_distinct() {
    let limits = Limits {
        io_chunk_bytes: 1024,
        max_allocation_bytes: 20_000,
        ..Limits::default()
    };
    let (target_error, output) = observe_error(
        &[0x80],
        request(240_000, 1, reference(1, 1)),
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
    assert!(output.is_empty());

    let (reference_error, output) = observe_error(
        &vec![0; 30_000],
        request(1, 1, reference(240_000, 1)),
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
    assert!(output.is_empty());
}

#[test]
fn mq_marker_failure_keeps_its_actual_byte_offset() {
    let limits = Limits::default();
    let table = table();
    let mut contexts = contexts(&limits);
    // Initial bytes are legal. FF followed by 90 is forbidden once byte-in
    // reaches that position; the final FF AC remains a separate tail.
    let bytes = [0x00, 0x00, 0xff, 0x90, 0xff, 0xac];
    let mut mq = MqDecoder::new(
        Payload::from(&bytes[..]),
        span(&bytes),
        &table,
        &mut contexts,
        &limits,
    )
    .unwrap();
    let mut output = Vec::new();
    let mut host = RefinementDecoder::new(&mut mq, &mut output, &limits, &NeverCancel).unwrap();
    let error = host
        .decode_bitmap(
            ReferenceStore::Other(&[0x80]),
            request(128, 1, reference(1, 1)),
        )
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
    assert!(output.is_empty());
}

#[test]
fn the_pixel_limit_is_checked_before_decoding() {
    let limits = Limits {
        max_image_pixels: 0,
        ..Limits::default()
    };
    let (error, output) = observe_error(
        &[0x80],
        request(1, 1, reference(1, 1)),
        limits,
        &NeverCancel,
    );
    assert!(
        matches!(
            error.kind,
            RefinementErrorKind::LimitExceeded {
                resource: "pixels per bitmap",
                limit: 0,
                attempted: 1,
            }
        ),
        "{error}"
    );
    assert!(output.is_empty());
}

#[test]
fn the_store_allocation_limit_applies_to_the_next_bitmap() {
    let limits = Limits::default();
    let table = table();
    let mut contexts = contexts(&limits);
    let bytes = [0xbf, 0xff, 0xac];
    let mut mq = MqDecoder::new(
        Payload::from(&bytes[..]),
        span(&bytes),
        &table,
        &mut contexts,
        &limits,
    )
    .unwrap();
    let reference_store = [0x80];
    let mut output = Vec::new();
    let host_limits = Limits {
        io_chunk_bytes: 1,
        max_allocation_bytes: 1,
        ..limits
    };
    let mut host =
        RefinementDecoder::new(&mut mq, &mut output, &host_limits, &NeverCancel).unwrap();
    host.decode_bitmap(
        ReferenceStore::Other(&reference_store),
        request(1, 1, reference(1, 1)),
    )
    .unwrap();
    let error = host
        .decode_bitmap(
            ReferenceStore::Other(&reference_store),
            request(1, 1, reference(1, 1)),
        )
        .unwrap_err();
    assert!(matches!(
        error.kind,
        RefinementErrorKind::LimitExceeded {
            resource: "refinement store bytes",
            limit: 1,
            attempted: 2,
        }
    ));
    assert_eq!(error.progress.completed_bitmaps, 1);
    assert_eq!(error.progress.pixels_decoded, 1);
    assert_eq!(error.progress.output_bytes_written, 1);
    assert_eq!(output, [0x80]);
}

#[test]
fn cancellation_is_checked_between_rows() {
    let cancelled = Rc::new(Cell::new(true));
    let signal = Flag(cancelled);
    let (error, output) = observe_error(
        &[0x80],
        request(9, 2, reference(1, 1)),
        Limits::default(),
        &signal,
    );
    assert!(matches!(error.kind, RefinementErrorKind::Cancelled));
    assert_eq!(error.progress.rows_written, 0);
    assert!(output.is_empty());
}

#[test]
fn maximal_geometry_is_refused_by_the_pixel_limit_before_decoding() {
    let limits = Limits {
        max_output_bytes: u64::MAX,
        ..Limits::default()
    };
    let (error, output) = observe_error(
        &[0x80],
        request(u32::MAX, u32::MAX, reference(1, 1)),
        limits,
        &NeverCancel,
    );
    assert!(
        matches!(
            error.kind,
            RefinementErrorKind::LimitExceeded {
                resource: "pixels per bitmap",
                limit: 12_000_000,
                attempted,
            } if attempted == u64::from(u32::MAX) * u64::from(u32::MAX)
        ),
        "{error}"
    );
    assert_eq!((error.row, error.x, error.offset), (0, 0, None));
    assert!(output.is_empty());
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
