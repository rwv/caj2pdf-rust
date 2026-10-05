// SPDX-License-Identifier: MIT

//! Routing by text framing when native fonts are supplied.

use super::native_document::{hnb_fixture, native_text, roles};
use super::*;

/// Convert through the routing entry point, with or without the test font.
fn route(bytes: &[u8], fonts: bool) -> (Result<ComposeReport, ComposeError>, Vec<u8>) {
    let mut source = Source::new(bytes.to_vec());
    let mut font = [C8FontSource {
        source: Source::new(crate::pdf::drawing_font()),
        face: 0,
    }];
    let mut sink = Sink::default();
    let limits = Limits::default();
    let (mut rows, mut first, mut second, mut refined) = Default::default();
    let result = ready(convert_document_pdf(
        &mut source,
        &mut sink,
        fonts.then(|| C8FontSources {
            sources: &mut font,
            roles: roles(),
        }),
        Some(&table()),
        workspaces(&mut rows, &mut first, &mut second, &mut refined, &limits),
        &mut Visitor::default(),
        ComposeOptions::default(),
        &limits,
        &NeverCancel,
    ));
    (result, sink.bytes)
}

fn workspaces<'a>(
    rows: &'a mut Scratch,
    first: &'a mut Scratch,
    second: &'a mut Scratch,
    refined: &'a mut Scratch,
    limits: &Limits,
) -> ComposeWorkspaces<'a, Scratch> {
    // The MQ table is leaked so the borrowed workspaces outlive this helper.
    let table: &'static MqTable = Box::leak(Box::new(mq_table(limits)));
    ComposeWorkspaces {
        rows,
        type3: Some(ComposeType3Workspaces {
            table,
            first,
            second,
            refined,
        }),
    }
}

fn native(bytes: &[u8]) -> Result<bool, ComposeError> {
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
) -> Result<bool, ComposeError> {
    ready(uses_native_text(
        &mut source,
        ComposeOptions::default(),
        limits,
        cancellation,
    ))
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

/// Compressed text before the first page without images, native after it.
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
    let direct = fixture_with_text(Variant::C8, &[images()], direct_text);
    let hnb = fixture(Variant::HnB, &[vec![Record::jpeg(3, 2, 120, 0, 0)]]);
    for fixture in [&hna, &legacy, &direct, &hnb] {
        assert!(!native(&fixture.bytes).unwrap());
        let (expected, read) = image_only(&fixture.bytes);
        assert!(expected.ends_with(b"%%EOF\n"));
        let (report, pdf) = route(&fixture.bytes, true);
        assert_eq!(pdf, expected);
        // Routing reads are reported; HN-A has no native text, so only
        // header fields before its page index are read.
        let routing = report.unwrap().conversion.input_bytes_read - read;
        assert!(routing > 0);
        assert_eq!(std::ptr::eq(fixture, &hna), routing < 64, "{routing}");
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
        let (mut rows, mut first, mut second, mut refined) = Default::default();
        let direct = ready(convert_c8_native_pdf(
            &mut source,
            &mut sink,
            C8FontSources {
                sources: &mut fonts,
                roles: roles(),
            },
            Some(&table()),
            workspaces(&mut rows, &mut first, &mut second, &mut refined, &limits),
            ComposeOptions::default(),
            &limits,
            &NeverCancel,
        ))
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
    assert_eq!((error.page, error.stage), (Some(1), ComposeStage::Text));
}

#[test]
fn mixed_documents_route_to_native_composition_at_their_first_native_page() {
    // Compressed page 1 is inspected first; native page 2 decides.
    let later = fixture_with_text(
        Variant::C8,
        &[vec![Record::jpeg(3, 2, 120, 30, 50)], vec![]],
        mixed_text,
    );
    let first = fixture_with_text(
        Variant::C8,
        &[vec![], vec![Record::jpeg(3, 2, 120, 30, 50)]],
        mixed_text,
    );
    for (fixture, failing) in [(&later, 1), (&first, 2)] {
        assert!(native(&fixture.bytes).unwrap());
        // Neither composer accepts both framings; each locates its refusal.
        let error = route(&fixture.bytes, true).0.unwrap_err();
        assert_eq!(error.page, Some(failing));
        assert_eq!(error.stage, ComposeStage::Text);
        let error = route(&fixture.bytes, false).0.unwrap_err();
        assert!(error.page.is_some());
    }
}

#[test]
fn document_defects_fall_back_to_the_image_composer_error() {
    let mut header = fixture(Variant::C8, &[images()]).bytes;
    header[0] = 0xc9;
    let mut row = fixture(Variant::C8, &[images(), images()]);
    row.bytes[row.index + 20 + 4..row.index + 20 + 8].copy_from_slice(&(-1_i32).to_le_bytes());
    let mut descriptor = fixture(Variant::C8, &[images()]);
    let at = descriptor.descriptors[0][1] as usize;
    descriptor.bytes[at..at + 4].copy_from_slice(&9_i32.to_le_bytes());
    let mut framing = fixture_with_text(Variant::C8, &[vec![]], native_text);
    let at = framing.text_offsets[0];
    framing.bytes[at..at + 2].copy_from_slice(&0x7fff_u16.to_le_bytes());
    for bytes in [header, row.bytes, descriptor.bytes, framing.bytes] {
        assert!(!native(&bytes).unwrap());
        let with_fonts = route(&bytes, true).0.unwrap_err();
        let without = route(&bytes, false).0.unwrap_err();
        assert_eq!(with_fonts.to_string(), without.to_string());
    }
}

#[test]
fn source_failures_cancellation_and_invalid_limits_are_returned() {
    let fixture = fixture_with_text(Variant::C8, &[vec![], vec![]], native_text);
    for (at, stage) in [
        (fixture.index as u64, ComposeStage::Container),
        (fixture.text_offsets[0] as u64, ComposeStage::Text),
    ] {
        let mut source = Source::new(fixture.bytes.clone());
        source.fault_at = Some((at, Fault::Io));
        let error = native_with(source, &Limits::default(), &NeverCancel).unwrap_err();
        assert_eq!(error.stage, stage);
        assert!(matches!(error.kind, ComposeErrorKind::Container(_)));
    }
    let cancelled = Flag(Rc::new(Cell::new(true)));
    let source = Source::new(fixture.bytes.clone());
    let error = native_with(source, &Limits::default(), &cancelled).unwrap_err();
    assert_eq!(error.stage, ComposeStage::Container);
    let invalid = Limits {
        io_chunk_bytes: 0,
        ..Limits::default()
    };
    let mut source = Source::new(fixture.bytes.clone());
    source.fault_at = Some((0, Fault::Io));
    let error = native_with(source, &invalid, &NeverCancel).unwrap_err();
    assert_eq!(error.stage, ComposeStage::Preflight);
    // A routing failure is reported before any output.
    let mut source = Source::new(fixture.bytes);
    source.fault_at = Some((fixture.index as u64, Fault::Io));
    let mut fonts = [C8FontSource {
        source: Source::new(crate::pdf::drawing_font()),
        face: 0,
    }];
    let mut sink = Sink::default();
    let mut rows = Scratch::default();
    let error = ready(convert_document_pdf(
        &mut source,
        &mut sink,
        Some(C8FontSources {
            sources: &mut fonts,
            roles: roles(),
        }),
        Some(&table()),
        &mut rows,
        &mut (),
        ComposeOptions::default(),
        &Limits::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert_eq!(error.stage, ComposeStage::Container);
    assert!(sink.bytes.is_empty());
}
