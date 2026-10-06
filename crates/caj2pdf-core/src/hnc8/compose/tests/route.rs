// SPDX-License-Identifier: MIT

//! Routing by text framing when native fonts are supplied.

use super::native_document::{hnb_fixture, native_text, roles};
use super::*;
use crate::Context;
use crate::test_support::page_image;
use crate::test_support::stage_of;

/// Convert through the routing entry point, with or without the test font.
fn route(bytes: &[u8], fonts: bool) -> (Result<ComposeReport>, Vec<u8>) {
    let mut source = Source::new(bytes.to_vec());
    let mut font = [C8FontSource {
        source: Source::new(crate::pdf::drawing_font()),
        face: 0,
    }];
    let mut sink = Sink::default();
    let limits = Limits::default();
    let result = convert_document_pdf(
        &mut source,
        &mut sink,
        fonts.then(|| C8FontSources {
            sources: &mut font,
            roles: roles(),
        }),
        Some(&table()),
        &mut Visitor::default(),
        ComposeOptions::default(),
        &limits,
        &NeverCancel,
    );
    (result, sink.bytes)
}

fn native(bytes: &[u8]) -> Result<bool> {
    native_with(
        Source::new(bytes.to_vec()),
        &Limits::default(),
        &NeverCancel,
    )
}

fn native_with(
    mut source: Source,
    limits: &Limits,
    cancellation: &impl Cancellation,
) -> Result<bool> {
    uses_native_text(&mut source, limits, cancellation)
}

/// The image path's PDF and read count, with no fonts supplied.
fn image_only(bytes: &[u8]) -> (Vec<u8>, u64) {
    let (report, pdf) = route(bytes, false);
    (pdf, report.unwrap().conversion.input_bytes_read)
}

fn images() -> Vec<Record> {
    vec![
        Record::type0(&rows(9), 20, 40),
        Record::jpeg(3, 2, 120, 30, 50),
        type3_record(3, 2, 40, 60),
    ]
}

/// Compressed text on pages with images, native text on pages without.
fn mixed_text(records: &[Record]) -> Vec<u8> {
    if records.is_empty() {
        native_text(records)
    } else {
        text(records)
    }
}

#[test]
fn image_documents_ignore_fonts_and_keep_byte_identical_output() {
    let hna = fixture(Variant::HnA, &[images(), vec![Record::jpeg(2, 3, 9, 0, 0)]]);
    let mut legacy = fixture(Variant::C8, &[images(), images()]);
    // Image-only C8 headers need not declare an admitted native mode.
    legacy.bytes[12..16].copy_from_slice(&23112_u32.to_le_bytes());
    let direct = fixture_with_text(Variant::C8, &[images(), images()], direct_text);
    for fixture in [&hna, &legacy, &direct] {
        assert!(!native(&fixture.bytes).unwrap());
        let (expected, read) = image_only(&fixture.bytes);
        assert!(expected.ends_with(b"%%EOF\n"));
        let (report, pdf) = route(&fixture.bytes, true);
        assert_eq!(pdf, expected);
        // Routing reads are reported. HN-A and a C8 header without an
        // admitted native mode decide from header fields alone; otherwise
        // only the first page with text is inspected.
        let routing = report.unwrap().conversion.input_bytes_read - read;
        assert!(routing > 0);
        assert_eq!(std::ptr::eq(fixture, &direct), routing >= 64, "{routing}");
    }
}

#[test]
fn native_documents_use_native_composition_only_with_fonts() {
    let c8 = fixture_with_text(
        Variant::C8,
        &[vec![Record::jpeg(3, 2, 120, 30, 50)], vec![]],
        native_text,
    );
    let hnb = hnb_fixture(12, 2);
    for fixture in [&c8, &hnb] {
        assert!(native(&fixture.bytes).unwrap());
        let mut source = Source::new(fixture.bytes.clone());
        let mut fonts = [C8FontSource {
            source: Source::new(crate::pdf::drawing_font()),
            face: 0,
        }];
        let mut sink = Sink::default();
        let limits = Limits::default();
        let direct = convert_c8_native_pdf(
            &mut source,
            &mut sink,
            C8FontSources {
                sources: &mut fonts,
                roles: roles(),
            },
            Some(&table()),
            ComposeOptions::default(),
            &limits,
            &NeverCancel,
        )
        .unwrap();
        let (report, pdf) = route(&fixture.bytes, true);
        let report = report.unwrap();
        assert_eq!(pdf, sink.bytes);
        assert_eq!(report.output_pages, 2);
        assert!(report.conversion.input_bytes_read > direct.conversion.input_bytes_read);
        assert!(contains(&pdf, b"/FontFile2 "));
    }
    // Without fonts the native C8 text is refused by image composition.
    let error = route(&c8.bytes, false).0.unwrap_err();
    assert_eq!(
        (page_image(&error).0, stage_of(&error)),
        (Some(1), Some(Hnc8Stage::Text))
    );
}

