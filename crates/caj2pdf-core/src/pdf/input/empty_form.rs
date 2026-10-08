// SPDX-License-Identifier: MIT

//! A measured empty Form serialized inside an equivalent empty Form wrapper.
//! Both headers declare zero content. CAJ fragments keep the identical inner
//! object. Indexed PDFs retain the outer identity through an appended empty
//! revision, requiring the different inner ID's live xref to point elsewhere.

use super::fragment_scan::StreamFailure;
use super::parser::{Dictionary, ObjectHead, Syntax, exact_name, exact_unsigned, is_space};
use super::{GapPatch, ObjectLocation, ObjectTail, PdfIndex, Reader, XrefKind, XrefSlot};
use crate::fallible::{push_bounded, reserve_exact};
use crate::pdf::{FragmentObject, PdfRange, PdfRef};
use crate::{Cancellation, Error, RangedSource, Result};

const MAX_HEAD: u64 = 256;
const MAX_SPACE: usize = 64;
// Two tails, each with two bounded whitespace gaps and two keywords, plus
// one delimiter lookahead. No search through arbitrary stream payloads.
const TAIL_WINDOW: usize = 2 * (2 * MAX_SPACE + 15) + 1;

pub(super) fn candidate<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    start: u64,
    stream: &StreamFailure,
) -> Result<Option<(FragmentObject, u64)>> {
    if stream.length != Some(0)
        || stream.direct.is_none()
        || stream.reference.generation != 0
        || stream.inspection.is_err()
        || stream.data_start > MAX_HEAD
    {
        return Ok(None);
    }
    let Some(outer) = head(reader, start)? else {
        return Ok(None);
    };
    if !matches!(outer.tail, ObjectTail::Stream { data_start } if data_start as u64 == stream.data_start)
        || outer.reference != stream.reference
    {
        return Ok(None);
    }
    let Some(a) = profile(&outer) else {
        return Ok(None);
    };
    // The inner object starts exactly at the outer data boundary: no gap,
    // partial content, scan for a likely header, or recursively nested case.
    let inner_at = start + stream.data_start;
    let Some(inner) = head(reader, inner_at)? else {
        return Ok(None);
    };
    if !inner.bytes.first().is_some_and(u8::is_ascii_digit) {
        return Ok(None);
    }
    let Some(b) = profile(&inner) else {
        return Ok(None);
    };
    if inner.reference != outer.reference || a != b {
        return Ok(None);
    }
    let ObjectTail::Stream { data_start } = inner.tail else {
        return Ok(None);
    };
    let data_at = inner_at + data_start as u64;
    let tails = reader.bytes(
        data_at,
        (reader.range.length - data_at).min(TAIL_WINDOW as u64) as usize,
    )?;
    let Some(inner_end) = tail_end(&tails, 0) else {
        return Ok(None);
    };
    let Some(outer_end) = tail_end(&tails, inner_end) else {
        return Ok(None);
    };
    if reader.bytes(start, stream.data_start as usize)? != outer.bytes[..stream.data_start as usize]
        || reader.bytes(inner_at, data_start)? != inner.bytes[..data_start]
        || reader.bytes(data_at, outer_end)? != tails[..outer_end]
    {
        return Err(reader.malformed(
            start,
            Some(stream.reference),
            "nested empty Form changed while reading",
        ));
    }
    Ok(Some((
        FragmentObject {
            reference: inner.reference,
            range: PdfRange {
                offset: reader.absolute(inner_at),
                length: data_start as u64 + inner_end as u64,
            },
        },
        data_at + outer_end as u64,
    )))
}

fn head<S: RangedSource, C: Cancellation>(
    reader: &mut Reader<'_, S, C>,
    at: u64,
) -> Result<Option<ObjectHead>> {
    let range = PdfRange {
        offset: reader.absolute(at),
        length: (reader.range.length - at).min(MAX_HEAD),
    };
    // Reuse the parser and allocation checks, with an independent
    // 256-byte range so a malformed lookalike cannot grow the header probe.
    let mut bounded = Reader::new(reader.source, range, reader.limits, reader.cancellation)?;
    match bounded.load_head(0, None) {
        Ok(head) => Ok(Some(head)),
        Err(error) if error.is_malformed_pdf() => Ok(None),
        Err(error) => Err(error),
    }
}

#[derive(PartialEq, Eq)]
struct Geometry<'a> {
    bbox: [&'a [u8]; 4],
    matrix: [&'a [u8]; 6],
}

fn profile(head: &ObjectHead) -> Option<Geometry<'_>> {
    if !matches!(head.tail, ObjectTail::Stream { .. }) || head.reference.generation != 0 {
        return None;
    }
    let d = head.dictionary.as_ref()?;
    if d.entries.len() != 5
        || d.value(b"Type").and_then(exact_name).as_deref() != Some(b"XObject")
        || d.value(b"Subtype").and_then(exact_name).as_deref() != Some(b"Form")
        || d.value(b"Length").and_then(exact_unsigned) != Some(0)
    {
        return None;
    }
    Some(Geometry {
        bbox: numbers(d, b"BBox")?,
        matrix: numbers(d, b"Matrix")?,
    })
}

