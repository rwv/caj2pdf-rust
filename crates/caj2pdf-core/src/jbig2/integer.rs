// SPDX-License-Identifier: MIT

//! T.88 Annex A.2 non-IAID integer decisions on an existing MQ stream.
//!
//! The first 6,656 MQ contexts are thirteen disjoint 512-slot procedure
//! banks. Other models may use contexts after them. This module does not own
//! an MQ byte stream, finish it, or include T.88 Table E.1 probability rows.

use super::mq::{MqBudget, MqContext, MqContexts, MqDecoder, MqError, MqErrorKind, MqResult};
use crate::{Cancellation, Limits, RangedSource};

pub const CONTEXTS_PER_PROCEDURE: usize = 512;
pub const INTEGER_CONTEXT_COUNT: usize = 13 * CONTEXTS_PER_PROCEDURE;
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
    fn base(self) -> usize {
        (self as usize) * CONTEXTS_PER_PROCEDURE
    }
}

/// A decoded signed value or the distinct negative-zero out-of-band marker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntegerValue {
    Signed(i64),
    OutOfBand,
}

/// Initially zeroed integer banks, optionally followed by other MQ contexts.
///
/// Create one bank set for a coding unit. Keep it across integer invocations.
/// T.88 §7.4.2.2 resets integer contexts for each new symbol dictionary but
/// may retain appended bitmap-model contexts. The borrowed MQ decoder must
/// be dropped or finished before either reset method is called.
#[derive(Debug)]
pub struct IntegerContextBanks {
    contexts: MqContexts,
}

impl IntegerContextBanks {
    pub fn new(limits: &Limits, budget: &MqBudget) -> MqResult<Self> {
        Self::with_extra_contexts(0, limits, budget)
    }

    /// Reserve additional contexts after the thirteen integer banks for other models.
    pub fn with_extra_contexts(extra: usize, limits: &Limits, budget: &MqBudget) -> MqResult<Self> {
        let count = INTEGER_CONTEXT_COUNT.checked_add(extra).ok_or(MqError {
            offset: None,
            context: None,
            kind: MqErrorKind::InvalidContext,
        })?;
        Ok(Self {
            contexts: MqContexts::new(count, limits, budget)?,
        })
    }

    pub fn mq_contexts_mut(&mut self) -> &mut MqContexts {
        &mut self.contexts
    }

    /// Reset only the thirteen arithmetic-integer banks; keep appended models.
    pub fn reset_integer_contexts(&mut self) -> MqResult<()> {
        for index in 0..INTEGER_CONTEXT_COUNT {
            self.contexts.set(index, MqContext::default())?;
        }
        Ok(())
    }

    /// Reset integer banks and every appended model context.
    pub fn reset_all(&mut self) {
        self.contexts.reset();
    }

    /// Reset every context, including appended models. Prefer the named reset.
    pub fn reset(&mut self) {
        self.reset_all();
    }
}

trait DecisionSource {
    async fn bit(&mut self, context: usize) -> MqResult<bool>;
}

impl<S: RangedSource, C: Cancellation> DecisionSource for MqDecoder<'_, S, C> {
    async fn bit(&mut self, context: usize) -> MqResult<bool> {
        self.decode_bit(context).await
    }
}

async fn take<D: DecisionSource>(
    source: &mut D,
    base: usize,
    prev: &mut u16,
    decisions: &mut u8,
) -> MqResult<bool> {
    let context = base + usize::from(*prev);
    if *decisions >= MAX_DECISIONS {
        return Err(MqError {
            offset: None,
            context: Some(context),
            kind: MqErrorKind::LimitExceeded {
                resource: "T.88 integer decisions",
                limit: u64::from(MAX_DECISIONS),
                attempted: u64::from(*decisions) + 1,
            },
        });
    }
    let bit = source.bit(context).await?;
    *decisions += 1;
    let next = (*prev << 1) | u16::from(bit);
    *prev = if *prev < 256 {
        next
    } else {
        (next & 511) | 256
    };
    Ok(bit)
}

