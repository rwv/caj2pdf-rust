// SPDX-License-Identifier: MIT

use super::*;
use crate::hnc8::C8PageFonts;
use crate::test_support::page_image;

pub(super) fn native_text(records: &[Record]) -> Vec<u8> {
    let mut words = vec![[0x8001, 60], [0x8002, 0x1084], [30, 0xa0c1]];
    for record in records {
        let c = record.coordinate;
        words.extend([
            [0x800a, 0xd300],
            [0xc000 | c.x, c.y],
            [0xc000 | c.width, c.height],
            [0xc050, 0xc033],
            [0xc037, 0xc000],
            [0xc06c, 0xc032],
            [0xc0f2, 0xc07a],
            [45, 0xa0c1],
        ]);
    }
    words.push([0x8004, 39]);
    words
        .into_iter()
        .flatten()
        .flat_map(u16::to_le_bytes)
        .collect()
}

pub(super) fn roles() -> C8PageFonts {
    C8PageFonts {
        cjk: 0,
        latin: 0,
        alternate_latin: Some(0),
        decoration: None,
        symbols: None,
        latin_state3: None,
        latin_state28: None,
        latin_state31: None,
    }
}

#[test]
fn native_document_streams_text_and_all_shared_image_codecs() {
    // Optional roles may alias an existing source without duplicate embedding.
    for (symbols, latin_state3) in [
        (None, None),
        (Some(0), None),
        (None, Some(0)),
        (Some(0), Some(0)),
    ] {
        let font_roles = C8PageFonts {
            symbols,
            latin_state3,
            latin_state28: Some(0),
            latin_state31: Some(0),
            ..roles()
        };
        let rows = vec![vec![false, true, false], vec![true, false, true]];
        let fixture = fixture_with_text(
            Variant::C8,
            &[
                vec![],
                vec![
                    Record::type0(&rows, 20, 40),
                    Record::jpeg(3, 2, 120, 30, 50),
                    type3_record(3, 2, 40, 60),
                ],
                vec![],
            ],
            native_text,
        );
        let mut source = Source::new(fixture.bytes);
        source.short = 3;
        let mut fonts = [C8FontSource {
            source: Source::new(crate::pdf::drawing_font()),
            face: 0,
        }];
        fonts[0].source.short = 3;
        let mut sink = Sink {
            short: Some(7),
            ..Default::default()
        };
        let limits = Limits {
            io_chunk_bytes: 64,
            ..Default::default()
        };
        let report = convert_c8_native_pdf(
            &mut source,
            &mut sink,
            C8FontSources {
                sources: &mut fonts,
                roles: font_roles,
            },
            Some(&table()),
            ComposeOptions::default(),
            &limits,
            &NeverCancel,
        )
        .unwrap();
        assert_eq!(report.output_pages, 3);
        assert_eq!(report.conversion.pages_converted, 3);
        assert_eq!(report.no_image_pages, 2);
        assert_eq!(
            (report.type0_images, report.jpeg_images, report.type3_images),
            (1, 1, 1)
        );
        assert!(
            source.max_request <= 64 && fonts[0].source.max_request <= 64 && sink.max_request <= 64
        );
        let pdf = crate::test_support::pdf_text(&sink.bytes);
        assert_eq!(
            pdf.matches("/FontFile2 ").count(),
            1,
            "shared roles embed one font"
        );
        assert!(pdf.contains("/Count 3"));
        assert_eq!(pdf.matches("<0041> Tj").count(), 6);
        assert!(pdf.ends_with("%%EOF\n"));
    }
}

#[test]
fn native_document_checks_resource_contract_before_output() {
    for count in [0, 1, 9] {
        let mut fonts: Vec<_> = (0..count)
            .map(|_| C8FontSource {
                source: Source::new(crate::pdf::drawing_font()),
                face: 0,
            })
            .collect();
        let mut role = roles();
        if count == 1 {
            role.latin = 1;
        }
        let mut sink = Sink::default();
        let error = convert_c8_native_pdf(
            &mut Source::new(vec![]),
            &mut sink,
            C8FontSources {
                sources: &mut fonts,
                roles: role,
            },
            None,
            ComposeOptions::default(),
            &Limits::default(),
            &NeverCancel,
        )
        .unwrap_err();
        assert!(matches!(error.kind, ErrorKind::Malformed));
        assert!(sink.bytes.is_empty());
    }
}

