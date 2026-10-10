// SPDX-License-Identifier: MIT

//! Bounded metadata traversal for the three independently measured HN/C8
//! container profiles. Image payloads and text are never loaded here.
//! [`convert_document_pdf`] and [`convert_source_pages_pdf`] convert whole
//! documents; type-0, JPEG and type-3 images are checked and emitted by the
//! one shared composition pipeline. Composition validates the observed text
//! frame while retaining only raw image-coordinate words. Empirical geometry remains diagnostic.
//! [`Hnc8Reader::application_info`] reads the trailing C8 application-info
//! package's DOI, URL and note count.

mod appinfo;
mod compose;
mod inflate;
mod jpeg;
mod native;
mod native_page;
mod outline;
mod placement;
mod structure;
mod text;
mod type3_image;

pub use appinfo::{
    ApplicationInfo, ApplicationInfoDefect, ApplicationInfoReport, ApplicationInfoStatus,
    MAX_APPLICATION_INFO_BYTES, MAX_APPLICATION_INFO_FIELD_BYTES,
};
pub use compose::{
    C8FontSource, C8FontSources, ComposeOptions, ComposePage, ComposeReport, ComposeVisitor,
    ComposedImage, convert_document_pdf, convert_source_pages_pdf, uses_native_text,
};
pub use jpeg::{JpegColor, JpegInfo, read_type2_jpeg_info};
pub(crate) use native::{
    NativeRecord, NativeRecordVisitor, decode_native_character, decode_native_character_for_mode,
    decode_native_image_coordinate,
};
#[cfg(test)]
pub(crate) use native_page::labelled_font;
pub(crate) use native_page::write_c8_native_page;
pub use native_page::{
    C8_DEFAULT_DECORATION_ALIAS, C8PageFonts, NativeSymbolGlyph, SymbolFontIdentity,
    is_mode_zero_symbol,
};
pub use outline::{MAX_RECORDED_OUTLINE_DEFECTS, OutlineDefect, OutlineRepair, OutlineReport};
pub(crate) use placement::{
    C8GlyphClass, EMPIRICAL_COORDINATE_POINTS_PER_UNIT, EmpiricalPageGeometry,
    empirical_c8_horizontal_decoration, empirical_c8_segment, empirical_image_transform,
    empirical_page_from_pixels,
};
pub use structure::{ApplicationInfoTail, TextFraming, TextStructure};
pub use text::RawTextCoordinate;
pub(crate) use text::TEXT_DECODER_RESERVATION_BYTES;

use crate::jbig1::Type0Span;
use crate::{
    Cancellation, Context, Error, ErrorKind, Hnc8Stage, Limits, RangedSource, Result, read_exact_at,
};

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

