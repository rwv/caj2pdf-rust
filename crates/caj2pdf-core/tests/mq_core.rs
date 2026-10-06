// SPDX-License-Identifier: MIT

//! Original small examples for the T.88 MQ control-flow API. The register
//! traces are hand-derived from the standard T.88 Table E.1 states.

use caj2pdf_core::{
    Error, Limits, Payload,
    jbig2::mq::{
        ArithmeticErrorKind, CodedSpan, ContextBank, ContextState, MqDecoder, MqTable, context_bank,
    },
};

fn table() -> MqTable {
    MqTable::standard()
}

fn bank(limits: &Limits) -> ContextBank {
    context_bank(1, limits).unwrap()
}

#[test]
fn context_banks_beyond_the_address_space_are_refused_without_allocating() {
    let limits = Limits {
        max_allocation_bytes: u64::MAX,
        ..Limits::default()
    };
    // The byte size of this bank overflows `usize`.
    assert!(matches!(
        context_bank(usize::MAX, &limits).unwrap_err().kind,
        ArithmeticErrorKind::InvalidContext
    ));
    // This byte size fits `usize` but exceeds `isize::MAX`, so the fallible
    // reservation fails before the allocator is called.
    #[cfg(target_pointer_width = "64")]
    assert!(matches!(
        context_bank(1 << 62, &limits).unwrap_err().kind,
        ArithmeticErrorKind::AllocationFailed
    ));
}

fn span(bytes: &[u8]) -> CodedSpan {
    CodedSpan {
        offset: 0,
        length: bytes.len() as u64,
    }
}

/// Decode `count` decisions in context zero, recording each bit with the
/// registers and the context state after it.
fn register_trace(bytes: &[u8], count: usize) -> Vec<(bool, u32, u32, u8, ContextState)> {
    let limits = Limits::default();
    let source: &[u8] = bytes;
    let state_table = table();
    let mut contexts = bank(&limits);
    let selected = span(source);
    let mut decoder = MqDecoder::new(
        Payload::from(source),
        selected,
        &state_table,
        &mut contexts,
        &limits,
    )
    .unwrap();
    assert_eq!(decoder.snapshot().interval, 0x8000);
    let mut trace = Vec::new();
    for _ in 0..count {
        let bit = decoder.decode_bit(0).unwrap();
        let progress = decoder.snapshot();
        trace.push((
            bit,
            progress.interval,
            progress.code,
            progress.bit_counter,
            decoder.context(0).unwrap(),
        ));
    }
    assert_eq!(decoder.snapshot().symbols_decoded, count as u64);
    decoder.finish(count as u64).unwrap();
    trace
}

fn state(state_index: u8, mps: bool) -> ContextState {
    ContextState { state_index, mps }
}

#[test]
fn initialization_mps_and_lps_exchanges_have_independent_register_expectations() {
    // Table E.1: state 0 has Qe 0x5601, NMPS = NLPS = 1 and SWITCH; state 1
    // has Qe 0x3401, NMPS 2, NLPS 6; state 2 has Qe 0x1801.
    // C = 0: A - Qe = 0x29FF < Qe exchanges the lower subinterval [0, Qe)
    // to the MPS. The second decision splits A = 0xAC02 normally, so C in
    // [0, Qe) is an LPS; its renormalization reads the terminal FF.
    assert_eq!(
        register_trace(&[0x00, 0x00, 0xff, 0xac], 2),
        [
            (false, 0xac02, 0, 0, state(1, false)),
            (true, 0xd004, 0x0003_fc00, 6, state(6, false)),
        ]
    );
    // C high 0x6000 lies in the upper subinterval, exchanged to the LPS,
    // which also switches the MPS.
    assert_eq!(
        register_trace(&[0xc0, 0x00, 0xff, 0xac], 1),
        [(true, 0xa7fc, 0x27fd_fe00, 7, state(1, true))]
    );
    // C high 0x2080: an exchanged MPS; then an ordinary MPS in the upper
    // subinterval with one renormalization shift; then, with A - Qe =
    // 0xD801 >= 0x8000, an MPS that neither renormalizes nor changes state.
    assert_eq!(
        register_trace(&[0x41, 0x00, 0xff, 0xac], 3),
        [
            (false, 0xac02, 0x4100_0000, 0, state(1, false)),
            (false, 0xf002, 0x19ff_fe00, 7, state(2, false)),
            (false, 0xd801, 0x01fe_fe00, 7, state(2, false)),
        ]
    );
}

