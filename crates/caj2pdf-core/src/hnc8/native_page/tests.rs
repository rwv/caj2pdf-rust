// SPDX-License-Identifier: MIT

use super::*;
use crate::ErrorKind;
use crate::Limits;
use crate::pdf::{BilevelImageSpec, OpenTypeFont};
use crate::test_support::page_image;
use std::io::Write;
use std::{cell::Cell, rc::Rc};

struct Source {
    bytes: Vec<u8>,
    fail: Rc<Cell<bool>>,
    largest: usize,
    signal_on_read: Option<(u64, Rc<Cell<bool>>)>,
}
impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> crate::Result<usize> {
        if self.fail.get() {
            return Err(invalid("original read failure"));
        }
        if let Some((at, signal)) = &self.signal_on_read
            && offset >= *at
        {
            signal.set(true);
        }
        self.largest = self.largest.max(out.len());
        let at = offset as usize;
        let count = out.len().min(3).min(self.bytes.len().saturating_sub(at));
        out[..count].copy_from_slice(&self.bytes[at..at + count]);
        Ok(count)
    }
}
#[derive(Default)]
struct Cancel(Rc<Cell<bool>>);
impl Cancellation for Cancel {
    fn is_cancelled(&self) -> bool {
        self.0.get()
    }
}
#[derive(Default)]
struct Sink {
    bytes: Vec<u8>,
    fail: Rc<Cell<bool>>,
}
impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.fail.get() {
            return Err(invalid("original write failure").into());
        }
        let count = bytes.len().min(7);
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn source(bytes: Vec<u8>) -> Source {
    Source {
        bytes,
        fail: Rc::default(),
        largest: 0,
        signal_on_read: None,
    }
}
fn fixture(words: &[[u16; 2]], images: u32) -> Source {
    let mut bytes = vec![0; 100];
    bytes[0] = 0xc8;
    bytes[8] = 1;
    bytes[12] = 2;
    for (offset, value) in [(28, 4652u16), (30, 4274), (32, 600), (34, 600)] {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }
    bytes[80..84].copy_from_slice(&100u32.to_le_bytes());
    bytes[84..88].copy_from_slice(&(words.len() as u32 * 4).to_le_bytes());
    bytes[88..92].copy_from_slice(&images.to_le_bytes());
    let end = 100 + words.len() as u32 * 4;
    bytes[96..100].copy_from_slice(&end.to_le_bytes());
    for pair in words {
        for word in pair {
            bytes.extend(word.to_le_bytes());
        }
    }
    source(bytes)
}
fn roles() -> C8PageFonts {
    C8PageFonts {
        cjk: 0,
        latin: 1,
        alternate_latin: Some(2),
        decoration: Some((1, 'A')),
        symbols: None,
        latin_state3: None,
        latin_state28: None,
        latin_state31: None,
    }
}
fn image() -> Vec<[u16; 2]> {
    vec![
        [0x800a, 0xd300],
        [0xc000 | 4682, 4314],
        [0xc000 | 80, 50],
        [0xc050, 0xc033],
        [0xc037, 0xc000],
        [0xc06c, 0xc032],
        [0xc0f2, 0xc07a],
    ]
}
fn ordinary() -> Vec<[u16; 2]> {
    vec![
        [0x8001, 4350],
        [0x8002, 0x1084],
        [4682, 0xd6d0],
        [4772, 0xa0c1],
    ]
}

// Every run uses actual ranged traversal, a bounded writer, embedded original
// font and an open PDF document. Faults start only after resource preparation.
fn convert(
    words: &[[u16; 2]],
    declared: u32,
    top_first: &[bool],
    roles: C8PageFonts,
    mode: u8,
) -> (Result<u32>, Vec<u8>, bool) {
    convert_with_fonts(words, declared, top_first, roles, mode, Vec::new())
}

/// Original drawing font with its rectangle and triangle relabelled, in
/// ascending code order. No external glyph data is introduced.
pub(crate) fn labelled_font(codes: [u32; 2]) -> Vec<u8> {
    let mut bytes = crate::pdf::drawing_font();
    let table = bytes[12..]
        .as_chunks::<16>()
        .0
        .iter()
        .find(|entry| &entry[..4] == b"cmap")
        .unwrap();
    let offset = u32::from_be_bytes(table[8..12].try_into().unwrap()) as usize;
    for (index, code) in codes.into_iter().enumerate() {
        for at in [offset + 28 + index * 12, offset + 32 + index * 12] {
            bytes[at..at + 4].copy_from_slice(&code.to_be_bytes());
        }
    }
    bytes
}

/// Like [`convert`], but `custom` (when nonempty) replaces the shared page
/// font with one distinct embedded resource per role index.
fn convert_with_fonts(
    words: &[[u16; 2]],
    declared: u32,
    top_first: &[bool],
    roles: C8PageFonts,
    mode: u8,
    custom: Vec<Vec<u8>>,
) -> (Result<u32>, Vec<u8>, bool) {
    let mut input = fixture(words, declared);
    if matches!(mode, 12 | 13 | 18 | 19 | 20 | 21) {
        // Original compact HN-B wrapper around the same authored record stream.
        let c8 = &input.bytes;
        let text_offset = if mode == 21 { 236 } else { 228 };
        let mut bytes = vec![0; text_offset];
        bytes[..4].copy_from_slice(b"HN\0\0");
        bytes[4..8].copy_from_slice(&200_u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&136_u32.to_le_bytes());
        bytes[144..148].copy_from_slice(&1_u32.to_le_bytes());
        bytes[148] = if matches!(mode, 18..=20) { 0 } else { 2 };
        bytes[164..172].copy_from_slice(&c8[28..36]);
        bytes[216..220].copy_from_slice(&(text_offset as u32).to_le_bytes());
        bytes[220..224].copy_from_slice(&(c8.len() as u32 - 100).to_le_bytes());
        if mode == 21 {
            bytes[136..140].copy_from_slice(&0xc8_u32.to_le_bytes());
            bytes[224..228].copy_from_slice(&declared.to_le_bytes());
            let end = (text_offset + c8.len() - 100) as u32;
            bytes[232..236].copy_from_slice(&end.to_le_bytes());
        }
        // Compact text-only indexes carry a zero third word.
        bytes.extend_from_slice(&c8[100..]);
        input = source(bytes);
    }
    let mut sink = Sink::default();
    let input_fault = input.fail.clone();
    let output_fault = sink.fail.clone();
    let limits = Limits {
        io_chunk_bytes: 64,
        ..Default::default()
    };
    let cancel = Cancel::default();
    let result = {
        let mut reader = Hnc8Reader::open(&mut input, &limits, &cancel).unwrap();
        reader.next_page().unwrap();
        match mode {
            1 => reader.header.variant = Variant::HnA,
            2 => reader.current = None,
            3 => reader.header.page_size = None,
            4 => reader.header.native_origin = None,
            5 => reader.header.page_size = Some([0, 1]),
            14 => reader.header.native_mode = None,
            15 => reader.header.native_mode = Some(0),
            16 => reader.header.native_mode = Some(99),
            17 => {
                reader.header.variant = Variant::HnB;
                reader.header.native_mode = Some(99);
            }
            _ => (),
        }
        let mut document = PdfDocument::new(&mut sink, &limits, &cancel).unwrap();
        let mut font_bytes = if mode == 20 {
            crate::pdf::symbol_font()
        } else {
            crate::pdf::drawing_font()
        };
        if matches!(mode, 11 | 13 | 19) {
            // Relabel an original glyph as the test's source symbol.
            // No source font outline or character shape is imported.
            let table = font_bytes[12..]
                .as_chunks::<16>()
                .0
                .iter()
                .find(|entry| &entry[..4] == b"cmap")
                .unwrap();
            let offset = u32::from_be_bytes(table[8..12].try_into().unwrap()) as usize;
            let code = words
                .iter()
                .find(|pair| {
                    pair[0] < 0x8000
                        && (pair[1] >= 0xa000 || matches!(pair[1], 0x9ff5 | 0x006c | 0x0070))
                })
                .unwrap()[1];
            let character = if code == 0x006c {
                Some('l')
            } else if code == 0x0070 {
                Some('p')
            } else if mode == 19 {
                decode_native_character_for_mode(0, code)
            } else {
                decode_native_character(code)
            }
            .unwrap() as u32;
            assert_ne!(character, 65);
            // Keep the two format-12 groups sorted for ASCII punctuation too.
            let group = offset + if character < 65 { 28 } else { 40 };
            for at in [group, group + 4] {
                font_bytes[at..at + 4].copy_from_slice(&character.to_be_bytes());
            }
        }
        let mut sources: Vec<_> = std::iter::once(font_bytes)
            .chain(custom)
            .map(source)
            .collect();
        let mut readers = Vec::new();
        for font_source in &mut sources {
            readers.push(OpenTypeFont::read(font_source, 0, &limits, &cancel).unwrap());
        }
        let font = document.add_font(&readers[0]).unwrap();
        let mut distinct = Vec::new();
        for reader in &readers[1..] {
            distinct.push(document.add_font(reader).unwrap());
        }
        let fonts = if distinct.is_empty() {
            vec![&font, &font, &font]
        } else {
            distinct.iter().collect()
        };
        let mut images = Vec::new();
        for _ in 0..declared {
            let mut image = document
                .begin_bilevel_image(BilevelImageSpec {
                    pixel_width: 2,
                    pixel_height: 2,
                    row_stride: 1,
                })
                .unwrap();
            image.write_all(&[0x80, 0x40]).unwrap();
            images.push(image.finish().unwrap());
        }
        if mode == 6 {
            input_fault.set(true);
        }
        if mode == 7 {
            output_fault.set(true);
        }
        if mode == 9 {
            reader.source_mut().signal_on_read = Some((112, cancel.0.clone()));
        }
        if mode == 10 {
            reader.source_mut().signal_on_read = Some((112, output_fault.clone()));
        }
        let slice = if mode == 8 { &[][..] } else { &images[..] };
        let outcome = write_c8_native_page(
            &mut reader,
            &mut document,
            &fonts,
            roles,
            &[],
            slice,
            top_first,
        );
        cancel.0.set(false);
        input_fault.set(false);
        output_fault.set(false);
        let handles = std::iter::once(&font).chain(&distinct);
        for (handle, reader) in handles.zip(&mut readers) {
            // A poisoned document also rejects the embedding; finish reports it.
            let _ = document.embed_font(handle, reader);
        }
        let finished = document.finish().is_ok();
        (outcome, finished)
    };
    assert!(input.largest <= 64);
    (result.0, sink.bytes, result.1)
}

pub(crate) fn mixed_page() -> Vec<u8> {
    let mut words = ordinary();
    words.extend(image());
    words.extend([
        [0x8006, 0xa381],
        [4682, 4350],
        [4912, 4350],
        [0xffff, 5],
        [0x801d, 4],
        [4772, 0xa0c1],
    ]);
    let mut second_image = image();
    second_image[3..].copy_from_slice(&[
        [0xc000, 0xc0ff],
        [0xc080, 0xc001],
        [0xc0fe, 0xc07f],
        [0xc000, 0xc0ff],
    ]);
    words.extend(second_image);
    words.extend([
        [0x801d, 0],
        [0x8067, 9],
        [4772, 0xa0c1],
        [0x8010, 1],
        [4682, 4524],
        [4832, 4524],
        [0x8004, 1],
    ]);
    let (result, pdf, finished) = convert(&words, 2, &[false, true], roles(), 0);
    assert_eq!(result.unwrap(), 0);
    assert!(finished);
    let content = crate::test_support::pdf_text(&pdf);
    let mut after = 0;
    for operator in [
        "/F0 1 Tf",
        "<4E2D> Tj",
        "/F1 1 Tf",
        "<0041> Tj",
        "/Im0 Do",
        " l S Q",
        "/F2 1 Tf",
        "/Im1 Do",
        "/F1 1 Tf",
        "/Artifact BMC",
    ] {
        after += content[after..].find(operator).unwrap() + operator.len();
    }
    assert!(content.contains("/ActualText ()"));
    assert_eq!(
        crate::test_support::bilevel_pixels(&pdf),
        [vec![0x80, 0x40], vec![0x80, 0x40]]
    );
    if let Some(path) = std::env::var_os("CAJ2PDF_NATIVE_PAGE_TEST_OUTPUT") {
        std::fs::write(path, &pdf).unwrap();
    }
    pdf
}

