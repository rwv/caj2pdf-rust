// SPDX-License-Identifier: MIT

//! Bounded reconstruction of indexed PDF object fragments.
//!
//! A format handler supplies the exact source span of every indirect object
//! and the intended page order. This module never infers page order from
//! object numbers or searches binary stream payloads for PDF delimiters.

use super::input::{FragmentKind, inspect_fragment_object, inspect_fragment_scalar};
use super::writer::MAX_PDF_OBJECTS;
use super::{MAX_CLASSIC_PDF_BYTES, PdfRange, PdfRef};
use crate::{
    Cancellation, ConversionReport, Error, Limits, PdfErrorKind, RangedSource, Result,
    SequentialSink, read_exact_at, write_all,
};
use std::mem::size_of;

const HEADER: &[u8] = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n";

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
        return Err(pdf_error(
            Some(reference),
            offset,
            PdfErrorKind::Malformed,
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
    let mut counted = CountingSource {
        inner: source,
        bytes_read: 0,
    };
    let source = &mut counted;
    limits.validate()?;
    let page_count = u32::try_from(plan.pages.len()).map_err(|_| {
        pdf_limit(
            plan.pages.first().copied(),
            plan.objects.first().map_or(0, |object| object.range.offset),
            "pages",
            u64::from(limits.max_pages),
            u64::try_from(plan.pages.len()).unwrap_or(u64::MAX),
        )
    })?;
    if page_count > limits.max_pages {
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
            u64::from(page_count),
        ));
    }
    if page_count == 0 {
        return Err(pdf_error(
            None,
            0,
            PdfErrorKind::Malformed,
            "fragment has no pages",
        ));
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
        .ok_or(Error::InvalidInput {
            reason: "PDF object count overflows address space",
        })?;
    if requested > MAX_PDF_OBJECTS as usize {
        return Err(pdf_limit(
            plan.objects.first().map(|object| object.reference),
            plan.objects.first().map_or(0, |object| object.range.offset),
            "PDF object count",
            u64::from(MAX_PDF_OBJECTS),
            requested as u64,
        ));
    }
    let record_bytes = requested
        .checked_mul(size_of::<Record>())
        .ok_or(Error::InvalidInput {
            reason: "PDF object index allocation overflows address space",
        })?;
    let record_bytes = u64::try_from(record_bytes).map_err(|_| Error::InvalidInput {
        reason: "PDF object index allocation exceeds 64 bits",
    })?;
    check_pdf_allocation(
        limits,
        record_bytes,
        plan.objects.first().map(|object| object.reference),
        plan.objects.first().map_or(0, |object| object.range.offset),
    )?;
    let mut records = Vec::new();
    records
        .try_reserve_exact(requested)
        .map_err(|_| Error::LimitExceeded {
            resource: "PDF object index allocation",
            limit: limits.max_allocation_bytes,
            attempted: record_bytes,
        })?;
    let mut fragment_bytes = 0_u64;
    for fragment in plan.objects {
        checked_reference(fragment.reference, fragment.range.offset)?;
        if fragment.range.length == 0 {
            return Err(pdf_error(
                Some(fragment.reference),
                fragment.range.offset,
                PdfErrorKind::Malformed,
                "indirect object span is empty",
            ));
        }
        let end = fragment.range.end().ok_or_else(|| {
            pdf_error(
                Some(fragment.reference),
                fragment.range.offset,
                PdfErrorKind::Malformed,
                "indirect object span overflows 64-bit offset",
            )
        })?;
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
        let previous_end = pair[0].range.end().ok_or_else(|| {
            pdf_error(
                Some(pair[0].reference),
                pair[0].range.offset,
                PdfErrorKind::Malformed,
                "indirect object span overflows 64-bit offset",
            )
        })?;
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
            return Err(pdf_error(
                Some(*page),
                0,
                PdfErrorKind::Malformed,
                "ordered page object is missing",
            ));
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
    let page_index_bytes = u64::try_from(page_index_bytes).map_err(|_| Error::InvalidInput {
        reason: "PDF page index allocation exceeds 64 bits",
    })?;
    check_pdf_allocation(
        limits,
        checked_add(record_bytes, page_index_bytes)?,
        plan.pages.first().copied(),
        plan.objects.first().map_or(0, |object| object.range.offset),
    )?;
    let mut sorted_pages = Vec::new();
    sorted_pages
        .try_reserve_exact(plan.pages.len())
        .map_err(|_| Error::LimitExceeded {
            resource: "PDF page index allocation",
            limit: limits.max_allocation_bytes,
            attempted: page_index_bytes,
        })?;
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
            return Err(pdf_error(
                Some(reference),
                0,
                PdfErrorKind::Malformed,
                "catalog object is missing",
            ));
        }
        reference
    } else {
        let largest = records.last().map_or(plan.pages_root.number, |record| {
            record.reference.number.max(plan.pages_root.number)
        });
        let number = largest.checked_add(1).ok_or_else(|| {
            pdf_limit(
                records.last().map(|record| record.reference),
                records.last().map_or(0, |record| record.range.offset),
                "PDF object number",
                u64::from(MAX_PDF_OBJECTS),
                u64::from(largest) + 1,
            )
        })?;
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

    let pages_prefix = synthetic_pages.then(|| {
        format!(
            "{} 0 obj\n<< /Type /Pages /Count {} /Kids [",
            plan.pages_root.number, page_count
        )
    });
    let pages_suffix = b"] >>\nendobj\n";
    let catalog_text = plan.catalog.is_none().then(|| {
        format!(
            "{} 0 obj\n<< /Type /Catalog /Pages {} 0 R >>\nendobj\n",
            catalog.number, plan.pages_root.number
        )
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
    let xref_bytes = xref_size.checked_mul(20).ok_or(Error::InvalidInput {
        reason: "PDF xref byte count overflows 64 bits",
    })?;
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
    buffer
        .try_reserve_exact(limits.io_chunk_bytes)
        .map_err(|_| Error::LimitExceeded {
            resource: "PDF I/O buffer allocation",
            limit: limits.max_allocation_bytes,
            attempted: output_working_bytes,
        })?;
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
            xref_entry(
                u64::from(next_free),
                if number == 0 { 65_535 } else { 0 },
                b'f',
            )?
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
    if attempted > MAX_CLASSIC_PDF_BYTES {
        return Err(Error::LimitExceeded {
            resource: "classic PDF file bytes",
            limit: MAX_CLASSIC_PDF_BYTES,
            attempted,
        });
    }
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
    Ok(entry)
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
    let scalar_bytes = u64::try_from(scalar_bytes).map_err(|_| Error::InvalidInput {
        reason: "PDF scalar index allocation exceeds 64 bits",
    })?;
    check_pdf_allocation(
        limits,
        checked_add(index_bytes, scalar_bytes)?,
        plan.objects.first().map(|object| object.reference),
        plan.objects.first().map_or(0, |object| object.range.offset),
    )?;
    let mut scalars = Vec::new();
    scalars
        .try_reserve_exact(records.len())
        .map_err(|_| Error::LimitExceeded {
            resource: "PDF scalar index allocation",
            limit: limits.max_allocation_bytes,
            attempted: scalar_bytes,
        })?;
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
    let kind_bytes = u64::try_from(kind_bytes).map_err(|_| Error::InvalidInput {
        reason: "PDF structure index allocation exceeds 64 bits",
    })?;
    let stream_bytes = u64::try_from(records.len()).map_err(|_| Error::InvalidInput {
        reason: "PDF stream index exceeds 64 bits",
    })?;
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
    kinds
        .try_reserve_exact(records.len())
        .map_err(|_| Error::LimitExceeded {
            resource: "PDF structure index allocation",
            limit: limits.max_allocation_bytes,
            attempted: kind_bytes,
        })?;
    let mut retained_structure_bytes = retained_base;
    let mut stream_flags = Vec::new();
    stream_flags
        .try_reserve_exact(records.len())
        .map_err(|_| Error::LimitExceeded {
            resource: "PDF stream index allocation",
            limit: limits.max_allocation_bytes,
            attempted: stream_bytes,
        })?;
    let mut content_evidence = Vec::new();
    let mut destination_count = 0_u32;
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
            if inspection.reference != record.reference {
                return Err(pdf_error(
                    Some(record.reference),
                    record.range.offset,
                    PdfErrorKind::Malformed,
                    "inspected object identity differs from the supplied span",
                ));
            }
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
                    return Err(pdf_error(
                        Some(record.reference),
                        record.range.offset,
                        PdfErrorKind::Malformed,
                        "indirect reference targets a missing object",
                    ));
                }
            }
            if let Some(destination) = inspection.destination {
                checked_reference(destination, record.range.offset)?;
                if sorted_pages.binary_search(&destination).is_err() {
                    return Err(pdf_error(
                        Some(record.reference),
                        record.range.offset,
                        PdfErrorKind::Malformed,
                        "outline destination does not target an ordered Page object",
                    ));
                }
                destination_count =
                    destination_count
                        .checked_add(1)
                        .ok_or(Error::InvalidInput {
                            reason: "PDF outline destination count overflows 32 bits",
                        })?;
                if destination_count > limits.max_bookmarks {
                    return Err(pdf_limit(
                        Some(record.reference),
                        record.range.offset,
                        "bookmarks",
                        u64::from(limits.max_bookmarks),
                        u64::from(destination_count),
                    ));
                }
            }
            if let FragmentKind::Pages { kids, .. } = &inspection.kind {
                let kids_bytes = kids
                    .len()
                    .checked_mul(size_of::<PdfRef>())
                    .and_then(|bytes| u64::try_from(bytes).ok())
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
        return Err(pdf_error(
            plan.catalog,
            0,
            PdfErrorKind::Malformed,
            "catalog object has the wrong type",
        ));
    }
    if let Some(catalog) = plan.catalog {
        let index = object_index(records, catalog).ok_or_else(|| {
            pdf_error(
                Some(catalog),
                0,
                PdfErrorKind::Malformed,
                "catalog object is missing",
            )
        })?;
        match &kinds[index] {
            Some(FragmentKind::Catalog { pages }) if *pages == plan.pages_root => {}
            _ => {
                return Err(pdf_error(
                    Some(catalog),
                    records[index].range.offset,
                    PdfErrorKind::Malformed,
                    "catalog does not reference the selected page tree",
                ));
            }
        }
    }
    let root_index = object_index(records, plan.pages_root).ok_or_else(|| {
        pdf_error(
            Some(plan.pages_root),
            0,
            PdfErrorKind::Malformed,
            "page tree root is missing from the repair index",
        )
    })?;
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
                        return Err(pdf_error(
                            Some(records[index].reference),
                            records[index].range.offset,
                            PdfErrorKind::Malformed,
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
            let index = object_index(records, *page).ok_or_else(|| {
                pdf_error(
                    Some(*page),
                    0,
                    PdfErrorKind::Malformed,
                    "ordered page object is missing",
                )
            })?;
            if !matches!(kinds[index], Some(FragmentKind::Page { .. })) {
                return Err(pdf_error(
                    Some(*page),
                    records[index].range.offset,
                    PdfErrorKind::Malformed,
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
        .and_then(|bytes| u64::try_from(bytes).ok())
        .ok_or(Error::InvalidInput {
            reason: "PDF content reference allocation overflows 64 bits",
        })?;
    let item_bytes = checked_add(size_of::<ContentEvidence>() as u64, reference_bytes)?;
    *retained_bytes = checked_add(*retained_bytes, item_bytes)?;
    check_pdf_allocation(limits, *retained_bytes, Some(item.reference), offset)?;
    evidence
        .try_reserve_exact(1)
        .map_err(|_| Error::LimitExceeded {
            resource: "PDF content evidence allocation",
            limit: limits.max_allocation_bytes,
            attempted: *retained_bytes,
        })?;
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
    let cache_bytes = u64::try_from(evidence.len()).map_err(|_| Error::InvalidInput {
        reason: "PDF content-array validation index exceeds 64 bits",
    })?;
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
    validated_arrays
        .try_reserve_exact(evidence.len())
        .map_err(|_| Error::LimitExceeded {
            resource: "PDF content-array validation index allocation",
            limit: limits.max_allocation_bytes,
            attempted: cache_bytes,
        })?;
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
        let target = item.references.first().copied().ok_or_else(|| {
            pdf_error(
                Some(item.reference),
                page_offset,
                PdfErrorKind::Malformed,
                "Page Contents lacks an indirect target",
            )
        })?;
        let target_index = object_index(records, target).ok_or_else(|| {
            pdf_error(
                Some(item.reference),
                page_offset,
                PdfErrorKind::Malformed,
                "Page Contents targets a missing object",
            )
        })?;
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
            return Err(pdf_error(
                Some(item.reference),
                page_offset,
                PdfErrorKind::Malformed,
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
    let index = object_index(records, target).ok_or_else(|| {
        pdf_error(
            Some(page),
            page_offset,
            PdfErrorKind::Malformed,
            "Page Contents targets a missing object",
        )
    })?;
    if !stream_flags[index] {
        return Err(pdf_error(
            Some(page),
            page_offset,
            PdfErrorKind::Malformed,
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
    let visit_bytes = u64::try_from(records.len()).map_err(|_| Error::InvalidInput {
        reason: "PDF page-tree visited index exceeds 64 bits",
    })?;
    let with_visited = checked_add(retained_bytes, visit_bytes)?;
    check_pdf_allocation(limits, with_visited, Some(plan.pages_root), root_offset)?;
    let mut visited = Vec::new();
    visited
        .try_reserve_exact(records.len())
        .map_err(|_| Error::LimitExceeded {
            resource: "PDF page-tree visited index allocation",
            limit: limits.max_allocation_bytes,
            attempted: visit_bytes,
        })?;
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
    let stack_bytes = u64::try_from(stack_bytes).map_err(|_| Error::InvalidInput {
        reason: "PDF page-tree traversal allocation exceeds 64 bits",
    })?;
    check_pdf_allocation(
        limits,
        checked_add(with_visited, stack_bytes)?,
        Some(plan.pages_root),
        root_offset,
    )?;
    let mut stack = Vec::new();
    stack
        .try_reserve_exact(stack_capacity)
        .map_err(|_| Error::LimitExceeded {
            resource: "PDF page-tree traversal allocation",
            limit: limits.max_allocation_bytes,
            attempted: stack_bytes,
        })?;
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
                let index = object_index(records, reference).ok_or_else(|| {
                    pdf_error(
                        Some(reference),
                        0,
                        PdfErrorKind::Malformed,
                        "page-tree child object is missing",
                    )
                })?;
                if visited[index] {
                    return Err(pdf_error(
                        Some(reference),
                        records[index].range.offset,
                        PdfErrorKind::Malformed,
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
                            return Err(pdf_error(
                                Some(reference),
                                records[index].range.offset,
                                PdfErrorKind::Malformed,
                                "Page /Parent link does not match the page tree",
                            ));
                        }
                        if !has_media_box && !inherited_media_box {
                            return Err(pdf_error(
                                Some(reference),
                                records[index].range.offset,
                                PdfErrorKind::Malformed,
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
                            return Err(pdf_error(
                                Some(reference),
                                records[index].range.offset,
                                PdfErrorKind::Malformed,
                                "Pages /Parent link does not match the page tree",
                            ));
                        }
                        if *count == 0 || kids.is_empty() {
                            return Err(pdf_error(
                                Some(reference),
                                records[index].range.offset,
                                PdfErrorKind::Malformed,
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
                        return Err(pdf_error(
                            Some(reference),
                            records[index].range.offset,
                            PdfErrorKind::Malformed,
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
                    return Err(pdf_error(
                        Some(reference),
                        0,
                        PdfErrorKind::Malformed,
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
mod tests {
    use super::*;
    use crate::NeverCancel;
    use std::{
        cell::Cell,
        future::Future,
        io,
        pin::pin,
        task::{Context, Poll, Waker},
    };

    fn run<F: Future>(future: F) -> F::Output {
        let mut context = Context::from_waker(Waker::noop());
        let mut future = pin!(future);
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("test I/O unexpectedly yielded"),
        }
    }

    struct BytesSource(Vec<u8>);

    impl RangedSource for BytesSource {
        fn size(&self) -> u64 {
            self.0.len() as u64
        }

        async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
            let start = usize::try_from(offset).map_err(|_| Error::InvalidInput {
                reason: "test read offset overflows usize",
            })?;
            let available = self.0.get(start..).unwrap_or_default();
            let copied = available.len().min(destination.len()).min(7);
            destination[..copied].copy_from_slice(&available[..copied]);
            Ok(copied)
        }
    }

    struct MeasuredSource {
        inner: BytesSource,
        bytes_read: u64,
    }

    impl RangedSource for MeasuredSource {
        fn size(&self) -> u64 {
            self.inner.size()
        }

        async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
            let read = self.inner.read_at(offset, destination).await?;
            self.bytes_read += read as u64;
            Ok(read)
        }
    }

    #[derive(Default)]
    struct BytesSink {
        bytes: Vec<u8>,
        fail_after: Option<usize>,
    }

    impl SequentialSink for BytesSink {
        async fn write(&mut self, bytes: &[u8]) -> Result<usize> {
            if self
                .fail_after
                .is_some_and(|threshold| self.bytes.len() >= threshold)
            {
                return Err(Error::Io(io::Error::other("injected sink failure")));
            }
            let copied = bytes.len().min(11);
            self.bytes.extend_from_slice(&bytes[..copied]);
            Ok(copied)
        }

        async fn flush(&mut self) -> Result<()> {
            Ok(())
        }
    }

    fn reference(number: u32) -> PdfRef {
        PdfRef {
            number,
            generation: 0,
        }
    }

    fn add_object(source: &mut Vec<u8>, number: u32, body: &[u8]) -> FragmentObject {
        let offset = source.len() as u64;
        source.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
        source.extend_from_slice(body);
        source.extend_from_slice(b"\nendobj\n");
        FragmentObject {
            reference: reference(number),
            range: PdfRange {
                offset,
                length: source.len() as u64 - offset,
            },
        }
    }

    fn two_page_fragment() -> (BytesSource, Vec<FragmentObject>, Vec<PdfRef>) {
        let mut bytes = b"CAJ\0unrelated metadata\n".to_vec();
        let second = add_object(
            &mut bytes,
            9,
            b"<< /Type /Page /Parent 5 0 R /MediaBox [0 0 400 250] /Resources << >> >>",
        );
        let first = add_object(
            &mut bytes,
            3,
            b"<< /Type /Page /Parent 5 0 R /MediaBox [0 0 200 300] /Resources << >> >>",
        );
        let fake_markers = b"endobj\nxref\nstartxref\n%%EOF\n";
        let scalar = add_object(&mut bytes, 4, fake_markers.len().to_string().as_bytes());
        let mut stream_body = b"<< /Length 4 0 R >>\nstream\n".to_vec();
        stream_body.extend_from_slice(fake_markers);
        stream_body.extend_from_slice(b"\nendstream");
        let stream = add_object(&mut bytes, 6, &stream_body);
        (
            BytesSource(bytes),
            vec![second, first, scalar, stream],
            vec![reference(9), reference(3)],
        )
    }

    fn existing_tree_fragment() -> (BytesSource, Vec<FragmentObject>, Vec<PdfRef>) {
        let mut bytes = b"CAJ\0object fragments\n".to_vec();
        let first = add_object(
            &mut bytes,
            9,
            b"<< /Type /Page /Parent 5 0 R /MediaBox [0 0 200 300] /Resources << >> >>",
        );
        let catalog = add_object(&mut bytes, 1, b"<< /Type /Catalog /Pages 5 0 R >>");
        let second = add_object(
            &mut bytes,
            3,
            b"<< /Type /Page /Parent 7 0 R /MediaBox [0 0 400 250] /Resources << >> >>",
        );
        let branch = add_object(
            &mut bytes,
            7,
            b"<< /Type /Pages /Parent 5 0 R /Count 1 /Kids [3 0 R] >>",
        );
        let root = add_object(
            &mut bytes,
            5,
            b"<< /Type /Pages /Count 2 /Kids [9 0 R 7 0 R] >>",
        );
        (
            BytesSource(bytes),
            vec![first, catalog, second, branch, root],
            vec![reference(9), reference(3)],
        )
    }

    fn replace_in_object(source: &mut BytesSource, object: FragmentObject, from: &[u8], to: &[u8]) {
        assert_eq!(from.len(), to.len());
        let start = object.range.offset as usize;
        let end = start + object.range.length as usize;
        let relative = source.0[start..end]
            .windows(from.len())
            .position(|window| window == from)
            .expect("test token is present");
        source.0[start + relative..start + relative + to.len()].copy_from_slice(to);
    }

    #[test]
    fn sparse_out_of_order_objects_use_explicit_page_order() {
        run(async {
            let (mut source, objects, pages) = two_page_fragment();
            let mut sink = BytesSink::default();
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: None,
            };
            let report = reconstruct_fragment(
                &mut source,
                &mut sink,
                &plan,
                &Limits::default(),
                &NeverCancel,
            )
            .await?;
            assert_eq!(report.pages_converted, 2);
            assert_eq!(report.output_bytes_written, sink.bytes.len() as u64);
            let pdf = String::from_utf8_lossy(&sink.bytes);
            assert!(pdf.starts_with("%PDF-1.7"));
            assert!(pdf.contains("/Kids [9 0 R 3 0 R ]"));
            assert!(pdf.contains("/Root 10 0 R"));
            assert!(pdf.contains("xref\n0 11\n"));
            Ok::<(), Error>(())
        })
        .unwrap();
    }

    #[test]
    fn input_report_counts_validation_and_copy_reads() {
        run(async {
            let (inner, objects, pages) = two_page_fragment();
            let mut source = MeasuredSource {
                inner,
                bytes_read: 0,
            };
            let mut sink = BytesSink::default();
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: None,
            };
            let report = reconstruct_fragment(
                &mut source,
                &mut sink,
                &plan,
                &Limits::default(),
                &NeverCancel,
            )
            .await?;
            assert_eq!(report.input_bytes_read, source.bytes_read);
            assert!(
                report.input_bytes_read
                    > objects
                        .iter()
                        .map(|object| object.range.length)
                        .sum::<u64>()
            );
            Ok::<(), Error>(())
        })
        .unwrap();
    }

    #[test]
    fn existing_nested_pages_and_catalog_are_preserved() {
        run(async {
            let (mut source, objects, pages) = existing_tree_fragment();
            let mut sink = BytesSink::default();
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: Some(reference(1)),
            };
            let report = reconstruct_fragment(
                &mut source,
                &mut sink,
                &plan,
                &Limits::default(),
                &NeverCancel,
            )
            .await?;
            assert_eq!(report.pages_converted, 2);
            let pdf = String::from_utf8_lossy(&sink.bytes);
            assert!(pdf.contains("/Root 1 0 R"));
            assert_eq!(pdf.matches("/Type /Catalog").count(), 1);
            assert_eq!(pdf.matches("/Type /Pages").count(), 2);
            Ok::<(), Error>(())
        })
        .unwrap();
    }

    #[test]
    fn existing_pages_can_gain_catalog_and_missing_pages_can_keep_catalog() {
        run(async {
            let (mut source, objects, pages) = existing_tree_fragment();
            let without_catalog: Vec<_> = objects
                .iter()
                .copied()
                .filter(|object| object.reference != reference(1))
                .collect();
            let mut sink = BytesSink::default();
            let plan = FragmentPlan {
                objects: &without_catalog,
                pages: &pages,
                pages_root: reference(5),
                catalog: None,
            };
            reconstruct_fragment(
                &mut source,
                &mut sink,
                &plan,
                &Limits::default(),
                &NeverCancel,
            )
            .await?;
            assert!(String::from_utf8_lossy(&sink.bytes).contains("/Root 10 0 R"));

            let mut bytes = b"CAJ\0".to_vec();
            let page = add_object(
                &mut bytes,
                9,
                b"<< /Type /Page /Parent 5 0 R /MediaBox [0 0 200 300] /Resources << >> >>",
            );
            let catalog = add_object(&mut bytes, 1, b"<< /Type /Catalog /Pages 5 0 R >>");
            let mut source = BytesSource(bytes);
            let objects = [page, catalog];
            let pages = [reference(9)];
            let mut sink = BytesSink::default();
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: Some(reference(1)),
            };
            reconstruct_fragment(
                &mut source,
                &mut sink,
                &plan,
                &Limits::default(),
                &NeverCancel,
            )
            .await?;
            let pdf = String::from_utf8_lossy(&sink.bytes);
            assert!(pdf.contains("/Root 1 0 R"));
            assert!(pdf.contains("/Kids [9 0 R ]"));
            Ok::<(), Error>(())
        })
        .unwrap();
    }

    #[test]
    fn malformed_page_tree_links_counts_cycles_and_catalog_fail_before_output() {
        run(async {
            for case in 0..6 {
                let (mut source, objects, mut pages) = existing_tree_fragment();
                let mut catalog = Some(reference(1));
                let expected = match case {
                    0 => {
                        replace_in_object(&mut source, objects[4], b"/Count 2", b"/Count 3");
                        PdfErrorKind::Malformed
                    }
                    1 => {
                        replace_in_object(&mut source, objects[2], b"/Parent 7", b"/Parent 5");
                        PdfErrorKind::Malformed
                    }
                    2 => {
                        replace_in_object(
                            &mut source,
                            objects[4],
                            b"/Kids [9 0 R 7 0 R]",
                            b"/Kids [9 0 R 5 0 R]",
                        );
                        PdfErrorKind::Malformed
                    }
                    3 => {
                        pages.reverse();
                        PdfErrorKind::AmbiguousRepair
                    }
                    4 => {
                        replace_in_object(&mut source, objects[1], b"/Pages 5", b"/Pages 7");
                        PdfErrorKind::Malformed
                    }
                    _ => {
                        catalog = None;
                        PdfErrorKind::AmbiguousRepair
                    }
                };
                let mut sink = BytesSink::default();
                let plan = FragmentPlan {
                    objects: &objects,
                    pages: &pages,
                    pages_root: reference(5),
                    catalog,
                };
                let error = reconstruct_fragment(
                    &mut source,
                    &mut sink,
                    &plan,
                    &Limits::default(),
                    &NeverCancel,
                )
                .await
                .unwrap_err();
                assert!(
                    matches!(error, Error::Pdf { kind, .. } if kind == expected),
                    "case {case}: {error}"
                );
                assert!(sink.bytes.is_empty());
            }
        });
    }

    #[test]
    fn ambiguous_parent_and_repeated_pages_fail_before_writing() {
        run(async {
            let (mut source, objects, pages) = two_page_fragment();
            let mut sink = BytesSink::default();
            let wrong_root = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(7),
                catalog: None,
            };
            assert!(matches!(
                reconstruct_fragment(
                    &mut source,
                    &mut sink,
                    &wrong_root,
                    &Limits::default(),
                    &NeverCancel,
                )
                .await,
                Err(Error::Pdf { .. })
            ));
            assert!(sink.bytes.is_empty());

            let repeated = [pages[0], pages[0]];
            let plan = FragmentPlan {
                objects: &objects,
                pages: &repeated,
                pages_root: reference(5),
                catalog: None,
            };
            assert!(matches!(
                reconstruct_fragment(
                    &mut source,
                    &mut sink,
                    &plan,
                    &Limits::default(),
                    &NeverCancel,
                )
                .await,
                Err(Error::Pdf {
                    kind: PdfErrorKind::AmbiguousRepair,
                    ..
                })
            ));
            assert!(sink.bytes.is_empty());
        });
    }

    #[test]
    fn a_sink_failure_never_returns_a_success_report() {
        run(async {
            let (mut source, objects, pages) = two_page_fragment();
            let mut sink = BytesSink {
                fail_after: Some(60),
                ..BytesSink::default()
            };
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: None,
            };
            assert!(matches!(
                reconstruct_fragment(
                    &mut source,
                    &mut sink,
                    &plan,
                    &Limits::default(),
                    &NeverCancel,
                )
                .await,
                Err(Error::Io(_))
            ));
            assert!(!sink.bytes.is_empty());
        });
    }

    #[test]
    fn malformed_indirect_stream_length_fails_before_output() {
        run(async {
            let (mut source, objects, pages) = two_page_fragment();
            let scalar_offset = objects[2].range.offset as usize + b"4 0 obj\n".len();
            source.0[scalar_offset] = b'1';
            let mut sink = BytesSink::default();
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: None,
            };
            assert!(matches!(
                reconstruct_fragment(
                    &mut source,
                    &mut sink,
                    &plan,
                    &Limits::default(),
                    &NeverCancel,
                )
                .await,
                Err(Error::Pdf {
                    kind: PdfErrorKind::Malformed,
                    ..
                })
            ));
            assert!(sink.bytes.is_empty());
        });
    }

    #[test]
    fn output_limit_is_preflighted_before_sink_write() {
        run(async {
            let (mut source, objects, pages) = two_page_fragment();
            let mut sink = BytesSink::default();
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: None,
            };
            let limits = Limits {
                max_output_bytes: 100,
                ..Limits::default()
            };
            assert!(matches!(
                reconstruct_fragment(&mut source, &mut sink, &plan, &limits, &NeverCancel).await,
                Err(Error::PdfLimitExceeded {
                    resource: "output bytes",
                    ..
                })
            ));
            assert!(sink.bytes.is_empty());
        });
    }

    #[test]
    fn outline_destinations_must_target_ordered_pages() {
        run(async {
            let (mut source, mut objects, pages) = two_page_fragment();
            let wrong_destination = add_object(
                &mut source.0,
                12,
                b"<< /Title (Wrong) /Dest [4 0 R /Fit] >>",
            );
            objects.push(wrong_destination);
            let mut sink = BytesSink::default();
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: None,
            };
            assert!(matches!(
                reconstruct_fragment(
                    &mut source,
                    &mut sink,
                    &plan,
                    &Limits::default(),
                    &NeverCancel,
                )
                .await,
                Err(Error::Pdf {
                    kind: PdfErrorKind::Malformed,
                    reason: "outline destination does not target an ordered Page object",
                    ..
                })
            ));
            assert!(sink.bytes.is_empty());

            let (mut source, mut objects, pages) = two_page_fragment();
            objects.push(add_object(
                &mut source.0,
                12,
                b"<< /Title (Named) /Dest (named-destination) >>",
            ));
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: None,
            };
            assert!(matches!(
                reconstruct_fragment(
                    &mut source,
                    &mut sink,
                    &plan,
                    &Limits::default(),
                    &NeverCancel,
                )
                .await,
                Err(Error::Pdf {
                    kind: PdfErrorKind::UnsupportedFeature,
                    ..
                })
            ));
            assert!(sink.bytes.is_empty());
        });
    }

    #[test]
    fn outline_count_limit_is_checked_before_writing() {
        run(async {
            let (mut source, mut objects, pages) = two_page_fragment();
            let outline = add_object(
                &mut source.0,
                12,
                b"<< /Title (Valid) /Dest [9 0 R /Fit] >>",
            );
            objects.push(outline);
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: None,
            };
            let mut sink = BytesSink::default();
            let limits = Limits {
                max_bookmarks: 0,
                ..Limits::default()
            };
            assert!(matches!(
                reconstruct_fragment(&mut source, &mut sink, &plan, &limits, &NeverCancel).await,
                Err(Error::PdfLimitExceeded {
                    resource: "bookmarks",
                    offset,
                    object: Some((12, 0)),
                    ..
                }) if offset == outline.range.offset
            ));
            assert!(sink.bytes.is_empty());
        });
    }

    #[test]
    fn one_byte_io_chunks_still_finish_with_bounded_output() {
        run(async {
            let (mut source, objects, pages) = two_page_fragment();
            let mut sink = BytesSink::default();
            let limits = Limits {
                io_chunk_bytes: 1,
                ..Limits::default()
            };
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: None,
            };
            let report =
                reconstruct_fragment(&mut source, &mut sink, &plan, &limits, &NeverCancel).await?;
            assert_eq!(report.pages_converted, 2);
            assert_eq!(report.output_bytes_written, sink.bytes.len() as u64);
            Ok::<(), Error>(())
        })
        .unwrap();
    }

    #[test]
    fn duplicate_overlapping_and_truncated_spans_fail_before_output() {
        run(async {
            for case in 0..3 {
                let (mut source, mut objects, pages) = two_page_fragment();
                match case {
                    0 => objects[1].reference = objects[0].reference,
                    1 => objects[1].range.offset = objects[0].range.offset + 1,
                    _ => objects[1].range.length = source.size(),
                }
                let mut sink = BytesSink::default();
                let plan = FragmentPlan {
                    objects: &objects,
                    pages: &pages,
                    pages_root: reference(5),
                    catalog: None,
                };
                let error = reconstruct_fragment(
                    &mut source,
                    &mut sink,
                    &plan,
                    &Limits::default(),
                    &NeverCancel,
                )
                .await
                .unwrap_err();
                assert!(
                    matches!(error, Error::Pdf { .. } | Error::TruncatedInput { .. }),
                    "case {case}: {error}"
                );
                assert!(sink.bytes.is_empty());
            }
        });
    }

    #[test]
    fn missing_reference_generation_and_memory_limit_are_typed() {
        run(async {
            let (mut source, mut objects, pages) = two_page_fragment();
            objects.push(add_object(&mut source.0, 12, b"<< /Contents 77 0 R >>"));
            let mut sink = BytesSink::default();
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: None,
            };
            assert!(matches!(
                reconstruct_fragment(
                    &mut source,
                    &mut sink,
                    &plan,
                    &Limits::default(),
                    &NeverCancel,
                )
                .await,
                Err(Error::Pdf {
                    kind: PdfErrorKind::Malformed,
                    reason: "indirect reference targets a missing object",
                    ..
                })
            ));
            assert!(sink.bytes.is_empty());

            let (mut source, mut objects, pages) = two_page_fragment();
            objects[0].reference.generation = 1;
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: None,
            };
            assert!(matches!(
                reconstruct_fragment(
                    &mut source,
                    &mut sink,
                    &plan,
                    &Limits::default(),
                    &NeverCancel,
                )
                .await,
                Err(Error::Pdf {
                    kind: PdfErrorKind::UnsupportedFeature,
                    ..
                })
            ));
            assert!(sink.bytes.is_empty());

            let (mut source, objects, pages) = two_page_fragment();
            let limits = Limits {
                io_chunk_bytes: 16,
                max_allocation_bytes: 64,
                ..Limits::default()
            };
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: None,
            };
            assert!(matches!(
                reconstruct_fragment(&mut source, &mut sink, &plan, &limits, &NeverCancel).await,
                Err(Error::PdfLimitExceeded {
                    resource: "PDF allocation bytes",
                    object: Some((9, 0)),
                    ..
                })
            ));
            assert!(sink.bytes.is_empty());
        });
    }

    #[test]
    fn empty_pages_missing_roots_and_invalid_ids_reject_without_output() {
        run(async {
            for case in 0..5 {
                let (mut source, objects, pages) = two_page_fragment();
                let mut requested = pages.clone();
                let mut root = reference(5);
                let mut catalog = None;
                match case {
                    0 => requested.clear(),
                    1 => requested[1] = reference(77),
                    2 => catalog = Some(reference(11)),
                    3 => root = reference(0),
                    _ => root = reference(MAX_PDF_OBJECTS + 1),
                }
                let plan = FragmentPlan {
                    objects: &objects,
                    pages: &requested,
                    pages_root: root,
                    catalog,
                };
                let mut sink = BytesSink::default();
                assert!(
                    reconstruct_fragment(
                        &mut source,
                        &mut sink,
                        &plan,
                        &Limits::default(),
                        &NeverCancel,
                    )
                    .await
                    .is_err(),
                    "case {case} unexpectedly succeeded"
                );
                assert!(sink.bytes.is_empty());
            }
        });
    }

    #[test]
    fn missing_root_rejects_nested_nodes_and_non_page_kids() {
        run(async {
            let (mut source, mut objects, pages) = two_page_fragment();
            objects.push(add_object(
                &mut source.0,
                7,
                b"<< /Type /Pages /Parent 5 0 R /Count 1 /Kids [9 0 R] >>",
            ));
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: None,
            };
            let mut sink = BytesSink::default();
            assert!(matches!(
                reconstruct_fragment(
                    &mut source,
                    &mut sink,
                    &plan,
                    &Limits::default(),
                    &NeverCancel,
                )
                .await,
                Err(Error::Pdf {
                    kind: PdfErrorKind::AmbiguousRepair,
                    ..
                })
            ));
            assert!(sink.bytes.is_empty());

            let (mut source, objects, mut pages) = two_page_fragment();
            pages[0] = reference(4);
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: None,
            };
            assert!(matches!(
                reconstruct_fragment(
                    &mut source,
                    &mut sink,
                    &plan,
                    &Limits::default(),
                    &NeverCancel,
                )
                .await,
                Err(Error::Pdf {
                    kind: PdfErrorKind::Malformed,
                    ..
                })
            ));
            assert!(sink.bytes.is_empty());
        });
    }

    #[test]
    fn existing_tree_rejects_empty_count_over_limit_and_non_page_child() {
        run(async {
            for case in 0..3 {
                let (mut source, objects, pages) = existing_tree_fragment();
                let mut limits = Limits::default();
                match case {
                    0 => replace_in_object(&mut source, objects[4], b"/Count 2", b"/Count 0"),
                    1 => {
                        replace_in_object(&mut source, objects[4], b"/Count 2", b"/Count 3");
                        limits.max_pages = 2;
                    }
                    _ => replace_in_object(
                        &mut source,
                        objects[4],
                        b"/Kids [9 0 R 7 0 R]",
                        b"/Kids [9 0 R 1 0 R]",
                    ),
                }
                let mut sink = BytesSink::default();
                let plan = FragmentPlan {
                    objects: &objects,
                    pages: &pages,
                    pages_root: reference(5),
                    catalog: Some(reference(1)),
                };
                assert!(
                    reconstruct_fragment(&mut source, &mut sink, &plan, &limits, &NeverCancel)
                        .await
                        .is_err(),
                    "case {case} unexpectedly succeeded"
                );
                assert!(sink.bytes.is_empty());
            }
        });
    }

    #[test]
    fn multi_digit_page_refs_and_small_chunk_flushes_are_supported() {
        run(async {
            let mut bytes = Vec::new();
            let first = add_object(
                &mut bytes,
                12,
                b"<< /Type /Page /Parent 5 0 R /MediaBox [0 0 200 300] /Resources << >> >>",
            );
            let second = add_object(
                &mut bytes,
                3,
                b"<< /Type /Page /Parent 5 0 R /MediaBox [0 0 400 250] /Resources << >> >>",
            );
            let mut source = BytesSource(bytes);
            let objects = [first, second];
            let pages = [reference(12), reference(3)];
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: None,
            };
            let limits = Limits {
                io_chunk_bytes: 20,
                ..Limits::default()
            };
            let mut sink = BytesSink::default();
            let report =
                reconstruct_fragment(&mut source, &mut sink, &plan, &limits, &NeverCancel).await?;
            assert_eq!(report.pages_converted, 2);
            assert!(String::from_utf8_lossy(&sink.bytes).contains("/Kids [12 0 R 3 0 R ]"));
            Ok::<(), Error>(())
        })
        .unwrap();
    }

    #[test]
    fn xref_rows_are_fixed_width_and_link_sparse_free_slots() {
        assert_eq!(xref_entry(17, 0, b'n').unwrap(), *b"0000000017 00000 n \n");
        assert_eq!(
            xref_entry(9, 65_535, b'f').unwrap(),
            *b"0000000009 65535 f \n"
        );
        let records = [
            Record::synthetic(reference(2)),
            Record::synthetic(reference(5)),
        ];
        assert_eq!(next_free_number(0, 5, &records, 0), 1);
        assert_eq!(next_free_number(1, 5, &records, 0), 3);
        assert_eq!(next_free_number(4, 5, &records, 1), 0);
    }

    #[test]
    fn repeated_page_tree_links_cannot_expand_the_walk_stack() {
        let root = reference(5);
        let leaf = reference(9);
        let records = [Record::synthetic(root), Record::synthetic(leaf)];
        let kinds = [
            Some(FragmentKind::Pages {
                parent: None,
                count: 1,
                kids: vec![leaf; 10],
                has_media_box: false,
            }),
            Some(FragmentKind::Page {
                parent: root,
                has_media_box: true,
            }),
        ];
        let pages = [leaf];
        let plan = FragmentPlan {
            objects: &[],
            pages: &pages,
            pages_root: root,
            catalog: None,
        };
        assert!(matches!(
            validate_existing_page_tree(&plan, &records, &kinds, 0, &Limits::default()),
            Err(Error::Pdf {
                kind: PdfErrorKind::AmbiguousRepair,
                ..
            })
        ));
    }

    #[test]
    fn page_inventory_catalog_role_and_branch_parent_must_agree() {
        run(async {
            for case in 0..3 {
                let (mut source, mut objects, pages) = existing_tree_fragment();
                let expected = match case {
                    0 => {
                        objects.push(add_object(
                            &mut source.0,
                            11,
                            b"<< /Type /Page /Parent 5 0 R /MediaBox [0 0 100 100] >>",
                        ));
                        PdfErrorKind::AmbiguousRepair
                    }
                    1 => {
                        replace_in_object(
                            &mut source,
                            objects[1],
                            b"/Type /Catalog",
                            b"/Type /Catolog",
                        );
                        PdfErrorKind::Malformed
                    }
                    _ => {
                        replace_in_object(&mut source, objects[3], b"/Parent 5", b"/Parent 9");
                        PdfErrorKind::Malformed
                    }
                };
                let plan = FragmentPlan {
                    objects: &objects,
                    pages: &pages,
                    pages_root: reference(5),
                    catalog: Some(reference(1)),
                };
                let mut sink = BytesSink::default();
                let error = reconstruct_fragment(
                    &mut source,
                    &mut sink,
                    &plan,
                    &Limits::default(),
                    &NeverCancel,
                )
                .await
                .unwrap_err();
                assert!(
                    matches!(error, Error::Pdf { kind, .. } if kind == expected),
                    "case {case}: {error}"
                );
                assert!(sink.bytes.is_empty());
            }
        });
    }

    #[test]
    fn page_media_box_must_be_direct_or_inherited_from_the_page_tree() {
        run(async {
            let (mut source, objects, pages) = two_page_fragment();
            replace_in_object(&mut source, objects[0], b"/MediaBox", b"/Mediabax");
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: None,
            };
            let mut sink = BytesSink::default();
            assert!(matches!(
                reconstruct_fragment(
                    &mut source,
                    &mut sink,
                    &plan,
                    &Limits::default(),
                    &NeverCancel,
                )
                .await,
                Err(Error::Pdf {
                    kind: PdfErrorKind::Malformed,
                    ..
                })
            ));
            assert!(sink.bytes.is_empty());

            let (mut source, objects, pages) = existing_tree_fragment();
            replace_in_object(&mut source, objects[0], b"/MediaBox", b"/Mediabax");
            replace_in_object(&mut source, objects[2], b"/MediaBox", b"/Mediabax");
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: Some(reference(1)),
            };
            assert!(matches!(
                reconstruct_fragment(
                    &mut source,
                    &mut sink,
                    &plan,
                    &Limits::default(),
                    &NeverCancel,
                )
                .await,
                Err(Error::Pdf {
                    kind: PdfErrorKind::Malformed,
                    ..
                })
            ));
            assert!(sink.bytes.is_empty());

            let mut bytes = Vec::new();
            let page = add_object(&mut bytes, 9, b"<< /Type /Page /Parent 7 0 R >>");
            let branch = add_object(
                &mut bytes,
                7,
                b"<< /Type /Pages /Parent 5 0 R /Count 1 /Kids [9 0 R] /MediaBox [0 0 200 300] >>",
            );
            let root = add_object(&mut bytes, 5, b"<< /Type /Pages /Count 1 /Kids [7 0 R] >>");
            let objects = [page, branch, root];
            let pages = [reference(9)];
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: None,
            };
            let mut source = BytesSource(bytes);
            let report = reconstruct_fragment(
                &mut source,
                &mut sink,
                &plan,
                &Limits::default(),
                &NeverCancel,
            )
            .await?;
            assert_eq!(report.pages_converted, 1);
            assert_eq!(report.output_bytes_written, sink.bytes.len() as u64);
            Ok::<(), Error>(())
        })
        .unwrap();
    }

    #[test]
    fn page_contents_references_only_streams_or_one_indirect_stream_array() {
        run(async {
            for case in 0..4 {
                let mut bytes = Vec::new();
                let contents = match case {
                    0 => b"4 0 R".as_slice(),
                    1 => b"[4 0 R]".as_slice(),
                    _ => b"4 0 R".as_slice(),
                };
                let mut page_body =
                    b"<< /Type /Page /Parent 5 0 R /MediaBox [0 0 200 300] /Contents ".to_vec();
                page_body.extend_from_slice(contents);
                page_body.extend_from_slice(b" >>");
                let page = add_object(&mut bytes, 9, &page_body);
                let target = match case {
                    0 | 1 => add_object(&mut bytes, 4, b"42"),
                    2 => add_object(&mut bytes, 4, b"[6 0 R]"),
                    _ => add_object(&mut bytes, 4, b"[7 0 R]"),
                };
                let stream = add_object(&mut bytes, 6, b"<< /Length 0 >>\nstream\nendstream");
                let scalar = add_object(&mut bytes, 7, b"0");
                let objects = [page, target, stream, scalar];
                let pages = [reference(9)];
                let plan = FragmentPlan {
                    objects: &objects,
                    pages: &pages,
                    pages_root: reference(5),
                    catalog: None,
                };
                let mut source = BytesSource(bytes);
                let mut sink = BytesSink::default();
                let result = reconstruct_fragment(
                    &mut source,
                    &mut sink,
                    &plan,
                    &Limits::default(),
                    &NeverCancel,
                )
                .await;
                if case == 2 {
                    let report = result?;
                    assert_eq!(report.pages_converted, 1);
                    assert_eq!(report.output_bytes_written, sink.bytes.len() as u64);
                } else {
                    assert!(
                        matches!(
                            result,
                            Err(Error::Pdf {
                                kind: PdfErrorKind::Malformed,
                                ..
                            })
                        ),
                        "case {case} was accepted"
                    );
                    assert!(sink.bytes.is_empty());
                }
            }
            Ok::<(), Error>(())
        })
        .unwrap();
    }

    #[test]
    fn many_pages_can_share_one_indirect_contents_array() {
        run(async {
            let mut bytes = Vec::new();
            let mut objects = Vec::new();
            let mut pages = Vec::new();
            for ordinal in 0..64 {
                let number = 10 + ordinal;
                pages.push(reference(number));
                objects.push(add_object(
                    &mut bytes,
                    number,
                    b"<< /Type /Page /Parent 5 0 R /MediaBox [0 0 200 300] /Contents 4 0 R >>",
                ));
            }
            let mut array = b"[".to_vec();
            for ordinal in 0..64 {
                array.extend_from_slice(format!("{} 0 R ", 1000 + ordinal).as_bytes());
            }
            array.extend_from_slice(b"]");
            objects.push(add_object(&mut bytes, 4, &array));
            for ordinal in 0..64 {
                objects.push(add_object(
                    &mut bytes,
                    1000 + ordinal,
                    b"<< /Length 0 >>\nstream\nendstream",
                ));
            }
            let mut source = BytesSource(bytes);
            let mut sink = BytesSink::default();
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: None,
            };
            let report = reconstruct_fragment(
                &mut source,
                &mut sink,
                &plan,
                &Limits::default(),
                &NeverCancel,
            )
            .await?;
            assert_eq!(report.pages_converted, 64);
            assert_eq!(report.output_bytes_written, sink.bytes.len() as u64);
            Ok::<(), Error>(())
        })
        .unwrap();
    }

    #[test]
    fn malformed_spans_and_orphaned_ordered_pages_fail_before_output() {
        run(async {
            for case in 0..2 {
                let (mut source, mut objects, pages) = two_page_fragment();
                if case == 0 {
                    objects[0].range.length = 0;
                } else {
                    objects[0].range.offset = u64::MAX;
                    objects[0].range.length = 2;
                }
                let plan = FragmentPlan {
                    objects: &objects,
                    pages: &pages,
                    pages_root: reference(5),
                    catalog: None,
                };
                let mut sink = BytesSink::default();
                assert!(matches!(
                    reconstruct_fragment(
                        &mut source,
                        &mut sink,
                        &plan,
                        &Limits::default(),
                        &NeverCancel,
                    )
                    .await,
                    Err(Error::Pdf {
                        kind: PdfErrorKind::Malformed,
                        ..
                    })
                ));
                assert!(sink.bytes.is_empty());
            }

            let (mut source, objects, pages) = two_page_fragment();
            replace_in_object(&mut source, objects[0], b"/Parent 5", b"/Parent 6");
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: None,
            };
            let mut sink = BytesSink::default();
            assert!(matches!(
                reconstruct_fragment(
                    &mut source,
                    &mut sink,
                    &plan,
                    &Limits::default(),
                    &NeverCancel,
                )
                .await,
                Err(Error::Pdf {
                    kind: PdfErrorKind::AmbiguousRepair,
                    ..
                })
            ));
            assert!(sink.bytes.is_empty());

            let mut bytes = Vec::new();
            let first = add_object(
                &mut bytes,
                9,
                b"<< /Type /Page /Parent 5 0 R /MediaBox [0 0 200 300] >>",
            );
            let second = add_object(
                &mut bytes,
                3,
                b"<< /Type /Page /Parent 5 0 R /MediaBox [0 0 200 300] >>",
            );
            let root = add_object(&mut bytes, 5, b"<< /Type /Pages /Count 1 /Kids [9 0 R] >>");
            let objects = [first, second, root];
            let pages = [reference(9), reference(3)];
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: None,
            };
            let mut source = BytesSource(bytes);
            assert!(matches!(
                reconstruct_fragment(
                    &mut source,
                    &mut sink,
                    &plan,
                    &Limits::default(),
                    &NeverCancel,
                )
                .await,
                Err(Error::Pdf {
                    kind: PdfErrorKind::AmbiguousRepair,
                    ..
                })
            ));
            assert!(sink.bytes.is_empty());
        });
    }

    struct TestCancel(Cell<bool>);

    impl Cancellation for TestCancel {
        fn is_cancelled(&self) -> bool {
            self.0.get()
        }
    }

    struct CancelOnFlushSink<'a> {
        bytes: Vec<u8>,
        cancellation: &'a TestCancel,
    }

    impl SequentialSink for CancelOnFlushSink<'_> {
        async fn write(&mut self, bytes: &[u8]) -> Result<usize> {
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        async fn flush(&mut self) -> Result<()> {
            self.cancellation.0.set(true);
            Ok(())
        }
    }

    #[test]
    fn cancellation_before_input_and_during_flush_never_reports_success() {
        run(async {
            let (mut source, objects, pages) = two_page_fragment();
            let plan = FragmentPlan {
                objects: &objects,
                pages: &pages,
                pages_root: reference(5),
                catalog: None,
            };
            let cancelled = TestCancel(Cell::new(true));
            let mut sink = BytesSink::default();
            assert!(matches!(
                reconstruct_fragment(
                    &mut source,
                    &mut sink,
                    &plan,
                    &Limits::default(),
                    &cancelled,
                )
                .await,
                Err(Error::Cancelled)
            ));
            assert!(sink.bytes.is_empty());

            let cancellation = TestCancel(Cell::new(false));
            let mut sink = CancelOnFlushSink {
                bytes: Vec::new(),
                cancellation: &cancellation,
            };
            assert!(matches!(
                reconstruct_fragment(
                    &mut source,
                    &mut sink,
                    &plan,
                    &Limits::default(),
                    &cancellation,
                )
                .await,
                Err(Error::Cancelled)
            ));
            assert!(!sink.bytes.is_empty());
        });
    }
}
