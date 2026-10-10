// SPDX-License-Identifier: MIT

//! [`convert`]: detection, dispatch to the format engines and the HN/C8
//! resource wiring.

use super::observe::{Observer, Progress, refused};
use super::{Detection, InputFormat, detect_source, pdf_range};
use crate::caj::convert_caj;
use crate::hnc8::{
    ApplicationInfoStatus, C8FontSource, C8FontSources, C8PageFonts, ComposeOptions, ComposePage,
    ComposeVisitor, NativeSymbolGlyph, OutlineReport, convert_document_pdf, uses_native_text,
};
use crate::jbig2::text::TextHeaderPolicy;
use crate::kdh::convert_kdh;
use crate::pdf::copy_pdf_range;
use crate::qm::QmTable;
use crate::{Error, ErrorKind, Limits, RangedSource, Result};
use std::io::Write;

/// The reason of the error [`convert`] returns when [`Fonts`] are given for a
/// document other than HN or C8.
pub const FONTS_REQUIRE_HNC8: &str =
    "explicit native font resources require converting a C8 or HN-B document";

/// Font resources for native HN/C8 text.
///
/// Up to eight ranged resources and their collection faces; `roles` index
/// into `sources`, and several roles may share one resource. Absent optional
/// roles and unmapped characters follow the [`C8PageFonts`] fallback rule.
#[derive(Default)]
pub struct Fonts<'a> {
    pub sources: Vec<C8FontSource<Box<dyn RangedSource + 'a>>>,
    /// Which source draws each role; sources without roles are refused.
    pub roles: Option<C8PageFonts>,
    /// Explicit source glyphs for HN-B mode-0 symbol codes, drawn from the
    /// `symbols` role without fallback. Semantic text is unchanged. Native
    /// documents of another profile refuse a non-empty map; like the fonts,
    /// it is unread for documents routed to image composition.
    pub symbol_glyphs: Vec<NativeSymbolGlyph>,
}

/// What [`convert`] produces from a document.
pub struct ConversionOptions<'a> {
    /// The input family; `None` detects it from the leading signature. An
    /// explicit PDF must start with its `%PDF-` header.
    pub format: Option<InputFormat>,
    /// Write the CAJ, PDF or HN-A outline. C8/HN-B outlines are unverified and
    /// never written; [`ConversionReport::outline`] says when one was omitted.
    pub include_bookmarks: bool,
    /// Replace damaged CAJ pages with blank pages and report every omission.
    pub allow_damaged: bool,
    /// Validation of HN/C8 type-3 text-region headers. The default admits the
    /// measured HN/C8 unused-refinement-template anomaly; general JBIG2 APIs
    /// and every other malformed flag stay strict.
    pub text_header_policy: TextHeaderPolicy,
    /// Fonts for native HN/C8 text. The core decides once per document
    /// whether its text is native ([`crate::hnc8::uses_native_text`]); image
    /// documents leave the fonts unread, so their PDF is byte-identical to a
    /// conversion without fonts. Any other format refuses fonts with
    /// [`FONTS_REQUIRE_HNC8`].
    pub fonts: Option<Fonts<'a>>,
}

impl Default for ConversionOptions<'_> {
    fn default() -> Self {
        Self {
            format: None,
            include_bookmarks: true,
            allow_damaged: false,
            text_header_policy: TextHeaderPolicy::HnC8UnusedRefinementTemplate,
            fonts: None,
        }
    }
}

/// A page whose content was replaced by an explicit blank page.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OmittedPage {
    pub page_index: u32,
    /// Absolute input offset of the damaged object or missing dependency owner.
    pub offset: u64,
}

/// Images drawn by an HN/C8 conversion, by codec.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ImageCounts {
    pub type0: u64,
    pub jpeg: u64,
    pub type3: u64,
}

