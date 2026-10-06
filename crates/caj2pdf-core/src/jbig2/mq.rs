// SPDX-License-Identifier: MIT

//! Experimental T.88 Annex E MQ arithmetic control flow for one bounded stream.
//!
//! The decoder uses the 47 standard probability states in
//! [`STANDARD_STATES`]. No Annex H vector, JBIG2 image model, or container
//! parser is included. The interval arithmetic is independent of the T.82
//! decoder; the context bank, errors, snapshot and counters are shared with
//! it in [`crate::arith`].

mod standard;
pub use standard::STANDARD_STATES;

pub use crate::arith::{
    ArithmeticError, ArithmeticErrorKind, ArithmeticResult, ArithmeticSnapshot, CodedSpan, Coder,
    ContextBank, ContextState,
};

use crate::arith::{Counters, check_span, valid_counts};
use crate::{Limits, MAX_BUDGET_COUNT, Payload};
use std::mem;

pub const MQ_STATE_COUNT: usize = 47;

/// Working bytes of a decoder with `contexts` contexts: the bank and the
/// state table.
fn allocation_bytes(contexts: usize) -> ArithmeticResult<u64> {
    contexts
        .checked_mul(mem::size_of::<ContextState>())
        .and_then(|value| value.checked_add(MQ_STATE_COUNT * mem::size_of::<MqState>()))
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| {
            ArithmeticError::configuration(Coder::T88, ArithmeticErrorKind::InvalidContext)
        })
}

/// One probability state, in T.88 Table E.1 column order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MqState {
    pub qe: u16,
    pub next_mps: u8,
    pub next_lps: u8,
    pub switch_mps: bool,
}

/// The standard T.88 Table E.1 state machine, [`STANDARD_STATES`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MqTable(());

impl MqTable {
    pub const fn standard() -> Self {
        Self(())
    }

    fn state(&self, index: u8) -> MqState {
        STANDARD_STATES[usize::from(index)]
    }
}

/// Per-stream CPU, input, context, and synthesized terminal bounds.
///
/// `max_symbols`, `max_work`, and `max_terminal_inputs` bound running
/// counters and must each be at most [`MAX_BUDGET_COUNT`]; a larger value is
/// rejected as `InvalidBudget` before any I/O.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MqBudget {
    pub max_span_bytes: u64,
    pub max_contexts: usize,
    /// Nonzero and at most [`MAX_BUDGET_COUNT`].
    pub max_symbols: u64,
    /// Includes symbol decisions, renormalization shifts, and byte-input
    /// events. Nonzero and at most [`MAX_BUDGET_COUNT`].
    pub max_work: u64,
    /// At most [`MAX_BUDGET_COUNT`].
    pub max_terminal_inputs: u64,
}

impl Default for MqBudget {
    fn default() -> Self {
        Self {
            max_span_bytes: 64 * 1024 * 1024,
            max_contexts: 65_536,
            max_symbols: 12_000_000,
            max_work: 400_000_000,
            max_terminal_inputs: 12_000_000,
        }
    }
}

impl MqBudget {
    fn validate(&self) -> ArithmeticResult<()> {
        if self.max_span_bytes == 0
            || self.max_contexts == 0
            || !valid_counts(self.max_symbols, self.max_work)
            || self.max_terminal_inputs > MAX_BUDGET_COUNT
        {
            return Err(ArithmeticError::configuration(
                Coder::T88,
                ArithmeticErrorKind::InvalidBudget,
            ));
        }
        Ok(())
    }

    /// Allocate a bank of `count` contexts within this budget and `limits`,
    /// before any decoder uses it.
    pub fn context_bank(&self, count: usize, limits: &Limits) -> ArithmeticResult<ContextBank> {
        self.validate()?;
        if count > self.max_contexts {
            return Err(ArithmeticError::configuration(
                Coder::T88,
                ArithmeticErrorKind::LimitExceeded {
                    resource: "MQ contexts",
                    limit: self.max_contexts as u64,
                    attempted: count as u64,
                },
            ));
        }
        ContextBank::new(count, limits).map_err(|error| ArithmeticError {
            coder: Some(Coder::T88),
            ..error
        })
    }
}

/// Decoder of one MQ arithmetic substream; no JBIG2 image pixels are produced.
pub struct MqDecoder<'a> {
    data: &'a [u8],
    span: CodedSpan,
    table: &'a MqTable,
    contexts: &'a mut ContextBank,
    budget: MqBudget,
    bp: u64,
    current_byte: u8,
    counters: Counters,
    interval: u32,
    code: u32,
    bit_counter: u8,
}

