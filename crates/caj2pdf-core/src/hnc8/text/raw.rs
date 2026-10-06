// SPDX-License-Identifier: MIT

//! Observed uncompressed HN-A records, with no text or whole-page buffering.

use super::records::Records;
use super::*;

pub(super) fn read<S: RangedSource, C: Cancellation>(
    source: &mut S,
    page: PageRecord,
    limits: &Limits,
    cancellation: &C,
    loc: Location,
    mut records: Records,
) -> Result<TextCoordinates> {
    let bytes = page.text.length;
    if bytes > limits.max_output_bytes {
        return Err(loc.limit("raw text bytes", limits.max_output_bytes, bytes));
    }
    let chunk = (limits.io_chunk_bytes.min(CHUNK_BYTES) as u64).min(bytes) as usize;
    let count = usize_from_u32(page.image_count);
    let mut buffer = allocate(chunk, 0_u8, limits, loc)?;
    let mut coordinates = allocate(count, RawTextCoordinate::default(), limits, loc)?;
    let owned_buffer_bytes =
        (buffer.capacity() + coordinates.capacity() * size_of::<RawTextCoordinate>()) as u64;
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
        )?;
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
    })
}