async fn decode_decisions<D: DecisionSource>(
    source: &mut D,
    procedure: IntegerProcedure,
) -> MqResult<IntegerValue> {
    let base = procedure.base();
    let mut prev = 1u16;
    let mut decisions = 0u8;
    let negative = take(source, base, &mut prev, &mut decisions).await?;
    let mut band = 0usize;
    while band < BANDS.len() - 1 && take(source, base, &mut prev, &mut decisions).await? {
        band += 1;
    }
    let (payload_bits, band_base) = BANDS[band];
    let mut payload = 0u64;
    for _ in 0..payload_bits {
        let bit = take(source, base, &mut prev, &mut decisions).await?;
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

/// Decode one non-IAID integer without ending or recreating the shared MQ stream.
///
/// The fixed layout reserves slots `0..6656` for the thirteen typed procedure
/// banks. Missing capacity is rejected before any decision. One invocation
/// consumes at most 38 MQ symbols; source, budget, and cancellation errors
/// are returned unchanged. The caller decides whether OOB is legal here.
pub async fn decode_integer<S: RangedSource, C: Cancellation>(
    decoder: &mut MqDecoder<'_, S, C>,
    procedure: IntegerProcedure,
) -> MqResult<IntegerValue> {
    let last = INTEGER_CONTEXT_COUNT - 1;
    if decoder.context(last).is_none() {
        return Err(MqError {
            offset: Some(decoder.snapshot().current_input_offset),
            context: Some(last),
            kind: MqErrorKind::InvalidContext,
        });
    }
    decode_decisions(decoder, procedure).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ready;
    use crate::{NeverCancel, native::SeekableSource};
    use std::{cell::Cell, io::Cursor, rc::Rc};

    /// Every test reads through this one source type, so their paths share
    /// one instantiation of the generic decoders.
    fn vec_source(bytes: &[u8]) -> SeekableSource<Cursor<Vec<u8>>> {
        SeekableSource::new(Cursor::new(bytes.to_vec())).unwrap()
    }

    struct Decisions {
        bits: Vec<bool>,
        cursor: usize,
        contexts: Vec<usize>,
    }

    impl Decisions {
        fn from_bits(bits: &str) -> Self {
            assert!(
                bits.bytes().all(|byte| byte == b'0' || byte == b'1'),
                "test decision trace must contain only 0 or 1"
            );
            Self {
                bits: bits.bytes().map(|byte| byte == b'1').collect(),
                cursor: 0,
                contexts: Vec::new(),
            }
        }
    }

    impl DecisionSource for Decisions {
        async fn bit(&mut self, context: usize) -> MqResult<bool> {
            self.contexts.push(context);
            let bit = self.bits.get(self.cursor).copied().ok_or(MqError {
                offset: Some(77),
                context: Some(context),
                kind: MqErrorKind::MissingTerminator,
            })?;
            self.cursor += 1;
            Ok(bit)
        }
    }

    #[test]
    fn official_a2_iadw_example_has_hand_derived_contexts() {
        // Annex A.2 describes these decisions and contexts, but no encoded MQ bytes.
        let mut bits = Decisions::from_bits("0101000");
        assert_eq!(
            ready(decode_decisions(&mut bits, IntegerProcedure::Iadw)).unwrap(),
            IntegerValue::Signed(12)
        );
        let base = IntegerProcedure::Iadw.base();
        assert_eq!(
            bits.contexts,
            [1, 2, 5, 10, 21, 42, 84].map(|index| base + index)
        );
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
            let mut bits = Decisions::from_bits(&code);
            assert_eq!(
                ready(decode_decisions(&mut bits, IntegerProcedure::Iaai)).unwrap(),
                expected,
                "code {code}"
            );
            assert_eq!(bits.cursor, code.len(), "code {code}");
            assert!(bits.cursor <= usize::from(MAX_DECISIONS));
        }
    }

    #[test]
    fn prev_rolls_at_nine_bits_and_keeps_recent_history() {
        let mut zeroes = Decisions::from_bits(&format!("011111{}", "0".repeat(32)));
        assert_eq!(
            ready(decode_decisions(&mut zeroes, IntegerProcedure::Iaai)).unwrap(),
            IntegerValue::Signed(4436)
        );
        assert_eq!(zeroes.contexts.len(), 38);
        assert_eq!(
            &zeroes.contexts[..12],
            &[1, 2, 5, 11, 23, 47, 95, 190, 380, 504, 496, 480]
        );
        assert_eq!(zeroes.contexts[37], 256);

        let mut ones = Decisions::from_bits(&format!("011111{}", "1".repeat(32)));
        assert_eq!(
            ready(decode_decisions(&mut ones, IntegerProcedure::Iaai)).unwrap(),
            IntegerValue::Signed(4_294_971_731)
        );
        assert_eq!(&ones.contexts[6..10], &[95, 191, 383, 511]);
        assert_eq!(ones.contexts[37], 511);
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
            let mut bits = Decisions::from_bits("0000");
            assert_eq!(
                ready(decode_decisions(&mut bits, procedure)).unwrap(),
                IntegerValue::Signed(0)
            );
            assert_eq!(bits.contexts, [1, 2, 4, 8].map(|index| bank * 512 + index));
        }
        assert_eq!(
            procedures.len() * CONTEXTS_PER_PROCEDURE,
            INTEGER_CONTEXT_COUNT
        );
    }

    #[test]
    fn incomplete_decision_trace_preserves_located_error() {
        for code in ["", "0", "011", "011110101"] {
            let mut bits = Decisions::from_bits(code);
            let error = ready(decode_decisions(&mut bits, IntegerProcedure::Iadw)).unwrap_err();
            assert!(matches!(error.kind, MqErrorKind::MissingTerminator));
            assert_eq!(error.offset, Some(77));
            assert_eq!(error.context, bits.contexts.last().copied());
        }
    }

    #[test]
    fn integer_decision_limit_is_checked_before_the_next_mq_symbol() {
        let mut bits = Decisions::from_bits("0");
        let mut prev = 1;
        let mut count = MAX_DECISIONS;
        let error = ready(take(&mut bits, 0, &mut prev, &mut count)).unwrap_err();
        assert!(matches!(
            error.kind,
            MqErrorKind::LimitExceeded {
                resource: "T.88 integer decisions",
                limit: 38,
                attempted: 39,
            }
        ));
        assert!(bits.contexts.is_empty());
        assert_eq!((prev, count), (1, MAX_DECISIONS));
    }

    fn invented_table(limits: &Limits) -> super::super::mq::MqTable {
        use super::super::mq::{MQ_STATE_COUNT, MqState, MqTable};
        let mut states = vec![
            MqState {
                qe: 0x4000,
                next_mps: 0,
                next_lps: 0,
                switch_mps: false,
            };
            MQ_STATE_COUNT
        ];
        states[0].next_mps = 1;
        states[0].next_lps = 1;
        states[1].next_mps = 2;
        states[1].next_lps = 2;
        MqTable::new(states, limits).unwrap()
    }

    #[test]
    fn bank_allocation_extra_capacity_and_explicit_reset() {
        let limits = Limits::default();
        let budget = MqBudget::default();
        let mut banks = IntegerContextBanks::with_extra_contexts(7, &limits, &budget).unwrap();
        assert!(
            banks
                .mq_contexts_mut()
                .get(INTEGER_CONTEXT_COUNT - 1)
                .is_some()
        );
        assert!(
            banks
                .mq_contexts_mut()
                .get(INTEGER_CONTEXT_COUNT + 6)
                .is_some()
        );
        assert!(
            banks
                .mq_contexts_mut()
                .get(INTEGER_CONTEXT_COUNT + 7)
                .is_none()
        );
        banks
            .mq_contexts_mut()
            .set(
                1,
                MqContext {
                    state_index: 1,
                    mps: true,
                },
            )
            .unwrap();
        let carried = MqContext {
            state_index: 2,
            mps: true,
        };
        banks
            .mq_contexts_mut()
            .set(INTEGER_CONTEXT_COUNT - 1, carried)
            .unwrap();
        banks
            .mq_contexts_mut()
            .set(INTEGER_CONTEXT_COUNT, carried)
            .unwrap();
        banks.reset_integer_contexts().unwrap();
        assert_eq!(banks.mq_contexts_mut().get(1), Some(MqContext::default()));
        assert_eq!(
            banks.mq_contexts_mut().get(INTEGER_CONTEXT_COUNT - 1),
            Some(MqContext::default())
        );
        assert_eq!(
            banks.mq_contexts_mut().get(INTEGER_CONTEXT_COUNT),
            Some(carried)
        );
        banks.reset_all();
        assert_eq!(
            banks.mq_contexts_mut().get(INTEGER_CONTEXT_COUNT),
            Some(MqContext::default())
        );
        // The unscoped alias clears both integer banks and appended models.
        for index in [1, INTEGER_CONTEXT_COUNT + 6] {
            banks.mq_contexts_mut().set(index, carried).unwrap();
        }
        banks.reset();
        for index in [1, INTEGER_CONTEXT_COUNT + 6] {
            assert_eq!(
                banks.mq_contexts_mut().get(index),
                Some(MqContext::default())
            );
        }
        let tight = MqBudget {
            max_contexts: INTEGER_CONTEXT_COUNT - 1,
            ..budget
        };
        assert!(matches!(
            IntegerContextBanks::new(&limits, &tight).unwrap_err().kind,
            MqErrorKind::LimitExceeded {
                resource: "MQ contexts",
                ..
            }
        ));
        assert!(matches!(
            IntegerContextBanks::with_extra_contexts(usize::MAX, &limits, &budget)
                .unwrap_err()
                .kind,
            MqErrorKind::InvalidContext
        ));
    }

    #[test]
    fn real_mq_stream_shares_contexts_and_finishes_only_after_integers() {
        let limits = Limits::default();
        let budget = MqBudget::default();
        let table = invented_table(&limits);
        let mut banks = IntegerContextBanks::new(&limits, &budget).unwrap();
        let bytes = vec![0x80, 0, 0, 0, 0, 0, 0xff, 0xac];
        let mut source = vec_source(&bytes);
        let mut decoder = ready(MqDecoder::new(
            &mut source,
            super::super::mq::MqSpan {
                offset: 0,
                length: bytes.len() as u64,
            },
            &table,
            banks.mq_contexts_mut(),
            &limits,
            &NeverCancel,
            budget,
        ))
        .unwrap();
        let first = ready(decode_integer(&mut decoder, IntegerProcedure::Iadw)).unwrap();
        let after_first = decoder.context(IntegerProcedure::Iadw.base() + 1).unwrap();
        let second = ready(decode_integer(&mut decoder, IntegerProcedure::Iadw)).unwrap();
        let after_second = decoder.context(IntegerProcedure::Iadw.base() + 1).unwrap();
        let other_before = decoder.context(IntegerProcedure::Iadh.base() + 1).unwrap();
        let _third = ready(decode_integer(&mut decoder, IntegerProcedure::Iadh)).unwrap();
        let symbols = decoder.snapshot().symbols_decoded;
        assert_eq!(
            (first, second),
            (
                IntegerValue::Signed(4_294_971_731),
                IntegerValue::Signed(-4_026_536_276)
            )
        );
        assert_ne!(after_first, super::super::mq::MqContext::default());
        assert_ne!(after_first, after_second);
        assert_eq!(other_before, super::super::mq::MqContext::default());
        assert_ne!(
            decoder.context(IntegerProcedure::Iadh.base() + 1),
            Some(other_before)
        );
        assert_eq!(
            decoder.context(IntegerProcedure::Iadw.base() + 1),
            Some(after_second)
        );
        ready(decoder.finish(symbols)).unwrap();
    }

    #[test]
    fn missing_last_bank_is_rejected_before_any_mq_decision() {
        let limits = Limits::default();
        let budget = MqBudget::default();
        let table = invented_table(&limits);
        let mut contexts = MqContexts::new(INTEGER_CONTEXT_COUNT - 1, &limits, &budget).unwrap();
        let bytes = [0x80, 0, 0xff, 0xac];
        let mut source = vec_source(&bytes);
        let mut decoder = ready(MqDecoder::new(
            &mut source,
            super::super::mq::MqSpan {
                offset: 0,
                length: bytes.len() as u64,
            },
            &table,
            &mut contexts,
            &limits,
            &NeverCancel,
            budget,
        ))
        .unwrap();
        let before = decoder.snapshot();
        let error = ready(decode_integer(&mut decoder, IntegerProcedure::Iaai)).unwrap_err();
        assert!(matches!(error.kind, MqErrorKind::InvalidContext));
        assert_eq!(error.context, Some(INTEGER_CONTEXT_COUNT - 1));
        assert_eq!(decoder.snapshot(), before);
        ready(decoder.finish(0)).unwrap();
    }

    struct Flag(Rc<Cell<bool>>);
    impl Cancellation for Flag {
        fn is_cancelled(&self) -> bool {
            self.0.get()
        }
    }

    #[test]
    fn real_mq_budget_cancel_and_marker_errors_remain_visible() {
        let limits = Limits::default();
        let table = invented_table(&limits);
        let bytes = [0x80, 0, 0, 0xff, 0xac];
        let budget = MqBudget {
            max_symbols: 3,
            ..MqBudget::default()
        };
        let mut banks = IntegerContextBanks::new(&limits, &budget).unwrap();
        let mut source = vec_source(&bytes);
        let mut decoder = ready(MqDecoder::new(
            &mut source,
            super::super::mq::MqSpan {
                offset: 0,
                length: bytes.len() as u64,
            },
            &table,
            banks.mq_contexts_mut(),
            &limits,
            &NeverCancel,
            budget,
        ))
        .unwrap();
        let error = ready(decode_integer(&mut decoder, IntegerProcedure::Iaai)).unwrap_err();
        assert!(matches!(
            error.kind,
            MqErrorKind::LimitExceeded {
                resource: "MQ symbols",
                ..
            }
        ));
        assert_eq!(decoder.snapshot().symbols_decoded, 3);

        let cancelled = Rc::new(Cell::new(false));
        let flag = Flag(cancelled.clone());
        let budget = MqBudget::default();
        let mut banks = IntegerContextBanks::new(&limits, &budget).unwrap();
        let mut source = vec_source(&bytes);
        let mut decoder = ready(MqDecoder::new(
            &mut source,
            super::super::mq::MqSpan {
                offset: 0,
                length: bytes.len() as u64,
            },
            &table,
            banks.mq_contexts_mut(),
            &limits,
            &flag,
            budget,
        ))
        .unwrap();
        cancelled.set(true);
        let error = ready(decode_integer(&mut decoder, IntegerProcedure::Iaai)).unwrap_err();
        assert!(matches!(error.kind, MqErrorKind::Cancelled));
        assert_eq!(decoder.snapshot().symbols_decoded, 0);

        let invalid = [
            0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0x90,
        ];
        let mut banks = IntegerContextBanks::new(&limits, &budget).unwrap();
        let mut source = vec_source(&invalid);
        let mut decoder = ready(MqDecoder::new(
            &mut source,
            super::super::mq::MqSpan {
                offset: 0,
                length: invalid.len() as u64,
            },
            &table,
            banks.mq_contexts_mut(),
            &limits,
            &NeverCancel,
            budget,
        ))
        .unwrap();
        let _ = ready(decode_integer(&mut decoder, IntegerProcedure::Iaai)).unwrap();
        let symbols = decoder.snapshot().symbols_decoded;
        let error = ready(decoder.finish(symbols)).unwrap_err();
        assert!(matches!(error.kind, MqErrorKind::InvalidMarker(0x90)));

        let short = [0x80, 0];
        let mut banks = IntegerContextBanks::new(&limits, &budget).unwrap();
        let mut source = vec_source(&short);
        let mut decoder = ready(MqDecoder::new(
            &mut source,
            super::super::mq::MqSpan {
                offset: 0,
                length: short.len() as u64,
            },
            &table,
            banks.mq_contexts_mut(),
            &limits,
            &NeverCancel,
            budget,
        ))
        .unwrap();
        let error = ready(decode_integer(&mut decoder, IntegerProcedure::Iaai)).unwrap_err();
        assert!(matches!(error.kind, MqErrorKind::MissingTerminator));
        assert_eq!(error.offset, Some(short.len() as u64));
    }
}
