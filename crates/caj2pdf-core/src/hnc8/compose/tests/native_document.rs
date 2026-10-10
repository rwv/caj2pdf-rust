// SPDX-License-Identifier: MIT

use super::*;
use crate::hnc8::{C8PageFonts, NativeSymbolGlyph, SymbolFontIdentity};
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
                symbol_glyphs: &[],
                symbol_font: None,
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
                symbol_glyphs: &[],
                symbol_font: None,
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
            symbol_glyphs: &[],
            symbol_font: None,
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
                symbol_glyphs: &[],
                symbol_font: None,
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
                symbol_glyphs: &[],
                symbol_font: None,
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
                symbol_glyphs: &[],
                symbol_font: None,
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
                symbol_glyphs: &[],
                symbol_font: None,
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

fn check_hnb_type3_after_text(corrupt: bool) -> Vec<u8> {
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
            symbol_glyphs: &[],
            symbol_font: None,
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
    }
    sink.bytes
}

#[test]
fn hnb_type3_after_text_keeps_order_and_rejects_malformed_payloads() {
    for corrupt in [false, true] {
        check_hnb_type3_after_text(corrupt);
    }
}

#[test]
fn independent_render_checks_hnb_type3_after_text() {
    let pdf = check_hnb_type3_after_text(false);
    let raster = render_original_pdf_at(&pdf, "741.9");
    assert!(raster.starts_with(b"P5\n100 200\n255\n"));
}

/// Mode-0 symbols: two codes decoding to U+2019, and U+2014/U+FF0D codes.
fn mode_zero_symbol_text(_: &[Record]) -> Vec<u8> {
    [
        [0x8001, 60],
        [0x8002, 0x1084],
        [10, 0xa1af],
        [40, 0xa3a7],
        [70, 0xa1aa],
        [100, 0xa3ad],
        [130, 0xa3a7],
        [0x8004, 1],
    ]
    .into_iter()
    .flatten()
    .flat_map(u16::to_le_bytes)
    .collect()
}

fn convert_mode_zero_symbols(
    variant: Variant,
    mode: u8,
    symbols: Option<usize>,
    symbol_glyphs: &[NativeSymbolGlyph],
) -> (Result<ComposeReport>, Vec<u8>) {
    convert_bound_symbols(variant, mode, symbols, symbol_glyphs, None)
}

fn convert_bound_symbols(
    variant: Variant,
    mode: u8,
    symbols: Option<usize>,
    symbol_glyphs: &[NativeSymbolGlyph],
    symbol_font: Option<&SymbolFontIdentity>,
) -> (Result<ComposeReport>, Vec<u8>) {
    let mut fixture = fixture_with_text(variant, &[vec![]], mode_zero_symbol_text);
    let count_at = if variant == Variant::C8 { 8 } else { 0x90 };
    fixture.bytes[count_at + 4] = mode;
    // Original rectangle (width 600) at U+E000 and triangle (1000) at U+E001.
    let mut fonts = [
        C8FontSource {
            source: Source::new(crate::pdf::drawing_font()),
            face: 0,
        },
        C8FontSource {
            source: Source::new(crate::hnc8::labelled_font([0xe000, 0xe001])),
            face: 0,
        },
    ];
    let mut sink = Sink::default();
    let result = convert_c8_native_pdf(
        &mut Source::new(fixture.bytes),
        &mut sink,
        C8FontSources {
            sources: &mut fonts,
            roles: C8PageFonts { symbols, ..roles() },
            symbol_glyphs,
            symbol_font,
        },
        None,
        ComposeOptions::default(),
        &Limits::default(),
        &NeverCancel,
    );
    (result, sink.bytes)
}

fn symbol_glyphs(entries: &[(u16, char)]) -> Vec<NativeSymbolGlyph> {
    entries
        .iter()
        .map(|&(code, glyph)| NativeSymbolGlyph { code, glyph })
        .collect()
}

