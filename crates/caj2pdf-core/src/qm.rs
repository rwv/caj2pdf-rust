// SPDX-License-Identifier: MIT

//! Experimental, bounded T.82 arithmetic decoder for an already isolated SCD.
//!
//! The caller supplies the probability states. This module contains no
//! normative state tuples, test vectors, CAJ framing, or image prediction.
//! Its input span contains arithmetic bytes after any container framing and
//! byte unstuffing have been handled by the caller.

use crate::{Cancellation, Error, Limits, RangedSource, read_exact_at};
use std::{error, fmt, mem};

/// Number of probability-estimation states in T.82 Table 24.
pub const QM_STATE_COUNT: usize = 113;
const INPUT_BUFFER_BYTES: usize = 256;

/// Count bytes actually returned by positioned reads, including prefetch and
/// successful prefixes of a later failing refill.
struct ProgressSource<'a, S> {
    inner: &'a mut S,
    fetched: &'a mut u64,
}

impl<S: RangedSource> RangedSource for ProgressSource<'_, S> {
    fn size(&self) -> u64 {
        self.inner.size()
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> crate::Result<usize> {
        let read = self.inner.read_at(offset, destination).await?;
        if read <= destination.len() {
            *self.fetched = self
                .fetched
                .checked_add(read as u64)
                .ok_or(Error::InvalidInput {
                    reason: "arithmetic fetched-byte counter overflows u64",
                })?;
        }
        Ok(read)
    }
}

/// A caller-supplied probability-estimation state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QmState {
    pub qe: u16,
    pub next_lps: u8,
    pub next_mps: u8,
    pub switch_mps: bool,
}

/// A validated, caller-supplied probability-estimation table.
#[derive(Debug)]
pub struct QmTable {
    states: Box<[QmState]>,
}

impl QmTable {
    /// Validate the exact state count, interval sizes, and transition indices.
    pub fn new(states: Vec<QmState>) -> ArithmeticResult<Self> {
        if states.len() != QM_STATE_COUNT {
            return Err(ArithmeticError::configuration(
                ArithmeticErrorKind::InvalidTable("expected exactly 113 states"),
            ));
        }
        for state in &states {
            if state.qe == 0 || state.qe >= 0x8000 {
                return Err(ArithmeticError::configuration(
                    ArithmeticErrorKind::InvalidTable("Qe must be in 1..0x8000"),
                ));
            }
            if usize::from(state.next_lps) >= QM_STATE_COUNT
                || usize::from(state.next_mps) >= QM_STATE_COUNT
            {
                return Err(ArithmeticError::configuration(
                    ArithmeticErrorKind::InvalidTable("state transition is out of range"),
                ));
            }
        }
        Ok(Self {
            states: states.into_boxed_slice(),
        })
    }

    fn get(&self, index: u8) -> QmState {
        self.states[usize::from(index)]
    }
}

/// One context's current state and most-probable symbol.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ContextState {
    pub state_index: u8,
    pub mps: bool,
}

/// Contexts kept between stripes or reset at a stripe boundary.
#[derive(Debug)]
pub struct ContextBank {
    states: Vec<ContextState>,
    ready_for_carry: bool,
}

impl ContextBank {
    /// Allocate a nonempty context bank under the caller's allocation limit.
    pub fn new(count: usize, limits: &Limits) -> ArithmeticResult<Self> {
        limits
            .validate()
            .map_err(ArithmeticError::from_configuration_source)?;
        if count == 0 {
            return Err(ArithmeticError::configuration(
                ArithmeticErrorKind::InvalidContext,
            ));
        }
        let bytes = count
            .checked_mul(mem::size_of::<ContextState>())
            .and_then(|value| u64::try_from(value).ok())
            .ok_or_else(|| ArithmeticError::configuration(ArithmeticErrorKind::InvalidContext))?;
        limits
            .check_allocation(bytes)
            .map_err(ArithmeticError::from_configuration_source)?;
        let mut states = Vec::new();
        states
            .try_reserve_exact(count)
            .map_err(|_| ArithmeticError::configuration(ArithmeticErrorKind::InvalidContext))?;
        states.resize(count, ContextState::default());
        Ok(Self {
            states,
            ready_for_carry: false,
        })
    }

    /// Discard previous stripe probability estimates and require a new reset stripe.
    pub fn reset(&mut self) {
        self.states.fill(ContextState::default());
        self.ready_for_carry = false;
    }

    /// Read a context without exposing mutable decoder state.
    pub fn state(&self, index: usize) -> Option<ContextState> {
        self.states.get(index).copied()
    }

    /// Set one context with a checked state index; this does not enable carry.
    pub fn set(&mut self, index: usize, state: ContextState) -> ArithmeticResult<()> {
        let destination = self.states.get_mut(index).ok_or(ArithmeticError {
            offset: None,
            context: Some(index),
            kind: ArithmeticErrorKind::InvalidContext,
        })?;
        if usize::from(state.state_index) >= QM_STATE_COUNT {
            return Err(ArithmeticError {
                offset: None,
                context: Some(index),
                kind: ArithmeticErrorKind::InvalidState,
            });
        }
        *destination = state;
        Ok(())
    }
}

/// Range in the supplied `RangedSource` containing only the arithmetic bytes
/// of one stripe. Its offset is absolute in that source, which may itself be
/// an unstuffed logical or temporary source rather than the original document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EncodedSpan {
    pub offset: u64,
    pub length: u64,
}

/// Whether a new stripe starts with fresh contexts or carries the previous ones.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StripeMode {
    Reset,
    Carry,
}

/// Per-stripe bounds. Work counts each symbol, renormalization shift, and
/// byte input. Each byte input can cause at most one bounded 256-byte refill.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArithmeticBudget {
    pub max_symbols: u64,
    pub max_work: u64,
}

