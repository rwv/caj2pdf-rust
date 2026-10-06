// SPDX-License-Identifier: MIT

//! T.88 Annex A.2 non-IAID integer decisions on an existing MQ stream, and
//! the fixed context layout of a symbol-dictionary or text-region coding
//! unit: the thirteen 512-slot integer procedure banks at
//! `0..INTEGER_CONTEXT_COUNT`, the 1,024 generic or refinement bitmap
//! contexts at [`BITMAP_BASE`], and the IAID contexts at
//! [`IAID_BASE`](super::iaid::IAID_BASE). This module does not own an MQ
//! byte stream, finish it, or include probability states.

use super::mq::MqDecoder;
use crate::arith::INVALID_CONTEXT;
use crate::{Error, Result};

pub const CONTEXTS_PER_PROCEDURE: usize = 512;
pub const INTEGER_CONTEXT_COUNT: usize = 13 * CONTEXTS_PER_PROCEDURE;
/// The first of the [`BITMAP_CONTEXT_COUNT`] generic-region (template 2) or
/// refinement (template 1) contexts that follow the integer banks.
pub const BITMAP_BASE: usize = INTEGER_CONTEXT_COUNT;
pub const BITMAP_CONTEXT_COUNT: usize = 1024;
const MAX_DECISIONS: u8 = 38;
const BANDS: [(u8, u64); 6] = [(2, 0), (4, 4), (6, 20), (8, 84), (12, 340), (32, 4436)];

// Every band has at most 32 payload bits and a base below 2^32, so a decoded
// magnitude stays below 2^33 and fits an `i64` without checked arithmetic.
const _: () = {
    let mut band = 0;
    while band < BANDS.len() {
        assert!(BANDS[band].0 <= 32 && BANDS[band].1 < 1 << 32);
        band += 1;
    }
};

/// One of the thirteen Annex A.2 procedures. IAID uses Annex A.3 instead.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum IntegerProcedure {
    Iaai,
    Iadh,
    Iads,
    Iadt,
    Iadw,
    Iaex,
    Iafs,
    Iait,
    Iardh,
    Iardw,
    Iardx,
    Iardy,
    Iari,
}

impl IntegerProcedure {
    /// The first context of this procedure's bank.
    pub fn base(self) -> usize {
        (self as usize) * CONTEXTS_PER_PROCEDURE
    }
}

/// A decoded signed value or the distinct negative-zero out-of-band marker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntegerValue {
    Signed(i64),
    OutOfBand,
}

fn take(
    decoder: &mut MqDecoder<'_>,
    base: usize,
    prev: &mut u16,
    decisions: &mut u8,
) -> Result<bool> {
    let context = base + usize::from(*prev);
    if *decisions >= MAX_DECISIONS {
        return Err(Error::limit(
            "T.88 integer decisions",
            u64::from(MAX_DECISIONS),
            u64::from(*decisions) + 1,
        ));
    }
    let bit = decoder.decode_bit(context)?;
    *decisions += 1;
    let next = (*prev << 1) | u16::from(bit);
    *prev = if *prev < 256 {
        next
    } else {
        (next & 511) | 256
    };
    Ok(bit)
}