#[test]
fn native_records_drive_actual_mixed_page_in_source_order() {
    mixed_page();
}

#[test]
fn unsupported_content_and_missing_glyphs_poison_the_open_page() {
    for tail in [
        vec![[4800, 0xa0a6]],
        vec![[4800, 0xa3a6]],
        vec![[4800, 0xa0c2]],
        vec![[4800, 0xa080]],
        vec![[0x8072, 1]],
        vec![[0x8072, 0xd2e4]],
        vec![[0x8072, 0xd2e6]],
        vec![[0x80ce, 0], [4800, 0xa1a1]],
        vec![[0x8006, 0xa384], [4682, 4350], [4912, 4350]],
        vec![[0x8004, 0]],
    ] {
        let mut words = ordinary();
        words.extend(tail);
        words.push([0x8004, 1]);
        let (result, _, finished) = convert(&words, 0, &[], roles(), 0);
        let error = result.unwrap_err();
        assert_eq!(page_image(&error).0, Some(1));
        assert!(error.offset >= Some(116));
        assert!(!finished);
    }
    let mut words = ordinary();
    words.push([0x8004, 1]);
    let mut missing = roles();
    missing.cjk = 10;
    let (result, _, finished) = convert(&words, 0, &[], missing, 0);
    assert!(result.is_err());
    assert!(!finished);
}

#[test]
fn resource_and_header_errors_are_checked_before_page_output() {
    let words = [[0x8004, 1]];
    for mode in 1..=5 {
        let (result, pdf, _) = convert(&words, 0, &[], roles(), mode);
        assert!(result.is_err());
        assert!(!crate::test_support::pdf_text(&pdf).contains("/Type /Page /"));
    }
    let (result, _, _) = convert(&words, 0, &[false], roles(), 0);
    assert!(result.is_err());
    let (result, _, _) = convert(&words, 1, &[], roles(), 8);
    assert!(result.is_err());
}

#[test]
fn source_and_output_failures_leave_no_finished_document() {
    let mut words = ordinary();
    words.push([0x8004, 1]);
    for mode in [6, 7, 9, 10] {
        let (result, _, finished) = convert(&words, 0, &[], roles(), mode);
        assert!(result.is_err());
        assert!(!finished);
    }
}

#[test]
fn decoration_requires_resource_style_and_verified_direction() {
    let drawing = [[0x8010, 1], [4682, 4524], [4832, 4524], [0x8004, 1]];
    let (result, _, finished) = convert(&drawing, 0, &[], roles(), 0);
    assert!(result.is_err());
    assert!(!finished);
    let mut words = ordinary();
    words.extend(drawing);
    // The fallback Latin font lacks the default alias, so this still fails.
    let mut missing = roles();
    missing.decoration = None;
    let (result, _, finished) = convert(&words, 0, &[], missing, 0);
    assert!(result.is_err());
    assert!(!finished);
    words[6][1] += 1; // diagonal endpoint
    let (result, _, finished) = convert(&words, 0, &[], roles(), 0);
    assert!(result.is_err());
    assert!(!finished);
}

#[test]
fn image_profile_and_coordinates_are_not_silently_guessed() {
    for index in [0, 3] {
        let mut words = image();
        words[index][1] = 0;
        words.push([0x8004, 1]);
        let (result, _, finished) = convert(&words, 1, &[false], roles(), 0);
        assert!(result.is_err());
        assert!(!finished);
    }
}

#[test]
fn fullwidth_colon_uses_active_latin_resource_and_independent_size_axes() {
    for (style, expected_width, expected_height) in [
        (0x10e7, 56.0, 56.0),
        (0x1048, 28.0, 63.0),
        (0x1102, 63.0, 28.0),
    ] {
        let words = [
            [0x8001, 4394],
            [0x8002, style],
            [5052, 0xa3ba],
            [0x801d, 4],
            [5052, 0xa3ba],
            [0x8004, 1],
        ];
        let (result, pdf, finished) = convert(&words, 0, &[], roles(), 11);
        assert_eq!(result.unwrap(), 0);
        assert!(finished);
        let text = crate::test_support::pdf_text(&pdf);
        assert!(text.contains("/F1 1 Tf"));
        assert!(text.contains("/F2 1 Tf"));
        assert_eq!(text.matches("<FF1A> Tj").count(), 2);
        let matrices: Vec<Vec<f64>> = text
            .lines()
            .filter_map(|line| line.split_once(" Tm ").map(|(matrix, _)| matrix))
            .map(|line| {
                line.split_whitespace()
                    .map(|word| word.parse().unwrap())
                    .collect()
            })
            .collect();
        assert_eq!(matrices.len(), 2);
        let width = expected_width * 75.0 / 301.0;
        let height = expected_height * 75.0 / 301.0;
        let unit = super::super::EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
        let expected = [
            width,
            0.0,
            0.0,
            height,
            420.0 * unit,
            480.0 * unit - height * 7.0 / 8.0,
        ];
        for matrix in matrices {
            for (actual, expected) in matrix.iter().zip(expected) {
                // PDF serialization keeps six fractional decimal places.
                assert!((actual - expected).abs() < 0.000001);
            }
        }
        let (missing, _, finished) = convert(&words, 0, &[], roles(), 0);
        assert!(missing.is_err());
        assert!(!finished);
    }
}

#[test]
fn controlled_symbols_preserve_unicode_resource_state_and_common_baseline() {
    let codes = [
        0xa0a6, 0xa0ae, 0xa0af, 0xa0ba, 0xaab1, 0xaab2, 0xa1aa, 0xa1ad, 0xa1ae, 0xa1c6, 0xa1c8,
        0xa2d9, 0xa2da, 0xa2db, 0xa2dc, 0xa2dd, 0xa2de, 0xa2df, 0xa3a3, 0xa3a5, 0xa3ab, 0xa3ac,
        0xa3ad, 0xa3ae, 0xa3af, 0xa3b0, 0xa3b1, 0xa3b2, 0xa3b3, 0xa3b4, 0xa3b5, 0xa3b6, 0xa3b7,
        0xa3b8, 0xa3b9, 0xa3bb, 0xa3bc, 0xa3bd, 0xa3be, 0xa3bf, 0xa3dc, 0xa3fb, 0xa3fd, 0xa9aa,
        0xaab3, 0xaca3,
    ];
    for code in codes {
        for style in [0x10e3, 0x1067] {
            let words = [
                [0x8001, 4394],
                [0x8002, style],
                [5072, code],
                [0x801d, 4],
                [5072, code],
                [0x8004, 1],
            ];
            let (result, pdf, finished) = convert(&words, 0, &[], roles(), 11);
            assert_eq!(result.unwrap(), 0);
            assert!(finished);
            let text = crate::test_support::pdf_text(&pdf);
            let unicode = decode_native_character(code).unwrap() as u32;
            assert_eq!(text.matches(&format!("<{unicode:04X}> Tj")).count(), 2);
            let fixed = matches!(code, 0xa1c6 | 0xa1c8 | 0xa9aa | 0xaab3 | 0xaca3);
            assert_eq!(text.matches("/F1 1 Tf").count(), if fixed { 2 } else { 1 });
            assert_eq!(text.matches("/F2 1 Tf").count(), usize::from(!fixed));
            let height = if style == 0x10e3 { 31.0 } else { 56.0 } * 75.0 / 301.0;
            let expected_y = 480.0 * super::super::EMPIRICAL_COORDINATE_POINTS_PER_UNIT - height;
            let mut count = 0;
            for matrix in text.lines().filter_map(|line| line.split_once(" Tm ")) {
                let y: f64 = matrix.0.split_whitespace().last().unwrap().parse().unwrap();
                assert!((y - expected_y).abs() < 0.000001);
                count += 1;
            }
            assert_eq!(count, 2);
        }
    }
}

#[test]
fn parentheses_preserve_independent_axes_and_active_resource() {
    // Predictions from the original independent-axis source control.
    for (style, opening_x, closing_x, down) in [
        (0x1067, 19.0, 18.0, -10.0),
        (0x10e3, 35.0, 33.0, 1.0),
        (0x1048, 18.0, 16.0, -14.0),
        (0x1102, 39.0, 37.0, 3.0),
    ] {
        for (code, x_offset) in [(0xa3a8, opening_x), (0xa3a9, closing_x)] {
            let words = [
                [0x8001, 4394],
                [0x8002, style],
                [4902, code],
                [0x801d, 4],
                [4902, code],
                [0x8004, 1],
            ];
            let (result, pdf, finished) = convert(&words, 0, &[], roles(), 11);
            assert_eq!(result.unwrap(), 0);
            assert!(finished);
            let text = crate::test_support::pdf_text(&pdf);
            assert!(text.contains("/F1 1 Tf"));
            assert!(text.contains("/F2 1 Tf"));
            let unicode = if code == 0xa3a8 { 0xff08 } else { 0xff09 };
            assert_eq!(text.matches(&format!("<{unicode:04X}> Tj")).count(), 2);
            let mut count = 0;
            for (matrix, _) in text.lines().filter_map(|line| line.split_once(" Tm ")) {
                let values: Vec<f64> = matrix
                    .split_whitespace()
                    .map(|word| word.parse().unwrap())
                    .collect();
                let unit = super::super::EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
                assert!((values[4] - (270.0 + x_offset) * unit).abs() < 0.000001);
                assert!((values[5] - ((495.0 - down) * unit - values[3])).abs() < 0.000002);
                count += 1;
            }
            assert_eq!(count, 2);
        }
    }
    // Invalid fields fail before indexing the measured offset table.
    for style in [0x1024, 0x1081, 0x1124, 0x1089] {
        let words = [[0x8001, 4394], [0x8002, style], [4902, 0xa3a8], [0x8004, 1]];
        let (result, _, finished) = convert(&words, 0, &[], roles(), 11);
        assert!(result.is_err());
        assert!(!finished);
    }
}

#[test]
fn ideographic_space_and_punctuation_keep_distinct_resources_and_baselines() {
    for (style, latin_down) in [(0x1067, 3.0), (0x10e3, 9.0), (0x1048, 1.0), (0x1102, 9.0)] {
        for (code, unicode) in [(0xa1a1, 0x3000), (0xa1a2, 0x3001), (0xa1a3, 0x3002)] {
            let words = [
                [0x8001, 4394],
                [0x8002, style],
                [4902, code],
                [0x801d, 4],
                [4902, code],
                [0x8004, 1],
            ];
            let (result, pdf, finished) = convert(
                &words,
                0,
                &[],
                roles(),
                if code == 0xa1a3 { 13 } else { 11 },
            );
            assert_eq!(result.unwrap(), 0);
            assert!(finished);
            let text = crate::test_support::pdf_text(&pdf);
            assert_eq!(text.matches(&format!("<{unicode:04X}> Tj")).count(), 2);
            if code == 0xa1a1 {
                assert_eq!(text.matches("/F0 1 Tf").count(), 2);
            } else {
                assert!(text.contains("/F1 1 Tf"));
                assert!(text.contains("/F2 1 Tf"));
            }
            let down = if code == 0xa1a1 { 0.0 } else { latin_down };
            for (matrix, _) in text.lines().filter_map(|line| line.split_once(" Tm ")) {
                let values: Vec<f64> = matrix
                    .split_whitespace()
                    .map(|word| word.parse().unwrap())
                    .collect();
                let unit = super::super::EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
                assert!((values[4] - 270.0 * unit).abs() < 0.000001);
                assert!((values[5] - ((495.0 - down) * unit - values[3])).abs() < 0.000002);
            }
        }
    }
}

