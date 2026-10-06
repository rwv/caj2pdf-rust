// SPDX-License-Identifier: MIT

//! Format detection and dispatch to the core conversion, inspection, and
//! outline-import operations.

use crate::CliError;
use crate::files::{Input, stdout_error};
use crate::progress::Progress;
use crate::signals::ProcessCancellation;
use caj2pdf_core::{
    Bookmark, ConversionOptions, Detection, Error, InputFormat, Limits, RangedSource, caj,
    detect_source,
    hnc8::{
        ApplicationInfoReport, ApplicationInfoStatus, ApplicationInfoTail, Header, OutlineReport,
    },
    kdh::{HEADER_SIGNATURE, KdhPdfSource, convert_kdh},
    native::{SeekableSource, WriteSink},
    pdf::{PdfIndex, PdfOutlineAppender, PdfRange, copy_pdf_range},
    read_exact_at,
};
use std::fs::File;
use std::io::Write;

pub fn format_name(format: InputFormat) -> &'static str {
    match format {
        InputFormat::Pdf => "PDF",
        InputFormat::Caj => "CAJ",
        InputFormat::Kdh => "KDH",
        InputFormat::Nh => "NH",
        InputFormat::Hn => "HN",
        InputFormat::C8 => "C8",
        InputFormat::Teb => "TEB",
    }
}

/// Whether this build can convert a recognized format.
pub fn conversion_supported(format: InputFormat) -> bool {
    matches!(
        format,
        InputFormat::Pdf | InputFormat::Caj | InputFormat::Kdh | InputFormat::Hn | InputFormat::C8
    )
}

/// Render a core error for a diagnostic.
fn text(error: Error) -> String {
    error.to_string()
}

fn detect<S: RangedSource>(source: &mut S, limits: &Limits) -> Result<Detection, String> {
    let detection = detect_source(source, limits, &ProcessCancellation).map_err(text)?;
    if source.size() == 0 {
        return Err("input is empty".to_owned());
    }
    detection.ok_or_else(|| "unrecognized input format".to_owned())
}

/// The PDF viewed from its `%PDF-` header, which may follow leading bytes.
fn pdf_range(size: u64, header_offset: u64) -> PdfRange {
    PdfRange {
        offset: header_offset,
        length: size - header_offset,
    }
}

/// Why a recognized format is never converted, when that is known.
///
/// TEB is a CNKI DRM container whose document entries are encrypted
/// (rwv/caj2pdf-samples research notes); this project does not decrypt it.
pub fn unsupported_reason(format: InputFormat) -> Option<&'static str> {
    matches!(format, InputFormat::Teb).then_some("drm-encrypted")
}

pub(crate) fn unsupported(format: InputFormat) -> String {
    match unsupported_reason(format) {
        Some(_) => "TEB input is a DRM-encrypted CNKI container; \
                    its document content is encrypted and cannot be converted"
            .to_owned(),
        None => format!(
            "{name} input is recognized, but {name} conversion is not supported",
            name = format_name(format)
        ),
    }
}

fn ranged(file: &mut File) -> Result<SeekableSource<&mut File>, String> {
    SeekableSource::new(file).map_err(text)
}

/// Diagnostics of a successful conversion, printed after the output commits.
#[derive(Debug, Default)]
pub struct Warnings {
    /// Skipped or clamped HN-A bookmarks, or omitted C8/HN-B outlines.
    pub outline: OutlineReport,
    pub omitted_pages: Vec<caj2pdf_core::OmittedPage>,
    /// A C8 application-info package, when ignored as defective.
    pub application_info: ApplicationInfoStatus,
}

/// Convert one input to PDF bytes written to `writer`. The returned warnings
/// are empty unless an HN/C8 outline or application-info package was skipped.
pub fn convert<W: Write>(
    input: &mut Input,
    writer: W,
    limits: &Limits,
    resources: &mut crate::hnc8::Resources,
    options: ConversionOptions,
    progress: Option<&mut dyn Write>,
) -> Result<Warnings, CliError> {
    let result = (|| {
        let mut source = Progress::new(ranged(&mut input.file)?, progress);
        let result = convert_source(&mut source, writer, limits, resources, options);
        source.finish();
        result
    })();
    result.map_err(|message| CliError::runtime(format!("cannot convert {}: {message}", input.name)))
}

