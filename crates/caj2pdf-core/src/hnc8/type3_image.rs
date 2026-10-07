// SPDX-License-Identifier: MIT

//! Bounded HN/C8 type-3 image preflight, decoding and emission for the
//! document composition pipeline.

use super::{At, ImageRecord};
use crate::jbig2::{
    SegmentSpan,
    dictionary::{
        DictionaryStores, ImportedDictionary, SymbolDictionaryDecoder, coding_unit_contexts,
        symbol_code_length,
    },
    generic::{GenericRegionDecoder, read_generic_region_header},
    iaid::IAID_BASE,
    mq::{ContextBank, MqTable},
    page_compose::{PageComposeReport, PageOrSink},
    page_info::{PageInfo, read_page_info},
    page_profile::{PageProfile, validate_observed_page_profile},
    read_embedded_directory,
    text::{TextHeaderPolicy, read_text_region_header_with_policy},
    text_composer::{TextComposeReport, TextComposer},
    text_instances::TextInstanceDecoder,
};
use crate::pdf::{BilevelImageSpec, ImageObject, PdfDocument};
use crate::{
    Cancellation, Error, Hnc8Stage, Limits, Payload, RangedSource, Result, Type3Stage,
    read_exact_at,
};
use std::io::Write;

const DIB_BYTES: u64 = 48;

/// The three symbol stores and the composed text region of one type-3
/// image, in memory. The composition pipeline owns them and reuses their
/// allocations between images; each is cleared before an image.
#[derive(Default)]
pub(super) struct Type3Stores {
    first: Vec<u8>,
    second: Vec<u8>,
    refined: Vec<u8>,
    text: Vec<u8>,
}

impl At {
    /// Locate a failure of type-3 decoding `stage`.
    fn stage(self, stage: Type3Stage, error: Error) -> Error {
        self.locate(Hnc8Stage::Type3(stage), error)
    }

    /// A type-3 DIB wrapper outside the observed one-bit profile.
    fn dib(self, reason: &'static str) -> Error {
        self.error(Hnc8Stage::Headers, Error::invalid(reason))
    }

    fn pdf(self) -> impl FnOnce(Error) -> Error {
        self.locator(Hnc8Stage::Pdf)
    }
}

fn work_stage<T>(result: Result<T>, at: At, stage: Type3Stage) -> Result<T> {
    result.map_err(|error| at.stage(stage, error))
}

/// Checked source metadata from one preflight pass. Geometry is exposed
/// without decoding pixels; the directory and profile are reused for decode.
#[derive(Debug)]
pub(super) struct CheckedType3 {
    image: ImageRecord,
    directory: crate::jbig2::SegmentDirectory,
    profile: PageProfile,
    /// The text-header policy the preflight admitted, reapplied by decode.
    policy: TextHeaderPolicy,
}

impl CheckedType3 {
    pub(super) fn page(&self) -> PageInfo {
        self.profile.page()
    }
}

/// Prepared text pixels, composed in the stores' text region. Symbol
/// decoder contexts and catalogs have already been released.
pub(super) struct PreparedType3 {
    checked: CheckedType3,
    text_report: TextComposeReport,
}

