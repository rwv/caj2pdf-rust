// SPDX-License-Identifier: MIT

//! Streaming metadata reader for the measured HN-A/C8 page-text profile.
//! Opaque text is discarded. Coordinate words have no assigned signedness
//! or physical units here, and this reader does not enable composition.

use super::{ErrorKind, Header, Location, PageRecord, Result, Span, Variant};
use crate::fallible::{len_u64, reserve_exact, usize_from_u32};
use crate::{Cancellation, Error, Limits, RangedSource, read_exact_at};
use flate2::{Decompress, FlushDecompress, Status};
use sha2::{Digest, Sha256};

const HEADER_BYTES: usize = 24;
const CHUNK_BYTES: usize = 64 * 1024;
const FIXED_WORKING_BYTES: u64 = 4096;
const C8_PREFIX: [u8; 32] = [
    0x07, 0x8c, 0x83, 0x0d, 0x36, 0x70, 0x8d, 0xb4, 0x1f, 0x5b, 0xdf, 0x37, 0x79, 0xe4, 0x22, 0x61,
    0x68, 0x69, 0x09, 0x2e, 0x87, 0x9c, 0x34, 0x75, 0x2d, 0xdc, 0x91, 0x96, 0x8d, 0xd3, 0x66, 0x67,
];
const HN_A_PREFIX: [u8; 32] = [
    0x1f, 0xc8, 0xa3, 0xa8, 0x38, 0xc4, 0xf3, 0xa7, 0xee, 0xaf, 0x72, 0x52, 0xcc, 0xdf, 0xe8, 0x45,
    0x5f, 0x21, 0x84, 0x71, 0x3c, 0xe9, 0xdc, 0x38, 0x3f, 0xa9, 0x1b, 0x96, 0x93, 0x4f, 0xbb, 0x12,
];

/// Conservative working reservation for the locked flate2/miniz_oxide
/// backend, including its 32 KiB dictionary and Huffman state. Reaudit this
/// reservation when changing the backend or lock. Compiler call stacks and
/// allocator overhead are not a process-memory guarantee.
/// The backend allocates its fixed state infallibly: this budget accounts
/// for it, but cannot make a process-wide allocator failure recoverable.
pub const TEXT_DECODER_RESERVATION_BYTES: u64 = 128 * 1024;

/// Separate limits for compressed text, expanded text, retained coordinates
/// and accounted working storage. Zero record/image ceilings are allowed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextBudget {
    pub max_span_bytes: u64,
    pub max_decoded_bytes: u64,
    pub max_records: u32,
    pub max_images: u32,
    /// Owned buffer capacities plus the decoder reservation and 4 KiB of
    /// fixed parser/hash scratch. This is not total process residency.
    pub max_working_bytes: u64,
}

impl Default for TextBudget {
    fn default() -> Self {
        Self {
            max_span_bytes: 1024 * 1024,
            max_decoded_bytes: 1024 * 1024,
            max_records: 65_536,
            max_images: 8192,
            max_working_bytes: 1024 * 1024,
        }
    }
}

/// The two little-endian words at +0/+2 of one 28-byte tail record.
/// These are raw bits; an unsigned Rust representation does not establish
/// the source format's signedness, units or accepted coordinate range.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RawTextCoordinate {
    pub x: u16,
    pub y: u16,
}

/// Validated framing and bounded image-order metadata, with no opaque text.
/// Coordinates become available only after the complete frame and checksum
/// have succeeded. There is no temporary spool or disk use in this reader.
#[derive(Debug, Eq, PartialEq)]
pub struct TextCoordinates {
    pub text: Span,
    pub zlib_frame: Span,
    pub decoded_length: u32,
    pub record_count: u32,
    pub coordinates: Vec<RawTextCoordinate>,
    /// SHA-256 of `zlib_frame` alone, excluding the 24-byte text header.
    pub encoded_sha256: [u8; 32],
    /// SHA-256 of all expanded bytes, including discarded opaque fields.
    pub decoded_sha256: [u8; 32],
    pub max_source_request_bytes: usize,
    pub max_decoder_output_chunk_bytes: usize,
    /// Capacities of the compressed/expanded scratch and coordinate Vecs.
    pub owned_buffer_bytes: u64,
    /// Owned buffers, fixed scratch and the conservative decoder reservation.
    pub working_memory_bytes: u64,
}

