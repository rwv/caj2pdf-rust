// SPDX-License-Identifier: MIT

//! Bounded reconstruction of indexed PDF object fragments.
//!
//! A format handler supplies the exact source span of every indirect object
//! and the intended page order. This module never infers page order from
//! object numbers or searches binary stream payloads for PDF delimiters.

use super::input::{FragmentKind, inspect_fragment_object, inspect_fragment_scalar};
use super::outline::{
    BookmarkView, MAX_OUTLINE_DEPTH, ObjectSink, OutlineItem, write_item, write_root,
};
use super::page_walk::{PageStep, PageWalk};
use super::writer::{HEADER, MAX_PDF_OBJECTS, Output, checked_object_number};
use super::xref::{Trailer, write_xref};
use super::{PdfRange, PdfRef};
use crate::fallible::{len_u64, reserve_exact, usize_from_u32};
use crate::{
    Bookmark, Cancellation, ConversionReport, CountingSource, Error, Limits, PdfErrorKind,
    RangedSource, Result, SequentialSink, read_exact_at,
};
use std::mem::size_of;

/// The complete byte range of one generation-zero indirect object, from its
/// `<number> 0 obj` header through its `endobj` keyword.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FragmentObject {
    pub reference: PdfRef,
    pub range: PdfRange,
}

/// An explicit, validated plan for rebuilding a classic-xref PDF.
///
/// `pages` is document order. When `pages_root` has no supplied object, every
/// page must refer directly to this missing parent; a single `/Pages` node is
/// synthesized. The catalog is always synthesized at the next available object
/// number, and a supplied catalog object is rejected. An existing page tree
/// root is retained only when its references are consistent with this plan.
#[derive(Clone, Copy, Debug)]
pub struct FragmentPlan<'a> {
    pub objects: &'a [FragmentObject],
    pub pages: &'a [PdfRef],
    pub pages_root: PdfRef,
}

#[derive(Clone, Copy, Debug)]
struct Record {
    reference: PdfRef,
    range: PdfRange,
    output_offset: u64,
}

#[derive(Clone, Copy)]
struct OutlineNode {
    item: OutlineItem<PdfRef>,
    parent_index: Option<usize>,
}

#[derive(Clone, Copy, Debug)]
enum ContentEvidenceKind {
    Page { direct_array: bool },
    ScalarArray,
}

#[derive(Debug)]
struct ContentEvidence {
    reference: PdfRef,
    kind: ContentEvidenceKind,
    references: Vec<PdfRef>,
}

const OVERREAD: &str = "PDF source reported more bytes than requested";

impl Record {
    fn from_fragment(fragment: FragmentObject) -> Self {
        Self {
            reference: fragment.reference,
            range: fragment.range,
            output_offset: 0,
        }
    }

    fn synthetic(reference: PdfRef) -> Self {
        Self {
            reference,
            range: PdfRange {
                offset: 0,
                length: 0,
            },
            output_offset: 0,
        }
    }
}

fn pdf_error(
    reference: Option<PdfRef>,
    offset: u64,
    kind: PdfErrorKind,
    reason: &'static str,
) -> Error {
    Error::Pdf {
        offset,
        object: reference.map(|r| (r.number, r.generation)),
        kind,
        reason,
    }
}

fn malformed(reference: Option<PdfRef>, offset: u64, reason: &'static str) -> Error {
    pdf_error(reference, offset, PdfErrorKind::Malformed, reason)
}

fn pdf_limit(
    reference: Option<PdfRef>,
    offset: u64,
    resource: &'static str,
    limit: u64,
    attempted: u64,
) -> Error {
    Error::PdfLimitExceeded {
        offset,
        object: reference.map(|r| (r.number, r.generation)),
        resource,
        limit,
        attempted,
    }
}

fn check_pdf_allocation(
    limits: &Limits,
    bytes: u64,
    reference: Option<PdfRef>,
    offset: u64,
) -> Result<()> {
    if bytes > limits.max_allocation_bytes {
        return Err(pdf_limit(
            reference,
            offset,
            "PDF allocation bytes",
            limits.max_allocation_bytes,
            bytes,
        ));
    }
    Ok(())
}

fn checked_add(left: u64, right: u64) -> Result<u64> {
    left.checked_add(right).ok_or(Error::InvalidInput {
        reason: "PDF arithmetic overflows 64 bits",
    })
}

/// The object slots a reconstruction writes: the planned objects, two
/// synthetic objects, and, with bookmarks, an outline root plus one item per
/// bookmark. A count beyond the PDF object limit is located at the first
/// planned object. It takes counts, not slices, so a test reaches the limit
/// without building millions of objects.
fn requested_objects(
    object_count: usize,
    bookmark_count: usize,
    first: Option<&FragmentObject>,
) -> Result<usize> {
    let outline_objects = match bookmark_count {
        0 => Some(0),
        count => count.checked_add(1),
    };
    let requested = outline_objects
        .and_then(|outline| object_count.checked_add(2)?.checked_add(outline))
        .ok_or(Error::InvalidInput {
            reason: "PDF object count overflows address space",
        })?;
    let offset = first.map_or(0, |object| object.range.offset);
    let reference = first.map(|object| (object.reference.number, object.reference.generation));
    checked_object_number(requested).map_err(|error| error.locate_pdf_limit(offset, reference))?;
    Ok(requested)
}

