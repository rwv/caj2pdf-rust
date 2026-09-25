// SPDX-License-Identifier: MIT

//! Bounded, row-streamed T.88 template-1 generic refinement bitmaps.
//!
//! This is one bitmap operation inside an existing MQ coding unit. The caller
//! owns the probability table, context layout, reference store, and output
//! store. No official MQ state rows or document pixels are bundled here.

use super::{
    dictionary::SymbolDescriptor,
    iaid::IaidLayout,
    mq::{MQ_STATE_COUNT, MqContext, MqDecoder, MqError, MqSnapshot, MqState},
};
use crate::{Cancellation, Error, Limits, RangedSource, SequentialSink};
use std::{error, fmt, io, mem};

const CONTEXT_COUNT: usize = 1024;
const MQ_BUFFER_BYTES: u64 = 256;

/// A checked bound for one refinement session. All totals include every
/// successfully decoded bitmap in the session; MQ itself has additional caps.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RefinementBudget {
    pub max_width: u32,
    pub max_height: u32,
    pub max_reference_width: u32,
    pub max_reference_height: u32,
    pub max_reference_pixels_per_bitmap: u64,
    pub max_reference_bytes_per_bitmap: u64,
    pub max_pixels_per_bitmap: u64,
    pub max_total_pixels: u64,
    pub max_bytes_per_bitmap: u64,
    pub max_total_output_bytes: u64,
    pub max_reference_reads: u64,
    pub max_reference_bytes_fetched: u64,
    pub max_sink_writes: u64,
    /// Maximum explicit store flush attempts between bitmap decodes.
    pub max_flushes: u64,
    pub max_source_request_bytes: usize,
    pub max_sink_request_bytes: usize,
    /// GR pixel decisions only; the borrowed MQ's own budget covers all
    /// interleaved dictionary decisions in the coding unit.
    pub max_mq_decisions: u64,
    /// Ten neighbor probes per explicitly decoded pixel.
    pub max_context_work: u64,
    /// Full caller MQ bank, table, MQ input buffer, two target rows and three
    /// reference rows; the backing reference/output stores are separate.
    pub max_working_bytes: u64,
}

impl Default for RefinementBudget {
    fn default() -> Self {
        Self {
            max_width: 32_768,
            max_height: 32_768,
            max_reference_width: 32_768,
            max_reference_height: 32_768,
            max_reference_pixels_per_bitmap: 24_000_000,
            max_reference_bytes_per_bitmap: 64 * 1024 * 1024,
            max_pixels_per_bitmap: 12_000_000,
            max_total_pixels: 24_000_000,
            max_bytes_per_bitmap: 64 * 1024 * 1024,
            max_total_output_bytes: 128 * 1024 * 1024,
            max_reference_reads: 1_000_000,
            max_reference_bytes_fetched: 128 * 1024 * 1024,
            max_sink_writes: 2_000_000,
            max_flushes: 4096,
            max_source_request_bytes: 256 * 1024,
            max_sink_request_bytes: 256 * 1024,
            max_mq_decisions: 24_000_000,
            max_context_work: 240_000_000,
            max_working_bytes: 16 * 1024 * 1024,
        }
    }
}

/// The bitmap in a caller-owned append-only store. `store_base` is the
/// absolute ranged-source coordinate of this dictionary's first append;
/// `symbol.relative_store_offset` is relative to that base. The adapter must
/// reopen the *same* store after preceding writes become readable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RefinementReference {
    pub store_base: u64,
    pub symbol: SymbolDescriptor,
}

/// One generic-refinement bitmap request. T.88 Table 6 permits zero geometry,
/// but this bounded first slice reports it as `Unsupported`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RefinementRequest {
    pub width: u32,
    pub height: u32,
    pub template: u8,
    pub typical_prediction: bool,
    pub reference_dx: i32,
    pub reference_dy: i32,
    pub reference: RefinementReference,
}

