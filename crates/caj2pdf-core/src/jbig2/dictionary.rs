// SPDX-License-Identifier: MIT

//! Bounded arithmetic T.88 symbol dictionaries (segment type 0, §6.5.5).
//!
//! One procedure decodes both observed HN/C8 dictionary profiles: a direct
//! dictionary (`0x0800`: template 2 with AT `(2, -1)` and no imports) and a
//! refinement dictionary over one imported direct dictionary (`0x1802`:
//! refinement template 1 and `REFAGGNINST = 1` for every symbol). Each new
//! symbol either decodes a template-2 bitmap or refines one earlier symbol;
//! aggregation, Huffman coding and bitmap-context carry are refused. The
//! caller owns every bitmap store, in memory, and the segment directory has
//! already framed the segment.

use super::{
    FieldCursor, FieldFault, PreflightKind, PreflightSite, SegmentHeader, SegmentSpan,
    generic::template2_context,
    iaid::{IAID_BASE, checked_symbol_index, decode_iaid},
    integer::{BITMAP_BASE, IntegerProcedure, IntegerValue, decode_integer},
    mq::{
        ArithmeticError, ArithmeticSnapshot, CodedSpan, ContextBank, ContextState, MQ_STATE_COUNT,
        MqBudget, MqDecoder, MqState, MqTable,
    },
    refinement::{
        ReferenceStore, RefinementBudget, RefinementDecoder, RefinementError, RefinementProgress,
        RefinementReference, RefinementRequest,
    },
};
use crate::fallible::try_convert;
use crate::{Cancellation, Error, Limits, MAX_BUDGET_COUNT, Payload, RangedSource};
use std::{error, fmt, mem};

/// Resource bounds for one symbol dictionary, in addition to `Limits` and `MqBudget`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DictionaryBudget {
    /// Segment-data dictionary header only.
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
    pub max_source_request_bytes: usize,
    /// Combined contexts, table, descriptor capacity, and rows.
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
            max_source_request_bytes: 256,
            max_working_bytes: 16 * 1024 * 1024,
        }
    }
}

/// Additional limits for a refinement dictionary's imported symbols and the
/// combined symbol set.
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
    /// Byte offset relative to the first byte appended by its decoder.
    pub relative_store_offset: u64,
    pub stored_bytes: u64,
}

/// The store that holds an exported symbol's packed bytes.
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

/// Complete new-symbol catalog and exported view in standard order.
#[derive(Debug, Eq, PartialEq)]
pub struct DictionaryCatalog {
    pub new_symbols: Vec<SymbolDescriptor>,
    pub exported_symbols: Vec<StoredSymbol>,
}

/// Complete refinement-branch counts. A count changes only after the whole
/// IAAI value has decoded; unsupported or malformed values remain visible.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct IaaiBranches {
    pub single_reference: u32,
    pub zero: u32,
    pub aggregation: u32,
}

/// Observable progress; a failed operation leaves the caller's store partial.
/// A direct dictionary counts its bitmap output in the first fields; a
/// refinement dictionary's bitmap output is in `refinement`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DictionaryProgress {
    pub completed_symbols: u32,
    pub stored_bitmap_bytes: u64,
    pub decoded_pixels: u64,
    pub height_classes: u32,
    pub export_runs: u32,
    pub iaai: IaaiBranches,
    /// Segment-data header bytes read; the directory read the framing.
    pub header_bytes_fetched: u64,
    pub refinement: RefinementProgress,
    pub mq: Option<ArithmeticSnapshot>,
}

/// Successful dictionary result. Store offsets are relative to the first byte
/// appended by this decoder.
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
    Mq(Box<ArithmeticError>),
    Refinement(Box<RefinementError>),
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
            DictionaryErrorKind::Mq(source) => write!(f, "MQ: {source}"),
            DictionaryErrorKind::Refinement(source) => write!(f, "refinement: {source}"),
        }
    }
}

impl error::Error for DictionaryError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.kind {
            DictionaryErrorKind::Source(error) => Some(error),
            DictionaryErrorKind::Mq(error) => Some(error),
            DictionaryErrorKind::Refinement(error) => Some(error),
            _ => None,
        }
    }
}

impl PreflightKind for DictionaryErrorKind {
    type Error = DictionaryError;

    fn locate(self, site: PreflightSite) -> DictionaryError {
        DictionaryError {
            segment: site.segment,
            offset: site.offset,
            progress: Box::new(DictionaryProgress {
                header_bytes_fetched: site.header_fetched,
                ..DictionaryProgress::default()
            }),
            kind: self,
        }
    }
}

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

/// Check a cap at a preflight site.
fn preflight_cap(
    site: PreflightSite,
    resource: &'static str,
    limit: u64,
    attempted: u64,
) -> DictionaryResult<()> {
    check_budget(resource, limit, attempted).map_err(|kind| site.error(kind))
}

struct HeaderCursor<'a> {
    header: &'a SegmentHeader,
    fields: FieldCursor,
}

impl HeaderCursor<'_> {
    fn error(&self, kind: DictionaryErrorKind) -> DictionaryError {
        self.error_at(self.fields.at, kind)
    }

    fn error_at(&self, offset: u64, kind: DictionaryErrorKind) -> DictionaryError {
        PreflightSite {
            segment: self.header.number,
            offset,
            header_fetched: self.fields.fetched,
        }
        .error(kind)
    }

    fn read<const N: usize, S: RangedSource, C: Cancellation>(
        &mut self,
        source: &mut S,
        name: &'static str,
        cancellation: &C,
    ) -> DictionaryResult<[u8; N]> {
        let mut bytes = [0u8; N];
        let result = self.fields.fill(source, cancellation, &mut bytes);
        result.map_err(|fault| self.fault(fault, name))?;
        Ok(bytes)
    }

    fn fault(&self, fault: FieldFault, name: &'static str) -> DictionaryError {
        match fault {
            FieldFault::LimitExceeded { attempted } => {
                self.error(DictionaryErrorKind::LimitExceeded {
                    resource: "dictionary header bytes",
                    limit: self.fields.max_header_bytes,
                    attempted,
                })
            }
            FieldFault::Overflow => {
                self.error(DictionaryErrorKind::InvalidSpan("header offset overflow"))
            }
            FieldFault::PastEnd | FieldFault::Ended { .. } => {
                self.error(DictionaryErrorKind::Truncated(name))
            }
            FieldFault::Cancelled => self.error(DictionaryErrorKind::Cancelled),
            FieldFault::Source(error) => self.error(DictionaryErrorKind::Source(error)),
            FieldFault::Overread => {
                self.error(DictionaryErrorKind::Malformed("source read length"))
            }
        }
    }
}

