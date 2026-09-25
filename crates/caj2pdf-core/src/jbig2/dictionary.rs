// SPDX-License-Identifier: MIT

//! Bounded arithmetic direct-coded T.88 symbol dictionaries (segment type 0).
//!
//! The caller supplies the probability table and an append-only bitmap store.
//! This first slice accepts template 2, AT `(2, -1)`, no imported symbols,
//! and no bitmap-context carry. Refinement/aggregate dictionaries are parsed
//! but explicitly refused before arithmetic or store output.

use super::{
    HeaderError, HeaderLimits, SegmentHeader, SegmentSpan,
    generic::template2_context,
    integer::{
        INTEGER_CONTEXT_COUNT, IntegerContextBanks, IntegerProcedure, IntegerValue, decode_integer,
    },
    mq::{
        MQ_STATE_COUNT, MqBudget, MqContext, MqDecoder, MqError, MqSnapshot, MqSpan, MqState,
        MqTable,
    },
    read_segment_header,
};
use crate::fallible::{len_u64, reserve_exact, try_convert, usize_from_u32};
use crate::{Cancellation, Error, Limits, RangedSource, SequentialSink};
use std::{error, fmt, io, mem};

const BITMAP_CONTEXTS: usize = 1024;
const TOTAL_CONTEXTS: usize = INTEGER_CONTEXT_COUNT + BITMAP_CONTEXTS;
const MQ_BUFFER_BYTES: u64 = 256;
/// This direct-coded first-dictionary slice accepts no imported symbols.
pub const MAX_IMPORTED_SYMBOLS: u32 = 0;

/// Resource bounds for one symbol dictionary, in addition to `Limits` and `MqBudget`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DictionaryBudget {
    /// Segment-data dictionary header only; the framing header retains the
    /// separate 64 KiB `HeaderLimits` cap during validation.
    pub max_data_header_bytes: u64,
    pub max_body_bytes: u64,
    pub max_new_symbols: u32,
    pub max_exported_symbols: u32,
    pub max_height_classes: u32,
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels_per_symbol: u64,
    pub max_bytes_per_symbol: u64,
    pub max_total_pixels: u64,
    pub max_stored_bitmap_bytes: u64,
    pub max_catalog_bytes: u64,
    pub max_export_runs: u32,
    pub max_sink_writes: u64,
    pub max_source_request_bytes: usize,
    pub max_sink_request_bytes: usize,
    /// Combined contexts, table, MQ buffer, descriptor capacity, and three rows.
    pub max_working_bytes: u64,
}

impl Default for DictionaryBudget {
    fn default() -> Self {
        Self {
            max_data_header_bytes: 64,
            max_body_bytes: 64 * 1024 * 1024,
            max_new_symbols: 4096,
            max_exported_symbols: 4096,
            max_height_classes: 8192,
            max_width: 32_768,
            max_height: 32_768,
            max_pixels_per_symbol: 12_000_000,
            max_bytes_per_symbol: 64 * 1024 * 1024,
            max_total_pixels: 24_000_000,
            max_stored_bitmap_bytes: 128 * 1024 * 1024,
            max_catalog_bytes: 1024 * 1024,
            max_export_runs: 8192,
            max_sink_writes: 2_000_000,
            max_source_request_bytes: 256,
            max_sink_request_bytes: 64 * 1024,
            max_working_bytes: 16 * 1024 * 1024,
        }
    }
}

/// Coding mode identified from the segment-data flags.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DictionaryMode {
    ArithmeticDirect,
    ArithmeticRefinementAggregate,
    HuffmanDirect,
    HuffmanRefinementAggregate,
}

/// Parsed segment-data header. `body` is an exact absolute source range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DictionaryDataHeader {
    pub flags: u16,
    pub mode: DictionaryMode,
    pub template: u8,
    pub refinement_template: u8,
    pub bitmap_context_used: bool,
    pub bitmap_context_retained: bool,
    /// Only the first `at_count` positions are present in the encoded header.
    pub at: [(i8, i8); 4],
    pub at_count: u8,
    /// Only the first `refinement_at_count` positions are present.
    pub refinement_at: [(i8, i8); 2],
    pub refinement_at_count: u8,
    pub exported_symbols: u32,
    pub new_symbols: u32,
    pub header_bytes: u64,
    pub body: SegmentSpan,
}

/// One packed bitmap in a caller-owned append-only store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SymbolDescriptor {
    pub width: u32,
    pub height: u32,
    pub row_stride: u32,
    /// Byte offset relative to the first byte appended by this decoder.
    pub relative_store_offset: u64,
    pub stored_bytes: u64,
}

/// Complete new-symbol catalog and exported view in standard order.
#[derive(Debug, Eq, PartialEq)]
pub struct DictionaryCatalog {
    pub new_symbols: Vec<SymbolDescriptor>,
    pub exported_symbols: Vec<SymbolDescriptor>,
}

/// Observable progress; a failed operation leaves the caller's store partial.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DictionaryProgress {
    pub completed_symbols: u32,
    pub stored_bitmap_bytes: u64,
    pub decoded_pixels: u64,
    pub height_classes: u32,
    pub export_runs: u32,
    pub sink_writes: u64,
    pub header_bytes_fetched: u64,
    /// MQ bytes fetched if initialization failed before a snapshot existed.
    /// Zero once a decoder was constructed; then `mq` includes prefetch.
    pub mq_initialization_bytes_fetched: u64,
    pub mq: Option<MqSnapshot>,
    pub poisoned: bool,
}

impl DictionaryProgress {
    /// Includes fixed-header reads and MQ prefetch/terminal lookahead.
    pub fn source_bytes_fetched(self) -> u64 {
        self.header_bytes_fetched
            .saturating_add(self.mq_initialization_bytes_fetched)
            .saturating_add(self.mq.map_or(0, |snapshot| snapshot.source_bytes_fetched))
    }
}

