// SPDX-License-Identifier: MIT

//! Measured unused interruptions. Every proposed omission is deferred
//! until the complete scan proves that no object references the missing ID.

use super::fragment_scan::{Pass, StreamFailure};
use super::parser::{Syntax, exact_name, exact_reference, exact_unsigned};
use super::recovery::shared_prefix;
use super::{ObjectTail, Reader};
use crate::pdf::{FragmentObject, PdfRange, PdfRef};
use crate::{Cancellation, Error, RangedSource, Result};

const MAX_HEADER: usize = 256;
const MIN_IMAGE_PREFIX: u64 = 256;
const MAX_IMAGE_PREFIX: u64 = 64 * 1024;
const MAX_IMAGE_CANDIDATES: usize = 64;

pub(super) fn opener<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    start: u64,
    error: &Error,
) -> Result<Option<(u64, FragmentObject)>> {
    let flate = match error.reason {
        "invalid PDF hexadecimal string" => false,
        "expected PDF name" => true,
        _ => return Ok(None),
    };
    let bound = if flate { MAX_HEADER as u64 } else { 64 };
    let bytes = reader.bytes(start, (reader.range.length - start).min(bound) as usize)?;
    let parsed = if flate {
        flate_declaration_prefix(&bytes)
    } else {
        opener_prefix(&bytes)
    };
    let Some((reference, split)) = parsed else {
        return Ok(None);
    };
    let next = start + split as u64;
    let head = match reader.load_head(next, None) {
        Ok(head) => head,
        Err(error) if error.is_malformed_pdf() => return Ok(None),
        Err(error) => return Err(error),
    };
    if head.reference == reference
        || head.reference.generation != 0
        || if flate {
            !matches!(head.tail, ObjectTail::Stream { data_start } if data_start <= MAX_HEADER)
        } else {
            !matches!(head.tail, ObjectTail::EndObject { end } if end <= MAX_HEADER)
        }
    {
        return Ok(None);
    }
    if reader.bytes(start, split)? != bytes[..split] {
        return Err(reader.malformed(
            start,
            Some(reference),
            "unused opener changed while reading",
        ));
    }
    Ok(Some((
        next,
        FragmentObject {
            reference,
            range: PdfRange {
                offset: reader.range.offset + start,
                length: split as u64 - 2,
            },
        },
    )))
}

fn opener_prefix(bytes: &[u8]) -> Option<(PdfRef, usize)> {
    let mut s = Syntax::new(bytes);
    let number = u32::try_from(s.unsigned().ok()?).ok()?;
    if number == 0 || s.unsigned().ok()? != 0 {
        return None;
    }
    s.skip_space();
    if !s.consume_keyword(b"obj").ok()? {
        return None;
    }
    s.skip_space();
    if bytes.get(s.pos..s.pos + 3)? != b"<\r\n" {
        return None;
    }
    Some((
        PdfRef {
            number,
            generation: 0,
        },
        s.pos + 3,
    ))
}

/// The measured declaration stops inside the FlateDecode name, before any
/// stream keyword or payload. Its missing ID must have no incoming reference
/// in the complete graph; this does not authorize dropping arbitrary values.
fn flate_declaration_prefix(bytes: &[u8]) -> Option<(PdfRef, usize)> {
    let mut s = Syntax::new(bytes);
    let number = u32::try_from(s.unsigned().ok()?).ok()?;
    if number == 0 || s.unsigned().ok()? != 0 {
        return None;
    }
    s.skip_space();
    if !s.consume_keyword(b"obj").ok()? {
        return None;
    }
    s.skip_space();
    if !s.consume_keyword(b"<<").ok()? {
        return None;
    }
    s.skip_space();
    if !s.consume_keyword(b"/Length").ok()? || s.unsigned().ok()? == 0 {
        return None;
    }
    s.skip_space();
    if !s.consume_keyword(b"/Filter").ok()? {
        return None;
    }
    s.skip_space();
    let suffix = b"/FlateDecod\r\n";
    if !bytes.get(s.pos..)?.starts_with(suffix) {
        return None;
    }
    Some((
        PdfRef {
            number,
            generation: 0,
        },
        s.pos + suffix.len(),
    ))
}