/// The source-independent checks before the data header is read. Keeping
/// them outside the generic reader shares one copy across every source and
/// cancellation type. Returns the data end.
fn data_header_bounds(
    header: &SegmentHeader,
    limits: &Limits,
    budget: DictionaryBudget,
    cancellation: &dyn Cancellation,
    source_size: u64,
) -> DictionaryResult<u64> {
    let site = PreflightSite {
        segment: header.number,
        offset: header.data.offset,
        header_fetched: 0,
    };
    limits
        .validate()
        .map_err(|e| site.error(DictionaryErrorKind::Source(e)))?;
    if budget.max_source_request_bytes == 0 {
        return Err(site.error(DictionaryErrorKind::Malformed("zero I/O request bound")));
    }
    if cancellation.is_cancelled() {
        return Err(site.error(DictionaryErrorKind::Cancelled));
    }
    if header.segment_type != 0 {
        return Err(site.error(DictionaryErrorKind::Unsupported {
            feature: "segment type",
            value: u64::from(header.segment_type),
        }));
    }
    preflight_cap(
        site,
        "dictionary data bytes",
        limits.max_input_bytes,
        header.data.length,
    )?;
    let end = header
        .data
        .offset
        .checked_add(header.data.length)
        .ok_or_else(|| site.error(DictionaryErrorKind::InvalidSpan("data end overflow")))?;
    if end > source_size {
        return Err(site.error(DictionaryErrorKind::InvalidSpan("data outside source")));
    }
    Ok(end)
}

/// Parse only the dictionary segment-data header, including conditional AT
/// fields, within `header.data`, which the segment directory has already
/// framed. This never initializes MQ or writes output.
/// `ArithmeticRefinementAggregate` is a classifier result, not a promise that
/// every such mode is decoded.
pub fn read_dictionary_data_header<S: RangedSource, C: Cancellation>(
    source: &mut S,
    header: &SegmentHeader,
    limits: &Limits,
    budget: DictionaryBudget,
    cancellation: &C,
) -> DictionaryResult<DictionaryDataHeader> {
    let end = data_header_bounds(header, limits, budget, cancellation, source.size())?;
    let mut cursor = HeaderCursor {
        header,
        fields: FieldCursor {
            start: header.data.offset,
            at: header.data.offset,
            end,
            fetched: 0,
            request_bytes: budget.max_source_request_bytes.min(limits.io_chunk_bytes),
            max_header_bytes: budget.max_data_header_bytes,
        },
    };
    let flags = u16::from_be_bytes(cursor.read(source, "dictionary flags", cancellation)?);
    let flags_error = |cursor: &HeaderCursor<'_>, reason| {
        cursor.error_at(header.data.offset, DictionaryErrorKind::Malformed(reason))
    };
    if flags & 0xe000 != 0 {
        return Err(flags_error(&cursor, "reserved dictionary flags"));
    }
    let huffman = flags & 1 != 0;
    let refinement = flags & 2 != 0;
    let template = ((flags >> 10) & 3) as u8;
    let refinement_template = ((flags >> 12) & 1) as u8;
    if !huffman && flags & 0xfc != 0 {
        return Err(flags_error(
            &cursor,
            "arithmetic dictionary Huffman selection flags",
        ));
    }
    if huffman {
        if ((flags >> 2) & 3) == 2 || ((flags >> 4) & 3) == 2 {
            return Err(flags_error(&cursor, "reserved Huffman selector"));
        }
        if template != 0 {
            return Err(flags_error(&cursor, "Huffman dictionary template"));
        }
        if !refinement && flags & 0x380 != 0 {
            return Err(flags_error(&cursor, "Huffman direct bitmap flags"));
        }
    }
    if !refinement && refinement_template != 0 {
        return Err(flags_error(&cursor, "unused refinement template"));
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
        let [x, y] = cursor.read(source, "dictionary AT", cancellation)?;
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
        let [x, y] = cursor.read(source, "dictionary refinement AT", cancellation)?;
        *position = (x as i8, y as i8);
    }
    let exported_offset = cursor.fields.at;
    let exported_symbols =
        u32::from_be_bytes(cursor.read(source, "exported symbol count", cancellation)?);
    let new_offset = cursor.fields.at;
    let new_symbols = u32::from_be_bytes(cursor.read(source, "new symbol count", cancellation)?);
    let header_bytes = cursor.fields.at - header.data.offset;
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
    let body_length = end - cursor.fields.at;
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
            offset: cursor.fields.at,
            length: body_length,
        },
    })
}

/// `SBSYMCODELEN` for `symbols` symbols: the IAID width, T.88 §6.5.8.2.3.
pub fn symbol_code_length(symbols: u64) -> u32 {
    if symbols <= 1 {
        0
    } else {
        64 - (symbols - 1).leading_zeros()
    }
}

