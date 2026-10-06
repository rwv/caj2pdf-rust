// SPDX-License-Identifier: MIT

//! Bounded CAJ-family type-0 rows using the standard T.82 probability states.

use crate::fallible::reserve_exact;
use crate::qm::{
    ArithmeticDecoder, ArithmeticSnapshot, CodedSpan, ContextBank, ContextState, QM_STATE_COUNT,
    QmState, QmTable,
};
use crate::{
    Cancellation, Context, Error, ErrorKind, Hnc8Stage, Limits, Payload, RangedSource, Result,
    read_exact_at, write_counted,
};
use std::io::Write;
use std::mem;

const DIB_BYTES: u64 = 48;
const CONTEXT_COUNT: usize = 1024;
const CONTROL_CONTEXT: usize = 457;

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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Type0Report {
    pub image: Type0Span,
    pub coded: CodedSpan,
    pub progress: Type0Progress,
}

/// A type-0 image is an HN/C8 image; the caller adds the page and image.
const HNC8: Context = Context::HNC8;

fn malformed(offset: u64, reason: &'static str) -> Error {
    Error::malformed(offset, reason).within(HNC8)
}

fn unsupported(offset: u64, reason: &'static str) -> Error {
    Error::unsupported(offset, reason).within(HNC8)
}

fn limit(offset: u64, resource: &'static str, maximum: u64, attempted: u64) -> Error {
    Error::limit(resource, maximum, attempted)
        .at(offset)
        .within(HNC8)
}

fn cancelled(offset: u64) -> Error {
    Error::cancelled().at(offset).within(HNC8)
}

/// A refused sink write is reported at the PDF stage.
fn sink(error: Error, offset: u64) -> Error {
    match error.kind {
        ErrorKind::Cancelled => cancelled(offset),
        _ => error.or_at(
            offset,
            Context::Hnc8 {
                variant: None,
                page: None,
                image: None,
                segment: None,
                stage: Some(Hnc8Stage::Pdf),
            },
        ),
    }
}

const OUTSIDE_SOURCE: &str = "image span is outside the source";

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

