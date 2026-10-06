// SPDX-License-Identifier: MIT

//! Bounded composition of T.88 arithmetic text-region instance bitmaps.
//!
//! A text stream can place a later symbol above an earlier one, so the whole
//! packed region is composed in a caller-owned bitmap in memory. The bitmap
//! is complete only after the stream's terminal has been checked.

use super::{
    dictionary::{StoredSymbol, SymbolDescriptor, SymbolStore},
    text::{SymbolCombination, TextHeaderAnomaly, TextRegionHeader},
    text_instances::{TextBitmap, TextInstance, TextInstanceDecoder, TextInstanceError},
};
use crate::{Cancellation, Error, Limits, MAX_BUDGET_COUNT};
use std::{error, fmt};

/// Independent composition bounds. Counters are also limited to
/// `MAX_BUDGET_COUNT`, leaving room for checked per-call accounting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextComposeBudget {
    pub max_exported_symbols: u32,
    pub max_instances: u32,
    pub max_region_pixels: u64,
    /// The packed region bitmap, also capped by `Limits::max_allocation_bytes`.
    pub max_scratch_bytes: u64,
    pub max_symbol_bytes: u64,
    pub max_touched_pixels_per_instance: u64,
    pub max_total_touched_pixels: u64,
    pub max_work_units: u64,
    pub max_row_bytes: usize,
}

