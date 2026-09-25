// SPDX-License-Identifier: MIT

//! Bounded, row-streamed arithmetic generic regions for the observed T.88
//! template-2 profile. The caller supplies the 47 MQ probability states.

use super::{
    SegmentHeader, SegmentSpan,
    mq::{
        MQ_STATE_COUNT, MqBudget, MqContext, MqContexts, MqDecoder, MqError, MqSnapshot, MqSpan,
        MqTable,
    },
};
use crate::{Cancellation, Error, Limits, RangedSource, SequentialSink, read_exact_at, write_all};
use std::{error, fmt, mem};

const HEADER_BYTES: u64 = 20;
const CONTEXT_COUNT: usize = 1024;
const MQ_BUFFER_BYTES: u64 = 256;

/// Bounds for one generic-region image model, in addition to `Limits` and `MqBudget`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenericBudget {
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels: u64,
    /// Maximum context-neighbor probes (ten per pixel).
    pub max_context_work: u64,
}

impl Default for GenericBudget {
    fn default() -> Self {
        Self {
            max_width: 32_768,
            max_height: 32_768,
            max_pixels: 12_000_000,
            max_context_work: 120_000_000,
        }
    }
}

/// Region geometry and external combination operator; no page composition occurs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenericRegionInfo {
    pub width: u32,
    pub height: u32,
    pub x: u32,
    pub y: u32,
    pub combination_operator: u8,
    pub row_stride: usize,
}

/// Progress includes the semantic MQ input byte and physical fetched bytes
/// separately. Prefetch and terminal lookahead can make these differ.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenericProgress {
    pub info: GenericRegionInfo,
    pub rows_written: u32,
    pub pixels_decoded: u64,
    pub output_bytes_written: u64,
    pub mq: MqSnapshot,
    /// Set before every awaited row operation. A dropped future leaves it set.
    pub poisoned: bool,
}

/// A successful, fully delimited single-region decode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenericReport {
    pub data: SegmentSpan,
    pub mq_span: MqSpan,
    pub progress: GenericProgress,
}

#[derive(Debug)]
pub struct GenericError {
    pub offset: u64,
    pub segment: u32,
    pub rows_written: u32,
    pub pixels_decoded: u64,
    pub output_bytes_written: u64,
    pub kind: GenericErrorKind,
}

#[derive(Debug)]
pub enum GenericErrorKind {
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
    Sink(Error),
    Mq(MqError),
    Incomplete,
    Poisoned,
}

pub type GenericResult<T> = Result<T, GenericError>;

impl fmt::Display for GenericError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "JBIG2 generic region segment {} at source byte {}: ",
            self.segment, self.offset
        )?;
        match &self.kind {
            GenericErrorKind::InvalidSpan(reason) => write!(f, "invalid span: {reason}"),
            GenericErrorKind::Truncated(field) => write!(f, "truncated {field}"),
            GenericErrorKind::Malformed(field) => write!(f, "malformed {field}"),
            GenericErrorKind::Unsupported { feature, value } => {
                write!(f, "unsupported {feature} ({value})")
            }
            GenericErrorKind::UnsupportedAt { x, y } => {
                write!(f, "unsupported adaptive pixel ({x}, {y})")
            }
            GenericErrorKind::LimitExceeded {
                resource,
                limit,
                attempted,
            } => write!(f, "{resource} limit {limit} exceeded by {attempted}"),
            GenericErrorKind::AllocationFailed => f.write_str("row allocation failed"),
            GenericErrorKind::Cancelled => f.write_str("cancelled"),
            GenericErrorKind::Source(source) => write!(f, "source: {source}"),
            GenericErrorKind::Sink(source) => write!(f, "sink: {source}"),
            GenericErrorKind::Mq(source) => write!(f, "MQ: {source}"),
            GenericErrorKind::Incomplete => f.write_str("not all rows were decoded"),
            GenericErrorKind::Poisoned => f.write_str("decoder state is poisoned"),
        }
    }
}