/// The physical counters include completed short reads and partial writes;
/// `reference_reads`, `sink_writes`, and `flushes` count attempted calls.
/// `poisoned` is set before the first await and stays set after any error or
/// dropped pending bitmap future.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RefinementProgress {
    pub completed_bitmaps: u32,
    pub rows_written: u64,
    pub pixels_decoded: u64,
    pub output_bytes_written: u64,
    pub reference_reads: u64,
    pub reference_bytes_fetched: u64,
    pub sink_writes: u64,
    pub flushes: u64,
    pub context_work: u64,
    pub mq: Option<MqSnapshot>,
    pub poisoned: bool,
}

/// A completed bitmap in the output store. Offset zero is the first byte
/// appended by this refinement session, even if the sink was prefilled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RefinementReport {
    pub target: SymbolDescriptor,
    pub progress: RefinementProgress,
}

#[derive(Debug)]
pub struct RefinementError {
    /// Reference-store or MQ source offset, when applicable.
    pub offset: Option<u64>,
    pub bitmap_index: u32,
    pub row: u32,
    pub x: u32,
    pub progress: Box<RefinementProgress>,
    pub kind: RefinementErrorKind,
}

#[derive(Debug)]
pub enum RefinementErrorKind {
    Malformed(&'static str),
    InvalidSpan(&'static str),
    TruncatedReference,
    Unsupported {
        feature: &'static str,
        value: u64,
    },
    LimitExceeded {
        resource: &'static str,
        limit: u64,
        attempted: u64,
    },
    AllocationFailed,
    Cancelled,
    ReferenceSource(Error),
    Sink(Error),
    Mq(Box<MqError>),
    Poisoned,
}

pub type RefinementResult<T> = Result<T, RefinementError>;

impl fmt::Display for RefinementError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "JBIG2 refinement bitmap {} row {} x {}",
            self.bitmap_index, self.row, self.x
        )?;
        if let Some(offset) = self.offset {
            write!(f, " at source byte {offset}")?;
        }
        f.write_str(": ")?;
        match &self.kind {
            RefinementErrorKind::Malformed(reason) => write!(f, "malformed {reason}"),
            RefinementErrorKind::InvalidSpan(reason) => write!(f, "invalid span: {reason}"),
            RefinementErrorKind::TruncatedReference => f.write_str("truncated reference bitmap"),
            RefinementErrorKind::Unsupported { feature, value } => {
                write!(f, "unsupported {feature} ({value})")
            }
            RefinementErrorKind::LimitExceeded {
                resource,
                limit,
                attempted,
            } => {
                write!(f, "{resource} limit {limit} exceeded by {attempted}")
            }
            RefinementErrorKind::AllocationFailed => f.write_str("row allocation failed"),
            RefinementErrorKind::Cancelled => f.write_str("cancelled"),
            RefinementErrorKind::ReferenceSource(source) => write!(f, "reference source: {source}"),
            RefinementErrorKind::Sink(source) => write!(f, "sink: {source}"),
            RefinementErrorKind::Mq(source) => write!(f, "MQ: {source}"),
            RefinementErrorKind::Poisoned => f.write_str("refinement host is poisoned"),
        }
    }
}

