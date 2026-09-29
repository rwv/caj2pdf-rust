// SPDX-License-Identifier: MIT

//! Shared bounded HN/C8 type-3 image emission and a selected-image PDF diagnostic.

use super::{Budget, Hnc8Error, Hnc8Reader, ImageRecord, Variant};
use crate::jbig2::{
    DirectoryLimits, HeaderLimits, SegmentSpan,
    dictionary::{DictionaryBudget, DirectDictionaryDecoder},
    generic::{GenericBudget, GenericRegionDecoder, read_generic_region_header},
    iaid::IaidContextBanks,
    integer::IntegerContextBanks,
    mq::{MqBudget, MqContexts, MqTable},
    page_compose::{PageComposeBudget, PageComposeReport, PageOrSink},
    page_info::{PageInfo, PageInfoBudget, read_page_info},
    page_profile::{PageProfile, validate_observed_page_profile},
    read_embedded_directory,
    refinement::RefinementBudget,
    refinement_dictionary::{RefinementDictionaryBudget, RefinementDictionaryDecoder},
    text::{
        TextHeaderAnomaly, TextHeaderPolicy, TextRegionBudget, read_text_region_header_with_policy,
    },
    text_composer::{
        BitmapView, RandomAccessScratch, TextComposeBudget, TextComposeError, TextComposeErrorKind,
        TextComposeReport, TextComposer,
    },
    text_instances::{TextInstanceBudget, TextInstanceDecoder},
};
use crate::pdf::{BilevelImageSpec, ImageObject, PageSpec, PdfDocument};
use crate::{
    Cancellation, ConversionReport, Error, Limits, RangedSource, SequentialSink, read_exact_at,
};
use sha2::{Digest, Sha256};
use std::{cell::Cell, error, fmt, rc::Rc};

const DIB_BYTES: u64 = 48;
const HASH_CHUNK: usize = 64 * 1024;
const POINTS_PER_INCH: f64 = 72.0;

/// One-based image identity. Earlier source pages are intentionally skipped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Type3ImageSelection {
    pub page_number: u32,
    pub image_number: u32,
}

/// One caller-owned symbol-dictionary store. Both read handles must observe
/// writes by the writer, including a changing length, during this call. All
/// handles refer to the same initially empty store; the writer appends from
/// byte zero. The caller disposes it on success or error.
pub struct Type3Store<'a, R: RangedSource, W: SequentialSink> {
    pub reader: &'a mut R,
    /// Independent handle for text composition while the instance decoder
    /// holds `reader`. It observes the same backing bytes and revision.
    pub compose_reader: &'a mut R,
    pub writer: &'a mut W,
}

/// One caller-owned refined-symbol store. Its reader observes writer growth
/// while text instances are decoded and composed. Both handles refer to the
/// same initially empty store; the writer appends from byte zero. The caller
/// disposes it on success or error.
pub struct Type3RefinedStore<'a, R: RangedSource, W: SequentialSink> {
    pub reader: &'a mut R,
    pub writer: &'a mut W,
}

/// Three bounded symbol stores plus the one full-page text scratch. Store
/// handles may be backed by temporary files, browser storage, or another
/// platform adapter. The second and refined stores must support reading while
/// their paired writer appends. No intermediate is a second full-page bitmap.
pub struct Type3Workspaces<'a, R: RangedSource, W: SequentialSink, T: RandomAccessScratch> {
    pub first: Type3Store<'a, R, W>,
    pub second: Type3Store<'a, R, W>,
    pub refined: Type3RefinedStore<'a, R, W>,
    pub text: &'a mut T,
}

/// Resource ceilings and explicit output scale for the observed profile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Type3PdfOptions {
    /// Each image pixel occupies `72 / pixels_per_inch` PDF points. This is
    /// a caller choice, not a recovered HN/C8 source-page dimension.
    pub pixels_per_inch: f64,
    pub container: Budget,
    pub header: HeaderLimits,
    pub directory: DirectoryLimits,
    pub page: PageInfoBudget,
    pub mq: MqBudget,
    pub dictionary: DictionaryBudget,
    pub refinement: RefinementBudget,
    pub refinement_dictionary: RefinementDictionaryBudget,
    pub text_region: TextRegionBudget,
    pub text_instance: TextInstanceBudget,
    pub text_compose: TextComposeBudget,
    pub generic: GenericBudget,
    pub page_compose: PageComposeBudget,
    /// Strict T.88 validation is the default; the named HN/C8 exception is
    /// opt-in and reported when it applies.
    pub text_header_policy: TextHeaderPolicy,
}

impl Default for Type3PdfOptions {
    fn default() -> Self {
        Self {
            pixels_per_inch: 300.0,
            container: Budget::default(),
            header: HeaderLimits::default(),
            directory: DirectoryLimits::default(),
            page: PageInfoBudget::default(),
            mq: MqBudget::default(),
            dictionary: DictionaryBudget::default(),
            refinement: RefinementBudget::default(),
            refinement_dictionary: RefinementDictionaryBudget::default(),
            text_region: TextRegionBudget::default(),
            text_instance: TextInstanceBudget::default(),
            text_compose: TextComposeBudget::default(),
            generic: GenericBudget::default(),
            page_compose: PageComposeBudget::default(),
            text_header_policy: TextHeaderPolicy::Strict,
        }
    }
}

/// Checked metadata and the completed one-page PDF. `page` is JBIG2 image
/// geometry, not a recovered HN/C8 document-page layout.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Type3SelectedPdfReport {
    pub conversion: ConversionReport,
    pub source_variant: Variant,
    pub source_pages: u32,
    pub image: ImageRecord,
    pub page: PageInfo,
    pub text_header_anomaly: Option<TextHeaderAnomaly>,
    pub page_compose: PageComposeReport,
}

