// SPDX-License-Identifier: MIT

use super::*;
use crate::ErrorKind;
use crate::test_support::variant_of;
use crate::test_support::{field_of, page_image};
use crate::{Error, Limits};
use std::{cell::Cell, rc::Rc};

#[derive(Clone, Default)]
struct Cancel(Rc<Cell<bool>>);
impl Cancellation for Cancel {
    fn is_cancelled(&self) -> bool {
        self.0.get()
    }
}

struct Source {
    bytes: Vec<u8>,
    short: usize,
    max_request: usize,
    reads: Rc<Cell<usize>>,
    fault: bool,
    cancel_on_read: Option<Cancel>,
}
impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> crate::Result<usize> {
        self.reads.set(self.reads.get() + 1);
        self.max_request = self.max_request.max(out.len());
        if self.fault {
            return Err(Error::invalid("synthetic source failure"));
        }
        let at = offset as usize;
        let n = self
            .short
            .min(out.len())
            .min(self.bytes.len().saturating_sub(at));
        out[..n].copy_from_slice(&self.bytes[at..at + n]);
        if let Some(cancel) = &self.cancel_on_read {
            cancel.0.set(true);
        }
        Ok(n)
    }
}
fn fixture(words: &[[u16; 2]], images: u16) -> Source {
    let mut bytes = vec![0; 100];
    bytes[0] = 0xc8;
    bytes[8..12].copy_from_slice(&1_u32.to_le_bytes());
    bytes[80..84].copy_from_slice(&100_u32.to_le_bytes());
    bytes[84..88].copy_from_slice(&(words.len() as u32 * 4).to_le_bytes());
    bytes[88..90].copy_from_slice(&images.to_le_bytes());
    for pair in words {
        for word in pair {
            bytes.extend(word.to_le_bytes());
        }
    }
    Source {
        bytes,
        short: 28,
        max_request: 0,
        reads: Rc::new(Cell::new(0)),
        fault: false,
        cancel_on_read: None,
    }
}
#[derive(Default)]
struct Visitor {
    events: Vec<(u64, NativeRecord)>,
    fail: bool,
    cancel: Option<Cancel>,
}
impl NativeRecordVisitor for Visitor {
    fn visit(&mut self, offset: u64, record: NativeRecord) -> crate::Result<()> {
        self.events.push((offset, record));
        if let Some(cancel) = &self.cancel {
            cancel.0.set(true);
        }
        if self.fail {
            return Err(Error::invalid("synthetic visitor failure"));
        }
        Ok(())
    }
}
fn parse(source: &mut Source, visitor: &mut Visitor) -> Result<u32> {
    (|| {
        let limits = Limits::default();
        let cancel = Cancel::default();
        let mut reader = Hnc8Reader::open(source, &limits, &cancel)?;
        reader.next_page()?;
        reader.visit_native_records(visitor)
    })()
}