/// The context count of a dictionary or text-region coding unit whose IAID
/// width is `code_len`: the integer and bitmap contexts, then `2^code_len`
/// IAID contexts. A direct dictionary, which decodes no IAID, uses exactly
/// [`IAID_BASE`] contexts.
pub fn coding_unit_contexts(code_len: u32) -> Option<usize> {
    1usize
        .checked_shl(code_len)
        .and_then(|ids| IAID_BASE.checked_add(ids))
}

/// The referred-to dictionary of a refinement dictionary: its segment and
/// its complete direct report.
#[derive(Clone, Copy, Debug)]
pub struct ImportedDictionary<'a> {
    pub segment: &'a SegmentHeader,
    pub report: &'a DictionaryReport,
}

/// The bitmap stores of one dictionary, in memory. A store's descriptor
/// offsets are relative to its base. New symbols are appended to `new`,
/// whose length must be `new_base`; a refinement dictionary reads earlier new
/// symbols back from it. A direct dictionary reads no store.
pub struct DictionaryStores<'a> {
    pub imported: &'a [u8],
    pub imported_base: u64,
    pub new: &'a mut Vec<u8>,
    pub new_base: u64,
}

/// Validate the imported report against its segment and the caller's view of
/// its store before any arithmetic, outside the generic decoder.
fn validate_imported(
    site: PreflightSite,
    segment: &SegmentHeader,
    imported: ImportedDictionary<'_>,
    store: (u64, u64),
    budget: RefinementDictionaryBudget,
    refinement_budget: RefinementBudget,
) -> DictionaryResult<()> {
    let (store_size, store_base) = store;
    let bad = |reason| site.error(DictionaryErrorKind::Malformed(reason));
    let imported_segment = imported.segment;
    let report = imported.report;
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
        || report.progress.completed_symbols != report.header.new_symbols
        || report.progress.mq.is_none()
        || report
            .catalog
            .exported_symbols
            .iter()
            .any(|stored| stored.store != SymbolStore::New)
    {
        return Err(bad("imported dictionary is not a complete direct report"));
    }
    if store_base > store_size {
        return Err(bad("imported store base outside the store"));
    }
    preflight_cap(
        site,
        "imported symbols",
        u64::from(budget.max_imported_symbols),
        report.catalog.exported_symbols.len() as u64,
    )?;
    preflight_cap(
        site,
        "imported catalog new symbols",
        u64::from(budget.max_imported_symbols),
        report.catalog.new_symbols.len() as u64,
    )?;
    // Both lengths equal checked u32 header counts, so this sum and its byte
    // product fit u64 on native and wasm32 targets.
    let imported_catalog_count =
        report.catalog.new_symbols.len() as u64 + report.catalog.exported_symbols.len() as u64;
    let imported_catalog_bytes = imported_catalog_count * mem::size_of::<SymbolDescriptor>() as u64;
    preflight_cap(
        site,
        "imported catalog metadata bytes",
        budget.max_catalog_bytes,
        imported_catalog_bytes,
    )?;
    let mut next_new = 0usize;
    for exported in &report.catalog.exported_symbols {
        let matching = report.catalog.new_symbols[next_new..]
            .iter()
            .position(|candidate| *candidate == exported.symbol)
            .ok_or_else(|| bad("imported exports do not follow new-symbol order"))?;
        next_new += matching + 1;
    }
    let mut previous_end = 0;
    let mut total_bytes = 0u64;
    for exported in &report.catalog.exported_symbols {
        let descriptor = exported.symbol;
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
            return Err(bad("imported descriptor outside the store"));
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
            preflight_cap(site, name, cap, value)?;
        }
        // A descriptor is at most u32-by-u32 packed pixels (<2^61 bytes),
        // while every prior total was capped below 2^48 on this loop.
        total_bytes += bytes;
        preflight_cap(
            site,
            "imported bitmap bytes",
            budget.max_imported_bitmap_bytes.min(MAX_BUDGET_COUNT),
            total_bytes,
        )?;
        previous_end = relative_end;
    }
    Ok(())
}

/// What a checked header fixes before MQ starts.
struct Plan {
    /// A refinement dictionary; otherwise a direct one.
    refine: bool,
    /// The IAID width over imported exports and new symbols.
    code_len: u32,
    /// Contexts, table, and catalog metadata.
    base_working: u64,
    working_cap: u64,
    refinement_budget: RefinementBudget,
}