fn convert_source<S: RangedSource, W: Write>(
    source: &mut S,
    writer: W,
    limits: &Limits,
    resources: &mut crate::hnc8::Resources,
    options: ConversionOptions,
) -> Result<Warnings, String> {
    let mut sink = WriteSink::new(writer);
    let Detection {
        format,
        header_offset,
        ..
    } = detect(source, limits)?;
    let pdf = pdf_range(source.size(), header_offset);
    if resources.has_fonts() && !matches!(format, InputFormat::C8 | InputFormat::Hn) {
        return Err("explicit native fonts require a C8 or HN-B document".into());
    }
    match format {
        InputFormat::Pdf => {
            copy_pdf_range(source, &mut sink, pdf, limits, &ProcessCancellation).map_err(text)
        }
        InputFormat::Caj => {
            caj::convert_caj(source, &mut sink, options, limits, &ProcessCancellation).map_err(text)
        }
        InputFormat::Kdh => {
            convert_kdh(source, &mut sink, limits, &ProcessCancellation).map_err(text)
        }
        InputFormat::Hn | InputFormat::C8 => {
            return crate::hnc8::convert(
                source,
                &mut sink,
                resources,
                options.include_bookmarks,
                limits,
            )
            .map(|(outline, application_info)| Warnings {
                outline,
                application_info,
                ..Warnings::default()
            });
        }
        other => Err(unsupported(other)),
    }
    .map(|report| Warnings {
        omitted_pages: report.omitted_pages,
        ..Warnings::default()
    })
}

/// Whether `input` is an HN/C8 document that converts with native text
/// composition and therefore needs fonts. Other and unrecognized formats
/// return `false`; conversion then reports its own errors.
pub fn uses_native_text(input: &mut Input, limits: &Limits) -> Result<bool, CliError> {
    (|| {
        let mut source = ranged(&mut input.file)?;
        match detect(&mut source, limits) {
            Ok(Detection {
                format: InputFormat::Hn | InputFormat::C8,
                ..
            }) => caj2pdf_core::hnc8::uses_native_text(
                &mut source,
                crate::hnc8::compose_options(false),
                limits,
                &ProcessCancellation,
            )
            .map_err(|e| e.to_string()),
            _ => Ok(false),
        }
    })()
    .map_err(read_error(&input.name))
}

/// Bounded document metadata for `inspect`.
#[derive(Debug, Eq, PartialEq)]
pub struct Inspection {
    pub format: InputFormat,
    /// The measured HN/C8 container layout, when applicable.
    pub variant: Option<&'static str>,
    pub page_count: Option<u32>,
    /// Whether the document has an outline, when known.
    pub has_outline: Option<bool>,
    /// The listed outline, when this format's outline can be read.
    pub bookmarks: Option<Vec<Bookmark>>,
    /// HN-A entries skipped or clamped while listing `bookmarks`; empty otherwise.
    pub outline: OutlineReport,
    /// The C8 application-info package; absent for other formats.
    pub application_info: ApplicationInfoReport,
    /// Document-level structure, read only for `--pages`.
    pub structure: Option<Structure>,
}

/// Structure-only document facts for `inspect --pages`; never document text.
#[derive(Debug, Eq, PartialEq)]
pub enum Structure {
    /// The leading wrapper bytes (at most 32) and whether they are the
    /// supported signature.
    Kdh { signature: Vec<u8>, supported: bool },
    Hnc8 {
        header: Header,
        page_row_bytes: u64,
        application_info: Option<ApplicationInfoTail>,
    },
}

fn index_pdf<S: RangedSource>(
    source: &mut S,
    header_offset: u64,
    limits: &Limits,
) -> Result<PdfIndex, String> {
    let range = pdf_range(source.size(), header_offset);
    PdfIndex::open(source, range, limits, &ProcessCancellation).map_err(text)
}

/// Page count and outline presence from an indexed PDF; unknown without one.
fn pdf_inspection(format: InputFormat, index: Option<&PdfIndex>) -> Inspection {
    Inspection {
        format,
        variant: None,
        page_count: index.map(|index| index.pages().len() as u32),
        has_outline: index.map(PdfIndex::has_outlines),
        bookmarks: None,
        outline: OutlineReport::default(),
        application_info: ApplicationInfoReport::default(),
        structure: None,
    }
}

/// Read the KDH wrapper signature. A different signature is reported as an
/// unknown-page document instead of an error, so it can be diagnosed.
fn kdh_signature<S: RangedSource>(
    source: &mut S,
    limits: &Limits,
) -> Result<Option<Inspection>, String> {
    let mut signature = vec![0; source.size().min(HEADER_SIGNATURE.len() as u64) as usize];
    read_exact_at(source, 0, &mut signature, limits, &ProcessCancellation).map_err(text)?;
    if signature == HEADER_SIGNATURE {
        return Ok(None);
    }
    Ok(Some(Inspection {
        structure: Some(Structure::Kdh {
            signature,
            supported: false,
        }),
        ..pdf_inspection(InputFormat::Kdh, None)
    }))
}

