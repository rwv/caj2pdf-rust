// SPDX-License-Identifier: MIT

//! One checked HN/C8 type-2 JPEG as a bounded, one-page PDF diagnostic.

use super::{
    Budget, Hnc8Error, Hnc8Reader, ImageRecord, JpegBudget, JpegColor, JpegInfo, Variant,
    read_type2_jpeg_info,
};
use crate::pdf::{ImageEncoding, ImageObject, ImageSpec, PageSpec, PdfDocument};
use crate::{Cancellation, ConversionReport, Error, Limits, RangedSource, SequentialSink};
use sha2::{Digest, Sha256};
use std::{error, fmt};

const POINTS_PER_INCH: f64 = 72.0;

/// One-based identity of a source type-2 image. Earlier pages are skipped.
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
    /// Selected source bytes changed after JPEG preflight; discard the sink.
    SourceChanged,
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
        f.write_str("HN/C8 type-2 JPEG PDF conversion")?;
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
            Type2PdfErrorKind::SourceChanged => {
                f.write_str("selected JPEG changed between preflight and PDF copy")
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

    fn error(self, kind: Type2PdfErrorKind) -> Type2PdfError {
        Type2PdfError {
            page: self.page,
            image: self.image,
            offset: self.offset,
            kind,
        }
    }

    fn pdf(self, error: Error) -> Type2PdfError {
        self.error(Type2PdfErrorKind::Pdf(error))
    }

    fn jpeg(self, error: Hnc8Error) -> Type2PdfError {
        At {
            offset: Some(error.offset),
            ..self
        }
        .error(Type2PdfErrorKind::Jpeg(Box::new(error)))
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

/// Count every byte returned by the source, including the two selected-JPEG
/// passes, without changing the source's stable-size contract.
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
        self.read = self.read.saturating_add(count as u64);
        Ok(count)
    }
}

/// The marker cursor and PDF writer both read the selected span from start to
/// end in order. Hash returned bytes in each pass, retaining only 32 bytes of
/// digest state. A short read remains the underlying reader's typed error.
struct DigestingSource<'a, S> {
    inner: &'a mut S,
    next: u64,
    end: u64,
    hash: Sha256,
}

impl<'a, S: RangedSource> DigestingSource<'a, S> {
    fn new(inner: &'a mut S, image: ImageRecord) -> Self {
        Self {
            inner,
            next: image.payload.offset,
            end: image
                .payload
                .checked_end()
                .expect("checked descriptor span"),
            hash: Sha256::new(),
        }
    }

    fn finish(self) -> [u8; 32] {
        debug_assert_eq!(self.next, self.end);
        self.hash.finalize().into()
    }
}

impl<S: RangedSource> RangedSource for DigestingSource<'_, S> {
    fn size(&self) -> u64 {
        self.inner.size()
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> crate::Result<usize> {
        debug_assert_eq!(offset, self.next);
        let count = self.inner.read_at(offset, destination).await?;
        if count <= destination.len() {
            self.hash.update(&destination[..count]);
            self.next += count as u64;
            debug_assert!(self.next <= self.end);
        }
        Ok(count)
    }
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

/// The same descriptor, geometry and digest from one complete JPEG traversal.
/// Private fields prevent callers pairing a checked hash with another span.
#[derive(Clone, Copy, Debug)]
pub(super) struct CheckedType2 {
    record: ImageRecord,
    info: JpegInfo,
    digest: [u8; 32],
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
    };
    if image.record_type != 2 {
        return Err(At {
            offset: Some(image.descriptor_offset),
            ..at
        }
        .error(Type2PdfErrorKind::UnsupportedImageType(image.record_type)));
    }
    let mut preflight = DigestingSource::new(source, image);
    let info = read_type2_jpeg_info(&mut preflight, image, limits, cancellation, budget)
        .await
        .map_err(|error| at.jpeg(error))?;
    Ok(CheckedType2 {
        record: image,
        info,
        digest: preflight.finish(),
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
    };
    let mut copy = DigestingSource::new(source, image);
    let object = document
        .add_image(
            &mut copy,
            image.payload.offset,
            image.payload.length,
            image_spec(checked.info()),
        )
        .await
        .map_err(|error| at.pdf(error))?;
    if copy.finish() != checked.digest {
        return Err(at.error(Type2PdfErrorKind::SourceChanged));
    }
    Ok(object)
}

/// Stream one checked HN/C8 type-2 JPEG record into a one-page PDF.
///
/// Source pages before the selected page are intentionally skipped. On the
/// selected page, descriptors through the chosen image are validated in chain
/// order, but other payloads are not decoded or placed. The selected JPEG is
/// traversed as a single-scan baseline/JFIF marker profile before output.
/// `/DeviceGray` is used for grayscale and `/DeviceRGB` with explicit
/// `/ColorTransform 1` for three-component JFIF YCbCr. A marker profile alone
/// does not establish decoded-pixel validity; PDF render conformance is a
/// separate test. JPEG bytes are not collected: the parser and PDF writer
/// each read the selected range through a bounded SHA-256 source wrapper, and
/// differing digests reject changed input. The PDF sink is forward-only and
/// must be discarded by the caller on any error.
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
    let mut source = CountingSource {
        inner: source,
        read: 0,
    };
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
        .map_err(|error| at.pdf(error))?;
    let object = emit_type2_xobject(reader.source_mut(), &mut document, checked).await?;
    document
        .add_page(page_spec(info, options.pixels_per_inch), &[object])
        .await
        .map_err(|error| at.pdf(error))?;
    let mut conversion = document.finish().await.map_err(|error| at.pdf(error))?;
    conversion.input_bytes_read = source.read;
    Ok(Type2SelectedPdfReport {
        conversion,
        source_variant: header.variant,
        source_pages: header.page_count,
        image,
        jpeg: info,
    })
}