/// Typed failure category for this experimental decoder.
#[derive(Debug)]
pub enum ArithmeticErrorKind {
    InvalidTable(&'static str),
    InvalidContext,
    InvalidState,
    InvalidSpan(&'static str),
    InvalidBudget,
    UnreadyCarry,
    IncompleteStripe {
        expected: u64,
        decoded: u64,
    },
    LimitExceeded {
        resource: &'static str,
        limit: u64,
        attempted: u64,
    },
    Cancelled,
    Source(Error),
    Poisoned,
    Invariant(&'static str),
}

/// An arithmetic error with the next offset in the supplied `RangedSource`
/// and a context index when those locations exist. The offset is not
/// automatically an offset in the original document container.
#[derive(Debug)]
pub struct ArithmeticError {
    pub offset: Option<u64>,
    pub context: Option<usize>,
    pub kind: ArithmeticErrorKind,
}

pub type ArithmeticResult<T> = std::result::Result<T, ArithmeticError>;

impl ArithmeticError {
    fn configuration(kind: ArithmeticErrorKind) -> Self {
        Self {
            offset: None,
            context: None,
            kind,
        }
    }

    fn from_configuration_source(source: Error) -> Self {
        Self::configuration(ArithmeticErrorKind::Source(source))
    }
}

impl fmt::Display for ArithmeticError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("T.82 arithmetic decoder")?;
        if let Some(offset) = self.offset {
            write!(f, " at source byte {offset}")?;
        }
        if let Some(context) = self.context {
            write!(f, ", context {context}")?;
        }
        f.write_str(": ")?;
        match &self.kind {
            ArithmeticErrorKind::InvalidTable(reason) => write!(f, "invalid Qm table: {reason}"),
            ArithmeticErrorKind::InvalidContext => f.write_str("invalid context index or count"),
            ArithmeticErrorKind::InvalidState => f.write_str("invalid context state index"),
            ArithmeticErrorKind::InvalidSpan(reason) => write!(f, "invalid SCD span: {reason}"),
            ArithmeticErrorKind::InvalidBudget => f.write_str("invalid arithmetic work budget"),
            ArithmeticErrorKind::UnreadyCarry => {
                f.write_str("carry requires a completed previous stripe")
            }
            ArithmeticErrorKind::IncompleteStripe { expected, decoded } => {
                write!(
                    f,
                    "incomplete stripe: expected {expected} symbols, decoded {decoded}"
                )
            }
            ArithmeticErrorKind::LimitExceeded {
                resource,
                limit,
                attempted,
            } => write!(f, "{resource} limit {limit} exceeded by {attempted}"),
            ArithmeticErrorKind::Cancelled => f.write_str("cancelled"),
            ArithmeticErrorKind::Source(source) => write!(f, "source error: {source}"),
            ArithmeticErrorKind::Poisoned => f.write_str("decoder is poisoned after an error"),
            ArithmeticErrorKind::Invariant(reason) => write!(f, "internal invariant: {reason}"),
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
/// register; `interval` is A and `bit_counter` is CT in T.82 §6.8.3.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArithmeticSnapshot {
    pub interval: u32,
    pub code: u32,
    pub bit_counter: u8,
    pub next_input_offset: u64,
    pub physical_bytes_consumed: u64,
    /// Bytes returned by source reads, including the fixed-buffer prefetch.
    pub source_bytes_fetched: u64,
    pub virtual_zero_bytes: u64,
    pub symbols_decoded: u64,
    pub work_done: u64,
    pub poisoned: bool,
}

/// Incremental decoder for one already isolated arithmetic stripe.
pub struct ArithmeticDecoder<'a, S: RangedSource, C: Cancellation> {
    source: &'a mut S,
    span: EncodedSpan,
    table: &'a QmTable,
    contexts: &'a mut ContextBank,
    limits: &'a Limits,
    cancellation: &'a C,
    budget: ArithmeticBudget,
    input_buffer: [u8; INPUT_BUFFER_BYTES],
    buffered: usize,
    buffer_position: usize,
    physical_bytes_consumed: u64,
    source_bytes_fetched: u64,
    virtual_zero_bytes: u64,
    interval: u32,
    code: u32,
    bit_counter: u8,
    symbols_decoded: u64,
    work_done: u64,
    poisoned: bool,
}

impl<'a, S: RangedSource, C: Cancellation> ArithmeticDecoder<'a, S, C> {
    /// Initialize C with three byte-input events. Bytes beyond the declared
    /// span are virtual zero; a short read *inside* the span is an error.
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        source: &'a mut S,
        span: EncodedSpan,
        table: &'a QmTable,
        contexts: &'a mut ContextBank,
        mode: StripeMode,
        limits: &'a Limits,
        cancellation: &'a C,
        budget: ArithmeticBudget,
    ) -> ArithmeticResult<Self> {
        limits
            .validate()
            .map_err(ArithmeticError::from_configuration_source)?;
        limits
            .check_input_size(span.length)
            .map_err(|source| ArithmeticError {
                offset: Some(span.offset),
                context: None,
                kind: ArithmeticErrorKind::Source(source),
            })?;
        let end = span
            .offset
            .checked_add(span.length)
            .ok_or(ArithmeticError {
                offset: Some(span.offset),
                context: None,
                kind: ArithmeticErrorKind::InvalidSpan("end overflows u64"),
            })?;
        if span.offset > source.size() || end > source.size() {
            return Err(ArithmeticError {
                offset: Some(span.offset),
                context: None,
                kind: ArithmeticErrorKind::InvalidSpan("outside source size"),
            });
        }
        if contexts.states.is_empty() {
            return Err(ArithmeticError::configuration(
                ArithmeticErrorKind::InvalidContext,
            ));
        }
        if mode == StripeMode::Carry && !contexts.ready_for_carry {
            return Err(ArithmeticError::configuration(
                ArithmeticErrorKind::UnreadyCarry,
            ));
        }
        if budget.max_symbols == 0 || budget.max_work == 0 {
            return Err(ArithmeticError::configuration(
                ArithmeticErrorKind::InvalidBudget,
            ));
        }
        if mode == StripeMode::Reset {
            contexts.reset();
        } else {
            contexts.ready_for_carry = false;
        }
        let mut decoder = Self {
            source,
            span,
            table,
            contexts,
            limits,
            cancellation,
            budget,
            input_buffer: [0; INPUT_BUFFER_BYTES],
            buffered: 0,
            buffer_position: 0,
            physical_bytes_consumed: 0,
            source_bytes_fetched: 0,
            virtual_zero_bytes: 0,
            interval: 0x10000,
            code: 0,
            bit_counter: 0,
            symbols_decoded: 0,
            work_done: 0,
            poisoned: false,
        };
        decoder.byte_in(None).await?;
        decoder.code <<= 8;
        decoder.byte_in(None).await?;
        decoder.code <<= 8;
        decoder.byte_in(None).await?;
        Ok(decoder)
    }

    /// Decode one bit for a checked context. Any error after work starts
    /// poisons this stripe so a partial register update cannot be reused.
    pub async fn decode_symbol(&mut self, context: usize) -> ArithmeticResult<bool> {
        if self.poisoned {
            return Err(self.at(Some(context), ArithmeticErrorKind::Poisoned));
        }
        if self.contexts.states.get(context).is_none() {
            return Err(self.at(Some(context), ArithmeticErrorKind::InvalidContext));
        }
        self.check_cancelled(Some(context))?;
        let attempted = self.symbols_decoded.checked_add(1).ok_or_else(|| {
            self.at(
                Some(context),
                ArithmeticErrorKind::LimitExceeded {
                    resource: "symbols",
                    limit: self.budget.max_symbols,
                    attempted: u64::MAX,
                },
            )
        })?;
        if attempted > self.budget.max_symbols {
            return Err(self.at(
                Some(context),
                ArithmeticErrorKind::LimitExceeded {
                    resource: "symbols",
                    limit: self.budget.max_symbols,
                    attempted,
                },
            ));
        }
        // A caller may drop this future while source I/O is pending. Mark the
        // partially advanced registers unusable before the first await.
        self.poisoned = true;
        self.contexts.ready_for_carry = false;
        let result = self.decode_symbol_inner(context).await;
        match result {
            Ok(bit) => {
                self.symbols_decoded = attempted;
                self.poisoned = false;
                Ok(bit)
            }
            Err(error) => Err(error),
        }
    }

    /// Observe registers without mutating decoder, source, or contexts.
    pub fn snapshot(&self) -> ArithmeticSnapshot {
        ArithmeticSnapshot {
            interval: self.interval,
            code: self.code,
            bit_counter: self.bit_counter,
            next_input_offset: self.span.offset + self.physical_bytes_consumed,
            physical_bytes_consumed: self.physical_bytes_consumed,
            source_bytes_fetched: self.source_bytes_fetched,
            virtual_zero_bytes: self.virtual_zero_bytes,
            symbols_decoded: self.symbols_decoded,
            work_done: self.work_done,
            poisoned: self.poisoned,
        }
    }

    /// Read one context while the decoder holds the bank's mutable borrow.
    pub fn context_state(&self, index: usize) -> Option<ContextState> {
        self.contexts.state(index)
    }

    /// Complete exactly `expected_symbols` and make final contexts available
    /// to `Carry`.
    /// Dropping a decoder without finishing leaves the bank unavailable.
    pub fn finish(self, expected_symbols: u64) -> ArithmeticResult<()> {
        if self.poisoned {
            return Err(self.at(None, ArithmeticErrorKind::Poisoned));
        }
        self.check_cancelled(None)?;
        if self.symbols_decoded != expected_symbols {
            return Err(self.at(
                None,
                ArithmeticErrorKind::IncompleteStripe {
                    expected: expected_symbols,
                    decoded: self.symbols_decoded,
                },
            ));
        }
        self.contexts.ready_for_carry = true;
        Ok(())
    }

    fn at(&self, context: Option<usize>, kind: ArithmeticErrorKind) -> ArithmeticError {
        ArithmeticError {
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

    fn charge(&mut self, cost: u64, context: Option<usize>) -> ArithmeticResult<()> {
        let attempted = self.work_done.checked_add(cost).ok_or_else(|| {
            self.at(
                context,
                ArithmeticErrorKind::LimitExceeded {
                    resource: "arithmetic work",
                    limit: self.budget.max_work,
                    attempted: u64::MAX,
                },
            )
        })?;
        if attempted > self.budget.max_work {
            return Err(self.at(
                context,
                ArithmeticErrorKind::LimitExceeded {
                    resource: "arithmetic work",
                    limit: self.budget.max_work,
                    attempted,
                },
            ));
        }
        self.work_done = attempted;
        Ok(())
    }

    async fn next_byte(&mut self, context: Option<usize>) -> ArithmeticResult<u8> {
        self.check_cancelled(context)?;
        if self.physical_bytes_consumed == self.span.length {
            self.virtual_zero_bytes = self.virtual_zero_bytes.checked_add(1).ok_or_else(|| {
                self.at(
                    context,
                    ArithmeticErrorKind::Invariant("virtual zero count overflow"),
                )
            })?;
            return Ok(0);
        }
        if self.buffer_position == self.buffered {
            let remaining = self.span.length - self.physical_bytes_consumed;
            let count = remaining
                .min(INPUT_BUFFER_BYTES as u64)
                .min(self.limits.io_chunk_bytes as u64) as usize;
            let offset = self.span.offset + self.physical_bytes_consumed;
            let mut source = ProgressSource {
                inner: self.source,
                fetched: &mut self.source_bytes_fetched,
            };
            read_exact_at(
                &mut source,
                offset,
                &mut self.input_buffer[..count],
                self.limits,
                self.cancellation,
            )
            .await
            .map_err(|source| {
                let failed_at = match source {
                    Error::TruncatedInput {
                        offset, available, ..
                    } => offset.saturating_add(available),
                    _ => offset,
                };
                ArithmeticError {
                    offset: Some(failed_at),
                    context,
                    kind: match source {
                        Error::Cancelled => ArithmeticErrorKind::Cancelled,
                        other => ArithmeticErrorKind::Source(other),
                    },
                }
            })?;
            self.buffered = count;
            self.buffer_position = 0;
        }
        let byte = self.input_buffer[self.buffer_position];
        self.buffer_position += 1;
        self.physical_bytes_consumed += 1;
        Ok(byte)
    }

    async fn byte_in(&mut self, context: Option<usize>) -> ArithmeticResult<()> {
        self.charge(1, context)?;
        let byte = self.next_byte(context).await?;
        self.code = self.code.wrapping_add(u32::from(byte) << 8);
        self.bit_counter = 8;
        Ok(())
    }

    async fn renormalize(&mut self, context: usize) -> ArithmeticResult<()> {
        loop {
            self.check_cancelled(Some(context))?;
            if self.bit_counter == 0 {
                self.byte_in(Some(context)).await?;
            }
            self.charge(1, Some(context))?;
            self.interval <<= 1;
            self.code <<= 1;
            self.bit_counter -= 1;
            if self.interval >= 0x8000 {
                break;
            }
        }
        if self.bit_counter == 0 {
            self.byte_in(Some(context)).await?;
        }
        Ok(())
    }

    async fn decode_symbol_inner(&mut self, context: usize) -> ArithmeticResult<bool> {
        self.charge(1, Some(context))?;
        let current = self.contexts.states[context];
        let state = self.table.get(current.state_index);
        let qe = u32::from(state.qe);
        let narrowed = self.interval.checked_sub(qe).ok_or_else(|| {
            self.at(
                Some(context),
                ArithmeticErrorKind::Invariant("interval underflow"),
            )
        })?;
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
                self.renormalize(context).await?;
                (current.mps ^ exchange, next)
            } else {
                (current.mps, current)
            }
        } else {
            self.code = self.code.checked_sub(narrowed << 16).ok_or_else(|| {
                self.at(
                    Some(context),
                    ArithmeticErrorKind::Invariant("code underflow"),
                )
            })?;
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
            self.renormalize(context).await?;
            (current.mps ^ exchange, next)
        };
        self.contexts.states[context] = next;
        Ok(bit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::run;
    use std::{
        cell::Cell,
        future::Future,
        rc::Rc,
        task::{Context, Poll, Waker},
    };

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

        async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> crate::Result<usize> {
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
            if let Some((cancel_at, flag)) = &self.cancel_at {
                if offset == *cancel_at {
                    flag.set(true);
                }
            }
            Ok(count)
        }
    }

    struct PendingSource([u8; 4]);

    impl RangedSource for PendingSource {
        fn size(&self) -> u64 {
            4
        }

        async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> crate::Result<usize> {
            if offset == 3 {
                std::future::pending::<()>().await;
            }
            destination[0] = self.0[offset as usize];
            Ok(1)
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

    fn synthetic_table() -> QmTable {
        let mut states = vec![
            QmState {
                qe: 0x4000,
                next_lps: 1,
                next_mps: 2,
                switch_mps: true,
            };
            QM_STATE_COUNT
        ];
        states[1].qe = 0x5000;
        states[1].next_mps = 3;
        states[1].next_lps = 4;
        QmTable::new(states).unwrap()
    }

    fn budget() -> ArithmeticBudget {
        ArithmeticBudget {
            max_symbols: 1000,
            max_work: 100_000,
        }
    }

    fn span(length: u64) -> EncodedSpan {
        EncodedSpan { offset: 0, length }
    }

    #[test]
    fn validates_table_contexts_spans_and_budgets_before_input() {
        let state = QmState {
            qe: 1,
            next_lps: 0,
            next_mps: 0,
            switch_mps: false,
        };
        assert!(matches!(
            QmTable::new(vec![state; QM_STATE_COUNT - 1])
                .unwrap_err()
                .kind,
            ArithmeticErrorKind::InvalidTable(_)
        ));
        for invalid in [
            QmState { qe: 0, ..state },
            QmState {
                qe: 0x8000,
                ..state
            },
            QmState {
                next_lps: QM_STATE_COUNT as u8,
                ..state
            },
            QmState {
                next_mps: QM_STATE_COUNT as u8,
                ..state
            },
        ] {
            assert!(matches!(
                QmTable::new(vec![invalid; QM_STATE_COUNT])
                    .unwrap_err()
                    .kind,
                ArithmeticErrorKind::InvalidTable(_)
            ));
        }
        let limits = Limits::default();
        assert!(matches!(
            ContextBank::new(0, &limits).unwrap_err().kind,
            ArithmeticErrorKind::InvalidContext
        ));
        let mut tiny = limits;
        tiny.io_chunk_bytes = 1;
        tiny.max_allocation_bytes = 1;
        assert!(matches!(
            ContextBank::new(1, &tiny).unwrap_err().kind,
            ArithmeticErrorKind::Source(Error::LimitExceeded { .. })
        ));
        let table = synthetic_table();
        let mut contexts = ContextBank::new(2, &limits).unwrap();
        assert!(matches!(
            contexts.set(2, ContextState::default()).unwrap_err().kind,
            ArithmeticErrorKind::InvalidContext
        ));
        assert!(matches!(
            contexts
                .set(
                    1,
                    ContextState {
                        state_index: QM_STATE_COUNT as u8,
                        mps: false,
                    }
                )
                .unwrap_err()
                .kind,
            ArithmeticErrorKind::InvalidState
        ));
        let mut source = MockSource::new(&[0, 0, 0]);
        let invalid_span = run(ArithmeticDecoder::new(
            &mut source,
            EncodedSpan {
                offset: u64::MAX,
                length: 2,
            },
            &table,
            &mut contexts,
            StripeMode::Reset,
            &limits,
            &NEVER,
            budget(),
        ));
        assert!(matches!(
            invalid_span.err().unwrap().kind,
            ArithmeticErrorKind::InvalidSpan(_)
        ));
        assert_eq!(source.calls, 0);
        let mut overreport = MockSource::new(&[0, 0, 0]);
        overreport.overreport_at = Some(0);
        let error = run(ArithmeticDecoder::new(
            &mut overreport,
            span(3),
            &table,
            &mut contexts,
            StripeMode::Reset,
            &limits,
            &NEVER,
            budget(),
        ))
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
        let input_limit = run(ArithmeticDecoder::new(
            &mut source,
            EncodedSpan {
                offset: 1,
                length: 3,
            },
            &table,
            &mut contexts,
            StripeMode::Reset,
            &small_input,
            &NEVER,
            budget(),
        ))
        .err()
        .unwrap();
        assert_eq!(input_limit.offset, Some(1));
        assert!(matches!(
            input_limit.kind,
            ArithmeticErrorKind::Source(Error::LimitExceeded { .. })
        ));
        assert_eq!(source.calls, 0);
        let invalid_budget = run(ArithmeticDecoder::new(
            &mut source,
            span(3),
            &table,
            &mut contexts,
            StripeMode::Reset,
            &limits,
            &NEVER,
            ArithmeticBudget {
                max_symbols: 0,
                max_work: 1,
            },
        ));
        assert!(matches!(
            invalid_budget.err().unwrap().kind,
            ArithmeticErrorKind::InvalidBudget
        ));
        assert_eq!(source.calls, 0);
    }

    #[test]
    fn assigned_context_survives_rejected_spans_without_reading_or_resetting() {
        let table = synthetic_table();
        let limits = Limits::default();
        let mut contexts = ContextBank::new(2, &limits).unwrap();
        let assigned = ContextState {
            state_index: (QM_STATE_COUNT - 1) as u8,
            mps: true,
        };
        contexts.set(1, assigned).unwrap();
        assert_eq!(contexts.state(1), Some(assigned));

        // A failed assignment must not overwrite the last valid state.
        let invalid = ContextState {
            state_index: QM_STATE_COUNT as u8,
            mps: false,
        };
        assert!(matches!(
            contexts.set(1, invalid).unwrap_err().kind,
            ArithmeticErrorKind::InvalidState
        ));
        assert_eq!(contexts.state(1), Some(assigned));

        let mut source = MockSource::new(&[0, 0, 0]);
        for span in [
            EncodedSpan {
                offset: 2,
                length: 2,
            },
            EncodedSpan {
                offset: 4,
                length: 0,
            },
        ] {
            let error = run(ArithmeticDecoder::new(
                &mut source,
                span,
                &table,
                &mut contexts,
                StripeMode::Reset,
                &limits,
                &NEVER,
                budget(),
            ))
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
                    "T.82 arithmetic decoder at source byte {}: invalid SCD span: outside source size",
                    span.offset
                )
            );
            assert_eq!(source.calls, 0);
            assert_eq!(contexts.state(1), Some(assigned));
        }

        contexts.reset();
        assert_eq!(contexts.state(1), Some(ContextState::default()));
    }

    #[test]
    fn arithmetic_errors_keep_source_causes_and_actionable_locations() {
        let table = synthetic_table();
        let limits = Limits::default();
        let mut contexts = ContextBank::new(1, &limits).unwrap();

        let invalid_context = contexts.set(1, ContextState::default()).unwrap_err();
        assert_eq!(
            invalid_context.to_string(),
            "T.82 arithmetic decoder, context 1: invalid context index or count"
        );
        assert!(std::error::Error::source(&invalid_context).is_none());

        let mut short = MockSource::new(&[0, 0]);
        short.advertised_size = 3;
        let source_error = run(ArithmeticDecoder::new(
            &mut short,
            span(3),
            &table,
            &mut contexts,
            StripeMode::Reset,
            &limits,
            &NEVER,
            budget(),
        ))
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
        let mut decoder = run(ArithmeticDecoder::new(
            &mut source,
            span(3),
            &table,
            &mut contexts,
            StripeMode::Reset,
            &limits,
            &NEVER,
            ArithmeticBudget {
                max_symbols: 1,
                max_work: 100,
            },
        ))
        .unwrap();
        assert!(!run(decoder.decode_symbol(0)).unwrap());
        let limit = run(decoder.decode_symbol(0)).unwrap_err();
        assert_eq!(
            limit.to_string(),
            "T.82 arithmetic decoder at source byte 3, context 0: symbols limit 1 exceeded by 2"
        );
        assert!(std::error::Error::source(&limit).is_none());
        let incomplete = decoder.finish(2).unwrap_err();
        assert_eq!(
            incomplete.to_string(),
            "T.82 arithmetic decoder at source byte 3: incomplete stripe: expected 2 symbols, decoded 1"
        );
    }

    #[test]
    fn initializes_registers_with_checked_short_reads_and_virtual_zeros() {
        let table = synthetic_table();
        let limits = Limits {
            io_chunk_bytes: 2,
            ..Limits::default()
        };
        let mut contexts = ContextBank::new(1, &limits).unwrap();
        let mut source = MockSource::new(&[0x12, 0x34, 0x56, 0x78]);
        source.max_read = 1;
        let decoder = run(ArithmeticDecoder::new(
            &mut source,
            span(3),
            &table,
            &mut contexts,
            StripeMode::Reset,
            &limits,
            &NEVER,
            budget(),
        ))
        .unwrap();
        let snapshot = decoder.snapshot();
        assert_eq!(snapshot.interval, 0x10000);
        assert_eq!(snapshot.code, 0x12345600);
        assert_eq!(snapshot.bit_counter, 8);
        assert_eq!(snapshot.physical_bytes_consumed, 3);
        assert_eq!(snapshot.virtual_zero_bytes, 0);
        assert_eq!(snapshot.next_input_offset, 3);
        decoder.finish(0).unwrap();
        assert_eq!(source.calls, 3);

        let mut empty = MockSource::new(&[0xff]);
        let decoder = run(ArithmeticDecoder::new(
            &mut empty,
            span(0),
            &table,
            &mut contexts,
            StripeMode::Carry,
            &limits,
            &NEVER,
            budget(),
        ))
        .unwrap();
        assert_eq!(decoder.snapshot().code, 0);
        assert_eq!(decoder.snapshot().virtual_zero_bytes, 3);
        assert_eq!(decoder.snapshot().physical_bytes_consumed, 0);
        decoder.finish(0).unwrap();
        assert_eq!(empty.calls, 0);

        let mut short = MockSource::new(&[0xaa, 0xbb]);
        short.advertised_size = 3;
        let error = run(ArithmeticDecoder::new(
            &mut short,
            span(3),
            &table,
            &mut contexts,
            StripeMode::Carry,
            &limits,
            &NEVER,
            budget(),
        ))
        .err()
        .unwrap();
        assert_eq!(error.offset, Some(2));
        assert!(matches!(
            error.kind,
            ArithmeticErrorKind::Source(Error::TruncatedInput { .. })
        ));
        let mut after_failure = MockSource::new(&[]);
        let carry = run(ArithmeticDecoder::new(
            &mut after_failure,
            span(0),
            &table,
            &mut contexts,
            StripeMode::Carry,
            &limits,
            &NEVER,
            budget(),
        ));
        assert!(matches!(
            carry.err().unwrap().kind,
            ArithmeticErrorKind::UnreadyCarry
        ));
    }

    #[test]
    fn mps_lps_and_conditional_exchanges_update_contexts() {
        let table = synthetic_table();
        let limits = Limits::default();
        let mut contexts = ContextBank::new(1, &limits).unwrap();
        let mut mps_source = MockSource::new(&[0, 0, 0]);
        let mut decoder = run(ArithmeticDecoder::new(
            &mut mps_source,
            span(3),
            &table,
            &mut contexts,
            StripeMode::Reset,
            &limits,
            &NEVER,
            budget(),
        ))
        .unwrap();
        assert!(!run(decoder.decode_symbol(0)).unwrap());
        assert_eq!(decoder.snapshot().interval, 0xc000);
        assert_eq!(decoder.context_state(0), Some(ContextState::default()));
        assert!(!run(decoder.decode_symbol(0)).unwrap());
        assert_eq!(decoder.snapshot().interval, 0x8000);
        assert!(!run(decoder.decode_symbol(0)).unwrap());
        assert_eq!(decoder.snapshot().interval, 0x8000);
        assert_eq!(decoder.snapshot().bit_counter, 7);
        assert_eq!(
            decoder.context_state(0),
            Some(ContextState {
                state_index: 2,
                mps: false,
            })
        );
        decoder.finish(3).unwrap();

        let mut lps_source = MockSource::new(&[0xc0, 0, 0]);
        let mut decoder = run(ArithmeticDecoder::new(
            &mut lps_source,
            span(3),
            &table,
            &mut contexts,
            StripeMode::Reset,
            &limits,
            &NEVER,
            budget(),
        ))
        .unwrap();
        assert!(run(decoder.decode_symbol(0)).unwrap());
        assert_eq!(decoder.snapshot().interval, 0x8000);
        assert_eq!(
            decoder.context_state(0),
            Some(ContextState {
                state_index: 1,
                mps: true,
            })
        );
        assert!(!run(decoder.decode_symbol(0)).unwrap());
        assert_eq!(decoder.snapshot().interval, 0xc000);
        assert_eq!(decoder.snapshot().bit_counter, 5);
        assert_eq!(
            decoder.context_state(0),
            Some(ContextState {
                state_index: 4,
                mps: false,
            })
        );
        decoder.finish(2).unwrap();

        let mut exchanged_lps_source = MockSource::new(&[0xd8, 0, 0]);
        let mut decoder = run(ArithmeticDecoder::new(
            &mut exchanged_lps_source,
            span(3),
            &table,
            &mut contexts,
            StripeMode::Reset,
            &limits,
            &NEVER,
            budget(),
        ))
        .unwrap();
        assert!(run(decoder.decode_symbol(0)).unwrap());
        assert!(run(decoder.decode_symbol(0)).unwrap());
        assert_eq!(decoder.snapshot().interval, 0xa000);
        assert_eq!(
            decoder.context_state(0),
            Some(ContextState {
                state_index: 3,
                mps: true,
            })
        );
        decoder.finish(2).unwrap();
    }

    #[test]
    fn fixed_width_register_operations_and_small_work_budget_are_explicit() {
        let table = synthetic_table();
        let limits = Limits {
            io_chunk_bytes: 1,
            ..Limits::default()
        };
        let mut contexts = ContextBank::new(1, &limits).unwrap();
        let mut source = MockSource::new(&[0, 0, 0, 0xff]);
        let mut decoder = run(ArithmeticDecoder::new(
            &mut source,
            span(4),
            &table,
            &mut contexts,
            StripeMode::Reset,
            &limits,
            &NEVER,
            budget(),
        ))
        .unwrap();
        decoder.code = 0xffff_ff00;
        run(decoder.byte_in(None)).unwrap();
        assert_eq!(decoder.snapshot().code, 0x0000_fe00);
        assert_eq!(decoder.snapshot().physical_bytes_consumed, 4);
        decoder.interval = 0x7fff;
        decoder.bit_counter = 8;
        run(decoder.renormalize(0)).unwrap();
        assert_eq!(decoder.snapshot().interval, 0xfffe);
        assert_eq!(decoder.snapshot().bit_counter, 7);
        decoder.budget.max_symbols = u64::MAX;
        decoder.symbols_decoded = u64::MAX;
        assert!(matches!(
            run(decoder.decode_symbol(0)).unwrap_err().kind,
            ArithmeticErrorKind::LimitExceeded {
                resource: "symbols",
                limit: u64::MAX,
                attempted: u64::MAX,
            }
        ));
        assert!(!decoder.snapshot().poisoned);
        decoder.symbols_decoded = 0;
        decoder.budget.max_work = u64::MAX;
        decoder.work_done = u64::MAX;
        assert!(matches!(
            decoder.charge(1, None).unwrap_err().kind,
            ArithmeticErrorKind::LimitExceeded {
                resource: "arithmetic work",
                limit: u64::MAX,
                attempted: u64::MAX,
            }
        ));
        decoder.finish(0).unwrap();

        let mut empty = MockSource::new(&[]);
        let mut decoder = run(ArithmeticDecoder::new(
            &mut empty,
            span(0),
            &table,
            &mut contexts,
            StripeMode::Reset,
            &limits,
            &NEVER,
            budget(),
        ))
        .unwrap();
        decoder.virtual_zero_bytes = u64::MAX;
        assert!(matches!(
            run(decoder.byte_in(None)).unwrap_err().kind,
            ArithmeticErrorKind::Invariant("virtual zero count overflow")
        ));
        decoder.finish(0).unwrap();

        let broad_limits = Limits::default();
        let mut large_span = MockSource::new(&[0; INPUT_BUFFER_BYTES]);
        let mut decoder = run(ArithmeticDecoder::new(
            &mut large_span,
            span(INPUT_BUFFER_BYTES as u64),
            &table,
            &mut contexts,
            StripeMode::Reset,
            &broad_limits,
            &NEVER,
            ArithmeticBudget {
                max_symbols: 1,
                max_work: 4,
            },
        ))
        .unwrap();
        assert!(!run(decoder.decode_symbol(0)).unwrap());
        assert_eq!(decoder.snapshot().work_done, 4);
        let error = run(decoder.decode_symbol(0)).unwrap_err();
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
        let state = QmState {
            qe: 1,
            next_lps: 0,
            next_mps: 0,
            switch_mps: false,
        };
        let table = QmTable::new(vec![state; QM_STATE_COUNT]).unwrap();
        let limits = Limits {
            io_chunk_bytes: 1,
            ..Limits::default()
        };
        let mut contexts = ContextBank::new(1, &limits).unwrap();
        let cancelled = Rc::new(Cell::new(true));
        let flag = Flag(Some(cancelled.clone()));
        let mut before_start = MockSource::new(&[0, 0, 0]);
        let error = run(ArithmeticDecoder::new(
            &mut before_start,
            span(3),
            &table,
            &mut contexts,
            StripeMode::Reset,
            &limits,
            &flag,
            budget(),
        ))
        .err()
        .unwrap();
        assert!(matches!(error.kind, ArithmeticErrorKind::Cancelled));
        assert_eq!(before_start.calls, 0);

        cancelled.set(false);
        let mut during_symbol = MockSource::new(&[0xff, 0xff, 0, 0x55]);
        during_symbol.cancel_at = Some((3, cancelled.clone()));
        let mut decoder = run(ArithmeticDecoder::new(
            &mut during_symbol,
            span(4),
            &table,
            &mut contexts,
            StripeMode::Reset,
            &limits,
            &flag,
            budget(),
        ))
        .unwrap();
        let error = run(decoder.decode_symbol(0)).unwrap_err();
        assert_eq!(error.offset, Some(3));
        assert_eq!(error.context, Some(0));
        assert!(matches!(error.kind, ArithmeticErrorKind::Cancelled));
        assert!(decoder.snapshot().poisoned);
        assert_eq!(decoder.context_state(0), Some(ContextState::default()));
        assert!(matches!(
            run(decoder.decode_symbol(0)).unwrap_err().kind,
            ArithmeticErrorKind::Poisoned
        ));
        assert!(matches!(
            decoder.finish(1).unwrap_err().kind,
            ArithmeticErrorKind::Poisoned
        ));
        assert_eq!(during_symbol.calls, 4);

        let mut short = MockSource::new(&[0xff, 0xff, 0]);
        short.advertised_size = 4;
        let mut decoder = run(ArithmeticDecoder::new(
            &mut short,
            span(4),
            &table,
            &mut contexts,
            StripeMode::Reset,
            &limits,
            &NEVER,
            budget(),
        ))
        .unwrap();
        let error = run(decoder.decode_symbol(0)).unwrap_err();
        assert_eq!(error.offset, Some(3));
        assert_eq!(error.context, Some(0));
        assert!(matches!(
            error.kind,
            ArithmeticErrorKind::Source(Error::TruncatedInput { .. })
        ));
        assert!(decoder.snapshot().poisoned);
        assert!(matches!(
            run(decoder.decode_symbol(0)).unwrap_err().kind,
            ArithmeticErrorKind::Poisoned
        ));
        assert!(matches!(
            decoder.finish(1).unwrap_err().kind,
            ArithmeticErrorKind::Poisoned
        ));
        let mut next = MockSource::new(&[]);
        assert!(matches!(
            run(ArithmeticDecoder::new(
                &mut next,
                span(0),
                &table,
                &mut contexts,
                StripeMode::Carry,
                &limits,
                &NEVER,
                budget(),
            ))
            .err()
            .unwrap()
            .kind,
            ArithmeticErrorKind::UnreadyCarry
        ));
    }

    #[test]
    fn dropping_a_pending_symbol_future_poisons_the_decoder() {
        let state = QmState {
            qe: 1,
            next_lps: 0,
            next_mps: 0,
            switch_mps: false,
        };
        let table = QmTable::new(vec![state; QM_STATE_COUNT]).unwrap();
        let limits = Limits {
            io_chunk_bytes: 1,
            ..Limits::default()
        };
        let mut contexts = ContextBank::new(1, &limits).unwrap();
        let mut source = PendingSource([0xff, 0xff, 0, 0x55]);
        let mut decoder = run(ArithmeticDecoder::new(
            &mut source,
            span(4),
            &table,
            &mut contexts,
            StripeMode::Reset,
            &limits,
            &NEVER,
            budget(),
        ))
        .unwrap();
        let mut pending = Box::pin(decoder.decode_symbol(0));
        let mut task_context = Context::from_waker(Waker::noop());
        assert!(matches!(
            pending.as_mut().poll(&mut task_context),
            Poll::Pending
        ));
        drop(pending);
        assert!(decoder.snapshot().poisoned);
        assert!(matches!(
            run(decoder.decode_symbol(0)).unwrap_err().kind,
            ArithmeticErrorKind::Poisoned
        ));
        let mut next = MockSource::new(&[]);
        assert!(matches!(
            run(ArithmeticDecoder::new(
                &mut next,
                span(0),
                &table,
                &mut contexts,
                StripeMode::Carry,
                &limits,
                &NEVER,
                budget(),
            ))
            .err()
            .unwrap()
            .kind,
            ArithmeticErrorKind::UnreadyCarry
        ));
    }

    #[test]
    fn work_error_poisoning_and_invalid_context_preflight_are_distinct() {
        let table = synthetic_table();
        let limits = Limits::default();
        let mut contexts = ContextBank::new(1, &limits).unwrap();
        let mut source = MockSource::new(&[0, 0, 0]);
        let mut decoder = run(ArithmeticDecoder::new(
            &mut source,
            span(3),
            &table,
            &mut contexts,
            StripeMode::Reset,
            &limits,
            &NEVER,
            budget(),
        ))
        .unwrap();
        let error = run(decoder.decode_symbol(1)).unwrap_err();
        assert_eq!(error.context, Some(1));
        assert!(matches!(error.kind, ArithmeticErrorKind::InvalidContext));
        assert!(!decoder.snapshot().poisoned);
        assert!(!run(decoder.decode_symbol(0)).unwrap());
        decoder.finish(1).unwrap();

        let mut source = MockSource::new(&[0xc0, 0, 0]);
        let mut decoder = run(ArithmeticDecoder::new(
            &mut source,
            span(3),
            &table,
            &mut contexts,
            StripeMode::Reset,
            &limits,
            &NEVER,
            ArithmeticBudget {
                max_symbols: 1,
                max_work: 4,
            },
        ))
        .unwrap();
        let error = run(decoder.decode_symbol(0)).unwrap_err();
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
    fn carry_requires_an_exactly_finished_stripe_and_reset_discards_state() {
        let table = synthetic_table();
        let limits = Limits::default();
        let mut contexts = ContextBank::new(1, &limits).unwrap();
        let mut cold = MockSource::new(&[]);
        assert!(matches!(
            run(ArithmeticDecoder::new(
                &mut cold,
                span(0),
                &table,
                &mut contexts,
                StripeMode::Carry,
                &limits,
                &NEVER,
                budget(),
            ))
            .err()
            .unwrap()
            .kind,
            ArithmeticErrorKind::UnreadyCarry
        ));
        assert_eq!(cold.calls, 0);

        let mut first = MockSource::new(&[0xc0, 0, 0]);
        let mut decoder = run(ArithmeticDecoder::new(
            &mut first,
            span(3),
            &table,
            &mut contexts,
            StripeMode::Reset,
            &limits,
            &NEVER,
            budget(),
        ))
        .unwrap();
        assert!(run(decoder.decode_symbol(0)).unwrap());
        assert!(matches!(
            decoder.finish(2).unwrap_err().kind,
            ArithmeticErrorKind::IncompleteStripe {
                expected: 2,
                decoded: 1,
            }
        ));
        let mut after_incomplete = MockSource::new(&[]);
        assert!(matches!(
            run(ArithmeticDecoder::new(
                &mut after_incomplete,
                span(0),
                &table,
                &mut contexts,
                StripeMode::Carry,
                &limits,
                &NEVER,
                budget(),
            ))
            .err()
            .unwrap()
            .kind,
            ArithmeticErrorKind::UnreadyCarry
        ));

        let mut first = MockSource::new(&[0xc0, 0, 0]);
        let mut decoder = run(ArithmeticDecoder::new(
            &mut first,
            span(3),
            &table,
            &mut contexts,
            StripeMode::Reset,
            &limits,
            &NEVER,
            budget(),
        ))
        .unwrap();
        assert!(run(decoder.decode_symbol(0)).unwrap());
        decoder.finish(1).unwrap();
        let carried = ContextState {
            state_index: 1,
            mps: true,
        };
        assert_eq!(contexts.state(0), Some(carried));

        let mut second = MockSource::new(&[0, 0, 0]);
        let mut decoder = run(ArithmeticDecoder::new(
            &mut second,
            span(3),
            &table,
            &mut contexts,
            StripeMode::Carry,
            &limits,
            &NEVER,
            budget(),
        ))
        .unwrap();
        assert_eq!(decoder.context_state(0), Some(carried));
        assert!(run(decoder.decode_symbol(0)).unwrap());
        decoder.finish(1).unwrap();
        assert_eq!(contexts.state(0), Some(carried));

        let mut third = MockSource::new(&[0, 0, 0]);
        let mut decoder = run(ArithmeticDecoder::new(
            &mut third,
            span(3),
            &table,
            &mut contexts,
            StripeMode::Reset,
            &limits,
            &NEVER,
            budget(),
        ))
        .unwrap();
        assert_eq!(decoder.context_state(0), Some(ContextState::default()));
        assert!(!run(decoder.decode_symbol(0)).unwrap());
        let mut after_drop = MockSource::new(&[]);
        assert!(matches!(
            run(ArithmeticDecoder::new(
                &mut after_drop,
                span(0),
                &table,
                &mut contexts,
                StripeMode::Carry,
                &limits,
                &NEVER,
                budget(),
            ))
            .err()
            .unwrap()
            .kind,
            ArithmeticErrorKind::UnreadyCarry
        ));
        contexts.reset();
        assert_eq!(contexts.state(0), Some(ContextState::default()));
    }

    #[test]
    fn fixed_budget_mutations_are_deterministic_and_bounded() {
        fn trace(input: &[u8; 8]) -> (Vec<bool>, ArithmeticSnapshot) {
            let table = synthetic_table();
            let limits = Limits::default();
            let mut contexts = ContextBank::new(2, &limits).unwrap();
            let mut source = MockSource::new(input);
            let mut decoder = run(ArithmeticDecoder::new(
                &mut source,
                span(input.len() as u64),
                &table,
                &mut contexts,
                StripeMode::Reset,
                &limits,
                &NEVER,
                ArithmeticBudget {
                    max_symbols: 64,
                    max_work: 512,
                },
            ))
            .unwrap();
            let mut bits = Vec::new();
            for symbol in 0..64 {
                bits.push(run(decoder.decode_symbol(symbol % 2)).unwrap());
            }
            let snapshot = decoder.snapshot();
            assert_eq!(snapshot.symbols_decoded, 64);
            assert!(snapshot.work_done <= 512);
            assert!(snapshot.physical_bytes_consumed <= input.len() as u64);
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
        let table = synthetic_table();
        let state = QmState {
            qe: 1,
            next_lps: 0,
            next_mps: 0,
            switch_mps: false,
        };
        let wrong_count = QmTable::new(vec![state; 1]).unwrap_err();
        assert_eq!(
            wrong_count.to_string(),
            "T.82 arithmetic decoder: invalid Qm table: expected exactly 113 states"
        );
        let mut contexts = ContextBank::new(2, &limits).unwrap();
        let bad_state = contexts
            .set(
                0,
                ContextState {
                    state_index: u8::MAX,
                    mps: true,
                },
            )
            .unwrap_err();
        assert_eq!(
            bad_state.to_string(),
            "T.82 arithmetic decoder, context 0: invalid context state index"
        );

        let mut source = MockSource::new(&[0, 0, 0]);
        let unready = run(ArithmeticDecoder::new(
            &mut source,
            span(3),
            &table,
            &mut contexts,
            StripeMode::Carry,
            &limits,
            &NEVER,
            budget(),
        ))
        .err()
        .unwrap();
        assert_eq!(
            unready.to_string(),
            "T.82 arithmetic decoder: carry requires a completed previous stripe"
        );
        let zero_budget = run(ArithmeticDecoder::new(
            &mut source,
            span(3),
            &table,
            &mut contexts,
            StripeMode::Reset,
            &limits,
            &NEVER,
            ArithmeticBudget {
                max_symbols: 1,
                max_work: 0,
            },
        ))
        .err()
        .unwrap();
        assert_eq!(
            zero_budget.to_string(),
            "T.82 arithmetic decoder: invalid arithmetic work budget"
        );
        let cancelled = run(ArithmeticDecoder::new(
            &mut source,
            span(3),
            &table,
            &mut contexts,
            StripeMode::Reset,
            &limits,
            &Flag(Some(Rc::new(Cell::new(true)))),
            budget(),
        ))
        .err()
        .unwrap();
        assert_eq!(
            cancelled.to_string(),
            "T.82 arithmetic decoder at source byte 0: cancelled"
        );
        assert_eq!(source.calls, 0);

        for (kind, message) in [
            (
                ArithmeticErrorKind::Poisoned,
                "decoder is poisoned after an error",
            ),
            (
                ArithmeticErrorKind::Invariant("register"),
                "internal invariant: register",
            ),
        ] {
            let error = ArithmeticError {
                offset: Some(9),
                context: Some(4),
                kind,
            };
            assert_eq!(
                error.to_string(),
                format!("T.82 arithmetic decoder at source byte 9, context 4: {message}")
            );
            assert!(std::error::Error::source(&error).is_none());
        }
    }

    #[test]
    fn an_empty_context_bank_is_rejected_before_input() {
        // `ContextBank::new` never builds an empty bank; the decoder still
        // refuses one rather than indexing it.
        let mut empty = ContextBank {
            states: Vec::new(),
            ready_for_carry: true,
        };
        let table = synthetic_table();
        let mut source = MockSource::new(&[0, 0, 0]);
        let error = run(ArithmeticDecoder::new(
            &mut source,
            span(3),
            &table,
            &mut empty,
            StripeMode::Carry,
            &Limits::default(),
            &NEVER,
            budget(),
        ))
        .err()
        .unwrap();
        assert!(matches!(error.kind, ArithmeticErrorKind::InvalidContext));
        assert_eq!((error.offset, error.context), (None, None));
        assert_eq!(source.calls, 0);
        assert!(empty.ready_for_carry);
    }
}
