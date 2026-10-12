// SPDX-License-Identifier: MIT

//! Generated C8 application-info tails on both composition paths. A valid
//! package reaches the PDF `/Info`; a defective one leaves the PDF unchanged.

use super::native_document::{native_text, roles};
use super::*;
use crate::hnc8::{
    ApplicationInfoDefect, ApplicationInfoStatus, ErrorKind, MAX_APPLICATION_INFO_BYTES,
};
use crate::test_support::CancelAfter;
use crate::test_support::stage_of;

const XML: &str = "<?xml version='1.0' encoding='UTF-8' ?>\n<Package><Note-Package>\
<NoteItems><Item/><Item/></NoteItems></Note-Package><FileProperty-Package>\
<DOI>INVENTED:ID.1</DOI><DURL><![CDATA[http://example.invalid/\u{4e2d}]]></DURL>\
</FileProperty-Package></Package>";

/// Append an invented package framed as observed: declared decoded and
/// compressed lengths, one zlib stream, then `APPINFOSIGN <start>`.
fn append_package(bytes: &mut Vec<u8>, xml: &[u8], decoded: u32) {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(xml).unwrap();
    let stream = encoder.finish().unwrap();
    let start = bytes.len();
    bytes.extend(decoded.to_le_bytes());
    bytes.extend((stream.len() as u32).to_le_bytes());
    bytes.extend(&stream);
    bytes.extend(format!("APPINFOSIGN {start}").as_bytes());
}

fn image_only(tail: Option<(&[u8], u32)>) -> (Result<ComposeReport>, Vec<u8>) {
    let mut case = Harness::type0();
    if let Some((xml, decoded)) = tail {
        append_package(&mut case.source.bytes, xml, decoded);
    }
    let result = case.run(
        Some(&table()),
        ComposeOptions::default(),
        &Limits::default(),
    );
    (result, case.sink.bytes)
}

fn utf16_hex(text: &str) -> String {
    text.encode_utf16()
        .map(|unit| format!("{unit:04X}"))
        .collect()
}

#[test]
fn image_only_c8_package_is_written_to_the_info_dictionary() {
    let (plain, plain_pdf) = image_only(None);
    let plain = plain.unwrap();
    assert_eq!(plain.application_info, ApplicationInfoStatus::Absent);
    assert!(!contains(&plain_pdf, b"/Info"));
    let (report, pdf) = image_only(Some((XML.as_bytes(), XML.len() as u32)));
    let report = report.unwrap();
    assert_eq!(report.application_info, ApplicationInfoStatus::Read);
    let info = format!(
        "<< /CNKI_DOI <FEFF{}> /CNKI_URL <FEFF{}> >>",
        utf16_hex("INVENTED:ID.1"),
        utf16_hex("http://example.invalid/\u{4e2d}")
    );
    assert!(contains(&pdf, info.as_bytes()));
    let text = String::from_utf8_lossy(&pdf);
    let trailer = &text[text.rfind("trailer").unwrap()..];
    assert!(trailer.contains(" /Info "), "{trailer}");
    assert_eq!(report.output_pages, plain.output_pages);
    // A package with neither value writes no dictionary.
    let notes = "<Package><Note-Package><NoteItems><Item/></NoteItems></Note-Package></Package>";
    let (report, pdf) = image_only(Some((notes.as_bytes(), notes.len() as u32)));
    assert_eq!(
        report.unwrap().application_info,
        ApplicationInfoStatus::Read
    );
    assert_eq!(pdf, plain_pdf);
}

#[test]
fn defective_c8_packages_warn_and_leave_pages_unchanged() {
    let (_, plain_pdf) = image_only(None);
    for (xml, decoded, field) in [
        (
            XML.as_bytes(),
            XML.len() as u32 + 1,
            "application-info zlib stream",
        ),
        (b"not xml".as_slice(), 7, "application-info XML"),
        (
            XML.as_bytes(),
            MAX_APPLICATION_INFO_BYTES + 1,
            "application-info decoded bytes",
        ),
    ] {
        let (report, pdf) = image_only(Some((xml, decoded)));
        let ApplicationInfoStatus::Ignored(ApplicationInfoDefect { field: found, .. }) =
            report.unwrap().application_info
        else {
            panic!("expected an ignored package");
        };
        assert_eq!(found, field);
        assert_eq!(pdf, plain_pdf);
    }
}

