// SPDX-License-Identifier: MIT

//! Scaffolding shared by the T.82 QM decoder ([`crate::qm`]) and the T.88 MQ
//! decoder ([`crate::jbig2::mq`]): the context bank, the coded span, the
//! located error, the register snapshot, the symbol and work counters, and
//! the bounded, counted input refill. Each decoder keeps its own interval
//! arithmetic and probability states.

use crate::{
    Cancellation, CountingSource, Error, Limits, MAX_BUDGET_COUNT, RangedSource, read_exact_at,
};
use std::{error, fmt, mem};

/// Bytes fetched by one refill of a decoder's fixed input buffer.
pub(crate) const INPUT_BUFFER_BYTES: usize = 256;

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
    pub fn new(count: usize, limits: &Limits) -> ArithmeticResult<Self> {
        let failed = |kind| ArithmeticError {
            coder: None,
            offset: None,
            context: None,
            kind,
        };
        limits
            .validate()
            .map_err(|source| failed(ArithmeticErrorKind::Source(source)))?;
        if count == 0 {
            return Err(failed(ArithmeticErrorKind::InvalidContext));
        }
        let bytes = count
            .checked_mul(mem::size_of::<ContextState>())
            .and_then(|value| u64::try_from(value).ok())
            .ok_or_else(|| failed(ArithmeticErrorKind::InvalidContext))?;
        limits
            .check_allocation(bytes)
            .map_err(|source| failed(ArithmeticErrorKind::Source(source)))?;
        let mut states = Vec::new();
        states
            .try_reserve_exact(count)
            .map_err(|_| failed(ArithmeticErrorKind::AllocationFailed))?;
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

/// The decoder that located an [`ArithmeticError`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Coder {
    /// The T.82 QM decoder of [`crate::qm`].
    T82,
    /// The T.88 MQ decoder of [`crate::jbig2::mq`].
    T88,
}

/// An arithmetic-decoding error with the next offset in the supplied
/// `RangedSource` and a context index when those locations exist. The offset
/// is not automatically an offset in the original document container. A
/// [`ContextBank`] allocation error names no decoder.
#[derive(Debug)]
pub struct ArithmeticError {
    pub coder: Option<Coder>,
    pub offset: Option<u64>,
    pub context: Option<usize>,
    pub kind: ArithmeticErrorKind,
}

#[derive(Debug)]
pub enum ArithmeticErrorKind {
    InvalidContext,
    InvalidSpan(&'static str),
    InvalidBudget,
    /// `finish` was given a symbol count other than the decoded count.
    SymbolCount {
        expected: u64,
        decoded: u64,
    },
    /// T.88 only: the coding unit does not end with `FF AC`.
    MissingTerminator,
    /// T.88 only: a marker other than the final `FF AC`.
    InvalidMarker(u8),
    LimitExceeded {
        resource: &'static str,
        limit: u64,
        attempted: u64,
    },
    AllocationFailed,
    Cancelled,
    Source(Error),
    Poisoned,
}

pub type ArithmeticResult<T> = std::result::Result<T, ArithmeticError>;

impl ArithmeticError {
    /// An error located only by its decoder, before any input is read.
    pub(crate) fn configuration(coder: Coder, kind: ArithmeticErrorKind) -> Self {
        Self {
            coder: Some(coder),
            offset: None,
            context: None,
            kind,
        }
    }
}

impl fmt::Display for ArithmeticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.coder {
            Some(Coder::T82) => "T.82 arithmetic decoder",
            Some(Coder::T88) => "T.88 MQ decoder",
            None => "arithmetic context bank",
        })?;
        if let Some(offset) = self.offset {
            write!(f, " at source byte {offset}")?;
        }
        if let Some(context) = self.context {
            write!(f, ", context {context}")?;
        }
        f.write_str(": ")?;
        match &self.kind {
            ArithmeticErrorKind::InvalidContext => f.write_str("invalid context index or count"),
            ArithmeticErrorKind::InvalidSpan(reason) => write!(f, "invalid span: {reason}"),
            ArithmeticErrorKind::InvalidBudget => f.write_str("invalid budget"),
            ArithmeticErrorKind::SymbolCount { expected, decoded } => {
                write!(f, "expected {expected} symbols, decoded {decoded}")
            }
            ArithmeticErrorKind::MissingTerminator => f.write_str("missing terminal marker"),
            ArithmeticErrorKind::InvalidMarker(second) => {
                write!(f, "invalid marker following 0xFF: {second:#04x}")
            }
            ArithmeticErrorKind::LimitExceeded {
                resource,
                limit,
                attempted,
            } => write!(f, "{resource} limit {limit} exceeded by {attempted}"),
            ArithmeticErrorKind::AllocationFailed => f.write_str("context allocation failed"),
            ArithmeticErrorKind::Cancelled => f.write_str("cancelled"),
            ArithmeticErrorKind::Source(source) => write!(f, "source error: {source}"),
            ArithmeticErrorKind::Poisoned => f.write_str("decoder is poisoned after an error"),
        }
    }
}