/// Where a conversion error occurred: the container variant, the one-based
/// page and image numbers, and the source offset, each when known.
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

    fn with_offset(self, offset: u64) -> Self {
        Self {
            offset: Some(offset),
            ..self
        }
    }

    /// Locate `error` at the conversion `stage`. An unlocated error takes
    /// this location; an HN/C8 or JBIG2 error keeps its own offset, numbers
    /// and stage and gains the ones it lacks.
    fn locate(self, stage: Hnc8Stage, error: Error) -> Error {
        let context = match error.context {
            Context::None => Context::Hnc8 {
                variant: self.variant,
                page: self.page,
                image: self.image,
                segment: None,
                stage: Some(stage),
            },
            Context::Jbig2 { segment } => Context::Hnc8 {
                variant: self.variant,
                page: self.page,
                image: self.image,
                segment,
                stage: Some(stage),
            },
            Context::Hnc8 {
                variant,
                page,
                image,
                segment,
                stage: own,
            } => Context::Hnc8 {
                variant: variant.or(self.variant),
                page: page.or(self.page),
                image: image.or(self.image),
                segment,
                stage: own.or(Some(stage)),
            },
            _ => return error,
        };
        Error {
            offset: error.offset.or(self.offset),
            context,
            ..error
        }
    }

    /// An unlocated error at `stage`.
    fn error(self, stage: Hnc8Stage, error: Error) -> Error {
        self.locate(stage, error)
    }

    /// A mapper that locates an error at `stage`.
    fn locator(self, stage: Hnc8Stage) -> impl FnOnce(Error) -> Error {
        move |error| self.locate(stage, error)
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
    /// Raw native mode at C8 offset 12 or HN-B offset 148; unknown for HN-A.
    /// Independently controlled modes 0 and 2 interpret some character codes
    /// differently. Preserving this word does not admit its rendering profile.
    pub native_mode: Option<u32>,
    /// Raw C8 origin at 28/30 or HN-B origin at 164/166; unknown for HN-A.
    /// The verified native-record profile subtracts these from record coordinates.
    /// This does not establish image placement or font baseline semantics.
    pub native_origin: Option<[u16; 2]>,
    /// Declared page extents in source units; native HN-B uses these too.
    pub page_size: Option<[u16; 2]>,
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
    /// For the compact HN-B profile, only +10/+11 exist; the remaining eight
    /// bytes are padding, not source data. Its admitted third word is zero.
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

    fn context(self) -> Context {
        Context::Hnc8 {
            variant: self.variant,
            page: self.page,
            image: self.image,
            segment: None,
            stage: None,
        }
    }

    /// Locate an unlocated error here.
    fn locate(self, error: Error) -> Error {
        error.or_at(self.offset, self.context())
    }

    fn malformed(self, reason: &'static str) -> Error {
        self.locate(Error::invalid(reason))
    }

    fn unsupported(self, reason: &'static str) -> Error {
        self.locate(Error::unsupported(self.offset, reason))
    }

    fn truncated(self, field: &'static str, expected: u64, available: u64) -> Error {
        self.locate(Error::truncated(self.offset, expected, available).because(field))
    }

    fn limit(self, resource: &'static str, limit: u64, attempted: u64) -> Error {
        self.locate(Error::limit(resource, limit, attempted))
    }

    fn cancelled(self) -> Error {
        self.locate(Error::cancelled())
    }
}

/// Every caller passes an offset and a length below 2^42: fixed header
/// offsets, a page index after at most `2^31` 308-byte outline records,
/// page rows within at most `2^31` 20-byte rows, and record fields and
/// descriptor cursors built from nonnegative i32 values. Their sum therefore
/// fits u64.
fn checked_span(
    source_size: u64,
    offset: u64,
    length: u64,
    loc: Location,
    field: &'static str,
) -> Result<Span> {
    debug_assert!(offset < 1 << 42 && length < 1 << 42);
    let end = offset + length;
    if offset > source_size || end > source_size {
        return Err(loc.truncated(field, length, source_size.saturating_sub(offset)));
    }
    Ok(Span { offset, length })
}

fn signed32(bytes: &[u8]) -> i32 {
    i32::from_le_bytes(bytes.try_into().expect("fixed field width"))
}

fn signed16(bytes: &[u8]) -> i16 {
    i16::from_le_bytes(bytes.try_into().expect("fixed field width"))
}

/// `reason` names the field and that it is negative.
fn nonnegative32(value: i32, loc: Location, reason: &'static str) -> Result<u64> {
    u64::try_from(value).map_err(|_| loc.malformed(reason))
}

