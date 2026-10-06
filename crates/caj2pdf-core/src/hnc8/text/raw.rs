// SPDX-License-Identifier: MIT

//! Observed uncompressed HN-A records, with no text or whole-page buffering.

use super::*;

#[derive(Clone, Copy)]
enum Stage {
    Start,
    X,
    Y,
    Glyphs,
    Images,
    End,
    Done,
}

struct Records {
    stage: Stage,
    bytes: [u8; 28],
    filled: usize,
    needed: usize,
    start: u64,
    count: u32,
    images: usize,
    coordinates: Vec<RawTextCoordinate>,
    compact: Option<super::records::Records>,
}

impl Records {
    fn consume(
        &mut self,
        offset: u64,
        input: &[u8],
        max_records: u32,
        loc: Location,
    ) -> Result<()> {
        if let Some(compact) = &mut self.compact {
            return compact.consume(input, &mut self.coordinates, loc);
        }
        for (index, &byte) in input.iter().enumerate() {
            if matches!(self.stage, Stage::Done) {
                break; // Remaining indexed bytes are opaque, but still read and hashed.
            }
            if self.filled == 0 {
                self.start = offset + index as u64;
            }
            self.bytes[self.filled] = byte;
            self.filled += 1;
            if self.filled != self.needed {
                continue;
            }
            let at = loc.at(self.start);
            let tag = u16::from_le_bytes([self.bytes[0], self.bytes[1]]);
            if self.needed == 28 {
                self.coordinates[self.images] = RawTextCoordinate {
                    x: u16::from_le_bytes([self.bytes[4], self.bytes[5]]),
                    y: u16::from_le_bytes([self.bytes[6], self.bytes[7]]),
                    width: u16::from_le_bytes([self.bytes[8], self.bytes[9]]),
                    height: u16::from_le_bytes([self.bytes[10], self.bytes[11]]),
                };
                self.images += 1;
                self.stage = if self.images == self.coordinates.len() {
                    Stage::End
                } else {
                    Stage::Images
                };
            } else {
                if self.count == max_records {
                    return Err(at.limit(
                        "text records",
                        u64::from(max_records),
                        u64::from(self.count) + 1,
                    ));
                }
                self.count += 1;
                self.stage = match (self.stage, tag) {
                    (Stage::Start | Stage::Glyphs, 0x8001) => Stage::X,
                    (Stage::X, 0x8070) => Stage::Y,
                    (Stage::Y, 0x8071) => Stage::Glyphs,
                    (Stage::Glyphs, 0..=0x7fff) => Stage::Glyphs,
                    (Stage::Glyphs | Stage::Images, 0x800a)
                        if self.images < self.coordinates.len() =>
                    {
                        self.needed = 28;
                        continue;
                    }
                    (Stage::Start | Stage::Glyphs | Stage::End, 0x8004)
                        if self.images == self.coordinates.len() =>
                    {
                        Stage::Done
                    }
                    _ => {
                        return Err(
                            at.malformed("raw text record", "unexpected tag or record order")
                        );
                    }
                };
            }
            self.filled = 0;
            self.needed = 4;
        }
        Ok(())
    }
}

pub(super) async fn read<S: RangedSource, C: Cancellation>(
    source: &mut S,
    page: PageRecord,
    limits: &Limits,
    cancellation: &C,
    budget: TextBudget,
    loc: Location,
    compact: Option<super::records::Records>,
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
    let coordinates = allocate(count, RawTextCoordinate::default(), limits, loc)?;
    let owned_buffer_bytes =
        (buffer.capacity() + coordinates.capacity() * size_of::<RawTextCoordinate>()) as u64;
    let working_memory_bytes = check_working(owned_buffer_bytes, false, budget, loc)?;
    let mut records = Records {
        stage: Stage::Start,
        bytes: [0; 28],
        filled: 0,
        needed: 4,
        start: page.text.offset,
        count: 0,
        images: 0,
        coordinates,
        compact,
    };
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
            budget.max_records,
            loc,
        )?;
        offset += length as u64;
    }
    let record_count = if let Some(compact) = records.compact {
        compact.finish(&mut records.coordinates, loc)?
    } else if !matches!(records.stage, Stage::Done) {
        return Err(loc
            .at(page.text.offset + bytes)
            .malformed("raw text records", "missing image records or end marker"));
    } else {
        records.count
    };
    Ok(TextCoordinates {
        text: page.text,
        page_size: None,
        zlib_frame: None,
        decoded_length: bytes as u32,
        record_count,
        coordinates: records.coordinates,
        max_source_request_bytes,
        max_decoder_output_chunk_bytes: 0,
        owned_buffer_bytes,
        working_memory_bytes,
    })
}