/// The overflow-checked sum of `values`. They are unsigned, so the sum
/// overflows in any order exactly when it exceeds `u64::MAX`.
fn checked_sum(values: &[u64]) -> Result<u64> {
    values
        .iter()
        .try_fold(0, |sum, &value| checked_add(sum, value))
}

fn checked_reference(reference: PdfRef, offset: u64) -> Result<()> {
    if reference.number == 0 {
        return Err(malformed(
            Some(reference),
            offset,
            "object zero cannot be referenced",
        ));
    }
    if reference.number > MAX_PDF_OBJECTS {
        return Err(pdf_limit(
            Some(reference),
            offset,
            "PDF object number",
            u64::from(MAX_PDF_OBJECTS),
            u64::from(reference.number),
        ));
    }
    if reference.generation != 0 {
        return Err(pdf_error(
            Some(reference),
            offset,
            PdfErrorKind::UnsupportedFeature,
            "fragment reconstruction supports generation-zero objects only",
        ));
    }
    Ok(())
}

fn object_index(records: &[Record], reference: PdfRef) -> Option<usize> {
    records
        .binary_search_by_key(&reference.number, |record| record.reference.number)
        .ok()
}

fn build_outline_nodes(
    bookmarks: &[Bookmark],
    pages: &[PdfRef],
    root: PdfRef,
    limits: &Limits,
    retained_bytes: u64,
) -> Result<(Vec<OutlineNode>, PdfRef, PdfRef)> {
    let metadata_bytes = bookmarks
        .len()
        .checked_mul(size_of::<OutlineNode>())
        .map(len_u64)
        .ok_or(Error::InvalidInput {
            reason: "PDF outline index allocation overflows 64 bits",
        })?;
    check_pdf_allocation(
        limits,
        checked_add(retained_bytes, metadata_bytes)?,
        Some(root),
        0,
    )?;
    let mut nodes: Vec<OutlineNode> = Vec::new();
    let refused = pdf_limit(
        None,
        0,
        "PDF outline index allocation",
        limits.max_allocation_bytes,
        metadata_bytes,
    );
    reserve_exact(&mut nodes, bookmarks.len(), refused)?;
    let mut stack: [Option<usize>; MAX_OUTLINE_DEPTH] = [None; MAX_OUTLINE_DEPTH];
    let mut first_root = None;
    let mut last_root: Option<usize> = None;
    let mut previous_depth = 0;
    for (index, bookmark) in bookmarks.iter().enumerate() {
        let depth = usize_from_u32(bookmark.depth);
        if depth >= MAX_OUTLINE_DEPTH {
            return Err(pdf_limit(
                None,
                0,
                "PDF outline depth",
                MAX_OUTLINE_DEPTH as u64,
                depth as u64 + 1,
            ));
        }
        if index == 0 && depth != 0 || index != 0 && depth > previous_depth + 1 {
            return Err(malformed(None, 0, "bookmark depth skips a parent"));
        }
        if bookmark.title.is_empty() {
            return Err(malformed(None, 0, "bookmark title is empty"));
        }
        let failure = malformed(None, 0, "bookmark destination is outside the ordered pages");
        let page = *pages.get(bookmark.page_index as usize).ok_or(failure)?;
        let failure = pdf_limit(
            Some(root),
            0,
            "PDF object number",
            u64::from(MAX_PDF_OBJECTS),
            u64::MAX,
        );
        let item_number = u32::try_from(index)
            .ok()
            .and_then(|position| position.checked_add(1))
            .and_then(|position| root.number.checked_add(position))
            .ok_or(failure)?;
        let reference = PdfRef {
            number: item_number,
            generation: 0,
        };
        checked_reference(reference, 0)?;
        let parent_index: Option<usize> = if depth == 0 {
            None
        } else {
            Some(stack[depth - 1].ok_or(malformed(None, 0, "bookmark parent is missing"))?)
        };
        let previous_index = if let Some(parent_index) = parent_index {
            nodes[parent_index].item.last_child
        } else {
            last_root.map(|previous| nodes[previous].item.reference)
        };
        let parent = parent_index.map_or(root, |parent_index| nodes[parent_index].item.reference);
        nodes.push(OutlineNode {
            item: OutlineItem::new(reference, parent, previous_index, page, BookmarkView::Xyz),
            parent_index,
        });
        if let Some(previous) = previous_index {
            let previous_position = previous
                .number
                .checked_sub(root.number)
                .and_then(|difference| difference.checked_sub(1))
                .and_then(|difference| usize::try_from(difference).ok())
                .ok_or(Error::InvalidInput {
                    reason: "PDF outline sibling index overflows",
                })?;
            nodes[previous_position].item.next = Some(reference);
        }
        if let Some(parent_index) = parent_index {
            let parent = &mut nodes[parent_index].item;
            parent.first_child.get_or_insert(reference);
            parent.last_child = Some(reference);
        } else {
            first_root.get_or_insert(reference);
            last_root = Some(index);
        }
        stack[depth] = Some(index);
        previous_depth = depth;
    }
    for index in (0..nodes.len()).rev() {
        if let Some(parent) = nodes[index].parent_index {
            let subtree =
                nodes[index]
                    .item
                    .descendants
                    .checked_add(1)
                    .ok_or(Error::InvalidInput {
                        reason: "PDF outline descendant count overflows",
                    })?;
            let parent = &mut nodes[parent].item;
            parent.descendants =
                parent
                    .descendants
                    .checked_add(subtree)
                    .ok_or(Error::InvalidInput {
                        reason: "PDF outline descendant count overflows",
                    })?;
        }
    }
    let first_root = first_root.ok_or(Error::InvalidInput {
        reason: "PDF outline root has no first item",
    })?;
    let last_root = nodes[last_root.ok_or(Error::InvalidInput {
        reason: "PDF outline root has no last item",
    })?]
    .item
    .reference;
    Ok((nodes, first_root, last_root))
}

