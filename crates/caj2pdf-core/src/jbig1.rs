// SPDX-License-Identifier: MIT

//! Bounded CAJ-family type-0 rows using caller-supplied T.82 probability states.

use crate::fallible::reserve_exact;
use crate::qm::{
    ArithmeticBudget, ArithmeticDecoder, ArithmeticError, ArithmeticErrorKind, ArithmeticSnapshot,
    ContextBank, ContextState, EncodedSpan, QM_STATE_COUNT, QmState, QmTable, StripeMode,
};
use crate::{Cancellation, Error, Limits, RangedSource, SequentialSink, read_exact_at, write_all};
use std::{error, fmt, mem};

const DIB_BYTES: u64 = 48;
const CONTEXT_COUNT: usize = 1024;
const CONTROL_CONTEXT: usize = 457;
const QM_BUFFER_BYTES: u64 = 256;

/// Limits for one image in addition to the shared I/O and arithmetic limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Type0Budget {
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels: u64,
    pub max_context_work: u64,
}

impl Default for Type0Budget {
    fn default() -> Self {
        Self {
            max_width: 32_768,
            max_height: 32_768,
            max_pixels: 12_000_000,
            max_context_work: 120_000_000,
        }
    }
}

/// Absolute DIB-plus-coded-byte range and type from the outer HN/C8 record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Type0Span {
    /// The caller must supply the parsed outer record type; only zero is accepted.
    pub record_type: u32,
    pub offset: u64,
    pub length: u64,
}

/// Checked DIB geometry. Rows emitted by this API have `dib_stride` bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Type0Info {
    pub width: u32,
    pub height: u32,
    pub dib_stride: usize,
    pub visible_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Type0Progress {
    pub info: Type0Info,
    pub rows_written: u32,
    pub output_bytes_written: u64,
    pub arithmetic: ArithmeticSnapshot,
    /// Set before a row or final flush awaits I/O; a dropped future leaves it set.
    pub poisoned: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Type0Report {
    pub image: Type0Span,
    pub coded: EncodedSpan,
    pub progress: Type0Progress,
}

#[derive(Debug)]
pub enum Type0ErrorKind {
    InvalidSpan(&'static str),
    Truncated(&'static str),
    Malformed(&'static str),
    Unsupported {
        field: &'static str,
        value: u64,
    },
    LimitExceeded {
        resource: &'static str,
        limit: u64,
        attempted: u64,
    },
    AllocationFailed,
    Cancelled,
    Source(Error),
    Sink(Error),
    Arithmetic(ArithmeticError),
    Incomplete,
    Poisoned,
}

#[derive(Debug)]
pub struct Type0Error {
    /// Absolute byte offset in the supplied source, when the operation failed.
    pub offset: u64,
    pub rows_written: u32,
    pub output_bytes_written: u64,
    pub kind: Type0ErrorKind,
}

pub type Type0Result<T> = Result<T, Type0Error>;

impl fmt::Display for Type0Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CAJ type-0 image at source byte {}: ", self.offset)?;
        match &self.kind {
            Type0ErrorKind::InvalidSpan(reason) => write!(f, "invalid image span: {reason}"),
            Type0ErrorKind::Truncated(field) => write!(f, "truncated {field}"),
            Type0ErrorKind::Malformed(field) => write!(f, "malformed {field}"),
            Type0ErrorKind::Unsupported { field, value } => {
                write!(f, "unsupported {field} ({value})")
            }
            Type0ErrorKind::LimitExceeded {
                resource,
                limit,
                attempted,
            } => {
                write!(f, "{resource} limit {limit} exceeded by {attempted}")
            }
            Type0ErrorKind::AllocationFailed => f.write_str("row allocation failed"),
            Type0ErrorKind::Cancelled => f.write_str("cancelled"),
            Type0ErrorKind::Source(source) => write!(f, "source: {source}"),
            Type0ErrorKind::Sink(source) => write!(f, "sink: {source}"),
            Type0ErrorKind::Arithmetic(source) => write!(f, "arithmetic: {source}"),
            Type0ErrorKind::Incomplete => f.write_str("not all rows were decoded"),
            Type0ErrorKind::Poisoned => f.write_str("decoder is poisoned"),
        }
    }
}

impl error::Error for Type0Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.kind {
            Type0ErrorKind::Source(error) | Type0ErrorKind::Sink(error) => Some(error),
            Type0ErrorKind::Arithmetic(error) => Some(error),
            _ => None,
        }
    }
}