/// The decoding stage where a typed underlying error arose.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Type3Stage {
    Directory,
    PageInfo,
    TextHeader,
    GenericHeader,
    Profile,
    FirstDictionary,
    SecondDictionary,
    TextInstances,
    TextCompose,
    GenericRegion,
    PageCompose,
    Contexts,
}

#[derive(Debug)]
pub enum Type3PdfErrorKind {
    InvalidOptions(&'static str),
    InvalidSelection(&'static str),
    Container(Box<Hnc8Error>),
    UnsupportedImageType(u32),
    DibMalformed(&'static str),
    Source(Error),
    SourceChanged,
    Workspace(&'static str),
    Stage {
        stage: Type3Stage,
        source: Box<dyn error::Error>,
    },
    Pdf(Error),
}

/// Located failure. `offset` is always an absolute source-byte anchor; the
/// nested text/page composition errors keep their separate scratch or output
/// coordinate. Any PDF sink bytes accepted before failure are partial output
/// and must be discarded by the caller.
#[derive(Debug)]
pub struct Type3PdfError {
    pub page: Option<u32>,
    pub image: Option<u32>,
    pub offset: Option<u64>,
    pub kind: Type3PdfErrorKind,
}

impl fmt::Display for Type3PdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HN/C8 type-3 PDF conversion")?;
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
            Type3PdfErrorKind::InvalidOptions(reason) => write!(f, "invalid options: {reason}"),
            Type3PdfErrorKind::InvalidSelection(reason) => write!(f, "invalid selection: {reason}"),
            Type3PdfErrorKind::Container(source) => write!(f, "{source}"),
            Type3PdfErrorKind::UnsupportedImageType(kind) => {
                write!(f, "unsupported image record type {kind}")
            }
            Type3PdfErrorKind::DibMalformed(reason) => write!(f, "malformed type-3 DIB: {reason}"),
            Type3PdfErrorKind::Source(source) => write!(f, "source: {source}"),
            Type3PdfErrorKind::SourceChanged => {
                f.write_str("selected type-3 source span changed between passes")
            }
            Type3PdfErrorKind::Workspace(reason) => write!(f, "workspace: {reason}"),
            Type3PdfErrorKind::Stage { stage, source } => write!(f, "{stage:?}: {source}"),
            Type3PdfErrorKind::Pdf(source) => write!(f, "PDF output: {source}"),
        }
    }
}

impl error::Error for Type3PdfError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.kind {
            Type3PdfErrorKind::Container(source) => Some(source),
            Type3PdfErrorKind::Source(source) | Type3PdfErrorKind::Pdf(source) => Some(source),
            Type3PdfErrorKind::Stage { source, .. } => Some(source.as_ref()),
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
    fn with_offset(self, offset: u64) -> Self {
        Self {
            offset: Some(offset),
            ..self
        }
    }
    fn error(self, kind: Type3PdfErrorKind) -> Type3PdfError {
        Type3PdfError {
            page: self.page,
            image: self.image,
            offset: self.offset,
            kind,
        }
    }
    fn stage<E: error::Error + 'static>(self, stage: Type3Stage, source: E) -> Type3PdfError {
        self.error(Type3PdfErrorKind::Stage {
            stage,
            source: Box::new(source),
        })
    }
    fn pdf(self, source: Error) -> Type3PdfError {
        self.error(Type3PdfErrorKind::Pdf(source))
    }
}

fn source_stage<T, E, F>(
    result: Result<T, E>,
    at: At,
    stage: Type3Stage,
    offset: F,
) -> Result<T, Type3PdfError>
where
    E: error::Error + 'static,
    F: FnOnce(&E) -> u64,
{
    result.map_err(|source| at.with_offset(offset(&source)).stage(stage, source))
}

fn work_stage<T, E>(result: Result<T, E>, at: At, stage: Type3Stage) -> Result<T, Type3PdfError>
where
    E: error::Error + 'static,
{
    result.map_err(|source| at.stage(stage, source))
}

fn composed_stage<T>(result: Result<T, TextComposeError>, at: At) -> Result<T, Type3PdfError> {
    result.map_err(|source| {
        let at = match &source.kind {
            TextComposeErrorKind::Instance(instance) => at.with_offset(instance.offset),
            _ => at,
        };
        at.stage(Type3Stage::TextCompose, source)
    })
}

fn selected_container(source: Hnc8Error, selection: Type3ImageSelection) -> Type3PdfError {
    Type3PdfError {
        page: source.page.or(Some(selection.page_number)),
        image: source.image.or(Some(selection.image_number)),
        offset: Some(source.offset),
        kind: Type3PdfErrorKind::Container(Box::new(source)),
    }
}

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

struct RevisionedSource<'a, R> {
    inner: &'a mut R,
    revision: Rc<Cell<u64>>,
}

impl<R: RangedSource> RangedSource for RevisionedSource<'_, R> {
    fn size(&self) -> u64 {
        self.inner.size()
    }
    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> crate::Result<usize> {
        self.inner.read_at(offset, destination).await
    }
}

impl<R: RangedSource> BitmapView for RevisionedSource<'_, R> {
    fn revision(&self) -> crate::Result<u64> {
        Ok(self.revision.get())
    }
}

struct RevisionedSink<'a, W> {
    inner: &'a mut W,
    revision: Rc<Cell<u64>>,
}

impl<W: SequentialSink> SequentialSink for RevisionedSink<'_, W> {
    async fn write(&mut self, bytes: &[u8]) -> crate::Result<usize> {
        let next = self
            .revision
            .get()
            .checked_add(1)
            .ok_or(Error::InvalidInput {
                reason: "bitmap revision exhausted",
            })?;
        let count = self.inner.write(bytes).await?;
        if count > 0 {
            self.revision.set(next);
        }
        Ok(count)
    }
    async fn flush(&mut self) -> crate::Result<()> {
        self.inner.flush().await
    }
}

