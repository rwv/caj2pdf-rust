// SPDX-License-Identifier: MIT

//! Experimental T.88 Annex E MQ arithmetic control flow for one bounded stream.
//!
//! The 47 probability states are supplied by the caller. This module contains
//! no published Table E.1 entries, Annex H vector, JBIG2 image model, or
//! container parser. It is independent of the T.82 `qm` stripe decoder.

use crate::{Cancellation, Error, Limits, RangedSource, read_exact_at};
use std::{error, fmt, mem};

pub const MQ_STATE_COUNT: usize = 47;
const INPUT_BUFFER_BYTES: usize = 256;

fn allocation_bytes(contexts: usize) -> MqResult<u64> {
    contexts
        .checked_mul(mem::size_of::<MqContext>())
        .and_then(|value| value.checked_add(MQ_STATE_COUNT * mem::size_of::<MqState>()))
        .and_then(|value| value.checked_add(INPUT_BUFFER_BYTES))
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| MqError::configuration(MqErrorKind::InvalidContext))
}

/// Count completed source reads even when a later short read or cancellation
/// makes the enclosing checked refill fail.
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
                    reason: "MQ fetched-byte counter overflows u64",
                })?;
        }
        Ok(read)
    }
}

/// One caller-supplied probability state, in T.88 Table E.1 column order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MqState {
    pub qe: u16,
    pub next_mps: u8,
    pub next_lps: u8,
    pub switch_mps: bool,
}

/// An owned, validated state machine; no normative values are bundled.
#[derive(Debug)]
pub struct MqTable {
    states: Box<[MqState]>,
}

impl MqTable {
    pub fn new(states: Vec<MqState>, limits: &Limits) -> MqResult<Self> {
        if states.len() != MQ_STATE_COUNT {
            return Err(MqError::configuration(MqErrorKind::InvalidTable(
                "expected exactly 47 states",
            )));
        }
        limits
            .check_allocation((MQ_STATE_COUNT * mem::size_of::<MqState>()) as u64)
            .map_err(MqError::configuration_source)?;
        for state in &states {
            if state.qe == 0 || state.qe >= 0x8000 {
                return Err(MqError::configuration(MqErrorKind::InvalidTable(
                    "Qe must be in 1..0x8000",
                )));
            }
            if usize::from(state.next_mps) >= MQ_STATE_COUNT
                || usize::from(state.next_lps) >= MQ_STATE_COUNT
            {
                return Err(MqError::configuration(MqErrorKind::InvalidTable(
                    "transition points outside the table",
                )));
            }
        }
        Ok(Self {
            states: states.into_boxed_slice(),
        })
    }

    fn state(&self, index: u8) -> MqState {
        self.states[usize::from(index)]
    }
}

/// The current probability index and more-probable bit of one context.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MqContext {
    pub state_index: u8,
    pub mps: bool,
}

/// Context storage controlled by the caller's JBIG2 model.
///
/// A newly allocated bank starts at index zero and MPS zero. The caller must
/// decide when a coding procedure resets or preserves it; no T.82 stripe rule
/// is applied here.
#[derive(Debug)]
pub struct MqContexts {
    states: Vec<MqContext>,
    // Set only by the typed IAID owner. A raw context array cannot claim to
    // contain an IAID model or reinterpret old statistics at a new width.
    iaid_code_len: Option<u32>,
}

impl MqContexts {
    pub fn new(count: usize, limits: &Limits, budget: &MqBudget) -> MqResult<Self> {
        limits.validate().map_err(MqError::configuration_source)?;
        budget.validate()?;
        if count == 0 {
            return Err(MqError::configuration(MqErrorKind::InvalidContext));
        }
        if count > budget.max_contexts {
            return Err(MqError::configuration(MqErrorKind::LimitExceeded {
                resource: "MQ contexts",
                limit: budget.max_contexts as u64,
                attempted: count as u64,
            }));
        }
        let bytes = allocation_bytes(count)?;
        limits
            .check_allocation(bytes)
            .map_err(MqError::configuration_source)?;
        let mut states = Vec::new();
        states
            .try_reserve_exact(count)
            .map_err(|_| MqError::configuration(MqErrorKind::AllocationFailed))?;
        states.resize(count, MqContext::default());
        Ok(Self {
            states,
            iaid_code_len: None,
        })
    }

