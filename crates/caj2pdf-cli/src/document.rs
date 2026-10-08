// SPDX-License-Identifier: MIT

//! The CLI's calls into the core facade, and the wording of its refusals.

use crate::CliError;
use crate::files::{Input, stdout_error};
use crate::progress::Progress;
use crate::report::Pages;
use crate::signals::ProcessCancellation;
use caj2pdf_core::{
    ConversionOptions, ConversionReport, DocumentInfo, Error, FONTS_REQUIRE_HNC8, InputFormat,
    InspectOptions, Limits, PageVisitor, RangedSource,
    hnc8::{ImageRecord, PageRecord, TextStructure},
    native::SeekableSource,
    pdf::PdfOutlineAppender,
};
use std::fs::File;
use std::io::{self, Write};

/// Why a recognized format is never converted, when that is known.
///
/// TEB is a CNKI DRM container whose document entries are encrypted
/// (rwv/caj2pdf-samples research notes); this project does not decrypt it.
pub fn unsupported_reason(format: InputFormat) -> Option<&'static str> {
    match format {
        InputFormat::Teb => Some("drm-encrypted"),
        InputFormat::Caa => Some("target-descriptor"),
        _ => None,
    }
}

pub(crate) fn unsupported(format: InputFormat) -> String {
    match format {
        InputFormat::Teb => "TEB input is a DRM-encrypted CNKI container; \
                    its document content is encrypted and cannot be converted"
            .to_owned(),
        InputFormat::Caa => "CAA input is a target descriptor, not a document; \
                            obtain the referenced document and convert that file"
            .to_owned(),
        _ => format!(
            "{name} input is recognized, but {name} conversion is not supported",
            name = format.name()
        ),
    }
}

fn ranged(file: &mut File) -> Result<SeekableSource<&mut File>, String> {
    SeekableSource::new(file).map_err(|error| error.to_string())
}

/// The CLI wording of a core error. An empty or unrecognized input is
/// worded here; `refusal` words a refusal of the reported format, if
/// `error` is one; any other error keeps its own message.
fn describe(
    error: Error,
    progress: &Progress<'_>,
    empty: bool,
    refusal: impl FnOnce(InputFormat, &Error) -> Option<String>,
) -> String {
    match progress.format {
        Some(None) if empty => "input is empty".to_owned(),
        Some(None) => "unrecognized input format".to_owned(),
        Some(Some(format)) => refusal(format, &error).unwrap_or_else(|| error.to_string()),
        None => error.to_string(),
    }
}

/// Convert one input to PDF bytes written to `writer`. The report carries
/// the skipped HN/C8 outline entries, the C8 application-info status and the
/// blank-substituted pages for the warnings.
pub fn convert<W: Write>(
    input: &mut Input,
    mut writer: W,
    limits: &Limits,
    resources: &mut crate::hnc8::Resources,
    options: ConversionOptions<'_>,
    terminal: Option<&mut dyn Write>,
) -> Result<ConversionReport, CliError> {
    let result = (|| {
        let mut source = ranged(&mut input.file)?;
        let fonts = resources.fonts().map_err(|error| error.to_string())?;
        let mut progress = Progress::new(terminal);
        let result = caj2pdf_core::convert(
            &mut source,
            &mut writer,
            ConversionOptions { fonts, ..options },
            limits,
            &mut progress,
        );
        let empty = source.size() == 0;
        let result = result.map_err(|error| {
            describe(error, &progress, empty, |format, error| {
                if error.reason == FONTS_REQUIRE_HNC8 {
                    Some("explicit native fonts require a C8 or HN-B document".to_owned())
                } else {
                    (!format.is_convertible()).then(|| unsupported(format))
                }
            })
        });
        progress.finish();
        result
    })();
    result.map_err(|message| CliError::runtime(format!("cannot convert {}: {message}", input.name)))
}

/// Whether `input` is an HN/C8 document that converts with native text
/// composition and therefore needs fonts. Other and unrecognized formats
/// return `false`; conversion then reports its own errors.
pub fn needs_fonts(input: &mut Input, limits: &Limits) -> Result<bool, CliError> {
    (|| {
        let mut source = ranged(&mut input.file)?;
        caj2pdf_core::needs_fonts(&mut source, limits, &mut Progress::new(None))
            .map_err(|error| error.to_string())
    })()
    .map_err(read_error(&input.name))
}