/// Writes synthetic objects, recording each one's output offset for the
/// cross-reference table.
struct SyntheticObjects<'o, 'a, 'r, W: SequentialSink, C: Cancellation> {
    out: &'o mut Output<'a, W, C>,
    records: &'r mut [Record],
}

impl<W: SequentialSink, C: Cancellation> ObjectSink for SyntheticObjects<'_, '_, '_, W, C> {
    type Ref = PdfRef;

    async fn begin_object(&mut self, reference: PdfRef) -> Result<()> {
        let index = object_index(self.records, reference).ok_or(Error::InvalidInput {
            reason: "synthetic PDF object was not indexed",
        })?;
        self.records[index].output_offset = self.out.position;
        self.out
            .write(format!("{} {} obj\n", reference.number, reference.generation).as_bytes())
            .await
    }

    async fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.out.write(bytes).await
    }

    async fn end_object(&mut self) -> Result<()> {
        self.out.write(b"\nendobj\n").await
    }
}

/// Reconstruct one PDF from indexed indirect objects and explicit page order,
/// with a CAJ outline in the same PDF revision.
///
/// All plan and source validation precedes the first sink write. A subsequent
/// source, sink, or cancellation failure leaves a partial output and returns
/// an error, never a successful report. Source object bytes are copied in
/// bounded chunks without materializing a whole object or PDF in memory.
///
/// Bookmark entries are depth-first and name zero-based positions in
/// `plan.pages`. All titles and links are checked before the first sink write;
/// title hex is then emitted in bounded chunks.
pub async fn reconstruct_fragment_with_bookmarks<
    R: RangedSource,
    W: SequentialSink,
    C: Cancellation,