/// The source-independent profile and resource checks between the data
/// header and MQ initialization, shared by every decoder instantiation.
/// `stores` are the imported store size and base, then the new store's.
#[allow(clippy::too_many_arguments)]
fn check_header(
    segment: &SegmentHeader,
    header: &DictionaryDataHeader,
    import: Option<ImportedDictionary<'_>>,
    stores: [(u64, u64); 2],
    context_count: usize,
    limits: &Limits,
    budget: DictionaryBudget,
    refinement_budget: RefinementBudget,
    second_budget: RefinementDictionaryBudget,
) -> DictionaryResult<Plan> {
    let site = PreflightSite {
        segment: segment.number,
        offset: header.body.offset,
        header_fetched: header.header_bytes,
    };
    let unsupported =
        |feature, value| site.error(DictionaryErrorKind::Unsupported { feature, value });
    let malformed = |reason| site.error(DictionaryErrorKind::Malformed(reason));
    let refine = match header.mode {
        DictionaryMode::ArithmeticDirect => false,
        DictionaryMode::ArithmeticRefinementAggregate => true,
        DictionaryMode::HuffmanDirect | DictionaryMode::HuffmanRefinementAggregate => {
            return Err(unsupported(
                "Huffman symbol dictionary",
                u64::from(header.flags),
            ));
        }
    };
    if !refine && header.template != 2 {
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
    if refine && (header.flags != 0x1802 || header.at[0] != (2, -1)) {
        return Err(unsupported(
            "second dictionary flags or adaptive template",
            u64::from(header.flags),
        ));
    }
    if segment.page_association != 1 {
        return Err(unsupported(
            "dictionary page association",
            u64::from(segment.page_association),
        ));
    }
    if stores[1].1 != stores[1].0 {
        return Err(site.error(DictionaryErrorKind::InvalidSpan(
            "new store base differs from the store length",
        )));
    }
    let imported_exports = match import {
        _ if !refine => {
            if !segment.referred_to.is_empty() {
                return Err(unsupported(
                    "imported dictionary references",
                    segment.referred_to.len() as u64,
                ));
            }
            // The parsed template-2 AT was already checked as backwards-only.
            if header.at[0] != (2, -1) {
                return Err(PreflightSite {
                    offset: segment.data.offset + 2,
                    ..site
                }
                .error(DictionaryErrorKind::UnsupportedAt {
                    x: header.at[0].0,
                    y: header.at[0].1,
                }));
            }
            0
        }
        None => {
            return Err(malformed(
                "expected exactly the supplied dictionary reference",
            ));
        }
        Some(import) => {
            validate_imported(
                PreflightSite {
                    offset: segment.data.offset,
                    ..site
                },
                segment,
                import,
                stores[0],
                second_budget,
                refinement_budget,
            )?;
            import.report.catalog.exported_symbols.len() as u64
        }
    };
    // Imported and declared counts are each bounded by u32 header fields.
    let total = imported_exports + u64::from(header.new_symbols);
    if refine {
        preflight_cap(
            site,
            "total symbols",
            u64::from(second_budget.max_total_symbols),
            total,
        )?;
    }
    if u64::from(header.exported_symbols) > total {
        return Err(malformed("exported count exceeds available symbols"));
    }
    let code_len = symbol_code_length(total);
    let expected_contexts = if refine {
        coding_unit_contexts(code_len)
    } else {
        Some(IAID_BASE)
    };
    if expected_contexts != Some(context_count) {
        return Err(malformed(if refine {
            "IAID width or GR context layout mismatch"
        } else {
            "expected exactly 7680 integer and bitmap MQ contexts"
        }));
    }
    // Every descriptor count is a u32 header count, and the combined
    // descriptor byte total is far below u64::MAX even at those maxima.
    let imported_metadata = import.filter(|_| refine).map_or(0, |import| {
        (import.report.catalog.new_symbols.len() as u64 + imported_exports)
            * mem::size_of::<SymbolDescriptor>() as u64
    });
    let new_metadata = u64::from(header.new_symbols) * mem::size_of::<SymbolDescriptor>() as u64;
    let export_metadata =
        u64::from(header.exported_symbols) * mem::size_of::<StoredSymbol>() as u64;
    let metadata = imported_metadata + new_metadata + export_metadata;
    let (catalog_cap, working_cap) = if refine {
        (
            second_budget
                .max_catalog_bytes
                .min(budget.max_catalog_bytes),
            second_budget
                .max_working_bytes
                .min(budget.max_working_bytes),
        )
    } else {
        (budget.max_catalog_bytes, budget.max_working_bytes)
    };
    preflight_cap(site, "catalog metadata bytes", catalog_cap, metadata)?;
    preflight_cap(
        site,
        "catalog allocation bytes",
        limits.max_allocation_bytes,
        new_metadata + export_metadata,
    )?;
    // The caller's context bank is already allocated. Its byte count is
    // bounded by isize::MAX; the u32-limited metadata cannot overflow u64.
    let base_working = context_count as u64 * mem::size_of::<ContextState>() as u64
        + (MQ_STATE_COUNT * mem::size_of::<MqState>()) as u64
        + metadata;
    preflight_cap(site, "dictionary working bytes", working_cap, base_working)?;
    let refinement_budget = RefinementBudget {
        max_width: refinement_budget.max_width.min(budget.max_width),
        max_height: refinement_budget.max_height.min(budget.max_height),
        max_pixels_per_bitmap: refinement_budget
            .max_pixels_per_bitmap
            .min(budget.max_pixels_per_symbol),
        max_total_pixels: refinement_budget
            .max_total_pixels
            .min(budget.max_total_pixels),
        max_bytes_per_bitmap: refinement_budget
            .max_bytes_per_bitmap
            .min(budget.max_bytes_per_symbol),
        max_total_output_bytes: refinement_budget
            .max_total_output_bytes
            .min(budget.max_stored_bitmap_bytes),
        ..refinement_budget
    };
    Ok(Plan {
        refine,
        code_len,
        base_working,
        working_cap,
        refinement_budget,
    })
}

fn reserve_catalog<T>(count: usize, site: PreflightSite) -> DictionaryResult<Vec<T>> {
    let mut entries = Vec::new();
    entries
        .try_reserve_exact(count)
        .map_err(|_| site.error(DictionaryErrorKind::AllocationFailed))?;
    Ok(entries)
}

/// Checked dimensions of one new symbol: width, height, pixels, and packed
/// bytes.
type SymbolGeometry = (u32, u32, u64, u64);

/// Validate one decoded symbol size against the dictionary budgets before
/// any bitmap work, given the pixels and stored bytes decoded so far. Not
/// generic, so every decoder instantiation shares it; the caller locates the
/// returned error kind at the current MQ offset.
fn symbol_geometry(
    width: i64,
    height: i64,
    budget: &DictionaryBudget,
    decoded_pixels: u64,
    stored_bytes: u64,
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
    check_budget(
        "symbol pixels",
        budget.max_pixels_per_symbol.min(MAX_BUDGET_COUNT),
        pixels,
    )?;
    let total_pixels =
        decoded_pixels
            .checked_add(pixels)
            .ok_or(DictionaryErrorKind::InvalidSpan(
                "total pixel count overflow",
            ))?;
    check_budget("dictionary pixels", budget.max_total_pixels, total_pixels)?;
    // The maximum stride is 2^29 bytes, so this product fits u64.
    let bytes = u64::from(width).div_ceil(8) * u64::from(height);
    check_budget(
        "symbol bytes",
        budget.max_bytes_per_symbol.min(MAX_BUDGET_COUNT),
        bytes,
    )?;
    let stored = stored_bytes
        .checked_add(bytes)
        .ok_or(DictionaryErrorKind::InvalidSpan(
            "stored byte count overflow",
        ))?;
    check_budget(
        "stored bitmap bytes",
        budget.max_stored_bitmap_bytes,
        stored,
    )?;
    Ok((width, height, pixels, bytes))
}

/// One symbol dictionary coding unit over the exact segment body (T.88
/// §6.5.5). A direct dictionary appends every new symbol's packed rows to
/// the new store; a refinement dictionary refines one imported or earlier
/// new symbol per new symbol. `decode()` then checks the export runs and the
/// single MQ tail. On error the caller must discard all bytes appended to the
/// new store.
pub struct SymbolDictionaryDecoder<'a, C: Cancellation> {
    mq: MqDecoder<'a>,
    stores: DictionaryStores<'a>,
    imported: &'a [StoredSymbol],
    plan: Plan,
    header: DictionaryDataHeader,
    segment: u32,
    limits: &'a Limits,
    cancellation: &'a C,
    budget: DictionaryBudget,
    progress: DictionaryProgress,
    catalog: DictionaryCatalog,
}

