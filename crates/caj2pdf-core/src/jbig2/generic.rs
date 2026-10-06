// SPDX-License-Identifier: MIT

//! Bounded, row-streamed arithmetic generic regions for the observed T.88
//! template-2 profile, decoded with the standard MQ probability states.

use super::{
    SegmentHeader, SegmentSpan,
    mq::{
        ArithmeticSnapshot, CodedSpan, ContextBank, ContextState, MQ_STATE_COUNT, MqDecoder,
        MqTable,
    },
    page_compose::PageOrSink,
};
use crate::fallible::reserve_exact;
use crate::{
    Cancellation, Context, Error, ErrorKind, Limits, Payload, RangedSource, Result, read_exact_at,
    write_counted,
};
use std::io::Write;
use std::mem;

const HEADER_BYTES: u64 = 20;
const CONTEXT_COUNT: usize = 1024;

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

/// Checked metadata that can be inspected before attaching the final page
/// sink. The decoder reparses the same header, and `arm_page_output` compares
/// it with this preflight value before emitting any row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenericRegionHeader {
    pub segment: u32,
    pub page_association: u32,
    pub reference_count: usize,
    pub data: SegmentSpan,
    pub info: GenericRegionInfo,
    pub mq_span: CodedSpan,
    pub pixels: u64,
}

/// Rows, decisions and output so far, with the MQ register snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenericProgress {
    pub info: GenericRegionInfo,
    pub rows_written: u32,
    pub pixels_decoded: u64,
    pub output_bytes_written: u64,
    pub mq: ArithmeticSnapshot,
}

/// A successful, fully delimited single-region decode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenericReport {
    pub data: SegmentSpan,
    pub mq_span: CodedSpan,
    pub progress: GenericProgress,
}

/// Locate an unlocated error at `offset` in `segment`.
fn at(segment: u32, offset: u64, error: Error) -> Error {
    error.or_at(
        offset,
        Context::Jbig2 {
            segment: Some(segment),
        },
    )
}

fn malformed(segment: u32, offset: u64, reason: &'static str) -> Error {
    at(segment, offset, Error::invalid(reason))
}

fn unsupported(segment: u32, offset: u64, reason: &'static str) -> Error {
    at(segment, offset, Error::unsupported(offset, reason))
}

fn limit(segment: u32, offset: u64, resource: &'static str, maximum: u64, attempted: u64) -> Error {
    at(segment, offset, Error::limit(resource, maximum, attempted))
}

fn checked_row(stride: usize, segment: u32, offset: u64, limits: &Limits) -> Result<Vec<u8>> {
    // checked_layout already caps all three rows plus table and contexts
    // before this first reserve.
    let mut row = Vec::new();
    let failed = at(
        segment,
        offset,
        limits.allocation_refused("generic row bytes", stride as u64),
    );
    reserve_exact(&mut row, stride, failed)?;
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

fn read_field<S: RangedSource, C: Cancellation>(
    source: &mut S,
    offset: u64,
    field: &mut [u8],
    name: &'static str,
    segment: u32,
    limits: &Limits,
    cancellation: &C,
) -> Result<()> {
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
        .map_err(|error| match error.kind {
            ErrorKind::Truncated { .. } => at(segment, current, error.because(name)),
            _ => at(segment, current, error),
        })?;
        done += count;
    }
    Ok(())
}