#[test]
fn streams_raw_glyph_context_and_atomic_drawing_image_records() {
    // Invented coordinates/payloads. Marker-looking point/image words must not
    // turn into controls, glyphs or premature ends.
    let words = [
        [0x8001, 13],
        [0x8002, 41],
        [0x801d, 4],
        [0x8067, 9],
        [90, 0xcec4],
        [20, 0xa0c4],
        [0x8001, 71],
        [0x8002, 89],
        [15, 0xffff],
        [0x8006, 0xa381],
        [0x8004, 8],
        [2, 0x8001],
        [0xffff, 5],
        [0x8006, 0xa383],
        [31, 47],
        [53, 59],
        [0x8006, 0xa38b],
        [61, 67],
        [71, 73],
        [0xffff, 5],
        [0x800a, 0xd300],
        [0x8004, 0x8006],
        [1, 2],
        [3, 4],
        [5, 6],
        [7, 8],
        [9, 10],
        [0x8004, 39],
    ];
    for short in 1..=28 {
        let mut source = fixture(&words, 1);
        source.short = short;
        let mut visitor = Visitor::default();
        assert_eq!(parse(&mut source, &mut visitor).unwrap(), 16);
        assert!(source.max_request <= 24);
        assert_eq!(
            visitor.events[4],
            (
                116,
                NativeRecord::Glyph {
                    x: 90,
                    y: 13,
                    style: 41,
                    code: 0xcec4
                }
            )
        );
        assert_eq!(
            visitor.events[5].1,
            NativeRecord::Glyph {
                x: 20,
                y: 13,
                style: 41,
                code: 0xa0c4
            }
        );
        assert_eq!(
            visitor.events[8].1,
            NativeRecord::Glyph {
                x: 15,
                y: 71,
                style: 89,
                code: 0xffff
            }
        );
        assert_eq!(
            visitor.events[9],
            (
                136,
                NativeRecord::Drawing {
                    tag: 0x8006,
                    style: 0xa381,
                    points: [[0x8004, 8], [2, 0x8001]]
                }
            )
        );
        assert_eq!(
            visitor.events[10],
            (
                148,
                NativeRecord::Control {
                    tag: 0xffff,
                    value: 5
                }
            )
        );
        assert_eq!(visitor.events[11].0, 152);
        assert_eq!(visitor.events[12].0, 164);
        assert_eq!(
            visitor.events[14].1,
            NativeRecord::Image {
                words: [0xd300, 0x8004, 0x8006, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
            }
        );
        assert_eq!(
            visitor.events[15],
            (208, NativeRecord::End { value: Some(39) })
        );
    }
}

#[test]
fn unsupported_records_stop_without_consuming_their_payload_as_glyphs() {
    for pair in [
        [0x8006, 0],
        [0x8006, 0xa384],
        [0x8010, 0],
        [0x8010, 0xa381],
        [0xc052, 7],
        [0xc055, 8],
        [0x8071, 0],
        [0x8075, 0],
        [0x801d, 1],
        [0x8067, 0],
        [0x800a, 0],
        [0xffff, 4],
    ] {
        let mut source = fixture(
            &[[0x8001, 5], [0x8002, 7], pair, [13, 0xcec4], [0x8004, 0]],
            0,
        );
        let mut visitor = Visitor::default();
        let error = parse(&mut source, &mut visitor).unwrap_err();
        assert_eq!(error.offset, Some(108));
        assert_eq!(page_image(&error).0, Some(1));
        assert!(matches!(
            error,
            Error {
                kind: ErrorKind::UnsupportedFormat,
                ..
            }
        ));
        assert_eq!(visitor.events.len(), 2);
    }
}

#[test]
fn context_end_and_image_counts_are_checked() {
    for words in [
        vec![[7, 0xcec4]],
        vec![[0x8001, 5], [7, 0xcec4]],
        vec![[0x8002, 5], [7, 0xcec4]],
        vec![],
        vec![[0x8001, 5]],
        vec![[0x8004, 0], [0x8004, 0]],
    ] {
        let error = parse(&mut fixture(&words, 0), &mut Visitor::default()).unwrap_err();
        assert!(matches!(
            error,
            Error {
                kind: ErrorKind::Malformed,
                ..
            }
        ));
    }
    let mut image = vec![[0x800a, 0xd300]];
    image.extend([[7, 11]; 6]);
    image.push([0x8004, 0]);
    for (words, count) in [(&image[..], 0), (&[[0x8004, 0]][..], 1)] {
        assert!(matches!(
            parse(&mut fixture(words, count), &mut Visitor::default()).unwrap_err(),
            Error {
                kind: ErrorKind::Malformed,
                ..
            }
        ));
    }
}

#[test]
fn never_reads_past_indexed_span_for_any_truncated_record() {
    for words in [
        vec![[0x8004, 0]],
        vec![[0x8006, 0xa381], [1, 2], [3, 4]],
        vec![[0x8006, 0xa383], [1, 2], [3, 4]],
        vec![[0x8006, 0xa385], [1, 2], [3, 4]],
        vec![[0x8010, 1], [1, 2], [3, 4]],
        vec![[0x8090, 0xa3e6], [0xd2c6, 4364], [0xc08f, 125]],
        vec![[0x8090, 0xa3b2], [0xd2c6, 4364], [0xc08f, 125]],
        vec![[0xc053, 0xffff]],
        vec![[0x8073, 0x8004]],
        vec![
            [0x800a, 0xd300],
            [1, 2],
            [3, 4],
            [5, 6],
            [7, 8],
            [9, 10],
            [11, 12],
        ],
    ] {
        for length in 1..words.len() * 4 {
            let mut source = fixture(&words, u16::from(words[0][0] == 0x800a));
            source.bytes[84..88].copy_from_slice(&(length as u32).to_le_bytes());
            let error = parse(&mut source, &mut Visitor::default()).unwrap_err();
            assert!(
                matches!(error.kind, ErrorKind::Truncated { .. }),
                "{length}: {error:?}"
            );
        }
    }
}

#[test]
fn protected_index_fails_before_emitting_records() {
    let mut source = fixture(&[[0x8004, 0]], 0);
    source.bytes[80..84].copy_from_slice(&96_u32.to_le_bytes());
    assert!(matches!(
        parse(&mut source, &mut Visitor::default()).unwrap_err(),
        Error {
            kind: ErrorKind::Malformed,
            ..
        }
    ));
}

#[test]
fn native_records_require_a_current_page() {
    let limits = Limits::default();
    let cancel = Cancel::default();
    let mut source = fixture(&[[0x8004, 0]], 0);
    let mut reader = Hnc8Reader::open(&mut source, &limits, &cancel).unwrap();
    let mut visitor = Visitor::default();
    assert!(matches!(
        reader.visit_native_records(&mut visitor).unwrap_err(),
        Error {
            kind: ErrorKind::Malformed,
            reason: "no current page",
            ..
        }
    ));
}

#[test]
fn cancellation_source_and_visitor_failures_are_located() {
    for fault in 0..5 {
        let limits = Limits::default();
        let cancel = Cancel::default();
        let mut source = fixture(&[[0x8004, 0]], 0);
        let mut reader = Hnc8Reader::open(&mut source, &limits, &cancel).unwrap();
        reader.next_page().unwrap();
        let mut visitor = Visitor::default();
        match fault {
            0 => cancel.0.set(true),
            1 => reader.source.cancel_on_read = Some(cancel.clone()),
            2 => visitor.cancel = Some(cancel.clone()),
            3 => reader.source.fault = true,
            _ => visitor.fail = true,
        }
        let error = reader.visit_native_records(&mut visitor).unwrap_err();
        assert_eq!(error.offset, Some(100));
        if fault < 3 {
            assert!(matches!(error.kind, ErrorKind::Cancelled));
        } else {
            assert!(error.reason.starts_with("synthetic"), "{error}");
        }
    }
}

#[test]
fn current_variant_is_enforced() {
    for variant in [Variant::HnA, Variant::HnB, Variant::C8] {
        let limits = Limits::default();
        let cancel = Cancel::default();
        let mut source = fixture(
            &[
                [0x801d, 0],
                [0x8067, 5],
                [0x8067, 6],
                [0x8067, 8],
                [0x8004, 0],
            ],
            0,
        );
        let mut reader = Hnc8Reader::open(&mut source, &limits, &cancel).unwrap();
        reader.next_page().unwrap();
        reader.header.variant = variant;
        let result = reader.visit_native_records(&mut Visitor::default());
        if variant == Variant::C8 {
            assert_eq!(result.unwrap(), 5);
            assert_eq!(reader.next_image().unwrap(), None);
            assert_eq!(reader.next_page().unwrap(), None);
        } else {
            assert!(matches!(
                result.unwrap_err(),
                Error {
                    kind: ErrorKind::UnsupportedFormat,
                    ..
                }
            ));
        }
    }
}

#[test]
fn maps_verified_alphanumeric_and_gbk_codes_without_inventing_unknowns() {
    // The complete alphanumeric alphabet was independently checked with a
    // controlled source and the pinned viewer's ordinary-copy operation.
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    for &ascii in alphabet {
        assert_eq!(
            decode_native_character(0xa000 | u16::from(ascii + 0x80)),
            Some(char::from(ascii))
        );
    }
    for (code, expected) in [
        (0xcec4, '文'),
        (0xb2e2, '测'),
        (0xcee4, '武'),
        (0xa1a1, '\u{3000}'),
        (0xa3ac, '\u{ff0c}'),
        (0xa3b0, '\u{ff10}'),
        (0xa0a6, '\u{ff06}'),
        (0xa0ad, '\u{ff0d}'),
        (0xa0ae, '\u{ff0e}'),
        (0xa0af, '\u{ff0f}'),
        (0xa0ba, ':'),
        (0xaab1, '\u{2219}'),
        (0xaab2, '-'),
        (0xaab3, '\u{2217}'),
        (0xaca3, '\u{25ba}'),
    ] {
        assert_eq!(decode_native_character(code), Some(expected));
    }
    // Viewer ordinary copy normalizes some punctuation/digits and even emits
    // U+0082 for a comma. Do not make those clipboard transformations our map.
    for code in [
        0,
        0x4170,
        0x8140_u16 - 1,
        0x817f,
        0xffff,
        0xa001,
        0xa0a5,
        0xa0a7,
        0xa0a0,
        0xa0ff,
        0xaab5,
        0xaab4,
        0xaca2,
        0xaca4,
        0xaaa1,
    ] {
        assert_eq!(decode_native_character(code), None, "{code:04x}");
    }
}

#[test]
fn a_text_consumer_rejects_unmapped_glyphs_at_their_source_record() {
    struct Text;
    impl NativeRecordVisitor for Text {
        fn visit(&mut self, _: u64, record: NativeRecord) -> crate::Result<()> {
            if let NativeRecord::Glyph { code, .. } = record {
                decode_native_character(code).ok_or(Error::from(ErrorKind::UnsupportedFormat))?;
            }
            Ok(())
        }
    }
    for code in [0xcec4, 0xa0da, 0xa0a5, 0xffff] {
        let mut source = fixture(&[[0x8001, 3], [0x8002, 5], [11, code], [0x8004, 0]], 0);
        let limits = Limits::default();
        let cancel = Cancel::default();
        let mut reader = Hnc8Reader::open(&mut source, &limits, &cancel).unwrap();
        reader.next_page().unwrap();
        let result = reader.visit_native_records(&mut Text);
        if matches!(code, 0xcec4 | 0xa0da) {
            assert_eq!(result.unwrap(), 4);
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.offset, Some(108));
            assert_eq!(page_image(&error).0, Some(1));
            assert!(matches!(error.kind, ErrorKind::UnsupportedFormat));
        }
    }
}

#[test]
fn preserves_additional_controls_and_atomic_8010_payload() {
    for short in [1, 3, 28] {
        let mut words = vec![[0x8001, 4700], [0x8002, 0x1084]];
        for tag in [0x8072, 0x8073, 0x8074, 0xc053, 0xc054] {
            // Marker-looking values must remain payload, not terminate the page.
            words.push([tag, 0x8004]);
        }
        words.extend([
            [0x8010, 1],
            [0x8004, 0x8001],
            [0x8006, 0xffff],
            [0xffff, 5],
            [5200, 0xd6d0],
            [0x8004, 1],
        ]);
        let mut source = fixture(&words, 0);
        source.short = short;
        let mut visitor = Visitor::default();
        assert_eq!(parse(&mut source, &mut visitor).unwrap(), 11);
        for (index, tag) in [0x8072, 0x8073, 0x8074, 0xc053, 0xc054]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                visitor.events[index + 2],
                (
                    108 + index as u64 * 4,
                    NativeRecord::Control { tag, value: 0x8004 }
                )
            );
        }
        assert_eq!(
            visitor.events[7],
            (
                128,
                NativeRecord::Drawing {
                    tag: 0x8010,
                    style: 1,
                    points: [[0x8004, 0x8001], [0x8006, 0xffff]],
                }
            )
        );
        assert_eq!(
            visitor.events[9],
            (
                144,
                NativeRecord::Glyph {
                    x: 5200,
                    y: 4700,
                    style: 0x1084,
                    code: 0xd6d0,
                }
            )
        );
        assert_eq!(
            visitor.events[10],
            (148, NativeRecord::End { value: Some(1) })
        );
    }
    let error = parse(
        &mut fixture(&[[0x8010, 1], [7, 9], [21, 13], [0xffff, 6]], 0),
        &mut Visitor::default(),
    )
    .unwrap_err();
    assert_eq!(error.offset, Some(112));
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::UnsupportedFormat,
            ..
        }
    ));
}

