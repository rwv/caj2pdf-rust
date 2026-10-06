// SPDX-License-Identifier: MIT

//! [`inspect`], the HN/C8 per-page report, and the inputs of an outline
//! import.

use super::observe::{Observer, Progress, refused};
use super::{Bookmark, BookmarkVisitor, Detection, InputFormat, pdf_range};
use crate::caj::parse_metadata;
use crate::hnc8::{
    ApplicationInfoReport, ApplicationInfoTail, Header, Hnc8Reader, ImageRecord, OutlineReport,
    PageRecord, TextStructure, Variant,
};
use crate::kdh::{HEADER_SIGNATURE, KdhPdfSource};
use crate::pdf::PdfIndex;
use crate::{CountingSource, Error, ErrorKind, Limits, RangedSource, Result, read_exact_at};

/// What [`inspect`] reads besides the summary.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InspectOptions {
    /// The input family; `None` detects it from the leading signature.
    pub format: Option<InputFormat>,
    /// List the outline entries where the format has a readable outline (CAJ
    /// and HN-A); otherwise they are only counted.
    pub bookmarks: bool,
    /// Read the document-level [`Structure`] for a per-page report.
    pub structure: bool,
}

/// Structure-only document facts; never document text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Structure {
    /// The leading wrapper bytes (at most 32) and whether they are the
    /// supported signature.
    Kdh { signature: Vec<u8>, supported: bool },
    /// The HN/C8 page-index layout and application-info tail.
    Hnc8 {
        header: Header,
        page_row_bytes: u64,
        application_info: Option<ApplicationInfoTail>,
    },
}

/// Bounded summary returned by [`inspect`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentInfo {
    pub format: InputFormat,
    /// The measured HN/C8 container layout; `None` for other families.
    pub variant: Option<Variant>,
    /// `None` for a family [`super::convert`] refuses, and for a KDH wrapper
    /// whose signature is unsupported.
    pub page_count: Option<u32>,
    /// Whether the document has an outline; `None` when unknown.
    pub has_outline: Option<bool>,
    /// Outline entries (CAJ, and the HN-A entries written after validation);
    /// `None` if the outline layout is unknown or not counted.
    pub bookmark_count: Option<u32>,
    /// The listed outline, with [`InspectOptions::bookmarks`], for CAJ and
    /// HN-A.
    pub bookmarks: Option<Vec<Bookmark>>,
    /// HN-A entries skipped or clamped while reading the outline; empty
    /// otherwise.
    pub outline: OutlineReport,
    /// The C8 application-info package; absent for other families. A
    /// defective package is reported here, not as an error.
    pub application_info: ApplicationInfoReport,
    /// Read only with [`InspectOptions::structure`], for KDH and HN/C8.
    pub structure: Option<Structure>,
    /// Input bytes read, detection included.
    pub input_bytes_read: u64,
}

impl DocumentInfo {
    fn unknown(format: InputFormat) -> Self {
        Self {
            format,
            variant: None,
            page_count: None,
            has_outline: None,
            bookmark_count: None,
            bookmarks: None,
            outline: OutlineReport::default(),
            application_info: ApplicationInfoReport::default(),
            structure: None,
            input_bytes_read: 0,
        }
    }

    fn indexed(&mut self, index: &PdfIndex) {
        // The PDF index enforces `Limits::max_pages`, a u32.
        self.page_count = Some(index.pages().len() as u32);
        self.has_outline = Some(index.has_outlines());
    }
}

/// Read bounded document metadata without writing or decoding any page.
///
/// The family is detected unless [`InspectOptions::format`] names it, and
/// reported to [`Progress::format`]; an empty or unrecognized input is
/// refused as by [`super::convert`]. PDF and KDH are indexed, CAJ and HN/C8
/// metadata and outlines are read, and NH and TEB report only their family.
pub fn inspect<S: RangedSource>(
    source: &mut S,
    options: &InspectOptions,
    limits: &Limits,
    progress: &mut dyn Progress,
) -> Result<DocumentInfo> {
    let observer = Observer::new(progress);
    let mut tracked = observer.track(source);
    let detection = observer.resolve(&mut tracked, options.format, limits)?;
    let mut input_bytes_read = 0;
    let mut info = read_info(
        &mut CountingSource::new(&mut tracked, &mut input_bytes_read),
        detection,
        options,
        limits,
        &observer,
    )?;
    info.input_bytes_read = input_bytes_read.saturating_add(detection.bytes_read);
    Ok(info)
}

