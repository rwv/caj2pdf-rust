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
    mq::{ArithmeticError, ArithmeticSnapshot, ContextState, MQ_STATE_COUNT, MqDecoder, MqState},
};
use crate::fallible::reserve_exact;
use crate::{Cancellation, Limits, MAX_BUDGET_COUNT};
use std::{error, fmt, mem};

const CONTEXT_COUNT: usize = 1024;

/// A checked bound for one refinement session. All totals include every
/// successfully decoded bitmap in the session; MQ itself has additional caps.
///
/// The fields that bound running counters (`max_pixels_per_bitmap`,
/// `max_total_pixels` and `max_total_output_bytes`) must each be at most
/// [`MAX_BUDGET_COUNT`]. [`RefinementDecoder::new`] rejects a larger value as
/// `LimitExceeded` before decoding.
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
    /// GR pixel decisions only; the borrowed MQ's own budget covers all
    /// interleaved dictionary decisions in the coding unit.
    pub max_mq_decisions: u64,
    /// Ten neighbor probes per explicitly decoded pixel.
    pub max_context_work: u64,
    /// Full caller MQ bank and table, two target rows and three reference
    /// rows; the reference and output stores are separate.
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
            max_mq_decisions: 24_000_000,
            max_context_work: 240_000_000,
            max_working_bytes: 16 * 1024 * 1024,
        }
    }
}

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
    pub context_work: u64,
    pub mq: Option<ArithmeticSnapshot>,
}

/// A completed bitmap in the output store. Offset zero is the first byte
/// appended by this refinement session, even if the store was prefilled.
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
    Mq(Box<ArithmeticError>),
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
            RefinementErrorKind::Mq(source) => write!(f, "MQ: {source}"),
        }
    }
}

impl error::Error for RefinementError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.kind {
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
    budget: RefinementBudget,
    progress: RefinementProgress,
}

