// SPDX-License-Identifier: MIT

//! Checked, ranged PDF input for classic tables and bounded xref streams.
//!
//! This module stores object positions and page references, never page or
//! stream payloads. All offsets in `PdfIndex` are relative to `PdfRange`;
//! diagnostics use absolute source offsets.

mod link_repair;
mod parser;

pub(crate) use link_repair::{
    LinkDestinationTarget, LinkRepairCandidate, LinkRepairKind, inspect_link_destination_candidate,
};

pub use parser::DictEntry;

use super::FragmentObject;
use super::types::{PdfRange, PdfRef};
use super::writer::MAX_PDF_OBJECTS;
use crate::error::PdfErrorKind;
use crate::fallible::{len_u64, reserve_exact};
use crate::{Cancellation, Error, Limits, RangedSource, Result, read_exact_at};
use flate2::{Decompress, FlushDecompress, Status};
use parser::{
    Dictionary, ObjectHead, ObjectTail, Syntax, destination_page, exact_name, exact_reference,
    exact_unsigned, media_box, parse_object_head, reference_array, unsigned_array, valid_id_array,
    valid_text_string,
};
use std::cmp::min;

const WINDOW_BYTES: usize = 8 * 1024;
const MAX_TAIL_SEARCH: u64 = 64 * 1024;
const MAX_OBJECT_SYNTAX: u64 = 4 * 1024 * 1024;
const MAX_XREF_SECTIONS: usize = 64;
const MAX_XREF_INDEX_VALUES: usize = 8192;
const MAX_XREF_STREAM_BYTES: u64 = 4 * 1024 * 1024;
const MAX_ORPHAN_GAP_BYTES: u64 = 64;
const MAX_ORPHAN_GAP_TOTAL: u64 = 64 * 1024;

/// A complete indirect object in a PDF input, relative to the PDF range.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectLocation {
    pub offset: u64,
    pub length: u64,
}

/// A structural object body that can replace a known malformed original.
#[derive(Clone, Debug)]
pub struct RepairObject {
    pub reference: PdfRef,
    /// A complete direct PDF value, excluding `n g obj` and `endobj`.
    pub body: Vec<u8>,
}

/// A short inactive object prefix, retained for source-stability checking
/// before its bytes are replaced with equal-length whitespace.
pub struct GapPatch {
    pub offset: u64,
    pub original: Vec<u8>,
}

/// Validated metadata retained from a PDF input.
pub struct PdfIndex {
    range: PdfRange,
    logical_end: u64,
    xref_offset: u64,
    trailer_size: u32,
    trailer_info: Option<PdfRef>,
    trailer_id: Option<Vec<u8>>,
    catalog: PdfRef,
    catalog_dict: Dictionary,
    pages: Vec<PdfRef>,
    has_outlines: bool,
    object_locations: Vec<Option<(u16, ObjectLocation)>>,
    repair_objects: Vec<RepairObject>,
    retained_repair_bytes: u64,
    stale_page_parents: Vec<(PdfRef, PdfRef)>,
    stream_separator_patches: Vec<u64>,
    gap_patches: Vec<GapPatch>,
    retained_gap_bytes: u64,
    max_referenced_object: u32,
}

impl PdfIndex {
    pub fn range(&self) -> PdfRange {
        self.range
    }
    pub fn logical_end(&self) -> u64 {
        self.logical_end
    }
    pub fn xref_offset(&self) -> u64 {
        self.xref_offset
    }
    pub fn trailer_size(&self) -> u32 {
        self.trailer_size
    }
    pub fn trailer_info(&self) -> Option<PdfRef> {
        self.trailer_info
    }
    pub fn trailer_id(&self) -> Option<&[u8]> {
        self.trailer_id.as_deref()
    }
    pub fn catalog(&self) -> PdfRef {
        self.catalog
    }
    pub fn catalog_dictionary(&self) -> &[u8] {
        &self.catalog_dict.bytes
    }
    pub fn catalog_entries(&self) -> &[DictEntry] {
        &self.catalog_dict.entries
    }
    pub fn pages(&self) -> &[PdfRef] {
        &self.pages
    }
    pub fn has_outlines(&self) -> bool {
        self.has_outlines
    }
    pub fn repair_objects(&self) -> &[RepairObject] {
        &self.repair_objects
    }
    pub fn stream_separator_patches(&self) -> &[u64] {
        &self.stream_separator_patches
    }
    pub fn gap_patches(&self) -> &[GapPatch] {
        &self.gap_patches
    }
    pub fn max_referenced_object(&self) -> u32 {
        self.max_referenced_object
    }

    pub fn next_free_object_number(&self) -> Result<u32> {
        let next = self
            .trailer_size
            .max(self.max_referenced_object.saturating_add(1));
        if next > MAX_PDF_OBJECTS {
            Err(Error::PdfLimitExceeded {
                offset: self.range.offset.saturating_add(self.xref_offset),
                object: None,
                resource: "PDF object number",
                limit: u64::from(MAX_PDF_OBJECTS),
                attempted: u64::from(next),
            })
        } else {
            Ok(next)
        }
    }

    pub fn object_location(&self, reference: PdfRef) -> Result<ObjectLocation> {
        let slot = self
            .object_locations
            .get(reference.number as usize)
            .and_then(|slot| *slot);
        match slot {
            Some((generation, location)) if generation == reference.generation => Ok(location),
            _ => Err(Error::Pdf {
                offset: self.range.offset,
                object: Some((reference.number, reference.generation)),
                kind: PdfErrorKind::Malformed,
                reason: "PDF reference does not resolve to a live object",
            }),
        }
    }

    /// Open, validate, and index a PDF without retaining its content streams.
    pub async fn open<S: RangedSource, C: Cancellation>(
        source: &mut S,
        range: PdfRange,
        limits: &Limits,
        cancellation: &C,
    ) -> Result<Self> {
        limits.validate()?;
        limits
            .check_input_size(range.length)
            .map_err(|error| error.locate_pdf_limit(range.offset, None))?;
        let end = range.end().ok_or(Error::InvalidInput {
            reason: "PDF source range overflows",
        })?;
        if end > source.size() {
            return Err(Error::TruncatedInput {
                offset: range.offset,
                expected: range.length,
                available: source.size().saturating_sub(range.offset),
            });
        }
        let mut reader = Reader::new(source, range, limits, cancellation)?;
        reader.check_header().await?;
        let (xref_offset, logical_end) = reader.find_tail().await?;
        let (slots, trailer) = reader.read_xref_chain(xref_offset).await?;
        let location_bytes = slots
            .len()
            .checked_mul(std::mem::size_of::<Option<(u16, ObjectLocation)>>())
            .ok_or(Error::InvalidInput {
                reason: "PDF object location index size overflows",
            })?;
        let total_index_bytes = location_bytes
            .checked_add(
                slots
                    .len()
                    .saturating_mul(std::mem::size_of::<Option<XrefSlot>>()),
            )
            .ok_or(Error::InvalidInput {
                reason: "PDF combined object index size overflows",
            })?;
        limits
            .check_allocation(total_index_bytes as u64)
            .map_err(|error| reader.locate_limit(xref_offset, None, error))?;
        let mut object_locations = Vec::new();
        let refused = reader.allocation_limit(
            xref_offset,
            None,
            "PDF object location index allocation",
            location_bytes as u64,
        );
        reserve_exact(&mut object_locations, slots.len(), refused)?;
        object_locations.resize(slots.len(), None);
        let mut index = Self {
            range,
            logical_end,
            xref_offset,
            trailer_size: trailer.size,
            trailer_info: trailer.info,
            trailer_id: trailer.id,
            catalog: trailer.root,
            catalog_dict: Dictionary {
                bytes: Vec::new(),
                entries: Vec::new(),
            },
            pages: Vec::new(),
            has_outlines: false,
            object_locations,
            repair_objects: Vec::new(),
            retained_repair_bytes: 0,
            stale_page_parents: Vec::new(),
            stream_separator_patches: Vec::new(),
            gap_patches: Vec::new(),
            retained_gap_bytes: 0,
            max_referenced_object: 0,
        };
        reader.validate_objects(&slots, &mut index).await?;
        index.stream_separator_patches.sort_unstable();
        reader.read_structure(&slots, &mut index).await?;
        if let Some((reference, _)) = index.stale_page_parents.first() {
            let location = index.object_location(*reference)?;
            return Err(reader.malformed(
                location.offset,
                Some(*reference),
                "stale page Parent is not reachable through validated Kids",
            ));
        }
        reader
            .validate_live_object_spans(&mut index, &slots, trailer.prev.is_none())
            .await?;
        Ok(index)
    }
}

#[derive(Clone, Copy, Debug)]
enum XrefKind {
    Free,
    InUse(u64),
}

#[derive(Clone, Copy, Debug)]
struct XrefSlot {
    generation: u16,
    kind: XrefKind,
}

#[derive(Debug)]
struct XrefRecord {
    number: u32,
    slot: XrefSlot,
}

#[derive(Clone, Copy)]
enum PageVisit {
    Enter {
        reference: PdfRef,
        parent: Option<PdfRef>,
        inherited_media_box: bool,
    },
    Exit {
        reference: PdfRef,
        declared_count: u32,
        first_leaf: usize,
    },
}

#[derive(Clone, Copy)]
struct OutlineVisit {
    reference: PdfRef,
    parent: PdfRef,
    previous: Option<PdfRef>,
    expected_last: PdfRef,
}

#[derive(Debug)]
struct Trailer {
    size: u32,
    root: PdfRef,
    info: Option<PdfRef>,
    id: Option<Vec<u8>>,
    prev: Option<u64>,
}

struct Reader<'a, S, C> {
    source: &'a mut S,
    range: PdfRange,
    limits: &'a Limits,
    cancellation: &'a C,
    window: Vec<u8>,
    window_offset: u64,
    window_len: usize,
}

impl<'a, S: RangedSource, C: Cancellation> Reader<'a, S, C> {
    fn syntax_limit(&self) -> u64 {
        MAX_OBJECT_SYNTAX
            .min(self.limits.max_allocation_bytes / 32)
            .max(1)
    }

    fn new(
        source: &'a mut S,
        range: PdfRange,
        limits: &'a Limits,
        cancellation: &'a C,
    ) -> Result<Self> {
        let size = min(WINDOW_BYTES, limits.io_chunk_bytes);
        limits.check_allocation(size as u64)?;
        Ok(Self {
            source,
            range,
            limits,
            cancellation,
            window: vec![0; size],
            window_offset: 0,
            window_len: 0,
        })
    }

    fn absolute(&self, relative: u64) -> u64 {
        self.range.offset.saturating_add(relative)
    }

    fn problem(
        &self,
        relative: u64,
        object: Option<PdfRef>,
        kind: PdfErrorKind,
        reason: &'static str,
    ) -> Error {
        Error::Pdf {
            offset: self.absolute(relative),
            object: object.map(|item| (item.number, item.generation)),
            kind,
            reason,
        }
    }

    fn malformed(&self, relative: u64, object: Option<PdfRef>, reason: &'static str) -> Error {
        self.problem(relative, object, PdfErrorKind::Malformed, reason)
    }

