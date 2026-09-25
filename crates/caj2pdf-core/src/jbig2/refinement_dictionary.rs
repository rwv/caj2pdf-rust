// SPDX-License-Identifier: MIT

//! Bounded single-reference arithmetic symbol dictionaries for the observed
//! T.88 segment-2 profile. The caller owns probability states and both bitmap
//! stores; this module contains no document or official table bytes.

use super::{
    SegmentHeader,
    dictionary::{
        DictionaryBudget, DictionaryDataHeader, DictionaryError, DictionaryMode, DictionaryReport,
        SymbolDescriptor, read_dictionary_data_header,
    },
    iaid::{IaidContextBanks, IaidLayout, checked_symbol_index, decode_iaid},
    integer::{IntegerProcedure, IntegerValue, decode_integer},
    mq::{
        MQ_STATE_COUNT, MqBudget, MqContext, MqDecoder, MqError, MqSnapshot, MqSpan, MqState,
        MqTable,
    },
    refinement::{
        RefinementBudget, RefinementDecoder, RefinementError, RefinementProgress,
        RefinementReference, RefinementRequest,
    },
};
use crate::{Cancellation, Error, Limits, RangedSource, SequentialSink};
use std::{error, fmt, mem};

const GR_CONTEXTS: usize = 1024;
const MQ_BUFFER_BYTES: u64 = 256;

/// Additional limits for imported symbols and the combined dictionary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RefinementDictionaryBudget {
    pub max_imported_symbols: u32,
    pub max_total_symbols: u32,
    pub max_imported_bitmap_bytes: u64,
    pub max_imported_store_span: u64,
    pub max_catalog_bytes: u64,
    pub max_working_bytes: u64,
}

impl Default for RefinementDictionaryBudget {
    fn default() -> Self {
        Self {
            max_imported_symbols: 4096,
            max_total_symbols: 8192,
            max_imported_bitmap_bytes: 128 * 1024 * 1024,
            max_imported_store_span: 128 * 1024 * 1024,
            max_catalog_bytes: 1024 * 1024,
            max_working_bytes: 16 * 1024 * 1024,
        }
    }
}

/// An exported symbol retains the owner of its packed bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SymbolStore {
    Imported,
    New,
}

/// Checked offset plus the store identity and absolute base supplied by the
/// caller. A consumer must reopen the corresponding store, not reinterpret an
/// imported offset in the new store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoredSymbol {
    pub store: SymbolStore,
    pub store_base: u64,
    pub symbol: SymbolDescriptor,
}

#[derive(Debug, Eq, PartialEq)]
pub struct RefinementDictionaryCatalog {
    pub new_symbols: Vec<SymbolDescriptor>,
    pub exported_symbols: Vec<StoredSymbol>,
}

/// Complete arithmetic-branch counts. A count changes only after the whole
/// IAAI value has decoded; unsupported or malformed values remain visible.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct IaaiBranches {
    pub single_reference: u32,
    pub zero: u32,
    pub aggregation: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RefinementDictionaryProgress {
    pub completed_symbols: u32,
    pub height_classes: u32,
    pub export_runs: u32,
    pub iaai: IaaiBranches,
    pub header_bytes_fetched: u64,
    pub mq_initialization_bytes_fetched: u64,
    pub refinement: RefinementProgress,
    pub mq: Option<MqSnapshot>,
    pub poisoned: bool,
}

impl RefinementDictionaryProgress {
    pub fn source_bytes_fetched(self) -> u64 {
        self.header_bytes_fetched
            .saturating_add(self.mq_initialization_bytes_fetched)
            .saturating_add(self.mq.map_or(0, |snapshot| snapshot.source_bytes_fetched))
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct RefinementDictionaryReport {
    pub header: DictionaryDataHeader,
    pub catalog: RefinementDictionaryCatalog,
    pub progress: RefinementDictionaryProgress,
}

#[derive(Debug)]
pub struct RefinementDictionaryError {
    pub segment: u32,
    pub offset: u64,
    pub progress: Box<RefinementDictionaryProgress>,
    pub kind: RefinementDictionaryErrorKind,
}

#[derive(Debug)]
pub enum RefinementDictionaryErrorKind {
    InvalidSpan(&'static str),
    Malformed(&'static str),
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
    Header(Box<DictionaryError>),
    Mq(Box<MqError>),
    Refinement(Box<RefinementError>),
    Sink(Error),
    Poisoned,
}

pub type RefinementDictionaryResult<T> = Result<T, RefinementDictionaryError>;

impl fmt::Display for RefinementDictionaryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "JBIG2 refinement dictionary {} at source byte {}: ",
            self.segment, self.offset
        )?;
        match &self.kind {
            RefinementDictionaryErrorKind::InvalidSpan(reason) => {
                write!(f, "invalid span: {reason}")
            }
            RefinementDictionaryErrorKind::Malformed(reason) => write!(f, "malformed {reason}"),
            RefinementDictionaryErrorKind::Unsupported { feature, value } => {
                write!(f, "unsupported {feature} ({value})")
            }
            RefinementDictionaryErrorKind::LimitExceeded {
                resource,
                limit,
                attempted,
            } => {
                write!(f, "{resource} limit {limit} exceeded by {attempted}")
            }
            RefinementDictionaryErrorKind::AllocationFailed => f.write_str("allocation failed"),
            RefinementDictionaryErrorKind::Cancelled => f.write_str("cancelled"),
            RefinementDictionaryErrorKind::Header(error) => write!(f, "dictionary header: {error}"),
            RefinementDictionaryErrorKind::Mq(error) => write!(f, "MQ: {error}"),
            RefinementDictionaryErrorKind::Refinement(error) => write!(f, "refinement: {error}"),
            RefinementDictionaryErrorKind::Sink(error) => write!(f, "sink: {error}"),
            RefinementDictionaryErrorKind::Poisoned => {
                f.write_str("decoder state is poisoned or complete")
            }
        }
    }
}

impl error::Error for RefinementDictionaryError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.kind {
            RefinementDictionaryErrorKind::Header(error) => Some(error),
            RefinementDictionaryErrorKind::Mq(error) => Some(error),
            RefinementDictionaryErrorKind::Refinement(error) => Some(error),
            RefinementDictionaryErrorKind::Sink(error) => Some(error),
            _ => None,
        }
    }
}