fn at(offset: u64, kind: Type0ErrorKind) -> Type0Error {
    Type0Error {
        offset,
        rows_written: 0,
        output_bytes_written: 0,
        kind,
    }
}

/// Report arithmetic cancellation as image cancellation.
fn arithmetic_kind(error: ArithmeticError) -> Type0ErrorKind {
    if matches!(error.kind, ArithmeticErrorKind::Cancelled) {
        Type0ErrorKind::Cancelled
    } else {
        Type0ErrorKind::Arithmetic(error)
    }
}

fn malformed(offset: u64, reason: &'static str) -> Type0Error {
    at(offset, Type0ErrorKind::Malformed(reason))
}

fn limit(offset: u64, resource: &'static str, maximum: u64, attempted: u64) -> Type0Error {
    at(
        offset,
        Type0ErrorKind::LimitExceeded {
            resource,
            limit: maximum,
            attempted,
        },
    )
}

fn le_u16(header: &[u8; 48], start: usize) -> u16 {
    u16::from_le_bytes([header[start], header[start + 1]])
}

fn le_u32(header: &[u8; 48], start: usize) -> u32 {
    u32::from_le_bytes([
        header[start],
        header[start + 1],
        header[start + 2],
        header[start + 3],
    ])
}

fn checked_info(
    header: &[u8; 48],
    span: Type0Span,
    limits: &Limits,
    arithmetic: ArithmeticBudget,
    budget: Type0Budget,
) -> Type0Result<Type0Info> {
    let base = span.offset;
    if le_u32(header, 0) != 40 {
        return Err(at(
            base,
            Type0ErrorKind::Unsupported {
                field: "DIB header size",
                value: u64::from(le_u32(header, 0)),
            },
        ));
    }
    let width_i = i32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    let height_i = i32::from_le_bytes([header[8], header[9], header[10], header[11]]);
    if width_i <= 0 || height_i <= 0 {
        return Err(malformed(base + 4, "nonpositive DIB dimensions"));
    }
    let width = width_i as u32;
    let height = height_i as u32;
    if width > budget.max_width {
        return Err(limit(
            base + 4,
            "image width",
            u64::from(budget.max_width),
            u64::from(width),
        ));
    }
    if height > budget.max_height {
        return Err(limit(
            base + 8,
            "image height",
            u64::from(budget.max_height),
            u64::from(height),
        ));
    }
    if le_u16(header, 12) != 1 {
        return Err(at(
            base + 12,
            Type0ErrorKind::Unsupported {
                field: "DIB planes",
                value: u64::from(le_u16(header, 12)),
            },
        ));
    }
    if le_u16(header, 14) != 1 {
        return Err(at(
            base + 14,
            Type0ErrorKind::Unsupported {
                field: "DIB bit count",
                value: u64::from(le_u16(header, 14)),
            },
        ));
    }
    if le_u32(header, 16) != 0 {
        return Err(at(
            base + 16,
            Type0ErrorKind::Unsupported {
                field: "DIB compression",
                value: u64::from(le_u32(header, 16)),
            },
        ));
    }
    let colors = le_u32(header, 32);
    if colors != 0 && colors != 2 {
        return Err(at(
            base + 32,
            Type0ErrorKind::Unsupported {
                field: "DIB colors used",
                value: u64::from(colors),
            },
        ));
    }
    if header[40..43] != [0xff; 3] || header[44..47] != [0; 3] {
        return Err(at(
            base + 40,
            Type0ErrorKind::Unsupported {
                field: "DIB palette",
                value: 0,
            },
        ));
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(malformed(base + 4, "pixel area overflows"))?;
    if pixels > budget.max_pixels {
        return Err(limit(base + 4, "image pixels", budget.max_pixels, pixels));
    }
    let potential_symbols = u64::from(width)
        .checked_add(1)
        .and_then(|row| row.checked_mul(u64::from(height)))
        .ok_or(malformed(base + 4, "symbol count overflows"))?;
    if potential_symbols > arithmetic.max_symbols {
        return Err(limit(
            base + 4,
            "arithmetic symbols",
            arithmetic.max_symbols,
            potential_symbols,
        ));
    }
    let context_work = pixels
        .checked_mul(10)
        .and_then(|value| value.checked_add(u64::from(height)))
        .ok_or(malformed(base + 4, "context work overflows"))?;
    if context_work > budget.max_context_work {
        return Err(limit(
            base + 4,
            "context work",
            budget.max_context_work,
            context_work,
        ));
    }
    let stride_u64 = u64::from(width)
        .checked_add(31)
        .map(|value| value / 32 * 4)
        .ok_or(malformed(base + 4, "DIB stride overflows"))?;
    let visible_u64 = u64::from(width)
        .checked_add(7)
        .map(|value| value / 8)
        .ok_or(malformed(base + 4, "visible stride overflows"))?;
    let output = stride_u64
        .checked_mul(u64::from(height))
        .ok_or(malformed(base + 4, "DIB output size overflows"))?;
    if output > limits.max_output_bytes {
        return Err(limit(
            base + 4,
            "output bytes",
            limits.max_output_bytes,
            output,
        ));
    }
    let declared_size = u64::from(le_u32(header, 20));
    if declared_size != 0 && declared_size != output {
        return Err(malformed(
            base + 20,
            "DIB image size differs from stride times height",
        ));
    }
    let failure = malformed(base + 4, "working allocation calculation overflows");
    let allocated = stride_u64
        .checked_mul(3)
        .and_then(|bytes| {
            bytes.checked_add(
                (CONTEXT_COUNT * mem::size_of::<ContextState>()
                    + QM_STATE_COUNT * mem::size_of::<QmState>()) as u64,
            )
        })
        .and_then(|bytes| bytes.checked_add(QM_BUFFER_BYTES))
        .ok_or(failure)?;
    if allocated > limits.max_allocation_bytes {
        return Err(limit(
            base + 4,
            "working allocation bytes",
            limits.max_allocation_bytes,
            allocated,
        ));
    }
    // A u32 width has a DIB stride of at most 2^29 + 4 bytes, which every
    // supported (at least 32-bit) `usize` represents.
    let dib_stride = stride_u64 as usize;
    let visible_bytes = visible_u64 as usize;
    Ok(Type0Info {
        width,
        height,
        dib_stride,
        visible_bytes,
    })
}

fn blank_row(stride: usize, offset: u64) -> Type0Result<Vec<u8>> {
    let mut row = Vec::new();
    let failed = at(offset, Type0ErrorKind::AllocationFailed);
    reserve_exact(&mut row, stride, failed)?;
    row.resize(stride, 0);
    Ok(row)
}

fn pixel(row: &[u8], width: u32, x: i64) -> usize {
    if x < 0 || x >= i64::from(width) {
        return 0;
    }
    let x = x as usize;
    usize::from(row[x / 8] & (0x80 >> (x % 8)) != 0)
}

/// Context order: current x-2..x-1, previous x-2..x+2,
/// then previous-two x-1..x+1, most-significant bit first.
fn three_line_context(
    previous_two: &[u8],
    previous: &[u8],
    current: &[u8],
    width: u32,
    x: u32,
) -> usize {
    let x = i64::from(x);
    let mut cx = 0;
    for dx in [-2, -1] {
        cx = (cx << 1) | pixel(current, width, x + dx);
    }
    for dx in [-2, -1, 0, 1, 2] {
        cx = (cx << 1) | pixel(previous, width, x + dx);
    }
    for dx in [-1, 0, 1] {
        cx = (cx << 1) | pixel(previous_two, width, x + dx);
    }
    cx
}

/// Checks that need no source bytes: shared limits, cancellation, the outer
/// record type, and the span's containment in the source. Not generic, so
/// every source and cancellation type shares one copy.
fn check_span(
    source_size: u64,
    image: Type0Span,
    limits: &Limits,
    cancellation: &dyn Cancellation,
) -> Type0Result<()> {
    limits
        .validate()
        .map_err(|error| at(image.offset, Type0ErrorKind::Source(error)))?;
    if cancellation.is_cancelled() {
        return Err(at(image.offset, Type0ErrorKind::Cancelled));
    }
    if image.record_type != 0 {
        return Err(at(
            image.offset,
            Type0ErrorKind::Unsupported {
                field: "HN/C8 image record type",
                value: u64::from(image.record_type),
            },
        ));
    }
    let end = image.offset.checked_add(image.length).ok_or_else(|| {
        at(
            image.offset,
            Type0ErrorKind::InvalidSpan("end overflows u64"),
        )
    })?;
    if image.length <= DIB_BYTES {
        return Err(at(end, Type0ErrorKind::Truncated("DIB and coded bytes")));
    }
    if end > source_size {
        return Err(at(
            image.offset,
            Type0ErrorKind::InvalidSpan("outside source size"),
        ));
    }
    if image.length > limits.max_input_bytes {
        return Err(limit(
            image.offset,
            "image span bytes",
            limits.max_input_bytes,
            image.length,
        ));
    }
    Ok(())
}

async fn read_info<S: RangedSource, C: Cancellation>(
    source: &mut S,
    image: Type0Span,
    limits: &Limits,
    cancellation: &C,
    arithmetic_budget: ArithmeticBudget,
    budget: Type0Budget,
) -> Type0Result<Type0Info> {
    let mut header = [0_u8; DIB_BYTES as usize];
    let mut done = 0;
    while done < header.len() {
        let count = (header.len() - done).min(limits.io_chunk_bytes);
        let absolute = image.offset + done as u64;
        read_exact_at(
            source,
            absolute,
            &mut header[done..done + count],
            limits,
            cancellation,
        )
        .await
        .map_err(|error| {
            at(
                absolute,
                match error {
                    Error::Cancelled => Type0ErrorKind::Cancelled,
                    other => Type0ErrorKind::Source(other),
                },
            )
        })?;
        done += count;
    }
    checked_info(&header, image, limits, arithmetic_budget, budget)
}

/// Validate one type-0 span and its 48-byte DIB wrapper without decoding.
///
/// This performs the same checks, in the same order, as the header part of
/// [`Type0Decoder::new`], except that it needs no context bank or sink. A
/// caller can use the returned geometry to prepare a destination (for
/// example a PDF image dictionary) before constructing the decoder. It reads
/// only the 48 wrapper bytes, in chunks of at most `Limits::io_chunk_bytes`.
pub async fn read_type0_info<S: RangedSource, C: Cancellation>(
    source: &mut S,
    image: Type0Span,
    limits: &Limits,
    cancellation: &C,
    arithmetic_budget: ArithmeticBudget,
    budget: Type0Budget,
) -> Type0Result<Type0Info> {
    check_span(source.size(), image, limits, cancellation)?;
    read_info(
        source,
        image,
        limits,
        cancellation,
        arithmetic_budget,
        budget,
    )
    .await
}

/// One image with one arithmetic SCD. A failed or dropped row future poisons
/// this object; the caller must discard any partial sink output.
pub struct Type0Decoder<'a, S: RangedSource, W: SequentialSink, C: Cancellation> {
    arithmetic: ArithmeticDecoder<'a, S, C>,
    sink: &'a mut W,
    limits: &'a Limits,
    cancellation: &'a C,
    image: Type0Span,
    coded: EncodedSpan,
    info: Type0Info,
    previous_two: Vec<u8>,
    previous: Vec<u8>,
    current: Vec<u8>,
    rows_written: u32,
    output_bytes_written: u64,
    poisoned: bool,
}

