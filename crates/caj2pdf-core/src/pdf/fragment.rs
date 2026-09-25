// SPDX-License-Identifier: MIT

//! Bounded reconstruction of indexed PDF object fragments.
//!
//! A format handler supplies the exact source span of every indirect object
//! and the intended page order. This module never infers page order from
//! object numbers or searches binary stream payloads for PDF delimiters.

use super::input::{FragmentKind, inspect_fragment_object, inspect_fragment_scalar};
use super::writer::{MAX_PDF_OBJECTS, check_classic_pdf_bytes, checked_object_number};
use super::{MAX_CLASSIC_PDF_BYTES, PdfRange, PdfRef};
use crate::fallible::{checked_read_count, len_u64, reserve_exact, usize_from_u32};
use crate::{
    Bookmark, Cancellation, ConversionReport, Error, Limits, PdfErrorKind, RangedSource, Result,
    SequentialSink, read_exact_at, write_all,
};
use std::mem::size_of;

const HEADER: &[u8] = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n";
const HEX: &[u8; 16] = b"0123456789ABCDEF";
const MAX_OUTLINE_DEPTH: usize = 256;

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
/// synthesized. A missing catalog is synthesized at the next available object
/// number. Existing root objects are retained only when their references are
/// consistent with this plan.
#[derive(Clone, Copy, Debug)]
pub struct FragmentPlan<'a> {
    pub objects: &'a [FragmentObject],
    pub pages: &'a [PdfRef],
    pub pages_root: PdfRef,
    pub catalog: Option<PdfRef>,
}

#[derive(Clone, Copy, Debug)]
struct Record {
    reference: PdfRef,
    range: PdfRange,
    output_offset: u64,
}

