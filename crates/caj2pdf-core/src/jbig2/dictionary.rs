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
    FieldCursor, FieldFault, SegmentHeader, SegmentSpan, Site,
    generic::template2_context,
    iaid::{IAID_BASE, checked_symbol_index, decode_iaid},
    integer::{BITMAP_BASE, IntegerProcedure, IntegerValue, decode_integer},
    mq::{ArithmeticSnapshot, CodedSpan, ContextBank, MqDecoder, MqTable},
    refinement::{
        ReferenceStore, RefinementDecoder, RefinementProgress, RefinementReference,
        RefinementRequest,
    },
    unsupported,
};
use crate::fallible::try_convert;
use crate::{Cancellation, Context, Error, Limits, Payload, RangedSource, Result};
use std::mem;

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

/// Fail with an unlocated limit error when `attempted` exceeds `limit`.
fn check_limit(resource: &'static str, limit: u64, attempted: u64) -> Result<()> {
    if attempted > limit {
        Err(Error::limit(resource, limit, attempted))
    } else {
        Ok(())
    }
}

struct HeaderCursor<'a> {
    header: &'a SegmentHeader,
    fields: FieldCursor,
}

impl HeaderCursor<'_> {
    fn error(&self, error: Error) -> Error {
        self.error_at(self.fields.at, error)
    }

    fn error_at(&self, offset: u64, error: Error) -> Error {
        Site {
            segment: self.header.number,
            offset,
        }
        .locate(error)
    }

    fn read<const N: usize, S: RangedSource, C: Cancellation>(
        &mut self,
        source: &mut S,
        name: &'static str,
        cancellation: &C,
    ) -> Result<[u8; N]> {
        let mut bytes = [0u8; N];
        let result = self.fields.fill(source, cancellation, &mut bytes);
        result.map_err(|fault| self.fault(fault, name))?;
        Ok(bytes)
    }

    fn fault(&self, fault: FieldFault, name: &'static str) -> Error {
        self.error(fault.error(self.fields.at, name))
    }
}

/// The source-independent checks before the data header is read. Keeping
/// them outside the generic reader shares one copy across every source and
/// cancellation type. Returns the data end.
fn data_header_bounds(
    header: &SegmentHeader,
    limits: &Limits,
    cancellation: &dyn Cancellation,
    source_size: u64,
) -> Result<u64> {
    let site = Site {
        segment: header.number,
        offset: header.data.offset,
    };
    if cancellation.is_cancelled() {
        return Err(site.locate(Error::cancelled()));
    }
    if header.segment_type != 0 {
        return Err(site.unsupported("segment type"));
    }
    site.cap(
        "dictionary data bytes",
        limits.max_input_bytes,
        header.data.length,
    )?;
    let end = header
        .data
        .offset
        .checked_add(header.data.length)
        .ok_or_else(|| site.malformed("data end overflow"))?;
    if end > source_size {
        return Err(site.malformed("data outside source"));
    }
    Ok(end)
}