impl<'a, C: Cancellation> SymbolDictionaryDecoder<'a, C> {
    /// Parse and check the dictionary from `input`, which must hold the
    /// whole segment data, then start its MQ coding unit over `contexts`:
    /// [`IAID_BASE`] contexts for a direct dictionary, or
    /// [`coding_unit_contexts`] of its `SBSYMCODELEN` for a refinement
    /// dictionary. Every context is reset; this profile never carries bitmap
    /// contexts. A refinement dictionary needs `import`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        input: Payload<'a>,
        segment: &SegmentHeader,
        import: Option<ImportedDictionary<'a>>,
        stores: DictionaryStores<'a>,
        table: &'a MqTable,
        contexts: &'a mut ContextBank,
        limits: &'a Limits,
        cancellation: &'a C,
        mq_budget: MqBudget,
        budget: DictionaryBudget,
        refinement_budget: RefinementBudget,
        second_budget: RefinementDictionaryBudget,
    ) -> DictionaryResult<Self> {
        let header =
            read_dictionary_data_header(&mut { input }, segment, limits, budget, cancellation)?;
        let plan = check_header(
            segment,
            &header,
            import,
            [
                (stores.imported.len() as u64, stores.imported_base),
                (stores.new.len() as u64, stores.new_base),
            ],
            contexts.len(),
            limits,
            budget,
            refinement_budget,
            second_budget,
        )?;
        let site = PreflightSite {
            segment: segment.number,
            offset: header.body.offset,
            header_fetched: header.header_bytes,
        };
        let new_symbols = reserve_catalog(header.new_symbols as usize, site)?;
        let exported_symbols = reserve_catalog(header.exported_symbols as usize, site)?;
        // T.88 §7.4.2.2 resets all arithmetic-integer statistics at each new
        // dictionary, and no bitmap context reuse is accepted.
        contexts.reset();
        let span = CodedSpan {
            offset: header.body.offset,
            length: header.body.length,
        };
        let mq =
            MqDecoder::new(input, span, table, contexts, limits, mq_budget).map_err(|error| {
                PreflightSite {
                    offset: error.offset.unwrap_or(site.offset),
                    ..site
                }
                .error(DictionaryErrorKind::Mq(Box::new(error)))
            })?;
        let progress = DictionaryProgress {
            header_bytes_fetched: header.header_bytes,
            mq: Some(mq.snapshot()),
            ..DictionaryProgress::default()
        };
        let imported = match import {
            Some(import) if plan.refine => import.report.catalog.exported_symbols.as_slice(),
            _ => &[],
        };
        Ok(Self {
            mq,
            stores,
            imported,
            plan,
            header,
            segment: segment.number,
            limits,
            cancellation,
            budget,
            progress,
            catalog: DictionaryCatalog {
                new_symbols,
                exported_symbols,
            },
        })
    }

    /// Current progress, with the MQ snapshot.
    pub fn progress(&self) -> DictionaryProgress {
        let mut progress = self.progress;
        progress.mq = Some(self.mq.snapshot());
        progress
    }

    /// Decode all new symbols, ordered exports, and the exact MQ terminal
    /// sequence. `Ok` is the only state in which the catalog and the new
    /// store are valid.
    pub fn decode(mut self) -> DictionaryResult<DictionaryReport> {
        let segment = self.segment;
        let initial_offset = self.mq.snapshot().input_offset;
        let initial_progress = self.progress();
        let mut session = Session {
            segment: self.segment,
            header: self.header,
            imported: self.imported,
            imported_store: self.stores.imported,
            imported_base: self.stores.imported_base,
            new_base: self.stores.new_base,
            plan: &self.plan,
            limits: self.limits,
            cancellation: self.cancellation,
            budget: self.budget,
            progress: &mut self.progress,
            catalog: &mut self.catalog,
            rows: [Vec::new(), Vec::new(), Vec::new()],
        };
        let mut unit = if self.plan.refine {
            let host = RefinementDecoder::new(
                &mut self.mq,
                &mut *self.stores.new,
                self.limits,
                self.cancellation,
                self.plan.refinement_budget,
            )
            .map_err(|error| DictionaryError {
                segment,
                offset: error.offset.unwrap_or(initial_offset),
                progress: Box::new(initial_progress),
                kind: DictionaryErrorKind::Refinement(Box::new(error)),
            })?;
            Unit::Refined(Box::new(host))
        } else {
            Unit::Direct {
                mq: &mut self.mq,
                store: &mut *self.stores.new,
            }
        };
        let decoded = session.decode_all(&mut unit);
        if let Unit::Refined(host) = &unit {
            self.progress.refinement = host.progress();
        }
        drop(unit);
        decoded?;
        let expected = self.mq.snapshot().symbols_decoded;
        let snapshot = self.mq.finish(expected).map_err(|error| {
            let offset = error.offset.unwrap_or(self.mq.snapshot().input_offset);
            self.error(DictionaryErrorKind::Mq(Box::new(error)), offset)
        })?;
        self.progress.mq = Some(snapshot);
        Ok(DictionaryReport {
            header: self.header,
            catalog: self.catalog,
            progress: self.progress,
        })
    }

    fn error(&self, kind: DictionaryErrorKind, offset: u64) -> DictionaryError {
        DictionaryError {
            segment: self.segment,
            offset,
            progress: Box::new(self.progress()),
            kind,
        }
    }
}

