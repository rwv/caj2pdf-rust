// SPDX-License-Identifier: MIT

use super::*;

fn text(words: &[[u16; 2]], mode: u8) -> String {
    let (result, pdf, finished) = convert(words, 0, &[], roles(), mode);
    result.unwrap();
    assert!(finished);
    crate::test_support::pdf_text(&pdf)
}

#[test]
fn measured_nju_controls_preserve_active_resources_and_geometry() {
    for state in [0, 4] {
        for mode in [0, 1] {
            for skew in [0x2800, 0x2815] {
                let base = [
                    [0x8001, 4394],
                    [0x8002, 0x1084],
                    [0x801d, state],
                    [0x80ce, mode],
                    [0x8024, skew],
                    [0x81ff, 1],
                    [0, 200],
                    [4902, 0xd6d0],
                    [5072, 0xa0c1],
                    [0x8004, 1],
                ];
                let expected = text(&base, 0);
                for control in [
                    [0x8021, 0x2009],
                    [0x8067, 0],
                    [0x8067, 4],
                    [0x8067, 7],
                    [0x8067, 11],
                ] {
                    let mut changed = base.to_vec();
                    changed.insert(8, control);
                    changed.insert(7, control);
                    assert_eq!(text(&changed, 0), expected);
                }
            }
        }
    }
    for control in [
        [0x8021, 0x2008],
        [0x8021, 0x200a],
        [0x8067, 1],
        [0x8067, 10],
        [0x8067, 12],
        [0x8024, 0x2814],
    ] {
        let mut words = ordinary();
        words.extend([control, [4902, 0xd6d0], [0x8004, 1]]);
        assert!(convert(&words, 0, &[], roles(), 0).0.is_err());
    }
}

#[test]
fn nju_style_flags_match_only_the_observed_c8_profiles() {
    for (style, reference) in [(0x6084, 0x1084), (0x0508, 0x1108), (0x64c6, 0x10c6)] {
        let mut words = [
            [0x8001, 4394],
            [0x8002, style],
            [4902, 0xd6d0],
            [5072, 0xa0c1],
            [0x8004, 1],
        ];
        let actual = text(&words, 0);
        words[1][1] = reference;
        assert_eq!(actual, text(&words, 0));
    }
    for style in [0x6085, 0x64c5, 0x0509] {
        let words = [[0x8001, 4394], [0x8002, style], [4902, 0xd6d0], [0x8004, 1]];
        assert!(convert(&words, 0, &[], roles(), 0).0.is_err());
    }
    for style in [0x6084, 0x64c6, 0x096b] {
        let words = [[0x8001, 4394], [0x8002, style], [4902, 0xd6d0], [0x8004, 1]];
        assert!(convert(&words, 0, &[], roles(), 12).0.is_err());
    }
}

#[test]
fn tiny_c8_axes_preserve_following_style_reset_and_reject_unmeasured_classes() {
    let words = [
        [0x8001, 4394],
        [0x8002, 0x1084],
        [0x8070, 1],
        [0x8071, 1],
        [4902, 0xd6d0],
        [0x8002, 0x1084],
        [5072, 0xa0c1],
        [0x8004, 1],
    ];
    let actual = text(&words, 0);
    assert_eq!(actual.matches(" Tj ET").count(), 2);
    let first = actual.lines().find(|line| line.contains(" Tm ")).unwrap();
    let matrix = first.split(" Tm ").next().unwrap();
    let values: Vec<f64> = matrix
        .split_whitespace()
        .map(|word| word.parse().unwrap())
        .collect();
    assert!((values[0] - 75.0 / 301.0).abs() < 0.000001);
    assert!((values[3] - 75.0 / 301.0).abs() < 0.000001);
    for pair in [[0x8071, 4], [0x801d, 0], [0x8071, 0]] {
        let mut changed = words;
        changed[3] = pair;
        assert!(convert(&changed, 0, &[], roles(), 0).0.is_err());
    }
    let mut latin = words;
    latin[4][1] = 0xa0c1;
    assert!(convert(&latin, 0, &[], roles(), 0).0.is_err());
    assert!(convert(&words, 0, &[], roles(), 12).0.is_err());
}

#[test]
fn nju_segments_preserve_order_without_inheriting_coordinate_flags() {
    for (tag, style) in [(0x8006, 0xa387), (0x8006, 0xa38d), (0x8008, 0xa380)] {
        for points in [
            [[4702, 4500], [5172, 4500]],
            [[4902, 4380], [4902, 4590]],
            [[4702, 4380], [5172, 4590]],
            [[5172, 4590], [4702, 4380]],
        ] {
            let mut words = ordinary();
            let at = words.len();
            words.extend([
                [tag, style],
                points[0],
                points[1],
                [5072, 0xa0c1],
                [0x8004, 1],
            ]);
            let actual = text(&words, 0);
            let mut reference = words.clone();
            reference[at] = [0x8006, 0xa381];
            assert_eq!(actual, text(&reference, 0));
            for coordinate in 0..4 {
                let mut flagged = words.clone();
                flagged[at + 1 + coordinate / 2][coordinate % 2] |= 0xc000;
                assert!(convert(&flagged, 0, &[], roles(), 0).0.is_err());
            }
            assert!(convert(&words, 0, &[], roles(), 12).0.is_err());
        }
    }
    for record in [
        [0x8006, 0xa386],
        [0x8006, 0xa38c],
        [0x8008, 0xa381],
        [0x8009, 0xa380],
    ] {
        let words = [record, [4702, 4380], [5172, 4590], [0x8004, 1]];
        assert!(convert(&words, 0, &[], roles(), 0).0.is_err());
    }
}

#[test]
fn c8_field_four_tortoise_brackets_match_discriminating_opener_controls() {
    for state in [0, 4] {
        for code in [0xa1b2, 0xa1b3] {
            let words = [
                [0x8001, 4374],
                [0x8002, 0x1084],
                [0x801d, state],
                [4672, code],
                [0x8004, 1],
            ];
            let actual = text(&words, 11);
            let reference = text(
                &[
                    [0x8001, 4380],
                    [0x8002, 0x1084],
                    [0x801d, state],
                    [4671, 0xa3a8],
                    [0x8004, 1],
                ],
                11,
            );
            let draw = |s: &str| {
                s.lines()
                    .find(|line| line.contains(" Tm "))
                    .unwrap()
                    .split(" Tm ")
                    .next()
                    .unwrap()
                    .to_owned()
            };
            assert_eq!(draw(&actual), draw(&reference));
            let mut unmeasured = words;
            unmeasured[1][1] = 0x1063;
            assert!(convert(&unmeasured, 0, &[], roles(), 11).0.is_err());
        }
    }
}
