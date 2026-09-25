// SPDX-License-Identifier: MIT

//! Bounded metadata traversal for the three independently measured HN/C8
//! container profiles. Image payloads and text are never loaded here.
//! [`convert_type0_pdf`] builds bounded PDF pages from type-0 records.

mod convert;

pub use convert::{
    MultipleImages, Type0PdfError, Type0PdfErrorKind, Type0PdfOptions, Type0PdfReport,
    convert_type0_pdf,
};

use crate::jbig1::Type0Span;
use crate::{Cancellation, Error, Limits, RangedSource, read_exact_at};
use std::{error, fmt};

const PAGE_ROW_BYTES: u64 = 20;
const IMAGE_RECORD_BYTES: u64 = 12;
const OUTLINE_RECORD_BYTES: u64 = 308;

/// The three measured container layouts; this is not a complete HN standard.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Variant {
    C8,
    HnA,
    HnB,
}

impl Variant {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::C8 => "C8",
            Self::HnA => "HN-A",
            Self::HnB => "HN-B",
        }
    }
}

/// A checked absolute interval in the source. The end is exclusive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Span {
    pub offset: u64,
    pub length: u64,
}

impl Span {
    pub fn checked_end(self) -> Option<u64> {
        self.offset.checked_add(self.length)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Header {
    pub variant: Variant,
    pub page_count: u32,
    pub page_index: Span,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageRecord {
    pub page_number: u32,
    pub row_offset: u64,
    pub text: Span,
    pub image_count: u32,
    /// Uninterpreted bytes at page-row offsets +10 through +19.
    pub unknown: [u8; 10],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageRecord {
    pub page_number: u32,
    pub image_number: u32,
    pub descriptor_offset: u64,
    /// Signed descriptor field at +0, checked to be in the observed 0..=3 set.
    pub record_type: u32,
    pub payload: Span,
}

impl ImageRecord {
    /// Pass exactly the outer type-0 descriptor and absolute payload span to
    /// the bounded row decoder. Other observed image types remain metadata.
    pub fn type0_span(self) -> Option<Type0Span> {
        (self.record_type == 0).then_some(Type0Span {
            record_type: self.record_type,
            offset: self.payload.offset,
            length: self.payload.length,
        })
    }
}

/// Format-specific ceilings independent of the shared `Limits` contract.
/// `max_input_bytes` bounds the entire selected source at open, while these
/// text/image limits bound each declared span separately.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Budget {
    pub max_outline_records: u32,
    pub max_images_per_page: u32,
    pub max_images_total: u64,
    pub max_text_span_bytes: u64,
    pub max_image_span_bytes: u64,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_outline_records: 100_000,
            max_images_per_page: 8_192,
            max_images_total: 1_000_000,
            max_text_span_bytes: 64 * 1024 * 1024,
            max_image_span_bytes: 64 * 1024 * 1024,
        }
    }
}

#[derive(Debug)]
pub enum ErrorKind {
    Malformed {
        field: &'static str,
        reason: &'static str,
    },
    Unsupported {
        field: &'static str,
        value: u64,
    },
    Truncated {
        field: &'static str,
        expected: u64,
        available: u64,
    },
    LimitExceeded {
        resource: &'static str,
        limit: u64,
        attempted: u64,
    },
    Source {
        field: &'static str,
        source: Error,
    },
    Cancelled,
    IncompletePage,
    NoCurrentPage,
    Poisoned,
}

impl ErrorKind {
    /// Stable field label for diagnostics and external metadata checks.
    pub const fn field(&self) -> &'static str {
        match self {
            Self::Malformed { field, .. }
            | Self::Unsupported { field, .. }
            | Self::Truncated { field, .. }
            | Self::Source { field, .. } => field,
            Self::LimitExceeded { resource, .. } => resource,
            Self::Cancelled => "cancellation",
            Self::IncompletePage => "image count",
            Self::NoCurrentPage => "page cursor",
            Self::Poisoned => "reader state",
        }
    }

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Malformed { .. } => "malformed",
            Self::Unsupported { .. } => "unsupported",
            Self::Truncated { .. } => "truncated",
            Self::LimitExceeded { .. } => "limit",
            Self::Source { .. } => "source",
            Self::Cancelled => "cancelled",
            Self::IncompletePage => "incomplete_page",
            Self::NoCurrentPage => "no_current_page",
            Self::Poisoned => "poisoned",
        }
    }
}

