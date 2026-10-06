// SPDX-License-Identifier: MIT

//! Bounded HN/C8 type-0 image pages to PDF with a caller-supplied QM table.

use super::{At, Budget, ErrorKind, Hnc8Error, Hnc8Reader, ImageRecord, Locate, Variant};
use crate::jbig1::{
    Type0Budget, Type0Decoder, Type0Error, Type0Info, Type0Report, read_type0_info,
};
use crate::pdf::{BilevelImageSpec, ImageObject, PageSpec, PdfDocument};
use crate::qm::{ArithmeticBudget, ArithmeticError, ContextBank, QmTable};
use crate::{
    Cancellation, ConversionReport, CountingSource, Error, Limits, MAX_BUDGET_COUNT, RangedSource,
    SequentialSink,
};
use std::{error, fmt};

/// The row model's fixed ten-bit context space.
const TYPE0_CONTEXTS: usize = 1024;
const POINTS_PER_INCH: f64 = 72.0;

/// What the legacy type-0 image-page adapter does with a multi-image row.
///
/// This API emits full-image pages. Source-page composition uses the separate
/// [`super::convert_source_pages_pdf`] entry point and its validated profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MultipleImages {
    /// Fail at the page row before reading any of its image descriptors.
    Reject,
    /// Emit each image, in record order, as its own PDF page.
    SeparatePages,
}

/// Conversion settings in addition to the shared `Limits`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Type0PdfOptions {
    /// Output scale: each image pixel is `72 / pixels_per_inch` points. This
    /// is a caller choice, not a measured HN/C8 field.
    pub pixels_per_inch: f64,
    pub multiple_images: MultipleImages,
    pub container: Budget,
    pub image: Type0Budget,
    /// Applied separately to each image's arithmetic stripe.
    pub arithmetic: ArithmeticBudget,
}

impl Default for Type0PdfOptions {
    fn default() -> Self {
        let image = Type0Budget::default();
        let max_symbols = image.max_pixels + u64::from(image.max_height);
        Self {
            pixels_per_inch: 300.0,
            multiple_images: MultipleImages::Reject,
            container: Budget::default(),
            image,
            arithmetic: ArithmeticBudget {
                max_symbols,
                max_work: max_symbols * 32 + 1024,
            },
        }
    }
}

/// Counters from a completed conversion.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Type0PdfReport {
    /// `input_bytes_read` counts every byte returned by the source.
    pub conversion: ConversionReport,
    pub source_pages: u32,
    pub images: u64,
}

/// One checked source image and the completed one-page PDF conversion.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Type0SelectedPdfReport {
    pub conversion: ConversionReport,
    pub source_variant: Variant,
    pub source_pages: u32,
    /// The actual descriptor and type-0 span validated by `Hnc8Reader`.
    pub image: ImageRecord,
}

/// One-based identity of a source type-0 image. Selection is diagnostic:
/// pages before `page_number` are intentionally not traversed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Type0ImageSelection {
    pub page_number: u32,
    pub image_number: u32,
}