impl error::Error for ArithmeticError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.kind {
            ArithmeticErrorKind::Source(source) => Some(source),
            _ => None,
        }
    }
}

/// A read-only register and progress snapshot. `code` is the full 32-bit C
/// register; `interval` is A and `bit_counter` is CT.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArithmeticSnapshot {
    pub interval: u32,
    pub code: u32,
    pub bit_counter: u8,
    /// The next coded byte the decoder consumes, in source coordinates.
    /// Prefetch and the T.88 terminal check can fetch beyond it.
    pub input_offset: u64,
    /// Bytes returned by source reads, including the fixed-buffer prefetch.
    pub source_bytes_fetched: u64,
    /// Inputs supplied beyond the coded bytes: T.82 zero bytes after the
    /// span, or T.88 one bits at the terminal marker.
    pub synthesized_inputs: u64,
    pub symbols_decoded: u64,
    pub work_done: u64,
    pub poisoned: bool,
}

/// Whether the symbol and work budgets of a decoder are in
/// `1..=MAX_BUDGET_COUNT`.
pub(crate) fn valid_counts(max_symbols: u64, max_work: u64) -> bool {
    (1..=MAX_BUDGET_COUNT).contains(&max_symbols) && (1..=MAX_BUDGET_COUNT).contains(&max_work)
}

/// Check a coded span against the input limit and the source size before
/// any read, returning its end.
pub(crate) fn check_span(
    coder: Coder,
    span: CodedSpan,
    source_size: u64,
    limits: &Limits,
) -> ArithmeticResult<u64> {
    let at = |kind| ArithmeticError {
        coder: Some(coder),
        offset: Some(span.offset),
        context: None,
        kind,
    };
    limits
        .check_input_size(span.length)
        .map_err(|source| at(ArithmeticErrorKind::Source(source)))?;
    let end = span
        .offset
        .checked_add(span.length)
        .ok_or_else(|| at(ArithmeticErrorKind::InvalidSpan("end overflows u64")))?;
    if end > source_size {
        return Err(at(ArithmeticErrorKind::InvalidSpan("outside source size")));
    }
    Ok(end)
}

/// The counters and poison flag of one coding unit.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Counters {
    pub symbols_decoded: u64,
    pub work_done: u64,
    pub source_bytes_fetched: u64,
    pub synthesized_inputs: u64,
    /// Set before any register update that awaits input, so that an error or
    /// a dropped future leaves the coding unit unusable.
    pub poisoned: bool,
}

impl Counters {
    /// Check the symbol budget for one more decision, returning the count
    /// after it. The caller poisons the unit before decoding the symbol.
    pub(crate) fn next_symbol(&self, max_symbols: u64) -> Result<u64, ArithmeticErrorKind> {
        // `symbols_decoded <= max_symbols <= MAX_BUDGET_COUNT`.
        let attempted = self.symbols_decoded + 1;
        if attempted > max_symbols {
            return Err(ArithmeticErrorKind::LimitExceeded {
                resource: "symbols",
                limit: max_symbols,
                attempted,
            });
        }
        Ok(attempted)
    }