#[test]
fn mixed_documents_follow_their_first_page_with_text() {
    // Compressed page 1 selects image composition, which refuses native
    // page 2 in its preflight; native page 1 selects native composition,
    // which refuses compressed page 2 while drawing it.
    let compressed_first = fixture_with_text(
        Variant::C8,
        &[vec![Record::jpeg(3, 2, 120, 30, 50)], vec![]],
        mixed_text,
    );
    let native_first = fixture_with_text(
        Variant::C8,
        &[vec![], vec![Record::jpeg(3, 2, 120, 30, 50)]],
        mixed_text,
    );
    for (fixture, routed, stage) in [
        (&compressed_first, false, Hnc8Stage::Preflight),
        (&native_first, true, Hnc8Stage::Text),
    ] {
        assert_eq!(native(&fixture.bytes).unwrap(), routed);
        let error = route(&fixture.bytes, true).0.unwrap_err();
        assert_eq!(
            (page_image(&error).0, stage_of(&error)),
            (Some(2), Some(stage))
        );
    }
}

#[test]
fn container_defects_fall_back_to_the_image_composer_error() {
    let mut header = fixture(Variant::C8, &[images()]).bytes;
    header[0] = 0xc9;
    let mut row = fixture(Variant::C8, &[images(), images()]);
    row.bytes[row.index + 20 + 4..row.index + 20 + 8].copy_from_slice(&(-1_i32).to_le_bytes());
    let mut descriptor = fixture(Variant::C8, &[images()]);
    let at = descriptor.descriptors[0][1] as usize;
    descriptor.bytes[at..at + 4].copy_from_slice(&9_i32.to_le_bytes());
    for bytes in [header, row.bytes, descriptor.bytes] {
        assert!(!native(&bytes).unwrap());
        let with_fonts = route(&bytes, true).0.unwrap_err();
        let without = route(&bytes, false).0.unwrap_err();
        assert_eq!(with_fonts.to_string(), without.to_string());
    }
}

#[test]
fn native_text_defects_are_reported_by_native_composition() {
    // C8 text that neither reader accepts, and HN-B text that is not native
    // records, which image composition would silently drop.
    let mut c8 = fixture_with_text(Variant::C8, &[vec![]], native_text);
    let at = c8.text_offsets[0];
    c8.bytes[at..at + 2].copy_from_slice(&0x7fff_u16.to_le_bytes());
    let hnb = fixture(Variant::HnB, &[vec![Record::jpeg(3, 2, 120, 0, 0)]]);
    for bytes in [&c8.bytes, &hnb.bytes] {
        assert!(native(bytes).unwrap());
        let error = route(bytes, true).0.unwrap_err();
        assert_eq!(
            (page_image(&error).0, stage_of(&error)),
            (Some(1), Some(Hnc8Stage::Text))
        );
    }
    assert!(route(&hnb.bytes, false).0.is_ok());
}

#[test]
fn source_failures_cancellation_and_invalid_limits_are_returned() {
    let fixture = fixture_with_text(Variant::C8, &[vec![], vec![]], native_text);
    for (at, stage) in [
        (fixture.index as u64, Hnc8Stage::Container),
        (fixture.text_offsets[0] as u64, Hnc8Stage::Text),
    ] {
        let mut source = Source::new(fixture.bytes.clone());
        source.fault_at = Some((at, Fault::Io));
        let error = native_with(source, &Limits::default(), &NeverCancel).unwrap_err();
        assert_eq!(stage_of(&error), Some(stage));
        assert!(matches!(error.context, Context::Hnc8 { .. }));
    }
    let cancelled = Flag(Rc::new(Cell::new(true)));
    let source = Source::new(fixture.bytes.clone());
    let error = native_with(source, &Limits::default(), &cancelled).unwrap_err();
    assert_eq!(stage_of(&error), Some(Hnc8Stage::Container));
    let invalid = Limits {
        io_chunk_bytes: 0,
        ..Limits::default()
    };
    let mut source = Source::new(fixture.bytes.clone());
    source.fault_at = Some((0, Fault::Io));
    let error = native_with(source, &invalid, &NeverCancel).unwrap_err();
    assert_eq!(stage_of(&error), Some(Hnc8Stage::Preflight));
    // A routing failure is reported before any output.
    let mut source = Source::new(fixture.bytes);
    source.fault_at = Some((fixture.index as u64, Fault::Io));
    let mut fonts = [C8FontSource {
        source: Source::new(crate::pdf::drawing_font()),
        face: 0,
    }];
    let mut sink = Sink::default();
    let error = convert_document_pdf(
        &mut source,
        &mut sink,
        Some(C8FontSources {
            sources: &mut fonts,
            roles: roles(),
        }),
        Some(&table()),
        &mut (),
        ComposeOptions::default(),
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert_eq!(stage_of(&error), Some(Hnc8Stage::Container));
    assert!(sink.bytes.is_empty());
}

/// No text on pages with images, native text on pages without.
fn native_after_empty(records: &[Record]) -> Vec<u8> {
    if records.is_empty() {
        native_text(records)
    } else {
        Vec::new()
    }
}

#[test]
fn pages_without_text_do_not_decide() {
    let none = fixture_with_text(Variant::C8, &[images(), images()], native_after_empty);
    assert!(!native(&none.bytes).unwrap());
    let later = fixture_with_text(Variant::C8, &[images(), vec![]], native_after_empty);
    assert!(native(&later.bytes).unwrap());
}