fn read_fixed<S: RangedSource, C: Cancellation>(
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
        .map_err(|error| match error.kind {
            ErrorKind::Truncated { .. } => loc.at(current).locate(error.because(field)),
            _ => loc.at(current).locate(error),
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

/// A one-page-at-a-time cursor. A failed read leaves the cursor where it was;
/// the caller abandons it. The source remains borrowable between records.
pub struct Hnc8Reader<'a, S: RangedSource, C: Cancellation> {
    source: &'a mut S,
    limits: &'a Limits,
    cancellation: &'a C,
    header: Header,
    page_row_bytes: u64,
    next_page: u32,
    current: Option<CurrentPage>,
}

impl<'a, S: RangedSource, C: Cancellation> Hnc8Reader<'a, S, C> {
    /// Open a cursor at the first page. `limits` are validated here; each
    /// image payload is bounded by `Limits::max_allocation_bytes`.
    pub fn open(source: &'a mut S, limits: &'a Limits, cancellation: &'a C) -> Result<Self> {
        Self::open_starting_at(source, limits, cancellation, 1)
    }

    /// Open a fresh diagnostic cursor at one page-index row.
    ///
    /// This deliberately skips earlier pages so that malformed pages can be
    /// inspected independently. Full-document conversion must use `open`;
    /// the CLI's per-page structure report uses this cursor.
    pub fn probe_at_page(
        source: &'a mut S,
        limits: &'a Limits,
        cancellation: &'a C,
        start_page: u32,
    ) -> Result<Self> {
        Self::open_starting_at(source, limits, cancellation, start_page)
    }

    fn open_starting_at(
        source: &'a mut S,
        limits: &'a Limits,
        cancellation: &'a C,
        start_page: u32,
    ) -> Result<Self> {
        let base = Location {
            variant: None,
            offset: 0,
            page: None,
            image: None,
        };
        limits.validate().map_err(|source| base.locate(source))?;
        if cancellation.is_cancelled() {
            return Err(base.cancelled());
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
        )?;
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
                )?;
                match marker {
                    [0x90, 0x01, 0, 0] => (Variant::HnA, 0x90, 0x15c),
                    [0xc8, 0, 0, 0] => (Variant::HnB, 0x90, 0xd8),
                    _ => {
                        return Err(base.at(4).unsupported("HN marker"));
                    }
                }
            }
            _ => {
                return Err(base.unsupported("signature"));
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
        )?;
        let signed_count = signed32(&count);
        if signed_count <= 0 {
            return Err(loc
                .at(count_offset)
                .malformed("page count: must be positive"));
        }
        let page_count = signed_count as u32;
        if page_count > limits.max_pages {
            return Err(loc.at(count_offset).limit(
                "pages",
                u64::from(limits.max_pages),
                u64::from(page_count),
            ));
        }
        let native_mode = if variant != Variant::HnA {
            let offset = count_offset + 4;
            let mut mode = [0; 4];
            read_fixed(
                source,
                limits,
                cancellation,
                offset,
                &mut mode,
                loc.at(offset),
                "native mode",
            )?;
            Some(u32::from_le_bytes(mode))
        } else {
            None
        };
        let native_origin = if variant != Variant::HnA {
            let offset = count_offset + 20;
            let mut origin = [0; 4];
            read_fixed(
                source,
                limits,
                cancellation,
                offset,
                &mut origin,
                loc.at(offset),
                "native coordinate origin",
            )?;
            Some([
                u16::from_le_bytes([origin[0], origin[1]]),
                u16::from_le_bytes([origin[2], origin[3]]),
            ])
        } else {
            None
        };
        let page_size = {
            let offset = count_offset + 24;
            let mut size = [0; 4];
            read_fixed(
                source,
                limits,
                cancellation,
                offset,
                &mut size,
                loc.at(offset),
                "page dimensions",
            )?;
            Some([
                u16::from_le_bytes([size[0], size[1]]),
                u16::from_le_bytes([size[2], size[3]]),
            ])
        };
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
            )?;
            let outline_count = nonnegative32(
                signed32(&outline),
                loc.at(0x158),
                "outline count: negative signed value",
            )?;
            // The count is below 2^31, so the index starts below 2^42.
            let outline_bytes = outline_count * OUTLINE_RECORD_BYTES;
            index_start + outline_bytes
        } else {
            index_start
        };
        let page_row_bytes = if variant == Variant::HnB {
            let mut marker = [0; 4];
            read_fixed(
                source,
                limits,
                cancellation,
                0x88,
                &mut marker,
                loc.at(0x88),
                "HN-B page-index layout",
            )?;
            match u32::from_le_bytes(marker) {
                0 => 12,
                0xc8 => PAGE_ROW_BYTES,
                _ => {
                    return Err(loc.at(0x88).unsupported("HN-B page-index layout"));
                }
            }
        } else {
            PAGE_ROW_BYTES
        };
        let index_length = u64::from(page_count) * page_row_bytes;
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
                .malformed("page number: outside declared page index"));
        }
        Ok(Self {
            source,
            limits,
            cancellation,
            header: Header {
                variant,
                native_mode,
                native_origin,
                page_size,
                page_count,
                page_index,
            },
            page_row_bytes,
            next_page: start_page,
            current: None,
        })
    }

    pub fn header(&self) -> Header {
        self.header
    }

    pub fn source_mut(&mut self) -> &mut S {
        self.source
    }

    pub fn next_page(&mut self) -> Result<Option<PageRecord>> {
        let loc = Location {
            variant: Some(self.header.variant),
            offset: self.header.page_index.offset,
            page: Some(self.next_page),
            image: None,
        };
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
            .malformed("page has unread image records"));
        }
        if self.next_page > self.header.page_count {
            return Ok(None);
        }
        if self.cancellation.is_cancelled() {
            return Err(loc.cancelled());
        }
        let page_number = self.next_page;
        let row_offset =
            self.header.page_index.offset + u64::from(page_number - 1) * self.page_row_bytes;
        let loc = loc.at(row_offset);
        let mut row = [0; PAGE_ROW_BYTES as usize];
        read_fixed(
            self.source,
            self.limits,
            self.cancellation,
            row_offset,
            &mut row[..self.page_row_bytes as usize],
            loc,
            "page row",
        )?;
        let text_offset = nonnegative32(
            signed32(&row[..4]),
            loc,
            "text offset: negative signed value",
        )?;
        let text_length = nonnegative32(
            signed32(&row[4..8]),
            loc.at(row_offset + 4),
            "text length: negative signed value",
        )?;
        let text = checked_span(
            self.source.size(),
            text_offset,
            text_length,
            loc.at(row_offset),
            "text span",
        )?;
        if self.page_row_bytes == 12 {
            if text.offset < self.header.page_index.checked_end().expect("checked index") {
                return Err(loc.malformed("text span: overlaps protected container index"));
            }
            let value = u32::from_le_bytes(row[8..12].try_into().expect("four bytes"));
            if value != 0 {
                return Err(loc
                    .at(row_offset + 8)
                    .unsupported("compact HN-B third word"));
            }
        }
        let signed_images = signed16(&row[8..10]);
        if signed_images < 0 {
            return Err(loc
                .at(row_offset + 8)
                .malformed("image count: negative signed value"));
        }
        let image_count = signed_images as u32;
        let mut unknown = [0; 10];
        unknown.copy_from_slice(&row[10..]);
        let page = PageRecord {
            page_number,
            row_offset,
            text,
            image_count,
            unknown,
        };
        self.current = Some(CurrentPage {
            page,
            next_image: 1,
            next_descriptor: text.checked_end().expect("checked text span"),
        });
        self.next_page += 1;
        Ok(Some(page))
    }

    pub fn next_image(&mut self) -> Result<Option<ImageRecord>> {
        let loc = Location {
            variant: Some(self.header.variant),
            offset: self.header.page_index.offset,
            page: self.current.map(|current| current.page.page_number),
            image: self.current.map(|current| current.next_image),
        };
        let current = self
            .current
            .ok_or_else(|| loc.malformed("no current page"))?;
        if current.next_image > current.page.image_count {
            return Ok(None);
        }
        if self.cancellation.is_cancelled() {
            return Err(loc.cancelled());
        }
        let descriptor_offset = current.next_descriptor;
        let loc = loc.at(descriptor_offset);
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
            return Err(loc.malformed("image descriptor: overlaps header or page index"));
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
        )?;
        let signed_type = signed32(&bytes[..4]);
        if signed_type < 0 {
            return Err(loc.malformed("image type: negative signed value"));
        }
        if signed_type > 3 {
            return Err(loc.unsupported("image type"));
        }
        let payload_offset = nonnegative32(
            signed32(&bytes[4..8]),
            loc.at(descriptor_offset + 4),
            "image offset: negative signed value",
        )?;
        let payload_length = nonnegative32(
            signed32(&bytes[8..12]),
            loc.at(descriptor_offset + 8),
            "image length: negative signed value",
        )?;
        if payload_length == 0 {
            return Err(loc
                .at(descriptor_offset + 8)
                .malformed("image length: zero-length payload"));
        }
        let payload = checked_span(
            self.source.size(),
            payload_offset,
            payload_length,
            loc.at(descriptor_offset + 4),
            "image payload",
        )?;
        if payload_length > self.limits.max_allocation_bytes {
            return Err(loc.at(descriptor_offset + 8).limit(
                "image span bytes",
                self.limits.max_allocation_bytes,
                payload_length,
            ));
        }
        if payload.offset < descriptor.checked_end().expect("checked descriptor span") {
            return Err(loc
                .at(descriptor_offset + 4)
                .malformed("image offset: payload overlaps descriptor or chain regresses"));
        }
        // The payload starts at or after the descriptor's end, and the
        // descriptor was checked to start after the protected header and
        // page index.
        debug_assert!(!overlaps_protected(
            payload,
            self.header
                .page_index
                .checked_end()
                .expect("checked page index"),
        ));
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
        Ok(Some(record))
    }
}