impl error::Error for GenericError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.kind {
            GenericErrorKind::Source(error) | GenericErrorKind::Sink(error) => Some(error),
            GenericErrorKind::Mq(error) => Some(error),
            _ => None,
        }
    }
}

fn malformed(segment: u32, offset: u64, reason: &'static str) -> GenericError {
    at(segment, offset, GenericErrorKind::Malformed(reason))
}

fn invalid_span(segment: u32, offset: u64, reason: &'static str) -> GenericError {
    at(segment, offset, GenericErrorKind::InvalidSpan(reason))
}

fn at(segment: u32, offset: u64, kind: GenericErrorKind) -> GenericError {
    GenericError {
        offset,
        segment,
        rows_written: 0,
        pixels_decoded: 0,
        output_bytes_written: 0,
        kind,
    }
}

fn limit(
    segment: u32,
    offset: u64,
    resource: &'static str,
    maximum: u64,
    attempted: u64,
) -> GenericError {
    at(
        segment,
        offset,
        GenericErrorKind::LimitExceeded {
            resource,
            limit: maximum,
            attempted,
        },
    )
}

fn checked_row(stride: usize, segment: u32, offset: u64) -> GenericResult<Vec<u8>> {
    // checked_layout already caps all three rows plus table, contexts, and
    // the MQ buffer before this first reserve.
    let mut row = Vec::new();
    row.try_reserve_exact(stride)
        .map_err(|_| at(segment, offset, GenericErrorKind::AllocationFailed))?;
    row.resize(stride, 0);
    Ok(row)
}

pub(super) fn template2_context(
    previous_two: &[u8],
    previous_one: &[u8],
    current: &[u8],
    width: u32,
    x: u32,
) -> usize {
    fn pixel(row: &[u8], x: i64, width: u32) -> usize {
        if x < 0 || x >= i64::from(width) {
            0
        } else {
            let x = x as usize;
            usize::from(row[x / 8] & (0x80 >> (x % 8)) != 0)
        }
    }
    let x = i64::from(x);
    let mut context = 0;
    for dx in [-1, 0, 1] {
        context = (context << 1) | pixel(previous_two, x + dx, width);
    }
    for dx in [-2, -1, 0, 1, 2] {
        context = (context << 1) | pixel(previous_one, x + dx, width);
    }
    for dx in [-2, -1] {
        context = (context << 1) | pixel(current, x + dx, width);
    }
    context
}

async fn read_field<S: RangedSource, C: Cancellation>(
    source: &mut S,
    offset: u64,
    field: &mut [u8],
    name: &'static str,
    segment: u32,
    limits: &Limits,
    cancellation: &C,
) -> GenericResult<()> {
    let mut done = 0;
    while done < field.len() {
        let count = (field.len() - done).min(limits.io_chunk_bytes);
        let current = offset + done as u64;
        read_exact_at(
            source,
            current,
            &mut field[done..done + count],
            limits,
            cancellation,
        )
        .await
        .map_err(|error| {
            let kind = match error {
                Error::Cancelled => GenericErrorKind::Cancelled,
                Error::TruncatedInput { .. } => GenericErrorKind::Truncated(name),
                Error::InvalidInput {
                    reason: "source reported more bytes than requested",
                } => GenericErrorKind::Malformed("source read length"),
                other => GenericErrorKind::Source(other),
            };
            at(segment, current, kind)
        })?;
        done += count;
    }
    Ok(())
}