/// Validate one declared page's text using the observed variant fingerprint.
///
/// The caller supplies metadata from the same stable [`RangedSource`],
/// normally [`super::Hnc8Reader`]. Public header/page values are rechecked
/// for consistent counts, row locations, nonoverlapping spans and source
/// bounds; this does not reread the container index to authenticate them.
/// HN-B is unsupported. Reads and decoder output are at most 64 KiB and
/// respect `Limits::io_chunk_bytes`; only four bytes per image are retained.
/// Source changes during the operation violate the stable-source contract.
/// A decoded-marker error is located at the source frame start: a compressed
/// byte offset cannot identify the corresponding expanded record byte.
pub async fn read_text_coordinates<S: RangedSource, C: Cancellation>(
    source: &mut S,
    header: Header,
    page: PageRecord,
    limits: &Limits,
    cancellation: &C,
    budget: TextBudget,
) -> Result<TextCoordinates> {
    let prefix = match header.variant {
        Variant::C8 => C8_PREFIX,
        Variant::HnA => HN_A_PREFIX,
        Variant::HnB => {
            return Err(location(header, page).error(ErrorKind::Unsupported {
                field: "text framing variant",
                value: 2,
            }));
        }
    };
    read_with_prefix(source, header, page, limits, cancellation, budget, prefix).await
}

fn location(header: Header, page: PageRecord) -> Location {
    Location {
        variant: Some(header.variant),
        offset: page.text.offset,
        page: Some(page.page_number),
        image: None,
    }
}

fn checked_end(span: Span, size: u64, loc: Location, field: &'static str) -> Result<u64> {
    let end = span
        .checked_end()
        .ok_or_else(|| loc.at(span.offset).malformed(field, "end overflows u64"))?;
    if end > size {
        return Err(loc.at(span.offset).error(ErrorKind::Truncated {
            field,
            expected: span.length,
            available: size.saturating_sub(span.offset),
        }));
    }
    Ok(end)
}

fn validate_metadata(
    header: Header,
    page: PageRecord,
    size: u64,
    limits: &Limits,
    budget: TextBudget,
    loc: Location,
) -> Result<()> {
    limits.validate().map_err(|source| {
        loc.error(ErrorKind::Source {
            field: "limits",
            source,
        })
    })?;
    if size > limits.max_input_bytes {
        return Err(loc.limit("source bytes", limits.max_input_bytes, size));
    }
    if header.page_count == 0 || header.page_count > i32::MAX as u32 {
        return Err(loc.malformed("page count", "outside positive signed 32-bit range"));
    }
    if header.page_count > limits.max_pages {
        return Err(loc.limit(
            "pages",
            u64::from(limits.max_pages),
            u64::from(header.page_count),
        ));
    }
    if page.page_number == 0 || page.page_number > header.page_count {
        return Err(loc.malformed("page number", "outside declared page index"));
    }
    if header.page_index.length != u64::from(header.page_count) * super::PAGE_ROW_BYTES {
        return Err(loc.malformed("page index", "length differs from declared row count"));
    }
    let index_end = checked_end(header.page_index, size, loc, "page index")?;
    let index_start_valid = match header.variant {
        Variant::C8 => header.page_index.offset == 0x50,
        Variant::HnA => {
            header.page_index.offset >= 0x15c
                && (header.page_index.offset - 0x15c) % super::OUTLINE_RECORD_BYTES == 0
                && (header.page_index.offset - 0x15c) / super::OUTLINE_RECORD_BYTES
                    <= i32::MAX as u64
        }
        Variant::HnB => false,
    };
    if !index_start_valid {
        return Err(loc.malformed("page index", "start differs from observed variant layout"));
    }
    // The index end was checked and page_number is within page_count.
    let row = header.page_index.offset + u64::from(page.page_number - 1) * super::PAGE_ROW_BYTES;
    if page.row_offset != row {
        return Err(loc
            .at(page.row_offset)
            .malformed("page row", "offset differs from declared index"));
    }
    checked_end(page.text, size, loc, "page text span")?;
    if page.text.offset < index_end {
        return Err(loc.malformed("page text span", "overlaps protected container index"));
    }
    if page.text.offset > i32::MAX as u64 || page.text.length > i32::MAX as u64 {
        return Err(loc.malformed("page text span", "outside nonnegative signed 32-bit range"));
    }
    if page.text.length < HEADER_BYTES as u64 {
        return Err(loc.error(ErrorKind::Truncated {
            field: "page text header",
            expected: HEADER_BYTES as u64,
            available: page.text.length,
        }));
    }
    if page.text.length > budget.max_span_bytes {
        return Err(loc.limit("page text bytes", budget.max_span_bytes, page.text.length));
    }
    let images = budget.max_images.min(i16::MAX as u32);
    if page.image_count > images {
        return Err(loc.limit(
            "text images",
            u64::from(images),
            u64::from(page.image_count),
        ));
    }
    Ok(())
}

