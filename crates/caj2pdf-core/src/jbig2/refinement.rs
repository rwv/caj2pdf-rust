// SPDX-License-Identifier: MIT

//! Bounded T.88 template-1 generic refinement bitmaps.
//!
//! This is one bitmap operation inside an existing MQ coding unit, over the
//! bitmap contexts at the coding unit's fixed [`BITMAP_BASE`]. The caller owns
//! the reference store and the append-only output store, both in memory. No
//! document pixels are bundled here.

use super::{
    dictionary::SymbolDescriptor,
    integer::BITMAP_BASE,
    mq::{ArithmeticSnapshot, MqDecoder},
    unsupported,
};
use crate::fallible::reserve_exact;
use crate::{Cancellation, Error, Limits, Result};
use std::mem;

const CONTEXT_COUNT: usize = 1024;

/// The reference bitmap in a store. `store_base` is the store offset of its
/// dictionary's first symbol; `symbol.relative_store_offset` is relative to
/// that base.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RefinementReference {
    pub store_base: u64,
    pub symbol: SymbolDescriptor,
}

/// The store that holds a refinement's reference bitmap.
#[derive(Clone, Copy, Debug)]
pub enum ReferenceStore<'r> {
    /// A store other than the output store.
    Other(&'r [u8]),
    /// The session's own output store, which a refinement dictionary reads
    /// its earlier new symbols from.
    Output,
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

/// Session totals over every completed bitmap and the current one.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RefinementProgress {
    pub completed_bitmaps: u32,
    pub rows_written: u64,
    pub pixels_decoded: u64,
    pub output_bytes_written: u64,
    pub mq: Option<ArithmeticSnapshot>,
}

/// A completed bitmap in the output store. Offset zero is the first byte
/// appended by this refinement session, even if the store was prefilled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RefinementReport {
    pub target: SymbolDescriptor,
    pub progress: RefinementProgress,
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
/// unit, *one* append-only output store, and persistent disjoint 1,024 GR
/// contexts at [`BITMAP_BASE`]. The bound store cannot change between
/// bitmaps, so relative descriptor offsets remain in one store.
/// No MQ initialization, finish, or context reset occurs here. After an
/// error the caller must discard the coding unit and the store's new bytes.
pub struct RefinementDecoder<'a, 'mq, C: Cancellation> {
    mq: &'a mut MqDecoder<'mq>,
    output: &'a mut Vec<u8>,
    limits: &'a Limits,
    cancellation: &'a C,
    progress: RefinementProgress,
}

impl<'a, 'mq, C: Cancellation> RefinementDecoder<'a, 'mq, C> {
    /// Check the context range before decoding. The caller retains all other
    /// model contexts. Each target bitmap is bounded by
    /// `Limits::max_image_pixels` and the output store by
    /// `Limits::max_allocation_bytes`.
    pub fn new(
        mq: &'a mut MqDecoder<'mq>,
        output: &'a mut Vec<u8>,
        limits: &'a Limits,
        cancellation: &'a C,
    ) -> Result<Self> {
        Self::new_continuing(
            mq,
            output,
            limits,
            cancellation,
            RefinementProgress::default(),
        )
    }

    /// Resume the same append-only store after a completed bitmap. The
    /// enclosing pull decoder owns the MQ unit and keeps this progress
    /// between calls; it must bind the same store each time.
    pub(crate) fn new_continuing(
        mq: &'a mut MqDecoder<'mq>,
        output: &'a mut Vec<u8>,
        limits: &'a Limits,
        cancellation: &'a C,
        previous: RefinementProgress,
    ) -> Result<Self> {
        if mq.context_count() < BITMAP_BASE + CONTEXT_COUNT {
            return Err(Error::invalid("coding unit lacks the GR context range"));
        }
        Ok(Self {
            mq,
            output,
            limits,
            cancellation,
            progress: previous,
        })
    }

    pub fn progress(&self) -> RefinementProgress {
        let mut progress = self.progress;
        progress.mq = Some(self.mq.snapshot());
        progress
    }

