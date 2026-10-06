// SPDX-License-Identifier: MIT

//! Image-only pages for the explicitly empirical HN-A/C8 type-0/type-1/type-2/type-3
//! profile, and the separately measured single-JPEG HN-B profile.
//! The CLI and WASM adapters share this core and reject omitted source rows.

use super::placement::{source_image_transform, source_page_geometry};
mod native;
mod route;
mod type3;
pub use native::{C8FontSource, C8FontSources, convert_c8_native_pdf};
pub use route::{convert_document_pdf, uses_native_text};

use super::type3_image::{CheckedType3, Type3PdfOptions, Type3Stage, preflight_type3};
use super::{
    ApplicationInfoStatus, At, Budget, Header, Hnc8Error, Hnc8Reader, ImageRecord, JpegBudget,
    JpegColor, JpegInfo, Locate, OutlineReport, PageRecord, RawTextCoordinate, TextBudget, Variant,
    empirical_image_transform, empirical_page_from_pixels, read_type2_jpeg_info,
};
use crate::fallible::{len_u64, reserve, reserve_exact, usize_from_u32};
use crate::jbig1::{
    Type0Budget, Type0Decoder, Type0Error, Type0ErrorKind, Type0Info, read_type0_info,
};
use crate::jbig2::text_composer::RandomAccessScratch;
use crate::jbig2::{mq::MqTable, text::TextHeaderAnomaly};
use crate::pdf::{
    BilevelImageSpec, BookmarkView, ImageEncoding, ImagePlacement, ImageSpec,
    MAX_PAGE_IMAGE_PLACEMENTS, PageSpec, PdfDocument,
};
use crate::qm::{ArithmeticBudget, ArithmeticError, ContextBank, QmTable};
use crate::{
    Bookmark, BookmarkVisitor, Cancellation, ConversionReport, CountingSource, Error, Limits,
    MAX_BUDGET_COUNT, RangedSource, SequentialSink,
};
use std::{error, fmt, mem::size_of};

/// Caller-owned stores reused between type-3 images: three symbol stores,
/// the full-page text scratch and the standard MQ table. Type-0 and JPEG
/// images stream straight to the PDF and use no store.
pub struct ComposeType3Workspaces<'a, T> {
    pub table: &'a MqTable,
    pub first: &'a mut T,
    pub second: &'a mut T,
    pub refined: &'a mut T,
    pub text: &'a mut T,
}

/// Per-page metadata and per-image type-3 storage ceilings. The type-3
/// limits cover all four stores together. Scratch work
/// charges requested read/write bytes, including requests that fail or make
/// short progress. These limits do not cap PDF indexes or process residency.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ComposeBudget {
    pub max_page_metadata_bytes: u64,
    pub max_type3_store_bytes: u64,
    pub max_type3_store_io_bytes: u64,
}

impl Default for ComposeBudget {
    fn default() -> Self {
        Self {
            max_page_metadata_bytes: 4 * 1024 * 1024,
            max_type3_store_bytes: 64 * 1024 * 1024,
            max_type3_store_io_bytes: 256 * 1024 * 1024,
        }
    }
}

/// Independent codec/resource choices. The geometry is the measured profile
/// with origin `[0, 0]`; arbitrary scaling and clipping are not performed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComposeOptions {
    /// Emit validated HN-A outlines after page composition. Defaults to false
    /// to preserve the existing image-only diagnostic API; C8/HN-B are refused.
    pub include_bookmarks: bool,
    pub container: Budget,
    pub text: TextBudget,
    pub jpeg: JpegBudget,
    pub image: Type0Budget,
    pub arithmetic: ArithmeticBudget,
    /// Type-3 decoder budgets and text-header policy.
    pub type3: Type3PdfOptions,
    pub budget: ComposeBudget,
}

impl Default for ComposeOptions {
    fn default() -> Self {
        // One decision per pixel plus one row-control decision per row of
        // the largest admitted type-0 image.
        let image = Type0Budget::default();
        let max_symbols = image.max_pixels + u64::from(image.max_height);
        Self {
            include_bookmarks: false,
            container: Budget::default(),
            text: TextBudget::default(),
            jpeg: JpegBudget::default(),
            image,
            arithmetic: ArithmeticBudget {
                max_symbols,
                max_work: max_symbols * 32 + 1024,
            },
            type3: Type3PdfOptions::default(),
            budget: ComposeBudget::default(),
        }
    }
}

/// Copyable per-descriptor codec state, paired with its record by the
/// private field of [`ComposedImage`]. Type-3 metadata owns a segment
/// directory, so it is retained beside the page plan and moved into its emit.
#[derive(Clone, Copy, Debug)]
enum CheckedImage {
    Type0(Type0Info),
    /// Marker/profile facts from one complete traversal; the bytes are
    /// copied unchanged.
    Jpeg(JpegInfo),
    Type3,
}

