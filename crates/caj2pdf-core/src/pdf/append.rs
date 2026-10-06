// SPDX-License-Identifier: MIT

//! Bounded pass-through and append-only outline updates for inspected PDFs.
//!
//! This module never reserializes page contents. The reader validates and
//! indexes the input first; the appender copies its PDF prefix and writes only
//! changed dictionaries, new outline items, a sparse xref, and a new trailer.

use super::input::{GapPatch, PdfIndex};
use super::outline::{
    BookmarkView, MAX_OUTLINE_DEPTH, ObjectAllocator, ObjectSink, OutlineBuilder,
};
use super::types::{PdfRange, PdfRef};
use super::writer::{MAX_PDF_OBJECTS, Output};
use super::xref::{Trailer, write_xref};
use crate::fallible::{reserve_exact, usize_from_u32};
use crate::{
    Bookmark, Cancellation, ConversionReport, CountingSource, Error, Limits, RangedSource, Result,
    read_exact_at,
};
use std::io::Write;
use std::mem::size_of;

// The second document ID is a version marker derived from the update's
// position, not a signature or cryptographic digest. The first ID is
// retained exactly as supplied.
const FNV128_BASIS: u128 = 0x6c62272e07bb014262b821756295c58d;
const FNV128_PRIME: u128 = 0x0000000001000000000000000000013b;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct XrefEntry {
    reference: PdfRef,
    offset: u64,
}

/// Copy a complete PDF after bounded structural inspection.
///
/// Recognized duplicate identical `/MediaBox` keys are normalized through an
/// appended revision. A validated opaque tail after the original `%%EOF` is
/// omitted. A clean PDF is copied byte-for-byte. The returned report is only
/// produced after the sink flush succeeds.
pub fn copy_pdf<R: RangedSource, W: Write, C: Cancellation>(
    source: &mut R,
    sink: &mut W,
    limits: &Limits,
    cancellation: &C,
) -> Result<ConversionReport> {
    copy_pdf_range(
        source,
        sink,
        PdfRange {
            offset: 0,
            length: source.size(),
        },
        limits,
        cancellation,
    )
}

/// Copy a PDF held in a bounded region of a larger random-access input.
pub fn copy_pdf_range<R: RangedSource, W: Write, C: Cancellation>(
    source: &mut R,
    sink: &mut W,
    range: PdfRange,
    limits: &Limits,
    cancellation: &C,
) -> Result<ConversionReport> {
    let mut input_bytes_read = 0;
    let mut counted =
        CountingSource::new(source, &mut input_bytes_read).rejecting_overread(OVERREAD);
    let index = PdfIndex::open(&mut counted, range, limits, cancellation)?;
    let mut report =
        PdfOutlineAppender::begin(&mut counted, sink, &index, limits, cancellation)?.finish()?;
    report.input_bytes_read = input_bytes_read;
    Ok(report)
}

const OVERREAD: &str = "PDF source reported more bytes than requested";

/// Append CAJ bookmark entries to a previously inspected PDF.
///
/// `begin` copies the PDF prefix before accepting bookmarks. A caller-owned
/// sink can therefore contain partial output if a later input or sink error
/// occurs; callers needing atomic path output should stage and rename it.
/// An existing nonempty outline tree is preserved: `add_bookmark` becomes a
/// no-op, and a clean PDF is copied byte-for-byte to the distinct sink.
/// The direct builder report counts bytes read by `begin`; `copy_pdf_range`
/// also counts its preceding index scan.
pub struct PdfOutlineAppender<'a, W: Write, C: Cancellation> {
    writer: AppendWriter<'a, W, C>,
    index: &'a PdfIndex,
    limits: &'a Limits,
    outline: OutlineBuilder<PdfRef>,
    input_bytes_read: u64,
}

/// The source-independent preconditions of an outline append, shared by
/// every source, sink, and cancellation type.
fn check_append_range(index: &PdfIndex, limits: &Limits, source_size: u64) -> Result<()> {
    limits.validate()?;
    let range = index.range();
    if range.length > limits.max_input_bytes {
        return Err(
            Error::limit("input bytes", limits.max_input_bytes, range.length)
                .at(range.offset)
                .in_pdf(Some((index.catalog().number, index.catalog().generation))),
        );
    }
    let range_end = range
        .end()
        .ok_or(Error::invalid("PDF source range end overflows"))?;
    if range_end > source_size || index.logical_end() > range.length {
        return Err(Error::invalid("PDF index range exceeds source"));
    }
    if index.logical_end() > limits.max_output_bytes {
        return Err(
            Error::limit("output bytes", limits.max_output_bytes, index.logical_end())
                .at(range.offset.saturating_add(index.xref_offset()))
                .in_pdf(Some((index.catalog().number, index.catalog().generation))),
        );
    }
    Ok(())
}

