// SPDX-License-Identifier: MIT

//! Experimental, bounded T.82 arithmetic decoder for an already isolated SCD.
//!
//! The decoder uses the standard probability states in [`STANDARD_STATES`].
//! No official test vectors, CAJ framing, or image prediction are included
//! in this module. Its input span contains arithmetic bytes after any
//! container framing and byte unstuffing have been handled by the caller.
//! The context bank, errors, snapshot, counters, and input refill are shared
//! with the T.88 MQ decoder in [`crate::arith`].

mod standard;
pub use standard::STANDARD_STATES;

pub use crate::arith::{
    ArithmeticError, ArithmeticErrorKind, ArithmeticResult, ArithmeticSnapshot, CodedSpan, Coder,
    ContextBank, ContextState,
};

use crate::arith::{Counters, INPUT_BUFFER_BYTES, InputBuffer, check_span, valid_counts};
use crate::{Cancellation, Limits, RangedSource};

/// Number of probability-estimation states in T.82 Table 24.
pub const QM_STATE_COUNT: usize = 113;

/// One probability-estimation state, in T.82 Table 24 column order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QmState {
    pub qe: u16,
    pub next_lps: u8,
    pub next_mps: u8,
    pub switch_mps: bool,
}

/// The standard T.82 Table 24 state machine, [`STANDARD_STATES`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct QmTable(());

impl QmTable {
    pub const fn standard() -> Self {
        Self(())
    }

    fn get(&self, index: u8) -> QmState {
        STANDARD_STATES[usize::from(index)]
    }
}

/// Per-stripe bounds. Work counts each symbol, renormalization shift, and
/// byte input. Each byte input can cause at most one bounded 256-byte refill.
///
/// Both fields must be in `1..=MAX_BUDGET_COUNT`; other values are rejected
/// as `InvalidBudget` before any I/O.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArithmeticBudget {
    /// In `1..=`[`MAX_BUDGET_COUNT`](crate::MAX_BUDGET_COUNT).
    pub max_symbols: u64,
    /// In `1..=`[`MAX_BUDGET_COUNT`](crate::MAX_BUDGET_COUNT).
    pub max_work: u64,
}

/// Incremental decoder for one already isolated arithmetic stripe.
pub struct ArithmeticDecoder<'a, S: RangedSource, C: Cancellation> {
    source: &'a mut S,
    span: CodedSpan,
    table: &'a QmTable,
    contexts: &'a mut ContextBank,
    limits: &'a Limits,
    cancellation: &'a C,
    budget: ArithmeticBudget,
    input: InputBuffer,
    physical_bytes_consumed: u64,
    counters: Counters,
    interval: u32,
    code: u32,
    bit_counter: u8,
}