/// Decode one non-IAID integer without ending or recreating the shared MQ stream.
///
/// The fixed layout reserves slots `0..6656` for the thirteen procedure
/// banks. Missing capacity is rejected before any decision. One invocation
/// consumes at most 38 MQ symbols; marker errors are returned
/// unchanged. The caller decides whether OOB is legal here.
pub fn decode_integer(
    decoder: &mut MqDecoder<'_>,
    procedure: IntegerProcedure,
) -> Result<IntegerValue> {
    let last = INTEGER_CONTEXT_COUNT - 1;
    if decoder.context(last).is_none() {
        return Err(decoder.at(INVALID_CONTEXT));
    }
    let base = procedure.base();
    let mut prev = 1u16;
    let mut decisions = 0u8;
    let negative = take(decoder, base, &mut prev, &mut decisions)?;
    let mut band = 0usize;
    while band < BANDS.len() - 1 && take(decoder, base, &mut prev, &mut decisions)? {
        band += 1;
    }
    let (payload_bits, band_base) = BANDS[band];
    let mut payload = 0u64;
    for _ in 0..payload_bits {
        let bit = take(decoder, base, &mut prev, &mut decisions)?;
        payload = payload * 2 + u64::from(bit);
    }
    // Bounded by the `BANDS` assertion above.
    let magnitude = band_base + payload;
    if negative && magnitude == 0 {
        return Ok(IntegerValue::OutOfBand);
    }
    let signed = magnitude as i64;
    Ok(IntegerValue::Signed(if negative {
        -signed
    } else {
        signed
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ErrorKind;
    use crate::jbig2::mq::{CodedSpan, ContextBank, ContextState, MqTable};
    use crate::test_support::mq_encoder;
    use crate::{Limits, Payload};

    fn whole(bytes: &[u8]) -> CodedSpan {
        CodedSpan {
            offset: 0,
            length: bytes.len() as u64,
        }
    }

    /// Decode one integer of `procedure` from a stream that codes
    /// `decisions` (context offsets within the procedure's bank), returning
    /// the value and every context state afterwards. The stream must hold
    /// exactly one integer.
    fn decode_coded(
        procedure: IntegerProcedure,
        decisions: &[(usize, bool)],
    ) -> (IntegerValue, Vec<ContextState>) {
        let mut encoder = mq_encoder();
        for &(context, bit) in decisions {
            encoder.encode(procedure.base() + context, bit);
        }
        let bytes = encoder.finish();
        let limits = Limits::default();
        let mut bank = ContextBank::new(INTEGER_CONTEXT_COUNT, &limits).unwrap();
        let source = Payload::from(&(&bytes)[..]);
        let table = MqTable::standard();
        let mut decoder =
            MqDecoder::new(source, whole(&bytes), &table, &mut bank, &limits).unwrap();
        let value = decode_integer(&mut decoder, procedure).unwrap();
        assert_eq!(
            decoder.snapshot().symbols_decoded,
            decisions.len() as u64,
            "one integer consumes exactly its decisions"
        );
        decoder.finish(decisions.len() as u64).unwrap();
        let states = (0..INTEGER_CONTEXT_COUNT)
            .map(|index| bank.get(index).unwrap())
            .collect();
        (value, states)
    }

    /// The decisions of `code` ("0"/"1" digits) in the hand-derived context
    /// offsets `contexts`.
    fn decisions(code: &str, contexts: &[usize]) -> Vec<(usize, bool)> {
        assert_eq!(code.len(), contexts.len());
        contexts
            .iter()
            .zip(code.bytes())
            .map(|(&context, digit)| (context, digit == b'1'))
            .collect()
    }

    /// Every first-use one bit is an LPS in a fresh context, which always
    /// changes that context's state; no context outside `decisions` moves.
    fn assert_contexts(
        procedure: IntegerProcedure,
        decisions: &[(usize, bool)],
        states: &[ContextState],
    ) {
        let base = procedure.base();
        let mut seen = Vec::new();
        for &(context, bit) in decisions {
            if bit && !seen.contains(&context) {
                assert_ne!(states[base + context], ContextState::default(), "{context}");
            }
            seen.push(context);
        }
        for (index, state) in states.iter().enumerate() {
            let used = index >= base && seen.contains(&(index - base));
            if !used {
                assert_eq!(*state, ContextState::default(), "context {index}");
            }
        }
    }

    #[test]
    fn official_a2_iadw_example_has_hand_derived_contexts() {
        // Annex A.2 describes these decisions and contexts, but no encoded
        // MQ bytes: 0101000 is IADW 12 in contexts 1, 2, 5, 10, 21, 42, 84.
        let coded = decisions("0101000", &[1, 2, 5, 10, 21, 42, 84]);
        let (value, states) = decode_coded(IntegerProcedure::Iadw, &coded);
        assert_eq!(value, IntegerValue::Signed(12));
        assert_contexts(IntegerProcedure::Iadw, &coded, &states);
    }

    #[test]
    fn every_magnitude_band_boundary_both_signs_and_oob() {
        let cases = [
            ("00", "00", IntegerValue::Signed(0)),
            ("00", "11", IntegerValue::Signed(3)),
            ("10", "00", IntegerValue::OutOfBand),
            ("10", "01", IntegerValue::Signed(-1)),
            ("10", "11", IntegerValue::Signed(-3)),
            ("010", "0000", IntegerValue::Signed(4)),
            ("010", "1111", IntegerValue::Signed(19)),
            ("110", "0000", IntegerValue::Signed(-4)),
            ("110", "1111", IntegerValue::Signed(-19)),
            ("0110", "000000", IntegerValue::Signed(20)),
            ("0110", "111111", IntegerValue::Signed(83)),
            ("1110", "000000", IntegerValue::Signed(-20)),
            ("1110", "111111", IntegerValue::Signed(-83)),
            ("01110", "00000000", IntegerValue::Signed(84)),
            ("01110", "11111111", IntegerValue::Signed(339)),
            ("11110", "00000000", IntegerValue::Signed(-84)),
            ("11110", "11111111", IntegerValue::Signed(-339)),
            ("011110", "000000000000", IntegerValue::Signed(340)),
            ("011110", "111111111111", IntegerValue::Signed(4435)),
            ("111110", "000000000000", IntegerValue::Signed(-340)),
            ("111110", "111111111111", IntegerValue::Signed(-4435)),
            (
                "011111",
                "00000000000000000000000000000000",
                IntegerValue::Signed(4436),
            ),
            (
                "111111",
                "00000000000000000000000000000000",
                IntegerValue::Signed(-4436),
            ),
            (
                "011111",
                "11111111111111111111111111111111",
                IntegerValue::Signed(4_294_971_731),
            ),
            (
                "111111",
                "11111111111111111111111111111111",
                IntegerValue::Signed(-4_294_971_731),
            ),
        ];
        for (prefix, payload, expected) in cases {
            let code = format!("{prefix}{payload}");
            // Annex A.2's PREV: shift in each bit, keeping nine bits plus
            // the marker once PREV reaches 256.
            let mut prev = 1_usize;
            let coded: Vec<_> = code
                .bytes()
                .map(|digit| {
                    let bit = digit == b'1';
                    let context = prev;
                    let next = (prev << 1) | usize::from(bit);
                    prev = if prev < 256 { next } else { (next & 511) | 256 };
                    (context, bit)
                })
                .collect();
            assert!(coded.len() <= usize::from(MAX_DECISIONS));
            let (value, _) = decode_coded(IntegerProcedure::Iaai, &coded);
            assert_eq!(value, expected, "code {code}");
        }
    }

    #[test]
    fn prev_rolls_at_nine_bits_and_keeps_recent_history() {
        let mut zeroes = decisions(
            "011111000000",
            &[1, 2, 5, 11, 23, 47, 95, 190, 380, 504, 496, 480],
        );
        // The remaining 26 zero payload bits after PREV has rolled: each
        // keeps the marker bit 256 and shifts in a zero.
        let mut prev = 448_usize;
        for _ in 0..26 {
            zeroes.push((prev, false));
            prev = ((prev << 1) & 511) | 256;
        }
        assert_eq!(zeroes.len(), 38);
        assert_eq!(zeroes[37].0, 256);
        let (value, states) = decode_coded(IntegerProcedure::Iaai, &zeroes);
        assert_eq!(value, IntegerValue::Signed(4436));
        assert_contexts(IntegerProcedure::Iaai, &zeroes, &states);

        let mut ones = decisions("0111111111", &[1, 2, 5, 11, 23, 47, 95, 191, 383, 511]);
        ones.extend([(511, true); 28]);
        let (value, states) = decode_coded(IntegerProcedure::Iaai, &ones);
        assert_eq!(value, IntegerValue::Signed(4_294_971_731));
        assert_contexts(IntegerProcedure::Iaai, &ones, &states);
    }

    #[test]
    fn all_thirteen_procedures_use_separate_banks() {
        let procedures = [
            IntegerProcedure::Iaai,
            IntegerProcedure::Iadh,
            IntegerProcedure::Iads,
            IntegerProcedure::Iadt,
            IntegerProcedure::Iadw,
            IntegerProcedure::Iaex,
            IntegerProcedure::Iafs,
            IntegerProcedure::Iait,
            IntegerProcedure::Iardh,
            IntegerProcedure::Iardw,
            IntegerProcedure::Iardx,
            IntegerProcedure::Iardy,
            IntegerProcedure::Iari,
        ];
        for (bank, procedure) in procedures.into_iter().enumerate() {
            assert_eq!(procedure.base(), bank * CONTEXTS_PER_PROCEDURE);
            // -1 is 1 0 01 in contexts 1, 3, 6, 12 of its own bank.
            let coded = decisions("1001", &[1, 3, 6, 12]);
            let (value, states) = decode_coded(procedure, &coded);
            assert_eq!(value, IntegerValue::Signed(-1));
            assert_contexts(procedure, &coded, &states);
        }
        assert_eq!(
            procedures.len() * CONTEXTS_PER_PROCEDURE,
            INTEGER_CONTEXT_COUNT
        );
    }

    #[test]
    fn incomplete_integer_preserves_the_located_mq_error() {
        // Spans without the FF AC terminal pair end inside the integer.
        for bytes in [&[0x00, 0x00][..], &[0x00, 0x00, 0x00, 0x00]] {
            let limits = Limits::default();
            let mut bank = ContextBank::new(INTEGER_CONTEXT_COUNT, &limits).unwrap();
            let source = Payload::from(bytes);
            let table = MqTable::standard();
            let mut decoder =
                MqDecoder::new(source, whole(bytes), &table, &mut bank, &limits).unwrap();
            let error = decode_integer(&mut decoder, IntegerProcedure::Iadw).unwrap_err();
            assert_eq!(error.reason, "MQ coding unit lacks its terminal marker");
            assert_eq!(error.offset, Some(bytes.len() as u64));
        }
    }

    #[test]
    fn integer_decision_limit_is_checked_before_the_next_mq_symbol() {
        let bytes = [0xff, 0xac];
        let limits = Limits::default();
        let mut bank = ContextBank::new(INTEGER_CONTEXT_COUNT, &limits).unwrap();
        let source = Payload::from(&(&bytes)[..]);
        let table = MqTable::standard();
        let mut decoder =
            MqDecoder::new(source, whole(&bytes), &table, &mut bank, &limits).unwrap();
        let mut prev = 1;
        let mut count = MAX_DECISIONS;
        let error = take(&mut decoder, 0, &mut prev, &mut count).unwrap_err();
        assert!(matches!(
            error,
            Error {
                kind: ErrorKind::LimitExceeded {
                    resource: "T.88 integer decisions",
                    limit: 38,
                    attempted: 39,
                },
                ..
            }
        ));
        assert_eq!(decoder.snapshot().symbols_decoded, 0);
        assert_eq!((prev, count), (1, MAX_DECISIONS));
    }

    #[test]
    fn real_mq_stream_shares_contexts_and_finishes_only_after_integers() {
        // Two IADW values adapt the same bank; IADH uses its own. Decoding
        // the encoded values back proves the decoder chose the same
        // contexts as the encoder for every decision.
        let values = [
            (IntegerProcedure::Iadw, Some(4_294_971_731)),
            (IntegerProcedure::Iadw, Some(-4_026_536_276)),
            (IntegerProcedure::Iadh, Some(2)),
            (IntegerProcedure::Iadw, None),
        ];
        let mut encoder = mq_encoder();
        for (procedure, value) in values {
            encoder.integer(procedure.base(), value);
        }
        let symbols = encoder.decisions();
        let bytes = encoder.finish();
        let limits = Limits::default();
        let table = MqTable::standard();
        let mut bank = ContextBank::new(INTEGER_CONTEXT_COUNT, &limits).unwrap();
        let source = Payload::from(&(&bytes)[..]);
        let mut decoder =
            MqDecoder::new(source, whole(&bytes), &table, &mut bank, &limits).unwrap();
        for (procedure, value) in values {
            let expected = value.map_or(IntegerValue::OutOfBand, IntegerValue::Signed);
            assert_eq!(decode_integer(&mut decoder, procedure).unwrap(), expected);
        }
        assert_eq!(decoder.snapshot().symbols_decoded, symbols);
        decoder.finish(symbols).unwrap();
    }

    #[test]
    fn missing_last_bank_is_rejected_before_any_mq_decision() {
        let limits = Limits::default();
        let table = MqTable::standard();
        let mut contexts = ContextBank::new(INTEGER_CONTEXT_COUNT - 1, &limits).unwrap();
        let bytes = [0xff, 0xac];
        let source = Payload::from(&(&bytes)[..]);
        let mut decoder =
            MqDecoder::new(source, whole(&bytes), &table, &mut contexts, &limits).unwrap();
        let before = decoder.snapshot();
        let error = decode_integer(&mut decoder, IntegerProcedure::Iaai).unwrap_err();
        assert_eq!(error.reason, "invalid arithmetic context index or count");
        assert_eq!(decoder.snapshot(), before);
        decoder.finish(0).unwrap();
    }

    #[test]
    fn real_mq_marker_errors_remain_visible() {
        let limits = Limits::default();
        let table = MqTable::standard();
        // One complete integer, then an invalid pair where FF AC belongs.
        let mut encoder = mq_encoder();
        encoder.integer(IntegerProcedure::Iaai.base(), Some(3));
        let mut invalid = encoder.finish_zero_padded();
        invalid.resize(19, 0);
        invalid.extend_from_slice(&[0xff, 0x90]);
        let mut bank = ContextBank::new(INTEGER_CONTEXT_COUNT, &limits).unwrap();
        let source = Payload::from(&(&invalid)[..]);
        let mut decoder =
            MqDecoder::new(source, whole(&invalid), &table, &mut bank, &limits).unwrap();
        assert_eq!(
            decode_integer(&mut decoder, IntegerProcedure::Iaai).unwrap(),
            IntegerValue::Signed(3)
        );
        let symbols = decoder.snapshot().symbols_decoded;
        let error = decoder.finish(symbols).unwrap_err();
        assert_eq!(error.reason, "invalid MQ marker following 0xFF");

        let short = [0x80, 0];
        let mut bank = ContextBank::new(INTEGER_CONTEXT_COUNT, &limits).unwrap();
        let source = Payload::from(&(&short)[..]);
        let mut decoder =
            MqDecoder::new(source, whole(&short), &table, &mut bank, &limits).unwrap();
        let error = decode_integer(&mut decoder, IntegerProcedure::Iaai).unwrap_err();
        assert_eq!(error.reason, "MQ coding unit lacks its terminal marker");
        assert_eq!(error.offset, Some(short.len() as u64));
    }
}
