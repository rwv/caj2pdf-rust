// SPDX-License-Identifier: MIT

//! Format detection and dispatch to the core conversion, inspection, and
//! outline-import operations.

use crate::CliError;
use crate::files::Input;
use caj2pdf_core::{
    Bookmark, ConversionOptions, Error, InputFormat, Limits, NeverCancel, RangedSource, caj,
    hnc8::{Budget, Hnc8Reader},
    kdh::{KdhPdfSource, convert_kdh},
    native::{SeekableSource, WriteSink},
    pdf::{PdfIndex, PdfOutlineAppender, PdfRange, copy_pdf},
    read_exact_at,
};
use std::fs::File;
use std::future::Future;
use std::io::Write;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

/// Drive a core future. The native adapters never suspend, so the first
/// poll completes; the loop only guards against a spurious `Pending`.
pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = pin!(future);
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
            return value;
        }
    }
}

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
        InputFormat::Pdf | InputFormat::Caj | InputFormat::Kdh
    )
}

/// Classify input by its leading signature, never by file name: some
/// observed `.caj` files are plain PDFs. Signatures are those recorded in
/// `tests/fixtures/README.md`, `docs/hnc8-container.md`, and `kdh.rs`.
pub fn detect_signature(header: &[u8]) -> Option<InputFormat> {
    const SIGNATURES: [(&[u8], InputFormat); 6] = [
        (b"%PDF-", InputFormat::Pdf),
        (b"CAJ", InputFormat::Caj),
        (b"KDH", InputFormat::Kdh),
        (b"HN", InputFormat::Hn),
        (b"\xc8\0\0\0", InputFormat::C8),
        (b"TEB", InputFormat::Teb),
    ];
    SIGNATURES
        .iter()
        .find(|(signature, _)| header.starts_with(signature))
        .map(|(_, format)| *format)
}

async fn detect<S: RangedSource>(source: &mut S, limits: &Limits) -> Result<InputFormat, String> {
    let mut header = [0; 8];
    let length = source.size().min(header.len() as u64) as usize;
    read_exact_at(source, 0, &mut header[..length], limits, &NeverCancel)
        .await
        .map_err(|error| error.to_string())?;
    if length == 0 {
        return Err("input is empty".to_owned());
    }
    detect_signature(&header[..length]).ok_or_else(|| "unrecognized input format".to_owned())
}

fn unsupported(format: InputFormat) -> String {
    match format {
        InputFormat::Teb => {
            "TEB input is recognized, but TEB conversion is not supported".to_owned()
        }
        other => format!(
            "{} input is recognized, but HN/C8 image decoding is not implemented yet",
            format_name(other)
        ),
    }
}

fn ranged(file: &mut File) -> Result<SeekableSource<&mut File>, String> {
    SeekableSource::new(file).map_err(|error| error.to_string())
}

/// Convert one input to PDF bytes written to `writer`.
pub fn convert<W: Write>(input: &mut Input, writer: W, limits: &Limits) -> Result<(), CliError> {
    let result = block_on(async {
        let mut source = ranged(&mut input.file)?;
        let mut sink = WriteSink::new(writer);
        let text = |error: Error| error.to_string();
        match detect(&mut source, limits).await? {
            InputFormat::Pdf => copy_pdf(&mut source, &mut sink, limits, &NeverCancel)
                .await
                .map_err(text),
            InputFormat::Caj => caj::convert_caj(
                &mut source,
                &mut sink,
                ConversionOptions::default(),
                limits,
                &NeverCancel,
            )
            .await
            .map_err(text),
            InputFormat::Kdh => convert_kdh(&mut source, &mut sink, limits, &NeverCancel)
                .await
                .map_err(text),
            other => Err(unsupported(other)),
        }
    });
    result
        .map(drop)
        .map_err(|message| CliError::runtime(format!("cannot convert {}: {message}", input.name)))
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
}