>(
    source: &mut R,
    sink: &mut W,
    plan: &FragmentPlan<'_>,
    bookmarks: &[Bookmark],
    limits: &Limits,
    cancellation: &C,
) -> Result<ConversionReport> {
    let mut input_bytes_read = 0;
    let mut counted =
        CountingSource::new(source, &mut input_bytes_read).rejecting_overread(OVERREAD);
    let source = &mut counted;
    limits.validate()?;
    if len_u64(bookmarks.len()) > u64::from(limits.max_bookmarks) {
        return Err(pdf_limit(
            None,
            0,
            "bookmarks",
            u64::from(limits.max_bookmarks),
            len_u64(bookmarks.len()),
        ));
    }
    // At most `max_bookmarks`, a `u32`.
    let bookmark_count = bookmarks.len() as u32;
    if len_u64(plan.pages.len()) > u64::from(limits.max_pages) {
        let first = plan.pages.first().copied();
        let offset = first
            .and_then(|reference| {
                plan.objects
                    .iter()
                    .find(|object| object.reference == reference)
                    .map(|object| object.range.offset)
            })
            .unwrap_or(0);
        return Err(pdf_limit(
            first,
            offset,
            "pages",
            u64::from(limits.max_pages),
            len_u64(plan.pages.len()),
        ));
    }
    // At most `max_pages`, a `u32`.
    let page_count = plan.pages.len() as u32;
    if page_count == 0 {
        return Err(malformed(None, 0, "fragment has no pages"));
    }
    checked_reference(plan.pages_root, 0)?;
    if cancellation.is_cancelled() {
        return Err(Error::Cancelled);
    }

    let requested = requested_objects(plan.objects.len(), bookmarks.len(), plan.objects.first())?;
    let record_bytes = requested
        .checked_mul(size_of::<Record>())
        .ok_or(Error::InvalidInput {
            reason: "PDF object index allocation overflows address space",
        })?;
    let record_bytes = len_u64(record_bytes);
    check_pdf_allocation(
        limits,
        record_bytes,
        plan.objects.first().map(|object| object.reference),
        plan.objects.first().map_or(0, |object| object.range.offset),
    )?;
    let mut records = Vec::new();
    let refused = limits.allocation_refused("PDF object index allocation", record_bytes);
    reserve_exact(&mut records, requested, refused)?;
    let mut fragment_bytes = 0_u64;
    for fragment in plan.objects {
        checked_reference(fragment.reference, fragment.range.offset)?;
        if fragment.range.length == 0 {
            return Err(malformed(
                Some(fragment.reference),
                fragment.range.offset,
                "indirect object span is empty",
            ));
        }
        let failure = malformed(
            Some(fragment.reference),
            fragment.range.offset,
            "indirect object span overflows 64-bit offset",
        );
        let end = fragment.range.end().ok_or(failure)?;
        if end > source.size() {
            return Err(Error::TruncatedInput {
                offset: fragment.range.offset,
                expected: fragment.range.length,
                available: source.size().saturating_sub(fragment.range.offset),
            });
        }
        fragment_bytes = fragment_bytes
            .checked_add(fragment.range.length)
            .ok_or_else(|| {
                pdf_limit(
                    Some(fragment.reference),
                    fragment.range.offset,
                    "input bytes",
                    limits.max_input_bytes,
                    u64::MAX,
                )
            })?;
        if fragment_bytes > limits.max_input_bytes {
            return Err(pdf_limit(
                Some(fragment.reference),
                fragment.range.offset,
                "input bytes",
                limits.max_input_bytes,
                fragment_bytes,
            ));
        }
        records.push(Record::from_fragment(*fragment));
    }
    records.sort_unstable_by_key(|record| record.range.offset);
    for pair in records.windows(2) {
        let failure = malformed(
            Some(pair[0].reference),
            pair[0].range.offset,
            "indirect object span overflows 64-bit offset",
        );
        let previous_end = pair[0].range.end().ok_or(failure)?;
        if previous_end > pair[1].range.offset {
            return Err(pdf_error(
                Some(pair[1].reference),
                pair[1].range.offset,
                PdfErrorKind::AmbiguousRepair,
                "indirect object spans overlap",
            ));
        }
    }
    records.sort_unstable_by_key(|record| record.reference.number);
    for pair in records.windows(2) {
        if pair[0].reference.number == pair[1].reference.number {
            return Err(pdf_error(
                Some(pair[1].reference),
                pair[1].range.offset,
                PdfErrorKind::AmbiguousRepair,
                "duplicate indirect object number",
            ));
        }
    }
    for page in plan.pages {
        checked_reference(*page, 0)?;
        if object_index(&records, *page).is_none() {
            return Err(malformed(Some(*page), 0, "ordered page object is missing"));
        }
    }
    // Check duplicate destinations with a bounded, sorted page-ref index.
    let page_index_bytes =
        plan.pages
            .len()
            .checked_mul(size_of::<PdfRef>())
            .ok_or(Error::InvalidInput {
                reason: "PDF page index allocation overflows address space",
            })?;
    let page_index_bytes = len_u64(page_index_bytes);
    check_pdf_allocation(
        limits,
        checked_add(record_bytes, page_index_bytes)?,
        plan.pages.first().copied(),
        plan.objects.first().map_or(0, |object| object.range.offset),
    )?;
    let mut sorted_pages = Vec::new();
    let refused = limits.allocation_refused("PDF page index allocation", page_index_bytes);
    reserve_exact(&mut sorted_pages, plan.pages.len(), refused)?;
    sorted_pages.extend_from_slice(plan.pages);
    sorted_pages.sort_unstable();
    for pair in sorted_pages.windows(2) {
        if pair[0] == pair[1] {
            return Err(pdf_error(
                Some(pair[1]),
                0,
                PdfErrorKind::AmbiguousRepair,
                "ordered page object is repeated",
            ));
        }
    }

    let synthetic_pages = object_index(&records, plan.pages_root).is_none();
    let largest = records.last().map_or(plan.pages_root.number, |record| {
        record.reference.number.max(plan.pages_root.number)
    });
    // Every record and the pages root passed `checked_reference`, so
    // `largest <= MAX_PDF_OBJECTS` and the successor fits `u32`.
    let catalog = PdfRef {
        number: largest + 1,
        generation: 0,
    };
    checked_reference(catalog, 0)?;
    if synthetic_pages {
        records.push(Record::synthetic(plan.pages_root));
    }
    records.push(Record::synthetic(catalog));
    records.sort_unstable_by_key(|record| record.reference.number);

    // Validate framing, stream lengths, references, and page-tree structure
    // against the supplied source spans before writing anything.
    validate_fragment_structure(
        source,
        plan,
        &records,
        &sorted_pages,
        checked_add(record_bytes, page_index_bytes)?,
        limits,
        cancellation,
    )
    .await?;
    drop(sorted_pages);

    let outline = if bookmarks.is_empty() {
        None
    } else {
        let largest = records.last().ok_or(Error::InvalidInput {
            reason: "fragment has no PDF objects",
        })?;
        // Every record passed `checked_reference`, so the successor fits.
        let root = PdfRef {
            number: largest.reference.number + 1,
            generation: 0,
        };
        checked_reference(root, 0)?;
        let parts = [record_bytes, page_index_bytes, limits.io_chunk_bytes as u64];
        let retained_bytes = checked_sum(&parts)?;
        let (nodes, first, last) =
            build_outline_nodes(bookmarks, plan.pages, root, limits, retained_bytes)?;
        records.push(Record::synthetic(root));
        for node in &nodes {
            records.push(Record::synthetic(node.item.reference));
        }
        records.sort_unstable_by_key(|record| record.reference.number);
        Some((root, nodes, first, last))
    };

    let pages_prefix = synthetic_pages.then(|| {
        format!(
            "{} 0 obj\n<< /Type /Pages /Count {} /Kids [",
            plan.pages_root.number, page_count
        )
    });
    let pages_suffix = b"] >>\nendobj\n";
    let catalog_text = {
        if let Some((outline_root, ..)) = &outline {
            format!(
                "{} 0 obj\n<< /Type /Catalog /Pages {} 0 R /Outlines {} 0 R >>\nendobj\n",
                catalog.number, plan.pages_root.number, outline_root.number
            )
        } else {
            format!(
                "{} 0 obj\n<< /Type /Catalog /Pages {} 0 R >>\nendobj\n",
                catalog.number, plan.pages_root.number
            )
        }
    };

    let output_working_bytes = checked_add(record_bytes, limits.io_chunk_bytes as u64)?;
    check_pdf_allocation(
        limits,
        output_working_bytes,
        Some(plan.pages_root),
        records
            .iter()
            .find(|record| record.reference == plan.pages_root)
            .map_or(0, |record| record.range.offset),
    )?;
    let mut buffer = Vec::new();
    let refused = limits.allocation_refused("PDF I/O buffer allocation", output_working_bytes);
    reserve_exact(&mut buffer, limits.io_chunk_bytes, refused)?;
    buffer.resize(limits.io_chunk_bytes, 0_u8);
    let mut out = Output::new(sink, limits, cancellation);
    out.write(HEADER).await?;
    for record in records.iter_mut().filter(|record| record.range.length != 0) {
        record.output_offset = out.position;
        copy_object(source, &mut out, record.range, &mut buffer).await?;
        out.write(b"\n").await?;
    }
    if let Some(prefix) = pages_prefix {
        let index = object_index(&records, plan.pages_root).ok_or(Error::InvalidInput {
            reason: "synthetic page tree root was not indexed",
        })?;
        records[index].output_offset = out.position;
        out.write(prefix.as_bytes()).await?;
        buffer.clear();
        for page in plan.pages {
            let mut encoded = [0_u8; 16];
            let bytes = page_reference(*page, &mut encoded);
            if bytes.len() > limits.io_chunk_bytes {
                out.write(&buffer).await?;
                buffer.clear();
                out.write(bytes).await?;
                continue;
            }
            if buffer.len() + bytes.len() > limits.io_chunk_bytes {
                out.write(&buffer).await?;
                buffer.clear();
            }
            buffer.extend_from_slice(bytes);
        }
        out.write(&buffer).await?;
        out.write(pages_suffix).await?;
    }
    let index = object_index(&records, catalog).ok_or(Error::InvalidInput {
        reason: "synthetic catalog was not indexed",
    })?;
    records[index].output_offset = out.position;
    out.write(catalog_text.as_bytes()).await?;
    if let Some((root, nodes, first, last)) = &outline {
        let mut objects = SyntheticObjects {
            out: &mut out,
            records: &mut records,
        };
        write_root(&mut objects, *root, *first, *last, bookmark_count).await?;
        for (bookmark, node) in bookmarks.iter().zip(nodes) {
            write_item(&mut objects, &node.item, &bookmark.title).await?;
        }
    }
    let largest = records
        .last()
        .ok_or(Error::InvalidInput {
            reason: "fragment has no PDF objects",
        })?
        .reference
        .number;
    let trailer = Trailer {
        size: u64::from(largest) + 1,
        root: catalog,
        prev: None,
        info: None,
        id: None,
    };
    let entries = records
        .iter()
        .map(|record| (record.reference, record.output_offset));
    write_xref(&mut out, entries, true, &trailer).await?;
    out.flush().await?;
    Ok(ConversionReport {
        input_bytes_read,
        output_bytes_written: out.position,
        pages_converted: page_count,
        bookmarks_written: bookmark_count,
        ..ConversionReport::default()
    })
}