impl<'a, W: Write, C: Cancellation> PdfOutlineAppender<'a, W, C> {
    /// Inspect before calling this method, then retain the index until finish.
    pub fn begin<R: RangedSource>(
        source: &mut R,
        sink: &'a mut W,
        index: &'a PdfIndex,
        limits: &'a Limits,
        cancellation: &'a C,
    ) -> Result<Self> {
        check_append_range(index, limits, source.size())?;
        let range = index.range();
        let mut writer = AppendWriter::new(sink, index, limits, cancellation);
        copy_prefix(
            source,
            &mut writer,
            range.offset,
            index.logical_end(),
            index,
            limits,
            cancellation,
        )?;
        Ok(Self {
            writer,
            index,
            limits,
            outline: OutlineBuilder::new(),
            input_bytes_read: index.logical_end(),
        })
    }

    /// True when the original nonempty outline will be preserved unchanged.
    pub fn preserves_existing_outlines(&self) -> bool {
        self.index.has_outlines()
    }

    /// Accept one bookmark in depth-first document order.
    ///
    /// A failure after validation, once earlier items may have been closed,
    /// makes later `add_bookmark` and `finish` calls fail.
    pub fn add_bookmark(&mut self, bookmark: Bookmark) -> Result<()> {
        if self.index.has_outlines() {
            return Ok(());
        }
        self.writer.out.ensure_healthy()?;
        self.outline.ensure_intact()?;
        let next_count = self
            .outline
            .written()
            .checked_add(1)
            .ok_or(Error::invalid("PDF bookmark count overflows"))?;
        self.limits.check_bookmarks(next_count)?;
        let page = *self
            .index
            .pages()
            .get(bookmark.page_index as usize)
            .ok_or(Error::invalid("bookmark page is outside the PDF page tree"))?;
        let depth = usize_from_u32(bookmark.depth);
        if depth >= MAX_OUTLINE_DEPTH {
            return Err(Error::limit(
                "PDF outline depth",
                MAX_OUTLINE_DEPTH as u64,
                depth as u64 + 1,
            ));
        }
        self.outline.check_depth(depth)?;
        if bookmark.title.is_empty() {
            return Err(Error::invalid("bookmark title is empty"));
        }
        self.outline.add(
            &mut self.writer,
            self.limits,
            depth,
            page,
            BookmarkView::Fit,
            bookmark.title,
        )
    }

    /// Finish any repair and outline update, then flush the output sink.
    ///
    /// Fails without writing if an earlier `add_bookmark` failed after it
    /// began closing outline items.
    pub fn finish(mut self) -> Result<ConversionReport> {
        self.writer.out.ensure_healthy()?;
        self.outline.ensure_intact()?;
        let outline_root = self.outline.finish(&mut self.writer)?;
        for repair in self.index.repair_objects() {
            self.writer.begin_object(repair.reference)?;
            self.writer.out.write(&repair.body)?;
            self.writer.end_object()?;
        }
        if let Some(outline_root) = outline_root {
            self.write_catalog(outline_root)?;
        }
        if self.writer.entries.is_empty() {
            self.writer.finish_copy()?;
        } else {
            self.writer.finish_update()?;
        }
        Ok(ConversionReport {
            input_bytes_read: self.input_bytes_read,
            output_bytes_written: self.writer.out.position,
            // `PdfIndex::open` visits each page object once, and object
            // numbers are at most `MAX_PDF_OBJECTS`, so the count fits `u32`.
            pages_converted: self.index.pages().len() as u32,
            bookmarks_written: self.outline.written(),
            ..ConversionReport::default()
        })
    }

    fn write_catalog(&mut self, outline_root: PdfRef) -> Result<()> {
        let dictionary = self.index.catalog_dictionary();
        self.writer.begin_object(self.index.catalog())?;
        self.writer.out.write(b"<<")?;
        for entry in self.index.catalog_entries() {
            if entry.name() == b"Outlines" {
                continue;
            }
            self.writer.out.write(b" ")?;
            self.writer.out.write(entry.raw_pair(dictionary))?;
        }
        self.writer.out.write(
            format!(
                " /Outlines {} {} R >>",
                outline_root.number, outline_root.generation
            )
            .as_bytes(),
        )?;
        self.writer.end_object()
    }
}

