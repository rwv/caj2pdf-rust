// SPDX-License-Identifier: MIT

//! Image-only pages for the explicitly empirical HN-A/C8 type-0/type-1/type-2/type-3
//! profile, and the separately measured single-JPEG HN-B profile.
//! The CLI and WASM adapters share this core and reject omitted source rows.

use super::convert::{Type0DecodeSettings, Type0PdfError, Type0PdfErrorKind, Type0PdfOptions};
use super::placement::{source_image_transform, source_page_geometry};
mod native;
mod type3;
pub use native::{C8FontSource, C8FontSources, convert_c8_native_pdf};

use super::convert_jbig2::{Type3PdfError, Type3PdfOptions, preflight_type3};
use super::convert_jpeg::{CheckedType2, Type2PdfError, emit_type2_xobject, preflight_type2};
use super::image_emit::{
    Type0ScratchBudget, Type0ScratchError, Type0ScratchErrorKind, Type0ScratchStage,
    emit_type0_xobject,
};
use super::{
    ApplicationInfoStatus, Budget, Header, Hnc8Error, Hnc8Reader, ImageRecord, JpegBudget,
    OutlineReport, PageRecord, RawTextCoordinate, TextBudget, Variant, empirical_image_transform,
    empirical_page_from_pixels,
};
use crate::fallible::{len_u64, reserve_exact, usize_from_u32};
use crate::jbig1::{Type0Budget, Type0Error, Type0ErrorKind, Type0Info, read_type0_info};
use crate::jbig2::text_composer::RandomAccessScratch;
use crate::jbig2::{mq::MqTable, text::TextHeaderAnomaly};
use crate::pdf::{BookmarkView, ImagePlacement, MAX_PAGE_IMAGE_PLACEMENTS, PageSpec, PdfDocument};
use crate::qm::{ArithmeticBudget, ArithmeticError, ContextBank, QmTable};
use crate::{
    Bookmark, BookmarkVisitor, Cancellation, ConversionReport, Error, Limits, MAX_BUDGET_COUNT,
    RangedSource, SequentialSink,
};
use std::{error, fmt, mem::size_of};

/// Caller-owned stores reused between images. Existing type-0/JPEG callers
/// may still pass `&mut scratch` directly. Type-3 callers additionally provide
/// three symbol stores and the caller-owned MQ table.
pub struct ComposeWorkspaces<'a, T> {
    pub rows: &'a mut T,
    pub type3: Option<ComposeType3Workspaces<'a, T>>,
}

pub struct ComposeType3Workspaces<'a, T> {
    pub table: &'a MqTable,
    pub first: &'a mut T,
    pub second: &'a mut T,
    pub refined: &'a mut T,
}

impl<'a, T> From<&'a mut T> for ComposeWorkspaces<'a, T> {
    fn from(rows: &'a mut T) -> Self {
        Self { rows, type3: None }
    }
}

/// Per-page metadata and per-image temporary-storage ceilings. Type-3 limits
/// cover all four stores together. Scratch work
/// charges requested read/write bytes, including requests that fail or make
/// short progress. These limits do not cap PDF indexes or process residency.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ComposeBudget {
    pub max_page_metadata_bytes: u64,
    pub max_row_store_bytes: u64,
    pub max_row_store_io_bytes: u64,
}