async fn index_pdf<S: RangedSource>(source: &mut S, limits: &Limits) -> Result<PdfIndex, String> {
    let range = PdfRange {
        offset: 0,
        length: source.size(),
    };
    PdfIndex::open(source, range, limits, &NeverCancel)
        .await
        .map_err(|error| error.to_string())
}

fn pdf_inspection(format: InputFormat, index: &PdfIndex) -> Inspection {
    Inspection {
        format,
        variant: None,
        page_count: Some(index.pages().len() as u32),
        has_outline: Some(index.has_outlines()),
        bookmarks: None,
    }
}

async fn inspect_source<S: RangedSource>(
    source: &mut S,
    limits: &Limits,
) -> Result<Inspection, String> {
    let format = detect(source, limits).await?;
    let text = |error: Error| error.to_string();
    Ok(match format {
        InputFormat::Pdf => pdf_inspection(format, &index_pdf(source, limits).await?),
        InputFormat::Kdh => {
            let mut decoded = KdhPdfSource::open(source, limits, &NeverCancel)
                .await
                .map_err(text)?;
            pdf_inspection(format, &index_pdf(&mut decoded, limits).await?)
        }
        InputFormat::Caj => {
            let metadata = caj::parse_metadata(source, limits, &NeverCancel)
                .await
                .map_err(text)?;
            Inspection {
                format,
                variant: None,
                page_count: Some(metadata.page_count),
                has_outline: Some(!metadata.bookmarks.is_empty()),
                bookmarks: Some(metadata.bookmarks),
            }
        }
        InputFormat::Hn | InputFormat::C8 => {
            let reader = Hnc8Reader::open(source, limits, &NeverCancel, Budget::default())
                .await
                .map_err(|error| error.to_string())?;
            let header = reader.header();
            Inspection {
                format,
                variant: Some(header.variant.as_str()),
                page_count: Some(header.page_count),
                has_outline: None,
                bookmarks: None,
            }
        }
        InputFormat::Nh | InputFormat::Teb => Inspection {
            format,
            variant: None,
            page_count: None,
            has_outline: None,
            bookmarks: None,
        },
    })
}

pub fn inspect(input: &mut Input, limits: &Limits) -> Result<Inspection, CliError> {
    block_on(async { inspect_source(&mut ranged(&mut input.file)?, limits).await })
        .map_err(|message| CliError::runtime(format!("cannot inspect {}: {message}", input.name)))
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
    let bookmarks = block_on(async {
        let mut source = ranged(outline_file)?;
        match detect(&mut source, limits).await? {
            InputFormat::Caj => caj::parse_metadata(&mut source, limits, &NeverCancel)
                .await
                .map(|metadata| metadata.bookmarks)
                .map_err(|error| error.to_string()),
            other => Err(format!(
                "expected a CAJ outline source, found {}",
                format_name(other)
            )),
        }
    })
    .map_err(read_error(&outline.name))?;
    if bookmarks.is_empty() {
        return Err(CliError::runtime(format!(
            "{} has no bookmarks to import",
            outline.name
        )));
    }
    let mut source = ranged(&mut pdf.file).map_err(read_error(&pdf.name))?;
    let index = block_on(async {
        match detect(&mut source, limits).await? {
            InputFormat::Pdf => index_pdf(&mut source, limits).await,
            other => Err(format!("expected a PDF, found {}", format_name(other))),
        }
    })
    .map_err(read_error(&pdf.name))?;
    if index.has_outlines() {
        return Err(CliError::runtime(format!(
            "{} already has an outline; refusing to replace it",
            pdf.name
        )));
    }
    let mut sink = WriteSink::new(writer);
    block_on(async {
        let mut appender =
            PdfOutlineAppender::begin(&mut source, &mut sink, &index, limits, &NeverCancel).await?;
        for bookmark in bookmarks {
            appender.add_bookmark(bookmark).await?;
        }
        appender.finish().await
    })
    .map(drop)
    .map_err(|error| {
        CliError::runtime(format!(
            "cannot add bookmarks from {} to {}: {error}",
            outline.name, pdf.name
        ))
    })
}