fn checked_layout(
    header: &SegmentHeader,
    source_size: u64,
    limits: &Limits,
    mq_budget: &MqBudget,
    budget: GenericBudget,
    bytes: [u8; 20],
) -> GenericResult<(GenericRegionInfo, MqSpan, u64)> {
    let offset = header.data.offset;
    let segment = header.number;
    let width = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let height = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    let x = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    let y = u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
    let region_flags = bytes[16];
    let ax = bytes[18] as i8;
    let ay = bytes[19] as i8;
    if width == 0 || height == 0 {
        return Err(malformed(segment, offset, "zero region dimension"));
    }
    if width > budget.max_width {
        return Err(limit(
            segment,
            offset,
            "region width",
            u64::from(budget.max_width),
            u64::from(width),
        ));
    }
    if height > budget.max_height {
        return Err(limit(
            segment,
            offset,
            "region height",
            u64::from(budget.max_height),
            u64::from(height),
        ));
    }
    // The only caller read these header bytes at `offset` within the source,
    // so field offsets cannot overflow even when built before their checks.
    let failure = malformed(segment, offset + 8, "region x plus width overflows");
    x.checked_add(width).ok_or(failure)?;
    let failure = malformed(segment, offset + 12, "region y plus height overflows");
    y.checked_add(height).ok_or(failure)?;
    if region_flags & 0xf8 != 0 {
        return Err(malformed(segment, offset + 16, "region reserved flags"));
    }
    if region_flags & 0x07 > 4 {
        return Err(malformed(
            segment,
            offset + 16,
            "region combination operator",
        ));
    }
    if ay > 0 || (ay == 0 && ax >= 0) {
        return Err(malformed(
            segment,
            offset + 18,
            "adaptive pixel references undecoded pixel",
        ));
    }
    if (ax, ay) != (2, -1) {
        return Err(at(
            segment,
            offset + 18,
            GenericErrorKind::UnsupportedAt { x: ax, y: ay },
        ));
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(malformed(segment, offset, "pixel area overflows"))?;
    if pixels > budget.max_pixels {
        return Err(limit(
            segment,
            offset,
            "region pixels",
            budget.max_pixels,
            pixels,
        ));
    }
    if pixels > mq_budget.max_symbols {
        return Err(limit(
            segment,
            offset,
            "MQ symbols",
            mq_budget.max_symbols,
            pixels,
        ));
    }
    let context_work =
        pixels
            .checked_mul(10)
            .ok_or(malformed(segment, offset, "context work overflows"))?;
    if context_work > budget.max_context_work {
        return Err(limit(
            segment,
            offset,
            "context work",
            budget.max_context_work,
            context_work,
        ));
    }
    let stride_u64 = u64::from(width).div_ceil(8);
    let failure = malformed(segment, offset, "output size overflows");
    let output = stride_u64.checked_mul(u64::from(height)).ok_or(failure)?;
    if output > limits.max_output_bytes {
        return Err(limit(
            segment,
            offset,
            "output bytes",
            limits.max_output_bytes,
            output,
        ));
    }
    let stride = usize::try_from(stride_u64)
        .map_err(|_| malformed(segment, offset, "row stride exceeds address space"))?;
    let failure = malformed(segment, offset, "allocation calculation overflows");
    let rows_alloc = stride_u64
        .checked_mul(3)
        .and_then(|v| {
            v.checked_add(
                (CONTEXT_COUNT * mem::size_of::<MqContext>()
                    + MQ_STATE_COUNT * mem::size_of::<super::mq::MqState>()) as u64,
            )
        })
        .and_then(|v| v.checked_add(MQ_BUFFER_BYTES))
        .ok_or(failure)?;
    if rows_alloc > limits.max_allocation_bytes {
        return Err(limit(
            segment,
            offset,
            "region working allocation bytes",
            limits.max_allocation_bytes,
            rows_alloc,
        ));
    }
    if mq_budget.max_contexts < CONTEXT_COUNT {
        return Err(limit(
            segment,
            offset,
            "MQ contexts",
            mq_budget.max_contexts as u64,
            CONTEXT_COUNT as u64,
        ));
    }
    let failure = invalid_span(segment, offset, "MQ start overflows");
    let payload_offset = offset.checked_add(HEADER_BYTES).ok_or(failure)?;
    let mq_span = MqSpan {
        offset: payload_offset,
        length: header.data.length - HEADER_BYTES,
    };
    if mq_span.length < 2 {
        return Err(at(
            segment,
            payload_offset,
            GenericErrorKind::Truncated("MQ terminal pair"),
        ));
    }
    if mq_span.length > mq_budget.max_span_bytes {
        return Err(limit(
            segment,
            payload_offset,
            "MQ span bytes",
            mq_budget.max_span_bytes,
            mq_span.length,
        ));
    }
    if payload_offset.checked_add(mq_span.length)
        != header.data.offset.checked_add(header.data.length)
        || payload_offset + mq_span.length > source_size
    {
        return Err(invalid_span(segment, offset, "MQ span outside source"));
    }
    Ok((
        GenericRegionInfo {
            width,
            height,
            x,
            y,
            combination_operator: region_flags & 7,
            row_stride: stride,
        },
        mq_span,
        pixels,
    ))
}

/// Stateful row decoder. A failed or dropped row future poisons this object;
/// partial sink output must be discarded by the caller.
pub struct GenericRegionDecoder<'a, S: RangedSource, W: SequentialSink, C: Cancellation> {
    mq: MqDecoder<'a, S, C>,
    sink: &'a mut W,
    limits: &'a Limits,
    cancellation: &'a C,
    segment: u32,
    data: SegmentSpan,
    mq_span: MqSpan,
    info: GenericRegionInfo,
    pixels: u64,
    previous_two: Vec<u8>,
    previous_one: Vec<u8>,
    current: Vec<u8>,
    rows_written: u32,
    output_bytes_written: u64,
    poisoned: bool,
}

