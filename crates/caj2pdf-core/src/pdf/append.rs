// SPDX-License-Identifier: MIT

//! Bounded pass-through and append-only outline updates for inspected PDFs.
//!
//! This module never reserializes page contents. The reader validates and
//! indexes the input first; the appender copies its PDF prefix and writes only
//! changed dictionaries, new outline items, a sparse xref, and a new trailer.

use super::input::{GapPatch, PdfIndex};
use super::types::{PdfRange, PdfRef};
use super::writer::{MAX_CLASSIC_PDF_BYTES, MAX_PDF_OBJECTS, check_classic_pdf_bytes};
use crate::fallible::{checked_read_count, reserve_exact, try_convert, usize_from_u32};
use crate::{
    Bookmark, Cancellation, ConversionReport, Error, Limits, PdfErrorKind, RangedSource, Result,
    SequentialSink, read_exact_at, write_all,
};
use std::mem::size_of;

const HEX: &[u8; 16] = b"0123456789ABCDEF";
/// An explicit depth cap keeps per-bookmark stack work bounded.
const MAX_OUTLINE_DEPTH: usize = 256;
// The second document ID is a content-derived version marker, not a signature
// or cryptographic digest. The first ID is retained exactly as supplied.
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
pub async fn copy_pdf<R: RangedSource, W: SequentialSink, C: Cancellation>(
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
    .await
}

/// Copy a PDF held in a bounded region of a larger random-access input.
pub async fn copy_pdf_range<R: RangedSource, W: SequentialSink, C: Cancellation>(
    source: &mut R,
    sink: &mut W,
    range: PdfRange,
    limits: &Limits,
    cancellation: &C,
) -> Result<ConversionReport> {
    let mut counted = CountingSource {
        inner: source,
        bytes_read: 0,
    };
    let index = PdfIndex::open(&mut counted, range, limits, cancellation).await?;
    let mut report = PdfOutlineAppender::begin(&mut counted, sink, &index, limits, cancellation)
        .await?
        .finish()
        .await?;
    report.input_bytes_read = counted.bytes_read;
    Ok(report)
}

const OVERREAD: &str = "PDF source reported more bytes than requested";

struct CountingSource<'a, R> {
    inner: &'a mut R,
    bytes_read: u64,
}

impl<R: RangedSource> RangedSource for CountingSource<'_, R> {
    fn size(&self) -> u64 {
        self.inner.size()
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        let read = self.inner.read_at(offset, destination).await?;
        let read = checked_read_count(read, destination.len(), OVERREAD)?;
        self.bytes_read = self
            .bytes_read
            .checked_add(read as u64)
            .ok_or(Error::InvalidInput {
                reason: "PDF input byte count overflows",
            })?;
        Ok(read)
    }
}

#[derive(Default)]
struct ChildLinks {
    first: Option<PdfRef>,
    last: Option<PdfRef>,
}

struct OpenOutline {
    reference: PdfRef,
    parent: PdfRef,
    previous: Option<PdfRef>,
    page: PdfRef,
    title: String,
    ordinal: u32,
    children: ChildLinks,
}

struct ClosedOutline {
    reference: PdfRef,
    parent: PdfRef,
    previous: Option<PdfRef>,
    page: PdfRef,
    title: String,
    first_child: Option<PdfRef>,
    last_child: Option<PdfRef>,
    descendants: u32,
}

/// Append CAJ bookmark entries to a previously inspected PDF.
///
/// `begin` copies the PDF prefix before accepting bookmarks. A caller-owned
/// sink can therefore contain partial output if a later input or sink error
/// occurs; callers needing atomic path output should stage and rename it.
/// An existing nonempty outline tree is preserved: `add_bookmark` becomes a
/// no-op, and a clean PDF is copied byte-for-byte to the distinct sink.
/// The direct builder report counts bytes read by `begin`; `copy_pdf_range`
/// also counts its preceding index scan.
pub struct PdfOutlineAppender<'a, W: SequentialSink, C: Cancellation> {
    writer: AppendWriter<'a, W, C>,
    index: &'a PdfIndex,
    limits: &'a Limits,
    next_number: Option<u32>,
    outline_root: Option<PdfRef>,
    root_links: ChildLinks,
    open: Vec<OpenOutline>,
    bookmarks_written: u32,
    retained_titles: u64,
    retained_nodes: u32,
    input_bytes_read: u64,
}