/// Successful dictionary result. Store offsets are relative to the first byte
/// appended by this decoder; the adapter owns subsequent ranged reopening.
#[derive(Debug, Eq, PartialEq)]
pub struct DictionaryReport {
    pub header: DictionaryDataHeader,
    pub catalog: DictionaryCatalog,
    pub progress: DictionaryProgress,
}

#[derive(Debug)]
pub struct DictionaryError {
    pub segment: u32,
    pub offset: u64,
    pub progress: Box<DictionaryProgress>,
    pub kind: DictionaryErrorKind,
}

#[derive(Debug)]
pub enum DictionaryErrorKind {
    InvalidSpan(&'static str),
    Truncated(&'static str),
    Malformed(&'static str),
    Unsupported {
        feature: &'static str,
        value: u64,
    },
    UnsupportedAt {
        x: i8,
        y: i8,
    },
    LimitExceeded {
        resource: &'static str,
        limit: u64,
        attempted: u64,
    },
    AllocationFailed,
    Cancelled,
    Source(Error),
    Header(Box<HeaderError>),
    Sink(Error),
    Mq(Box<MqError>),
    Poisoned,
}

pub type DictionaryResult<T> = Result<T, DictionaryError>;

impl fmt::Display for DictionaryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "JBIG2 symbol dictionary segment {} at source byte {}: ",
            self.segment, self.offset
        )?;
        match &self.kind {
            DictionaryErrorKind::InvalidSpan(reason) => write!(f, "invalid span: {reason}"),
            DictionaryErrorKind::Truncated(field) => write!(f, "truncated {field}"),
            DictionaryErrorKind::Malformed(field) => write!(f, "malformed {field}"),
            DictionaryErrorKind::Unsupported { feature, value } => {
                write!(f, "unsupported {feature} ({value})")
            }
            DictionaryErrorKind::UnsupportedAt { x, y } => {
                write!(f, "unsupported adaptive pixel ({x}, {y})")
            }
            DictionaryErrorKind::LimitExceeded {
                resource,
                limit,
                attempted,
            } => {
                write!(f, "{resource} limit {limit} exceeded by {attempted}")
            }
            DictionaryErrorKind::AllocationFailed => f.write_str("allocation failed"),
            DictionaryErrorKind::Cancelled => f.write_str("cancelled"),
            DictionaryErrorKind::Source(source) => write!(f, "source: {source}"),
            DictionaryErrorKind::Header(source) => write!(f, "segment header: {source}"),
            DictionaryErrorKind::Sink(source) => write!(f, "sink: {source}"),
            DictionaryErrorKind::Mq(source) => write!(f, "MQ: {source}"),
            DictionaryErrorKind::Poisoned => f.write_str("decoder state is poisoned or complete"),
        }
    }
}

impl error::Error for DictionaryError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.kind {
            DictionaryErrorKind::Source(error) | DictionaryErrorKind::Sink(error) => Some(error),
            DictionaryErrorKind::Header(error) => Some(error),
            DictionaryErrorKind::Mq(error) => Some(error),
            _ => None,
        }
    }
}

fn at(header: &SegmentHeader, offset: u64, kind: DictionaryErrorKind) -> DictionaryError {
    DictionaryError {
        segment: header.number,
        offset,
        progress: Box::new(DictionaryProgress::default()),
        kind,
    }
}

/// A refused catalog reservation, reporting the header bytes already read.
/// Not generic, so every decoder instantiation shares it.
fn allocation_failed(header: &SegmentHeader, offset: u64, header_fetched: u64) -> DictionaryError {
    let mut error = at(header, offset, DictionaryErrorKind::AllocationFailed);
    error.progress.header_bytes_fetched = header_fetched;
    error
}

fn limit(
    header: &SegmentHeader,
    offset: u64,
    resource: &'static str,
    maximum: u64,
    attempted: u64,
) -> DictionaryError {
    at(
        header,
        offset,
        DictionaryErrorKind::LimitExceeded {
            resource,
            limit: maximum,
            attempted,
        },
    )
}

fn check_limit(
    header: &SegmentHeader,
    offset: u64,
    resource: &'static str,
    maximum: u64,
    attempted: u64,
) -> DictionaryResult<()> {
    if attempted > maximum {
        Err(limit(header, offset, resource, maximum, attempted))
    } else {
        Ok(())
    }
}

struct HeaderCursor<'a> {
    header: &'a SegmentHeader,
    at: u64,
    end: u64,
    fetched: u64,
    request_bytes: usize,
    max_data_header_bytes: u64,
}

struct CountingSource<'a, S> {
    source: &'a mut S,
    fetched: u64,
}

impl<S: RangedSource> RangedSource for CountingSource<'_, S> {
    fn size(&self) -> u64 {
        self.source.size()
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> crate::Result<usize> {
        let count = self.source.read_at(offset, destination).await?;
        if count <= destination.len() {
            self.fetched = self.fetched.saturating_add(count as u64);
        }
        Ok(count)
    }
}