#[test]
fn native_image_coordinates_keep_source_axes_and_reject_unknown_profiles() {
    let mut words = [0; 13];
    words[..5].copy_from_slice(&[0xd300, 0xc000 | 4682, 4314, 0xc000 | 80, 50]);
    let coordinate = decode_native_image_coordinate(&words).unwrap();
    assert_eq!(
        (
            coordinate.x,
            coordinate.y,
            coordinate.width,
            coordinate.height
        ),
        (4682, 4314, 80, 50)
    );
    for (field, delta) in [(1, 20), (2, 20), (3, 20), (4, 20)] {
        let mut changed = words;
        changed[field] += delta;
        let decoded = decode_native_image_coordinate(&changed).unwrap();
        let mut expected = [4682, 4314, 80, 50];
        expected[field - 1] += delta;
        assert_eq!(
            [decoded.x, decoded.y, decoded.width, decoded.height],
            expected
        );
    }
    for (field, value) in [
        (0, 0xd301),
        (1, 0),
        (1, 0x4000),
        (1, 0x8000),
        (3, 0),
        (3, 0x4001),
        (3, 0x8001),
        (3, 0xc000),
        (4, 0),
    ] {
        let mut invalid = words;
        invalid[field] = value;
        assert!(decode_native_image_coordinate(&invalid).is_none());
    }
    words[..5].copy_from_slice(&[0xd300, 0xffff, 0xffff, 0xffff, 0xffff]);
    let extreme = decode_native_image_coordinate(&words).unwrap();
    assert_eq!(
        [extreme.x, extreme.y, extreme.width, extreme.height],
        [0x3fff, 0xffff, 0x3fff, 0xffff]
    );
    words[1] = 0xc000;
    words[2] = 0;
    let origin = decode_native_image_coordinate(&words).unwrap();
    assert_eq!([origin.x, origin.y], [0, 0]);
}

#[test]
fn a385_drawing_preserves_coordinates_and_following_glyph_context() {
    for short in [1, 3, 7, 28] {
        let mut source = fixture(
            &[
                [0x8001, 47],
                [0x8002, 0x1084],
                [0x8006, 0xa385],
                [0x8004, 17],
                [23, 0x8001],
                [0xffff, 5],
                [31, 0xd6d0],
                [0x8004, 1],
            ],
            0,
        );
        source.short = short;
        let mut visitor = Visitor::default();
        assert_eq!(parse(&mut source, &mut visitor).unwrap(), 6);
        assert_eq!(
            visitor.events[2],
            (
                108,
                NativeRecord::Drawing {
                    tag: 0x8006,
                    style: 0xa385,
                    points: [[0x8004, 17], [23, 0x8001]],
                }
            )
        );
        assert_eq!(
            visitor.events[4],
            (
                124,
                NativeRecord::Glyph {
                    x: 31,
                    y: 47,
                    style: 0x1084,
                    code: 0xd6d0,
                }
            )
        );
        assert_eq!(
            visitor.events[5],
            (128, NativeRecord::End { value: Some(1) })
        );
        assert!(source.max_request <= 28);
    }
}

#[test]
fn drawing_boundary_preserves_independent_y_end_and_control_records() {
    for (tag, value) in [
        (0x8006, 0xa381),
        (0x8006, 0xa383),
        (0x8006, 0xa385),
        (0x8006, 0xa38b),
        (0x8010, 1),
    ] {
        for short in [1, 3, 7, 28] {
            let mut source = fixture(
                &[
                    [0x8001, 47],
                    [0x8002, 0x1084],
                    [tag, value],
                    [0x8004, 17],
                    [23, 0x8001],
                    [0x8001, 71],
                    [31, 0xd6d0],
                    [0xffff, 5],
                    [0x8004, 1],
                ],
                0,
            );
            source.short = short;
            let mut visitor = Visitor::default();
            assert_eq!(parse(&mut source, &mut visitor).unwrap(), 7);
            assert_eq!(
                visitor.events[3],
                (
                    120,
                    NativeRecord::Control {
                        tag: 0x8001,
                        value: 71
                    }
                )
            );
            assert_eq!(
                visitor.events[4].1,
                NativeRecord::Glyph {
                    x: 31,
                    y: 71,
                    style: 0x1084,
                    code: 0xd6d0
                }
            );
            assert_eq!(
                visitor.events[5],
                (
                    128,
                    NativeRecord::Control {
                        tag: 0xffff,
                        value: 5
                    }
                )
            );
            let mut source = fixture(&[[tag, value], [1, 2], [3, 4], [0x8004, 1]], 0);
            assert_eq!(parse(&mut source, &mut Visitor::default()).unwrap(), 2);
        }
    }
}

fn encoded_string_fixture(characters: usize) -> Source {
    let mut source = fixture(
        &[[0x8001, 37], [0x8002, 0x1084], [71, 0xcec4], [0x8004, 0]],
        0,
    );
    let mut prefix = Vec::new();
    prefix.extend(0x80cc_u16.to_le_bytes());
    prefix.extend((0x102 + characters as u16).to_le_bytes());
    for byte in b"Fixture /~".iter().cycle().take(characters) {
        prefix.extend((0xe000 | u16::from(*byte)).to_le_bytes());
    }
    source.bytes.splice(108..108, prefix);
    let length = source.bytes.len() as u32 - 100;
    source.bytes[84..88].copy_from_slice(&length.to_le_bytes());
    source
}

#[test]
fn encoded_strings_preserve_run_context_and_bounded_source_spans() {
    for characters in [0, 5, 28, 253] {
        for short in [1, 3, 7, 28] {
            let mut source = encoded_string_fixture(characters);
            source.short = short;
            let mut visitor = Visitor::default();
            assert_eq!(parse(&mut source, &mut visitor).unwrap(), 5);
            assert_eq!(
                visitor.events[2],
                (
                    108,
                    NativeRecord::EncodedString {
                        value: 0x102 + characters as u16,
                        payload: super::super::Span {
                            offset: 112,
                            length: characters as u64 * 2
                        },
                    }
                )
            );
            assert_eq!(
                visitor.events[3],
                (
                    112 + characters as u64 * 2,
                    NativeRecord::Glyph {
                        x: 71,
                        y: 37,
                        style: 0x1084,
                        code: 0xcec4,
                    }
                )
            );
            assert!(source.max_request <= 28);
        }
    }
}

#[test]
fn encoded_strings_reject_unknown_lengths_and_embedded_markers() {
    for value in [0_u16, 0x0100, 0x0101, 0x0200, 0xffff] {
        let mut source = encoded_string_fixture(5);
        source.bytes[110..112].copy_from_slice(&value.to_le_bytes());
        let mut visitor = Visitor::default();
        let error = parse(&mut source, &mut visitor).unwrap_err();
        assert_eq!(error.offset, Some(108));
        assert!(matches!(
            error,
            Error {
                kind: ErrorKind::UnsupportedFormat,
                ..
            }
        ));
        assert_eq!(visitor.events.len(), 2);
    }
    for word in [0xe000_u16, 0xe01f, 0xe07f, 0xe080, 0x8004, 0x8001, 0xffff] {
        let mut source = encoded_string_fixture(28);
        source.bytes[140..142].copy_from_slice(&word.to_le_bytes());
        let limits = Limits::default();
        let cancel = Cancel::default();
        let mut reader = Hnc8Reader::open(&mut source, &limits, &cancel).unwrap();
        reader.next_page().unwrap();
        let mut visitor = Visitor::default();
        let error = reader.visit_native_records(&mut visitor).unwrap_err();
        assert_eq!(error.offset, Some(140));
        assert!(matches!(
            error,
            Error {
                kind: ErrorKind::UnsupportedFormat,
                reason: "native encoded-string word",
                ..
            }
        ));
        assert_eq!(visitor.events.len(), 2);
    }
}

#[test]
fn terminated_encoded_strings_preserve_the_following_glyph() {
    // Include a terminator crossing the 28-byte read boundary and the maximum
    // record length. A NUL does not shorten the declared source span.
    for characters in [1, 14, 15, 30, 253] {
        let mut source = encoded_string_fixture(characters);
        let end = 112 + characters * 2;
        source.bytes[end - 2..end].copy_from_slice(&0xe000_u16.to_le_bytes());
        source.short = 1;
        let mut visitor = Visitor::default();
        assert_eq!(parse(&mut source, &mut visitor).unwrap(), 5);
        assert_eq!(visitor.events[3].0, end as u64);
        assert!(matches!(
            visitor.events[3].1,
            NativeRecord::Glyph { code: 0xcec4, .. }
        ));
        assert!(source.max_request <= 28);
    }
}