impl error::Error for RefinementError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.kind {
            RefinementErrorKind::ReferenceSource(source) | RefinementErrorKind::Sink(source) => {
                Some(source)
            }
            RefinementErrorKind::Mq(source) => Some(source),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
struct Geometry {
    reference_offset: u64,
    reference_width: u32,
    reference_height: u32,
    reference_stride: usize,
    target_stride: usize,
    pixels: u64,
    bytes: u64,
}

/// Reusable template-1 refinement host borrowing *one* existing MQ coding
/// unit, *one* append-only output sink, and persistent disjoint 1,024 GR
/// contexts. Construct it with the typed layout returned by
/// `IaidContextBanks::with_bitmap_contexts`. The bound sink cannot change
/// between bitmaps, so relative descriptor offsets remain in one store.
/// No MQ initialization, finish, context reset, or store flush occurs here.
pub struct RefinementDecoder<'a, 'mq, M: RangedSource, C: Cancellation, W: SequentialSink> {
    mq: &'a mut MqDecoder<'mq, M, C>,
    sink: &'a mut W,
    context_base: usize,
    limits: &'a Limits,
    cancellation: &'a C,
    budget: RefinementBudget,
    progress: RefinementProgress,
}

impl<M: RangedSource, C: Cancellation, W: SequentialSink> Drop
    for RefinementDecoder<'_, '_, M, C, W>
{
    fn drop(&mut self) {
        if self.progress.poisoned {
            self.mq.poison();
        }
    }
}

impl<'a, 'mq, M: RangedSource, C: Cancellation, W: SequentialSink>
    RefinementDecoder<'a, 'mq, M, C, W>
{
    /// Check the context range and fixed working-memory configuration before
    /// any source or sink call. The caller retains all other model contexts.
    pub fn new(
        mq: &'a mut MqDecoder<'mq, M, C>,
        layout: IaidLayout,
        sink: &'a mut W,
        limits: &'a Limits,
        cancellation: &'a C,
        budget: RefinementBudget,
    ) -> RefinementResult<Self> {
        let invalid = |kind| RefinementError {
            offset: None,
            bitmap_index: 0,
            row: 0,
            x: 0,
            progress: Box::new(RefinementProgress::default()),
            kind,
        };
        limits
            .validate()
            .map_err(|_| invalid(RefinementErrorKind::Malformed("Limits")))?;
        if budget.max_source_request_bytes == 0 || budget.max_sink_request_bytes == 0 {
            return Err(invalid(RefinementErrorKind::Malformed(
                "zero I/O request cap",
            )));
        }
        let context_base = layout.bitmap_base();
        if mq.iaid_code_len() != Some(layout.code_len())
            || layout.total_contexts() != mq.context_count()
            || layout
                .total_contexts()
                .checked_sub(context_base)
                .is_none_or(|available| available < CONTEXT_COUNT)
        {
            return Err(invalid(RefinementErrorKind::InvalidSpan(
                "GR context range does not match IAID owner",
            )));
        }
        if mq.snapshot().poisoned {
            return Err(invalid(RefinementErrorKind::Poisoned));
        }
        Ok(Self {
            mq,
            sink,
            context_base,
            limits,
            cancellation,
            budget,
            progress: RefinementProgress::default(),
        })
    }

    pub fn progress(&self) -> RefinementProgress {
        let mut progress = self.progress;
        progress.mq = Some(self.mq.snapshot());
        progress
    }

    /// Borrow the same coding unit for interleaved dictionary integer or
    /// IAID decisions. No GR statistics, bitmap offset, or budget is reset.
    /// The enclosing dictionary must abandon this session if one of its own
    /// semantic decisions fails, even when MQ itself remains usable.
    pub fn mq_mut(&mut self) -> RefinementResult<&mut MqDecoder<'mq, M, C>> {
        if self.progress.poisoned || self.mq.snapshot().poisoned {
            self.progress.poisoned = true;
            return Err(self.error(RefinementErrorKind::Poisoned, None, 0, 0));
        }
        self.check_cancelled(0, 0)?;
        Ok(self.mq)
    }

    /// Flush the bound output store before an adapter reopens newly appended
    /// symbols as references. This does not finish MQ or reset GR statistics.
    /// An error or dropped pending flush poisons the session and coding unit;
    /// a successful flush leaves both usable for more interleaved decisions.
    pub async fn flush_store(&mut self) -> RefinementResult<()> {
        if self.progress.poisoned || self.mq.snapshot().poisoned {
            self.progress.poisoned = true;
            return Err(self.error(RefinementErrorKind::Poisoned, None, 0, 0));
        }
        self.progress.poisoned = true;
        let result = self.flush_store_inner().await;
        match result {
            Ok(()) => {
                self.progress.poisoned = false;
                Ok(())
            }
            Err(error) => {
                self.mq.poison();
                Err(error)
            }
        }
    }

    async fn flush_store_inner(&mut self) -> RefinementResult<()> {
        self.check_cancelled(0, 0)?;
        let attempted = self
            .progress
            .flushes
            .checked_add(1)
            .ok_or_else(|| self.invalid_span("store flush count overflows u64", 0))?;
        if attempted > self.budget.max_flushes {
            return Err(self.limit("store flushes", self.budget.max_flushes, attempted, 0, 0));
        }
        self.progress.flushes = attempted;
        self.sink
            .flush()
            .await
            .map_err(|error| self.error(RefinementErrorKind::Sink(error), None, 0, 0))?;
        self.check_cancelled(0, 0)?;
        Ok(())
    }