impl<'a> MqDecoder<'a> {
    /// Start a coding unit over the span's bytes in `input` and over
    /// `contexts`, which keep their states: the caller resets them as its
    /// coding procedure requires.
    pub fn new(
        input: Payload<'a>,
        span: CodedSpan,
        table: &'a MqTable,
        contexts: &'a mut ContextBank,
        limits: &Limits,
        budget: MqBudget,
    ) -> ArithmeticResult<Self> {
        limits.validate().map_err(|source| {
            ArithmeticError::configuration(Coder::T88, ArithmeticErrorKind::Source(source))
        })?;
        budget.validate()?;
        let at_start = |kind| ArithmeticError {
            coder: Some(Coder::T88),
            offset: Some(span.offset),
            context: None,
            kind,
        };
        if span.length < 2 {
            return Err(at_start(ArithmeticErrorKind::InvalidSpan(
                "requires at least two terminal bytes",
            )));
        }
        if span.length > budget.max_span_bytes {
            return Err(at_start(ArithmeticErrorKind::LimitExceeded {
                resource: "MQ span bytes",
                limit: budget.max_span_bytes,
                attempted: span.length,
            }));
        }
        let data = check_span(Coder::T88, span, input, limits)?;
        if contexts.len() > budget.max_contexts {
            return Err(ArithmeticError::configuration(
                Coder::T88,
                ArithmeticErrorKind::InvalidContext,
            ));
        }
        limits
            .check_allocation(allocation_bytes(contexts.len())?)
            .map_err(|source| {
                ArithmeticError::configuration(Coder::T88, ArithmeticErrorKind::Source(source))
            })?;
        let mut decoder = Self {
            data,
            span,
            table,
            contexts,
            budget,
            bp: 0,
            current_byte: 0,
            counters: Counters::default(),
            interval: 0x8000,
            code: 0,
            bit_counter: 0,
        };
        decoder.current_byte = decoder.read_byte(0, None)?;
        decoder.code = u32::from(decoder.current_byte) << 16;
        decoder.byte_in(None)?;
        decoder.code <<= 7;
        decoder.bit_counter -= 7;
        Ok(decoder)
    }

    /// Decode one decision. After an error the coding unit's registers are
    /// undefined; the caller must abandon it.
    pub fn decode_bit(&mut self, context: usize) -> ArithmeticResult<bool> {
        if self.contexts.get(context).is_none() {
            return Err(self.at(Some(context), ArithmeticErrorKind::InvalidContext));
        }
        let symbols = self
            .counters
            .next_symbol(self.budget.max_symbols)
            .map_err(|kind| self.at(Some(context), kind))?;
        let bit = self.decode_bit_inner(context)?;
        self.counters.symbols_decoded = symbols;
        Ok(bit)
    }

    pub fn snapshot(&self) -> ArithmeticSnapshot {
        self.counters.snapshot(
            self.interval,
            self.code,
            self.bit_counter,
            self.span.offset + self.bp,
        )
    }

    pub fn context(&self, index: usize) -> Option<ContextState> {
        self.contexts.get(index)
    }

    /// The size of the caller-owned context bank, including other models.
    pub(crate) fn context_count(&self) -> usize {
        self.contexts.len()
    }

    /// Verify the caller's symbol count and the exact terminal pair, and
    /// return the final snapshot. The decoder makes no further decisions.
    pub fn finish(&mut self, expected_symbols: u64) -> ArithmeticResult<ArithmeticSnapshot> {
        if self.counters.symbols_decoded != expected_symbols {
            return Err(self.at(
                None,
                ArithmeticErrorKind::SymbolCount {
                    expected: expected_symbols,
                    decoded: self.counters.symbols_decoded,
                },
            ));
        }
        let tail = self.span.length - 2;
        let first = self.read_byte(tail, None)?;
        let second = self.read_byte(tail + 1, None)?;
        if first != 0xFF {
            return Err(self.located(
                self.span.offset + tail,
                None,
                ArithmeticErrorKind::MissingTerminator,
            ));
        }
        if second != 0xAC {
            return Err(self.located(
                self.span.offset + tail + 1,
                None,
                ArithmeticErrorKind::InvalidMarker(second),
            ));
        }
        Ok(self.snapshot())
    }

    fn located(
        &self,
        offset: u64,
        context: Option<usize>,
        kind: ArithmeticErrorKind,
    ) -> ArithmeticError {
        ArithmeticError {
            coder: Some(Coder::T88),
            offset: Some(offset),
            context,
            kind,
        }
    }