/// The source-independent preconditions of an outline append, shared by
/// every source, sink, and cancellation type.
fn check_append_range(index: &PdfIndex, limits: &Limits, source_size: u64) -> Result<()> {
    limits.validate()?;
    let range = index.range();
    if range.length > limits.max_input_bytes {
        return Err(Error::PdfLimitExceeded {
            offset: range.offset,
            object: Some((index.catalog().number, index.catalog().generation)),
            resource: "input bytes",
            limit: limits.max_input_bytes,
            attempted: range.length,
        });
    }
    let range_end = range.end().ok_or(Error::InvalidInput {
        reason: "PDF source range end overflows",
    })?;
    if range_end > source_size || index.logical_end() > range.length {
        return Err(Error::InvalidInput {
            reason: "PDF index range exceeds source",
        });
    }
    if index.logical_end() > limits.max_output_bytes {
        return Err(Error::PdfLimitExceeded {
            offset: range.offset.saturating_add(index.xref_offset()),
            object: Some((index.catalog().number, index.catalog().generation)),
            resource: "output bytes",
            limit: limits.max_output_bytes,
            attempted: index.logical_end(),
        });
    }
    Ok(())
}

impl<'a, W: SequentialSink, C: Cancellation> PdfOutlineAppender<'a, W, C> {
    /// Inspect before calling this method, then retain the index until finish.
    pub async fn begin<R: RangedSource>(
        source: &mut R,
        sink: &'a mut W,
        index: &'a PdfIndex,
        limits: &'a Limits,
        cancellation: &'a C,
    ) -> Result<Self> {
        check_append_range(index, limits, source.size())?;
        let range = index.range();
        let mut writer = AppendWriter::new(sink, limits, cancellation);
        copy_prefix(
            source,
            &mut writer,
            range.offset,
            index.logical_end(),
            index,
            limits,
            cancellation,
        )
        .await?;
        writer.start_update_hash(index.trailer_id());
        Ok(Self {
            writer,
            index,
            limits,
            next_number: None,
            outline_root: None,
            root_links: ChildLinks::default(),
            open: Vec::new(),
            bookmarks_written: 0,
            retained_titles: 0,
            retained_nodes: 0,
            input_bytes_read: index.logical_end(),
        })
    }

    /// True when the original nonempty outline will be preserved unchanged.
    pub fn preserves_existing_outlines(&self) -> bool {
        self.index.has_outlines()
    }

    /// Accept one bookmark in depth-first document order.
    pub async fn add_bookmark(&mut self, bookmark: Bookmark) -> Result<()> {
        if self.index.has_outlines() {
            return Ok(());
        }
        self.writer.ensure_healthy()?;
        let next_count = self
            .bookmarks_written
            .checked_add(1)
            .ok_or(Error::InvalidInput {
                reason: "PDF bookmark count overflows",
            })?;
        self.limits.check_bookmarks(next_count)?;
        let page =
            *self
                .index
                .pages()
                .get(bookmark.page_index as usize)
                .ok_or(Error::InvalidInput {
                    reason: "bookmark page is outside the PDF page tree",
                })?;
        let depth = usize_from_u32(bookmark.depth);
        if depth >= MAX_OUTLINE_DEPTH {
            return Err(Error::LimitExceeded {
                resource: "PDF outline depth",
                limit: MAX_OUTLINE_DEPTH as u64,
                attempted: depth as u64 + 1,
            });
        }
        if depth > self.open.len() {
            return Err(Error::InvalidInput {
                reason: "bookmark depth skips a parent",
            });
        }
        if bookmark.title.is_empty() {
            return Err(Error::InvalidInput {
                reason: "bookmark title is empty",
            });
        }
        self.preflight_outline_memory(depth, bookmark.title.capacity() as u64)?;
        let root = if let Some(root) = self.outline_root {
            root
        } else {
            let root = self.reserve_new_object()?;
            self.outline_root = Some(root);
            root
        };
        let reference = self.reserve_new_object()?;
        if let Some(previous) = self.close_outlines_to(depth).await? {
            self.emit_outline_item(previous, Some(reference)).await?;
        }
        let parent = if depth == 0 {
            root
        } else {
            self.open[depth - 1].reference
        };
        let links = if depth == 0 {
            &mut self.root_links
        } else {
            &mut self.open[depth - 1].children
        };
        let previous = links.last;
        if links.first.is_none() {
            links.first = Some(reference);
        }
        links.last = Some(reference);
        self.retained_titles = self
            .retained_titles
            .checked_add(bookmark.title.capacity() as u64)
            .ok_or(Error::InvalidInput {
                reason: "retained PDF bookmark title bytes overflow",
            })?;
        self.retained_nodes = self
            .retained_nodes
            .checked_add(1)
            .ok_or(Error::InvalidInput {
                reason: "retained PDF bookmark count overflows",
            })?;
        self.open.push(OpenOutline {
            reference,
            parent,
            previous,
            page,
            title: bookmark.title,
            ordinal: self.bookmarks_written,
            children: ChildLinks::default(),
        });
        self.bookmarks_written = next_count;
        Ok(())
    }