    fn locate_limit(&self, relative: u64, object: Option<PdfRef>, error: Error) -> Error {
        error.locate_pdf_limit(
            self.absolute(relative),
            object.map(|item| (item.number, item.generation)),
        )
    }

    /// A located error for a failed reservation within the allocation limit.
    fn allocation_limit(
        &self,
        relative: u64,
        object: Option<PdfRef>,
        resource: &'static str,
        attempted: u64,
    ) -> Error {
        self.locate_limit(
            relative,
            object,
            self.limits.allocation_refused(resource, attempted),
        )
    }

    fn parse_issue(
        &self,
        relative: u64,
        object: Option<PdfRef>,
        issue: parser::ParseIssue,
    ) -> Error {
        if let Some((resource, limit, attempted)) = issue.limit {
            Error::PdfLimitExceeded {
                offset: self.absolute(relative + issue.at as u64),
                object: object.map(|item| (item.number, item.generation)),
                resource,
                limit,
                attempted,
            }
        } else {
            self.problem(
                relative + issue.at as u64,
                object,
                if issue.ambiguous {
                    PdfErrorKind::AmbiguousRepair
                } else {
                    PdfErrorKind::Malformed
                },
                issue.reason,
            )
        }
    }

    async fn byte(&mut self, position: u64) -> Result<Option<u8>> {
        if position >= self.range.length {
            return Ok(None);
        }
        if self.window_len != 0
            && position >= self.window_offset
            && position - self.window_offset < self.window_len as u64
        {
            return Ok(Some(self.window[(position - self.window_offset) as usize]));
        }
        let amount = min(self.window.len() as u64, self.range.length - position) as usize;
        read_exact_at(
            self.source,
            self.absolute(position),
            &mut self.window[..amount],
            self.limits,
            self.cancellation,
        )
        .await?;
        self.window_offset = position;
        self.window_len = amount;
        Ok(Some(self.window[0]))
    }

    async fn bytes(&mut self, position: u64, length: usize) -> Result<Vec<u8>> {
        let length_u64 = len_u64(length);
        if position
            .checked_add(length_u64)
            .is_none_or(|end| end > self.range.length)
        {
            return Err(self.malformed(position, None, "PDF range ends inside required syntax"));
        }
        self.limits
            .check_allocation(length_u64)
            .map_err(|error| self.locate_limit(position, None, error))?;
        let mut result = Vec::new();
        let refused =
            self.allocation_limit(position, None, "PDF read buffer allocation", length_u64);
        reserve_exact(&mut result, length, refused)?;
        result.resize(length, 0);
        let mut done = 0;
        while done < length {
            let count = min(length - done, self.limits.io_chunk_bytes);
            let relative = position + done as u64;
            read_exact_at(
                self.source,
                self.absolute(relative),
                &mut result[done..done + count],
                self.limits,
                self.cancellation,
            )
            .await?;
            done += count;
        }
        Ok(result)
    }

    async fn check_header(&mut self) -> Result<()> {
        if self.range.length < 8 {
            return Err(self.malformed(0, None, "PDF header is truncated"));
        }
        let header = self.bytes(0, 8).await?;
        if header.starts_with(b"%PDF-2.") && header[7].is_ascii_digit() {
            return Err(self.problem(
                0,
                None,
                PdfErrorKind::UnsupportedFeature,
                "PDF 2.x is outside the supported input profile",
            ));
        }
        if !header.starts_with(b"%PDF-1.") || !matches!(header[7], b'0'..=b'7') {
            return Err(self.malformed(0, None, "PDF 1.0 through 1.7 header is required"));
        }
        Ok(())
    }

