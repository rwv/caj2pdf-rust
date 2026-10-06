// SPDX-License-Identifier: MIT

//! Scaffolding shared by the T.82 QM decoder ([`crate::qm`]) and the T.88 MQ
//! decoder ([`crate::jbig2::mq`]): the context bank, the coded span, the
//! register snapshot, and the symbol counters.
//! Each decoder keeps its own interval arithmetic and probability states and
//! reads its coded bytes from a [`Payload`] in memory.

use crate::{Error, Limits, Payload, Result};
use std::mem;

/// One context's probability-state index and more-probable symbol.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ContextState {
    pub state_index: u8,
    pub mps: bool,
}

/// Adaptive contexts, each starting at state zero with MPS zero. A decoder
/// resets or keeps them as its coding procedure requires.
#[derive(Debug)]
pub struct ContextBank {
    states: Vec<ContextState>,
}

impl ContextBank {
    /// Allocate a nonempty bank under the caller's allocation limit.
    pub fn new(count: usize, limits: &Limits) -> Result<Self> {
        let invalid = || Error::invalid(INVALID_CONTEXT);
        if count == 0 {
            return Err(invalid());
        }
        let bytes = count
            .checked_mul(mem::size_of::<ContextState>())
            .and_then(|value| u64::try_from(value).ok())
            .ok_or_else(invalid)?;
        limits.check_allocation(bytes)?;
        let mut states = Vec::new();
        states
            .try_reserve_exact(count)
            .map_err(|_| limits.allocation_refused("arithmetic context bytes", bytes))?;
        states.resize(count, ContextState::default());
        Ok(Self { states })
    }

    /// Return every context to state zero with MPS zero.
    pub fn reset(&mut self) {
        self.states.fill(ContextState::default());
    }

    pub fn get(&self, index: usize) -> Option<ContextState> {
        self.states.get(index).copied()
    }

    /// The number of contexts in this bank.
    pub fn len(&self) -> usize {
        self.states.len()
    }

    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    /// The state of a context the decoder has already checked.
    pub(crate) fn state(&self, index: usize) -> ContextState {
        self.states[index]
    }

    pub(crate) fn update(&mut self, index: usize, state: ContextState) {
        self.states[index] = state;
    }
}

/// The arithmetic-coded bytes of one coding unit, in source coordinates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CodedSpan {
    pub offset: u64,
    pub length: u64,
}

/// The reason for a context index outside the bank or an empty bank.
pub(crate) const INVALID_CONTEXT: &str = "invalid arithmetic context index or count";

/// A read-only register and progress snapshot. `code` is the full 32-bit C
/// register; `interval` is A and `bit_counter` is CT.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArithmeticSnapshot {
    pub interval: u32,
    pub code: u32,
    pub bit_counter: u8,
    /// The next coded byte the decoder consumes, in source coordinates.
    /// The T.88 terminal check reads beyond it.
    pub input_offset: u64,
    /// Inputs supplied beyond the coded bytes: T.82 zero bytes after the
    /// span, or T.88 one bits at the terminal marker.
    pub synthesized_inputs: u64,
    pub symbols_decoded: u64,
}

/// Check a coded span against the input limit and the payload, returning
/// its bytes.
pub(crate) fn check_span<'a>(
    span: CodedSpan,
    input: Payload<'a>,
    limits: &Limits,
) -> Result<&'a [u8]> {
    limits
        .check_input_size(span.length)
        .map_err(|error| error.at(span.offset))?;
    span.offset
        .checked_add(span.length)
        .ok_or_else(|| Error::invalid("coded span end overflows u64").at(span.offset))?;
    input
        .get(span.offset, span.length)
        .ok_or_else(|| Error::invalid(OUTSIDE_SOURCE).at(span.offset))
}

/// The reason for a coded span outside its payload.
pub(crate) const OUTSIDE_SOURCE: &str = "coded span is outside the source";

/// The reason for a decoded symbol count other than the caller's.
pub(crate) const SYMBOL_COUNT: &str = "decoded symbol count differs from the expected count";

/// The counters of one coding unit. Each step adds one, so neither can
/// reach `u64::MAX`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Counters {
    pub symbols_decoded: u64,
    pub synthesized_inputs: u64,
}

impl Counters {
    pub(crate) fn snapshot(
        &self,
        interval: u32,
        code: u32,
        bit_counter: u8,
        input_offset: u64,
    ) -> ArithmeticSnapshot {
        ArithmeticSnapshot {
            interval,
            code,
            bit_counter,
            input_offset,
            synthesized_inputs: self.synthesized_inputs,
            symbols_decoded: self.symbols_decoded,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_banks_are_nonempty_bounded_and_reset_to_state_zero() {
        let limits = Limits::default();
        let empty = ContextBank::new(0, &limits).unwrap_err();
        assert!(matches!(empty.kind, crate::ErrorKind::Malformed));
        assert_eq!(
            empty.to_string(),
            "invalid input: invalid arithmetic context index or count"
        );
        let tiny = Limits {
            io_chunk_bytes: 1,
            max_allocation_bytes: 1,
            ..limits
        };
        assert!(matches!(
            ContextBank::new(1, &tiny).unwrap_err().kind,
            crate::ErrorKind::LimitExceeded { .. }
        ));
        let mut bank = ContextBank::new(2, &limits).unwrap();
        assert_eq!((bank.len(), bank.is_empty()), (2, false));
        assert_eq!(bank.get(2), None);
        let adapted = ContextState {
            state_index: 7,
            mps: true,
        };
        bank.update(1, adapted);
        assert_eq!(bank.get(1), Some(adapted));
        bank.reset();
        assert_eq!(bank.get(1), Some(ContextState::default()));
    }

    #[test]
    fn context_banks_beyond_the_address_space_are_refused_without_allocating() {
        let limits = Limits {
            max_allocation_bytes: u64::MAX,
            ..Limits::default()
        };
        // The byte size of this bank overflows `usize`.
        assert_eq!(
            ContextBank::new(usize::MAX, &limits).unwrap_err().reason,
            INVALID_CONTEXT
        );
        // This byte size fits `usize` but exceeds `isize::MAX`, so the fallible
        // reservation fails before the allocator is called.
        #[cfg(target_pointer_width = "64")]
        assert!(matches!(
            ContextBank::new(1 << 62, &limits).unwrap_err().kind,
            crate::ErrorKind::LimitExceeded {
                resource: "arithmetic context bytes",
                ..
            }
        ));
    }
}