/// Check one type-3 record's DIB wrapper and JBIG2 metadata without decoding
/// pixels. `image_at` locates the descriptor; failures are anchored at the
/// payload or the failing segment.
pub(super) fn preflight_type3<S: RangedSource, C: Cancellation>(
    source: &mut S,
    image: ImageRecord,
    image_at: At,
    policy: TextHeaderPolicy,
    limits: &Limits,
    cancellation: &C,
) -> Result<CheckedType3> {
    debug_assert_eq!(image.record_type, 3);
    let at = image_at.with_offset(image.payload.offset);
    if image.payload.length <= DIB_BYTES {
        return Err(at.dib("type-3 DIB: record has no enclosed JBIG2 segments"));
    }
    let mut dib = [0_u8; DIB_BYTES as usize];
    read_exact_at(source, image.payload.offset, &mut dib, limits, cancellation)
        .map_err(|source| at.error(Hnc8Stage::Headers, source))?;
    if u32::from_le_bytes(dib[0..4].try_into().expect("fixed DIB field")) != 40 {
        return Err(at.dib("type-3 DIB: header size differs from 40 bytes"));
    }
    let dib_width = i32::from_le_bytes(dib[4..8].try_into().expect("fixed DIB field"));
    let dib_height = i32::from_le_bytes(dib[8..12].try_into().expect("fixed DIB field"));
    if dib_width <= 0 || dib_height <= 0 {
        return Err(at
            .with_offset(image.payload.offset + 4)
            .dib("type-3 DIB: nonpositive dimensions"));
    }
    if dib[12..14] != 1_u16.to_le_bytes()
        || dib[14..16] != 1_u16.to_le_bytes()
        || dib[16..20] != 0_u32.to_le_bytes()
    {
        return Err(at
            .with_offset(image.payload.offset + 12)
            .dib("type-3 DIB: expected one plane, one bit, and uncompressed DIB"));
    }
    if dib[40..48] != [255, 255, 255, 0, 0, 0, 0, 0] {
        return Err(at
            .with_offset(image.payload.offset + 40)
            .dib("type-3 DIB: expected observed white/black palette"));
    }
    let embedded = SegmentSpan {
        offset: image.payload.offset + DIB_BYTES,
        length: image.payload.length - DIB_BYTES,
    };
    let directory = read_embedded_directory(source, embedded, limits, cancellation)
        .map_err(|error| at.stage(Type3Stage::Directory, error))?;
    if directory.segments.len() != 5 {
        return Err(at.stage(
            Type3Stage::Profile,
            Error::unsupported(embedded.offset, "segment count").in_jbig2(None),
        ));
    }
    let page = read_page_info(source, &directory.segments[0], limits, cancellation)
        .map_err(|error| at.stage(Type3Stage::PageInfo, error))?;
    if page.width != dib_width as u32 || page.height != dib_height as u32 {
        return Err(at
            .with_offset(image.payload.offset + 4)
            .dib("type-3 DIB: DIB and JBIG2 page dimensions differ"));
    }
    let text = read_text_region_header_with_policy(
        source,
        &directory.segments[3],
        &directory.segments[2],
        limits,
        cancellation,
        policy,
    )
    .map_err(|error| at.stage(Type3Stage::TextHeader, error))?;
    let generic = read_generic_region_header(source, &directory.segments[4], limits, cancellation)
        .map_err(|error| at.stage(Type3Stage::GenericHeader, error))?;
    let profile =
        validate_observed_page_profile(&directory, page, &text, generic).map_err(|error| {
            // A profile error names its segment; locate it at the segment data.
            let segment = match error.context {
                crate::Context::Jbig2 { segment } => segment,
                _ => None,
            };
            let offset = segment
                .and_then(|number| {
                    directory
                        .segments
                        .iter()
                        .find(|candidate| candidate.number == number)
                        .map(|candidate| candidate.data.offset)
                })
                .unwrap_or(embedded.offset);
            at.with_offset(offset).stage(Type3Stage::Profile, error)
        })?;
    Ok(CheckedType3 {
        image,
        directory,
        profile,
        policy,
    })
}

/// Decode one checked type-3 record from its `payload` into `document`,
/// using and first clearing `stores`.
pub(super) fn emit_type3<W: Write, C: Cancellation>(
    payload: Payload<'_>,
    document: &mut PdfDocument<'_, W, C>,
    stores: &mut Type3Stores,
    checked: CheckedType3,
    image_at: At,
    limits: &Limits,
    cancellation: &C,
) -> Result<(ImageObject, PageComposeReport)> {
    for store in [
        &mut stores.first,
        &mut stores.second,
        &mut stores.refined,
        &mut stores.text,
    ] {
        store.clear();
    }
    let prepared = prepare_type3_image(payload, stores, checked, image_at, limits, cancellation)?;
    emit_type3_xobject(
        payload,
        document,
        prepared,
        &stores.text,
        image_at,
        limits,
        cancellation,
    )
}