    fn error(
        &self,
        kind: RefinementErrorKind,
        offset: Option<u64>,
        row: u32,
        x: u32,
    ) -> RefinementError {
        RefinementError {
            offset,
            bitmap_index: self.progress.completed_bitmaps,
            row,
            x,
            progress: Box::new(self.progress()),
            kind,
        }
    }

    fn invalid_span(&self, reason: &'static str, row: u32) -> RefinementError {
        self.error(RefinementErrorKind::InvalidSpan(reason), None, row, 0)
    }

    fn limit(
        &self,
        resource: &'static str,
        limit: u64,
        attempted: u64,
        row: u32,
        x: u32,
    ) -> RefinementError {
        self.error(
            RefinementErrorKind::LimitExceeded {
                resource,
                limit,
                attempted,
            },
            None,
            row,
            x,
        )
    }

    fn check_cancelled(&self, row: u32, x: u32) -> RefinementResult<()> {
        if self.cancellation.is_cancelled() {
            Err(self.error(RefinementErrorKind::Cancelled, None, row, x))
        } else {
            Ok(())
        }
    }

    fn cap(&self, resource: &'static str, maximum: u64, attempted: u64) -> RefinementResult<()> {
        if attempted > maximum {
            Err(self.limit(resource, maximum, attempted, 0, 0))
        } else {
            Ok(())
        }
    }