#[derive(Debug)]
pub struct Hnc8Error {
    pub variant: Option<Variant>,
    pub offset: u64,
    pub page: Option<u32>,
    pub image: Option<u32>,
    pub kind: ErrorKind,
}

pub type Result<T> = std::result::Result<T, Hnc8Error>;

impl fmt::Display for Hnc8Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HN/C8")?;
        if let Some(variant) = self.variant {
            write!(f, " {}", variant.as_str())?;
        }
        write!(f, " at byte {}", self.offset)?;
        if let Some(page) = self.page {
            write!(f, ", page {page}")?;
        }
        if let Some(image) = self.image {
            write!(f, ", image {image}")?;
        }
        write!(f, ": {}", self.kind)
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed { field, reason } => write!(f, "malformed {field}: {reason}"),
            Self::Unsupported { field, value } => write!(f, "unsupported {field}: {value}"),
            Self::Truncated {
                field,
                expected,
                available,
            } => write!(
                f,
                "truncated {field}: expected {expected} bytes, available {available}"
            ),
            Self::LimitExceeded {
                resource,
                limit,
                attempted,
            } => write!(f, "{resource} limit {limit} exceeded by {attempted}"),
            Self::Source { field, source } => write!(f, "{field} source error: {source}"),
            Self::Cancelled => f.write_str("cancelled"),
            Self::IncompletePage => f.write_str("page has unread image records"),
            Self::NoCurrentPage => f.write_str("no current page"),
            Self::Poisoned => f.write_str("reader is poisoned after an interrupted or failed read"),
        }
    }
}

impl error::Error for Hnc8Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.kind {
            ErrorKind::Source { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
struct Location {
    variant: Option<Variant>,
    offset: u64,
    page: Option<u32>,
    image: Option<u32>,
}

impl Location {
    fn at(self, offset: u64) -> Self {
        Self { offset, ..self }
    }
    fn error(self, kind: ErrorKind) -> Hnc8Error {
        Hnc8Error {
            variant: self.variant,
            offset: self.offset,
            page: self.page,
            image: self.image,
            kind,
        }
    }
    fn malformed(self, field: &'static str, reason: &'static str) -> Hnc8Error {
        self.error(ErrorKind::Malformed { field, reason })
    }
    fn limit(self, resource: &'static str, limit: u64, attempted: u64) -> Hnc8Error {
        self.error(ErrorKind::LimitExceeded {
            resource,
            limit,
            attempted,
        })
    }
}

fn checked_span(
    source_size: u64,
    offset: u64,
    length: u64,
    loc: Location,
    field: &'static str,
) -> Result<Span> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| loc.malformed(field, "end overflows u64"))?;
    if offset > source_size || end > source_size {
        return Err(loc.error(ErrorKind::Truncated {
            field,
            expected: length,
            available: source_size.saturating_sub(offset),
        }));
    }
    Ok(Span { offset, length })
}

fn signed32(bytes: &[u8]) -> i32 {
    i32::from_le_bytes(bytes.try_into().expect("fixed field width"))
}

fn signed16(bytes: &[u8]) -> i16 {
    i16::from_le_bytes(bytes.try_into().expect("fixed field width"))
}

fn nonnegative32(value: i32, loc: Location, field: &'static str) -> Result<u64> {
    u64::try_from(value).map_err(|_| loc.malformed(field, "negative signed value"))
}

async fn read_fixed<S: RangedSource, C: Cancellation>(
    source: &mut S,
    limits: &Limits,
    cancellation: &C,
    offset: u64,
    bytes: &mut [u8],
    loc: Location,
    field: &'static str,
) -> Result<()> {
    checked_span(source.size(), offset, bytes.len() as u64, loc, field)?;
    let mut done = 0;
    while done < bytes.len() {
        let count = (bytes.len() - done).min(limits.io_chunk_bytes);
        let current = offset + done as u64;
        read_exact_at(
            source,
            current,
            &mut bytes[done..done + count],
            limits,
            cancellation,
        )
        .await
        .map_err(|error| match error {
            Error::Cancelled => loc.at(current).error(ErrorKind::Cancelled),
            Error::TruncatedInput { available, .. } => {
                loc.at(current).error(ErrorKind::Truncated {
                    field,
                    expected: count as u64,
                    available,
                })
            }
            other => loc.at(current).error(ErrorKind::Source {
                field,
                source: other,
            }),
        })?;
        done += count;
    }
    Ok(())
}