struct DiscardSink;

impl SequentialSink for DiscardSink {
    async fn write(&mut self, bytes: &[u8]) -> crate::Result<usize> {
        Ok(bytes.len())
    }
    async fn flush(&mut self) -> crate::Result<()> {
        Ok(())
    }
}

async fn digest_span<S: RangedSource, C: Cancellation>(
    source: &mut S,
    image: ImageRecord,
    limits: &Limits,
    cancellation: &C,
    at: At,
) -> Result<[u8; 32], Type3PdfError> {
    let mut buffer = vec![0_u8; HASH_CHUNK.min(limits.io_chunk_bytes)];
    let mut hash = Sha256::new();
    let mut done = 0_u64;
    while done < image.payload.length {
        let count = (image.payload.length - done).min(buffer.len() as u64) as usize;
        let offset = image.payload.offset + done;
        read_exact_at(source, offset, &mut buffer[..count], limits, cancellation)
            .await
            .map_err(|source| {
                at.with_offset(offset)
                    .error(Type3PdfErrorKind::Source(source))
            })?;
        hash.update(&buffer[..count]);
        done += count as u64;
    }
    Ok(hash.finalize().into())
}

fn page_spec(page: PageInfo, pixels_per_inch: f64, at: At) -> Result<PageSpec, Type3PdfError> {
    let scale = POINTS_PER_INCH / pixels_per_inch;
    let spec = PageSpec {
        width_points: f64::from(page.width) * scale,
        height_points: f64::from(page.height) * scale,
    };
    if !spec.width_points.is_finite()
        || !spec.height_points.is_finite()
        || !(0.000_001..=14_400.0).contains(&spec.width_points)
        || !(0.000_001..=14_400.0).contains(&spec.height_points)
    {
        return Err(at.error(Type3PdfErrorKind::InvalidOptions(
            "selected image PDF dimensions are outside 0.000001..=14400 points",
        )));
    }
    Ok(spec)
}

/// Convert one checked type-3 JBIG2 record to one PDF image page.
///
/// This is restricted to the observed five-segment HN/C8 profile. The 47-state
/// MQ table is caller supplied and never bundled. Selected source bytes are
/// hashed before preflight, after preflight, and after decode. Intermediate
/// stores and the full-page text scratch are caller owned and bounded by the
/// explicit budgets; platform adapters must clean them up on all paths. The
/// selected image is emitted top-down, MSB-first, `1 = black`, with zero low
/// padding and `/Decode [1 0]`. A failed call can leave a partial PDF sink.
#[allow(clippy::too_many_arguments)]
pub async fn convert_type3_image_pdf<
    S: RangedSource,
    P: SequentialSink,
    R: RangedSource,
    W: SequentialSink,
    T: RandomAccessScratch,
    C: Cancellation,