#[test]
fn encoded_string_truncation_never_consumes_outside_the_page_span() {
    for bytes in 1..60 {
        let mut source = encoded_string_fixture(28);
        source.bytes[84..88].copy_from_slice(&(8_u32 + bytes).to_le_bytes());
        let mut visitor = Visitor::default();
        let error = parse(&mut source, &mut visitor).unwrap_err();
        assert!(
            matches!(error.kind, ErrorKind::Truncated { .. }),
            "{bytes}: {error:?}"
        );
        assert_eq!(visitor.events.len(), 2);
    }
}

#[test]
fn additional_controls_preserve_raw_values_without_inventing_glyphs() {
    let controls = [
        [0x801c, 4],
        [0x801d, 3],
        [0x801d, 28],
        [0x801d, 31],
        [0x8070, 4],
        [0x8071, 4],
        [0x80ce, 0],
        [0x80ce, 1],
        [0x8024, 0x2800],
        [0x8024, 0x281d],
        [0x8024, 0x281c],
        [0x8021, 0x2000],
        [0x80d0, 0],
        [0x80d1, 1],
        [0x80d2, 0],
        [0x80d3, 0],
        [0x80d3, 1],
        [0x80d3, 2],
        [0x80d5, 0],
        [0x9002, 0],
    ];
    for control in controls {
        let mut source = fixture(
            &[
                [0x8001, 17],
                [0x8002, 0x1084],
                [31, 0xd6d0],
                control,
                [73, 0xcec4],
                [0x8004, 0],
            ],
            0,
        );
        source.short = 1;
        let mut visitor = Visitor::default();
        assert_eq!(parse(&mut source, &mut visitor).unwrap(), 6);
        assert_eq!(
            visitor.events[3],
            (
                112,
                NativeRecord::Control {
                    tag: control[0],
                    value: control[1]
                }
            )
        );
        assert_eq!(
            visitor.events[4],
            (
                116,
                NativeRecord::Glyph {
                    x: 73,
                    y: 17,
                    style: 0x1084,
                    code: 0xcec4
                }
            )
        );
        assert!(source.max_request <= 24);
        source.bytes[114..116].copy_from_slice(&0xffff_u16.to_le_bytes());
        let error = parse(&mut source, &mut Visitor::default()).unwrap_err();
        assert_eq!(error.offset, Some(112));
        assert!(matches!(
            error,
            Error {
                kind: ErrorKind::UnsupportedFormat,
                ..
            }
        ));
    }
}

#[test]
fn extended_controls_are_atomic_and_bounded_even_with_marker_payloads() {
    for (tag, value) in [(0x81ff, 1), (0x81ff, 2), (0x81ff, 3), (0x80cc, 0x0204)] {
        for words in [[0, 200], [33, 5], [0x8004, 0x8001]] {
            for short in 1..=8 {
                let mut source = fixture(
                    &[
                        [0x8001, 17],
                        [0x8002, 0x1084],
                        [tag, value],
                        words,
                        [73, 0xcec4],
                        [0x8004, 0],
                    ],
                    0,
                );
                source.short = short;
                let mut visitor = Visitor::default();
                assert_eq!(parse(&mut source, &mut visitor).unwrap(), 5);
                assert_eq!(
                    visitor.events[2],
                    (108, NativeRecord::ExtendedControl { tag, value, words })
                );
                assert_eq!(
                    visitor.events[3],
                    (
                        116,
                        NativeRecord::Glyph {
                            x: 73,
                            y: 17,
                            style: 0x1084,
                            code: 0xcec4
                        }
                    )
                );
            }
        }
        for length in 1..8_u32 {
            let mut source = fixture(&[[tag, value], [0, 200], [0x8004, 0]], 0);
            source.bytes[84..88].copy_from_slice(&length.to_le_bytes());
            let mut visitor = Visitor::default();
            assert!(matches!(
                parse(&mut source, &mut visitor).unwrap_err().kind,
                ErrorKind::Truncated { .. }
            ));
            assert!(visitor.events.is_empty());
        }
    }
    for pair in [[0x81ff, 0], [0x81ff, 4], [0x80cc, 0x0203], [0x80cc, 0x0205]] {
        let mut source = fixture(&[pair, [0, 200], [0x8004, 0]], 0);
        assert!(matches!(
            parse(&mut source, &mut Visitor::default()).unwrap_err(),
            Error {
                kind: ErrorKind::UnsupportedFormat,
                ..
            }
        ));
    }
}

fn image_reference_fixture(name: &[u8], declared_images: u16) -> Source {
    let mut source = fixture(&[], declared_images);
    for word in [0x810a_u16, 0xd300, 4673, 4297, 83, 51, 0, name.len() as u16] {
        source.bytes.extend(word.to_le_bytes());
    }
    source.bytes.extend(name);
    source.bytes.push(0);
    source
        .bytes
        .resize(source.bytes.len().next_multiple_of(4), 0);
    for word in [0x8004_u16, 1] {
        source.bytes.extend(word.to_le_bytes());
    }
    let length = source.bytes.len() as u32 - 100;
    source.bytes[84..88].copy_from_slice(&length.to_le_bytes());
    source
}

#[test]
fn image_references_stream_names_with_byte_lengths_and_aligned_ends() {
    for length in [0, 3, 4, 5, 8, 27, 28, 29, 260, 65535] {
        let name: Vec<_> = b"../original\x04\x80name"
            .iter()
            .copied()
            .cycle()
            .take(length)
            .collect();
        for short in [1, 7, 28] {
            let mut source = image_reference_fixture(&name, 1);
            source.short = short;
            let mut visitor = Visitor::default();
            assert_eq!(parse(&mut source, &mut visitor).unwrap(), 2);
            assert_eq!(
                visitor.events[0],
                (
                    100,
                    NativeRecord::ImageReference {
                        coordinate: super::super::RawTextCoordinate {
                            x: 4673,
                            y: 4297,
                            width: 83,
                            height: 51
                        },
                        reference: super::super::Span {
                            offset: 116,
                            length: length as u64
                        },
                    }
                )
            );
            assert_eq!(visitor.events[1].0, source.bytes.len() as u64 - 4);
            assert!(source.max_request <= 28);
        }
    }
}

#[test]
fn image_reference_flags_padding_and_counts_fail_at_their_source_positions() {
    for (offset, value, at) in [(102, 0_u16, 100), (112, 1, 112)] {
        let mut source = image_reference_fixture(b"abcd", 1);
        source.bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        let mut visitor = Visitor::default();
        let error = parse(&mut source, &mut visitor).unwrap_err();
        assert_eq!(error.offset, Some(at));
        assert!(matches!(
            error,
            Error {
                kind: ErrorKind::UnsupportedFormat,
                ..
            }
        ));
        assert!(visitor.events.is_empty());
    }
    for offset in 117..120 {
        let mut source = image_reference_fixture(b"a", 1);
        source.bytes[offset] = 1;
        let mut visitor = Visitor::default();
        let error = parse(&mut source, &mut visitor).unwrap_err();
        assert_eq!(error.offset, Some(offset as u64));
        assert!(
            (matches!(
                error,
                Error {
                    kind: ErrorKind::Malformed,
                    ..
                }
            ) && field_of(&error) == "native image reference")
        );
        assert!(visitor.events.is_empty());
    }
    for count in [0, 2] {
        let mut source = image_reference_fixture(b"abcd", count);
        assert!(
            (matches!(
                parse(&mut source, &mut Visitor::default()).unwrap_err(),
                Error {
                    kind: ErrorKind::Malformed,
                    ..
                }
            ) && field_of(&parse(&mut source, &mut Visitor::default()).unwrap_err())
                == "native image records")
        );
    }
}

