// SPDX-License-Identifier: MIT

//! Bounded composition of T.88 arithmetic text-region instance bitmaps.
//!
//! A text stream can place a later symbol above an earlier one, so the whole
//! packed region is composed in a caller-owned bitmap in memory. The bitmap
//! is complete only after the stream's terminal has been checked.

use super::{
    dictionary::{StoredSymbol, SymbolDescriptor, SymbolStore},
    text::{SymbolCombination, TextHeaderAnomaly, TextRegionHeader},
    text_instances::{TextBitmap, TextInstance, TextInstanceDecoder},
};
use crate::{Cancellation, Context, Error, Limits, Result};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TextComposeStage {
    #[default]
    Preflight,
    Initialize,
    Instance,
    Complete,
}

/// `touched_pixels` counts the region pixels instances have visited.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextComposeProgress {
    pub stage: TextComposeStage,
    pub completed_instances: u32,
    pub current_row: u32,
    pub touched_pixels: u64,
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

/// Locate an unlocated composition error in `segment`. Region and store
/// coordinates are not input offsets, so none is attached.
fn locate(segment: u32, error: Error) -> Error {
    if error.context == Context::None {
        error.in_jbig2(Some(segment))
    } else {
        error
    }
}

fn cap(resource: &'static str, limit: u64, attempted: u64) -> Result<()> {
    if attempted > limit {
        Err(Error::limit(resource, limit, attempted))
    } else {
        Ok(())
    }
}

fn descriptor_bytes(descriptor: SymbolDescriptor) -> Result<(u64, u64)> {
    if descriptor.width == 0 || descriptor.height == 0 {
        return Err(Error::invalid("zero bitmap dimension"));
    }
    let stride = u64::from(descriptor.width).div_ceil(8);
    let bytes = stride * u64::from(descriptor.height);
    if u64::from(descriptor.row_stride) != stride || descriptor.stored_bytes != bytes {
        return Err(Error::invalid("noncanonical bitmap descriptor"));
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
    row_stride: usize,
    packed_bytes: u64,
    allocation_limit: u64,
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
    ) -> Result<Self> {
        let bad = |error| locate(segment, error);
        if instances.segment() != segment || instances.header() != header {
            return Err(bad(Error::invalid(
                "instance stream segment or header differs",
            )));
        }
        if header.segment != segment {
            return Err(bad(Error::invalid(
                "text header segment differs from composer",
            )));
        }
        if header.region.width == 0 || header.region.height == 0 {
            return Err(bad(Error::invalid("zero region dimension")));
        }
        if header.flags.huffman || header.flags.refinement_template != 1 && header.flags.refine {
            return Err(bad(Error::invalid("unsupported text stream profile")));
        }
        let pixels = u64::from(header.region.width) * u64::from(header.region.height);
        cap("region pixels", limits.max_image_pixels, pixels).map_err(&bad)?;
        let stride = u64::from(header.region.width).div_ceil(8);
        let packed_bytes = stride * u64::from(header.region.height);
        cap(
            "region bitmap bytes",
            limits.max_allocation_bytes,
            packed_bytes,
        )
        .map_err(&bad)?;
        let row_stride = usize::try_from(stride)
            .map_err(|_| bad(Error::limit("region row bytes", usize::MAX as u64, stride)))?;
        if header.instances > 0 && catalog.is_empty() {
            return Err(bad(Error::invalid("nonempty text region with no symbols")));
        }
        if imported_base > imported.len() as u64
            || new_base > new.len() as u64
            || refined_base > instances.refined_store().len() as u64
        {
            return Err(bad(Error::invalid("bitmap store base beyond its store")));
        }
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
            row_stride,
            packed_bytes,
            allocation_limit: limits.max_allocation_bytes,
            progress: TextComposeProgress::default(),
        })
    }

    pub fn progress(&self) -> TextComposeProgress {
        self.progress
    }

    fn error(&self, error: Error) -> Error {
        locate(self.segment, error)
    }

    fn check_cancelled(&self) -> Result<()> {
        if self.cancellation.is_cancelled() {
            Err(self.error(Error::cancelled()))
        } else {
            Ok(())
        }
    }

    fn store(&self, store: BitmapStore) -> &[u8] {
        match store {
            BitmapStore::Imported => self.imported,
            BitmapStore::New => self.new,
            BitmapStore::Refined => self.instances.refined_store(),
        }
    }

    fn checked_event(&self, instance: TextInstance) -> Result<Option<CheckedEvent>> {
        if instance.index != self.progress.completed_instances
            || instance.index >= self.header.instances
        {
            return Err(self.error(Error::invalid("instance order or count")));
        }
        if i32::try_from(instance.x).is_err() || i32::try_from(instance.y).is_err() {
            return Err(self.error(Error::invalid("placement outside signed 32-bit range")));
        }
        let reference = self
            .catalog
            .get(instance.symbol_id as usize)
            .ok_or_else(|| self.error(Error::invalid("symbol ID outside catalog")))?;
        let (store, base, descriptor) = match instance.bitmap {
            TextBitmap::Stored(stored) if !instance.ri && stored == *reference => {
                let (store, base) = match stored.store {
                    SymbolStore::Imported => (BitmapStore::Imported, self.imported_base),
                    SymbolStore::New => (BitmapStore::New, self.new_base),
                };
                if stored.store_base != base {
                    return Err(
                        self.error(Error::invalid("bitmap handle store base differs from view"))
                    );
                }
                (store, base, stored.symbol)
            }
            TextBitmap::Refined { store_base, symbol }
                if instance.ri && store_base == self.refined_base =>
            {
                (BitmapStore::Refined, self.refined_base, symbol)
            }
            _ => {
                return Err(self.error(Error::invalid(
                    "bitmap handle or RI differs from catalog/store",
                )));
            }
        };
        if descriptor.width != instance.width || descriptor.height != instance.height {
            return Err(self.error(Error::invalid("bitmap geometry differs from placement")));
        }
        let (_, bytes) = descriptor_bytes(descriptor).map_err(|error| self.error(error))?;
        let start = base
            .checked_add(descriptor.relative_store_offset)
            .ok_or_else(|| self.error(Error::invalid("symbol start overflows")))?;
        let end = start
            .checked_add(bytes)
            .ok_or_else(|| self.error(Error::invalid("symbol end overflows")))?;
        if end > self.store(store).len() as u64 {
            return Err(self.error(Error::invalid("symbol outside bitmap store")));
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

    fn compose_event(&mut self, instance: TextInstance) -> Result<()> {
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
        self.progress.completed_instances += 1;
        Ok(())
    }

    /// Compose every instance into the bitmap, which first holds the default
    /// pixel. Any failure leaves a partial bitmap.
    pub fn compose(mut self) -> Result<TextComposeReport> {
        self.progress.stage = TextComposeStage::Initialize;
        self.check_cancelled()?;
        // The packed size was capped by `max_allocation_bytes`, so it fits a
        // `usize` on this target.
        let packed = self.packed_bytes as usize;
        self.bitmap.clear();
        if self.bitmap.try_reserve_exact(packed).is_err() {
            return Err(self.error(Error::limit(
                "region bitmap bytes",
                self.allocation_limit,
                self.packed_bytes,
            )));
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
        loop {
            self.progress.stage = TextComposeStage::Instance;
            self.check_cancelled()?;
            let event = self
                .instances
                .next_instance()
                .map_err(|error| self.error(error))?;
            match event {
                Some(instance) => self.compose_event(instance)?,
                None if self.progress.completed_instances == self.header.instances => break,
                None => {
                    return Err(self.error(Error::invalid(
                        "instance stream ended before declared count",
                    )));
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