fn overlaps_protected(span: Span, protected_end: u64) -> bool {
    span.length > 0 && span.offset < protected_end
}

#[derive(Clone, Copy)]
struct CurrentPage {
    page: PageRecord,
    next_image: u32,
    next_descriptor: u64,
}

/// A one-page-at-a-time cursor. A failed or dropped read poisons the cursor.
/// The source remains borrowable for type-0 decoding between records.
pub struct Hnc8Reader<'a, S: RangedSource, C: Cancellation> {
    source: &'a mut S,
    limits: &'a Limits,
    cancellation: &'a C,
    budget: Budget,
    header: Header,
    next_page: u32,
    current: Option<CurrentPage>,
    declared_images: u64,
    poisoned: bool,
}

impl<'a, S: RangedSource, C: Cancellation> Hnc8Reader<'a, S, C> {
    pub async fn open(
        source: &'a mut S,
        limits: &'a Limits,
        cancellation: &'a C,
        budget: Budget,
    ) -> Result<Self> {
        Self::open_starting_at(source, limits, cancellation, budget, 1).await
    }

    /// Open a fresh diagnostic cursor at one page-index row.
    ///
    /// This deliberately skips earlier pages so that malformed pages can be
    /// inspected independently. Its total-image budget covers only the
    /// selected suffix, not the whole document. Conversion must use `open`.
    pub async fn probe_at_page(
        source: &'a mut S,
        limits: &'a Limits,
        cancellation: &'a C,
        budget: Budget,
        start_page: u32,
    ) -> Result<Self> {
        Self::open_starting_at(source, limits, cancellation, budget, start_page).await
    }

    async fn open_starting_at(
        source: &'a mut S,
        limits: &'a Limits,
        cancellation: &'a C,
        budget: Budget,
        start_page: u32,
    ) -> Result<Self> {
        let base = Location {
            variant: None,
            offset: 0,
            page: None,
            image: None,
        };
        limits.validate().map_err(|source| {
            base.error(ErrorKind::Source {
                field: "limits",
                source,
            })
        })?;
        if cancellation.is_cancelled() {
            return Err(base.error(ErrorKind::Cancelled));
        }
        if source.size() > limits.max_input_bytes {
            return Err(base.limit("source bytes", limits.max_input_bytes, source.size()));
        }
        let mut magic = [0; 4];
        read_fixed(
            source,
            limits,
            cancellation,
            0,
            &mut magic,
            base,
            "signature",
        )
        .await?;
        let (variant, count_offset, index_start): (Variant, u64, u64) = match magic {
            [0xc8, 0, 0, 0] => (Variant::C8, 0x08, 0x50),
            [b'H', b'N', 0, 0] => {
                let mut marker = [0; 4];
                read_fixed(
                    source,
                    limits,
                    cancellation,
                    4,
                    &mut marker,
                    base.at(4),
                    "HN marker",
                )
                .await?;
                match marker {
                    [0x90, 0x01, 0, 0] => (Variant::HnA, 0x90, 0x15c),
                    [0xc8, 0, 0, 0] => (Variant::HnB, 0x90, 0xd8),
                    _ => {
                        return Err(base.at(4).error(ErrorKind::Unsupported {
                            field: "HN marker",
                            value: u64::from(u32::from_le_bytes(marker)),
                        }));
                    }
                }
            }
            _ => {
                return Err(base.error(ErrorKind::Unsupported {
                    field: "signature",
                    value: u64::from(u32::from_le_bytes(magic)),
                }));
            }
        };
        let loc = Location {
            variant: Some(variant),
            ..base
        };
        let mut count = [0; 4];
        read_fixed(
            source,
            limits,
            cancellation,
            count_offset,
            &mut count,
            loc.at(count_offset),
            "page count",
        )
        .await?;
        let signed_count = signed32(&count);
        if signed_count <= 0 {
            return Err(loc
                .at(count_offset)
                .malformed("page count", "must be positive"));
        }
        let page_count = signed_count as u32;
        if page_count > limits.max_pages {
            return Err(loc.at(count_offset).limit(
                "pages",
                u64::from(limits.max_pages),
                u64::from(page_count),
            ));
        }
        let index_start = if variant == Variant::HnA {
            let mut outline = [0; 4];
            read_fixed(
                source,
                limits,
                cancellation,
                0x158,
                &mut outline,
                loc.at(0x158),
                "outline count",
            )
            .await?;
            let outline_count = nonnegative32(signed32(&outline), loc.at(0x158), "outline count")?;
            if outline_count > u64::from(budget.max_outline_records) {
                return Err(loc.at(0x158).limit(
                    "outline records",
                    u64::from(budget.max_outline_records),
                    outline_count,
                ));
            }
            let outline_bytes =
                outline_count
                    .checked_mul(OUTLINE_RECORD_BYTES)
                    .ok_or_else(|| {
                        loc.at(0x158)
                            .malformed("outline count", "byte count overflows")
                    })?;
            index_start
                .checked_add(outline_bytes)
                .ok_or_else(|| loc.at(0x158).malformed("page index", "start overflows"))?
        } else {
            index_start
        };
        let index_length = u64::from(page_count)
            .checked_mul(PAGE_ROW_BYTES)
            .ok_or_else(|| {
                loc.at(count_offset)
                    .malformed("page count", "index bytes overflow")
            })?;
        let page_index = checked_span(
            source.size(),
            index_start,
            index_length,
            loc.at(index_start),
            "page index",
        )?;
        if start_page == 0 || start_page > page_count {
            return Err(loc
                .at(index_start)
                .malformed("page number", "outside declared page index"));
        }
        Ok(Self {
            source,
            limits,
            cancellation,
            budget,
            header: Header {
                variant,
                page_count,
                page_index,
            },
            next_page: start_page,
            current: None,
            declared_images: 0,
            poisoned: false,
        })
    }