/// The coding unit of one `decode` call: the raw MQ decoder and the new
/// store for a direct dictionary, or the refinement host that borrows both.
enum Unit<'u, 'mq, C: Cancellation> {
    Direct {
        mq: &'u mut MqDecoder<'mq>,
        store: &'u mut Vec<u8>,
    },
    Refined(Box<RefinementDecoder<'u, 'mq, C>>),
}

impl<'mq, C: Cancellation> Unit<'_, 'mq, C> {
    fn mq(&mut self) -> &mut MqDecoder<'mq> {
        match self {
            Unit::Direct { mq, .. } => mq,
            Unit::Refined(host) => host.mq_mut(),
        }
    }
}

/// The state of one `decode` call, apart from the coding unit.
struct Session<'s, C: Cancellation> {
    segment: u32,
    header: DictionaryDataHeader,
    imported: &'s [StoredSymbol],
    imported_store: &'s [u8],
    imported_base: u64,
    new_base: u64,
    plan: &'s Plan,
    limits: &'s Limits,
    cancellation: &'s C,
    budget: DictionaryBudget,
    progress: &'s mut DictionaryProgress,
    catalog: &'s mut DictionaryCatalog,
    /// Two previous rows and the current row of a direct bitmap.
    rows: [Vec<u8>; 3],
}