impl Default for ComposeBudget {
    fn default() -> Self {
        Self {
            max_page_metadata_bytes: 4 * 1024 * 1024,
            max_row_store_bytes: 64 * 1024 * 1024,
            max_row_store_io_bytes: 256 * 1024 * 1024,
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
    /// Decoder budgets/policy; selected-image scale/container fields are ignored.
    pub type3: Type3PdfOptions,
    pub budget: ComposeBudget,
}

impl Default for ComposeOptions {
    fn default() -> Self {
        let type0 = Type0PdfOptions::default();
        Self {
            include_bookmarks: false,
            container: Budget::default(),
            text: TextBudget::default(),
            jpeg: JpegBudget::default(),
            image: type0.image,
            arithmetic: type0.arithmetic,
            type3: Type3PdfOptions::default(),
            budget: ComposeBudget::default(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum CheckedImage {
    Type0(Type0Info),
    Jpeg(CheckedType2),
    Type3 { digest: [u8; 32] },
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

/// Successful traversal totals. Peaks are accounted handler storage, not
/// an RSS measurement. Row-store totals include physical successful bytes;
/// the per-image work ceiling separately charges requested bytes.
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
    pub peak_page_metadata_bytes: u64,
    pub peak_text_working_bytes: u64,
    pub peak_row_store_bytes: u64,
    pub row_store_read_bytes: u64,
    pub row_store_written_bytes: u64,
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
    Jpeg(Box<Type2PdfError>),
    Type3(Box<Type3PdfError>),
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
            ComposeErrorKind::Type3(error) => write!(f, "{error}"),
            ComposeErrorKind::NoImages => f.write_str("image-only output has no image to draw"),
            ComposeErrorKind::Container(error) => write!(f, "{error}"),
            ComposeErrorKind::Image(error) => write!(f, "{error}"),
            ComposeErrorKind::Jpeg(error) => write!(f, "{error}"),
            ComposeErrorKind::Contexts(error) => write!(f, "{error}"),
            ComposeErrorKind::Io(error) => write!(f, "{error}"),
            ComposeErrorKind::Cleanup { primary, cleanup } => {
                write!(f, "{primary}; row-store cleanup also failed: {cleanup}")
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
            ComposeErrorKind::Type3(error) => Some(error),
            ComposeErrorKind::Io(error) => Some(error),
            ComposeErrorKind::Cleanup { primary, .. } => Some(primary),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
struct At {
    variant: Option<Variant>,
    page: Option<u32>,
    image: Option<u32>,
    offset: Option<u64>,
}

impl At {
    const NONE: Self = Self {
        variant: None,
        page: None,
        image: None,
        offset: None,
    };
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
    fn error(self, stage: ComposeStage, kind: ComposeErrorKind) -> ComposeError {
        ComposeError {
            variant: self.variant,
            page: self.page,
            image: self.image,
            offset: self.offset,
            stage,
            kind,
        }
    }
    fn io(self, stage: ComposeStage) -> impl FnOnce(Error) -> ComposeError {
        move |error| self.error(stage, ComposeErrorKind::Io(error))
    }
    fn type0(self, stage: ComposeStage) -> impl FnOnce(Type0Error) -> ComposeError {
        move |error| {
            Self {
                offset: Some(error.offset),
                ..self
            }
            .error(stage, ComposeErrorKind::Image(Box::new(error)))
        }
    }
    fn jpeg(self, stage: ComposeStage) -> impl FnOnce(Type2PdfError) -> ComposeError {
        move |error| {
            Self {
                offset: error.offset.or(self.offset),
                ..self
            }
            .error(stage, ComposeErrorKind::Jpeg(Box::new(error)))
        }
    }
    fn contexts(self) -> impl FnOnce(ArithmeticError) -> ComposeError {
        move |error| {
            self.error(
                ComposeStage::Decode,
                ComposeErrorKind::Contexts(Box::new(error)),
            )
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
    .error(stage, ComposeErrorKind::Container(Box::new(error)))
}

fn type0_decode(at: At, error: Type0PdfError) -> ComposeError {
    let at = At {
        offset: error.offset.or(at.offset),
        ..at
    };
    match error.kind {
        Type0PdfErrorKind::Image(error) => {
            let stage = if matches!(error.kind, Type0ErrorKind::Sink(_)) {
                ComposeStage::Scratch
            } else {
                ComposeStage::Decode
            };
            at.error(stage, ComposeErrorKind::Image(error))
        }
        Type0PdfErrorKind::Pdf(error) => at.error(ComposeStage::Pdf, ComposeErrorKind::Io(error)),
        Type0PdfErrorKind::Contexts(error) => {
            at.error(ComposeStage::Decode, ComposeErrorKind::Contexts(error))
        }
        Type0PdfErrorKind::Container(error) => container(*error, ComposeStage::Container),
        Type0PdfErrorKind::InvalidOptions(reason) | Type0PdfErrorKind::InvalidSelection(reason) => {
            at.error(
                ComposeStage::Decode,
                ComposeErrorKind::InvalidOptions(reason),
            )
        }
        Type0PdfErrorKind::UnsupportedImageType(kind) => at.error(
            ComposeStage::Decode,
            ComposeErrorKind::UnsupportedImageType(kind),
        ),
        Type0PdfErrorKind::MultipleImages(_) => at.error(
            ComposeStage::Decode,
            ComposeErrorKind::Unsupported("type-0 decoding profile"),
        ),
        Type0PdfErrorKind::NoImages => at.error(ComposeStage::Decode, ComposeErrorKind::NoImages),
    }
}

fn scratch_error(at: At, error: Type0ScratchError) -> ComposeError {
    let store_stage = if error.stage == Type0ScratchStage::Cleanup {
        ComposeStage::Cleanup
    } else {
        ComposeStage::Scratch
    };
    let primary = match error.kind {
        Type0ScratchErrorKind::Decode(error) => type0_decode(at, error),
        Type0ScratchErrorKind::Store(error) => at.io(store_stage)(error),
        Type0ScratchErrorKind::Pdf(error) => at.io(ComposeStage::Pdf)(error),
    };
    match error.cleanup_error {
        None => primary,
        Some(cleanup) => at.error(
            ComposeStage::Cleanup,
            ComposeErrorKind::Cleanup {
                primary: Box::new(primary),
                cleanup,
            },
        ),
    }
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
        return Err(at.error(
            ComposeStage::Headers,
            ComposeErrorKind::Unsupported("repeated image type or length differs"),
        ));
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
            return Err(At {
                offset: Some(repeated.payload.offset + offset),
                ..at
            }
            .error(
                ComposeStage::Headers,
                ComposeErrorKind::Unsupported("repeated image payload differs"),
            ));
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
        || !counters.contains(&options.budget.max_row_store_bytes)
        || !counters.contains(&options.budget.max_row_store_io_bytes)
    {
        return Err(At::NONE.error(
            ComposeStage::Preflight,
            ComposeErrorKind::InvalidOptions(
                "composition and arithmetic counters must be in 1..=MAX_BUDGET_COUNT",
            ),
        ));
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

fn add_store_bytes(total: u64, bytes: u64) -> crate::Result<u64> {
    total.checked_add(bytes).ok_or(Error::InvalidInput {
        reason: "document row-store byte count overflows u64",
    })
}

struct CountingSource<'a, S> {
    source: &'a mut S,
    bytes: u64,
}
impl<S: RangedSource> RangedSource for CountingSource<'_, S> {
    fn size(&self) -> u64 {
        self.source.size()
    }
    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> crate::Result<usize> {
        let read = self.source.read_at(offset, destination).await?;
        self.bytes = self.bytes.saturating_add(read as u64);
        Ok(read)
    }
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
) -> Result<(CheckedImage, u32, u32, u32), ComposeError> {
    Ok(match record.record_type {
        0 if variant != Variant::HnB => {
            if table.is_none() {
                return Err(image_at.error(ComposeStage::Headers, ComposeErrorKind::MissingTable));
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
                info.width,
                display_width,
                info.height,
            )
        }
        3 if variant != Variant::HnB => {
            if !has_type3_workspaces {
                return Err(image_at.error(
                    ComposeStage::Headers,
                    ComposeErrorKind::MissingType3Workspaces,
                ));
            }
            let checked = preflight_type3(source, record, options.type3, limits, cancellation)
                .await
                .map_err(|error| type3::error(image_at, ComposeStage::Headers, error))?;
            let page = checked.page();
            let display_width = page.width;
            (
                CheckedImage::Type3 {
                    digest: checked.digest(),
                },
                page.width,
                display_width,
                page.height,
            )
        }
        // Type 1 reuses the validated JPEG path in the measured
        // HN-A/C8 composition profile; HN-B remains type-2 only.
        1 | 2 if record.record_type == 2 || variant != Variant::HnB => {
            let checked = preflight_type2(source, record, limits, cancellation, options.jpeg)
                .await
                .map_err(image_at.jpeg(ComposeStage::Headers))?;
            let info = checked.info();
            let width = u32::from(info.width);
            let height = u32::from(info.height);
            (CheckedImage::Jpeg(checked), width, width, height)
        }
        _ => {
            return Err(image_at.error(
                ComposeStage::Headers,
                ComposeErrorKind::UnsupportedImageType(record.record_type),
            ));
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
    image_at: At,
    contexts: &mut Option<ContextBank>,
    workspaces: &mut ComposeWorkspaces<'_, T>,
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
            let settings = Type0DecodeSettings {
                table: table.expect("type-zero table checked"),
                arithmetic: options.arithmetic,
                image: options.image,
                limits,
                cancellation,
            };
            let (object, scratch_report) = emit_type0_xobject(
                source,
                document,
                image.record,
                info,
                contexts.as_mut().expect("contexts constructed"),
                workspaces.rows,
                Type0ScratchBudget {
                    max_bytes: options.budget.max_row_store_bytes,
                    max_work_bytes: options.budget.max_row_store_io_bytes,
                },
                &settings,
            )
            .await
            .map_err(|error| scratch_error(image_at, *error))?;
            report.peak_row_store_bytes = report
                .peak_row_store_bytes
                .max(scratch_report.peak_scratch_bytes);
            report.row_store_read_bytes = add_store_bytes(
                report.row_store_read_bytes,
                scratch_report.scratch_read_bytes,
            )
            .map_err(image_at.io(ComposeStage::Scratch))?;
            report.row_store_written_bytes = add_store_bytes(
                report.row_store_written_bytes,
                scratch_report.scratch_write_bytes,
            )
            .map_err(image_at.io(ComposeStage::Scratch))?;
            report.type0_images += 1;
            object
        }
        CheckedImage::Type3 { digest } => {
            let (object, stats) = type3::emit(
                source,
                document,
                image_at,
                image.record,
                digest,
                workspaces,
                options,
                limits,
                cancellation,
            )
            .await?;
            report.peak_row_store_bytes = report.peak_row_store_bytes.max(stats.peak);
            report.row_store_read_bytes = add_store_bytes(report.row_store_read_bytes, stats.read)
                .map_err(image_at.io(ComposeStage::Scratch))?;
            report.row_store_written_bytes =
                add_store_bytes(report.row_store_written_bytes, stats.written)
                    .map_err(image_at.io(ComposeStage::Scratch))?;
            image.type3_text_header_anomaly = stats.anomaly;
            report.type3_images += 1;
            object
        }
        CheckedImage::Jpeg(checked) => {
            let object = emit_type2_xobject(source, document, checked)
                .await
                .map_err(image_at.jpeg(ComposeStage::Pdf))?;
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
/// tables and omitted draws are errors. A caller table is never redistributed.
///
/// Source-declared page and image extents determine HN-A/C8 layout using
/// the empirical coordinate unit. Zero extents are errors. DIB storage
/// padding is omitted from PDF image widths and streams. Type-0 rows are
/// reversed in one bounded store before the negative-height CTM is applied.
/// JPEG bytes are copied unchanged and SHA revalidated. Only current-page
/// plans/placements are held; the existing PDF writer retains its indexes.
/// Type-3 retains top-first packed rows while streaming to an equivalent
/// positive-height CTM. Three symbol stores and text scratch
/// are cleared per image; their aggregate size and I/O share the store budget.
/// Normal completed type-0/type-3 paths truncate their stores, including errors.
/// A dropped pending future cannot perform async cleanup: the adapter must
/// dispose of its store and partial output, and never resume that session.
#[allow(clippy::too_many_arguments)]
pub async fn convert_source_pages_pdf<'a, S, W, T, V, C>(
    source: &mut S,
    sink: &mut W,
    table: Option<&QmTable>,
    workspaces: impl Into<ComposeWorkspaces<'a, T>>,
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
    let mut workspaces = workspaces.into();
    let mut counted = CountingSource { source, bytes: 0 };
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
                return Err(at.error(ComposeStage::Preflight, ComposeErrorKind::NoImages));
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
            return Err(at.error(
                ComposeStage::Preflight,
                ComposeErrorKind::Unsupported("HN-B image-bearing rows require exactly one JPEG"),
            ));
        }
        let count = usize_from_u32(page.image_count);
        if count > MAX_PAGE_IMAGE_PLACEMENTS {
            return Err(at.io(ComposeStage::Preflight)(Error::LimitExceeded {
                resource: "PDF image placements per page",
                limit: MAX_PAGE_IMAGE_PLACEMENTS as u64,
                attempted: u64::from(page.image_count),
            }));
        }
        // Both requested Vec sizes and their coexistence are checked before
        // either allocation or any image output from this page.
        let plan_bytes = metadata_bytes(
            u64::from(page.image_count),
            size_of::<ComposedImage>() as u64,
        )
        .map_err(at.io(ComposeStage::Preflight))?;
        let placement_bytes = metadata_bytes(
            u64::from(page.image_count),
            size_of::<ImagePlacement>() as u64,
        )
        .map_err(at.io(ComposeStage::Preflight))?;
        limits
            .check_allocation(plan_bytes)
            .map_err(at.io(ComposeStage::Preflight))?;
        limits
            .check_allocation(placement_bytes)
            .map_err(at.io(ComposeStage::Preflight))?;
        check_metadata(plan_bytes + placement_bytes, options.budget)
            .map_err(at.io(ComposeStage::Preflight))?;
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
                super::text::ReadPurpose::Compose,
            )
            .await
            .map_err(|error| container(error, ComposeStage::Text))?;
            report.peak_text_working_bytes = report
                .peak_text_working_bytes
                .max(text.working_memory_bytes);
            page_size = text.page_size.or(page_size);
            coordinates = text.coordinates;
        }
        if header.variant != Variant::HnB
            && (coordinates.is_empty() || !count.is_multiple_of(coordinates.len()))
        {
            return Err(at.error(
                ComposeStage::Text,
                ComposeErrorKind::Unsupported(
                    "image descriptor count is not a complete coordinate group",
                ),
            ));
        }
        let mut images: Vec<ComposedImage> = page_vector(count, limits, "current-page image plans")
            .map_err(at.io(ComposeStage::Preflight))?;
        let plan_capacity = capacity_bytes::<ComposedImage>(images.capacity());
        let planning_peak =
            plan_capacity + capacity_bytes::<RawTextCoordinate>(coordinates.capacity());
        check_metadata(planning_peak, options.budget).map_err(at.io(ComposeStage::Preflight))?;
        report.peak_page_metadata_bytes = report.peak_page_metadata_bytes.max(planning_peak);
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
            let (checked, visible_width, display_width, height) = preflight_image(
                reader.source_mut(),
                record,
                header.variant,
                image_at,
                table,
                workspaces.type3.is_some(),
                options,
                limits,
                cancellation,
            )
            .await?;
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
            if matches!(checked, CheckedImage::Type3 { .. }) {
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
        let metadata_peak = plan_capacity + placement_capacity;
        check_metadata(metadata_peak, options.budget).map_err(at.io(ComposeStage::Preflight))?;
        report.peak_page_metadata_bytes = report.peak_page_metadata_bytes.max(metadata_peak);
        for index in 0..images.len() {
            if let Some(original) = images[index].duplicate_of {
                images[index].type3_text_header_anomaly =
                    images[usize_from_u32(original - 1)].type3_text_header_anomaly;
                report.duplicate_image_records += 1;
                continue;
            }
            let image = &mut images[index];
            let image_at = at.image(image.record);
            let object = emit_image(
                reader.source_mut(),
                &mut document,
                image,
                image_at,
                &mut contexts,
                &mut workspaces,
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
        return Err(document_at.error(ComposeStage::Preflight, ComposeErrorKind::NoImages));
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
    report.conversion.input_bytes_read = counted.bytes;
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
            peak_page_metadata_bytes: 0,
            peak_text_working_bytes: 0,
            peak_row_store_bytes: 0,
            row_store_read_bytes: 0,
            row_store_written_bytes: 0,
            outline: OutlineReport::default(),
            application_info: ApplicationInfoStatus::Absent,
        }
    }
}