fn copy_prefix<R: RangedSource, W: Write, C: Cancellation>(
    source: &mut R,
    writer: &mut AppendWriter<'_, W, C>,
    offset: u64,
    length: u64,
    index: &PdfIndex,
    limits: &Limits,
    cancellation: &C,
) -> Result<()> {
    let chunk = length.min(limits.io_chunk_bytes as u64) as usize;
    limits.check_allocation(chunk as u64)?;
    let mut buffer = Vec::new();
    let refused = limits.allocation_refused("PDF copy buffer allocation", chunk as u64);
    reserve_exact(&mut buffer, chunk, refused)?;
    buffer.resize(chunk, 0);
    let mut patches = CopyPatches::new(index);
    let mut done = 0;
    while done < length {
        let count = (length - done).min(chunk as u64) as usize;
        let at = offset
            .checked_add(done)
            .ok_or(Error::invalid("PDF copy offset overflows"))?;
        read_exact_at(source, at, &mut buffer[..count], limits, cancellation)?;
        patches.apply(&mut buffer[..count], done)?;
        writer.out.write_unbounded(&buffer[..count])?;
        done = done
            .checked_add(count as u64)
            .ok_or(Error::invalid("PDF copied-byte count overflows"))?;
    }
    patches.check_consumed()
}

/// The recorded source patches applied while copying a PDF prefix. The
/// patching needs no I/O, so it is shared by every source and sink type.
struct CopyPatches<'a> {
    separators: &'a [u64],
    gaps: &'a [GapPatch],
    next_separator: usize,
    next_gap: usize,
}

impl<'a> CopyPatches<'a> {
    fn new(index: &'a PdfIndex) -> Self {
        Self {
            separators: index.stream_separator_patches(),
            gaps: index.gap_patches(),
            next_separator: 0,
            next_gap: 0,
        }
    }

    /// Patch one copied chunk that starts `done` bytes into the prefix.
    fn apply(&mut self, buffer: &mut [u8], done: u64) -> Result<()> {
        let end = done + buffer.len() as u64;
        while let Some(&patch_at) = self.separators.get(self.next_separator) {
            if patch_at >= end {
                break;
            }
            // `patch_at < end`, so the patch lies inside this chunk.
            let within = patch_at.checked_sub(done).ok_or(Error::invalid(
                "PDF stream separator patches are not sorted",
            ))? as usize;
            if buffer[within] != b'\r' {
                return Err(Error::invalid(
                    "PDF stream separator changed after inspection",
                ));
            }
            buffer[within] = b'\n';
            self.next_separator += 1;
        }
        while let Some(gap) = self.gaps.get(self.next_gap) {
            let gap_end = gap
                .offset
                .checked_add(gap.original.len() as u64)
                .ok_or(Error::invalid("PDF orphan gap patch overflows"))?;
            if gap.offset >= end {
                break;
            }
            let overlap_start = gap.offset.max(done);
            let overlap_end = gap_end.min(end);
            if overlap_start < overlap_end {
                let source_start = (overlap_start - done) as usize;
                let source_end = (overlap_end - done) as usize;
                let original_start = (overlap_start - gap.offset) as usize;
                let original_end = (overlap_end - gap.offset) as usize;
                if buffer[source_start..source_end] != gap.original[original_start..original_end] {
                    return Err(Error::invalid("PDF orphan gap changed after inspection"));
                }
                buffer[source_start..source_end].fill(b' ');
            }
            if gap_end > end {
                break;
            }
            self.next_gap += 1;
        }
        Ok(())
    }

    fn check_consumed(&self) -> Result<()> {
        if self.next_separator != self.separators.len() {
            return Err(Error::invalid(
                "PDF stream separator patch exceeds copied prefix",
            ));
        }
        if self.next_gap != self.gaps.len() {
            return Err(Error::invalid("PDF orphan gap patch exceeds copied prefix"));
        }
        Ok(())
    }
}

struct AppendWriter<'a, W: Write, C: Cancellation> {
    out: Output<'a, W, C>,
    index: &'a PdfIndex,
    /// The next new object number, once the first one has been reserved.
    next_number: Option<u32>,
    entries: Vec<XrefEntry>,
    open: bool,
}

impl<'a, W: Write, C: Cancellation> AppendWriter<'a, W, C> {
    fn new(sink: &'a mut W, index: &'a PdfIndex, limits: &'a Limits, cancellation: &'a C) -> Self {
        Self {
            out: Output::new(sink, limits, cancellation),
            index,
            next_number: None,
            entries: Vec::new(),
            open: false,
        }
    }