    /// Finish any repair and outline update, then flush the output sink.
    pub async fn finish(mut self) -> Result<ConversionReport> {
        if let Some(last_root) = self.close_outlines_to(0).await? {
            self.emit_outline_item(last_root, None).await?;
        }
        if let Some(root) = self.outline_root {
            let first = self.root_links.first.ok_or(Error::InvalidInput {
                reason: "PDF outline root has no first item",
            })?;
            let last = self.root_links.last.ok_or(Error::InvalidInput {
                reason: "PDF outline root has no last item",
            })?;
            self.writer.begin_object(root).await?;
            self.writer
                .write_raw(
                    format!(
                        "<< /Type /Outlines /First {} /Last {} /Count {} >>",
                        pdf_ref(first),
                        pdf_ref(last),
                        self.bookmarks_written
                    )
                    .as_bytes(),
                )
                .await?;
            self.writer.end_object().await?;
        }
        for repair in self.index.repair_objects() {
            self.writer.begin_object(repair.reference).await?;
            self.writer.write_raw(&repair.body).await?;
            self.writer.end_object().await?;
        }
        if let Some(outline_root) = self.outline_root {
            self.write_catalog(outline_root).await?;
        }
        if self.writer.entries.is_empty() {
            self.writer.finish_copy().await?;
        } else {
            self.writer.finish_update(self.index).await?;
        }
        Ok(ConversionReport {
            input_bytes_read: self.input_bytes_read,
            output_bytes_written: self.writer.position,
            // `PdfIndex::open` visits each page object once, and object
            // numbers are at most `MAX_PDF_OBJECTS`, so the count fits `u32`.
            pages_converted: self.index.pages().len() as u32,
            bookmarks_written: self.bookmarks_written,
        })
    }

    fn reserve_new_object(&mut self) -> Result<PdfRef> {
        let number = if let Some(number) = self.next_number {
            number
        } else {
            self.index.next_free_object_number()?
        };
        if number == 0 || number > MAX_PDF_OBJECTS {
            return Err(Error::LimitExceeded {
                resource: "PDF object number",
                limit: u64::from(MAX_PDF_OBJECTS),
                attempted: u64::from(number),
            });
        }
        self.next_number = Some(number.checked_add(1).ok_or(Error::InvalidInput {
            reason: "PDF object number overflows",
        })?);
        Ok(PdfRef {
            number,
            generation: 0,
        })
    }

    fn preflight_outline_memory(&mut self, depth: usize, title_capacity: u64) -> Result<()> {
        let mut released_titles = 0_u64;
        let mut released_nodes = 0_u32;
        // Released items are a subset of the retained ones, whose title
        // capacities and count already sum to `retained_titles` and
        // `retained_nodes` without overflow.
        let mut release = |title: &String| {
            released_titles += title.capacity() as u64;
            released_nodes += 1;
        };
        // Every item that inserting at `depth` closes is written before the
        // new item is retained; no other closed item is ever held unwritten.
        for item in self.open.iter().skip(depth) {
            release(&item.title);
        }
        let titles = self
            .retained_titles
            .checked_sub(released_titles)
            .and_then(|n| n.checked_add(title_capacity))
            .ok_or(Error::InvalidInput {
                reason: "PDF bookmark title budget overflows",
            })?;
        let nodes = self
            .retained_nodes
            .checked_sub(released_nodes)
            .and_then(|n| n.checked_add(1))
            .ok_or(Error::InvalidInput {
                reason: "PDF bookmark node budget overflows",
            })?;
        let metadata = u64::from(nodes)
            .checked_mul(size_of::<OpenOutline>() as u64)
            .and_then(|n| n.checked_add(titles))
            .ok_or(Error::InvalidInput {
                reason: "PDF bookmark allocation overflows",
            })?;
        self.limits.check_allocation(metadata)?;
        if depth == self.open.len() && self.open.len() == self.open.capacity() {
            let cap = self.open.capacity().max(1).saturating_mul(2);
            let max = self.limits.max_bookmarks as usize;
            let next_cap = cap.min(max).max(self.open.len() + 1);
            let bytes = (next_cap as u64) * size_of::<OpenOutline>() as u64;
            self.limits.check_allocation(bytes)?;
            let refused = self
                .limits
                .allocation_refused("PDF bookmark stack allocation", bytes);
            let additional = next_cap - self.open.len();
            reserve_exact(&mut self.open, additional, refused)?;
        }
        Ok(())
    }