/// Decode symbol dictionaries and the text layer before the image's PDF
/// stream is opened, so their failures leave no partial image object.
pub(super) fn prepare_type3_image<C: Cancellation>(
    payload: Payload<'_>,
    stores: &mut Type3Stores,
    checked: CheckedType3,
    image_at: At,
    limits: &Limits,
    cancellation: &C,
) -> Result<PreparedType3> {
    let table = &MqTable::standard();
    let policy = checked.policy;
    let image = checked.image;
    let directory = &checked.directory;
    let text = checked.profile.text_header();
    let at = image_at.with_offset(image.payload.offset);
    let first_contexts = ContextBank::new(IAID_BASE, limits);
    let first_at = at.with_offset(directory.segments[1].data.offset);
    let mut first_contexts = work_stage(first_contexts, first_at, Type3Stage::Contexts)?;
    // A direct dictionary has no import and never reads its own store.
    let first_decoder = SymbolDictionaryDecoder::new(
        payload,
        &directory.segments[1],
        None,
        DictionaryStores {
            imported: &[],
            imported_base: 0,
            new: &mut stores.first,
            new_base: 0,
        },
        table,
        &mut first_contexts,
        limits,
        cancellation,
    )
    .map_err(|error| at.stage(Type3Stage::FirstDictionary, error))?;
    let first_report = work_stage(first_decoder.decode_hnc8(), at, Type3Stage::FirstDictionary)?;
    let imported_count = u64::from(first_report.header.exported_symbols);
    let second_count = u64::from(read_second_new_symbol_count(
        directory,
        payload,
        limits,
        cancellation,
        at,
    )?);
    let second_contexts = context_bank(symbol_code_length(imported_count + second_count), limits);
    let second_at = at.with_offset(directory.segments[2].data.offset);
    let mut second_contexts = work_stage(second_contexts, second_at, Type3Stage::Contexts)?;
    let second_decoder = SymbolDictionaryDecoder::new(
        payload,
        &directory.segments[2],
        Some(ImportedDictionary {
            segment: &directory.segments[1],
            report: &first_report,
        }),
        DictionaryStores {
            imported: &stores.first,
            imported_base: 0,
            new: &mut stores.second,
            new_base: 0,
        },
        table,
        &mut second_contexts,
        limits,
        cancellation,
    )
    .map_err(|error| at.stage(Type3Stage::SecondDictionary, error))?;
    let second_report = work_stage(
        second_decoder.decode_hnc8(),
        at,
        Type3Stage::SecondDictionary,
    )?;
    let text_contexts = context_bank(
        symbol_code_length(second_report.catalog.exported_symbols.len() as u64),
        limits,
    );
    let text_at = at.with_offset(directory.segments[3].data.offset);
    let mut text_contexts = work_stage(text_contexts, text_at, Type3Stage::Contexts)?;
    let text_decoder = TextInstanceDecoder::new_with_header_policy(
        payload,
        &directory.segments[3],
        text,
        &directory.segments[2],
        &second_report,
        &stores.first,
        0,
        &stores.second,
        0,
        &mut stores.refined,
        0,
        table,
        &mut text_contexts,
        limits,
        cancellation,
        policy,
    );
    let mut text_decoder = work_stage(text_decoder, at, Type3Stage::TextInstances)?;
    let composer = TextComposer::new(
        directory.segments[3].number,
        text,
        &second_report.catalog.exported_symbols,
        &mut text_decoder,
        &stores.first,
        0,
        &stores.second,
        0,
        0,
        &mut stores.text,
        limits,
        cancellation,
    );
    let composer = work_stage(composer, at, Type3Stage::TextCompose)?;
    let text_report = work_stage(composer.compose(), at, Type3Stage::TextCompose)?;
    Ok(PreparedType3 {
        checked,
        text_report,
    })
}

/// Append one image to an existing PDF. Page creation/placement belongs to
/// the caller; the decoder never opens or finishes another document.
pub(super) fn emit_type3_xobject<W: Write, C: Cancellation>(
    payload: Payload<'_>,
    document: &mut PdfDocument<'_, W, C>,
    prepared: PreparedType3,
    text: &[u8],
    image_at: At,
    limits: &Limits,
    cancellation: &C,
) -> Result<(ImageObject, PageComposeReport)> {
    let PreparedType3 {
        checked,
        text_report,
    } = prepared;
    let image = checked.image;
    let page = checked.page();
    let display_width = page.width;
    let profile = checked.profile;
    let directory = &checked.directory;
    let at = image_at.with_offset(image.payload.offset);
    // TextComposer proved the packed byte count; PageOrSink rechecks it
    // before forwarding the first combined row to the PDF image stream.
    let mut rows = document
        .begin_bilevel_image(BilevelImageSpec {
            pixel_width: display_width,
            pixel_height: page.height,
            row_stride: (display_width as usize).div_ceil(8),
        })
        .map_err(at.pdf())?;
    let mut padded = PaddedRows {
        sink: &mut rows,
        stride: page.row_stride,
        column: 0,
        padding: (display_width as usize).div_ceil(8) - page.row_stride,
    };
    let mut page_sink = PageOrSink::new(
        profile,
        text_report,
        text,
        &mut padded,
        limits,
        cancellation,
    )
    .map_err(|error| {
        at.stage(
            Type3Stage::PageCompose,
            Error {
                offset: None,
                ..error
            },
        )
    })?;
    let generic_result = ContextBank::new(1024, limits);
    let generic_at = at.with_offset(directory.segments[4].data.offset);
    let mut generic_contexts = work_stage(generic_result, generic_at, Type3Stage::Contexts)?;
    let generic_report = decode_generic(
        payload,
        &directory.segments[4],
        &mut generic_contexts,
        &mut page_sink,
        profile,
        limits,
        cancellation,
    )
    .map_err(|error| match page_sink.take_failure() {
        // A page OR offset is a page byte, not an input offset.
        Some(failure) => at.stage(
            Type3Stage::PageCompose,
            Error {
                offset: None,
                ..failure
            },
        ),
        None => at.stage(Type3Stage::GenericRegion, error),
    })?;
    let page_compose = page_sink.finish(&generic_report).map_err(|error| {
        at.stage(
            Type3Stage::PageCompose,
            Error {
                offset: None,
                ..error
            },
        )
    })?;
    let object = rows.finish().map_err(at.pdf())?;
    Ok((object, page_compose))
}