#[derive(Debug)]
pub enum Type0PdfErrorKind {
    InvalidOptions(&'static str),
    InvalidSelection(&'static str),
    Container(Box<Hnc8Error>),
    Image(Box<Type0Error>),
    Pdf(Error),
    Contexts(Box<ArithmeticError>),
    /// A measured image record type with no codec assignment (1, 2, or 3).
    UnsupportedImageType(u32),
    /// A page declares this many images under [`MultipleImages::Reject`].
    MultipleImages(u32),
    /// A page declares no images, so there is nothing to draw.
    NoImages,
}

/// A conversion failure with the one-based source page and image, and the
/// absolute source byte offset, when known. Output already accepted by the
/// sink is a partial PDF and must be discarded.
#[derive(Debug)]
pub struct Type0PdfError {
    pub page: Option<u32>,
    pub image: Option<u32>,
    pub offset: Option<u64>,
    pub kind: Type0PdfErrorKind,
}

impl fmt::Display for Type0PdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HN/C8 type-0 PDF conversion")?;
        if let Some(page) = self.page {
            write!(f, ", page {page}")?;
        }
        if let Some(image) = self.image {
            write!(f, ", image {image}")?;
        }
        if let Some(offset) = self.offset {
            write!(f, ", source byte {offset}")?;
        }
        f.write_str(": ")?;
        match &self.kind {
            Type0PdfErrorKind::InvalidOptions(reason) => write!(f, "invalid options: {reason}"),
            Type0PdfErrorKind::InvalidSelection(reason) => {
                write!(f, "invalid selection: {reason}")
            }
            Type0PdfErrorKind::Container(error) => write!(f, "{error}"),
            Type0PdfErrorKind::Image(error) => write!(f, "{error}"),
            Type0PdfErrorKind::Pdf(error) => write!(f, "PDF output: {error}"),
            Type0PdfErrorKind::Contexts(error) => write!(f, "arithmetic contexts: {error}"),
            Type0PdfErrorKind::UnsupportedImageType(value) => {
                write!(f, "unsupported image record type {value}")
            }
            Type0PdfErrorKind::MultipleImages(count) => {
                write!(
                    f,
                    "page declares {count} images; this image-page API does not compose source pages"
                )
            }
            Type0PdfErrorKind::NoImages => f.write_str("page declares no images"),
        }
    }
}

impl error::Error for Type0PdfError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.kind {
            Type0PdfErrorKind::Container(error) => Some(error),
            Type0PdfErrorKind::Image(error) => Some(error),
            Type0PdfErrorKind::Pdf(error) => Some(error),
            Type0PdfErrorKind::Contexts(error) => Some(error),
            _ => None,
        }
    }
}

impl Locate for Type0PdfErrorKind {
    type Error = Type0PdfError;

    fn locate(self, at: At) -> Type0PdfError {
        Type0PdfError {
            page: at.page,
            image: at.image,
            offset: at.offset,
            kind: self,
        }
    }
}

/// Locate a JBIG decoder error at its own offset within `at`.
fn image_error(at: At) -> impl FnOnce(Type0Error) -> Type0PdfError {
    move |error| {
        at.with_offset(error.offset)
            .error(Type0PdfErrorKind::Image(Box::new(error)))
    }
}

fn container(error: Hnc8Error) -> Type0PdfError {
    Type0PdfError {
        page: error.page,
        image: error.image,
        offset: Some(error.offset),
        kind: Type0PdfErrorKind::Container(Box::new(error)),
    }
}

fn selected_container(error: Hnc8Error, selection: Type0ImageSelection) -> Type0PdfError {
    let mut converted = container(error);
    if matches!(
        &converted.kind,
        Type0PdfErrorKind::Container(inner)
            if matches!(inner.kind, ErrorKind::Malformed { field: "page number", .. })
    ) {
        converted.page = Some(selection.page_number);
        converted.image = Some(selection.image_number);
    }
    converted
}

fn page_spec(width: u32, height: u32, pixels_per_inch: f64) -> PageSpec {
    let scale = POINTS_PER_INCH / pixels_per_inch;
    PageSpec {
        width_points: f64::from(width) * scale,
        height_points: f64::from(height) * scale,
    }
}

fn validate_options(options: Type0PdfOptions) -> Result<(), Type0PdfError> {
    if !options.pixels_per_inch.is_finite() || options.pixels_per_inch <= 0.0 {
        return Err(At::NONE.error(Type0PdfErrorKind::InvalidOptions(
            "pixels per inch must be finite and positive",
        )));
    }
    // Refuse an arithmetic budget that every image would reject, before any
    // container read or PDF output.
    let counters = 1..=MAX_BUDGET_COUNT;
    if !counters.contains(&options.arithmetic.max_symbols)
        || !counters.contains(&options.arithmetic.max_work)
    {
        return Err(At::NONE.error(Type0PdfErrorKind::InvalidOptions(
            "arithmetic budget fields must be in 1..=MAX_BUDGET_COUNT",
        )));
    }
    Ok(())
}