impl HeaderCursor<'_> {
    fn error(&self, kind: DictionaryErrorKind) -> DictionaryError {
        self.error_at(self.at, kind)
    }

    fn invalid_span(&self, reason: &'static str) -> DictionaryError {
        self.error(DictionaryErrorKind::InvalidSpan(reason))
    }

    fn error_at(&self, offset: u64, kind: DictionaryErrorKind) -> DictionaryError {
        let mut error = at(self.header, offset, kind);
        error.progress.header_bytes_fetched = self.fetched;
        error
    }

    async fn read<const N: usize, S: RangedSource, C: Cancellation>(
        &mut self,
        source: &mut S,
        name: &'static str,
        cancellation: &C,
    ) -> DictionaryResult<[u8; N]> {
        let mut bytes = [0u8; N];
        self.fill(source, name, cancellation, &mut bytes).await?;
        Ok(bytes)
    }

    /// Fill `bytes` from the cursor. Not generic over the field width, so
    /// every header field shares one instantiation per source type.
    async fn fill<S: RangedSource, C: Cancellation>(
        &mut self,
        source: &mut S,
        name: &'static str,
        cancellation: &C,
        bytes: &mut [u8],
    ) -> DictionaryResult<()> {
        let count = len_u64(bytes.len());
        let future = self
            .at
            .checked_add(count)
            .ok_or_else(|| self.invalid_span("header offset overflow"))?;
        let attempted = future - self.header.data.offset;
        if attempted > self.max_data_header_bytes {
            return Err(self.error(DictionaryErrorKind::LimitExceeded {
                resource: "dictionary header bytes",
                limit: self.max_data_header_bytes,
                attempted,
            }));
        }
        if future > self.end {
            return Err(self.error(DictionaryErrorKind::Truncated(name)));
        }
        let mut done = 0;
        while done < bytes.len() {
            if cancellation.is_cancelled() {
                return Err(self.error(DictionaryErrorKind::Cancelled));
            }
            let request = (bytes.len() - done).min(self.request_bytes);
            let got = source
                .read_at(self.at, &mut bytes[done..done + request])
                .await
                .map_err(|error| {
                    self.error(if matches!(error, Error::Cancelled) {
                        DictionaryErrorKind::Cancelled
                    } else {
                        DictionaryErrorKind::Source(error)
                    })
                })?;
            if got > request {
                return Err(self.error(DictionaryErrorKind::Malformed("source read length")));
            }
            if got == 0 {
                return Err(self.error(DictionaryErrorKind::Truncated(name)));
            }
            // `got <= request <= bytes.len() - done`, so `at` stays at or
            // below the checked `future`. `fetched` starts at the framing
            // header's exact length (the reparse read each header byte once),
            // so it stays at or below `at - header_start`.
            self.at += got as u64;
            self.fetched += got as u64;
            done += got;
            if cancellation.is_cancelled() {
                return Err(self.error(DictionaryErrorKind::Cancelled));
            }
        }
        Ok(())
    }
}

/// The source-independent checks that precede the framing reparse. Keeping
/// them outside the generic reader shares one copy across every source and
/// cancellation type. Returns the data end and the framing header start.
fn data_header_bounds(
    header: &SegmentHeader,
    limits: &Limits,
    budget: DictionaryBudget,
    cancellation: &dyn Cancellation,
    source_size: u64,
) -> DictionaryResult<(u64, u64)> {
    limits
        .validate()
        .map_err(|e| at(header, header.data.offset, DictionaryErrorKind::Source(e)))?;
    if budget.max_source_request_bytes == 0 || budget.max_sink_request_bytes == 0 {
        return Err(at(
            header,
            header.data.offset,
            DictionaryErrorKind::Malformed("zero I/O request bound"),
        ));
    }
    if cancellation.is_cancelled() {
        return Err(at(
            header,
            header.data.offset,
            DictionaryErrorKind::Cancelled,
        ));
    }
    if header.segment_type != 0 {
        return Err(at(
            header,
            header.data.offset,
            DictionaryErrorKind::Unsupported {
                feature: "segment type",
                value: u64::from(header.segment_type),
            },
        ));
    }
    check_limit(
        header,
        header.data.offset,
        "dictionary data bytes",
        limits.max_input_bytes,
        header.data.length,
    )?;
    let end = header
        .data
        .offset
        .checked_add(header.data.length)
        .ok_or_else(|| {
            at(
                header,
                header.data.offset,
                DictionaryErrorKind::InvalidSpan("data end overflow"),
            )
        })?;
    if end > source_size {
        return Err(at(
            header,
            header.data.offset,
            DictionaryErrorKind::InvalidSpan("data outside source"),
        ));
    }
    let header_start = header
        .data
        .offset
        .checked_sub(header.header_length)
        .ok_or_else(|| {
            at(
                header,
                header.data.offset,
                DictionaryErrorKind::InvalidSpan("segment header start underflow"),
            )
        })?;
    Ok((end, header_start))
}

/// Compare the framing reparse with the caller's header, outside the generic
/// reader so every instantiation shares it.
fn check_reparsed_header(
    header: &SegmentHeader,
    verified: Result<SegmentHeader, HeaderError>,
    header_start: u64,
    framing_fetched: u64,
) -> DictionaryResult<()> {
    let verified = verified.map_err(|error| {
        let mut located = at(
            header,
            error.offset,
            DictionaryErrorKind::Header(Box::new(error)),
        );
        located.progress.header_bytes_fetched = framing_fetched;
        located
    })?;
    if &verified != header {
        let mut located = at(
            header,
            header_start,
            DictionaryErrorKind::Malformed("segment header metadata mismatch"),
        );
        located.progress.header_bytes_fetched = framing_fetched;
        return Err(located);
    }
    Ok(())
}