    fn geometry<R: RangedSource>(
        &self,
        reference_source: &R,
        request: RefinementRequest,
    ) -> RefinementResult<Geometry> {
        if request.template != 1 {
            return Err(self.error(
                RefinementErrorKind::Unsupported {
                    feature: "refinement template",
                    value: u64::from(request.template),
                },
                None,
                0,
                0,
            ));
        }
        if request.typical_prediction {
            return Err(self.error(
                RefinementErrorKind::Unsupported {
                    feature: "TPGRON",
                    value: 1,
                },
                None,
                0,
                0,
            ));
        }
        let symbol = request.reference.symbol;
        if request.width == 0 || request.height == 0 || symbol.width == 0 || symbol.height == 0 {
            return Err(self.error(
                RefinementErrorKind::Unsupported {
                    feature: "zero bitmap dimension",
                    value: 0,
                },
                None,
                0,
                0,
            ));
        }
        self.cap(
            "target width",
            u64::from(self.budget.max_width),
            u64::from(request.width),
        )?;
        self.cap(
            "target height",
            u64::from(self.budget.max_height),
            u64::from(request.height),
        )?;
        self.cap(
            "reference width",
            u64::from(self.budget.max_reference_width),
            u64::from(symbol.width),
        )?;
        self.cap(
            "reference height",
            u64::from(self.budget.max_reference_height),
            u64::from(symbol.height),
        )?;
        let target_stride = u64::from(request.width).div_ceil(8);
        let reference_stride = u64::from(symbol.width).div_ceil(8);
        if u64::from(symbol.row_stride) != reference_stride {
            return Err(self.error(
                RefinementErrorKind::Malformed("noncanonical reference row stride"),
                None,
                0,
                0,
            ));
        }
        // A u32 width has a stride of at most 2^29 bytes. Multiplication by
        // a u32 height therefore fits u64, even before the configured caps.
        let reference_bytes = reference_stride * u64::from(symbol.height);
        if symbol.stored_bytes != reference_bytes {
            return Err(self.error(
                RefinementErrorKind::Malformed("reference stored byte length"),
                None,
                0,
                0,
            ));
        }
        // u32::MAX squared remains below u64::MAX.
        let reference_pixels = u64::from(symbol.width) * u64::from(symbol.height);
        self.cap(
            "reference pixels per bitmap",
            self.budget.max_reference_pixels_per_bitmap,
            reference_pixels,
        )?;
        self.cap(
            "reference bytes per bitmap",
            self.budget.max_reference_bytes_per_bitmap,
            reference_bytes,
        )?;
        let reference_offset = request
            .reference
            .store_base
            .checked_add(symbol.relative_store_offset)
            .ok_or_else(|| self.invalid_span("reference offset overflows u64", 0))?;
        let reference_end = reference_offset
            .checked_add(reference_bytes)
            .ok_or_else(|| {
                self.error(
                    RefinementErrorKind::InvalidSpan("reference end overflows u64"),
                    Some(reference_offset),
                    0,
                    0,
                )
            })?;
        if reference_end > reference_source.size() {
            return Err(self.error(
                RefinementErrorKind::InvalidSpan("reference outside ranged source"),
                Some(reference_offset),
                0,
                0,
            ));
        }
        self.cap(
            "reference stored bytes",
            self.limits.max_input_bytes,
            reference_bytes,
        )?;
        let pixels = u64::from(request.width) * u64::from(request.height);
        let bytes = target_stride * u64::from(request.height);
        self.cap(
            "pixels per bitmap",
            self.budget.max_pixels_per_bitmap,
            pixels,
        )?;
        self.cap("bytes per bitmap", self.budget.max_bytes_per_bitmap, bytes)?;
        let total_pixels = self
            .progress
            .pixels_decoded
            .checked_add(pixels)
            .ok_or_else(|| self.invalid_span("total pixels overflows u64", 0))?;
        let total_bytes = self
            .progress
            .output_bytes_written
            .checked_add(bytes)
            .ok_or_else(|| self.invalid_span("total output bytes overflows u64", 0))?;
        self.cap("total pixels", self.budget.max_total_pixels, total_pixels)?;
        self.cap(
            "total output bytes",
            self.budget
                .max_total_output_bytes
                .min(self.limits.max_output_bytes),
            total_bytes,
        )?;
        self.cap("MQ decisions", self.budget.max_mq_decisions, total_pixels)?;
        let work = total_pixels
            .checked_mul(10)
            .ok_or_else(|| self.invalid_span("context work overflows u64", 0))?;
        self.cap("context work", self.budget.max_context_work, work)?;
        let target_stride = usize::try_from(target_stride)
            .map_err(|_| self.invalid_span("target stride exceeds address space", 0))?;
        let reference_stride = usize::try_from(reference_stride)
            .map_err(|_| self.invalid_span("reference stride exceeds address space", 0))?;
        self.limits
            .check_allocation(target_stride as u64)
            .map_err(|_| {
                self.limit(
                    "target row allocation",
                    self.limits.max_allocation_bytes,
                    target_stride as u64,
                    0,
                    0,
                )
            })?;
        self.limits
            .check_allocation(reference_stride as u64)
            .map_err(|_| {
                self.limit(
                    "reference row allocation",
                    self.limits.max_allocation_bytes,
                    reference_stride as u64,
                    0,
                    0,
                )
            })?;
        let row_bytes = 2 * target_stride as u64 + 3 * reference_stride as u64;
        // The existing MQ bank's constructor already checked this allocation
        // with the same fixed table/buffer terms. Include the entire bank,
        // not only the GR slice, in the combined cap.
        let mq_bytes = self.mq.context_count() as u64 * mem::size_of::<MqContext>() as u64
            + (MQ_STATE_COUNT * mem::size_of::<MqState>()) as u64
            + MQ_BUFFER_BYTES;
        let working = row_bytes
            .checked_add(mq_bytes)
            .ok_or_else(|| self.invalid_span("working memory overflows u64", 0))?;
        self.cap("working bytes", self.budget.max_working_bytes, working)?;
        Ok(Geometry {
            reference_offset,
            reference_width: symbol.width,
            reference_height: symbol.height,
            reference_stride,
            target_stride,
            pixels,
            bytes,
        })
    }

    fn row(&self, length: usize) -> RefinementResult<Vec<u8>> {
        let mut row = Vec::new();
        row.try_reserve_exact(length)
            .map_err(|_| self.error(RefinementErrorKind::AllocationFailed, None, 0, 0))?;
        row.resize(length, 0);
        Ok(row)
    }

