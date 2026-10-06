// SPDX-License-Identifier: MIT

//! Public IAID API checks with original bytes for the standard MQ states.
//! These are arithmetic control-flow tests, not T.88 Table E.1 conformance.

use caj2pdf_core::{
    Limits, Payload,
    jbig2::{
        dictionary::coding_unit_contexts,
        iaid::{
            EMPTY_SYMBOL_SET, IAID_BASE, SYMBOL_ARRAY_LENGTH, SYMBOL_OUT_OF_RANGE,
            TOO_MANY_SYMBOLS, checked_symbol_index, decode_iaid,
        },
        integer::{BITMAP_BASE, IntegerProcedure, decode_integer},
        mq::{CodedSpan, ContextBank, ContextState, MqDecoder, MqTable},
    },
};

fn table() -> MqTable {
    MqTable::standard()
}

fn span_of(bytes: &[u8]) -> CodedSpan {
    CodedSpan {
        offset: 0,
        length: bytes.len() as u64,
    }
}

/// The contexts of a coding unit whose IAID width is `code_len`.
fn coding_unit(code_len: u32, limits: &Limits) -> ContextBank {
    ContextBank::new(coding_unit_contexts(code_len).unwrap(), limits).unwrap()
}

#[test]
fn zero_length_id_and_symbol_array_boundary() {
    let limits = Limits::default();
    let table = table();
    let mut contexts = coding_unit(0, &limits);
    assert_eq!(contexts.len(), IAID_BASE + 1);
    let source: &[u8] = &[0x7f, 0xff, 0xac];
    let span = span_of(source);
    let mut decoder =
        MqDecoder::new(Payload::from(source), span, &table, &mut contexts, &limits).unwrap();
    let before = decoder.snapshot();
    assert_eq!(decode_iaid(&mut decoder, 0).unwrap(), 0);
    assert_eq!(decoder.snapshot(), before);
    decoder.finish(0).unwrap();

    let reason = |result: caj2pdf_core::Result<usize>| result.map_err(|error| error.reason);
    assert_eq!(reason(checked_symbol_index(0, 1, 1)), Ok(0));
    assert_eq!(reason(checked_symbol_index(2, 3, 3)), Ok(2));
    assert_eq!(reason(checked_symbol_index(0, 0, 0)), Err(EMPTY_SYMBOL_SET));
    assert_eq!(
        reason(checked_symbol_index(3, 3, 3)),
        Err(SYMBOL_OUT_OF_RANGE)
    );
    assert_eq!(
        reason(checked_symbol_index(2, 3, 2)),
        Err(SYMBOL_ARRAY_LENGTH)
    );
    assert_eq!(
        reason(checked_symbol_index(0, 1, 2)),
        Err(SYMBOL_ARRAY_LENGTH)
    );
    if usize::BITS < 64 {
        assert_eq!(
            reason(checked_symbol_index(0, u64::MAX, 0)),
            Err(TOO_MANY_SYMBOLS)
        );
    }
}

#[test]
fn iaid_and_a2_use_distinct_adaptive_banks_on_one_stream() {
    let limits = Limits::default();
    let table = table();
    let bytes = [0x00, 0x00, 0x0a, 0xc0, 0x76, 0x01, 0xff, 0xac];
    let source: &[u8] = &bytes;
    let span = span_of(source);
    let mut contexts = coding_unit(1, &limits);
    let mut decoder =
        MqDecoder::new(Payload::from(source), span, &table, &mut contexts, &limits).unwrap();
    let first = decode_iaid(&mut decoder, 1).unwrap();
    let iaid_after_first = decoder.context(IAID_BASE + 1).unwrap();
    assert_ne!(iaid_after_first, ContextState::default());
    let _integer = decode_integer(&mut decoder, IntegerProcedure::Iaai).unwrap();
    assert_eq!(decoder.context(IAID_BASE + 1), Some(iaid_after_first));
    assert_ne!(decoder.context(1), Some(ContextState::default()));
    assert_eq!(decoder.context(BITMAP_BASE), Some(ContextState::default()));
    let second = decode_iaid(&mut decoder, 1).unwrap();
    assert_eq!((first, second), (0, 1));
    assert_ne!(decoder.context(IAID_BASE + 1), Some(iaid_after_first));
    let symbols = decoder.snapshot().symbols_decoded;
    assert!(symbols >= 6);
    decoder.finish(symbols).unwrap();
}

#[test]
fn the_full_iaid_range_is_checked_before_a_decision() {
    let limits = Limits::default();
    let table = table();
    let bytes = [0x7f, 0xff, 0xac];
    let source: &[u8] = &bytes;
    let span = span_of(source);
    let mut contexts = coding_unit(1, &limits);
    let mut decoder =
        MqDecoder::new(Payload::from(source), span, &table, &mut contexts, &limits).unwrap();
    for code_len in [2, 63, 64, u32::MAX] {
        let before = decoder.snapshot();
        let error = decode_iaid(&mut decoder, code_len).unwrap_err();
        assert_eq!(error.reason, "invalid arithmetic context index or count");
        assert_eq!(decoder.snapshot(), before);
    }
    decoder.finish(0).unwrap();
}

#[test]
fn marker_and_terminator_errors_are_located() {
    let limits = Limits {
        io_chunk_bytes: 1,
        ..Limits::default()
    };
    let table = table();
    for (bytes, invalid_marker) in [
        (&[0x80, 0][..], false),
        (&[0x80, 0xff, 0x90, 0xff, 0xac][..], true),
    ] {
        let source: &[u8] = bytes;
        let span = span_of(source);
        let mut contexts = coding_unit(2, &limits);
        let mut decoder =
            MqDecoder::new(Payload::from(source), span, &table, &mut contexts, &limits).unwrap();
        let error = decode_iaid(&mut decoder, 2).unwrap_err();
        if invalid_marker {
            assert_eq!(error.reason, "invalid MQ marker following 0xFF");
        } else {
            assert_eq!(error.reason, "MQ coding unit lacks its terminal marker");
        }
        assert_eq!(error.offset, Some(2));
    }

    let source: &[u8] = &[0x80, 0, 0xff, 0x90];
    let span = span_of(source);
    let mut contexts = coding_unit(0, &limits);
    let mut decoder =
        MqDecoder::new(Payload::from(source), span, &table, &mut contexts, &limits).unwrap();
    assert_eq!(decode_iaid(&mut decoder, 0).unwrap(), 0);
    let error = decoder.finish(0).unwrap_err();
    assert_eq!(error.reason, "invalid MQ marker following 0xFF");
}

#[test]
fn bounded_mutation_smoke_keeps_decisions_and_reads_within_limits() {
    let limits = Limits {
        io_chunk_bytes: 1,
        max_input_bytes: 5,
        max_allocation_bytes: 20_000,
        ..Limits::default()
    };
    let table = table();
    for seed in 0..128u8 {
        let source: &[u8] = &[seed, seed ^ 0x55, 0, 0xff, 0xac];
        let span = span_of(source);
        let mut contexts = coding_unit(3, &limits);
        if let Ok(mut decoder) =
            MqDecoder::new(Payload::from(source), span, &table, &mut contexts, &limits)
        {
            let _ = decode_iaid(&mut decoder, 3);
            // A three-bit IAID decodes at most three decisions.
            assert!(decoder.snapshot().symbols_decoded <= 3);
        }
    }
}