fn before_mq(
    segment: &SegmentHeader,
    offset: u64,
    header_fetched: u64,
    kind: RefinementDictionaryErrorKind,
) -> RefinementDictionaryError {
    RefinementDictionaryError {
        segment: segment.number,
        offset,
        progress: Box::new(RefinementDictionaryProgress {
            header_bytes_fetched: header_fetched,
            ..RefinementDictionaryProgress::default()
        }),
        kind,
    }
}

fn checked_cap(
    segment: &SegmentHeader,
    offset: u64,
    header_fetched: u64,
    resource: &'static str,
    maximum: u64,
    attempted: u64,
) -> RefinementDictionaryResult<()> {
    if attempted > maximum {
        Err(before_mq(
            segment,
            offset,
            header_fetched,
            RefinementDictionaryErrorKind::LimitExceeded {
                resource,
                limit: maximum,
                attempted,
            },
        ))
    } else {
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_imported(
    segment: &SegmentHeader,
    imported_segment: &SegmentHeader,
    imported_report: &DictionaryReport,
    store_size: u64,
    store_base: u64,
    budget: RefinementDictionaryBudget,
    refinement_budget: RefinementBudget,
    header_fetched: u64,
) -> RefinementDictionaryResult<()> {
    let bad = |reason| {
        before_mq(
            segment,
            segment.data.offset,
            header_fetched,
            RefinementDictionaryErrorKind::Malformed(reason),
        )
    };
    if segment.referred_to.as_slice() != [imported_segment.number] {
        return Err(bad("expected exactly the supplied dictionary reference"));
    }
    if imported_segment.segment_type != 0
        || imported_segment.number >= segment.number
        || imported_segment.page_association != segment.page_association
        || !imported_segment.referred_to.is_empty()
    {
        return Err(bad("imported dictionary segment metadata"));
    }
    let report = imported_report;
    let expected_body_offset = imported_segment
        .data
        .offset
        .checked_add(report.header.header_bytes)
        .ok_or_else(|| bad("imported dictionary body offset overflow"))?;
    let imported_end = report
        .header
        .body
        .offset
        .checked_add(report.header.body.length)
        .ok_or_else(|| bad("imported dictionary body end overflow"))?;
    let segment_end = imported_segment
        .data
        .offset
        .checked_add(imported_segment.data.length)
        .ok_or_else(|| bad("imported segment data end overflow"))?;
    if report.header.mode != DictionaryMode::ArithmeticDirect
        || report.header.flags != 0x0800
        || report.header.body.offset != expected_body_offset
        || imported_end != segment_end
        || report.header.new_symbols as usize != report.catalog.new_symbols.len()
        || report.header.exported_symbols as usize != report.catalog.exported_symbols.len()
        || report.progress.poisoned
        || report.progress.completed_symbols != report.header.new_symbols
        || report.progress.mq.is_none_or(|mq| mq.poisoned)
    {
        return Err(bad("imported dictionary is not a complete direct report"));
    }
    if store_base > store_size {
        return Err(bad("imported store base outside source"));
    }
    checked_cap(
        segment,
        segment.data.offset,
        header_fetched,
        "imported symbols",
        u64::from(budget.max_imported_symbols),
        report.catalog.exported_symbols.len() as u64,
    )?;
    checked_cap(
        segment,
        segment.data.offset,
        header_fetched,
        "imported catalog new symbols",
        u64::from(budget.max_imported_symbols),
        report.catalog.new_symbols.len() as u64,
    )?;
    // Both lengths equal checked u32 header counts, so this sum and its byte
    // product fit u64 on native and wasm32 targets.
    let imported_catalog_count =
        report.catalog.new_symbols.len() as u64 + report.catalog.exported_symbols.len() as u64;
    let imported_catalog_bytes = imported_catalog_count * mem::size_of::<SymbolDescriptor>() as u64;
    checked_cap(
        segment,
        segment.data.offset,
        header_fetched,
        "imported catalog metadata bytes",
        budget.max_catalog_bytes,
        imported_catalog_bytes,
    )?;
    let mut next_new = 0usize;
    for exported in &report.catalog.exported_symbols {
        let matching = report.catalog.new_symbols[next_new..]
            .iter()
            .position(|candidate| candidate == exported)
            .ok_or_else(|| bad("imported exports do not follow new-symbol order"))?;
        next_new += matching + 1;
    }
    let mut previous_end = 0;
    let mut total_bytes = 0u64;
    for descriptor in &report.catalog.exported_symbols {
        if descriptor.width == 0 || descriptor.height == 0 {
            return Err(bad("zero imported bitmap dimension"));
        }
        let stride = u64::from(descriptor.width).div_ceil(8);
        let bytes = stride * u64::from(descriptor.height);
        let pixels = u64::from(descriptor.width) * u64::from(descriptor.height);
        if u64::from(descriptor.row_stride) != stride || descriptor.stored_bytes != bytes {
            return Err(bad("noncanonical imported bitmap descriptor"));
        }
        if descriptor.relative_store_offset < previous_end {
            return Err(bad("overlapping or unordered imported descriptors"));
        }
        let relative_end = descriptor
            .relative_store_offset
            .checked_add(bytes)
            .ok_or_else(|| bad("imported descriptor end overflow"))?;
        let absolute_end = store_base
            .checked_add(relative_end)
            .ok_or_else(|| bad("imported store absolute end overflow"))?;
        if absolute_end > store_size {
            return Err(bad("imported descriptor outside ranged source"));
        }
        for (name, cap, value) in [
            (
                "imported width",
                u64::from(refinement_budget.max_reference_width),
                u64::from(descriptor.width),
            ),
            (
                "imported height",
                u64::from(refinement_budget.max_reference_height),
                u64::from(descriptor.height),
            ),
            (
                "imported pixels per bitmap",
                refinement_budget.max_reference_pixels_per_bitmap,
                pixels,
            ),
            (
                "imported bytes per bitmap",
                refinement_budget.max_reference_bytes_per_bitmap,
                bytes,
            ),
            (
                "imported store span",
                budget.max_imported_store_span,
                relative_end,
            ),
        ] {
            checked_cap(
                segment,
                segment.data.offset,
                header_fetched,
                name,
                cap,
                value,
            )?;
        }
        total_bytes = total_bytes
            .checked_add(bytes)
            .ok_or_else(|| bad("imported byte total overflow"))?;
        previous_end = relative_end;
    }
    checked_cap(
        segment,
        segment.data.offset,
        header_fetched,
        "imported bitmap bytes",
        budget.max_imported_bitmap_bytes,
        total_bytes,
    )
}

/// One coding unit over the exact second dictionary body. The imported source
/// and a separately reopened view of the new store permit bounded row reads.
/// The caller must ensure the new source and sink identify the same storage.
pub struct RefinementDictionaryDecoder<
    'a,
    S: RangedSource,
    RI: RangedSource,
    RN: RangedSource,
    W: SequentialSink,
    C: Cancellation,
> {
    mq: MqDecoder<'a, S, C>,
    imported_source: &'a mut RI,
    new_source: &'a mut RN,
    new_sink: &'a mut W,
    imported: &'a [SymbolDescriptor],
    imported_base: u64,
    new_base: u64,
    layout: IaidLayout,
    header: DictionaryDataHeader,
    segment: u32,
    limits: &'a Limits,
    cancellation: &'a C,
    dictionary_budget: DictionaryBudget,
    refinement_budget: RefinementBudget,
    budget: RefinementDictionaryBudget,
    base_working: u64,
    progress: RefinementDictionaryProgress,
    refinement_observer: RefinementProgress,
    catalog: RefinementDictionaryCatalog,
    poisoned: bool,
    complete: bool,
}

impl<'a, S: RangedSource, RI: RangedSource, RN: RangedSource, W: SequentialSink, C: Cancellation>
    RefinementDictionaryDecoder<'a, S, RI, RN, W, C>
{
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        source: &'a mut S,
        segment: &SegmentHeader,
        imported_segment: &SegmentHeader,
        imported_report: &'a DictionaryReport,
        imported_source: &'a mut RI,
        imported_base: u64,
        new_source: &'a mut RN,
        new_sink: &'a mut W,
        new_base: u64,
        table: &'a MqTable,
        banks: &'a mut IaidContextBanks,
        limits: &'a Limits,
        cancellation: &'a C,
        mq_budget: MqBudget,
        dictionary_budget: DictionaryBudget,
        refinement_budget: RefinementBudget,
        budget: RefinementDictionaryBudget,
    ) -> RefinementDictionaryResult<Self> {
        let header =
            read_dictionary_data_header(source, segment, limits, dictionary_budget, cancellation)
                .await
                .map_err(|error| {
                    let offset = error.offset;
                    let progress = RefinementDictionaryProgress {
                        header_bytes_fetched: error.progress.header_bytes_fetched,
                        ..RefinementDictionaryProgress::default()
                    };
                    RefinementDictionaryError {
                        segment: segment.number,
                        offset,
                        progress: Box::new(progress),
                        kind: RefinementDictionaryErrorKind::Header(Box::new(error)),
                    }
                })?;
        let location = header.body.offset;
        let fetched = segment.header_length + header.header_bytes;
        let bad = |kind| before_mq(segment, location, fetched, kind);
        if header.mode != DictionaryMode::ArithmeticRefinementAggregate {
            return Err(bad(RefinementDictionaryErrorKind::Unsupported {
                feature: "dictionary coding mode",
                value: u64::from(header.flags),
            }));
        }
        if header.bitmap_context_used || header.bitmap_context_retained {
            return Err(bad(RefinementDictionaryErrorKind::Unsupported {
                feature: "bitmap context carry",
                value: u64::from(header.flags & 0x300),
            }));
        }
        if header.flags != 0x1802
            || header.template != 2
            || header.refinement_template != 1
            || header.at_count != 1
            || header.at[0] != (2, -1)
            || header.refinement_at_count != 0
        {
            return Err(bad(RefinementDictionaryErrorKind::Unsupported {
                feature: "second dictionary flags or adaptive template",
                value: u64::from(header.flags),
            }));
        }
        if segment.page_association != 1 {
            return Err(bad(RefinementDictionaryErrorKind::Unsupported {
                feature: "dictionary page association",
                value: u64::from(segment.page_association),
            }));
        }
        validate_imported(
            segment,
            imported_segment,
            imported_report,
            imported_source.size(),
            imported_base,
            budget,
            refinement_budget,
            fetched,
        )?;
        if new_base > new_source.size() {
            return Err(bad(RefinementDictionaryErrorKind::InvalidSpan(
                "new store base outside ranged source",
            )));
        }
        if refinement_budget.max_source_request_bytes == 0
            || refinement_budget.max_sink_request_bytes == 0
        {
            return Err(bad(RefinementDictionaryErrorKind::Malformed(
                "zero refinement I/O request bound",
            )));
        }
        // Imported and declared counts are each bounded by u32 header fields.
        let imported_count = imported_report.catalog.exported_symbols.len() as u64;
        let total = imported_count + u64::from(header.new_symbols);
        checked_cap(
            segment,
            location,
            fetched,
            "total symbols",
            u64::from(budget.max_total_symbols),
            total,
        )?;
        if u64::from(header.exported_symbols) > total {
            return Err(bad(RefinementDictionaryErrorKind::Malformed(
                "exported count exceeds available symbols",
            )));
        }
        let code_len = if total <= 1 {
            0
        } else {
            64 - (total - 1).leading_zeros()
        };
        let layout = banks.layout();
        if layout.code_len() != code_len
            || layout.total_contexts() != layout.bitmap_base() + GR_CONTEXTS
            || banks.mq_contexts_mut().count() != layout.total_contexts()
        {
            return Err(bad(RefinementDictionaryErrorKind::Malformed(
                "IAID width or GR context layout mismatch",
            )));
        }
        // Every descriptor count is a u32 header count, and the combined
        // descriptor byte total is far below u64::MAX even at those maxima.
        let imported_catalog_count =
            imported_count + imported_report.catalog.new_symbols.len() as u64;
        let imported_metadata = imported_catalog_count * mem::size_of::<SymbolDescriptor>() as u64;
        let new_metadata =
            u64::from(header.new_symbols) * mem::size_of::<SymbolDescriptor>() as u64;
        let export_metadata =
            u64::from(header.exported_symbols) * mem::size_of::<StoredSymbol>() as u64;
        let metadata = imported_metadata + new_metadata + export_metadata;
        checked_cap(
            segment,
            location,
            fetched,
            "catalog metadata bytes",
            budget
                .max_catalog_bytes
                .min(dictionary_budget.max_catalog_bytes),
            metadata,
        )?;
        checked_cap(
            segment,
            location,
            fetched,
            "catalog allocation bytes",
            limits.max_allocation_bytes,
            new_metadata + export_metadata,
        )?;
        // The caller's context bank is already allocated. Its byte count is
        // bounded by isize::MAX; the u32-limited metadata cannot overflow u64.
        let base_working = layout.total_contexts() as u64 * mem::size_of::<MqContext>() as u64
            + (MQ_STATE_COUNT * mem::size_of::<MqState>()) as u64
            + MQ_BUFFER_BYTES
            + metadata;
        checked_cap(
            segment,
            location,
            fetched,
            "dictionary working bytes",
            budget
                .max_working_bytes
                .min(dictionary_budget.max_working_bytes),
            base_working,
        )?;
        checked_cap(
            segment,
            location,
            fetched,
            "MQ source request bytes",
            dictionary_budget.max_source_request_bytes as u64,
            limits.io_chunk_bytes.min(MQ_BUFFER_BYTES as usize) as u64,
        )?;
        let mut new_symbols = Vec::new();
        new_symbols
            .try_reserve_exact(header.new_symbols as usize)
            .map_err(|_| bad(RefinementDictionaryErrorKind::AllocationFailed))?;
        let mut exported_symbols = Vec::new();
        exported_symbols
            .try_reserve_exact(header.exported_symbols as usize)
            .map_err(|_| bad(RefinementDictionaryErrorKind::AllocationFailed))?;
        banks
            .reset_for_symbol_dictionary()
            .map_err(|error| bad(RefinementDictionaryErrorKind::Mq(Box::new(error))))?;
        for index in layout.bitmap_base()..layout.total_contexts() {
            banks
                .mq_contexts_mut()
                .set(index, MqContext::default())
                .map_err(|error| bad(RefinementDictionaryErrorKind::Mq(Box::new(error))))?;
        }
        let mut initial_fetched = 0;
        let mq = MqDecoder::new_with_init_progress(
            source,
            MqSpan {
                offset: header.body.offset,
                length: header.body.length,
            },
            table,
            banks.mq_contexts_mut(),
            limits,
            cancellation,
            mq_budget,
            &mut initial_fetched,
        )
        .await
        .map_err(|error| {
            let offset = error.offset.unwrap_or(location);
            let mut located = bad(RefinementDictionaryErrorKind::Mq(Box::new(error)));
            located.offset = offset;
            located.progress.mq_initialization_bytes_fetched = initial_fetched;
            located
        })?;
        let progress = RefinementDictionaryProgress {
            header_bytes_fetched: fetched,
            mq: Some(mq.snapshot()),
            ..RefinementDictionaryProgress::default()
        };
        let refinement_budget = RefinementBudget {
            max_width: refinement_budget.max_width.min(dictionary_budget.max_width),
            max_height: refinement_budget
                .max_height
                .min(dictionary_budget.max_height),
            max_pixels_per_bitmap: refinement_budget
                .max_pixels_per_bitmap
                .min(dictionary_budget.max_pixels_per_symbol),
            max_total_pixels: refinement_budget
                .max_total_pixels
                .min(dictionary_budget.max_total_pixels),
            max_bytes_per_bitmap: refinement_budget
                .max_bytes_per_bitmap
                .min(dictionary_budget.max_bytes_per_symbol),
            max_total_output_bytes: refinement_budget
                .max_total_output_bytes
                .min(dictionary_budget.max_stored_bitmap_bytes),
            max_sink_writes: refinement_budget
                .max_sink_writes
                .min(dictionary_budget.max_sink_writes),
            max_source_request_bytes: refinement_budget
                .max_source_request_bytes
                .min(dictionary_budget.max_source_request_bytes),
            max_sink_request_bytes: refinement_budget
                .max_sink_request_bytes
                .min(dictionary_budget.max_sink_request_bytes),
            ..refinement_budget
        };
        Ok(Self {
            mq,
            imported_source,
            new_source,
            new_sink,
            imported: &imported_report.catalog.exported_symbols,
            imported_base,
            new_base,
            layout,
            header,
            segment: segment.number,
            limits,
            cancellation,
            dictionary_budget,
            refinement_budget,
            budget,
            base_working,
            progress,
            refinement_observer: RefinementProgress::default(),
            catalog: RefinementDictionaryCatalog {
                new_symbols,
                exported_symbols,
            },
            poisoned: false,
            complete: false,
        })
    }

    pub fn progress(&self) -> RefinementDictionaryProgress {
        let mut progress = self.progress;
        progress.refinement = self.refinement_observer;
        progress.mq = Some(self.mq.snapshot());
        progress.poisoned = self.poisoned || progress.mq.is_some_and(|mq| mq.poisoned);
        progress
    }

    /// Decode all new symbols, ordered exports, and the exact MQ terminal
    /// sequence. A failed or abandoned call poisons this decoder and requires
    /// the caller to discard the new store's appended bytes.
    pub async fn decode(&mut self) -> RefinementDictionaryResult<RefinementDictionaryReport> {
        if self.poisoned || self.complete {
            return Err(self.error(
                RefinementDictionaryErrorKind::Poisoned,
                self.mq.snapshot().current_input_offset,
            ));
        }
        self.poisoned = true;
        let segment_number = self.segment;
        let initial_offset = self.mq.snapshot().current_input_offset;
        let initial_progress = self.progress();
        let mut host = RefinementDecoder::new_observed(
            &mut self.mq,
            self.layout,
            self.new_sink,
            self.limits,
            self.cancellation,
            self.refinement_budget,
            Some(&mut self.refinement_observer),
        )
        .map_err(|error| RefinementDictionaryError {
            segment: segment_number,
            offset: error.offset.unwrap_or(initial_offset),
            progress: Box::new(initial_progress),
            kind: RefinementDictionaryErrorKind::Refinement(Box::new(error)),
        })?;
        let mut session = Session {
            segment: self.segment,
            header: self.header,
            imported: self.imported,
            imported_base: self.imported_base,
            new_base: self.new_base,
            layout: self.layout,
            limits: self.limits,
            cancellation: self.cancellation,
            dictionary_budget: self.dictionary_budget,
            budget: self.budget,
            base_working: self.base_working,
            progress: &mut self.progress,
            catalog: &mut self.catalog,
        };
        let decoded = session
            .decode_all(&mut host, self.imported_source, self.new_source)
            .await;
        drop(host);
        decoded?;
        self.progress.refinement = self.refinement_observer;
        let expected = self.mq.snapshot().symbols_decoded;
        let snapshot = self
            .mq
            .finish_with_snapshot_mut(expected)
            .await
            .map_err(|error| {
                let offset = error
                    .offset
                    .unwrap_or(self.mq.snapshot().current_input_offset);
                self.error(RefinementDictionaryErrorKind::Mq(Box::new(error)), offset)
            })?;
        self.progress.mq = Some(snapshot);
        if self.cancellation.is_cancelled() {
            return Err(self.error(
                RefinementDictionaryErrorKind::Cancelled,
                snapshot.current_input_offset,
            ));
        }
        self.new_sink.flush().await.map_err(|error| {
            self.error(
                if matches!(error, Error::Cancelled) {
                    RefinementDictionaryErrorKind::Cancelled
                } else {
                    RefinementDictionaryErrorKind::Sink(error)
                },
                snapshot.current_input_offset,
            )
        })?;
        if self.cancellation.is_cancelled() {
            return Err(self.error(
                RefinementDictionaryErrorKind::Cancelled,
                snapshot.current_input_offset,
            ));
        }
        self.complete = true;
        self.poisoned = false;
        let catalog = mem::replace(
            &mut self.catalog,
            RefinementDictionaryCatalog {
                new_symbols: Vec::new(),
                exported_symbols: Vec::new(),
            },
        );
        Ok(RefinementDictionaryReport {
            header: self.header,
            catalog,
            progress: self.progress(),
        })
    }

    fn error(&self, kind: RefinementDictionaryErrorKind, offset: u64) -> RefinementDictionaryError {
        RefinementDictionaryError {
            segment: self.segment,
            offset,
            progress: Box::new(self.progress()),
            kind,
        }
    }
}

struct Session<'a, C: Cancellation> {
    segment: u32,
    header: DictionaryDataHeader,
    imported: &'a [SymbolDescriptor],
    imported_base: u64,
    new_base: u64,
    layout: IaidLayout,
    limits: &'a Limits,
    cancellation: &'a C,
    dictionary_budget: DictionaryBudget,
    budget: RefinementDictionaryBudget,
    base_working: u64,
    progress: &'a mut RefinementDictionaryProgress,
    catalog: &'a mut RefinementDictionaryCatalog,
}