    fn reserve_entry(&mut self) -> Result<()> {
        let next = self
            .entries
            .len()
            .checked_add(1)
            .ok_or(Error::invalid("PDF append xref entry count overflows"))?;
        if next > self.entries.capacity() {
            let cap = self.entries.capacity().max(4).saturating_mul(2).max(next);
            let bytes = (cap as u64)
                .checked_mul(size_of::<XrefEntry>() as u64)
                .ok_or(Error::invalid("PDF append xref allocation overflows"))?;
            let limits = self.out.limits;
            limits.check_allocation(bytes)?;
            let refused = limits.allocation_refused("PDF append xref allocation", bytes);
            let additional = cap - self.entries.len();
            reserve_exact(&mut self.entries, additional, refused)?;
        }
        Ok(())
    }

    fn begin_object(&mut self, reference: PdfRef) -> Result<()> {
        self.out.ensure_healthy()?;
        if self.open || reference.number == 0 {
            return Err(Error::invalid("invalid PDF append object state or number"));
        }
        self.reserve_entry()?;
        self.out.write(b"\n")?;
        let offset = self.out.position;
        self.out
            .write(format!("{} {} obj\n", reference.number, reference.generation).as_bytes())?;
        self.entries.push(XrefEntry { reference, offset });
        self.open = true;
        Ok(())
    }

    fn end_object(&mut self) -> Result<()> {
        if !self.open {
            return Err(Error::invalid("no PDF append object is open"));
        }
        self.out.write(b"\nendobj\n")?;
        self.open = false;
        Ok(())
    }

    fn ensure_closed(&self) -> Result<()> {
        if self.open {
            return Err(Error::invalid("PDF append object remains open"));
        }
        Ok(())
    }

    fn finish_copy(&mut self) -> Result<()> {
        self.ensure_closed()?;
        self.out.flush()
    }

    fn finish_update(&mut self) -> Result<()> {
        self.ensure_closed()?;
        self.out.ensure_healthy()?;
        self.entries
            .sort_unstable_by_key(|entry| entry.reference.number);
        for pair in self.entries.windows(2) {
            if pair[0].reference.number == pair[1].reference.number {
                return Err(Error::invalid("PDF update defines an object twice"));
            }
        }
        let highest = self
            .entries
            .last()
            .ok_or(Error::invalid("PDF update has no xref entries"))?
            .reference
            .number;
        let index = self.index;
        let size = index.trailer_size().max(
            highest
                .checked_add(1)
                .ok_or(Error::invalid("PDF trailer Size overflows"))?,
        );
        let id = index.trailer_first_id().map(|first| {
            let second = update_id(
                index.trailer_id().unwrap_or_default(),
                index.logical_end(),
                self.out.position,
            );
            (first, second)
        });
        let trailer = Trailer {
            size: u64::from(size),
            root: index.catalog(),
            prev: Some(index.xref_offset()),
            info: index.trailer_info(),
            id,
        };
        let entries = self
            .entries
            .iter()
            .map(|entry| (entry.reference, entry.offset));
        write_xref(&mut self.out, entries, false, &trailer)?;
        self.out.flush()
    }
}

impl<W: Write, C: Cancellation> ObjectSink for AppendWriter<'_, W, C> {
    type Ref = PdfRef;

    fn begin_object(&mut self, reference: PdfRef) -> Result<()> {
        AppendWriter::begin_object(self, reference)
    }

    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.out.write(bytes)
    }

    fn end_object(&mut self) -> Result<()> {
        AppendWriter::end_object(self)
    }
}

impl<W: Write, C: Cancellation> ObjectAllocator for AppendWriter<'_, W, C> {
    /// Number new objects from the input's first free number.
    fn reserve(&mut self) -> Result<PdfRef> {
        let number = match self.next_number {
            Some(number) => number,
            None => self.index.next_free_object_number()?,
        };
        if number == 0 || number > MAX_PDF_OBJECTS {
            return Err(Error::limit(
                "PDF object number",
                u64::from(MAX_PDF_OBJECTS),
                u64::from(number),
            ));
        }
        self.next_number = Some(
            number
                .checked_add(1)
                .ok_or(Error::invalid("PDF object number overflows"))?,
        );
        Ok(PdfRef {
            number,
            generation: 0,
        })
    }
}

/// The second `/ID` string of an update: FNV-128 over the old `/ID` array,
/// then the little-endian copied prefix length and xref offset.
fn update_id(old_id: &[u8], prefix_length: u64, xref_offset: u64) -> u128 {
    [
        old_id,
        &prefix_length.to_le_bytes(),
        &xref_offset.to_le_bytes(),
    ]
    .into_iter()
    .flatten()
    .fold(FNV128_BASIS, |hash, byte| {
        (hash ^ u128::from(*byte)).wrapping_mul(FNV128_PRIME)
    })
}

#[cfg(test)]
mod tests;
