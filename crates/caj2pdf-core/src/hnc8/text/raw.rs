// SPDX-License-Identifier: MIT

//! Observed uncompressed HN-A records, with no text or whole-page buffering.

use super::records::Records;
use super::*;

pub(super) async fn read<S: RangedSource, C: Cancellation>(
    source: &mut S,
    page: PageRecord,
    limits: &Limits,
    cancellation: &C,
    budget: TextBudget,
    loc: Location,
    mut records: Records,
) -> Result<TextCoordinates> {
    let bytes = page.text.length;
    let ceiling = budget.max_decoded_bytes.min(limits.max_output_bytes);
    if bytes > ceiling {
        return Err(loc.limit("raw text bytes", ceiling, bytes));
    }
    let chunk = (limits.io_chunk_bytes.min(CHUNK_BYTES) as u64).min(bytes) as usize;
    let count = usize_from_u32(page.image_count);
    let planned = chunk as u64 + count as u64 * size_of::<RawTextCoordinate>() as u64;
    // Raw parsing needs only fixed record/hash scratch, not an inflater.
    check_working(planned, false, budget, loc)?;
    let mut buffer = allocate(chunk, 0_u8, limits, loc)?;
    let mut coordinates = allocate(count, RawTextCoordinate::default(), limits, loc)?;
    let owned_buffer_bytes =
        (buffer.capacity() + coordinates.capacity() * size_of::<RawTextCoordinate>()) as u64;
    let working_memory_bytes = check_working(owned_buffer_bytes, false, budget, loc)?;
    let mut offset = 0;
    let mut max_source_request_bytes = 0;
    while offset < bytes {
        let length = (bytes - offset).min(chunk as u64) as usize;
        read_chunks(
            source,
            page.text.offset + offset,
            &mut buffer[..length],
            limits,
            cancellation,
            loc,
            &mut max_source_request_bytes,
        )
        .await?;
        records.consume(
            page.text.offset + offset,
            &buffer[..length],
            &mut coordinates,
            loc,
        )?;
        offset += length as u64;
    }
    let record_count = records.finish(&mut coordinates, loc)?;
    Ok(TextCoordinates {
        text: page.text,
        page_size: None,
        zlib_frame: None,
        decoded_length: bytes as u32,
        record_count,
        coordinates,
        max_source_request_bytes,
        max_decoder_output_chunk_bytes: 0,
        owned_buffer_bytes,
        working_memory_bytes,
    })
}