async fn read_chunks<S: RangedSource, C: Cancellation>(
    source: &mut S,
    offset: u64,
    bytes: &mut [u8],
    limits: &Limits,
    cancellation: &C,
    loc: Location,
    max_request: &mut usize,
) -> Result<()> {
    let mut current = offset;
    for chunk in bytes.chunks_mut(limits.io_chunk_bytes.min(CHUNK_BYTES)) {
        *max_request = (*max_request).max(chunk.len());
        read_exact_at(source, current, chunk, limits, cancellation)
            .await
            .map_err(|error| match error {
                Error::Cancelled => loc.at(current).error(ErrorKind::Cancelled),
                Error::TruncatedInput { available, .. } => loc
                    .at(current.saturating_add(available))
                    .error(ErrorKind::Truncated {
                        field: "page text read",
                        expected: len_u64(chunk.len()),
                        available,
                    }),
                source => loc.at(current).error(ErrorKind::Source {
                    field: "page text read",
                    source,
                }),
            })?;
        // All callers validated the containing span, so this addition fits.
        current += len_u64(chunk.len());
    }
    Ok(())
}

fn allocate<T: Clone>(count: usize, value: T, limits: &Limits, loc: Location) -> Result<Vec<T>> {
    let bytes = len_u64(count).saturating_mul(len_u64(size_of::<T>()));
    if bytes > limits.max_allocation_bytes {
        return Err(loc.limit("text allocation bytes", limits.max_allocation_bytes, bytes));
    }
    let mut result = Vec::new();
    let failure = loc.limit("text allocation bytes", limits.max_allocation_bytes, bytes);
    reserve_exact(&mut result, count, failure)?;
    result.resize(count, value);
    Ok(result)
}

fn check_working(owned: u64, budget: TextBudget, loc: Location) -> Result<u64> {
    let working = owned.saturating_add(TEXT_DECODER_RESERVATION_BYTES + FIXED_WORKING_BYTES);
    if working > budget.max_working_bytes {
        return Err(loc.limit("text working bytes", budget.max_working_bytes, working));
    }
    Ok(working)
}

struct Accumulator {
    glyph_end: u32,
    tail_start: u32,
    coordinates: Vec<RawTextCoordinate>,
    loc: Location,
}

impl Accumulator {
    fn consume(&mut self, offset: u64, bytes: &[u8]) -> Result<()> {
        for (index, byte) in bytes.iter().copied().enumerate() {
            let at = offset + len_u64(index);
            if at >= 8 && at < u64::from(self.glyph_end) {
                let within = (at - 8) % 16;
                let expected = match within {
                    0 | 1 => Some(0x8070_u16.to_le_bytes()[within as usize]),
                    4 | 5 => Some(0x8071_u16.to_le_bytes()[(within - 4) as usize]),
                    8 | 9 => Some(0x8001_u16.to_le_bytes()[(within - 8) as usize]),
                    _ => None,
                };
                if expected.is_some_and(|expected| expected != byte) {
                    return Err(self
                        .loc
                        .malformed("decoded text marker", "differs from observed record marker"));
                }
            } else if at >= u64::from(self.tail_start) {
                let relative = at - u64::from(self.tail_start);
                let within = relative % 28;
                if within < 4 {
                    let coordinate = &mut self.coordinates[(relative / 28) as usize];
                    if within < 2 {
                        coordinate.x |= u16::from(byte) << (within * 8);
                    } else {
                        coordinate.y |= u16::from(byte) << ((within - 2) * 8);
                    }
                }
            }
        }
        Ok(())
    }
}