#[derive(Clone, Copy)]
struct OutlineNode {
    reference: PdfRef,
    parent: PdfRef,
    parent_index: Option<usize>,
    previous: Option<PdfRef>,
    next: Option<PdfRef>,
    first_child: Option<PdfRef>,
    last_child: Option<PdfRef>,
    descendants: u32,
    page: PdfRef,
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

fn decimal_digits(mut value: u64) -> u64 {
    let mut digits = 1;
    while value >= 10 {
        value /= 10;
        digits += 1;
    }
    digits
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
            nodes[parent_index].last_child
        } else {
            last_root.map(|previous| nodes[previous].reference)
        };
        let parent = parent_index.map_or(root, |parent_index| nodes[parent_index].reference);
        nodes.push(OutlineNode {
            reference,
            parent,
            parent_index,
            previous: previous_index,
            next: None,
            first_child: None,
            last_child: None,
            descendants: 0,
            page,
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
            nodes[previous_position].next = Some(reference);
        }
        if let Some(parent_index) = parent_index {
            if nodes[parent_index].first_child.is_none() {
                nodes[parent_index].first_child = Some(reference);
            }
            nodes[parent_index].last_child = Some(reference);
        } else {
            first_root.get_or_insert(reference);
            last_root = Some(index);
        }
        stack[depth] = Some(index);
        previous_depth = depth;
    }
    for index in (0..nodes.len()).rev() {
        if let Some(parent) = nodes[index].parent_index {
            let subtree = nodes[index]
                .descendants
                .checked_add(1)
                .ok_or(Error::InvalidInput {
                    reason: "PDF outline descendant count overflows",
                })?;
            nodes[parent].descendants =
                nodes[parent]
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
    .reference;
    Ok((nodes, first_root, last_root))
}

fn outline_item_prefix(node: &OutlineNode) -> String {
    format!("{} 0 obj\n<< /Title <FEFF", node.reference.number)
}

fn outline_item_suffix(node: &OutlineNode) -> String {
    let mut suffix = format!(
        "> /Parent {} 0 R /Dest [{} 0 R /XYZ null null null]",
        node.parent.number, node.page.number
    );
    if let Some(previous) = node.previous {
        suffix.push_str(&format!(" /Prev {} 0 R", previous.number));
    }
    if let Some(next) = node.next {
        suffix.push_str(&format!(" /Next {} 0 R", next.number));
    }
    if let (Some(first), Some(last)) = (node.first_child, node.last_child) {
        suffix.push_str(&format!(
            " /First {} 0 R /Last {} 0 R /Count {}",
            first.number, last.number, node.descendants
        ));
    }
    suffix.push_str(" >>\nendobj\n");
    suffix
}

fn outline_title_hex_len(title: &str) -> Result<u64> {
    let units = len_u64(title.encode_utf16().count());
    units.checked_mul(4).ok_or(Error::InvalidInput {
        reason: "PDF outline title hex length overflows 64 bits",
    })
}

async fn emit_outline_item<W: SequentialSink, C: Cancellation>(
    sink: &mut W,
    report: &mut ConversionReport,
    bookmark: &Bookmark,
    node: &OutlineNode,
    limits: &Limits,
    cancellation: &C,
) -> Result<()> {
    emit(
        sink,
        outline_item_prefix(node).as_bytes(),
        report,
        limits,
        cancellation,
    )
    .await?;
    let mut hex = [0_u8; 4096];
    let mut used = 0;
    for unit in bookmark.title.encode_utf16() {
        if used == hex.len() {
            emit(sink, &hex, report, limits, cancellation).await?;
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
    emit(sink, &hex[..used], report, limits, cancellation).await?;
    emit(
        sink,
        outline_item_suffix(node).as_bytes(),
        report,
        limits,
        cancellation,
    )
    .await
}

/// Reconstruct one PDF from indexed indirect objects and explicit page order.
///
/// All plan and source validation precedes the first sink write. A subsequent
/// source, sink, or cancellation failure leaves a partial output and returns
/// an error, never a successful report. Source object bytes are copied in
/// bounded chunks without materializing a whole object or PDF in memory.
pub async fn reconstruct_fragment<R: RangedSource, W: SequentialSink, C: Cancellation>(
    source: &mut R,
    sink: &mut W,
    plan: &FragmentPlan<'_>,
    limits: &Limits,
    cancellation: &C,
) -> Result<ConversionReport> {
    reconstruct_fragment_with_bookmarks(source, sink, plan, &[], limits, cancellation).await
}

/// Reconstruct PDF fragments and write a CAJ outline in the same PDF revision.
///
/// Bookmark entries are depth-first and name zero-based positions in
/// `plan.pages`. All titles and links are checked before the first sink write;
/// title hex is then emitted in bounded chunks. Existing fragment Catalog
/// objects cannot currently be updated with a new outline.
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
    let mut counted = CountingSource {
        inner: source,
        bytes_read: 0,
    };
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
    if bookmark_count != 0 && plan.catalog.is_some() {
        return Err(pdf_error(
            plan.catalog,
            0,
            PdfErrorKind::UnsupportedFeature,
            "fragment outline import requires a synthetic Catalog",
        ));
    }
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
    if let Some(catalog) = plan.catalog {
        checked_reference(catalog, 0)?;
    }
    if cancellation.is_cancelled() {
        return Err(Error::Cancelled);
    }

    let requested = plan
        .objects
        .len()
        .checked_add(2)
        .and_then(|count| {
            if bookmarks.is_empty() {
                Some(count)
            } else {
                bookmarks
                    .len()
                    .checked_add(1)
                    .and_then(|n| count.checked_add(n))
            }
        })
        .ok_or(Error::InvalidInput {
            reason: "PDF object count overflows address space",
        })?;
    let first_object = plan.objects.first();
    let first_offset = first_object.map_or(0, |object| object.range.offset);
    let first_reference = first_object.map(|object| {
        let reference = object.reference;
        (reference.number, reference.generation)
    });
    checked_object_number(requested)
        .map_err(|error| error.locate_pdf_limit(first_offset, first_reference))?;
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
    let catalog = if let Some(reference) = plan.catalog {
        if object_index(&records, reference).is_none() {
            return Err(malformed(Some(reference), 0, "catalog object is missing"));
        }
        reference
    } else {
        let largest = records.last().map_or(plan.pages_root.number, |record| {
            record.reference.number.max(plan.pages_root.number)
        });
        // Every record and the pages root passed `checked_reference`, so
        // `largest <= MAX_PDF_OBJECTS` and the successor fits `u32`.
        let number = largest + 1;
        let reference = PdfRef {
            number,
            generation: 0,
        };
        checked_reference(reference, 0)?;
        reference
    };
    if synthetic_pages {
        records.push(Record::synthetic(plan.pages_root));
    }
    if plan.catalog.is_none() {
        records.push(Record::synthetic(catalog));
    }
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
        let retained_bytes = checked_add(
            checked_add(record_bytes, page_index_bytes)?,
            limits.io_chunk_bytes as u64,
        )?;
        let (nodes, first, last) =
            build_outline_nodes(bookmarks, plan.pages, root, limits, retained_bytes)?;
        let root_text = format!(
            "{} 0 obj\n<< /Type /Outlines /First {} 0 R /Last {} 0 R /Count {} >>\nendobj\n",
            root.number, first.number, last.number, bookmark_count
        );
        records.push(Record::synthetic(root));
        for node in &nodes {
            records.push(Record::synthetic(node.reference));
        }
        records.sort_unstable_by_key(|record| record.reference.number);
        Some((root, nodes, root_text))
    };

    let pages_prefix = synthetic_pages.then(|| {
        format!(
            "{} 0 obj\n<< /Type /Pages /Count {} /Kids [",
            plan.pages_root.number, page_count
        )
    });
    let pages_suffix = b"] >>\nendobj\n";
    let catalog_text = plan.catalog.is_none().then(|| {
        if let Some((outline_root, _, _)) = &outline {
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
    });
    let mut body_bytes = HEADER.len() as u64;
    for record in &records {
        if record.range.length != 0 {
            body_bytes = checked_add(body_bytes, checked_add(record.range.length, 1)?)?;
        }
    }
    if let Some(prefix) = &pages_prefix {
        body_bytes = checked_add(body_bytes, prefix.len() as u64)?;
        for page in plan.pages {
            body_bytes = checked_add(body_bytes, decimal_digits(u64::from(page.number)) + 5)?;
        }
        body_bytes = checked_add(body_bytes, pages_suffix.len() as u64)?;
    }
    if let Some(text) = &catalog_text {
        body_bytes = checked_add(body_bytes, text.len() as u64)?;
    }
    if let Some((_, nodes, root_text)) = &outline {
        body_bytes = checked_add(body_bytes, root_text.len() as u64)?;
        for (bookmark, node) in bookmarks.iter().zip(nodes) {
            let framing = checked_add(
                outline_item_prefix(node).len() as u64,
                outline_item_suffix(node).len() as u64,
            )?;
            body_bytes = checked_add(
                body_bytes,
                checked_add(framing, outline_title_hex_len(&bookmark.title)?)?,
            )?;
        }
    }
    let largest = records
        .last()
        .ok_or(Error::InvalidInput {
            reason: "fragment has no PDF objects",
        })?
        .reference
        .number;
    let xref_size = u64::from(largest) + 1;
    let xref_header = format!("xref\n0 {xref_size}\n");
    let trailer = format!(
        "trailer\n<< /Size {xref_size} /Root {} 0 R >>\nstartxref\n{body_bytes}\n%%EOF\n",
        catalog.number
    );
    // `largest` is a `u32`, so this product fits `u64`.
    let xref_bytes = xref_size * 20;
    let final_size = checked_add(
        checked_add(body_bytes, xref_header.len() as u64)?,
        checked_add(xref_bytes, trailer.len() as u64)?,
    )?;
    if final_size > MAX_CLASSIC_PDF_BYTES {
        return Err(pdf_limit(
            Some(catalog),
            records
                .iter()
                .find(|record| record.reference == catalog)
                .map_or(0, |record| record.range.offset),
            "classic PDF file bytes",
            MAX_CLASSIC_PDF_BYTES,
            final_size,
        ));
    }
    if final_size > limits.max_output_bytes {
        return Err(pdf_limit(
            Some(catalog),
            records
                .iter()
                .find(|record| record.reference == catalog)
                .map_or(0, |record| record.range.offset),
            "output bytes",
            limits.max_output_bytes,
            final_size,
        ));
    }

    let mut report = ConversionReport {
        pages_converted: page_count,
        bookmarks_written: bookmark_count,
        ..ConversionReport::default()
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
    emit(sink, HEADER, &mut report, limits, cancellation).await?;
    for record in records.iter_mut().filter(|record| record.range.length != 0) {
        record.output_offset = report.output_bytes_written;
        copy_object(
            source,
            sink,
            record.range,
            &mut buffer,
            &mut report,
            limits,
            cancellation,
        )
        .await?;
        emit(sink, b"\n", &mut report, limits, cancellation).await?;
    }
    if let Some(prefix) = pages_prefix {
        let index = object_index(&records, plan.pages_root).ok_or(Error::InvalidInput {
            reason: "synthetic page tree root was not indexed",
        })?;
        records[index].output_offset = report.output_bytes_written;
        emit(sink, prefix.as_bytes(), &mut report, limits, cancellation).await?;
        buffer.clear();
        for page in plan.pages {
            let mut encoded = [0_u8; 16];
            let bytes = page_reference(*page, &mut encoded);
            if bytes.len() > limits.io_chunk_bytes {
                emit(sink, &buffer, &mut report, limits, cancellation).await?;
                buffer.clear();
                emit(sink, bytes, &mut report, limits, cancellation).await?;
                continue;
            }
            if buffer.len() + bytes.len() > limits.io_chunk_bytes {
                emit(sink, &buffer, &mut report, limits, cancellation).await?;
                buffer.clear();
            }
            buffer.extend_from_slice(bytes);
        }
        emit(sink, &buffer, &mut report, limits, cancellation).await?;
        emit(sink, pages_suffix, &mut report, limits, cancellation).await?;
    }
    if let Some(text) = catalog_text {
        let index = object_index(&records, catalog).ok_or(Error::InvalidInput {
            reason: "synthetic catalog was not indexed",
        })?;
        records[index].output_offset = report.output_bytes_written;
        emit(sink, text.as_bytes(), &mut report, limits, cancellation).await?;
    }
    if let Some((root, nodes, root_text)) = &outline {
        let index = object_index(&records, *root).ok_or(Error::InvalidInput {
            reason: "synthetic outline root was not indexed",
        })?;
        records[index].output_offset = report.output_bytes_written;
        emit(
            sink,
            root_text.as_bytes(),
            &mut report,
            limits,
            cancellation,
        )
        .await?;
        for (bookmark, node) in bookmarks.iter().zip(nodes) {
            let index = object_index(&records, node.reference).ok_or(Error::InvalidInput {
                reason: "synthetic outline item was not indexed",
            })?;
            records[index].output_offset = report.output_bytes_written;
            emit_outline_item(sink, &mut report, bookmark, node, limits, cancellation).await?;
        }
    }
    if report.output_bytes_written != body_bytes {
        return Err(Error::InvalidInput {
            reason: "PDF body byte count differs from its preflight size",
        });
    }
    emit(
        sink,
        xref_header.as_bytes(),
        &mut report,
        limits,
        cancellation,
    )
    .await?;
    buffer.clear();
    let mut record_index = 0;
    for number in 0..=largest {
        let entry = if number != 0
            && records
                .get(record_index)
                .is_some_and(|record| record.reference.number == number)
        {
            let record = records[record_index];
            record_index += 1;
            xref_entry(record.output_offset, 0, b'n')?
        } else {
            let next_free = next_free_number(number, largest, &records, record_index);
            // A `u32` object number always fits the ten-digit field.
            xref_digits(
                u64::from(next_free),
                if number == 0 { 65_535 } else { 0 },
                b'f',
            )
        };
        if entry.len() > limits.io_chunk_bytes {
            emit(sink, &buffer, &mut report, limits, cancellation).await?;
            buffer.clear();
            emit(sink, &entry, &mut report, limits, cancellation).await?;
            continue;
        }
        if buffer.len() + entry.len() > limits.io_chunk_bytes {
            emit(sink, &buffer, &mut report, limits, cancellation).await?;
            buffer.clear();
        }
        buffer.extend_from_slice(&entry);
    }
    emit(sink, &buffer, &mut report, limits, cancellation).await?;
    emit(sink, trailer.as_bytes(), &mut report, limits, cancellation).await?;
    if report.output_bytes_written != final_size {
        return Err(Error::InvalidInput {
            reason: "PDF final byte count differs from its preflight size",
        });
    }
    if cancellation.is_cancelled() {
        return Err(Error::Cancelled);
    }
    sink.flush().await?;
    if cancellation.is_cancelled() {
        return Err(Error::Cancelled);
    }
    report.input_bytes_read = counted.bytes_read;
    Ok(report)
}

async fn emit<W: SequentialSink, C: Cancellation>(
    sink: &mut W,
    bytes: &[u8],
    report: &mut ConversionReport,
    limits: &Limits,
    cancellation: &C,
) -> Result<()> {
    let attempted = checked_add(report.output_bytes_written, bytes.len() as u64)?;
    check_classic_pdf_bytes(attempted)?;
    write_all(
        sink,
        bytes,
        &mut report.output_bytes_written,
        limits,
        cancellation,
    )
    .await
}

async fn copy_object<R: RangedSource, W: SequentialSink, C: Cancellation>(
    source: &mut R,
    sink: &mut W,
    range: PdfRange,
    buffer: &mut [u8],
    report: &mut ConversionReport,
    limits: &Limits,
    cancellation: &C,
) -> Result<()> {
    let mut copied = 0;
    while copied < range.length {
        let length = (range.length - copied).min(buffer.len() as u64) as usize;
        let offset = checked_add(range.offset, copied)?;
        read_exact_at(source, offset, &mut buffer[..length], limits, cancellation).await?;
        emit(sink, &buffer[..length], report, limits, cancellation).await?;
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

fn xref_entry(offset: u64, generation: u16, status: u8) -> Result<[u8; 20]> {
    if offset > MAX_CLASSIC_PDF_BYTES {
        return Err(Error::LimitExceeded {
            resource: "classic PDF xref offset",
            limit: MAX_CLASSIC_PDF_BYTES,
            attempted: offset,
        });
    }
    Ok(xref_digits(offset, generation, status))
}

/// Format an xref entry whose `offset` has at most ten decimal digits.
fn xref_digits(offset: u64, generation: u16, status: u8) -> [u8; 20] {
    let mut entry = *b"0000000000 00000 n \n";
    let mut value = offset;
    for digit in entry[..10].iter_mut().rev() {
        *digit = b'0' + (value % 10) as u8;
        value /= 10;
    }
    let mut value = generation;
    for digit in entry[11..16].iter_mut().rev() {
        *digit = b'0' + (value % 10) as u8;
        value /= 10;
    }
    entry[17] = status;
    entry
}

fn next_free_number(current: u32, largest: u32, records: &[Record], mut index: usize) -> u32 {
    let mut candidate = current.saturating_add(1);
    while candidate <= largest {
        while records
            .get(index)
            .is_some_and(|record| record.reference.number < candidate)
        {
            index += 1;
        }
        if records
            .get(index)
            .is_some_and(|record| record.reference.number == candidate)
        {
            candidate += 1;
        } else {
            return candidate;
        }
    }
    0
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
    let retained_base = checked_add(
        checked_add(checked_add(index_bytes, scalar_bytes)?, kind_bytes)?,
        stream_bytes,
    )?;
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
    let mut found_catalogs = 0_u32;
    for (index, kind) in kinds.iter().enumerate() {
        match kind {
            Some(FragmentKind::Page { .. }) => found_pages += 1,
            Some(FragmentKind::Catalog { .. }) => {
                found_catalogs += 1;
                if Some(records[index].reference) != plan.catalog {
                    return Err(pdf_error(
                        Some(records[index].reference),
                        records[index].range.offset,
                        PdfErrorKind::AmbiguousRepair,
                        "unselected catalog object is present",
                    ));
                }
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
    if (plan.catalog.is_some() && found_catalogs != 1)
        || (plan.catalog.is_none() && found_catalogs != 0)
    {
        return Err(malformed(
            plan.catalog,
            0,
            "catalog object has the wrong type",
        ));
    }
    if let Some(catalog) = plan.catalog {
        let failure = malformed(Some(catalog), 0, "catalog object is missing");
        let index = object_index(records, catalog).ok_or(failure)?;
        match &kinds[index] {
            Some(FragmentKind::Catalog { pages }) if *pages == plan.pages_root => {}
            _ => {
                return Err(malformed(
                    Some(catalog),
                    records[index].range.offset,
                    "catalog does not reference the selected page tree",
                ));
            }
        }
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

#[derive(Clone, Copy)]
enum WalkStep {
    Enter {
        reference: PdfRef,
        parent: Option<PdfRef>,
        inherited_media_box: bool,
    },
    Exit {
        reference: PdfRef,
        leaves_before: usize,
        declared_count: u32,
    },
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
            .checked_mul(size_of::<WalkStep>())
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
    stack.push(WalkStep::Enter {
        reference: plan.pages_root,
        parent: None,
        inherited_media_box: false,
    });
    let mut leaves = 0_usize;
    while let Some(step) = stack.pop() {
        match step {
            WalkStep::Enter {
                reference,
                parent,
                inherited_media_box,
            } => {
                let failure = malformed(Some(reference), 0, "page-tree child object is missing");
                let index = object_index(records, reference).ok_or(failure)?;
                if visited[index] {
                    return Err(malformed(
                        Some(reference),
                        records[index].range.offset,
                        "page tree contains a repeated child or cycle",
                    ));
                }
                visited[index] = true;
                match kinds[index].as_ref() {
                    Some(FragmentKind::Page {
                        parent: actual,
                        has_media_box,
                    }) => {
                        if Some(*actual) != parent {
                            return Err(malformed(
                                Some(reference),
                                records[index].range.offset,
                                "Page /Parent link does not match the page tree",
                            ));
                        }
                        if !has_media_box && !inherited_media_box {
                            return Err(malformed(
                                Some(reference),
                                records[index].range.offset,
                                "Page has no direct or inherited MediaBox",
                            ));
                        }
                        if plan.pages.get(leaves) != Some(&reference) {
                            return Err(pdf_error(
                                Some(reference),
                                records[index].range.offset,
                                PdfErrorKind::AmbiguousRepair,
                                "page-tree order differs from explicit page order",
                            ));
                        }
                        leaves += 1;
                    }
                    Some(FragmentKind::Pages {
                        parent: actual,
                        count,
                        kids,
                        has_media_box,
                    }) => {
                        if *actual != parent {
                            return Err(malformed(
                                Some(reference),
                                records[index].range.offset,
                                "Pages /Parent link does not match the page tree",
                            ));
                        }
                        if *count == 0 || kids.is_empty() {
                            return Err(malformed(
                                Some(reference),
                                records[index].range.offset,
                                "Pages node is empty",
                            ));
                        }
                        if u64::from(*count) > u64::from(limits.max_pages) {
                            return Err(pdf_limit(
                                Some(reference),
                                records[index].range.offset,
                                "page-tree count",
                                u64::from(limits.max_pages),
                                u64::from(*count),
                            ));
                        }
                        let pending = stack
                            .len()
                            .checked_add(1)
                            .and_then(|length| length.checked_add(kids.len()))
                            .ok_or(Error::InvalidInput {
                                reason: "PDF page-tree traversal length overflows address space",
                            })?;
                        if pending > stack_capacity {
                            return Err(pdf_error(
                                Some(reference),
                                records[index].range.offset,
                                PdfErrorKind::AmbiguousRepair,
                                "page tree has more child links than indexed objects",
                            ));
                        }
                        stack.push(WalkStep::Exit {
                            reference,
                            leaves_before: leaves,
                            declared_count: *count,
                        });
                        for child in kids.iter().rev() {
                            stack.push(WalkStep::Enter {
                                reference: *child,
                                parent: Some(reference),
                                inherited_media_box: inherited_media_box || *has_media_box,
                            });
                        }
                    }
                    _ => {
                        return Err(malformed(
                            Some(reference),
                            records[index].range.offset,
                            "page-tree child has neither Page nor Pages type",
                        ));
                    }
                }
            }
            WalkStep::Exit {
                reference,
                leaves_before,
                declared_count,
            } => {
                if leaves - leaves_before != declared_count as usize {
                    return Err(malformed(
                        Some(reference),
                        0,
                        "Pages /Count differs from descendant page count",
                    ));
                }
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
