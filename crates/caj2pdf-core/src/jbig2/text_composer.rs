// SPDX-License-Identifier: MIT

//! Bounded composition of T.88 arithmetic text-region instance bitmaps.
//!
//! A text stream can place a later symbol above an earlier one. The caller
//! therefore supplies random-access scratch storage; only after the stream's
//! terminal has been checked are packed rows sent to the final sink.

use super::{
    dictionary::SymbolDescriptor,
    refinement_dictionary::{StoredSymbol, SymbolStore},
    text::{SymbolCombination, TextRegionHeader},
    text_instances::{
        TextBitmap, TextInstance, TextInstanceDecoder, TextInstanceError, TextInstanceResult,
    },
};
use crate::{Cancellation, Error, Limits, MAX_BUDGET_COUNT, RangedSource, SequentialSink};
use std::{error, fmt};

/// Caller-owned random-access storage for one packed region bitmap.
///
/// `set_len` must establish exactly the requested size or fail. Completed
/// positioned writes must be visible to later reads on the same handle;
/// `flush` makes them visible to other handles and finalizes buffered writes.
/// Reads and writes may complete a nonempty prefix. An adapter must never
/// report more bytes than supplied. The composer initializes every byte, so
/// sparse allocation must not expose uninitialized pixels.
#[allow(async_fn_in_trait)]
pub trait RandomAccessScratch {
    fn size(&self) -> u64;
    async fn set_len(&mut self, bytes: u64) -> crate::Result<()>;
    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> crate::Result<usize>;
    async fn write_at(&mut self, offset: u64, bytes: &[u8]) -> crate::Result<usize>;
    async fn flush(&mut self) -> crate::Result<()>;
}

/// An ordered #86 instance stream. `None` must mean its count and MQ terminal
/// were validated; the production implementation is `TextInstanceDecoder`.
#[allow(async_fn_in_trait)]
pub trait TextInstanceSource {
    fn segment(&self) -> u32;
    fn header(&self) -> TextRegionHeader;
    async fn next(&mut self) -> TextInstanceResult<Option<TextInstance>>;
}

impl<S, RI, RN, W, C> TextInstanceSource for TextInstanceDecoder<'_, S, RI, RN, W, C>
where
    S: RangedSource,
    RI: RangedSource,
    RN: RangedSource,
    W: SequentialSink,
    C: Cancellation,
{
    fn segment(&self) -> u32 {
        self.segment()
    }

    fn header(&self) -> TextRegionHeader {
        self.header()
    }

    async fn next(&mut self) -> TextInstanceResult<Option<TextInstance>> {
        self.next().await
    }
}

/// Independent composition bounds. Counters are also limited to
/// `MAX_BUDGET_COUNT`, leaving room for checked per-call accounting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextComposeBudget {
    pub max_exported_symbols: u32,
    pub max_instances: u32,
    pub max_region_pixels: u64,
    pub max_scratch_bytes: u64,
    pub max_symbol_bytes: u64,
    pub max_touched_pixels_per_instance: u64,
    pub max_total_touched_pixels: u64,
    pub max_source_read_bytes: u64,
    pub max_scratch_read_bytes: u64,
    pub max_scratch_write_bytes: u64,
    pub max_output_bytes: u64,
    pub max_work_units: u64,
    pub max_source_read_calls: u64,
    pub max_scratch_read_calls: u64,
    pub max_scratch_write_calls: u64,
    pub max_output_write_calls: u64,
    pub max_row_bytes: usize,
    pub max_request_bytes: usize,
    pub max_resident_bytes: u64,
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
            max_source_read_bytes: 4 * 1024 * 1024 * 1024,
            max_scratch_read_bytes: 8 * 1024 * 1024 * 1024,
            max_scratch_write_bytes: 8 * 1024 * 1024 * 1024,
            max_output_bytes: 128 * 1024 * 1024,
            max_work_units: 2_000_000_000,
            max_source_read_calls: 10_000_000,
            max_scratch_read_calls: 10_000_000,
            max_scratch_write_calls: 10_000_000,
            max_output_write_calls: 10_000_000,
            max_row_bytes: 1024 * 1024,
            max_request_bytes: 256 * 1024,
            max_resident_bytes: 2 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TextComposeStage {
    #[default]
    Preflight,
    Initialize,
    Instance,
    Readback,
    OutputFlush,
    Complete,
}