    async fn read_reference_row<R: RangedSource>(
        &mut self,
        source: &mut R,
        geometry: Geometry,
        row_id: i64,
        row: &mut [u8],
        target_y: u32,
    ) -> RefinementResult<()> {
        // `geometry` checked that `reference_offset` plus the stored
        // reference bytes fits `u64`. This row and every byte offset within
        // it lie below that end, because `row_id` is below the reference
        // height and the row is one reference stride long.
        let offset = geometry.reference_offset + row_id as u64 * geometry.reference_stride as u64;
        let mut done = 0usize;
        while done < row.len() {
            self.check_cancelled(target_y, 0)?;
            let count = (row.len() - done)
                .min(self.limits.io_chunk_bytes)
                .min(self.budget.max_source_request_bytes);
            let attempted = self
                .progress
                .reference_reads
                .checked_add(1)
                .ok_or_else(|| self.invalid_span("reference read count overflows u64", target_y))?;
            if attempted > self.budget.max_reference_reads {
                return Err(self.limit(
                    "reference reads",
                    self.budget.max_reference_reads,
                    attempted,
                    target_y,
                    0,
                ));
            }
            let requested_bytes = self
                .progress
                .reference_bytes_fetched
                .checked_add(count as u64)
                .ok_or_else(|| self.invalid_span("reference byte count overflows u64", target_y))?;
            if requested_bytes > self.budget.max_reference_bytes_fetched {
                return Err(self.limit(
                    "reference bytes",
                    self.budget.max_reference_bytes_fetched,
                    requested_bytes,
                    target_y,
                    0,
                ));
            }
            let current = offset + done as u64;
            self.progress.reference_reads = attempted;
            let read = source
                .read_at(current, &mut row[done..done + count])
                .await
                .map_err(|error| {
                    self.error(
                        RefinementErrorKind::ReferenceSource(error),
                        Some(current),
                        target_y,
                        0,
                    )
                })?;
            if read > count {
                return Err(self.error(
                    RefinementErrorKind::ReferenceSource(Error::InvalidInput {
                        reason: "source overreported reference read",
                    }),
                    Some(current),
                    target_y,
                    0,
                ));
            }
            self.progress.reference_bytes_fetched += read as u64;
            self.check_cancelled(target_y, 0)?;
            if read == 0 {
                return Err(self.error(
                    RefinementErrorKind::TruncatedReference,
                    Some(current),
                    target_y,
                    0,
                ));
            }
            done += read;
        }
        Ok(())
    }