/// Decode the generic region into the armed page sink.
fn decode_generic<W: Write, C: Cancellation>(
    payload: Payload<'_>,
    segment: &crate::jbig2::SegmentHeader,
    contexts: &mut ContextBank,
    page_sink: &mut PageOrSink<'_, W, C>,
    profile: PageProfile,
    limits: &Limits,
    cancellation: &C,
) -> Result<crate::jbig2::generic::GenericReport> {
    let table = MqTable::standard();
    let mut decoder = GenericRegionDecoder::new(
        payload,
        segment,
        &table,
        contexts,
        page_sink,
        limits,
        cancellation,
    )?;
    decoder.arm_page_output(profile.generic_header())?;
    while decoder.decode_next_row()? {}
    decoder.finish()
}

/// Internal adapter for a checked visible/DIB width. BilevelImageWriter
/// accepts entire writes or fails; padding is at most three white bytes.
struct PaddedRows<'a, W> {
    sink: &'a mut W,
    stride: usize,
    column: usize,
    padding: usize,
}

impl<W: Write> Write for PaddedRows<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let count = bytes.len().min(self.stride - self.column);
        let written = self.sink.write(&bytes[..count])?;
        self.column += written;
        if self.column == self.stride {
            // Only BilevelImageWriter is wrapped: it accepts the complete
            // slice or fails, including its own bounded/partial sink writes.
            self.sink.write(&[0; 3][..self.padding])?;
            self.column = 0;
        }
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.sink.flush()
    }
}

/// The contexts of a coding unit whose IAID width is `code_len`.
fn context_bank(code_len: u32, limits: &Limits) -> Result<ContextBank> {
    let count = coding_unit_contexts(code_len)
        .ok_or_else(|| Error::invalid(crate::arith::INVALID_CONTEXT))?;
    ContextBank::new(count, limits)
}