struct OutlineSink<'a, 'b, W: SequentialSink, C: Cancellation>(&'a mut PdfDocument<'b, W, C>);

impl<W: SequentialSink, C: Cancellation> BookmarkVisitor for OutlineSink<'_, '_, W, C> {
    async fn visit(&mut self, bookmark: Bookmark) -> crate::Result<()> {
        self.0
            .add_bookmark_with_view(bookmark, BookmarkView::Xyz)
            .await
    }
}

/// Source-derived facts for a drawn image or verified alias, in descriptor order.
/// The private checked codec state prevents mismatching a header and span.
#[derive(Clone, Copy, Debug)]
pub struct ComposedImage {
    pub record: ImageRecord,
    pub visible_width: u32,
    pub display_width: u32,
    pub height: u32,
    pub transform: [f64; 6],
    /// One-based first-group record when this byte-identical descriptor was
    /// validated but not drawn again. Its transform describes the original draw.
    pub duplicate_of: Option<u32>,
    pub type3_text_header_anomaly: Option<TextHeaderAnomaly>,
    checked: CheckedImage,
}

/// Borrowed result for every traversed source row. PDF page numbers are
/// one-based. The narrow HN-B path reports no-image rows with `None` and an
/// empty image slice; these rows are not compatibility passes or blank pages.
#[derive(Clone, Copy, Debug)]
pub struct ComposePage<'a> {
    pub source: PageRecord,
    pub output_page: Option<u32>,
    pub size: Option<PageSpec>,
    pub images: &'a [ComposedImage],
}

/// Streaming source/output mapping. Events borrow only the current page.
/// A visitor failure invalidates the partial PDF, including previous pages.
#[allow(async_fn_in_trait)]
pub trait ComposeVisitor {
    async fn page(&mut self, page: ComposePage<'_>) -> crate::Result<()>;
}

impl ComposeVisitor for () {
    async fn page(&mut self, _page: ComposePage<'_>) -> crate::Result<()> {
        Ok(())
    }
}

/// Successful traversal totals.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComposeReport {
    pub conversion: ConversionReport,
    pub source_variant: Variant,
    pub source_pages: u32,
    pub output_pages: u32,
    pub no_image_pages: u32,
    pub type0_images: u64,
    pub jpeg_images: u64,
    pub type3_images: u64,
    pub duplicate_image_records: u64,
    /// HN-A outline totals and skipped or clamped entries; empty unless
    /// `ComposeOptions::include_bookmarks` is set.
    pub outline: OutlineReport,
    /// Whether a C8 application-info package was read into the PDF `/Info`
    /// or ignored as defective; a defect never fails conversion.
    pub application_info: ApplicationInfoStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComposeStage {
    Preflight,
    Container,
    Text,
    Headers,
    Geometry,
    Decode,
    Scratch,
    Pdf,
    Visitor,
    Cleanup,
}

#[derive(Debug)]
pub enum ComposeErrorKind {
    InvalidOptions(&'static str),
    Unsupported(&'static str),
    UnsupportedImageType(u32),
    MissingTable,
    MissingType3Workspaces,
    NoImages,
    Container(Box<Hnc8Error>),
    Image(Box<Type0Error>),
    /// A type-1/type-2 JPEG marker or profile failure.
    Jpeg(Box<Hnc8Error>),
    /// A type-3 DIB wrapper outside the observed one-bit profile.
    Type3Dib(&'static str),
    /// A typed failure of one type-3 JBIG2 decoding stage.
    Type3 {
        stage: Type3Stage,
        source: Box<dyn error::Error>,
    },
    Contexts(Box<ArithmeticError>),
    Io(Error),
    /// Both failures are preserved; neither makes the partial output valid.
    Cleanup {
        primary: Box<ComposeError>,
        cleanup: Error,
    },
}

/// One-based source identity and absolute source offset when known. Every
/// error requires discarding the PDF. Caller-owned storage must be disposed
/// by its adapter if a pending conversion future is dropped.
#[derive(Debug)]
pub struct ComposeError {
    pub variant: Option<Variant>,
    pub page: Option<u32>,
    pub image: Option<u32>,
    pub offset: Option<u64>,
    pub stage: ComposeStage,
    pub kind: ComposeErrorKind,
}

impl fmt::Display for ComposeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HN/C8 page composition, {:?}", self.stage)?;
        if let Some(variant) = self.variant {
            write!(f, ", {}", variant.as_str())?;
        }
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
            ComposeErrorKind::InvalidOptions(reason) => write!(f, "invalid options: {reason}"),
            ComposeErrorKind::Unsupported(reason) => write!(f, "unsupported profile: {reason}"),
            ComposeErrorKind::UnsupportedImageType(kind) => {
                write!(f, "unsupported image record type {kind}")
            }
            ComposeErrorKind::MissingTable => {
                f.write_str("type-0 image requires a caller QM table")
            }
            ComposeErrorKind::MissingType3Workspaces => {
                f.write_str("type-3 image requires MQ table and symbol stores")
            }
            ComposeErrorKind::Type3Dib(reason) => write!(f, "malformed type-3 DIB: {reason}"),
            ComposeErrorKind::Type3 { stage, source } => write!(f, "type-3 {stage:?}: {source}"),
            ComposeErrorKind::NoImages => f.write_str("image-only output has no image to draw"),
            ComposeErrorKind::Container(error) => write!(f, "{error}"),
            ComposeErrorKind::Image(error) => write!(f, "{error}"),
            ComposeErrorKind::Jpeg(error) => write!(f, "{error}"),
            ComposeErrorKind::Contexts(error) => write!(f, "{error}"),
            ComposeErrorKind::Io(error) => write!(f, "{error}"),
            ComposeErrorKind::Cleanup { primary, cleanup } => {
                write!(f, "{primary}; store cleanup also failed: {cleanup}")
            }
        }
    }
}

impl error::Error for ComposeError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.kind {
            ComposeErrorKind::Container(error) => Some(error),
            ComposeErrorKind::Image(error) => Some(error),
            ComposeErrorKind::Jpeg(error) => Some(error),
            ComposeErrorKind::Contexts(error) => Some(error),
            ComposeErrorKind::Type3 { source, .. } => Some(source.as_ref()),
            ComposeErrorKind::Io(error) => Some(error),
            ComposeErrorKind::Cleanup { primary, .. } => Some(primary),
            _ => None,
        }
    }
}

