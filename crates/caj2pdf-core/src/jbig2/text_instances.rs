// SPDX-License-Identifier: MIT

//! Pull decoding of bounded arithmetic text-region instances (T.88 §6.4.5).
//!
//! The caller owns both dictionary stores and the temporary refinement store,
//! all in memory. This module emits checked placements; it does not allocate
//! or compose a region bitmap. It contains no normative MQ probability states.

use super::{
    SegmentHeader, Site,
    dictionary::{
        DictionaryMode, DictionaryReport, StoredSymbol, SymbolDescriptor, SymbolStore,
        coding_unit_contexts, symbol_code_length,
    },
    iaid::{checked_symbol_index, decode_iaid},
    integer::{IntegerProcedure, IntegerValue, decode_integer},
    mq::{ArithmeticSnapshot, CodedSpan, ContextBank, MqDecoder, MqTable},
    refinement::{
        ReferenceStore, RefinementDecoder, RefinementProgress, RefinementReference,
        RefinementRequest,
    },
    text::{
        ReferenceCorner, TextHeaderPolicy, TextRegionHeader, read_text_region_header_with_policy,
    },
};
use crate::{Cancellation, Context, Error, Limits, Payload, Result};

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

/// MQ and refinement counters and the next semantic decision.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextInstanceProgress {
    pub completed_instances: u32,
    pub ri_zero: u32,
    pub ri_one: u32,
    pub strips: u32,
    pub total_instance_pixels: u64,
    pub decision: TextDecision,
    pub header_bytes_fetched: u64,
    pub refinement: RefinementProgress,
    pub mq: Option<ArithmeticSnapshot>,
}

fn validate_descriptor(
    site: Site,
    stored: StoredSymbol,
    expected_store: SymbolStore,
    expected_base: u64,
    source_size: u64,
) -> Result<()> {
    let bad = |reason| site.malformed(reason);
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
    let absolute_end = expected_base
        .checked_add(relative_end)
        .ok_or_else(|| bad("dictionary absolute store end overflow"))?;
    if absolute_end > source_size {
        return Err(bad("dictionary bitmap outside its store"));
    }
    Ok(())
}

/// A coordinate within the T.88 signed 32-bit range.
fn cap_coordinate(value: i64) -> Result<i64> {
    if i32::try_from(value).is_err() {
        Err(Error::invalid(
            "text coordinate outside T.88 signed 32-bit range",
        ))
    } else {
        Ok(value)
    }
}

fn checked_coordinate(value: Option<i64>) -> Result<i64> {
    cap_coordinate(value.ok_or(Error::invalid("coordinate overflow"))?)
}

fn geometry(
    s: i64,
    t: i64,
    width: u32,
    height: u32,
    corner: ReferenceCorner,
    transposed: bool,
) -> Result<(i64, i64, i64)> {
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
    let s = checked_coordinate(s.checked_add(pre))?;
    let (mut x, mut y) = if transposed { (t, s) } else { (s, t) };
    if right {
        x = checked_coordinate(x.checked_sub(i64::from(width) - 1))?;
    }
    if bottom {
        y = checked_coordinate(y.checked_sub(i64::from(height) - 1))?;
    }
    let next_s = checked_coordinate(s.checked_add(post))?;
    Ok((cap_coordinate(x)?, cap_coordinate(y)?, next_s))
}