#[test]
fn cancellation_while_reading_the_package_still_fails() {
    let mut bytes = Harness::type0().source.bytes;
    append_package(&mut bytes, XML.as_bytes(), XML.len() as u32);
    let run = |cancellation: &CancelAfter| {
        convert_source_pages_pdf(
            &mut Source::new(bytes.clone()),
            &mut Sink::default(),
            Some(&table()),
            &mut Visitor::default(),
            ComposeOptions::default(),
            &Limits::default(),
            cancellation,
        )
    };
    let counter = CancelAfter::never();
    run(&counter).unwrap();
    // The package is read after every page, so search back from the end for
    // a cancellation reported by the package reader itself.
    let cancelled = (0..counter.queries())
        .rev()
        .map(|allowed| run(&CancelAfter::new(allowed)))
        .find_map(|result| {
            result.err().filter(|error| {
                stage_of(error) == Some(Hnc8Stage::Container)
                    && error.offset > Some(0)
                    && matches!(error.kind, ErrorKind::Cancelled)
            })
        });
    assert!(cancelled.is_some());
}

#[test]
fn native_c8_package_is_written_to_the_info_dictionary() {
    let mut pdfs = Vec::new();
    for tail in [false, true] {
        let mut fixture = fixture_with_text(
            Variant::C8,
            &[vec![Record::jpeg(3, 2, 120, 20, 40)]],
            native_text,
        );
        if tail {
            let xml = XML.replace("<NoteItems><Item/><Item/></NoteItems>", "<NoteItems/>");
            append_package(&mut fixture.bytes, xml.as_bytes(), xml.len() as u32);
        }
        let mut fonts = [C8FontSource {
            source: Source::new(crate::pdf::drawing_font()),
            face: 0,
        }];
        let mut sink = Sink::default();
        let report = convert_c8_native_pdf(
            &mut Source::new(fixture.bytes),
            &mut sink,
            C8FontSources {
                embedding: Default::default(),
                glyph_aliases: &[],
                sources: &mut fonts,
                roles: roles(),
                symbol_glyphs: &[],
                symbol_font: None,
                decoration_glyphs: &[],
                decoration_font: None,
            },
            None,
            ComposeOptions::default(),
            &Limits::default(),
            &NeverCancel,
        )
        .unwrap();
        assert_eq!(report.application_info == ApplicationInfoStatus::Read, tail);
        pdfs.push(sink.bytes);
    }
    assert!(!contains(&pdfs[0], b"/Info"));
    assert!(contains(&pdfs[1], b"/CNKI_DOI <FEFF"));
    assert!(contains(&pdfs[1], b"/CNKI_URL <FEFF"));
}

#[test]
fn native_links_preserve_body_and_warn_or_refuse_without_silent_loss() {
    let xml = "<Package><Note-Package><NoteItems Unit='PointsM' Version='1.0'><Item Type='Link' Page='2' Color='00808080' LineStyle='4' ShowType='1'><RC l='100' t='200' r='900' b='500'/><Item Type='UrlLink' Aim='https://example.invalid/?x=1&amp;y=2'/></Item></NoteItems></Note-Package></Package>";
    let run = |xml: Option<&str>| {
        let mut fixture = fixture_with_text(Variant::C8, &[vec![], vec![]], native_text);
        if let Some(xml) = xml {
            append_package(&mut fixture.bytes, xml.as_bytes(), xml.len() as u32);
        }
        let mut source = Source::new(fixture.bytes);
        source.short = 3;
        let mut fonts = [C8FontSource {
            source: Source::new(crate::pdf::drawing_font()),
            face: 0,
        }];
        let mut sink = Sink::default();
        let limits = Limits {
            io_chunk_bytes: 31,
            ..Limits::default()
        };
        let result = convert_c8_native_pdf(
            &mut source,
            &mut sink,
            C8FontSources {
                embedding: Default::default(),
                glyph_aliases: &[],
                sources: &mut fonts,
                roles: roles(),
                symbol_glyphs: &[],
                symbol_font: None,
                decoration_glyphs: &[],
                decoration_font: None,
            },
            None,
            ComposeOptions::default(),
            &limits,
            &NeverCancel,
        );
        assert!(source.max_request <= 31 && sink.max_request <= 31);
        (result, sink.bytes)
    };
    let (plain, body) = run(None);
    plain.unwrap();
    let (result, linked) = run(Some(xml));
    assert_eq!(
        result.unwrap().application_info,
        ApplicationInfoStatus::Read
    );
    let text = String::from_utf8_lossy(&linked);
    assert_eq!(text.matches("/Subtype /Link").count(), 1);
    assert_eq!(text.matches("/Annots").count(), 1);
    assert_eq!(
        crate::test_support::inflated_stream(&linked, b"/FlateDecode"),
        crate::test_support::inflated_stream(&body, b"/FlateDecode")
    );
    let (result, damaged) = run(Some(&xml.replace("r='900'", "r='0'")));
    assert!(matches!(
        result.unwrap().application_info,
        ApplicationInfoStatus::Ignored(_)
    ));
    assert_eq!(damaged, body);
    let (result, refused) = run(Some(&xml.replace("LineStyle='4'", "LineStyle='9'")));
    assert!(matches!(
        result.unwrap_err().kind,
        ErrorKind::UnsupportedFormat
    ));
    assert!(refused.is_empty());
}