#[test]
fn square_brackets_keep_ordinary_resource_under_alternate_state() {
    for (style, x_offset, down) in [
        (0x1021, 21.0, 3.0),
        (0x1022, 21.0, 1.0),
        (0x1041, 24.0, 3.0),
        (0x1067, 27.0, -15.0),
        (0x10e3, 48.0, -1.0),
        (0x1048, 24.0, -18.0),
        (0x1102, 54.0, 1.0),
    ] {
        for (code, unicode) in [(0xa3db, 0xff3b), (0xa3dd, 0xff3d)] {
            let words = [
                [0x8001, 4394],
                [0x8002, style],
                [4902, code],
                [0x801d, 4],
                [4902, code],
                [0x8004, 1],
            ];
            let (result, pdf, finished) = convert(&words, 0, &[], roles(), 11);
            assert_eq!(result.unwrap(), 0);
            assert!(finished);
            let text = crate::test_support::pdf_text(&pdf);
            assert_eq!(text.matches(&format!("<{unicode:04X}> Tj")).count(), 2);
            assert_eq!(text.matches("/F1 1 Tf").count(), 2);
            assert!(!text.contains("/F2 1 Tf"));
            for (matrix, _) in text.lines().filter_map(|line| line.split_once(" Tm ")) {
                let values: Vec<f64> = matrix
                    .split_whitespace()
                    .map(|word| word.parse().unwrap())
                    .collect();
                let unit = super::super::EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
                assert!((values[4] - (270.0 + x_offset) * unit).abs() < 0.000001);
                assert!((values[5] - ((495.0 - down) * unit - values[3])).abs() < 0.000002);
            }
        }
    }
}

#[test]
fn quotation_marks_and_middle_dot_reuse_controlled_offsets() {
    for (style, small_x, quote_x, quote_down) in [
        (0x1067, 7.0, 18.0, -10.0),
        (0x10e3, 13.0, 33.0, 1.0),
        (0x1048, 7.0, 16.0, -14.0),
        (0x1102, 15.0, 37.0, 3.0),
    ] {
        for (code, unicode) in [
            (0xa1a4, 0x00b7),
            (0xa1af, 0x2019),
            (0xa1b0, 0x201c),
            (0xa1b1, 0x201d),
        ] {
            let words = [
                [0x8001, 4394],
                [0x8002, style],
                [4902, code],
                [0x801d, 4],
                [4902, code],
                [0x8004, 1],
            ];
            let (result, pdf, finished) = convert(&words, 0, &[], roles(), 11);
            assert_eq!(result.unwrap(), 0);
            assert!(finished);
            let text = crate::test_support::pdf_text(&pdf);
            assert_eq!(text.matches(&format!("<{unicode:04X}> Tj")).count(), 2);
            assert!(text.contains("/F1 1 Tf"));
            assert!(text.contains("/F2 1 Tf"));
            for (matrix, _) in text.lines().filter_map(|line| line.split_once(" Tm ")) {
                let values: Vec<f64> = matrix
                    .split_whitespace()
                    .map(|word| word.parse().unwrap())
                    .collect();
                let unit = super::super::EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
                let (dx, down) = match code {
                    0xa1a4 => (small_x, 15.0 - values[3] / (8.0 * unit)),
                    0xa1af => (small_x, 15.0),
                    _ => (quote_x, quote_down),
                };
                assert!((values[4] - (270.0 + dx) * unit).abs() < 0.000001);
                assert!((values[5] - ((495.0 - down) * unit - values[3])).abs() < 0.000002);
            }
        }
    }
}

#[test]
fn controlled_nonpainting_records_preserve_mixed_page_output() {
    let make_words = |control: Option<&[[u16; 2]]>| {
        let mut words = vec![[0x8001, 4350], [0x8002, 0x1084], [0x801d, 4], [0x8067, 6]];
        let mut operations = vec![vec![[4682, 0xd6d0], [4772, 0xa0c1]]];
        for style in [0xa381, 0xa383, 0xa38b] {
            let mut operation = vec![[0x8006, style], [4682, 4350], [4912, 4380]];
            if style != 0xa383 {
                operation.push([0xffff, 5]);
            }
            operations.push(operation);
        }
        operations.push(vec![[0x8010, 1], [4682, 4524], [4832, 4524], [0xffff, 5]]);
        operations.push(image());
        for operation in operations {
            if let Some(control) = control {
                words.extend_from_slice(control);
            }
            words.extend(operation);
        }
        words.push([0x8004, 1]);
        words
    };
    let (result, baseline, finished) = convert(&make_words(None), 1, &[false], roles(), 0);
    assert_eq!(result.unwrap(), 0);
    assert!(finished);
    for (tag, values) in [
        (0x80ce, &[1][..]),
        (0x8021, &[0x2000][..]),
        (0x80d0, &[0][..]),
        (0x80d1, &[1][..]),
        (0x80d2, &[0][..]),
        (0x80d3, &[0, 1, 2][..]),
        (0x80d5, &[0][..]),
        (0x9002, &[0][..]),
        (0x8072, &[0, 0x1042, 0xa3a8, 0xa0f2, 0xd2e5][..]),
        (0x8073, &[0, 8, 38, 39, 40, 41, 42, 43, 0x8004, 0xffff][..]),
        (
            0x8074,
            &[
                0, 0x0204, 0xb4a2, 0xd4b4, 0x24a7, 0xa1a1, 0xa3a9, 0x8004, 0xffff,
            ][..],
        ),
        (0xc053, &[0, 0x1377, 0x137b, 5200, 5700, 0xffff][..]),
        (
            0xc054,
            &[0, 0x139e, 0x15a8, 0x1607, 0x1676, 5200, 5700, 0xffff][..],
        ),
    ] {
        for &value in values {
            let (result, pdf, finished) =
                convert(&make_words(Some(&[[tag, value]])), 1, &[false], roles(), 0);
            assert_eq!(result.unwrap(), 0);
            assert!(finished);
            assert_eq!(pdf, baseline, "control {tag:04x}/{value:04x}");
        }
    }
    for mode in [0, 1] {
        let (result, expected, finished) = convert(
            &make_words(Some(&[[0x80ce, mode]])),
            1,
            &[false],
            roles(),
            0,
        );
        result.unwrap();
        assert!(finished);
        let words = make_words(Some(&[[0x80ce, mode], [0x9002, 0]]));
        let (result, pdf, finished) = convert(&words, 1, &[false], roles(), 0);
        assert!(result.is_ok() && finished);
        assert_eq!(pdf, expected);
        assert!(convert(&words, 1, &[false], roles(), 12).0.is_err());
        for payload in [[342, 5], [420, 7], [0, 0], [0xffff, 0xffff], [0x8004, 1]] {
            let words = make_words(Some(&[[0x80ce, mode], [0x80cc, 0x0204], payload]));
            let (result, pdf, finished) = convert(&words, 1, &[false], roles(), 0);
            result.unwrap();
            assert!(finished);
            assert_eq!(pdf, expected);
            assert!(convert(&words, 1, &[false], roles(), 12).0.is_err());
        }
    }
    for payload in [
        "".to_owned(),
        "fixture!".to_owned(),
        "E:\\fixture\\missing".to_owned(),
        "font.ttf".to_owned(),
        "x".repeat(252),
    ] {
        let mut raw = vec![0x80cc, 0x102 + payload.len() as u16];
        raw.extend(payload.bytes().map(|byte| 0xe000 | u16::from(byte)));
        assert_eq!(raw.len() % 2, 0);
        let control = raw.as_chunks::<2>().0;
        let (result, pdf, finished) = convert(&make_words(Some(control)), 1, &[false], roles(), 0);
        assert_eq!(result.unwrap(), 0);
        assert!(finished);
        assert_eq!(pdf, baseline);
    }
}

#[test]
fn end_payload_does_not_change_rendered_content() {
    let mut words = ordinary();
    words.extend(image());
    words.push([0x8004, 1]);
    let (result, baseline, finished) = convert(&words, 1, &[false], roles(), 0);
    assert_eq!(result.unwrap(), 0);
    assert!(finished);
    for value in [0, 39, 40, 41, 42, 43, 44, 0xffff] {
        *words.last_mut().unwrap() = [0x8004, value];
        let (result, pdf, finished) = convert(&words, 1, &[false], roles(), 0);
        assert_eq!(result.unwrap(), 0);
        assert!(finished);
        assert_eq!(pdf, baseline, "end payload {value:04x}");
    }
}

#[test]
fn explicit_axes_override_style_and_reset_at_the_next_style() {
    let words = [
        [0x8001, 4394],
        [0x8002, 0],
        [0x8070, 36],
        [0x8071, 36],
        [5072, 0xa0c1],
        [0x8002, 0x1084],
        [5072, 0xa0c1],
        [0x8004, 1],
    ];
    let (result, pdf, finished) = convert(&words, 0, &[], roles(), 0);
    result.unwrap();
    assert!(finished);
    let text = crate::test_support::pdf_text(&pdf);
    let matrices: Vec<Vec<f64>> = text
        .lines()
        .filter_map(|line| line.split_once(" Tm "))
        .map(|(matrix, _)| {
            matrix
                .split_whitespace()
                .map(|x| x.parse().unwrap())
                .collect()
        })
        .collect();
    assert_eq!(matrices.len(), 2);
    for (matrix, step) in matrices.iter().zip([36.0, 35.0]) {
        let em = step * 75.0 / 301.0;
        assert!((matrix[0] - em).abs() < 0.000001);
        assert!((matrix[3] - em).abs() < 0.000001);
        let unit = super::super::EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
        assert!((matrix[4] - (440.0 * unit + em / 8.0)).abs() < 0.000001);
        assert!((matrix[5] - (487.0 * unit - em)).abs() < 0.000001);
    }
    let mut reversed = words;
    reversed.swap(2, 3);
    assert_eq!(convert(&reversed, 0, &[], roles(), 0).1, pdf);
}

#[test]
fn unverified_axis_combinations_and_style_specific_offsets_fail_explicitly() {
    for tail in [
        vec![[0x8070, 36], [5072, 0xa0c1]],
        vec![[0x8071, 36], [5072, 0xa0c1]],
        vec![[0x8070, 36], [0x8071, 36], [5072, 0xa3a8]],
        vec![
            [0x8070, 36],
            [0x8071, 36],
            [0x8010, 1],
            [4800, 4400],
            [5000, 4400],
        ],
        vec![[0x8002, 0xe58c], [5072, 0xa3a8]],
    ] {
        let mut words = vec![[0x8001, 4394], [0x8002, 0x1084]];
        words.extend(tail);
        words.push([0x8004, 1]);
        let (result, _, finished) = convert(&words, 0, &[], roles(), 0);
        assert!(result.is_err());
        assert!(!finished);
    }
}

#[test]
fn skew_uses_width_survives_style_changes_and_resets_explicitly() {
    for (control, factor, mode, styles) in [
        (0x281d, 0.24, 0, &[0x1067, 0x10e3, 0xe58c][..]),
        (0x281c, 0.225, 0, &[0x1067, 0x10e3, 0xe58c][..]),
        (0x2815, 0.105, 12, &[0x1084, 0x10a5, 0x10a4, 0x08a5][..]),
    ] {
        for style in styles {
            let words = [
                [0x8001, 4394],
                [0x8002, *style],
                [4682, 0xd6d0],
                [0x8024, control],
                [4682, 0xd6d0],
                [0x8002, *style],
                [4682, 0xd6d0],
                [0x8024, 0x2800],
                [4682, 0xd6d0],
                [0x8004, 1],
            ];
            let (result, pdf, finished) = convert(&words, 0, &[], roles(), mode);
            result.unwrap();
            assert!(finished);
            let text = crate::test_support::pdf_text(&pdf);
            let matrices: Vec<Vec<f64>> = text
                .lines()
                .filter_map(|line| line.split_once(" Tm "))
                .map(|(matrix, _)| {
                    matrix
                        .split_whitespace()
                        .map(|x| x.parse().unwrap())
                        .collect()
                })
                .collect();
            assert_eq!(matrices.len(), 4);
            assert_eq!(matrices[0], matrices[3]);
            assert_eq!(matrices[1], matrices[2]);
            assert_eq!(matrices[0][2], 0.0);
            assert!((matrices[1][2] - matrices[0][0] * factor).abs() < 0.000001);
            for index in [0, 1, 3, 4, 5] {
                assert_eq!(matrices[0][index], matrices[1][index]);
            }
        }
    }
}

#[test]
fn unresolved_skewed_nontext_content_remains_an_error() {
    for tail in [vec![[0x8006, 0xa381], [4800, 4400], [5000, 4400]], image()] {
        let mut words = vec![[0x8001, 4394], [0x8002, 0x1084], [0x8024, 0x281d]];
        words.extend(tail);
        words.push([0x8004, 1]);
        let (result, _, finished) = convert(&words, 1, &[false], roles(), 0);
        assert!(result.is_err());
        assert!(!finished);
    }
}