    /// Record a completed symbol and clear the poison set before it.
    pub(crate) fn complete_symbol(&mut self, symbols: u64) {
        self.symbols_decoded = symbols;
        self.poisoned = false;
    }

    /// Charge `count` units of work. Every charge is at most one input
    /// refill, so with `work_done <= max_work <= MAX_BUDGET_COUNT` the sum
    /// cannot overflow.
    pub(crate) fn charge(&mut self, count: u64, max_work: u64) -> Result<(), ArithmeticErrorKind> {
        debug_assert!(count <= INPUT_BUFFER_BYTES as u64);
        let attempted = self.work_done + count;
        if attempted > max_work {
            return Err(ArithmeticErrorKind::LimitExceeded {
                resource: "arithmetic work",
                limit: max_work,
                attempted,
            });
        }
        self.work_done = attempted;
        Ok(())
    }

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
            source_bytes_fetched: self.source_bytes_fetched,
            synthesized_inputs: self.synthesized_inputs,
            symbols_decoded: self.symbols_decoded,
            work_done: self.work_done,
            poisoned: self.poisoned,
        }
    }
}

/// A fixed window of coded bytes, refilled by bounded positioned reads.
pub(crate) struct InputBuffer {
    bytes: [u8; INPUT_BUFFER_BYTES],
    /// Span-relative offset of `bytes[0]`.
    start: u64,
    len: usize,
}

impl InputBuffer {
    pub(crate) const fn new() -> Self {
        Self {
            bytes: [0; INPUT_BUFFER_BYTES],
            start: 0,
            len: 0,
        }
    }

    /// The cached byte at span-relative `relative`, if any.
    pub(crate) fn get(&self, relative: u64) -> Option<u8> {
        let index = relative.checked_sub(self.start)?;
        (index < self.len as u64).then(|| self.bytes[index as usize])
    }

    /// Replace the window with `count` bytes from span-relative `relative`.
    /// Completed reads stay counted in `fetched` when a later short read or
    /// a cancellation fails the refill; the error is located at the first
    /// byte not read.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn refill<S: RangedSource, C: Cancellation>(
        &mut self,
        source: &mut S,
        span: CodedSpan,
        relative: u64,
        count: usize,
        fetched: &mut u64,
        limits: &Limits,
        cancellation: &C,
    ) -> Result<(), (u64, ArithmeticErrorKind)> {
        let offset = span.offset + relative;
        let mut counted = CountingSource::new(source, fetched);
        read_exact_at(
            &mut counted,
            offset,
            &mut self.bytes[..count],
            limits,
            cancellation,
        )
        .map_err(|source| {
            let failed_at = match source {
                Error::TruncatedInput {
                    offset, available, ..
                } => offset.saturating_add(available),
                _ => offset,
            };
            let kind = match source {
                Error::Cancelled => ArithmeticErrorKind::Cancelled,
                other => ArithmeticErrorKind::Source(other),
            };
            (failed_at, kind)
        })?;
        self.start = relative;
        self.len = count;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_banks_are_nonempty_bounded_and_reset_to_state_zero() {
        let limits = Limits::default();
        let empty = ContextBank::new(0, &limits).unwrap_err();
        assert!(matches!(empty.kind, ArithmeticErrorKind::InvalidContext));
        assert_eq!(
            empty.to_string(),
            "arithmetic context bank: invalid context index or count"
        );
        let tiny = Limits {
            io_chunk_bytes: 1,
            max_allocation_bytes: 1,
            ..limits
        };
        assert!(matches!(
            ContextBank::new(1, &tiny).unwrap_err().kind,
            ArithmeticErrorKind::Source(Error::LimitExceeded { .. })
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
        assert!(matches!(
            ContextBank::new(usize::MAX, &limits).unwrap_err().kind,
            ArithmeticErrorKind::InvalidContext
        ));
        // This byte size fits `usize` but exceeds `isize::MAX`, so the fallible
        // reservation fails before the allocator is called.
        #[cfg(target_pointer_width = "64")]
        assert!(matches!(
            ContextBank::new(1 << 62, &limits).unwrap_err().kind,
            ArithmeticErrorKind::AllocationFailed
        ));
    }
}