impl Locate for (ComposeStage, ComposeErrorKind) {
    type Error = ComposeError;

    fn locate(self, at: At) -> ComposeError {
        let (stage, kind) = self;
        ComposeError {
            variant: at.variant,
            page: at.page,
            image: at.image,
            offset: at.offset,
            stage,
            kind,
        }
    }
}

impl At {
    fn page(header: Header, page: PageRecord) -> Self {
        Self {
            variant: Some(header.variant),
            page: Some(page.page_number),
            image: None,
            offset: Some(page.row_offset),
        }
    }
    fn image(self, record: ImageRecord) -> Self {
        Self {
            image: Some(record.image_number),
            offset: Some(record.descriptor_offset),
            ..self
        }
    }
    fn io(self, stage: ComposeStage) -> impl FnOnce(Error) -> ComposeError {
        move |error| self.error((stage, ComposeErrorKind::Io(error)))
    }
    fn type0(self, stage: ComposeStage) -> impl FnOnce(Type0Error) -> ComposeError {
        move |error| {
            self.with_offset(error.offset)
                .error((stage, ComposeErrorKind::Image(Box::new(error))))
        }
    }
    fn jpeg(self, stage: ComposeStage) -> impl FnOnce(Hnc8Error) -> ComposeError {
        move |error| {
            self.with_offset(error.offset)
                .error((stage, ComposeErrorKind::Jpeg(Box::new(error))))
        }
    }
    fn contexts(self) -> impl FnOnce(ArithmeticError) -> ComposeError {
        move |error| {
            self.error((
                ComposeStage::Decode,
                ComposeErrorKind::Contexts(Box::new(error)),
            ))
        }
    }
}

/// Finish the PDF with any C8 application-info `DOI` element value as the
/// custom `/CNKI_DOI` key and its URL as `/CNKI_URL`. The observed values are
/// CNKI identifiers rather than registered DOIs, so they are stored verbatim. A defective package is recorded in the
/// report and ignored; only cancellation fails.
async fn finish_document<S: RangedSource, W: SequentialSink, C: Cancellation>(
    reader: &mut Hnc8Reader<'_, S, C>,
    document: PdfDocument<'_, W, C>,
    report: &mut ComposeReport,
    at: At,
) -> Result<ConversionReport, ComposeError> {
    let read = reader
        .application_info_report()
        .await
        .map_err(|error| container(error, ComposeStage::Container))?;
    report.application_info = read.status;
    let info = read.info.unwrap_or_default();
    document
        .finish_with_info(&[
            ("CNKI_DOI", info.doi.as_deref()),
            ("CNKI_URL", info.url.as_deref()),
        ])
        .await
        .map_err(at.io(ComposeStage::Pdf))
}

fn container(error: Hnc8Error, stage: ComposeStage) -> ComposeError {
    At {
        variant: error.variant,
        page: error.page,
        image: error.image,
        offset: Some(error.offset),
    }
    .error((stage, ComposeErrorKind::Container(Box::new(error))))
}