#[test]
fn hnb_native_text_and_controlled_state_reuse_sequential_page_output() {
    let make = |control: &[[u16; 2]]| {
        let mut words = ordinary();
        words.extend_from_slice(control);
        words.extend([
            [4682, 0xd6d0],
            [4772, 0xa0c1],
            [0x8006, 0xa381],
            [4682, 4350],
            [4912, 4380],
            [0x8004, 1],
        ]);
        words
    };
    let (result, baseline, finished) = convert(&make(&[]), 0, &[], roles(), 12);
    result.unwrap();
    assert!(finished);
    assert_eq!(baseline, convert(&make(&[]), 0, &[], roles(), 0).1);
    for control in [
        vec![[0x8067, 7]],
        vec![[0x8069, 0x1084]],
        vec![[0x80ce, 1]],
        vec![[0x8072, 0x1084]],
        vec![[0x8072, 0xa0f3]],
        vec![[0x8072, 0xa0e7]],
        vec![[0x8072, 0xc2db]],
        vec![[0x8072, 0xd2f2]],
        vec![[0x8072, 0xcdc1]],
        vec![[0x8073, 79]],
        vec![[0x8073, 80]],
        vec![[0x8073, 81]],
        vec![[0x8073, 82]],
        vec![[0x8073, 83]],
        vec![[0x8074, 0x2815]],
        vec![[0x8074, 0xa0ec]],
        vec![[0x8074, 0xd3c9]],
        vec![[0x8074, 0xb0d7]],
        vec![[0x8074, 0xd1e9]],
        vec![[0x8073, 30]],
        vec![[0x8073, 31]],
        vec![[0x8073, 32]],
        vec![[0x8074, 0xb7bd]],
        vec![[0x8074, 0xcfc8]],
        vec![[0x8074, 0xc8cb]],
        vec![[0xc052, 0xa385], [0xd290, 0xb675]],
    ] {
        let (result, pdf, finished) = convert(&make(&control), 0, &[], roles(), 12);
        result.unwrap();
        assert!(finished);
        assert_eq!(pdf, baseline);
    }
    let (result, _, finished) = convert(&make(&[]), 1, &[false], roles(), 12);
    assert!(result.is_err());
    assert!(!finished);
}

#[test]
fn native_book_title_marks_preserve_verified_style_five_offsets_and_resources() {
    for mode in [11, 13] {
        for (code, unicode, x) in [(0xa1b6, 0x300a, 30.0), (0xa1b7, 0x300b, 20.0)] {
            let words = [
                [0x8001, 4394],
                [0x8002, 0x10a5],
                [4902, code],
                [0x801d, 4],
                [4902, code],
                [0x8004, 1],
            ];
            let (result, pdf, finished) = convert(&words, 0, &[], roles(), mode);
            result.unwrap();
            assert!(finished);
            let text = crate::test_support::pdf_text(&pdf);
            assert_eq!(text.matches(&format!("<{unicode:04X}> Tj")).count(), 2);
            assert!(text.contains("/F1 1 Tf") && text.contains("/F2 1 Tf"));
            for (matrix, _) in text.lines().filter_map(|line| line.split_once(" Tm ")) {
                let m: Vec<f64> = matrix
                    .split_whitespace()
                    .map(|v| v.parse().unwrap())
                    .collect();
                let unit = super::super::EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
                assert!((m[4] - (270.0 + x) * unit).abs() < 0.000001);
                assert!((m[5] - (499.0 * unit - m[3])).abs() < 0.000002);
            }
            let mut other = words;
            other[1][1] = 0x1084;
            assert!(convert(&other, 0, &[], roles(), mode).0.is_err());
        }
    }
}

#[test]
fn unverified_native_modes_cannot_use_mode_two_rendering() {
    for mode in 14..=17 {
        let (result, pdf, _) = convert(&ordinary(), 0, &[], roles(), mode);
        let error = result.unwrap_err();
        assert!(
            matches!(
                error,
                Error {
                    kind: ErrorKind::UnsupportedFormat,
                    reason: "native page rendering mode",
                    ..
                }
            ),
            "{error:?}"
        );
        assert!(!crate::test_support::pdf_text(&pdf).contains("/Type /Page "));
    }
}

#[test]
fn mode_zero_renders_cjk_and_distinct_latin_alphabets_in_source_order() {
    let words = [
        [0x8001, 4350],
        [0x8002, 0x1084],
        [4682, 0xd6d0],
        [0x801d, 4],
        [0x8067, 6],
        [4772, 0xa3c1],
        [4862, 0xa980],
        [0x801d, 0],
        [4952, 0xa3c1],
        [0x8004, 1],
    ];
    let (result, pdf, finished) = convert(&words, 0, &[], roles(), 18);
    assert_eq!(result.unwrap(), 0);
    assert!(finished);
    let pdf = crate::test_support::pdf_text(&pdf);
    let mut after = 0;
    for token in [
        "/F0 1 Tf",
        "<4E2D> Tj",
        "/F1 1 Tf",
        "<0041> Tj",
        "/F2 1 Tf",
        "<0041> Tj",
        "/F1 1 Tf",
    ] {
        after += pdf[after..].find(token).unwrap() + token.len();
    }
    let extent = 700.0 * super::super::EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
    let dimensions = pdf
        .split("/MediaBox [0 0 ")
        .nth(1)
        .unwrap()
        .split(']')
        .next()
        .unwrap();
    for dimension in dimensions.split_whitespace() {
        assert!((dimension.parse::<f64>().unwrap() - extent).abs() <= 0.000_000_5);
    }
}

#[test]
fn mode_zero_styles_axes_and_late_failures_remain_explicit() {
    for (style, code, axes) in [
        (0, 0xd6d0, false),
        (0x1000, 0xa3c1, false),
        (0x0484, 0xa3c1, false),
        (0x0884, 0xa980, false),
        (0x9c84, 0xd6d0, false),
        (0x0ca4, 0xd6d0, false),
        (0x10a4, 0xa3c1, false),
        (0x10a5, 0xa980, false),
        (0x04e7, 0xd6d0, false),
        (0x0ce7, 0xd6d0, false),
        (0x154a, 0xd6d0, false),
        (0, 0xa3c1, true),
    ] {
        let mut words = vec![[0x8001, 4350], [0x8002, style]];
        if axes {
            words.extend([[0x8070, 36], [0x8071, 36]]);
        }
        words.extend([[4682, code], [0x8004, 1]]);
        let (result, _, finished) = convert(&words, 0, &[], roles(), 18);
        assert!(result.is_ok(), "{style:x}: {result:?}");
        assert!(finished);
    }
    for tail in [
        vec![[4772, 0xa0c1]], // Mode-2 alias is not a mode-0 alphabet.
        vec![[4772, 0x9ff5]], // Decoded symbol requires an explicit symbol font.
        vec![[4772, 0xa1a1]], // Space also uses the separate symbol resource.
        vec![[4772, 0xa3c2]], // Caller font lacks B.
        vec![[0x8072, 1]],
        vec![[0x8006, 0xa381], [4682, 4350], [4912, 4350], [0xffff, 5]],
        vec![[0x8002, 0x1042], [4772, 0xa3c1]],
        vec![[0x8070, 36], [4772, 0xa3c1]],
    ] {
        let mut words = vec![[0x8001, 4350], [0x8002, 0x1084], [4682, 0xd6d0]];
        words.extend(tail);
        words.push([0x8004, 1]);
        let (result, _, finished) = convert(&words, 0, &[], roles(), 18);
        let error = result.unwrap_err();
        assert_eq!(page_image(&error).0, Some(1));
        assert!(error.offset >= Some(240));
        assert!(!finished);
    }
}

#[test]
fn mode_zero_digits_keep_unicode_and_use_ordinary_latin_with_independent_offsets() {
    for (style, axes) in [
        (0x1000, false),
        (0x1084, false),
        (0x10a4, false),
        (0x10a5, false),
        (0, true),
    ] {
        let mut words = vec![[0x8001, 4350], [0x8002, style], [0x801d, 4]];
        if axes {
            words.extend([[0x8070, 36], [0x8071, 36]]);
        }
        words.extend([[4772, 0xa3b0], [0x8004, 1]]);
        let (result, pdf, finished) = convert(&words, 0, &[], roles(), 19);
        assert!(result.is_ok(), "{style:x}: {result:?}");
        assert!(finished);
        let text = crate::test_support::pdf_text(&pdf);
        assert!(text.contains("/F1 1 Tf"));
        assert!(text.contains("<0030> Tj"));
        assert!(!text.contains("/F2 1 Tf"));
    }
    let words = [
        [0x8001, 4350],
        [0x8002, 0x154a],
        [4772, 0xa3b0],
        [0x8004, 1],
    ];
    assert!(convert(&words, 0, &[], roles(), 19).0.is_err());
}

#[test]
fn mode_zero_a385_lines_decode_markers_on_both_endpoints() {
    let mut expected = None;
    for marked in [false, true] {
        let mark = if marked { 0xc000 } else { 0 };
        let words = [
            [0x8006, 0xa385],
            [100 | mark, 200],
            [500 | mark, 300],
            [0xffff, 5],
            [0x8001, 4350],
            [0x8002, 0x1084],
            [4682, 0xd6d0],
            [0x8004, 1],
        ];
        let (result, pdf, finished) = convert(&words, 0, &[], roles(), 18);
        assert!(result.is_ok(), "{result:?}");
        assert!(finished);
        if let Some(plain) = &expected {
            assert_eq!(&pdf, plain);
        } else {
            expected = Some(pdf);
        }
    }
}

#[test]
fn mode_zero_parentheses_and_slash_use_their_controlled_font_roles() {
    for (code, character, role) in [
        (0xa3a8, "FF08", 1),
        (0xa3a9, "FF09", 1),
        (0xa3af, "FF0F", 0),
    ] {
        let words = [[0x8001, 4350], [0x8002, 0x1084], [4772, code], [0x8004, 1]];
        let (result, pdf, finished) = convert(&words, 0, &[], roles(), 19);
        assert!(result.is_ok(), "{code:x}: {result:?}");
        assert!(finished);
        let text = crate::test_support::pdf_text(&pdf);
        assert!(text.contains(&format!("/F{role} 1 Tf")));
        assert!(text.contains(&format!("<{character}> Tj")));
    }
}

#[test]
fn mode_zero_space_and_colon_use_symbol_resource_or_fallback() {
    let words = [
        [0x8001, 4350],
        [0x8002, 0x04e7],
        [4772, 0xa1a1],
        [4862, 0xa3ba],
        [0x8004, 1],
    ];
    let mut fonts = roles();
    fonts.symbols = Some(2);
    let (result, pdf, finished) = convert(&words, 0, &[], fonts, 20);
    assert!(result.is_ok(), "{result:?}");
    assert!(finished);
    let text = crate::test_support::pdf_text(&pdf);
    assert!(text.contains("/F2 1 Tf"));
    assert!(text.contains("<0020> Tj"));
    assert!(text.contains("<FF1A> Tj"));
    fonts.symbols = Some(3);
    let (result, _, finished) = convert(&words, 0, &[], fonts, 20);
    assert!(result.is_err());
    assert!(!finished);
    // Absent symbols: ASCII space uses Latin, the fullwidth colon uses CJK.
    fonts.symbols = None;
    let (result, pdf, finished) = convert(&words, 0, &[], fonts, 20);
    assert!(result.is_ok(), "{result:?}");
    assert!(finished);
    let text = crate::test_support::pdf_text(&pdf);
    let space = text.find("/F1 1 Tf").unwrap();
    let colon = text[space..].find("/F0 1 Tf").unwrap() + space;
    assert!(text[space..colon].contains("<0020> Tj"));
    assert!(text[colon..].contains("<FF1A> Tj"));
    assert!(!text.contains("/F2 1 Tf"));
}

#[test]
fn mode_zero_hyphen_requires_verified_geometry_and_symbol_resource() {
    for (style, accepted) in [
        (0x1000, true),
        (0x1084, true),
        (0x10a5, true),
        (0x04e7, false),
    ] {
        let words = [[0x8001, 4350], [0x8002, style], [4772, 0xaab2], [0x8004, 1]];
        let mut fonts = roles();
        fonts.symbols = Some(2);
        let (result, pdf, finished) = convert(&words, 0, &[], fonts, 19);
        assert_eq!(result.is_ok(), accepted, "{style:x}: {result:?}");
        assert_eq!(finished, accepted);
        if accepted {
            assert!(crate::test_support::pdf_text(&pdf).contains("/F2 1 Tf"));
        }
    }
}

