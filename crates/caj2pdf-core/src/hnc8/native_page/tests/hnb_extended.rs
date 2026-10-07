// SPDX-License-Identifier: MIT

use super::*;

fn words(style: u16, code: u16) -> Vec<[u16; 2]> {
    vec![[0x8001, 4350], [0x8002, style], [4682, code], [0x8004, 1]]
}

fn text(words: &[[u16; 2]], mode: u8, roles: C8PageFonts) -> String {
    let (result, pdf, finished) = convert(words, 0, &[], roles, mode);
    result.unwrap();
    assert!(finished);
    crate::test_support::pdf_text(&pdf)
}

#[test]
fn metadata_words_are_atomic_and_do_not_change_following_glyph_state() {
    let mut base = ordinary();
    base.push([0x8004, 1]);
    let expected = text(&base, 12, roles());
    for tag in 0x8072..=0x8074 {
        for value in [0, 278, 0xa3ac, 0x8001, 0x8004, 0xffff] {
            let mut changed = base.clone();
            changed.insert(2, [tag, value]);
            assert_eq!(text(&changed, 12, roles()), expected);
        }
    }
    let mut changed = base.clone();
    changed.insert(2, [0x8067, 18]);
    assert_eq!(text(&changed, 12, roles()), expected);
    for control in [[0x8067, 19], [0x8075, 0], [0x801d, 18]] {
        let mut changed = base.clone();
        changed.insert(2, control);
        assert!(convert(&changed, 0, &[], roles(), 12).0.is_err());
    }
}

#[test]
fn title_zero_and_small_brackets_match_independent_c8_controls() {
    for (style, code) in [
        (0x0929, 0xd6d0),
        (0x1129, 0xa0c1),
        (0x1000, 0xa0c1),
        (0x1021, 0xa3db),
        (0x1021, 0xa3dd),
    ] {
        let record = words(style, code);
        let hnb = text(&record, if code == 0xa0c1 { 12 } else { 13 }, roles());
        let c8 = text(&record, if code == 0xa0c1 { 0 } else { 11 }, roles());
        assert_eq!(hnb, c8);
    }
    for style in [0x1001, 0x1020, 0x116b] {
        assert!(
            convert(&words(style, 0xa0c1), 0, &[], roles(), 12)
                .0
                .is_err()
        );
    }
}

#[test]
fn hnb_symbols_preserve_resource_state_except_measured_resets() {
    let mut fonts = roles();
    fonts.latin_state3 = Some(0);
    for code in [
        0xa1b4, 0xa1b5, 0xa1c0, 0xa1c1, 0xa1c3, 0xa1d6, 0xa1dd, 0xa1e4, 0xa2f2, 0xa6b8, 0xa6c4,
        0xa6cc, 0xa6d2,
    ] {
        let mut record = words(0x1084, code);
        record.insert(2, [0x801d, 3]);
        record.insert(4, [4800, 0xa0c1]);
        let result = text(&record, 13, fonts);
        let selected = if matches!(code, 0xa1d6 | 0xa1dd) {
            1
        } else {
            0
        };
        assert_eq!(result.matches(&format!("BT /F{selected} 1 Tf")).count(), 2);
        let codepoint = decode_native_character(code).unwrap() as u32;
        assert!(result.contains(&format!("<{codepoint:04X}> Tj")));
    }
}

#[test]
fn beta_style_five_matches_the_measured_downward_shift() {
    for state in [0, 3] {
        let mut fonts = roles();
        fonts.latin_state3 = Some(0);
        let mut beta = words(0x10a5, 0xa6c2);
        beta.insert(2, [0x801d, state]);
        let mut reference = beta.clone();
        reference[0][1] += 25;
        reference[3][1] = 0xa3ac;
        let matrix = |s: String| {
            s.lines()
                .find_map(|line| line.split_once(" Tm "))
                .unwrap()
                .0
                .to_owned()
        };
        assert_eq!(
            matrix(text(&beta, 13, fonts)),
            matrix(text(&reference, 13, fonts))
        );
    }
    for style in [0x1084, 0x10c6] {
        assert!(
            convert(&words(style, 0xa6c2), 0, &[], roles(), 13)
                .0
                .is_err()
        );
    }
}

#[test]
fn private_glyph_keeps_its_code_and_reports_no_guessed_unicode_mapping() {
    assert_eq!(decode_native_character(0xa661), None);
    for (codepoint, shown, actual) in [(0xe6c7, "E6C7", false), (0x0403, "0403", true)] {
        let fonts = vec![labelled_font([65, codepoint]); 3];
        let (result, pdf, finished) =
            convert_with_fonts(&words(0x1084, 0xa661), 0, &[], roles(), 12, fonts);
        result.unwrap();
        assert!(finished);
        let text = crate::test_support::pdf_text(&pdf);
        assert!(text.contains(&format!("<{shown}> Tj")));
        assert_eq!(text.contains("/ActualText <FEFFE6C7>"), actual);
    }
    // Neither a missing display alias nor a neighboring private code is dropped.
    for code in [0xa661, 0xa662] {
        assert!(
            convert(&words(0x1084, code), 0, &[], roles(), 12)
                .0
                .is_err()
        );
    }
}