#[test]
fn aligned_image_names_need_no_nul_and_preserve_following_records() {
    for length in [0, 4, 24, 260] {
        for short in [1, 7, 28] {
            let mut source = image_reference_fixture(&vec![b'x'; length], 1);
            let next = 116 + length;
            source.bytes.drain(next..next + 4);
            source.short = short;
            let text_length = source.bytes.len() as u32 - 100;
            source.bytes[84..88].copy_from_slice(&text_length.to_le_bytes());
            let mut visitor = Visitor::default();
            assert_eq!(parse(&mut source, &mut visitor).unwrap(), 2);
            assert_eq!(
                visitor.events[1],
                (next as u64, NativeRecord::End { value: Some(1) })
            );
            assert!(source.max_request <= 28);
        }
    }
}

#[test]
fn image_reference_lengths_cannot_cross_the_indexed_span() {
    for length in 1..48_u32 {
        let mut source = image_reference_fixture(&[b'a'; 29], 1);
        source.bytes[84..88].copy_from_slice(&length.to_le_bytes());
        let mut visitor = Visitor::default();
        let error = parse(&mut source, &mut visitor).unwrap_err();
        assert!(
            matches!(error.kind, ErrorKind::Truncated { .. }),
            "{length}: {error:?}"
        );
        assert!(visitor.events.is_empty());
    }
    let mut source = image_reference_fixture(b"abc", 1);
    source.bytes[114..116].copy_from_slice(&65535_u16.to_le_bytes());
    assert!(matches!(
        parse(&mut source, &mut Visitor::default())
            .unwrap_err()
            .kind,
        ErrorKind::Truncated { .. }
    ));
}

#[test]
fn image_reference_mid_payload_failure_and_cancellation_are_reported() {
    struct Interrupt {
        source: Source,
        cancel: Cancel,
        fail: bool,
    }
    impl RangedSource for Interrupt {
        fn size(&self) -> u64 {
            self.source.size()
        }
        fn read_at(&mut self, offset: u64, bytes: &mut [u8]) -> crate::Result<usize> {
            if offset >= 144 {
                self.source.fault = self.fail;
                if !self.fail {
                    self.source.cancel_on_read = Some(self.cancel.clone());
                }
            }
            self.source.read_at(offset, bytes)
        }
    }
    for fail in [false, true] {
        let limits = Limits::default();
        let cancel = Cancel::default();
        let mut source = Interrupt {
            source: image_reference_fixture(&[b'a'; 100], 1),
            cancel: cancel.clone(),
            fail,
        };
        let mut reader = Hnc8Reader::open(&mut source, &limits, &cancel).unwrap();
        reader.next_page().unwrap();
        let mut visitor = Visitor::default();
        let error = reader.visit_native_records(&mut visitor).unwrap_err();
        assert!(
            matches!(error.kind, ErrorKind::Cancelled) || error.reason.starts_with("synthetic"),
            "{error}"
        );
        assert!(visitor.events.is_empty());
    }
}

#[test]
fn reference_and_legacy_images_share_the_declared_page_count() {
    let mut source = image_reference_fixture(b"abcd", 2);
    source.bytes.truncate(124);
    for pair in [
        [0x800a_u16, 0xd300],
        [0xc101, 17],
        [0xc021, 19],
        [0, 0],
        [0, 0],
        [0, 0],
        [0, 0],
        [0x8004, 1],
    ] {
        for word in pair {
            source.bytes.extend(word.to_le_bytes());
        }
    }
    source.bytes[84..88].copy_from_slice(&56_u32.to_le_bytes());
    let mut visitor = Visitor::default();
    assert_eq!(parse(&mut source, &mut visitor).unwrap(), 3);
    assert!(matches!(
        visitor.events[0].1,
        NativeRecord::ImageReference { .. }
    ));
    assert!(matches!(
        visitor.events[1],
        (124, NativeRecord::Image { .. })
    ));
    assert_eq!(
        visitor.events[2],
        (152, NativeRecord::End { value: Some(1) })
    );
}