    pub fn header(&self) -> Header {
        self.header
    }

    pub fn source_mut(&mut self) -> &mut S {
        self.source
    }

    pub async fn next_page(&mut self) -> Result<Option<PageRecord>> {
        let loc = Location {
            variant: Some(self.header.variant),
            offset: self.header.page_index.offset,
            page: Some(self.next_page),
            image: None,
        };
        if self.poisoned {
            return Err(loc.error(ErrorKind::Poisoned));
        }
        if let Some(current) = self
            .current
            .filter(|page| page.next_image <= page.page.image_count)
        {
            return Err(Location {
                page: Some(current.page.page_number),
                image: Some(current.next_image),
                offset: current.next_descriptor,
                ..loc
            }
            .error(ErrorKind::IncompletePage));
        }
        if self.next_page > self.header.page_count {
            return Ok(None);
        }
        if self.cancellation.is_cancelled() {
            return Err(loc.error(ErrorKind::Cancelled));
        }
        let page_number = self.next_page;
        let row_offset =
            self.header.page_index.offset + u64::from(page_number - 1) * PAGE_ROW_BYTES;
        let loc = loc.at(row_offset);
        self.poisoned = true;
        let mut row = [0; PAGE_ROW_BYTES as usize];
        read_fixed(
            self.source,
            self.limits,
            self.cancellation,
            row_offset,
            &mut row,
            loc,
            "page row",
        )
        .await?;
        let text_offset = nonnegative32(signed32(&row[..4]), loc, "text offset")?;
        let text_length =
            nonnegative32(signed32(&row[4..8]), loc.at(row_offset + 4), "text length")?;
        let text = checked_span(
            self.source.size(),
            text_offset,
            text_length,
            loc.at(row_offset),
            "text span",
        )?;
        if text_length > self.budget.max_text_span_bytes {
            return Err(loc.at(row_offset + 4).limit(
                "text span bytes",
                self.budget.max_text_span_bytes,
                text_length,
            ));
        }
        let signed_images = signed16(&row[8..10]);
        if signed_images < 0 {
            return Err(loc
                .at(row_offset + 8)
                .malformed("image count", "negative signed value"));
        }
        let image_count = signed_images as u32;
        if image_count > self.budget.max_images_per_page {
            return Err(loc.at(row_offset + 8).limit(
                "images per page",
                u64::from(self.budget.max_images_per_page),
                u64::from(image_count),
            ));
        }
        let total = self
            .declared_images
            .checked_add(u64::from(image_count))
            .ok_or_else(|| {
                loc.at(row_offset + 8)
                    .malformed("image count", "total overflows")
            })?;
        if total > self.budget.max_images_total {
            return Err(loc.at(row_offset + 8).limit(
                "images total",
                self.budget.max_images_total,
                total,
            ));
        }
        let mut unknown = [0; 10];
        unknown.copy_from_slice(&row[10..]);
        let page = PageRecord {
            page_number,
            row_offset,
            text,
            image_count,
            unknown,
        };
        self.declared_images = total;
        self.current = Some(CurrentPage {
            page,
            next_image: 1,
            next_descriptor: text.checked_end().expect("checked text span"),
        });
        self.next_page += 1;
        self.poisoned = false;
        Ok(Some(page))
    }