#[test]
fn mode_zero_controlled_states_preserve_fonts_and_explicit_axes() {
    for (style, axes) in [(0x1084, false), (0x04e7, false), (0, true)] {
        let mut words = vec![[0x8001, 4350], [0x8002, style], [0x801d, 4]];
        if axes {
            words.extend([[0x8070, 36], [0x8071, 36]]);
        }
        words.extend([
            [4682, 0xd6d0],
            [4772, 0xa3c1],
            [4862, 0xa980],
            [0x8001, 4500],
        ]);
        let insertion = words.len();
        words.extend([[4682, 0xd6d0], [4772, 0xa3c1], [4862, 0xa980], [0x8004, 1]]);
        let (result, baseline, finished) = convert(&words, 0, &[], roles(), 18);
        assert!(result.is_ok() && finished);
        for control in [
            [0x8072, 0],
            [0x8072, 0xc2c7],
            [0x80ce, 1],
            [0x8073, 41],
            [0x8073, 42],
            [0x8073, 43],
            [0x8074, 0xc8ce],
            [0x8074, 0xb5c8],
            [0x8074, 0xb5c4],
            [0xc053, 0xe9],
            [0xc053, 0xb47],
        ] {
            let mut controlled = words.clone();
            controlled.insert(insertion, control);
            let (result, actual, finished) = convert(&controlled, 0, &[], roles(), 18);
            assert!(result.is_ok() && finished);
            assert_eq!(actual, baseline);
        }
    }
}

#[test]
fn hnb_first_image_selects_persistent_bilevel_composition() {
    let (result, _, finished) = convert(&image(), 1, &[false], roles(), 18);
    assert!(result.is_err());
    assert!(!finished);
    for top_first in [false, true] {
        let mut words = image();
        words.extend(image());
        words.extend(ordinary());
        words.push([0x8004, 1]);
        let (result, pdf, finished) = convert(&words, 2, &[top_first; 2], roles(), 21);
        assert!(result.is_ok(), "{result:?}");
        assert!(finished);
        let text = crate::test_support::pdf_text(&pdf);
        let first = text.find("/Im0 Do").unwrap();
        let second = text.find("/Im1 Do").unwrap();
        let glyph = text.find("<4E2D> Tj").unwrap();
        assert!(first < second && second < glyph);
    }
    for leading_image in [false, true] {
        let mut words = if leading_image { image() } else { vec![] };
        words.extend(ordinary());
        words.extend(image());
        words.push([0x8004, 1]);
        let count = if leading_image { 2 } else { 1 };
        let (result, pdf, finished) =
            convert(&words, count, &vec![false; count as usize], roles(), 21);
        result.unwrap();
        assert!(finished);
        let text = crate::test_support::pdf_text(&pdf);
        assert_eq!(text.contains("/BM /Multiply"), !leading_image);
        assert_eq!(
            text.matches("/BilevelOverlay gs").count(),
            usize::from(!leading_image)
        );
        assert!(text.find("<4E2D> Tj").unwrap() < text.rfind(" Do").unwrap());
    }
}

#[test]
fn hnb_title_style_114a_matches_controlled_cjk_154a_geometry() {
    let mut words = [
        [0x8001, 4350],
        [0x8002, 0x114a],
        [4682, 0xd6d0],
        [0x8004, 1],
    ];
    let (result, title, finished) = convert(&words, 0, &[], roles(), 12);
    assert!(result.is_ok() && finished);
    let (result, c8_title, finished) = convert(&words, 0, &[], roles(), 0);
    assert!(result.is_ok() && finished);
    assert_eq!(title, c8_title);
    words[1][1] = 0x154a;
    let (result, calibrated, finished) = convert(&words, 0, &[], roles(), 12);
    assert!(result.is_ok() && finished);
    assert_eq!(title, calibrated);
    words[1][1] = 0x114a;
    words[2][1] = 0xa0c1;
    assert!(convert(&words, 0, &[], roles(), 12).0.is_err());
}

#[test]
fn hnb_state_and_axis_43_registration_preserve_verified_style_reset() {
    let base = ordinary();
    let mut expected = base.clone();
    expected.push([0x8004, 1]);
    let baseline = convert(&expected, 0, &[], roles(), 12).1;
    for controls in [
        vec![[0x801c, 4]],
        vec![[0x8070, 43], [0x8071, 43], [0x8002, 0x1084]],
        vec![[0x801c, 4], [0x8070, 43], [0x8071, 43], [0x8002, 0x1084]],
    ] {
        let mut words = base.clone();
        words.splice(2..2, controls);
        words.push([0x8004, 1]);
        let (result, actual, finished) = convert(&words, 0, &[], roles(), 12);
        assert!(result.is_ok(), "{result:?}");
        assert!(finished);
        assert_eq!(actual, baseline);
    }
    let mut words = base;
    words.splice(2..2, [[0x8070, 43]]);
    words.push([0x8004, 1]);
    let (result, _, finished) = convert(&words, 0, &[], roles(), 12);
    assert!(result.is_err());
    assert!(!finished);
}

#[test]
fn hnb_regular_style_flags_preserve_controlled_resources_and_geometry() {
    for (width, height) in (2..=8).map(|field| (field, field)).chain([(5, 4)]) {
        for (code, mode) in [(0xd6d0, 12), (0xa0c1, 12), (0xaab3, 13)] {
            let size = (width << 5) | height;
            let mut words = [
                [0x8001, 4350],
                [0x8002, 0x0400 | size],
                [4682, code],
                [0x8004, 1],
            ];
            let (result, actual, finished) = convert(&words, 0, &[], roles(), mode);
            assert!(result.is_ok() && finished);
            words[1][1] = 0x1000 | size;
            let (result, baseline, finished) = convert(&words, 0, &[], roles(), mode);
            assert!(result.is_ok() && finished);
            assert_eq!(actual, baseline);
        }
    }
}

#[test]
fn native_state_three_uses_its_resource_or_latin_fallback_and_switches_back() {
    let words = [
        [0x8001, 4350],
        [0x8002, 0x10a5],
        [0x801d, 3],
        [4682, 0xa0c1],
        [0x801d, 4],
        [4772, 0xa0c1],
        [0x801d, 3],
        [4862, 0xa0c1],
        [0x801d, 0],
        [4952, 0xa0c1],
        [0x8004, 1],
    ];
    for mode in [0, 12] {
        let mut fonts = roles();
        // An index outside the page resources is reported, not replaced.
        fonts.latin_state3 = Some(3);
        let (result, _, finished) = convert(&words, 0, &[], fonts, mode);
        assert!(result.is_err());
        assert!(!finished);
        for (index, expected) in [(Some(0), [0, 2, 0, 1]), (None, [1, 2, 1, 1])] {
            fonts.latin_state3 = index;
            let (result, pdf, finished) = convert(&words, 0, &[], fonts, mode);
            assert!(result.is_ok(), "{result:?}");
            assert!(finished);
            let text = crate::test_support::pdf_text(&pdf);
            let mut at = 0;
            for role in expected {
                let token = format!("/F{role} 1 Tf");
                at += text[at..].find(&token).unwrap() + token.len();
            }
            assert_eq!(text.matches("<0041> Tj").count(), 4);
        }
    }
}

#[test]
fn hnb_tortoise_shell_brackets_preserve_controlled_offsets_and_resources() {
    for (style, dx, dy) in [
        (0x10a5, 25.0, 5.0),
        (0x08a5, 25.0, 5.0),
        (0x0ca5, 25.0, 5.0),
        (0x1084, 21.0, 6.0),
        (0x0884, 21.0, 6.0),
    ] {
        for (code, unicode) in [(0xa1b2, 0x3014), (0xa1b3, 0x3015)] {
            for (state, font) in [(0, 1), (3, 0), (4, 2)] {
                let words = [
                    [0x8001, 4394],
                    [0x8002, style],
                    [0x801d, state],
                    [4902, code],
                    [0x8004, 1],
                ];
                let mut fonts = roles();
                fonts.latin_state3 = Some(0);
                let (result, pdf, finished) = convert(&words, 0, &[], fonts, 13);
                result.unwrap();
                assert!(finished);
                let text = crate::test_support::pdf_text(&pdf);
                assert!(text.contains(&format!("<{unicode:04X}> Tj")));
                assert!(text.contains(&format!("/F{font} 1 Tf")));
                let (matrix, _) = text
                    .lines()
                    .find_map(|line| line.split_once(" Tm "))
                    .unwrap();
                let m: Vec<f64> = matrix
                    .split_whitespace()
                    .map(|v| v.parse().unwrap())
                    .collect();
                let unit = super::super::EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
                assert!((m[4] - (270.0 + dx) * unit).abs() < 0.000001);
                assert!((m[5] - ((495.0 - dy) * unit - m[3])).abs() < 0.000002);
                let mut other = words;
                other[1][1] = 0x1063;
                assert!(convert(&other, 0, &[], fonts, 13).0.is_err());
                assert_eq!(
                    convert(&words, 0, &[], fonts, 11).0.is_ok(),
                    style == 0x1084
                );
            }
        }
    }
}

#[test]
fn hnb_paired_axes_define_dimensions_and_allow_implicit_style() {
    for (width, height) in [(28, 28), (43, 43), (28, 43), (43, 28)] {
        let words = [
            [0x8001, 4350],
            [0x8070, width],
            [0x8071, height],
            [4682, 0xd6d0],
            [4772, 0xa0c1],
            [0x8004, 1],
        ];
        let (result, implicit, finished) = convert(&words, 0, &[], roles(), 12);
        result.unwrap();
        assert!(finished);
        let mut explicit = words.to_vec();
        explicit.insert(1, [0x8002, 0]);
        let (result, pdf, _) = convert(&explicit, 0, &[], roles(), 12);
        result.unwrap();
        assert_eq!(implicit, pdf);
        let text = crate::test_support::pdf_text(&pdf);
        let matrices: Vec<Vec<f64>> = text
            .lines()
            .filter_map(|line| line.split_once(" Tm "))
            .map(|(matrix, _)| {
                matrix
                    .split_whitespace()
                    .map(|v| v.parse().unwrap())
                    .collect()
            })
            .collect();
        let w = f64::from(width) * 75.0 / 301.0;
        let h = f64::from(height) * 75.0 / 301.0;
        let unit = super::super::EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
        assert!((matrices[0][0] - w).abs() < 0.000001);
        assert!((matrices[0][3] - h).abs() < 0.000001);
        assert!((matrices[1][4] - matrices[0][4] - 90.0 * unit - w / 8.0).abs() < 0.000002);
        let baseline = if height == 28 { 9.0 } else { 6.0 };
        assert!((matrices[0][5] - matrices[1][5] - baseline * unit).abs() < 0.000002);
        assert!(convert(&words, 0, &[], roles(), 0).0.is_err());
        for missing in [1, 2] {
            let mut partial = words.to_vec();
            partial.remove(missing);
            assert!(convert(&partial, 0, &[], roles(), 12).0.is_err());
        }
    }
}

#[test]
fn hnb_axis_punctuation_preserves_verified_offsets() {
    for (size, code, unicode, dx, dy) in [
        (43, 0xa3a8, 0xff08, 27.0, -4.0),
        (43, 0xa3a9, 0xff09, 25.0, -4.0),
        (43, 0xa1b0, 0x201c, 25.0, -4.0),
        (43, 0xa1b1, 0x201d, 25.0, -4.0),
        (43, 0xa1b6, 0x300a, 30.0, -4.0),
        (43, 0xa1b7, 0x300b, 20.0, -4.0),
        (43, 0xa1b2, 0x3014, 25.0, 4.0),
        (43, 0xa1b3, 0x3015, 25.0, 4.0),
        (28, 0xa1b2, 0x3014, 16.0, 8.0),
        (28, 0xa1b3, 0x3015, 16.0, 8.0),
    ] {
        let words = [
            [0x8001, 4394],
            [0x8070, size],
            [0x8071, size],
            [4902, code],
            [0x8004, 1],
        ];
        let (result, pdf, finished) = convert(&words, 0, &[], roles(), 13);
        result.unwrap();
        assert!(finished);
        let text = crate::test_support::pdf_text(&pdf);
        assert!(text.contains(&format!("<{unicode:04X}> Tj")));
        let (matrix, _) = text
            .lines()
            .find_map(|line| line.split_once(" Tm "))
            .unwrap();
        let m: Vec<f64> = matrix
            .split_whitespace()
            .map(|v| v.parse().unwrap())
            .collect();
        let unit = super::super::EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
        assert!((m[4] - (270.0 + dx) * unit).abs() < 0.000001);
        assert!((m[5] - ((495.0 - dy) * unit - m[3])).abs() < 0.000002);
        let mut mixed = words;
        mixed[2][1] = if size == 43 { 28 } else { 43 };
        assert!(convert(&mixed, 0, &[], roles(), 13).0.is_err());
    }
}