impl Default for TextComposeBudget {
    fn default() -> Self {
        Self {
            max_exported_symbols: 8192,
            max_instances: 1_000_000,
            max_region_pixels: 256 * 1024 * 1024,
            max_scratch_bytes: 128 * 1024 * 1024,
            max_symbol_bytes: 128 * 1024 * 1024,
            max_touched_pixels_per_instance: 12_000_000,
            max_total_touched_pixels: 1_000_000_000,
            max_work_units: 2_000_000_000,
            max_row_bytes: 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TextComposeStage {
    #[default]
    Preflight,
    Initialize,
    Instance,
    Complete,
}

/// `work_units` counts initialized bytes and visited pixels.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextComposeProgress {
    pub stage: TextComposeStage,
    pub completed_instances: u32,
    pub current_row: u32,
    pub touched_pixels: u64,
    pub work_units: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextComposeReport {
    /// Complete source header used by this composer. Page composition binds
    /// every placement and decoding field to its preflighted #3 segment.
    pub header: TextRegionHeader,
    pub width: u32,
    pub height: u32,
    pub row_stride: u32,
    pub packed_bytes: u64,
    /// Encoded Figure 36 flags, including any accepted nonconforming bit.
    pub text_flags_raw: u16,
    /// An accepted deviation, or `None` for a strictly valid text header.
    pub header_anomaly: Option<TextHeaderAnomaly>,
    pub progress: TextComposeProgress,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BitmapStore {
    Imported,
    New,
    Refined,
}

#[derive(Clone, Copy, Debug)]
struct CheckedEvent {
    store: BitmapStore,
    /// Store offset of the bitmap's first byte.
    start: u64,
    descriptor: SymbolDescriptor,
    x0: u64,
    y0: u64,
    x1: u64,
    y1: u64,
}

#[derive(Debug)]
pub enum TextComposeErrorKind {
    InvalidSpan(&'static str),
    Malformed(&'static str),
    LimitExceeded {
        resource: &'static str,
        limit: u64,
        attempted: u64,
    },
    AllocationFailed,
    Cancelled,
    Instance(Box<TextInstanceError>),
}

/// `offset` names the active region or store byte according to
/// `progress.stage` and `kind`; instance failures preserve their MQ offset.
#[derive(Debug)]
pub struct TextComposeError {
    pub segment: u32,
    pub offset: u64,
    pub progress: Box<TextComposeProgress>,
    pub kind: TextComposeErrorKind,
}

pub type TextComposeResult<T> = Result<T, TextComposeError>;

impl fmt::Display for TextComposeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "JBIG2 text composition segment {} at byte {} ({:?}, instance {}): ",
            self.segment, self.offset, self.progress.stage, self.progress.completed_instances
        )?;
        match &self.kind {
            TextComposeErrorKind::InvalidSpan(reason) => write!(f, "invalid span: {reason}"),
            TextComposeErrorKind::Malformed(reason) => write!(f, "malformed {reason}"),
            TextComposeErrorKind::LimitExceeded {
                resource,
                limit,
                attempted,
            } => write!(f, "{resource} limit {limit} exceeded by {attempted}"),
            TextComposeErrorKind::AllocationFailed => f.write_str("bitmap allocation failed"),
            TextComposeErrorKind::Cancelled => f.write_str("cancelled"),
            TextComposeErrorKind::Instance(error) => write!(f, "instance: {error}"),
        }
    }
}

impl error::Error for TextComposeError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.kind {
            TextComposeErrorKind::Instance(error) => Some(error),
            _ => None,
        }
    }
}

fn limited(resource: &'static str, limit: u64, attempted: u64) -> TextComposeErrorKind {
    TextComposeErrorKind::LimitExceeded {
        resource,
        limit,
        attempted,
    }
}

fn cap(resource: &'static str, limit: u64, attempted: u64) -> Result<(), TextComposeErrorKind> {
    if attempted > limit {
        Err(limited(resource, limit, attempted))
    } else {
        Ok(())
    }
}

fn descriptor_bytes(descriptor: SymbolDescriptor) -> Result<(u64, u64), TextComposeErrorKind> {
    if descriptor.width == 0 || descriptor.height == 0 {
        return Err(TextComposeErrorKind::Malformed("zero bitmap dimension"));
    }
    let stride = u64::from(descriptor.width).div_ceil(8);
    let bytes = stride * u64::from(descriptor.height);
    if u64::from(descriptor.row_stride) != stride || descriptor.stored_bytes != bytes {
        return Err(TextComposeErrorKind::Malformed(
            "noncanonical bitmap descriptor",
        ));
    }
    Ok((stride, bytes))
}

fn combine(target: bool, source: bool, operator: SymbolCombination) -> bool {
    match operator {
        SymbolCombination::Or => target | source,
        SymbolCombination::And => target & source,
        SymbolCombination::Xor => target ^ source,
        SymbolCombination::Xnor => target == source,
    }
}

fn padding_mask(width: u32) -> u8 {
    let used = width % 8;
    if used == 0 { 0xff } else { 0xff << (8 - used) }
}

/// One region composition into `bitmap`. After a failure the bitmap holds a
/// partial region; the caller must discard it.
///
/// The imported and new symbol stores are the dictionary stores the
/// instance decoder reads; refined bitmaps are read from the decoder's own
/// refined store.
pub struct TextComposer<'a, 'd, C: Cancellation> {
    header: TextRegionHeader,
    segment: u32,
    catalog: &'a [StoredSymbol],
    instances: &'a mut TextInstanceDecoder<'d, C>,
    imported: &'a [u8],
    imported_base: u64,
    new: &'a [u8],
    new_base: u64,
    refined_base: u64,
    bitmap: &'a mut Vec<u8>,
    cancellation: &'a C,
    budget: TextComposeBudget,
    row_stride: usize,
    packed_bytes: u64,
    progress: TextComposeProgress,
}

impl<'a, 'd, C: Cancellation> TextComposer<'a, 'd, C> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        segment: u32,
        header: TextRegionHeader,
        catalog: &'a [StoredSymbol],
        instances: &'a mut TextInstanceDecoder<'d, C>,
        imported: &'a [u8],
        imported_base: u64,
        new: &'a [u8],
        new_base: u64,
        refined_base: u64,
        bitmap: &'a mut Vec<u8>,
        limits: &Limits,
        cancellation: &'a C,
        budget: TextComposeBudget,
    ) -> TextComposeResult<Self> {
        let bad = |kind| TextComposeError {
            segment,
            offset: 0,
            progress: Box::new(TextComposeProgress::default()),
            kind,
        };
        limits.validate().map_err(|error| {
            bad(match error {
                Error::LimitExceeded {
                    resource,
                    limit,
                    attempted,
                } => limited(resource, limit, attempted),
                _ => TextComposeErrorKind::Malformed("invalid global limits"),
            })
        })?;
        if instances.segment() != segment || instances.header() != header {
            return Err(bad(TextComposeErrorKind::Malformed(
                "instance stream segment or header differs",
            )));
        }
        if header.segment != segment {
            return Err(bad(TextComposeErrorKind::Malformed(
                "text header segment differs from composer",
            )));
        }
        if header.region.width == 0 || header.region.height == 0 {
            return Err(bad(TextComposeErrorKind::Malformed(
                "zero region dimension",
            )));
        }
        if header.flags.huffman || header.flags.refinement_template != 1 && header.flags.refine {
            return Err(bad(TextComposeErrorKind::Malformed(
                "unsupported text stream profile",
            )));
        }
        let pixels = u64::from(header.region.width) * u64::from(header.region.height);
        cap("region pixels", budget.max_region_pixels, pixels).map_err(&bad)?;
        let stride = u64::from(header.region.width).div_ceil(8);
        let packed_bytes = stride * u64::from(header.region.height);
        cap("scratch bytes", budget.max_scratch_bytes, packed_bytes).map_err(&bad)?;
        cap(
            "region bitmap bytes",
            limits.max_allocation_bytes,
            packed_bytes,
        )
        .map_err(&bad)?;
        cap("row bytes", budget.max_row_bytes as u64, stride).map_err(&bad)?;
        cap(
            "exported symbols",
            u64::from(budget.max_exported_symbols),
            catalog.len() as u64,
        )
        .map_err(&bad)?;
        cap(
            "instances",
            u64::from(budget.max_instances),
            u64::from(header.instances),
        )
        .map_err(&bad)?;
        if header.instances > 0 && catalog.is_empty() {
            return Err(bad(TextComposeErrorKind::Malformed(
                "nonempty text region with no symbols",
            )));
        }
        for (name, count) in [
            ("region pixels budget", budget.max_region_pixels),
            ("scratch budget", budget.max_scratch_bytes),
            ("symbol budget", budget.max_symbol_bytes),
            (
                "per-instance touched pixels budget",
                budget.max_touched_pixels_per_instance,
            ),
            (
                "total touched pixels budget",
                budget.max_total_touched_pixels,
            ),
            ("work budget", budget.max_work_units),
        ] {
            cap(name, MAX_BUDGET_COUNT, count).map_err(&bad)?;
        }
        if imported_base > imported.len() as u64
            || new_base > new.len() as u64
            || refined_base > instances.refined_store().len() as u64
        {
            return Err(bad(TextComposeErrorKind::InvalidSpan(
                "bitmap store base beyond its store",
            )));
        }
        // The row-byte cap above is a `usize`, so `stride` fits this target.
        let row_stride = stride as usize;
        Ok(Self {
            header,
            segment,
            catalog,
            instances,
            imported,
            imported_base,
            new,
            new_base,
            refined_base,
            bitmap,
            cancellation,
            budget,
            row_stride,
            packed_bytes,
            progress: TextComposeProgress::default(),
        })
    }

    pub fn progress(&self) -> TextComposeProgress {
        self.progress
    }

    fn error(&self, offset: u64, kind: TextComposeErrorKind) -> TextComposeError {
        TextComposeError {
            segment: self.segment,
            offset,
            progress: Box::new(self.progress),
            kind,
        }
    }

    fn check_cancelled(&self, offset: u64) -> TextComposeResult<()> {
        if self.cancellation.is_cancelled() {
            Err(self.error(offset, TextComposeErrorKind::Cancelled))
        } else {
            Ok(())
        }
    }

    fn check_cap(
        &self,
        resource: &'static str,
        limit: u64,
        attempted: u64,
    ) -> TextComposeResult<()> {
        cap(resource, limit, attempted).map_err(|kind| self.error(0, kind))
    }

    fn store(&self, store: BitmapStore) -> &[u8] {
        match store {
            BitmapStore::Imported => self.imported,
            BitmapStore::New => self.new,
            BitmapStore::Refined => self.instances.refined_store(),
        }
    }

    fn checked_event(&self, instance: TextInstance) -> TextComposeResult<Option<CheckedEvent>> {
        if instance.index != self.progress.completed_instances
            || instance.index >= self.header.instances
        {
            return Err(self.error(
                0,
                TextComposeErrorKind::Malformed("instance order or count"),
            ));
        }
        if i32::try_from(instance.x).is_err() || i32::try_from(instance.y).is_err() {
            return Err(self.error(
                0,
                TextComposeErrorKind::Malformed("placement outside signed 32-bit range"),
            ));
        }
        let reference = self
            .catalog
            .get(instance.symbol_id as usize)
            .ok_or_else(|| {
                self.error(
                    0,
                    TextComposeErrorKind::Malformed("symbol ID outside catalog"),
                )
            })?;
        let (store, base, descriptor) = match instance.bitmap {
            TextBitmap::Stored(stored) if !instance.ri && stored == *reference => {
                let (store, base) = match stored.store {
                    SymbolStore::Imported => (BitmapStore::Imported, self.imported_base),
                    SymbolStore::New => (BitmapStore::New, self.new_base),
                };
                if stored.store_base != base {
                    return Err(self.error(
                        0,
                        TextComposeErrorKind::Malformed(
                            "bitmap handle store base differs from view",
                        ),
                    ));
                }
                (store, base, stored.symbol)
            }
            TextBitmap::Refined { store_base, symbol }
                if instance.ri && store_base == self.refined_base =>
            {
                (BitmapStore::Refined, self.refined_base, symbol)
            }
            _ => {
                return Err(self.error(
                    0,
                    TextComposeErrorKind::Malformed(
                        "bitmap handle or RI differs from catalog/store",
                    ),
                ));
            }
        };
        if descriptor.width != instance.width || descriptor.height != instance.height {
            return Err(self.error(
                0,
                TextComposeErrorKind::Malformed("bitmap geometry differs from placement"),
            ));
        }
        let (stride, bytes) = descriptor_bytes(descriptor).map_err(|kind| self.error(0, kind))?;
        self.check_cap("symbol bytes", self.budget.max_symbol_bytes, bytes)?;
        self.check_cap("row bytes", self.budget.max_row_bytes as u64, stride)?;
        let start = base
            .checked_add(descriptor.relative_store_offset)
            .ok_or_else(|| {
                self.error(
                    0,
                    TextComposeErrorKind::InvalidSpan("symbol start overflows"),
                )
            })?;
        let end = start.checked_add(bytes).ok_or_else(|| {
            self.error(0, TextComposeErrorKind::InvalidSpan("symbol end overflows"))
        })?;
        if end > self.store(store).len() as u64 {
            return Err(self.error(
                start,
                TextComposeErrorKind::InvalidSpan("symbol outside bitmap store"),
            ));
        }
        let right = instance.x + i64::from(instance.width);
        let bottom = instance.y + i64::from(instance.height);
        let x0 = instance.x.max(0) as u64;
        let y0 = instance.y.max(0) as u64;
        let x1 = right.min(i64::from(self.header.region.width)).max(0) as u64;
        let y1 = bottom.min(i64::from(self.header.region.height)).max(0) as u64;
        if x0 >= x1 || y0 >= y1 {
            return Ok(None);
        }
        let touched = (x1 - x0) * (y1 - y0);
        self.check_cap(
            "per-instance touched pixels",
            self.budget.max_touched_pixels_per_instance,
            touched,
        )?;
        self.check_cap(
            "total touched pixels",
            self.budget.max_total_touched_pixels,
            self.progress.touched_pixels + touched,
        )?;
        self.check_cap(
            "composition work",
            self.budget.max_work_units,
            self.progress.work_units + touched,
        )?;
        Ok(Some(CheckedEvent {
            store,
            start,
            descriptor,
            x0,
            y0,
            x1,
            y1,
        }))
    }

    fn compose_event(&mut self, instance: TextInstance) -> TextComposeResult<()> {
        self.progress.stage = TextComposeStage::Instance;
        let Some(CheckedEvent {
            store,
            start,
            descriptor,
            x0,
            y0,
            x1,
            y1,
        }) = self.checked_event(instance)?
        else {
            self.progress.completed_instances += 1;
            return Ok(());
        };
        let source_stride = descriptor.row_stride as usize;
        let combination = self.header.flags.combination;
        let padding = padding_mask(self.header.region.width);
        let row_stride = self.row_stride;
        let source_store = match store {
            BitmapStore::Imported => self.imported,
            BitmapStore::New => self.new,
            BitmapStore::Refined => self.instances.refined_store(),
        };
        for y in y0..y1 {
            self.progress.current_row = y as u32;
            // `checked_event` proved the whole symbol lies in its store, and
            // `y0..y1` and `x0..x1` lie in both the symbol and the region.
            let source_y = (y as i64 - instance.y) as usize;
            let source_start = start as usize + source_y * source_stride;
            let source = &source_store[source_start..source_start + source_stride];
            let target_start = y as usize * row_stride;
            let target = &mut self.bitmap[target_start..target_start + row_stride];
            for x in x0..x1 {
                let sx = (x as i64 - instance.x) as usize;
                let source_pixel = source[sx / 8] & (0x80 >> (sx % 8)) != 0;
                let byte = &mut target[x as usize / 8];
                let mask = 0x80 >> (x as usize % 8);
                let target_pixel = *byte & mask != 0;
                if combine(target_pixel, source_pixel, combination) {
                    *byte |= mask;
                } else {
                    *byte &= !mask;
                }
            }
            target[row_stride - 1] &= padding;
        }
        let touched = (x1 - x0) * (y1 - y0);
        self.progress.touched_pixels += touched;
        self.progress.work_units += touched;
        self.progress.completed_instances += 1;
        Ok(())
    }

    /// Compose every instance into the bitmap, which first holds the default
    /// pixel. Any failure leaves a partial bitmap.
    pub fn compose(mut self) -> TextComposeResult<TextComposeReport> {
        self.progress.stage = TextComposeStage::Initialize;
        self.check_cancelled(0)?;
        self.check_cap(
            "composition work",
            self.budget.max_work_units,
            self.packed_bytes,
        )?;
        // The packed size was capped by `max_allocation_bytes`, so it fits a
        // `usize` on this target.
        let packed = self.packed_bytes as usize;
        self.bitmap.clear();
        if self.bitmap.try_reserve_exact(packed).is_err() {
            return Err(self.error(0, TextComposeErrorKind::AllocationFailed));
        }
        let fill = if self.header.flags.default_pixel {
            0xff
        } else {
            0
        };
        self.bitmap.resize(packed, fill);
        let padding = padding_mask(self.header.region.width);
        for row in self.bitmap.chunks_exact_mut(self.row_stride) {
            row[self.row_stride - 1] &= padding;
        }
        self.progress.work_units += self.packed_bytes;
        loop {
            self.progress.stage = TextComposeStage::Instance;
            self.check_cancelled(self.packed_bytes)?;
            let event = self.instances.next_instance().map_err(|error| {
                let offset = error.offset;
                self.error(offset, TextComposeErrorKind::Instance(Box::new(error)))
            })?;
            match event {
                Some(instance) => self.compose_event(instance)?,
                None if self.progress.completed_instances == self.header.instances => break,
                None => {
                    return Err(self.error(
                        0,
                        TextComposeErrorKind::Malformed(
                            "instance stream ended before declared count",
                        ),
                    ));
                }
            }
        }
        self.progress.stage = TextComposeStage::Complete;
        Ok(TextComposeReport {
            header: self.header,
            text_flags_raw: self.header.flags.raw,
            header_anomaly: self.header.anomaly,
            width: self.header.region.width,
            height: self.header.region.height,
            row_stride: self.row_stride as u32,
            packed_bytes: self.packed_bytes,
            progress: self.progress,
        })
    }
}

#[cfg(test)]
#[path = "text_composer/tests.rs"]
mod tests;