fn refined_geometry(
    reference: SymbolDescriptor,
    rdw: i64,
    rdh: i64,
    rdx: i64,
    rdy: i64,
) -> Result<(u32, u32, i32, i32)> {
    let width = i64::from(reference.width)
        .checked_add(rdw)
        .ok_or(Error::invalid("refined width overflow"))?;
    let height = i64::from(reference.height)
        .checked_add(rdh)
        .ok_or(Error::invalid("refined height overflow"))?;
    if width <= 0 || height <= 0 || width > i64::from(u32::MAX) || height > i64::from(u32::MAX) {
        return Err(Error::invalid("nonpositive or oversized refined geometry"));
    }
    // T.88 Table 12 uses mathematical floor, including negative odd deltas.
    let dx = rdw
        .div_euclid(2)
        .checked_add(rdx)
        .and_then(|value| i32::try_from(value).ok())
        .ok_or(Error::invalid("refinement X offset overflow"))?;
    let dy = rdh
        .div_euclid(2)
        .checked_add(rdy)
        .and_then(|value| i32::try_from(value).ok())
        .ok_or(Error::invalid("refinement Y offset overflow"))?;
    Ok((width as u32, height as u32, dx, dy))
}

/// One MQ coding unit and sequential pull cursor. `next` must be called until
/// it returns `None` to check the terminal pair. On any failure, discard all
/// refined bitmap output for this region.
pub struct TextInstanceDecoder<'a, C: Cancellation> {
    mq: MqDecoder<'a>,
    imported: &'a [u8],
    new: &'a [u8],
    refined: &'a mut Vec<u8>,
    refined_base: u64,
    dictionary: &'a [StoredSymbol],
    header: TextRegionHeader,
    segment: u32,
    code_len: u32,
    limits: &'a Limits,
    cancellation: &'a C,
    progress: TextInstanceProgress,
    strip_t: i64,
    first_s: i64,
    current_s: i64,
    initialized: bool,
    strip_open: bool,
    complete: bool,
}