/// Counters and warnings from a completed conversion.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConversionReport {
    /// Input bytes read, detection included.
    pub input_bytes_read: u64,
    pub output_bytes_written: u64,
    pub pages_converted: u32,
    /// Private-use glyphs drawn with a visual substitute from the caller's
    /// font. Original private-use codes are retained in PDF ActualText; this
    /// count does not imply a standard Unicode identity for their shapes.
    pub substituted_glyphs: u64,
    pub bookmarks_written: u32,
    /// Blank substitutions in source page order; indices are zero-based.
    pub omitted_pages: Vec<OmittedPage>,
    /// HN-A outline entries skipped or clamped, or C8/HN-B bookmarks omitted
    /// as unverified; empty for other formats.
    pub outline: OutlineReport,
    /// Whether a C8 application-info package was read into the PDF `/Info`
    /// or ignored as defective; `Absent` for other formats.
    pub application_info: ApplicationInfoStatus,
    /// Images drawn by an HN/C8 conversion; zero for other formats.
    pub images: ImageCounts,
}

/// Convert one document to PDF bytes written to `sink`.
///
/// The family is detected from the leading signature unless
/// [`ConversionOptions::format`] names it, and reported to
/// [`Progress::format`]. PDF is copied from its `%PDF-` header, CAJ and KDH
/// are rebuilt, and HN/C8 pages are composed, with native text when the
/// document needs it and [`ConversionOptions::fonts`] are given. Every
/// source page becomes an output page or the conversion fails; only
/// [`ConversionOptions::allow_damaged`] substitutes blank CAJ pages, and
/// [`ConversionReport::omitted_pages`] lists them.
///
/// An empty or unrecognized input, and NH or TEB, are refused with an
/// [`ErrorKind::UnsupportedFormat`] error that has no offset, context or
/// reason. On any error the bytes already written are not a valid PDF;
/// callers that need atomic output stage it and commit only on success.
///
/// ```no_run
/// use caj2pdf_core::{ConversionOptions, Limits, NeverCancel, convert, native::SeekableSource};
/// use std::{fs::File, io::BufWriter};
///
/// let mut input = SeekableSource::new(File::open("paper.caj")?)?;
/// let mut output = BufWriter::new(File::create("paper.pdf")?);
/// let report = convert(
///     &mut input,
///     &mut output,
///     ConversionOptions::default(),
///     &Limits::default(),
///     &mut NeverCancel,
/// )?;
/// println!("{} pages, {} bookmarks", report.pages_converted, report.bookmarks_written);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// Any [`RangedSource`] works, a byte slice included:
///
/// ```
/// use caj2pdf_core::{ConversionOptions, ErrorKind, Limits, NeverCancel, convert};
///
/// let mut output = Vec::new();
/// let error = convert(
///     &mut &b"not a document"[..],
///     &mut output,
///     ConversionOptions::default(),
///     &Limits::default(),
///     &mut NeverCancel,
/// )
/// .unwrap_err();
/// assert!(matches!(error.kind, ErrorKind::UnsupportedFormat));
/// assert!(output.is_empty());
/// ```
pub fn convert<S: RangedSource, W: Write>(
    source: &mut S,
    sink: &mut W,
    options: ConversionOptions<'_>,
    limits: &Limits,
    progress: &mut dyn Progress,
) -> Result<ConversionReport> {
    convert_inner(source, sink, options, None, limits, progress)
}

/// Convert the measured TTKN server-auth PDF profile using an explicit response.
/// No authentication endpoint is contacted. The response is case-sensitive and
/// is never included in errors. Other TTKN profiles remain unsupported.
/// Atomic-output requirements are the same as for [`convert`].
pub fn convert_with_ttkn_response<S: RangedSource, W: Write>(
    source: &mut S,
    sink: &mut W,
    options: ConversionOptions<'_>,
    response: &crate::pdf::TtknResponse,
    limits: &Limits,
    progress: &mut dyn Progress,
) -> Result<ConversionReport> {
    convert_inner(source, sink, options, Some(response), limits, progress)
}