impl<C: Cancellation> Session<'_, C> {
    /// An error with the session's progress, the coding unit's snapshot, and
    /// the refinement host's progress when there is one.
    fn error_with(
        &self,
        mq: Option<ArithmeticSnapshot>,
        refinement: Option<RefinementProgress>,
        kind: DictionaryErrorKind,
    ) -> DictionaryError {
        let mut progress = *self.progress;
        if let Some(refinement) = refinement {
            progress.refinement = refinement;
        }
        progress.mq = mq;
        DictionaryError {
            segment: self.segment,
            offset: mq.map_or(self.header.body.offset, |snapshot| snapshot.input_offset),
            progress: Box::new(progress),
            kind,
        }
    }

    fn error(&self, unit: &Unit<'_, '_, C>, kind: DictionaryErrorKind) -> DictionaryError {
        match unit {
            Unit::Direct { mq, .. } => self.error_with(Some(mq.snapshot()), None, kind),
            Unit::Refined(host) => {
                let progress = host.progress();
                self.error_with(progress.mq, Some(progress), kind)
            }
        }
    }

    /// Locate an error at its own offset when it has one.
    fn located(
        &self,
        unit: &Unit<'_, '_, C>,
        offset: Option<u64>,
        kind: DictionaryErrorKind,
    ) -> DictionaryError {
        let mut error = self.error(unit, kind);
        error.offset = offset.unwrap_or(error.offset);
        error
    }

    fn malformed(&self, unit: &Unit<'_, '_, C>, reason: &'static str) -> DictionaryError {
        self.error(unit, DictionaryErrorKind::Malformed(reason))
    }

    fn cap(
        &self,
        unit: &Unit<'_, '_, C>,
        resource: &'static str,
        maximum: u64,
        attempted: u64,
    ) -> DictionaryResult<()> {
        check_budget(resource, maximum, attempted).map_err(|kind| self.error(unit, kind))
    }

    fn check_cancelled(&self, unit: &Unit<'_, '_, C>) -> DictionaryResult<()> {
        if self.cancellation.is_cancelled() {
            Err(self.error(unit, DictionaryErrorKind::Cancelled))
        } else {
            Ok(())
        }
    }

    /// The next value of a `u32` counter, refusing a wrap.
    fn next_count(
        &self,
        unit: &Unit<'_, '_, C>,
        value: u32,
        field: &'static str,
    ) -> DictionaryResult<u32> {
        value
            .checked_add(1)
            .ok_or_else(|| self.error(unit, DictionaryErrorKind::InvalidSpan(field)))
    }

    fn integer(
        &self,
        unit: &mut Unit<'_, '_, C>,
        procedure: IntegerProcedure,
    ) -> DictionaryResult<IntegerValue> {
        decode_integer(unit.mq(), procedure).map_err(|error| {
            let offset = error.offset;
            self.located(unit, offset, DictionaryErrorKind::Mq(Box::new(error)))
        })
    }

    fn iaid(&self, unit: &mut Unit<'_, '_, C>) -> DictionaryResult<u64> {
        decode_iaid(unit.mq(), self.plan.code_len).map_err(|error| {
            let offset = error.offset;
            self.located(unit, offset, DictionaryErrorKind::Mq(Box::new(error)))
        })
    }

    fn signed(
        &self,
        unit: &Unit<'_, '_, C>,
        value: IntegerValue,
        field: &'static str,
    ) -> DictionaryResult<i64> {
        match value {
            IntegerValue::Signed(value) => Ok(value),
            IntegerValue::OutOfBand => Err(self.malformed(unit, field)),
        }
    }

    /// The stored symbol with active index `index` (T.88 §6.5.5 step 4c):
    /// the imported exports, then the new symbols decoded so far.
    fn stored(&self, index: usize) -> StoredSymbol {
        if index < self.imported.len() {
            StoredSymbol {
                store: SymbolStore::Imported,
                store_base: self.imported_base,
                symbol: self.imported[index].symbol,
            }
        } else {
            StoredSymbol {
                store: SymbolStore::New,
                store_base: self.new_base,
                symbol: self.catalog.new_symbols[index - self.imported.len()],
            }
        }
    }

    /// Check one decoded size against every budget before any bitmap work,
    /// including the mode's working rows: three for a direct bitmap.
    fn geometry(
        &self,
        unit: &Unit<'_, '_, C>,
        width: i64,
        height: i64,
    ) -> DictionaryResult<SymbolGeometry> {
        let (pixels_so_far, bytes_so_far) = match unit {
            Unit::Direct { .. } => (
                self.progress.decoded_pixels,
                self.progress.stored_bitmap_bytes,
            ),
            Unit::Refined(host) => {
                let progress = host.progress();
                (progress.pixels_decoded, progress.output_bytes_written)
            }
        };
        let geometry = symbol_geometry(width, height, &self.budget, pixels_so_far, bytes_so_far)
            .map_err(|kind| self.error(unit, kind))?;
        let (width, _, _, bytes) = geometry;
        // `symbol_geometry` capped both totals, so neither sum overflows.
        let stored = bytes_so_far + bytes;
        if let Unit::Refined(_) = unit {
            // The refinement host caps output against Limits before writing,
            // and its working rows depend on the reference.
            return Ok(geometry);
        }
        self.cap(unit, "output bytes", self.limits.max_output_bytes, stored)?;
        let scratch = u64::from(width).div_ceil(8) * 3;
        self.cap(
            unit,
            "row scratch bytes",
            self.limits.max_allocation_bytes,
            scratch,
        )?;
        self.cap(
            unit,
            "dictionary working bytes",
            self.plan.working_cap,
            self.plan.base_working + scratch,
        )?;
        Ok(geometry)
    }

    fn decode_all(&mut self, unit: &mut Unit<'_, '_, C>) -> DictionaryResult<()> {
        self.decode_symbols(unit)?;
        self.decode_exports(unit)?;
        self.check_cancelled(unit)
    }

    /// T.88 §6.5.5 steps 4b–4c: height classes of new symbols, each either a
    /// direct bitmap or the single-reference refinement of an earlier symbol.
    fn decode_symbols(&mut self, unit: &mut Unit<'_, '_, C>) -> DictionaryResult<()> {
        let mut class_height = 0i64;
        while self.progress.completed_symbols < self.header.new_symbols {
            self.check_cancelled(unit)?;
            let classes = self.next_count(
                unit,
                self.progress.height_classes,
                "height class count overflow",
            )?;
            self.cap(
                unit,
                "height classes",
                u64::from(self.budget.max_height_classes),
                u64::from(classes),
            )?;
            self.progress.height_classes = classes;
            let value = self.integer(unit, IntegerProcedure::Iadh)?;
            // Each prior class height is at most u32::MAX, while Annex A.2
            // integer magnitudes stay below 2^33, so this fits i64.
            class_height += self.signed(unit, value, "IADH out of band")?;
            if class_height < 0 || class_height > i64::from(u32::MAX) {
                return Err(self.malformed(unit, "height class dimension"));
            }
            self.cap(
                unit,
                "height class",
                u64::from(self.budget.max_height),
                class_height as u64,
            )?;
            let mut class_width = 0i64;
            loop {
                let value = self.integer(unit, IntegerProcedure::Iadw)?;
                let delta = match value {
                    IntegerValue::OutOfBand => break,
                    IntegerValue::Signed(delta) => delta,
                };
                if self.progress.completed_symbols == self.header.new_symbols {
                    return Err(self.malformed(unit, "symbol-count overrun before width OOB"));
                }
                // The preceding accepted width is at most u32::MAX and an
                // Annex A.2 signed delta has magnitude below 2^33.
                class_width += delta;
                let (width, height, pixels, bytes) =
                    self.geometry(unit, class_width, class_height)?;
                let descriptor = match unit {
                    Unit::Direct { mq, store } => {
                        self.direct_bitmap(mq, store, width, height, pixels, bytes)?
                    }
                    Unit::Refined(_) => self.refined_bitmap(unit, width, height)?,
                };
                self.catalog.new_symbols.push(descriptor);
                self.progress.completed_symbols += 1;
            }
        }
        Ok(())
    }

    /// T.88 §6.5.8.2: one REFAGGNINST = 1 symbol refining one active symbol.
    fn refined_bitmap(
        &mut self,
        unit: &mut Unit<'_, '_, C>,
        width: u32,
        height: u32,
    ) -> DictionaryResult<SymbolDescriptor> {
        let instances = self.integer(unit, IntegerProcedure::Iaai)?;
        let instances = self.signed(unit, instances, "REFAGGNINST OOB")?;
        if instances == 0 {
            self.progress.iaai.zero += 1;
            return Err(self.malformed(unit, "REFAGGNINST zero"));
        }
        if instances < 0 {
            return Err(self.malformed(unit, "REFAGGNINST negative"));
        }
        if instances > 1 {
            self.progress.iaai.aggregation += 1;
            return Err(self.error(
                unit,
                DictionaryErrorKind::Unsupported {
                    feature: "REFAGGNINST aggregation",
                    value: instances as u64,
                },
            ));
        }
        self.progress.iaai.single_reference += 1;
        let raw_id = self.iaid(unit)?;
        let active = self.imported.len() + self.catalog.new_symbols.len();
        let index = checked_symbol_index(raw_id, active as u64, active)
            .map_err(|_| self.malformed(unit, "future, self, or absent symbol ID"))?;
        let reference = self.stored(index);
        let rows =
            2 * u64::from(width).div_ceil(8) + 3 * u64::from(reference.symbol.width).div_ceil(8);
        // The allocated context bank is below isize::MAX bytes, catalog
        // counts are u32-bounded, and five packed rows add at most 2.7 GiB.
        self.cap(
            unit,
            "dictionary working bytes",
            self.plan.working_cap,
            self.plan.base_working + rows,
        )?;
        let dx = self.integer(unit, IntegerProcedure::Iardx)?;
        let dy = self.integer(unit, IntegerProcedure::Iardy)?;
        let dx = self.signed(unit, dx, "IARDX out of band")?;
        let dy = self.signed(unit, dy, "IARDY out of band")?;
        let dx = i32::try_from(dx)
            .map_err(|_| self.malformed(unit, "IARDX outside signed 32-bit range"))?;
        let dy = i32::try_from(dy)
            .map_err(|_| self.malformed(unit, "IARDY outside signed 32-bit range"))?;
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
        let store = match reference.store {
            SymbolStore::Imported => ReferenceStore::Other(self.imported_store),
            SymbolStore::New => ReferenceStore::Output,
        };
        let Unit::Refined(host) = unit else {
            unreachable!("a refined bitmap needs the refinement host");
        };
        match host.decode_bitmap(store, request) {
            Ok(report) => Ok(report.target),
            Err(error) => {
                let offset = error.offset;
                Err(self.located(
                    unit,
                    offset,
                    DictionaryErrorKind::Refinement(Box::new(error)),
                ))
            }
        }
    }

    /// One template-2 generic bitmap (T.88 §6.2 with TPGDON off), appended
    /// row by row to the new store.
    fn direct_bitmap(
        &mut self,
        mq: &mut MqDecoder<'_>,
        store: &mut Vec<u8>,
        width: u32,
        height: u32,
        pixels: u64,
        bytes: u64,
    ) -> DictionaryResult<SymbolDescriptor> {
        let fail = |session: &Self, mq: &MqDecoder<'_>, kind| {
            session.error_with(Some(mq.snapshot()), None, kind)
        };
        let relative_store_offset = self.progress.stored_bitmap_bytes;
        // The store is in memory, so its length fits a `u64`.
        let attempted = (store.len() as u64).saturating_add(bytes);
        if attempted > self.limits.max_allocation_bytes {
            return Err(fail(
                self,
                mq,
                DictionaryErrorKind::LimitExceeded {
                    resource: "symbol store bytes",
                    limit: self.limits.max_allocation_bytes,
                    attempted,
                },
            ));
        }
        // `bytes` fits the allocation limit, hence a `usize` on this target.
        if store.try_reserve(bytes as usize).is_err() {
            return Err(fail(self, mq, DictionaryErrorKind::AllocationFailed));
        }
        // At most 2^29 bytes; even wasm32's usize can represent it.
        let stride = width.div_ceil(8) as usize;
        for row in &mut self.rows {
            if row.len() < stride && row.try_reserve_exact(stride - row.len()).is_err() {
                return Err(fail(self, mq, DictionaryErrorKind::AllocationFailed));
            }
            row.resize(stride, 0);
            row.fill(0);
        }
        for _ in 0..height {
            if self.cancellation.is_cancelled() {
                return Err(fail(self, mq, DictionaryErrorKind::Cancelled));
            }
            for x in 0..width {
                let [previous_two, previous_one, current] = &self.rows;
                let context =
                    BITMAP_BASE + template2_context(previous_two, previous_one, current, width, x);
                let bit = match mq.decode_bit(context) {
                    Ok(bit) => bit,
                    Err(error) => {
                        let offset = error.offset;
                        let mut located = fail(self, mq, DictionaryErrorKind::Mq(Box::new(error)));
                        located.offset = offset.unwrap_or(located.offset);
                        return Err(located);
                    }
                };
                if bit {
                    self.rows[2][x as usize / 8] |= 0x80 >> (x % 8);
                }
            }
            store.extend_from_slice(&self.rows[2]);
            // `symbol_geometry` checked that this symbol's packed bytes fit
            // `max_stored_bitmap_bytes` after every earlier symbol.
            self.progress.stored_bitmap_bytes += stride as u64;
            self.rows.rotate_left(1);
            self.rows[2].fill(0);
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

    /// T.88 §6.5.10: alternating export runs over the imported exports and
    /// the new symbols. The first IAEX decode precedes the repeat-until
    /// condition, so even a zero-total dictionary consumes one zero run.
    fn decode_exports(&mut self, unit: &mut Unit<'_, '_, C>) -> DictionaryResult<()> {
        let total = self.imported.len() + self.catalog.new_symbols.len();
        let mut index = 0usize;
        let mut export = false;
        loop {
            self.check_cancelled(unit)?;
            let runs =
                self.next_count(unit, self.progress.export_runs, "export run count overflow")?;
            self.cap(
                unit,
                "export runs",
                u64::from(self.budget.max_export_runs),
                u64::from(runs),
            )?;
            self.progress.export_runs = runs;
            let value = self.integer(unit, IntegerProcedure::Iaex)?;
            let length = self.signed(unit, value, "IAEX out of band")?;
            if length < 0 {
                return Err(self.malformed(unit, "negative export run"));
            }
            // `index` is a u32 symbol count; nonnegative IAEX is at most
            // i64::MAX, so this sum cannot overflow u64.
            let end = index as u64 + length as u64;
            if end > total as u64 {
                return Err(self.malformed(unit, "export run overshoot"));
            }
            let end = end as usize;
            if export {
                let next = self.catalog.exported_symbols.len() + (end - index);
                if next > self.header.exported_symbols as usize {
                    return Err(self.malformed(unit, "exported symbol total"));
                }
                for id in index..end {
                    let symbol = self.stored(id);
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
            return Err(self.malformed(unit, "exported symbol total"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