fn read_second_new_symbol_count<C: Cancellation>(
    directory: &crate::jbig2::SegmentDirectory,
    payload: Payload<'_>,
    limits: &Limits,
    cancellation: &C,
    at: At,
) -> Result<u32> {
    use crate::jbig2::dictionary::read_dictionary_data_header;
    let result = work_stage(
        read_dictionary_data_header(
            &mut { payload },
            &directory.segments[2],
            limits,
            cancellation,
        ),
        at,
        Type3Stage::SecondDictionary,
    )?;
    Ok(result.new_symbols)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hnc8::Variant;
    use crate::pdf::PageSpec;
    use crate::test_support::{page_image, segment, stage_of};
    use std::error::Error as _;

    fn invalid_input() -> Error {
        Error::invalid("invented fault")
    }

    #[test]
    fn located_errors_keep_their_stage_and_distinct_messages() {
        let at = At {
            variant: Some(Variant::HnA),
            page: Some(2),
            image: Some(3),
            offset: Some(77),
        };
        let type3 = Hnc8Stage::Type3;
        let examples = [
            (
                at.dib("type-3 DIB: bad palette"),
                Hnc8Stage::Headers,
                "malformed HN/C8 HN-A at byte 77, page 2, image 3: type-3 DIB: bad palette",
            ),
            (
                at.stage(Type3Stage::Directory, invalid_input()),
                type3(Type3Stage::Directory),
                "malformed HN/C8 HN-A at byte 77, page 2, image 3: invented fault",
            ),
            (
                at.stage(Type3Stage::Profile, invalid_input()),
                type3(Type3Stage::Profile),
                "malformed HN/C8 HN-A at byte 77, page 2, image 3: invented fault",
            ),
            (
                at.stage(Type3Stage::FirstDictionary, invalid_input()),
                type3(Type3Stage::FirstDictionary),
                "malformed HN/C8 HN-A at byte 77, page 2, image 3: invented fault",
            ),
            (
                at.stage(Type3Stage::PageCompose, invalid_input()),
                type3(Type3Stage::PageCompose),
                "malformed HN/C8 HN-A at byte 77, page 2, image 3: invented fault",
            ),
            (
                at.pdf()(invalid_input()),
                Hnc8Stage::Pdf,
                "malformed HN/C8 HN-A at byte 77, page 2, image 3: invented fault",
            ),
        ];
        for (error, stage, message) in examples {
            assert_eq!(stage_of(&error), Some(stage), "{error}");
            assert_eq!(error.to_string(), message);
            assert!(error.source().is_none());
        }
        assert_eq!(symbol_code_length(0), 0);
        assert_eq!(symbol_code_length(1), 0);
        assert_eq!(symbol_code_length(2), 1);
        assert_eq!(symbol_code_length(5), 3);
    }

    #[test]
    fn text_composition_failures_keep_their_absolute_source_offset() {
        let at = At {
            page: Some(4),
            image: Some(2),
            offset: Some(100),
            ..At::NONE
        };
        let instance = Error::invalid("invented terminal")
            .at(555)
            .in_jbig2(Some(3));
        let error = work_stage::<()>(Err(instance), at, Type3Stage::TextCompose).unwrap_err();
        assert_eq!(
            (page_image(&error), error.offset, segment(&error)),
            ((Some(4), Some(2)), Some(555), Some(3))
        );
        assert!(error.to_string().contains("at byte 555"), "{error}");

        let malformed = Error::invalid("invented").in_jbig2(Some(3));
        let error = work_stage::<()>(Err(malformed), at, Type3Stage::TextCompose).unwrap_err();
        assert_eq!(error.offset, Some(100));
        assert!(error.to_string().contains("at byte 100"), "{error}");
    }

    use crate::test_support::mq_encoder;

    mod fixture {
        include!("../../tests/common/type3_fixture.rs");
    }

    #[derive(Clone, Default)]
    struct Memory(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);

    impl Write for Memory {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let count = bytes.len().min(3);
            self.0.borrow_mut().extend_from_slice(&bytes[..count]);
            Ok(count)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn shared_emitter_appends_two_asymmetric_images_to_an_existing_document() {
        use crate::{NeverCancel, hnc8::Span};
        let limits = Limits::default();
        let policy = TextHeaderPolicy::default();
        let mut sink = Memory::default();
        let output = sink.clone();
        let mut document = PdfDocument::new(&mut sink, &limits, &NeverCancel).unwrap();
        let size = PageSpec {
            width_points: 30.0,
            height_points: 20.0,
        };
        let mut placements = Vec::new();
        let mut stores = Type3Stores::default();
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
            let checked = preflight_type3(
                &mut &bytes[..],
                image,
                At::NONE,
                policy,
                &limits,
                &NeverCancel,
            )
            .unwrap();
            let payload = Payload::from(&bytes[..]);
            assert_eq!(
                (checked.page().width, checked.page().height),
                (width, height)
            );
            let before = output.0.borrow().len();
            let prepared = prepare_type3_image(
                payload,
                &mut stores,
                checked,
                At::NONE,
                &limits,
                &NeverCancel,
            )
            .unwrap();
            assert_eq!(
                output.0.borrow().len(),
                before,
                "preparation must not write PDF bytes"
            );
            let (object, report) = emit_type3_xobject(
                payload,
                &mut document,
                prepared,
                &stores.text,
                At::NONE,
                &limits,
                &NeverCancel,
            )
            .unwrap();
            assert_eq!(report.text_header_anomaly, None);
            if number == 1 {
                document.add_page(size, &[object]).unwrap();
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
            stores = Type3Stores::default();
        }
        document.add_placed_page(size, &placements).unwrap();
        let report = document.finish().unwrap();
        assert_eq!(report.pages_converted, 2);
        let bytes = output.0.borrow();
        assert_eq!(report.output_bytes_written, bytes.len() as u64);
        let text = String::from_utf8_lossy(&bytes);
        assert_eq!(text.matches("%PDF-").count(), 1);
        assert_eq!(text.matches("%%EOF").count(), 1);
        assert_eq!(text.matches("/Subtype /Image").count(), 2);
        assert_eq!(
            crate::test_support::bilevel_pixels(&bytes),
            [vec![0x80, 0], vec![0x80, 0, 0, 0, 0, 0]]
        );
        assert!(text.contains("3 0 0 2 10 0 cm"));
        assert!(text.contains("9 0 0 3 20 0 cm"));
    }
}