struct Type0PageSettings<'a, C> {
    table: &'a QmTable,
    options: Type0PdfOptions,
    limits: &'a Limits,
    cancellation: &'a C,
}

/// Shared checked row decoding; image placement remains with the caller.
pub(super) struct Type0DecodeSettings<'a, C> {
    pub table: &'a QmTable,
    pub arithmetic: ArithmeticBudget,
    pub image: Type0Budget,
    pub limits: &'a Limits,
    pub cancellation: &'a C,
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn decode_type0_rows<S: RangedSource, R: SequentialSink, C: Cancellation>(
    source: &mut S,
    record: ImageRecord,
    contexts: &mut ContextBank,
    rows: &mut R,
    settings: &Type0DecodeSettings<'_, C>,
) -> Result<Type0Report, Type0PdfError> {
    let at = At {
        page: Some(record.page_number),
        image: Some(record.image_number),
        offset: Some(record.descriptor_offset),
        ..At::NONE
    };
    let span = record
        .type0_span()
        .ok_or_else(|| at.error(Type0PdfErrorKind::UnsupportedImageType(record.record_type)))?;
    let mut decoder = Type0Decoder::new(
        source,
        span,
        settings.table,
        contexts,
        rows,
        settings.limits,
        settings.cancellation,
        settings.arithmetic,
        settings.image,
    )
    .await
    .map_err(image_error(at))?;
    while decoder.decode_next_row().await.map_err(image_error(at))? {}
    decoder.finish().await.map_err(image_error(at))
}

/// The existing selected-image sample convention: visible width, top row first.
pub(super) async fn emit_type0_xobject<S: RangedSource, W: SequentialSink, C: Cancellation>(
    source: &mut S,
    document: &mut PdfDocument<'_, W, C>,
    record: ImageRecord,
    checked: Type0Info,
    contexts: &mut ContextBank,
    settings: &Type0DecodeSettings<'_, C>,
) -> Result<ImageObject, Type0PdfError> {
    let at = At {
        page: Some(record.page_number),
        image: Some(record.image_number),
        offset: Some(record.descriptor_offset),
        ..At::NONE
    };
    let mut rows = document
        .begin_bilevel_image(BilevelImageSpec {
            pixel_width: checked.width,
            pixel_height: checked.height,
            row_stride: checked.dib_stride,
        })
        .await
        .map_err(at.wrap(Type0PdfErrorKind::Pdf))?;
    decode_type0_rows(source, record, contexts, &mut rows, settings).await?;
    rows.finish().await.map_err(at.wrap(Type0PdfErrorKind::Pdf))
}

async fn emit_type0_image<S: RangedSource, W: SequentialSink, C: Cancellation>(
    reader: &mut Hnc8Reader<'_, S, C>,
    document: &mut PdfDocument<'_, W, C>,
    record: ImageRecord,
    output_page: u32,
    contexts: &mut ContextBank,
    settings: Type0PageSettings<'_, C>,
) -> Result<(), Type0PdfError> {
    let Type0PageSettings {
        table,
        options,
        limits,
        cancellation,
    } = settings;
    let at = At {
        page: Some(record.page_number),
        image: Some(record.image_number),
        offset: Some(record.descriptor_offset),
        ..At::NONE
    };
    let span = record
        .type0_span()
        .ok_or_else(|| at.error(Type0PdfErrorKind::UnsupportedImageType(record.record_type)))?;
    // Refuse a page beyond `max_pages` before its image is read, decoded, or written.
    limits
        .check_pages(output_page)
        .map_err(at.wrap(Type0PdfErrorKind::Pdf))?;
    let info = read_type0_info(
        reader.source_mut(),
        span,
        limits,
        cancellation,
        options.arithmetic,
        options.image,
    )
    .await
    .map_err(image_error(at))?;
    let object = emit_type0_xobject(
        reader.source_mut(),
        document,
        record,
        info,
        contexts,
        &Type0DecodeSettings {
            table,
            arithmetic: options.arithmetic,
            image: options.image,
            limits,
            cancellation,
        },
    )
    .await?;
    document
        .add_page(
            page_spec(info.width, info.height, options.pixels_per_inch),
            &[object],
        )
        .await
        .map_err(at.wrap(Type0PdfErrorKind::Pdf))?;
    Ok(())
}