/// Parse only the dictionary segment-data header, including conditional AT
/// fields, within `header.data`. This never initializes MQ or writes output.
/// `ArithmeticRefinementAggregate` includes the observed `0x1802` mode and is
/// a classifier result, not a promise that the mode is decoded here.
pub async fn read_dictionary_data_header<S: RangedSource, C: Cancellation>(
    source: &mut S,
    header: &SegmentHeader,
    limits: &Limits,
    budget: DictionaryBudget,
    cancellation: &C,
) -> DictionaryResult<DictionaryDataHeader> {
    let (end, header_start) =
        data_header_bounds(header, limits, budget, cancellation, source.size())?;
    // `header_length <= data.offset`, and the checked data end fits u64;
    // therefore header_length + data.length also fits.
    let complete_length = header.header_length + header.data.length;
    let framing_limits = HeaderLimits {
        max_data_bytes: budget
            .max_body_bytes
            .saturating_add(budget.max_data_header_bytes),
        ..HeaderLimits::default()
    };
    let framing_io_limits = Limits {
        io_chunk_bytes: limits.io_chunk_bytes.min(budget.max_source_request_bytes),
        ..*limits
    };
    let mut framing_source = CountingSource { source, fetched: 0 };
    let verified_result = read_segment_header(
        &mut framing_source,
        SegmentSpan {
            offset: header_start,
            length: complete_length,
        },
        &framing_io_limits,
        framing_limits,
        cancellation,
    )
    .await;
    let framing_fetched = framing_source.fetched;
    check_reparsed_header(header, verified_result, header_start, framing_fetched)?;
    let mut cursor = HeaderCursor {
        header,
        at: header.data.offset,
        end,
        fetched: framing_fetched,
        request_bytes: budget.max_source_request_bytes.min(limits.io_chunk_bytes),
        max_data_header_bytes: budget.max_data_header_bytes,
    };
    let flags = u16::from_be_bytes(
        cursor
            .read(source, "dictionary flags", cancellation)
            .await?,
    );
    if flags & 0xe000 != 0 {
        return Err(cursor.error_at(
            header.data.offset,
            DictionaryErrorKind::Malformed("reserved dictionary flags"),
        ));
    }
    let huffman = flags & 1 != 0;
    let refinement = flags & 2 != 0;
    let template = ((flags >> 10) & 3) as u8;
    let refinement_template = ((flags >> 12) & 1) as u8;
    if !huffman && flags & 0xfc != 0 {
        return Err(cursor.error_at(
            header.data.offset,
            DictionaryErrorKind::Malformed("arithmetic dictionary Huffman selection flags"),
        ));
    }
    if huffman {
        if ((flags >> 2) & 3) == 2 || ((flags >> 4) & 3) == 2 {
            return Err(cursor.error_at(
                header.data.offset,
                DictionaryErrorKind::Malformed("reserved Huffman selector"),
            ));
        }
        if template != 0 {
            return Err(cursor.error_at(
                header.data.offset,
                DictionaryErrorKind::Malformed("Huffman dictionary template"),
            ));
        }
        if !refinement && flags & 0x380 != 0 {
            return Err(cursor.error_at(
                header.data.offset,
                DictionaryErrorKind::Malformed("Huffman direct bitmap flags"),
            ));
        }
    }
    if !refinement && refinement_template != 0 {
        return Err(cursor.error_at(
            header.data.offset,
            DictionaryErrorKind::Malformed("unused refinement template"),
        ));
    }
    let mode = match (huffman, refinement) {
        (false, false) => DictionaryMode::ArithmeticDirect,
        (false, true) => DictionaryMode::ArithmeticRefinementAggregate,
        (true, false) => DictionaryMode::HuffmanDirect,
        (true, true) => DictionaryMode::HuffmanRefinementAggregate,
    };
    let mut at = [(0, 0); 4];
    let at_count = if huffman {
        0
    } else if template == 0 {
        4
    } else {
        1
    };
    for position in at.iter_mut().take(at_count) {
        let [x, y] = cursor.read(source, "dictionary AT", cancellation).await?;
        *position = (x as i8, y as i8);
    }
    if !huffman && template == 2 {
        let (x, y) = at[0];
        if y > 0 || (y == 0 && x >= 0) {
            return Err(cursor.error_at(
                header.data.offset + 2,
                DictionaryErrorKind::Malformed("adaptive pixel references undecoded pixel"),
            ));
        }
    }
    let mut refinement_at = [(0, 0); 2];
    let refinement_at_count = if refinement && refinement_template == 0 {
        2
    } else {
        0
    };
    for position in refinement_at.iter_mut().take(refinement_at_count) {
        let [x, y] = cursor
            .read(source, "dictionary refinement AT", cancellation)
            .await?;
        *position = (x as i8, y as i8);
    }
    let exported_offset = cursor.at;
    let exported_symbols = u32::from_be_bytes(
        cursor
            .read(source, "exported symbol count", cancellation)
            .await?,
    );
    let new_offset = cursor.at;
    let new_symbols = u32::from_be_bytes(
        cursor
            .read(source, "new symbol count", cancellation)
            .await?,
    );
    let header_bytes = cursor.at - header.data.offset;
    if new_symbols > budget.max_new_symbols {
        return Err(cursor.error_at(
            new_offset,
            DictionaryErrorKind::LimitExceeded {
                resource: "new symbols",
                limit: u64::from(budget.max_new_symbols),
                attempted: u64::from(new_symbols),
            },
        ));
    }
    if exported_symbols > budget.max_exported_symbols {
        return Err(cursor.error_at(
            exported_offset,
            DictionaryErrorKind::LimitExceeded {
                resource: "exported symbols",
                limit: u64::from(budget.max_exported_symbols),
                attempted: u64::from(exported_symbols),
            },
        ));
    }
    let body_length = end - cursor.at;
    if body_length > budget.max_body_bytes {
        return Err(cursor.error(DictionaryErrorKind::LimitExceeded {
            resource: "dictionary body bytes",
            limit: budget.max_body_bytes,
            attempted: body_length,
        }));
    }
    if !huffman && body_length < 2 {
        return Err(cursor.error(DictionaryErrorKind::Truncated("MQ body terminal pair")));
    }
    Ok(DictionaryDataHeader {
        flags,
        mode,
        template,
        refinement_template,
        bitmap_context_used: flags & 0x100 != 0,
        bitmap_context_retained: flags & 0x200 != 0,
        at,
        at_count: at_count as u8,
        refinement_at,
        refinement_at_count: refinement_at_count as u8,
        exported_symbols,
        new_symbols,
        header_bytes,
        body: SegmentSpan {
            offset: cursor.at,
            length: body_length,
        },
    })
}

