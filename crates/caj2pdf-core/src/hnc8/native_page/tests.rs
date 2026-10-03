// SPDX-License-Identifier: MIT

use super::*;
use crate::Limits;
use crate::pdf::{BilevelImageSpec, TrueTypeFont};
use crate::test_support::ready;
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
    async fn read_at(&mut self, offset: u64, out: &mut [u8]) -> crate::Result<usize> {
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
impl SequentialSink for Sink {
    async fn write(&mut self, bytes: &[u8]) -> crate::Result<usize> {
        if self.fail.get() {
            return Err(invalid("original write failure"));
        }
        let count = bytes.len().min(7);
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }
    async fn flush(&mut self) -> crate::Result<()> {
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
        alternate_latin: 2,
        decoration: Some((1, 'A')),
        symbols: None,
        latin_state3: None,
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
    let result = ready(async {
        let mut reader = Hnc8Reader::open(&mut input, &limits, &cancel, Default::default())
            .await
            .unwrap();
        reader.next_page().await.unwrap();
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
        let mut document = PdfDocument::new(&mut sink, &limits, &cancel).await.unwrap();
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
                .find(|pair| pair[0] < 0x8000 && (pair[1] >= 0xa000 || pair[1] == 0x9ff5))
                .unwrap()[1];
            let character = if mode == 19 {
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
        let mut font_source = source(font_bytes);
        let mut font = TrueTypeFont::read(&mut font_source, &limits, &cancel)
            .await
            .unwrap();
        let font = document.add_font(&mut font).await.unwrap();
        let fonts = [&font, &font, &font];
        let mut images = Vec::new();
        for _ in 0..declared {
            let mut image = document
                .begin_bilevel_image(BilevelImageSpec {
                    pixel_width: 2,
                    pixel_height: 2,
                    row_stride: 1,
                })
                .await
                .unwrap();
            image.write(&[0x80, 0x40]).await.unwrap();
            images.push(image.finish().await.unwrap());
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
            slice,
            top_first,
            TextBudget::default(),
        )
        .await;
        cancel.0.set(false);
        input_fault.set(false);
        output_fault.set(false);
        let finished = document.finish().await.is_ok();
        (outcome, finished)
    });
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
    let content = String::from_utf8_lossy(&pdf);
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
        vec![[0x801d, 3]],
        vec![[0x8072, 1]],
        vec![[0x8073, 43]],
        vec![[0x8074, 0xffff]],
        vec![[0x8006, 0xa384], [4682, 4350], [4912, 4350]],
        vec![[0x8004, 0]],
    ] {
        let mut words = ordinary();
        words.extend(tail);
        words.push([0x8004, 1]);
        let (result, _, finished) = convert(&words, 0, &[], roles(), 0);
        let error = result.unwrap_err();
        assert_eq!(error.page, Some(1));
        assert!(error.offset >= 116);
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
        assert!(!String::from_utf8_lossy(&pdf).contains("/Type /Page /"));
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
        let text = String::from_utf8_lossy(&pdf);
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
            let text = String::from_utf8_lossy(&pdf);
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
            let text = String::from_utf8_lossy(&pdf);
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
            let text = String::from_utf8_lossy(&pdf);
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
            let text = String::from_utf8_lossy(&pdf);
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
            let text = String::from_utf8_lossy(&pdf);
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
    let make_words = |control: Option<[u16; 2]>| {
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
                words.push(control);
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
        (0x8072, &[0, 0x1042, 0xa3a8, 0xa0f2][..]),
        (0x8073, &[38, 39, 40, 41, 42][..]),
        (0x8074, &[0, 0xb4a2, 0xd4b4, 0x24a7, 0xa1a1, 0xa3a9][..]),
        (0xc053, &[0, 0x1377, 0x137b, 5200, 5700, 0xffff][..]),
        (
            0xc054,
            &[0, 0x139e, 0x15a8, 0x1607, 0x1676, 5200, 5700, 0xffff][..],
        ),
    ] {
        for &value in values {
            let (result, pdf, finished) =
                convert(&make_words(Some([tag, value])), 1, &[false], roles(), 0);
            assert_eq!(result.unwrap(), 0);
            assert!(finished);
            assert_eq!(pdf, baseline, "control {tag:04x}/{value:04x}");
        }
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
    let text = String::from_utf8_lossy(&pdf);
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
            let text = String::from_utf8_lossy(&pdf);
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
fn hnb_book_title_marks_preserve_verified_style_five_offsets_and_resources() {
    for (code, unicode, x) in [(0xa1b6, 0x300a, 30.0), (0xa1b7, 0x300b, 20.0)] {
        let words = [
            [0x8001, 4394],
            [0x8002, 0x10a5],
            [4902, code],
            [0x801d, 4],
            [4902, code],
            [0x8004, 1],
        ];
        let (result, pdf, finished) = convert(&words, 0, &[], roles(), 13);
        result.unwrap();
        assert!(finished);
        let text = String::from_utf8_lossy(&pdf);
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
        assert!(convert(&words, 0, &[], roles(), 11).0.is_err());
        let mut other = words;
        other[1][1] = 0x1084;
        assert!(convert(&other, 0, &[], roles(), 13).0.is_err());
    }
}

#[test]
fn unverified_native_modes_cannot_use_mode_two_rendering() {
    for mode in 14..=17 {
        let (result, pdf, _) = convert(&ordinary(), 0, &[], roles(), mode);
        let error = result.unwrap_err();
        assert!(
            matches!(
                error.kind,
                ErrorKind::Unsupported {
                    field: "native page rendering mode",
                    ..
                }
            ),
            "{error:?}"
        );
        assert!(!String::from_utf8_lossy(&pdf).contains("/Type /Page "));
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
    let pdf = String::from_utf8_lossy(&pdf);
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
        assert_eq!(error.page, Some(1));
        assert!(error.offset >= 240);
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
        let text = String::from_utf8_lossy(&pdf);
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
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains(&format!("/F{role} 1 Tf")));
        assert!(text.contains(&format!("<{character}> Tj")));
    }
}

#[test]
fn mode_zero_space_and_colon_use_explicit_symbol_resource() {
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
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.contains("/F2 1 Tf"));
    assert!(text.contains("<0020> Tj"));
    assert!(text.contains("<FF1A> Tj"));
    for symbols in [None, Some(3)] {
        fonts.symbols = symbols;
        let (result, _, finished) = convert(&words, 0, &[], fonts, 20);
        assert!(result.is_err());
        assert!(!finished);
    }
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
            assert!(String::from_utf8_lossy(&pdf).contains("/F2 1 Tf"));
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
fn hnb_leading_images_preserve_order_and_reject_later_raster_operations() {
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
        let text = String::from_utf8_lossy(&pdf);
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
        let (result, _, finished) =
            convert(&words, count, &vec![false; count as usize], roles(), 21);
        assert!(result.is_err());
        assert!(!finished);
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
    assert!(convert(&words, 0, &[], roles(), 0).0.is_err());
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
fn hnb_state_three_requires_explicit_resource_and_switches_back() {
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
    let mut fonts = roles();
    for index in [None, Some(3)] {
        fonts.latin_state3 = index;
        let (result, _, finished) = convert(&words, 0, &[], fonts, 12);
        assert!(result.is_err());
        assert!(!finished);
    }
    fonts.latin_state3 = Some(0);
    let (result, pdf, finished) = convert(&words, 0, &[], fonts, 12);
    assert!(result.is_ok(), "{result:?}");
    assert!(finished);
    let text = String::from_utf8_lossy(&pdf);
    let mut at = 0;
    for role in [0, 2, 0, 1] {
        let token = format!("/F{role} 1 Tf");
        at += text[at..].find(&token).unwrap() + token.len();
    }
    assert_eq!(text.matches("<0041> Tj").count(), 4);
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
                let text = String::from_utf8_lossy(&pdf);
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
                assert!(convert(&words, 0, &[], fonts, 11).0.is_err());
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
        let text = String::from_utf8_lossy(&pdf);
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
        let text = String::from_utf8_lossy(&pdf);
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
fn hnb_fullwidth_at_sign_matches_controlled_comma_placement_and_resource() {
    for (state, font) in [(0, 1), (3, 0), (4, 2)] {
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
            let (result, pdf, finished) = convert(&words, 0, &[], fonts, 13);
            result.unwrap();
            assert!(finished);
            let text = String::from_utf8_lossy(&pdf);
            assert!(text.contains(&format!("/F{font} 1 Tf")));
            glyphs.push(
                text.lines()
                    .find(|line| line.contains(" Tm "))
                    .unwrap()
                    .replace("<FF20>", "<FF0C>"),
            );
            if code == 0xa3c0 {
                assert!(text.contains("<FF20> Tj"));
                assert!(convert(&words, 0, &[], fonts, 11).0.is_err());
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
            let text = String::from_utf8_lossy(&pdf);
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