#[test]
fn normal_and_post_ff_input_have_distinct_initial_c_and_ct() {
    let limits = Limits::default();
    let state_table = table();
    let source: &[u8] = &[0xff, 0x7f, 0x00, 0xff, 0xac];
    let mut contexts = bank(&limits);
    let selected = span(source);
    let mut decoder = MqDecoder::new(
        Payload::from(source),
        selected,
        &state_table,
        &mut contexts,
        &limits,
    )
    .unwrap();
    let initial = decoder.snapshot();
    assert_eq!(
        (initial.interval, initial.code, initial.bit_counter),
        (0x8000, 0x7fff_0000, 0)
    );
    assert_eq!(initial.input_offset, 1);
    // C high 0x7FFF is an exchanged LPS; two renormalization shifts read
    // the byte after the stuffed one as eight fresh bits.
    assert!(decoder.decode_bit(0).unwrap());
    let after = decoder.snapshot();
    assert_eq!(after.input_offset, 2);
    assert_eq!(after.bit_counter, 6);
    decoder.finish(1).unwrap();
}

#[test]
fn terminal_input_is_counted_and_marker_is_bounded() {
    let limits = Limits::default();
    let state_table = table();
    let source: &[u8] = &[0x00, 0xff, 0xac];
    let mut contexts = bank(&limits);
    let selected = span(source);
    let mut decoder = MqDecoder::new(
        Payload::from(source),
        selected,
        &state_table,
        &mut contexts,
        &limits,
    )
    .unwrap();
    assert!(!decoder.decode_bit(0).unwrap());
    assert!(decoder.decode_bit(0).unwrap());
    let progress = decoder.snapshot();
    assert_eq!(progress.input_offset, 1);
    assert_eq!(progress.synthesized_inputs, 1);
    decoder.finish(2).unwrap();

    let malformed: &[u8] = &[0xff, 0x90];
    let mut contexts = bank(&limits);
    let error = MqDecoder::new(
        Payload::from(malformed),
        CodedSpan {
            offset: 0,
            length: 2,
        },
        &state_table,
        &mut contexts,
        &limits,
    )
    .err()
    .expect("malformed marker must fail");
    assert!(matches!(
        error.kind,
        ArithmeticErrorKind::InvalidMarker(0x90)
    ));
    assert_eq!(error.offset, Some(1));
}

#[test]
fn spans_beyond_the_payload_are_refused() {
    let limits = Limits::default();
    let state_table = table();
    let mut contexts = bank(&limits);
    let error = MqDecoder::new(
        Payload::from(&[0][..]),
        CodedSpan {
            offset: 0,
            length: 4,
        },
        &state_table,
        &mut contexts,
        &limits,
    )
    .err()
    .unwrap();
    assert!(matches!(error.kind, ArithmeticErrorKind::InvalidSpan(_)));
    assert_eq!(error.offset, Some(0));
    assert!(std::error::Error::source(&error).is_none());
}

