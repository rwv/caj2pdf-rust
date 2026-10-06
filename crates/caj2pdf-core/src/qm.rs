// SPDX-License-Identifier: MIT

//! Experimental, bounded T.82 arithmetic decoder for an already isolated SCD.
//!
//! The decoder uses the standard probability states in [`STANDARD_STATES`].
//! No official test vectors, CAJ framing, or image prediction are included
//! in this module. Its input span contains arithmetic bytes after any
//! container framing and byte unstuffing have been handled by the caller.
//! The context bank, errors, snapshot and counters are shared with the T.88
//! MQ decoder in [`crate::arith`].

mod standard;
pub use standard::STANDARD_STATES;

pub use crate::arith::{
    ArithmeticError, ArithmeticErrorKind, ArithmeticResult, ArithmeticSnapshot, CodedSpan, Coder,
    ContextBank, ContextState,
};

use crate::arith::{Counters, check_span, valid_counts};
use crate::{Limits, Payload};

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
/// byte input.
///
/// Both fields must be in `1..=MAX_BUDGET_COUNT`; other values are rejected
/// as `InvalidBudget` before decoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArithmeticBudget {
    /// In `1..=`[`MAX_BUDGET_COUNT`](crate::MAX_BUDGET_COUNT).
    pub max_symbols: u64,
    /// In `1..=`[`MAX_BUDGET_COUNT`](crate::MAX_BUDGET_COUNT).
    pub max_work: u64,
}

/// Incremental decoder for one already isolated arithmetic stripe.
pub struct ArithmeticDecoder<'a> {
    data: &'a [u8],
    span: CodedSpan,
    table: &'a QmTable,
    contexts: &'a mut ContextBank,
    budget: ArithmeticBudget,
    physical_bytes_consumed: u64,
    counters: Counters,
    interval: u32,
    code: u32,
    bit_counter: u8,
}