fn hnb_source(width: usize, pages: &[&[[u16; 2]]]) -> Source {
    let mut source = fixture(&[], 0);
    source.bytes = vec![0; 216 + width * pages.len()];
    for (offset, value) in [
        (0, 0x4e48_u32),
        (4, 200),
        (8, 136),
        (136, if width == 12 { 0 } else { 0xc8 }),
        (144, pages.len() as u32),
        (148, 2),
    ] {
        source.bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    for (index, words) in pages.iter().enumerate() {
        let row = 216 + index * width;
        let offset = source.bytes.len() as u32;
        source.bytes[row..row + 4].copy_from_slice(&offset.to_le_bytes());
        source.bytes[row + 4..row + 8].copy_from_slice(&(words.len() as u32 * 4).to_le_bytes());
        for pair in *words {
            for word in pair {
                source.bytes.extend(word.to_le_bytes());
            }
        }
    }
    source
}

#[test]
fn hnb_glyph_runs_use_both_verified_indexes_without_crossing_pages() {
    let first = [
        [0x8001, 4700],
        [0x8002, 0x1084],
        [0x801d, 0],
        [0x8067, 6],
        [5200, 0xd6d0],
        [0x8004, 1],
    ];
    let second = [
        [0x8001, 5200],
        [0x8002, 0x1084],
        [0x801d, 0],
        [0x8067, 6],
        [5300, 0xa0c1],
        [5600, 0xa0cd],
        [0x8004, 2],
    ];
    for width in [12, 20] {
        for short in [1, 3, 7, 28] {
            let mut source = hnb_source(width, &[&first, &second]);
            source.short = short;
            {
                let limits = Limits::default();
                let cancel = Cancel::default();
                let mut reader = Hnc8Reader::open(&mut source, &limits, &cancel).unwrap();
                assert_eq!(reader.header().variant, Variant::HnB);
                for (page, words) in [&first[..], &second[..]].into_iter().enumerate() {
                    assert_eq!(
                        reader.next_page().unwrap().unwrap().page_number,
                        page as u32 + 1
                    );
                    let mut visitor = Visitor::default();
                    assert_eq!(
                        reader.visit_native_records(&mut visitor).unwrap(),
                        words.len() as u32
                    );
                    let start = (216 + width * 2 + page * first.len() * 4) as u64;
                    assert_eq!(
                        visitor.events[4],
                        (
                            start + 16,
                            NativeRecord::Glyph {
                                x: words[4][0],
                                y: words[0][1],
                                style: words[1][1],
                                code: words[4][1],
                            }
                        )
                    );
                    assert_eq!(
                        visitor.events.last(),
                        Some(&(
                            start + (words.len() as u64 - 1) * 4,
                            NativeRecord::End {
                                value: Some(page as u16 + 1)
                            }
                        ))
                    );
                }
                assert!(reader.next_page().unwrap().is_none());
            };
            assert!(source.max_request <= 28);
        }
    }
}

#[test]
fn hnb_does_not_inherit_unverified_c8_records_or_font_controls() {
    for pair in [
        [0x8006, 0xa38b],
        [0x80cc, 0x0102],
        [0x800a, 0xd301],
        [0x801d, 1],
        [0x8067, 10],
        [0x8075, 0xb7bd],
        [0xc052, 0xa384],
    ] {
        let mut source = hnb_source(
            12,
            &[&[[0x8001, 4700], [0x8002, 0x1084], pair, [0x8004, 1]]],
        );
        let mut visitor = Visitor::default();
        let error = parse(&mut source, &mut visitor).unwrap_err();
        assert_eq!(variant_of(&error), Some(Variant::HnB));
        assert_eq!(page_image(&error).0, Some(1));
        assert_eq!(error.offset, Some(236));
        assert!(matches!(
            error,
            Error {
                kind: ErrorKind::UnsupportedFormat,
                reason: "HN-B native record tag/value",
                ..
            }
        ));
        assert_eq!(visitor.events.len(), 2);
    }
}

#[test]
fn hnb_truncated_record_keeps_the_next_page_unread() {
    for width in [12, 20] {
        for length in 1..4_u32 {
            let mut source = hnb_source(width, &[&[[0x8001, 4700]], &[[0x8004, 2]]]);
            source.bytes[220..224].copy_from_slice(&length.to_le_bytes());
            {
                let limits = Limits::default();
                let cancel = Cancel::default();
                let mut reader = Hnc8Reader::open(&mut source, &limits, &cancel).unwrap();
                reader.next_page().unwrap();
                let mut visitor = Visitor::default();
                let error = reader.visit_native_records(&mut visitor).unwrap_err();
                assert!(
                    matches!(error.kind, ErrorKind::Truncated { expected: 4, available, .. } if available == u64::from(length))
                );
                assert!(visitor.events.is_empty());
            };
        }
    }
}

#[test]
fn hnb_prefix_is_one_atomic_eight_byte_record() {
    for width in [12, 20] {
        for payload in [[0xffff, 5], [0x8004, 1], [5200, 0xd6d0]] {
            for short in [1, 3, 7, 28] {
                let mut source = hnb_source(
                    width,
                    &[&[
                        [0xc052, 0xa385],
                        payload,
                        [0x8001, 4700],
                        [0x8002, 0x1084],
                        [5200, 0xd6d0],
                        [0x8004, 1],
                    ]],
                );
                source.short = short;
                let mut visitor = Visitor::default();
                assert_eq!(parse(&mut source, &mut visitor).unwrap(), 5);
                assert_eq!(
                    visitor.events[0],
                    (
                        (216 + width) as u64,
                        NativeRecord::ExtendedControl {
                            tag: 0xc052,
                            value: 0xa385,
                            words: payload
                        }
                    )
                );
                assert!(matches!(
                    visitor.events[3].1,
                    NativeRecord::Glyph {
                        x: 5200,
                        y: 4700,
                        style: 0x1084,
                        code: 0xd6d0
                    }
                ));
            }
        }
        for length in 1..8_u32 {
            let mut source = hnb_source(width, &[&[[0xc052, 0xa385], [0x8004, 1]], &[[0x8004, 2]]]);
            source.bytes[220..224].copy_from_slice(&length.to_le_bytes());
            let mut visitor = Visitor::default();
            assert!(matches!(
                parse(&mut source, &mut visitor).unwrap_err().kind,
                ErrorKind::Truncated { .. }
            ));
            assert!(visitor.events.is_empty());
        }
    }
}

#[test]
fn hnb_run_controls_and_drawing_preserve_following_glyph_context() {
    let controls = [
        [0x8074, 0xb7bd],
        [0x8074, 0xcfc8],
        [0x8074, 0x8004],
        [0x8074, 0xffff],
        [0x8067, 5],
        [0x801d, 3],
        [0x801d, 4],
        [0x8070, 0x001c],
        [0x801c, 4],
        [0x8067, 7],
        [0x8067, 18],
        [0x8072, 1],
        [0x8072, 0x8004],
        [0x8073, 278],
        [0x8073, 0x8004],
        [0x8067, 9],
        [0x8069, 0x1084],
        [0x8070, 0x0024],
        [0x8070, 0x002b],
        [0x80ce, 0],
        [0x80ce, 1],
        [0x8071, 0x0024],
        [0x8071, 0x002b],
        [0x8073, 0x001e],
        [0x8073, 0x001f],
        [0x8073, 0x0020],
        [0x8073, 0x0029],
        [0x8073, 0x002a],
        [0x8073, 0x002b],
        [0x8072, 0x1084],
        [0x8072, 0xc2c7],
        [0x8072, 0xcdc1],
        [0x8072, 0],
        [0x8024, 0x2800],
        [0x8024, 0x281d],
        [0xc053, 0],
        [0xc053, 0x00e8],
        [0xc053, 0x00e9],
        [0xc053, 0x12d8],
        [0xc053, 0x8004],
        [0xc053, 0xffff],
        [0xffff, 5],
    ];
    for width in [12, 20] {
        for short in [1, 3, 7, 28] {
            for control in controls {
                let mut source = hnb_source(
                    width,
                    &[&[
                        [0x8001, 4700],
                        [0x8002, 0x1084],
                        control,
                        [5200, 0xd6d0],
                        [0x8004, 1],
                    ]],
                );
                source.short = short;
                let mut visitor = Visitor::default();
                assert_eq!(parse(&mut source, &mut visitor).unwrap(), 5);
                assert_eq!(
                    visitor.events[2].1,
                    NativeRecord::Control {
                        tag: control[0],
                        value: control[1]
                    }
                );
                assert_eq!(
                    visitor.events[3].1,
                    NativeRecord::Glyph {
                        x: 5200,
                        y: 4700,
                        style: 0x1084,
                        code: 0xd6d0
                    }
                );
            }
            for style in [0xa381, 0xa383, 0xa385] {
                for following in [[0xffff, 5], [0x8001, 5000]] {
                    let mut source = hnb_source(
                        width,
                        &[&[
                            [0x8001, 4700],
                            [0x8002, 0x1084],
                            [0x8006, style],
                            [0x8004, 1],
                            [0x8001, 0x8002],
                            following,
                            [5200, 0xd6d0],
                            [0x8004, 1],
                        ]],
                    );
                    source.short = short;
                    let mut visitor = Visitor::default();
                    assert_eq!(parse(&mut source, &mut visitor).unwrap(), 6);
                    assert_eq!(
                        visitor.events[2].1,
                        NativeRecord::Drawing {
                            tag: 0x8006,
                            style,
                            points: [[0x8004, 1], [0x8001, 0x8002]]
                        }
                    );
                    assert_eq!(
                        visitor.events[4].1,
                        NativeRecord::Glyph {
                            x: 5200,
                            y: if following[0] == 0x8001 { 5000 } else { 4700 },
                            style: 0x1084,
                            code: 0xd6d0
                        }
                    );
                    assert!(source.max_request <= 28);
                }
            }
        }
        for style in [0xa381, 0xa383, 0xa385] {
            let mut source = hnb_source(
                width,
                &[&[[0x8006, style], [5200, 4800], [6300, 4850], [0x8004, 1]]],
            );
            let mut visitor = Visitor::default();
            assert_eq!(parse(&mut source, &mut visitor).unwrap(), 2);
            assert_eq!(visitor.events[1].1, NativeRecord::End { value: Some(1) });
        }
        for length in 1..12_u32 {
            let mut source = hnb_source(
                width,
                &[&[[0x8006, 0xa385], [5200, 4800], [6300, 4850], [0xffff, 5]]],
            );
            source.bytes[220..224].copy_from_slice(&length.to_le_bytes());
            let mut visitor = Visitor::default();
            assert!(matches!(
                parse(&mut source, &mut visitor).unwrap_err().kind,
                ErrorKind::Truncated { .. }
            ));
            assert!(visitor.events.is_empty());
        }
        let mut source = hnb_source(
            width,
            &[&[
                [0x8006, 0xa385],
                [5200, 4800],
                [6300, 4850],
                [0xffff, 4],
                [0x8004, 1],
            ]],
        );
        assert!(matches!(
            parse(&mut source, &mut Visitor::default()).unwrap_err(),
            Error {
                kind: ErrorKind::UnsupportedFormat,
                ..
            }
        ));
    }
    for control in [
        [0x801c, 5],
        [0x801d, 1],
        [0x801d, 28],
        [0x801d, 31],
        [0x8067, 8],
        [0x8069, 0x1085],
        [0x8070, 0x0023],
        [0x8070, 0x002c],
        [0x80ce, 2],
        [0x8071, 0x0025],
        [0x8024, 0x281c],
        [0xc054, 0x00e9],
        [0x8006, 0xa384],
    ] {
        let mut source = hnb_source(12, &[&[control, [0x8004, 1]]]);
        assert!(matches!(
            parse(&mut source, &mut Visitor::default()).unwrap_err(),
            Error {
                kind: ErrorKind::UnsupportedFormat,
                ..
            }
        ));
    }
    // These values remain HN-B-only. Independently admitted C8 numeric
    // controls and a385 drawings have their own positive tests above.
    // Explicit axis 36 is now shared; native-page tests cover both orders.
    for control in [
        [0x8067, 7],
        [0x8069, 0x1084],
        [0x8070, 0x002b],
        [0x8071, 0x002b],
    ] {
        let mut source = fixture(&[control, [0x8004, 1]], 0);
        assert!(matches!(
            parse(&mut source, &mut Visitor::default()).unwrap_err(),
            Error {
                kind: ErrorKind::UnsupportedFormat,
                ..
            }
        ));
    }
}

#[test]
fn hnb_implicit_style_requires_verified_paired_axes() {
    for width in [12, 20] {
        for (controls, admitted) in [
            (vec![], false),
            (vec![[0x8070, 0x002b]], false),
            (vec![[0x8071, 0x002b]], false),
            (vec![[0x8070, 0x002b], [0x8071, 0x002b]], true),
        ] {
            let mut words = vec![[0x8001, 4700]];
            words.extend(controls);
            words.extend([[5200, 0xd6d0], [0x8004, 1]]);
            let mut source = hnb_source(width, &[&words]);
            if admitted {
                assert_eq!(parse(&mut source, &mut Visitor::default()).unwrap(), 5);
                continue;
            }
            let error = parse(&mut source, &mut Visitor::default()).unwrap_err();
            assert!(matches!(
                error,
                Error {
                    kind: ErrorKind::UnsupportedFormat,
                    reason: "HN-B implicit native glyph style",
                    ..
                }
            ));
        }
    }
}

#[test]
fn hnb_images_preserve_atomic_words_and_following_glyph_order() {
    let image = [
        [0x800a, 0xd300],
        [0xc000 | 4682, 4314],
        [0xc000 | 80, 50],
        [0xc050, 0xc033],
        [0xc037, 0xc000],
        [0xc06c, 0xc032],
        [0xc0f2, 0xc07a],
    ];
    for short in [1, 3, 7, 28] {
        for declared in [0_u16, 1, 2, 3] {
            let mut words = image.to_vec();
            words.extend([[0x8001, 4330], [0x8002, 0x1084], [4700, 0xd6d0]]);
            words.extend(image);
            words.push([0x8004, 1]);
            let mut source = hnb_source(20, &[&words]);
            source.bytes[224..226].copy_from_slice(&declared.to_le_bytes());
            source.short = short;
            let mut visitor = Visitor::default();
            let result = parse(&mut source, &mut visitor);
            if declared == 2 {
                assert_eq!(result.unwrap(), 6);
                assert_eq!(
                    visitor.events[0],
                    (
                        236,
                        NativeRecord::Image {
                            words: [
                                0xd300, 0xd24a, 4314, 0xc050, 50, 0xc050, 0xc033, 0xc037, 0xc000,
                                0xc06c, 0xc032, 0xc0f2, 0xc07a
                            ],
                        }
                    )
                );
                assert!(matches!(visitor.events[3].1, NativeRecord::Glyph { .. }));
                assert_eq!(visitor.events[4].1, visitor.events[0].1);
                assert_eq!(visitor.events[1].0 - visitor.events[0].0, 28);
            } else {
                assert!(matches!(
                    result.unwrap_err(),
                    Error {
                        kind: ErrorKind::Malformed,
                        ..
                    }
                ));
            }
        }
    }
}

#[test]
fn hnb_truncated_image_does_not_consume_the_following_page() {
    for length in 4..28 {
        let mut source = hnb_source(
            20,
            &[
                &[
                    [0x800a, 0xd300],
                    [0; 2],
                    [0; 2],
                    [0; 2],
                    [0; 2],
                    [0; 2],
                    [0; 2],
                ],
                &[[0x8004, 2]],
            ],
        );
        source.bytes[220..224].copy_from_slice(&(length as u32).to_le_bytes());
        source.bytes[224..226].copy_from_slice(&1_u16.to_le_bytes());
        {
            let limits = Limits::default();
            let cancel = Cancel::default();
            let mut reader = Hnc8Reader::open(&mut source, &limits, &cancel).unwrap();
            reader.next_page().unwrap();
            let mut visitor = Visitor::default();
            let error = reader.visit_native_records(&mut visitor).unwrap_err();
            assert!(
                matches!(error.kind, ErrorKind::Truncated { expected: 24, available, .. }
                if available == u64::from(length as u32 - 4))
            );
            assert_eq!(page_image(&error).0, Some(1));
            assert!(visitor.events.is_empty());
        };
    }
}

#[test]
fn hnb_bare_end_tags_stay_inside_each_indexed_page() {
    for width in [12, 20] {
        for short in [1, 3, 28] {
            let mut source = hnb_source(width, &[&[[0x8004, 1]], &[[0x8004, 2]]]);
            let start = 216 + width * 2;
            source.bytes.drain(start + 2..start + 4);
            source.bytes.truncate(start + 4);
            source.bytes[220..224].copy_from_slice(&2u32.to_le_bytes());
            source.bytes[216 + width..220 + width]
                .copy_from_slice(&((start + 2) as u32).to_le_bytes());
            source.bytes[220 + width..224 + width].copy_from_slice(&2u32.to_le_bytes());
            source.short = short;
            {
                let limits = Limits::default();
                let cancel = Cancel::default();
                let mut reader = Hnc8Reader::open(&mut source, &limits, &cancel).unwrap();
                for page in 0..2 {
                    reader.next_page().unwrap().unwrap();
                    let mut visitor = Visitor::default();
                    assert_eq!(reader.visit_native_records(&mut visitor).unwrap(), 1);
                    assert_eq!(
                        visitor.events,
                        [(start as u64 + page * 2, NativeRecord::End { value: None })]
                    );
                }
                assert!(reader.next_page().unwrap().is_none());
            };
        }
    }
    let mut source = hnb_source(12, &[&[[0x8074, 0xb7bd]]]);
    source.bytes.truncate(230);
    source.bytes[220..224].copy_from_slice(&2u32.to_le_bytes());
    let error = parse(&mut source, &mut Visitor::default()).unwrap_err();
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Truncated {
                expected: 4,
                available: 2
            },
            reason: "native record",
            ..
        }
    ));
    let mut c8 = fixture(&[[0x8004, 1]], 0);
    c8.bytes.truncate(102);
    c8.bytes[84..88].copy_from_slice(&2u32.to_le_bytes());
    assert!(parse(&mut c8, &mut Visitor::default()).is_err());
}

