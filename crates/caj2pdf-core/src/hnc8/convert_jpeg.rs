// SPDX-License-Identifier: MIT

//! One checked HN/C8 type-1/type-2 JPEG as a bounded, one-page PDF diagnostic.

use super::{
    At, Budget, Hnc8Error, Hnc8Reader, ImageRecord, JpegBudget, JpegColor, JpegInfo, Locate,
    Variant, read_type2_jpeg_info,
};
use crate::pdf::{ImageEncoding, ImageObject, ImageSpec, PageSpec, PdfDocument};
use crate::{
    Cancellation, ConversionReport, CountingSource, Error, Limits, RangedSource, SequentialSink,
};
use std::{error, fmt};

const POINTS_PER_INCH: f64 = 72.0;

/// One-based identity of a source type-1/type-2 image. Earlier pages are skipped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Type2ImageSelection {
    pub page_number: u32,
    pub image_number: u32,
}

/// Explicit container, JPEG, and PDF choices for one selected image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Type2PdfOptions {
    /// Each JPEG pixel occupies `72 / pixels_per_inch` PDF points.
    /// This is a caller choice, not a measured HN/C8 page field.
    pub pixels_per_inch: f64,
    pub container: Budget,
    pub jpeg: JpegBudget,
}

impl Default for Type2PdfOptions {
    fn default() -> Self {
        Self {
            pixels_per_inch: 300.0,
            container: Budget::default(),
            jpeg: JpegBudget::default(),
        }
    }
}

/// Checked source identity and the finished one-page PDF report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Type2SelectedPdfReport {
    /// Includes container, JPEG preflight, and PDF-copy source bytes.
    pub conversion: ConversionReport,
    pub source_variant: Variant,
    pub source_pages: u32,
    pub image: ImageRecord,
    /// Marker/profile facts, not a claim that JPEG entropy was decoded.
    pub jpeg: JpegInfo,
}

#[derive(Debug)]
pub enum Type2PdfErrorKind {
    InvalidOptions(&'static str),
    InvalidSelection(&'static str),
    Container(Box<Hnc8Error>),
    Jpeg(Box<Hnc8Error>),
    Pdf(Error),
    UnsupportedImageType(u32),
}

/// A failure located at a one-based source page/image and absolute source
/// byte when known. The caller must discard any partial sink output.
#[derive(Debug)]
pub struct Type2PdfError {
    pub page: Option<u32>,
    pub image: Option<u32>,
    pub offset: Option<u64>,
    pub kind: Type2PdfErrorKind,
}

impl fmt::Display for Type2PdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HN/C8 type-1/type-2 JPEG PDF conversion")?;
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
            Type2PdfErrorKind::InvalidOptions(reason) => write!(f, "invalid options: {reason}"),
            Type2PdfErrorKind::InvalidSelection(reason) => write!(f, "invalid selection: {reason}"),
            Type2PdfErrorKind::Container(error) | Type2PdfErrorKind::Jpeg(error) => {
                write!(f, "{error}")
            }
            Type2PdfErrorKind::Pdf(error) => write!(f, "PDF output: {error}"),
            Type2PdfErrorKind::UnsupportedImageType(kind) => {
                write!(f, "unsupported image record type {kind}")
            }
        }
    }
}

impl error::Error for Type2PdfError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.kind {
            Type2PdfErrorKind::Container(error) | Type2PdfErrorKind::Jpeg(error) => Some(error),
            Type2PdfErrorKind::Pdf(error) => Some(error),
            _ => None,
        }
    }
}

impl Locate for Type2PdfErrorKind {
    type Error = Type2PdfError;

    fn locate(self, at: At) -> Type2PdfError {
        Type2PdfError {
            page: at.page,
            image: at.image,
            offset: at.offset,
            kind: self,
        }
    }
}

fn container(error: Hnc8Error) -> Type2PdfError {
    Type2PdfError {
        page: error.page,
        image: error.image,
        offset: Some(error.offset),
        kind: Type2PdfErrorKind::Container(Box::new(error)),
    }
}

fn selected_container(error: Hnc8Error, selection: Type2ImageSelection) -> Type2PdfError {
    let mut converted = container(error);
    // Preserve the exact descriptor identity when the reader reached one;
    // attach the requested identity to header/page errors that lack it.
    converted.page.get_or_insert(selection.page_number);
    converted.image.get_or_insert(selection.image_number);
    converted
}

fn page_spec(info: JpegInfo, pixels_per_inch: f64) -> PageSpec {
    let scale = POINTS_PER_INCH / pixels_per_inch;
    PageSpec {
        width_points: f64::from(info.width) * scale,
        height_points: f64::from(info.height) * scale,
    }
}

fn image_spec(info: JpegInfo) -> ImageSpec {
    let encoding = match info.color {
        JpegColor::Gray => ImageEncoding::JpegGray8,
        JpegColor::Ycbcr => ImageEncoding::JpegRgb8,
    };
    ImageSpec {
        pixel_width: u32::from(info.width),
        pixel_height: u32::from(info.height),
        encoding,
    }
}

/// The same descriptor and geometry from one complete JPEG traversal.
/// Private fields keep the checked geometry paired with its span.
#[derive(Clone, Copy, Debug)]
pub(super) struct CheckedType2 {
    record: ImageRecord,
    info: JpegInfo,
}

