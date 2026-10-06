// SPDX-License-Identifier: MIT

//! Streaming metadata reader for the measured HN-A/C8 page-text profile.
//! Opaque text is discarded. Coordinate words have no assigned signedness
//! or physical units here, and this reader does not enable composition.

use super::inflate::{ExactInflate, InflateFault, InflateFaultKind};
use super::{ErrorKind, Header, Hnc8Error, Location, PageRecord, Result, Span, Variant};
use crate::fallible::{len_u64, reserve_exact, usize_from_u32};
use crate::{Cancellation, Error, Limits, RangedSource, read_exact_at};

mod raw;
mod records;

use records::Records;

const HEADER_BYTES: usize = 24;
const CHUNK_BYTES: usize = 64 * 1024;
const FIXED_WORKING_BYTES: u64 = 4096;
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

/// The position and display extent words at +0/+2/+4/+6 of a tail record.
/// These are raw bits; an unsigned Rust representation does not establish
/// the source format's signedness, units or accepted coordinate range.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RawTextCoordinate {
    pub x: u16,
    pub y: u16,
    /// Declared display extent in source units, independent of decoded pixels.
    pub width: u16,
    pub height: u16,
}

/// Validated framing and bounded image-order metadata, with no opaque text.
/// Coordinates become available only after the complete frame and checksum
/// have succeeded. There is no temporary spool or disk use in this reader.
#[derive(Debug, Eq, PartialEq)]
pub struct TextCoordinates {
    pub text: Span,
    /// Per-page HN-A dimensions from the validated paired `8003` prefix.
    /// None for other framing; zero values remain inspectable but cannot render.
    pub page_size: Option<[u16; 2]>,
    /// None for uncompressed records.
    pub zlib_frame: Option<Span>,
    /// Expanded length, or the full indexed raw span including its opaque tail.
    pub decoded_length: u32,
    /// Fixed-layout glyph records, or logical raw/direct-frame records.
    pub record_count: u32,
    pub coordinates: Vec<RawTextCoordinate>,
    pub max_source_request_bytes: usize,
    pub max_decoder_output_chunk_bytes: usize,
    /// Capacities of the compressed/expanded scratch and coordinate Vecs.
    pub owned_buffer_bytes: u64,
    /// Owned buffers and fixed scratch; compressed input also reserves a decoder.
    pub working_memory_bytes: u64,
}

/// Page text the coordinate readers accept, or text they do not frame.
pub(super) enum PageText {
    /// Compressed or raw text with its image coordinates.
    Framed(TextCoordinates),
    /// A span shorter than the compressed header, or one starting with
    /// neither `COMPRESSTEXT` header nor raw HN-A records. C8 native text is
    /// framed like this; the error reports it to readers that need records.
    Unframed(Hnc8Error),
}

/// Validate either compressed HN-A/C8 text layout or uncompressed HN-A records
/// and return the image coordinates for composition.
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
/// Composition may receive fewer coordinates than descriptors; it must prove
/// all additional descriptor payloads repeat that coordinate group.
pub(super) fn read_coordinates<S: RangedSource, C: Cancellation>(
    source: &mut S,
    header: Header,
    page: PageRecord,
    limits: &Limits,
    cancellation: &C,
    budget: TextBudget,
) -> Result<TextCoordinates> {
    match read_page_text(source, header, page, limits, cancellation, budget)? {
        PageText::Framed(text) => Ok(text),
        PageText::Unframed(error) => Err(error),
    }
}