#[test]
fn native_document_late_unknown_record_cannot_finish_pdf() {
    let mut fixture = fixture_with_text(Variant::C8, &[vec![], vec![]], native_text);
    let second = fixture.text_offsets[1];
    fixture.bytes[second..second + 2].copy_from_slice(&0x8072u16.to_le_bytes());
    let mut sink = Sink::default();
    let mut fonts = [C8FontSource {
        source: Source::new(crate::pdf::drawing_font()),
        face: 0,
    }];
    let error = convert_c8_native_pdf(
        &mut Source::new(fixture.bytes),
        &mut sink,
        C8FontSources {
            sources: &mut fonts,
            roles: roles(),
        },
        None,
        ComposeOptions::default(),
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert_eq!(page_image(&error).0, Some(2));
    assert!(!sink.bytes.ends_with(b"%%EOF\n"));
}

#[test]
fn native_c8_bookmark_request_is_reported_not_written_or_failed() {
    let mut outputs = Vec::new();
    for include_bookmarks in [false, true] {
        let fixture = fixture_with_text(
            Variant::C8,
            &[vec![Record::jpeg(3, 2, 120, 20, 40)]],
            native_text,
        );
        let mut fonts = [C8FontSource {
            source: Source::new(crate::pdf::drawing_font()),
            face: 0,
        }];
        let mut role = roles();
        role.decoration = Some((0, 'A'));
        let mut sink = Sink::default();
        let report = convert_c8_native_pdf(
            &mut Source::new(fixture.bytes),
            &mut sink,
            C8FontSources {
                sources: &mut fonts,
                roles: role,
            },
            None,
            ComposeOptions {
                include_bookmarks,
                ..ComposeOptions::default()
            },
            &Limits::default(),
            &NeverCancel,
        )
        .unwrap();
        assert_eq!(report.outline.unverified, include_bookmarks);
        assert_eq!(report.conversion.bookmarks_written, 0);
        outputs.push(sink.bytes);
    }
    assert_eq!(outputs[0], outputs[1]);
}

#[test]
fn native_document_errors_preserve_preflight_and_source_locations() {
    for mode in [0, 1, 2, 3, 4, 6, 7] {
        let mut fixture = fixture_with_text(
            Variant::C8,
            &[vec![Record::jpeg(3, 2, 120, 20, 40)]],
            native_text,
        );
        let options = ComposeOptions::default();
        let mut fonts = [C8FontSource {
            source: Source::new(crate::pdf::drawing_font()),
            face: 0,
        }];
        let mut role = roles();
        role.decoration = Some((0, 'A'));
        match mode {
            0 => fixture.bytes[0] = 0,
            1 => {
                fixture = fixture_with_text(
                    Variant::HnA,
                    &[vec![Record::jpeg(3, 2, 120, 20, 40)]],
                    native_text,
                )
            }
            2 => fixture.bytes[80..84].copy_from_slice(&u32::MAX.to_le_bytes()),
            3 => {
                let at = fixture.descriptors[0][0] as usize;
                fixture.bytes[at..at + 4].copy_from_slice(&99i32.to_le_bytes());
            }
            4 => fonts[0].source.bytes.clear(),
            6 => fixture.bytes[88..90].copy_from_slice(&32767u16.to_le_bytes()),
            7 => {
                let at = fixture.descriptors[0][0] as usize + 4;
                fixture.bytes[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            }
            _ => unreachable!(),
        }
        let mut sink = Sink::default();
        let result = convert_c8_native_pdf(
            &mut Source::new(fixture.bytes),
            &mut sink,
            C8FontSources {
                sources: &mut fonts,
                roles: role,
            },
            None,
            options,
            &Limits::default(),
            &NeverCancel,
        );
        let error = result.unwrap_err();
        assert!(!sink.bytes.ends_with(b"%%EOF\n"), "mode {mode}");
        if matches!(mode, 2 | 3 | 6 | 7) {
            assert_eq!(page_image(&error).0, Some(1), "mode {mode}: {error}");
        }
        if mode == 1 {
            assert!(sink.bytes.is_empty());
        }
    }
}

#[test]
fn native_document_font_io_output_and_cancellation_fail_explicitly() {
    for mode in 0..4 {
        let fixture = fixture_with_text(Variant::C8, &[vec![]], native_text);
        let mut fonts = [C8FontSource {
            source: Source::new(crate::pdf::drawing_font()),
            face: 0,
        }];
        let flag = Rc::new(Cell::new(false));
        let mut sink = Sink::default();
        match mode {
            0 => fonts[0].source.fault_at = Some((0, Fault::Io)),
            1 => sink.fail_after = Some(64),
            2 => sink.cancel = Some(flag.clone()),
            3 => sink.fail_flush = true,
            _ => unreachable!(),
        }
        let result = convert_c8_native_pdf(
            &mut Source::new(fixture.bytes),
            &mut sink,
            C8FontSources {
                sources: &mut fonts,
                roles: roles(),
            },
            None,
            ComposeOptions::default(),
            &Limits::default(),
            &Flag(flag),
        );
        assert!(result.is_err(), "mode {mode}");
        if mode != 3 {
            assert!(!sink.bytes.ends_with(b"%%EOF\n"));
        }
    }
}

pub(super) fn hnb_fixture(width: usize, pages: u32) -> Fixture {
    let index = 216;
    let mut bytes = vec![0; index + pages as usize * width];
    bytes[..4].copy_from_slice(b"HN\0\0");
    bytes[4..8].copy_from_slice(&200_u32.to_le_bytes());
    bytes[8..12].copy_from_slice(&136_u32.to_le_bytes());
    if width == 20 {
        bytes[136..140].copy_from_slice(&200_u32.to_le_bytes());
    }
    bytes[144..148].copy_from_slice(&pages.to_le_bytes());
    bytes[148] = 2;
    bytes[168..170].copy_from_slice(&100_u16.to_le_bytes());
    bytes[170..172].copy_from_slice(&200_u16.to_le_bytes());
    let mut offsets = Vec::new();
    for page in 0..pages as usize {
        let mut text = native_text(&[]);
        text.truncate(text.len() - 2); // Independently admitted HN-B bare end.
        let offset = bytes.len();
        let row = index + page * width;
        bytes[row..row + 4].copy_from_slice(&(offset as u32).to_le_bytes());
        bytes[row + 4..row + 8].copy_from_slice(&(text.len() as u32).to_le_bytes());
        bytes.extend(text);
        if width == 20 {
            let end = bytes.len() as u32;
            bytes[row + 16..row + 20].copy_from_slice(&end.to_le_bytes());
        }
        offsets.push(offset);
    }
    Fixture {
        bytes,
        index,
        text_offsets: offsets,
        descriptors: vec![vec![]; pages as usize],
        payloads: vec![vec![]; pages as usize],
    }
}

#[test]
fn hnb_native_document_streams_every_compact_page_and_keeps_late_errors_located() {
    for corrupt in [false, true] {
        let mut fixture = hnb_fixture(12, 3);
        if corrupt {
            let at = fixture.text_offsets[2];
            fixture.bytes[at..at + 2].copy_from_slice(&0x8099_u16.to_le_bytes());
        }
        let mut source = Source::new(fixture.bytes);
        source.short = 3;
        let mut fonts = [C8FontSource {
            source: Source::new(crate::pdf::drawing_font()),
            face: 0,
        }];
        fonts[0].source.short = 3;
        let mut sink = Sink {
            short: Some(7),
            ..Default::default()
        };
        let result = convert_c8_native_pdf(
            &mut source,
            &mut sink,
            C8FontSources {
                sources: &mut fonts,
                roles: roles(),
            },
            None,
            ComposeOptions::default(),
            &Limits::default(),
            &NeverCancel,
        );
        if corrupt {
            assert_eq!(page_image(&result.unwrap_err()).0, Some(3));
            assert!(!sink.bytes.ends_with(b"%%EOF\n"));
        } else {
            let report = result.unwrap();
            assert_eq!(report.output_pages, 3);
            assert_eq!(report.no_image_pages, 3);
            let pdf = crate::test_support::pdf_text(&sink.bytes);
            assert!(pdf.contains("/Count 3"));
            assert_eq!(pdf.matches("<0041> Tj").count(), 3);
            assert!(pdf.ends_with("%%EOF\n"));
        }
    }
}

#[test]
fn hnb_type3_after_text_retains_pixels_and_rejects_malformed_payloads() {
    for corrupt in [false, true] {
        let record = type3_record(3, 2, 20, 40);
        let mut fixture = fixture_with_text(Variant::HnB, &[vec![record]], native_text);
        if corrupt {
            fixture.bytes[fixture.payloads[0][0] as usize] = 0;
        }
        let mut source = Source::new(fixture.bytes);
        source.short = 3;
        let mut fonts = [C8FontSource {
            source: Source::new(crate::pdf::drawing_font()),
            face: 0,
        }];
        let mut sink = Sink::default();
        let limits = Limits {
            io_chunk_bytes: 64,
            ..Limits::default()
        };
        let result = convert_c8_native_pdf(
            &mut source,
            &mut sink,
            C8FontSources {
                sources: &mut fonts,
                roles: roles(),
            },
            None,
            ComposeOptions::default(),
            &limits,
            &NeverCancel,
        );
        assert!(source.max_request <= 64);
        if corrupt {
            let error = result.unwrap_err();
            assert_eq!(page_image(&error), (Some(1), Some(1)));
            assert!(!sink.bytes.ends_with(b"%%EOF\n"));
        } else {
            let report = result.unwrap();
            assert_eq!(report.type3_images, 1);
            assert_eq!(report.output_pages, 1);
            let text = crate::test_support::pdf_text(&sink.bytes);
            assert!(text.contains("/BM /Multiply"));
            let first = text.find("<0041> Tj").unwrap();
            let image = text.find("/Im0 Do").unwrap();
            let last = text.rfind("<0041> Tj").unwrap();
            assert!(first < image && image < last);
            let raster = render_original_pdf_at(&sink.bytes, "741.9");
            assert!(raster.starts_with(b"P5\n100 200\n255\n"));
        }
    }
}