#[test]
fn mode_zero_symbol_glyphs_keep_source_shapes_independent_of_text() {
    let width = |glyph| if glyph == '\u{e000}' { 600 } else { 1000 };
    for (quote, apostrophe, dash) in [
        ('\u{e000}', '\u{e001}', '\u{e000}'),
        ('\u{e001}', '\u{e000}', '\u{e001}'),
    ] {
        // Two codes share U+2019 but not a glyph; one glyph carries U+2014
        // and U+FF0D.
        let map = symbol_glyphs(&[
            (0xa1af, quote),
            (0xa3a7, apostrophe),
            (0xa1aa, dash),
            (0xa3ad, dash),
        ]);
        let (result, bytes) = convert_mode_zero_symbols(Variant::HnB, 0, Some(1), &map);
        assert_eq!(result.unwrap().output_pages, 1);
        let pdf = crate::test_support::pdf_text(&bytes);
        assert!(!pdf.contains("/ActualText"));
        let symbol_draws: Vec<_> = pdf
            .lines()
            .filter(|line| line.starts_with("BT /F4 1 Tf"))
            .collect();
        assert_eq!(symbol_draws.len(), 5);
        let shown = pdf.split("BT /F4 1 Tf").skip(1).map(|draw| {
            draw.split(" Tj ET")
                .next()
                .unwrap()
                .rsplit(' ')
                .next()
                .unwrap()
        });
        assert_eq!(
            shown.collect::<Vec<_>>(),
            ["<0001>", "<0002>", "<0003>", "<0004>", "<0002>"]
        );
        let widths = [quote, apostrophe, dash, dash].map(width);
        assert!(pdf.contains(&format!(
            "/W [ 1 [ {} {} {} {} ] ]",
            widths[0], widths[1], widths[2], widths[3]
        )));
        let unicode = pdf
            .split("/ToUnicode ")
            .skip(1)
            .map(|tail| {
                let object = tail.split(' ').next().unwrap();
                crate::test_support::inflated_stream(&bytes, format!("\n{object} 0 obj").as_bytes())
            })
            .find(|cmap| cmap.windows(16).any(|part| part == b"CajMappedUnicode"))
            .unwrap();
        let unicode = String::from_utf8(unicode).unwrap();
        assert!(
            unicode.contains(
                "4 beginbfchar\n<0001> <2019>\n<0002> <2019>\n<0003> <2014>\n<0004> <FF0D>\n"
            ),
            "{unicode}"
        );
    }
}

#[test]
fn mode_zero_symbol_glyph_maps_refuse_missing_ambiguous_or_inapplicable_entries() {
    let valid = symbol_glyphs(&[(0xa1af, '\u{e000}')]);
    for (variant, mode, symbols, map, reason) in [
        (
            Variant::HnB,
            0,
            None,
            valid.clone(),
            "native symbol glyphs require a symbols font role",
        ),
        (
            Variant::HnB,
            0,
            Some(1),
            symbol_glyphs(&[(0xa3c1, '\u{e000}')]),
            "native symbol glyph code is not an HN-B mode-0 symbol",
        ),
        (
            Variant::HnB,
            0,
            Some(1),
            symbol_glyphs(&[(0xa1af, '\u{e000}'), (0xa1af, '\u{e001}')]),
            "native symbol glyph code is mapped more than once",
        ),
        (
            Variant::HnB,
            0,
            Some(1),
            symbol_glyphs(&[(0xa1af, 'B')]),
            "symbols font does not map a native symbol glyph",
        ),
        (
            Variant::HnB,
            2,
            Some(1),
            valid.clone(),
            "native symbol glyphs apply only to HN-B mode-0 text",
        ),
        (
            Variant::C8,
            2,
            Some(1),
            valid,
            "native symbol glyphs apply only to HN-B mode-0 text",
        ),
    ] {
        let (result, bytes) = convert_mode_zero_symbols(variant, mode, symbols, &map);
        let error = result.unwrap_err();
        assert_eq!(error.reason, reason);
        assert!(matches!(error.kind, ErrorKind::Malformed), "{error:?}");
        assert!(!bytes.ends_with(b"%%EOF\n"));
    }
}

#[test]
fn symbol_glyph_maps_bound_to_a_font_identity_refuse_other_fonts() {
    let mut source = Source::new(crate::hnc8::labelled_font([0xe000, 0xe001]));
    let font =
        crate::pdf::OpenTypeFont::read(&mut source, 0, &Limits::default(), &NeverCancel).unwrap();
    let identity = SymbolFontIdentity {
        checksum_adjustment: font.checksum_adjustment().unwrap(),
        postscript_name: font.postscript_name().unwrap(),
    };
    let map = symbol_glyphs(&[
        (0xa1af, '\u{e000}'),
        (0xa3a7, '\u{e001}'),
        (0xa1aa, '\u{e000}'),
        (0xa3ad, '\u{e000}'),
    ]);
    let (result, bytes) = convert_bound_symbols(Variant::HnB, 0, Some(1), &map, Some(&identity));
    assert_eq!(result.unwrap().output_pages, 1);
    assert!(bytes.ends_with(b"%%EOF\n"));
    let other_checksum = SymbolFontIdentity {
        checksum_adjustment: identity.checksum_adjustment ^ 1,
        ..identity.clone()
    };
    let other_name = SymbolFontIdentity {
        postscript_name: identity.postscript_name.clone() + "X",
        ..identity.clone()
    };
    for (map, symbol_font, reason) in [
        (
            map.clone(),
            other_checksum,
            "symbols font does not match the expected identity",
        ),
        (
            map.clone(),
            other_name,
            "symbols font does not match the expected identity",
        ),
        (
            vec![],
            identity,
            "symbol font identity requires native symbol glyphs",
        ),
    ] {
        let (result, bytes) =
            convert_bound_symbols(Variant::HnB, 0, Some(1), &map, Some(&symbol_font));
        let error = result.unwrap_err();
        assert_eq!(error.reason, reason);
        assert!(!bytes.ends_with(b"%%EOF\n"));
    }
}