impl<'a, 'mq, C: Cancellation> RefinementDecoder<'a, 'mq, C> {
    /// Check the context range and fixed working-memory configuration before
    /// decoding. The caller retains all other model contexts.
    pub fn new(
        mq: &'a mut MqDecoder<'mq>,
        output: &'a mut Vec<u8>,
        limits: &'a Limits,
        cancellation: &'a C,
        budget: RefinementBudget,
    ) -> RefinementResult<Self> {
        Self::new_continuing(
            mq,
            output,
            limits,
            cancellation,
            budget,
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
        budget: RefinementBudget,
        previous: RefinementProgress,
    ) -> RefinementResult<Self> {
        let invalid = |kind| RefinementError {
            offset: None,
            bitmap_index: previous.completed_bitmaps,
            row: 0,
            x: 0,
            progress: Box::new(previous),
            kind,
        };
        limits
            .validate()
            .map_err(|_| invalid(RefinementErrorKind::Malformed("Limits")))?;
        for (resource, value) in [
            ("pixels per bitmap budget", budget.max_pixels_per_bitmap),
            ("total pixels budget", budget.max_total_pixels),
            ("total output bytes budget", budget.max_total_output_bytes),
        ] {
            if value > MAX_BUDGET_COUNT {
                return Err(invalid(RefinementErrorKind::LimitExceeded {
                    resource,
                    limit: MAX_BUDGET_COUNT,
                    attempted: value,
                }));
            }
        }
        if mq.context_count() < BITMAP_BASE + CONTEXT_COUNT {
            return Err(invalid(RefinementErrorKind::InvalidSpan(
                "coding unit lacks the GR context range",
            )));
        }
        if previous.output_bytes_written > budget.max_total_output_bytes
            || previous.pixels_decoded > budget.max_total_pixels
        {
            return Err(invalid(RefinementErrorKind::Malformed(
                "previous progress exceeds the budget",
            )));
        }
        Ok(Self {
            mq,
            output,
            limits,
            cancellation,
            budget,
            progress: previous,
        })
    }

    pub fn progress(&self) -> RefinementProgress {
        let mut progress = self.progress;
        progress.mq = Some(self.mq.snapshot());
        progress
    }

    /// Borrow the same coding unit for interleaved dictionary integer or
    /// IAID decisions. No GR statistics, bitmap offset, or budget is reset.
    pub fn mq_mut(&mut self) -> &mut MqDecoder<'mq> {
        self.mq
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

    fn check_cancelled(&self, row: u32) -> RefinementResult<()> {
        if self.cancellation.is_cancelled() {
            Err(self.error(RefinementErrorKind::Cancelled, None, row, 0))
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

    fn geometry(
        &self,
        reference_size: u64,
        request: RefinementRequest,
    ) -> RefinementResult<Geometry> {
        // One-pixel bitmaps within a MAX_BUDGET_COUNT pixel budget could
        // otherwise complete more bitmaps than the u32 index represents.
        let next = u64::from(self.progress.completed_bitmaps) + 1;
        self.cap("completed bitmaps", u64::from(u32::MAX), next)?;
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
        if reference_end > reference_size {
            return Err(self.error(
                RefinementErrorKind::InvalidSpan("reference outside its store"),
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
        // Every completed or failed bitmap passed these caps, so
        // `pixels_decoded <= max_total_pixels` and `output_bytes_written <=
        // max_total_output_bytes`, both at most MAX_BUDGET_COUNT (2^48).
        // With `pixels <= max_pixels_per_bitmap <= 2^48` and `bytes < 2^61`
        // (a stride of at most 2^29 times a u32 height), neither sum
        // overflows, and ten times `total_pixels <= 2^49` fits u64 below.
        let total_pixels = self.progress.pixels_decoded + pixels;
        let total_bytes = self.progress.output_bytes_written + bytes;
        self.cap("total pixels", self.budget.max_total_pixels, total_pixels)?;
        self.cap(
            "total output bytes",
            self.budget
                .max_total_output_bytes
                .min(self.limits.max_output_bytes),
            total_bytes,
        )?;
        self.cap("MQ decisions", self.budget.max_mq_decisions, total_pixels)?;
        let work = total_pixels * 10;
        self.cap("context work", self.budget.max_context_work, work)?;
        // Both strides are at most 2^29 bytes, which every supported
        // (at least 32-bit) `usize` represents.
        let target_stride = target_stride as usize;
        let reference_stride = reference_stride as usize;
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
        // with the same fixed table term. Include the entire bank, not only
        // the GR slice, in the combined cap.
        let mq_bytes = self.mq.context_count() as u64 * mem::size_of::<ContextState>() as u64
            + (MQ_STATE_COUNT * mem::size_of::<MqState>()) as u64;
        // `row_bytes <= 5 * 2^29`, and the allocated context bank occupies
        // at most `isize::MAX` bytes, so this sum stays below 2^64.
        let working = row_bytes + mq_bytes;
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
        let failed = self.error(RefinementErrorKind::AllocationFailed, None, 0, 0);
        reserve_exact(&mut row, length, failed)?;
        row.resize(length, 0);
        Ok(row)
    }

    /// Reserve room for `bytes` more output bytes within the allocation limit.
    fn reserve_output(&mut self, bytes: u64) -> RefinementResult<()> {
        // The output store is in memory, so its length fits a `u64`.
        let attempted = (self.output.len() as u64).saturating_add(bytes);
        self.limits.check_allocation(attempted).map_err(|_| {
            self.limit(
                "refinement store bytes",
                self.limits.max_allocation_bytes,
                attempted,
                0,
                0,
            )
        })?;
        // `bytes` fits the allocation limit, hence a `usize` on this target.
        self.output
            .try_reserve(bytes as usize)
            .map_err(|_| self.error(RefinementErrorKind::AllocationFailed, None, 0, 0))
    }

    /// Append one packed bitmap; retain the GR contexts and MQ state for the
    /// next bitmap while restarting target-row history at zero.
    pub fn decode_bitmap(
        &mut self,
        reference: ReferenceStore<'_>,
        request: RefinementRequest,
    ) -> RefinementResult<RefinementReport> {
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
            self.check_cancelled(y)?;
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
                let bit = self.mq.decode_bit(BITMAP_BASE + context).map_err(|error| {
                    let offset = error.offset;
                    self.error(RefinementErrorKind::Mq(Box::new(error)), offset, y, x)
                })?;
                if bit {
                    current[(x / 8) as usize] |= 0x80 >> (x % 8);
                }
                self.progress.pixels_decoded += 1;
                self.progress.context_work += 10;
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
    use crate::jbig2::iaid::IAID_BASE;
    use crate::jbig2::mq::{CodedSpan, ContextBank, MqBudget, MqTable};
    use crate::{MAX_BUDGET_COUNT, NeverCancel};

    #[test]
    fn the_bitmap_index_cannot_pass_u32_max() {
        let limits = Limits::default();
        let mq_budget = MqBudget::default();
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
            mq_budget,
        )
        .unwrap();
        let mut sink = Vec::new();
        let budget = RefinementBudget {
            max_total_pixels: MAX_BUDGET_COUNT,
            ..RefinementBudget::default()
        };
        let mut decoder =
            RefinementDecoder::new(&mut mq, &mut sink, &limits, &NeverCancel, budget).unwrap();
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
        assert_eq!(error.bitmap_index, u32::MAX);
        assert!(matches!(
            error.kind,
            RefinementErrorKind::LimitExceeded {
                resource: "completed bitmaps",
                limit,
                attempted,
            } if limit == u64::from(u32::MAX) && attempted == limit + 1
        ));
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