    pub fn reset(&mut self) {
        self.states.fill(MqContext::default());
    }

    pub fn get(&self, index: usize) -> Option<MqContext> {
        self.states.get(index).copied()
    }

    /// Number of independently adapted MQ contexts in this bank.
    pub fn count(&self) -> usize {
        self.states.len()
    }

    pub(super) fn bind_iaid_code_len(&mut self, code_len: u32) {
        self.iaid_code_len = Some(code_len);
    }

    pub fn set(&mut self, index: usize, state: MqContext) -> MqResult<()> {
        if usize::from(state.state_index) >= MQ_STATE_COUNT {
            return Err(MqError {
                offset: None,
                context: Some(index),
                kind: MqErrorKind::InvalidState,
            });
        }
        let destination = self.states.get_mut(index).ok_or(MqError {
            offset: None,
            context: Some(index),
            kind: MqErrorKind::InvalidContext,
        })?;
        *destination = state;
        Ok(())
    }
}

/// Exactly one MQ byte stream, including its terminal bytes, in source coordinates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MqSpan {
    pub offset: u64,
    pub length: u64,
}

/// Per-stream CPU, input, context, and synthesized terminal bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MqBudget {
    pub max_span_bytes: u64,
    pub max_contexts: usize,
    pub max_symbols: u64,
    /// Includes symbol decisions, renormalization shifts, byte-input events,
    /// and physical bytes fetched into the fixed input buffer.
    pub max_work: u64,
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
    fn validate(&self) -> MqResult<()> {
        if self.max_span_bytes == 0
            || self.max_contexts == 0
            || self.max_symbols == 0
            || self.max_work == 0
        {
            return Err(MqError::configuration(MqErrorKind::InvalidBudget));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct MqError {
    pub offset: Option<u64>,
    pub context: Option<usize>,
    pub kind: MqErrorKind,
}

#[derive(Debug)]
pub enum MqErrorKind {
    InvalidTable(&'static str),
    InvalidContext,
    InvalidState,
    InvalidSpan(&'static str),
    InvalidBudget,
    MissingTerminator,
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
    Invariant(&'static str),
    WrongSymbolCount {
        expected: u64,
        decoded: u64,
    },
}

pub type MqResult<T> = std::result::Result<T, MqError>;

impl MqError {
    pub(super) fn configuration(kind: MqErrorKind) -> Self {
        Self {
            offset: None,
            context: None,
            kind,
        }
    }

    fn configuration_source(source: Error) -> Self {
        Self::configuration(MqErrorKind::Source(source))
    }
}

impl fmt::Display for MqError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("T.88 MQ decoder")?;
        if let Some(offset) = self.offset {
            write!(f, " at source byte {offset}")?;
        }
        if let Some(context) = self.context {
            write!(f, ", context {context}")?;
        }
        f.write_str(": ")?;
        match &self.kind {
            MqErrorKind::InvalidTable(reason) => write!(f, "invalid table: {reason}"),
            MqErrorKind::InvalidContext => f.write_str("invalid context index or count"),
            MqErrorKind::InvalidState => f.write_str("invalid context state index"),
            MqErrorKind::InvalidSpan(reason) => write!(f, "invalid MQ span: {reason}"),
            MqErrorKind::InvalidBudget => f.write_str("invalid MQ budget"),
            MqErrorKind::MissingTerminator => f.write_str("missing terminal marker"),
            MqErrorKind::InvalidMarker(second) => {
                write!(f, "invalid terminal marker following 0xFF: {second:#04x}")
            }
            MqErrorKind::LimitExceeded {
                resource,
                limit,
                attempted,
            } => write!(f, "{resource} limit {limit} exceeded by {attempted}"),
            MqErrorKind::AllocationFailed => f.write_str("context allocation failed"),
            MqErrorKind::Cancelled => f.write_str("cancelled"),
            MqErrorKind::Source(source) => write!(f, "source error: {source}"),
            MqErrorKind::Poisoned => f.write_str("decoder state is poisoned"),
            MqErrorKind::Invariant(reason) => write!(f, "internal invariant: {reason}"),
            MqErrorKind::WrongSymbolCount { expected, decoded } => {
                write!(f, "expected {expected} symbols but decoded {decoded}")
            }
        }
    }
}

impl error::Error for MqError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.kind {
            MqErrorKind::Source(source) => Some(source),
            _ => None,
        }
    }
}

/// The code register C is 32 bits; A and CT are the interval and bit counter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MqSnapshot {
    pub interval: u32,
    pub code: u32,
    pub bit_counter: u8,
    pub current_input_offset: u64,
    /// Source bytes fetched, including bounded lookahead and prefetch.
    pub source_bytes_fetched: u64,
    pub terminal_inputs: u64,
    pub symbols_decoded: u64,
    pub work_done: u64,
    pub poisoned: bool,
}