#[test]
fn finish_and_context_errors_keep_public_locations() {
    use std::error::Error as _;

    let limits = Limits::default();
    let state_table = table();
    let mut contexts = bank(&limits);

    let source: &[u8] = &[0, 0];
    let mut decoder = MqDecoder::new(
        Payload::from(source),
        CodedSpan {
            offset: 0,
            length: 2,
        },
        &state_table,
        &mut contexts,
        &limits,
    )
    .unwrap();
    let bad_context = decoder.decode_bit(1).unwrap_err();
    assert!(matches!(
        bad_context.kind,
        ArithmeticErrorKind::InvalidContext
    ));
    assert_eq!(bad_context.context, Some(1));
    assert!(bad_context.to_string().contains("context 1"));
    assert!(bad_context.source().is_none());
    decoder.decode_bit(0).unwrap();
    let at_end = decoder.decode_bit(0).unwrap_err();
    assert!(matches!(
        at_end.kind,
        ArithmeticErrorKind::MissingTerminator
    ));
    assert_eq!(at_end.offset, Some(2));
    assert_eq!(at_end.context, Some(0));
    assert!(at_end.to_string().contains("source byte 2"));

    let source: &[u8] = &[0, 0, 0];
    let mut contexts = bank(&limits);
    let mut decoder = MqDecoder::new(
        Payload::from(source),
        CodedSpan {
            offset: 0,
            length: 3,
        },
        &state_table,
        &mut contexts,
        &limits,
    )
    .unwrap();
    let wrong_count = decoder.finish(1).unwrap_err();
    assert!(matches!(
        wrong_count.kind,
        ArithmeticErrorKind::SymbolCount {
            expected: 1,
            decoded: 0
        }
    ));
    assert!(wrong_count.to_string().contains("expected 1 symbols"));

    let mut decoder = MqDecoder::new(
        Payload::from(source),
        CodedSpan {
            offset: 0,
            length: 3,
        },
        &state_table,
        &mut contexts,
        &limits,
    )
    .unwrap();
    let marker = decoder.finish(0).unwrap_err();
    assert!(matches!(
        marker.kind,
        ArithmeticErrorKind::MissingTerminator
    ));
    assert_eq!(marker.offset, Some(1));
    assert!(marker.to_string().contains("missing terminal marker"));

    let interior_marker: &[u8] = &[0xff, 0xac, 0, 0xff, 0xac];
    let error = MqDecoder::new(
        Payload::from(interior_marker),
        CodedSpan {
            offset: 0,
            length: 5,
        },
        &state_table,
        &mut contexts,
        &limits,
    )
    .err()
    .unwrap();
    assert!(matches!(
        error.kind,
        ArithmeticErrorKind::InvalidMarker(0xac)
    ));
    assert_eq!(error.offset, Some(1));

    // A terminal pair beyond the initial prefetch is checked only by finish.
    let mut bytes = vec![0; 300];
    bytes[298..].copy_from_slice(&[0xff, 0xab]);
    let late_marker: &[u8] = &bytes;
    let mut decoder = MqDecoder::new(
        Payload::from(late_marker),
        CodedSpan {
            offset: 0,
            length: 300,
        },
        &state_table,
        &mut contexts,
        &limits,
    )
    .unwrap();
    let error = decoder.finish(0).unwrap_err();
    assert!(matches!(
        error.kind,
        ArithmeticErrorKind::InvalidMarker(0xab)
    ));
    assert_eq!(error.offset, Some(299));
}

#[test]
fn invalid_configuration_fails_before_decoding() {
    let limits = Limits::default();
    assert!(matches!(
        context_bank(0, &limits).unwrap_err().kind,
        ArithmeticErrorKind::InvalidContext
    ));
    let source: &[u8] = &[0, 0, 0xff, 0xac];
    let state_table = table();
    let mut contexts = bank(&limits);
    for selected in [
        CodedSpan {
            offset: 0,
            length: 1,
        },
        CodedSpan {
            offset: u64::MAX - 1,
            length: 3,
        },
        CodedSpan {
            offset: 3,
            length: 3,
        },
    ] {
        let error = MqDecoder::new(
            Payload::from(source),
            selected,
            &state_table,
            &mut contexts,
            &limits,
        )
        .err()
        .unwrap();
        assert!(matches!(error.kind, ArithmeticErrorKind::InvalidSpan(_)));
    }
}