pub(super) fn image_prefix<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    pass: &mut Pass<'_>,
    start: u64,
    stream: &StreamFailure,
) -> Result<Option<(u64, FragmentObject)>> {
    let Some(length) = stream.length else {
        return Ok(None);
    };
    if stream.direct.is_some() || length <= MIN_IMAGE_PREFIX || pass.candidates.is_empty() {
        return Ok(None);
    }
    let head = reader.load_head(start, Some(stream.reference))?;
    if stream.data_start > MAX_HEADER as u64
        || !matches!(head.tail, ObjectTail::Stream { data_start } if data_start as u64 == stream.data_start)
    {
        return Ok(None);
    }
    let Some(dictionary) = head.dictionary.as_ref() else {
        return Ok(None);
    };
    if !dictionary
        .value(b"Length")
        .and_then(exact_reference)
        .is_some_and(|r| r.generation == 0 && pass.lengths.get(&r) == Some(&length))
        || dictionary.entries.len() != 9
        || dictionary.value(b"Type").and_then(exact_name).as_deref() != Some(b"XObject")
        || dictionary.value(b"Subtype").and_then(exact_name).as_deref() != Some(b"Image")
        || dictionary.value(b"Filter").and_then(exact_name).as_deref() != Some(b"FlateDecode")
        || dictionary
            .value(b"ColorSpace")
            .and_then(exact_name)
            .as_deref()
            != Some(b"DeviceRGB")
        || dictionary
            .value(b"BitsPerComponent")
            .and_then(exact_unsigned)
            != Some(8)
        || dictionary.value(b"Name").and_then(exact_name).is_none()
        || !dictionary
            .value(b"Width")
            .and_then(exact_unsigned)
            .is_some_and(|n| n > 0)
        || !dictionary
            .value(b"Height")
            .and_then(exact_unsigned)
            .is_some_and(|n| n > 0)
    {
        return Ok(None);
    }
    let data_at = start + stream.data_start;
    let mut inspected = 0;
    let mut found = None;
    for index in 0..pass.candidates.len() {
        if reader.cancellation.is_cancelled() {
            return Err(crate::ErrorKind::Cancelled.into());
        }
        let candidate = pass.candidates[index].object;
        if candidate.reference == stream.reference
            || candidate.reference.generation != 0
            || candidate.range.offset <= reader.range.offset + data_at
            || candidate.range.length <= length
            || candidate.range.length - length > (MAX_HEADER + 96) as u64
        {
            continue;
        }
        inspected += 1;
        if inspected > MAX_IMAGE_CANDIDATES {
            return Ok(None);
        }
        let Some(at) = candidate.range.offset.checked_sub(reader.range.offset) else {
            continue;
        };
        if candidate.range.length > reader.range.length.saturating_sub(at) {
            continue;
        }
        let other = reader.load_head(at, Some(candidate.reference))?;
        let ObjectTail::Stream { data_start } = other.tail else {
            continue;
        };
        if data_start > MAX_HEADER {
            continue;
        }
        let Some(other_dict) = other.dictionary.as_ref() else {
            continue;
        };
        if other_dict.entries.len() != dictionary.entries.len()
            || !other_dict
                .value(b"Length")
                .and_then(exact_reference)
                .is_some_and(|r| r.generation == 0)
            || dictionary.entries.iter().any(|e| {
                e.name() != b"Length"
                    && other_dict.value(e.name()) != Some(e.value(&dictionary.bytes))
            })
        {
            continue;
        }
        let other_data = at + data_start as u64;
        let Some(after) = other_data.checked_add(length) else {
            continue;
        };
        match reader.check_stream_tail(after, Some(candidate.reference)) {
            Ok(end) if end == at + candidate.range.length => {}
            Ok(_) => continue,
            Err(error) if error.is_malformed_pdf() => continue,
            Err(error) => return Err(error),
        }
        let bound = length
            .min(MAX_IMAGE_PREFIX)
            .min(reader.range.length - data_at);
        let matched = shared_prefix(reader, data_at, other_data, bound)?;
        if matched < MIN_IMAGE_PREFIX || matched == bound {
            continue;
        }
        let boundary = data_at + matched;
        if reader.range.length - boundary < 2 || reader.bytes(boundary, 2)? != b"\r\n" {
            continue;
        }
        let next = boundary + 2;
        let integer = match reader.load_head(next, None) {
            Ok(head) => head,
            Err(error) if error.is_malformed_pdf() => continue,
            Err(error) => return Err(error),
        };
        let Some(value) = integer
            .scalar
            .as_ref()
            .and_then(|span| exact_unsigned(&integer.bytes[span.clone()]))
        else {
            continue;
        };
        if !matches!(integer.tail, ObjectTail::EndObject { .. })
            || integer.reference.generation != 0
            || pass.lengths.contains_key(&integer.reference)
            || !pass
                .pending_lengths
                .iter()
                .any(|p| p.target == integer.reference && p.length == value)
        {
            continue;
        }
        if reader.bytes(start, stream.data_start as usize)?
            != head.bytes[..stream.data_start as usize]
            || reader.bytes(at, data_start)? != other.bytes[..data_start]
            || shared_prefix(reader, data_at, other_data, matched)? != matched
        {
            return Err(reader.malformed(
                start,
                Some(stream.reference),
                "unused image prefix changed while reading",
            ));
        }
        if found.replace((index, next, boundary)).is_some() {
            return Ok(None);
        }
    }
    let Some((index, next, boundary)) = found else {
        return Ok(None);
    };
    pass.candidates[index].used = true;
    Ok(Some((
        next,
        FragmentObject {
            reference: stream.reference,
            range: PdfRange {
                offset: reader.range.offset + start,
                length: boundary - start,
            },
        },
    )))
}