impl<'a, C: Cancellation> TextInstanceDecoder<'a, C> {
    /// Reparse the segment header from `input`, which must hold the whole
    /// segment data, and validate both dictionary stores before MQ input.
    /// Refined bitmaps are appended to `refined`, whose length must be
    /// `refined_base`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        input: Payload<'a>,
        segment: &SegmentHeader,
        parsed: TextRegionHeader,
        dictionary_segment: &SegmentHeader,
        dictionary: &'a DictionaryReport,
        imported: &'a [u8],
        imported_store_base: u64,
        new: &'a [u8],
        new_store_base: u64,
        refined: &'a mut Vec<u8>,
        refined_base: u64,
        table: &'a MqTable,
        contexts: &'a mut ContextBank,
        limits: &'a Limits,
        cancellation: &'a C,
    ) -> Result<Self> {
        Self::new_with_header_policy(
            input,
            segment,
            parsed,
            dictionary_segment,
            dictionary,
            imported,
            imported_store_base,
            new,
            new_store_base,
            refined,
            refined_base,
            table,
            contexts,
            limits,
            cancellation,
            TextHeaderPolicy::Strict,
        )
    }

    /// Revalidate a text header using an explicit HN/C8 compatibility policy.
    /// The original `new` constructor always keeps strict T.88 validation.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_header_policy(
        input: Payload<'a>,
        segment: &SegmentHeader,
        parsed: TextRegionHeader,
        dictionary_segment: &SegmentHeader,
        dictionary: &'a DictionaryReport,
        imported: &'a [u8],
        imported_store_base: u64,
        new: &'a [u8],
        new_store_base: u64,
        refined: &'a mut Vec<u8>,
        refined_base: u64,
        table: &'a MqTable,
        contexts: &'a mut ContextBank,
        limits: &'a Limits,
        cancellation: &'a C,
        policy: TextHeaderPolicy,
    ) -> Result<Self> {
        let checked = read_text_region_header_with_policy(
            &mut { input },
            segment,
            dictionary_segment,
            limits,
            cancellation,
            policy,
        )?;
        let at = checked.body.offset;
        let site = Site {
            segment: segment.number,
            offset: at,
        };
        let bad = |reason| site.malformed(reason);
        if checked != parsed {
            return Err(bad("supplied text header differs from source"));
        }
        if parsed.flags.huffman || parsed.huffman_flags.is_some() {
            return Err(site.unsupported("Huffman text region"));
        }
        if parsed.flags.refine
            && (parsed.flags.refinement_template != 1 || parsed.refinement_at.is_some())
        {
            return Err(site.unsupported("refinement template 0"));
        }
        // `read_text_region_header` has just rechecked the exact segment
        // type, reference, order, and page relation against these arguments.
        let dictionary_header = dictionary.header;
        let dictionary_end = dictionary_segment
            .data
            .offset
            .checked_add(dictionary_segment.data.length)
            .ok_or_else(|| bad("dictionary segment end overflow"))?;
        let body_end = dictionary_header
            .body
            .offset
            .checked_add(dictionary_header.body.length)
            .ok_or_else(|| bad("dictionary body end overflow"))?;
        let expected_body = dictionary_segment
            .data
            .offset
            .checked_add(dictionary_header.header_bytes)
            .ok_or_else(|| bad("dictionary body start overflow"))?;
        if dictionary_header.mode != DictionaryMode::ArithmeticRefinementAggregate
            || dictionary_header.flags != 0x1802
            || dictionary_header.body.offset != expected_body
            || body_end != dictionary_end
            || dictionary_header.new_symbols as usize != dictionary.catalog.new_symbols.len()
            || dictionary_header.exported_symbols as usize
                != dictionary.catalog.exported_symbols.len()
            || dictionary.progress.completed_symbols != dictionary_header.new_symbols
            || dictionary.progress.mq.is_none()
        {
            return Err(bad("dictionary is not a complete ordered report"));
        }
        if parsed.instances != 0 && dictionary.catalog.exported_symbols.is_empty() {
            return Err(bad("nonempty text region with no symbols"));
        }
        if refined.len() as u64 != refined_base {
            return Err(bad("refined store base differs from the store length"));
        }
        if imported_store_base > imported.len() as u64 || new_store_base > new.len() as u64 {
            return Err(bad("dictionary store base outside its store"));
        }
        let mut last_catalog_end = 0u64;
        for descriptor in &dictionary.catalog.new_symbols {
            if descriptor.relative_store_offset < last_catalog_end {
                return Err(bad("unordered or overlapping new symbols"));
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
                new.len() as u64,
            )?;
            last_catalog_end = descriptor.relative_store_offset + descriptor.stored_bytes;
        }
        let mut seen_new = false;
        let mut imported_end = 0u64;
        let mut new_end = 0u64;
        for stored in &dictionary.catalog.exported_symbols {
            let (source_size, expected_base) = match stored.store {
                SymbolStore::Imported => {
                    if seen_new {
                        return Err(bad("imported export follows new export"));
                    }
                    (imported.len() as u64, imported_store_base)
                }
                SymbolStore::New => {
                    seen_new = true;
                    (new.len() as u64, new_store_base)
                }
            };
            validate_descriptor(site, *stored, stored.store, expected_base, source_size)?;
            let end = stored.symbol.relative_store_offset + stored.symbol.stored_bytes;
            let prior_end = match stored.store {
                SymbolStore::Imported => &mut imported_end,
                SymbolStore::New => &mut new_end,
            };
            if stored.symbol.relative_store_offset < *prior_end {
                return Err(bad("unordered or overlapping exported symbols"));
            }
            *prior_end = end;
            if stored.store == SymbolStore::New {
                let index = dictionary
                    .catalog
                    .new_symbols
                    .binary_search_by_key(&stored.symbol.relative_store_offset, |descriptor| {
                        descriptor.relative_store_offset
                    })
                    .map_err(|_| bad("new export absent from new catalog"))?;
                if dictionary.catalog.new_symbols[index] != stored.symbol {
                    return Err(bad("new export order differs from catalog"));
                }
            }
        }
        let code_len = symbol_code_length(dictionary.catalog.exported_symbols.len() as u64);
        if coding_unit_contexts(code_len) != Some(contexts.len()) {
            return Err(bad("IAID width or GR context layout mismatch"));
        }
        // A fresh text region resets every arithmetic statistic.
        contexts.reset();
        let mq = MqDecoder::new(
            input,
            CodedSpan {
                offset: parsed.body.offset,
                length: parsed.body.length,
            },
            table,
            contexts,
            limits,
        )
        .map_err(|error| site.locate(error))?;
        Ok(Self {
            mq,
            imported,
            new,
            refined,
            refined_base,
            dictionary: &dictionary.catalog.exported_symbols,
            header: parsed,
            segment: segment.number,
            code_len,
            limits,
            cancellation,
            progress: TextInstanceProgress {
                header_bytes_fetched: checked.header_bytes,
                ..TextInstanceProgress::default()
            },
            strip_t: 0,
            first_s: 0,
            current_s: 0,
            initialized: false,
            strip_open: false,
            complete: false,
        })
    }

    pub fn progress(&self) -> TextInstanceProgress {
        let mut progress = self.progress;
        progress.mq = Some(self.mq.snapshot());
        progress
    }

    /// The refined store, which holds every refined bitmap returned so far.
    pub fn refined_store(&self) -> &[u8] {
        self.refined
    }

    /// The validated region header owned by this instance stream.
    pub fn header(&self) -> TextRegionHeader {
        self.header
    }

    /// The type-6 segment number associated with this validated stream.
    pub fn segment(&self) -> u32 {
        self.segment
    }

    /// Locate an unlocated error at the next MQ byte.
    fn error(&self, error: Error) -> Error {
        error.or_at(
            self.mq.snapshot().input_offset,
            Context::Jbig2 {
                segment: Some(self.segment),
            },
        )
    }

    fn cap(&self, resource: &'static str, limit: u64, attempted: u64) -> Result<()> {
        if attempted > limit {
            Err(self.error(Error::limit(resource, limit, attempted)))
        } else {
            Ok(())
        }
    }

    fn coordinate(&self, value: Option<i64>) -> Result<i64> {
        checked_coordinate(value).map_err(|error| self.error(error))
    }

    fn check_cancelled(&self) -> Result<()> {
        if self.cancellation.is_cancelled() {
            Err(self.error(Error::cancelled()))
        } else {
            Ok(())
        }
    }

    fn signed(&mut self, procedure: IntegerProcedure, decision: TextDecision) -> Result<i64> {
        self.progress.decision = decision;
        match decode_integer(&mut self.mq, procedure) {
            Ok(IntegerValue::Signed(value)) => Ok(value),
            Ok(IntegerValue::OutOfBand) => {
                Err(self.error(Error::invalid("unexpected arithmetic OOB")))
            }
            Err(error) => Err(self.error(error)),
        }
    }

    fn decode_refined(
        &mut self,
        reference: StoredSymbol,
        request: RefinementRequest,
    ) -> Result<SymbolDescriptor> {
        let store = match reference.store {
            SymbolStore::Imported => self.imported,
            SymbolStore::New => self.new,
        };
        let mut host = RefinementDecoder::new_continuing(
            &mut self.mq,
            self.refined,
            self.limits,
            self.cancellation,
            self.progress.refinement,
        )?;
        let report = host.decode_bitmap(ReferenceStore::Other(store), request);
        self.progress.refinement = host.progress();
        Ok(report?.target)
    }

    /// Decode one placement. `None` means the declared count and exact MQ
    /// terminal were checked; subsequent calls return `None`.
    pub fn next_instance(&mut self) -> Result<Option<TextInstance>> {
        if self.complete {
            return Ok(None);
        }
        self.decode_instance()
    }

    fn decode_instance(&mut self) -> Result<Option<TextInstance>> {
        self.check_cancelled()?;
        if !self.initialized {
            let initial = self.signed(IntegerProcedure::Iadt, TextDecision::InitialStripT)?;
            self.strip_t = self.coordinate(initial.checked_neg())?;
            self.initialized = true;
        }
        loop {
            if self.progress.completed_instances == self.header.instances {
                self.progress.decision = TextDecision::Terminal;
                let expected = self.mq.snapshot().symbols_decoded;
                let snapshot = self
                    .mq
                    .finish(expected)
                    .map_err(|error| self.error(error))?;
                self.progress.mq = Some(snapshot);
                self.check_cancelled()?;
                self.complete = true;
                self.progress.decision = TextDecision::Complete;
                return Ok(None);
            }
            if !self.strip_open {
                // Every strip places at least one instance, so the strip
                // count stays at most the u32 instance count.
                self.progress.strips += 1;
                let dt = self.signed(IntegerProcedure::Iadt, TextDecision::StripDeltaT)?;
                // Annex A.2 emits <2^33 magnitude and SBSTRIPS <= 8, so
                // multiplication stays below 2^36. The accumulated T still
                // requires a checked add and the signed 32-bit range.
                self.strip_t = self.coordinate(
                    self.strip_t
                        .checked_add(dt * i64::from(self.header.flags.strips())),
                )?;
                let dfs = self.signed(IntegerProcedure::Iafs, TextDecision::FirstS)?;
                self.first_s = self.coordinate(self.first_s.checked_add(dfs))?;
                self.current_s = self.first_s;
                self.strip_open = true;
            } else {
                self.progress.decision = TextDecision::DeltaS;
                match decode_integer(&mut self.mq, IntegerProcedure::Iads) {
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
                        return Err(self.error(error));
                    }
                }
            }
            let within = if self.header.flags.strips() == 1 {
                0
            } else {
                let value = self.signed(IntegerProcedure::Iait, TextDecision::WithinStripT)?;
                if value < 0 || value >= i64::from(self.header.flags.strips()) {
                    return Err(self.error(Error::invalid("IAIT outside strip")));
                }
                value
            };
            let t = self.coordinate(self.strip_t.checked_add(within))?;
            self.progress.decision = TextDecision::SymbolId;
            let raw_id =
                decode_iaid(&mut self.mq, self.code_len).map_err(|error| self.error(error))?;
            let id =
                checked_symbol_index(raw_id, self.dictionary.len() as u64, self.dictionary.len())
                    .map_err(|_| self.error(Error::invalid("IAID outside exported catalog")))?;
            let reference = self.dictionary[id];
            let mut bitmap = TextBitmap::Stored(reference);
            let mut width = reference.symbol.width;
            let mut height = reference.symbol.height;
            let mut refinement_request = None;
            let ri = if self.header.flags.refine {
                self.signed(IntegerProcedure::Iari, TextDecision::RefinementFlag)?
            } else {
                0
            };
            if ri != 0 && ri != 1 {
                return Err(self.error(Error::invalid("IARI is not a bit")));
            }
            if ri == 1 {
                let rdw = self.signed(IntegerProcedure::Iardw, TextDecision::DeltaWidth)?;
                let rdh = self.signed(IntegerProcedure::Iardh, TextDecision::DeltaHeight)?;
                let rdx = self.signed(IntegerProcedure::Iardx, TextDecision::DeltaX)?;
                let rdy = self.signed(IntegerProcedure::Iardy, TextDecision::DeltaY)?;
                let (target_width, target_height, reference_dx, reference_dy) =
                    refined_geometry(reference.symbol, rdw, rdh, rdx, rdy)
                        .map_err(|error| self.error(error))?;
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
            self.cap("instance pixels", self.limits.max_image_pixels, pixels)?;
            let total = self.progress.total_instance_pixels.saturating_add(pixels);
            let (x, y, next_s) = geometry(
                self.current_s,
                t,
                width,
                height,
                self.header.flags.reference_corner,
                self.header.flags.transposed,
            )
            .map_err(|error| self.error(error))?;
            if let Some(request) = refinement_request {
                self.progress.decision = TextDecision::RefinementBitmap;
                let descriptor = self
                    .decode_refined(reference, request)
                    .map_err(|error| self.error(error))?;
                bitmap = TextBitmap::Refined {
                    store_base: self.refined_base,
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