    fn at(&self, context: Option<usize>, kind: ArithmeticErrorKind) -> ArithmeticError {
        self.located(self.span.offset + self.bp, context, kind)
    }

    fn charge(&mut self, context: Option<usize>) -> ArithmeticResult<()> {
        self.counters
            .charge(self.budget.max_work)
            .map_err(|kind| self.at(context, kind))
    }

    fn read_byte(&self, relative: u64, context: Option<usize>) -> ArithmeticResult<u8> {
        if relative >= self.span.length {
            return Err(self.located(
                self.span.offset + self.span.length,
                context,
                ArithmeticErrorKind::MissingTerminator,
            ));
        }
        // `check_span` proved that the span's bytes are all in `data`.
        Ok(self.data[relative as usize])
    }

    fn byte_in(&mut self, context: Option<usize>) -> ArithmeticResult<()> {
        self.charge(context)?;
        let next = self.bp + 1;
        let next_byte = self.read_byte(next, context)?;
        if self.current_byte == 0xFF {
            if next_byte > 0x8F {
                if next_byte != 0xAC || self.bp != self.span.length - 2 {
                    return Err(self.located(
                        self.span.offset + next,
                        context,
                        ArithmeticErrorKind::InvalidMarker(next_byte),
                    ));
                }
                // Bounded by `max_terminal_inputs <= MAX_BUDGET_COUNT`.
                let attempted = self.counters.synthesized_inputs + 1;
                if attempted > self.budget.max_terminal_inputs {
                    return Err(self.at(
                        context,
                        ArithmeticErrorKind::LimitExceeded {
                            resource: "MQ terminal inputs",
                            limit: self.budget.max_terminal_inputs,
                            attempted,
                        },
                    ));
                }
                self.counters.synthesized_inputs = attempted;
                self.code = self.code.wrapping_add(0xFF00);
                self.bit_counter = 8;
            } else {
                self.bp = next;
                self.current_byte = next_byte;
                self.code = self.code.wrapping_add(u32::from(next_byte) << 9);
                self.bit_counter = 7;
            }
        } else {
            self.bp = next;
            self.current_byte = next_byte;
            self.code = self.code.wrapping_add(u32::from(next_byte) << 8);
            self.bit_counter = 8;
        }
        Ok(())
    }

    fn renormalize(&mut self, context: usize) -> ArithmeticResult<()> {
        while self.interval < 0x8000 {
            if self.bit_counter == 0 {
                self.byte_in(Some(context))?;
            }
            self.charge(Some(context))?;
            self.interval <<= 1;
            self.code <<= 1;
            self.bit_counter -= 1;
        }
        Ok(())
    }