/// A decoder failure at its own offset; a refused PDF image write surfaces
/// as a sink error and is reported at the PDF stage.
fn type0_decode(at: At) -> impl Fn(Type0Error) -> ComposeError {
    move |error| {
        let stage = if matches!(error.kind, Type0ErrorKind::Sink(_)) {
            ComposeStage::Pdf
        } else {
            ComposeStage::Decode
        };
        at.type0(stage)(error)
    }
}

/// Stream one preflighted type-0 record's rows to a bilevel image XObject in
/// decode (top-first) order; the caller's positive-height matrix puts the
/// first row on top. The PDF writer drops each row's DIB storage padding.
#[allow(clippy::too_many_arguments)]
async fn emit_type0<S, W, C>(
    source: &mut S,
    document: &mut PdfDocument<'_, W, C>,
    record: ImageRecord,
    info: Type0Info,
    table: &QmTable,
    contexts: &mut ContextBank,
    at: At,
    options: ComposeOptions,
    limits: &Limits,
    cancellation: &C,
) -> Result<crate::pdf::ImageObject, ComposeError>
where
    S: RangedSource,
    W: SequentialSink,
    C: Cancellation,
{
    let mut rows = document
        .begin_bilevel_image(BilevelImageSpec {
            pixel_width: info.width,
            pixel_height: info.height,
            row_stride: info.dib_stride,
        })
        .await
        .map_err(at.io(ComposeStage::Pdf))?;
    let span = record
        .type0_span()
        .expect("type-0 record checked by composition preflight");
    let mut decoder = Type0Decoder::new(
        source,
        span,
        table,
        contexts,
        &mut rows,
        limits,
        cancellation,
        options.arithmetic,
        options.image,
    )
    .await
    .map_err(type0_decode(at))?;
    while decoder.decode_next_row().await.map_err(type0_decode(at))? {}
    decoder.finish().await.map_err(type0_decode(at))?;
    rows.finish().await.map_err(at.io(ComposeStage::Pdf))
}

/// Additional descriptor groups are accepted only after a complete byte
/// comparison. Two fixed 1 KiB buffers avoid per-image hashes or allocations.
async fn verify_repeated_image<S: RangedSource, C: Cancellation>(
    source: &mut S,
    original: ImageRecord,
    repeated: ImageRecord,
    at: At,
    limits: &Limits,
    cancellation: &C,
) -> Result<(), ComposeError> {
    if original.record_type != repeated.record_type
        || original.payload.length != repeated.payload.length
    {
        return Err(at.error((
            ComposeStage::Headers,
            ComposeErrorKind::Unsupported("repeated image type or length differs"),
        )));
    }
    let mut first = [0; 1024];
    let mut second = [0; 1024];
    let mut offset = 0;
    while offset < original.payload.length {
        let count = (original.payload.length - offset)
            .min(limits.io_chunk_bytes.min(first.len()) as u64) as usize;
        crate::read_exact_at(
            source,
            original.payload.offset + offset,
            &mut first[..count],
            limits,
            cancellation,
        )
        .await
        .map_err(at.io(ComposeStage::Headers))?;
        crate::read_exact_at(
            source,
            repeated.payload.offset + offset,
            &mut second[..count],
            limits,
            cancellation,
        )
        .await
        .map_err(at.io(ComposeStage::Headers))?;
        if first[..count] != second[..count] {
            return Err(at.with_offset(repeated.payload.offset + offset).error((
                ComposeStage::Headers,
                ComposeErrorKind::Unsupported("repeated image payload differs"),
            )));
        }
        offset += count as u64;
    }
    Ok(())
}

fn validate(options: ComposeOptions, limits: &Limits) -> Result<(), ComposeError> {
    limits
        .validate()
        .map_err(At::NONE.io(ComposeStage::Preflight))?;
    let counters = 1..=MAX_BUDGET_COUNT;
    if !counters.contains(&options.arithmetic.max_symbols)
        || !counters.contains(&options.arithmetic.max_work)
        || !counters.contains(&options.budget.max_page_metadata_bytes)
        || !counters.contains(&options.budget.max_type3_store_bytes)
        || !counters.contains(&options.budget.max_type3_store_io_bytes)
    {
        return Err(At::NONE.error((
            ComposeStage::Preflight,
            ComposeErrorKind::InvalidOptions(
                "composition and arithmetic counters must be in 1..=MAX_BUDGET_COUNT",
            ),
        )));
    }
    Ok(())
}

fn metadata_bytes(count: u64, element_bytes: u64) -> crate::Result<u64> {
    count.checked_mul(element_bytes).ok_or(Error::InvalidInput {
        reason: "page metadata byte count overflows u64",
    })
}