>(
    source: &mut S,
    sink: &mut P,
    table: &MqTable,
    workspaces: &mut Type3Workspaces<'_, R, W, T>,
    selection: Type3ImageSelection,
    options: Type3PdfOptions,
    limits: &Limits,
    cancellation: &C,
) -> Result<Type3SelectedPdfReport, Type3PdfError> {
    if !options.pixels_per_inch.is_finite() || options.pixels_per_inch <= 0.0 {
        return Err(At::NONE.error(Type3PdfErrorKind::InvalidOptions(
            "pixels per inch must be finite and positive",
        )));
    }
    if selection.page_number == 0 || selection.image_number == 0 {
        return Err(At {
            page: Some(selection.page_number),
            image: Some(selection.image_number),
            offset: Some(0),
        }
        .error(Type3PdfErrorKind::InvalidSelection(
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
    let page_record = reader
        .next_page()
        .await
        .map_err(|error| selected_container(error, selection))?
        .expect("probe validated selected page");
    if selection.image_number > page_record.image_count {
        return Err(At {
            page: Some(selection.page_number),
            image: Some(selection.image_number),
            offset: Some(page_record.row_offset + 8),
        }
        .error(Type3PdfErrorKind::InvalidSelection(
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
    let checked =
        preflight_type3(reader.source_mut(), image, options, limits, cancellation).await?;
    let page = checked.page();
    let at = At {
        page: Some(image.page_number),
        image: Some(image.image_number),
        offset: Some(image.payload.offset),
    };
    let pdf_page = page_spec(page, options.pixels_per_inch, at)?;
    let prepared = prepare_type3_image(
        reader.source_mut(),
        table,
        workspaces,
        checked,
        options,
        limits,
        cancellation,
    )
    .await?;
    let mut document = PdfDocument::new(sink, limits, cancellation)
        .await
        .map_err(|error| at.pdf(error))?;
    let (object, page_compose) = emit_type3_xobject(
        reader.source_mut(),
        &mut document,
        table,
        prepared,
        options,
        limits,
        cancellation,
    )
    .await?;
    document
        .add_page(pdf_page, &[object])
        .await
        .map_err(|error| at.pdf(error))?;
    let mut conversion = document.finish().await.map_err(|error| at.pdf(error))?;
    conversion.input_bytes_read = source.read;
    Ok(Type3SelectedPdfReport {
        conversion,
        source_variant: header.variant,
        source_pages: header.page_count,
        image,
        page,
        text_header_anomaly: page_compose.text_header_anomaly,
        page_compose,
    })
}

/// Checked source metadata and digest. Geometry is exposed without decoding
/// pixels; the remaining fields stay paired with their original image span.
pub(super) struct CheckedType3 {
    image: ImageRecord,
    directory: crate::jbig2::SegmentDirectory,
    profile: PageProfile,
    initial_digest: [u8; 32],
}

impl CheckedType3 {
    pub(super) fn page(&self) -> PageInfo {
        self.profile.page()
    }
}

/// Prepared text pixels borrow their scratch until image emission finishes.
/// Symbol decoder contexts/catalogs have already been released.
pub(super) struct PreparedType3<'a, T> {
    checked: CheckedType3,
    text_report: TextComposeReport,
    text: &'a mut T,
}

pub(super) async fn preflight_type3<S: RangedSource, C: Cancellation>(
    source: &mut S,
    image: ImageRecord,
    options: Type3PdfOptions,
    limits: &Limits,
    cancellation: &C,
) -> Result<CheckedType3, Type3PdfError> {
    let at = At {
        page: Some(image.page_number),
        image: Some(image.image_number),
        offset: Some(image.payload.offset),
    };
    if image.record_type != 3 {
        return Err(at
            .with_offset(image.descriptor_offset)
            .error(Type3PdfErrorKind::UnsupportedImageType(image.record_type)));
    }
    if image.payload.length <= DIB_BYTES {
        return Err(at.error(Type3PdfErrorKind::DibMalformed(
            "record has no enclosed JBIG2 segments",
        )));
    }
    let initial_digest = digest_span(source, image, limits, cancellation, at).await?;
    let mut dib = [0_u8; DIB_BYTES as usize];
    read_exact_at(source, image.payload.offset, &mut dib, limits, cancellation)
        .await
        .map_err(|source| at.error(Type3PdfErrorKind::Source(source)))?;
    if u32::from_le_bytes(dib[0..4].try_into().expect("fixed DIB field")) != 40 {
        return Err(at.error(Type3PdfErrorKind::DibMalformed(
            "header size differs from 40 bytes",
        )));
    }
    let dib_width = i32::from_le_bytes(dib[4..8].try_into().expect("fixed DIB field"));
    let dib_height = i32::from_le_bytes(dib[8..12].try_into().expect("fixed DIB field"));
    if dib_width <= 0 || dib_height <= 0 {
        return Err(at
            .with_offset(image.payload.offset + 4)
            .error(Type3PdfErrorKind::DibMalformed("nonpositive dimensions")));
    }
    if dib[12..14] != 1_u16.to_le_bytes()
        || dib[14..16] != 1_u16.to_le_bytes()
        || dib[16..20] != 0_u32.to_le_bytes()
    {
        return Err(at.with_offset(image.payload.offset + 12).error(
            Type3PdfErrorKind::DibMalformed("expected one plane, one bit, and uncompressed DIB"),
        ));
    }
    if dib[40..48] != [255, 255, 255, 0, 0, 0, 0, 0] {
        return Err(at.with_offset(image.payload.offset + 40).error(
            Type3PdfErrorKind::DibMalformed("expected observed white/black palette"),
        ));
    }
    let embedded = SegmentSpan {
        offset: image.payload.offset + DIB_BYTES,
        length: image.payload.length - DIB_BYTES,
    };
    let directory = read_embedded_directory(
        source,
        embedded,
        limits,
        options.header,
        options.directory,
        cancellation,
    )
    .await
    .map_err(|error| {
        let offset = error.offset;
        at.with_offset(offset).stage(Type3Stage::Directory, error)
    })?;
    if directory.segments.len() != 5 {
        return Err(at.with_offset(embedded.offset).stage(
            Type3Stage::Profile,
            crate::jbig2::page_profile::PageProfileError {
                segment: None,
                kind: crate::jbig2::page_profile::PageProfileErrorKind::Unsupported {
                    feature: "segment count",
                    value: directory.segments.len() as u64,
                },
            },
        ));
    }
    let page = read_page_info(
        source,
        &directory.segments[0],
        limits,
        options.page,
        cancellation,
    )
    .await
    .map_err(|error| {
        let offset = error.offset;
        at.with_offset(offset).stage(Type3Stage::PageInfo, error)
    })?;
    if page.width != dib_width as u32 || page.height != dib_height as u32 {
        return Err(at.with_offset(image.payload.offset + 4).error(
            Type3PdfErrorKind::DibMalformed("DIB and JBIG2 page dimensions differ"),
        ));
    }
    let text = read_text_region_header_with_policy(
        source,
        &directory.segments[3],
        &directory.segments[2],
        limits,
        options.text_region,
        cancellation,
        options.text_header_policy,
    )
    .await
    .map_err(|error| {
        let offset = error.offset;
        at.with_offset(offset).stage(Type3Stage::TextHeader, error)
    })?;
    let generic = read_generic_region_header(
        source,
        &directory.segments[4],
        limits,
        cancellation,
        options.mq,
        options.generic,
    )
    .await
    .map_err(|error| {
        let offset = error.offset;
        at.with_offset(offset)
            .stage(Type3Stage::GenericHeader, error)
    })?;
    let profile =
        validate_observed_page_profile(&directory, page, &text, generic).map_err(|error| {
            let offset = error
                .segment
                .and_then(|number| {
                    directory
                        .segments
                        .iter()
                        .find(|segment| segment.number == number)
                        .map(|segment| segment.data.offset)
                })
                .unwrap_or(embedded.offset);
            at.with_offset(offset).stage(Type3Stage::Profile, error)
        })?;
    Ok(CheckedType3 {
        image,
        directory,
        profile,
        initial_digest,
    })
}

/// Decode symbol dictionaries and the text layer without opening a PDF stream.
/// This preserves the selected-image API's rejection before PDF output.
pub(super) async fn prepare_type3_image<'a, S, R, W, T, C>(
    source: &mut S,
    table: &MqTable,
    workspaces: &'a mut Type3Workspaces<'_, R, W, T>,
    checked: CheckedType3,
    options: Type3PdfOptions,
    limits: &Limits,
    cancellation: &C,
) -> Result<PreparedType3<'a, T>, Type3PdfError>
where
    S: RangedSource,
    R: RangedSource,
    W: SequentialSink,
    T: RandomAccessScratch,
    C: Cancellation,
{
    let image = checked.image;
    let directory = &checked.directory;
    let text = checked.profile.text_header();
    let initial_digest = checked.initial_digest;
    let at = At {
        page: Some(image.page_number),
        image: Some(image.image_number),
        offset: Some(image.payload.offset),
    };
    if digest_span(source, image, limits, cancellation, at).await? != initial_digest {
        return Err(at.error(Type3PdfErrorKind::SourceChanged));
    }
    if workspaces.first.reader.size() != 0
        || workspaces.first.compose_reader.size() != 0
        || workspaces.second.reader.size() != 0
        || workspaces.second.compose_reader.size() != 0
        || workspaces.refined.reader.size() != 0
        || workspaces
            .text
            .size()
            .map_err(|source| at.error(Type3PdfErrorKind::Source(source)))?
            != 0
    {
        return Err(at.error(Type3PdfErrorKind::Workspace(
            "intermediate stores and text scratch must start empty",
        )));
    }
    let first_revision = Rc::new(Cell::new(0));
    let second_revision = Rc::new(Cell::new(0));
    let refined_revision = Rc::new(Cell::new(0));
    let mut first_reader = RevisionedSource {
        inner: workspaces.first.reader,
        revision: first_revision.clone(),
    };
    let mut first_compose_reader = RevisionedSource {
        inner: workspaces.first.compose_reader,
        revision: first_revision.clone(),
    };
    let mut second_reader = RevisionedSource {
        inner: workspaces.second.reader,
        revision: second_revision.clone(),
    };
    let mut second_compose_reader = RevisionedSource {
        inner: workspaces.second.compose_reader,
        revision: second_revision.clone(),
    };
    let mut refined_reader = RevisionedSource {
        inner: workspaces.refined.reader,
        revision: refined_revision.clone(),
    };
    let mut first_writer = RevisionedSink {
        inner: workspaces.first.writer,
        revision: first_revision,
    };
    let mut second_writer = RevisionedSink {
        inner: workspaces.second.writer,
        revision: second_revision,
    };
    let mut refined_writer = RevisionedSink {
        inner: workspaces.refined.writer,
        revision: refined_revision,
    };
    let mut first_banks = IntegerContextBanks::with_extra_contexts(1024, limits, &options.mq)
        .map_err(|error| {
            at.with_offset(directory.segments[1].data.offset)
                .stage(Type3Stage::Contexts, error)
        })?;
    let mut first_decoder = DirectDictionaryDecoder::new(
        source,
        &directory.segments[1],
        table,
        &mut first_banks,
        &mut first_writer,
        limits,
        cancellation,
        options.mq,
        options.dictionary,
    )
    .await
    .map_err(|error| {
        let offset = error.offset;
        at.with_offset(offset)
            .stage(Type3Stage::FirstDictionary, error)
    })?;
    let first_report = source_stage(
        first_decoder.decode().await,
        at,
        Type3Stage::FirstDictionary,
        |error| error.offset,
    )?;
    drop(first_decoder);
    let imported_count = u64::from(first_report.header.exported_symbols);
    let second_count = u64::from(
        read_second_new_symbol_count(
            directory,
            source,
            limits,
            cancellation,
            options.dictionary,
            at,
        )
        .await?,
    );
    let first_code_len = code_length(imported_count + second_count);
    let second_contexts =
        IaidContextBanks::with_bitmap_contexts(first_code_len, 1024, limits, &options.mq);
    let second_at = at.with_offset(directory.segments[2].data.offset);
    let mut second_banks = work_stage(second_contexts, second_at, Type3Stage::Contexts)?;
    let mut second_decoder = RefinementDictionaryDecoder::new(
        source,
        &directory.segments[2],
        &directory.segments[1],
        &first_report,
        &mut first_reader,
        0,
        &mut second_reader,
        &mut second_writer,
        0,
        table,
        &mut second_banks,
        limits,
        cancellation,
        options.mq,
        options.dictionary,
        options.refinement,
        options.refinement_dictionary,
    )
    .await
    .map_err(|error| {
        let offset = error.offset;
        at.with_offset(offset)
            .stage(Type3Stage::SecondDictionary, error)
    })?;
    let second_report = source_stage(
        second_decoder.decode().await,
        at,
        Type3Stage::SecondDictionary,
        |error| error.offset,
    )?;
    drop(second_decoder);
    let code_len = code_length(second_report.catalog.exported_symbols.len() as u64);
    let text_contexts = IaidContextBanks::with_bitmap_contexts(code_len, 1024, limits, &options.mq);
    let text_at = at.with_offset(directory.segments[3].data.offset);
    let mut text_banks = work_stage(text_contexts, text_at, Type3Stage::Contexts)?;
    let text_decoder = TextInstanceDecoder::new_with_header_policy(
        source,
        &directory.segments[3],
        text,
        &directory.segments[2],
        &second_report,
        &mut first_reader,
        0,
        &mut second_reader,
        0,
        &mut refined_writer,
        0,
        table,
        &mut text_banks,
        limits,
        cancellation,
        options.mq,
        options.text_region,
        options.refinement,
        options.text_instance,
        options.text_header_policy,
    )
    .await;
    let mut text_decoder = source_stage(text_decoder, at, Type3Stage::TextInstances, |error| {
        error.offset
    })?;
    let mut discard = DiscardSink;
    // The shared I/O limit is another ceiling. The text composer requires its
    // own request cap to be no larger than that limit, rather than taking the
    // minimum internally as the other stages do.
    let text_compose_budget = TextComposeBudget {
        max_request_bytes: options
            .text_compose
            .max_request_bytes
            .min(limits.io_chunk_bytes),
        ..options.text_compose
    };
    let text_report = {
        let composer_result = TextComposer::new(
            directory.segments[3].number,
            text,
            &second_report.catalog.exported_symbols,
            &mut text_decoder,
            &mut first_compose_reader,
            0,
            &mut second_compose_reader,
            0,
            &mut refined_reader,
            0,
            workspaces.text,
            &mut discard,
            limits,
            cancellation,
            text_compose_budget,
        );
        let mut composer = work_stage(composer_result, at, Type3Stage::TextCompose)?;
        composed_stage(composer.compose().await, at)?
    };
    drop(text_decoder);
    Ok(PreparedType3 {
        checked,
        text_report,
        text: workspaces.text,
    })
}

/// Append one image to an existing PDF. Page creation/placement belongs to
/// the caller; the decoder never opens or finishes another document.
pub(super) async fn emit_type3_xobject<S, W, T, C>(
    source: &mut S,
    document: &mut PdfDocument<'_, W, C>,
    table: &MqTable,
    prepared: PreparedType3<'_, T>,
    options: Type3PdfOptions,
    limits: &Limits,
    cancellation: &C,
) -> Result<(ImageObject, PageComposeReport), Type3PdfError>
where
    S: RangedSource,
    W: SequentialSink,
    T: RandomAccessScratch,
    C: Cancellation,
{
    let PreparedType3 {
        checked,
        text_report,
        text,
    } = prepared;
    let image = checked.image;
    let page = checked.page();
    let profile = checked.profile;
    let directory = &checked.directory;
    let initial_digest = checked.initial_digest;
    let at = At {
        page: Some(image.page_number),
        image: Some(image.image_number),
        offset: Some(image.payload.offset),
    };
    // TextComposer proved the packed byte count; PageOrSink rechecks it
    // before forwarding the first combined row to the PDF image stream.
    let mut rows = document
        .begin_bilevel_image(BilevelImageSpec {
            pixel_width: page.width,
            pixel_height: page.height,
            row_stride: page.row_stride,
        })
        .await
        .map_err(|error| at.pdf(error))?;
    let mut page_sink = PageOrSink::new(
        profile,
        text_report,
        text,
        &mut rows,
        limits,
        cancellation,
        options.page_compose,
    )
    .map_err(|error| at.stage(Type3Stage::PageCompose, error))?;
    let generic_result = MqContexts::new(1024, limits, &options.mq);
    let generic_at = at.with_offset(directory.segments[4].data.offset);
    let mut generic_contexts = work_stage(generic_result, generic_at, Type3Stage::Contexts)?;
    let generic_result = async {
        let mut decoder = GenericRegionDecoder::new(
            source,
            &directory.segments[4],
            table,
            &mut generic_contexts,
            &mut page_sink,
            limits,
            cancellation,
            options.mq,
            options.generic,
        )
        .await?;
        decoder.arm_page_output(profile.generic_header())?;
        while decoder.decode_next_row().await? {}
        decoder.finish().await
    }
    .await;
    let generic_report = generic_result.map_err(|error| {
        if let Some(failure) = page_sink.take_failure() {
            at.stage(Type3Stage::PageCompose, failure)
        } else {
            at.with_offset(error.offset)
                .stage(Type3Stage::GenericRegion, error)
        }
    })?;
    let page_compose = page_sink
        .finish(&generic_report)
        .await
        .map_err(|error| at.stage(Type3Stage::PageCompose, error))?;
    drop(page_sink);
    if digest_span(source, image, limits, cancellation, at).await? != initial_digest {
        return Err(at.error(Type3PdfErrorKind::SourceChanged));
    }
    let object = rows.finish().await.map_err(|error| at.pdf(error))?;
    Ok((object, page_compose))
}

fn code_length(symbols: u64) -> u32 {
    if symbols <= 1 {
        0
    } else {
        64 - (symbols - 1).leading_zeros()
    }
}

async fn read_second_new_symbol_count<S: RangedSource, C: Cancellation>(
    directory: &crate::jbig2::SegmentDirectory,
    source: &mut S,
    limits: &Limits,
    cancellation: &C,
    budget: DictionaryBudget,
    at: At,
) -> Result<u32, Type3PdfError> {
    use crate::jbig2::dictionary::read_dictionary_data_header;
    let result = source_stage(
        read_dictionary_data_header(source, &directory.segments[2], limits, budget, cancellation)
            .await,
        at,
        Type3Stage::SecondDictionary,
        |error| error.offset,
    )?;
    Ok(result.new_symbols)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hnc8::ErrorKind;
    use crate::jbig2::{
        text_composer::TextComposeProgress,
        text_instances::{TextInstanceError, TextInstanceErrorKind, TextInstanceProgress},
    };
    use crate::test_support::ready;
    use std::error::Error as _;

    fn invalid_input() -> Error {
        Error::InvalidInput {
            reason: "invented fault",
        }
    }

    #[test]
    fn located_errors_keep_source_chain_and_distinct_refusal_messages() {
        let container = Hnc8Error {
            variant: Some(Variant::HnA),
            offset: 77,
            page: Some(2),
            image: Some(3),
            kind: ErrorKind::Malformed {
                field: "invented field",
                reason: "invented fault",
            },
        };
        let examples = [
            (
                Type3PdfErrorKind::InvalidOptions("bad scale"),
                "invalid options",
                false,
            ),
            (
                Type3PdfErrorKind::InvalidSelection("bad index"),
                "invalid selection",
                false,
            ),
            (
                Type3PdfErrorKind::Container(Box::new(container)),
                "HN/C8 HN-A",
                true,
            ),
            (
                Type3PdfErrorKind::UnsupportedImageType(2),
                "record type 2",
                false,
            ),
            (
                Type3PdfErrorKind::DibMalformed("bad palette"),
                "malformed type-3 DIB",
                false,
            ),
            (Type3PdfErrorKind::Source(invalid_input()), "source:", true),
            (
                Type3PdfErrorKind::SourceChanged,
                "changed between passes",
                false,
            ),
            (
                Type3PdfErrorKind::Workspace("dirty store"),
                "workspace:",
                false,
            ),
            (
                Type3PdfErrorKind::Stage {
                    stage: Type3Stage::Directory,
                    source: Box::new(invalid_input()),
                },
                "Directory:",
                true,
            ),
            (Type3PdfErrorKind::Pdf(invalid_input()), "PDF output:", true),
        ];
        for (kind, fragment, chained) in examples {
            let error = Type3PdfError {
                page: Some(2),
                image: Some(3),
                offset: Some(77),
                kind,
            };
            let message = error.to_string();
            assert!(message.contains("page 2, image 3, source byte 77"));
            assert!(message.contains(fragment), "{message}");
            assert_eq!(error.source().is_some(), chained);
        }
        let unlocated = At::NONE.error(Type3PdfErrorKind::InvalidOptions("bad scale"));
        assert_eq!(
            unlocated.to_string(),
            "HN/C8 type-3 PDF conversion: invalid options: bad scale"
        );
        assert_eq!(code_length(0), 0);
        assert_eq!(code_length(1), 0);
        assert_eq!(code_length(2), 1);
        assert_eq!(code_length(5), 3);
    }

    #[test]
    fn text_instance_failure_uses_its_absolute_source_offset() {
        let at = At {
            page: Some(4),
            image: Some(2),
            offset: Some(100),
        };
        let instance = TextInstanceError {
            segment: 3,
            offset: 555,
            progress: Box::new(TextInstanceProgress::default()),
            kind: TextInstanceErrorKind::Malformed("invented terminal"),
        };
        let nested = TextComposeError {
            segment: 3,
            offset: 7,
            progress: Box::new(TextComposeProgress::default()),
            kind: TextComposeErrorKind::Instance(Box::new(instance)),
        };
        let error = composed_stage::<()>(Err(nested), at).unwrap_err();
        assert_eq!(
            (error.page, error.image, error.offset),
            (Some(4), Some(2), Some(555))
        );
        assert!(error.to_string().contains("source byte 555"));

        let scratch = TextComposeError {
            segment: 3,
            offset: 7,
            progress: Box::new(TextComposeProgress::default()),
            kind: TextComposeErrorKind::Scratch(invalid_input()),
        };
        let error = composed_stage::<()>(Err(scratch), at).unwrap_err();
        assert_eq!(error.offset, Some(100));
        assert!(error.to_string().contains("source byte 100"));
    }

    struct ByteReader(Vec<u8>);

    impl RangedSource for ByteReader {
        fn size(&self) -> u64 {
            self.0.len() as u64
        }

        async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> crate::Result<usize> {
            let start = offset as usize;
            let count = destination.len().min(self.0.len().saturating_sub(start));
            destination[..count].copy_from_slice(&self.0[start..start + count]);
            Ok(count)
        }
    }

    struct ShortWriter {
        bytes: Vec<u8>,
        fail: bool,
    }

    impl SequentialSink for ShortWriter {
        async fn write(&mut self, bytes: &[u8]) -> crate::Result<usize> {
            if self.fail {
                return Err(invalid_input());
            }
            let count = bytes.len().min(1);
            self.bytes.extend_from_slice(&bytes[..count]);
            Ok(count)
        }

        async fn flush(&mut self) -> crate::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn bitmap_revision_advances_only_when_a_store_accepts_bytes() {
        let revision = Rc::new(Cell::new(0));
        let mut reader = ByteReader(vec![0x80]);
        let mut viewed = RevisionedSource {
            inner: &mut reader,
            revision: revision.clone(),
        };
        let mut byte = [0];
        assert_eq!(viewed.size(), 1);
        assert_eq!(ready(viewed.read_at(0, &mut byte)).unwrap(), 1);
        assert_eq!(byte, [0x80]);
        assert_eq!(viewed.revision().unwrap(), 0);

        let mut store = ShortWriter {
            bytes: Vec::new(),
            fail: false,
        };
        let mut writer = RevisionedSink {
            inner: &mut store,
            revision,
        };
        assert_eq!(ready(writer.write(&[0xaa, 0xbb])).unwrap(), 1);
        assert_eq!(viewed.revision().unwrap(), 1);
        assert_eq!(ready(writer.write(&[])).unwrap(), 0);
        assert_eq!(viewed.revision().unwrap(), 1);
        ready(writer.flush()).unwrap();
        writer.inner.fail = true;
        assert!(ready(writer.write(&[0xcc])).is_err());
        assert_eq!(viewed.revision().unwrap(), 1);
        assert_eq!(writer.inner.bytes, [0xaa]);
    }
    mod fixture {
        include!("../../tests/common/type3_fixture.rs");
    }

    #[derive(Clone, Default)]
    struct Memory(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);

    impl RangedSource for Memory {
        fn size(&self) -> u64 {
            self.0.borrow().len() as u64
        }
        async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> crate::Result<usize> {
            let bytes = self.0.borrow();
            let start = offset as usize;
            let count = destination
                .len()
                .min(bytes.len().saturating_sub(start))
                .min(2);
            destination[..count].copy_from_slice(&bytes[start..start + count]);
            Ok(count)
        }
    }

    impl SequentialSink for Memory {
        async fn write(&mut self, bytes: &[u8]) -> crate::Result<usize> {
            let count = bytes.len().min(3);
            self.0.borrow_mut().extend_from_slice(&bytes[..count]);
            Ok(count)
        }
        async fn flush(&mut self) -> crate::Result<()> {
            Ok(())
        }
    }

    impl RandomAccessScratch for Memory {
        fn size(&self) -> crate::Result<u64> {
            Ok(RangedSource::size(self))
        }
        async fn set_len(&mut self, length: u64) -> crate::Result<()> {
            self.0.borrow_mut().resize(length as usize, 0);
            Ok(())
        }
        async fn read_at(&mut self, offset: u64, bytes: &mut [u8]) -> crate::Result<usize> {
            RangedSource::read_at(self, offset, bytes).await
        }
        async fn write_at(&mut self, offset: u64, bytes: &[u8]) -> crate::Result<usize> {
            let start = offset as usize;
            self.0.borrow_mut()[start..start + bytes.len()].copy_from_slice(bytes);
            Ok(bytes.len())
        }
        async fn flush(&mut self) -> crate::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn shared_emitter_appends_two_asymmetric_images_to_an_existing_document() {
        use crate::{
            NeverCancel,
            hnc8::Span,
            jbig2::mq::{MQ_STATE_COUNT, MqState},
        };
        let limits = Limits::default();
        let options = Type3PdfOptions::default();
        let table = MqTable::new(
            vec![
                MqState {
                    qe: 1,
                    next_mps: 0,
                    next_lps: 0,
                    switch_mps: false,
                };
                MQ_STATE_COUNT
            ],
            &limits,
        )
        .unwrap();
        let mut sink = Memory::default();
        let output = sink.clone();
        let mut document = ready(PdfDocument::new(&mut sink, &limits, &NeverCancel)).unwrap();
        let size = PageSpec {
            width_points: 30.0,
            height_points: 20.0,
        };
        let mut placements = Vec::new();
        let mut scratch = Memory::default();
        for (number, width, height) in [(1, 3, 2), (2, 9, 3)] {
            let bytes = fixture::payload(width, height, 0x10);
            let image = ImageRecord {
                page_number: 1,
                image_number: number,
                descriptor_offset: 0,
                record_type: 3,
                payload: Span {
                    offset: 0,
                    length: bytes.len() as u64,
                },
            };
            let mut source = ByteReader(bytes);
            let checked = ready(preflight_type3(
                &mut source,
                image,
                options,
                &limits,
                &NeverCancel,
            ))
            .unwrap();
            assert_eq!(
                (checked.page().width, checked.page().height),
                (width, height)
            );
            let mut first = Memory::default();
            let (mut first_reader, mut first_compose) = (first.clone(), first.clone());
            let mut second = Memory::default();
            let (mut second_reader, mut second_compose) = (second.clone(), second.clone());
            let mut refined = Memory::default();
            let mut refined_reader = refined.clone();
            let mut workspaces = Type3Workspaces {
                first: Type3Store {
                    reader: &mut first_reader,
                    compose_reader: &mut first_compose,
                    writer: &mut first,
                },
                second: Type3Store {
                    reader: &mut second_reader,
                    compose_reader: &mut second_compose,
                    writer: &mut second,
                },
                refined: Type3RefinedStore {
                    reader: &mut refined_reader,
                    writer: &mut refined,
                },
                text: &mut scratch,
            };
            let before = output.0.borrow().len();
            let prepared = ready(prepare_type3_image(
                &mut source,
                &table,
                &mut workspaces,
                checked,
                options,
                &limits,
                &NeverCancel,
            ))
            .unwrap();
            assert_eq!(
                output.0.borrow().len(),
                before,
                "preparation must not write PDF bytes"
            );
            let (object, report) = ready(emit_type3_xobject(
                &mut source,
                &mut document,
                &table,
                prepared,
                options,
                &limits,
                &NeverCancel,
            ))
            .unwrap();
            assert_eq!(report.text_header_anomaly, None);
            if number == 1 {
                ready(document.add_page(size, &[object])).unwrap();
            }
            placements.push(crate::pdf::ImagePlacement {
                image: object,
                transform: [
                    f64::from(width),
                    0.0,
                    0.0,
                    f64::from(height),
                    f64::from(number * 10),
                    0.0,
                ],
            });
            ready(scratch.set_len(0)).unwrap();
        }
        ready(document.add_placed_page(size, &placements)).unwrap();
        let report = ready(document.finish()).unwrap();
        assert_eq!(report.pages_converted, 2);
        let bytes = output.0.borrow();
        assert_eq!(report.output_bytes_written, bytes.len() as u64);
        let text = String::from_utf8_lossy(&bytes);
        assert_eq!(text.matches("%PDF-").count(), 1);
        assert_eq!(text.matches("%%EOF").count(), 1);
        assert_eq!(text.matches("/Subtype /Image").count(), 2);
        for (width, height, expected) in
            [(3, 2, &[0x80, 0][..]), (9, 3, &[0x80, 0, 0, 0, 0, 0][..])]
        {
            let marker = format!("/Width {width}\n/Height {height}");
            let start = bytes
                .windows(marker.len())
                .position(|part| part == marker.as_bytes())
                .unwrap();
            let stream = bytes[start..]
                .windows(8)
                .position(|part| part == b"\nstream\n")
                .unwrap()
                + start
                + 8;
            assert_eq!(&bytes[stream..stream + expected.len()], expected);
        }
        assert!(text.contains("3 0 0 2 10 0 cm"));
        assert!(text.contains("9 0 0 3 20 0 cm"));
    }
}
