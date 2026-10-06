// SPDX-License-Identifier: MIT

//! Experimental T.88 Annex E MQ arithmetic control flow for one bounded stream.
//!
//! The decoder uses the 47 standard probability states in
//! [`STANDARD_STATES`]. No Annex H vector, JBIG2 image model, or container
//! parser is included. The interval arithmetic is independent of the T.82
//! decoder; the context bank, snapshot and counters are shared with it in
//! [`crate::arith`].

mod standard;
pub use standard::STANDARD_STATES;

pub use crate::arith::{ArithmeticSnapshot, CodedSpan, ContextBank, ContextState};

use crate::arith::{Counters, INVALID_CONTEXT, SYMBOL_COUNT, check_span};
use crate::{Error, Limits, Payload, Result};

const MISSING_TERMINATOR: &str = "MQ coding unit lacks its terminal marker";
const INVALID_MARKER: &str = "invalid MQ marker following 0xFF";

pub const MQ_STATE_COUNT: usize = 47;

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

/// Decoder of one MQ arithmetic substream; no JBIG2 image pixels are produced.
pub struct MqDecoder<'a> {
    data: &'a [u8],
    span: CodedSpan,
    table: &'a MqTable,
    contexts: &'a mut ContextBank,
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
    ) -> Result<Self> {
        if span.length < 2 {
            return Err(Error::invalid("MQ coded span lacks two terminal bytes").at(span.offset));
        }
        let data = check_span(span, input, limits)?;
        let mut decoder = Self {
            data,
            span,
            table,
            contexts,
            bp: 0,
            current_byte: 0,
            counters: Counters::default(),
            interval: 0x8000,
            code: 0,
            bit_counter: 0,
        };
        decoder.current_byte = decoder.read_byte(0)?;
        decoder.code = u32::from(decoder.current_byte) << 16;
        decoder.byte_in()?;
        decoder.code <<= 7;
        decoder.bit_counter -= 7;
        Ok(decoder)
    }

    /// Decode one decision. After an error the coding unit's registers are
    /// undefined; the caller must abandon it.
    pub fn decode_bit(&mut self, context: usize) -> Result<bool> {
        if self.contexts.get(context).is_none() {
            return Err(self.at(INVALID_CONTEXT));
        }
        let bit = self.decode_bit_inner(context)?;
        self.counters.symbols_decoded += 1;
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
    pub fn finish(&mut self, expected_symbols: u64) -> Result<ArithmeticSnapshot> {
        if self.counters.symbols_decoded != expected_symbols {
            return Err(self.at(SYMBOL_COUNT));
        }
        let tail = self.span.length - 2;
        let first = self.read_byte(tail)?;
        let second = self.read_byte(tail + 1)?;
        if first != 0xFF {
            return Err(Error::malformed(
                self.span.offset + tail,
                MISSING_TERMINATOR,
            ));
        }
        if second != 0xAC {
            return Err(Error::malformed(
                self.span.offset + tail + 1,
                INVALID_MARKER,
            ));
        }
        Ok(self.snapshot())
    }

    /// An invalid-input error at the next coded byte.
    pub(crate) fn at(&self, reason: &'static str) -> Error {
        Error::invalid(reason).at(self.span.offset + self.bp)
    }

    fn read_byte(&self, relative: u64) -> Result<u8> {
        if relative >= self.span.length {
            return Err(Error::malformed(
                self.span.offset + self.span.length,
                MISSING_TERMINATOR,
            ));
        }
        // `check_span` proved that the span's bytes are all in `data`.
        Ok(self.data[relative as usize])
    }

    fn byte_in(&mut self) -> Result<()> {
        let next = self.bp + 1;
        let next_byte = self.read_byte(next)?;
        if self.current_byte == 0xFF {
            if next_byte > 0x8F {
                if next_byte != 0xAC || self.bp != self.span.length - 2 {
                    return Err(Error::malformed(self.span.offset + next, INVALID_MARKER));
                }
                self.counters.synthesized_inputs += 1;
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

    fn renormalize(&mut self) -> Result<()> {
        while self.interval < 0x8000 {
            if self.bit_counter == 0 {
                self.byte_in()?;
            }
            self.interval <<= 1;
            self.code <<= 1;
            self.bit_counter -= 1;
        }
        Ok(())
    }

    fn decode_bit_inner(&mut self, context: usize) -> Result<bool> {
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
        self.renormalize()?;
        self.contexts.update(context, updated);
        Ok(bit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `test` on the result of initializing a decoder over `bytes` with
    /// one context.
    fn with_init<R>(bytes: &[u8], test: impl FnOnce(Result<MqDecoder<'_>>) -> R) -> R {
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
        ))
    }

    #[test]
    fn initialization_checks_markers_on_its_first_byte() {
        with_init(&[0xff, 0x90], |decoder| {
            let error = decoder.err().expect("MQ initialization must fail");
            assert_eq!((error.offset, error.reason), (Some(1), INVALID_MARKER));
        });
        // The terminal marker at initialization synthesizes one bits.
        with_init(&[0xff, 0xac], |decoder| {
            assert_eq!(decoder.unwrap().counters.synthesized_inputs, 1);
        });
        // A stuffed 0xFF followed by a data byte is consumed as seven bits.
        with_init(&[0xff, 0x7f, 0xff, 0xac], |decoder| {
            let decoder = decoder.unwrap();
            assert_eq!((decoder.bp, decoder.current_byte), (1, 0x7f));
        });
    }

    #[test]
    fn decisions_reject_unknown_contexts() {
        with_init(&[0, 0xff, 0xac], |decoder| {
            let mut decoder = decoder.unwrap();
            assert_eq!(decoder.decode_bit(1).unwrap_err().reason, INVALID_CONTEXT);
        });
    }
}