impl<'a, S: RangedSource, C: Cancellation> ArithmeticDecoder<'a, S, C> {
    /// Initialize C with three byte-input events. Bytes beyond the declared
    /// span are virtual zero; a short read *inside* the span is an error.
    /// Every stripe starts from reset contexts.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source: &'a mut S,
        span: CodedSpan,
        table: &'a QmTable,
        contexts: &'a mut ContextBank,
        limits: &'a Limits,
        cancellation: &'a C,
        budget: ArithmeticBudget,
    ) -> ArithmeticResult<Self> {
        limits.validate().map_err(|source| {
            ArithmeticError::configuration(Coder::T82, ArithmeticErrorKind::Source(source))
        })?;
        check_span(Coder::T82, span, source.size(), limits)?;
        if !valid_counts(budget.max_symbols, budget.max_work) {
            return Err(ArithmeticError::configuration(
                Coder::T82,
                ArithmeticErrorKind::InvalidBudget,
            ));
        }
        contexts.reset();
        let mut decoder = Self {
            source,
            span,
            table,
            contexts,
            limits,
            cancellation,
            budget,
            input: InputBuffer::new(),
            physical_bytes_consumed: 0,
            counters: Counters::default(),
            interval: 0x10000,
            code: 0,
            bit_counter: 0,
        };
        decoder.byte_in(None)?;
        decoder.code <<= 8;
        decoder.byte_in(None)?;
        decoder.code <<= 8;
        decoder.byte_in(None)?;
        Ok(decoder)
    }

    /// Decode one bit for a checked context. Any error after work starts
    /// poisons this stripe so a partial register update cannot be reused.
    pub fn decode_symbol(&mut self, context: usize) -> ArithmeticResult<bool> {
        if self.counters.poisoned {
            return Err(self.at(Some(context), ArithmeticErrorKind::Poisoned));
        }
        if self.contexts.get(context).is_none() {
            return Err(self.at(Some(context), ArithmeticErrorKind::InvalidContext));
        }
        self.check_cancelled(Some(context))?;
        let symbols = self
            .counters
            .next_symbol(self.budget.max_symbols)
            .map_err(|kind| self.at(Some(context), kind))?;
        // A caller may drop this future while source I/O is pending. Mark the
        // partially advanced registers unusable before the first await.
        self.counters.poisoned = true;
        let bit = self.decode_symbol_inner(context)?;
        self.counters.complete_symbol(symbols);
        Ok(bit)
    }

    /// Observe registers without mutating decoder, source, or contexts.
    pub fn snapshot(&self) -> ArithmeticSnapshot {
        self.counters.snapshot(
            self.interval,
            self.code,
            self.bit_counter,
            self.span.offset + self.physical_bytes_consumed,
        )
    }

    /// Read one context while the decoder holds the bank's mutable borrow.
    pub fn context_state(&self, index: usize) -> Option<ContextState> {
        self.contexts.get(index)
    }

    /// Check that exactly `expected_symbols` were decoded.
    pub fn finish(self, expected_symbols: u64) -> ArithmeticResult<()> {
        if self.counters.poisoned {
            return Err(self.at(None, ArithmeticErrorKind::Poisoned));
        }
        self.check_cancelled(None)?;
        if self.counters.symbols_decoded != expected_symbols {
            return Err(self.at(
                None,
                ArithmeticErrorKind::SymbolCount {
                    expected: expected_symbols,
                    decoded: self.counters.symbols_decoded,
                },
            ));
        }
        Ok(())
    }

    fn at(&self, context: Option<usize>, kind: ArithmeticErrorKind) -> ArithmeticError {
        ArithmeticError {
            coder: Some(Coder::T82),
            offset: Some(self.span.offset + self.physical_bytes_consumed),
            context,
            kind,
        }
    }

    fn check_cancelled(&self, context: Option<usize>) -> ArithmeticResult<()> {
        if self.cancellation.is_cancelled() {
            Err(self.at(context, ArithmeticErrorKind::Cancelled))
        } else {
            Ok(())
        }
    }

    fn charge(&mut self, context: Option<usize>) -> ArithmeticResult<()> {
        self.counters
            .charge(1, self.budget.max_work)
            .map_err(|kind| self.at(context, kind))
    }

    fn next_byte(&mut self, context: Option<usize>) -> ArithmeticResult<u8> {
        self.check_cancelled(context)?;
        let relative = self.physical_bytes_consumed;
        if relative == self.span.length {
            // Only `byte_in` reads bytes, after charging one unit of work,
            // so this count stays below `max_work <= MAX_BUDGET_COUNT`.
            self.counters.synthesized_inputs += 1;
            return Ok(0);
        }
        let byte = match self.input.get(relative) {
            Some(byte) => byte,
            None => {
                let count = (self.span.length - relative)
                    .min(INPUT_BUFFER_BYTES as u64)
                    .min(self.limits.io_chunk_bytes as u64) as usize;
                self.input
                    .refill(
                        self.source,
                        self.span,
                        relative,
                        count,
                        &mut self.counters.source_bytes_fetched,
                        self.limits,
                        self.cancellation,
                    )
                    .map_err(|(offset, kind)| ArithmeticError {
                        coder: Some(Coder::T82),
                        offset: Some(offset),
                        context,
                        kind,
                    })?;
                self.input.get(relative).expect("refilled byte")
            }
        };
        self.physical_bytes_consumed += 1;
        Ok(byte)
    }

    fn byte_in(&mut self, context: Option<usize>) -> ArithmeticResult<()> {
        self.charge(context)?;
        let byte = self.next_byte(context)?;
        self.code = self.code.wrapping_add(u32::from(byte) << 8);
        self.bit_counter = 8;
        Ok(())
    }

    fn renormalize(&mut self, context: usize) -> ArithmeticResult<()> {
        loop {
            self.check_cancelled(Some(context))?;
            if self.bit_counter == 0 {
                self.byte_in(Some(context))?;
            }
            self.charge(Some(context))?;
            self.interval <<= 1;
            self.code <<= 1;
            self.bit_counter -= 1;
            if self.interval >= 0x8000 {
                break;
            }
        }
        if self.bit_counter == 0 {
            self.byte_in(Some(context))?;
        }
        Ok(())
    }

    fn decode_symbol_inner(&mut self, context: usize) -> ArithmeticResult<bool> {
        self.charge(Some(context))?;
        let current = self.contexts.state(context);
        let state = self.table.get(current.state_index);
        let qe = u32::from(state.qe);
        // A symbol starts with `interval` in 0x8000..=0x10000: `new` sets
        // 0x10000, an MPS without renormalization keeps at least 0x8000,
        // renormalization doubles it until it reaches 0x8000, and a failed
        // renormalization poisons the decoder before another symbol. Every
        // standard `qe` is below 0x8000.
        debug_assert!((0x8000..=0x10000).contains(&self.interval) && qe < 0x8000);
        let narrowed = self.interval - qe;
        self.interval = narrowed;
        let high = self.code >> 16;
        let (bit, next) = if high < narrowed {
            if narrowed < 0x8000 {
                let exchange = narrowed < qe;
                let next = if exchange {
                    ContextState {
                        state_index: state.next_lps,
                        mps: current.mps ^ state.switch_mps,
                    }
                } else {
                    ContextState {
                        state_index: state.next_mps,
                        mps: current.mps,
                    }
                };
                self.renormalize(context)?;
                (current.mps ^ exchange, next)
            } else {
                (current.mps, current)
            }
        } else {
            // Here `narrowed <= code >> 16`, so the subtraction cannot wrap.
            self.code -= narrowed << 16;
            self.interval = qe;
            let exchange = narrowed >= qe;
            let next = if exchange {
                ContextState {
                    state_index: state.next_lps,
                    mps: current.mps ^ state.switch_mps,
                }
            } else {
                ContextState {
                    state_index: state.next_mps,
                    mps: current.mps,
                }
            };
            self.renormalize(context)?;
            (current.mps ^ exchange, next)
        };
        self.contexts.update(context, next);
        Ok(bit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Error, MAX_BUDGET_COUNT};
    use std::{cell::Cell, rc::Rc};

    #[derive(Debug)]
    struct MockSource {
        bytes: Vec<u8>,
        advertised_size: u64,
        max_read: usize,
        calls: usize,
        cancel_at: Option<(u64, Rc<Cell<bool>>)>,
        overreport_at: Option<u64>,
    }

    impl MockSource {
        fn new(bytes: &[u8]) -> Self {
            Self {
                bytes: bytes.to_vec(),
                advertised_size: bytes.len() as u64,
                max_read: usize::MAX,
                calls: 0,
                cancel_at: None,
                overreport_at: None,
            }
        }
    }

    impl RangedSource for MockSource {
        fn size(&self) -> u64 {
            self.advertised_size
        }

        fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> crate::Result<usize> {
            self.calls += 1;
            if self.overreport_at == Some(offset) {
                return Ok(destination.len() + 1);
            }
            let start = usize::try_from(offset).unwrap_or(usize::MAX);
            let available = self.bytes.len().saturating_sub(start);
            let count = available.min(destination.len()).min(self.max_read);
            if count > 0 {
                destination[..count].copy_from_slice(&self.bytes[start..start + count]);
            }
            if let Some((cancel_at, flag)) = &self.cancel_at
                && offset == *cancel_at
            {
                flag.set(true);
            }
            Ok(count)
        }
    }

    /// The one cancellation type of these tests, so every decoder path
    /// shares one instantiation; [`NEVER`] never trips.
    struct Flag(Option<Rc<Cell<bool>>>);

    const NEVER: Flag = Flag(None);

    impl Cancellation for Flag {
        fn is_cancelled(&self) -> bool {
            self.0.as_ref().is_some_and(|flag| flag.get())
        }
    }

    fn budget() -> ArithmeticBudget {
        ArithmeticBudget {
            max_symbols: 1000,
            max_work: 100_000,
        }
    }

    fn span(length: u64) -> CodedSpan {
        CodedSpan { offset: 0, length }
    }

    #[test]
    fn validates_spans_and_budgets_before_input() {
        let limits = Limits::default();
        let table = QmTable::standard();
        let mut contexts = ContextBank::new(2, &limits).unwrap();
        let mut source = MockSource::new(&[0, 0, 0]);
        let invalid_span = ArithmeticDecoder::new(
            &mut source,
            CodedSpan {
                offset: u64::MAX,
                length: 2,
            },
            &table,
            &mut contexts,
            &limits,
            &NEVER,
            budget(),
        );
        assert!(matches!(
            invalid_span.err().unwrap().kind,
            ArithmeticErrorKind::InvalidSpan(_)
        ));
        assert_eq!(source.calls, 0);
        let mut overreport = MockSource::new(&[0, 0, 0]);
        overreport.overreport_at = Some(0);
        let error = ArithmeticDecoder::new(
            &mut overreport,
            span(3),
            &table,
            &mut contexts,
            &limits,
            &NEVER,
            budget(),
        )
        .err()
        .unwrap();
        assert_eq!(error.offset, Some(0));
        assert!(matches!(
            error.kind,
            ArithmeticErrorKind::Source(Error::InvalidInput { .. })
        ));
        assert_eq!(overreport.calls, 1);
        let mut small_input = limits;
        small_input.max_input_bytes = 2;
        let input_limit = ArithmeticDecoder::new(
            &mut source,
            CodedSpan {
                offset: 1,
                length: 3,
            },
            &table,
            &mut contexts,
            &small_input,
            &NEVER,
            budget(),
        )
        .err()
        .unwrap();
        assert_eq!(input_limit.offset, Some(1));
        assert!(matches!(
            input_limit.kind,
            ArithmeticErrorKind::Source(Error::LimitExceeded { .. })
        ));
        assert_eq!(source.calls, 0);
        let invalid_budget = ArithmeticDecoder::new(
            &mut source,
            span(3),
            &table,
            &mut contexts,
            &limits,
            &NEVER,
            ArithmeticBudget {
                max_symbols: 0,
                max_work: 1,
            },
        );
        assert!(matches!(
            invalid_budget.err().unwrap().kind,
            ArithmeticErrorKind::InvalidBudget
        ));
        for (max_symbols, max_work) in [(MAX_BUDGET_COUNT + 1, 1), (1, MAX_BUDGET_COUNT + 1)] {
            let above_ceiling = ArithmeticDecoder::new(
                &mut source,
                span(3),
                &table,
                &mut contexts,
                &limits,
                &NEVER,
                ArithmeticBudget {
                    max_symbols,
                    max_work,
                },
            );
            assert!(matches!(
                above_ceiling.err().unwrap().kind,
                ArithmeticErrorKind::InvalidBudget
            ));
        }
        assert_eq!(source.calls, 0);
        ArithmeticDecoder::new(
            &mut source,
            span(3),
            &table,
            &mut contexts,
            &limits,
            &NEVER,
            ArithmeticBudget {
                max_symbols: MAX_BUDGET_COUNT,
                max_work: MAX_BUDGET_COUNT,
            },
        )
        .unwrap();
    }

    #[test]
    fn adapted_context_survives_rejected_spans_without_reading_or_resetting() {
        let table = QmTable::standard();
        let limits = Limits::default();
        let mut contexts = ContextBank::new(2, &limits).unwrap();
        // An LPS in context 1 switches its MPS and moves it to state 1.
        let mut source = MockSource::new(&[0xc0, 0, 0]);
        let mut decoder = ArithmeticDecoder::new(
            &mut source,
            span(3),
            &table,
            &mut contexts,
            &limits,
            &NEVER,
            budget(),
        )
        .unwrap();
        assert!(decoder.decode_symbol(1).unwrap());
        decoder.finish(1).unwrap();
        let adapted = ContextState {
            state_index: 1,
            mps: true,
        };
        assert_eq!(contexts.get(1), Some(adapted));

        let mut source = MockSource::new(&[0, 0, 0]);
        for span in [
            CodedSpan {
                offset: 2,
                length: 2,
            },
            CodedSpan {
                offset: 4,
                length: 0,
            },
        ] {
            let error = ArithmeticDecoder::new(
                &mut source,
                span,
                &table,
                &mut contexts,
                &limits,
                &NEVER,
                budget(),
            )
            .err()
            .unwrap();
            assert_eq!(error.offset, Some(span.offset));
            assert!(matches!(
                error.kind,
                ArithmeticErrorKind::InvalidSpan("outside source size")
            ));
            assert_eq!(
                error.to_string(),
                format!(
                    "T.82 arithmetic decoder at source byte {}: invalid span: outside source size",
                    span.offset
                )
            );
            assert_eq!(source.calls, 0);
            assert_eq!(contexts.get(1), Some(adapted));
        }

        contexts.reset();
        assert_eq!(contexts.get(1), Some(ContextState::default()));
    }

    #[test]
    fn arithmetic_errors_keep_source_causes_and_actionable_locations() {
        let table = QmTable::standard();
        let limits = Limits::default();
        let mut contexts = ContextBank::new(1, &limits).unwrap();

        let mut short = MockSource::new(&[0, 0]);
        short.advertised_size = 3;
        let source_error = ArithmeticDecoder::new(
            &mut short,
            span(3),
            &table,
            &mut contexts,
            &limits,
            &NEVER,
            budget(),
        )
        .err()
        .unwrap();
        assert_eq!(source_error.offset, Some(2));
        assert!(matches!(
            &source_error.kind,
            ArithmeticErrorKind::Source(Error::TruncatedInput {
                offset: 0,
                expected: 3,
                available: 2,
            })
        ));
        let cause = std::error::Error::source(&source_error).unwrap();
        assert_eq!(
            cause.to_string(),
            "truncated input at offset 0: needed 3 bytes, got 2"
        );
        assert_eq!(
            source_error.to_string(),
            format!("T.82 arithmetic decoder at source byte 2: source error: {cause}")
        );

        let mut source = MockSource::new(&[0, 0, 0]);
        let mut decoder = ArithmeticDecoder::new(
            &mut source,
            span(3),
            &table,
            &mut contexts,
            &limits,
            &NEVER,
            ArithmeticBudget {
                max_symbols: 1,
                max_work: 100,
            },
        )
        .unwrap();
        let invalid_context = decoder.decode_symbol(1).unwrap_err();
        assert_eq!(
            invalid_context.to_string(),
            "T.82 arithmetic decoder at source byte 3, context 1: invalid context index or count"
        );
        assert!(std::error::Error::source(&invalid_context).is_none());
        assert!(!decoder.decode_symbol(0).unwrap());
        let limit = decoder.decode_symbol(0).unwrap_err();
        assert_eq!(
            limit.to_string(),
            "T.82 arithmetic decoder at source byte 3, context 0: symbols limit 1 exceeded by 2"
        );
        assert!(std::error::Error::source(&limit).is_none());
        let incomplete = decoder.finish(2).unwrap_err();
        assert_eq!(
            incomplete.to_string(),
            "T.82 arithmetic decoder at source byte 3: expected 2 symbols, decoded 1"
        );
    }

    #[test]
    fn initializes_registers_with_checked_short_reads_and_virtual_zeros() {
        let table = QmTable::standard();
        let limits = Limits {
            io_chunk_bytes: 2,
            ..Limits::default()
        };
        let mut contexts = ContextBank::new(1, &limits).unwrap();
        let mut source = MockSource::new(&[0x12, 0x34, 0x56, 0x78]);
        source.max_read = 1;
        let decoder = ArithmeticDecoder::new(
            &mut source,
            span(3),
            &table,
            &mut contexts,
            &limits,
            &NEVER,
            budget(),
        )
        .unwrap();
        let snapshot = decoder.snapshot();
        assert_eq!(snapshot.interval, 0x10000);
        assert_eq!(snapshot.code, 0x12345600);
        assert_eq!(snapshot.bit_counter, 8);
        assert_eq!(snapshot.synthesized_inputs, 0);
        assert_eq!(snapshot.input_offset, 3);
        decoder.finish(0).unwrap();
        assert_eq!(source.calls, 3);

        let mut empty = MockSource::new(&[0xff]);
        let decoder = ArithmeticDecoder::new(
            &mut empty,
            span(0),
            &table,
            &mut contexts,
            &limits,
            &NEVER,
            budget(),
        )
        .unwrap();
        assert_eq!(decoder.snapshot().code, 0);
        assert_eq!(decoder.snapshot().synthesized_inputs, 3);
        assert_eq!(decoder.snapshot().input_offset, 0);
        decoder.finish(0).unwrap();
        assert_eq!(empty.calls, 0);

        let mut short = MockSource::new(&[0xaa, 0xbb]);
        short.advertised_size = 3;
        let error = ArithmeticDecoder::new(
            &mut short,
            span(3),
            &table,
            &mut contexts,
            &limits,
            &NEVER,
            budget(),
        )
        .err()
        .unwrap();
        assert_eq!(error.offset, Some(2));
        assert!(matches!(
            error.kind,
            ArithmeticErrorKind::Source(Error::TruncatedInput { .. })
        ));
    }

    #[test]
    fn mps_lps_and_conditional_exchanges_update_contexts() {
        // Hand-derived from T.82 Table 24: state 0 has Qe 0x5A1D, NMPS and
        // NLPS 1 and SWITCH; state 1 has Qe 0x2586 and NMPS 2.
        let table = QmTable::standard();
        let limits = Limits::default();
        let mut contexts = ContextBank::new(1, &limits).unwrap();
        let mut decode = |bytes: &[u8], symbols: usize| {
            let mut source = MockSource::new(bytes);
            let mut decoder = ArithmeticDecoder::new(
                &mut source,
                span(3),
                &table,
                &mut contexts,
                &limits,
                &NEVER,
                budget(),
            )
            .unwrap();
            let mut trace = Vec::new();
            for _ in 0..symbols {
                let bit = decoder.decode_symbol(0).unwrap();
                let snapshot = decoder.snapshot();
                trace.push((
                    bit,
                    snapshot.interval,
                    snapshot.bit_counter,
                    decoder.context_state(0).unwrap(),
                ));
            }
            decoder.finish(symbols as u64).unwrap();
            trace
        };
        let state = |state_index, mps| ContextState { state_index, mps };
        // A zero code stays in the lower subinterval: an MPS that keeps
        // A = 0xA5E3 unrenormalized, then A - Qe = 0x4BC6 < Qe exchanges
        // that subinterval to the LPS, which switches the MPS; the third
        // symbol is the new MPS in state 1.
        assert_eq!(
            decode(&[0, 0, 0], 3),
            [
                (false, 0xa5e3, 8, state(0, false)),
                (true, 0x978c, 7, state(1, true)),
                (true, 0xe40c, 6, state(2, true)),
            ]
        );
        // C = 0xC000 lies in the upper subinterval: an ordinary LPS. The next
        // symbol is an MPS that needs no renormalization.
        assert_eq!(
            decode(&[0xc0, 0, 0], 2),
            [
                (true, 0xb43a, 7, state(1, true)),
                (true, 0x8eb4, 7, state(1, true)),
            ]
        );
        // C = 0x8000 is an MPS first, then lies in the upper subinterval
        // of an exchanged split: an MPS that renormalizes.
        assert_eq!(
            decode(&[0x80, 0, 0], 2),
            [
                (false, 0xa5e3, 8, state(0, false)),
                (false, 0xb43a, 7, state(1, false)),
            ]
        );
    }

    #[test]
    fn fixed_width_register_operations_and_small_work_budget_are_explicit() {
        let table = QmTable::standard();
        let limits = Limits {
            io_chunk_bytes: 1,
            ..Limits::default()
        };
        let mut contexts = ContextBank::new(1, &limits).unwrap();
        let mut source = MockSource::new(&[0, 0, 0, 0xff]);
        let mut decoder = ArithmeticDecoder::new(
            &mut source,
            span(4),
            &table,
            &mut contexts,
            &limits,
            &NEVER,
            budget(),
        )
        .unwrap();
        decoder.code = 0xffff_ff00;
        decoder.byte_in(None).unwrap();
        assert_eq!(decoder.snapshot().code, 0x0000_fe00);
        assert_eq!(decoder.snapshot().input_offset, 4);
        decoder.interval = 0x7fff;
        decoder.bit_counter = 8;
        decoder.renormalize(0).unwrap();
        assert_eq!(decoder.snapshot().interval, 0xfffe);
        assert_eq!(decoder.snapshot().bit_counter, 7);
        decoder.budget.max_symbols = MAX_BUDGET_COUNT;
        decoder.counters.symbols_decoded = MAX_BUDGET_COUNT;
        assert!(matches!(
            decoder.decode_symbol(0).unwrap_err().kind,
            ArithmeticErrorKind::LimitExceeded {
                resource: "symbols",
                limit: MAX_BUDGET_COUNT,
                attempted,
            } if attempted == MAX_BUDGET_COUNT + 1
        ));
        assert!(!decoder.snapshot().poisoned);
        decoder.counters.symbols_decoded = 0;
        decoder.budget.max_work = MAX_BUDGET_COUNT;
        decoder.counters.work_done = MAX_BUDGET_COUNT;
        assert!(matches!(
            decoder.charge(None).unwrap_err().kind,
            ArithmeticErrorKind::LimitExceeded {
                resource: "arithmetic work",
                limit: MAX_BUDGET_COUNT,
                attempted,
            } if attempted == MAX_BUDGET_COUNT + 1
        ));
        decoder.finish(0).unwrap();

        let broad_limits = Limits::default();
        let mut large_span = MockSource::new(&[0; INPUT_BUFFER_BYTES]);
        let mut decoder = ArithmeticDecoder::new(
            &mut large_span,
            span(INPUT_BUFFER_BYTES as u64),
            &table,
            &mut contexts,
            &broad_limits,
            &NEVER,
            ArithmeticBudget {
                max_symbols: 1,
                max_work: 4,
            },
        )
        .unwrap();
        assert!(!decoder.decode_symbol(0).unwrap());
        assert_eq!(decoder.snapshot().work_done, 4);
        let error = decoder.decode_symbol(0).unwrap_err();
        assert!(matches!(
            error.kind,
            ArithmeticErrorKind::LimitExceeded {
                resource: "symbols",
                limit: 1,
                attempted: 2,
            }
        ));
        assert!(!decoder.snapshot().poisoned);
        decoder.finish(1).unwrap();
        assert_eq!(large_span.calls, 1);
    }

    #[test]
    fn cancellation_and_physical_eof_during_a_symbol_poison_the_stripe() {
        let table = QmTable::standard();
        let limits = Limits {
            io_chunk_bytes: 1,
            ..Limits::default()
        };
        let mut contexts = ContextBank::new(1, &limits).unwrap();
        let cancelled = Rc::new(Cell::new(true));
        let flag = Flag(Some(cancelled.clone()));
        let mut before_start = MockSource::new(&[0, 0, 0]);
        let error = ArithmeticDecoder::new(
            &mut before_start,
            span(3),
            &table,
            &mut contexts,
            &limits,
            &flag,
            budget(),
        )
        .err()
        .unwrap();
        assert!(matches!(error.kind, ArithmeticErrorKind::Cancelled));
        assert_eq!(before_start.calls, 0);

        cancelled.set(false);
        let mut during_symbol = MockSource::new(&[0xff, 0xff, 0, 0x55]);
        during_symbol.cancel_at = Some((3, cancelled.clone()));
        let mut decoder = ArithmeticDecoder::new(
            &mut during_symbol,
            span(4),
            &table,
            &mut contexts,
            &limits,
            &flag,
            budget(),
        )
        .unwrap();
        // The fourth byte enters C during the renormalization of a later
        // symbol; the symbols before it complete.
        let (decoded, before, error) = decode_until_error(&mut decoder);
        assert_eq!(error.offset, Some(3));
        assert_eq!(error.context, Some(0));
        assert!(matches!(error.kind, ArithmeticErrorKind::Cancelled));
        assert!(decoder.snapshot().poisoned);
        assert_eq!(decoder.snapshot().symbols_decoded, decoded);
        assert_eq!(decoder.context_state(0), Some(before));
        assert!(matches!(
            decoder.decode_symbol(0).unwrap_err().kind,
            ArithmeticErrorKind::Poisoned
        ));
        assert!(matches!(
            decoder.finish(decoded + 1).unwrap_err().kind,
            ArithmeticErrorKind::Poisoned
        ));
        assert_eq!(during_symbol.calls, 4);

        let mut short = MockSource::new(&[0xff, 0xff, 0]);
        short.advertised_size = 4;
        let mut decoder = ArithmeticDecoder::new(
            &mut short,
            span(4),
            &table,
            &mut contexts,
            &limits,
            &NEVER,
            budget(),
        )
        .unwrap();
        let (decoded, _, error) = decode_until_error(&mut decoder);
        assert_eq!(error.offset, Some(3));
        assert_eq!(error.context, Some(0));
        assert!(matches!(
            error.kind,
            ArithmeticErrorKind::Source(Error::TruncatedInput { .. })
        ));
        assert!(decoder.snapshot().poisoned);
        assert!(matches!(
            decoder.decode_symbol(0).unwrap_err().kind,
            ArithmeticErrorKind::Poisoned
        ));
        assert!(matches!(
            decoder.finish(decoded + 1).unwrap_err().kind,
            ArithmeticErrorKind::Poisoned
        ));
    }

    /// Decode context zero until a symbol fails, returning the completed
    /// symbol count, the context state before the failing symbol, and its
    /// error. The fixtures fail within a few symbols.
    fn decode_until_error<S: RangedSource>(
        decoder: &mut ArithmeticDecoder<'_, S, Flag>,
    ) -> (u64, ContextState, ArithmeticError) {
        for decoded in 0..16 {
            let before = decoder.context_state(0).unwrap();
            if let Err(error) = decoder.decode_symbol(0) {
                return (decoded, before, error);
            }
        }
        panic!("the fixture never failed");
    }

    #[test]
    fn work_error_poisoning_and_invalid_context_preflight_are_distinct() {
        let table = QmTable::standard();
        let limits = Limits::default();
        let mut contexts = ContextBank::new(1, &limits).unwrap();
        let mut source = MockSource::new(&[0, 0, 0]);
        let mut decoder = ArithmeticDecoder::new(
            &mut source,
            span(3),
            &table,
            &mut contexts,
            &limits,
            &NEVER,
            budget(),
        )
        .unwrap();
        let error = decoder.decode_symbol(1).unwrap_err();
        assert_eq!(error.context, Some(1));
        assert!(matches!(error.kind, ArithmeticErrorKind::InvalidContext));
        assert!(!decoder.snapshot().poisoned);
        assert!(!decoder.decode_symbol(0).unwrap());
        decoder.finish(1).unwrap();

        let mut source = MockSource::new(&[0xc0, 0, 0]);
        let mut decoder = ArithmeticDecoder::new(
            &mut source,
            span(3),
            &table,
            &mut contexts,
            &limits,
            &NEVER,
            ArithmeticBudget {
                max_symbols: 1,
                max_work: 4,
            },
        )
        .unwrap();
        let error = decoder.decode_symbol(0).unwrap_err();
        assert_eq!(error.context, Some(0));
        assert!(matches!(
            error.kind,
            ArithmeticErrorKind::LimitExceeded {
                resource: "arithmetic work",
                limit: 4,
                attempted: 5,
            }
        ));
        assert!(decoder.snapshot().poisoned);
        assert_eq!(decoder.snapshot().symbols_decoded, 0);
        assert_eq!(decoder.context_state(0), Some(ContextState::default()));
    }

    #[test]
    fn finish_requires_the_exact_symbol_count_and_each_stripe_resets_contexts() {
        let table = QmTable::standard();
        let limits = Limits::default();
        let mut contexts = ContextBank::new(1, &limits).unwrap();
        let mut first = MockSource::new(&[0xc0, 0, 0]);
        let mut decoder = ArithmeticDecoder::new(
            &mut first,
            span(3),
            &table,
            &mut contexts,
            &limits,
            &NEVER,
            budget(),
        )
        .unwrap();
        assert!(decoder.decode_symbol(0).unwrap());
        assert!(matches!(
            decoder.finish(2).unwrap_err().kind,
            ArithmeticErrorKind::SymbolCount {
                expected: 2,
                decoded: 1,
            }
        ));

        let mut first = MockSource::new(&[0xc0, 0, 0]);
        let mut decoder = ArithmeticDecoder::new(
            &mut first,
            span(3),
            &table,
            &mut contexts,
            &limits,
            &NEVER,
            budget(),
        )
        .unwrap();
        assert!(decoder.decode_symbol(0).unwrap());
        decoder.finish(1).unwrap();
        assert_eq!(
            contexts.get(0),
            Some(ContextState {
                state_index: 1,
                mps: true,
            })
        );

        let mut second = MockSource::new(&[0, 0, 0]);
        let mut decoder = ArithmeticDecoder::new(
            &mut second,
            span(3),
            &table,
            &mut contexts,
            &limits,
            &NEVER,
            budget(),
        )
        .unwrap();
        assert_eq!(decoder.context_state(0), Some(ContextState::default()));
        assert!(!decoder.decode_symbol(0).unwrap());
        contexts.reset();
        assert_eq!(contexts.get(0), Some(ContextState::default()));
    }

    #[test]
    fn fixed_budget_mutations_are_deterministic_and_bounded() {
        fn trace(input: &[u8; 8]) -> (Vec<bool>, ArithmeticSnapshot) {
            let table = QmTable::standard();
            let limits = Limits::default();
            let mut contexts = ContextBank::new(2, &limits).unwrap();
            let mut source = MockSource::new(input);
            let mut decoder = ArithmeticDecoder::new(
                &mut source,
                span(input.len() as u64),
                &table,
                &mut contexts,
                &limits,
                &NEVER,
                ArithmeticBudget {
                    max_symbols: 64,
                    max_work: 512,
                },
            )
            .unwrap();
            let mut bits = Vec::new();
            for symbol in 0..64 {
                bits.push(decoder.decode_symbol(symbol % 2).unwrap());
            }
            let snapshot = decoder.snapshot();
            assert_eq!(snapshot.symbols_decoded, 64);
            assert!(snapshot.work_done <= 512);
            assert!(snapshot.input_offset <= input.len() as u64);
            decoder.finish(64).unwrap();
            (bits, snapshot)
        }

        let base = [0x13, 0x57, 0x9b, 0xdf, 0x24, 0x68, 0xac, 0xe0];
        let baseline = trace(&base);
        assert_eq!(trace(&base), baseline);
        let mut changed = false;
        for index in 0..base.len() {
            for bit in 0..8 {
                let mut mutant = base;
                mutant[index] ^= 1 << bit;
                let outcome = trace(&mutant);
                assert_eq!(outcome, trace(&mutant));
                changed |= outcome.0 != baseline.0;
            }
        }
        assert!(
            changed,
            "at least one input bit must affect the decoded symbols"
        );
    }

    #[test]
    fn configuration_and_state_errors_have_distinct_messages() {
        let limits = Limits::default();
        let table = QmTable::standard();
        let mut contexts = ContextBank::new(2, &limits).unwrap();
        let mut source = MockSource::new(&[0, 0, 0]);
        let zero_budget = ArithmeticDecoder::new(
            &mut source,
            span(3),
            &table,
            &mut contexts,
            &limits,
            &NEVER,
            ArithmeticBudget {
                max_symbols: 1,
                max_work: 0,
            },
        )
        .err()
        .unwrap();
        assert_eq!(
            zero_budget.to_string(),
            "T.82 arithmetic decoder: invalid budget"
        );
        let cancelled = ArithmeticDecoder::new(
            &mut source,
            span(3),
            &table,
            &mut contexts,
            &limits,
            &Flag(Some(Rc::new(Cell::new(true)))),
            budget(),
        )
        .err()
        .unwrap();
        assert_eq!(
            cancelled.to_string(),
            "T.82 arithmetic decoder at source byte 0: cancelled"
        );
        assert_eq!(source.calls, 0);

        let poisoned = ArithmeticError {
            coder: Some(Coder::T82),
            offset: Some(9),
            context: Some(4),
            kind: ArithmeticErrorKind::Poisoned,
        };
        assert_eq!(
            poisoned.to_string(),
            "T.82 arithmetic decoder at source byte 9, context 4: decoder is poisoned after an error"
        );
        assert!(std::error::Error::source(&poisoned).is_none());
    }
}