/// Convert every page of a measured HN/C8 container whose images are all
/// type 0 into a PDF with one 1 bpp image per page.
///
/// Pages are read with [`Hnc8Reader`] from page 1 in index order. Each
/// type-0 image is decoded row by row with [`Type0Decoder`] and the caller's
/// validated `table`; its display-order rows are streamed into a
/// [`BilevelImageSpec`] XObject (first row at the top, DIB bit 1 black) with
/// DIB padding removed. The page is `width × height` pixels at
/// `options.pixels_per_inch`. A page without images, an image of record
/// type 1–3, or (by default) a page with several images is a typed error with
/// its page and image number; nothing is skipped or moved to another page.
/// Text spans, outline-like records, and unknown page fields are not used.
///
/// Retained memory is the container cursor's fixed buffers, three DIB-stride
/// rows, 1,024 contexts, the borrowed table, the QM core's 256-byte buffer,
/// and the PDF writer's per-object offsets and page index. None of these
/// depend on the source size or image byte length; the per-page part is
/// bounded by `Limits::max_pages` and `Limits::max_allocation_bytes`. The
/// source must be seekable or ranged; a forward-only input must be spooled
/// by its platform adapter first.
pub async fn convert_type0_pdf<S: RangedSource, W: SequentialSink, C: Cancellation>(
    source: &mut S,
    sink: &mut W,
    table: &QmTable,
    options: Type0PdfOptions,
    limits: &Limits,
    cancellation: &C,
) -> Result<Type0PdfReport, Type0PdfError> {
    validate_options(options)?;
    let mut contexts = ContextBank::new(TYPE0_CONTEXTS, limits)
        .map_err(|error| At::NONE.error(Type0PdfErrorKind::Contexts(Box::new(error))))?;
    let mut input_bytes_read = 0;
    let mut source = CountingSource::new(source, &mut input_bytes_read);
    let mut reader = Hnc8Reader::open(&mut source, limits, cancellation, options.container)
        .await
        .map_err(container)?;
    let mut document = PdfDocument::new(sink, limits, cancellation)
        .await
        .map_err(At::NONE.wrap(Type0PdfErrorKind::Pdf))?;
    let mut images = 0_u64;
    while let Some(page) = reader.next_page().await.map_err(container)? {
        let at_page = At {
            page: Some(page.page_number),
            image: None,
            offset: Some(page.row_offset + 8),
            ..At::NONE
        };
        match (page.image_count, options.multiple_images) {
            (0, _) => return Err(at_page.error(Type0PdfErrorKind::NoImages)),
            (1, _) | (_, MultipleImages::SeparatePages) => {}
            (count, MultipleImages::Reject) => {
                return Err(at_page.error(Type0PdfErrorKind::MultipleImages(count)));
            }
        }
        while let Some(record) = reader.next_image().await.map_err(container)? {
            emit_type0_image(
                &mut reader,
                &mut document,
                record,
                u32::try_from(images + 1).unwrap_or(u32::MAX),
                &mut contexts,
                Type0PageSettings {
                    table,
                    options,
                    limits,
                    cancellation,
                },
            )
            .await?;
            images += 1;
        }
    }
    let source_pages = reader.header().page_count;
    let mut conversion = document
        .finish()
        .await
        .map_err(At::NONE.wrap(Type0PdfErrorKind::Pdf))?;
    conversion.input_bytes_read = input_bytes_read;
    Ok(Type0PdfReport {
        conversion,
        source_pages,
        images,
    })
}