/// Decoder of one MQ arithmetic substream; no JBIG2 image pixels are produced.
pub struct MqDecoder<'a, S: RangedSource, C: Cancellation> {
    source: &'a mut S,
    span: MqSpan,
    table: &'a MqTable,
    contexts: &'a mut MqContexts,
    limits: &'a Limits,
    cancellation: &'a C,
    budget: MqBudget,
    buffer: [u8; INPUT_BUFFER_BYTES],
    buffer_start: u64,
    buffered: usize,
    bp: u64,
    current_byte: u8,
    source_bytes_fetched: u64,
    terminal_inputs: u64,
    interval: u32,
    code: u32,
    bit_counter: u8,
    symbols_decoded: u64,
    work_done: u64,
    poisoned: bool,
}

impl<'a, S: RangedSource, C: Cancellation> MqDecoder<'a, S, C> {
    pub(super) fn iaid_code_len(&self) -> Option<u32> {
        self.contexts.iaid_code_len
    }

    pub(super) fn check_ready(&self, context: Option<usize>) -> MqResult<()> {
        if self.poisoned {
            return Err(self.at(context, MqErrorKind::Poisoned));
        }
        self.check_cancelled(context)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        source: &'a mut S,
        span: MqSpan,
        table: &'a MqTable,
        contexts: &'a mut MqContexts,
        limits: &'a Limits,
        cancellation: &'a C,
        budget: MqBudget,
    ) -> MqResult<Self> {
        let mut ignored_fetched = 0;
        Self::new_with_init_progress(
            source,
            span,
            table,
            contexts,
            limits,
            cancellation,
            budget,
            &mut ignored_fetched,
        )
        .await
    }