fn checked_layout(
    header: &SegmentHeader,
    source_size: u64,
    limits: &Limits,
    bytes: [u8; 20],
) -> Result<(GenericRegionInfo, CodedSpan, u64)> {
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
    // Both callers read these header bytes at `offset` within the source, so
    // field offsets cannot overflow even when built before their checks.
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
        return Err(unsupported(segment, offset + 18, "adaptive pixel"));
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(malformed(segment, offset, "pixel area overflows"))?;
    if pixels > limits.max_image_pixels {
        return Err(limit(
            segment,
            offset,
            "region pixels",
            limits.max_image_pixels,
            pixels,
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
    // A u32 width has a stride of at most 2^29 bytes, which every supported
    // (at least 32-bit) `usize` represents.
    let stride = stride_u64 as usize;
    let failure = malformed(segment, offset, "allocation calculation overflows");
    let rows_alloc = stride_u64
        .checked_mul(3)
        .and_then(|v| {
            v.checked_add(
                (CONTEXT_COUNT * mem::size_of::<ContextState>()
                    + MQ_STATE_COUNT * mem::size_of::<super::mq::MqState>()) as u64,
            )
        })
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
    let failure = malformed(segment, offset, "MQ start overflows");
    let payload_offset = offset.checked_add(HEADER_BYTES).ok_or(failure)?;
    let mq_span = CodedSpan {
        offset: payload_offset,
        length: header.data.length - HEADER_BYTES,
    };
    if mq_span.length < 2 {
        return Err(at(
            segment,
            payload_offset,
            Error::truncated(payload_offset, 2, mq_span.length).because("MQ terminal pair"),
        ));
    }
    if payload_offset.checked_add(mq_span.length)
        != header.data.offset.checked_add(header.data.length)
        || payload_offset + mq_span.length > source_size
    {
        return Err(malformed(segment, offset, "MQ span outside source"));
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

/// Read and validate only the observed template-2 generic-region header.
/// This is a bounded preflight for page composition; it does not initialize
/// MQ contexts, read compressed decisions, or emit a pixel. The row decoder
/// uses this same parser, so supported flags and limits cannot drift.
pub fn read_generic_region_header<S: RangedSource, C: Cancellation>(
    source: &mut S,
    header: &SegmentHeader,
    limits: &Limits,
    cancellation: &C,
) -> Result<GenericRegionHeader> {
    let segment = header.number;
    let offset = header.data.offset;
    if cancellation.is_cancelled() {
        return Err(at(segment, offset, Error::cancelled()));
    }
    if header.segment_type != 38 {
        return Err(unsupported(segment, offset, "segment type"));
    }
    let failure = malformed(segment, offset, "segment end overflows");
    let end = offset.checked_add(header.data.length).ok_or(failure)?;
    if end > source.size() {
        return Err(malformed(segment, offset, "segment data outside source"));
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
            Error::truncated(end, 18, header.data.length).because("generic flags"),
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
    )?;
    let flags = bytes[17];
    if flags & 0xf0 != 0 {
        return Err(malformed(segment, offset + 17, "generic reserved flags"));
    }
    if flags & 1 != 0 {
        return Err(unsupported(segment, offset + 17, "MMR generic coding"));
    }
    if (flags >> 1) & 3 != 2 {
        return Err(unsupported(segment, offset + 17, "generic template"));
    }
    if flags & 8 != 0 {
        return Err(unsupported(segment, offset + 17, "typical prediction"));
    }
    if header.data.length < HEADER_BYTES {
        return Err(at(
            segment,
            end,
            Error::truncated(end, HEADER_BYTES, header.data.length)
                .because("template-2 adaptive pixel"),
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
    )?;
    let (info, mq_span, pixels) = checked_layout(header, source.size(), limits, bytes)?;
    Ok(GenericRegionHeader {
        segment: header.number,
        page_association: header.page_association,
        reference_count: header.referred_to.len(),
        data: header.data,
        info,
        mq_span,
        pixels,
    })
}

/// Stateful row decoder over a segment read into memory. After an error the
/// caller must discard the decoder and any partial sink output.
pub struct GenericRegionDecoder<'a, W: Write, C: Cancellation> {
    mq: MqDecoder<'a>,
    sink: &'a mut W,
    limits: &'a Limits,
    cancellation: &'a C,
    segment: u32,
    page_association: u32,
    reference_count: usize,
    data: SegmentSpan,
    mq_span: CodedSpan,
    info: GenericRegionInfo,
    pixels: u64,
    previous_two: Vec<u8>,
    previous_one: Vec<u8>,
    current: Vec<u8>,
    rows_written: u32,
    output_bytes_written: u64,
}

impl<'a, W: Write, C: Cancellation> GenericRegionDecoder<'a, W, C> {
    /// Parse the segment's header from `input`, which must hold the whole
    /// segment data, and start its MQ coding unit.
    pub fn new(
        input: Payload<'a>,
        header: &SegmentHeader,
        table: &'a MqTable,
        contexts: &'a mut ContextBank,
        sink: &'a mut W,
        limits: &'a Limits,
        cancellation: &'a C,
    ) -> Result<Self> {
        let segment = header.number;
        let offset = header.data.offset;
        let checked = read_generic_region_header(&mut { input }, header, limits, cancellation)?;
        let info = checked.info;
        let mq_span = checked.mq_span;
        let pixels = checked.pixels;
        if contexts.len() != CONTEXT_COUNT {
            return Err(malformed(
                segment,
                offset,
                "expected 1024 generic MQ contexts",
            ));
        }
        let previous_two = checked_row(info.row_stride, segment, offset, limits)?;
        let previous_one = checked_row(info.row_stride, segment, offset, limits)?;
        let current = checked_row(info.row_stride, segment, offset, limits)?;
        contexts.reset();
        let mq = MqDecoder::new(input, mq_span, table, contexts, limits)
            .map_err(|error| at(segment, mq_span.offset, error))?;
        Ok(Self {
            mq,
            sink,
            limits,
            cancellation,
            segment,
            page_association: checked.page_association,
            reference_count: checked.reference_count,
            data: header.data,
            mq_span,
            info,
            pixels,
            previous_two,
            previous_one,
            current,
            rows_written: 0,
            output_bytes_written: 0,
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
        }
    }

    /// Metadata rechecked by this decoder before any row was emitted.
    pub fn checked_header(&self) -> GenericRegionHeader {
        GenericRegionHeader {
            segment: self.segment,
            page_association: self.page_association,
            reference_count: self.reference_count,
            data: self.data,
            info: self.info,
            mq_span: self.mq_span,
            pixels: self.pixels,
        }
    }

    /// Locate an unlocated error at the next MQ byte.
    fn error(&self, error: Error) -> Error {
        at(self.segment, self.mq.snapshot().input_offset, error)
    }

    /// Emits one packed row. Returns `false` after the final row.
    pub fn decode_next_row(&mut self) -> Result<bool> {
        if self.rows_written == self.info.height {
            return Ok(false);
        }
        if self.cancellation.is_cancelled() {
            return Err(self.error(Error::cancelled()));
        }
        for x in 0..self.info.width {
            let context = template2_context(
                &self.previous_two,
                &self.previous_one,
                &self.current,
                self.info.width,
                x,
            );
            let bit = self
                .mq
                .decode_bit(context)
                .map_err(|error| self.error(error))?;
            if bit {
                self.current[x as usize / 8] |= 0x80 >> (x % 8);
            }
        }
        write_counted(
            self.sink,
            &self.current,
            &mut self.output_bytes_written,
            self.limits,
            self.cancellation,
        )
        .map_err(|error| self.error(error))?;
        mem::swap(&mut self.previous_two, &mut self.previous_one);
        mem::swap(&mut self.previous_one, &mut self.current);
        self.current.fill(0);
        self.rows_written += 1;
        Ok(true)
    }

    /// Verify exactly width × height MQ decisions and the delimited FF AC tail,
    /// then flush. The semantic MQ byte need not be the terminal byte.
    pub fn finish(mut self) -> Result<GenericReport> {
        if self.rows_written != self.info.height {
            return Err(self.error(Error::invalid("not all generic rows were decoded")));
        }
        let progress = self.progress();
        let segment = self.segment;
        let mq_span = self.mq_span;
        let mq = self
            .mq
            .finish(self.pixels)
            .map_err(|error| at(segment, mq_span.offset, error))?;
        if self.cancellation.is_cancelled() {
            return Err(at(segment, mq.input_offset, Error::cancelled()));
        }
        self.sink
            .flush()
            .map_err(|error| at(segment, mq.input_offset, Error::from(error)))?;
        // A flush can take long enough for the caller to give up.
        if self.cancellation.is_cancelled() {
            return Err(at(segment, mq.input_offset, Error::cancelled()));
        }
        Ok(GenericReport {
            data: self.data,
            mq_span,
            progress: GenericProgress { mq, ..progress },
        })
    }
}

impl<'p, P, PC, C> GenericRegionDecoder<'_, PageOrSink<'p, P, PC>, C>
where
    P: Write,
    PC: Cancellation,
    C: Cancellation,
{
    /// Bind a pre-inspected header to the page sink, which rejects all
    /// writes until armed with the header this decoder parsed itself.
    pub fn arm_page_output(&mut self, expected: GenericRegionHeader) -> Result<()> {
        if self.rows_written != 0 {
            return Err(self.error(Error::invalid("page output armed after the first row")));
        }
        let checked = self.checked_header();
        if checked != expected {
            return Err(self.error(Error::invalid("generic header differs from page preflight")));
        }
        self.sink
            .arm_checked_header(checked)
            .map_err(|error| self.error(error))
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