#[test]
fn native_fullwidth_at_sign_matches_controlled_comma_placement_and_resource() {
    for (mode, state, font) in [
        (13, 0, 1),
        (13, 3, 0),
        (13, 4, 2),
        (11, 0, 1),
        (11, 4, 2),
        (11, 28, 1),
        (11, 31, 2),
    ] {
        let mut glyphs = Vec::new();
        for code in [0xa3ac, 0xa3c0] {
            let words = [
                [0x8001, 4394],
                [0x8002, 0x1084],
                [0x801d, state],
                [4902, code],
                [0x8004, 1],
            ];
            let mut fonts = roles();
            fonts.latin_state3 = Some(0);
            fonts.latin_state28 = Some(1);
            fonts.latin_state31 = Some(2);
            let (result, pdf, finished) = convert(&words, 0, &[], fonts, mode);
            result.unwrap();
            assert!(finished);
            let text = crate::test_support::pdf_text(&pdf);
            assert!(text.contains(&format!("/F{font} 1 Tf")));
            glyphs.push(
                text.lines()
                    .find(|line| line.contains(" Tm "))
                    .unwrap()
                    .replace("<FF20>", "<FF0C>"),
            );
            if code == 0xa3c0 {
                assert!(text.contains("<FF20> Tj"));
            }
        }
        assert_eq!(glyphs[0], glyphs[1]);
    }
}

#[test]
fn hnb_fullwidth_hyphen_keeps_unicode_and_explicit_axis_placement() {
    for (state, font) in [(0, 1), (3, 0), (4, 2)] {
        let mut glyphs = Vec::new();
        for code in [0xa3ad, 0xa0ad] {
            let words = [
                [0x8001, 4394],
                [0x8002, 0],
                [0x8070, 36],
                [0x8071, 36],
                [0x801d, state],
                [4902, code],
                [0x8004, 1],
            ];
            let mut fonts = roles();
            fonts.latin_state3 = Some(0);
            let (result, pdf, finished) = convert(&words, 0, &[], fonts, 13);
            result.unwrap();
            assert!(finished);
            let text = crate::test_support::pdf_text(&pdf);
            assert!(text.contains(&format!("/F{font} 1 Tf")));
            assert!(text.contains("<FF0D> Tj"));
            glyphs.push(
                text.lines()
                    .find(|line| line.contains(" Tm "))
                    .unwrap()
                    .to_owned(),
            );
            if code == 0xa0ad {
                assert!(convert(&words, 0, &[], fonts, 11).0.is_err());
            }
        }
        assert_eq!(glyphs[0], glyphs[1]);
    }
}

#[test]
fn large_title_punctuation_is_rejected_before_regular_offset_lookup() {
    for style in [0x114a, 0x154a, 0xe58c] {
        for code in [0xa1a4, 0xa1af, 0xa1b0, 0xa3a8, 0xa3db] {
            let words = [[0x8001, 4350], [0x8002, style], [4682, code], [0x8004, 1]];
            let (result, _, finished) = convert(&words, 0, &[], roles(), 13);
            let error = result.unwrap_err();
            assert_eq!(page_image(&error).0, Some(1));
            assert!(!finished);
        }
    }
}

#[test]
fn c8_verified_color_control_preserves_black_across_style_and_resource_changes() {
    for value in 1..=3 {
        let mut words = ordinary();
        words.extend([[0x81ff, value], [0, 200]]);
        words.extend(ordinary());
        words.extend([
            [0x801d, 4],
            [4772, 0xa0c1],
            [0x8002, 0x10a5],
            [4682, 0xd6d0],
            [0x8004, 1],
        ]);
        let (result, pdf, finished) = convert(&words, 0, &[], roles(), 0);
        result.unwrap();
        assert!(finished);
        let text = crate::test_support::pdf_text(&pdf);
        assert_eq!(text.matches("0.266667 g\n").count(), 2);
        assert_eq!(text.matches("0.000000 g\n").count(), 4);
        assert!(text.contains("/F2 1 Tf"));
        assert!(convert(&words, 0, &[], roles(), 12).0.is_err());
        for payload in [[1, 200], [0, 199]] {
            words[5] = payload;
            let (result, _, finished) = convert(&words, 0, &[], roles(), 0);
            assert!(result.is_err());
            assert!(!finished);
        }
    }
}

#[test]
fn c8_cjk_mode_survives_resource_changes_and_one_restores_latin() {
    for state in [0, 4] {
        let mut words = vec![[0x80ce, 0], [0x801d, state]];
        words.extend(ordinary());
        words.extend([[0x80ce, 1], [4772, 0xa0c1], [0x8004, 1]]);
        let (result, pdf, finished) = convert(&words, 0, &[], roles(), 0);
        result.unwrap();
        assert!(finished);
        let text = crate::test_support::pdf_text(&pdf);
        assert_eq!(text.matches("/F0 1 Tf").count(), 2);
        assert_eq!(
            text.matches(&format!("/F{} 1 Tf", if state == 0 { 1 } else { 2 }))
                .count(),
            1
        );
        assert!(convert(&words, 0, &[], roles(), 12).0.is_err());
    }
}

#[test]
fn c8_fullwidth_alphabet_uses_cjk_resource_independent_of_latin_selection() {
    let words = [
        [0x8001, 4350],
        [0x8002, 0x1084],
        [4772, 0xd6d0],
        [0x8004, 1],
    ];
    let (result, baseline, _) = convert(&words, 0, &[], roles(), 0);
    result.unwrap();
    let baseline_text = crate::test_support::pdf_text(&baseline);
    let matrix = baseline_text
        .lines()
        .find(|line| line.contains(" Tm "))
        .unwrap();
    for (index, code) in (0xa3c1..=0xa3da).chain(0xa3e1..=0xa3fa).enumerate() {
        let (state, mode) = [(0, 1), (4, 1), (28, 1), (31, 1), (31, 0)][index % 5];
        let words = [
            [0x8001, 4350],
            [0x8002, 0x1084],
            [0x801d, state],
            [0x80ce, mode],
            [4772, code],
            [0x8004, 1],
        ];
        let mut fonts = roles();
        fonts.latin_state28 = Some(1);
        fonts.latin_state31 = Some(2);
        let (result, pdf, finished) = convert(&words, 0, &[], fonts, 11);
        result.unwrap();
        assert!(finished);
        let text = crate::test_support::pdf_text(&pdf);
        assert!(text.contains("/F0 1 Tf"));
        assert_eq!(
            text.lines()
                .find(|line| line.contains(" Tm "))
                .unwrap()
                .split(" Tm ")
                .next(),
            matrix.split(" Tm ").next()
        );
        assert_eq!(
            decode_native_character(code),
            char::from_u32(u32::from(code) - 0xa3a1 + 0xff01)
        );
    }
}

#[test]
fn c8_extended_latin_states_use_their_resources_or_fallback_and_restore_selection() {
    for state in [28, 31] {
        let words = [
            [0x8001, 4350],
            [0x8002, 0x1084],
            [0x801d, state],
            [4682, 0xa0c1],
            [0x801d, 4],
            [4772, 0xa0c1],
            [0x801d, state],
            [4862, 0xa0c1],
            [0x801d, 0],
            [4952, 0xa0c1],
            [0x8004, 1],
        ];
        for index in [None, Some(3), Some(0)] {
            let mut fonts = roles();
            if state == 28 {
                fonts.latin_state28 = index;
            } else {
                fonts.latin_state31 = index;
            }
            let (result, pdf, finished) = convert(&words, 0, &[], fonts, 0);
            if let Some(selected) = index.map_or(Some(1), |index| (index == 0).then_some(0)) {
                result.unwrap();
                assert!(finished);
                let text = crate::test_support::pdf_text(&pdf);
                let mut at = 0;
                for role in [selected, 2, selected, 1] {
                    let token = format!("/F{role} 1 Tf");
                    at += text[at..].find(&token).unwrap() + token.len();
                }
            } else {
                assert!(result.is_err());
                assert!(!finished);
            }
        }
    }
}

#[test]
fn c8_state_preserves_axes_and_style_reset() {
    for style in [0x1084, 0x10a5] {
        for axes in [None, Some(4), Some(36)] {
            let mut words = vec![[0x8001, 4350], [0x8002, style]];
            if let Some(size) = axes {
                words.extend([[0x8070, size], [0x8071, size]]);
            }
            words.extend([[4682, 0xd6d0], [4772, 0xa0c1], [0x8004, 1]]);
            let (result, expected, finished) = convert(&words, 0, &[], roles(), 0);
            assert!(result.is_ok() && finished);
            words.insert(2, [0x801c, 4]);
            let (result, actual, finished) = convert(&words, 0, &[], roles(), 0);
            assert!(result.is_ok() && finished);
            assert_eq!(actual, expected);
            if axes.is_some() {
                words.insert(words.len() - 3, [0x8002, style]);
                let (result, actual, finished) = convert(&words, 0, &[], roles(), 0);
                assert!(result.is_ok() && finished);
                words.drain(2..words.len() - 3);
                assert_eq!(actual, convert(&words, 0, &[], roles(), 0).1);
            }
        }
    }
    for controls in [
        vec![[0x8070, 4]],
        vec![[0x8071, 4]],
        vec![[0x801c, 5]],
        vec![[0x8070, 4], [0x8071, 36]],
    ] {
        let mut words = ordinary();
        words.splice(2..2, controls);
        words.push([0x8004, 1]);
        let (result, _, finished) = convert(&words, 0, &[], roles(), 0);
        assert!(result.is_err() && !finished);
    }
}

#[test]
fn c8_low_letter_preserves_cjk_resource_and_symbol_baseline_in_both_modes() {
    for (state, code) in [(0, 0x006c), (4, 0x006c), (0, 0x0070), (4, 0x0070)] {
        for mode in [0, 1] {
            for skew in [0x2800, 0x281c] {
                let mut words = [
                    [0x8001, 4334],
                    [0x8002, 0x10a5],
                    [0x801d, state],
                    [0x80ce, mode],
                    [0x8024, skew],
                    [4672, code],
                    [0x8004, 1],
                ];
                let (result, pdf, finished) = convert(&words, 0, &[], roles(), 11);
                assert!(result.is_ok() && finished);
                let text = crate::test_support::pdf_text(&pdf);
                assert!(text.contains("/F0 1 Tf"));
                assert!(text.contains(&format!("<{code:04X}> Tj")));
                let matrix = text
                    .lines()
                    .find_map(|line| line.split_once(" Tm "))
                    .unwrap()
                    .0;
                assert!(convert(&words, 0, &[], roles(), 13).0.is_err());
                words[0][1] += 15;
                words[5][1] = 0xd6d0;
                let (result, reference, _) = convert(&words, 0, &[], roles(), 0);
                result.unwrap();
                let reference = crate::test_support::pdf_text(&reference);
                let reference_matrix = reference
                    .lines()
                    .find_map(|line| line.split_once(" Tm "))
                    .unwrap()
                    .0;
                for (actual, expected) in matrix
                    .split_whitespace()
                    .zip(reference_matrix.split_whitespace())
                {
                    let actual: f64 = actual.parse().unwrap();
                    let expected: f64 = expected.parse().unwrap();
                    assert!((actual - expected).abs() < 1e-10);
                }
            }
        }
    }
    for code in [0x006b, 0x006d, 0x006f, 0x0071] {
        let words = [[0x8001, 4334], [0x8002, 0x10a5], [4672, code], [0x8004, 1]];
        assert!(convert(&words, 0, &[], roles(), 0).0.is_err());
    }
}

