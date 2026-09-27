// SPDX-License-Identifier: MIT

//! Pull decoding of bounded arithmetic text-region instances (T.88 §6.4.5).
//!
//! The caller owns both dictionary stores and the temporary refinement store.
//! This module emits checked placements; it does not allocate or compose a
//! region bitmap. It contains no normative MQ probability states.

use super::{
    SegmentHeader,
    dictionary::{DictionaryMode, SymbolDescriptor},
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
    refinement_dictionary::{RefinementDictionaryReport, StoredSymbol, SymbolStore},
    text::{
        ReferenceCorner, TextRegionBudget, TextRegionError, TextRegionHeader,
        read_text_region_header,
    },
};
use crate::{Cancellation, Limits, MAX_BUDGET_COUNT, RangedSource, SequentialSink};
use std::{error, fmt, mem};

const GR_CONTEXTS: usize = 1024;
const MQ_BUFFER_BYTES: u64 = 256;

/// Additional bounds for one text-region instance stream. The MQ and generic
/// refinement budgets separately cap arithmetic work, row I/O, and writes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextInstanceBudget {
    pub max_exported_symbols: u32,
    pub max_instances: u32,
    pub max_strips: u32,
    pub max_coordinate_magnitude: i64,
    pub max_pixels_per_instance: u64,
    pub max_total_instance_pixels: u64,
    pub max_imported_store_span: u64,
    pub max_new_store_span: u64,
    pub max_temporary_store_bytes: u64,
    pub max_metadata_bytes: u64,
    pub max_working_bytes: u64,
}

impl Default for TextInstanceBudget {
    fn default() -> Self {
        Self {
            max_exported_symbols: 8192,
            max_instances: 1_000_000,
            max_strips: 1_000_000,
            max_coordinate_magnitude: 1_000_000_000,
            max_pixels_per_instance: 12_000_000,
            max_total_instance_pixels: 1_000_000_000,
            max_imported_store_span: 128 * 1024 * 1024,
            max_new_store_span: 128 * 1024 * 1024,
            max_temporary_store_bytes: 128 * 1024 * 1024,
            max_metadata_bytes: 1024 * 1024,
            max_working_bytes: 16 * 1024 * 1024,
        }
    }
}

/// A bitmap handle retains the store identity. A refined instance is not a
/// dictionary symbol and can never be used as an IAID reference.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextBitmap {
    Stored(StoredSymbol),
    Refined {
        store_base: u64,
        symbol: SymbolDescriptor,
    },
}

/// One checked top-left placement in region-local coordinates. Negative and
/// wholly off-region placements are valid; the later composer clips pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextInstance {
    pub index: u32,
    pub strip: u32,
    pub symbol_id: u32,
    pub ri: bool,
    pub x: i64,
    pub y: i64,
    pub width: u32,
    pub height: u32,
    pub bitmap: TextBitmap,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TextDecision {
    #[default]
    InitialStripT,
    StripDeltaT,
    FirstS,
    DeltaS,
    WithinStripT,
    SymbolId,
    RefinementFlag,
    DeltaWidth,
    DeltaHeight,
    DeltaX,
    DeltaY,
    RefinementBitmap,
    Terminal,
    Complete,
}

/// Physical MQ/refinement counters and the next semantic decision. A dropped
/// pending `next` leaves `poisoned` set and requires discarding temporary data.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextInstanceProgress {
    pub completed_instances: u32,
    pub ri_zero: u32,
    pub ri_one: u32,
    pub strips: u32,
    pub total_instance_pixels: u64,
    pub decision: TextDecision,
    pub header_bytes_fetched: u64,
    /// MQ bytes fetched only if initialization failed before a snapshot existed.
    /// Zero after construction because `mq` already includes the prefetch.
    pub mq_initialization_bytes_fetched: u64,
    pub refinement: RefinementProgress,
    pub mq: Option<MqSnapshot>,
    pub poisoned: bool,
}

impl TextInstanceProgress {
    pub fn source_bytes_fetched(self) -> u64 {
        self.header_bytes_fetched
            .saturating_add(self.mq_initialization_bytes_fetched)
            .saturating_add(self.mq.map_or(0, |mq| mq.source_bytes_fetched))
    }
}

#[derive(Debug)]
pub struct TextInstanceError {
    pub segment: u32,
    pub offset: u64,
    pub progress: Box<TextInstanceProgress>,
    pub kind: TextInstanceErrorKind,
}