#[test]
fn input_mutations_terminate_and_stay_within_span() {
    let limits = Limits {
        io_chunk_bytes: 3,
        ..Limits::default()
    };
    let state_table = table();
    for index in 0..4 {
        for replacement in [0x00, 0x7f, 0x90, 0xff] {
            let mut bytes = [0x00, 0x00, 0xff, 0xac];
            bytes[index] = replacement;
            let source: &[u8] = &bytes;
            let mut contexts = bank(&limits);
            let result = MqDecoder::new(
                Payload::from(source),
                CodedSpan {
                    offset: 0,
                    length: 4,
                },
                &state_table,
                &mut contexts,
                &limits,
            );
            if let Ok(mut decoder) = result {
                for _ in 0..8 {
                    if decoder.decode_bit(0).is_err() {
                        break;
                    }
                    assert!(decoder.snapshot().input_offset <= 4);
                }
            }
        }
    }
}

#[test]
fn wide_interval_mps_in_upper_subinterval_skips_renormalization() {
    // The third decision of the C-high-0x2080 trace above: A - Qe = 0xD801
    // keeps the MPS without shifting, reading input, or changing state.
    let limits = Limits::default();
    let state_table = table();
    let source: &[u8] = &[0x41, 0x00, 0xff, 0xac];
    let mut contexts = bank(&limits);
    let whole = span(source);
    let mut decoder = MqDecoder::new(
        Payload::from(source),
        whole,
        &state_table,
        &mut contexts,
        &limits,
    )
    .unwrap();
    decoder.decode_bit(0).unwrap();
    decoder.decode_bit(0).unwrap();
    let first = decoder.snapshot();
    assert!(!decoder.decode_bit(0).unwrap());
    let second = decoder.snapshot();
    assert_eq!(second.interval, 0xd801);
    assert_eq!(second.bit_counter, first.bit_counter);
    assert_eq!(second.input_offset, first.input_offset);
    assert_eq!(decoder.context(0), Some(state(2, false)));
    decoder.finish(3).unwrap();
}

#[test]
fn span_beyond_input_limit_is_rejected_before_reading() {
    let limits = Limits {
        max_input_bytes: 3,
        ..Limits::default()
    };
    let state_table = table();
    let source: &[u8] = &[0, 0, 0xff, 0xac];
    let mut contexts = bank(&limits);
    let whole = span(source);
    let error = MqDecoder::new(
        Payload::from(source),
        whole,
        &state_table,
        &mut contexts,
        &limits,
    )
    .err()
    .unwrap();
    assert_eq!(error.offset, Some(0));
    assert!(matches!(
        error.kind,
        ArithmeticErrorKind::Source(Error::LimitExceeded {
            limit: 3,
            attempted: 4,
            ..
        })
    ));
}

#[test]
fn configuration_errors_render_without_a_source_location() {
    use caj2pdf_core::jbig2::mq::{ArithmeticError, Coder};

    let limits = Limits::default();
    let mut contexts = bank(&limits);
    let state_table = table();
    let source: &[u8] = &[0, 0, 0];
    let short_span = MqDecoder::new(
        Payload::from(source),
        CodedSpan {
            offset: 1,
            length: 1,
        },
        &state_table,
        &mut contexts,
        &limits,
    )
    .err()
    .unwrap();
    assert_eq!(
        short_span.to_string(),
        "T.88 MQ decoder at source byte 1: invalid span: requires at least two terminal bytes"
    );

    let error = ArithmeticError {
        coder: Some(Coder::T88),
        offset: None,
        context: None,
        kind: ArithmeticErrorKind::AllocationFailed,
    };
    assert_eq!(
        error.to_string(),
        "T.88 MQ decoder: context allocation failed"
    );
    assert!(std::error::Error::source(&error).is_none());
}