/// Physical I/O counts include short successful calls before a later error.
/// `work_units` counts initialized bytes, visited pixels, and emitted bytes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextComposeProgress {
    pub stage: TextComposeStage,
    pub completed_instances: u32,
    pub output_rows: u32,
    pub current_row: u32,
    pub touched_pixels: u64,
    pub source_bytes_read: u64,
    pub scratch_bytes_read: u64,
    pub scratch_bytes_written: u64,
    pub output_bytes_written: u64,
    pub source_read_calls: u64,
    pub scratch_read_calls: u64,
    pub scratch_write_calls: u64,
    pub output_write_calls: u64,
    pub work_units: u64,
    pub max_request_bytes: usize,
    pub peak_resident_bytes: u64,
    pub poisoned: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextComposeReport {
    pub width: u32,
    pub height: u32,
    pub row_stride: u32,
    pub packed_bytes: u64,
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
    source_size: u64,
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
    Source {
        store: BitmapStore,
        error: Error,
    },
    Scratch(Error),
    Output(Error),
    Poisoned,
}

/// `offset` names the active bitmap-store, scratch, or output byte according
/// to `progress.stage` and `kind`; instance failures preserve their MQ offset.
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
            TextComposeErrorKind::AllocationFailed => f.write_str("row allocation failed"),
            TextComposeErrorKind::Cancelled => f.write_str("cancelled"),
            TextComposeErrorKind::Instance(error) => write!(f, "instance: {error}"),
            TextComposeErrorKind::Source { store, error } => {
                write!(f, "{store:?} symbol source: {error}")
            }
            TextComposeErrorKind::Scratch(error) => write!(f, "scratch: {error}"),
            TextComposeErrorKind::Output(error) => write!(f, "output: {error}"),
            TextComposeErrorKind::Poisoned => f.write_str("composer is poisoned or complete"),
        }
    }
}

