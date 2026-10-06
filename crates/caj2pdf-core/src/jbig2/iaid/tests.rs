// SPDX-License-Identifier: MIT

use super::*;
use crate::jbig2::mq::{CodedSpan, ContextBank, ContextState, MqBudget, MqTable};
use crate::test_support::{mq_encoder, ready};
use crate::{Limits, NeverCancel, native::SeekableSource};
use std::io::Cursor;

type Source = SeekableSource<Cursor<Vec<u8>>>;

/// Decode `values` as consecutive IAIDs of `code_len` bits from a stream
/// that codes them, over a bank of exactly the contexts the width needs.
/// Returns the decoded values and every context state afterwards.
fn round_trip(code_len: u32, values: &[u64]) -> (Vec<u64>, Vec<ContextState>) {
    let mut encoder = mq_encoder();
    for &value in values {
        encoder.iaid(IAID_BASE, code_len, value);
    }
    let symbols = encoder.decisions();
    let bytes = encoder.finish();
    let limits = Limits::default();
    let contexts = IAID_BASE + (1 << code_len);
    let mut bank = ContextBank::new(contexts, &limits).unwrap();
    let mut source = Source::new(Cursor::new(bytes.clone())).unwrap();
    let table = MqTable::standard();
    let mut decoder = ready(MqDecoder::new(
        &mut source,
        CodedSpan {
            offset: 0,
            length: bytes.len() as u64,
        },
        &table,
        &mut bank,
        &limits,
        &NeverCancel,
        MqBudget::default(),
    ))
    .unwrap();
    let decoded = values
        .iter()
        .map(|_| ready(decode_iaid(&mut decoder, code_len)).unwrap())
        .collect();
    assert_eq!(decoder.snapshot().symbols_decoded, symbols);
    assert_eq!(symbols, u64::from(code_len) * values.len() as u64);
    ready(decoder.finish(symbols)).unwrap();
    let states = (0..contexts)
        .map(|index| bank.get(index).unwrap())
        .collect();
    (decoded, states)
}

#[test]
fn annex_a3_example_and_every_small_codeword() {
    // Annex A.3: with SBSYMCODELEN 3, ID 2 is 010 in IAID contexts 1, 2, 5.
    // Its one bit is an LPS in a fresh context, which always moves it; no
    // context outside the IAID bank moves.
    let (decoded, states) = round_trip(3, &[2]);
    assert_eq!(decoded, [2]);
    assert_ne!(states[IAID_BASE + 2], ContextState::default());
    for (index, state) in states.iter().enumerate() {
        if ![IAID_BASE + 1, IAID_BASE + 2, IAID_BASE + 5].contains(&index) {
            assert_eq!(*state, ContextState::default(), "context {index}");
        }
    }
    for len in 0..=3u32 {
        let values: Vec<_> = (0..1u64 << len).collect();
        assert_eq!(round_trip(len, &values).0, values);
    }
}

#[test]
fn largest_default_width_and_truncated_stream_error() {
    let (decoded, states) = round_trip(15, &[0x5555, 0x7fff, 0]);
    assert_eq!(decoded, [0x5555, 0x7fff, 0]);
    assert_eq!(states.len(), IAID_BASE + 32_768);
    assert!(
        states[..IAID_BASE]
            .iter()
            .all(|state| *state == ContextState::default())
    );

    // A span without the FF AC terminal pair ends within the ID.
    let bytes = vec![0x00, 0x00];
    let limits = Limits::default();
    let mut bank = ContextBank::new(IAID_BASE + 32_768, &limits).unwrap();
    let mut source = Source::new(Cursor::new(bytes)).unwrap();
    let table = MqTable::standard();
    let mut decoder = ready(MqDecoder::new(
        &mut source,
        CodedSpan {
            offset: 0,
            length: 2,
        },
        &table,
        &mut bank,
        &limits,
        &NeverCancel,
        MqBudget::default(),
    ))
    .unwrap();
    let error = ready(decode_iaid(&mut decoder, 15)).unwrap_err();
    assert!(matches!(error.kind, ArithmeticErrorKind::MissingTerminator));
    assert_eq!(error.offset, Some(2));
    assert!(
        error
            .context
            .is_some_and(|context| (IAID_BASE..IAID_BASE + 32_768).contains(&context))
    );
}

#[test]
fn widths_beyond_the_bank_or_the_address_space_are_refused_before_input() {
    let bytes = vec![0xff, 0xac];
    let limits = Limits::default();
    let mut bank = ContextBank::new(IAID_BASE + 8, &limits).unwrap();
    let mut source = Source::new(Cursor::new(bytes)).unwrap();
    let table = MqTable::standard();
    let mut decoder = ready(MqDecoder::new(
        &mut source,
        CodedSpan {
            offset: 0,
            length: 2,
        },
        &table,
        &mut bank,
        &limits,
        &NeverCancel,
        MqBudget::default(),
    ))
    .unwrap();
    let before = decoder.snapshot();
    // This is 32 on wasm32 and 64 on x86_64: the first invalid `usize`
    // shift fails without naming a context.
    for (len, context) in [
        (4, Some(IAID_BASE + 15)),
        (usize::BITS, None),
        (u32::MAX, None),
    ] {
        let error = ready(decode_iaid(&mut decoder, len)).unwrap_err();
        assert!(matches!(error.kind, ArithmeticErrorKind::InvalidContext));
        assert_eq!(error.context, context, "width {len}");
        assert_eq!(decoder.snapshot(), before);
    }
    assert_eq!(ready(decode_iaid(&mut decoder, 0)).unwrap(), 0);
    assert_eq!(decoder.snapshot(), before);
    ready(decoder.finish(0)).unwrap();
}