fn numbers<'a, const N: usize>(d: &'a Dictionary, key: &[u8]) -> Option<[&'a [u8]; N]> {
    let value = d.value(key)?.strip_prefix(b"[")?.strip_suffix(b"]")?;
    let mut tokens = value.split(|b| is_space(*b)).filter(|v| !v.is_empty());
    let mut result = [&[][..]; N];
    for slot in &mut result {
        let token = tokens.next()?;
        // The ordinary parser already checked number grammar. Exclude names,
        // references, comments and other array values; compare exact tokens,
        // never rounded floating-point values or whitespace-stripped digits.
        if !token
            .iter()
            .all(|b| b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.'))
        {
            return None;
        }
        *slot = token;
    }
    tokens.next().is_none().then_some(result)
}

fn tail_end(bytes: &[u8], at: usize) -> Option<usize> {
    let mut s = Syntax::new(bytes);
    s.pos = at;
    for keyword in [b"endstream".as_slice(), b"endobj"] {
        let start = s.pos;
        while bytes.get(s.pos).is_some_and(|b| is_space(*b)) {
            s.pos += 1;
            if s.pos - start > MAX_SPACE {
                return None;
            }
        }
        if s.consume_keyword(keyword) != Ok(true) {
            return None;
        }
    }
    Some(s.pos)
}

impl<S: RangedSource, C: Cancellation> Reader<'_, S, C> {
    pub(super) fn recover_indexed_empty_form(
        &mut self,
        at: u64,
        reference: PdfRef,
        slots: &[Option<XrefSlot>],
        index: &mut PdfIndex,
    ) -> Result<Option<(ObjectHead, ObjectLocation)>> {
        let Some(outer) = head(self, at)? else {
            return Ok(None);
        };
        let Some(a) = profile(&outer) else {
            return Ok(None);
        };
        if outer.reference != reference {
            return Ok(None);
        }
        let ObjectTail::Stream { data_start } = outer.tail else {
            return Ok(None);
        };
        let inner_at = at + data_start as u64;
        let Some(inner) = head(self, inner_at)? else {
            return Ok(None);
        };
        let Some(b) = profile(&inner) else {
            return Ok(None);
        };
        if !inner.bytes.first().is_some_and(u8::is_ascii_digit)
            || inner.reference == reference
            || !a
                .bbox
                .into_iter()
                .chain(a.matrix)
                .zip(b.bbox.into_iter().chain(b.matrix))
                .all(|(a, b)| trim_decimal_zeros(a) == trim_decimal_zeros(b))
        {
            return Ok(None);
        }
        let ObjectTail::Stream {
            data_start: inner_data,
        } = inner.tail
        else {
            return Ok(None);
        };
        let data_at = inner_at + inner_data as u64;
        let tails = self.bytes(
            data_at,
            (self.range.length - data_at).min(TAIL_WINDOW as u64) as usize,
        )?;
        let Some(inner_end) = tail_end(&tails, 0) else {
            return Ok(None);
        };
        let Some(outer_end) = tail_end(&tails, inner_end) else {
            return Ok(None);
        };
        let end = data_at + outer_end as u64;
        // The inner label is not a replacement identity. Its actual live
        // object must be elsewhere; the ordinary sorted span validation also
        // rejects ANY other live xref inside this entire outer frame.
        if !matches!(slots.get(inner.reference.number as usize).and_then(|slot| *slot),
            Some(XrefSlot { generation: 0, kind: XrefKind::InUse(live) }) if live < at || live >= end)
        {
            return Ok(None);
        }
        let length = end - at;
        let cap = (64 * 1024).min(self.limits.max_allocation_bytes / 8);
        let retained = index.retained_empty_form_bytes.saturating_add(length);
        if retained > cap {
            return Err(self.locate_limit(
                at,
                Some(reference),
                Error::limit("PDF empty Form proof bytes", cap, retained),
            ));
        }
        let original = self.bytes(at, length as usize)?;
        if original[..data_start] != outer.bytes[..data_start]
            || original[data_start..data_start + inner_data] != inner.bytes[..inner_data]
            || original[data_start + inner_data..] != tails[..outer_end]
        {
            return Err(self.malformed(
                at,
                Some(reference),
                "nested empty Form changed while reading",
            ));
        }
        // Keep the same dictionary serializer, repair budget and append-only
        // writer as the existing indexed repairs. This is an empty stream,
        // never a copied or decoded content payload.
        self.push_repair_body(
            outer.dictionary.as_ref().unwrap(),
            reference,
            at,
            |_, _| true,
            b"",
            &mut index.repair_objects,
            &mut index.retained_repair_bytes,
        )?;
        let suffix = b"\nstream\n\nendstream";
        let retained_body = index
            .retained_repair_bytes
            .saturating_add(suffix.len() as u64);
        let body_cap = self.limits.max_allocation_bytes / 2;
        if retained_body > body_cap {
            return Err(self.locate_limit(
                at,
                Some(reference),
                Error::limit("PDF repair object bytes", body_cap, retained_body),
            ));
        }
        // validate_objects visits each live ID once, so this repair is last.
        let body = &mut index.repair_objects.last_mut().unwrap().body;
        reserve_exact(
            body,
            suffix.len(),
            self.allocation_limit(
                at,
                Some(reference),
                "PDF empty Form repair allocation",
                body.len() as u64 + suffix.len() as u64,
            ),
        )?;
        body.extend_from_slice(suffix);
        index.retained_repair_bytes = retained_body;
        push_bounded(
            &mut index.empty_form_checks,
            GapPatch {
                offset: at,
                original,
            },
            cap,
            "PDF empty Form proof index",
        )
        .map_err(self.locator(at, Some(reference)))?;
        index.retained_empty_form_bytes = retained;
        Ok(Some((outer, ObjectLocation { offset: at, length })))
    }
}

/// Normalize only redundant fractional zeros. Keep signs and integer digits
/// exact; do not broaden CAJ's lexical comparison or round long decimals.
fn trim_decimal_zeros(mut token: &[u8]) -> &[u8] {
    if token.contains(&b'.') {
        while token.last() == Some(&b'0') {
            token = &token[..token.len() - 1];
        }
        if token.last() == Some(&b'.') {
            token = &token[..token.len() - 1];
        }
    }
    token
}

#[cfg(test)]
mod indexed_tests;
#[cfg(test)]
mod tests;
