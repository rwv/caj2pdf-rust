// SPDX-License-Identifier: MIT

//! Checked metadata reader for the observed `CAJ\0` container variant.
//!
//! The public format observations identify the page count, outline records,
//! and a second-level pointer to the PDF body. The corpus also contains a
//! twelve-byte page table at that pointer. Its final offset is only a body
//! *hint*: one observed file understates the final stream by 25 bytes.
//! Conversion must validate the PDF objects before using that boundary.

mod converter;
mod gb18030;

pub use converter::convert_caj;

use crate::fallible::{reserve_exact, usize_from_u32};
use crate::{Bookmark, Cancellation, Error, Limits, RangedSource, Result, read_exact_at};
use std::mem::size_of;

const MAGIC: &[u8; 4] = b"CAJ\0";
const PAGE_COUNT_OFFSET: u64 = 0x10;
const PAGE_TABLE_POINTER_OFFSET: u64 = 0x14;
const TOC_COUNT_OFFSET: u64 = 0x110;
const TOC_RECORDS_OFFSET: u64 = 0x114;
const TOC_RECORD_BYTES: u64 = 308;
const PAGE_ROW_BYTES: u64 = 12;
const PAGE_TABLE_BATCH_ROWS: usize = 512;
const PAGE_TABLE_BATCH_BYTES: usize = PAGE_TABLE_BATCH_ROWS * PAGE_ROW_BYTES as usize;

/// One page-table entry in the document's intended page order.
///
/// A zero-length entry is valid: its page object can reside in another row's
/// bytes. Row boundaries need not coincide with PDF object boundaries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CajPageRow {
    pub offset: u64,
    pub length: u64,
    pub page_object_id: u32,
}

/// Bounded CAJ metadata required to reconstruct the embedded PDF body.
///
/// Header bytes other than the magic and two documented fields, as well as
/// the TOC record's two unknown fields, are intentionally left uninterpreted.
/// `body_end_hint` comes from the page table and is not a validated PDF end.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CajMetadata {
    pub page_count: u32,
    pub body_start: u64,
    pub body_end_hint: u64,
    pub page_rows: Vec<CajPageRow>,
    pub bookmarks: Vec<Bookmark>,
}

fn malformed(offset: u64, record: Option<u32>, reason: &'static str) -> Error {
    Error::Caj {
        offset,
        record,
        reason,
    }
}

fn limit(
    offset: u64,
    record: Option<u32>,
    resource: &'static str,
    maximum: u64,
    attempted: u64,
) -> Error {
    Error::CajLimitExceeded {
        offset,
        record,
        resource,
        limit: maximum,
        attempted,
    }
}

fn checked_end(offset: u64, length: u64, record: Option<u32>) -> Result<u64> {
    offset
        .checked_add(length)
        .ok_or_else(|| malformed(offset, record, "CAJ range end overflows"))
}

fn checked_mul(left: u64, right: u64, offset: u64) -> Result<u64> {
    left.checked_mul(right)
        .ok_or_else(|| malformed(offset, None, "CAJ record size overflows"))
}

fn check_allocation<T>(
    limits: &Limits,
    count: u32,
    offset: u64,
    resource: &'static str,
) -> Result<usize> {
    let bytes = checked_mul(u64::from(count), size_of::<T>() as u64, offset)?;
    if bytes > limits.max_allocation_bytes {
        return Err(limit(
            offset,
            None,
            resource,
            limits.max_allocation_bytes,
            bytes,
        ));
    }
    Ok(usize_from_u32(count))
}

async fn read_field<S: RangedSource, C: Cancellation>(
    source: &mut S,
    offset: u64,
    destination: &mut [u8],
    record: Option<u32>,
    limits: &Limits,
    cancellation: &C,
) -> Result<()> {
    let end = checked_end(offset, destination.len() as u64, record)?;
    if end > source.size() {
        return Err(malformed(offset, record, "CAJ field extends beyond source"));
    }
    let mut read = 0usize;
    while read < destination.len() {
        let chunk = (destination.len() - read).min(limits.io_chunk_bytes);
        read_exact_at(
            source,
            offset + read as u64,
            &mut destination[read..read + chunk],
            limits,
            cancellation,
        )
        .await?;
        read += chunk;
    }
    Ok(())
}