    /// Close every open item deeper than `depth` and return the last one
    /// closed, which is the item at `depth` itself when one was open.
    ///
    /// Each closed item is the last child of the next item closed, which
    /// writes it; the caller writes the returned item once its `/Next` link
    /// is known. Because the only unwritten closed item is carried here
    /// rather than stored on its parent's links, a sibling can never be
    /// left unwritten.
    async fn close_outlines_to(&mut self, depth: usize) -> Result<Option<ClosedOutline>> {
        let mut closed = None;
        while let Some(item) = super::pop_deeper_than(&mut self.open, depth) {
            closed = Some(self.close_outline(item, closed).await?);
        }
        Ok(closed)
    }

    async fn close_outline(
        &mut self,
        item: OpenOutline,
        last_child: Option<ClosedOutline>,
    ) -> Result<ClosedOutline> {
        if let Some(last_child) = last_child {
            self.emit_outline_item(last_child, None).await?;
        }
        let descendants =
            self.bookmarks_written
                .checked_sub(item.ordinal + 1)
                .ok_or(Error::InvalidInput {
                    reason: "PDF bookmark descendant count underflows",
                })?;
        Ok(ClosedOutline {
            reference: item.reference,
            parent: item.parent,
            previous: item.previous,
            page: item.page,
            title: item.title,
            first_child: item.children.first,
            last_child: item.children.last,
            descendants,
        })
    }

    async fn emit_outline_item(&mut self, item: ClosedOutline, next: Option<PdfRef>) -> Result<()> {
        self.writer.begin_object(item.reference).await?;
        self.writer.write_raw(b"<< /Title <FEFF").await?;
        let mut hex = [0_u8; 4096];
        let mut used = 0;
        for unit in item.title.encode_utf16() {
            if used == hex.len() {
                self.writer.write_raw(&hex).await?;
                used = 0;
            }
            for byte in unit.to_be_bytes() {
                hex[used] = HEX[(byte >> 4) as usize];
                hex[used + 1] = HEX[(byte & 0x0f) as usize];
                used += 2;
            }
        }
        // Bookmark titles are nonempty, so the final chunk is too.
        debug_assert!(used != 0);
        self.writer.write_raw(&hex[..used]).await?;
        self.writer
            .write_raw(
                format!(
                    "> /Parent {} /Dest [{} /Fit]",
                    pdf_ref(item.parent),
                    pdf_ref(item.page)
                )
                .as_bytes(),
            )
            .await?;
        if let Some(previous) = item.previous {
            self.writer
                .write_raw(format!(" /Prev {}", pdf_ref(previous)).as_bytes())
                .await?;
        }
        if let Some(next) = next {
            self.writer
                .write_raw(format!(" /Next {}", pdf_ref(next)).as_bytes())
                .await?;
        }
        if let Some(first) = item.first_child {
            let last = item.last_child.ok_or(Error::InvalidInput {
                reason: "PDF bookmark has a first child but no last child",
            })?;
            self.writer
                .write_raw(
                    format!(
                        " /First {} /Last {} /Count {}",
                        pdf_ref(first),
                        pdf_ref(last),
                        item.descendants
                    )
                    .as_bytes(),
                )
                .await?;
        }
        self.writer.write_raw(b" >>").await?;
        self.writer.end_object().await?;
        self.retained_titles = self
            .retained_titles
            .checked_sub(item.title.capacity() as u64)
            .ok_or(Error::InvalidInput {
                reason: "retained PDF bookmark title bytes underflow",
            })?;
        self.retained_nodes = self
            .retained_nodes
            .checked_sub(1)
            .ok_or(Error::InvalidInput {
                reason: "retained PDF bookmark node count underflows",
            })?;
        Ok(())
    }