impl error::Error for TextComposeError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.kind {
            TextComposeErrorKind::Instance(error) => Some(error),
            TextComposeErrorKind::Source { error, .. }
            | TextComposeErrorKind::Scratch(error)
            | TextComposeErrorKind::Output(error) => Some(error),
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

/// One region composition session. A failed or abandoned `compose` poisons
/// scratch and final output; the caller must discard both.
pub struct TextComposer<
    'a,
    I: TextInstanceSource,
    RI: RangedSource,
    RN: RangedSource,
    RT: RangedSource,
    T: RandomAccessScratch,
    W: SequentialSink,
    C: Cancellation,
> {
    header: TextRegionHeader,
    segment: u32,
    catalog: &'a [StoredSymbol],
    instances: &'a mut I,
    imported: &'a mut RI,
    imported_base: u64,
    imported_size: u64,
    new: &'a mut RN,
    new_base: u64,
    new_size: u64,
    refined: &'a mut RT,
    refined_base: u64,
    scratch: &'a mut T,
    output: &'a mut W,
    limits: &'a Limits,
    cancellation: &'a C,
    budget: TextComposeBudget,
    row_stride: usize,
    packed_bytes: u64,
    progress: TextComposeProgress,
    started: bool,
    poisoned: bool,
    complete: bool,
}

impl<'a, I, RI, RN, RT, T, W, C> TextComposer<'a, I, RI, RN, RT, T, W, C>
where
    I: TextInstanceSource,
    RI: RangedSource,
    RN: RangedSource,
    RT: RangedSource,
    T: RandomAccessScratch,
    W: SequentialSink,
    C: Cancellation,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        segment: u32,
        header: TextRegionHeader,
        catalog: &'a [StoredSymbol],
        instances: &'a mut I,
        imported: &'a mut RI,
        imported_base: u64,
        new: &'a mut RN,
        new_base: u64,
        refined: &'a mut RT,
        refined_base: u64,
        scratch: &'a mut T,
        output: &'a mut W,
        limits: &'a Limits,
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
            "final output bytes",
            budget.max_output_bytes.min(limits.max_output_bytes),
            packed_bytes,
        )
        .map_err(&bad)?;
        cap("row bytes", budget.max_row_bytes as u64, stride).map_err(&bad)?;
        cap(
            "resident row bytes",
            budget.max_resident_bytes.min(limits.max_allocation_bytes),
            stride,
        )
        .map_err(&bad)?;
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
            ("source read budget", budget.max_source_read_bytes),
            ("scratch read budget", budget.max_scratch_read_bytes),
            ("scratch write budget", budget.max_scratch_write_bytes),
            ("output budget", budget.max_output_bytes),
            ("work budget", budget.max_work_units),
            ("source read calls budget", budget.max_source_read_calls),
            ("scratch read calls budget", budget.max_scratch_read_calls),
            ("scratch write calls budget", budget.max_scratch_write_calls),
            ("output write calls budget", budget.max_output_write_calls),
        ] {
            cap(name, MAX_BUDGET_COUNT, count).map_err(&bad)?;
        }
        if budget.max_request_bytes == 0 || budget.max_request_bytes > limits.io_chunk_bytes {
            return Err(bad(TextComposeErrorKind::Malformed(
                "invalid composition request cap",
            )));
        }
        if imported_base > imported.size() || new_base > new.size() || refined_base > refined.size()
        {
            return Err(bad(TextComposeErrorKind::InvalidSpan(
                "bitmap store base beyond source",
            )));
        }
        if scratch.size() != 0 {
            return Err(bad(TextComposeErrorKind::Malformed(
                "scratch store must be empty",
            )));
        }
        // The row-byte cap above is a `usize`, so `stride` fits this target.
        let row_stride = stride as usize;
        Ok(Self {
            header,
            segment,
            catalog,
            instances,
            imported_size: imported.size(),
            imported,
            imported_base,
            new_size: new.size(),
            new,
            new_base,
            refined,
            refined_base,
            scratch,
            output,
            limits,
            cancellation,
            budget,
            row_stride,
            packed_bytes,
            progress: TextComposeProgress::default(),
            started: false,
            poisoned: false,
            complete: false,
        })
    }

    pub fn progress(&self) -> TextComposeProgress {
        let mut progress = self.progress;
        progress.poisoned = self.poisoned;
        progress
    }

    fn error(&self, offset: u64, kind: TextComposeErrorKind) -> TextComposeError {
        TextComposeError {
            segment: self.segment,
            offset,
            progress: Box::new(self.progress()),
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

    fn scratch_error(&self, offset: u64, error: Error) -> TextComposeError {
        self.error(
            offset,
            match error {
                Error::Cancelled => TextComposeErrorKind::Cancelled,
                other => TextComposeErrorKind::Scratch(other),
            },
        )
    }

    fn output_error(&self, offset: u64, error: Error) -> TextComposeError {
        self.error(
            offset,
            match error {
                Error::Cancelled => TextComposeErrorKind::Cancelled,
                other => TextComposeErrorKind::Output(other),
            },
        )
    }

    fn source_error(&self, offset: u64, store: BitmapStore, error: Error) -> TextComposeError {
        self.error(
            offset,
            match error {
                Error::Cancelled => TextComposeErrorKind::Cancelled,
                other => TextComposeErrorKind::Source {
                    store,
                    error: other,
                },
            },
        )
    }

    fn check_cap(
        &self,
        resource: &'static str,
        limit: u64,
        attempted: u64,
    ) -> TextComposeResult<()> {
        cap(resource, limit, attempted).map_err(|kind| self.error(0, kind))
    }

    fn note_request(&mut self, bytes: usize) {
        self.progress.max_request_bytes = self.progress.max_request_bytes.max(bytes);
    }

    fn note_resident(&mut self, target: &Vec<u8>, source: &Vec<u8>) -> TextComposeResult<()> {
        let resident = target.capacity() as u64 + source.capacity() as u64;
        self.check_cap(
            "resident row bytes",
            self.budget
                .max_resident_bytes
                .min(self.limits.max_allocation_bytes),
            resident,
        )?;
        self.progress.peak_resident_bytes = self.progress.peak_resident_bytes.max(resident);
        Ok(())
    }

    fn make_target_row(&mut self) -> TextComposeResult<Vec<u8>> {
        let mut row = Vec::new();
        row.try_reserve_exact(self.row_stride)
            .map_err(|_| self.error(0, TextComposeErrorKind::AllocationFailed))?;
        row.resize(self.row_stride, 0);
        self.note_resident(&row, &Vec::new())?;
        Ok(row)
    }

    fn grow_source_row(
        &mut self,
        row: &mut Vec<u8>,
        size: usize,
        target: &Vec<u8>,
    ) -> TextComposeResult<()> {
        self.check_cap("row bytes", self.budget.max_row_bytes as u64, size as u64)?;
        self.check_cap(
            "resident row bytes",
            self.budget
                .max_resident_bytes
                .min(self.limits.max_allocation_bytes),
            target.capacity() as u64 + size as u64,
        )?;
        if size > row.len() {
            row.try_reserve_exact(size - row.len())
                .map_err(|_| self.error(0, TextComposeErrorKind::AllocationFailed))?;
            row.resize(size, 0);
        }
        self.note_resident(target, row)
    }

    async fn scratch_write(&mut self, offset: u64, bytes: &[u8]) -> TextComposeResult<()> {
        if self.scratch.size() != self.packed_bytes {
            return Err(self.error(
                offset,
                TextComposeErrorKind::Malformed("scratch size changed"),
            ));
        }
        let mut done = 0;
        while done < bytes.len() {
            let at = offset + done as u64;
            self.check_cancelled(at)?;
            let len = (bytes.len() - done).min(self.budget.max_request_bytes);
            self.check_cap(
                "scratch write calls",
                self.budget.max_scratch_write_calls,
                self.progress.scratch_write_calls + 1,
            )?;
            self.check_cap(
                "scratch write bytes",
                self.budget.max_scratch_write_bytes,
                self.progress.scratch_bytes_written + len as u64,
            )?;
            self.note_request(len);
            self.progress.scratch_write_calls += 1;
            let written = self
                .scratch
                .write_at(at, &bytes[done..done + len])
                .await
                .map_err(|error| self.scratch_error(at, error))?;
            if written > len {
                return Err(self.error(
                    at,
                    TextComposeErrorKind::Malformed("scratch overreported write"),
                ));
            }
            if written == 0 {
                return Err(self.error(
                    at,
                    TextComposeErrorKind::Scratch(Error::Io(std::io::Error::new(
                        std::io::ErrorKind::WriteZero,
                        "scratch made no progress",
                    ))),
                ));
            }
            done += written;
            self.progress.scratch_bytes_written += written as u64;
            self.check_cancelled(at + written as u64)?;
            if self.scratch.size() != self.packed_bytes {
                return Err(self.error(at, TextComposeErrorKind::Malformed("scratch size changed")));
            }
        }
        Ok(())
    }

    async fn scratch_read(&mut self, offset: u64, bytes: &mut [u8]) -> TextComposeResult<()> {
        if self.scratch.size() != self.packed_bytes {
            return Err(self.error(
                offset,
                TextComposeErrorKind::Malformed("scratch size changed"),
            ));
        }
        let mut done = 0;
        while done < bytes.len() {
            let at = offset + done as u64;
            self.check_cancelled(at)?;
            let len = (bytes.len() - done).min(self.budget.max_request_bytes);
            self.check_cap(
                "scratch read calls",
                self.budget.max_scratch_read_calls,
                self.progress.scratch_read_calls + 1,
            )?;
            self.check_cap(
                "scratch read bytes",
                self.budget.max_scratch_read_bytes,
                self.progress.scratch_bytes_read + len as u64,
            )?;
            self.note_request(len);
            self.progress.scratch_read_calls += 1;
            let read = self
                .scratch
                .read_at(at, &mut bytes[done..done + len])
                .await
                .map_err(|error| self.scratch_error(at, error))?;
            if read > len {
                return Err(self.error(
                    at,
                    TextComposeErrorKind::Malformed("scratch overreported read"),
                ));
            }
            if read == 0 {
                return Err(self.error(
                    at,
                    TextComposeErrorKind::Scratch(Error::TruncatedInput {
                        offset: at,
                        expected: len as u64,
                        available: 0,
                    }),
                ));
            }
            done += read;
            self.progress.scratch_bytes_read += read as u64;
            self.check_cancelled(at + read as u64)?;
            if self.scratch.size() != self.packed_bytes {
                return Err(self.error(at, TextComposeErrorKind::Malformed("scratch size changed")));
            }
        }
        Ok(())
    }

    async fn source_read(
        &mut self,
        store: BitmapStore,
        expected_size: u64,
        offset: u64,
        bytes: &mut [u8],
    ) -> TextComposeResult<()> {
        let mut done = 0;
        while done < bytes.len() {
            let at = offset + done as u64;
            self.check_cancelled(at)?;
            let size = match store {
                BitmapStore::Imported => self.imported.size(),
                BitmapStore::New => self.new.size(),
                BitmapStore::Refined => self.refined.size(),
            };
            if size != expected_size {
                return Err(self.error(
                    at,
                    TextComposeErrorKind::Malformed("bitmap store size changed"),
                ));
            }
            let len = (bytes.len() - done).min(self.budget.max_request_bytes);
            self.check_cap(
                "source read calls",
                self.budget.max_source_read_calls,
                self.progress.source_read_calls + 1,
            )?;
            self.check_cap(
                "source read bytes",
                self.budget.max_source_read_bytes,
                self.progress.source_bytes_read + len as u64,
            )?;
            self.note_request(len);
            self.progress.source_read_calls += 1;
            let result = match store {
                BitmapStore::Imported => {
                    self.imported
                        .read_at(at, &mut bytes[done..done + len])
                        .await
                }
                BitmapStore::New => self.new.read_at(at, &mut bytes[done..done + len]).await,
                BitmapStore::Refined => {
                    self.refined.read_at(at, &mut bytes[done..done + len]).await
                }
            };
            let read = result.map_err(|error| self.source_error(at, store, error))?;
            if read > len {
                return Err(self.error(
                    at,
                    TextComposeErrorKind::Malformed("symbol source overreported read"),
                ));
            }
            if read == 0 {
                return Err(self.error(
                    at,
                    TextComposeErrorKind::Source {
                        store,
                        error: Error::TruncatedInput {
                            offset: at,
                            expected: len as u64,
                            available: 0,
                        },
                    },
                ));
            }
            done += read;
            self.progress.source_bytes_read += read as u64;
            self.check_cancelled(at + read as u64)?;
            let after_size = match store {
                BitmapStore::Imported => self.imported.size(),
                BitmapStore::New => self.new.size(),
                BitmapStore::Refined => self.refined.size(),
            };
            if after_size != expected_size {
                return Err(self.error(
                    at,
                    TextComposeErrorKind::Malformed("bitmap store size changed"),
                ));
            }
        }
        Ok(())
    }

    async fn output_write(&mut self, offset: u64, bytes: &[u8]) -> TextComposeResult<()> {
        let mut done = 0;
        while done < bytes.len() {
            let at = offset + done as u64;
            self.check_cancelled(at)?;
            let len = (bytes.len() - done).min(self.budget.max_request_bytes);
            self.check_cap(
                "output write calls",
                self.budget.max_output_write_calls,
                self.progress.output_write_calls + 1,
            )?;
            self.check_cap(
                "output bytes",
                self.budget
                    .max_output_bytes
                    .min(self.limits.max_output_bytes),
                self.progress.output_bytes_written + len as u64,
            )?;
            self.note_request(len);
            self.progress.output_write_calls += 1;
            let written = self
                .output
                .write(&bytes[done..done + len])
                .await
                .map_err(|error| self.output_error(at, error))?;
            if written > len {
                return Err(self.error(
                    at,
                    TextComposeErrorKind::Malformed("output overreported write"),
                ));
            }
            if written == 0 {
                return Err(self.error(
                    at,
                    TextComposeErrorKind::Output(Error::Io(std::io::Error::new(
                        std::io::ErrorKind::WriteZero,
                        "output made no progress",
                    ))),
                ));
            }
            done += written;
            self.progress.output_bytes_written += written as u64;
            self.check_cancelled(at + written as u64)?;
        }
        Ok(())
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
        let (store, base, descriptor, size) = match instance.bitmap {
            TextBitmap::Stored(stored) if !instance.ri && stored == *reference => {
                match stored.store {
                    SymbolStore::Imported => (
                        BitmapStore::Imported,
                        self.imported_base,
                        stored.symbol,
                        self.imported_size,
                    ),
                    SymbolStore::New => (
                        BitmapStore::New,
                        self.new_base,
                        stored.symbol,
                        self.new_size,
                    ),
                }
            }
            TextBitmap::Refined { store_base, symbol }
                if instance.ri && store_base == self.refined_base =>
            {
                (
                    BitmapStore::Refined,
                    self.refined_base,
                    symbol,
                    self.refined.size(),
                )
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
        let current_size = match store {
            BitmapStore::Imported => self.imported.size(),
            BitmapStore::New => self.new.size(),
            BitmapStore::Refined => self.refined.size(),
        };
        if current_size != size {
            return Err(self.error(
                0,
                TextComposeErrorKind::Malformed("bitmap store size changed"),
            ));
        }
        if descriptor.width != instance.width || descriptor.height != instance.height {
            return Err(self.error(
                0,
                TextComposeErrorKind::Malformed("bitmap geometry differs from placement"),
            ));
        }
        let (stride, bytes) = descriptor_bytes(descriptor).map_err(|kind| self.error(0, kind))?;
        self.check_cap("symbol bytes", self.budget.max_symbol_bytes, bytes)?;
        self.check_cap("row bytes", self.budget.max_row_bytes as u64, stride)?;
        let absolute = base
            .checked_add(descriptor.relative_store_offset)
            .ok_or_else(|| {
                self.error(
                    0,
                    TextComposeErrorKind::InvalidSpan("symbol start overflows"),
                )
            })?;
        let end = absolute.checked_add(bytes).ok_or_else(|| {
            self.error(0, TextComposeErrorKind::InvalidSpan("symbol end overflows"))
        })?;
        if end > size {
            return Err(self.error(
                absolute,
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
            source_size: size,
            descriptor,
            x0,
            y0,
            x1,
            y1,
        }))
    }

    async fn compose_event(
        &mut self,
        instance: TextInstance,
        target: &mut Vec<u8>,
        source: &mut Vec<u8>,
    ) -> TextComposeResult<()> {
        self.progress.stage = TextComposeStage::Instance;
        let Some(CheckedEvent {
            store,
            source_size,
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
        self.grow_source_row(source, source_stride, target)?;
        let source_base = match store {
            BitmapStore::Imported => self.imported_base,
            BitmapStore::New => self.new_base,
            BitmapStore::Refined => self.refined_base,
        } + descriptor.relative_store_offset;
        for y in y0..y1 {
            self.progress.current_row = y as u32;
            let source_y = (y as i64 - instance.y) as u64;
            let source_offset = source_base + source_y * source_stride as u64;
            self.source_read(
                store,
                source_size,
                source_offset,
                &mut source[..source_stride],
            )
            .await?;
            let target_offset = y * self.row_stride as u64;
            self.scratch_read(target_offset, target).await?;
            for x in x0..x1 {
                let sx = (x as i64 - instance.x) as usize;
                let source_pixel = source[sx / 8] & (0x80 >> (sx % 8)) != 0;
                let byte = &mut target[x as usize / 8];
                let mask = 0x80 >> (x as usize % 8);
                let target_pixel = *byte & mask != 0;
                let value = combine(target_pixel, source_pixel, self.header.flags.combination);
                if value {
                    *byte |= mask;
                } else {
                    *byte &= !mask;
                }
            }
            target[self.row_stride - 1] &= padding_mask(self.header.region.width);
            self.scratch_write(target_offset, target).await?;
        }
        let touched = (x1 - x0) * (y1 - y0);
        self.progress.touched_pixels += touched;
        self.progress.work_units += touched;
        self.progress.completed_instances += 1;
        Ok(())
    }

    async fn compose_inner(&mut self) -> TextComposeResult<TextComposeReport> {
        let mut target = self.make_target_row()?;
        let mut source = Vec::new();
        self.progress.stage = TextComposeStage::Initialize;
        self.check_cancelled(0)?;
        self.scratch
            .set_len(self.packed_bytes)
            .await
            .map_err(|error| self.scratch_error(0, error))?;
        if self.scratch.size() != self.packed_bytes {
            return Err(self.error(
                0,
                TextComposeErrorKind::Malformed("scratch set_len size differs"),
            ));
        }
        target.fill(if self.header.flags.default_pixel {
            0xff
        } else {
            0
        });
        target[self.row_stride - 1] &= padding_mask(self.header.region.width);
        self.check_cap(
            "composition work",
            self.budget.max_work_units,
            self.packed_bytes,
        )?;
        for y in 0..self.header.region.height {
            self.progress.current_row = y;
            self.scratch_write(u64::from(y) * self.row_stride as u64, &target)
                .await?;
            self.progress.work_units += self.row_stride as u64;
        }
        self.check_cancelled(self.packed_bytes)?;
        self.scratch
            .flush()
            .await
            .map_err(|error| self.scratch_error(self.packed_bytes, error))?;
        loop {
            self.progress.stage = TextComposeStage::Instance;
            self.check_cancelled(self.packed_bytes)?;
            let event = self.instances.next().await.map_err(|error| {
                let offset = error.offset;
                self.error(offset, TextComposeErrorKind::Instance(Box::new(error)))
            })?;
            match event {
                Some(instance) => {
                    self.compose_event(instance, &mut target, &mut source)
                        .await?
                }
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
        self.check_cancelled(self.packed_bytes)?;
        self.scratch
            .flush()
            .await
            .map_err(|error| self.scratch_error(self.packed_bytes, error))?;
        self.progress.stage = TextComposeStage::Readback;
        self.check_cap(
            "composition work",
            self.budget.max_work_units,
            self.progress.work_units + self.packed_bytes,
        )?;
        for y in 0..self.header.region.height {
            self.progress.current_row = y;
            let offset = u64::from(y) * self.row_stride as u64;
            self.scratch_read(offset, &mut target).await?;
            // A malformed or mutating store must not leak padding bits.
            target[self.row_stride - 1] &= padding_mask(self.header.region.width);
            self.output_write(offset, &target).await?;
            self.progress.work_units += self.row_stride as u64;
            self.progress.output_rows += 1;
        }
        self.progress.stage = TextComposeStage::OutputFlush;
        self.check_cancelled(self.packed_bytes)?;
        self.output
            .flush()
            .await
            .map_err(|error| self.output_error(self.packed_bytes, error))?;
        self.check_cancelled(self.packed_bytes)?;
        if self.scratch.size() != self.packed_bytes {
            return Err(self.error(
                self.packed_bytes,
                TextComposeErrorKind::Malformed("scratch size changed"),
            ));
        }
        Ok(TextComposeReport {
            width: self.header.region.width,
            height: self.header.region.height,
            row_stride: self.row_stride as u32,
            packed_bytes: self.packed_bytes,
            progress: self.progress,
        })
    }

    /// Run once. Dropping the pending future or any failure poisons this
    /// session, its scratch contents, and any partial final output.
    pub async fn compose(&mut self) -> TextComposeResult<TextComposeReport> {
        if self.started || self.poisoned || self.complete {
            return Err(self.error(0, TextComposeErrorKind::Poisoned));
        }
        self.started = true;
        self.poisoned = true;
        let result = self.compose_inner().await;
        if result.is_ok() {
            self.poisoned = false;
            self.complete = true;
            self.progress.stage = TextComposeStage::Complete;
        }
        result.map(|mut report| {
            report.progress = self.progress();
            report
        })
    }
}

#[cfg(test)]
#[path = "text_composer/tests.rs"]
mod tests;