impl CheckedType2 {
    pub(super) fn record(self) -> ImageRecord {
        self.record
    }

    pub(super) fn info(self) -> JpegInfo {
        self.info
    }
}

pub(super) async fn preflight_type2<S: RangedSource, C: Cancellation>(
    source: &mut S,
    image: ImageRecord,
    limits: &Limits,
    cancellation: &C,
    budget: JpegBudget,
) -> Result<CheckedType2, Type2PdfError> {
    let at = At {
        page: Some(image.page_number),
        image: Some(image.image_number),
        offset: Some(image.payload.offset),
        ..At::NONE
    };
    if !matches!(image.record_type, 1 | 2) {
        return Err(at
            .with_offset(image.descriptor_offset)
            .error(Type2PdfErrorKind::UnsupportedImageType(image.record_type)));
    }
    let info = read_type2_jpeg_info(source, image, limits, cancellation, budget)
        .await
        .map_err(|error| {
            at.with_offset(error.offset)
                .error(Type2PdfErrorKind::Jpeg(Box::new(error)))
        })?;
    Ok(CheckedType2 {
        record: image,
        info,
    })
}

pub(super) async fn emit_type2_xobject<S: RangedSource, W: SequentialSink, C: Cancellation>(
    source: &mut S,
    document: &mut PdfDocument<'_, W, C>,
    checked: CheckedType2,
) -> Result<ImageObject, Type2PdfError> {
    let image = checked.record();
    let at = At {
        page: Some(image.page_number),
        image: Some(image.image_number),
        offset: Some(image.payload.offset),
        ..At::NONE
    };
    document
        .add_image(
            source,
            image.payload.offset,
            image.payload.length,
            image_spec(checked.info()),
        )
        .await
        .map_err(at.wrap(Type2PdfErrorKind::Pdf))
}

/// Stream one checked HN/C8 type-1 or type-2 JPEG record into a one-page PDF.
///
/// Source pages before the selected page are intentionally skipped. On the
/// selected page, descriptors through the chosen image are validated in chain
/// order, but other payloads are not decoded or placed. The selected JPEG is
/// traversed as a single-scan baseline/JFIF marker profile before output.
/// `/DeviceGray` is used for grayscale and `/DeviceRGB` with explicit
/// `/ColorTransform 1` for three-component JFIF YCbCr. A marker profile alone
/// does not establish decoded-pixel validity; PDF render conformance is a
/// separate test. JPEG bytes are not collected: the marker walk and the PDF
/// copy each read the selected range once, in bounded chunks. Like every
/// `RangedSource`, the source must not change during the call. The PDF sink
/// is forward-only and must be discarded by the caller on any error.
pub async fn convert_type2_image_pdf<S: RangedSource, W: SequentialSink, C: Cancellation>(
    source: &mut S,
    sink: &mut W,
    selection: Type2ImageSelection,
    options: Type2PdfOptions,
    limits: &Limits,
    cancellation: &C,
) -> Result<Type2SelectedPdfReport, Type2PdfError> {
    if !options.pixels_per_inch.is_finite() || options.pixels_per_inch <= 0.0 {
        return Err(At::NONE.error(Type2PdfErrorKind::InvalidOptions(
            "pixels per inch must be finite and positive",
        )));
    }
    if selection.page_number == 0 || selection.image_number == 0 {
        return Err(At::NONE.error(Type2PdfErrorKind::InvalidSelection(
            "page and image numbers must be one-based",
        )));
    }
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
        .map_err(|error| selected_container(error, selection))?
        .expect("probe validated the selected page");
    if selection.image_number > page.image_count {
        return Err(At {
            page: Some(page.page_number),
            image: Some(selection.image_number),
            offset: Some(page.row_offset + 8),
            ..At::NONE
        }
        .error(Type2PdfErrorKind::InvalidSelection(
            "image number exceeds page image count",
        )));
    }
    let mut image = reader
        .next_image()
        .await
        .map_err(|error| selected_container(error, selection))?
        .expect("selected image count was checked");
    for _ in 1..selection.image_number {
        image = reader
            .next_image()
            .await
            .map_err(|error| selected_container(error, selection))?
            .expect("selected image count was checked");
    }
    let at = At {
        page: Some(image.page_number),
        image: Some(image.image_number),
        offset: Some(image.payload.offset),
        ..At::NONE
    };
    let checked = preflight_type2(
        reader.source_mut(),
        image,
        limits,
        cancellation,
        options.jpeg,
    )
    .await?;
    let info = checked.info();
    let mut document = PdfDocument::new(sink, limits, cancellation)
        .await
        .map_err(at.wrap(Type2PdfErrorKind::Pdf))?;
    let object = emit_type2_xobject(reader.source_mut(), &mut document, checked).await?;
    document
        .add_page(page_spec(info, options.pixels_per_inch), &[object])
        .await
        .map_err(at.wrap(Type2PdfErrorKind::Pdf))?;
    let mut conversion = document
        .finish()
        .await
        .map_err(at.wrap(Type2PdfErrorKind::Pdf))?;
    conversion.input_bytes_read = input_bytes_read;
    Ok(Type2SelectedPdfReport {
        conversion,
        source_variant: header.variant,
        source_pages: header.page_count,
        image,
        jpeg: info,
    })
}