fn read_info<S: RangedSource>(
    source: &mut S,
    detection: Detection,
    options: &InspectOptions,
    limits: &Limits,
    observer: &Observer<'_>,
) -> Result<DocumentInfo> {
    let mut info = DocumentInfo::unknown(detection.format);
    match detection.format {
        InputFormat::Pdf => {
            let range = pdf_range(source.size(), detection.header_offset);
            info.indexed(&PdfIndex::open(source, range, limits, observer)?);
        }
        InputFormat::Kdh => {
            if options.structure {
                // A different signature is reported instead of an error, so
                // that it can be diagnosed.
                let mut signature =
                    vec![0; source.size().min(HEADER_SIGNATURE.len() as u64) as usize];
                read_exact_at(source, 0, &mut signature, limits, observer)?;
                let supported = signature == HEADER_SIGNATURE;
                info.structure = Some(Structure::Kdh {
                    signature,
                    supported,
                });
                if !supported {
                    return Ok(info);
                }
            }
            let mut decoded = KdhPdfSource::open(source, limits, observer)?;
            let range = pdf_range(decoded.size(), 0);
            info.indexed(&PdfIndex::open(&mut decoded, range, limits, observer)?);
        }
        InputFormat::Caj => {
            let metadata = parse_metadata(source, limits, observer)?;
            info.page_count = Some(metadata.page_count);
            info.has_outline = Some(!metadata.bookmarks.is_empty());
            // `parse_metadata` enforces `Limits::max_bookmarks`, a u32.
            info.bookmark_count = Some(metadata.bookmarks.len() as u32);
            info.bookmarks = options.bookmarks.then_some(metadata.bookmarks);
        }
        InputFormat::Hn | InputFormat::C8 => {
            inspect_hnc8(source, &mut info, options, limits, observer)?;
        }
        InputFormat::Nh | InputFormat::Teb => {}
    }
    Ok(info)
}

/// Keep only bounded outline and application-info metadata; image payloads
/// are never read. The structure adds the page-index layout and the
/// application-info tail.
fn inspect_hnc8<S: RangedSource>(
    source: &mut S,
    info: &mut DocumentInfo,
    options: &InspectOptions,
    limits: &Limits,
    observer: &Observer<'_>,
) -> Result<()> {
    let mut reader = Hnc8Reader::open(source, limits, observer)?;
    let header = reader.header();
    info.variant = Some(header.variant);
    info.page_count = Some(header.page_count);
    if options.structure {
        info.structure = Some(Structure::Hnc8 {
            header,
            page_row_bytes: reader.page_row_bytes(),
            application_info: reader.application_info_tail()?,
        });
    }
    info.application_info = reader.application_info_report()?;
    let Some(count) = reader.declared_bookmark_count() else {
        return Ok(());
    };
    let map = |page: u32| Some(page - 1);
    let outline = if options.bookmarks {
        let mut collected = Collected::new(count, limits)?;
        let outline = reader.visit_bookmarks(64, header.page_count, map, &mut collected)?;
        info.bookmarks = Some(collected.items);
        outline
    } else {
        reader.visit_bookmarks(64, header.page_count, map, &mut Counted)?
    };
    info.bookmark_count = Some(outline.written);
    info.has_outline = Some(outline.written != 0);
    info.outline = outline;
    Ok(())
}

/// HN-A bookmarks, with the records and their retained titles counted
/// against the allocation limit.
struct Collected<'l> {
    items: Vec<Bookmark>,
    limits: &'l Limits,
    bytes: u64,
}

impl<'l> Collected<'l> {
    /// Reserve the `count` declared records. A count over the bookmark limit
    /// reserves nothing: the outline traversal refuses it first.
    fn new(count: u32, limits: &'l Limits) -> Result<Self> {
        let bytes = u64::from(count) * size_of::<Bookmark>() as u64;
        let mut items = Vec::new();
        if count <= limits.max_bookmarks {
            limits.check_allocation(bytes)?;
            items
                .try_reserve_exact(count as usize)
                .map_err(|_| limits.allocation_refused("HN-A outline metadata", bytes))?;
        }
        Ok(Self {
            items,
            limits,
            bytes,
        })
    }
}

impl BookmarkVisitor for Collected<'_> {
    fn visit(&mut self, bookmark: Bookmark) -> Result<()> {
        self.bytes += bookmark.title.capacity() as u64;
        self.limits.check_allocation(self.bytes)?;
        self.items.push(bookmark);
        Ok(())
    }
}