    pub async fn next_image(&mut self) -> Result<Option<ImageRecord>> {
        let loc = Location {
            variant: Some(self.header.variant),
            offset: self.header.page_index.offset,
            page: self.current.map(|current| current.page.page_number),
            image: self.current.map(|current| current.next_image),
        };
        if self.poisoned {
            return Err(loc.error(ErrorKind::Poisoned));
        }
        let current = self
            .current
            .ok_or_else(|| loc.error(ErrorKind::NoCurrentPage))?;
        if current.next_image > current.page.image_count {
            return Ok(None);
        }
        if self.cancellation.is_cancelled() {
            return Err(loc.error(ErrorKind::Cancelled));
        }
        let descriptor_offset = current.next_descriptor;
        let loc = loc.at(descriptor_offset);
        self.poisoned = true;
        let descriptor = checked_span(
            self.source.size(),
            descriptor_offset,
            IMAGE_RECORD_BYTES,
            loc,
            "image descriptor",
        )?;
        if overlaps_protected(
            descriptor,
            self.header
                .page_index
                .checked_end()
                .expect("checked page index"),
        ) {
            return Err(loc.malformed("image descriptor", "overlaps header or page index"));
        }
        let mut bytes = [0; IMAGE_RECORD_BYTES as usize];
        read_fixed(
            self.source,
            self.limits,
            self.cancellation,
            descriptor_offset,
            &mut bytes,
            loc,
            "image descriptor",
        )
        .await?;
        let signed_type = signed32(&bytes[..4]);
        if signed_type < 0 {
            return Err(loc.malformed("image type", "negative signed value"));
        }
        if signed_type > 3 {
            return Err(loc.error(ErrorKind::Unsupported {
                field: "image type",
                value: signed_type as u64,
            }));
        }
        let payload_offset = nonnegative32(
            signed32(&bytes[4..8]),
            loc.at(descriptor_offset + 4),
            "image offset",
        )?;
        let payload_length = nonnegative32(
            signed32(&bytes[8..12]),
            loc.at(descriptor_offset + 8),
            "image length",
        )?;
        if payload_length == 0 {
            return Err(loc
                .at(descriptor_offset + 8)
                .malformed("image length", "zero-length payload"));
        }
        let payload = checked_span(
            self.source.size(),
            payload_offset,
            payload_length,
            loc.at(descriptor_offset + 4),
            "image payload",
        )?;
        if payload_length > self.budget.max_image_span_bytes {
            return Err(loc.at(descriptor_offset + 8).limit(
                "image span bytes",
                self.budget.max_image_span_bytes,
                payload_length,
            ));
        }
        if payload.offset < descriptor.checked_end().expect("checked descriptor span") {
            return Err(loc.at(descriptor_offset + 4).malformed(
                "image offset",
                "payload overlaps descriptor or chain regresses",
            ));
        }
        if overlaps_protected(
            payload,
            self.header
                .page_index
                .checked_end()
                .expect("checked page index"),
        ) {
            return Err(loc
                .at(descriptor_offset + 4)
                .malformed("image payload", "overlaps header or page index"));
        }
        let record_type = signed_type as u32;
        let record = ImageRecord {
            page_number: current.page.page_number,
            image_number: current.next_image,
            descriptor_offset,
            record_type,
            payload,
        };
        self.current = Some(CurrentPage {
            next_image: current.next_image + 1,
            next_descriptor: payload.checked_end().expect("checked image span"),
            ..current
        });
        self.poisoned = false;
        Ok(Some(record))
    }
}