    async fn write_catalog(&mut self, outline_root: PdfRef) -> Result<()> {
        let dictionary = self.index.catalog_dictionary();
        self.writer.begin_object(self.index.catalog()).await?;
        self.writer.write_raw(b"<<").await?;
        for entry in self.index.catalog_entries() {
            if entry.name() == b"Outlines" {
                continue;
            }
            self.writer.write_raw(b" ").await?;
            self.writer.write_raw(entry.raw_pair(dictionary)).await?;
        }
        self.writer
            .write_raw(format!(" /Outlines {} >>", pdf_ref(outline_root)).as_bytes())
            .await?;
        self.writer.end_object().await
    }
}

fn pdf_ref(reference: PdfRef) -> String {
    format!("{} {} R", reference.number, reference.generation)
}

async fn copy_prefix<R: RangedSource, W: SequentialSink, C: Cancellation>(
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
        let at = offset.checked_add(done).ok_or(Error::InvalidInput {
            reason: "PDF copy offset overflows",
        })?;
        read_exact_at(source, at, &mut buffer[..count], limits, cancellation).await?;
        patches.apply(&mut buffer[..count], done)?;
        writer.write_raw(&buffer[..count]).await?;
        done = done.checked_add(count as u64).ok_or(Error::InvalidInput {
            reason: "PDF copied-byte count overflows",
        })?;
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
            let within: usize = try_convert(
                patch_at.checked_sub(done).ok_or(Error::InvalidInput {
                    reason: "PDF stream separator patches are not sorted",
                })?,
                Error::InvalidInput {
                    reason: "PDF stream separator patch exceeds chunk",
                },
            )?;
            if buffer[within] != b'\r' {
                return Err(Error::InvalidInput {
                    reason: "PDF stream separator changed after inspection",
                });
            }
            buffer[within] = b'\n';
            self.next_separator += 1;
        }
        while let Some(gap) = self.gaps.get(self.next_gap) {
            let gap_end =
                gap.offset
                    .checked_add(gap.original.len() as u64)
                    .ok_or(Error::InvalidInput {
                        reason: "PDF orphan gap patch overflows",
                    })?;
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
                    return Err(Error::InvalidInput {
                        reason: "PDF orphan gap changed after inspection",
                    });
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
            return Err(Error::InvalidInput {
                reason: "PDF stream separator patch exceeds copied prefix",
            });
        }
        if self.next_gap != self.gaps.len() {
            return Err(Error::InvalidInput {
                reason: "PDF orphan gap patch exceeds copied prefix",
            });
        }
        Ok(())
    }
}

struct AppendWriter<'a, W: SequentialSink, C: Cancellation> {
    sink: &'a mut W,
    limits: &'a Limits,
    cancellation: &'a C,
    position: u64,
    entries: Vec<XrefEntry>,
    open: bool,
    poisoned: bool,
    update_hash: Option<u128>,
}

impl<'a, W: SequentialSink, C: Cancellation> AppendWriter<'a, W, C> {
    fn new(sink: &'a mut W, limits: &'a Limits, cancellation: &'a C) -> Self {
        Self {
            sink,
            limits,
            cancellation,
            position: 0,
            entries: Vec::new(),
            open: false,
            poisoned: false,
            update_hash: None,
        }
    }

    fn ensure_healthy(&self) -> Result<()> {
        if self.poisoned {
            Err(Error::InvalidInput {
                reason: "PDF append writer cannot continue after an output failure",
            })
        } else {
            Ok(())
        }
    }

    fn start_update_hash(&mut self, old_id: Option<&[u8]>) {
        let mut hash = FNV128_BASIS;
        if let Some(old_id) = old_id {
            hash = fnv128(hash, old_id);
        }
        hash = fnv128(hash, &self.position.to_le_bytes());
        self.update_hash = Some(hash);
    }

    async fn write_raw(&mut self, bytes: &[u8]) -> Result<()> {
        self.ensure_healthy()?;
        let attempted =
            self.position
                .checked_add(bytes.len() as u64)
                .ok_or(Error::InvalidInput {
                    reason: "PDF appended byte count overflows",
                })?;
        if self.update_hash.is_some() {
            check_classic_pdf_bytes(attempted)?;
        }
        let result = write_all(
            self.sink,
            bytes,
            &mut self.position,
            self.limits,
            self.cancellation,
        )
        .await;
        if result.is_err() {
            self.poisoned = true;
            return result;
        }
        if let Some(hash) = &mut self.update_hash {
            *hash = fnv128(*hash, bytes);
        }
        Ok(())
    }