/// Inspect `input`, listing its outline; `pages` also reads the
/// document-level structure.
pub fn inspect(input: &mut Input, limits: &Limits, pages: bool) -> Result<DocumentInfo, CliError> {
    (|| {
        let mut source = ranged(&mut input.file)?;
        let mut progress = Progress::new(None);
        let options = InspectOptions {
            format: None,
            bookmarks: true,
            structure: pages,
        };
        let result = caj2pdf_core::inspect(&mut source, &options, limits, &mut progress);
        let empty = source.size() == 0;
        result.map_err(|error| describe(error, &progress, empty, |_, _| None))
    })()
    .map_err(|message| inspect_error(input, message))
}

fn inspect_error(input: &Input, message: String) -> CliError {
    CliError::runtime(format!("cannot inspect {}: {message}", input.name))
}

/// The per-page report as a core [`PageVisitor`]. A failed report write is
/// kept here and ends the traversal.
struct PageReport<'p, 'w, W: Write> {
    pages: &'p mut Pages<'w, W>,
    failed: Option<io::Error>,
}

impl<W: Write> PageReport<'_, '_, W> {
    fn output(&mut self, result: io::Result<()>) -> caj2pdf_core::Result<()> {
        result.map_err(|error| {
            self.failed = Some(error);
            Error::invalid("page report output failed")
        })
    }
}

impl<W: Write> PageVisitor for PageReport<'_, '_, W> {
    fn begin(&mut self) -> caj2pdf_core::Result<()> {
        let result = self.pages.begin();
        self.output(result)
    }

    fn page(&mut self, number: u32, row: Option<&PageRecord>) -> caj2pdf_core::Result<()> {
        let result = self.pages.page(number, row);
        self.output(result)
    }

    fn image(&mut self, image: &ImageRecord) -> caj2pdf_core::Result<()> {
        let result = self.pages.image(image);
        self.output(result)
    }

    fn end_page(
        &mut self,
        text: Option<&TextStructure>,
        text_error: Option<&Error>,
        error: Option<&Error>,
    ) -> caj2pdf_core::Result<()> {
        let text_error = text_error.map(Error::to_string);
        let error = error.map(Error::to_string);
        let result = self
            .pages
            .end_page(text, text_error.as_deref(), error.as_deref());
        self.output(result)
    }

    fn finish(&mut self) -> caj2pdf_core::Result<()> {
        let result = self.pages.finish();
        self.output(result)
    }
}

/// Stream the per-page report after the document report. Only HN/C8 has
/// per-page records; other formats report that none are available.
pub fn write_pages<W: Write>(
    input: &mut Input,
    limits: &Limits,
    info: &DocumentInfo,
    pages: &mut Pages<'_, W>,
) -> Result<(), CliError> {
    let mut report = PageReport {
        pages,
        failed: None,
    };
    let result = (|| {
        let mut source = ranged(&mut input.file)?;
        let mut progress = Progress::new(None);
        caj2pdf_core::inspect_pages(&mut source, info, limits, &mut progress, &mut report)
            .map_err(|error| error.to_string())
    })();
    if let Some(error) = report.failed {
        return Err(stdout_error(error));
    }
    match result {
        Ok(true) => Ok(()),
        Ok(false) => report.pages.unavailable(info.format).map_err(stdout_error),
        Err(message) => Err(inspect_error(input, message)),
    }
}

fn read_error(name: &str) -> impl Fn(String) -> CliError + '_ {
    move |message| CliError::runtime(format!("cannot read {name}: {message}"))
}

/// `expected` refuses any other reported format.
fn expected(
    name: &'static str,
    format: InputFormat,
) -> impl FnOnce(InputFormat, &Error) -> Option<String> {
    move |found, _| (found != format).then(|| format!("expected {name}, found {}", found.name()))
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
        let mut progress = Progress::new(None);
        let result = caj2pdf_core::read_outline(&mut source, limits, &mut progress);
        let empty = source.size() == 0;
        result.map_err(|error| {
            describe(
                error,
                &progress,
                empty,
                expected("a CAJ outline source", InputFormat::Caj),
            )
        })
    })()
    .map_err(read_error(&outline.name))?;
    if bookmarks.is_empty() {
        return Err(CliError::runtime(format!(
            "{} has no bookmarks to import",
            outline.name
        )));
    }
    let mut source = ranged(&mut pdf.file).map_err(read_error(&pdf.name))?;
    let mut progress = Progress::new(None);
    let index = caj2pdf_core::index_pdf(&mut source, limits, &mut progress).map_err(|error| {
        let empty = source.size() == 0;
        read_error(&pdf.name)(describe(
            error,
            &progress,
            empty,
            expected("a PDF", InputFormat::Pdf),
        ))
    })?;
    if index.has_outlines() {
        return Err(CliError::runtime(format!(
            "{} already has an outline; refusing to replace it",
            pdf.name
        )));
    }
    let mut sink = writer;
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