    /// Keep the physical prefetch count when initialization returns an error
    /// before a decoder and its regular snapshot can be returned.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn new_with_init_progress(
        source: &'a mut S,
        span: MqSpan,
        table: &'a MqTable,
        contexts: &'a mut MqContexts,
        limits: &'a Limits,
        cancellation: &'a C,
        budget: MqBudget,
        fetched: &mut u64,
    ) -> MqResult<Self> {
        *fetched = 0;
        limits.validate().map_err(MqError::configuration_source)?;
        budget.validate()?;
        if span.length < 2 {
            return Err(MqError {
                offset: Some(span.offset),
                context: None,
                kind: MqErrorKind::InvalidSpan("requires at least two terminal bytes"),
            });
        }
        if span.length > budget.max_span_bytes {
            return Err(MqError {
                offset: Some(span.offset),
                context: None,
                kind: MqErrorKind::LimitExceeded {
                    resource: "MQ span bytes",
                    limit: budget.max_span_bytes,
                    attempted: span.length,
                },
            });
        }
        limits
            .check_input_size(span.length)
            .map_err(|source| MqError {
                offset: Some(span.offset),
                context: None,
                kind: MqErrorKind::Source(source),
            })?;
        let end = span.offset.checked_add(span.length).ok_or(MqError {
            offset: Some(span.offset),
            context: None,
            kind: MqErrorKind::InvalidSpan("end overflows u64"),
        })?;
        if end > source.size() {
            return Err(MqError {
                offset: Some(span.offset),
                context: None,
                kind: MqErrorKind::InvalidSpan("outside source size"),
            });
        }
        if contexts.states.is_empty() || contexts.states.len() > budget.max_contexts {
            return Err(MqError::configuration(MqErrorKind::InvalidContext));
        }
        limits
            .check_allocation(allocation_bytes(contexts.states.len())?)
            .map_err(MqError::configuration_source)?;
        let mut decoder = Self {
            source,
            span,
            table,
            contexts,
            limits,
            cancellation,
            budget,
            buffer: [0; INPUT_BUFFER_BYTES],
            buffer_start: 0,
            buffered: 0,
            bp: 0,
            current_byte: 0,
            source_bytes_fetched: 0,
            terminal_inputs: 0,
            interval: 0x8000,
            code: 0,
            bit_counter: 0,
            symbols_decoded: 0,
            work_done: 0,
            poisoned: false,
        };
        let initialized = async {
            decoder.check_cancelled(None)?;
            decoder.current_byte = decoder.read_byte(0, None).await?;
            decoder.code = u32::from(decoder.current_byte) << 16;
            decoder.byte_in(None).await?;
            decoder.code <<= 7;
            decoder.bit_counter -= 7;
            Ok::<(), MqError>(())
        }
        .await;
        *fetched = decoder.source_bytes_fetched;
        initialized?;
        Ok(decoder)
    }

    pub async fn decode_bit(&mut self, context: usize) -> MqResult<bool> {
        if self.poisoned {
            return Err(self.at(Some(context), MqErrorKind::Poisoned));
        }
        if self.contexts.states.get(context).is_none() {
            return Err(self.at(Some(context), MqErrorKind::InvalidContext));
        }
        self.check_cancelled(Some(context))?;
        let attempted = self.symbols_decoded.checked_add(1).ok_or_else(|| {
            self.at(
                Some(context),
                MqErrorKind::LimitExceeded {
                    resource: "MQ symbols",
                    limit: self.budget.max_symbols,
                    attempted: u64::MAX,
                },
            )
        })?;
        if attempted > self.budget.max_symbols {
            return Err(self.at(
                Some(context),
                MqErrorKind::LimitExceeded {
                    resource: "MQ symbols",
                    limit: self.budget.max_symbols,
                    attempted,
                },
            ));
        }
        // A source future can be dropped after a partial register update.
        self.poisoned = true;
        let result = self.decode_bit_inner(context).await;
        match result {
            Ok(bit) => {
                self.symbols_decoded = attempted;
                self.poisoned = false;
                Ok(bit)
            }
            Err(error) => Err(error),
        }
    }

    pub fn snapshot(&self) -> MqSnapshot {
        MqSnapshot {
            interval: self.interval,
            code: self.code,
            bit_counter: self.bit_counter,
            current_input_offset: self.span.offset + self.bp,
            source_bytes_fetched: self.source_bytes_fetched,
            terminal_inputs: self.terminal_inputs,
            symbols_decoded: self.symbols_decoded,
            work_done: self.work_done,
            poisoned: self.poisoned,
        }
    }

    pub fn context(&self, index: usize) -> Option<MqContext> {
        self.contexts.get(index)
    }

    /// The size of the caller-owned context bank, including other models.
    pub(crate) fn context_count(&self) -> usize {
        self.contexts.count()
    }

    /// An enclosing format operation failed after consuming decisions or
    /// writing output. Reusing this coding unit would misinterpret its state.
    pub(crate) fn poison(&mut self) {
        self.poisoned = true;
    }

    /// Verify the caller's symbol count and the exact terminal pair.
    /// This does not prove that the supplied table matches T.88 Table E.1.
    pub async fn finish(self, expected_symbols: u64) -> MqResult<()> {
        self.finish_with_snapshot(expected_symbols)
            .await
            .map(|_| ())
    }

    /// Verify the tail and return a snapshot including any physical bytes
    /// fetched while checking it. The semantic input offset can remain earlier.
    pub async fn finish_with_snapshot(mut self, expected_symbols: u64) -> MqResult<MqSnapshot> {
        self.finish_with_snapshot_mut(expected_symbols).await
    }

    /// Verify the one coding-unit tail while retaining an inspectable decoder.
    /// A failed or dropped pending check poisons this decoder; its snapshot
    /// still reports the bytes and work reached before failure.
    pub(crate) async fn finish_with_snapshot_mut(
        &mut self,
        expected_symbols: u64,
    ) -> MqResult<MqSnapshot> {
        if self.poisoned {
            return Err(self.at(None, MqErrorKind::Poisoned));
        }
        self.check_cancelled(None)?;
        if self.symbols_decoded != expected_symbols {
            return Err(self.at(
                None,
                MqErrorKind::WrongSymbolCount {
                    expected: expected_symbols,
                    decoded: self.symbols_decoded,
                },
            ));
        }
        self.poisoned = true;
        let tail = self.span.length - 2;
        let first = self.read_byte(tail, None).await?;
        let second = self.read_byte(tail + 1, None).await?;
        if first != 0xFF {
            return Err(MqError {
                offset: Some(self.span.offset + tail),
                context: None,
                kind: MqErrorKind::MissingTerminator,
            });
        }
        if second != 0xAC {
            return Err(MqError {
                offset: Some(self.span.offset + tail + 1),
                context: None,
                kind: MqErrorKind::InvalidMarker(second),
            });
        }
        self.check_cancelled(None)?;
        self.poisoned = false;
        Ok(self.snapshot())
    }

    fn at(&self, context: Option<usize>, kind: MqErrorKind) -> MqError {
        MqError {
            offset: Some(self.span.offset + self.bp),
            context,
            kind,
        }
    }

    fn check_cancelled(&self, context: Option<usize>) -> MqResult<()> {
        if self.cancellation.is_cancelled() {
            Err(self.at(context, MqErrorKind::Cancelled))
        } else {
            Ok(())
        }
    }

    fn charge(&mut self, count: u64, context: Option<usize>) -> MqResult<()> {
        let attempted = self.work_done.checked_add(count).ok_or_else(|| {
            self.at(
                context,
                MqErrorKind::LimitExceeded {
                    resource: "MQ work",
                    limit: self.budget.max_work,
                    attempted: u64::MAX,
                },
            )
        })?;
        if attempted > self.budget.max_work {
            return Err(self.at(
                context,
                MqErrorKind::LimitExceeded {
                    resource: "MQ work",
                    limit: self.budget.max_work,
                    attempted,
                },
            ));
        }
        self.work_done = attempted;
        Ok(())
    }

    async fn read_byte(&mut self, relative: u64, context: Option<usize>) -> MqResult<u8> {
        self.check_cancelled(context)?;
        if relative >= self.span.length {
            return Err(MqError {
                offset: Some(self.span.offset + self.span.length),
                context,
                kind: MqErrorKind::MissingTerminator,
            });
        }
        let cached_end = self.buffer_start + self.buffered as u64;
        if self.buffered == 0 || relative < self.buffer_start || relative >= cached_end {
            let remaining = self.span.length - relative;
            let available_work = self.budget.max_work.saturating_sub(self.work_done);
            let count = remaining
                .min(INPUT_BUFFER_BYTES as u64)
                .min(self.limits.io_chunk_bytes as u64)
                .min(available_work);
            if count == 0 {
                return Err(self.at(
                    context,
                    MqErrorKind::LimitExceeded {
                        resource: "MQ work",
                        limit: self.budget.max_work,
                        attempted: self.work_done.saturating_add(1),
                    },
                ));
            }
            self.charge(count, context)?;
            let offset = self.span.offset + relative;
            let mut source = ProgressSource {
                inner: self.source,
                fetched: &mut self.source_bytes_fetched,
            };
            read_exact_at(
                &mut source,
                offset,
                &mut self.buffer[..count as usize],
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
                MqError {
                    offset: Some(failed_at),
                    context,
                    kind: match source {
                        Error::Cancelled => MqErrorKind::Cancelled,
                        other => MqErrorKind::Source(other),
                    },
                }
            })?;
            self.buffer_start = relative;
            self.buffered = count as usize;
        }
        Ok(self.buffer[(relative - self.buffer_start) as usize])
    }

    async fn byte_in(&mut self, context: Option<usize>) -> MqResult<()> {
        self.check_cancelled(context)?;
        self.charge(1, context)?;
        let next = self.bp + 1;
        let next_byte = self.read_byte(next, context).await?;
        if self.current_byte == 0xFF {
            if next_byte > 0x8F {
                if next_byte != 0xAC || self.bp != self.span.length - 2 {
                    return Err(MqError {
                        offset: Some(self.span.offset + next),
                        context,
                        kind: MqErrorKind::InvalidMarker(next_byte),
                    });
                }
                let attempted = self.terminal_inputs.checked_add(1).ok_or_else(|| {
                    self.at(
                        context,
                        MqErrorKind::LimitExceeded {
                            resource: "MQ terminal inputs",
                            limit: self.budget.max_terminal_inputs,
                            attempted: u64::MAX,
                        },
                    )
                })?;
                if attempted > self.budget.max_terminal_inputs {
                    return Err(self.at(
                        context,
                        MqErrorKind::LimitExceeded {
                            resource: "MQ terminal inputs",
                            limit: self.budget.max_terminal_inputs,
                            attempted,
                        },
                    ));
                }
                self.terminal_inputs = attempted;
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

    async fn renormalize(&mut self, context: usize) -> MqResult<()> {
        while self.interval < 0x8000 {
            self.check_cancelled(Some(context))?;
            if self.bit_counter == 0 {
                self.byte_in(Some(context)).await?;
            }
            self.charge(1, Some(context))?;
            self.interval <<= 1;
            self.code <<= 1;
            self.bit_counter -= 1;
        }
        Ok(())
    }

    async fn decode_bit_inner(&mut self, context: usize) -> MqResult<bool> {
        self.charge(1, Some(context))?;
        let current = self.contexts.states[context];
        let state = self.table.state(current.state_index);
        let qe = u32::from(state.qe);
        let narrowed = self
            .interval
            .checked_sub(qe)
            .ok_or_else(|| self.at(Some(context), MqErrorKind::Invariant("interval underflow")))?;
        self.interval = narrowed;
        let (bit, updated) = if self.code >> 16 < qe {
            self.interval = qe;
            if narrowed < qe {
                (
                    current.mps,
                    MqContext {
                        state_index: state.next_mps,
                        mps: current.mps,
                    },
                )
            } else {
                (
                    !current.mps,
                    MqContext {
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
                    MqContext {
                        state_index: state.next_lps,
                        mps: current.mps ^ state.switch_mps,
                    },
                )
            } else {
                (
                    current.mps,
                    MqContext {
                        state_index: state.next_mps,
                        mps: current.mps,
                    },
                )
            }
        };
        self.renormalize(context).await?;
        self.contexts.states[context] = updated;
        Ok(bit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ready;
    use crate::{NeverCancel, native::SeekableSource};
    use std::io::Cursor;

    type TestDecoder<'a> = MqDecoder<'a, SeekableSource<Cursor<Vec<u8>>>, NeverCancel>;

    /// Runs `test` on the result of initializing a decoder over `bytes` with
    /// a flat, single-context table and the given budget.
    fn with_flat_init<R>(
        bytes: &[u8],
        budget: MqBudget,
        test: impl FnOnce(MqResult<TestDecoder<'_>>) -> R,
    ) -> R {
        let limits = Limits::default();
        let states = vec![
            MqState {
                qe: 0x4000,
                next_mps: 0,
                next_lps: 0,
                switch_mps: false,
            };
            MQ_STATE_COUNT
        ];
        let table = MqTable::new(states, &limits).unwrap();
        let mut contexts = MqContexts::new(1, &limits, &budget).unwrap();
        let mut source = SeekableSource::new(Cursor::new(bytes.to_vec())).unwrap();
        test(ready(MqDecoder::new(
            &mut source,
            MqSpan {
                offset: 0,
                length: bytes.len() as u64,
            },
            &table,
            &mut contexts,
            &limits,
            &NeverCancel,
            budget,
        )))
    }

    /// Runs `test` on a decoder over a three-byte span with a flat,
    /// single-context table and the given budget.
    fn with_flat_decoder<R>(budget: MqBudget, test: impl FnOnce(&mut TestDecoder<'_>) -> R) -> R {
        with_flat_init(&[0, 0xff, 0xac], budget, |decoder| {
            test(&mut decoder.unwrap())
        })
    }

    /// The initialization error over `bytes`; the unit tests share the one
    /// source type, so these paths belong to the same instantiation as the
    /// counter-overflow tests below.
    fn flat_init_error(bytes: &[u8], budget: MqBudget) -> MqErrorKind {
        with_flat_init(bytes, budget, |decoder| {
            decoder.err().expect("MQ initialization must fail").kind
        })
    }

    #[test]
    fn initialization_checks_markers_and_budgets_on_its_first_byte() {
        assert!(matches!(
            flat_init_error(&[0xff, 0x90], MqBudget::default()),
            MqErrorKind::InvalidMarker(0x90)
        ));
        let no_terminal = MqBudget {
            max_terminal_inputs: 0,
            ..MqBudget::default()
        };
        assert!(matches!(
            flat_init_error(&[0xff, 0xac], no_terminal),
            MqErrorKind::LimitExceeded {
                resource: "MQ terminal inputs",
                limit: 0,
                attempted: 1,
            }
        ));
        // A stuffed 0xFF followed by a data byte is consumed as seven bits.
        with_flat_init(&[0xff, 0x7f, 0xff, 0xac], MqBudget::default(), |decoder| {
            let decoder = decoder.unwrap();
            assert_eq!((decoder.bp, decoder.current_byte), (1, 0x7f));
        });
    }

    #[test]
    fn decisions_reject_unknown_contexts_excess_work_and_a_poisoned_decoder() {
        with_flat_decoder(MqBudget::default(), |decoder| {
            assert!(matches!(
                ready(decoder.decode_bit(1)).unwrap_err().kind,
                MqErrorKind::InvalidContext
            ));
            let limit = decoder.budget.max_work;
            let error = decoder.charge(limit, Some(0)).unwrap_err();
            assert!(matches!(
                error.kind,
                MqErrorKind::LimitExceeded {
                    resource: "MQ work",
                    attempted,
                    ..
                } if attempted > limit
            ));
            decoder.poisoned = true;
            assert!(matches!(
                ready(decoder.decode_bit(0)).unwrap_err().kind,
                MqErrorKind::Poisoned
            ));
        });
    }

    #[test]
    fn work_and_terminal_counters_reject_u64_max_overflow() {
        let budget = MqBudget {
            max_work: u64::MAX,
            max_terminal_inputs: u64::MAX,
            ..MqBudget::default()
        };
        with_flat_decoder(budget, |decoder| {
            decoder.work_done = u64::MAX;
            assert!(matches!(
                decoder.charge(1, None).unwrap_err().kind,
                MqErrorKind::LimitExceeded {
                    resource: "MQ work",
                    ..
                }
            ));
            decoder.work_done = 0;
            decoder.terminal_inputs = u64::MAX;
            assert!(matches!(
                ready(decoder.byte_in(None)).unwrap_err().kind,
                MqErrorKind::LimitExceeded {
                    resource: "MQ terminal inputs",
                    ..
                }
            ));
        });
    }

    #[test]
    fn symbol_counter_rejects_u64_max_overflow_before_any_work() {
        let budget = MqBudget {
            max_symbols: u64::MAX,
            ..MqBudget::default()
        };
        with_flat_decoder(budget, |decoder| {
            decoder.symbols_decoded = u64::MAX;
            let before = decoder.snapshot();
            let error = ready(decoder.decode_bit(0)).unwrap_err();
            assert_eq!(error.context, Some(0));
            assert!(matches!(
                error.kind,
                MqErrorKind::LimitExceeded {
                    resource: "MQ symbols",
                    limit: u64::MAX,
                    attempted: u64::MAX,
                }
            ));
            // The preflight failure leaves registers, work, and poison untouched.
            assert_eq!(decoder.snapshot(), before);
        });
    }
}