impl<'a, S: RangedSource, W: SequentialSink, C: Cancellation> GenericRegionDecoder<'a, S, W, C> {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        source: &'a mut S,
        header: &SegmentHeader,
        table: &'a MqTable,
        contexts: &'a mut MqContexts,
        sink: &'a mut W,
        limits: &'a Limits,
        cancellation: &'a C,
        mq_budget: MqBudget,
        budget: GenericBudget,
    ) -> GenericResult<Self> {
        let segment = header.number;
        let offset = header.data.offset;
        limits
            .validate()
            .map_err(|e| at(segment, offset, GenericErrorKind::Source(e)))?;
        if cancellation.is_cancelled() {
            return Err(at(segment, offset, GenericErrorKind::Cancelled));
        }
        if header.segment_type != 38 {
            return Err(at(
                segment,
                offset,
                GenericErrorKind::Unsupported {
                    feature: "segment type",
                    value: u64::from(header.segment_type),
                },
            ));
        }
        let failure = invalid_span(segment, offset, "segment end overflows");
        let end = offset.checked_add(header.data.length).ok_or(failure)?;
        if end > source.size() {
            return Err(invalid_span(segment, offset, "segment data outside source"));
        }
        if header.data.length > limits.max_input_bytes {
            return Err(limit(
                segment,
                offset,
                "input bytes",
                limits.max_input_bytes,
                header.data.length,
            ));
        }
        if header.data.length < 18 {
            return Err(at(
                segment,
                end,
                GenericErrorKind::Truncated("generic flags"),
            ));
        }
        let mut bytes = [0; 20];
        read_field(
            source,
            offset,
            &mut bytes[..18],
            "region and generic flags",
            segment,
            limits,
            cancellation,
        )
        .await?;
        let flags = bytes[17];
        if flags & 0xf0 != 0 {
            return Err(malformed(segment, offset + 17, "generic reserved flags"));
        }
        if flags & 1 != 0 {
            return Err(at(
                segment,
                offset + 17,
                GenericErrorKind::Unsupported {
                    feature: "MMR generic coding",
                    value: 1,
                },
            ));
        }
        if (flags >> 1) & 3 != 2 {
            return Err(at(
                segment,
                offset + 17,
                GenericErrorKind::Unsupported {
                    feature: "generic template",
                    value: u64::from((flags >> 1) & 3),
                },
            ));
        }
        if flags & 8 != 0 {
            return Err(at(
                segment,
                offset + 17,
                GenericErrorKind::Unsupported {
                    feature: "typical prediction",
                    value: 1,
                },
            ));
        }
        if header.data.length < HEADER_BYTES {
            return Err(at(
                segment,
                end,
                GenericErrorKind::Truncated("template-2 adaptive pixel"),
            ));
        }
        read_field(
            source,
            offset + 18,
            &mut bytes[18..],
            "template-2 adaptive pixel",
            segment,
            limits,
            cancellation,
        )
        .await?;
        let (info, mq_span, pixels) =
            checked_layout(header, source.size(), limits, &mq_budget, budget, bytes)?;
        if contexts.count() != CONTEXT_COUNT {
            return Err(malformed(
                segment,
                offset,
                "expected 1024 generic MQ contexts",
            ));
        }
        let previous_two = checked_row(info.row_stride, segment, offset)?;
        let previous_one = checked_row(info.row_stride, segment, offset)?;
        let current = checked_row(info.row_stride, segment, offset)?;
        contexts.reset();
        let mq = MqDecoder::new(
            source,
            mq_span,
            table,
            contexts,
            limits,
            cancellation,
            mq_budget,
        )
        .await
        .map_err(|e| {
            at(
                segment,
                e.offset.unwrap_or(mq_span.offset),
                GenericErrorKind::Mq(e),
            )
        })?;
        Ok(Self {
            mq,
            sink,
            limits,
            cancellation,
            segment,
            data: header.data,
            mq_span,
            info,
            pixels,
            previous_two,
            previous_one,
            current,
            rows_written: 0,
            output_bytes_written: 0,
            poisoned: false,
        })
    }

    pub fn progress(&self) -> GenericProgress {
        let mq = self.mq.snapshot();
        GenericProgress {
            info: self.info,
            rows_written: self.rows_written,
            pixels_decoded: mq.symbols_decoded,
            output_bytes_written: self.output_bytes_written,
            mq,
            poisoned: self.poisoned || mq.poisoned,
        }
    }

    fn error(&self, kind: GenericErrorKind) -> GenericError {
        let mq = self.mq.snapshot();
        GenericError {
            offset: mq.current_input_offset,
            segment: self.segment,
            rows_written: self.rows_written,
            pixels_decoded: mq.symbols_decoded,
            output_bytes_written: self.output_bytes_written,
            kind,
        }
    }

    /// Emits one packed row. Returns `false` after the final row. Any error or
    /// dropped pending future leaves this decoder poisoned and the sink partial.
    pub async fn decode_next_row(&mut self) -> GenericResult<bool> {
        if self.poisoned {
            return Err(self.error(GenericErrorKind::Poisoned));
        }
        if self.rows_written == self.info.height {
            return Ok(false);
        }
        if self.cancellation.is_cancelled() {
            return Err(self.error(GenericErrorKind::Cancelled));
        }
        self.poisoned = true;
        for x in 0..self.info.width {
            if self.cancellation.is_cancelled() {
                return Err(self.error(GenericErrorKind::Cancelled));
            }
            let context = template2_context(
                &self.previous_two,
                &self.previous_one,
                &self.current,
                self.info.width,
                x,
            );
            let bit = self.mq.decode_bit(context).await.map_err(|e| {
                let offset = e.offset;
                let mut error = self.error(GenericErrorKind::Mq(e));
                if let Some(offset) = offset {
                    error.offset = offset;
                }
                error
            })?;
            if bit {
                self.current[x as usize / 8] |= 0x80 >> (x % 8);
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
        .map_err(|e| {
            self.error(match e {
                Error::Cancelled => GenericErrorKind::Cancelled,
                other => GenericErrorKind::Sink(other),
            })
        })?;
        mem::swap(&mut self.previous_two, &mut self.previous_one);
        mem::swap(&mut self.previous_one, &mut self.current);
        self.current.fill(0);
        self.rows_written += 1;
        self.poisoned = false;
        Ok(true)
    }

    /// Verify exactly width × height MQ decisions and the delimited FF AC tail,
    /// then flush. The semantic MQ byte need not be the terminal byte.
    pub async fn finish(mut self) -> GenericResult<GenericReport> {
        if self.poisoned {
            return Err(self.error(GenericErrorKind::Poisoned));
        }
        if self.rows_written != self.info.height {
            return Err(self.error(GenericErrorKind::Incomplete));
        }
        self.poisoned = true;
        let progress = self.progress();
        let segment = self.segment;
        let data = self.data;
        let mq_span = self.mq_span;
        let mut report = GenericReport {
            data,
            mq_span,
            progress,
        };
        report.progress.mq = self
            .mq
            .finish_with_snapshot(self.pixels)
            .await
            .map_err(|e| GenericError {
                offset: e.offset.unwrap_or(mq_span.offset),
                segment,
                rows_written: progress.rows_written,
                pixels_decoded: progress.pixels_decoded,
                output_bytes_written: progress.output_bytes_written,
                kind: GenericErrorKind::Mq(e),
            })?;
        if self.cancellation.is_cancelled() {
            return Err(GenericError {
                offset: report.progress.mq.current_input_offset,
                segment,
                rows_written: progress.rows_written,
                pixels_decoded: progress.pixels_decoded,
                output_bytes_written: progress.output_bytes_written,
                kind: GenericErrorKind::Cancelled,
            });
        }
        self.sink.flush().await.map_err(|e| GenericError {
            offset: report.progress.mq.current_input_offset,
            segment,
            rows_written: progress.rows_written,
            pixels_decoded: progress.pixels_decoded,
            output_bytes_written: progress.output_bytes_written,
            kind: GenericErrorKind::Sink(e),
        })?;
        if self.cancellation.is_cancelled() {
            return Err(GenericError {
                offset: report.progress.mq.current_input_offset,
                segment,
                rows_written: progress.rows_written,
                pixels_decoded: progress.pixels_decoded,
                output_bytes_written: progress.output_bytes_written,
                kind: GenericErrorKind::Cancelled,
            });
        }
        report.progress.poisoned = false;
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::template2_context;

    #[test]
    fn template2_bits_include_adaptive_neighbor_and_zero_fill_edges() {
        // Width five, x=2: y-2 is 010, y-1 is 01111 (AT is x+2),
        // current is 11. This is 0b010_01111_11 = 319.
        let two = [0b1010_0000];
        let one = [0b0111_1000];
        let current = [0b1100_0000];
        assert_eq!(template2_context(&two, &one, &current, 5, 2), 319);
        // At x=4, x+1 and the AT x+2 lie beyond the row and contribute zero.
        assert_eq!(template2_context(&two, &one, &current, 5, 4), 112);
        // Left edge: all negative columns are zero, including current-row bits.
        assert_eq!(template2_context(&two, &one, &current, 5, 0), 268);
        assert_eq!(template2_context(&[0], &[0], &[0], 1, 0), 0);

        // Every neighbor is distinguishable: a single set pixel must map to
        // its independently assigned bit, including the adaptive x+2 pixel.
        let positions = [
            (0, 2, 1u16 << 9),
            (0, 3, 1 << 8),
            (0, 4, 1 << 7),
            (1, 1, 1 << 6),
            (1, 2, 1 << 5),
            (1, 3, 1 << 4),
            (1, 4, 1 << 3),
            (1, 5, 1 << 2),
            (2, 1, 1 << 1),
            (2, 2, 1),
        ];
        for (row, column, expected) in positions {
            let mut rows = [[0u8; 1]; 3];
            rows[row][0] = 0x80 >> column;
            assert_eq!(
                template2_context(&rows[0], &rows[1], &rows[2], 7, 3),
                usize::from(expected)
            );
        }
    }
}