/// The source-independent checks between the data header and MQ
/// initialization, shared by every decoder instantiation.
fn check_direct_header(
    segment: &SegmentHeader,
    header: &DictionaryDataHeader,
    context_count: usize,
    limits: &Limits,
    budget: DictionaryBudget,
) -> DictionaryResult<()> {
    let location = header.body.offset;
    let header_fetched = segment.header_length + header.header_bytes;
    let after_header = |offset, kind| {
        let mut error = at(segment, offset, kind);
        error.progress.header_bytes_fetched = header_fetched;
        error
    };
    let check_after_header = |resource, maximum, attempted| {
        check_limit(segment, location, resource, maximum, attempted).map_err(|mut error| {
            error.progress.header_bytes_fetched = header_fetched;
            error
        })
    };
    let unsupported = |feature, value| {
        after_header(
            location,
            DictionaryErrorKind::Unsupported { feature, value },
        )
    };
    match header.mode {
        DictionaryMode::ArithmeticDirect => {}
        DictionaryMode::ArithmeticRefinementAggregate => {
            return Err(unsupported(
                "symbol dictionary refinement/aggregation",
                u64::from(header.flags),
            ));
        }
        DictionaryMode::HuffmanDirect | DictionaryMode::HuffmanRefinementAggregate => {
            return Err(unsupported(
                "Huffman symbol dictionary",
                u64::from(header.flags),
            ));
        }
    }
    if header.template != 2 {
        return Err(unsupported(
            "dictionary generic template",
            u64::from(header.template),
        ));
    }
    if header.bitmap_context_used || header.bitmap_context_retained {
        return Err(unsupported(
            "bitmap context carry",
            u64::from(header.flags & 0x300),
        ));
    }
    if segment.page_association != 1 {
        return Err(unsupported(
            "dictionary page association",
            u64::from(segment.page_association),
        ));
    }
    if !segment.referred_to.is_empty() {
        return Err(unsupported(
            "imported dictionary references",
            segment.referred_to.len() as u64,
        ));
    }
    // A nonempty reference list was rejected above; this slice's fixed
    // imported-symbol limit is `MAX_IMPORTED_SYMBOLS` (zero).
    // The parsed template-2 AT was already checked as backwards-only.
    if header.at[0] != (2, -1) {
        return Err(after_header(
            segment.data.offset + 2,
            DictionaryErrorKind::UnsupportedAt {
                x: header.at[0].0,
                y: header.at[0].1,
            },
        ));
    }
    if header.exported_symbols > header.new_symbols {
        return Err(after_header(
            location,
            DictionaryErrorKind::Malformed("exported count exceeds available symbols"),
        ));
    }
    if context_count != TOTAL_CONTEXTS {
        return Err(after_header(
            location,
            DictionaryErrorKind::Malformed("expected exactly 7680 integer and bitmap MQ contexts"),
        ));
    }
    // Both counts are u32. Even their sum times the fixed descriptor
    // size is far below u64::MAX on every supported target.
    let descriptor_count = u64::from(header.new_symbols) + u64::from(header.exported_symbols);
    let catalog_bytes = descriptor_count * mem::size_of::<SymbolDescriptor>() as u64;
    check_after_header(
        "catalog metadata bytes",
        budget.max_catalog_bytes,
        catalog_bytes,
    )?;
    check_after_header(
        "catalog allocation bytes",
        limits.max_allocation_bytes,
        catalog_bytes,
    )?;
    let base_working = (TOTAL_CONTEXTS * mem::size_of::<MqContext>()) as u64
        + (MQ_STATE_COUNT * mem::size_of::<MqState>()) as u64
        + MQ_BUFFER_BYTES
        + catalog_bytes;
    check_after_header(
        "dictionary working bytes",
        budget.max_working_bytes,
        base_working,
    )?;
    let mq_request = limits.io_chunk_bytes.min(MQ_BUFFER_BYTES as usize);
    check_after_header(
        "MQ source request bytes",
        budget.max_source_request_bytes as u64,
        mq_request as u64,
    )?;
    Ok(())
}

/// Checked dimensions of one new symbol: width, height, row stride, pixels,
/// and packed bytes.
type SymbolGeometry = (u32, u32, usize, u64, u64);

fn check_budget(
    resource: &'static str,
    limit: u64,
    attempted: u64,
) -> Result<(), DictionaryErrorKind> {
    if attempted > limit {
        Err(DictionaryErrorKind::LimitExceeded {
            resource,
            limit,
            attempted,
        })
    } else {
        Ok(())
    }
}