/// Convert one checked HN/C8 type-0 image record to a one-page PDF.
///
/// This diagnostic entry point deliberately skips source pages before
/// `selection.page_number`, including their descriptors and image codecs.
/// Within the selected page, preceding image descriptors are checked in
/// chain order; their payloads are not decoded. Neighboring image records
/// are not represented in the resulting PDF. Selection ignores
/// `options.multiple_images`; the full-document converter's
/// no-image, multi-image, and unsupported-type rejection policy is unchanged.
/// The same bounded decoder and PDF writer emit the selected image. Output
/// already accepted by the sink on failure must be discarded.
pub async fn convert_type0_image_pdf<S: RangedSource, W: SequentialSink, C: Cancellation>(
    source: &mut S,
    sink: &mut W,
    table: &QmTable,
    selection: Type0ImageSelection,
    options: Type0PdfOptions,
    limits: &Limits,
    cancellation: &C,
) -> Result<Type0SelectedPdfReport, Type0PdfError> {
    validate_options(options)?;
    if selection.page_number == 0 || selection.image_number == 0 {
        return Err(At::NONE.error(Type0PdfErrorKind::InvalidSelection(
            "page and image numbers must be one-based",
        )));
    }
    let mut contexts = ContextBank::new(TYPE0_CONTEXTS, limits)
        .map_err(|error| At::NONE.error(Type0PdfErrorKind::Contexts(Box::new(error))))?;
    let mut input_bytes_read = 0;
    let mut source = CountingSource::new(source, &mut input_bytes_read);
    let mut reader = Hnc8Reader::probe_at_page(
        &mut source,
        limits,
        cancellation,
        options.container,
        selection.page_number,
    )
    .await
    .map_err(|error| selected_container(error, selection))?;
    let header = reader.header();
    let page = reader
        .next_page()
        .await
        .map_err(container)?
        .expect("probe validated the selected page");
    if selection.image_number > page.image_count {
        return Err(At {
            page: Some(page.page_number),
            image: Some(selection.image_number),
            offset: Some(page.row_offset + 8),
            ..At::NONE
        }
        .error(Type0PdfErrorKind::InvalidSelection(
            "image number exceeds page image count",
        )));
    }
    // Every descriptor up to the selected one is checked. Earlier image
    // types can be unsupported; only the chosen record is decoded.
    let mut record = reader
        .next_image()
        .await
        .map_err(container)?
        .expect("selected image count was checked");
    for _ in 1..selection.image_number {
        record = reader
            .next_image()
            .await
            .map_err(container)?
            .expect("selected image count was checked");
    }
    debug_assert_eq!(record.page_number, selection.page_number);
    debug_assert_eq!(record.image_number, selection.image_number);
    let at = At {
        page: Some(record.page_number),
        image: Some(record.image_number),
        offset: Some(record.descriptor_offset),
        ..At::NONE
    };
    let mut document = PdfDocument::new(sink, limits, cancellation)
        .await
        .map_err(at.wrap(Type0PdfErrorKind::Pdf))?;
    emit_type0_image(
        &mut reader,
        &mut document,
        record,
        1,
        &mut contexts,
        Type0PageSettings {
            table,
            options,
            limits,
            cancellation,
        },
    )
    .await?;
    let mut conversion = document
        .finish()
        .await
        .map_err(at.wrap(Type0PdfErrorKind::Pdf))?;
    conversion.input_bytes_read = input_bytes_read;
    Ok(Type0SelectedPdfReport {
        conversion,
        source_variant: header.variant,
        source_pages: header.page_count,
        image: record,
    })
}
