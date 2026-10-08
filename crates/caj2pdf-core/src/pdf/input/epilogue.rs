// SPDX-License-Identifier: MIT

//! A measured obsolete xref followed by an encoded copy of the CAJ header.
//! No xref offset is used as an object boundary. The redundant suffix must
//! match the original header and end at a pending stream's exact Length object.

use super::fragment_scan::Pass;
use super::parser::{Syntax, exact_reference, exact_unsigned, is_space};
use super::{ObjectTail, Reader, XrefKind, parse_xref_entry};
use crate::{Cancellation, RangedSource, Result, read_exact_at};

const MAX_EPILOGUE_BYTES: usize = 4096;
const MAX_XREF_ENTRIES: u64 = 128;
const HEADER_COPY_START: u64 = 144;
const MIN_HEADER_COPY: usize = 128;
const MAX_HEADER_COPY: usize = 64 * 1024;
const XOR_CYCLE: &[u8; 6] = b"FZHMEI";

pub(super) fn end<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    pass: &Pass<'_>,
    start: u64,
) -> Result<Option<u64>> {
    let remaining = reader.range.length - start;
    if remaining < 5 || reader.bytes(start, 5)? != b"xref\r" {
        return Ok(None);
    }
    let count = remaining
        .min(MAX_EPILOGUE_BYTES as u64)
        .min(reader.syntax_limit());
    let bytes = reader.bytes(start, count as usize)?;
    let Some(copy_start) = syntax_end(&bytes) else {
        return Ok(None);
    };
    let copy_start = start + copy_start as u64;
    let mut header = [0; 24];
    if reader.source.size() < header.len() as u64 {
        return Ok(None);
    }
    read_at(reader, 0, &mut header)?;
    if header[..4] != *b"CAJ\0" {
        return Ok(None);
    }
    let table = u64::from(u32::from_le_bytes(header[20..24].try_into().unwrap()));
    let mut first_row = [0; 12];
    if table < HEADER_COPY_START || table + 12 > reader.range.offset {
        return Ok(None);
    }
    read_at(reader, table, &mut first_row)?;
    let body_start = u64::from(u32::from_le_bytes(first_row[..4].try_into().unwrap()));
    if body_start <= table || body_start > reader.range.offset {
        return Ok(None);
    }
    let bound = (table - HEADER_COPY_START)
        .min(reader.range.length - copy_start)
        .min(MAX_HEADER_COPY as u64) as usize;
    if bound < MIN_HEADER_COPY {
        return Ok(None);
    }
    // Only the observed phase is admitted. Every byte must match, so no
    // interpretation of the copied header fields or bookmark text is needed.
    let matched = matched_header(reader, copy_start, bound)?;
    if matched < MIN_HEADER_COPY || matched == bound {
        return Ok(None);
    }
    let after_copy = copy_start + matched as u64;
    if reader.range.length - after_copy < 2 || reader.bytes(after_copy, 2)? != b"\r\n" {
        return Ok(None);
    }
    let next = after_copy + 2;
    let head = match reader.load_head(next, None) {
        Ok(head) => head,
        Err(error) if error.is_malformed_pdf() => return Ok(None),
        Err(error) => return Err(error),
    };
    let Some(value) = head
        .scalar
        .as_ref()
        .and_then(|span| exact_unsigned(&head.bytes[span.clone()]))
    else {
        return Ok(None);
    };
    if !matches!(head.tail, ObjectTail::EndObject { .. })
        || head.reference.generation != 0
        || pass.lengths.contains_key(&head.reference)
        || !pass
            .pending_lengths
            .iter()
            .any(|pending| pending.target == head.reference && pending.length == value)
    {
        return Ok(None);
    }
    // Recheck both the small framing metadata and the complete copy, which
    // can exceed the initial syntax window, before omitting either one.
    let mut current_header = [0; 24];
    let mut current_row = [0; 12];
    read_at(reader, 0, &mut current_header)?;
    read_at(reader, table, &mut current_row)?;
    if current_header != header
        || current_row != first_row
        || reader.bytes(start, bytes.len())? != bytes
        || matched_header(reader, copy_start, matched)? != matched
    {
        return Err(reader.malformed(start, None, "CAJ epilogue changed while reading"));
    }
    Ok(Some(next))
}