fn check_metadata(bytes: u64, budget: ComposeBudget) -> crate::Result<()> {
    if bytes > budget.max_page_metadata_bytes {
        return Err(Error::LimitExceeded {
            resource: "current-page metadata bytes",
            limit: budget.max_page_metadata_bytes,
            attempted: bytes,
        });
    }
    Ok(())
}

fn capacity_bytes<T>(capacity: usize) -> u64 {
    // All allocations were preflighted against <=2^48; actual capacities
    // must fit addressable bytes.
    len_u64(capacity) * size_of::<T>() as u64
}

fn page_vector<T>(count: usize, limits: &Limits, resource: &'static str) -> crate::Result<Vec<T>> {
    let bytes = metadata_bytes(len_u64(count), size_of::<T>() as u64)?;
    limits.check_allocation(bytes)?;
    let mut values = Vec::new();
    reserve_exact(
        &mut values,
        count,
        limits.allocation_refused(resource, bytes),
    )?;
    limits.check_allocation(capacity_bytes::<T>(values.capacity()))?;
    Ok(values)
}

/// Admit one page's image count before any allocation or image output from
/// it: the PDF placement ceiling, each per-image vector of `element_bytes`
/// against the allocation limit, and their coexistence against the
/// page-metadata budget. Shared by image-only and native composition.
fn admit_page_images(
    page: PageRecord,
    element_bytes: &[usize],
    at: At,
    options: ComposeOptions,
    limits: &Limits,
) -> Result<usize, ComposeError> {
    let count = usize_from_u32(page.image_count);
    if count > MAX_PAGE_IMAGE_PLACEMENTS {
        return Err(at.io(ComposeStage::Preflight)(Error::LimitExceeded {
            resource: "PDF image placements per page",
            limit: MAX_PAGE_IMAGE_PLACEMENTS as u64,
            attempted: u64::from(page.image_count),
        }));
    }
    let mut total = 0;
    for &element in element_bytes {
        let bytes = metadata_bytes(u64::from(page.image_count), len_u64(element))
            .map_err(at.io(ComposeStage::Preflight))?;
        limits
            .check_allocation(bytes)
            .map_err(at.io(ComposeStage::Preflight))?;
        total += bytes;
    }
    check_metadata(total, options.budget).map_err(at.io(ComposeStage::Preflight))?;
    Ok(count)
}

/// Check one descriptor without decoding it. Native mixed pages and the
/// image-only path must use the same codec admission, budgets and diagnostics.
#[allow(clippy::too_many_arguments)]
async fn preflight_image<S: RangedSource, C: Cancellation>(
    source: &mut S,
    record: ImageRecord,
    variant: Variant,
    image_at: At,
    table: Option<&QmTable>,
    has_type3_workspaces: bool,
    options: ComposeOptions,
    limits: &Limits,
    cancellation: &C,
) -> Result<(CheckedImage, Option<CheckedType3>, u32, u32, u32), ComposeError> {
    Ok(match record.record_type {
        0 if variant != Variant::HnB => {
            if table.is_none() {
                return Err(image_at.error((ComposeStage::Headers, ComposeErrorKind::MissingTable)));
            }
            let info = read_type0_info(
                source,
                record.type0_span().expect("matched type zero"),
                limits,
                cancellation,
                options.arithmetic,
                options.image,
            )
            .await
            .map_err(image_at.type0(ComposeStage::Headers))?;
            let display_width = info.width;
            (
                CheckedImage::Type0(info),
                None,
                info.width,
                display_width,
                info.height,
            )
        }
        3 if variant != Variant::HnB => {
            if !has_type3_workspaces {
                return Err(image_at.error((
                    ComposeStage::Headers,
                    ComposeErrorKind::MissingType3Workspaces,
                )));
            }
            let checked = preflight_type3(
                source,
                record,
                image_at,
                options.type3,
                limits,
                cancellation,
            )
            .await?;
            let page = checked.page();
            let display_width = page.width;
            (
                CheckedImage::Type3,
                Some(checked),
                page.width,
                display_width,
                page.height,
            )
        }
        // Type 1 reuses the validated JPEG path in the measured
        // HN-A/C8 composition profile; HN-B remains type-2 only.
        1 | 2 if record.record_type == 2 || variant != Variant::HnB => {
            let info = read_type2_jpeg_info(source, record, limits, cancellation, options.jpeg)
                .await
                .map_err(image_at.jpeg(ComposeStage::Headers))?;
            let width = u32::from(info.width);
            let height = u32::from(info.height);
            (CheckedImage::Jpeg(info), None, width, width, height)
        }
        _ => {
            return Err(image_at.error((
                ComposeStage::Headers,
                ComposeErrorKind::UnsupportedImageType(record.record_type),
            )));
        }
    })
}