    async fn write_row(&mut self, row: &[u8], y: u32) -> RefinementResult<()> {
        let mut done = 0usize;
        while done < row.len() {
            self.check_cancelled(y, 0)?;
            let count = (row.len() - done)
                .min(self.limits.io_chunk_bytes)
                .min(self.budget.max_sink_request_bytes);
            let attempted = self
                .progress
                .sink_writes
                .checked_add(1)
                .ok_or_else(|| self.invalid_span("sink write count overflows u64", y))?;
            if attempted > self.budget.max_sink_writes {
                return Err(self.limit(
                    "sink writes",
                    self.budget.max_sink_writes,
                    attempted,
                    y,
                    0,
                ));
            }
            self.progress.sink_writes = attempted;
            let written = self
                .sink
                .write(&row[done..done + count])
                .await
                .map_err(|error| self.error(RefinementErrorKind::Sink(error), None, y, 0))?;
            if written > count {
                return Err(self.error(
                    RefinementErrorKind::Sink(Error::InvalidInput {
                        reason: "sink overreported refinement write",
                    }),
                    None,
                    y,
                    0,
                ));
            }
            if written == 0 {
                return Err(self.error(
                    RefinementErrorKind::Sink(Error::Io(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "sink made no progress",
                    ))),
                    None,
                    y,
                    0,
                ));
            }
            self.progress.output_bytes_written += written as u64;
            self.check_cancelled(y, 0)?;
            done += written;
        }
        self.progress.rows_written += 1;
        Ok(())
    }

    /// Append one packed bitmap; retain the GR contexts and MQ state for the
    /// next bitmap while restarting target-row history at zero. On failure,
    /// discard the partial store output and enclosing coding unit. Dropping a
    /// pending future leaves this host poisoned; dropping the host poisons MQ.
    pub async fn decode_bitmap<R: RangedSource>(
        &mut self,
        reference_source: &mut R,
        request: RefinementRequest,
    ) -> RefinementResult<RefinementReport> {
        if self.progress.poisoned || self.mq.snapshot().poisoned {
            self.progress.poisoned = true;
            return Err(self.error(RefinementErrorKind::Poisoned, None, 0, 0));
        }
        self.progress.poisoned = true;
        let result = self.decode_bitmap_inner(reference_source, request).await;
        match result {
            Ok(target) => {
                self.progress.poisoned = false;
                Ok(RefinementReport {
                    target,
                    progress: self.progress(),
                })
            }
            Err(error) => {
                self.mq.poison();
                Err(error)
            }
        }
    }

    async fn decode_bitmap_inner<R: RangedSource>(
        &mut self,
        reference_source: &mut R,
        request: RefinementRequest,
    ) -> RefinementResult<SymbolDescriptor> {
        let geometry = self.geometry(reference_source, request)?;
        let start_offset = self.progress.output_bytes_written;
        self.check_cancelled(0, 0)?;
        let mut previous = self.row(geometry.target_stride)?;
        let mut current = self.row(geometry.target_stride)?;
        let mut reference_rows = [
            self.row(geometry.reference_stride)?,
            self.row(geometry.reference_stride)?,
            self.row(geometry.reference_stride)?,
        ];
        // Row `r` is cached only in slot `r mod 3`, so the three consecutive
        // rows a target row needs never evict one another.
        let mut cached: [Option<i64>; 3] = [None; 3];
        for y in 0..request.height {
            self.check_cancelled(y, 0)?;
            current.fill(0);
            let reference_y = i64::from(y) - i64::from(request.reference_dy);
            let needed = [reference_y - 1, reference_y, reference_y + 1];
            for row_id in needed {
                let slot = cache_slot(row_id);
                if row_id < 0
                    || row_id >= i64::from(geometry.reference_height)
                    || cached[slot] == Some(row_id)
                {
                    continue;
                }
                self.read_reference_row(
                    reference_source,
                    geometry,
                    row_id,
                    &mut reference_rows[slot],
                    y,
                )
                .await?;
                cached[slot] = Some(row_id);
            }
            for x in 0..request.width {
                self.check_cancelled(y, x)?;
                let reference_x = i64::from(x) - i64::from(request.reference_dx);
                let context = template1_context(
                    &previous,
                    &current,
                    request.width,
                    &reference_rows,
                    &cached,
                    geometry.reference_width,
                    reference_x,
                    reference_y,
                    x,
                );
                let bit = self
                    .mq
                    .decode_bit(self.context_base + context)
                    .await
                    .map_err(|error| {
                        let offset = error.offset;
                        self.error(RefinementErrorKind::Mq(Box::new(error)), offset, y, x)
                    })?;
                if bit {
                    current[(x / 8) as usize] |= 0x80 >> (x % 8);
                }
                self.progress.pixels_decoded += 1;
                self.progress.context_work += 10;
            }
            self.write_row(&current, y).await?;
            mem::swap(&mut previous, &mut current);
        }
        let target = SymbolDescriptor {
            width: request.width,
            height: request.height,
            row_stride: geometry.target_stride as u32,
            relative_store_offset: start_offset,
            stored_bytes: geometry.bytes,
        };
        self.progress.completed_bitmaps += 1;
        debug_assert_eq!(
            self.progress.output_bytes_written - start_offset,
            geometry.bytes
        );
        debug_assert_eq!(
            geometry.pixels,
            u64::from(request.width) * u64::from(request.height)
        );
        Ok(target)
    }
}

fn packed_pixel(row: &[u8], x: i64, width: u32) -> usize {
    if x < 0 || x >= i64::from(width) {
        0
    } else {
        let x = x as usize;
        usize::from(row[x / 8] & (0x80 >> (x % 8)) != 0)
    }
}

fn reference_pixel(
    rows: &[Vec<u8>; 3],
    cached: &[Option<i64>; 3],
    width: u32,
    x: i64,
    y: i64,
) -> usize {
    let slot = cache_slot(y);
    if cached[slot] == Some(y) {
        packed_pixel(&rows[slot], x, width)
    } else {
        0
    }
}

/// The only reference-cache slot that may hold row `y`.
fn cache_slot(y: i64) -> usize {
    y.rem_euclid(3) as usize
}