    /// Borrow the same coding unit for interleaved dictionary integer or
    /// IAID decisions. No GR statistics or bitmap offset is reset.
    pub fn mq_mut(&mut self) -> &mut MqDecoder<'mq> {
        self.mq
    }

    fn check_cancelled(&self) -> Result<()> {
        if self.cancellation.is_cancelled() {
            Err(Error::cancelled())
        } else {
            Ok(())
        }
    }

    fn cap(&self, resource: &'static str, maximum: u64, attempted: u64) -> Result<()> {
        if attempted > maximum {
            Err(Error::limit(resource, maximum, attempted))
        } else {
            Ok(())
        }
    }

    fn geometry(&self, reference_size: u64, request: RefinementRequest) -> Result<Geometry> {
        // One-pixel bitmaps could otherwise complete more bitmaps than the
        // u32 index represents.
        let next = u64::from(self.progress.completed_bitmaps) + 1;
        self.cap("completed bitmaps", u64::from(u32::MAX), next)?;
        if request.template != 1 {
            return Err(unsupported("refinement template"));
        }
        if request.typical_prediction {
            return Err(unsupported("TPGRON"));
        }
        let symbol = request.reference.symbol;
        if request.width == 0 || request.height == 0 || symbol.width == 0 || symbol.height == 0 {
            return Err(unsupported("zero bitmap dimension"));
        }
        let target_stride = u64::from(request.width).div_ceil(8);
        let reference_stride = u64::from(symbol.width).div_ceil(8);
        if u64::from(symbol.row_stride) != reference_stride {
            return Err(Error::invalid("noncanonical reference row stride"));
        }
        // A u32 width has a stride of at most 2^29 bytes. Multiplication by
        // a u32 height therefore fits u64, even before the configured caps.
        let reference_bytes = reference_stride * u64::from(symbol.height);
        if symbol.stored_bytes != reference_bytes {
            return Err(Error::invalid("reference stored byte length"));
        }
        let reference_offset = request
            .reference
            .store_base
            .checked_add(symbol.relative_store_offset)
            .ok_or_else(|| Error::invalid("reference offset overflows u64"))?;
        let reference_end = reference_offset
            .checked_add(reference_bytes)
            .ok_or_else(|| Error::invalid("reference end overflows u64"))?;
        if reference_end > reference_size {
            return Err(Error::invalid("reference outside its store"));
        }
        let pixels = u64::from(request.width) * u64::from(request.height);
        let bytes = target_stride * u64::from(request.height);
        self.cap("pixels per bitmap", self.limits.max_image_pixels, pixels)?;
        // Both strides are at most 2^29 bytes, which every supported
        // (at least 32-bit) `usize` represents.
        let target_stride = target_stride as usize;
        let reference_stride = reference_stride as usize;
        self.limits
            .check_allocation(target_stride as u64)
            .map_err(|_| {
                Error::limit(
                    "target row allocation",
                    self.limits.max_allocation_bytes,
                    target_stride as u64,
                )
            })?;
        self.limits
            .check_allocation(reference_stride as u64)
            .map_err(|_| {
                Error::limit(
                    "reference row allocation",
                    self.limits.max_allocation_bytes,
                    reference_stride as u64,
                )
            })?;
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

    fn row(&self, length: usize) -> Result<Vec<u8>> {
        let mut row = Vec::new();
        let failed = self
            .limits
            .allocation_refused("refinement row bytes", length as u64);
        reserve_exact(&mut row, length, failed)?;
        row.resize(length, 0);
        Ok(row)
    }

    /// Reserve room for `bytes` more output bytes within the allocation limit.
    fn reserve_output(&mut self, bytes: u64) -> Result<()> {
        // The output store is in memory, so its length fits a `u64`.
        let attempted = (self.output.len() as u64).saturating_add(bytes);
        self.limits.check_allocation(attempted).map_err(|_| {
            Error::limit(
                "refinement store bytes",
                self.limits.max_allocation_bytes,
                attempted,
            )
        })?;
        // `bytes` fits the allocation limit, hence a `usize` on this target.
        self.output.try_reserve(bytes as usize).map_err(|_| {
            self.limits
                .allocation_refused("refinement store bytes", attempted)
        })
    }

    /// Append one packed bitmap; retain the GR contexts and MQ state for the
    /// next bitmap while restarting target-row history at zero.
    pub fn decode_bitmap(
        &mut self,
        reference: ReferenceStore<'_>,
        request: RefinementRequest,
    ) -> Result<RefinementReport> {
        let reference_size = match reference {
            ReferenceStore::Other(bytes) => bytes.len(),
            ReferenceStore::Output => self.output.len(),
        } as u64;
        let geometry = self.geometry(reference_size, request)?;
        self.reserve_output(geometry.bytes)?;
        let start_offset = self.progress.output_bytes_written;
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
            self.check_cancelled()?;
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
                // `geometry` checked that the whole reference bitmap lies in
                // its store, so this row does too.
                let start = (geometry.reference_offset
                    + row_id as u64 * geometry.reference_stride as u64)
                    as usize;
                let store = match reference {
                    ReferenceStore::Other(bytes) => bytes,
                    ReferenceStore::Output => self.output.as_slice(),
                };
                reference_rows[slot]
                    .copy_from_slice(&store[start..start + geometry.reference_stride]);
                cached[slot] = Some(row_id);
            }
            for x in 0..request.width {
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
                let bit = self.mq.decode_bit(BITMAP_BASE + context)?;
                if bit {
                    current[(x / 8) as usize] |= 0x80 >> (x % 8);
                }
                self.progress.pixels_decoded += 1;
            }
            self.output.extend_from_slice(&current);
            self.progress.output_bytes_written += current.len() as u64;
            self.progress.rows_written += 1;
            mem::swap(&mut previous, &mut current);
        }
        let target = SymbolDescriptor {
            width: request.width,
            height: request.height,
            row_stride: geometry.target_stride as u32,
            relative_store_offset: start_offset,
            stored_bytes: geometry.bytes,
        };
        // `geometry` refused a bitmap that would pass u32::MAX.
        self.progress.completed_bitmaps += 1;
        debug_assert_eq!(
            self.progress.output_bytes_written - start_offset,
            geometry.bytes
        );
        debug_assert_eq!(
            geometry.pixels,
            u64::from(request.width) * u64::from(request.height)
        );
        Ok(RefinementReport {
            target,
            progress: self.progress(),
        })
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
    use super::*;
    use crate::ErrorKind;
    use crate::NeverCancel;
    use crate::jbig2::iaid::IAID_BASE;
    use crate::jbig2::mq::{CodedSpan, ContextBank, MqTable};

    #[test]
    fn the_bitmap_index_cannot_pass_u32_max() {
        let limits = Limits::default();
        let table = MqTable::standard();
        let mut banks = ContextBank::new(IAID_BASE + 2, &limits).unwrap();
        let bytes = [0, 0xff, 0xac];
        let mut mq = MqDecoder::new(
            (&bytes[..]).into(),
            CodedSpan {
                offset: 0,
                length: 3,
            },
            &table,
            &mut banks,
            &limits,
        )
        .unwrap();
        let mut sink = Vec::new();
        let mut decoder =
            RefinementDecoder::new(&mut mq, &mut sink, &limits, &NeverCancel).unwrap();
        decoder.progress.completed_bitmaps = u32::MAX;
        let symbol = SymbolDescriptor {
            width: 1,
            height: 1,
            row_stride: 1,
            relative_store_offset: 0,
            stored_bytes: 1,
        };
        let request = RefinementRequest {
            width: 1,
            height: 1,
            template: 1,
            typical_prediction: false,
            reference_dx: 0,
            reference_dy: 0,
            reference: RefinementReference {
                store_base: 0,
                symbol,
            },
        };
        let error = decoder
            .decode_bitmap(ReferenceStore::Other(&[0]), request)
            .unwrap_err();
        assert!(matches!(error, Error { kind: ErrorKind::LimitExceeded {
                resource: "completed bitmaps",
                limit,
                attempted,
            }, .. } if limit == u64::from(u32::MAX) && attempted == limit + 1));
    }

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

    #[test]
    fn the_three_rows_around_any_reference_row_use_distinct_cache_slots() {
        // A target row `y < 2^32` with `dy` in `i32` centers on reference
        // row `y - dy`, strictly between `-2^31` and `2^32 + 2^31`.
        let extremes = [-(1_i64 << 31), (1_i64 << 32) + (1 << 31)];
        for center in (-7..=7).chain(extremes) {
            let mut slots = [center - 1, center, center + 1].map(super::cache_slot);
            slots.sort_unstable();
            assert_eq!(slots, [0, 1, 2], "row {center}");
        }
    }
}