/// Decode one preflighted descriptor into the current PDF document. Kept
/// independent of page placement so native text pages can reuse the same
/// codec, scratch accounting and cleanup path as image-only composition.
#[allow(clippy::too_many_arguments)]
async fn emit_image<S, W, T, C>(
    source: &mut S,
    document: &mut PdfDocument<'_, W, C>,
    image: &mut ComposedImage,
    type3: Option<CheckedType3>,
    image_at: At,
    contexts: &mut Option<ContextBank>,
    stores: &mut Option<ComposeType3Workspaces<'_, T>>,
    table: Option<&QmTable>,
    options: ComposeOptions,
    limits: &Limits,
    cancellation: &C,
    report: &mut ComposeReport,
) -> Result<crate::pdf::ImageObject, ComposeError>
where
    S: RangedSource,
    W: SequentialSink,
    T: RandomAccessScratch,
    C: Cancellation,
{
    let object = match image.checked {
        CheckedImage::Type0(info) => {
            if contexts.is_none() {
                *contexts = Some(ContextBank::new(1024, limits).map_err(image_at.contexts())?);
            }
            let object = emit_type0(
                source,
                document,
                image.record,
                info,
                table.expect("type-zero table checked"),
                contexts.as_mut().expect("contexts constructed"),
                image_at,
                options,
                limits,
                cancellation,
            )
            .await?;
            report.type0_images += 1;
            object
        }
        CheckedImage::Type3 => {
            let (object, anomaly) = type3::emit(
                source,
                document,
                image_at,
                type3.expect("type-3 metadata retained from preflight"),
                stores.as_mut().expect("type-3 stores checked"),
                options,
                limits,
                cancellation,
            )
            .await?;
            image.type3_text_header_anomaly = anomaly;
            report.type3_images += 1;
            object
        }
        CheckedImage::Jpeg(info) => {
            // The checked marker profile is streamed unchanged: /DeviceGray
            // for grayscale, /DeviceRGB with /ColorTransform 1 for YCbCr.
            let record = image.record;
            let encoding = match info.color {
                JpegColor::Gray => ImageEncoding::JpegGray8,
                JpegColor::Ycbcr => ImageEncoding::JpegRgb8,
            };
            let spec = ImageSpec {
                pixel_width: u32::from(info.width),
                pixel_height: u32::from(info.height),
                encoding,
            };
            let object = document
                .add_image(source, record.payload.offset, record.payload.length, spec)
                .await
                .map_err(
                    image_at
                        .with_offset(record.payload.offset)
                        .io(ComposeStage::Pdf),
                )?;
            report.jpeg_images += 1;
            object
        }
    };
    Ok(object)
}

