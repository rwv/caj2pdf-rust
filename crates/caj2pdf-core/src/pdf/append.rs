// SPDX-License-Identifier: MIT

//! Bounded pass-through and append-only outline updates for inspected PDFs.
//!
//! This module never reserializes page contents. The reader validates and
//! indexes the input first; the appender copies its PDF prefix and writes only
//! changed dictionaries, new outline items, a sparse xref, and a new trailer.

use super::input::PdfIndex;
use super::types::{PdfRange, PdfRef};
use super::writer::{MAX_CLASSIC_PDF_BYTES, MAX_PDF_OBJECTS};
use crate::fallible::{reserve_exact, try_convert};
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
        if read > destination.len() {
            return Err(Error::InvalidInput {
                reason: "PDF source reported more bytes than requested",
            });
        }
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
    pending: Option<ClosedOutline>,
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

impl<'a, W: SequentialSink, C: Cancellation> PdfOutlineAppender<'a, W, C> {
    /// Inspect before calling this method, then retain the index until finish.
    pub async fn begin<R: RangedSource>(
        source: &mut R,
        sink: &'a mut W,
        index: &'a PdfIndex,
        limits: &'a Limits,
        cancellation: &'a C,
    ) -> Result<Self> {
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
        if range_end > source.size() || index.logical_end() > range.length {
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
        let depth: usize = try_convert(
            bookmark.depth,
            Error::InvalidInput {
                reason: "bookmark depth exceeds address space",
            },
        )?;
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
        while self.open.len() > depth {
            self.close_outline().await?;
        }
        let links = if depth == 0 {
            &mut self.root_links
        } else {
            &mut self.open[depth - 1].children
        };
        if let Some(previous) = links.pending.take() {
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
        while !self.open.is_empty() {
            self.close_outline().await?;
        }
        if let Some(last_root) = self.root_links.pending.take() {
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
            pages_converted: try_convert(
                self.index.pages().len(),
                Error::InvalidInput {
                    reason: "PDF page count exceeds 32 bits",
                },
            )?,
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
        let mut release = |title: &String| -> Result<()> {
            released_titles = released_titles.checked_add(title.capacity() as u64).ok_or(
                Error::InvalidInput {
                    reason: "released PDF bookmark title bytes overflow",
                },
            )?;
            released_nodes = released_nodes.checked_add(1).ok_or(Error::InvalidInput {
                reason: "released PDF bookmark count overflows",
            })?;
            Ok(())
        };
        for item in self.open.iter().skip(depth) {
            release(&item.title)?;
            if let Some(pending) = &item.children.pending {
                release(&pending.title)?;
            }
        }
        let pending = if depth == 0 {
            self.root_links.pending.as_ref()
        } else {
            self.open[depth - 1].children.pending.as_ref()
        };
        if let Some(pending) = pending {
            release(&pending.title)?;
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

    async fn close_outline(&mut self) -> Result<()> {
        let mut item = self.open.pop().ok_or(Error::InvalidInput {
            reason: "no open PDF bookmark to close",
        })?;
        if let Some(last_child) = item.children.pending.take() {
            self.emit_outline_item(last_child, None).await?;
        }
        let descendants =
            self.bookmarks_written
                .checked_sub(item.ordinal + 1)
                .ok_or(Error::InvalidInput {
                    reason: "PDF bookmark descendant count underflows",
                })?;
        let closed = ClosedOutline {
            reference: item.reference,
            parent: item.parent,
            previous: item.previous,
            page: item.page,
            title: item.title,
            first_child: item.children.first,
            last_child: item.children.last,
            descendants,
        };
        let links = match self.open.last_mut() {
            Some(parent) => &mut parent.children,
            None => &mut self.root_links,
        };
        if links.pending.is_some() {
            return Err(Error::InvalidInput {
                reason: "previous PDF bookmark sibling was not emitted",
            });
        }
        links.pending = Some(closed);
        Ok(())
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
        if used != 0 {
            self.writer.write_raw(&hex[..used]).await?;
        }
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
    let separator_patches = index.stream_separator_patches();
    let gap_patches = index.gap_patches();
    let mut done = 0;
    let mut next_patch = 0;
    let mut next_gap = 0;
    while done < length {
        let count = (length - done).min(chunk as u64) as usize;
        let at = offset.checked_add(done).ok_or(Error::InvalidInput {
            reason: "PDF copy offset overflows",
        })?;
        read_exact_at(source, at, &mut buffer[..count], limits, cancellation).await?;
        while let Some(&patch_at) = separator_patches.get(next_patch) {
            if patch_at >= done + count as u64 {
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
            next_patch += 1;
        }
        while let Some(gap) = gap_patches.get(next_gap) {
            let gap_end =
                gap.offset
                    .checked_add(gap.original.len() as u64)
                    .ok_or(Error::InvalidInput {
                        reason: "PDF orphan gap patch overflows",
                    })?;
            if gap.offset >= done + count as u64 {
                break;
            }
            let overlap_start = gap.offset.max(done);
            let overlap_end = gap_end.min(done + count as u64);
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
            if gap_end > done + count as u64 {
                break;
            }
            next_gap += 1;
        }
        writer.write_raw(&buffer[..count]).await?;
        done = done.checked_add(count as u64).ok_or(Error::InvalidInput {
            reason: "PDF copied-byte count overflows",
        })?;
    }
    if next_patch != separator_patches.len() {
        return Err(Error::InvalidInput {
            reason: "PDF stream separator patch exceeds copied prefix",
        });
    }
    if next_gap != gap_patches.len() {
        return Err(Error::InvalidInput {
            reason: "PDF orphan gap patch exceeds copied prefix",
        });
    }
    Ok(())
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
        if self.update_hash.is_some() && attempted > MAX_CLASSIC_PDF_BYTES {
            return Err(Error::LimitExceeded {
                resource: "classic PDF file bytes",
                limit: MAX_CLASSIC_PDF_BYTES,
                attempted,
            });
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
mod tests {
    use super::*;
    use crate::test_support::{CancelAfter, run};
    use crate::{
        NeverCancel,
        native::{SeekableSource, WriteSink},
        pdf::{ImageEncoding, ImageSpec, PageSpec, PdfDocument},
    };
    use std::io::{self, Cursor};

    fn unoutlined_pdf() -> Result<Vec<u8>> {
        let mut image = SeekableSource::new(Cursor::new(vec![0x7f]))?;
        let mut output = WriteSink::new(Vec::<u8>::new());
        let limits = Limits::default();
        run(async {
            let mut pdf = PdfDocument::new(&mut output, &limits, &NeverCancel).await?;
            pdf.add_image_page(
                &mut image,
                0,
                1,
                PageSpec {
                    width_points: 72.0,
                    height_points: 72.0,
                },
                ImageSpec {
                    pixel_width: 1,
                    pixel_height: 1,
                    encoding: ImageEncoding::Gray8,
                },
            )
            .await?;
            pdf.finish().await?;
            Ok::<(), Error>(())
        })?;
        Ok(output.into_inner())
    }

    fn with_id(mut pdf: Vec<u8>) -> Vec<u8> {
        let needle = b" >>\nstartxref\n";
        let marker = pdf
            .windows(needle.len())
            .rposition(|window| window == needle)
            .expect("generated trailer has a closing dictionary");
        pdf.splice(
            marker..marker + needle.len(),
            b" /ID [<00112233445566778899AABBCCDDEEFF> <00112233445566778899AABBCCDDEEFF>] >>\nstartxref\n"
                .iter()
                .copied(),
        );
        pdf
    }

    fn import_one(pdf: &[u8], title: &str) -> Result<Vec<u8>> {
        let mut source = SeekableSource::new(Cursor::new(pdf))?;
        let mut output = WriteSink::new(Vec::<u8>::new());
        let limits = Limits::default();
        run(async {
            let index = PdfIndex::open(
                &mut source,
                PdfRange {
                    offset: 0,
                    length: pdf.len() as u64,
                },
                &limits,
                &NeverCancel,
            )
            .await?;
            let mut appender =
                PdfOutlineAppender::begin(&mut source, &mut output, &index, &limits, &NeverCancel)
                    .await?;
            appender
                .add_bookmark(Bookmark {
                    depth: 0,
                    title: title.into(),
                    page_index: 0,
                })
                .await?;
            let report = appender.finish().await?;
            assert_eq!(report.pages_converted, 1);
            assert_eq!(report.bookmarks_written, 1);
            Ok::<(), Error>(())
        })?;
        Ok(output.into_inner())
    }

    fn final_id(pdf: &[u8]) -> &[u8] {
        let trailer = pdf
            .windows(b"trailer\n".len())
            .rposition(|window| window == b"trailer\n")
            .expect("output trailer exists");
        let tail = &pdf[trailer..];
        let id = tail
            .windows(b" /ID [".len())
            .position(|window| window == b" /ID [")
            .expect("output ID exists");
        &tail[id..]
    }

    #[test]
    fn direct_id_parser_accepts_hex_literal_and_comments() {
        assert_eq!(
            first_id_string(b" [ % comment\r\n <0123> (second) ] ", 0).unwrap(),
            b"<0123>"
        );
        assert_eq!(
            first_id_string(br"[(first\)id) <ABCD>]", 0).unwrap(),
            br"(first\)id)"
        );
    }

    #[test]
    fn direct_id_parser_rejects_indirect_and_unterminated_values() {
        for raw in [
            b"[1 0 R <00>]".as_slice(),
            b"[<00> <11>".as_slice(),
            b"[(unterminated <11>]".as_slice(),
        ] {
            assert!(matches!(
                first_id_string(raw, 123),
                Err(Error::Pdf {
                    offset: 123,
                    kind: PdfErrorKind::UnsupportedFeature,
                    ..
                })
            ));
        }
    }

    #[test]
    fn update_id_hash_changes_with_update_bytes() {
        let first = fnv128(FNV128_BASIS, b"[<first> <old>]");
        assert_ne!(fnv128(first, b"outline A"), fnv128(first, b"outline B"));
    }

    #[test]
    fn clean_existing_outline_is_copied_byte_for_byte() -> Result<()> {
        let original = include_bytes!("../../../../tests/fixtures/valid_nested_outline.pdf");
        let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
        let mut output = WriteSink::new(Vec::<u8>::new());
        let report = run(copy_pdf(
            &mut source,
            &mut output,
            &Limits::default(),
            &NeverCancel,
        ))?;
        assert_eq!(output.into_inner(), original);
        assert_eq!(report.output_bytes_written, original.len() as u64);
        assert!(report.input_bytes_read >= original.len() as u64);
        assert_eq!(report.pages_converted, 2);
        assert_eq!(report.bookmarks_written, 0);
        Ok(())
    }

    #[test]
    fn importing_into_existing_outline_preserves_original_navigation() -> Result<()> {
        let original = include_bytes!("../../../../tests/fixtures/valid_nested_outline.pdf");
        let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
        let mut output = WriteSink::new(Vec::<u8>::new());
        let limits = Limits::default();
        let report = run(async {
            let index = PdfIndex::open(
                &mut source,
                PdfRange {
                    offset: 0,
                    length: original.len() as u64,
                },
                &limits,
                &NeverCancel,
            )
            .await?;
            let mut appender =
                PdfOutlineAppender::begin(&mut source, &mut output, &index, &limits, &NeverCancel)
                    .await?;
            assert!(appender.preserves_existing_outlines());
            appender
                .add_bookmark(Bookmark {
                    depth: 0,
                    title: "New title".into(),
                    page_index: 999,
                })
                .await?;
            appender.finish().await
        })?;
        assert_eq!(report.bookmarks_written, 0);
        assert_eq!(output.into_inner(), original);
        Ok(())
    }

    #[test]
    fn embedded_pdf_range_is_copied_without_container_bytes() -> Result<()> {
        let original = include_bytes!("../../../../tests/fixtures/valid_nested_outline.pdf");
        let prefix = b"container bytes before embedded PDF";
        let mut container = prefix.to_vec();
        container.extend_from_slice(original);
        container.extend_from_slice(b"container suffix");
        let mut source = SeekableSource::new(Cursor::new(container))?;
        let mut output = WriteSink::new(Vec::<u8>::new());
        let report = run(copy_pdf_range(
            &mut source,
            &mut output,
            PdfRange {
                offset: prefix.len() as u64,
                length: original.len() as u64,
            },
            &Limits::default(),
            &NeverCancel,
        ))?;
        assert_eq!(output.into_inner(), original);
        assert_eq!(report.pages_converted, 2);
        Ok(())
    }

    #[test]
    fn new_outline_update_preserves_pdf_prefix_and_changes_id_by_title() -> Result<()> {
        let original = with_id(unoutlined_pdf()?);
        let one = import_one(&original, "AA")?;
        let two = import_one(&original, "BB")?;
        assert!(one.starts_with(&original));
        assert!(two.starts_with(&original));
        assert_ne!(final_id(&one), final_id(&two));
        assert!(String::from_utf8_lossy(&one).contains("/Title <FEFF00410041>"));
        assert!(String::from_utf8_lossy(&two).contains("/Title <FEFF00420042>"));
        Ok(())
    }

    #[test]
    fn nested_siblings_and_long_title_reopen_as_one_outline_tree() -> Result<()> {
        let original = unoutlined_pdf()?;
        let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
        let mut output = WriteSink::new(Vec::<u8>::new());
        let limits = Limits::default();
        let report = run(async {
            let index = PdfIndex::open(
                &mut source,
                PdfRange {
                    offset: 0,
                    length: original.len() as u64,
                },
                &limits,
                &NeverCancel,
            )
            .await?;
            let mut appender =
                PdfOutlineAppender::begin(&mut source, &mut output, &index, &limits, &NeverCancel)
                    .await?;
            for (depth, title) in [
                (0, "Root".to_owned()),
                (1, "A".repeat(3000)),
                (1, "第二章".to_owned()),
                (0, "After".to_owned()),
            ] {
                appender
                    .add_bookmark(Bookmark {
                        depth,
                        title,
                        page_index: 0,
                    })
                    .await?;
            }
            appender.finish().await
        })?;
        assert_eq!(report.bookmarks_written, 4);
        let pdf = output.into_inner();
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Title <FEFF7B2C4E8C7AE0>"));
        assert_eq!(text.matches(" /Next ").count(), 2);
        assert_eq!(text.matches(" /Prev ").count(), 3); // two item links + trailer
        let mut source = SeekableSource::new(Cursor::new(pdf.as_slice()))?;
        let index = run(PdfIndex::open(
            &mut source,
            PdfRange {
                offset: 0,
                length: pdf.len() as u64,
            },
            &limits,
            &NeverCancel,
        ))?;
        assert!(index.has_outlines());
        assert_eq!(index.pages().len(), 1);
        Ok(())
    }

    #[test]
    fn bookmark_limits_reject_input_before_new_objects() -> Result<()> {
        let original = unoutlined_pdf()?;
        let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
        let mut output = WriteSink::new(Vec::<u8>::new());
        let limits = Limits {
            max_bookmarks: 1,
            max_allocation_bytes: 256 * 1024,
            ..Limits::default()
        };
        run(async {
            let index = PdfIndex::open(
                &mut source,
                PdfRange {
                    offset: 0,
                    length: original.len() as u64,
                },
                &limits,
                &NeverCancel,
            )
            .await?;
            let mut appender =
                PdfOutlineAppender::begin(&mut source, &mut output, &index, &limits, &NeverCancel)
                    .await?;
            assert!(matches!(
                appender
                    .add_bookmark(Bookmark {
                        depth: 0,
                        title: "too far".into(),
                        page_index: 1,
                    })
                    .await,
                Err(Error::InvalidInput { .. })
            ));
            assert!(matches!(
                appender
                    .add_bookmark(Bookmark {
                        depth: 0,
                        title: "".into(),
                        page_index: 0,
                    })
                    .await,
                Err(Error::InvalidInput { .. })
            ));
            assert!(matches!(
                appender
                    .add_bookmark(Bookmark {
                        depth: MAX_OUTLINE_DEPTH as u32,
                        title: "deep".into(),
                        page_index: 0,
                    })
                    .await,
                Err(Error::LimitExceeded {
                    resource: "PDF outline depth",
                    ..
                })
            ));
            assert!(matches!(
                appender
                    .add_bookmark(Bookmark {
                        depth: 0,
                        title: "X".repeat(300_000),
                        page_index: 0,
                    })
                    .await,
                Err(Error::LimitExceeded { .. })
            ));
            appender
                .add_bookmark(Bookmark {
                    depth: 0,
                    title: "Allowed".into(),
                    page_index: 0,
                })
                .await?;
            assert!(matches!(
                appender
                    .add_bookmark(Bookmark {
                        depth: 0,
                        title: "second".into(),
                        page_index: 0,
                    })
                    .await,
                Err(Error::LimitExceeded {
                    resource: "bookmarks",
                    ..
                })
            ));
            Ok::<(), Error>(())
        })
    }

    #[test]
    fn copy_output_limit_is_checked_before_sink_writes() -> Result<()> {
        let original = include_bytes!("../../../../tests/fixtures/valid_nested_outline.pdf");
        let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
        let mut output = WriteSink::new(Vec::<u8>::new());
        let limits = Limits {
            max_output_bytes: original.len() as u64 - 1,
            ..Limits::default()
        };
        assert!(matches!(
            run(copy_pdf(&mut source, &mut output, &limits, &NeverCancel)),
            Err(Error::PdfLimitExceeded {
                resource: "output bytes",
                object: Some((1, 0)),
                ..
            })
        ));
        assert!(output.into_inner().is_empty());
        Ok(())
    }

    #[test]
    fn invalid_bookmark_rejects_without_claiming_success() -> Result<()> {
        let original = unoutlined_pdf()?;
        let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
        let mut output = WriteSink::new(Vec::<u8>::new());
        let limits = Limits::default();
        run(async {
            let index = PdfIndex::open(
                &mut source,
                PdfRange {
                    offset: 0,
                    length: original.len() as u64,
                },
                &limits,
                &NeverCancel,
            )
            .await?;
            let mut appender =
                PdfOutlineAppender::begin(&mut source, &mut output, &index, &limits, &NeverCancel)
                    .await?;
            assert!(matches!(
                appender
                    .add_bookmark(Bookmark {
                        depth: 1,
                        title: "orphan".into(),
                        page_index: 0,
                    })
                    .await,
                Err(Error::InvalidInput { .. })
            ));
            Ok::<(), Error>(())
        })
    }

    struct FailingSink {
        accepted: usize,
        remaining: usize,
        fail_flush: bool,
    }

    impl SequentialSink for FailingSink {
        async fn write(&mut self, bytes: &[u8]) -> Result<usize> {
            if self.remaining == 0 {
                return Err(Error::Io(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "injected sink failure",
                )));
            }
            let count = bytes.len().min(self.remaining).min(7);
            self.remaining -= count;
            self.accepted += count;
            Ok(count)
        }

        async fn flush(&mut self) -> Result<()> {
            if self.fail_flush {
                Err(Error::Io(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "injected flush failure",
                )))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn sink_failure_has_no_success_report() -> Result<()> {
        let original = include_bytes!("../../../../tests/fixtures/valid_nested_outline.pdf");
        let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
        let mut sink = FailingSink {
            accepted: 0,
            remaining: 35,
            fail_flush: false,
        };
        let result = run(copy_pdf(
            &mut source,
            &mut sink,
            &Limits::default(),
            &NeverCancel,
        ));
        assert!(matches!(result, Err(Error::Io(_))));
        assert_eq!(sink.accepted, 35);
        Ok(())
    }

    #[test]
    fn flush_failure_has_no_success_report() -> Result<()> {
        let original = include_bytes!("../../../../tests/fixtures/valid_nested_outline.pdf");
        let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
        let mut sink = FailingSink {
            accepted: 0,
            remaining: original.len(),
            fail_flush: true,
        };
        let result = run(copy_pdf(
            &mut source,
            &mut sink,
            &Limits::default(),
            &NeverCancel,
        ));
        assert!(matches!(result, Err(Error::Io(_))));
        assert_eq!(sink.accepted, original.len());
        Ok(())
    }

    /// A classic-xref PDF whose objects are numbered `1..=objects.len()`.
    /// `gap` is inserted immediately before object `gap.0`.
    fn classic_pdf(objects: &[&str], gap: Option<(u32, &[u8])>, trailer_extra: &str) -> Vec<u8> {
        let mut pdf = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in objects.iter().enumerate() {
            let number = index as u32 + 1;
            if let Some((_, bytes)) = gap.filter(|(before, _)| *before == number) {
                pdf.extend_from_slice(bytes);
            }
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{number} 0 obj\n{body}\nendobj\n").as_bytes());
        }
        let xref = pdf.len();
        let size = objects.len() + 1;
        pdf.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {size} /Root 1 0 R {trailer_extra} >>\nstartxref\n{xref}\n%%EOF\n"
            )
            .as_bytes(),
        );
        pdf
    }

    const CATALOG: &str = "<< /Type /Catalog /Pages 2 0 R >>";
    const PAGES: &str = "<< /Type /Pages /Kids [3 0 R] /Count 1 >>";
    const PAGE: &str = "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 200] >>";

    fn open_index(pdf: &[u8], limits: &Limits) -> Result<PdfIndex> {
        let mut source = SeekableSource::new(Cursor::new(pdf))?;
        run(PdfIndex::open(
            &mut source,
            PdfRange {
                offset: 0,
                length: pdf.len() as u64,
            },
            limits,
            &NeverCancel,
        ))
    }

    #[test]
    fn id_parser_rejects_non_arrays_trailing_values_and_bad_strings() {
        for raw in [
            b"<00> <11>".as_slice(),
            b"[<00> <11>] junk",
            b"[<0G> <11>]",
            b"[<00> <11",
            br"[(ends with escape\",
            b"[(unbalanced (nested) <11>]",
            b"[<< >> <11>]",
        ] {
            assert!(
                matches!(
                    first_id_string(raw, 77),
                    Err(Error::Pdf {
                        offset: 77,
                        object: None,
                        kind: PdfErrorKind::UnsupportedFeature,
                        reason: "unsupported PDF trailer ID syntax",
                    })
                ),
                "accepted {raw:?}"
            );
        }
        assert_eq!(
            first_id_string(b"[(a (nested) b) < 0a 1B >]", 0).unwrap(),
            b"(a (nested) b)"
        );
        assert_eq!(
            first_id_string(b"[<00 11\n22> (x)] % trailing comment", 0).unwrap(),
            b"<00 11\n22>"
        );
    }

    #[test]
    fn update_keeps_trailer_info_and_replaces_an_empty_outline_root() -> Result<()> {
        let original = classic_pdf(
            &[
                "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R /PageMode /UseNone >>",
                PAGES,
                PAGE,
                "<< /Type /Outlines /Count 0 >>",
                "<< /Producer (unit test) >>",
            ],
            None,
            "/Info 5 0 R",
        );
        let limits = Limits::default();
        let index = open_index(&original, &limits)?;
        assert!(!index.has_outlines());
        let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
        let mut output = WriteSink::new(Vec::<u8>::new());
        let report = run(async {
            let mut appender =
                PdfOutlineAppender::begin(&mut source, &mut output, &index, &limits, &NeverCancel)
                    .await?;
            assert!(!appender.preserves_existing_outlines());
            appender
                .add_bookmark(Bookmark {
                    depth: 0,
                    title: "Only".into(),
                    page_index: 0,
                })
                .await?;
            appender.finish().await
        })?;
        assert_eq!(report.bookmarks_written, 1);
        assert_eq!(report.input_bytes_read, original.len() as u64);
        let pdf = output.into_inner();
        assert_eq!(report.output_bytes_written, pdf.len() as u64);
        assert!(pdf.starts_with(&original));
        let update = String::from_utf8_lossy(&pdf[original.len()..]);
        assert!(
            update.contains(
                "1 0 obj\n<< /Type /Catalog /Pages 2 0 R /PageMode /UseNone /Outlines 6 0 R >>"
            ),
            "{update}"
        );
        assert!(!update.contains("/Outlines 4 0 R"), "{update}");
        assert!(
            update.contains("6 0 obj\n<< /Type /Outlines /First 7 0 R /Last 7 0 R /Count 1 >>")
        );
        let xref = original
            .windows(5)
            .position(|window| window == b"xref\n")
            .unwrap();
        assert!(
            update.contains(&format!(
                "trailer\n<< /Size 8 /Root 1 0 R /Prev {xref} /Info 5 0 R >>"
            )),
            "{update}"
        );
        let reopened = open_index(&pdf, &limits)?;
        assert!(reopened.has_outlines());
        assert_eq!(reopened.trailer_info(), index.trailer_info());
        Ok(())
    }

    #[test]
    fn orphan_gap_scrubbing_spans_small_copy_chunks() -> Result<()> {
        let gap = b"4 0 obj\r<\r\n";
        let original = classic_pdf(
            &[CATALOG, PAGES, PAGE, "(live but unreferenced)"],
            Some((4, gap)),
            "",
        );
        let at = original
            .windows(gap.len())
            .position(|window| window == gap)
            .unwrap();
        let limits = Limits {
            io_chunk_bytes: 4,
            ..Limits::default()
        };
        let index = open_index(&original, &limits)?;
        let [patch] = index.gap_patches() else {
            panic!("expected one orphan gap patch");
        };
        // The inactive span may include separator whitespace before the
        // aborted header; it must end at the next live object.
        let start = patch.offset as usize;
        let end = start + patch.original.len();
        assert!(start <= at && patch.original.ends_with(gap));
        assert!(original[end..].starts_with(b"4 0 obj\n("));
        // More than two 4-byte chunks, so the scrub crosses chunk boundaries.
        assert!(patch.original.len() > 8);
        let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
        let mut output = WriteSink::new(Vec::<u8>::new());
        let report = run(copy_pdf(&mut source, &mut output, &limits, &NeverCancel))?;
        let mut expected = original.clone();
        expected[start..end].fill(b' ');
        let copied = output.into_inner();
        assert_eq!(copied, expected);
        assert_eq!(report.output_bytes_written, expected.len() as u64);
        assert_eq!(report.bookmarks_written, 0);
        assert!(
            open_index(&copied, &Limits::default())?
                .gap_patches()
                .is_empty()
        );
        Ok(())
    }

    #[test]
    fn writer_refuses_new_bookmarks_after_an_output_failure() -> Result<()> {
        let original = unoutlined_pdf()?;
        let limits = Limits::default();
        let index = open_index(&original, &limits)?;
        let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
        let mut sink = FailingSink {
            accepted: 0,
            remaining: original.len() + 3,
            fail_flush: false,
        };
        let poisoned = run(async {
            let mut appender =
                PdfOutlineAppender::begin(&mut source, &mut sink, &index, &limits, &NeverCancel)
                    .await?;
            let bookmark = |title: &str| Bookmark {
                depth: 0,
                title: title.into(),
                page_index: 0,
            };
            appender.add_bookmark(bookmark("first")).await?;
            // The second sibling emits the first item and exhausts the sink.
            assert!(matches!(
                appender.add_bookmark(bookmark("second")).await,
                Err(Error::Io(_))
            ));
            let retry = appender.add_bookmark(bookmark("third")).await;
            let finish = appender.finish().await;
            Ok::<_, Error>((retry, finish))
        })?;
        for result in [poisoned.0.map(|_| ()), poisoned.1.map(|_| ())] {
            assert!(matches!(
                result,
                Err(Error::InvalidInput {
                    reason: "PDF append writer cannot continue after an output failure"
                })
            ));
        }
        assert_eq!(sink.accepted, original.len() + 3);
        Ok(())
    }

    #[derive(Default)]
    struct RecordingSink {
        bytes: Vec<u8>,
        flushes: u32,
    }

    impl SequentialSink for RecordingSink {
        async fn write(&mut self, bytes: &[u8]) -> Result<usize> {
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        async fn flush(&mut self) -> Result<()> {
            self.flushes += 1;
            Ok(())
        }
    }

    #[test]
    fn cancellation_around_the_final_flush_prevents_a_success_report() -> Result<()> {
        let original = include_bytes!("../../../../tests/fixtures/valid_nested_outline.pdf");
        let copy = |allowed: u64| -> (Result<ConversionReport>, RecordingSink, u64) {
            let mut source = SeekableSource::new(Cursor::new(original.as_slice())).unwrap();
            let mut sink = RecordingSink::default();
            let cancellation = CancelAfter::new(allowed);
            let result = run(copy_pdf(
                &mut source,
                &mut sink,
                &Limits::default(),
                &cancellation,
            ));
            (result, sink, cancellation.queries())
        };
        let (result, sink, checks) = copy(u64::MAX);
        result?;
        assert_eq!(sink.bytes, original);
        assert_eq!(sink.flushes, 1);

        // The penultimate check precedes the flush; the last one follows it.
        let (before_flush, sink, _) = copy(checks - 2);
        assert!(matches!(before_flush, Err(Error::Cancelled)));
        assert_eq!(sink.bytes, original);
        assert_eq!(sink.flushes, 0);

        let (after_flush, sink, _) = copy(checks - 1);
        assert!(matches!(after_flush, Err(Error::Cancelled)));
        assert_eq!(sink.flushes, 1);
        Ok(())
    }

    fn invalid(reason: &'static str) -> impl Fn(&Result<()>) -> bool {
        move |result| matches!(result, Err(Error::InvalidInput { reason: actual }) if *actual == reason)
    }

    #[test]
    fn append_writer_rejects_misordered_object_calls() -> Result<()> {
        let limits = Limits::default();
        let mut sink = RecordingSink::default();
        let reference = PdfRef {
            number: 7,
            generation: 0,
        };
        run(async {
            let mut writer = AppendWriter::new(&mut sink, &limits, &NeverCancel);
            let misuse = invalid("invalid PDF append object state or number");
            assert!(misuse(
                &writer
                    .begin_object(PdfRef {
                        number: 0,
                        generation: 0,
                    })
                    .await
            ));
            assert!(invalid("no PDF append object is open")(
                &writer.end_object().await
            ));
            writer.begin_object(reference).await?;
            assert!(misuse(&writer.begin_object(reference).await));
            let open = invalid("PDF append object remains open");
            assert!(open(&writer.finish_copy().await));
            writer.end_object().await
        })?;
        assert_eq!(sink.bytes, b"\n7 0 obj\n\nendobj\n");
        Ok(())
    }

    #[test]
    fn append_update_rejects_open_duplicate_and_oversized_state() -> Result<()> {
        let limits = Limits::default();
        let index = open_index(&classic_pdf(&[CATALOG, PAGES, PAGE], None, ""), &limits)?;
        let reference = PdfRef {
            number: 4,
            generation: 0,
        };
        let mut sink = RecordingSink::default();
        run(async {
            let mut writer = AppendWriter::new(&mut sink, &limits, &NeverCancel);
            writer.begin_object(reference).await?;
            assert!(invalid("PDF append object remains open")(
                &writer.finish_update(&index).await
            ));
            writer.end_object().await?;
            writer.begin_object(reference).await?;
            writer.end_object().await?;
            assert!(invalid("PDF update defines an object twice")(
                &writer.finish_update(&index).await
            ));
            Ok::<_, Error>(())
        })?;

        let mut sink = RecordingSink::default();
        let oversized = run(async {
            let mut writer = AppendWriter::new(&mut sink, &limits, &NeverCancel);
            writer.position = MAX_CLASSIC_PDF_BYTES + 1;
            writer.finish_update(&index).await
        });
        assert!(matches!(
            oversized,
            Err(Error::LimitExceeded {
                resource: "classic PDF file bytes",
                attempted,
                ..
            }) if attempted == MAX_CLASSIC_PDF_BYTES + 1
        ));

        let mut sink = RecordingSink::default();
        let far_object = run(async {
            let mut writer = AppendWriter::new(&mut sink, &limits, &NeverCancel);
            writer.entries.push(XrefEntry {
                reference,
                offset: MAX_CLASSIC_PDF_BYTES + 1,
            });
            writer.finish_update(&index).await
        });
        assert!(matches!(
            far_object,
            Err(Error::LimitExceeded {
                resource: "classic PDF object offset",
                ..
            })
        ));
        Ok(())
    }
}