fn little_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn parse_page_number(
    bytes: &[u8; 308],
    record_offset: u64,
    record: u32,
    pages: u32,
) -> Result<u32> {
    let field = &bytes[280..292];
    let end = field
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(field.len());
    let raw = &field[..end];
    let first = raw
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .ok_or_else(|| malformed(record_offset + 280, Some(record), "empty TOC page number"))?;
    let mut page = 0u32;
    for (index, byte) in raw[first..].trim_ascii_end().iter().enumerate() {
        if !byte.is_ascii_digit() {
            return Err(malformed(
                record_offset + 280 + (first + index) as u64,
                Some(record),
                "TOC page number is not ASCII decimal",
            ));
        }
        page = page
            .checked_mul(10)
            .and_then(|value| value.checked_add(u32::from(byte - b'0')))
            .ok_or_else(|| {
                malformed(
                    record_offset + 280 + (first + index) as u64,
                    Some(record),
                    "TOC page number overflows",
                )
            })?;
    }
    if page == 0 || page > pages {
        return Err(malformed(
            record_offset + 280,
            Some(record),
            "TOC page number is outside the document",
        ));
    }
    Ok(page - 1)
}

/// Read CAJ page and outline metadata from a seekable source.
///
/// The returned vectors are bounded by `Limits`; no PDF payload is copied or
/// buffered. Unknown header flags are accepted because their meaning has not
/// been established by the public format observations.
pub async fn parse_metadata<S: RangedSource, C: Cancellation>(
    source: &mut S,
    limits: &Limits,
    cancellation: &C,
) -> Result<CajMetadata> {
    limits.validate()?;

    let mut magic = [0u8; 4];
    read_field(source, 0, &mut magic, None, limits, cancellation).await?;
    if &magic != MAGIC {
        return Err(Error::UnsupportedFormat);
    }

    let mut page_header = [0u8; 8];
    read_field(
        source,
        PAGE_COUNT_OFFSET,
        &mut page_header,
        None,
        limits,
        cancellation,
    )
    .await?;
    let raw_page_count = i32::from_le_bytes(page_header[..4].try_into().expect("four bytes"));
    if raw_page_count <= 0 {
        return Err(malformed(
            PAGE_COUNT_OFFSET,
            None,
            "CAJ page count must be positive",
        ));
    }
    let page_count = raw_page_count as u32;
    if page_count > limits.max_pages {
        return Err(limit(
            PAGE_COUNT_OFFSET,
            None,
            "CAJ pages",
            u64::from(limits.max_pages),
            u64::from(page_count),
        ));
    }
    let table_start = u64::from(little_u32(&page_header[4..8]));

    let mut toc_count_bytes = [0u8; 4];
    read_field(
        source,
        TOC_COUNT_OFFSET,
        &mut toc_count_bytes,
        None,
        limits,
        cancellation,
    )
    .await?;
    let raw_toc_count = i32::from_le_bytes(toc_count_bytes);
    if raw_toc_count < 0 {
        return Err(malformed(
            TOC_COUNT_OFFSET,
            None,
            "CAJ TOC count is negative",
        ));
    }
    let toc_count = raw_toc_count as u32;
    if toc_count > limits.max_bookmarks {
        return Err(limit(
            TOC_COUNT_OFFSET,
            None,
            "CAJ bookmarks",
            u64::from(limits.max_bookmarks),
            u64::from(toc_count),
        ));
    }

    let toc_bytes = checked_mul(u64::from(toc_count), TOC_RECORD_BYTES, TOC_COUNT_OFFSET)?;
    let toc_end = checked_end(TOC_RECORDS_OFFSET, toc_bytes, None)?;
    if toc_end > source.size() {
        let record = ((source.size().saturating_sub(TOC_RECORDS_OFFSET) / TOC_RECORD_BYTES)
            .min(u64::from(toc_count.saturating_sub(1)))
            + 1) as u32;
        return Err(malformed(
            source.size(),
            Some(record),
            "CAJ TOC record is truncated",
        ));
    }
    if table_start < toc_end {
        return Err(malformed(
            PAGE_TABLE_POINTER_OFFSET,
            None,
            "CAJ page table overlaps the header or TOC",
        ));
    }
    let table_bytes = checked_mul(u64::from(page_count), PAGE_ROW_BYTES, PAGE_COUNT_OFFSET)?;
    let table_end = checked_end(table_start, table_bytes, None)?;
    if table_end > source.size() {
        return Err(malformed(
            PAGE_TABLE_POINTER_OFFSET,
            None,
            "CAJ page table extends beyond source",
        ));
    }

    let page_capacity =
        check_allocation::<CajPageRow>(limits, page_count, PAGE_COUNT_OFFSET, "CAJ page metadata")?;
    let mut page_rows = Vec::new();
    let refused = malformed(
        PAGE_COUNT_OFFSET,
        None,
        "CAJ page metadata allocation failed",
    );
    reserve_exact(&mut page_rows, page_capacity, refused)?;

    let mut previous_end = None;
    let mut batch = [0u8; PAGE_TABLE_BATCH_BYTES];
    for batch_start in (0..page_count).step_by(PAGE_TABLE_BATCH_ROWS) {
        let batch_count = (page_count - batch_start).min(PAGE_TABLE_BATCH_ROWS as u32);
        let batch_offset = table_start + u64::from(batch_start) * PAGE_ROW_BYTES;
        read_field(
            source,
            batch_offset,
            &mut batch[..batch_count as usize * PAGE_ROW_BYTES as usize],
            Some(batch_start + 1),
            limits,
            cancellation,
        )
        .await?;
        for local_index in 0..batch_count {
            let index = batch_start + local_index;
            let row_offset = table_start + u64::from(index) * PAGE_ROW_BYTES;
            let record = index + 1;
            let row_start = local_index as usize * PAGE_ROW_BYTES as usize;
            let row = &batch[row_start..row_start + PAGE_ROW_BYTES as usize];
            let offset = u64::from(little_u32(&row[..4]));
            let length = u64::from(little_u32(&row[4..8]));
            let page_object_id = little_u32(&row[8..12]);
            if page_object_id == 0 {
                return Err(malformed(
                    row_offset + 8,
                    Some(record),
                    "CAJ page object number must be positive",
                ));
            }
            if let Some(expected) = previous_end {
                if offset != expected {
                    return Err(malformed(
                        row_offset,
                        Some(record),
                        "CAJ page spans are not contiguous",
                    ));
                }
            } else if offset < table_end {
                return Err(malformed(
                    row_offset,
                    Some(record),
                    "CAJ PDF body overlaps the page table",
                ));
            }
            let end = checked_end(offset, length, Some(record))?;
            if end > source.size() {
                return Err(malformed(
                    row_offset + 4,
                    Some(record),
                    "CAJ page span extends beyond source",
                ));
            }
            previous_end = Some(end);
            page_rows.push(CajPageRow {
                offset,
                length,
                page_object_id,
            });
        }
    }

    let body_start = page_rows[0].offset;
    let body_end_hint = previous_end.expect("positive page count");
    if body_end_hint == body_start {
        return Err(malformed(table_start, None, "CAJ PDF body is empty"));
    }
    let body_length = body_end_hint - body_start;
    if body_length > limits.max_input_bytes {
        return Err(limit(
            body_start,
            None,
            "CAJ PDF input bytes",
            limits.max_input_bytes,
            body_length,
        ));
    }

    let order_capacity =
        check_allocation::<(u32, u32)>(limits, page_count, PAGE_COUNT_OFFSET, "CAJ page order")?;
    let mut ordered_ids = Vec::new();
    ordered_ids
        .try_reserve_exact(order_capacity)
        .map_err(|_| malformed(PAGE_COUNT_OFFSET, None, "CAJ page order allocation failed"))?;
    for (index, row) in page_rows.iter().enumerate() {
        ordered_ids.push((row.page_object_id, index as u32));
    }
    ordered_ids.sort_unstable_by_key(|entry| entry.0);
    if let Some((_, duplicate_index)) = ordered_ids
        .windows(2)
        .find(|pair| pair[0].0 == pair[1].0)
        .map(|pair| pair[1])
    {
        let record = duplicate_index + 1;
        return Err(malformed(
            table_start + u64::from(duplicate_index) * PAGE_ROW_BYTES + 8,
            Some(record),
            "duplicate CAJ page object number",
        ));
    }

    let bookmark_capacity =
        check_allocation::<Bookmark>(limits, toc_count, TOC_COUNT_OFFSET, "CAJ bookmarks")?;
    let mut bookmarks = Vec::new();
    bookmarks
        .try_reserve_exact(bookmark_capacity)
        .map_err(|_| malformed(TOC_COUNT_OFFSET, None, "CAJ bookmark allocation failed"))?;
    let mut previous_level = 0u32;
    for index in 0..toc_count {
        let record = index + 1;
        let record_offset = TOC_RECORDS_OFFSET + u64::from(index) * TOC_RECORD_BYTES;
        let mut bytes = [0u8; TOC_RECORD_BYTES as usize];
        read_field(
            source,
            record_offset,
            &mut bytes,
            Some(record),
            limits,
            cancellation,
        )
        .await?;
        let title_end = bytes[..256]
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(256);
        if title_end == 0 {
            return Err(malformed(
                record_offset,
                Some(record),
                "empty CAJ TOC title",
            ));
        }
        // The decoder initially reserves the encoded length and may grow its
        // String once. GB18030 never needs more than two UTF-8 bytes per input
        // byte, so this bound covers its maximum requested title allocation.
        let title_budget = (title_end as u64) * 2;
        if title_budget > limits.max_allocation_bytes {
            return Err(limit(
                record_offset,
                Some(record),
                "CAJ title allocation bytes",
                limits.max_allocation_bytes,
                title_budget,
            ));
        }
        let title = gb18030::decode(&bytes[..title_end]).map_err(|error| {
            malformed(
                record_offset + error.offset as u64,
                Some(record),
                "CAJ TOC title is not valid GB18030",
            )
        })?;
        // The decoded title is within the budget checked above.
        debug_assert!(title.len() as u64 <= title_budget);
        let page_index = parse_page_number(&bytes, record_offset, record, page_count)?;
        let raw_level = i32::from_le_bytes([bytes[304], bytes[305], bytes[306], bytes[307]]);
        if raw_level <= 0 {
            return Err(malformed(
                record_offset + 304,
                Some(record),
                "CAJ TOC level must be positive",
            ));
        }
        let level = raw_level as u32;
        if level > previous_level + 1 {
            return Err(malformed(
                record_offset + 304,
                Some(record),
                "CAJ TOC level skips a parent",
            ));
        }
        previous_level = level;
        bookmarks.push(Bookmark {
            depth: level - 1,
            title,
            page_index,
        });
    }

    Ok(CajMetadata {
        page_count,
        body_start,
        body_end_hint,
        page_rows,
        bookmarks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NeverCancel;
    use crate::test_support::ready;

    struct Source {
        bytes: Vec<u8>,
        largest_request: usize,
        read_calls: usize,
        max_transfer: Option<usize>,
    }

    impl RangedSource for Source {
        fn size(&self) -> u64 {
            self.bytes.len() as u64
        }

        async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
            self.read_calls += 1;
            self.largest_request = self.largest_request.max(destination.len());
            let start = offset as usize;
            let length = destination
                .len()
                .min(self.max_transfer.unwrap_or(usize::MAX))
                .min(self.bytes.len().saturating_sub(start));
            if length > 0 {
                destination[..length].copy_from_slice(&self.bytes[start..start + length]);
            }
            Ok(length)
        }
    }

    fn sample() -> Source {
        let first = b"1 0 obj\n<< /Type /Page >>\nendobj\n";
        let second = b"2 0 obj\n<< /Type /Page >>\nendobj\n";
        let table_start = 0x400usize;
        let body_start = table_start + 24;
        let mut bytes = vec![0u8; body_start + first.len() + second.len()];
        bytes[..4].copy_from_slice(MAGIC);
        bytes[PAGE_COUNT_OFFSET as usize..PAGE_COUNT_OFFSET as usize + 4]
            .copy_from_slice(&2u32.to_le_bytes());
        bytes[PAGE_TABLE_POINTER_OFFSET as usize..PAGE_TABLE_POINTER_OFFSET as usize + 4]
            .copy_from_slice(&(table_start as u32).to_le_bytes());
        bytes[TOC_COUNT_OFFSET as usize..TOC_COUNT_OFFSET as usize + 4]
            .copy_from_slice(&2u32.to_le_bytes());
        bytes[TOC_RECORDS_OFFSET as usize..TOC_RECORDS_OFFSET as usize + 5]
            .copy_from_slice(b"Intro");
        bytes[TOC_RECORDS_OFFSET as usize + 280] = b'1';
        bytes[TOC_RECORDS_OFFSET as usize + 304..TOC_RECORDS_OFFSET as usize + 308]
            .copy_from_slice(&1u32.to_le_bytes());
        let second_toc = TOC_RECORDS_OFFSET as usize + TOC_RECORD_BYTES as usize;
        bytes[second_toc..second_toc + 3].copy_from_slice(b"Sub");
        bytes[second_toc + 280] = b'2';
        bytes[second_toc + 304..second_toc + 308].copy_from_slice(&2u32.to_le_bytes());
        bytes[table_start..table_start + 4].copy_from_slice(&(body_start as u32).to_le_bytes());
        bytes[table_start + 4..table_start + 8]
            .copy_from_slice(&(first.len() as u32).to_le_bytes());
        bytes[table_start + 8..table_start + 12].copy_from_slice(&1u32.to_le_bytes());
        let row_two = table_start + 12;
        bytes[row_two..row_two + 4]
            .copy_from_slice(&((body_start + first.len()) as u32).to_le_bytes());
        bytes[row_two + 4..row_two + 8].copy_from_slice(&(second.len() as u32).to_le_bytes());
        bytes[row_two + 8..row_two + 12].copy_from_slice(&2u32.to_le_bytes());
        bytes[body_start..body_start + first.len()].copy_from_slice(first);
        bytes[body_start + first.len()..].copy_from_slice(second);
        Source {
            bytes,
            largest_request: 0,
            read_calls: 0,
            max_transfer: None,
        }
    }

    fn many_pages(page_count: u32) -> Source {
        let table_start = 0x400usize;
        let body_start = table_start + page_count as usize * PAGE_ROW_BYTES as usize;
        let body_end = body_start + 1;
        let mut bytes = vec![0u8; body_end];
        bytes[..4].copy_from_slice(MAGIC);
        bytes[PAGE_COUNT_OFFSET as usize..PAGE_COUNT_OFFSET as usize + 4]
            .copy_from_slice(&page_count.to_le_bytes());
        bytes[PAGE_TABLE_POINTER_OFFSET as usize..PAGE_TABLE_POINTER_OFFSET as usize + 4]
            .copy_from_slice(&(table_start as u32).to_le_bytes());
        for index in 0..page_count as usize {
            let row_start = table_start + index * PAGE_ROW_BYTES as usize;
            let offset = if index == 0 { body_start } else { body_end };
            bytes[row_start..row_start + 4].copy_from_slice(&(offset as u32).to_le_bytes());
            bytes[row_start + 4..row_start + 8]
                .copy_from_slice(&u32::from(index == 0).to_le_bytes());
            bytes[row_start + 8..row_start + 12].copy_from_slice(&(index as u32 + 1).to_le_bytes());
        }
        Source {
            bytes,
            largest_request: 0,
            read_calls: 0,
            max_transfer: None,
        }
    }

    fn parse(source: &mut Source, limits: &Limits) -> Result<CajMetadata> {
        ready(parse_metadata(source, limits, &NeverCancel))
    }

    #[test]
    fn parses_page_order_and_outline_with_one_byte_reads() {
        let mut source = sample();
        let limits = Limits {
            io_chunk_bytes: 1,
            ..Limits::default()
        };
        let metadata = parse(&mut source, &limits).unwrap();
        assert_eq!(metadata.page_count, 2);
        assert_eq!(metadata.body_start, 0x418);
        assert_eq!(metadata.body_end_hint, source.size());
        assert_eq!(metadata.page_rows[0].page_object_id, 1);
        assert_eq!(metadata.page_rows[1].page_object_id, 2);
        assert_eq!(metadata.bookmarks[0].title, "Intro");
        assert_eq!(metadata.bookmarks[1].depth, 1);
        assert_eq!(metadata.bookmarks[1].page_index, 1);
        assert_eq!(source.largest_request, 1);
    }

    #[test]
    fn page_table_batches_reduce_ranged_round_trips() {
        let page_count = PAGE_TABLE_BATCH_ROWS as u32 * 2 + 1;
        let mut source = many_pages(page_count);
        let metadata = parse(&mut source, &Limits::default()).unwrap();
        assert_eq!(metadata.page_count, page_count);
        assert_eq!(metadata.page_rows.len(), page_count as usize);
        assert_eq!(metadata.page_rows[0].page_object_id, 1);
        assert_eq!(
            metadata.page_rows.last().unwrap().page_object_id,
            page_count
        );
        assert_eq!(metadata.body_end_hint, source.size());
        // Three fixed header reads and three bounded table batches.
        assert_eq!(source.read_calls, 6);
        assert_eq!(source.largest_request, PAGE_TABLE_BATCH_BYTES);
    }

    #[test]
    fn page_table_batches_handle_short_reads_and_locate_boundary_errors() {
        let page_count = PAGE_TABLE_BATCH_ROWS as u32 + 1;
        let limits = Limits {
            io_chunk_bytes: 17,
            ..Limits::default()
        };
        let mut source = many_pages(page_count);
        source.max_transfer = Some(5);
        let metadata = parse(&mut source, &limits).unwrap();
        assert_eq!(
            metadata.page_rows.last().unwrap().page_object_id,
            page_count
        );
        assert_eq!(source.largest_request, limits.io_chunk_bytes);
        assert!(source.read_calls > page_count as usize);

        let mut malformed = many_pages(page_count);
        malformed.max_transfer = Some(5);
        let last_id_offset = 0x400 + (page_count as usize - 1) * PAGE_ROW_BYTES as usize + 8;
        malformed.bytes[last_id_offset..last_id_offset + 4].fill(0);
        assert!(matches!(
            parse(&mut malformed, &limits),
            Err(Error::Caj {
                offset,
                record: Some(record),
                reason: "CAJ page object number must be positive",
            }) if offset == last_id_offset as u64 && record == page_count
        ));
    }

    #[test]
    fn accepts_zero_length_page_row_and_unknown_header_flags() {
        let mut source = sample();
        source.bytes[4..8].copy_from_slice(&[0, 0, 2, 0]);
        let table_start = 0x400usize;
        let source_size = source.size() as usize;
        source.bytes[table_start + 4..table_start + 8]
            .copy_from_slice(&((source_size - 0x418) as u32).to_le_bytes());
        source.bytes[table_start + 12..table_start + 16]
            .copy_from_slice(&(source_size as u32).to_le_bytes());
        source.bytes[table_start + 16..table_start + 20].copy_from_slice(&0u32.to_le_bytes());
        let metadata = parse(&mut source, &Limits::default()).unwrap();
        assert_eq!(metadata.page_rows[1].length, 0);
    }

    #[test]
    fn decodes_two_and_four_byte_gb18030_titles() {
        let mut source = sample();
        let first = TOC_RECORDS_OFFSET as usize;
        source.bytes[first..first + 5].copy_from_slice(&[0xd5, 0xaa, 0xd2, 0xaa, 0]);
        let second = first + TOC_RECORD_BYTES as usize;
        source.bytes[second..second + 5].copy_from_slice(&[0x94, 0x39, 0xfc, 0x36, 0]);
        let metadata = parse(&mut source, &Limits::default()).unwrap();
        assert_eq!(metadata.bookmarks[0].title, "摘要");
        assert_eq!(metadata.bookmarks[1].title, "😀");
    }

    #[test]
    fn rejects_truncation_overlap_and_discontiguous_rows() {
        let mut short = sample();
        short.bytes.truncate(0x114 + 100);
        assert!(matches!(
            parse(&mut short, &Limits::default()),
            Err(Error::Caj {
                record: Some(1),
                ..
            })
        ));

        let mut overlap = sample();
        overlap.bytes[0x14..0x18].copy_from_slice(&0x150u32.to_le_bytes());
        assert!(matches!(
            parse(&mut overlap, &Limits::default()),
            Err(Error::Caj { offset: 0x14, .. })
        ));

        let mut gap = sample();
        gap.bytes[0x40c..0x410].copy_from_slice(&0x43au32.to_le_bytes());
        assert!(matches!(
            parse(&mut gap, &Limits::default()),
            Err(Error::Caj {
                record: Some(2),
                reason: "CAJ page spans are not contiguous",
                ..
            })
        ));
    }

    #[test]
    fn rejects_duplicate_pages_bad_titles_and_invalid_destinations() {
        let mut duplicate = sample();
        duplicate.bytes[0x414..0x418].copy_from_slice(&1u32.to_le_bytes());
        assert!(matches!(
            parse(&mut duplicate, &Limits::default()),
            Err(Error::Caj {
                record: Some(2),
                reason: "duplicate CAJ page object number",
                ..
            })
        ));

        let mut title = sample();
        title.bytes[0x114] = 0x81;
        title.bytes[0x115] = 0;
        assert!(matches!(
            parse(&mut title, &Limits::default()),
            Err(Error::Caj {
                offset: 0x114,
                record: Some(1),
                ..
            })
        ));

        let mut destination = sample();
        destination.bytes[0x114 + 280] = b'3';
        assert!(matches!(
            parse(&mut destination, &Limits::default()),
            Err(Error::Caj {
                offset: 0x22c,
                record: Some(1),
                ..
            })
        ));
    }

    #[test]
    fn rejects_level_jumps_and_locates_limits() {
        let mut level = sample();
        let second_level = (TOC_RECORDS_OFFSET + TOC_RECORD_BYTES + 304) as usize;
        level.bytes[second_level..second_level + 4].copy_from_slice(&3u32.to_le_bytes());
        assert!(matches!(
            parse(&mut level, &Limits::default()),
            Err(Error::Caj {
                record: Some(2),
                reason: "CAJ TOC level skips a parent",
                ..
            })
        ));

        let mut pages = sample();
        let limits = Limits {
            max_pages: 1,
            ..Limits::default()
        };
        assert!(matches!(
            parse(&mut pages, &limits),
            Err(Error::CajLimitExceeded {
                offset: 0x10,
                attempted: 2,
                ..
            })
        ));

        let mut allocation = sample();
        let limits = Limits {
            io_chunk_bytes: 1,
            max_allocation_bytes: 32,
            ..Limits::default()
        };
        assert!(matches!(
            parse(&mut allocation, &limits),
            Err(Error::CajLimitExceeded {
                resource: "CAJ page metadata",
                ..
            })
        ));
    }

    #[test]
    fn malformed_header_and_page_table_report_the_field_location() {
        let mut wrong_magic = sample();
        wrong_magic.bytes[0] = b'X';
        assert!(matches!(
            parse(&mut wrong_magic, &Limits::default()),
            Err(Error::UnsupportedFormat)
        ));

        for count in [0i32, -1] {
            let mut invalid_count = sample();
            invalid_count.bytes[0x10..0x14].copy_from_slice(&count.to_le_bytes());
            assert!(matches!(
                parse(&mut invalid_count, &Limits::default()),
                Err(Error::Caj {
                    offset: 0x10,
                    reason: "CAJ page count must be positive",
                    ..
                })
            ));
        }

        let mut truncated_table = sample();
        truncated_table.bytes[0x14..0x18].copy_from_slice(&0x500u32.to_le_bytes());
        assert!(matches!(
            parse(&mut truncated_table, &Limits::default()),
            Err(Error::Caj {
                offset: 0x14,
                reason: "CAJ page table extends beyond source",
                ..
            })
        ));

        let mut zero_id = sample();
        zero_id.bytes[0x408..0x40c].fill(0);
        assert!(matches!(
            parse(&mut zero_id, &Limits::default()),
            Err(Error::Caj {
                offset: 0x408,
                record: Some(1),
                ..
            })
        ));

        let mut overlapping_body = sample();
        overlapping_body.bytes[0x400..0x404].copy_from_slice(&0x400u32.to_le_bytes());
        assert!(matches!(
            parse(&mut overlapping_body, &Limits::default()),
            Err(Error::Caj {
                offset: 0x400,
                record: Some(1),
                reason: "CAJ PDF body overlaps the page table"
            })
        ));

        let mut outside_body = sample();
        outside_body.bytes[0x404..0x408].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            parse(&mut outside_body, &Limits::default()),
            Err(Error::Caj {
                offset: 0x404,
                record: Some(1),
                reason: "CAJ page span extends beyond source"
            })
        ));

        let mut empty_body = sample();
        empty_body.bytes[0x404..0x408].fill(0);
        empty_body.bytes[0x40c..0x410].copy_from_slice(&0x418u32.to_le_bytes());
        empty_body.bytes[0x410..0x414].fill(0);
        assert!(matches!(
            parse(&mut empty_body, &Limits::default()),
            Err(Error::Caj {
                offset: 0x400,
                reason: "CAJ PDF body is empty",
                ..
            })
        ));
    }

    #[test]
    fn malformed_toc_numbers_and_levels_report_the_record() {
        let mut negative_count = sample();
        negative_count.bytes[0x110..0x114].copy_from_slice(&(-1i32).to_le_bytes());
        assert!(matches!(
            parse(&mut negative_count, &Limits::default()),
            Err(Error::Caj {
                offset: 0x110,
                reason: "CAJ TOC count is negative",
                ..
            })
        ));

        let mut empty_title = sample();
        empty_title.bytes[0x114] = 0;
        assert!(matches!(
            parse(&mut empty_title, &Limits::default()),
            Err(Error::Caj {
                offset: 0x114,
                record: Some(1),
                reason: "empty CAJ TOC title"
            })
        ));

        let mut empty_page = sample();
        empty_page.bytes[0x22c] = b' ';
        assert!(matches!(
            parse(&mut empty_page, &Limits::default()),
            Err(Error::Caj {
                offset: 0x22c,
                record: Some(1),
                reason: "empty TOC page number"
            })
        ));

        let mut invalid_digit = sample();
        invalid_digit.bytes[0x22c] = b'a';
        assert!(matches!(
            parse(&mut invalid_digit, &Limits::default()),
            Err(Error::Caj {
                offset: 0x22c,
                record: Some(1),
                reason: "TOC page number is not ASCII decimal"
            })
        ));

        let mut number_overflow = sample();
        number_overflow.bytes[0x22c..0x238].copy_from_slice(b"429496729599");
        assert!(matches!(
            parse(&mut number_overflow, &Limits::default()),
            Err(Error::Caj {
                record: Some(1),
                reason: "TOC page number overflows",
                ..
            })
        ));

        let mut zero_level = sample();
        zero_level.bytes[0x244..0x248].fill(0);
        assert!(matches!(
            parse(&mut zero_level, &Limits::default()),
            Err(Error::Caj {
                offset: 0x244,
                record: Some(1),
                reason: "CAJ TOC level must be positive"
            })
        ));
    }

    #[test]
    fn metadata_limits_cover_toc_count_body_length_and_title_expansion() {
        let mut toc_count = sample();
        let bookmark_limit = Limits {
            max_bookmarks: 1,
            ..Limits::default()
        };
        assert!(matches!(
            parse(&mut toc_count, &bookmark_limit),
            Err(Error::CajLimitExceeded {
                offset: 0x110,
                resource: "CAJ bookmarks",
                attempted: 2,
                ..
            })
        ));

        let mut body = sample();
        let body_limit = Limits {
            io_chunk_bytes: 1,
            max_input_bytes: 1,
            ..Limits::default()
        };
        assert!(matches!(
            parse(&mut body, &body_limit),
            Err(Error::CajLimitExceeded {
                offset: 0x418,
                resource: "CAJ PDF input bytes",
                ..
            })
        ));

        let mut title = sample();
        title.bytes[0x114..0x114 + 80].fill(b'A');
        title.bytes[0x114 + 80] = 0;
        let title_limit = Limits {
            io_chunk_bytes: 1,
            max_allocation_bytes: 100,
            ..Limits::default()
        };
        assert!(matches!(
            parse(&mut title, &title_limit),
            Err(Error::CajLimitExceeded {
                offset: 0x114,
                record: Some(1),
                resource: "CAJ title allocation bytes",
                ..
            })
        ));
    }
}