impl<'a> ArithmeticDecoder<'a> {
    /// Initialize C with three byte-input events over the span's bytes in
    /// `input`. Bytes beyond the declared span are virtual zero. Every
    /// stripe starts from reset contexts.
    pub fn new(
        input: Payload<'a>,
        span: CodedSpan,
        table: &'a QmTable,
        contexts: &'a mut ContextBank,
        limits: &Limits,
        budget: ArithmeticBudget,
    ) -> ArithmeticResult<Self> {
        limits.validate().map_err(|source| {
            ArithmeticError::configuration(Coder::T82, ArithmeticErrorKind::Source(source))
        })?;
        let data = check_span(Coder::T82, span, input, limits)?;
        if !valid_counts(budget.max_symbols, budget.max_work) {
            return Err(ArithmeticError::configuration(
                Coder::T82,
                ArithmeticErrorKind::InvalidBudget,
            ));
        }
        contexts.reset();
        let mut decoder = Self {
            data,
            span,
            table,
            contexts,
            budget,
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

    /// Decode one bit for a checked context. After an error the stripe's
    /// registers are undefined; the caller must abandon it.
    pub fn decode_symbol(&mut self, context: usize) -> ArithmeticResult<bool> {
        if self.contexts.get(context).is_none() {
            return Err(self.at(Some(context), ArithmeticErrorKind::InvalidContext));
        }
        let symbols = self
            .counters
            .next_symbol(self.budget.max_symbols)
            .map_err(|kind| self.at(Some(context), kind))?;
        let bit = self.decode_symbol_inner(context)?;
        self.counters.symbols_decoded = symbols;
        Ok(bit)
    }

    /// Observe registers without mutating decoder or contexts.
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

    fn charge(&mut self, context: Option<usize>) -> ArithmeticResult<()> {
        self.counters
            .charge(self.budget.max_work)
            .map_err(|kind| self.at(context, kind))
    }

    fn next_byte(&mut self) -> u8 {
        let relative = self.physical_bytes_consumed;
        if relative == self.span.length {
            // Only `byte_in` reads bytes, after charging one unit of work,
            // so this count stays below `max_work <= MAX_BUDGET_COUNT`.
            self.counters.synthesized_inputs += 1;
            return 0;
        }
        self.physical_bytes_consumed += 1;
        // `check_span` proved that the span's bytes are all in `data`.
        self.data[relative as usize]
    }

    fn byte_in(&mut self, context: Option<usize>) -> ArithmeticResult<()> {
        self.charge(context)?;
        let byte = self.next_byte();
        self.code = self.code.wrapping_add(u32::from(byte) << 8);
        self.bit_counter = 8;
        Ok(())
    }

    fn renormalize(&mut self, context: usize) -> ArithmeticResult<()> {
        loop {
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
        // 0x10000, an MPS without renormalization keeps at least 0x8000, and
        // renormalization doubles it until it reaches 0x8000; a failed
        // renormalization ends the stripe. Every standard `qe` is below 0x8000.
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
    use crate::MAX_BUDGET_COUNT;

    fn budget() -> ArithmeticBudget {
        ArithmeticBudget {
            max_symbols: 1000,
            max_work: 100_000,
        }
    }

    fn span(length: u64) -> CodedSpan {
        CodedSpan { offset: 0, length }
    }

    /// A decoder over `bytes`, which start at source offset zero.
    fn decoder<'a>(
        bytes: &'a [u8],
        span: CodedSpan,
        contexts: &'a mut ContextBank,
        limits: &Limits,
        budget: ArithmeticBudget,
    ) -> ArithmeticResult<ArithmeticDecoder<'a>> {
        const TABLE: QmTable = QmTable::standard();
        ArithmeticDecoder::new(bytes.into(), span, &TABLE, contexts, limits, budget)
    }

    #[test]
    fn validates_spans_and_budgets_before_input() {
        let limits = Limits::default();
        let mut contexts = ContextBank::new(2, &limits).unwrap();
        let bytes = [0, 0, 0];
        let invalid_span = decoder(
            &bytes,
            CodedSpan {
                offset: u64::MAX,
                length: 2,
            },
            &mut contexts,
            &limits,
            budget(),
        );
        assert!(matches!(
            invalid_span.err().unwrap().kind,
            ArithmeticErrorKind::InvalidSpan(_)
        ));
        let mut small_input = limits;
        small_input.max_input_bytes = 2;
        let input_limit = decoder(
            &bytes,
            CodedSpan {
                offset: 1,
                length: 3,
            },
            &mut contexts,
            &small_input,
            budget(),
        )
        .err()
        .unwrap();
        assert_eq!(input_limit.offset, Some(1));
        assert!(matches!(
            input_limit.kind,
            ArithmeticErrorKind::Source(crate::Error::LimitExceeded { .. })
        ));
        let invalid_budget = decoder(
            &bytes,
            span(3),
            &mut contexts,
            &limits,
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
            let above_ceiling = decoder(
                &bytes,
                span(3),
                &mut contexts,
                &limits,
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
        decoder(
            &bytes,
            span(3),
            &mut contexts,
            &limits,
            ArithmeticBudget {
                max_symbols: MAX_BUDGET_COUNT,
                max_work: MAX_BUDGET_COUNT,
            },
        )
        .unwrap();
    }

    #[test]
    fn adapted_context_survives_rejected_spans_without_resetting() {
        let limits = Limits::default();
        let mut contexts = ContextBank::new(2, &limits).unwrap();
        // An LPS in context 1 switches its MPS and moves it to state 1.
        let mut stripe = decoder(&[0xc0, 0, 0], span(3), &mut contexts, &limits, budget()).unwrap();
        assert!(stripe.decode_symbol(1).unwrap());
        stripe.finish(1).unwrap();
        let adapted = ContextState {
            state_index: 1,
            mps: true,
        };
        assert_eq!(contexts.get(1), Some(adapted));

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
            let error = decoder(&[0, 0, 0], span, &mut contexts, &limits, budget())
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
            assert_eq!(contexts.get(1), Some(adapted));
        }

        contexts.reset();
        assert_eq!(contexts.get(1), Some(ContextState::default()));
    }

    #[test]
    fn arithmetic_errors_keep_actionable_locations() {
        let limits = Limits::default();
        let mut contexts = ContextBank::new(1, &limits).unwrap();
        let mut stripe = decoder(
            &[0, 0, 0],
            span(3),
            &mut contexts,
            &limits,
            ArithmeticBudget {
                max_symbols: 1,
                max_work: 100,
            },
        )
        .unwrap();
        let invalid_context = stripe.decode_symbol(1).unwrap_err();
        assert_eq!(
            invalid_context.to_string(),
            "T.82 arithmetic decoder at source byte 3, context 1: invalid context index or count"
        );
        assert!(std::error::Error::source(&invalid_context).is_none());
        assert!(!stripe.decode_symbol(0).unwrap());
        let limit = stripe.decode_symbol(0).unwrap_err();
        assert_eq!(
            limit.to_string(),
            "T.82 arithmetic decoder at source byte 3, context 0: symbols limit 1 exceeded by 2"
        );
        assert!(std::error::Error::source(&limit).is_none());
        let incomplete = stripe.finish(2).unwrap_err();
        assert_eq!(
            incomplete.to_string(),
            "T.82 arithmetic decoder at source byte 3: expected 2 symbols, decoded 1"
        );
    }

    #[test]
    fn initializes_registers_with_virtual_zeros() {
        let limits = Limits::default();
        let mut contexts = ContextBank::new(1, &limits).unwrap();
        let stripe = decoder(
            &[0x12, 0x34, 0x56, 0x78],
            span(3),
            &mut contexts,
            &limits,
            budget(),
        )
        .unwrap();
        let snapshot = stripe.snapshot();
        assert_eq!(snapshot.interval, 0x10000);
        assert_eq!(snapshot.code, 0x12345600);
        assert_eq!(snapshot.bit_counter, 8);
        assert_eq!(snapshot.synthesized_inputs, 0);
        assert_eq!(snapshot.input_offset, 3);
        stripe.finish(0).unwrap();

        let stripe = decoder(&[0xff], span(0), &mut contexts, &limits, budget()).unwrap();
        assert_eq!(stripe.snapshot().code, 0);
        assert_eq!(stripe.snapshot().synthesized_inputs, 3);
        assert_eq!(stripe.snapshot().input_offset, 0);
        stripe.finish(0).unwrap();
    }

    #[test]
    fn mps_lps_and_conditional_exchanges_update_contexts() {
        // Hand-derived from T.82 Table 24: state 0 has Qe 0x5A1D, NMPS and
        // NLPS 1 and SWITCH; state 1 has Qe 0x2586 and NMPS 2.
        let limits = Limits::default();
        let mut contexts = ContextBank::new(1, &limits).unwrap();
        let mut decode = |bytes: &[u8], symbols: usize| {
            let mut stripe = decoder(bytes, span(3), &mut contexts, &limits, budget()).unwrap();
            let mut trace = Vec::new();
            for _ in 0..symbols {
                let bit = stripe.decode_symbol(0).unwrap();
                let snapshot = stripe.snapshot();
                trace.push((
                    bit,
                    snapshot.interval,
                    snapshot.bit_counter,
                    stripe.context_state(0).unwrap(),
                ));
            }
            stripe.finish(symbols as u64).unwrap();
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
        let limits = Limits::default();
        let mut contexts = ContextBank::new(1, &limits).unwrap();
        let mut stripe =
            decoder(&[0, 0, 0, 0xff], span(4), &mut contexts, &limits, budget()).unwrap();
        stripe.code = 0xffff_ff00;
        stripe.byte_in(None).unwrap();
        assert_eq!(stripe.snapshot().code, 0x0000_fe00);
        assert_eq!(stripe.snapshot().input_offset, 4);
        stripe.interval = 0x7fff;
        stripe.bit_counter = 8;
        stripe.renormalize(0).unwrap();
        assert_eq!(stripe.snapshot().interval, 0xfffe);
        assert_eq!(stripe.snapshot().bit_counter, 7);
        stripe.budget.max_symbols = MAX_BUDGET_COUNT;
        stripe.counters.symbols_decoded = MAX_BUDGET_COUNT;
        assert!(matches!(
            stripe.decode_symbol(0).unwrap_err().kind,
            ArithmeticErrorKind::LimitExceeded {
                resource: "symbols",
                limit: MAX_BUDGET_COUNT,
                attempted,
            } if attempted == MAX_BUDGET_COUNT + 1
        ));
        stripe.counters.symbols_decoded = 0;
        stripe.budget.max_work = MAX_BUDGET_COUNT;
        stripe.counters.work_done = MAX_BUDGET_COUNT;
        assert!(matches!(
            stripe.charge(None).unwrap_err().kind,
            ArithmeticErrorKind::LimitExceeded {
                resource: "arithmetic work",
                limit: MAX_BUDGET_COUNT,
                attempted,
            } if attempted == MAX_BUDGET_COUNT + 1
        ));
        stripe.finish(0).unwrap();

        let large_span = [0; 256];
        let mut stripe = decoder(
            &large_span,
            span(256),
            &mut contexts,
            &limits,
            ArithmeticBudget {
                max_symbols: 1,
                max_work: 4,
            },
        )
        .unwrap();
        assert!(!stripe.decode_symbol(0).unwrap());
        assert_eq!(stripe.snapshot().work_done, 4);
        let error = stripe.decode_symbol(0).unwrap_err();
        assert!(matches!(
            error.kind,
            ArithmeticErrorKind::LimitExceeded {
                resource: "symbols",
                limit: 1,
                attempted: 2,
            }
        ));
        stripe.finish(1).unwrap();
    }

    #[test]
    fn work_errors_and_invalid_context_preflight_are_distinct() {
        let limits = Limits::default();
        let mut contexts = ContextBank::new(1, &limits).unwrap();
        let mut stripe = decoder(&[0, 0, 0], span(3), &mut contexts, &limits, budget()).unwrap();
        let error = stripe.decode_symbol(1).unwrap_err();
        assert_eq!(error.context, Some(1));
        assert!(matches!(error.kind, ArithmeticErrorKind::InvalidContext));
        assert!(!stripe.decode_symbol(0).unwrap());
        stripe.finish(1).unwrap();

        let mut stripe = decoder(
            &[0xc0, 0, 0],
            span(3),
            &mut contexts,
            &limits,
            ArithmeticBudget {
                max_symbols: 1,
                max_work: 4,
            },
        )
        .unwrap();
        let error = stripe.decode_symbol(0).unwrap_err();
        assert_eq!(error.context, Some(0));
        assert!(matches!(
            error.kind,
            ArithmeticErrorKind::LimitExceeded {
                resource: "arithmetic work",
                limit: 4,
                attempted: 5,
            }
        ));
        assert_eq!(stripe.snapshot().symbols_decoded, 0);
        assert_eq!(stripe.context_state(0), Some(ContextState::default()));
    }

    #[test]
    fn finish_requires_the_exact_symbol_count_and_each_stripe_resets_contexts() {
        let limits = Limits::default();
        let mut contexts = ContextBank::new(1, &limits).unwrap();
        let mut stripe = decoder(&[0xc0, 0, 0], span(3), &mut contexts, &limits, budget()).unwrap();
        assert!(stripe.decode_symbol(0).unwrap());
        assert!(matches!(
            stripe.finish(2).unwrap_err().kind,
            ArithmeticErrorKind::SymbolCount {
                expected: 2,
                decoded: 1,
            }
        ));

        let mut stripe = decoder(&[0xc0, 0, 0], span(3), &mut contexts, &limits, budget()).unwrap();
        assert!(stripe.decode_symbol(0).unwrap());
        stripe.finish(1).unwrap();
        assert_eq!(
            contexts.get(0),
            Some(ContextState {
                state_index: 1,
                mps: true,
            })
        );

        let mut stripe = decoder(&[0, 0, 0], span(3), &mut contexts, &limits, budget()).unwrap();
        assert_eq!(stripe.context_state(0), Some(ContextState::default()));
        assert!(!stripe.decode_symbol(0).unwrap());
        contexts.reset();
        assert_eq!(contexts.get(0), Some(ContextState::default()));
    }

    #[test]
    fn fixed_budget_mutations_are_deterministic_and_bounded() {
        fn trace(input: &[u8; 8]) -> (Vec<bool>, ArithmeticSnapshot) {
            let limits = Limits::default();
            let mut contexts = ContextBank::new(2, &limits).unwrap();
            let mut stripe = decoder(
                input,
                span(input.len() as u64),
                &mut contexts,
                &limits,
                ArithmeticBudget {
                    max_symbols: 64,
                    max_work: 512,
                },
            )
            .unwrap();
            let mut bits = Vec::new();
            for symbol in 0..64 {
                bits.push(stripe.decode_symbol(symbol % 2).unwrap());
            }
            let snapshot = stripe.snapshot();
            assert_eq!(snapshot.symbols_decoded, 64);
            assert!(snapshot.work_done <= 512);
            assert!(snapshot.input_offset <= input.len() as u64);
            stripe.finish(64).unwrap();
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
    fn configuration_errors_have_distinct_messages() {
        let limits = Limits::default();
        let mut contexts = ContextBank::new(2, &limits).unwrap();
        let zero_budget = decoder(
            &[0, 0, 0],
            span(3),
            &mut contexts,
            &limits,
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
    }
}