fn checked_info(header: &[u8; 48], span: Type0Span, limits: &Limits) -> Result<Type0Info> {
    let base = span.offset;
    if le_u32(header, 0) != 40 {
        return Err(unsupported(base, "DIB header size"));
    }
    let width_i = i32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    let height_i = i32::from_le_bytes([header[8], header[9], header[10], header[11]]);
    if width_i <= 0 || height_i <= 0 {
        return Err(malformed(base + 4, "nonpositive DIB dimensions"));
    }
    let width = width_i as u32;
    let height = height_i as u32;
    if le_u16(header, 12) != 1 {
        return Err(unsupported(base + 12, "DIB planes"));
    }
    if le_u16(header, 14) != 1 {
        return Err(unsupported(base + 14, "DIB bit count"));
    }
    if le_u32(header, 16) != 0 {
        return Err(unsupported(base + 16, "DIB compression"));
    }
    let colors = le_u32(header, 32);
    if colors != 0 && colors != 2 {
        return Err(unsupported(base + 32, "DIB colors used"));
    }
    if header[40..43] != [0xff; 3] || header[44..47] != [0; 3] {
        return Err(unsupported(base + 40, "DIB palette"));
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(malformed(base + 4, "pixel area overflows"))?;
    if pixels > limits.max_image_pixels {
        return Err(limit(
            base + 4,
            "image pixels",
            limits.max_image_pixels,
            pixels,
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

fn blank_row(stride: usize, offset: u64, limits: &Limits) -> Result<Vec<u8>> {
    let mut row = Vec::new();
    let failed = limits
        .allocation_refused("type-0 row bytes", stride as u64)
        .at(offset)
        .within(HNC8);
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

/// Checks that need no source bytes: cancellation, the outer
/// record type, and the span's containment in the source. Not generic, so
/// every source and cancellation type shares one copy.
fn check_span(
    source_size: u64,
    image: Type0Span,
    limits: &Limits,
    cancellation: &dyn Cancellation,
) -> Result<()> {
    if cancellation.is_cancelled() {
        return Err(cancelled(image.offset));
    }
    if image.record_type != 0 {
        return Err(unsupported(image.offset, "HN/C8 image record type"));
    }
    let end = image
        .offset
        .checked_add(image.length)
        .ok_or_else(|| malformed(image.offset, "image span end overflows u64"))?;
    if image.length <= DIB_BYTES {
        return Err(Error::truncated(end, DIB_BYTES + 1, image.length)
            .because("DIB and coded bytes")
            .within(HNC8));
    }
    if end > source_size {
        return Err(malformed(image.offset, OUTSIDE_SOURCE));
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

fn read_info<S: RangedSource, C: Cancellation>(
    source: &mut S,
    image: Type0Span,
    limits: &Limits,
    cancellation: &C,
) -> Result<Type0Info> {
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
        .map_err(|error| error.or_at(absolute, HNC8))?;
        done += count;
    }
    checked_info(&header, image, limits)
}

/// Validate one type-0 span and its 48-byte DIB wrapper without decoding.
///
/// This performs the same checks, in the same order, as the header part of
/// [`Type0Decoder::new`], except that it needs no context bank or sink. A
/// caller can use the returned geometry to prepare a destination (for
/// example a PDF image dictionary) before constructing the decoder. It reads
/// only the 48 wrapper bytes, in chunks of at most `Limits::io_chunk_bytes`.
pub fn read_type0_info<S: RangedSource, C: Cancellation>(
    source: &mut S,
    image: Type0Span,
    limits: &Limits,
    cancellation: &C,
) -> Result<Type0Info> {
    check_span(source.size(), image, limits, cancellation)?;
    read_info(source, image, limits, cancellation)
}

/// One image with one arithmetic SCD read from memory. After an error the
/// caller must discard the decoder and any partial sink output.
pub struct Type0Decoder<'a, W: Write, C: Cancellation> {
    arithmetic: ArithmeticDecoder<'a>,
    sink: &'a mut W,
    limits: &'a Limits,
    cancellation: &'a C,
    image: Type0Span,
    coded: CodedSpan,
    info: Type0Info,
    previous_two: Vec<u8>,
    previous: Vec<u8>,
    current: Vec<u8>,
    rows_written: u32,
    output_bytes_written: u64,
}

impl<'a, W: Write, C: Cancellation> Type0Decoder<'a, W, C> {
    /// Check the DIB wrapper and start the SCD of `image`, whose bytes must
    /// all be in `input`.
    pub fn new(
        input: Payload<'a>,
        image: Type0Span,
        table: &'a QmTable,
        contexts: &'a mut ContextBank,
        sink: &'a mut W,
        limits: &'a Limits,
        cancellation: &'a C,
    ) -> Result<Self> {
        check_span(input.size(), image, limits, cancellation)?;
        if contexts.get(CONTEXT_COUNT - 1).is_none() || contexts.get(CONTEXT_COUNT).is_some() {
            return Err(malformed(
                image.offset,
                "expected exactly 1024 arithmetic contexts",
            ));
        }
        let header = input
            .get(image.offset, DIB_BYTES)
            .and_then(|header| <&[u8; DIB_BYTES as usize]>::try_from(header).ok())
            .ok_or_else(|| malformed(image.offset, OUTSIDE_SOURCE))?;
        let info = checked_info(header, image, limits)?;
        let previous_two = blank_row(info.dib_stride, image.offset, limits)?;
        let previous = blank_row(info.dib_stride, image.offset, limits)?;
        let current = blank_row(info.dib_stride, image.offset, limits)?;
        let coded = CodedSpan {
            offset: image.offset + DIB_BYTES,
            length: image.length - DIB_BYTES,
        };
        let arithmetic = ArithmeticDecoder::new(input, coded, table, contexts, limits)
            .map_err(|error| error.or_at(coded.offset, HNC8))?;
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
        })
    }

    pub fn progress(&self) -> Type0Progress {
        Type0Progress {
            info: self.info,
            rows_written: self.rows_written,
            output_bytes_written: self.output_bytes_written,
            arithmetic: self.arithmetic.snapshot(),
        }
    }

    /// Locate an error at the next coded byte unless it has its own offset.
    fn failed(&self, error: Error) -> Error {
        error.or_at(self.arithmetic.snapshot().input_offset, HNC8)
    }

    /// Decode and write exactly one display-order DIB-stride row.
    pub fn decode_next_row(&mut self) -> Result<bool> {
        if self.rows_written == self.info.height {
            return Ok(false);
        }
        if self.cancellation.is_cancelled() {
            return Err(cancelled(self.arithmetic.snapshot().input_offset));
        }
        self.current.fill(0);
        let copy_previous = self
            .arithmetic
            .decode_symbol(CONTROL_CONTEXT)
            .map_err(|error| self.failed(error))?;
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
                    .map_err(|error| self.failed(error))?;
                if bit {
                    let x = x as usize;
                    self.current[x / 8] |= 0x80 >> (x % 8);
                }
            }
        }
        write_counted(
            self.sink,
            &self.current,
            &mut self.output_bytes_written,
            self.limits,
            self.cancellation,
        )
        .map_err(|error| sink(error, self.arithmetic.snapshot().input_offset))?;
        mem::swap(&mut self.previous_two, &mut self.previous);
        mem::swap(&mut self.previous, &mut self.current);
        self.rows_written += 1;
        Ok(true)
    }

    /// Validate row count and flush only after all rows were written.
    pub fn finish(self) -> Result<Type0Report> {
        let snapshot = self.arithmetic.snapshot();
        let offset = snapshot.input_offset;
        if self.rows_written != self.info.height {
            return Err(malformed(offset, "not all type-0 rows were decoded"));
        }
        if self.cancellation.is_cancelled() {
            return Err(cancelled(offset));
        }
        let rows_written = self.rows_written;
        let output_bytes_written = self.output_bytes_written;
        // The expected count is the decoder's own, so this cannot fail.
        self.arithmetic
            .finish(snapshot.symbols_decoded)
            .map_err(|error| error.or_at(offset, HNC8))?;
        self.sink
            .flush()
            .map_err(|error| sink(Error::from(error), offset))?;
        // A flush can take long enough for the caller to give up.
        if self.cancellation.is_cancelled() {
            return Err(cancelled(offset));
        }
        Ok(Type0Report {
            image: self.image,
            coded: self.coded,
            progress: Type0Progress {
                info: self.info,
                rows_written,
                output_bytes_written,
                arithmetic: snapshot,
            },
        })
    }
}

#[cfg(test)]
mod tests;