impl<C: Cancellation> Session<'_, C> {
    fn snapshot<M: RangedSource, W: SequentialSink>(
        &self,
        host: &RefinementDecoder<'_, '_, M, C, W>,
    ) -> RefinementDictionaryProgress {
        let mut progress = *self.progress;
        progress.refinement = host.progress();
        progress.mq = progress.refinement.mq;
        progress.poisoned = true;
        progress
    }

    fn offset<M: RangedSource, W: SequentialSink>(
        &self,
        host: &RefinementDecoder<'_, '_, M, C, W>,
    ) -> u64 {
        host.progress()
            .mq
            .map_or(self.header.body.offset, |snapshot| {
                snapshot.current_input_offset
            })
    }

    fn error<M: RangedSource, W: SequentialSink>(
        &self,
        host: &RefinementDecoder<'_, '_, M, C, W>,
        kind: RefinementDictionaryErrorKind,
    ) -> RefinementDictionaryError {
        RefinementDictionaryError {
            segment: self.segment,
            offset: self.offset(host),
            progress: Box::new(self.snapshot(host)),
            kind,
        }
    }

    fn cap<M: RangedSource, W: SequentialSink>(
        &self,
        host: &RefinementDecoder<'_, '_, M, C, W>,
        resource: &'static str,
        maximum: u64,
        attempted: u64,
    ) -> RefinementDictionaryResult<()> {
        if attempted > maximum {
            Err(self.error(
                host,
                RefinementDictionaryErrorKind::LimitExceeded {
                    resource,
                    limit: maximum,
                    attempted,
                },
            ))
        } else {
            Ok(())
        }
    }

    fn check_cancelled<M: RangedSource, W: SequentialSink>(
        &self,
        host: &RefinementDecoder<'_, '_, M, C, W>,
    ) -> RefinementDictionaryResult<()> {
        if self.cancellation.is_cancelled() {
            Err(self.error(host, RefinementDictionaryErrorKind::Cancelled))
        } else {
            Ok(())
        }
    }

    async fn integer<M: RangedSource, W: SequentialSink>(
        &self,
        host: &mut RefinementDecoder<'_, '_, M, C, W>,
        procedure: IntegerProcedure,
    ) -> RefinementDictionaryResult<IntegerValue> {
        let mq = match host.mq_mut() {
            Ok(mq) => mq,
            Err(error) => {
                return Err(self.error(
                    host,
                    RefinementDictionaryErrorKind::Refinement(Box::new(error)),
                ));
            }
        };
        let result = decode_integer(mq, procedure).await;
        result.map_err(|error| {
            let offset = error.offset;
            let mut located = self.error(host, RefinementDictionaryErrorKind::Mq(Box::new(error)));
            located.offset = offset.unwrap_or(located.offset);
            located
        })
    }

    async fn iaid<M: RangedSource, W: SequentialSink>(
        &self,
        host: &mut RefinementDecoder<'_, '_, M, C, W>,
    ) -> RefinementDictionaryResult<u64> {
        let mq = match host.mq_mut() {
            Ok(mq) => mq,
            Err(error) => {
                return Err(self.error(
                    host,
                    RefinementDictionaryErrorKind::Refinement(Box::new(error)),
                ));
            }
        };
        let result = decode_iaid(mq, self.layout).await;
        result.map_err(|error| {
            let offset = error.offset;
            let mut located = self.error(host, RefinementDictionaryErrorKind::Mq(Box::new(error)));
            located.offset = offset.unwrap_or(located.offset);
            located
        })
    }

    fn signed<M: RangedSource, W: SequentialSink>(
        &self,
        host: &RefinementDecoder<'_, '_, M, C, W>,
        value: IntegerValue,
        field: &'static str,
    ) -> RefinementDictionaryResult<i64> {
        match value {
            IntegerValue::Signed(value) => Ok(value),
            IntegerValue::OutOfBand => {
                Err(self.error(host, RefinementDictionaryErrorKind::Malformed(field)))
            }
        }
    }

    fn geometry<M: RangedSource, W: SequentialSink>(
        &self,
        host: &RefinementDecoder<'_, '_, M, C, W>,
        width: i64,
        height: i64,
    ) -> RefinementDictionaryResult<(u32, u32, u64, u64)> {
        if width < 0 || height < 0 {
            return Err(self.error(
                host,
                RefinementDictionaryErrorKind::Malformed("negative symbol dimension"),
            ));
        }
        if width == 0 || height == 0 {
            return Err(self.error(
                host,
                RefinementDictionaryErrorKind::Unsupported {
                    feature: "zero-dimension symbol bitmap",
                    value: 0,
                },
            ));
        }
        let width = u32::try_from(width).map_err(|_| {
            self.error(
                host,
                RefinementDictionaryErrorKind::Malformed("symbol width exceeds 32 bits"),
            )
        })?;
        let height = u32::try_from(height).map_err(|_| {
            self.error(
                host,
                RefinementDictionaryErrorKind::Malformed("symbol height exceeds 32 bits"),
            )
        })?;
        self.cap(
            host,
            "symbol width",
            u64::from(self.dictionary_budget.max_width),
            u64::from(width),
        )?;
        self.cap(
            host,
            "symbol height",
            u64::from(self.dictionary_budget.max_height),
            u64::from(height),
        )?;
        let pixels = u64::from(width) * u64::from(height);
        let bytes = u64::from(width).div_ceil(8) * u64::from(height);
        self.cap(
            host,
            "symbol pixels",
            self.dictionary_budget.max_pixels_per_symbol,
            pixels,
        )?;
        self.cap(
            host,
            "symbol bytes",
            self.dictionary_budget.max_bytes_per_symbol,
            bytes,
        )?;
        let future_pixels = host
            .progress()
            .pixels_decoded
            .checked_add(pixels)
            .ok_or_else(|| {
                self.error(
                    host,
                    RefinementDictionaryErrorKind::InvalidSpan("total pixel count overflow"),
                )
            })?;
        let future_bytes = host
            .progress()
            .output_bytes_written
            .checked_add(bytes)
            .ok_or_else(|| {
                self.error(
                    host,
                    RefinementDictionaryErrorKind::InvalidSpan("stored byte count overflow"),
                )
            })?;
        self.cap(
            host,
            "dictionary pixels",
            self.dictionary_budget.max_total_pixels,
            future_pixels,
        )?;
        self.cap(
            host,
            "stored bitmap bytes",
            self.dictionary_budget.max_stored_bitmap_bytes,
            future_bytes,
        )?;
        self.cap(
            host,
            "output bytes",
            self.limits.max_output_bytes,
            future_bytes,
        )?;
        Ok((width, height, pixels, bytes))
    }

    fn reference<M: RangedSource, W: SequentialSink>(
        &self,
        host: &RefinementDecoder<'_, '_, M, C, W>,
        raw_id: u64,
    ) -> RefinementDictionaryResult<StoredSymbol> {
        let active = self.imported.len() + self.catalog.new_symbols.len();
        let index = checked_symbol_index(raw_id, active as u64, active).map_err(|_| {
            self.error(
                host,
                RefinementDictionaryErrorKind::Malformed("future, self, or absent symbol ID"),
            )
        })?;
        if index < self.imported.len() {
            Ok(StoredSymbol {
                store: SymbolStore::Imported,
                store_base: self.imported_base,
                symbol: self.imported[index],
            })
        } else {
            Ok(StoredSymbol {
                store: SymbolStore::New,
                store_base: self.new_base,
                symbol: self.catalog.new_symbols[index - self.imported.len()],
            })
        }
    }

    fn working<M: RangedSource, W: SequentialSink>(
        &self,
        host: &RefinementDecoder<'_, '_, M, C, W>,
        target_width: u32,
        reference_width: u32,
    ) -> RefinementDictionaryResult<()> {
        let rows =
            2 * u64::from(target_width).div_ceil(8) + 3 * u64::from(reference_width).div_ceil(8);
        // The allocated context bank is below isize::MAX bytes, catalog
        // counts are u32-bounded, and five packed rows add at most 2.7 GiB.
        let working = self.base_working + rows;
        self.cap(
            host,
            "dictionary working bytes",
            self.budget
                .max_working_bytes
                .min(self.dictionary_budget.max_working_bytes),
            working,
        )
    }

    async fn decode_all<M: RangedSource, RI: RangedSource, RN: RangedSource, W: SequentialSink>(
        &mut self,
        host: &mut RefinementDecoder<'_, '_, M, C, W>,
        imported_source: &mut RI,
        new_source: &mut RN,
    ) -> RefinementDictionaryResult<()> {
        self.decode_symbols(host, imported_source, new_source)
            .await?;
        self.decode_exports(host).await?;
        self.check_cancelled(host)
    }

    async fn decode_symbols<
        M: RangedSource,
        RI: RangedSource,
        RN: RangedSource,
        W: SequentialSink,
    >(
        &mut self,
        host: &mut RefinementDecoder<'_, '_, M, C, W>,
        imported_source: &mut RI,
        new_source: &mut RN,
    ) -> RefinementDictionaryResult<()> {
        let mut class_height = 0i64;
        while self.progress.completed_symbols < self.header.new_symbols {
            self.check_cancelled(host)?;
            let classes = self.progress.height_classes.checked_add(1).ok_or_else(|| {
                self.error(
                    host,
                    RefinementDictionaryErrorKind::InvalidSpan("height class count overflow"),
                )
            })?;
            self.cap(
                host,
                "height classes",
                u64::from(self.dictionary_budget.max_height_classes),
                u64::from(classes),
            )?;
            self.progress.height_classes = classes;
            let delta = self.integer(host, IntegerProcedure::Iadh).await?;
            class_height = class_height
                .checked_add(self.signed(host, delta, "IADH out of band")?)
                .ok_or_else(|| {
                    self.error(
                        host,
                        RefinementDictionaryErrorKind::Malformed("height class overflow"),
                    )
                })?;
            if class_height < 0 || class_height > i64::from(u32::MAX) {
                return Err(self.error(
                    host,
                    RefinementDictionaryErrorKind::Malformed("height class dimension"),
                ));
            }
            self.cap(
                host,
                "height class",
                u64::from(self.dictionary_budget.max_height),
                class_height as u64,
            )?;
            let mut class_width = 0i64;
            loop {
                let value = self.integer(host, IntegerProcedure::Iadw).await?;
                let delta = match value {
                    IntegerValue::OutOfBand => break,
                    IntegerValue::Signed(value) => value,
                };
                if self.progress.completed_symbols == self.header.new_symbols {
                    return Err(self.error(
                        host,
                        RefinementDictionaryErrorKind::Malformed(
                            "symbol-count overrun before width OOB",
                        ),
                    ));
                }
                class_width = class_width.checked_add(delta).ok_or_else(|| {
                    self.error(
                        host,
                        RefinementDictionaryErrorKind::Malformed("symbol width overflow"),
                    )
                })?;
                let (width, height, _, _) = self.geometry(host, class_width, class_height)?;
                let instances = self.integer(host, IntegerProcedure::Iaai).await?;
                let instances = self.signed(host, instances, "REFAGGNINST OOB")?;
                if instances == 0 {
                    self.progress.iaai.zero += 1;
                    return Err(self.error(
                        host,
                        RefinementDictionaryErrorKind::Malformed("REFAGGNINST zero"),
                    ));
                }
                if instances < 0 {
                    return Err(self.error(
                        host,
                        RefinementDictionaryErrorKind::Malformed("REFAGGNINST negative"),
                    ));
                }
                if instances > 1 {
                    self.progress.iaai.aggregation += 1;
                    return Err(self.error(
                        host,
                        RefinementDictionaryErrorKind::Unsupported {
                            feature: "REFAGGNINST aggregation",
                            value: instances as u64,
                        },
                    ));
                }
                self.progress.iaai.single_reference += 1;
                let id = self.iaid(host).await?;
                let reference = self.reference(host, id)?;
                self.working(host, width, reference.symbol.width)?;
                let dx = self.integer(host, IntegerProcedure::Iardx).await?;
                let dy = self.integer(host, IntegerProcedure::Iardy).await?;
                let dx = self.signed(host, dx, "IARDX out of band")?;
                let dy = self.signed(host, dy, "IARDY out of band")?;
                let dx = i32::try_from(dx).map_err(|_| {
                    self.error(
                        host,
                        RefinementDictionaryErrorKind::Malformed(
                            "IARDX outside signed 32-bit range",
                        ),
                    )
                })?;
                let dy = i32::try_from(dy).map_err(|_| {
                    self.error(
                        host,
                        RefinementDictionaryErrorKind::Malformed(
                            "IARDY outside signed 32-bit range",
                        ),
                    )
                })?;
                let request = RefinementRequest {
                    width,
                    height,
                    template: 1,
                    typical_prediction: false,
                    reference_dx: dx,
                    reference_dy: dy,
                    reference: RefinementReference {
                        store_base: reference.store_base,
                        symbol: reference.symbol,
                    },
                };
                let result = match reference.store {
                    SymbolStore::Imported => host.decode_bitmap(imported_source, request).await,
                    SymbolStore::New => {
                        host.flush_store().await.map_err(|error| {
                            self.error(
                                host,
                                RefinementDictionaryErrorKind::Refinement(Box::new(error)),
                            )
                        })?;
                        host.decode_bitmap(new_source, request).await
                    }
                };
                let descriptor = result
                    .map_err(|error| {
                        let offset = error.offset;
                        let mut located = self.error(
                            host,
                            RefinementDictionaryErrorKind::Refinement(Box::new(error)),
                        );
                        located.offset = offset.unwrap_or(located.offset);
                        located
                    })?
                    .target;
                self.catalog.new_symbols.push(descriptor);
                self.progress.completed_symbols += 1;
            }
        }
        Ok(())
    }

    async fn decode_exports<M: RangedSource, W: SequentialSink>(
        &mut self,
        host: &mut RefinementDecoder<'_, '_, M, C, W>,
    ) -> RefinementDictionaryResult<()> {
        let total = self.imported.len() + self.catalog.new_symbols.len();
        let mut index = 0usize;
        let mut export = false;
        loop {
            self.check_cancelled(host)?;
            let runs = self.progress.export_runs.checked_add(1).ok_or_else(|| {
                self.error(
                    host,
                    RefinementDictionaryErrorKind::InvalidSpan("export run count overflow"),
                )
            })?;
            self.cap(
                host,
                "export runs",
                u64::from(self.dictionary_budget.max_export_runs),
                u64::from(runs),
            )?;
            self.progress.export_runs = runs;
            let value = self.integer(host, IntegerProcedure::Iaex).await?;
            let length = self.signed(host, value, "IAEX out of band")?;
            if length < 0 {
                return Err(self.error(
                    host,
                    RefinementDictionaryErrorKind::Malformed("negative export run"),
                ));
            }
            // `index` is capped by max_total_symbols (u32); nonnegative IAEX
            // is at most i64::MAX, so this sum cannot overflow u64.
            let end = index as u64 + length as u64;
            if end > total as u64 {
                return Err(self.error(
                    host,
                    RefinementDictionaryErrorKind::Malformed("export run overshoot"),
                ));
            }
            let end = end as usize;
            if export {
                let next = self.catalog.exported_symbols.len() + (end - index);
                if next > self.header.exported_symbols as usize {
                    return Err(self.error(
                        host,
                        RefinementDictionaryErrorKind::Malformed("exported symbol total"),
                    ));
                }
                for id in index..end {
                    let symbol = if id < self.imported.len() {
                        StoredSymbol {
                            store: SymbolStore::Imported,
                            store_base: self.imported_base,
                            symbol: self.imported[id],
                        }
                    } else {
                        StoredSymbol {
                            store: SymbolStore::New,
                            store_base: self.new_base,
                            symbol: self.catalog.new_symbols[id - self.imported.len()],
                        }
                    };
                    self.catalog.exported_symbols.push(symbol);
                }
            }
            index = end;
            export = !export;
            if index == total {
                break;
            }
        }
        if self.catalog.exported_symbols.len() != self.header.exported_symbols as usize {
            return Err(self.error(
                host,
                RefinementDictionaryErrorKind::Malformed("exported symbol total"),
            ));
        }
        Ok(())
    }
}