    async fn find_tail(&mut self) -> Result<(u64, u64)> {
        let take = min(self.range.length, MAX_TAIL_SEARCH) as usize;
        let start = self.range.length - take as u64;
        let tail = self.bytes(start, take).await?;
        for eof_at in (0..=tail.len().saturating_sub(5)).rev() {
            if tail.get(eof_at..eof_at + 5) != Some(b"%%EOF".as_slice()) {
                continue;
            }
            if eof_at > 0 && !matches!(tail[eof_at - 1], b'\r' | b'\n') {
                continue;
            }
            let lookback = eof_at.saturating_sub(128);
            let Some(marker_at) = tail[lookback..eof_at]
                .windows(9)
                .rposition(|part| part == b"startxref")
            else {
                continue;
            };
            let mut cursor = lookback + marker_at + 9;
            while cursor < eof_at && matches!(tail[cursor], b' ' | b'\t' | b'\r' | b'\n') {
                cursor += 1;
            }
            let first = cursor;
            let mut offset = 0_u64;
            while cursor < eof_at && tail[cursor].is_ascii_digit() {
                let failure =
                    self.malformed(start + cursor as u64, None, "startxref offset overflows");
                offset = offset
                    .checked_mul(10)
                    .and_then(|value| value.checked_add(u64::from(tail[cursor] - b'0')))
                    .ok_or(failure)?;
                cursor += 1;
            }
            if cursor == first || offset >= start + eof_at as u64 {
                continue;
            }
            if !tail[cursor..eof_at]
                .iter()
                .all(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
            {
                continue;
            }
            let mut logical_end = start + eof_at as u64 + 5;
            while logical_end < self.range.length
                && matches!(
                    self.byte(logical_end).await?,
                    Some(b' ' | b'\t' | b'\r' | b'\n' | 0 | 12)
                )
            {
                logical_end += 1;
            }
            if logical_end < self.range.length {
                let suffix = &tail[(logical_end - start) as usize..];
                let known_caj_footer = (suffix.starts_with(b"WebFastLoadP")
                    || suffix.starts_with(b"WebFastLoadW"))
                    && ![
                        b"startxref".as_slice(),
                        b"%%EOF",
                        b"xref",
                        b"trailer",
                        b"obj",
                    ]
                    .iter()
                    .any(|marker| suffix.windows(marker.len()).any(|window| window == *marker));
                if !known_caj_footer {
                    return Err(self.problem(
                        logical_end,
                        None,
                        PdfErrorKind::AmbiguousRepair,
                        "bytes after PDF EOF are not a recognized CAJ footer",
                    ));
                }
            }
            return Ok((offset, logical_end));
        }
        Err(self.malformed(
            self.range.length.saturating_sub(take as u64),
            None,
            "PDF startxref and EOF were not found in bounded tail",
        ))
    }

    async fn skip_space(&mut self, cursor: &mut u64) -> Result<()> {
        loop {
            match self.byte(*cursor).await? {
                Some(b' ' | b'\t' | b'\r' | b'\n' | 0 | 12) => *cursor += 1,
                Some(b'%') => {
                    while let Some(byte) = self.byte(*cursor).await? {
                        *cursor += 1;
                        if byte == b'\r' || byte == b'\n' {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
        Ok(())
    }

    async fn word(&mut self, cursor: &mut u64, maximum: usize) -> Result<Vec<u8>> {
        self.skip_space(cursor).await?;
        let start = *cursor;
        let mut result = Vec::new();
        while let Some(byte) = self.byte(*cursor).await? {
            if matches!(
                byte,
                b' ' | b'\t' | b'\r' | b'\n' | 0 | 12 | b'<' | b'>' | b'[' | b']' | b'/' | b'%'
            ) {
                break;
            }
            if result.len() == maximum {
                return Err(self.malformed(start, None, "PDF token is too long"));
            }
            result.push(byte);
            *cursor += 1;
        }
        if result.is_empty() {
            return Err(self.malformed(start, None, "expected PDF token"));
        }
        Ok(result)
    }

    async fn unsigned(&mut self, cursor: &mut u64) -> Result<u64> {
        let start = *cursor;
        let word = self.word(cursor, 20).await?;
        if !word.iter().all(u8::is_ascii_digit) {
            return Err(self.malformed(start, None, "expected nonnegative PDF integer"));
        }
        let mut value = 0_u64;
        for digit in word {
            value = value
                .checked_mul(10)
                .and_then(|n| n.checked_add(u64::from(digit - b'0')))
                .ok_or(self.malformed(start, None, "PDF integer overflows"))?;
        }
        Ok(value)
    }

    async fn dictionary_at(&mut self, at: u64, maximum: u64) -> Result<(Dictionary, u64)> {
        let mut amount = min(512, min(maximum, self.range.length.saturating_sub(at))) as usize;
        if amount == 0 {
            return Err(self.malformed(at, None, "PDF dictionary is truncated"));
        }
        loop {
            let bytes = self.bytes(at, amount).await?;
            let mut parser = Syntax::new(&bytes);
            parser.skip_space();
            let start = parser.pos;
            match parser.dictionary(0) {
                Ok(mut entries) => {
                    let end = parser.pos;
                    for entry in &mut entries {
                        entry.pair.start -= start;
                        entry.pair.end -= start;
                        entry.value.start -= start;
                        entry.value.end -= start;
                    }
                    self.limits
                        .check_allocation(
                            (end - start) as u64
                                + (entries.len() * std::mem::size_of::<DictEntry>()) as u64,
                        )
                        .map_err(|error| self.locate_limit(at, None, error))?;
                    let body = bytes[start..end].to_vec();
                    return Ok((
                        Dictionary {
                            bytes: body,
                            entries,
                        },
                        at + end as u64,
                    ));
                }
                Err(issue)
                    if issue.incomplete
                        && (amount as u64) < maximum
                        && at + (amount as u64) < self.range.length =>
                {
                    amount = min(
                        amount.saturating_mul(2),
                        min(maximum, self.range.length - at) as usize,
                    );
                }
                Err(issue) if issue.incomplete && maximum == self.syntax_limit() => {
                    return Err(self.locate_limit(
                        at,
                        None,
                        Error::LimitExceeded {
                            resource: "PDF dictionary syntax bytes",
                            limit: maximum,
                            attempted: maximum.saturating_add(1),
                        },
                    ));
                }
                Err(issue) => {
                    return Err(self.parse_issue(at, None, issue));
                }
            }
        }
    }

    async fn read_xref_chain(&mut self, latest: u64) -> Result<(Vec<Option<XrefSlot>>, Trailer)> {
        let mut cursor = latest;
        let mut slots: Vec<Option<XrefSlot>> = Vec::new();
        let mut latest_trailer = None;
        // Each Prev must point strictly before its section, so the chain
        // cannot revisit a section and needs no cycle check.
        for _ in 0..MAX_XREF_SECTIONS {
            let (records, trailer) = self.read_xref_section(cursor).await?;
            if slots.is_empty() {
                let slots_len = trailer.size as usize;
                let bytes = slots_len
                    .checked_mul(std::mem::size_of::<Option<XrefSlot>>())
                    .ok_or(Error::InvalidInput {
                        reason: "PDF xref index size overflows",
                    })?;
                self.limits
                    .check_allocation(bytes as u64)
                    .map_err(|error| self.locate_limit(cursor, None, error))?;
                let refused =
                    self.allocation_limit(cursor, None, "PDF xref index allocation", bytes as u64);
                reserve_exact(&mut slots, slots_len, refused)?;
                slots.resize(slots_len, None);
                latest_trailer = Some(Trailer {
                    size: trailer.size,
                    root: trailer.root,
                    info: trailer.info,
                    id: trailer.id.clone(),
                    prev: trailer.prev,
                });
            }
            for record in records {
                let failure = self.malformed(cursor, None, "xref object exceeds trailer Size");
                let place = slots.get_mut(record.number as usize).ok_or(failure)?;
                if place.is_none() {
                    *place = Some(record.slot);
                }
            }
            if let Some(previous) = trailer.prev {
                if previous >= cursor {
                    return Err(self.malformed(
                        previous,
                        None,
                        "xref Prev must point to an earlier section",
                    ));
                }
                cursor = previous;
            } else {
                let failure = self.malformed(cursor, None, "PDF xref has no trailer");
                let final_trailer = latest_trailer.ok_or(failure)?;
                return Ok((slots, final_trailer));
            }
        }
        Err(self.malformed(cursor, None, "PDF xref revision limit exceeded"))
    }

    async fn read_xref_section(&mut self, at: u64) -> Result<(Vec<XrefRecord>, Trailer)> {
        let mut cursor = at;
        if self.bytes(cursor, 4).await?.as_slice() != b"xref" {
            return self.read_xref_stream(at).await;
        }
        cursor += 4;
        let mut records = Vec::new();
        loop {
            self.skip_space(&mut cursor).await?;
            if self.bytes(cursor, 7).await?.as_slice() == b"trailer" {
                cursor += 7;
                self.skip_space(&mut cursor).await?;
                let (dictionary, _) = self.dictionary_at(cursor, self.syntax_limit()).await?;
                let trailer = self.parse_trailer(&dictionary, cursor)?;
                records.sort_unstable_by_key(|record: &XrefRecord| record.number);
                if records
                    .windows(2)
                    .any(|pair| pair[0].number == pair[1].number)
                {
                    return Err(self.malformed(at, None, "duplicate xref entry in one revision"));
                }
                return Ok((records, trailer));
            }
            let subsection_at = cursor;
            let start = self.unsigned(&mut cursor).await?;
            let count = self.unsigned(&mut cursor).await?;
            if count == 0 || start.checked_add(count).is_none() {
                return Err(self.malformed(
                    subsection_at,
                    None,
                    "invalid PDF xref subsection range",
                ));
            }
            if start + count > u64::from(MAX_PDF_OBJECTS) + 1 {
                return Err(self.locate_limit(
                    subsection_at,
                    None,
                    Error::LimitExceeded {
                        resource: "PDF object index",
                        limit: u64::from(MAX_PDF_OBJECTS) + 1,
                        attempted: start + count,
                    },
                ));
            }
            self.skip_space(&mut cursor).await?;
            for step in 0..count {
                let number = (start + step) as u32;
                let line = self.bytes(cursor, 20).await?;
                let failure = self.malformed(cursor, None, "invalid fixed-width xref entry");
                let slot = parse_xref_entry(&line).ok_or(failure)?;
                if let XrefKind::InUse(offset) = slot.kind {
                    if offset >= self.range.length {
                        return Err(self.malformed(
                            cursor,
                            Some(PdfRef {
                                number,
                                generation: slot.generation,
                            }),
                            "xref object offset exceeds PDF range",
                        ));
                    }
                }
                push_bounded(
                    &mut records,
                    XrefRecord { number, slot },
                    self.limits.max_allocation_bytes,
                    "PDF xref records",
                )
                .map_err(|error| self.locate_limit(cursor, None, error))?;
                cursor += 20;
            }
        }
    }

    async fn read_xref_stream(&mut self, at: u64) -> Result<(Vec<XrefRecord>, Trailer)> {
        let head = self.load_head(at, None).await?;
        let reference = head.reference;
        let failure = self.malformed(at, Some(reference), "xref stream lacks a dictionary");
        let dictionary = head.dictionary.as_ref().ok_or(failure)?;
        if reference.generation != 0
            || dictionary.value(b"Type").and_then(exact_name).as_deref() != Some(b"XRef")
        {
            return Err(self.malformed(
                at,
                Some(reference),
                "startxref does not point to an xref table or stream",
            ));
        }
        let ObjectTail::Stream { data_start } = head.tail else {
            return Err(self.malformed(at, Some(reference), "xref object is not a stream"));
        };
        let trailer = self.parse_trailer(dictionary, at)?;
        let widths = dictionary
            .value(b"W")
            .and_then(|value| unsigned_array(value, 3))
            .filter(|values| values.len() == 3 && values.iter().all(|width| *width <= 8))
            .ok_or(self.malformed(at, Some(reference), "xref stream has invalid W"))?;
        let row_width = widths.iter().sum::<u64>();
        if row_width == 0 {
            return Err(self.malformed(at, Some(reference), "xref stream has empty W"));
        }
        let indices = match dictionary.value(b"Index") {
            Some(value) => unsigned_array(value, MAX_XREF_INDEX_VALUES),
            None => Some(vec![0, u64::from(trailer.size)]),
        }
        .filter(|values| !values.is_empty() && values.len() % 2 == 0)
        .ok_or(self.malformed(at, Some(reference), "xref stream has invalid Index"))?;
        let mut rows = 0_u64;
        let mut previous_end = 0_u64;
        for pair in indices.chunks_exact(2) {
            let [start, count] = [pair[0], pair[1]];
            let failure = self.malformed(at, Some(reference), "xref Index range overflows");
            let end = start.checked_add(count).ok_or(failure)?;
            if count == 0 || start < previous_end || end > u64::from(trailer.size) {
                return Err(self.malformed(
                    at,
                    Some(reference),
                    "xref Index ranges overlap or exceed Size",
                ));
            }
            previous_end = end;
            let failure = self.malformed(at, Some(reference), "xref Index row count overflows");
            rows = rows.checked_add(count).ok_or(failure)?;
        }
        let failure = self.malformed(at, Some(reference), "xref stream decoded size overflows");
        let decoded_len = rows.checked_mul(row_width).ok_or(failure)?;
        // Encoded and decoded buffers may coexist during inflation. Keep their
        // combined ceiling within one quarter of the configured allocation cap.
        let cap = MAX_XREF_STREAM_BYTES.min(self.limits.max_allocation_bytes / 8);
        if decoded_len.saturating_add(1) > cap {
            return Err(self.locate_limit(
                at,
                Some(reference),
                Error::LimitExceeded {
                    resource: "PDF xref decoded bytes",
                    limit: cap,
                    attempted: decoded_len.saturating_add(1),
                },
            ));
        }
        let length = dictionary
            .value(b"Length")
            .and_then(exact_unsigned)
            .ok_or_else(|| {
                self.problem(
                    at,
                    Some(reference),
                    PdfErrorKind::UnsupportedFeature,
                    "xref stream requires a direct Length",
                )
            })?;
        if length > cap {
            return Err(self.locate_limit(
                at,
                Some(reference),
                Error::LimitExceeded {
                    resource: "PDF xref encoded bytes",
                    limit: cap,
                    attempted: length,
                },
            ));
        }
        if dictionary.value(b"DecodeParms").is_some() || dictionary.value(b"F").is_some() {
            return Err(self.problem(
                at,
                Some(reference),
                PdfErrorKind::UnsupportedFeature,
                "xref stream decode parameters are unsupported",
            ));
        }
        let filter = dictionary
            .value(b"Filter")
            .map(exact_name)
            .transpose_option()
            .ok_or_else(|| {
                self.problem(
                    at,
                    Some(reference),
                    PdfErrorKind::UnsupportedFeature,
                    "xref stream filter is unsupported",
                )
            })?;
        if filter.as_deref().is_some_and(|name| name != b"FlateDecode") {
            return Err(self.problem(
                at,
                Some(reference),
                PdfErrorKind::UnsupportedFeature,
                "xref stream filter is unsupported",
            ));
        }
        let failure = self.malformed(at, Some(reference), "xref stream offset overflows");
        let data_at = at.checked_add(data_start as u64).ok_or(failure)?;
        let failure = self.malformed(data_at, Some(reference), "xref stream length overflows");
        let after_data = data_at.checked_add(length).ok_or(failure)?;
        self.check_stream_tail(after_data, Some(reference)).await?;
        let encoded = self.bytes(data_at, length as usize).await?;
        let decoded = if filter.is_some() {
            match inflate_xref(
                &encoded,
                decoded_len as usize,
                self.limits.io_chunk_bytes,
                self.cancellation,
            ) {
                Ok(bytes) => bytes,
                Err(InflateXrefError::TooLong) => {
                    return Err(self.locate_limit(
                        data_at,
                        Some(reference),
                        Error::LimitExceeded {
                            resource: "PDF xref decoded bytes",
                            limit: decoded_len,
                            attempted: decoded_len + 1,
                        },
                    ));
                }
                Err(InflateXrefError::Malformed) => {
                    return Err(self.malformed(
                        data_at,
                        Some(reference),
                        "xref stream Flate data is invalid",
                    ));
                }
                Err(InflateXrefError::Cancelled) => return Err(Error::Cancelled),
            }
        } else {
            if encoded.len() != decoded_len as usize {
                return Err(self.malformed(
                    data_at,
                    Some(reference),
                    "xref stream length disagrees with W and Index",
                ));
            }
            encoded
        };
        let mut records = Vec::new();
        let mut position = 0;
        for pair in indices.chunks_exact(2) {
            for number in pair[0]..pair[0] + pair[1] {
                // A one-byte row can fit millions of entries in the bounded
                // stream, so check cancellation during row parsing as well.
                if records.len() % 1024 == 0 && self.cancellation.is_cancelled() {
                    return Err(Error::Cancelled);
                }
                // Exact decoded length was checked against the declared row
                // geometry, but still reject any future parser drift safely.
                let mut field = |width| {
                    read_be(&decoded, &mut position, width).ok_or(self.malformed(
                        data_at,
                        Some(reference),
                        "xref stream row is truncated",
                    ))
                };
                let kind = if widths[0] == 0 {
                    1
                } else {
                    field(widths[0] as usize)?
                };
                let field2 = field(widths[1] as usize)?;
                let field3 = field(widths[2] as usize)?;
                let kind = match kind {
                    0 => XrefKind::Free,
                    1 => {
                        if field2 >= self.range.length {
                            return Err(self.malformed(
                                data_at,
                                Some(reference),
                                "xref object offset exceeds PDF range",
                            ));
                        }
                        XrefKind::InUse(field2)
                    }
                    2 => {
                        return Err(self.problem(
                            data_at,
                            Some(reference),
                            PdfErrorKind::UnsupportedFeature,
                            "compressed PDF objects are unsupported",
                        ));
                    }
                    _ => {
                        return Err(self.problem(
                            data_at,
                            Some(reference),
                            PdfErrorKind::UnsupportedFeature,
                            "xref entry type is unsupported",
                        ));
                    }
                };
                let generation = u16::try_from(field3).map_err(|_| {
                    self.malformed(data_at, Some(reference), "xref generation exceeds 16 bits")
                })?;
                push_bounded(
                    &mut records,
                    XrefRecord {
                        number: number as u32,
                        slot: XrefSlot { generation, kind },
                    },
                    self.limits.max_allocation_bytes,
                    "PDF xref records",
                )
                .map_err(|error| self.locate_limit(data_at, Some(reference), error))?;
            }
        }
        let self_entry = records
            .iter()
            .find(|record| record.number == reference.number);
        if !self_entry.is_some_and(|record| matches!(record.slot, XrefSlot { generation: 0, kind: XrefKind::InUse(offset) } if offset == at)) {
            return Err(self.malformed(at, Some(reference), "xref stream has no valid self entry"));
        }
        Ok((records, trailer))
    }

    fn parse_trailer(&self, dictionary: &Dictionary, at: u64) -> Result<Trailer> {
        self.reject_duplicate_names(dictionary, at, None)?;
        let required = |name: &[u8], reason: &'static str| {
            dictionary
                .value(name)
                .ok_or(self.malformed(at, None, reason))
        };
        if dictionary.value(b"Encrypt").is_some() {
            return Err(self.problem(
                at,
                None,
                PdfErrorKind::Encrypted,
                "encrypted PDFs are unsupported",
            ));
        }
        if dictionary.value(b"XRefStm").is_some() {
            return Err(self.problem(
                at,
                None,
                PdfErrorKind::UnsupportedFeature,
                "hybrid xref streams are unsupported",
            ));
        }
        let size_raw = exact_unsigned(required(b"Size", "PDF trailer lacks Size")?)
            .ok_or(self.malformed(at, None, "invalid PDF trailer Size"))?;
        if size_raw == 0 {
            return Err(self.malformed(at, None, "invalid PDF trailer Size"));
        }
        if size_raw > u64::from(MAX_PDF_OBJECTS) + 1 {
            return Err(self.locate_limit(
                at,
                None,
                Error::LimitExceeded {
                    resource: "PDF object index",
                    limit: u64::from(MAX_PDF_OBJECTS) + 1,
                    attempted: size_raw,
                },
            ));
        }
        let size = size_raw as u32;
        let root = exact_reference(required(b"Root", "PDF trailer lacks Root")?)
            .ok_or(self.malformed(at, None, "invalid PDF trailer Root"))?;
        let info = dictionary
            .value(b"Info")
            .map(exact_reference)
            .transpose_option()
            .ok_or(self.malformed(at, None, "invalid PDF trailer Info"))?;
        let id = dictionary.value(b"ID").map(|value| value.to_vec());
        if id.as_deref().is_some_and(|value| !valid_id_array(value)) {
            return Err(self.malformed(at, None, "PDF trailer ID must be an array of two strings"));
        }
        let prev = dictionary
            .value(b"Prev")
            .map(exact_unsigned)
            .transpose_option()
            .ok_or(self.malformed(at, None, "invalid PDF trailer Prev"))?;
        Ok(Trailer {
            size,
            root,
            info,
            id,
            prev,
        })
    }

    fn reject_duplicate_names(
        &self,
        dictionary: &Dictionary,
        at: u64,
        object: Option<PdfRef>,
    ) -> Result<()> {
        let bytes = dictionary
            .entries
            .len()
            .checked_mul(std::mem::size_of::<usize>())
            .ok_or(Error::InvalidInput {
                reason: "PDF dictionary key index size overflows",
            })?;
        self.limits
            .check_allocation(bytes as u64)
            .map_err(|error| self.locate_limit(at, object, error))?;
        let mut keys = Vec::new();
        let refused = self.allocation_limit(at, object, "PDF dictionary key index", bytes as u64);
        reserve_exact(&mut keys, dictionary.entries.len(), refused)?;
        keys.extend(0..dictionary.entries.len());
        keys.sort_unstable_by(|left, right| {
            dictionary.entries[*left]
                .name
                .cmp(&dictionary.entries[*right].name)
        });
        if keys
            .windows(2)
            .any(|pair| dictionary.entries[pair[0]].name == dictionary.entries[pair[1]].name)
        {
            return Err(self.problem(
                at,
                object,
                PdfErrorKind::AmbiguousRepair,
                "duplicate PDF dictionary keys have undefined value",
            ));
        }
        Ok(())
    }

    async fn load_head(&mut self, at: u64, expected: Option<PdfRef>) -> Result<ObjectHead> {
        let maximum = min(self.syntax_limit(), self.range.length.saturating_sub(at));
        let mut amount = min(512, maximum) as usize;
        if amount == 0 {
            return Err(self.malformed(at, expected, "indirect object is truncated"));
        }
        loop {
            let bytes = self.bytes(at, amount).await?;
            match parse_object_head(bytes) {
                Ok(head) => {
                    if expected.is_some_and(|reference| reference != head.reference) {
                        return Err(self.malformed(
                            at,
                            expected,
                            "xref points to a different object header",
                        ));
                    }
                    let overhead = head.dictionary.as_ref().map_or(0, |dictionary| {
                        dictionary.bytes.len().saturating_add(
                            dictionary
                                .entries
                                .len()
                                .saturating_mul(std::mem::size_of::<DictEntry>()),
                        )
                    });
                    self.limits
                        .check_allocation(
                            (amount
                                + overhead
                                + head.references.len() * std::mem::size_of::<PdfRef>())
                                as u64,
                        )
                        .map_err(|error| self.locate_limit(at, expected, error))?;
                    return Ok(head);
                }
                Err(issue)
                    if (issue.incomplete || issue.at >= amount.saturating_sub(32))
                        && (amount as u64) < maximum =>
                {
                    amount = min(amount.saturating_mul(2), maximum as usize);
                }
                Err(issue) if issue.incomplete && maximum == self.syntax_limit() => {
                    return Err(self.locate_limit(
                        at,
                        expected,
                        Error::LimitExceeded {
                            resource: "PDF object syntax bytes",
                            limit: maximum,
                            attempted: maximum.saturating_add(1),
                        },
                    ));
                }
                Err(issue) => {
                    return Err(self.parse_issue(at, expected, issue));
                }
            }
        }
    }

    async fn load_object(
        &mut self,
        at: u64,
        expected: PdfRef,
        slots: &[Option<XrefSlot>],
    ) -> Result<(ObjectHead, ObjectLocation)> {
        let head = self.load_head(at, Some(expected)).await?;
        // `load_head` parsed at most `range.length - at` bytes, so offsets
        // within the head stay inside the range, and `check_stream_tail`
        // reads its `endobj` there too.
        let end = match head.tail {
            ObjectTail::EndObject { end } => at + end as u64,
            ObjectTail::Stream { data_start } => {
                let failure = self.malformed(at, Some(expected), "stream has no dictionary");
                let dictionary = head.dictionary.as_ref().ok_or(failure)?;
                let failure = self.malformed(at, Some(expected), "stream lacks Length");
                let length_value = dictionary.value(b"Length").ok_or(failure)?;
                let length = if let Some(value) = exact_unsigned(length_value) {
                    value
                } else if let Some(reference) = exact_reference(length_value) {
                    self.resolve_length(reference, slots).await?
                } else {
                    return Err(self.malformed(at, Some(expected), "invalid stream Length"));
                };
                let data_at = at + data_start as u64;
                let failure = self.malformed(data_at, Some(expected), "stream extent overflows");
                let after_data = data_at.checked_add(length).ok_or(failure)?;
                self.check_stream_tail(after_data, Some(expected)).await?
            }
        };
        debug_assert!(end <= self.range.length);
        Ok((
            head,
            ObjectLocation {
                offset: at,
                length: end - at,
            },
        ))
    }

    async fn resolve_length(
        &mut self,
        reference: PdfRef,
        slots: &[Option<XrefSlot>],
    ) -> Result<u64> {
        let offset = match slots.get(reference.number as usize).and_then(|slot| *slot) {
            Some(XrefSlot {
                generation,
                kind: XrefKind::InUse(offset),
            }) if generation == reference.generation => offset,
            _ => {
                return Err(self.malformed(
                    0,
                    Some(reference),
                    "indirect stream Length does not resolve",
                ));
            }
        };
        let head = self.load_head(offset, Some(reference)).await?;
        if !matches!(head.tail, ObjectTail::EndObject { .. }) {
            return Err(self.malformed(
                offset,
                Some(reference),
                "indirect stream Length is not an integer object",
            ));
        }
        let failure = self.malformed(
            offset,
            Some(reference),
            "indirect stream Length is not an integer",
        );
        let scalar = head.scalar.ok_or(failure)?;
        exact_unsigned(&head.bytes[scalar]).ok_or(self.malformed(
            offset,
            Some(reference),
            "invalid indirect stream Length",
        ))
    }

    async fn check_stream_tail(&mut self, after_data: u64, object: Option<PdfRef>) -> Result<u64> {
        let mut cursor = after_data;
        match self.byte(cursor).await? {
            Some(b'\r') => {
                cursor += 1;
                if self.byte(cursor).await? == Some(b'\n') {
                    cursor += 1;
                }
            }
            Some(b'\n') => cursor += 1,
            _ => {}
        }
        if self.bytes(cursor, 9).await?.as_slice() != b"endstream" {
            return Err(self.malformed(cursor, object, "stream Length does not end at endstream"));
        }
        cursor += 9;
        self.skip_space(&mut cursor).await?;
        if self.bytes(cursor, 6).await?.as_slice() != b"endobj" {
            return Err(self.malformed(cursor, object, "stream lacks endobj"));
        }
        Ok(cursor + 6)
    }

    async fn validate_objects(
        &mut self,
        slots: &[Option<XrefSlot>],
        index: &mut PdfIndex,
    ) -> Result<()> {
        for (number, slot) in slots.iter().enumerate().skip(1) {
            let Some(XrefSlot {
                generation,
                kind: XrefKind::InUse(offset),
            }) = slot
            else {
                continue;
            };
            let reference = PdfRef {
                number: number as u32,
                generation: *generation,
            };
            if *offset >= index.logical_end {
                return Err(self.malformed(
                    *offset,
                    Some(reference),
                    "live PDF object begins after logical EOF",
                ));
            }
            let (head, location) = self.load_object(*offset, reference, slots).await?;
            if location
                .offset
                .checked_add(location.length)
                .is_none_or(|end| end > index.logical_end)
            {
                return Err(self.malformed(
                    *offset,
                    Some(reference),
                    "live PDF object extends past logical EOF",
                ));
            }
            if let ObjectTail::Stream { data_start } = &head.tail {
                if *data_start > 0 && head.bytes[*data_start - 1] == b'\r' {
                    let failure = self.malformed(
                        *offset,
                        Some(reference),
                        "stream separator offset overflows",
                    );
                    let patch_at = offset.checked_add(*data_start as u64 - 1).ok_or(failure)?;
                    push_bounded(
                        &mut index.stream_separator_patches,
                        patch_at,
                        self.limits.max_allocation_bytes / 8,
                        "PDF stream separator patches",
                    )
                    .map_err(|error| self.locate_limit(patch_at, Some(reference), error))?;
                }
            }
            if let Some(dictionary) = &head.dictionary {
                let kind = dictionary
                    .value(b"Type")
                    .and_then(exact_name)
                    .unwrap_or_default();
                self.check_dictionary_duplicates(
                    dictionary,
                    reference,
                    *offset,
                    &kind,
                    &mut index.repair_objects,
                    &mut index.retained_repair_bytes,
                )?;
            }
            let stale_parent = head.dictionary.as_ref().and_then(|dictionary| {
                (dictionary.value(b"Type").and_then(exact_name).as_deref() == Some(b"Page")
                    && matches!(&head.tail, ObjectTail::EndObject { .. }))
                .then(|| dictionary.value(b"Parent").and_then(exact_reference))
                .flatten()
            });
            for target in &head.references {
                let live = slots.get(target.number as usize).and_then(|slot| *slot);
                if !matches!(live,Some(XrefSlot{generation,kind:XrefKind::InUse(_)}) if generation==target.generation)
                {
                    if matches!(
                        live,
                        Some(XrefSlot {
                            kind: XrefKind::Free,
                            ..
                        })
                    ) && stale_parent == Some(*target)
                        && head
                            .references
                            .iter()
                            .filter(|item| *item == target)
                            .count()
                            == 1
                    {
                        push_bounded(
                            &mut index.stale_page_parents,
                            (reference, *target),
                            self.limits.max_allocation_bytes / 8,
                            "PDF stale page parent candidates",
                        )
                        .map_err(|error| self.locate_limit(*offset, Some(reference), error))?;
                        continue;
                    }
                    return Err(self.malformed(
                        *offset,
                        Some(reference),
                        "PDF object contains a dangling indirect reference",
                    ));
                }
            }
            index.max_referenced_object = index
                .max_referenced_object
                .max(head.max_reference)
                .max(number as u32);
            index.object_locations[number] = Some((*generation, location));
        }
        Ok(())
    }

    async fn read_structure(
        &mut self,
        slots: &[Option<XrefSlot>],
        index: &mut PdfIndex,
    ) -> Result<()> {
        if let Some(info) = index.trailer_info {
            let location = index.object_location(info).map_err(|_| {
                self.malformed(
                    index.xref_offset,
                    Some(info),
                    "trailer Info does not resolve to a live object",
                )
            })?;
            let (head, _) = self.load_object(location.offset, info, slots).await?;
            let failure = self.malformed(
                location.offset,
                Some(info),
                "trailer Info is not a dictionary",
            );
            let dictionary = head.dictionary.ok_or(failure)?;
            if dictionary.value(b"Type").is_some() {
                return Err(self.malformed(
                    location.offset,
                    Some(info),
                    "trailer Info references a typed non-information dictionary",
                ));
            }
        }
        let catalog_location = index.object_location(index.catalog)?;
        let (catalog_head, _) = self
            .load_object(catalog_location.offset, index.catalog, slots)
            .await?;
        let failure = self.malformed(
            catalog_location.offset,
            Some(index.catalog),
            "Catalog is not a dictionary",
        );
        let catalog = catalog_head.dictionary.ok_or(failure)?;
        if catalog.value(b"Type").and_then(exact_name).as_deref() != Some(b"Catalog".as_slice()) {
            return Err(self.malformed(
                catalog_location.offset,
                Some(index.catalog),
                "trailer Root is not a Catalog",
            ));
        }
        if catalog.value(b"Perms").is_some() {
            return Err(self.problem(
                catalog_location.offset,
                Some(index.catalog),
                PdfErrorKind::UnsupportedFeature,
                "signed PDF edits are unsupported",
            ));
        }
        if let Some(value) = catalog.value(b"AcroForm") {
            let form_ref = exact_reference(value).ok_or_else(|| {
                self.problem(
                    catalog_location.offset,
                    Some(index.catalog),
                    PdfErrorKind::UnsupportedFeature,
                    "direct or malformed AcroForm dictionaries are unsupported",
                )
            })?;
            let location = index.object_location(form_ref)?;
            let (form_head, _) = self.load_object(location.offset, form_ref, slots).await?;
            let failure = self.malformed(
                location.offset,
                Some(form_ref),
                "AcroForm is not a dictionary",
            );
            let form = form_head.dictionary.ok_or(failure)?;
            if let Some(flags) = form.value(b"SigFlags") {
                let failure = self.malformed(
                    location.offset,
                    Some(form_ref),
                    "AcroForm SigFlags is invalid",
                );
                let flags = exact_unsigned(flags).ok_or(failure)?;
                if flags != 0 {
                    return Err(self.problem(
                        location.offset,
                        Some(form_ref),
                        PdfErrorKind::UnsupportedFeature,
                        "AcroForm signature indicators are unsupported",
                    ));
                }
            }
        }
        let failure = self.malformed(
            catalog_location.offset,
            Some(index.catalog),
            "Catalog lacks Pages reference",
        );
        let pages_root = catalog
            .value(b"Pages")
            .and_then(exact_reference)
            .ok_or(failure)?;
        let mut pages = Vec::new();
        let mut stack = Vec::new();
        push_bounded(
            &mut stack,
            PageVisit::Enter {
                reference: pages_root,
                parent: None,
                inherited_media_box: false,
            },
            self.limits.max_allocation_bytes,
            "PDF page tree stack",
        )
        .map_err(|error| self.locate_limit(catalog_location.offset, Some(index.catalog), error))?;
        // `PdfIndex::open` admitted more than one byte per slot under this
        // allocation limit, so a one-byte-per-slot index fits it.
        debug_assert!(self.limits.check_allocation(slots.len() as u64).is_ok());
        let mut visited = Vec::new();
        let refused = self.allocation_limit(
            catalog_location.offset,
            Some(index.catalog),
            "PDF page tree visited index",
            slots.len() as u64,
        );
        reserve_exact(&mut visited, slots.len(), refused)?;
        visited.resize(slots.len(), false);
        let mut contents_validated = Vec::new();
        let refused = self.allocation_limit(
            catalog_location.offset,
            Some(index.catalog),
            "PDF page content validation index",
            slots.len() as u64,
        );
        reserve_exact(&mut contents_validated, slots.len(), refused)?;
        contents_validated.resize(slots.len(), false);
        while let Some(task) = stack.pop() {
            let (reference, parent, inherited_media_box) = match task {
                PageVisit::Exit {
                    reference,
                    declared_count,
                    first_leaf,
                } => {
                    if pages.len().saturating_sub(first_leaf) != declared_count as usize {
                        let location = index.object_location(reference)?;
                        return Err(self.malformed(
                            location.offset,
                            Some(reference),
                            "Pages Count does not equal leaf descendants",
                        ));
                    }
                    continue;
                }
                PageVisit::Enter {
                    reference,
                    parent,
                    inherited_media_box,
                } => (reference, parent, inherited_media_box),
            };
            let location = index.object_location(reference)?;
            // A resolved reference has a slot, and `visited` has one entry
            // per slot.
            let seen = &mut visited[reference.number as usize];
            if *seen {
                return Err(self.malformed(
                    0,
                    Some(reference),
                    "page tree contains a cycle or duplicate child",
                ));
            }
            *seen = true;
            let (head, _) = self.load_object(location.offset, reference, slots).await?;
            let failure = self.malformed(
                location.offset,
                Some(reference),
                "page tree object is not a dictionary",
            );
            let dictionary = head.dictionary.ok_or(failure)?;
            let failure = self.malformed(
                location.offset,
                Some(reference),
                "page tree object lacks Type",
            );
            let kind = dictionary
                .value(b"Type")
                .and_then(exact_name)
                .ok_or(failure)?;
            let actual_parent = dictionary.value(b"Parent").map(exact_reference);
            if actual_parent == Some(None) {
                return Err(self.malformed(
                    location.offset,
                    Some(reference),
                    "page tree Parent is not a reference",
                ));
            }
            let actual_parent = actual_parent.flatten();
            if actual_parent != parent {
                if kind == b"Page" {
                    if let (Some(expected), Some(stale)) = (parent, actual_parent) {
                        if let Some(candidate) = index
                            .stale_page_parents
                            .iter()
                            .position(|item| *item == (reference, stale))
                        {
                            self.repair_page_parent(
                                &dictionary,
                                reference,
                                expected,
                                location.offset,
                                index,
                            )?;
                            index.stale_page_parents.swap_remove(candidate);
                        } else {
                            return Err(self.malformed(
                                location.offset,
                                Some(reference),
                                "page tree Parent link disagrees with Kids",
                            ));
                        }
                    } else {
                        return Err(self.malformed(
                            location.offset,
                            Some(reference),
                            "page tree Parent link disagrees with Kids",
                        ));
                    }
                } else {
                    return Err(self.malformed(
                        location.offset,
                        Some(reference),
                        "page tree Parent link disagrees with Kids",
                    ));
                }
            }
            let has_media_box = match dictionary.value(b"MediaBox") {
                Some(value) => {
                    self.validate_media_box(value, reference, location.offset, slots, index)
                        .await?;
                    true
                }
                None => inherited_media_box,
            };
            match kind.as_slice() {
                b"Pages" => {
                    let failure = self.malformed(
                        location.offset,
                        Some(reference),
                        "Pages node lacks valid Count",
                    );
                    let count = dictionary
                        .value(b"Count")
                        .and_then(exact_unsigned)
                        .and_then(|count| u32::try_from(count).ok())
                        .ok_or(failure)?;
                    self.limits.check_pages(count).map_err(|error| {
                        self.locate_limit(location.offset, Some(reference), error)
                    })?;
                    let failure = self.malformed(
                        location.offset,
                        Some(reference),
                        "Pages node lacks valid Kids",
                    );
                    let kids = dictionary
                        .value(b"Kids")
                        .and_then(|value| reference_array(value, self.limits.max_pages as usize))
                        .ok_or(failure)?;
                    if kids.is_empty() || kids.len() > count as usize {
                        return Err(self.malformed(
                            location.offset,
                            Some(reference),
                            "Pages Count/Kids are inconsistent",
                        ));
                    }
                    push_bounded(
                        &mut stack,
                        PageVisit::Exit {
                            reference,
                            declared_count: count,
                            first_leaf: pages.len(),
                        },
                        self.limits.max_allocation_bytes,
                        "PDF page tree stack",
                    )
                    .map_err(|error| self.locate_limit(location.offset, Some(reference), error))?;
                    for kid in kids.into_iter().rev() {
                        push_bounded(
                            &mut stack,
                            PageVisit::Enter {
                                reference: kid,
                                parent: Some(reference),
                                inherited_media_box: has_media_box,
                            },
                            self.limits.max_allocation_bytes,
                            "PDF page tree stack",
                        )
                        .map_err(|error| {
                            self.locate_limit(location.offset, Some(reference), error)
                        })?;
                    }
                }
                b"Page" => {
                    if !has_media_box {
                        return Err(self.malformed(
                            location.offset,
                            Some(reference),
                            "Page lacks inherited MediaBox",
                        ));
                    }
                    if let Some(contents) = dictionary.value(b"Contents") {
                        self.validate_page_contents(
                            contents,
                            reference,
                            location.offset,
                            slots,
                            index,
                            &mut contents_validated,
                        )
                        .await?;
                    }
                    push_bounded(
                        &mut pages,
                        reference,
                        self.limits.max_allocation_bytes,
                        "PDF page index",
                    )
                    .map_err(|error| self.locate_limit(location.offset, Some(reference), error))?;
                    self.limits
                        .check_pages(pages.len() as u32)
                        .map_err(|error| {
                            self.locate_limit(location.offset, Some(reference), error)
                        })?;
                }
                _ => {
                    return Err(self.malformed(
                        location.offset,
                        Some(reference),
                        "Kids entry is not Page or Pages",
                    ));
                }
            }
        }
        // The walk ends without error only after the root is a Page or its
        // Exit confirmed as many leaves as its nonzero Count, both read from
        // the single load of each node above.
        debug_assert!(!pages.is_empty());
        index.pages = pages;
        index.catalog_dict = catalog;
        if let Some(outline_value) = index.catalog_dict.value(b"Outlines") {
            let failure = self.malformed(
                catalog_location.offset,
                Some(index.catalog),
                "invalid Catalog Outlines reference",
            );
            let outline_ref = exact_reference(outline_value).ok_or(failure)?;
            let outline_location = index.object_location(outline_ref)?;
            let (head, _) = self
                .load_object(outline_location.offset, outline_ref, slots)
                .await?;
            let failure = self.malformed(
                outline_location.offset,
                Some(outline_ref),
                "Outlines root is not a dictionary",
            );
            let outline = head.dictionary.ok_or(failure)?;
            index.has_outlines = self
                .validate_outline_tree(&outline, outline_ref, outline_location.offset, slots, index)
                .await?;
        }
        Ok(())
    }

    async fn validate_media_box(
        &mut self,
        value: &[u8],
        owner: PdfRef,
        owner_offset: u64,
        slots: &[Option<XrefSlot>],
        index: &PdfIndex,
    ) -> Result<()> {
        if media_box(value).is_some() {
            return Ok(());
        }
        if let Some(reference) = exact_reference(value) {
            let location = index.object_location(reference)?;
            let (head, _) = self.load_object(location.offset, reference, slots).await?;
            if head
                .scalar
                .as_ref()
                .and_then(|span| media_box(&head.bytes[span.clone()]))
                .is_some()
            {
                return Ok(());
            }
            return Err(self.malformed(
                location.offset,
                Some(reference),
                "indirect MediaBox is not a valid rectangle array",
            ));
        }
        Err(self.malformed(owner_offset, Some(owner), "page tree MediaBox is invalid"))
    }

    async fn validate_page_contents(
        &mut self,
        value: &[u8],
        page: PdfRef,
        page_offset: u64,
        slots: &[Option<XrefSlot>],
        index: &PdfIndex,
        contents_validated: &mut [bool],
    ) -> Result<()> {
        let references = if let Some(reference) = exact_reference(value) {
            if contents_validated
                .get(reference.number as usize)
                .copied()
                .unwrap_or(false)
            {
                return Ok(());
            }
            let location = index.object_location(reference)?;
            let (head, _) = self.load_object(location.offset, reference, slots).await?;
            if matches!(head.tail, ObjectTail::Stream { .. }) {
                contents_validated[reference.number as usize] = true;
                return Ok(());
            }
            let scalar = head.scalar.as_ref().map(|range| &head.bytes[range.clone()]);
            let failure = self.malformed(
                page_offset,
                Some(page),
                "Page Contents is not a stream or stream array",
            );
            let references = scalar
                .and_then(|raw| reference_array(raw, slots.len()))
                .ok_or(failure)?;
            for stream in &references {
                self.validate_content_stream(*stream, slots, index).await?;
            }
            contents_validated[reference.number as usize] = true;
            return Ok(());
        } else {
            reference_array(value, slots.len()).ok_or(self.malformed(
                page_offset,
                Some(page),
                "Page Contents must be a stream reference or reference array",
            ))?
        };
        for reference in references {
            self.validate_content_stream(reference, slots, index)
                .await?;
        }
        Ok(())
    }

    async fn validate_content_stream(
        &mut self,
        reference: PdfRef,
        slots: &[Option<XrefSlot>],
        index: &PdfIndex,
    ) -> Result<()> {
        let location = index.object_location(reference)?;
        let (head, _) = self.load_object(location.offset, reference, slots).await?;
        if !matches!(head.tail, ObjectTail::Stream { .. }) {
            return Err(self.malformed(
                location.offset,
                Some(reference),
                "Page Contents array member is not a stream",
            ));
        }
        Ok(())
    }

    async fn validate_outline_tree(
        &mut self,
        root: &Dictionary,
        root_ref: PdfRef,
        root_offset: u64,
        slots: &[Option<XrefSlot>],
        index: &PdfIndex,
    ) -> Result<bool> {
        if root
            .value(b"Type")
            .is_some_and(|value| exact_name(value).as_deref() != Some(b"Outlines"))
        {
            return Err(self.malformed(
                root_offset,
                Some(root_ref),
                "outline root Type is invalid",
            ));
        }
        let first = root.value(b"First").map(exact_reference);
        let last = root.value(b"Last").map(exact_reference);
        let root_count = root.value(b"Count").map(exact_unsigned);
        if root_count == Some(None) {
            return Err(self.malformed(
                root_offset,
                Some(root_ref),
                "outline root Count is invalid",
            ));
        }
        let (Some(first), Some(last)) = (
            first.transpose_option().flatten(),
            last.transpose_option().flatten(),
        ) else {
            if first.is_none() && last.is_none() {
                if root_count.flatten().unwrap_or(0) != 0 {
                    return Err(self.malformed(
                        root_offset,
                        Some(root_ref),
                        "empty outline root has nonzero Count",
                    ));
                }
                return Ok(false);
            }
            return Err(self.malformed(
                root_offset,
                Some(root_ref),
                "outline root First and Last must be valid references",
            ));
        };
        if root_count.flatten() == Some(0) {
            return Err(self.malformed(
                root_offset,
                Some(root_ref),
                "nonempty outline root has zero Count",
            ));
        }
        // `PdfIndex::open` admitted more than one byte per slot under this
        // allocation limit, so a one-byte-per-slot index fits it.
        let index_bytes = slots.len() as u64;
        debug_assert!(self.limits.check_allocation(index_bytes).is_ok());
        let mut visited = Vec::new();
        let refused = self.allocation_limit(
            root_offset,
            Some(root_ref),
            "PDF outline visited index",
            index_bytes,
        );
        reserve_exact(&mut visited, slots.len(), refused)?;
        visited.resize(slots.len(), false);
        let target_bytes = slots
            .len()
            .checked_mul(std::mem::size_of::<Option<u16>>())
            .ok_or(Error::InvalidInput {
                reason: "PDF outline destination index size overflows",
            })?;
        self.limits
            .check_allocation(target_bytes as u64)
            .map_err(|error| self.locate_limit(root_offset, Some(root_ref), error))?;
        let mut page_targets = Vec::new();
        let refused = self.allocation_limit(
            root_offset,
            Some(root_ref),
            "PDF outline destination index",
            target_bytes as u64,
        );
        reserve_exact(&mut page_targets, slots.len(), refused)?;
        page_targets.resize(slots.len(), None);
        for page in &index.pages {
            page_targets[page.number as usize] = Some(page.generation);
        }
        let mut stack = Vec::new();
        push_bounded(
            &mut stack,
            OutlineVisit {
                reference: first,
                parent: root_ref,
                previous: None,
                expected_last: last,
            },
            self.limits.max_allocation_bytes,
            "PDF outline stack",
        )
        .map_err(|error| self.locate_limit(root_offset, Some(root_ref), error))?;
        let mut item_count = 0_u32;
        while let Some(task) = stack.pop() {
            let location = index.object_location(task.reference)?;
            // A resolved reference has a slot, and `visited` has one entry
            // per slot.
            let seen = &mut visited[task.reference.number as usize];
            if *seen {
                return Err(self.malformed(
                    location.offset,
                    Some(task.reference),
                    "outline tree contains a cycle or repeated item",
                ));
            }
            *seen = true;
            item_count = item_count.checked_add(1).ok_or(Error::LimitExceeded {
                resource: "PDF outline items",
                limit: u64::from(self.limits.max_bookmarks),
                attempted: u64::MAX,
            })?;
            self.limits
                .check_bookmarks(item_count)
                .map_err(|error| self.locate_limit(location.offset, Some(task.reference), error))?;
            let (head, _) = self
                .load_object(location.offset, task.reference, slots)
                .await?;
            let failure = self.malformed(
                location.offset,
                Some(task.reference),
                "outline item is not a dictionary",
            );
            let item = head.dictionary.ok_or(failure)?;
            if !item.value(b"Title").is_some_and(valid_text_string) {
                return Err(self.malformed(
                    location.offset,
                    Some(task.reference),
                    "outline item lacks a valid text Title",
                ));
            }
            if item.value(b"Parent").and_then(exact_reference) != Some(task.parent) {
                return Err(self.malformed(
                    location.offset,
                    Some(task.reference),
                    "outline item Parent link is invalid",
                ));
            }
            let previous = item.value(b"Prev").map(exact_reference);
            if previous == Some(None) || previous.flatten() != task.previous {
                return Err(self.malformed(
                    location.offset,
                    Some(task.reference),
                    "outline item Prev link is invalid",
                ));
            }
            if item.value(b"A").is_some() {
                return Err(self.problem(
                    location.offset,
                    Some(task.reference),
                    PdfErrorKind::UnsupportedFeature,
                    "outline actions are outside the supported input profile",
                ));
            }
            if let Some(value) = item.value(b"Dest") {
                let page = destination_page(value).ok_or_else(|| {
                    self.problem(
                        location.offset,
                        Some(task.reference),
                        PdfErrorKind::UnsupportedFeature,
                        "outline destination must be a direct page array",
                    )
                })?;
                if page_targets.get(page.number as usize).copied().flatten()
                    != Some(page.generation)
                {
                    return Err(self.malformed(
                        location.offset,
                        Some(task.reference),
                        "outline destination does not target a page",
                    ));
                }
            }
            let next = item.value(b"Next").map(exact_reference);
            let next = match next {
                Some(None) => {
                    return Err(self.malformed(
                        location.offset,
                        Some(task.reference),
                        "outline Next reference is invalid",
                    ));
                }
                Some(Some(next)) => Some(next),
                None => None,
            };
            if let Some(next) = next {
                push_bounded(
                    &mut stack,
                    OutlineVisit {
                        reference: next,
                        parent: task.parent,
                        previous: Some(task.reference),
                        expected_last: task.expected_last,
                    },
                    self.limits.max_allocation_bytes,
                    "PDF outline stack",
                )
                .map_err(|error| self.locate_limit(location.offset, Some(task.reference), error))?;
            } else if task.reference != task.expected_last {
                return Err(self.malformed(
                    location.offset,
                    Some(task.reference),
                    "outline Last link disagrees with sibling chain",
                ));
            }
            let child_first = item.value(b"First").map(exact_reference);
            let child_last = item.value(b"Last").map(exact_reference);
            match (child_first, child_last) {
                (None, None) => {}
                (Some(Some(first)), Some(Some(last))) => {
                    push_bounded(
                        &mut stack,
                        OutlineVisit {
                            reference: first,
                            parent: task.reference,
                            previous: None,
                            expected_last: last,
                        },
                        self.limits.max_allocation_bytes,
                        "PDF outline stack",
                    )
                    .map_err(|error| {
                        self.locate_limit(location.offset, Some(task.reference), error)
                    })?;
                }
                _ => {
                    return Err(self.malformed(
                        location.offset,
                        Some(task.reference),
                        "outline child First and Last must be valid references",
                    ));
                }
            }
        }
        Ok(true)
    }

    fn check_dictionary_duplicates(
        &self,
        dictionary: &Dictionary,
        reference: PdfRef,
        at: u64,
        kind: &[u8],
        repairs: &mut Vec<RepairObject>,
        retained_repair_bytes: &mut u64,
    ) -> Result<()> {
        let order_bytes = dictionary
            .entries
            .len()
            .saturating_mul(std::mem::size_of::<usize>());
        self.limits
            .check_allocation(order_bytes as u64)
            .map_err(|error| self.locate_limit(at, Some(reference), error))?;
        let mut order = Vec::new();
        let refused = self.allocation_limit(
            at,
            Some(reference),
            "PDF dictionary key index",
            order_bytes as u64,
        );
        reserve_exact(&mut order, dictionary.entries.len(), refused)?;
        order.extend(0..dictionary.entries.len());
        order.sort_unstable_by(|left, right| {
            dictionary.entries[*left]
                .name
                .cmp(&dictionary.entries[*right].name)
        });
        let mut duplicate_media_box = false;
        for pair in order.windows(2) {
            let left = &dictionary.entries[pair[0]];
            let right = &dictionary.entries[pair[1]];
            if left.name == right.name {
                if kind == b"Pages"
                    && left.name == b"MediaBox"
                    && media_box(left.value(&dictionary.bytes)).is_some()
                    && media_box(left.value(&dictionary.bytes))
                        == media_box(right.value(&dictionary.bytes))
                    && dictionary.entries_named(b"MediaBox").count() == 2
                {
                    duplicate_media_box = true;
                    continue;
                }
                return Err(self.problem(
                    at,
                    Some(reference),
                    PdfErrorKind::AmbiguousRepair,
                    "duplicate PDF dictionary keys have conflicting or unsupported values",
                ));
            }
        }
        if duplicate_media_box {
            let first_media_box = dictionary
                .entries
                .iter()
                .position(|entry| entry.name == b"MediaBox");
            self.push_repair_body(
                dictionary,
                reference,
                at,
                |position, entry| entry.name != b"MediaBox" || Some(position) == first_media_box,
                b"",
                repairs,
                retained_repair_bytes,
            )?;
        }
        Ok(())
    }

    fn repair_page_parent(
        &self,
        dictionary: &Dictionary,
        reference: PdfRef,
        parent: PdfRef,
        at: u64,
        index: &mut PdfIndex,
    ) -> Result<()> {
        let replacement = format!("/Parent {} {} R\n", parent.number, parent.generation);
        self.push_repair_body(
            dictionary,
            reference,
            at,
            |_, entry| entry.name != b"Parent",
            replacement.as_bytes(),
            &mut index.repair_objects,
            &mut index.retained_repair_bytes,
        )
    }

    /// Records a repaired dictionary body built from the entries accepted by
    /// `keep`, followed by `appended`, charging exactly the bytes it retains
    /// against the repair budget. The size and the body come from the same
    /// entry list, so they cannot disagree.
    #[allow(clippy::too_many_arguments)]
    fn push_repair_body(
        &self,
        dictionary: &Dictionary,
        reference: PdfRef,
        at: u64,
        keep: impl Fn(usize, &DictEntry) -> bool,
        appended: &[u8],
        repairs: &mut Vec<RepairObject>,
        retained_repair_bytes: &mut u64,
    ) -> Result<()> {
        let kept = || {
            dictionary
                .entries
                .iter()
                .enumerate()
                .filter(|(position, entry)| keep(*position, entry))
                .map(|(_, entry)| entry.raw_pair(&dictionary.bytes))
        };
        // "<<\n" and ">>" frame the retained pairs, one newline each, and the
        // appended bytes. A saturated size cannot be allocated: it exceeds the
        // cap below on 64-bit targets and fails the reservation on 32-bit ones.
        let needed = kept().fold(5_usize.saturating_add(appended.len()), |size, pair| {
            size.saturating_add(pair.len() + 1)
        });
        let next_retained = retained_repair_bytes.saturating_add(needed as u64);
        let cap = self.limits.max_allocation_bytes / 2;
        if next_retained > cap {
            return Err(self.locate_limit(
                at,
                Some(reference),
                Error::LimitExceeded {
                    resource: "PDF repair object bytes",
                    limit: cap,
                    attempted: next_retained,
                },
            ));
        }
        let mut body = Vec::new();
        let refused = self.locate_limit(
            at,
            Some(reference),
            Error::LimitExceeded {
                resource: "PDF repair object allocation",
                limit: cap,
                attempted: needed as u64,
            },
        );
        reserve_exact(&mut body, needed, refused)?;
        body.extend_from_slice(b"<<\n");
        for pair in kept() {
            body.extend_from_slice(pair);
            body.push(b'\n');
        }
        body.extend_from_slice(appended);
        body.extend_from_slice(b">>");
        push_bounded(
            repairs,
            RepairObject { reference, body },
            cap,
            "PDF repair index",
        )
        .map_err(|error| self.locate_limit(at, Some(reference), error))?;
        *retained_repair_bytes = next_retained;
        Ok(())
    }

    async fn validate_live_object_spans(
        &mut self,
        index: &mut PdfIndex,
        slots: &[Option<XrefSlot>],
        check_gaps: bool,
    ) -> Result<()> {
        let object_count = index.object_locations.iter().flatten().count();
        let bytes = object_count
            .checked_mul(std::mem::size_of::<(u32, ObjectLocation)>())
            .ok_or(Error::InvalidInput {
                reason: "PDF object span index size overflows",
            })?;
        self.limits
            .check_allocation(bytes as u64)
            .map_err(|error| self.locate_limit(index.xref_offset, None, error))?;
        let mut locations = Vec::new();
        let refused = self.allocation_limit(
            index.xref_offset,
            None,
            "PDF object span index",
            bytes as u64,
        );
        reserve_exact(&mut locations, object_count, refused)?;
        locations.extend(
            index
                .object_locations
                .iter()
                .enumerate()
                .filter_map(|(number, slot)| slot.map(|(_, location)| (number as u32, location))),
        );
        locations.sort_unstable_by_key(|(_, location)| location.offset);
        let mut previous_end = 0_u64;
        for (number, location) in locations {
            if location.offset < previous_end {
                return Err(self.malformed(location.offset, None, "PDF objects overlap"));
            }
            if check_gaps && previous_end != 0 {
                self.validate_gap(previous_end, location.offset, Some(number), slots, index)
                    .await?;
            }
            previous_end = location.offset + location.length;
        }
        if check_gaps && previous_end < index.xref_offset {
            self.validate_gap(previous_end, index.xref_offset, None, slots, index)
                .await?;
        }
        Ok(())
    }

    async fn validate_gap(
        &mut self,
        start: u64,
        end: u64,
        next_live: Option<u32>,
        slots: &[Option<XrefSlot>],
        index: &mut PdfIndex,
    ) -> Result<()> {
        let mut cursor = start;
        while cursor < end {
            match self.byte(cursor).await? {
                Some(b' ' | b'\t' | b'\r' | b'\n' | 0 | 12) => cursor += 1,
                Some(b'%') => {
                    while cursor < end {
                        let byte = self.byte(cursor).await?.unwrap_or(0);
                        cursor += 1;
                        if byte == b'\r' || byte == b'\n' {
                            break;
                        }
                    }
                }
                _ => {
                    let length = end - start;
                    if length <= MAX_ORPHAN_GAP_BYTES {
                        let original = self.bytes(start, length as usize).await?;
                        if let Some(number) = parse_orphan_gap(&original) {
                            let free = matches!(
                                slots.get(number as usize).and_then(|slot| *slot),
                                Some(XrefSlot {
                                    kind: XrefKind::Free,
                                    ..
                                })
                            );
                            if free || next_live == Some(number) {
                                let failure =
                                    self.malformed(start, None, "orphan gap repair size overflows");
                                let retained = index
                                    .retained_gap_bytes
                                    .checked_add(length)
                                    .ok_or(failure)?;
                                let cap =
                                    MAX_ORPHAN_GAP_TOTAL.min(self.limits.max_allocation_bytes / 8);
                                if retained > cap {
                                    return Err(self.locate_limit(
                                        start,
                                        None,
                                        Error::LimitExceeded {
                                            resource: "PDF orphan gap repair bytes",
                                            limit: cap,
                                            attempted: retained,
                                        },
                                    ));
                                }
                                push_bounded(
                                    &mut index.gap_patches,
                                    GapPatch {
                                        offset: start,
                                        original,
                                    },
                                    cap,
                                    "PDF orphan gap repair index",
                                )
                                .map_err(|error| self.locate_limit(start, None, error))?;
                                index.retained_gap_bytes = retained;
                                return Ok(());
                            }
                        }
                    }
                    return Err(self.malformed(
                        cursor,
                        None,
                        "unindexed bytes between PDF objects",
                    ));
                }
            }
        }
        Ok(())
    }
}

fn parse_xref_entry(line: &[u8]) -> Option<XrefSlot> {
    if line.len() != 20
        || !line[..10].iter().all(u8::is_ascii_digit)
        || line[10] != b' '
        || !line[11..16].iter().all(u8::is_ascii_digit)
        || line[16] != b' '
        || !matches!((line[18], line[19]), (b' ', b'\r' | b'\n') | (b'\r', b'\n'))
    {
        return None;
    }
    let offset = std::str::from_utf8(&line[..10]).ok()?.parse().ok()?;
    let generation = std::str::from_utf8(&line[11..16]).ok()?.parse().ok()?;
    let kind = match line[17] {
        b'n' => XrefKind::InUse(offset),
        b'f' => XrefKind::Free,
        _ => return None,
    };
    Some(XrefSlot { generation, kind })
}

fn read_be(bytes: &[u8], position: &mut usize, width: usize) -> Option<u64> {
    let end = position.checked_add(width)?;
    let mut value = 0_u64;
    for byte in bytes.get(*position..end)? {
        value = (value << 8) | u64::from(*byte);
    }
    *position = end;
    Some(value)
}

fn parse_orphan_gap(bytes: &[u8]) -> Option<u32> {
    let mut value = bytes.trim_ascii_start();
    let digits = value
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if digits == 0 || !value.get(digits).is_some_and(u8::is_ascii_whitespace) {
        return None;
    }
    let number = std::str::from_utf8(&value[..digits])
        .ok()?
        .parse::<u32>()
        .ok()?;
    if number == 0 || number > MAX_PDF_OBJECTS {
        return None;
    }
    value = value[digits..].trim_ascii_start();
    if !value.starts_with(b"0") || value.get(1).is_some_and(|byte| !byte.is_ascii_whitespace()) {
        return None;
    }
    value = value[1..].trim_ascii_start();
    if value.is_empty() {
        return Some(number);
    }
    if !value.starts_with(b"obj") || value.get(3).is_some_and(|byte| !byte.is_ascii_whitespace()) {
        return None;
    }
    value = value[3..].trim_ascii();
    if value.is_empty() || value == b"<" {
        return Some(number);
    }
    let scalar_digits = value
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if scalar_digits == 0 {
        return None;
    }
    let rest = value[scalar_digits..].trim_ascii();
    (rest.is_empty() || rest == b"e").then_some(number)
}

enum InflateXrefError {
    Malformed,
    TooLong,
    Cancelled,
}

fn inflate_xref<C: Cancellation>(
    encoded: &[u8],
    expected: usize,
    chunk_bytes: usize,
    cancellation: &C,
) -> std::result::Result<Vec<u8>, InflateXrefError> {
    let length = expected.checked_add(1).ok_or(InflateXrefError::TooLong)?;
    let mut decoded = Vec::new();
    reserve_exact(&mut decoded, length, InflateXrefError::TooLong)?;
    decoded.resize(length, 0);
    let mut inflater = Decompress::new(true);
    loop {
        if cancellation.is_cancelled() {
            return Err(InflateXrefError::Cancelled);
        }
        let before_in = inflater.total_in();
        let before_out = inflater.total_out();
        let input_end = (before_in as usize)
            .saturating_add(chunk_bytes)
            .min(encoded.len());
        let output_end = (before_out as usize)
            .saturating_add(chunk_bytes)
            .min(decoded.len());
        let status = inflater
            .decompress(
                &encoded[before_in as usize..input_end],
                &mut decoded[before_out as usize..output_end],
                FlushDecompress::None,
            )
            .map_err(|_| InflateXrefError::Malformed)?;
        if inflater.total_out() as usize > expected {
            return Err(InflateXrefError::TooLong);
        }
        if status == Status::StreamEnd {
            if inflater.total_in() as usize != encoded.len()
                || inflater.total_out() as usize != expected
            {
                return Err(InflateXrefError::Malformed);
            }
            decoded.truncate(expected);
            return Ok(decoded);
        }
        if inflater.total_in() == before_in && inflater.total_out() == before_out {
            return Err(InflateXrefError::Malformed);
        }
    }
}

fn push_bounded<T>(
    items: &mut Vec<T>,
    item: T,
    max_bytes: u64,
    resource: &'static str,
) -> Result<()> {
    let element_bytes = std::mem::size_of::<T>();
    let next = items.len().checked_add(1).ok_or(Error::InvalidInput {
        reason: "PDF index length overflows address space",
    })?;
    if next > items.capacity() {
        let target = items.capacity().saturating_mul(2).max(4).max(next);
        let attempted = target
            .checked_mul(element_bytes)
            .ok_or(Error::InvalidInput {
                reason: "PDF index allocation overflows address space",
            })?;
        if attempted as u64 > max_bytes {
            return Err(Error::LimitExceeded {
                resource,
                limit: max_bytes,
                attempted: attempted as u64,
            });
        }
        let additional = target - items.len();
        let refused = Error::LimitExceeded {
            resource,
            limit: max_bytes,
            attempted: attempted as u64,
        };
        reserve_exact(items, additional, refused)?;
    }
    items.push(item);
    Ok(())
}

trait TransposeOption<T> {
    fn transpose_option(self) -> Option<Option<T>>;
}
impl<T> TransposeOption<T> for Option<Option<T>> {
    fn transpose_option(self) -> Option<Option<T>> {
        match self {
            Some(Some(value)) => Some(Some(value)),
            Some(None) => None,
            None => Some(None),
        }
    }
}

/// The structural role of one supplied PDF-style object fragment.
#[derive(Clone, Debug)]
pub(crate) enum FragmentKind {
    Page {
        parent: PdfRef,
        has_media_box: bool,
    },
    Pages {
        parent: Option<PdfRef>,
        count: u32,
        kids: Vec<PdfRef>,
        has_media_box: bool,
    },
    Catalog {
        pages: PdfRef,
    },
    Other,
}

#[derive(Clone, Debug)]
pub(crate) struct FragmentInspection {
    pub reference: PdfRef,
    pub kind: FragmentKind,
    pub references: Vec<PdfRef>,
    pub max_referenced_object: u32,
    pub destination: Option<PdfRef>,
    pub is_stream: bool,
    pub contents: Option<Vec<PdfRef>>,
    pub contents_is_direct_array: bool,
    pub scalar_reference_array: Option<Vec<PdfRef>>,
}

/// Inspect one complete caller-supplied object span, never scanning adjacent
/// CAJ container bytes. `resolve_length` supplies already indexed integer
/// objects used by an indirect stream `/Length`.
pub(crate) async fn inspect_fragment_object<
    S: RangedSource,
    C: Cancellation,
    F: Fn(PdfRef) -> Option<u64>,
>(
    source: &mut S,
    range: PdfRange,
    expected: PdfRef,
    limits: &Limits,
    cancellation: &C,
    resolve_length: F,
) -> Result<FragmentInspection> {
    limits.validate()?;
    let mut reader = Reader::new(source, range, limits, cancellation)?;
    let head = reader.load_head(0, Some(expected)).await?;
    let is_stream = matches!(head.tail, ObjectTail::Stream { .. });
    let end = match head.tail {
        ObjectTail::EndObject { end } => end as u64,
        ObjectTail::Stream { data_start } => {
            let failure = reader.malformed(0, Some(expected), "stream lacks dictionary");
            let dictionary = head.dictionary.as_ref().ok_or(failure)?;
            let failure = reader.malformed(0, Some(expected), "stream lacks Length");
            let value = dictionary.value(b"Length").ok_or(failure)?;
            let length = exact_unsigned(value)
                .or_else(|| exact_reference(value).and_then(&resolve_length))
                .ok_or(reader.malformed(0, Some(expected), "stream Length does not resolve"))?;
            let after_data = (data_start as u64)
                .checked_add(length)
                .ok_or(reader.malformed(0, Some(expected), "stream extent overflows"))?;
            reader.check_stream_tail(after_data, Some(expected)).await?
        }
    };
    let mut rest = end;
    reader.skip_space(&mut rest).await?;
    if rest != range.length {
        return Err(reader.malformed(
            rest,
            Some(expected),
            "fragment has trailing non-whitespace bytes",
        ));
    }
    if let Some(dictionary) = &head.dictionary {
        reader.reject_duplicate_names(dictionary, 0, Some(expected))?;
    }
    let destination = if let Some(dictionary) = &head.dictionary {
        if dictionary.value(b"Title").is_some() {
            if dictionary.value(b"A").is_some() {
                return Err(reader.problem(
                    0,
                    Some(expected),
                    PdfErrorKind::UnsupportedFeature,
                    "outline actions in PDF fragments are unsupported",
                ));
            }
            match dictionary.value(b"Dest") {
                Some(value) => Some(destination_page(value).ok_or_else(|| {
                    reader.problem(
                        0,
                        Some(expected),
                        PdfErrorKind::UnsupportedFeature,
                        "outline destination is not a direct page array",
                    )
                })?),
                None => None,
            }
        } else {
            None
        }
    } else {
        None
    };
    let scalar_reference_array = head
        .scalar
        .as_ref()
        .and_then(|span| reference_array(&head.bytes[span.clone()], head.references.len()));
    let mut contents = None;
    let mut contents_is_direct_array = false;
    let kind = if let Some(dictionary) = &head.dictionary {
        let name = dictionary
            .value(b"Type")
            .and_then(exact_name)
            .unwrap_or_default();
        let has_media_box = match dictionary.value(b"MediaBox") {
            Some(value) if media_box(value).is_some() => true,
            Some(value)
                if (name == b"Page" || name == b"Pages") && exact_reference(value).is_some() =>
            {
                return Err(reader.problem(
                    0,
                    Some(expected),
                    PdfErrorKind::UnsupportedFeature,
                    "indirect MediaBox in PDF fragments is unsupported",
                ));
            }
            Some(_) if name == b"Page" || name == b"Pages" => {
                return Err(reader.malformed(
                    0,
                    Some(expected),
                    "fragment page tree MediaBox is invalid",
                ));
            }
            _ => false,
        };
        match name.as_slice() {
            b"Page" => {
                if let Some(value) = dictionary.value(b"Contents") {
                    let references = if let Some(reference) = exact_reference(value) {
                        vec![reference]
                    } else {
                        contents_is_direct_array = true;
                        reference_array(value, head.references.len()).ok_or(reader.malformed(
                            0,
                            Some(expected),
                            "fragment Page Contents is not a reference array",
                        ))?
                    };
                    contents = Some(references);
                }
                FragmentKind::Page {
                    parent: dictionary
                        .value(b"Parent")
                        .and_then(exact_reference)
                        .ok_or(reader.malformed(0, Some(expected), "Page lacks Parent"))?,
                    has_media_box,
                }
            }
            b"Pages" => FragmentKind::Pages {
                parent: match dictionary.value(b"Parent") {
                    Some(value) => Some(exact_reference(value).ok_or(reader.malformed(
                        0,
                        Some(expected),
                        "fragment Pages Parent is not a reference",
                    ))?),
                    None => None,
                },
                count: dictionary
                    .value(b"Count")
                    .and_then(exact_unsigned)
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or(reader.malformed(0, Some(expected), "Pages lacks Count"))?,
                has_media_box,
                kids: dictionary
                    .value(b"Kids")
                    .and_then(|v| reference_array(v, limits.max_pages as usize))
                    .ok_or(reader.malformed(0, Some(expected), "Pages lacks Kids"))?,
            },
            b"Catalog" => {
                if dictionary.value(b"Outlines").is_some() {
                    return Err(reader.problem(
                        0,
                        Some(expected),
                        PdfErrorKind::UnsupportedFeature,
                        "preexisting outline trees in PDF fragments are unsupported",
                    ));
                }
                FragmentKind::Catalog {
                    pages: dictionary
                        .value(b"Pages")
                        .and_then(exact_reference)
                        .ok_or(reader.malformed(0, Some(expected), "Catalog lacks Pages"))?,
                }
            }
            _ => FragmentKind::Other,
        }
    } else {
        FragmentKind::Other
    };
    Ok(FragmentInspection {
        reference: expected,
        kind,
        references: head.references,
        max_referenced_object: head.max_reference,
        destination,
        is_stream,
        contents,
        contents_is_direct_array,
        scalar_reference_array,
    })
}

/// Read a complete integer-only object for a fragment `/Length` lookup.
pub(crate) async fn inspect_fragment_scalar<S: RangedSource, C: Cancellation>(
    source: &mut S,
    range: PdfRange,
    expected: PdfRef,
    limits: &Limits,
    cancellation: &C,
) -> Result<Option<u64>> {
    limits.validate()?;
    let mut reader = Reader::new(source, range, limits, cancellation)?;
    let head = reader.load_head(0, Some(expected)).await?;
    let ObjectTail::EndObject { end } = head.tail else {
        return Ok(None);
    };
    let mut rest = end as u64;
    reader.skip_space(&mut rest).await?;
    if rest != range.length {
        return Err(reader.malformed(rest, Some(expected), "integer fragment has trailing bytes"));
    }
    Ok(head
        .scalar
        .and_then(|span| exact_unsigned(&head.bytes[span])))
}

mod fragment_scan;

pub(crate) use fragment_scan::{PatchedSource, scan_fragment_objects};

#[cfg(test)]
mod tests;