/// [`read_coordinates`], reporting unframed text apart from other errors.
pub(super) fn read_page_text<S: RangedSource, C: Cancellation>(
    source: &mut S,
    header: Header,
    page: PageRecord,
    limits: &Limits,
    cancellation: &C,
    budget: TextBudget,
) -> Result<PageText> {
    if header.variant == Variant::HnB {
        return Err(location(header, page).error(ErrorKind::Unsupported {
            field: "text framing variant",
            value: 2,
        }));
    }
    let loc = location(header, page);
    validate_metadata(header, page, source.size(), limits, loc)?;
    if page.text.length < HEADER_BYTES as u64 {
        return Ok(PageText::Unframed(loc.error(ErrorKind::Truncated {
            field: "page text header",
            expected: HEADER_BYTES as u64,
            available: page.text.length,
        })));
    }
    validate_budget(page, budget, loc)?;
    let mut prefix = [0; 4];
    read_chunks(
        source,
        page.text.offset,
        &mut prefix,
        limits,
        cancellation,
        loc,
        &mut 0,
    )?;
    let tag = u16::from_le_bytes([prefix[0], prefix[1]]);
    // The same paired page-prefix records also precede uncompressed HN-A
    // records. Decide only at the indexed start, never by marker searching.
    let mut prefixed_raw = false;
    let mut page_size = None;
    if header.variant == Variant::HnA && tag == 0x8003 {
        let mut following = [0; 16];
        read_chunks(
            source,
            page.text.offset + 4,
            &mut following,
            limits,
            cancellation,
            loc,
            &mut 0,
        )?;
        if following[..2] == [0x03, 0x80] {
            page_size = Some([
                u16::from_le_bytes([prefix[2], prefix[3]]),
                u16::from_le_bytes([following[2], following[3]]),
            ]);
        }
        prefixed_raw = following[..2] == [0x03, 0x80]
            && matches!(
                u16::from_le_bytes([following[4], following[5]]),
                0x800a | 0x801c
            );
    }
    let mut text = if header.variant == Variant::HnA
        && (prefixed_raw || matches!(tag, 0x8001 | 0x800a | 0x8004))
    {
        let records = if prefixed_raw || tag == 0x800a {
            Records::tagged(budget.max_records, prefixed_raw).decode_markers(true)
        } else {
            Records::ordered(budget.max_records)
        };
        raw::read(source, page, limits, cancellation, budget, loc, records)?
    } else {
        match read_compressed_text(source, header, page, limits, cancellation, budget)? {
            PageText::Framed(text) => text,
            unframed => return Ok(unframed),
        }
    };
    text.page_size = page_size;
    Ok(PageText::Framed(text))
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
    let index_start_valid = if header.variant == Variant::C8 {
        header.page_index.offset == 0x50
    } else {
        // HN-B was rejected before metadata validation.
        header.page_index.offset >= 0x15c
            && (header.page_index.offset - 0x15c).is_multiple_of(super::OUTLINE_RECORD_BYTES)
            && (header.page_index.offset - 0x15c) / super::OUTLINE_RECORD_BYTES <= i32::MAX as u64
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
    Ok(())
}

fn validate_budget(page: PageRecord, budget: TextBudget, loc: Location) -> Result<()> {
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

fn read_chunks<S: RangedSource, C: Cancellation>(
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
        read_exact_at(source, current, chunk, limits, cancellation).map_err(
            |error| match error {
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
            },
        )?;
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

fn check_working(owned: u64, compressed: bool, budget: TextBudget, loc: Location) -> Result<u64> {
    let decoder = if compressed {
        TEXT_DECODER_RESERVATION_BYTES
    } else {
        0
    };
    let working = owned.saturating_add(decoder + FIXED_WORKING_BYTES);
    if working > budget.max_working_bytes {
        return Err(loc.limit("text working bytes", budget.max_working_bytes, working));
    }
    Ok(working)
}

fn read_compressed_text<S: RangedSource, C: Cancellation>(
    source: &mut S,
    header: Header,
    page: PageRecord,
    limits: &Limits,
    cancellation: &C,
    budget: TextBudget,
) -> Result<PageText> {
    let loc = location(header, page);

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
    )?;
    // A direct marker selects compact records. The older tagged prefix
    // selects the fixed glyph/tail layout; its +2/+6 payload words vary.
    // Both paths validate the declared length and complete zlib frame.
    let header_bytes = if &fixed[..12] == b"COMPRESSTEXT" {
        16
    } else if fixed[..2] == [0x03, 0x80]
        && fixed[4..6] == [0x03, 0x80]
        && &fixed[8..20] == b"COMPRESSTEXT"
    {
        HEADER_BYTES
    } else {
        return Ok(PageText::Unframed(loc.malformed(
            "page text prefix",
            "unsupported compressed text header",
        )));
    };
    let decoded_length = u32::from_le_bytes(
        fixed[header_bytes - 4..header_bytes]
            .try_into()
            .expect("fixed field width"),
    );
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
    let (glyph_bytes, record_count) = if header_bytes == HEADER_BYTES {
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
        (glyph_bytes, record_count)
    } else {
        (0, 0)
    };
    let zlib_frame = Span {
        offset: page.text.offset + header_bytes as u64,
        length: page.text.length - header_bytes as u64,
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
    check_working(planned_buffers, true, budget, loc)?;
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
    let mut coordinates = allocate(images, RawTextCoordinate::default(), limits, loc)?;
    let mut records = if header_bytes == 16 {
        Records::tagged(budget.max_records, false)
    } else {
        Records::fixed(
            8 + glyph_bytes,
            decoded_bytes.saturating_sub(tail_bytes),
            record_count,
        )
        .decode_markers(header.variant == Variant::HnA)
    };
    let frame_loc = loc.at(zlib_frame.offset);
    let owned_buffer_bytes = len_u64(input.capacity())
        .saturating_add(len_u64(output.capacity()))
        .saturating_add(
            len_u64(coordinates.capacity()).saturating_mul(size_of::<RawTextCoordinate>() as u64),
        );
    let working_memory_bytes = check_working(owned_buffer_bytes, true, budget, loc)?;
    let mut inflate = ExactInflate::new(zlib_frame.offset, zlib_frame.length, decoded_bytes);
    let fault = |fault: InflateFault| {
        let (field, reason) = match fault.kind {
            InflateFaultKind::Invalid => {
                ("text zlib frame", "invalid stream, dictionary or checksum")
            }
            InflateFaultKind::Excess => ("decoded text length", "output exceeds declared length"),
            InflateFaultKind::EndMismatch => (
                "text zlib frame",
                "end differs from declared compressed/decoded span",
            ),
            InflateFaultKind::Stalled => (
                "text zlib frame",
                "truncated stream or decoder made no progress",
            ),
        };
        loc.at(fault.offset).malformed(field, reason)
    };
    let mut max_decoder_output_chunk_bytes = 0;
    loop {
        if cancellation.is_cancelled() {
            return Err(loc.at(inflate.position()).error(ErrorKind::Cancelled));
        }
        if let Some((at, length)) = inflate.next_read(input.len()) {
            read_chunks(
                source,
                at,
                &mut input[..length],
                limits,
                cancellation,
                loc,
                &mut max_source_request_bytes,
            )?;
        }
        let writable = inflate.writable(output.len());
        let step = inflate
            .step(&input, &mut output[..writable])
            .map_err(fault)?;
        if cancellation.is_cancelled() {
            return Err(loc.at(inflate.position()).error(ErrorKind::Cancelled));
        }
        inflate.check_length(&step).map_err(fault)?;
        max_decoder_output_chunk_bytes = max_decoder_output_chunk_bytes.max(step.produced);
        records.consume(
            step.before_out,
            &output[..step.produced],
            &mut coordinates,
            frame_loc,
        )?;
        if inflate.finished(&step).map_err(fault)? {
            break;
        }
    }
    let record_count = records.finish(&mut coordinates, frame_loc)?;
    Ok(PageText::Framed(TextCoordinates {
        text: page.text,
        page_size: None,
        zlib_frame: Some(zlib_frame),
        decoded_length,
        record_count,
        coordinates,
        max_source_request_bytes,
        max_decoder_output_chunk_bytes,
        owned_buffer_bytes,
        working_memory_bytes,
    }))
}

#[cfg(test)]
mod tests;