/// Validate one decoded symbol size against every budget before any bitmap
/// work. Not generic, so every decoder instantiation shares it; the caller
/// locates the returned error kind at the current MQ offset.
fn symbol_geometry(
    width: i64,
    height: i64,
    budget: &DictionaryBudget,
    limits: &Limits,
    header: &DictionaryDataHeader,
    progress: &DictionaryProgress,
) -> Result<SymbolGeometry, DictionaryErrorKind> {
    if width < 0 || height < 0 {
        return Err(DictionaryErrorKind::Malformed("negative symbol dimension"));
    }
    if width == 0 || height == 0 {
        return Err(DictionaryErrorKind::Unsupported {
            feature: "zero-dimension symbol bitmap",
            value: 0,
        });
    }
    let width = try_convert(
        width,
        DictionaryErrorKind::Malformed("symbol width exceeds 32 bits"),
    )?;
    let height = try_convert(
        height,
        DictionaryErrorKind::Malformed("symbol height exceeds 32 bits"),
    )?;
    check_budget(
        "symbol width",
        u64::from(budget.max_width),
        u64::from(width),
    )?;
    check_budget(
        "symbol height",
        u64::from(budget.max_height),
        u64::from(height),
    )?;
    // A product of two u32 dimensions fits u64 exactly.
    let pixels = u64::from(width) * u64::from(height);
    check_budget("symbol pixels", budget.max_pixels_per_symbol, pixels)?;
    let total_pixels =
        progress
            .decoded_pixels
            .checked_add(pixels)
            .ok_or(DictionaryErrorKind::InvalidSpan(
                "total pixel count overflow",
            ))?;
    check_budget("dictionary pixels", budget.max_total_pixels, total_pixels)?;
    let stride = u64::from(width).div_ceil(8);
    // The maximum stride is 2^29 bytes, so this product fits u64.
    let bytes = stride * u64::from(height);
    check_budget("symbol bytes", budget.max_bytes_per_symbol, bytes)?;
    let stored =
        progress
            .stored_bitmap_bytes
            .checked_add(bytes)
            .ok_or(DictionaryErrorKind::InvalidSpan(
                "stored byte count overflow",
            ))?;
    check_budget(
        "stored bitmap bytes",
        budget.max_stored_bitmap_bytes,
        stored,
    )?;
    check_budget("output bytes", limits.max_output_bytes, stored)?;
    let scratch = stride * 3;
    check_budget("row scratch bytes", limits.max_allocation_bytes, scratch)?;
    let metadata = (u64::from(header.new_symbols) + u64::from(header.exported_symbols))
        * mem::size_of::<SymbolDescriptor>() as u64;
    let working = (TOTAL_CONTEXTS * mem::size_of::<MqContext>()) as u64
        + (MQ_STATE_COUNT * mem::size_of::<MqState>()) as u64
        + MQ_BUFFER_BYTES
        + metadata
        + scratch;
    check_budget(
        "dictionary working bytes",
        budget.max_working_bytes,
        working,
    )?;
    // At most 2^29 bytes; even wasm32's usize can represent it.
    let stride_usize = stride as usize;
    Ok((width, height, stride_usize, pixels, bytes))
}

/// Stateful direct dictionary decode. The caller constructs an
/// `IntegerContextBanks::with_extra_contexts(1024, ...)` owner; an IAID owner
/// cannot be passed here and its bitmap range cannot alias this dictionary.
///
/// `decode()` streams every symbol's packed rows to `store`, then checks IAEX
/// and the single MQ tail. A failed or dropped pending call permanently poisons
/// this object. The caller must discard all bytes appended to `store` unless a
/// complete `DictionaryReport` is returned.
pub struct DirectDictionaryDecoder<'a, S: RangedSource, W: SequentialSink, C: Cancellation> {
    mq: MqDecoder<'a, S, C>,
    store: &'a mut W,
    cancellation: &'a C,
    header: DictionaryDataHeader,
    segment: u32,
    budget: DictionaryBudget,
    io_limits: &'a Limits,
    catalog: DictionaryCatalog,
    progress: DictionaryProgress,
    previous_two: Vec<u8>,
    previous_one: Vec<u8>,
    current: Vec<u8>,
    completed: bool,
    poisoned: bool,
}