#[derive(Debug)]
pub enum TextInstanceErrorKind {
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
    Cancelled,
    Header(Box<TextRegionError>),
    Mq(Box<MqError>),
    Refinement(Box<RefinementError>),
    Poisoned,
}

pub type TextInstanceResult<T> = Result<T, TextInstanceError>;

impl fmt::Display for TextInstanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "JBIG2 text instance segment {} at source byte {} (instance {}, {:?}): ",
            self.segment, self.offset, self.progress.completed_instances, self.progress.decision
        )?;
        match &self.kind {
            TextInstanceErrorKind::InvalidSpan(value) => write!(f, "invalid span: {value}"),
            TextInstanceErrorKind::Malformed(value) => write!(f, "malformed {value}"),
            TextInstanceErrorKind::Unsupported { feature, value } => {
                write!(f, "unsupported {feature} ({value})")
            }
            TextInstanceErrorKind::LimitExceeded {
                resource,
                limit,
                attempted,
            } => write!(f, "{resource} limit {limit} exceeded by {attempted}"),
            TextInstanceErrorKind::Cancelled => f.write_str("cancelled"),
            TextInstanceErrorKind::Header(value) => write!(f, "header: {value}"),
            TextInstanceErrorKind::Mq(value) => write!(f, "MQ: {value}"),
            TextInstanceErrorKind::Refinement(value) => write!(f, "refinement: {value}"),
            TextInstanceErrorKind::Poisoned => f.write_str("decoder is poisoned or complete"),
        }
    }
}

impl error::Error for TextInstanceError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.kind {
            TextInstanceErrorKind::Header(value) => Some(value),
            TextInstanceErrorKind::Mq(value) => Some(value),
            TextInstanceErrorKind::Refinement(value) => Some(value),
            _ => None,
        }
    }
}

fn preflight_error(
    segment: u32,
    offset: u64,
    fetched: u64,
    kind: TextInstanceErrorKind,
) -> TextInstanceError {
    TextInstanceError {
        segment,
        offset,
        progress: Box::new(TextInstanceProgress {
            header_bytes_fetched: fetched,
            ..TextInstanceProgress::default()
        }),
        kind,
    }
}