fn matched_header<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    start: u64,
    bound: usize,
) -> Result<usize> {
    let mut original = [0; 256];
    let mut matched = 0;
    while matched < bound {
        let count = (bound - matched)
            .min(original.len())
            .min(reader.limits.io_chunk_bytes);
        read_at(
            reader,
            HEADER_COPY_START + matched as u64,
            &mut original[..count],
        )?;
        let encoded = reader.bytes(start + matched as u64, count)?;
        let equal = original[..count]
            .iter()
            .zip(&encoded)
            .enumerate()
            .take_while(|(i, (plain, encoded))| {
                **plain == (**encoded ^ XOR_CYCLE[(matched + i + 2) % 6])
            })
            .count();
        matched += equal;
        if equal != count {
            break;
        }
    }
    Ok(matched)
}

/// Header bytes are outside the fragment range, but retain the same bounded
/// short-read and cancellation contract as reads inside it.
fn read_at<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    mut offset: u64,
    destination: &mut [u8],
) -> Result<()> {
    for chunk in destination.chunks_mut(reader.limits.io_chunk_bytes) {
        read_exact_at(
            reader.source,
            offset,
            chunk,
            reader.limits,
            reader.cancellation,
        )?;
        offset += chunk.len() as u64;
    }
    Ok(())
}

/// Return the byte after the exact EOF/CR, without following old offsets.
fn syntax_end(bytes: &[u8]) -> Option<usize> {
    let mut s = Syntax::new(bytes);
    if !s.consume_keyword(b"xref").ok()? {
        return None;
    }
    let mut previous_end = 0;
    let mut total = 0_u64;
    loop {
        s.skip_space();
        if s.consume_keyword(b"trailer").ok()? {
            break;
        }
        let first = s.unsigned().ok()?;
        let count = s.unsigned().ok()?;
        let end = first.checked_add(count)?;
        if total == 0 && first != 0 {
            return None;
        }
        total = total.checked_add(count)?;
        if count == 0 || first < previous_end || total > MAX_XREF_ENTRIES || end > 8_388_607 {
            return None;
        }
        s.skip_space();
        for index in first..end {
            let entry = parse_xref_entry(bytes.get(s.pos..s.pos + 20)?)?;
            if index == 0 {
                if total != count
                    || first != 0
                    || count != 1
                    || entry.generation != 65535
                    || !matches!(entry.kind, XrefKind::Free)
                {
                    return None;
                }
            } else if !matches!(entry.kind, XrefKind::InUse(_)) || entry.generation > 1 {
                return None;
            }
            s.pos += 20;
        }
        previous_end = end;
    }
    if total == 0 {
        return None;
    }
    s.skip_space();
    let entries = s.dictionary(0).ok()?;
    if entries.len() != 5 {
        return None;
    }
    let get = |name: &[u8]| {
        entries
            .iter()
            .find(|e| e.name() == name)
            .map(|e| e.value(bytes))
    };
    if exact_unsigned(get(b"Size")?)? != previous_end
        || exact_unsigned(get(b"Prev")?)? == 0
        || exact_reference(get(b"Root")?)?.generation != 0
        || exact_reference(get(b"Info")?)?.generation != 0
        || !id_pair(get(b"ID")?)
    {
        return None;
    }
    s.skip_space();
    if !s.consume_keyword(b"startxref").ok()? || s.unsigned().ok()? == 0 {
        return None;
    }
    while bytes.get(s.pos).copied().is_some_and(is_space) {
        s.pos += 1;
    }
    (bytes.get(s.pos..s.pos + 6)? == b"%%EOF\r").then_some(s.pos + 6)
}

fn id_pair(bytes: &[u8]) -> bool {
    let mut compact = bytes.iter().copied().filter(|b| !is_space(*b));
    if compact.next() != Some(b'[') {
        return false;
    }
    for _ in 0..2 {
        if compact.next() != Some(b'<') {
            return false;
        }
        for _ in 0..32 {
            if !compact.next().is_some_and(|b| b.is_ascii_hexdigit()) {
                return false;
            }
        }
        if compact.next() != Some(b'>') {
            return false;
        }
    }
    compact.next() == Some(b']') && compact.next().is_none()
}