/// Validates and counts HN-A bookmarks without keeping them.
struct Counted;

impl BookmarkVisitor for Counted {
    fn visit(&mut self, _: Bookmark) -> Result<()> {
        Ok(())
    }
}

/// A recipient for [`inspect_pages`]. An error it returns ends the report
/// and is returned as it is.
pub trait PageVisitor {
    /// Called once, before the first page.
    fn begin(&mut self) -> Result<()>;
    /// A page's row, or `None` when the row could not be read.
    fn page(&mut self, number: u32, row: Option<&PageRecord>) -> Result<()>;
    /// One image descriptor of the current page, in order.
    fn image(&mut self, image: &ImageRecord) -> Result<()>;
    /// The end of the current page: its text structure or the error reading
    /// it, or, for a page whose row or image descriptors failed, that
    /// `error` alone.
    fn end_page(
        &mut self,
        text: Option<&TextStructure>,
        text_error: Option<&Error>,
        error: Option<&Error>,
    ) -> Result<()>;
    /// Called once, after the last page.
    fn finish(&mut self) -> Result<()>;
}

/// Stream one structural record per page of the HN/C8 document that `info`
/// (from [`inspect`]) describes; image payloads and text content are never
/// reported. Returns `false` without calling `visitor` when the family has
/// no per-page records.
///
/// Each page uses a fresh cursor, so a malformed page is reported and later
/// pages are still inspected; only one page's row, the current descriptor
/// and bounded text-reader state are held at a time. A failure that would
/// repeat on every later page (reopening the container, a failed read or
/// cancellation) ends the report with an error.
pub fn inspect_pages<S: RangedSource>(
    source: &mut S,
    info: &DocumentInfo,
    limits: &Limits,
    progress: &mut dyn Progress,
    visitor: &mut dyn PageVisitor,
) -> Result<bool> {
    let (Some(_), Some(page_count)) = (info.variant, info.page_count) else {
        return Ok(false);
    };
    let observer = Observer::new(progress);
    let mut source = observer.track(source);
    visitor.begin()?;
    for number in 1..=page_count {
        let mut reader = Hnc8Reader::probe_at_page(&mut source, limits, &observer, number)?;
        let row = match reader.next_page() {
            Ok(row) => row.expect("a probe opens at a declared page"),
            Err(error) => {
                let error = page_error(error)?;
                visitor.page(number, None)?;
                visitor.end_page(None, None, Some(&error))?;
                continue;
            }
        };
        visitor.page(number, Some(&row))?;
        let error = loop {
            match reader.next_image() {
                Ok(Some(image)) => visitor.image(&image)?,
                Ok(None) => break None,
                Err(error) => break Some(page_error(error)?),
            }
        };
        if error.is_some() {
            visitor.end_page(None, None, error.as_ref())?;
            continue;
        }
        match reader.inspect_text() {
            Ok(text) => visitor.end_page(Some(&text), None, None)?,
            Err(error) => visitor.end_page(None, Some(&page_error(error)?), None)?,
        }
    }
    visitor.finish()?;
    Ok(true)
}

/// A failure of one page, reported for that page; a failed read or
/// cancellation would repeat on every later page and ends the report.
fn page_error(error: Error) -> Result<Error> {
    match error.kind {
        ErrorKind::Cancelled | ErrorKind::Io(_) => Err(error),
        _ => Ok(error),
    }
}

/// The outline of a CAJ document, for an outline import. Any other family
/// is refused as by [`super::convert`], after [`Progress::format`] names it.
pub fn read_outline<S: RangedSource>(
    source: &mut S,
    limits: &Limits,
    progress: &mut dyn Progress,
) -> Result<Vec<Bookmark>> {
    let observer = Observer::new(progress);
    let mut source = observer.track(source);
    match observer.resolve(&mut source, None, limits)?.format {
        InputFormat::Caj => Ok(parse_metadata(&mut source, limits, &observer)?.bookmarks),
        _ => Err(refused()),
    }
}