fn preflight_cap(
    segment: u32,
    offset: u64,
    fetched: u64,
    resource: &'static str,
    limit: u64,
    attempted: u64,
) -> TextInstanceResult<()> {
    if attempted > limit {
        Err(preflight_error(
            segment,
            offset,
            fetched,
            TextInstanceErrorKind::LimitExceeded {
                resource,
                limit,
                attempted,
            },
        ))
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct PreflightSite {
    segment: u32,
    offset: u64,
    fetched: u64,
}

impl PreflightSite {
    fn error(self, kind: TextInstanceErrorKind) -> TextInstanceError {
        preflight_error(self.segment, self.offset, self.fetched, kind)
    }

    fn cap(self, resource: &'static str, limit: u64, attempted: u64) -> TextInstanceResult<()> {
        preflight_cap(
            self.segment,
            self.offset,
            self.fetched,
            resource,
            limit,
            attempted,
        )
    }
}

fn validate_descriptor(
    site: PreflightSite,
    stored: StoredSymbol,
    expected_store: SymbolStore,
    expected_base: u64,
    source_size: u64,
    max_span: u64,
) -> TextInstanceResult<()> {
    let bad = |reason| site.error(TextInstanceErrorKind::Malformed(reason));
    if stored.store != expected_store || stored.store_base != expected_base {
        return Err(bad("dictionary store identity or base"));
    }
    let descriptor = stored.symbol;
    if descriptor.width == 0 || descriptor.height == 0 {
        return Err(bad("zero dictionary symbol dimension"));
    }
    let stride = u64::from(descriptor.width).div_ceil(8);
    let bytes = stride * u64::from(descriptor.height);
    if u64::from(descriptor.row_stride) != stride || descriptor.stored_bytes != bytes {
        return Err(bad("noncanonical dictionary symbol descriptor"));
    }
    let relative_end = descriptor
        .relative_store_offset
        .checked_add(bytes)
        .ok_or_else(|| bad("dictionary descriptor end overflow"))?;
    site.cap("dictionary store span", max_span, relative_end)?;
    let absolute_end = expected_base
        .checked_add(relative_end)
        .ok_or_else(|| bad("dictionary absolute store end overflow"))?;
    if absolute_end > source_size {
        return Err(bad("dictionary bitmap outside ranged store"));
    }
    Ok(())
}

fn cap_coordinate(value: i64, magnitude: i64) -> Result<i64, TextInstanceErrorKind> {
    if value.unsigned_abs() > magnitude as u64 {
        Err(TextInstanceErrorKind::LimitExceeded {
            resource: "signed text coordinate magnitude",
            limit: magnitude as u64,
            attempted: value.unsigned_abs(),
        })
    } else {
        Ok(value)
    }
}

fn checked_coordinate(value: Option<i64>, magnitude: i64) -> Result<i64, TextInstanceErrorKind> {
    cap_coordinate(
        value.ok_or(TextInstanceErrorKind::Malformed("coordinate overflow"))?,
        magnitude,
    )
}

fn geometry(
    s: i64,
    t: i64,
    width: u32,
    height: u32,
    corner: ReferenceCorner,
    transposed: bool,
    magnitude: i64,
) -> Result<(i64, i64, i64), TextInstanceErrorKind> {
    let right = matches!(
        corner,
        ReferenceCorner::TopRight | ReferenceCorner::BottomRight
    );
    let bottom = matches!(
        corner,
        ReferenceCorner::BottomLeft | ReferenceCorner::BottomRight
    );
    let extent = i64::from(if transposed { height } else { width }) - 1;
    let pre = if if transposed { bottom } else { right } {
        extent
    } else {
        0
    };
    let post = if if transposed { !bottom } else { !right } {
        extent
    } else {
        0
    };
    let s = checked_coordinate(s.checked_add(pre), magnitude)?;
    let (mut x, mut y) = if transposed { (t, s) } else { (s, t) };
    if right {
        x = checked_coordinate(x.checked_sub(i64::from(width) - 1), magnitude)?;
    }
    if bottom {
        y = checked_coordinate(y.checked_sub(i64::from(height) - 1), magnitude)?;
    }
    let next_s = checked_coordinate(s.checked_add(post), magnitude)?;
    Ok((
        cap_coordinate(x, magnitude)?,
        cap_coordinate(y, magnitude)?,
        next_s,
    ))
}

fn refined_geometry(
    reference: SymbolDescriptor,
    rdw: i64,
    rdh: i64,
    rdx: i64,
    rdy: i64,
) -> Result<(u32, u32, i32, i32), TextInstanceErrorKind> {
    let width = i64::from(reference.width)
        .checked_add(rdw)
        .ok_or(TextInstanceErrorKind::Malformed("refined width overflow"))?;
    let height = i64::from(reference.height)
        .checked_add(rdh)
        .ok_or(TextInstanceErrorKind::Malformed("refined height overflow"))?;
    if width <= 0 || height <= 0 || width > i64::from(u32::MAX) || height > i64::from(u32::MAX) {
        return Err(TextInstanceErrorKind::Malformed(
            "nonpositive or oversized refined geometry",
        ));
    }
    // T.88 Table 12 uses mathematical floor, including negative odd deltas.
    let dx = rdw
        .div_euclid(2)
        .checked_add(rdx)
        .and_then(|value| i32::try_from(value).ok())
        .ok_or(TextInstanceErrorKind::Malformed(
            "refinement X offset overflow",
        ))?;
    let dy = rdh
        .div_euclid(2)
        .checked_add(rdy)
        .and_then(|value| i32::try_from(value).ok())
        .ok_or(TextInstanceErrorKind::Malformed(
            "refinement Y offset overflow",
        ))?;
    Ok((width as u32, height as u32, dx, dy))
}

/// One MQ coding unit and sequential pull cursor. `next` must be called until
/// it returns `None` to check the terminal pair. On any failure or abandoned
/// pending call, discard all temporary bitmap output for this region.
pub struct TextInstanceDecoder<
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
    temporary_sink: &'a mut W,
    temporary_store_base: u64,
    dictionary: &'a [StoredSymbol],
    header: TextRegionHeader,
    segment: u32,
    layout: IaidLayout,
    limits: &'a Limits,
    cancellation: &'a C,
    refinement_budget: RefinementBudget,
    budget: TextInstanceBudget,
    progress: TextInstanceProgress,
    strip_t: i64,
    first_s: i64,
    current_s: i64,
    initialized: bool,
    strip_open: bool,
    poisoned: bool,
    complete: bool,
}

impl<S: RangedSource, RI: RangedSource, RN: RangedSource, W: SequentialSink, C: Cancellation> Drop
    for TextInstanceDecoder<'_, S, RI, RN, W, C>
{
    fn drop(&mut self) {
        if !self.complete {
            self.mq.poison();
        }
    }
}

impl<'a, S: RangedSource, RI: RangedSource, RN: RangedSource, W: SequentialSink, C: Cancellation>
    TextInstanceDecoder<'a, S, RI, RN, W, C>
{
    /// Reparse the segment header and validate both stores before MQ input.
    /// The supplied temporary sink must append to the store represented by
    /// `temporary_store_base`; that identity is owned by the caller.
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        source: &'a mut S,
        segment: &SegmentHeader,
        parsed: TextRegionHeader,
        dictionary_segment: &SegmentHeader,
        dictionary: &'a RefinementDictionaryReport,
        imported_source: &'a mut RI,
        imported_store_base: u64,
        new_source: &'a mut RN,
        new_store_base: u64,
        temporary_sink: &'a mut W,
        temporary_store_base: u64,
        table: &'a MqTable,
        banks: &'a mut IaidContextBanks,
        limits: &'a Limits,
        cancellation: &'a C,
        mq_budget: MqBudget,
        header_budget: TextRegionBudget,
        refinement_budget: RefinementBudget,
        budget: TextInstanceBudget,
    ) -> TextInstanceResult<Self> {
        let checked = read_text_region_header(
            source,
            segment,
            dictionary_segment,
            limits,
            header_budget,
            cancellation,
        )
        .await
        .map_err(|error| {
            let offset = error.offset;
            let fetched = error.bytes_fetched;
            preflight_error(
                segment.number,
                offset,
                fetched,
                TextInstanceErrorKind::Header(Box::new(error)),
            )
        })?;
        let at = checked.body.offset;
        let fetched = checked.header_bytes;
        let site = PreflightSite {
            segment: segment.number,
            offset: at,
            fetched,
        };
        let bad = |kind| site.error(kind);
        if checked != parsed {
            return Err(bad(TextInstanceErrorKind::Malformed(
                "supplied text header differs from source",
            )));
        }
        if parsed.flags.huffman || parsed.huffman_flags.is_some() {
            return Err(bad(TextInstanceErrorKind::Unsupported {
                feature: "Huffman text region",
                value: u64::from(parsed.flags.raw),
            }));
        }
        if parsed.flags.refine
            && (parsed.flags.refinement_template != 1 || parsed.refinement_at.is_some())
        {
            return Err(bad(TextInstanceErrorKind::Unsupported {
                feature: "refinement template 0",
                value: u64::from(parsed.flags.raw),
            }));
        }
        // `read_text_region_header` has just rechecked the exact segment
        // type, reference, order, and page relation against these arguments.
        let dictionary_header = dictionary.header;
        let dictionary_end = dictionary_segment
            .data
            .offset
            .checked_add(dictionary_segment.data.length)
            .ok_or_else(|| {
                bad(TextInstanceErrorKind::InvalidSpan(
                    "dictionary segment end overflow",
                ))
            })?;
        let body_end = dictionary_header
            .body
            .offset
            .checked_add(dictionary_header.body.length)
            .ok_or_else(|| {
                bad(TextInstanceErrorKind::InvalidSpan(
                    "dictionary body end overflow",
                ))
            })?;
        let expected_body = dictionary_segment
            .data
            .offset
            .checked_add(dictionary_header.header_bytes)
            .ok_or_else(|| {
                bad(TextInstanceErrorKind::InvalidSpan(
                    "dictionary body start overflow",
                ))
            })?;
        if dictionary_header.mode != DictionaryMode::ArithmeticRefinementAggregate
            || dictionary_header.flags != 0x1802
            || dictionary_header.body.offset != expected_body
            || body_end != dictionary_end
            || dictionary_header.new_symbols as usize != dictionary.catalog.new_symbols.len()
            || dictionary_header.exported_symbols as usize
                != dictionary.catalog.exported_symbols.len()
            || dictionary.progress.completed_symbols != dictionary_header.new_symbols
            || dictionary.progress.poisoned
            || dictionary.progress.mq.is_none_or(|mq| mq.poisoned)
        {
            return Err(bad(TextInstanceErrorKind::Malformed(
                "dictionary is not a complete ordered report",
            )));
        }
        if budget.max_coordinate_magnitude < 0 {
            return Err(bad(TextInstanceErrorKind::Malformed(
                "negative coordinate cap",
            )));
        }
        for (name, value) in [
            ("instance pixels budget", budget.max_pixels_per_instance),
            (
                "total instance pixels budget",
                budget.max_total_instance_pixels,
            ),
            ("temporary store budget", budget.max_temporary_store_bytes),
        ] {
            preflight_cap(segment.number, at, fetched, name, MAX_BUDGET_COUNT, value)?;
        }
        preflight_cap(
            segment.number,
            at,
            fetched,
            "exported symbols",
            u64::from(budget.max_exported_symbols),
            dictionary.catalog.exported_symbols.len() as u64,
        )?;
        preflight_cap(
            segment.number,
            at,
            fetched,
            "instances",
            u64::from(budget.max_instances),
            u64::from(parsed.instances),
        )?;
        if parsed.instances != 0 && dictionary.catalog.exported_symbols.is_empty() {
            return Err(bad(TextInstanceErrorKind::Malformed(
                "nonempty text region with no symbols",
            )));
        }
        if temporary_store_base
            .checked_add(budget.max_temporary_store_bytes)
            .is_none()
        {
            return Err(bad(TextInstanceErrorKind::InvalidSpan(
                "temporary store range overflow",
            )));
        }
        if imported_store_base > imported_source.size() || new_store_base > new_source.size() {
            return Err(bad(TextInstanceErrorKind::InvalidSpan(
                "dictionary store base outside ranged source",
            )));
        }
        let mut last_catalog_end = 0u64;
        for descriptor in &dictionary.catalog.new_symbols {
            if descriptor.relative_store_offset < last_catalog_end {
                return Err(bad(TextInstanceErrorKind::Malformed(
                    "unordered or overlapping new symbols",
                )));
            }
            validate_descriptor(
                site,
                StoredSymbol {
                    store: SymbolStore::New,
                    store_base: new_store_base,
                    symbol: *descriptor,
                },
                SymbolStore::New,
                new_store_base,
                new_source.size(),
                budget.max_new_store_span,
            )?;
            last_catalog_end = descriptor.relative_store_offset + descriptor.stored_bytes;
        }
        let mut seen_new = false;
        let mut imported_end = 0u64;
        let mut new_end = 0u64;
        for stored in &dictionary.catalog.exported_symbols {
            let (source_size, expected_base, max_span) = match stored.store {
                SymbolStore::Imported => {
                    if seen_new {
                        return Err(bad(TextInstanceErrorKind::Malformed(
                            "imported export follows new export",
                        )));
                    }
                    (
                        imported_source.size(),
                        imported_store_base,
                        budget.max_imported_store_span,
                    )
                }
                SymbolStore::New => {
                    seen_new = true;
                    (new_source.size(), new_store_base, budget.max_new_store_span)
                }
            };
            validate_descriptor(
                site,
                *stored,
                stored.store,
                expected_base,
                source_size,
                max_span,
            )?;
            let end = stored.symbol.relative_store_offset + stored.symbol.stored_bytes;
            let prior_end = match stored.store {
                SymbolStore::Imported => &mut imported_end,
                SymbolStore::New => &mut new_end,
            };
            if stored.symbol.relative_store_offset < *prior_end {
                return Err(bad(TextInstanceErrorKind::Malformed(
                    "unordered or overlapping exported symbols",
                )));
            }
            *prior_end = end;
            if stored.store == SymbolStore::New {
                let index = dictionary
                    .catalog
                    .new_symbols
                    .binary_search_by_key(&stored.symbol.relative_store_offset, |descriptor| {
                        descriptor.relative_store_offset
                    })
                    .map_err(|_| {
                        bad(TextInstanceErrorKind::Malformed(
                            "new export absent from new catalog",
                        ))
                    })?;
                if dictionary.catalog.new_symbols[index] != stored.symbol {
                    return Err(bad(TextInstanceErrorKind::Malformed(
                        "new export order differs from catalog",
                    )));
                }
            }
        }
        let count = dictionary.catalog.exported_symbols.len() as u64;
        let code_len = if count <= 1 {
            0
        } else {
            64 - (count - 1).leading_zeros()
        };
        let layout = banks.layout();
        if layout.code_len() != code_len
            || layout.total_contexts() != layout.bitmap_base() + GR_CONTEXTS
            || banks.mq_contexts_mut().count() != layout.total_contexts()
        {
            return Err(bad(TextInstanceErrorKind::Malformed(
                "IAID width or GR context layout mismatch",
            )));
        }
        let metadata_count = dictionary.catalog.new_symbols.len() as u64
            * mem::size_of::<SymbolDescriptor>() as u64
            + count * mem::size_of::<StoredSymbol>() as u64;
        preflight_cap(
            segment.number,
            at,
            fetched,
            "catalog metadata bytes",
            budget.max_metadata_bytes,
            metadata_count,
        )?;
        let target_row = u64::from(refinement_budget.max_width).div_ceil(8);
        let reference_row = u64::from(refinement_budget.max_reference_width).div_ceil(8);
        let working = layout.total_contexts() as u64 * mem::size_of::<MqContext>() as u64
            + (MQ_STATE_COUNT * mem::size_of::<MqState>()) as u64
            + MQ_BUFFER_BYTES
            + metadata_count
            + 2 * target_row
            + 3 * reference_row;
        preflight_cap(
            segment.number,
            at,
            fetched,
            "resident text working bytes",
            budget.max_working_bytes.min(limits.max_allocation_bytes),
            working,
        )?;
        let refinement_budget = RefinementBudget {
            max_total_output_bytes: refinement_budget
                .max_total_output_bytes
                .min(budget.max_temporary_store_bytes),
            max_pixels_per_bitmap: refinement_budget
                .max_pixels_per_bitmap
                .min(budget.max_pixels_per_instance),
            max_total_pixels: refinement_budget
                .max_total_pixels
                .min(budget.max_total_instance_pixels),
            ..refinement_budget
        };
        banks.reset_for_text_region();
        let mut init_fetched = 0;
        let mq = MqDecoder::new_with_init_progress(
            source,
            MqSpan {
                offset: parsed.body.offset,
                length: parsed.body.length,
            },
            table,
            banks.mq_contexts_mut(),
            limits,
            cancellation,
            mq_budget,
            &mut init_fetched,
        )
        .await
        .map_err(|error| {
            let offset = error.offset.unwrap_or(at);
            let mut located = preflight_error(
                segment.number,
                offset,
                fetched,
                TextInstanceErrorKind::Mq(Box::new(error)),
            );
            located.progress.mq_initialization_bytes_fetched = init_fetched;
            located
        })?;
        Ok(Self {
            mq,
            imported_source,
            new_source,
            temporary_sink,
            temporary_store_base,
            dictionary: &dictionary.catalog.exported_symbols,
            header: parsed,
            segment: segment.number,
            layout,
            limits,
            cancellation,
            refinement_budget,
            budget,
            progress: TextInstanceProgress {
                header_bytes_fetched: fetched,
                ..TextInstanceProgress::default()
            },
            strip_t: 0,
            first_s: 0,
            current_s: 0,
            initialized: false,
            strip_open: false,
            poisoned: false,
            complete: false,
        })
    }

    pub fn progress(&self) -> TextInstanceProgress {
        let mut progress = self.progress;
        progress.mq = Some(self.mq.snapshot());
        progress.poisoned = self.poisoned || progress.mq.is_some_and(|mq| mq.poisoned);
        progress
    }

    fn error(&self, kind: TextInstanceErrorKind) -> TextInstanceError {
        TextInstanceError {
            segment: self.segment,
            offset: self.mq.snapshot().current_input_offset,
            progress: Box::new(self.progress()),
            kind,
        }
    }

    fn cap(&self, resource: &'static str, limit: u64, attempted: u64) -> TextInstanceResult<()> {
        if attempted > limit {
            Err(self.error(TextInstanceErrorKind::LimitExceeded {
                resource,
                limit,
                attempted,
            }))
        } else {
            Ok(())
        }
    }

    fn coordinate(&self, value: Option<i64>) -> TextInstanceResult<i64> {
        checked_coordinate(value, self.budget.max_coordinate_magnitude)
            .map_err(|kind| self.error(kind))
    }

    fn check_cancelled(&self) -> TextInstanceResult<()> {
        if self.cancellation.is_cancelled() {
            Err(self.error(TextInstanceErrorKind::Cancelled))
        } else {
            Ok(())
        }
    }

    async fn signed(
        &mut self,
        procedure: IntegerProcedure,
        decision: TextDecision,
    ) -> TextInstanceResult<i64> {
        self.progress.decision = decision;
        match decode_integer(&mut self.mq, procedure).await {
            Ok(IntegerValue::Signed(value)) => Ok(value),
            Ok(IntegerValue::OutOfBand) => Err(self.error(TextInstanceErrorKind::Malformed(
                "unexpected arithmetic OOB",
            ))),
            Err(error) => Err(self.error(TextInstanceErrorKind::Mq(Box::new(error)))),
        }
    }

    async fn decode_refined(
        &mut self,
        reference: StoredSymbol,
        request: RefinementRequest,
    ) -> Result<SymbolDescriptor, RefinementError> {
        let prior = self.progress.refinement;
        let mut host = RefinementDecoder::new_continuing(
            &mut self.mq,
            self.layout,
            self.temporary_sink,
            self.limits,
            self.cancellation,
            self.refinement_budget,
            prior,
            &mut self.progress.refinement,
        )?;
        let report = match reference.store {
            SymbolStore::Imported => host.decode_bitmap(self.imported_source, request).await,
            SymbolStore::New => host.decode_bitmap(self.new_source, request).await,
        }?;
        // Pull consumers may reopen the temporary store as soon as this
        // handle is returned. Make the appended rows visible first.
        host.flush_store().await?;
        Ok(report.target)
    }

    /// Decode one placement. `None` means the declared count and exact MQ
    /// terminal were checked; subsequent calls return `None` without I/O.
    pub async fn next(&mut self) -> TextInstanceResult<Option<TextInstance>> {
        if self.complete {
            return Ok(None);
        }
        if self.poisoned {
            return Err(self.error(TextInstanceErrorKind::Poisoned));
        }
        self.poisoned = true;
        let result = self.next_inner().await;
        if result.is_ok() {
            self.poisoned = false;
        }
        result
    }

    async fn next_inner(&mut self) -> TextInstanceResult<Option<TextInstance>> {
        self.check_cancelled()?;
        if !self.initialized {
            let initial = self
                .signed(IntegerProcedure::Iadt, TextDecision::InitialStripT)
                .await?;
            self.strip_t = self.coordinate(initial.checked_neg())?;
            self.initialized = true;
        }
        loop {
            if self.progress.completed_instances == self.header.instances {
                self.progress.decision = TextDecision::Terminal;
                let expected = self.mq.snapshot().symbols_decoded;
                let snapshot = self
                    .mq
                    .finish_with_snapshot_mut(expected)
                    .await
                    .map_err(|error| self.error(TextInstanceErrorKind::Mq(Box::new(error))))?;
                self.progress.mq = Some(snapshot);
                self.check_cancelled()?;
                self.complete = true;
                self.progress.decision = TextDecision::Complete;
                return Ok(None);
            }
            if !self.strip_open {
                let strips = u64::from(self.progress.strips) + 1;
                self.cap("text strips", u64::from(self.budget.max_strips), strips)?;
                // This branch implies strips <= max_strips <= u32::MAX.
                self.progress.strips = strips as u32;
                let dt = self
                    .signed(IntegerProcedure::Iadt, TextDecision::StripDeltaT)
                    .await?;
                // Annex A.2 emits <2^33 magnitude and SBSTRIPS <= 8, so
                // multiplication stays below 2^36. The accumulated T still
                // requires a checked add and the configured signed cap.
                self.strip_t = self.coordinate(
                    self.strip_t
                        .checked_add(dt * i64::from(self.header.flags.strips())),
                )?;
                let dfs = self
                    .signed(IntegerProcedure::Iafs, TextDecision::FirstS)
                    .await?;
                self.first_s = self.coordinate(self.first_s.checked_add(dfs))?;
                self.current_s = self.first_s;
                self.strip_open = true;
            } else {
                self.progress.decision = TextDecision::DeltaS;
                match decode_integer(&mut self.mq, IntegerProcedure::Iads).await {
                    Ok(IntegerValue::OutOfBand) => {
                        self.strip_open = false;
                        continue;
                    }
                    Ok(IntegerValue::Signed(delta)) => {
                        self.current_s =
                            self.coordinate(self.current_s.checked_add(delta).and_then(|value| {
                                value.checked_add(i64::from(self.header.flags.ds_offset))
                            }))?;
                    }
                    Err(error) => {
                        return Err(self.error(TextInstanceErrorKind::Mq(Box::new(error))));
                    }
                }
            }
            let within = if self.header.flags.strips() == 1 {
                0
            } else {
                let value = self
                    .signed(IntegerProcedure::Iait, TextDecision::WithinStripT)
                    .await?;
                if value < 0 || value >= i64::from(self.header.flags.strips()) {
                    return Err(self.error(TextInstanceErrorKind::Malformed("IAIT outside strip")));
                }
                value
            };
            let t = self.coordinate(self.strip_t.checked_add(within))?;
            self.progress.decision = TextDecision::SymbolId;
            let raw_id = decode_iaid(&mut self.mq, self.layout)
                .await
                .map_err(|error| self.error(TextInstanceErrorKind::Mq(Box::new(error))))?;
            let id =
                checked_symbol_index(raw_id, self.dictionary.len() as u64, self.dictionary.len())
                    .map_err(|_| {
                    self.error(TextInstanceErrorKind::Malformed(
                        "IAID outside exported catalog",
                    ))
                })?;
            let reference = self.dictionary[id];
            let mut bitmap = TextBitmap::Stored(reference);
            let mut width = reference.symbol.width;
            let mut height = reference.symbol.height;
            let mut refinement_request = None;
            let ri = if self.header.flags.refine {
                self.signed(IntegerProcedure::Iari, TextDecision::RefinementFlag)
                    .await?
            } else {
                0
            };
            if ri != 0 && ri != 1 {
                return Err(self.error(TextInstanceErrorKind::Malformed("IARI is not a bit")));
            }
            if ri == 1 {
                let rdw = self
                    .signed(IntegerProcedure::Iardw, TextDecision::DeltaWidth)
                    .await?;
                let rdh = self
                    .signed(IntegerProcedure::Iardh, TextDecision::DeltaHeight)
                    .await?;
                let rdx = self
                    .signed(IntegerProcedure::Iardx, TextDecision::DeltaX)
                    .await?;
                let rdy = self
                    .signed(IntegerProcedure::Iardy, TextDecision::DeltaY)
                    .await?;
                let (target_width, target_height, reference_dx, reference_dy) =
                    refined_geometry(reference.symbol, rdw, rdh, rdx, rdy)
                        .map_err(|kind| self.error(kind))?;
                width = target_width;
                height = target_height;
                refinement_request = Some(RefinementRequest {
                    width,
                    height,
                    template: 1,
                    typical_prediction: false,
                    reference_dx,
                    reference_dy,
                    reference: RefinementReference {
                        store_base: reference.store_base,
                        symbol: reference.symbol,
                    },
                });
            }
            let pixels = u64::from(width) * u64::from(height);
            self.cap(
                "instance pixels",
                self.budget.max_pixels_per_instance,
                pixels,
            )?;
            // Both addends are at most MAX_BUDGET_COUNT after constructor and
            // per-instance checks; their sum fits u64 before the total cap.
            let total = self.progress.total_instance_pixels + pixels;
            self.cap(
                "total instance pixels",
                self.budget.max_total_instance_pixels,
                total,
            )?;
            let (x, y, next_s) = geometry(
                self.current_s,
                t,
                width,
                height,
                self.header.flags.reference_corner,
                self.header.flags.transposed,
                self.budget.max_coordinate_magnitude,
            )
            .map_err(|kind| self.error(kind))?;
            if let Some(request) = refinement_request {
                self.progress.decision = TextDecision::RefinementBitmap;
                let descriptor =
                    self.decode_refined(reference, request)
                        .await
                        .map_err(|error| {
                            self.error(TextInstanceErrorKind::Refinement(Box::new(error)))
                        })?;
                bitmap = TextBitmap::Refined {
                    store_base: self.temporary_store_base,
                    symbol: descriptor,
                };
            }
            self.current_s = next_s;
            self.progress.completed_instances += 1;
            self.progress.total_instance_pixels = total;
            if ri == 0 {
                self.progress.ri_zero += 1;
            } else {
                self.progress.ri_one += 1;
            }
            return Ok(Some(TextInstance {
                index: self.progress.completed_instances - 1,
                strip: self.progress.strips - 1,
                symbol_id: id as u32,
                ri: ri == 1,
                x,
                y,
                width,
                height,
                bitmap,
            }));
        }
    }
}

#[cfg(test)]
#[path = "text_instances/tests.rs"]
mod tests;