impl<'a, S: RangedSource, W: SequentialSink, C: Cancellation> DirectDictionaryDecoder<'a, S, W, C> {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        source: &'a mut S,
        segment: &SegmentHeader,
        table: &'a MqTable,
        banks: &'a mut IntegerContextBanks,
        store: &'a mut W,
        limits: &'a Limits,
        cancellation: &'a C,
        mq_budget: MqBudget,
        budget: DictionaryBudget,
    ) -> DictionaryResult<Self> {
        let header =
            read_dictionary_data_header(source, segment, limits, budget, cancellation).await?;
        let header_fetched = segment.header_length + header.header_bytes;
        check_direct_header(
            segment,
            &header,
            banks.mq_contexts_mut().count(),
            limits,
            budget,
        )?;
        let location = header.body.offset;
        let mut new_symbols = Vec::new();
        let mut exported_symbols = Vec::new();
        let failed = allocation_failed(segment, location, header_fetched);
        reserve_exact(&mut new_symbols, usize_from_u32(header.new_symbols), failed)?;
        let failed = allocation_failed(segment, location, header_fetched);
        let exported_count = usize_from_u32(header.exported_symbols);
        reserve_exact(&mut exported_symbols, exported_count, failed)?;
        // No bitmap context reuse is accepted. T.88 §7.4.2.2 also resets all
        // arithmetic-integer statistics at each new dictionary.
        banks.reset_all();
        let span = MqSpan {
            offset: header.body.offset,
            length: header.body.length,
        };
        let mut mq_initialization_bytes_fetched = 0;
        let mq = MqDecoder::new_with_init_progress(
            source,
            span,
            table,
            banks.mq_contexts_mut(),
            limits,
            cancellation,
            mq_budget,
            &mut mq_initialization_bytes_fetched,
        )
        .await
        .map_err(|error| {
            let mut located = at(
                segment,
                error.offset.unwrap_or(location),
                DictionaryErrorKind::Mq(Box::new(error)),
            );
            located.progress.header_bytes_fetched = header_fetched;
            located.progress.mq_initialization_bytes_fetched = mq_initialization_bytes_fetched;
            located
        })?;
        let progress = DictionaryProgress {
            header_bytes_fetched: header_fetched,
            mq: Some(mq.snapshot()),
            ..DictionaryProgress::default()
        };
        Ok(Self {
            mq,
            store,
            cancellation,
            header,
            segment: segment.number,
            budget,
            io_limits: limits,
            catalog: DictionaryCatalog {
                new_symbols,
                exported_symbols,
            },
            progress,
            previous_two: Vec::new(),
            previous_one: Vec::new(),
            current: Vec::new(),
            completed: false,
            poisoned: false,
        })
    }

    /// Current progress. A failed or dropped pending `decode` call reports
    /// `poisoned=true`; `MqSnapshot` distinguishes semantic and fetched input.
    pub fn progress(&self) -> DictionaryProgress {
        let mut progress = self.progress;
        progress.mq = Some(self.mq.snapshot());
        progress.poisoned = self.poisoned || progress.mq.is_some_and(|mq| mq.poisoned);
        progress
    }

    fn error(&self, offset: u64, kind: DictionaryErrorKind) -> DictionaryError {
        DictionaryError {
            segment: self.segment,
            offset,
            progress: Box::new(self.progress()),
            kind,
        }
    }

    fn malformed(&self, reason: &'static str) -> DictionaryError {
        self.error(
            self.current_offset(),
            DictionaryErrorKind::Malformed(reason),
        )
    }

    fn current_offset(&self) -> u64 {
        self.mq.snapshot().current_input_offset
    }

    fn check_cancelled(&self) -> DictionaryResult<()> {
        if self.cancellation.is_cancelled() {
            Err(self.error(self.current_offset(), DictionaryErrorKind::Cancelled))
        } else {
            Ok(())
        }
    }

    fn check(&self, resource: &'static str, limit: u64, attempted: u64) -> DictionaryResult<()> {
        check_budget(resource, limit, attempted)
            .map_err(|kind| self.error(self.current_offset(), kind))
    }

    async fn integer(&mut self, procedure: IntegerProcedure) -> DictionaryResult<IntegerValue> {
        let result = decode_integer(&mut self.mq, procedure).await;
        result.map_err(|error| {
            self.error(
                error.offset.unwrap_or(self.current_offset()),
                DictionaryErrorKind::Mq(Box::new(error)),
            )
        })
    }

    fn signed(&self, value: IntegerValue, field: &'static str) -> DictionaryResult<i64> {
        match value {
            IntegerValue::Signed(value) => Ok(value),
            IntegerValue::OutOfBand => {
                Err(self.error(self.current_offset(), DictionaryErrorKind::Malformed(field)))
            }
        }
    }

    fn checked_geometry(&self, width: i64, height: i64) -> DictionaryResult<SymbolGeometry> {
        symbol_geometry(
            width,
            height,
            &self.budget,
            self.io_limits,
            &self.header,
            &self.progress,
        )
        .map_err(|kind| self.error(self.current_offset(), kind))
    }

    fn prepare_row(row: &mut Vec<u8>, stride: usize) -> Result<(), ()> {
        if row.len() < stride {
            row.try_reserve_exact(stride - row.len()).map_err(|_| ())?;
        }
        row.resize(stride, 0);
        row.fill(0);
        Ok(())
    }

    async fn write_row(&mut self) -> DictionaryResult<()> {
        let mut done = 0;
        while done < self.current.len() {
            self.check_cancelled()?;
            // Every earlier write stored at least one byte (a failed write
            // poisons the decoder), so `sink_writes <= stored_bitmap_bytes`.
            // At least one byte of this symbol is still pending, and the
            // whole symbol fits `max_stored_bitmap_bytes`, so
            // `stored_bitmap_bytes < u64::MAX` and this cannot overflow.
            debug_assert!(self.progress.sink_writes <= self.progress.stored_bitmap_bytes);
            let attempted_writes = self.progress.sink_writes + 1;
            self.check("sink writes", self.budget.max_sink_writes, attempted_writes)?;
            let count = (self.current.len() - done)
                .min(self.io_limits.io_chunk_bytes)
                .min(self.budget.max_sink_request_bytes);
            self.progress.sink_writes = attempted_writes;
            let written = self
                .store
                .write(&self.current[done..done + count])
                .await
                .map_err(|error| {
                    self.error(
                        self.current_offset(),
                        if matches!(error, Error::Cancelled) {
                            DictionaryErrorKind::Cancelled
                        } else {
                            DictionaryErrorKind::Sink(error)
                        },
                    )
                })?;
            if written > count {
                return Err(self.malformed("sink write length"));
            }
            if written == 0 {
                return Err(self.error(
                    self.current_offset(),
                    DictionaryErrorKind::Sink(Error::Io(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "symbol store made no progress",
                    ))),
                ));
            }
            // `symbol_geometry` checked that this symbol's packed bytes fit
            // `max_stored_bitmap_bytes` after every earlier symbol, and the
            // rows written so far never exceed them.
            self.progress.stored_bitmap_bytes += written as u64;
            done += written;
            self.check_cancelled()?;
        }
        Ok(())
    }

    async fn bitmap(
        &mut self,
        width: u32,
        height: u32,
        stride: usize,
        pixels: u64,
        bytes: u64,
    ) -> DictionaryResult<SymbolDescriptor> {
        let relative_store_offset = self.progress.stored_bitmap_bytes;
        // Built before the rows are borrowed, as the refinement and generic
        // decoders build their refused-row errors.
        let failed = self.error(self.current_offset(), DictionaryErrorKind::AllocationFailed);
        Self::prepare_row(&mut self.previous_two, stride)
            .and_then(|()| Self::prepare_row(&mut self.previous_one, stride))
            .and_then(|()| Self::prepare_row(&mut self.current, stride))
            .or(Err(failed))?;
        for _ in 0..height {
            for x in 0..width {
                self.check_cancelled()?;
                let context = INTEGER_CONTEXT_COUNT
                    + template2_context(
                        &self.previous_two,
                        &self.previous_one,
                        &self.current,
                        width,
                        x,
                    );
                let bit = self.mq.decode_bit(context).await.map_err(|error| {
                    self.error(
                        error.offset.unwrap_or(self.current_offset()),
                        DictionaryErrorKind::Mq(Box::new(error)),
                    )
                })?;
                if bit {
                    self.current[x as usize / 8] |= 0x80 >> (x % 8);
                }
            }
            self.write_row().await?;
            mem::swap(&mut self.previous_two, &mut self.previous_one);
            mem::swap(&mut self.previous_one, &mut self.current);
            self.current.fill(0);
        }
        self.progress.decoded_pixels += pixels;
        debug_assert_eq!(
            self.progress.stored_bitmap_bytes - relative_store_offset,
            bytes
        );
        Ok(SymbolDescriptor {
            width,
            height,
            row_stride: stride as u32,
            relative_store_offset,
            stored_bytes: bytes,
        })
    }

    async fn decode_symbols(&mut self) -> DictionaryResult<()> {
        let mut class_height = 0i64;
        while self.progress.completed_symbols < self.header.new_symbols {
            self.check_cancelled()?;
            let classes = u64::from(self.progress.height_classes) + 1;
            self.check(
                "height classes",
                u64::from(self.budget.max_height_classes),
                classes,
            )?;
            // At most `max_height_classes`, a u32.
            self.progress.height_classes = classes as u32;
            let value = self.integer(IntegerProcedure::Iadh).await?;
            let delta = self.signed(value, "IADH out of band")?;
            // `class_height` is in `0..=u32::MAX` here and every decoded
            // integer's magnitude is below 2^33, so the sum fits i64.
            class_height += delta;
            if class_height < 0 || class_height > i64::from(u32::MAX) {
                return Err(self.malformed("height class dimension"));
            }
            self.check(
                "height class",
                u64::from(self.budget.max_height),
                class_height as u64,
            )?;
            let mut class_width = 0i64;
            loop {
                let value = self.integer(IntegerProcedure::Iadw).await?;
                let delta = match value {
                    IntegerValue::OutOfBand => break,
                    IntegerValue::Signed(delta) => delta,
                };
                if self.progress.completed_symbols == self.header.new_symbols {
                    return Err(self.malformed("symbol-count overrun before width OOB"));
                }
                // A checked symbol width is in `1..=u32::MAX`, so the same
                // integer bound applies.
                class_width += delta;
                let (width, height, stride, pixels, bytes) =
                    self.checked_geometry(class_width, class_height)?;
                let descriptor = self.bitmap(width, height, stride, pixels, bytes).await?;
                self.catalog.new_symbols.push(descriptor);
                self.progress.completed_symbols += 1;
            }
        }
        Ok(())
    }

    async fn decode_exports(&mut self) -> DictionaryResult<()> {
        let mut index = 0u32;
        let mut flag = false;
        // T.88 §6.5.10 performs the first IAEX decode before its repeat-until
        // condition. Even a zero-total dictionary consumes one zero run.
        loop {
            self.check_cancelled()?;
            let runs = u64::from(self.progress.export_runs) + 1;
            self.check("export runs", u64::from(self.budget.max_export_runs), runs)?;
            // At most `max_export_runs`, a u32.
            self.progress.export_runs = runs as u32;
            let value = self.integer(IntegerProcedure::Iaex).await?;
            let length = self.signed(value, "IAEX out of band")?;
            if length < 0 {
                return Err(self.malformed("negative export run"));
            }
            // `length` is nonnegative i64 and `index` is u32, so the sum
            // remains below u64::MAX.
            let end = u64::from(index) + length as u64;
            if end > u64::from(self.header.new_symbols) {
                return Err(self.malformed("export run overshoot"));
            }
            let end = end as u32;
            if flag {
                let exports = self.catalog.exported_symbols.len() as u64 + u64::from(end - index);
                if exports > u64::from(self.header.exported_symbols) {
                    return Err(self.malformed("exported symbol total"));
                }
                self.catalog
                    .exported_symbols
                    .extend_from_slice(&self.catalog.new_symbols[index as usize..end as usize]);
            }
            index = end;
            flag = !flag;
            if index == self.header.new_symbols {
                break;
            }
        }
        if self.catalog.exported_symbols.len() != self.header.exported_symbols as usize {
            return Err(self.malformed("exported symbol total"));
        }
        Ok(())
    }

    /// Stream all new bitmaps, decode export runs, check one complete MQ body,
    /// then flush. `Ok` is the only state in which the catalog/store is valid.
    pub async fn decode(&mut self) -> DictionaryResult<DictionaryReport> {
        if self.poisoned || self.completed {
            return Err(self.error(self.current_offset(), DictionaryErrorKind::Poisoned));
        }
        self.poisoned = true;
        self.decode_symbols().await?;
        self.decode_exports().await?;
        self.check_cancelled()?;
        let decisions = self.mq.snapshot().symbols_decoded;
        let snapshot = self
            .mq
            .finish_with_snapshot_mut(decisions)
            .await
            .map_err(|error| {
                self.error(
                    error.offset.unwrap_or(self.current_offset()),
                    DictionaryErrorKind::Mq(Box::new(error)),
                )
            })?;
        self.progress.mq = Some(snapshot);
        self.check_cancelled()?;
        self.store.flush().await.map_err(|error| {
            self.error(
                self.current_offset(),
                if matches!(error, Error::Cancelled) {
                    DictionaryErrorKind::Cancelled
                } else {
                    DictionaryErrorKind::Sink(error)
                },
            )
        })?;
        self.check_cancelled()?;
        self.completed = true;
        self.poisoned = false;
        let catalog = mem::replace(
            &mut self.catalog,
            DictionaryCatalog {
                new_symbols: Vec::new(),
                exported_symbols: Vec::new(),
            },
        );
        Ok(DictionaryReport {
            header: self.header,
            catalog,
            progress: self.progress(),
        })
    }
}

#[cfg(test)]
mod tests;