#[test]
fn small_glyph_punctuation_is_rejected_before_regular_offset_lookup() {
    for style in [0x1021, 0x1022, 0x1041] {
        for code in [0xa1a4, 0xa1af, 0xa1b0, 0xa1b1, 0xa3a8, 0xa3a9] {
            let words = [[0x8001, 4350], [0x8002, style], [4682, code], [0x8004, 1]];
            let (result, _, finished) = convert(&words, 0, &[], roles(), 13);
            let error = result.unwrap_err();
            assert_eq!(page_image(&error).0, Some(1));
            assert!(!finished);
        }
    }
}

#[test]
fn c8_required_symbols_follow_latin_state_and_symbol_baseline() {
    for (state, font) in [(0, 1), (3, 0), (4, 2)] {
        for (code, unicode) in [
            (0xa1c1, "00D7"),
            (0xa1de, "221E"),
            (0xa1e4, "2032"),
            (0xa6b8, "03A9"),
            (0xa6c4, "03B4"),
            (0xa6c5, "03B5"),
            (0xa6c8, "03B8"),
        ] {
            let mut fonts = roles();
            fonts.latin_state3 = Some(0);
            let mut matrices = Vec::new();
            for raw in [code, 0xa3ac] {
                let words = [
                    [0x8001, 4350],
                    [0x8002, 0x10a5],
                    [0x801d, state],
                    [4682, raw],
                    [0x8004, 1],
                ];
                let (result, pdf, finished) = convert(&words, 0, &[], fonts, 11);
                result.unwrap();
                assert!(finished);
                let text = crate::test_support::pdf_text(&pdf);
                assert!(text.contains(&format!("/F{font} 1 Tf")));
                if raw == code {
                    assert!(text.contains(&format!("<{unicode}> Tj")));
                    assert_eq!(
                        convert(&words, 0, &[], fonts, 13).0.is_ok(),
                        matches!(code, 0xa1c1 | 0xa1e4 | 0xa6b8 | 0xa6c4)
                    );
                }
                matrices.push(
                    text.lines()
                        .find_map(|line| line.split_once(" Tm "))
                        .unwrap()
                        .0
                        .to_owned(),
                );
            }
            assert_eq!(matrices[0], matrices[1]);
        }
    }
    for code in [0xa6c3, 0xa6c6, 0xa6c7, 0xa6c9] {
        let words = [[0x8001, 4350], [0x8002, 0x10a5], [4682, code], [0x8004, 1]];
        assert!(convert(&words, 0, &[], roles(), 11).0.is_err());
    }
}

#[test]
fn c8_radical_outputs_one_joined_path_and_rejects_unverified_geometry() {
    for flag in [0, 0xc000] {
        let words = [
            [0x8001, 4350],
            [0x8002, 0x1021],
            [0x8090, 0xa3e6],
            [4802 | flag, 4354],
            [143 | flag, 125],
            [4682, 0xa0c1],
            [0x8004, 1],
        ];
        let (result, pdf, finished) = convert(&words, 0, &[], roles(), 0);
        result.unwrap();
        assert!(finished);
        let text = crate::test_support::pdf_text(&pdf);
        assert_eq!(text.matches(" m\n").count(), 1);
        assert_eq!(text.matches(" l\n").count(), 4);
        assert_eq!(text.matches("S Q\n").count(), 1);
        assert!(text.find("S Q").unwrap() < text.find("<0041> Tj").unwrap());
        assert!(convert(&words, 0, &[], roles(), 12).0.is_err());
        for value in [0, 1, 0xa3b1, 0xa3b2, 0xa0c1, 0x8004, 0xffff] {
            let mut alias = words;
            alias[2][1] = value;
            let (result, alias_pdf, finished) = convert(&alias, 0, &[], roles(), 0);
            result.unwrap();
            assert!(finished);
            assert_eq!(pdf, alias_pdf);
            assert!(convert(&alias, 0, &[], roles(), 12).0.is_err());
        }
    }
    for (x, y, width, height, axis) in [
        (0x4000, 4354, 143, 125, false),
        (4802, 4354, 0xc08f, 125, false),
        (4802, 0x4000, 143, 125, false),
        (4802, 4354, 143, 0x4000, false),
        (4802, 4354, 29, 125, false),
        (4802, 4354, 143, 44, false),
        (4802, 4354, 143, 125, true),
    ] {
        let mut words = vec![[0x8001, 4350], [0x8002, 0x1021]];
        if axis {
            words.push([0x8070, 36]);
        }
        words.extend([[0x8090, 0xa3e6], [x, y], [width, height], [0x8004, 1]]);
        let (result, _, finished) = convert(&words, 0, &[], roles(), 0);
        assert!(result.is_err());
        assert!(!finished);
    }
}

#[test]
fn c8_image_references_reuse_descriptor_order_and_legacy_geometry() {
    let reference = |name| vec![[0x810a, 0xd300], [4682, 4314], [80, 50], [0, 1], [name, 0]];
    for orientation in [[false, true], [true, false]] {
        let mut old = ordinary();
        old.extend(image());
        old.extend(ordinary());
        old.extend(image());
        old.push([0x8004, 1]);
        let (result, expected, finished) = convert(&old, 2, &orientation, roles(), 0);
        result.unwrap();
        assert!(finished);
        for names in [*b"ab", *b"ba"] {
            let mut words = ordinary();
            words.extend(reference(u16::from(names[0])));
            words.extend(ordinary());
            words.extend(reference(u16::from(names[1])));
            words.push([0x8004, 1]);
            let (result, pdf, finished) = convert(&words, 2, &orientation, roles(), 0);
            result.unwrap();
            assert!(finished);
            assert_eq!(pdf, expected);
            assert!(convert(&words, 2, &orientation, roles(), 12).0.is_err());
        }
    }
    for case in 0..3 {
        let mut words = reference(u16::from(b'a'));
        match case {
            0 => words[2][0] = 0,
            1 => words[2][1] = 0,
            _ => words.insert(0, [0x8024, 0x281c]),
        }
        words.push([0x8004, 1]);
        let (result, _, finished) = convert(&words, 1, &[false], roles(), 0);
        assert!(result.is_err());
        assert!(!finished);
    }
}

#[test]
fn zero_field_styles_admit_only_the_controlled_hnb_square_profile() {
    for style in [0x1000, 0x1001, 0x1020] {
        for (code, unicode, mode) in [(0xa0c1, "0041", 0), (0xa3b1, "FF11", 11)] {
            let words = [[0x8001, 4350], [0x8002, style], [4682, code], [0x8004, 1]];
            let (result, pdf, finished) = convert(&words, 0, &[], roles(), mode);
            result.unwrap();
            assert!(finished);
            assert!(crate::test_support::pdf_text(&pdf).contains(&format!("<{unicode}> Tj")));
            assert!(
                convert(&words, 0, &[], roles(), if mode == 0 { 12 } else { 13 })
                    .0
                    .is_ok()
                    == (style == 0x1000)
            );
        }
    }
}

#[test]
fn opaque_controls_preserve_both_mode_two_profiles() {
    for control in [[0x8073, 8], [0x8074, 0xffff]] {
        let mut words = ordinary();
        words.extend([control, [0x8004, 1]]);
        assert!(convert(&words, 0, &[], roles(), 0).0.is_ok());
        let (result, _, finished) = convert(&words, 0, &[], roles(), 12);
        result.unwrap();
        assert!(finished);
    }
}

#[test]
fn c8_parallel_resets_latin_until_an_explicit_resource_selection() {
    for (state, selected) in [(3, 0), (4, 2)] {
        let mut fonts = roles();
        fonts.latin_state3 = Some(0);
        for (code, first, unicode) in [(0xa1ce, 1, "2225"), (0xa1e4, selected, "2032")] {
            let words = [
                [0x8001, 4350],
                [0x8002, 0x10a5],
                [0x801d, state],
                [4682, code],
                [4782, 0xa0c1],
                [0x801d, state],
                [4882, 0xa0c1],
                [0x8004, 1],
            ];
            let (result, pdf, finished) = convert(&words, 0, &[], fonts, 11);
            result.unwrap();
            assert!(finished);
            let text = crate::test_support::pdf_text(&pdf);
            assert!(text.contains(&format!("<{unicode}> Tj")));
            let resources: Vec<_> = text.lines().filter(|line| line.contains(" 1 Tf")).collect();
            assert_eq!(
                resources,
                [
                    format!("BT /F{first} 1 Tf"),
                    format!("BT /F{first} 1 Tf"),
                    format!("BT /F{selected} 1 Tf")
                ]
            );
            assert_eq!(convert(&words, 0, &[], fonts, 13).0.is_ok(), code == 0xa1e4);
        }
    }
}

/// F0 maps Han and a fullwidth colon, F1 ASCII `A` and the default decoration
/// alias, F2 only ASCII `A`/`B`: each fallback is visible as a distinct resource.
fn fallback_fonts() -> Vec<Vec<u8>> {
    vec![
        labelled_font([0x4e2d, 0xff1a]),
        labelled_font([0x41, 0x25ba]),
        labelled_font([0x41, 0x42]),
    ]
}

fn required_roles() -> C8PageFonts {
    C8PageFonts {
        cjk: 0,
        latin: 1,
        alternate_latin: None,
        decoration: None,
        symbols: None,
        latin_state3: None,
        latin_state28: None,
        latin_state31: None,
    }
}

fn assert_in_order(pdf: &[u8], tokens: &[&str]) {
    let text = crate::test_support::pdf_text(pdf);
    let mut after = 0;
    for token in tokens {
        after += text[after..].find(token).unwrap() + token.len();
    }
}

#[test]
fn absent_roles_and_unmapped_characters_fall_back_by_character_class() {
    let words = [
        [0x8001, 4350],
        [0x8002, 0x1084],
        [4682, 0xd6d0],
        [0x801d, 4],
        [4772, 0xa0c1],
        [0x801d, 0],
        [4862, 0xa3ba],
        [0x8010, 1],
        [4682, 4524],
        [4832, 4524],
        [0x8004, 1],
    ];
    // Absent alternate and decoration roles use Latin; the Latin role's
    // unmapped fullwidth colon uses CJK.
    let (result, pdf, finished) =
        convert_with_fonts(&words, 0, &[], required_roles(), 0, fallback_fonts());
    assert_eq!(result.unwrap(), 0);
    assert!(finished);
    let text = crate::test_support::pdf_text(&pdf);
    assert!(!text.contains("/F2 1 Tf"));
    assert_in_order(
        &pdf,
        &[
            "/F0 1 Tf",
            "<4E2D> Tj",
            "/F1 1 Tf",
            "<0041> Tj",
            "/F0 1 Tf",
            "<FF1A> Tj",
            "/Artifact BMC",
            "/F1 1 Tf",
            "<25BA> Tj",
        ],
    );
    // Explicit roles take precedence over the fallback whenever they map
    // the character. The explicit decoration font lacks its alias, so the
    // alias also falls back to Latin.
    let roles = C8PageFonts {
        alternate_latin: Some(2),
        decoration: Some((2, C8_DEFAULT_DECORATION_ALIAS)),
        ..required_roles()
    };
    let (result, pdf, finished) = convert_with_fonts(&words, 0, &[], roles, 0, fallback_fonts());
    assert_eq!(result.unwrap(), 0);
    assert!(finished);
    assert_in_order(
        &pdf,
        &[
            "<4E2D> Tj",
            "/F2 1 Tf",
            "<0041> Tj",
            "/F0 1 Tf",
            "<FF1A> Tj",
            "/F1 1 Tf",
            "<25BA> Tj",
        ],
    );
}

#[test]
fn glyphs_missing_from_the_fallback_font_still_fail_with_a_location() {
    // `B` is not CJK-coded and Latin lacks it; U+3000 is CJK-coded and CJK
    // lacks it. An explicit role that maps `B` still succeeds.
    for (code, alternate, accepted) in [
        (0xa0c2, None, false),
        (0xa1a1, None, false),
        (0xa0c2, Some(2), true),
    ] {
        let words = [
            [0x8001, 4350],
            [0x8002, 0x1084],
            [0x801d, 4],
            [4800, code],
            [0x8004, 1],
        ];
        let roles = C8PageFonts {
            alternate_latin: alternate,
            ..required_roles()
        };
        let (result, _, finished) = convert_with_fonts(&words, 0, &[], roles, 0, fallback_fonts());
        assert_eq!(finished, accepted);
        if !accepted {
            let error = result.unwrap_err();
            assert_eq!(page_image(&error).0, Some(1));
            assert!(error.offset >= Some(112));
            assert!(
                error.to_string().contains("no supported BMP glyph"),
                "{error}"
            );
        }
    }
}

