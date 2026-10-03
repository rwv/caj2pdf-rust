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
    if matches!(mode, 12 | 13) {
        // Original compact HN-B wrapper around the same authored record stream.
        let c8 = &input.bytes;
        let mut bytes = vec![0; 228];
        bytes[..4].copy_from_slice(b"HN\0\0");
        bytes[4..8].copy_from_slice(&200_u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&136_u32.to_le_bytes());
        bytes[144..148].copy_from_slice(&1_u32.to_le_bytes());
        bytes[164..172].copy_from_slice(&c8[28..36]);
        bytes[216..220].copy_from_slice(&228_u32.to_le_bytes());
        bytes[220..224].copy_from_slice(&(c8.len() as u32 - 100).to_le_bytes());
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
            _ => (),
        }
        let mut document = PdfDocument::new(&mut sink, &limits, &cancel).await.unwrap();
        let mut font_bytes = crate::pdf::drawing_font();
        if matches!(mode, 11 | 13) {
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
                .find(|pair| pair[0] < 0x8000 && pair[1] >= 0xa000)
                .unwrap()[1];
            let character = decode_native_character(code).unwrap() as u32;
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
    for style in [0x1067, 0x10e3, 0xe58c] {
        let words = [
            [0x8001, 4394],
            [0x8002, style],
            [4682, 0xd6d0],
            [0x8024, 0x281d],
            [4682, 0xd6d0],
            [0x8002, style],
            [4682, 0xd6d0],
            [0x8024, 0x2800],
            [4682, 0xd6d0],
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
        assert_eq!(matrices.len(), 4);
        assert_eq!(matrices[0], matrices[3]);
        assert_eq!(matrices[1], matrices[2]);
        assert_eq!(matrices[0][2], 0.0);
        assert!((matrices[1][2] - matrices[0][0] * 0.24).abs() < 0.000001);
        for index in [0, 1, 3, 4, 5] {
            assert_eq!(matrices[0][index], matrices[1][index]);
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
