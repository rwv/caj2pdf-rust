// SPDX-License-Identifier: MIT

//! Bounded HN/C8 type-0 image pages to PDF with a caller-supplied QM table.

use super::{Budget, Hnc8Error, Hnc8Reader};
use crate::jbig1::{Type0Budget, Type0Decoder, Type0Error, read_type0_info};
use crate::pdf::{BilevelImageSpec, PageSpec, PdfDocument};
use crate::qm::{ArithmeticBudget, ArithmeticError, ContextBank, QmTable};
use crate::{Cancellation, ConversionReport, Error, Limits, RangedSource, SequentialSink};
use std::{error, fmt};

/// The row model's fixed ten-bit context space.
const TYPE0_CONTEXTS: usize = 1024;
const POINTS_PER_INCH: f64 = 72.0;

/// What to do with a source page that declares more than one image.
///
/// No placement geometry for additional images has been measured, so the
/// converter never composes them onto one page.
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
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Type0PdfReport {
    /// `input_bytes_read` counts every byte returned by the source.
    pub conversion: ConversionReport,
    pub source_pages: u32,
    pub images: u64,
}

#[derive(Debug)]
pub enum Type0PdfErrorKind {
    InvalidOptions(&'static str),
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
            Type0PdfErrorKind::Container(error) => write!(f, "{error}"),
            Type0PdfErrorKind::Image(error) => write!(f, "{error}"),
            Type0PdfErrorKind::Pdf(error) => write!(f, "PDF output: {error}"),
            Type0PdfErrorKind::Contexts(error) => write!(f, "arithmetic contexts: {error}"),
            Type0PdfErrorKind::UnsupportedImageType(value) => {
                write!(f, "unsupported image record type {value}")
            }
            Type0PdfErrorKind::MultipleImages(count) => {
                write!(f, "page declares {count} images; placement is not measured")
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

#[derive(Clone, Copy)]
struct At {
    page: Option<u32>,
    image: Option<u32>,
    offset: Option<u64>,
}

impl At {
    const NONE: Self = Self {
        page: None,
        image: None,
        offset: None,
    };

    fn error(self, kind: Type0PdfErrorKind) -> Type0PdfError {
        Type0PdfError {
            page: self.page,
            image: self.image,
            offset: self.offset,
            kind,
        }
    }

    fn pdf(self) -> impl FnOnce(Error) -> Type0PdfError {
        move |error| self.error(Type0PdfErrorKind::Pdf(error))
    }

    fn image(self) -> impl FnOnce(Type0Error) -> Type0PdfError {
        move |error| {
            At {
                offset: Some(error.offset),
                ..self
            }
            .error(Type0PdfErrorKind::Image(Box::new(error)))
        }
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

/// Counts bytes returned by every source read for the report.
struct CountingSource<'a, S> {
    inner: &'a mut S,
    read: u64,
}

impl<S: RangedSource> RangedSource for CountingSource<'_, S> {
    fn size(&self) -> u64 {
        self.inner.size()
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> crate::Result<usize> {
        let count = self.inner.read_at(offset, destination).await?;
        // Overreports are rejected by the callers' read helpers; saturate so
        // this counter cannot mask them with an overflow panic.
        self.read = self.read.saturating_add(count as u64);
        Ok(count)
    }
}

fn page_spec(width: u32, height: u32, pixels_per_inch: f64) -> PageSpec {
    let scale = POINTS_PER_INCH / pixels_per_inch;
    PageSpec {
        width_points: f64::from(width) * scale,
        height_points: f64::from(height) * scale,
    }
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
    if !options.pixels_per_inch.is_finite() || options.pixels_per_inch <= 0.0 {
        return Err(At::NONE.error(Type0PdfErrorKind::InvalidOptions(
            "pixels per inch must be finite and positive",
        )));
    }
    let mut contexts = ContextBank::new(TYPE0_CONTEXTS, limits)
        .map_err(|error| At::NONE.error(Type0PdfErrorKind::Contexts(Box::new(error))))?;
    let mut source = CountingSource {
        inner: source,
        read: 0,
    };
    let mut reader = Hnc8Reader::open(&mut source, limits, cancellation, options.container)
        .await
        .map_err(container)?;
    let mut document = PdfDocument::new(sink, limits, cancellation)
        .await
        .map_err(At::NONE.pdf())?;
    let mut images = 0_u64;
    while let Some(page) = reader.next_page().await.map_err(container)? {
        let at_page = At {
            page: Some(page.page_number),
            image: None,
            offset: Some(page.row_offset + 8),
        };
        match (page.image_count, options.multiple_images) {
            (0, _) => return Err(at_page.error(Type0PdfErrorKind::NoImages)),
            (1, _) | (_, MultipleImages::SeparatePages) => {}
            (count, MultipleImages::Reject) => {
                return Err(at_page.error(Type0PdfErrorKind::MultipleImages(count)));
            }
        }
        while let Some(record) = reader.next_image().await.map_err(container)? {
            let at = At {
                page: Some(page.page_number),
                image: Some(record.image_number),
                offset: Some(record.descriptor_offset),
            };
            let span = record.type0_span().ok_or_else(|| {
                at.error(Type0PdfErrorKind::UnsupportedImageType(record.record_type))
            })?;
            let info = read_type0_info(
                reader.source_mut(),
                span,
                limits,
                cancellation,
                options.arithmetic,
                options.image,
            )
            .await
            .map_err(at.image())?;
            let mut rows = document
                .begin_bilevel_image(BilevelImageSpec {
                    pixel_width: info.width,
                    pixel_height: info.height,
                    row_stride: info.dib_stride,
                })
                .await
                .map_err(at.pdf())?;
            let mut decoder = Type0Decoder::new(
                reader.source_mut(),
                span,
                table,
                &mut contexts,
                &mut rows,
                limits,
                cancellation,
                options.arithmetic,
                options.image,
            )
            .await
            .map_err(at.image())?;
            while decoder.decode_next_row().await.map_err(at.image())? {}
            decoder.finish().await.map_err(at.image())?;
            let object = rows.finish().await.map_err(at.pdf())?;
            document
                .add_page(
                    page_spec(info.width, info.height, options.pixels_per_inch),
                    &[object],
                )
                .await
                .map_err(at.pdf())?;
            images += 1;
        }
    }
    let source_pages = reader.header().page_count;
    let mut conversion = document.finish().await.map_err(At::NONE.pdf())?;
    conversion.input_bytes_read = source.read;
    Ok(Type0PdfReport {
        conversion,
        source_pages,
        images,
    })
}