fn inspect_source<S: RangedSource>(
    source: &mut S,
    limits: &Limits,
    pages: bool,
) -> Result<Inspection, String> {
    let Detection {
        format,
        header_offset,
        ..
    } = detect(source, limits)?;
    Ok(match format {
        InputFormat::Pdf => {
            pdf_inspection(format, Some(&index_pdf(source, header_offset, limits)?))
        }
        InputFormat::Kdh => {
            if pages && let Some(mismatch) = kdh_signature(source, limits)? {
                return Ok(mismatch);
            }
            let mut decoded =
                KdhPdfSource::open(source, limits, &ProcessCancellation).map_err(text)?;
            Inspection {
                structure: pages.then(|| Structure::Kdh {
                    signature: HEADER_SIGNATURE.to_vec(),
                    supported: true,
                }),
                ..pdf_inspection(format, Some(&index_pdf(&mut decoded, 0, limits)?))
            }
        }
        InputFormat::Caj => {
            let metadata =
                caj::parse_metadata(source, limits, &ProcessCancellation).map_err(text)?;
            Inspection {
                format,
                variant: None,
                page_count: Some(metadata.page_count),
                has_outline: Some(!metadata.bookmarks.is_empty()),
                bookmarks: Some(metadata.bookmarks),
                outline: OutlineReport::default(),
                application_info: ApplicationInfoReport::default(),
                structure: None,
            }
        }
        InputFormat::Hn | InputFormat::C8 => {
            let inspected = crate::hnc8::inspect(source, limits, pages)?;
            Inspection {
                format,
                variant: Some(inspected.header.variant.as_str()),
                page_count: Some(inspected.header.page_count),
                has_outline: inspected.bookmarks.as_ref().map(|items| !items.is_empty()),
                bookmarks: inspected.bookmarks,
                outline: inspected.outline,
                application_info: inspected.application_info,
                structure: inspected.structure,
            }
        }
        InputFormat::Nh | InputFormat::Teb => pdf_inspection(format, None),
    })
}

/// Inspect `input`; `pages` also reads the document-level structure.
pub fn inspect(input: &mut Input, limits: &Limits, pages: bool) -> Result<Inspection, CliError> {
    (|| inspect_source(&mut ranged(&mut input.file)?, limits, pages))()
        .map_err(|message| inspect_error(input, message))
}

fn inspect_error(input: &Input, message: String) -> CliError {
    CliError::runtime(format!("cannot inspect {}: {message}", input.name))
}

/// Stream the per-page report after the document report. Only HN/C8 has
/// per-page records; other formats report that none are available.
pub fn write_pages<W: Write>(
    input: &mut Input,
    limits: &Limits,
    info: &Inspection,
    pages: &mut crate::report::Pages<'_, W>,
) -> Result<(), CliError> {
    if !matches!(info.format, InputFormat::Hn | InputFormat::C8) {
        return pages.unavailable(info.format).map_err(stdout_error);
    }
    let page_count = info.page_count.expect("HN/C8 inspection has a page count");
    (|| {
        let mut source = ranged(&mut input.file).map_err(crate::hnc8::PagesError::Input)?;
        crate::hnc8::write_pages(&mut source, limits, page_count, pages)
    })()
    .map_err(|error| match error {
        crate::hnc8::PagesError::Output(error) => stdout_error(error),
        crate::hnc8::PagesError::Input(message) => inspect_error(input, message),
    })
}

fn read_error(name: &str) -> impl Fn(String) -> CliError + '_ {
    move |message| CliError::runtime(format!("cannot read {name}: {message}"))
}

/// Copy `pdf` to `writer` with the CAJ outline of `outline` appended.
/// Every input check completes before the first output byte is written.
pub fn add_bookmarks<W: Write>(
    outline: &mut Input,
    pdf: &mut Input,
    writer: W,
    limits: &Limits,
) -> Result<(), CliError> {
    let outline_file = &mut outline.file;
    let bookmarks = (|| {
        let mut source = ranged(outline_file)?;
        match detect(&mut source, limits)?.format {
            InputFormat::Caj => caj::parse_metadata(&mut source, limits, &ProcessCancellation)
                .map(|metadata| metadata.bookmarks)
                .map_err(text),
            other => Err(format!(
                "expected a CAJ outline source, found {}",
                format_name(other)
            )),
        }
    })()
    .map_err(read_error(&outline.name))?;
    if bookmarks.is_empty() {
        return Err(CliError::runtime(format!(
            "{} has no bookmarks to import",
            outline.name
        )));
    }
    let mut source = ranged(&mut pdf.file).map_err(read_error(&pdf.name))?;
    let index = (|| {
        let detection = detect(&mut source, limits)?;
        match detection.format {
            InputFormat::Pdf => index_pdf(&mut source, detection.header_offset, limits),
            other => Err(format!("expected a PDF, found {}", format_name(other))),
        }
    })()
    .map_err(read_error(&pdf.name))?;
    if index.has_outlines() {
        return Err(CliError::runtime(format!(
            "{} already has an outline; refusing to replace it",
            pdf.name
        )));
    }
    let mut sink = WriteSink::new(writer);
    (|| {
        let mut appender = PdfOutlineAppender::begin(
            &mut source,
            &mut sink,
            &index,
            limits,
            &ProcessCancellation,
        )?;
        for bookmark in bookmarks {
            appender.add_bookmark(bookmark)?;
        }
        appender.finish()
    })()
    .map(drop)
    .map_err(|error| {
        CliError::runtime(format!(
            "cannot add bookmarks from {} to {}: {error}",
            outline.name, pdf.name
        ))
    })
}