impl<'a, S: RangedSource, W: SequentialSink, C: Cancellation> Type0Decoder<'a, S, W, C> {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        source: &'a mut S,
        image: Type0Span,
        table: &'a QmTable,
        contexts: &'a mut ContextBank,
        sink: &'a mut W,
        limits: &'a Limits,
        cancellation: &'a C,
        arithmetic_budget: ArithmeticBudget,
        budget: Type0Budget,
    ) -> Type0Result<Self> {
        check_span(source.size(), image, limits, cancellation)?;
        if contexts.state(CONTEXT_COUNT - 1).is_none() || contexts.state(CONTEXT_COUNT).is_some() {
            return Err(malformed(
                image.offset,
                "expected exactly 1024 arithmetic contexts",
            ));
        }
        let info = read_info(
            source,
            image,
            limits,
            cancellation,
            arithmetic_budget,
            budget,
        )
        .await?;
        let previous_two = blank_row(info.dib_stride, image.offset)?;
        let previous = blank_row(info.dib_stride, image.offset)?;
        let current = blank_row(info.dib_stride, image.offset)?;
        let coded = EncodedSpan {
            offset: image.offset + DIB_BYTES,
            length: image.length - DIB_BYTES,
        };
        let arithmetic = ArithmeticDecoder::new(
            source,
            coded,
            table,
            contexts,
            StripeMode::Reset,
            limits,
            cancellation,
            arithmetic_budget,
        )
        .await
        .map_err(|error| at(error.offset.unwrap_or(coded.offset), arithmetic_kind(error)))?;
        Ok(Self {
            arithmetic,
            sink,
            limits,
            cancellation,
            image,
            coded,
            info,
            previous_two,
            previous,
            current,
            rows_written: 0,
            output_bytes_written: 0,
            poisoned: false,
        })
    }

    pub fn progress(&self) -> Type0Progress {
        Type0Progress {
            info: self.info,
            rows_written: self.rows_written,
            output_bytes_written: self.output_bytes_written,
            arithmetic: self.arithmetic.snapshot(),
            poisoned: self.poisoned,
        }
    }

    fn failed(&self, kind: Type0ErrorKind) -> Type0Error {
        let offset = match &kind {
            Type0ErrorKind::Arithmetic(error) => error
                .offset
                .unwrap_or(self.arithmetic.snapshot().next_input_offset),
            _ => self.arithmetic.snapshot().next_input_offset,
        };
        Type0Error {
            offset,
            rows_written: self.rows_written,
            output_bytes_written: self.output_bytes_written,
            kind,
        }
    }

    fn arithmetic_failure(&self, error: ArithmeticError) -> Type0Error {
        self.failed(arithmetic_kind(error))
    }

    /// Decode and write exactly one display-order DIB-stride row.
    pub async fn decode_next_row(&mut self) -> Type0Result<bool> {
        if self.poisoned {
            return Err(self.failed(Type0ErrorKind::Poisoned));
        }
        if self.rows_written == self.info.height {
            return Ok(false);
        }
        if self.cancellation.is_cancelled() {
            return Err(self.failed(Type0ErrorKind::Cancelled));
        }
        // Poison before awaiting source or sink; a dropped future cannot resume
        // an arithmetic register or partially written row.
        self.poisoned = true;
        self.current.fill(0);
        let copy_previous = self
            .arithmetic
            .decode_symbol(CONTROL_CONTEXT)
            .await
            .map_err(|error| self.arithmetic_failure(error))?;
        if copy_previous {
            self.current.copy_from_slice(&self.previous);
        } else {
            for x in 0..self.info.width {
                let cx = three_line_context(
                    &self.previous_two,
                    &self.previous,
                    &self.current,
                    self.info.width,
                    x,
                );
                let bit = self
                    .arithmetic
                    .decode_symbol(cx)
                    .await
                    .map_err(|error| self.arithmetic_failure(error))?;
                if bit {
                    let x = x as usize;
                    self.current[x / 8] |= 0x80 >> (x % 8);
                }
            }
        }
        write_all(
            self.sink,
            &self.current,
            &mut self.output_bytes_written,
            self.limits,
            self.cancellation,
        )
        .await
        .map_err(|error| match error {
            Error::Cancelled => self.failed(Type0ErrorKind::Cancelled),
            other => self.failed(Type0ErrorKind::Sink(other)),
        })?;
        mem::swap(&mut self.previous_two, &mut self.previous);
        mem::swap(&mut self.previous, &mut self.current);
        self.rows_written += 1;
        self.poisoned = false;
        Ok(true)
    }

    /// Validate row count and flush only after all rows were written.
    pub async fn finish(mut self) -> Type0Result<Type0Report> {
        if self.poisoned {
            return Err(self.failed(Type0ErrorKind::Poisoned));
        }
        if self.rows_written != self.info.height {
            return Err(self.failed(Type0ErrorKind::Incomplete));
        }
        if self.cancellation.is_cancelled() {
            return Err(self.failed(Type0ErrorKind::Cancelled));
        }
        self.poisoned = true;
        let snapshot = self.arithmetic.snapshot();
        let offset = snapshot.next_input_offset;
        let rows_written = self.rows_written;
        let output_bytes_written = self.output_bytes_written;
        // Only cancellation can fail here: the expected count is the
        // decoder's own, and the arithmetic decoder is poisoned only inside
        // a row, which leaves this decoder poisoned too.
        self.arithmetic
            .finish(snapshot.symbols_decoded)
            .map_err(|error| Type0Error {
                offset: error.offset.unwrap_or(offset),
                rows_written,
                output_bytes_written,
                kind: arithmetic_kind(error),
            })?;
        self.sink.flush().await.map_err(|error| Type0Error {
            offset,
            rows_written,
            output_bytes_written,
            kind: match error {
                Error::Cancelled => Type0ErrorKind::Cancelled,
                other => Type0ErrorKind::Sink(other),
            },
        })?;
        if self.cancellation.is_cancelled() {
            return Err(Type0Error {
                offset,
                rows_written,
                output_bytes_written,
                kind: Type0ErrorKind::Cancelled,
            });
        }
        Ok(Type0Report {
            image: self.image,
            coded: self.coded,
            progress: Type0Progress {
                info: self.info,
                rows_written,
                output_bytes_written,
                arithmetic: snapshot,
                poisoned: false,
            },
        })
    }
}

#[cfg(test)]
mod tests;