    fn reserve_entry(&mut self) -> Result<()> {
        let next = self
            .entries
            .len()
            .checked_add(1)
            .ok_or(Error::InvalidInput {
                reason: "PDF append xref entry count overflows",
            })?;
        if next > self.entries.capacity() {
            let cap = self.entries.capacity().max(4).saturating_mul(2).max(next);
            let bytes = (cap as u64)
                .checked_mul(size_of::<XrefEntry>() as u64)
                .ok_or(Error::InvalidInput {
                    reason: "PDF append xref allocation overflows",
                })?;
            self.limits.check_allocation(bytes)?;
            let refused = self
                .limits
                .allocation_refused("PDF append xref allocation", bytes);
            let additional = cap - self.entries.len();
            reserve_exact(&mut self.entries, additional, refused)?;
        }
        Ok(())
    }

    async fn begin_object(&mut self, reference: PdfRef) -> Result<()> {
        self.ensure_healthy()?;
        if self.open || reference.number == 0 {
            return Err(Error::InvalidInput {
                reason: "invalid PDF append object state or number",
            });
        }
        self.reserve_entry()?;
        self.write_raw(b"\n").await?;
        let offset = self.position;
        self.write_raw(format!("{} {} obj\n", reference.number, reference.generation).as_bytes())
            .await?;
        self.entries.push(XrefEntry { reference, offset });
        self.open = true;
        Ok(())
    }

    async fn end_object(&mut self) -> Result<()> {
        if !self.open {
            return Err(Error::InvalidInput {
                reason: "no PDF append object is open",
            });
        }
        self.write_raw(b"\nendobj\n").await?;
        self.open = false;
        Ok(())
    }

    async fn finish_copy(&mut self) -> Result<()> {
        if self.open {
            return Err(Error::InvalidInput {
                reason: "PDF append object remains open",
            });
        }
        self.ensure_healthy()?;
        if self.cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        self.sink.flush().await.inspect_err(|_| {
            self.poisoned = true;
        })?;
        if self.cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        Ok(())
    }

    async fn finish_update(&mut self, index: &PdfIndex) -> Result<()> {
        if self.open {
            return Err(Error::InvalidInput {
                reason: "PDF append object remains open",
            });
        }
        self.ensure_healthy()?;
        if self.position > MAX_CLASSIC_PDF_BYTES {
            return Err(Error::LimitExceeded {
                resource: "classic PDF file bytes",
                limit: MAX_CLASSIC_PDF_BYTES,
                attempted: self.position,
            });
        }
        self.entries
            .sort_unstable_by_key(|entry| entry.reference.number);
        for pair in self.entries.windows(2) {
            if pair[0].reference.number == pair[1].reference.number {
                return Err(Error::InvalidInput {
                    reason: "PDF update defines an object twice",
                });
            }
        }
        let xref_source_offset = index
            .range()
            .offset
            .checked_add(index.xref_offset())
            .ok_or(Error::InvalidInput {
                reason: "PDF xref source offset overflows",
            })?;
        let id_first = index
            .trailer_id()
            .map(|raw| first_id_string(raw, xref_source_offset))
            .transpose()?;
        let xref_offset = self.position;
        let entries = std::mem::take(&mut self.entries);
        self.write_raw(b"xref\n").await?;
        let mut cursor = 0;
        while cursor < entries.len() {
            let first = entries[cursor].reference.number;
            let mut end = cursor + 1;
            while end < entries.len()
                && entries[end].reference.number == entries[end - 1].reference.number + 1
            {
                end += 1;
            }
            self.write_raw(format!("{first} {}\n", end - cursor).as_bytes())
                .await?;
            for entry in &entries[cursor..end] {
                if entry.offset > MAX_CLASSIC_PDF_BYTES {
                    return Err(Error::LimitExceeded {
                        resource: "classic PDF object offset",
                        limit: MAX_CLASSIC_PDF_BYTES,
                        attempted: entry.offset,
                    });
                }
                let row = format!(
                    "{:010} {:05} n \n",
                    entry.offset, entry.reference.generation
                );
                debug_assert_eq!(row.len(), 20);
                self.write_raw(row.as_bytes()).await?;
            }
            cursor = end;
        }
        let highest = entries
            .last()
            .ok_or(Error::InvalidInput {
                reason: "PDF update has no xref entries",
            })?
            .reference
            .number;
        let size = index
            .trailer_size()
            .max(highest.checked_add(1).ok_or(Error::InvalidInput {
                reason: "PDF trailer Size overflows",
            })?);
        self.write_raw(
            format!(
                "trailer\n<< /Size {size} /Root {} /Prev {}",
                pdf_ref(index.catalog()),
                index.xref_offset()
            )
            .as_bytes(),
        )
        .await?;
        if let Some(info) = index.trailer_info() {
            self.write_raw(format!(" /Info {}", pdf_ref(info)).as_bytes())
                .await?;
        }
        if let Some(first) = id_first {
            let hash = self.update_hash.ok_or(Error::InvalidInput {
                reason: "PDF update hash is missing",
            })?;
            self.write_raw(b" /ID [").await?;
            self.write_raw(first).await?;
            self.write_raw(format!(" <{hash:032X}>]").as_bytes())
                .await?;
        }
        self.write_raw(format!(" >>\nstartxref\n{xref_offset}\n%%EOF\n").as_bytes())
            .await?;
        self.finish_copy().await
    }
}