#[test]
fn mode_zero_absent_alternate_and_symbol_roles_fall_back_by_character_class() {
    let words = [
        [0x8001, 4350],
        [0x8002, 0x1084],
        [4682, 0xd6d0],
        [4772, 0xa980],
        [4862, 0xa3ba],
        [0x8004, 1],
    ];
    let (result, pdf, finished) =
        convert_with_fonts(&words, 0, &[], required_roles(), 18, fallback_fonts());
    assert_eq!(result.unwrap(), 0);
    assert!(finished);
    assert_in_order(
        &pdf,
        &[
            "/F0 1 Tf",
            "<4E2D> Tj",
            "/F1 1 Tf",
            "<0041> Tj",
            "/F0 1 Tf",
            "<FF1A> Tj",
        ],
    );
    assert!(is_cjk_coded('\u{3000}') && is_cjk_coded('\u{ff1a}') && is_cjk_coded('\u{fe10}'));
    assert!(!is_cjk_coded('A') && !is_cjk_coded('\u{2217}') && !is_cjk_coded('\u{25ba}'));
}

#[test]
fn c8_new_explicit_axes_preserve_implicit_runs_and_style_resets() {
    for size in [22, 34, 38, 40] {
        let words = [
            [0x8001, 4394],
            [0x8070, size],
            [0x8071, size],
            [4902, 0xd6d0],
            [5072, 0xa0c1],
            [0x8002, 0x1084],
            [5072, 0xa0c1],
            [0x8004, 1],
        ];
        let (result, implicit, finished) = convert(&words, 0, &[], roles(), 0);
        result.unwrap();
        assert!(finished);
        let mut explicit = words.to_vec();
        explicit.insert(1, [0x8002, 0]);
        let (result, pdf, _) = convert(&explicit, 0, &[], roles(), 0);
        result.unwrap();
        assert_eq!(implicit, pdf);
        let text = crate::test_support::pdf_text(&pdf);
        assert_eq!(text.matches(" Tj ET").count(), 3);
        let last = text.lines().rfind(|line| line.contains(" Tm ")).unwrap();
        let baseline = convert(
            &[
                [0x8001, 4394],
                [0x8002, 0x1084],
                [5072, 0xa0c1],
                [0x8004, 1],
            ],
            0,
            &[],
            roles(),
            0,
        )
        .1;
        assert!(
            crate::test_support::pdf_text(&baseline)
                .lines()
                .any(|line| line == last)
        );
        for missing in [1, 2] {
            let mut partial = words.to_vec();
            partial.remove(missing);
            assert!(convert(&partial, 0, &[], roles(), 0).0.is_err());
        }
    }
}

#[test]
fn c8_explicit_parentheses_and_brackets_keep_the_following_glyph() {
    for size in [22, 34, 40] {
        for code in [0xa3a8, 0xa3a9, 0xa3db, 0xa3dd] {
            let words = [
                [0x8001, 4394],
                [0x8070, size],
                [0x8071, size],
                [4902, code],
                [5072, 0xa0c1],
                [0x8004, 1],
            ];
            let (result, pdf, finished) = convert(&words, 0, &[], roles(), 11);
            result.unwrap();
            assert!(finished);
            let text = crate::test_support::pdf_text(&pdf);
            assert_eq!(text.matches(" Tj ET").count(), 2);
            assert!(text.contains("<0041> Tj"));
            let mut unequal = words;
            unequal[2][1] = if size == 22 { 34 } else { 22 };
            assert!(convert(&unequal, 0, &[], roles(), 11).0.is_err());
        }
    }
}

#[test]
fn c8_decoration_values_keep_glyph_replay_and_explicit_sizes() {
    for axis in [None, Some(34), Some(40)] {
        let mut words = ordinary();
        if let Some(size) = axis {
            words.extend([[0x8070, size], [0x8071, size]]);
        }
        let at = words.len();
        words.extend([
            [0x8010, 1],
            [4682, 4524],
            [4932, 4524],
            [5072, 0xa0c1],
            [0x8004, 1],
        ]);
        let (result, baseline, finished) = convert(&words, 0, &[], roles(), 0);
        result.unwrap();
        assert!(finished);
        assert!(crate::test_support::pdf_text(&baseline).contains("/Artifact BMC"));
        for value in [2, 46] {
            words[at][1] = value;
            let (result, pdf, finished) = convert(&words, 0, &[], roles(), 0);
            result.unwrap();
            assert!(finished);
            assert_eq!(pdf, baseline);
        }
        // A diagonal cannot be silently treated as a horizontal decoration.
        words[at + 2][1] += 1;
        assert!(convert(&words, 0, &[], roles(), 0).0.is_err());
    }
}

#[test]
fn c8_added_symbol_roles_match_independent_reference_glyphs() {
    for (code, reference) in [
        (0xa1a3, 0xa1a2),
        (0xa1ab, 0xa3ac),
        (0xa3a7, 0xa3ac),
        (0xa3fc, 0xa3ac),
        (0xa3a6, 0xd6d0),
    ] {
        let mut matrices = Vec::new();
        for code in [code, reference] {
            let (result, pdf, finished) = convert(
                &[[0x8001, 4394], [0x8002, 0x1084], [4902, code], [0x8004, 1]],
                0,
                &[],
                roles(),
                11,
            );
            result.unwrap();
            assert!(finished);
            let text = crate::test_support::pdf_text(&pdf);
            let matrix = text
                .lines()
                .find_map(|line| line.split_once(" Tm "))
                .unwrap()
                .0;
            matrices.push(matrix.to_owned());
        }
        assert_eq!(matrices[0], matrices[1]);
    }
}

mod hnb_extended;

#[test]
fn c8_decoration_117_keeps_clipping_and_following_text_in_measured_states() {
    for style in [0x1084, 0x10a5] {
        for length in [200, 600] {
            let mut words = ordinary();
            words.extend([
                [0x8002, style],
                [0x8010, 1],
                [4672, 4500],
                [4672 + length, 4500],
                [0xffff, 5],
                [4772, 0xa0c1],
                [0x8004, 1],
            ]);
            let (result, reference, finished) = convert(&words, 0, &[], roles(), 0);
            result.unwrap();
            assert!(finished);
            words[5][1] = 117;
            let (result, pdf, finished) = convert(&words, 0, &[], roles(), 0);
            result.unwrap();
            assert!(finished);
            // Identical caller-supplied aliases give identical PDF painting,
            // including partial final tiles, clipping, and text after the line.
            assert_eq!(pdf, reference);
            for value in [116, 118] {
                words[5][1] = value;
                assert!(convert(&words, 0, &[], roles(), 0).0.is_err());
            }
            words[5][1] = 117;
            for other in [0x1063, 0x10c6] {
                words[4][1] = other;
                assert!(convert(&words, 0, &[], roles(), 0).0.is_err());
            }
            words[4][1] = style;
            assert!(convert(&words, 0, &[], roles(), 12).0.is_err());
            let mut explicit = words.clone();
            explicit.splice(5..5, [[0x8070, 34], [0x8071, 34]]);
            assert!(convert(&explicit, 0, &[], roles(), 0).0.is_err());
            words[7][1] += 1;
            assert!(convert(&words, 0, &[], roles(), 0).0.is_err());
        }
    }
}

#[test]
fn c8_b94c_title_uses_unequal_cjk_axes_without_expanding_other_classes() {
    let mut words = vec![[0x8001, 4350], [0x8002, 0xb94c], [4682, 0xd6d0]];
    words.extend(ordinary());
    words.push([0x8004, 1]);
    let (result, pdf, finished) = convert(&words, 0, &[], roles(), 0);
    result.unwrap();
    assert!(finished);
    let text = crate::test_support::pdf_text(&pdf);
    let (matrix, glyph) = text
        .lines()
        .find_map(|line| line.split_once(" Tm "))
        .unwrap();
    assert!(glyph.contains("<4E2D> Tj"));
    let m: Vec<f64> = matrix
        .split_whitespace()
        .map(|v| v.parse().unwrap())
        .collect();
    let unit = super::super::EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
    let expected = [
        84.0 * 75.0 / 301.0,
        0.0,
        0.0,
        109.0 * 75.0 / 301.0,
        50.0 * unit,
        539.0 * unit - 109.0 * 75.0 / 301.0,
    ];
    for (actual, expected) in m.iter().zip(expected) {
        assert!((actual - expected).abs() < 0.000001);
    }
    assert_eq!(text.matches("<4E2D> Tj").count(), 2);
    assert!(text.contains("<0041> Tj"));
    assert!(convert(&words, 0, &[], roles(), 12).0.is_err());
    for code in [0xa0c1, 0xa1a4, 0xa3a8, 0xa3db] {
        words[2][1] = code;
        assert!(convert(&words, 0, &[], roles(), 0).0.is_err());
    }
    words[2][1] = 0xd6d0;
    for style in [0xb94b, 0xb94d, 0xb92c, 0xb96c] {
        words[1][1] = style;
        assert!(convert(&words, 0, &[], roles(), 0).0.is_err());
    }
}

#[test]
fn c8_style_five_arrow_resets_latin_for_itself_and_following_text() {
    for state in [0, 4] {
        let words = [
            [0x8001, 4350],
            [0x8002, 0x10a5],
            [0x801d, state],
            [4682, 0xa1fa],
            [4772, 0xa0c1],
            [0x801d, state],
            [4772, 0xa0c1],
            [0x8004, 1],
        ];
        let (result, pdf, finished) = convert(&words, 0, &[], roles(), 11);
        result.unwrap();
        assert!(finished);
        let text = crate::test_support::pdf_text(&pdf);
        assert!(text.contains("<2192> Tj"));
        let resources: Vec<_> = text.lines().filter(|line| line.contains(" 1 Tf")).collect();
        assert_eq!(
            resources,
            [
                "BT /F1 1 Tf",
                "BT /F1 1 Tf",
                if state == 4 {
                    "BT /F2 1 Tf"
                } else {
                    "BT /F1 1 Tf"
                }
            ]
        );
        let mut reference = words;
        reference[2][1] = 0;
        reference[3][1] = 0xa3ac;
        let (result, reference_pdf, _) = convert(&reference, 0, &[], roles(), 11);
        result.unwrap();
        let reference_text = crate::test_support::pdf_text(&reference_pdf);
        let matrices = |s: &str| {
            s.lines()
                .filter_map(|line| line.split_once(" Tm "))
                .map(|(m, _)| m.to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(matrices(&text), matrices(&reference_text));
        for style in [0x1084, 0x10c6] {
            let mut other = words;
            other[1][1] = style;
            assert!(convert(&other, 0, &[], roles(), 11).0.is_err());
        }
        let mut explicit = words.to_vec();
        explicit.splice(2..2, [[0x8070, 34], [0x8071, 34]]);
        assert!(convert(&explicit, 0, &[], roles(), 11).0.is_err());
        assert!(convert(&words, 0, &[], roles(), 13).0.is_err());
    }
}

#[test]
fn c8_8007_segments_match_controlled_strokes_without_consuming_following_text() {
    for points in [
        [[4772, 4374], [4772, 4574]],
        [[4802, 4404], [4802, 4604]],
        [[4772, 4374], [4972, 4574]],
        [[4772, 4574], [4772, 4374]],
    ] {
        let mut words = ordinary();
        words.extend([
            [0x8006, 0xa381],
            points[0],
            points[1],
            [0xffff, 5],
            [4772, 0xa0c1],
            [0x8004, 1],
        ]);
        let (result, reference, finished) = convert(&words, 0, &[], roles(), 0);
        result.unwrap();
        assert!(finished);
        for value in [0xa380, 0xa382] {
            words[4] = [0x8007, value];
            let (result, pdf, finished) = convert(&words, 0, &[], roles(), 0);
            result.unwrap();
            assert!(finished);
            assert_eq!(pdf, reference);
            assert!(convert(&words, 0, &[], roles(), 12).0.is_err());
        }
        for value in [0xa37f, 0xa381, 0xa383] {
            words[4] = [0x8007, value];
            assert!(convert(&words, 0, &[], roles(), 0).0.is_err());
        }
    }
}

mod nju_profiles;