/// Compose every source row of the measured image-only profiles in order.
///
/// HN-A/C8 require validated text framing and types 0, 1, 2 or 3. HN-B accepts only
/// one JPEG on an image-bearing row and separately reports its no-image rows.
/// Pure-text-only documents, unsupported types/profiles, missing type-0
/// tables and omitted draws are errors.
///
/// Source-declared page and image extents determine HN-A/C8 layout using
/// the empirical coordinate unit. Zero extents are errors. DIB storage
/// padding is omitted from PDF image widths and streams. Type-0 and type-3
/// rows stream top-first under a positive-height CTM equivalent to the
/// reference's negative-height matrix for bottom-first rows. JPEG bytes are
/// copied unchanged. Only current-page plans/placements and checked type-3
/// directories are held; the existing PDF writer retains its indexes. Only
/// type-3 images use the `type3` stores: three symbol stores and the text
/// scratch are cleared per image; their aggregate size and I/O share the
/// store budget. Normal completed type-3 paths truncate their stores,
/// including errors.
/// A dropped pending future cannot perform async cleanup: the adapter must
/// dispose of its store and partial output, and never resume that session.
#[allow(clippy::too_many_arguments)]
pub async fn convert_source_pages_pdf<'a, S, W, T, V, C>(
    source: &mut S,
    sink: &mut W,
    table: Option<&QmTable>,
    mut type3: Option<ComposeType3Workspaces<'a, T>>,
    visitor: &mut V,
    options: ComposeOptions,
    limits: &Limits,
    cancellation: &C,
) -> Result<ComposeReport, ComposeError>
where
    S: RangedSource,
    W: SequentialSink,
    T: RandomAccessScratch + 'a,
    V: ComposeVisitor,
    C: Cancellation,
{
    validate(options, limits)?;
    let mut input_bytes_read = 0;
    let mut counted = CountingSource::new(source, &mut input_bytes_read);
    let mut reader = Hnc8Reader::open(&mut counted, limits, cancellation, options.container)
        .await
        .map_err(|error| container(error, ComposeStage::Container))?;
    let header = reader.header();
    let document_at = At {
        variant: Some(header.variant),
        offset: Some(0),
        ..At::NONE
    };
    let mut document = PdfDocument::new(sink, limits, cancellation)
        .await
        .map_err(document_at.io(ComposeStage::Pdf))?;
    let mut report = ComposeReport::new(header);
    // Only HN-A outlines are verified; for C8/HN-B a request writes nothing
    // and is reported rather than failing the whole conversion.
    let include_bookmarks = options.include_bookmarks && header.variant == Variant::HnA;
    report.outline.unverified = options.include_bookmarks && !include_bookmarks;
    let mut contexts = None;
    while let Some(page) = reader
        .next_page()
        .await
        .map_err(|error| container(error, ComposeStage::Container))?
    {
        let at = At::page(header, page);
        if page.image_count == 0 {
            if header.variant != Variant::HnB {
                return Err(at.error((ComposeStage::Preflight, ComposeErrorKind::NoImages)));
            }
            report.no_image_pages += 1;
            visitor
                .page(ComposePage {
                    source: page,
                    output_page: None,
                    size: None,
                    images: &[],
                })
                .await
                .map_err(at.io(ComposeStage::Visitor))?;
            continue;
        }
        if header.variant == Variant::HnB && page.image_count != 1 {
            return Err(at.error((
                ComposeStage::Preflight,
                ComposeErrorKind::Unsupported("HN-B image-bearing rows require exactly one JPEG"),
            )));
        }
        // Both Vec sizes and their coexistence are admitted up front.
        let count = admit_page_images(
            page,
            &[size_of::<ComposedImage>(), size_of::<ImagePlacement>()],
            at,
            options,
            limits,
        )?;
        let mut coordinates = Vec::new();
        // Legacy HN-B image-only composition derives its canvas from the image.
        // Its header extents are admitted for native text composition separately.
        let mut page_size = if header.variant == Variant::HnB {
            None
        } else {
            header.page_size
        };
        if header.variant != Variant::HnB {
            let text = super::text::read_coordinates(
                reader.source_mut(),
                header,
                page,
                limits,
                cancellation,
                options.text,
            )
            .await
            .map_err(|error| container(error, ComposeStage::Text))?;
            page_size = text.page_size.or(page_size);
            coordinates = text.coordinates;
        }
        if header.variant != Variant::HnB
            && (coordinates.is_empty() || !count.is_multiple_of(coordinates.len()))
        {
            return Err(at.error((
                ComposeStage::Text,
                ComposeErrorKind::Unsupported(
                    "image descriptor count is not a complete coordinate group",
                ),
            )));
        }
        let mut images: Vec<ComposedImage> = page_vector(count, limits, "current-page image plans")
            .map_err(at.io(ComposeStage::Preflight))?;
        let plan_capacity = capacity_bytes::<ComposedImage>(images.capacity());
        let planning_peak =
            plan_capacity + capacity_bytes::<RawTextCoordinate>(coordinates.capacity());
        check_metadata(planning_peak, options.budget).map_err(at.io(ComposeStage::Preflight))?;
        // Checked type-3 metadata in emit order, charged to the same budget.
        let mut type3_plans: Vec<CheckedType3> = Vec::new();
        let mut type3_bytes = 0;
        let mut geometry = page_size
            .map(source_page_geometry)
            .transpose()
            .map_err(at.io(ComposeStage::Geometry))?;
        while let Some(record) = reader
            .next_image()
            .await
            .map_err(|error| container(error, ComposeStage::Container))?
        {
            let image_at = at.image(record);
            if header.variant != Variant::HnB && images.len() >= coordinates.len() {
                let original = images[images.len() % coordinates.len()];
                verify_repeated_image(
                    reader.source_mut(),
                    original.record,
                    record,
                    image_at,
                    limits,
                    cancellation,
                )
                .await?;
                images.push(ComposedImage {
                    record,
                    duplicate_of: Some(original.record.image_number),
                    ..original
                });
                continue;
            }
            let (checked, plan, visible_width, display_width, height) = preflight_image(
                reader.source_mut(),
                record,
                header.variant,
                image_at,
                table,
                type3.is_some(),
                options,
                limits,
                cancellation,
            )
            .await?;
            if let Some(plan) = plan {
                type3_bytes += plan.retained_bytes();
                let wanted = capacity_bytes::<CheckedType3>(type3_plans.len() + 1);
                reserve(
                    &mut type3_plans,
                    1,
                    limits.allocation_refused("current-page type-3 plans", wanted),
                )
                .map_err(image_at.io(ComposeStage::Preflight))?;
                type3_plans.push(plan);
                let peak = planning_peak
                    + capacity_bytes::<CheckedType3>(type3_plans.capacity())
                    + type3_bytes;
                check_metadata(peak, options.budget).map_err(at.io(ComposeStage::Preflight))?;
            }
            if geometry.is_none() {
                // Only HN-B lacks source page dimensions. Its admitted single
                // JPEG supplies page size; the codec preflight is independent.
                geometry = Some(
                    empirical_page_from_pixels(visible_width, height, [0.0, 0.0])
                        .map_err(image_at.io(ComposeStage::Geometry))?,
                );
            }
            let coordinate = if header.variant == Variant::HnB {
                RawTextCoordinate::default()
            } else {
                coordinates[usize_from_u32(record.image_number - 1)]
            };
            let page_geometry = geometry.expect("source page or HN-B first image checked");
            let mut transform = if header.variant == Variant::HnB {
                empirical_image_transform(page_geometry, display_width, height, coordinate)
            } else {
                source_image_transform(page_geometry, coordinate)
            }
            .map_err(image_at.io(ComposeStage::Geometry))?;
            if matches!(checked, CheckedImage::Type0(_) | CheckedImage::Type3) {
                // Top-first rows give the same placement as bottom-first rows
                // under the reference's negative-height matrix, without a copy.
                transform[5] += transform[3];
                transform[3] = -transform[3];
            }
            images.push(ComposedImage {
                record,
                visible_width,
                display_width,
                height,
                transform,
                checked,
                type3_text_header_anomaly: None,
                duplicate_of: None,
            });
        }
        drop(coordinates);
        let mut placements = page_vector(count, limits, "current-page image placements")
            .map_err(at.io(ComposeStage::Preflight))?;
        let placement_capacity = capacity_bytes::<ImagePlacement>(placements.capacity());
        let metadata_peak = plan_capacity
            + placement_capacity
            + capacity_bytes::<CheckedType3>(type3_plans.capacity())
            + type3_bytes;
        check_metadata(metadata_peak, options.budget).map_err(at.io(ComposeStage::Preflight))?;
        let mut type3_plans = type3_plans.into_iter();
        for index in 0..images.len() {
            if let Some(original) = images[index].duplicate_of {
                images[index].type3_text_header_anomaly =
                    images[usize_from_u32(original - 1)].type3_text_header_anomaly;
                report.duplicate_image_records += 1;
                continue;
            }
            let image = &mut images[index];
            let image_at = at.image(image.record);
            let plan = match image.checked {
                CheckedImage::Type3 => type3_plans.next(),
                _ => None,
            };
            let object = emit_image(
                reader.source_mut(),
                &mut document,
                image,
                plan,
                image_at,
                &mut contexts,
                &mut type3,
                table,
                options,
                limits,
                cancellation,
                &mut report,
            )
            .await?;
            placements.push(ImagePlacement {
                image: object,
                transform: image.transform,
            });
        }
        let size = geometry.expect("positive source image count").size;
        // PdfDocument returns a zero-based page index; the streaming map
        // and successful report expose one-based pages and the page count.
        report.output_pages = document
            .add_placed_page(size, &placements)
            .await
            .map_err(at.io(ComposeStage::Pdf))?
            + 1;
        visitor
            .page(ComposePage {
                source: page,
                output_page: Some(report.output_pages),
                size: Some(size),
                images: &images,
            })
            .await
            .map_err(at.io(ComposeStage::Visitor))?;
        // All current-page coordinates, plans and placements drop here before
        // the next row; only the reusable contexts and PDF indexes persist.
    }
    if report.output_pages == 0 {
        return Err(document_at.error((ComposeStage::Preflight, ComposeErrorKind::NoImages)));
    }
    if include_bookmarks {
        // This HN-A composer emits every source row in order and rejects
        // no-image rows, so its actual source/output map is the identity map.
        debug_assert_eq!(report.output_pages, header.page_count);
        report.outline = reader
            .visit_bookmarks(
                64,
                report.output_pages,
                |page| Some(page - 1),
                &mut OutlineSink(&mut document),
            )
            .await
            .map_err(|error| container(error, ComposeStage::Container))?;
    }
    report.conversion = finish_document(&mut reader, document, &mut report, document_at).await?;
    report.conversion.input_bytes_read = input_bytes_read;
    Ok(report)
}

#[cfg(test)]
mod tests;

impl ComposeReport {
    fn new(header: Header) -> Self {
        Self {
            conversion: ConversionReport::default(),
            source_variant: header.variant,
            source_pages: header.page_count,
            output_pages: 0,
            no_image_pages: 0,
            type0_images: 0,
            jpeg_images: 0,
            type3_images: 0,
            duplicate_image_records: 0,
            outline: OutlineReport::default(),
            application_info: ApplicationInfoStatus::Absent,
        }
    }
}