// A private parameter avoids retaining private document prefixes in tests:
// synthetic fixtures substitute only their invented prefix digest and use
// precisely this production validation/streaming/assembly implementation.
async fn read_with_prefix<S: RangedSource, C: Cancellation>(
    source: &mut S,
    header: Header,
    page: PageRecord,
    limits: &Limits,
    cancellation: &C,
    budget: TextBudget,
    expected_prefix: [u8; 32],
) -> Result<TextCoordinates> {
    let loc = location(header, page);
    validate_metadata(header, page, source.size(), limits, budget, loc)?;
    if cancellation.is_cancelled() {
        return Err(loc.error(ErrorKind::Cancelled));
    }
    let mut max_source_request_bytes = 0;
    let mut fixed = [0; HEADER_BYTES];
    read_chunks(
        source,
        page.text.offset,
        &mut fixed,
        limits,
        cancellation,
        loc,
        &mut max_source_request_bytes,
    )
    .await?;
    if <[u8; 32]>::from(Sha256::digest(&fixed[..20])) != expected_prefix {
        return Err(loc.malformed(
            "page text prefix",
            "differs from observed variant fingerprint",
        ));
    }
    let decoded_length = u32::from_le_bytes(fixed[20..24].try_into().expect("fixed field width"));
    let decoded_bytes = u64::from(decoded_length);
    if decoded_bytes > budget.max_decoded_bytes {
        return Err(loc.limit(
            "decoded text bytes",
            budget.max_decoded_bytes,
            decoded_bytes,
        ));
    }
    if decoded_bytes > limits.max_output_bytes {
        return Err(loc.limit(
            "text decoded output bytes",
            limits.max_output_bytes,
            decoded_bytes,
        ));
    }
    let tail_bytes = u64::from(page.image_count) * 28;
    let glyph_bytes = decoded_bytes.checked_sub(12 + tail_bytes).ok_or_else(|| {
        loc.malformed(
            "decoded text layout",
            "too short for declared image records",
        )
    })?;
    if glyph_bytes % 16 != 0 {
        return Err(loc.malformed("decoded text layout", "record area is not a multiple of 16"));
    }
    let record_count = (glyph_bytes / 16) as u32;
    if record_count > budget.max_records {
        return Err(loc.limit(
            "text records",
            u64::from(budget.max_records),
            u64::from(record_count),
        ));
    }
    let zlib_frame = Span {
        offset: page.text.offset + HEADER_BYTES as u64,
        length: page.text.length - HEADER_BYTES as u64,
    };
    if zlib_frame.length < 6 {
        return Err(loc.at(zlib_frame.offset).error(ErrorKind::Truncated {
            field: "text zlib frame",
            expected: 6,
            available: zlib_frame.length,
        }));
    }
    let chunk = limits.io_chunk_bytes.min(CHUNK_BYTES);
    let input_count = zlib_frame.length.min(len_u64(chunk)) as usize;
    let output_count = (decoded_bytes + 1).min(len_u64(chunk)) as usize;
    let planned_buffers = len_u64(input_count + output_count)
        + u64::from(page.image_count) * size_of::<RawTextCoordinate>() as u64;
    check_working(planned_buffers, budget, loc)?;
    if TEXT_DECODER_RESERVATION_BYTES > limits.max_allocation_bytes {
        return Err(loc.limit(
            "text decoder allocation reservation",
            limits.max_allocation_bytes,
            TEXT_DECODER_RESERVATION_BYTES,
        ));
    }
    let mut input = allocate(input_count, 0_u8, limits, loc)?;
    let mut output = allocate(output_count, 0_u8, limits, loc)?;
    let images = usize_from_u32(page.image_count);
    let coordinates = allocate(images, RawTextCoordinate::default(), limits, loc)?;
    let mut accumulator = Accumulator {
        glyph_end: (8 + glyph_bytes) as u32,
        tail_start: (decoded_bytes - tail_bytes) as u32,
        coordinates,
        loc: loc.at(zlib_frame.offset),
    };
    let owned_buffer_bytes = len_u64(input.capacity())
        .saturating_add(len_u64(output.capacity()))
        .saturating_add(
            len_u64(accumulator.coordinates.capacity())
                .saturating_mul(size_of::<RawTextCoordinate>() as u64),
        );
    let working_memory_bytes = check_working(owned_buffer_bytes, budget, loc)?;
    let mut inflater = Decompress::new(true);
    let mut encoded_hash = Sha256::new();
    let mut decoded_hash = Sha256::new();
    let mut fetched = 0_u64;
    let mut buffered = 0;
    let mut used = 0;
    let mut max_decoder_output_chunk_bytes = 0;
    loop {
        if cancellation.is_cancelled() {
            return Err(loc
                .at(zlib_frame.offset + inflater.total_in())
                .error(ErrorKind::Cancelled));
        }
        if used == buffered && fetched < zlib_frame.length {
            buffered = (zlib_frame.length - fetched).min(len_u64(input.len())) as usize;
            read_chunks(
                source,
                zlib_frame.offset + fetched,
                &mut input[..buffered],
                limits,
                cancellation,
                loc,
                &mut max_source_request_bytes,
            )
            .await?;
            encoded_hash.update(&input[..buffered]);
            fetched += len_u64(buffered);
            used = 0;
        }
        let before_in = inflater.total_in();
        let before_out = inflater.total_out();
        // A one-byte excess sentinel remains available even at the declared
        // length, so the decoder can consume its trailer and report StreamEnd.
        let writable = (decoded_bytes - before_out + 1).min(len_u64(output.len())) as usize;
        let status = inflater
            .decompress(
                &input[used..buffered],
                &mut output[..writable],
                FlushDecompress::None,
            )
            .map_err(|_| {
                loc.at(zlib_frame.offset + before_in)
                    .malformed("text zlib frame", "invalid stream, dictionary or checksum")
            })?;
        if cancellation.is_cancelled() {
            return Err(loc
                .at(zlib_frame.offset + inflater.total_in())
                .error(ErrorKind::Cancelled));
        }
        used += (inflater.total_in() - before_in) as usize;
        let produced = (inflater.total_out() - before_out) as usize;
        if inflater.total_out() > decoded_bytes {
            return Err(loc
                .at(zlib_frame.offset + before_in)
                .malformed("decoded text length", "output exceeds declared length"));
        }
        max_decoder_output_chunk_bytes = max_decoder_output_chunk_bytes.max(produced);
        decoded_hash.update(&output[..produced]);
        accumulator.consume(before_out, &output[..produced])?;
        if status == Status::StreamEnd {
            if inflater.total_in() != zlib_frame.length || inflater.total_out() != decoded_bytes {
                return Err(loc.at(zlib_frame.offset + inflater.total_in()).malformed(
                    "text zlib frame",
                    "end differs from declared compressed/decoded span",
                ));
            }
            break;
        }
        if inflater.total_in() == before_in && inflater.total_out() == before_out {
            return Err(loc.at(zlib_frame.offset + before_in).malformed(
                "text zlib frame",
                "truncated stream or decoder made no progress",
            ));
        }
    }
    Ok(TextCoordinates {
        text: page.text,
        zlib_frame,
        decoded_length,
        record_count,
        coordinates: accumulator.coordinates,
        encoded_sha256: encoded_hash.finalize().into(),
        decoded_sha256: decoded_hash.finalize().into(),
        max_source_request_bytes,
        max_decoder_output_chunk_bytes,
        owned_buffer_bytes,
        working_memory_bytes,
    })
}

#[cfg(test)]
mod tests;