#[test]
fn hnb_end_stops_before_opaque_tail_and_next_page_uses_its_index() {
    for width in [12, 20] {
        let first = [
            [0x8001, 4700],
            [0x8002, 0x1084],
            [5200, 0xd6d0],
            [0x8004, 44],
            [5300, 0xcec4],
            [0x8099, 0xffff],
        ];
        let second = [
            [0x8001, 4800],
            [0x8002, 0x1084],
            [5300, 0xcec4],
            [0x8004, 45],
        ];
        let mut source = hnb_source(width, &[&first, &second]);
        let second_at = 216 + width * 2 + first.len() * 4;
        source
            .bytes
            .splice(second_at..second_at, [0xff, 0x01, 0x80]);
        source.bytes[220..224].copy_from_slice(&(first.len() as u32 * 4 + 3).to_le_bytes());
        source.bytes[216 + width..220 + width]
            .copy_from_slice(&((second_at + 3) as u32).to_le_bytes());
        source.short = 1;
        {
            let limits = Limits::default();
            let cancel = Cancel::default();
            let mut reader = Hnc8Reader::open(&mut source, &limits, &cancel).unwrap();
            for (code, value) in [(0xd6d0, 44), (0xcec4, 45)] {
                reader.next_page().unwrap().unwrap();
                let mut visitor = Visitor::default();
                assert_eq!(reader.visit_native_records(&mut visitor).unwrap(), 4);
                assert!(
                    matches!(visitor.events[2].1, NativeRecord::Glyph { code: actual, .. } if actual == code)
                );
                assert_eq!(
                    visitor.events[3].1,
                    NativeRecord::End { value: Some(value) }
                );
            }
            assert!(reader.next_page().unwrap().is_none());
        };
    }
    // C8 retains its independently established strict terminal position.
    let mut c8 = fixture(&[[0x8004, 1], [0x8099, 0xffff]], 0);
    let error = parse(&mut c8, &mut Visitor::default()).unwrap_err();
    assert_eq!(field_of(&error), "native page end");
}