/// Index a PDF, whose `%PDF-` header may follow leading bytes, for
/// [`crate::pdf::PdfOutlineAppender`]. Any other family is refused as by
/// [`super::convert`], after [`Progress::format`] names it.
pub fn index_pdf<S: RangedSource>(
    source: &mut S,
    limits: &Limits,
    progress: &mut dyn Progress,
) -> Result<PdfIndex> {
    let observer = Observer::new(progress);
    let mut source = observer.track(source);
    let detection = observer.resolve(&mut source, None, limits)?;
    match detection.format {
        InputFormat::Pdf => {
            let range = pdf_range(source.size(), detection.header_offset);
            PdfIndex::open(&mut source, range, limits, &observer)
        }
        _ => Err(refused()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Context, NeverCancel};

    fn hn_a(count: usize, levels: impl Fn(usize) -> u8) -> Vec<u8> {
        let mut bytes = vec![0; 0x15c + count * 308 + 20];
        bytes[..8].copy_from_slice(&[72, 78, 0, 0, 0x90, 1, 0, 0]);
        bytes[0x90] = 1;
        bytes[0x158] = count as u8;
        for number in 0..count {
            let at = 0x15c + number * 308;
            bytes[at..at + 4].copy_from_slice(b"Root");
            bytes[at + 280] = levels(number);
            bytes[at + 304] = 1;
        }
        bytes
    }

    fn listing() -> InspectOptions {
        InspectOptions {
            bookmarks: true,
            ..InspectOptions::default()
        }
    }

    #[test]
    fn outline_collection_accounts_for_records_and_retained_titles() {
        let bytes = hn_a(1, |_| b'1');
        for (max_bookmarks, max_allocation_bytes) in
            [(0, 4096), (1, 1), (1, size_of::<Bookmark>() as u64)]
        {
            let limits = Limits {
                max_bookmarks,
                max_allocation_bytes,
                io_chunk_bytes: 1,
                ..Limits::default()
            };
            let error = inspect(&mut &bytes[..], &listing(), &limits, &mut NeverCancel)
                .unwrap_err()
                .to_string();
            assert!(error.contains("limit"), "{error}");
        }
    }

    #[test]
    fn hn_a_outlines_are_listed_or_counted_with_the_same_validation() {
        // One valid root, then entries whose page 0 is outside the document.
        let bytes = hn_a(3, |number| if number == 0 { b'1' } else { b'0' });
        let listed = inspect(
            &mut &bytes[..],
            &listing(),
            &Limits::default(),
            &mut NeverCancel,
        )
        .unwrap();
        let counted = inspect(
            &mut &bytes[..],
            &InspectOptions::default(),
            &Limits::default(),
            &mut NeverCancel,
        )
        .unwrap();
        assert_eq!(listed.bookmarks.as_ref().map(Vec::len), Some(1));
        assert_eq!(counted.bookmarks, None);
        for info in [&listed, &counted] {
            assert_eq!(info.variant, Some(Variant::HnA));
            assert_eq!((info.page_count, info.bookmark_count), (Some(1), Some(1)));
            assert_eq!(info.has_outline, Some(true));
            assert_eq!(info.outline.defects, 2);
            assert!(info.input_bytes_read > 0);
        }
    }

    #[test]
    fn only_repeating_page_failures_end_the_page_report() {
        let page = Context::Hnc8 {
            variant: None,
            page: Some(2),
            image: None,
            segment: None,
            stage: None,
        };
        let io = std::io::Error::other("disk failed");
        for error in [Error::cancelled(), Error::from(ErrorKind::Io(io))] {
            assert!(page_error(error.at(7).within(page)).is_err());
        }
        let error = Error::malformed(7, "page has unread image records").within(page);
        assert_eq!(
            page_error(error).unwrap().to_string(),
            "malformed HN/C8 at byte 7, page 2: page has unread image records"
        );
    }

    #[test]
    fn families_without_page_records_skip_the_page_report() {
        struct Refuse;
        impl PageVisitor for Refuse {
            fn begin(&mut self) -> Result<()> {
                unreachable!()
            }
            fn page(&mut self, _: u32, _: Option<&PageRecord>) -> Result<()> {
                unreachable!()
            }
            fn image(&mut self, _: &ImageRecord) -> Result<()> {
                unreachable!()
            }
            fn end_page(
                &mut self,
                _: Option<&TextStructure>,
                _: Option<&Error>,
                _: Option<&Error>,
            ) -> Result<()> {
                unreachable!()
            }
            fn finish(&mut self) -> Result<()> {
                unreachable!()
            }
        }
        let info = DocumentInfo::unknown(InputFormat::Teb);
        let reported = inspect_pages(
            &mut &b"TEB"[..],
            &info,
            &Limits::default(),
            &mut NeverCancel,
            &mut Refuse,
        );
        assert!(!reported.unwrap());
    }
}