async fn copy_object<R: RangedSource, W: SequentialSink, C: Cancellation>(
    source: &mut R,
    out: &mut Output<'_, W, C>,
    range: PdfRange,
    buffer: &mut [u8],
) -> Result<()> {
    let mut copied = 0;
    while copied < range.length {
        let length = (range.length - copied).min(buffer.len() as u64) as usize;
        let offset = checked_add(range.offset, copied)?;
        read_exact_at(
            source,
            offset,
            &mut buffer[..length],
            out.limits,
            out.cancellation,
        )
        .await?;
        out.write(&buffer[..length]).await?;
        copied += length as u64;
    }
    Ok(())
}

fn page_reference(reference: PdfRef, buffer: &mut [u8; 16]) -> &[u8] {
    let mut digits = [0_u8; 10];
    let mut number = reference.number;
    let mut count = 0;
    loop {
        digits[count] = b'0' + (number % 10) as u8;
        count += 1;
        number /= 10;
        if number == 0 {
            break;
        }
    }
    for position in 0..count {
        buffer[position] = digits[count - position - 1];
    }
    buffer[count..count + 5].copy_from_slice(b" 0 R ");
    &buffer[..count + 5]
}

async fn validate_fragment_structure<R: RangedSource, C: Cancellation>(
    source: &mut R,
    plan: &FragmentPlan<'_>,
    records: &[Record],
    sorted_pages: &[PdfRef],
    index_bytes: u64,
    limits: &Limits,
    cancellation: &C,
) -> Result<()> {
    let scalar_bytes =
        records
            .len()
            .checked_mul(size_of::<Option<u64>>())
            .ok_or(Error::InvalidInput {
                reason: "PDF scalar index allocation overflows address space",
            })?;
    let scalar_bytes = len_u64(scalar_bytes);
    check_pdf_allocation(
        limits,
        checked_add(index_bytes, scalar_bytes)?,
        plan.objects.first().map(|object| object.reference),
        plan.objects.first().map_or(0, |object| object.range.offset),
    )?;
    let mut scalars = Vec::new();
    let refused = limits.allocation_refused("PDF scalar index allocation", scalar_bytes);
    reserve_exact(&mut scalars, records.len(), refused)?;
    for record in records {
        let scalar = if record.range.length == 0 {
            None
        } else {
            inspect_fragment_scalar(source, record.range, record.reference, limits, cancellation)
                .await?
        };
        scalars.push(scalar);
    }

    let kind_bytes = records
        .len()
        .checked_mul(size_of::<Option<FragmentKind>>())
        .ok_or(Error::InvalidInput {
            reason: "PDF structure index allocation overflows address space",
        })?;
    let kind_bytes = len_u64(kind_bytes);
    let stream_bytes = len_u64(records.len());
    let parts = [index_bytes, scalar_bytes, kind_bytes, stream_bytes];
    let retained_base = checked_sum(&parts)?;
    check_pdf_allocation(
        limits,
        retained_base,
        plan.objects.first().map(|object| object.reference),
        plan.objects.first().map_or(0, |object| object.range.offset),
    )?;
    let mut kinds = Vec::new();
    let refused = limits.allocation_refused("PDF structure index allocation", kind_bytes);
    reserve_exact(&mut kinds, records.len(), refused)?;
    let mut retained_structure_bytes = retained_base;
    let mut stream_flags = Vec::new();
    let refused = limits.allocation_refused("PDF stream index allocation", stream_bytes);
    reserve_exact(&mut stream_flags, records.len(), refused)?;
    let mut content_evidence = Vec::new();
    // At most one destination per record, so a `u64` count cannot overflow.
    let mut destination_count = 0_u64;
    for record in records {
        let kind = if record.range.length == 0 {
            None
        } else {
            let inspection = inspect_fragment_object(
                source,
                record.range,
                record.reference,
                limits,
                cancellation,
                |reference| {
                    object_index(records, reference).and_then(|index| {
                        (records[index].reference == reference)
                            .then_some(scalars[index])
                            .flatten()
                    })
                },
            )
            .await?;
            // The inspector rejects any header other than the expected one.
            debug_assert_eq!(inspection.reference, record.reference);
            if inspection.max_referenced_object > MAX_PDF_OBJECTS {
                return Err(pdf_error(
                    Some(record.reference),
                    record.range.offset,
                    PdfErrorKind::UnsupportedFeature,
                    "indirect reference exceeds the supported PDF profile",
                ));
            }
            for reference in inspection.references {
                checked_reference(reference, record.range.offset)?;
                if object_index(records, reference).is_none() {
                    return Err(malformed(
                        Some(record.reference),
                        record.range.offset,
                        "indirect reference targets a missing object",
                    ));
                }
            }
            if let Some(destination) = inspection.destination {
                checked_reference(destination, record.range.offset)?;
                if sorted_pages.binary_search(&destination).is_err() {
                    return Err(malformed(
                        Some(record.reference),
                        record.range.offset,
                        "outline destination does not target an ordered Page object",
                    ));
                }
                destination_count += 1;
                if destination_count > u64::from(limits.max_bookmarks) {
                    return Err(pdf_limit(
                        Some(record.reference),
                        record.range.offset,
                        "bookmarks",
                        u64::from(limits.max_bookmarks),
                        destination_count,
                    ));
                }
            }
            if let FragmentKind::Pages { kids, .. } = &inspection.kind {
                let kids_bytes = kids
                    .len()
                    .checked_mul(size_of::<PdfRef>())
                    .map(len_u64)
                    .ok_or(Error::InvalidInput {
                        reason: "PDF page-tree child allocation overflows 64 bits",
                    })?;
                retained_structure_bytes = checked_add(retained_structure_bytes, kids_bytes)?;
                check_pdf_allocation(
                    limits,
                    retained_structure_bytes,
                    Some(record.reference),
                    record.range.offset,
                )?;
            }
            if let Some(references) = inspection.contents {
                retain_content_evidence(
                    &mut content_evidence,
                    &mut retained_structure_bytes,
                    ContentEvidence {
                        reference: record.reference,
                        kind: ContentEvidenceKind::Page {
                            direct_array: inspection.contents_is_direct_array,
                        },
                        references,
                    },
                    record.range.offset,
                    limits,
                )?;
            }
            if let Some(references) = inspection.scalar_reference_array {
                retain_content_evidence(
                    &mut content_evidence,
                    &mut retained_structure_bytes,
                    ContentEvidence {
                        reference: record.reference,
                        kind: ContentEvidenceKind::ScalarArray,
                        references,
                    },
                    record.range.offset,
                    limits,
                )?;
            }
            stream_flags.push(inspection.is_stream);
            Some(inspection.kind)
        };
        if kind.is_none() {
            stream_flags.push(false);
        }
        kinds.push(kind);
    }
    validate_fragment_contents(
        records,
        &stream_flags,
        &content_evidence,
        retained_structure_bytes,
        limits,
    )?;
    drop(scalars);
    let retained_without_scalars =
        retained_structure_bytes
            .checked_sub(scalar_bytes)
            .ok_or(Error::InvalidInput {
                reason: "PDF validation memory accounting underflowed",
            })?;

    let mut found_pages = 0_u32;
    for (index, kind) in kinds.iter().enumerate() {
        match kind {
            Some(FragmentKind::Page { .. }) => found_pages += 1,
            Some(FragmentKind::Catalog) => {
                return Err(pdf_error(
                    Some(records[index].reference),
                    records[index].range.offset,
                    PdfErrorKind::AmbiguousRepair,
                    "unselected catalog object is present",
                ));
            }
            _ => {}
        }
    }
    if found_pages != plan.pages.len() as u32 {
        return Err(pdf_error(
            None,
            0,
            PdfErrorKind::AmbiguousRepair,
            "supplied page objects do not match explicit page order",
        ));
    }
    let failure = malformed(
        Some(plan.pages_root),
        0,
        "page tree root is missing from the repair index",
    );
    let root_index = object_index(records, plan.pages_root).ok_or(failure)?;
    if records[root_index].range.length == 0 {
        for (index, kind) in kinds.iter().enumerate() {
            match kind {
                Some(FragmentKind::Page {
                    parent,
                    has_media_box,
                }) => {
                    if *parent != plan.pages_root {
                        return Err(pdf_error(
                            Some(records[index].reference),
                            records[index].range.offset,
                            PdfErrorKind::AmbiguousRepair,
                            "page parent differs from the missing root",
                        ));
                    }
                    if !has_media_box {
                        return Err(malformed(
                            Some(records[index].reference),
                            records[index].range.offset,
                            "Page has no MediaBox and the synthesized root cannot provide one",
                        ));
                    }
                }
                Some(FragmentKind::Pages { .. }) => {
                    return Err(pdf_error(
                        Some(records[index].reference),
                        records[index].range.offset,
                        PdfErrorKind::AmbiguousRepair,
                        "missing page root cannot be synthesized around nested Pages nodes",
                    ));
                }
                _ => {}
            }
        }
        for page in plan.pages {
            let failure = malformed(Some(*page), 0, "ordered page object is missing");
            let index = object_index(records, *page).ok_or(failure)?;
            if !matches!(kinds[index], Some(FragmentKind::Page { .. })) {
                return Err(malformed(
                    Some(*page),
                    records[index].range.offset,
                    "ordered page reference is not a Page object",
                ));
            }
        }
    } else {
        validate_existing_page_tree(plan, records, &kinds, retained_without_scalars, limits)?;
    }
    Ok(())
}

