// SPDX-License-Identifier: MIT

//! Experimental, bounded T.82 arithmetic decoder for an already isolated SCD.
//!
//! The decoder uses the standard probability states in [`STANDARD_STATES`].
//! No official test vectors, CAJ framing, or image prediction are included
//! in this module. Its input span contains arithmetic bytes after any
//! container framing and byte unstuffing have been handled by the caller.
//! The context bank, snapshot and counters are shared with the T.88 MQ
//! decoder in [`crate::arith`].

mod standard;
pub use standard::STANDARD_STATES;

pub use crate::arith::{ArithmeticSnapshot, CodedSpan, ContextBank, ContextState};

use crate::arith::{Counters, INVALID_CONTEXT, SYMBOL_COUNT, check_span};
use crate::{Error, Limits, Payload, Result};

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

/// Incremental decoder for one already isolated arithmetic stripe.
pub struct ArithmeticDecoder<'a> {
    data: &'a [u8],
    span: CodedSpan,
    table: &'a QmTable,
    contexts: &'a mut ContextBank,
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
    ) -> Result<Self> {
        let data = check_span(span, input, limits)?;
        contexts.reset();
        let mut decoder = Self {
            data,
            span,
            table,
            contexts,
            physical_bytes_consumed: 0,
            counters: Counters::default(),
            interval: 0x10000,
            code: 0,
            bit_counter: 0,
        };
        decoder.byte_in();
        decoder.code <<= 8;
        decoder.byte_in();
        decoder.code <<= 8;
        decoder.byte_in();
        Ok(decoder)
    }

    /// Decode one bit for a checked context. After an error the stripe's
    /// registers are undefined; the caller must abandon it.
    pub fn decode_symbol(&mut self, context: usize) -> Result<bool> {
        if self.contexts.get(context).is_none() {
            return Err(self.at(INVALID_CONTEXT));
        }
        let bit = self.decode_symbol_inner(context);
        self.counters.symbols_decoded += 1;
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
    pub fn finish(self, expected_symbols: u64) -> Result<()> {
        if self.counters.symbols_decoded != expected_symbols {
            return Err(self.at(SYMBOL_COUNT));
        }
        Ok(())
    }

    fn at(&self, reason: &'static str) -> Error {
        Error::invalid(reason).at(self.span.offset + self.physical_bytes_consumed)
    }

    fn next_byte(&mut self) -> u8 {
        let relative = self.physical_bytes_consumed;
        if relative == self.span.length {
            self.counters.synthesized_inputs += 1;
            return 0;
        }
        self.physical_bytes_consumed += 1;
        // `check_span` proved that the span's bytes are all in `data`.
        self.data[relative as usize]
    }

    fn byte_in(&mut self) {
        let byte = self.next_byte();
        self.code = self.code.wrapping_add(u32::from(byte) << 8);
        self.bit_counter = 8;
    }

    fn renormalize(&mut self) {
        loop {
            if self.bit_counter == 0 {
                self.byte_in();
            }
            self.interval <<= 1;
            self.code <<= 1;
            self.bit_counter -= 1;
            if self.interval >= 0x8000 {
                break;
            }
        }
        if self.bit_counter == 0 {
            self.byte_in();
        }
    }

    fn decode_symbol_inner(&mut self, context: usize) -> bool {
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
                self.renormalize();
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
            self.renormalize();
            (current.mps ^ exchange, next)
        };
        self.contexts.update(context, next);
        bit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(length: u64) -> CodedSpan {
        CodedSpan { offset: 0, length }
    }

    /// A decoder over `bytes`, which start at source offset zero.
    fn decoder<'a>(
        bytes: &'a [u8],
        span: CodedSpan,
        contexts: &'a mut ContextBank,
        limits: &Limits,
    ) -> Result<ArithmeticDecoder<'a>> {
        const TABLE: QmTable = QmTable::standard();
        ArithmeticDecoder::new(bytes.into(), span, &TABLE, contexts, limits)
    }

    #[test]
    fn validates_spans_before_input() {
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
        );
        let invalid_span = invalid_span.err().unwrap();
        assert!(matches!(invalid_span.kind, crate::ErrorKind::Malformed));
        assert_eq!(invalid_span.offset, Some(u64::MAX));
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
        )
        .err()
        .unwrap();
        assert_eq!(input_limit.offset, Some(1));
        assert!(matches!(
            input_limit.kind,
            crate::ErrorKind::LimitExceeded { .. }
        ));
    }

    #[test]
    fn adapted_context_survives_rejected_spans_without_resetting() {
        let limits = Limits::default();
        let mut contexts = ContextBank::new(2, &limits).unwrap();
        // An LPS in context 1 switches its MPS and moves it to state 1.
        let mut stripe = decoder(&[0xc0, 0, 0], span(3), &mut contexts, &limits).unwrap();
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
            let error = decoder(&[0, 0, 0], span, &mut contexts, &limits)
                .err()
                .unwrap();
            assert_eq!(error.offset, Some(span.offset));
            assert_eq!(error.reason, crate::arith::OUTSIDE_SOURCE);
            assert_eq!(
                error.to_string(),
                format!(
                    "invalid input at byte {}: coded span is outside the source",
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
        let mut stripe = decoder(&[0, 0, 0], span(3), &mut contexts, &limits).unwrap();
        let invalid_context = stripe.decode_symbol(1).unwrap_err();
        assert_eq!(
            invalid_context.to_string(),
            "invalid input at byte 3: invalid arithmetic context index or count"
        );
        assert!(std::error::Error::source(&invalid_context).is_none());
        assert!(!stripe.decode_symbol(0).unwrap());
        let incomplete = stripe.finish(2).unwrap_err();
        assert_eq!(
            incomplete.to_string(),
            "invalid input at byte 3: decoded symbol count differs from the expected count"
        );
    }

    #[test]
    fn initializes_registers_with_virtual_zeros() {
        let limits = Limits::default();
        let mut contexts = ContextBank::new(1, &limits).unwrap();
        let stripe = decoder(&[0x12, 0x34, 0x56, 0x78], span(3), &mut contexts, &limits).unwrap();
        let snapshot = stripe.snapshot();
        assert_eq!(snapshot.interval, 0x10000);
        assert_eq!(snapshot.code, 0x12345600);
        assert_eq!(snapshot.bit_counter, 8);
        assert_eq!(snapshot.synthesized_inputs, 0);
        assert_eq!(snapshot.input_offset, 3);
        stripe.finish(0).unwrap();

        let stripe = decoder(&[0xff], span(0), &mut contexts, &limits).unwrap();
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
            let mut stripe = decoder(bytes, span(3), &mut contexts, &limits).unwrap();
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
    fn fixed_width_register_operations_are_explicit() {
        let limits = Limits::default();
        let mut contexts = ContextBank::new(1, &limits).unwrap();
        let mut stripe = decoder(&[0, 0, 0, 0xff], span(4), &mut contexts, &limits).unwrap();
        stripe.code = 0xffff_ff00;
        stripe.byte_in();
        assert_eq!(stripe.snapshot().code, 0x0000_fe00);
        assert_eq!(stripe.snapshot().input_offset, 4);
        stripe.interval = 0x7fff;
        stripe.bit_counter = 8;
        stripe.renormalize();
        assert_eq!(stripe.snapshot().interval, 0xfffe);
        assert_eq!(stripe.snapshot().bit_counter, 7);
        stripe.finish(0).unwrap();
    }

    #[test]
    fn invalid_context_preflight_leaves_the_stripe_usable() {
        let limits = Limits::default();
        let mut contexts = ContextBank::new(1, &limits).unwrap();
        let mut stripe = decoder(&[0, 0, 0], span(3), &mut contexts, &limits).unwrap();
        let error = stripe.decode_symbol(1).unwrap_err();
        assert_eq!(error.reason, INVALID_CONTEXT);
        assert!(!stripe.decode_symbol(0).unwrap());
        stripe.finish(1).unwrap();
    }

    #[test]
    fn finish_requires_the_exact_symbol_count_and_each_stripe_resets_contexts() {
        let limits = Limits::default();
        let mut contexts = ContextBank::new(1, &limits).unwrap();
        let mut stripe = decoder(&[0xc0, 0, 0], span(3), &mut contexts, &limits).unwrap();
        assert!(stripe.decode_symbol(0).unwrap());
        assert_eq!(stripe.finish(2).unwrap_err().reason, SYMBOL_COUNT);

        let mut stripe = decoder(&[0xc0, 0, 0], span(3), &mut contexts, &limits).unwrap();
        assert!(stripe.decode_symbol(0).unwrap());
        stripe.finish(1).unwrap();
        assert_eq!(
            contexts.get(0),
            Some(ContextState {
                state_index: 1,
                mps: true,
            })
        );

        let mut stripe = decoder(&[0, 0, 0], span(3), &mut contexts, &limits).unwrap();
        assert_eq!(stripe.context_state(0), Some(ContextState::default()));
        assert!(!stripe.decode_symbol(0).unwrap());
        contexts.reset();
        assert_eq!(contexts.get(0), Some(ContextState::default()));
    }

    #[test]
    fn input_mutations_are_deterministic_and_bounded() {
        fn trace(input: &[u8; 8]) -> (Vec<bool>, ArithmeticSnapshot) {
            let limits = Limits::default();
            let mut contexts = ContextBank::new(2, &limits).unwrap();
            let mut stripe =
                decoder(input, span(input.len() as u64), &mut contexts, &limits).unwrap();
            let mut bits = Vec::new();
            for symbol in 0..64 {
                bits.push(stripe.decode_symbol(symbol % 2).unwrap());
            }
            let snapshot = stripe.snapshot();
            assert_eq!(snapshot.symbols_decoded, 64);
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
}