/// Figure 13, in reading order: target above left/center/right, target left;
/// reference above center, reference center left/center/right, reference below
/// center/right. This maps to context bits 9..0. T.88 permits any stable bit
/// assignment; the centered reference pixel is bit 3 in this mapping.
#[allow(clippy::too_many_arguments)]
fn template1_context(
    target_above: &[u8],
    target_current: &[u8],
    target_width: u32,
    reference_rows: &[Vec<u8>; 3],
    cached: &[Option<i64>; 3],
    reference_width: u32,
    reference_x: i64,
    reference_y: i64,
    target_x: u32,
) -> usize {
    let x = i64::from(target_x);
    let pixels = [
        packed_pixel(target_above, x - 1, target_width),
        packed_pixel(target_above, x, target_width),
        packed_pixel(target_above, x + 1, target_width),
        packed_pixel(target_current, x - 1, target_width),
        reference_pixel(
            reference_rows,
            cached,
            reference_width,
            reference_x,
            reference_y - 1,
        ),
        reference_pixel(
            reference_rows,
            cached,
            reference_width,
            reference_x - 1,
            reference_y,
        ),
        reference_pixel(
            reference_rows,
            cached,
            reference_width,
            reference_x,
            reference_y,
        ),
        reference_pixel(
            reference_rows,
            cached,
            reference_width,
            reference_x + 1,
            reference_y,
        ),
        reference_pixel(
            reference_rows,
            cached,
            reference_width,
            reference_x,
            reference_y + 1,
        ),
        reference_pixel(
            reference_rows,
            cached,
            reference_width,
            reference_x + 1,
            reference_y + 1,
        ),
    ];
    pixels
        .into_iter()
        .fold(0, |context, pixel| (context << 1) | pixel)
}

#[cfg(test)]
mod tests {
    use super::template1_context;

    #[test]
    fn figure_13_each_of_ten_pixels_has_one_distinct_context_bit() {
        let mappings = [
            // Target previous row: x-1, x, x+1; target current: x-1.
            (0, 0, 0x80),
            (0, 0, 0x40),
            (0, 0, 0x20),
            (1, 0, 0x80),
            // Reference: center above; left, center, right at center row;
            // center and right below.
            (2, 0, 0x40),
            (2, 1, 0x80),
            (2, 1, 0x40),
            (2, 1, 0x20),
            (2, 2, 0x40),
            (2, 2, 0x20),
        ];
        for (bit, &(plane, row, mask)) in mappings.iter().enumerate() {
            let mut above = [0u8];
            let mut current = [0u8];
            let mut reference = [vec![0], vec![0], vec![0]];
            let target = match plane {
                0 => &mut above[0],
                1 => &mut current[0],
                _ => &mut reference[row][0],
            };
            *target = mask;
            let actual = template1_context(
                &above,
                &current,
                3,
                &reference,
                &[Some(0), Some(1), Some(2)],
                3,
                1,
                1,
                1,
            );
            assert_eq!(actual, 1 << (9 - bit), "tap {bit}");
        }
    }

    #[test]
    fn figure_13_zero_extends_all_edges() {
        let above = [0xff];
        let current = [0xff];
        let reference = [vec![0xff], vec![0xff], vec![0xff]];
        let cached = [Some(0), Some(1), Some(2)];
        // At the target left edge, the target left neighbors are absent.
        let left = template1_context(&above, &current, 1, &reference, &cached, 1, 0, 1, 0);
        assert_eq!(left & ((1 << 9) | (1 << 6)), 0);
        // The target above-right tap is also outside this one-pixel row.
        assert_eq!(left & (1 << 7), 0);
        // Moving the aligned reference center outside either side yields
        // zero for all six reference bits, regardless of source row padding.
        for reference_x in [-2, 2] {
            let context = template1_context(
                &above,
                &current,
                1,
                &reference,
                &cached,
                1,
                reference_x,
                1,
                0,
            );
            assert_eq!(context & 0x3f, 0);
        }
        let top = template1_context(&above, &current, 1, &reference, &cached, 1, 0, -1, 0);
        assert_eq!(top & 0x3f, 1 << 1); // Only reference below-center is in row zero.
        let bottom = template1_context(&above, &current, 1, &reference, &cached, 1, 0, 3, 0);
        assert_eq!(bottom & 0x3f, 1 << 5); // Only reference above-center is in row two.
    }
}