fn retain_content_evidence(
    evidence: &mut Vec<ContentEvidence>,
    retained_bytes: &mut u64,
    item: ContentEvidence,
    offset: u64,
    limits: &Limits,
) -> Result<()> {
    let reference_bytes = item
        .references
        .len()
        .checked_mul(size_of::<PdfRef>())
        .map(len_u64)
        .ok_or(Error::InvalidInput {
            reason: "PDF content reference allocation overflows 64 bits",
        })?;
    let item_bytes = checked_add(size_of::<ContentEvidence>() as u64, reference_bytes)?;
    *retained_bytes = checked_add(*retained_bytes, item_bytes)?;
    check_pdf_allocation(limits, *retained_bytes, Some(item.reference), offset)?;
    let refused = limits.allocation_refused("PDF content evidence allocation", *retained_bytes);
    reserve_exact(evidence, 1, refused)?;
    evidence.push(item);
    Ok(())
}

fn validate_fragment_contents(
    records: &[Record],
    stream_flags: &[bool],
    evidence: &[ContentEvidence],
    retained_bytes: u64,
    limits: &Limits,
) -> Result<()> {
    let cache_bytes = len_u64(evidence.len());
    let first = evidence.first();
    let first_offset = first
        .and_then(|item| object_index(records, item.reference))
        .and_then(|index| records.get(index))
        .map_or(0, |record| record.range.offset);
    check_pdf_allocation(
        limits,
        checked_add(retained_bytes, cache_bytes)?,
        first.map(|item| item.reference),
        first_offset,
    )?;
    let mut validated_arrays = Vec::new();
    let refused =
        limits.allocation_refused("PDF content-array validation index allocation", cache_bytes);
    reserve_exact(&mut validated_arrays, evidence.len(), refused)?;
    validated_arrays.resize(evidence.len(), false);
    for item in evidence {
        let ContentEvidenceKind::Page { direct_array } = item.kind else {
            continue;
        };
        let page_index = object_index(records, item.reference).ok_or(Error::InvalidInput {
            reason: "validated Page content evidence is absent from the object index",
        })?;
        let page_offset = records[page_index].range.offset;
        if direct_array {
            for reference in &item.references {
                require_content_stream(
                    records,
                    stream_flags,
                    *reference,
                    item.reference,
                    page_offset,
                )?;
            }
            continue;
        }
        let failure = malformed(
            Some(item.reference),
            page_offset,
            "Page Contents lacks an indirect target",
        );
        let target = item.references.first().copied().ok_or(failure)?;
        let failure = malformed(
            Some(item.reference),
            page_offset,
            "Page Contents targets a missing object",
        );
        let target_index = object_index(records, target).ok_or(failure)?;
        if stream_flags[target_index] {
            continue;
        }
        let array_index = evidence
            .binary_search_by_key(&target.number, |candidate| candidate.reference.number)
            .ok();
        let Some((array_index, array)) = array_index
            .and_then(|index| evidence.get(index).map(|candidate| (index, candidate)))
            .filter(|(_, candidate)| matches!(candidate.kind, ContentEvidenceKind::ScalarArray))
        else {
            return Err(malformed(
                Some(item.reference),
                page_offset,
                "Page Contents target is neither a stream nor a stream reference array",
            ));
        };
        if !validated_arrays[array_index] {
            for reference in &array.references {
                require_content_stream(
                    records,
                    stream_flags,
                    *reference,
                    item.reference,
                    page_offset,
                )?;
            }
            validated_arrays[array_index] = true;
        }
    }
    Ok(())
}