/// Parse only the dictionary segment-data header, including conditional AT
/// fields, within `header.data`, which the segment directory has already
/// framed. This never initializes MQ or writes output.
/// `ArithmeticRefinementAggregate` is a classifier result, not a promise that
/// every such mode is decoded. The new and exported symbol counts are each
/// bounded by `Limits::max_symbols`.
pub fn read_dictionary_data_header<S: RangedSource, C: Cancellation>(
    source: &mut S,
    header: &SegmentHeader,
    limits: &Limits,
    cancellation: &C,
) -> Result<DictionaryDataHeader> {
    let end = data_header_bounds(header, limits, cancellation, source.size())?;
    let mut cursor = HeaderCursor {
        header,
        fields: FieldCursor {
            start: header.data.offset,
            at: header.data.offset,
            end,
            fetched: 0,
            request_bytes: limits.io_chunk_bytes.max(1),
        },
    };
    let flags = u16::from_be_bytes(cursor.read(source, "dictionary flags", cancellation)?);
    let flags_error = |cursor: &HeaderCursor<'_>, reason| {
        cursor.error_at(header.data.offset, Error::invalid(reason))
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
                Error::invalid("adaptive pixel references undecoded pixel"),
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
    if new_symbols > limits.max_symbols {
        return Err(cursor.error_at(
            new_offset,
            Error::limit(
                "new symbols",
                u64::from(limits.max_symbols),
                u64::from(new_symbols),
            ),
        ));
    }
    if exported_symbols > limits.max_symbols {
        return Err(cursor.error_at(
            exported_offset,
            Error::limit(
                "exported symbols",
                u64::from(limits.max_symbols),
                u64::from(exported_symbols),
            ),
        ));
    }
    let body_length = end - cursor.fields.at;
    if !huffman && body_length < 2 {
        return Err(cursor.error(
            Error::truncated(cursor.fields.at, 2, body_length).because("MQ body terminal pair"),
        ));
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
    site: Site,
    segment: &SegmentHeader,
    imported: ImportedDictionary<'_>,
    store: (u64, u64),
) -> Result<()> {
    let (store_size, store_base) = store;
    let bad = |reason| site.malformed(reason);
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
    let mut next_new = 0usize;
    for exported in &report.catalog.exported_symbols {
        let matching = report.catalog.new_symbols[next_new..]
            .iter()
            .position(|candidate| *candidate == exported.symbol)
            .ok_or_else(|| bad("imported exports do not follow new-symbol order"))?;
        next_new += matching + 1;
    }
    let mut previous_end = 0;
    for exported in &report.catalog.exported_symbols {
        let descriptor = exported.symbol;
        if descriptor.width == 0 || descriptor.height == 0 {
            return Err(bad("zero imported bitmap dimension"));
        }
        let stride = u64::from(descriptor.width).div_ceil(8);
        let bytes = stride * u64::from(descriptor.height);
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
}

/// The source-independent profile and resource checks between the data
/// header and MQ initialization, shared by every decoder instantiation.
/// `stores` are the imported store size and base, then the new store's.
fn check_header(
    segment: &SegmentHeader,
    header: &DictionaryDataHeader,
    import: Option<ImportedDictionary<'_>>,
    stores: [(u64, u64); 2],
    context_count: usize,
    limits: &Limits,
) -> Result<Plan> {
    let site = Site {
        segment: segment.number,
        offset: header.body.offset,
    };
    let unsupported = |feature| site.unsupported(feature);
    let malformed = |reason| site.malformed(reason);
    let refine = match header.mode {
        DictionaryMode::ArithmeticDirect => false,
        DictionaryMode::ArithmeticRefinementAggregate => true,
        DictionaryMode::HuffmanDirect | DictionaryMode::HuffmanRefinementAggregate => {
            return Err(unsupported("Huffman symbol dictionary"));
        }
    };
    if !refine && header.template != 2 {
        return Err(unsupported("dictionary generic template"));
    }
    if header.bitmap_context_used || header.bitmap_context_retained {
        return Err(unsupported("bitmap context carry"));
    }
    if refine && (header.flags != 0x1802 || header.at[0] != (2, -1)) {
        return Err(unsupported("second dictionary flags or adaptive template"));
    }
    if segment.page_association != 1 {
        return Err(unsupported("dictionary page association"));
    }
    if stores[1].1 != stores[1].0 {
        return Err(site.malformed("new store base differs from the store length"));
    }
    let imported_exports = match import {
        _ if !refine => {
            if !segment.referred_to.is_empty() {
                return Err(unsupported("imported dictionary references"));
            }
            // The parsed template-2 AT was already checked as backwards-only.
            if header.at[0] != (2, -1) {
                return Err(Site {
                    offset: segment.data.offset + 2,
                    ..site
                }
                .unsupported("adaptive pixel"));
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
                Site {
                    offset: segment.data.offset,
                    ..site
                },
                segment,
                import,
                stores[0],
            )?;
            import.report.catalog.exported_symbols.len() as u64
        }
    };
    // Imported and declared counts are each bounded by u32 header fields.
    let total = imported_exports + u64::from(header.new_symbols);
    if refine {
        site.cap("total symbols", u64::from(limits.max_symbols), total)?;
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
    // Both counts are u32 header fields, so the byte total fits u64.
    let new_metadata = u64::from(header.new_symbols) * mem::size_of::<SymbolDescriptor>() as u64;
    let export_metadata =
        u64::from(header.exported_symbols) * mem::size_of::<StoredSymbol>() as u64;
    site.cap(
        "catalog allocation bytes",
        limits.max_allocation_bytes,
        new_metadata + export_metadata,
    )?;
    Ok(Plan { refine, code_len })
}

fn reserve_catalog<T>(count: usize, site: Site, limits: &Limits) -> Result<Vec<T>> {
    let mut entries = Vec::new();
    entries.try_reserve_exact(count).map_err(|_| {
        site.locate(limits.allocation_refused(
            "catalog allocation bytes",
            (count as u64).saturating_mul(mem::size_of::<T>() as u64),
        ))
    })?;
    Ok(entries)
}

/// Checked dimensions of one new symbol: width, height, pixels, and packed
/// bytes.
type SymbolGeometry = (u32, u32, u64, u64);

/// Validate one decoded symbol size before any bitmap work. Not generic, so
/// every decoder instantiation shares it; the caller locates the returned
/// error at the current MQ offset.
fn symbol_geometry(width: i64, height: i64, limits: &Limits) -> Result<SymbolGeometry> {
    if width < 0 || height < 0 {
        return Err(Error::invalid("negative symbol dimension"));
    }
    if width == 0 || height == 0 {
        return Err(unsupported("zero-dimension symbol bitmap"));
    }
    let width = try_convert(width, Error::invalid("symbol width exceeds 32 bits"))?;
    let height = try_convert(height, Error::invalid("symbol height exceeds 32 bits"))?;
    // A product of two u32 dimensions fits u64 exactly.
    let pixels = u64::from(width) * u64::from(height);
    check_limit("symbol pixels", limits.max_image_pixels, pixels)?;
    // The maximum stride is 2^29 bytes, so this product fits u64.
    let bytes = u64::from(width).div_ceil(8) * u64::from(height);
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
    ) -> Result<Self> {
        let header = read_dictionary_data_header(&mut { input }, segment, limits, cancellation)?;
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
        )?;
        let site = Site {
            segment: segment.number,
            offset: header.body.offset,
        };
        let new_symbols = reserve_catalog(header.new_symbols as usize, site, limits)?;
        let exported_symbols = reserve_catalog(header.exported_symbols as usize, site, limits)?;
        // T.88 §7.4.2.2 resets all arithmetic-integer statistics at each new
        // dictionary, and no bitmap context reuse is accepted.
        contexts.reset();
        let span = CodedSpan {
            offset: header.body.offset,
            length: header.body.length,
        };
        let mq = MqDecoder::new(input, span, table, contexts, limits)
            .map_err(|error| site.locate(error))?;
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
    pub fn decode(self) -> Result<DictionaryReport> {
        self.decode_inner(false)
    }

    /// HN/C8 empty dictionaries may contain only the MQ terminal marker,
    /// omitting the zero IAEX run required by the strict T.88 procedure.
    /// Header validation, zero symbol counts and the exact marker are still
    /// required; an ordinary dictionary never takes this path.
    pub(crate) fn decode_hnc8(self) -> Result<DictionaryReport> {
        self.decode_inner(true)
    }

    fn decode_inner(mut self, allow_empty_body: bool) -> Result<DictionaryReport> {
        let site = Site {
            segment: self.segment,
            offset: self.mq.snapshot().input_offset,
        };
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
            )
            .map_err(|error| site.locate(error))?;
            Unit::Refined(Box::new(host))
        } else {
            Unit::Direct {
                mq: &mut self.mq,
                store: &mut *self.stores.new,
            }
        };
        let empty_body = allow_empty_body
            && session.imported.is_empty()
            && session.header.new_symbols == 0
            && session.header.exported_symbols == 0
            && session.header.body.length == 2;
        let decoded = if empty_body {
            session.check_cancelled(&unit)
        } else {
            session.decode_all(&mut unit)
        };
        if let Unit::Refined(host) = &unit {
            self.progress.refinement = host.progress();
        }
        drop(unit);
        decoded?;
        let expected = self.mq.snapshot().symbols_decoded;
        let offset = self.mq.snapshot().input_offset;
        let snapshot = self.mq.finish(expected).map_err(|error| {
            Site {
                segment: self.segment,
                offset,
            }
            .locate(error)
        })?;
        self.progress.mq = Some(snapshot);
        Ok(DictionaryReport {
            header: self.header,
            catalog: self.catalog,
            progress: self.progress,
        })
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
    progress: &'s mut DictionaryProgress,
    catalog: &'s mut DictionaryCatalog,
    /// Two previous rows and the current row of a direct bitmap.
    rows: [Vec<u8>; 3],
}

impl<C: Cancellation> Session<'_, C> {
    /// Locate an unlocated error at the next MQ byte of the coding unit.
    fn locate(&self, mq: Option<ArithmeticSnapshot>, error: Error) -> Error {
        error.or_at(
            mq.map_or(self.header.body.offset, |snapshot| snapshot.input_offset),
            Context::Jbig2 {
                segment: Some(self.segment),
            },
        )
    }

    fn error(&self, unit: &Unit<'_, '_, C>, error: Error) -> Error {
        match unit {
            Unit::Direct { mq, .. } => self.locate(Some(mq.snapshot()), error),
            Unit::Refined(host) => self.locate(host.progress().mq, error),
        }
    }

    fn malformed(&self, unit: &Unit<'_, '_, C>, reason: &'static str) -> Error {
        self.error(unit, Error::invalid(reason))
    }

    fn cap(
        &self,
        unit: &Unit<'_, '_, C>,
        resource: &'static str,
        maximum: u64,
        attempted: u64,
    ) -> Result<()> {
        check_limit(resource, maximum, attempted).map_err(|error| self.error(unit, error))
    }

    fn check_cancelled(&self, unit: &Unit<'_, '_, C>) -> Result<()> {
        if self.cancellation.is_cancelled() {
            Err(self.error(unit, Error::cancelled()))
        } else {
            Ok(())
        }
    }

    /// The next value of a `u32` counter, refusing a wrap.
    fn next_count(&self, unit: &Unit<'_, '_, C>, value: u32, field: &'static str) -> Result<u32> {
        value
            .checked_add(1)
            .ok_or_else(|| self.malformed(unit, field))
    }

    fn integer(
        &self,
        unit: &mut Unit<'_, '_, C>,
        procedure: IntegerProcedure,
    ) -> Result<IntegerValue> {
        decode_integer(unit.mq(), procedure).map_err(|error| self.error(unit, error))
    }

    fn iaid(&self, unit: &mut Unit<'_, '_, C>) -> Result<u64> {
        decode_iaid(unit.mq(), self.plan.code_len).map_err(|error| self.error(unit, error))
    }

    fn signed(
        &self,
        unit: &Unit<'_, '_, C>,
        value: IntegerValue,
        field: &'static str,
    ) -> Result<i64> {
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

    /// Check one decoded size before any bitmap work, including the three
    /// working rows of a direct bitmap; a refinement host checks its own.
    fn geometry(&self, unit: &Unit<'_, '_, C>, width: i64, height: i64) -> Result<SymbolGeometry> {
        let geometry =
            symbol_geometry(width, height, self.limits).map_err(|error| self.error(unit, error))?;
        if let Unit::Direct { .. } = unit {
            let scratch = u64::from(geometry.0).div_ceil(8) * 3;
            self.cap(
                unit,
                "row scratch bytes",
                self.limits.max_allocation_bytes,
                scratch,
            )?;
        }
        Ok(geometry)
    }

    fn decode_all(&mut self, unit: &mut Unit<'_, '_, C>) -> Result<()> {
        self.decode_symbols(unit)?;
        self.decode_exports(unit)?;
        self.check_cancelled(unit)
    }

    /// T.88 §6.5.5 steps 4b–4c: height classes of new symbols, each either a
    /// direct bitmap or the single-reference refinement of an earlier symbol.
    fn decode_symbols(&mut self, unit: &mut Unit<'_, '_, C>) -> Result<()> {
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
                u64::from(self.limits.max_symbols),
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
    ) -> Result<SymbolDescriptor> {
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
            return Err(self.error(unit, unsupported("REFAGGNINST aggregation")));
        }
        self.progress.iaai.single_reference += 1;
        let raw_id = self.iaid(unit)?;
        let active = self.imported.len() + self.catalog.new_symbols.len();
        let index = checked_symbol_index(raw_id, active as u64, active)
            .map_err(|_| self.malformed(unit, "future, self, or absent symbol ID"))?;
        let reference = self.stored(index);
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
            Err(error) => Err(self.error(unit, error)),
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
    ) -> Result<SymbolDescriptor> {
        let fail =
            |session: &Self, mq: &MqDecoder<'_>, error| session.locate(Some(mq.snapshot()), error);
        let relative_store_offset = self.progress.stored_bitmap_bytes;
        // The store is in memory, so its length fits a `u64`.
        let attempted = (store.len() as u64).saturating_add(bytes);
        if attempted > self.limits.max_allocation_bytes {
            return Err(fail(
                self,
                mq,
                Error::limit(
                    "symbol store bytes",
                    self.limits.max_allocation_bytes,
                    attempted,
                ),
            ));
        }
        // `bytes` fits the allocation limit, hence a `usize` on this target.
        if store.try_reserve(bytes as usize).is_err() {
            let refused = self
                .limits
                .allocation_refused("symbol store bytes", attempted);
            return Err(fail(self, mq, refused));
        }
        // At most 2^29 bytes; even wasm32's usize can represent it.
        let stride = width.div_ceil(8) as usize;
        for row in &mut self.rows {
            if row.len() < stride && row.try_reserve_exact(stride - row.len()).is_err() {
                let refused = self
                    .limits
                    .allocation_refused("row scratch bytes", stride as u64);
                return Err(fail(self, mq, refused));
            }
            row.resize(stride, 0);
            row.fill(0);
        }
        for _ in 0..height {
            if self.cancellation.is_cancelled() {
                return Err(fail(self, mq, Error::cancelled()));
            }
            for x in 0..width {
                let [previous_two, previous_one, current] = &self.rows;
                let context =
                    BITMAP_BASE + template2_context(previous_two, previous_one, current, width, x);
                let bit = match mq.decode_bit(context) {
                    Ok(bit) => bit,
                    Err(error) => return Err(fail(self, mq, error)),
                };
                if bit {
                    self.rows[2][x as usize / 8] |= 0x80 >> (x % 8);
                }
            }
            store.extend_from_slice(&self.rows[2]);
            // The store, and so this running count, fits the allocation limit.
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
    fn decode_exports(&mut self, unit: &mut Unit<'_, '_, C>) -> Result<()> {
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
                u64::from(self.limits.max_symbols),
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