fn convert_inner<S: RangedSource, W: Write>(
    source: &mut S,
    sink: &mut W,
    options: ConversionOptions<'_>,
    response: Option<&crate::pdf::TtknResponse>,
    limits: &Limits,
    progress: &mut dyn Progress,
) -> Result<ConversionReport> {
    let observer = Observer::new(progress);
    let mut source = observer.track(source);
    let Detection {
        format,
        header_offset,
        bytes_read,
    } = observer.resolve(&mut source, options.format, limits)?;
    if options.fonts.is_some() && !matches!(format, InputFormat::Hn | InputFormat::C8) {
        return Err(Error::invalid(FONTS_REQUIRE_HNC8));
    }
    if response.is_some() && format != InputFormat::Pdf {
        return Err(Error::invalid("TTKN response requires a PDF input"));
    }
    let mut report = match format {
        InputFormat::Pdf => {
            let range = pdf_range(source.size(), header_offset);
            match response {
                Some(response) => {
                    crate::pdf::convert_ttkn(&mut source, sink, range, response, limits, &observer)?
                }
                None => copy_pdf_range(&mut source, sink, range, limits, &observer)?,
            }
        }
        InputFormat::Caj => convert_caj(&mut source, sink, &options, limits, &observer)?,
        InputFormat::Kdh => convert_kdh(&mut source, sink, limits, &observer)?,
        InputFormat::Hn | InputFormat::C8 => {
            convert_hnc8(&mut source, sink, options, limits, &observer)?
        }
        InputFormat::Nh | InputFormat::Teb | InputFormat::Caa => return Err(refused()),
    };
    report.input_bytes_read = report.input_bytes_read.saturating_add(bytes_read);
    Ok(report)
}

/// Every source page must become an output page: a page without image
/// content is refused, never dropped.
struct CompletePages;

impl ComposeVisitor for CompletePages {
    fn page(&mut self, page: ComposePage<'_>) -> Result<()> {
        if page.output_page.is_none() {
            return Err(Error::from(ErrorKind::UnsupportedFormat)
                .because("HN/C8 conversion cannot omit source pages without image content"));
        }
        Ok(())
    }
}

/// Compose an HN/C8 document with the standard QM table, routing it to
/// native composition when it has native text and fonts are given.
fn convert_hnc8<S: RangedSource, W: Write>(
    source: &mut S,
    sink: &mut W,
    options: ConversionOptions<'_>,
    limits: &Limits,
    observer: &Observer<'_>,
) -> Result<ConversionReport> {
    let compose = ComposeOptions {
        text_header_policy: options.text_header_policy,
        include_bookmarks: options.include_bookmarks,
    };
    let mut fonts = match options.fonts {
        None => None,
        Some(Fonts {
            sources,
            roles,
            symbol_glyphs,
        }) => {
            let roles = roles.ok_or(Error::invalid("C8 font resources require explicit roles"))?;
            for font in &sources {
                limits.check_input_size(font.source.size())?;
            }
            Some((sources, roles, symbol_glyphs))
        }
    };
    let report = convert_document_pdf(
        source,
        sink,
        fonts
            .as_mut()
            .map(|(sources, roles, symbol_glyphs)| C8FontSources {
                sources,
                roles: *roles,
                symbol_glyphs,
            }),
        Some(&QmTable::standard()),
        &mut CompletePages,
        compose,
        limits,
        observer,
    )?;
    Ok(ConversionReport {
        outline: report.outline,
        application_info: report.application_info,
        images: ImageCounts {
            type0: report.type0_images,
            jpeg: report.jpeg_images,
            type3: report.type3_images,
        },
        ..report.conversion
    })
}

/// Whether [`convert`] would draw native text for this input and therefore
/// needs [`Fonts`]: an HN or C8 document that
/// [`crate::hnc8::uses_native_text`] routes to native composition. Other
/// families, and empty or unrecognized inputs, need none; converting them
/// reports their own errors. Only a failed read or cancellation is an error.
/// [`Progress::format`] is not called.
pub fn needs_fonts<S: RangedSource>(
    source: &mut S,
    limits: &Limits,
    progress: &mut dyn Progress,
) -> Result<bool> {
    let observer = Observer::new(progress);
    let mut source = observer.track(source);
    match detect_source(&mut source, limits, &observer) {
        Ok(Some(Detection {
            format: InputFormat::Hn | InputFormat::C8,
            ..
        })) => uses_native_text(&mut source, limits, &observer),
        _ => Ok(false),
    }
}