fn fnv128(mut hash: u128, bytes: &[u8]) -> u128 {
    for byte in bytes {
        hash ^= u128::from(*byte);
        hash = hash.wrapping_mul(FNV128_PRIME);
    }
    hash
}

fn first_id_string(raw: &[u8], offset: u64) -> Result<&[u8]> {
    let mut cursor = 0;
    skip_pdf_space(raw, &mut cursor);
    if raw.get(cursor) != Some(&b'[') {
        return Err(unsupported_id(offset));
    }
    cursor += 1;
    skip_pdf_space(raw, &mut cursor);
    let first_start = cursor;
    skip_pdf_string(raw, &mut cursor).ok_or_else(|| unsupported_id(offset))?;
    let first = &raw[first_start..cursor];
    skip_pdf_space(raw, &mut cursor);
    skip_pdf_string(raw, &mut cursor).ok_or_else(|| unsupported_id(offset))?;
    skip_pdf_space(raw, &mut cursor);
    if raw.get(cursor) != Some(&b']') {
        return Err(unsupported_id(offset));
    }
    cursor += 1;
    skip_pdf_space(raw, &mut cursor);
    if cursor != raw.len() {
        return Err(unsupported_id(offset));
    }
    Ok(first)
}

fn unsupported_id(offset: u64) -> Error {
    Error::Pdf {
        offset,
        object: None,
        kind: PdfErrorKind::UnsupportedFeature,
        reason: "unsupported PDF trailer ID syntax",
    }
}

fn skip_pdf_space(raw: &[u8], cursor: &mut usize) {
    loop {
        while raw
            .get(*cursor)
            .is_some_and(|byte| matches!(byte, b'\0' | b'\t' | b'\n' | b'\x0c' | b'\r' | b' '))
        {
            *cursor += 1;
        }
        if raw.get(*cursor) == Some(&b'%') {
            while raw
                .get(*cursor)
                .is_some_and(|byte| *byte != b'\n' && *byte != b'\r')
            {
                *cursor += 1;
            }
        } else {
            break;
        }
    }
}

fn skip_pdf_string(raw: &[u8], cursor: &mut usize) -> Option<()> {
    match raw.get(*cursor)? {
        b'<' if raw.get(*cursor + 1) != Some(&b'<') => {
            *cursor += 1;
            while let Some(byte) = raw.get(*cursor) {
                *cursor += 1;
                if *byte == b'>' {
                    return Some(());
                }
                if !byte.is_ascii_hexdigit()
                    && !matches!(byte, b'\0' | b'\t' | b'\n' | b'\x0c' | b'\r' | b' ')
                {
                    return None;
                }
            }
            None
        }
        b'(' => {
            *cursor += 1;
            let mut depth = 1_u32;
            while let Some(byte) = raw.get(*cursor) {
                *cursor += 1;
                match *byte {
                    b'\\' => {
                        *cursor = cursor.checked_add(1)?;
                        if *cursor > raw.len() {
                            return None;
                        }
                    }
                    b'(' => depth = depth.checked_add(1)?,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(());
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests;