fn require_content_stream(
    records: &[Record],
    stream_flags: &[bool],
    target: PdfRef,
    page: PdfRef,
    page_offset: u64,
) -> Result<()> {
    let failure = malformed(
        Some(page),
        page_offset,
        "Page Contents targets a missing object",
    );
    let index = object_index(records, target).ok_or(failure)?;
    if !stream_flags[index] {
        return Err(malformed(
            Some(page),
            page_offset,
            "Page Contents array contains a non-stream object",
        ));
    }
    Ok(())
}

fn validate_existing_page_tree(
    plan: &FragmentPlan<'_>,
    records: &[Record],
    kinds: &[Option<FragmentKind>],
    retained_bytes: u64,
    limits: &Limits,
) -> Result<()> {
    let root_offset = object_index(records, plan.pages_root)
        .and_then(|index| records.get(index))
        .map_or(0, |record| record.range.offset);
    let visit_bytes = len_u64(records.len());
    let with_visited = checked_add(retained_bytes, visit_bytes)?;
    check_pdf_allocation(limits, with_visited, Some(plan.pages_root), root_offset)?;
    let mut visited = Vec::new();
    let refused = limits.allocation_refused("PDF page-tree visited index allocation", visit_bytes);
    reserve_exact(&mut visited, records.len(), refused)?;
    visited.resize(records.len(), false);

    let stack_capacity = records
        .len()
        .checked_mul(2)
        .and_then(|n| n.checked_add(1))
        .ok_or(Error::InvalidInput {
            reason: "PDF page-tree traversal allocation overflows address space",
        })?;
    let stack_bytes =
        stack_capacity
            .checked_mul(size_of::<PageStep>())
            .ok_or(Error::InvalidInput {
                reason: "PDF page-tree traversal allocation overflows address space",
            })?;
    let stack_bytes = len_u64(stack_bytes);
    check_pdf_allocation(
        limits,
        checked_add(with_visited, stack_bytes)?,
        Some(plan.pages_root),
        root_offset,
    )?;
    let mut stack = Vec::new();
    let refused = limits.allocation_refused("PDF page-tree traversal allocation", stack_bytes);
    reserve_exact(&mut stack, stack_capacity, refused)?;
    // The reserved stack has room for every indexed object twice, so only a
    // page tree that links more children than that can exhaust it.
    let push_within = |reference, offset| {
        move |stack: &mut Vec<PageStep>, step| {
            if stack.len() >= stack_capacity {
                return Err(pdf_error(
                    Some(reference),
                    offset,
                    PdfErrorKind::AmbiguousRepair,
                    "page tree has more child links than indexed objects",
                ));
            }
            stack.push(step);
            Ok(())
        }
    };
    let mut walk = PageWalk::new(plan.pages_root, stack, push_within(plan.pages_root, 0))?;
    let mut leaves = 0_usize;
    while let Some(visit) = walk.next(leaves) {
        let node = visit.map_err(|reference| {
            malformed(
                Some(reference),
                0,
                "Pages /Count differs from descendant page count",
            )
        })?;
        let reference = node.reference;
        let failure = malformed(Some(reference), 0, "page-tree child object is missing");
        let index = object_index(records, reference).ok_or(failure)?;
        let offset = records[index].range.offset;
        if visited[index] {
            return Err(malformed(
                Some(reference),
                offset,
                "page tree contains a repeated child or cycle",
            ));
        }
        visited[index] = true;
        match kinds[index].as_ref() {
            Some(FragmentKind::Page {
                parent,
                has_media_box,
            }) => {
                if Some(*parent) != node.parent {
                    return Err(malformed(
                        Some(reference),
                        offset,
                        "Page /Parent link does not match the page tree",
                    ));
                }
                if !has_media_box && !node.inherited_media_box {
                    return Err(malformed(
                        Some(reference),
                        offset,
                        "Page has no direct or inherited MediaBox",
                    ));
                }
                if plan.pages.get(leaves) != Some(&reference) {
                    return Err(pdf_error(
                        Some(reference),
                        offset,
                        PdfErrorKind::AmbiguousRepair,
                        "page-tree order differs from explicit page order",
                    ));
                }
                leaves += 1;
            }
            Some(FragmentKind::Pages {
                parent,
                count,
                kids,
                has_media_box,
            }) => {
                if *parent != node.parent {
                    return Err(malformed(
                        Some(reference),
                        offset,
                        "Pages /Parent link does not match the page tree",
                    ));
                }
                if *count == 0 || kids.is_empty() {
                    return Err(malformed(Some(reference), offset, "Pages node is empty"));
                }
                if u64::from(*count) > u64::from(limits.max_pages) {
                    return Err(pdf_limit(
                        Some(reference),
                        offset,
                        "page-tree count",
                        u64::from(limits.max_pages),
                        u64::from(*count),
                    ));
                }
                let media_box = node.inherited_media_box || *has_media_box;
                let push = push_within(reference, offset);
                walk.push_kids(reference, *count, kids, media_box, leaves, push)?;
            }
            _ => {
                return Err(malformed(
                    Some(reference),
                    offset,
                    "page-tree child has neither Page nor Pages type",
                ));
            }
        }
    }
    if leaves != plan.pages.len() {
        return Err(pdf_error(
            Some(plan.pages_root),
            0,
            PdfErrorKind::AmbiguousRepair,
            "page tree omits an explicitly ordered page",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