#[test]
fn native_mode_keeps_independently_copied_alphabets_separate() {
    for (first, ascii) in [
        (0xa980, b'A'),
        (0xa99a, b'a'),
        (0xa3c1, b'A'),
        (0xa3e1, b'a'),
    ] {
        for index in 0..26_u8 {
            assert_eq!(
                decode_native_character_for_mode(0, first + u16::from(index)),
                Some(char::from(ascii + index))
            );
        }
    }
    for (code, character) in [(0xa0c1, 'A'), (0xa3c1, 'Ａ'), (0xd6d0, '中')] {
        assert_eq!(decode_native_character_for_mode(2, code), Some(character));
    }
    // Do not import mode-2 aliases, adjacent symbols or unverified classes.
    for code in [
        0xa0c1, 0xa97f, 0xa9b4, 0xa3c0, 0xa3dc, 0xa3e0, 0xa3fb, 0x8140, 0xffff,
    ] {
        assert_eq!(decode_native_character_for_mode(0, code), None);
    }
    for mode in [1, 3, u32::MAX] {
        assert_eq!(decode_native_character_for_mode(mode, 0xa980), None);
    }
    assert_eq!(decode_native_character_for_mode(2, 0xffff), None);
}

#[test]
fn mode0_required_symbols_and_han_match_original_copy_controls() {
    let groups: &[(&[u16], &str)] = &[
        (
            &[
                0xb0a1, 0xb5d8, 0xccec, 0xd6d0, 0xcec4, 0xf7fe, 0xa1a1, 0xa1a2, 0xa1a3, 0xa1aa,
                0xa1ae, 0xa1af, 0xa1b0,
            ],
            "啊地天中文齄 、。—‘’“",
        ),
        (
            &[
                0xa1b1, 0xa3a7, 0xa3a8, 0xa3a9, 0xa3ab, 0xa3ac, 0xa3ad, 0xa3ae, 0xa3af, 0xa3b0,
                0xa3b1, 0xa3b2, 0xa3b3,
            ],
            "”’（）＋，－．／0123",
        ),
        (
            &[
                0xa3b4, 0xa3b5, 0xa3b6, 0xa3b7, 0xa3b8, 0xa3b9, 0xa3ba, 0xa3bb, 0xa3bf, 0xa3db,
                0xa3dd, 0xaab1, 0xaab2,
            ],
            "456789：；？［］.-",
        ),
    ];
    for &(codes, expected) in groups {
        let actual: Option<String> = codes
            .iter()
            .map(|&code| decode_native_character_for_mode(0, code))
            .collect();
        assert_eq!(actual.as_deref(), Some(expected));
    }
    // Classic Han rows exclude their holes, extensions and unassigned cells.
    for code in [
        0xb0a0, 0xb0ff, 0xb100, 0xd7fa, 0xf7ff, 0xf8a1, 0xa0c1, 0xaab3,
    ] {
        assert_eq!(decode_native_character_for_mode(0, code), None);
    }
    assert_eq!(decode_native_character_for_mode(0, 0x9ff5), Some('／'));
    assert_eq!(decode_native_character_for_mode(2, 0x9ff5), Some('燉'));
    assert_eq!(decode_native_character_for_mode(2, 0xaab1), Some('∙'));
    assert_eq!(decode_native_character_for_mode(2, 0xa3a7), Some('＇'));
    assert_eq!(decode_native_character_for_mode(2, 0xa3b0), Some('０'));
}

#[test]
fn c8_control_9002_requires_its_complete_value_word() {
    for length in 1..4u32 {
        let mut source = fixture(&[[0x9002, 0], [0x8004, 1]], 0);
        source.bytes[84..88].copy_from_slice(&length.to_le_bytes());
        source.bytes[96..100].copy_from_slice(&(100 + length).to_le_bytes());
        source.bytes.truncate(100 + length as usize);
        source.short = 1;
        let mut visitor = Visitor::default();
        assert!(parse(&mut source, &mut visitor).is_err());
        assert!(visitor.events.is_empty());
    }
}

#[test]
fn c8_radical_is_atomic_and_preserves_following_glyph_context() {
    for (short, style) in [1, 3, 7, 28].into_iter().flat_map(|short| {
        [0, 1, 0xa3b1, 0xa3b2, 0xa3e6, 0xa0c1, 0x8004, 0xffff].map(|style| (short, style))
    }) {
        for points in [
            [[0xd2c6, 4364], [0xc08f, 125]],
            [[4806, 4364], [143, 125]],
            [[0x8004, 17], [23, 0x8001]],
        ] {
            let mut source = fixture(
                &[
                    [0x8001, 47],
                    [0x8002, 0x1021],
                    [0x8090, style],
                    points[0],
                    points[1],
                    [31, 0xd6d0],
                    [0x8004, 1],
                ],
                0,
            );
            source.short = short;
            let mut visitor = Visitor::default();
            assert_eq!(parse(&mut source, &mut visitor).unwrap(), 5);
            assert_eq!(
                visitor.events[2],
                (
                    108,
                    NativeRecord::Drawing {
                        tag: 0x8090,
                        style,
                        points,
                    }
                )
            );
            assert_eq!(
                visitor.events[3],
                (
                    120,
                    NativeRecord::Glyph {
                        x: 31,
                        y: 47,
                        style: 0x1021,
                        code: 0xd6d0,
                    }
                )
            );
            assert_eq!(
                visitor.events[4],
                (124, NativeRecord::End { value: Some(1) })
            );
            assert!(source.max_request <= 28);
        }
    }
    for (tag, value) in [(0x808f, 0xa3e6), (0x8091, 0xa3e6), (0x8006, 0), (0x8010, 3)] {
        let mut source = fixture(&[[tag, value], [1, 2], [3, 4]], 0);
        let mut visitor = Visitor::default();
        assert!(parse(&mut source, &mut visitor).is_err());
        assert!(visitor.events.is_empty());
    }
}

#[test]
fn c8_80d5_preserves_strict_indexed_end_boundaries() {
    for value in [10, 11] {
        let mut source = fixture(&[[0x80d5, 0], [0x8004, value], [0x8099, 0]], 0);
        source.bytes[84..88].copy_from_slice(&8_u32.to_le_bytes());
        source.short = 1;
        let mut visitor = Visitor::default();
        assert_eq!(parse(&mut source, &mut visitor).unwrap(), 2);
        assert_eq!(
            visitor.events[1],
            (104, NativeRecord::End { value: Some(value) })
        );
        let mut source = fixture(&[[0x80d5, 0], [0x8004, value], [0x8099, 0]], 0);
        assert!(parse(&mut source, &mut Visitor::default()).is_err());
    }
    for length in 1..4_u32 {
        let mut source = fixture(&[[0x80d5, 0], [0x8004, 1]], 0);
        source.bytes[84..88].copy_from_slice(&length.to_le_bytes());
        source.short = 1;
        let mut visitor = Visitor::default();
        assert!(parse(&mut source, &mut visitor).is_err());
        assert!(visitor.events.is_empty());
    }
    for value in [1, 0xffff] {
        let mut source = fixture(&[[0x80d5, value], [0x8004, 1]], 0);
        assert!(parse(&mut source, &mut Visitor::default()).is_err());
    }
}

#[test]
fn c8_80d3_requires_a_complete_verified_value() {
    for length in 1..4 {
        let mut source = fixture(&[[0x80d3, 1]], 0);
        source.bytes[84..88].copy_from_slice(&(length as u32).to_le_bytes());
        let mut visitor = Visitor::default();
        assert!(parse(&mut source, &mut visitor).is_err());
        assert!(visitor.events.is_empty());
    }
    for value in [3, 0xffff] {
        let mut source = fixture(&[[0x80d3, value], [0x8004, 1]], 0);
        let mut visitor = Visitor::default();
        assert!(parse(&mut source, &mut visitor).is_err());
        assert!(visitor.events.is_empty());
    }
}

#[test]
fn hnb_opaque_metadata_never_reads_a_truncated_value_as_the_next_page() {
    for width in [12, 20] {
        for tag in 0x8072..=0x8074 {
            for length in [2_u32, 3] {
                let mut source = hnb_source(width, &[&[[tag, 0x8004]], &[[0x8004, 1]]]);
                source.bytes[220..224].copy_from_slice(&length.to_le_bytes());
                let limits = Limits::default();
                let cancel = Cancel::default();
                let mut reader = Hnc8Reader::open(&mut source, &limits, &cancel).unwrap();
                reader.next_page().unwrap();
                let mut visitor = Visitor::default();
                let error = reader.visit_native_records(&mut visitor).unwrap_err();
                assert_eq!(page_image(&error).0, Some(1));
                assert!(visitor.events.is_empty());
                reader.next_page().unwrap();
                assert_eq!(reader.visit_native_records(&mut visitor).unwrap(), 1);
            }
        }
    }
}