    fn decode_bit_inner(&mut self, context: usize) -> ArithmeticResult<bool> {
        self.charge(Some(context))?;
        let current = self.contexts.state(context);
        let state = self.table.state(current.state_index);
        let qe = u32::from(state.qe);
        // A decision starts with `interval >= 0x8000`: initialization sets
        // 0x8000, an MPS path without renormalization keeps at least 0x8000,
        // and renormalization doubles it until it reaches 0x8000; a failed
        // renormalization ends the coding unit. Every standard `qe` is below
        // 0x8000, so this cannot wrap.
        debug_assert!(self.interval >= 0x8000 && qe < 0x8000);
        let narrowed = self.interval - qe;
        self.interval = narrowed;
        let (bit, updated) = if self.code >> 16 < qe {
            self.interval = qe;
            if narrowed < qe {
                (
                    current.mps,
                    ContextState {
                        state_index: state.next_mps,
                        mps: current.mps,
                    },
                )
            } else {
                (
                    !current.mps,
                    ContextState {
                        state_index: state.next_lps,
                        mps: current.mps ^ state.switch_mps,
                    },
                )
            }
        } else {
            self.code = self.code.wrapping_sub(qe << 16);
            if narrowed >= 0x8000 {
                (current.mps, current)
            } else if narrowed < qe {
                (
                    !current.mps,
                    ContextState {
                        state_index: state.next_lps,
                        mps: current.mps ^ state.switch_mps,
                    },
                )
            } else {
                (
                    current.mps,
                    ContextState {
                        state_index: state.next_mps,
                        mps: current.mps,
                    },
                )
            }
        };
        self.renormalize(context)?;
        self.contexts.update(context, updated);
        Ok(bit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `test` on the result of initializing a decoder over `bytes` with
    /// one context and the given budget.
    fn with_init<R>(
        bytes: &[u8],
        budget: MqBudget,
        test: impl FnOnce(ArithmeticResult<MqDecoder<'_>>) -> R,
    ) -> R {
        let limits = Limits::default();
        let table = MqTable::standard();
        let mut contexts = ContextBank::new(1, &limits).unwrap();
        test(MqDecoder::new(
            bytes.into(),
            CodedSpan {
                offset: 0,
                length: bytes.len() as u64,
            },
            &table,
            &mut contexts,
            &limits,
            budget,
        ))
    }

    /// Runs `test` on a decoder over a three-byte span with one context
    /// and the given budget.
    fn with_decoder<R>(budget: MqBudget, test: impl FnOnce(&mut MqDecoder<'_>) -> R) -> R {
        with_init(&[0, 0xff, 0xac], budget, |decoder| {
            test(&mut decoder.unwrap())
        })
    }

    /// The initialization error over `bytes`.
    fn init_error(bytes: &[u8], budget: MqBudget) -> ArithmeticErrorKind {
        with_init(bytes, budget, |decoder| {
            decoder.err().expect("MQ initialization must fail").kind
        })
    }

    #[test]
    fn initialization_checks_markers_and_budgets_on_its_first_byte() {
        assert!(matches!(
            init_error(&[0xff, 0x90], MqBudget::default()),
            ArithmeticErrorKind::InvalidMarker(0x90)
        ));
        let no_terminal = MqBudget {
            max_terminal_inputs: 0,
            ..MqBudget::default()
        };
        assert!(matches!(
            init_error(&[0xff, 0xac], no_terminal),
            ArithmeticErrorKind::LimitExceeded {
                resource: "MQ terminal inputs",
                limit: 0,
                attempted: 1,
            }
        ));
        // A stuffed 0xFF followed by a data byte is consumed as seven bits.
        with_init(&[0xff, 0x7f, 0xff, 0xac], MqBudget::default(), |decoder| {
            let decoder = decoder.unwrap();
            assert_eq!((decoder.bp, decoder.current_byte), (1, 0x7f));
        });
    }

    #[test]
    fn decisions_reject_unknown_contexts_and_excess_work() {
        with_decoder(MqBudget::default(), |decoder| {
            assert!(matches!(
                decoder.decode_bit(1).unwrap_err().kind,
                ArithmeticErrorKind::InvalidContext
            ));
            let limit = decoder.budget.max_work;
            decoder.counters.work_done = limit - 1;
            decoder.charge(Some(0)).unwrap();
            let error = decoder.charge(Some(0)).unwrap_err();
            assert!(matches!(
                error.kind,
                ArithmeticErrorKind::LimitExceeded {
                    resource: "arithmetic work",
                    attempted,
                    ..
                } if attempted == limit + 1
            ));
        });
    }

    #[test]
    fn counters_at_the_budget_ceiling_report_the_exact_excess() {
        let ceiling = MqBudget {
            max_symbols: MAX_BUDGET_COUNT,
            max_work: MAX_BUDGET_COUNT,
            max_terminal_inputs: MAX_BUDGET_COUNT,
            ..MqBudget::default()
        };
        with_decoder(ceiling, |decoder| {
            decoder.counters.work_done = MAX_BUDGET_COUNT;
            assert!(matches!(
                decoder.charge(None).unwrap_err().kind,
                ArithmeticErrorKind::LimitExceeded {
                    resource: "arithmetic work",
                    limit: MAX_BUDGET_COUNT,
                    attempted,
                } if attempted == MAX_BUDGET_COUNT + 1
            ));
            decoder.counters.work_done = 0;
            decoder.counters.synthesized_inputs = MAX_BUDGET_COUNT;
            assert!(matches!(
                decoder.byte_in(None).unwrap_err().kind,
                ArithmeticErrorKind::LimitExceeded {
                    resource: "MQ terminal inputs",
                    limit: MAX_BUDGET_COUNT,
                    attempted,
                } if attempted == MAX_BUDGET_COUNT + 1
            ));
        });
        with_decoder(ceiling, |decoder| {
            decoder.counters.symbols_decoded = MAX_BUDGET_COUNT;
            let before = decoder.snapshot();
            let error = decoder.decode_bit(0).unwrap_err();
            assert_eq!(error.context, Some(0));
            assert!(matches!(
                error.kind,
                ArithmeticErrorKind::LimitExceeded {
                    resource: "symbols",
                    limit: MAX_BUDGET_COUNT,
                    attempted,
                } if attempted == MAX_BUDGET_COUNT + 1
            ));
            // The preflight failure leaves registers and work untouched.
            assert_eq!(decoder.snapshot(), before);
        });
    }
}
