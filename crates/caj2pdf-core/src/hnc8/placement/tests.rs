// SPDX-License-Identifier: MIT

//! Original calculations and caller-input checks; no document bytes or tables.

use super::*;

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 0.000_000_000_001,
        "{actual} differs from {expected}"
    );
}

fn page() -> EmpiricalPageGeometry {
    empirical_page_from_pixels(100, 250, [-3.25, 1.125]).unwrap()
}

#[test]
fn documented_factors_and_comparison_precision_are_explicit() {
    assert_eq!(EMPIRICAL_COORDINATE_POINTS_PER_UNIT, 240.0 / 2473.0);
    assert_eq!(EMPIRICAL_PIXEL_POINTS, 0.24);
    assert_eq!(EMPIRICAL_PLACEMENT_TOLERANCE_POINTS, 0.00005);
}

#[test]
fn image_dimensions_round_the_exact_point_ratio_once() {
    let page = empirical_page_from_pixels(2071, 153, [0.0; 2]).unwrap();
    // Predetermined decimal physical dimensions catch the extra rounding
    // from multiplying the binary approximation of 0.24. That older result
    // serializes as 497.03999999999996 and changes full-page edge pixels.
    assert_eq!(page.size.width_points, 497.04);
    assert_eq!(page.size.height_points, 36.72);
    assert_ne!(page.size.width_points, 2071.0 * EMPIRICAL_PIXEL_POINTS);
    let transform = empirical_image_transform(page, 2071, 153, point(0, 0)).unwrap();
    assert_eq!(transform, [497.04, 0.0, 0.0, -36.72, 0.0, 36.72]);
    // The whole public unsigned dimension range keeps the numerator exact.
    let largest = empirical_page_from_pixels(u32::MAX, u32::MAX, [0.0; 2]).unwrap();
    assert_eq!(largest.size.width_points, 1_030_792_150.8);
    assert_eq!(largest.size.height_points, 1_030_792_150.8);
}

#[test]
fn fractional_negative_pdf_origin_and_off_page_translation_are_preserved() {
    let page = page();
    assert_eq!(
        page.size,
        PageSpec {
            width_points: 24.0,
            height_points: 60.0
        }
    );
    assert_eq!(page.media_box().unwrap(), [-3.25, 1.125, 20.75, 61.125]);
    let ctm = empirical_image_transform(page, 3, 7, point(2473, 2473)).unwrap();
    close(ctm[0], 0.72);
    assert_eq!(&ctm[1..3], &[0.0, 0.0]);
    close(ctm[3], -1.68);
    assert_eq!(&ctm[4..], &[236.75, -178.875]);
    // The image is outside both page axes. Its source origin is not clipped.
    assert!(ctm[4] > page.media_box().unwrap()[2]);
    assert!(ctm[5] < page.media_box().unwrap()[1]);
}

#[test]
fn zero_origin_is_top_left_and_positive_y_moves_downward() {
    let page = empirical_page_from_pixels(50, 100, [0.0, 0.0]).unwrap();
    let top = empirical_image_transform(page, 50, 100, point(0, 0)).unwrap();
    assert_eq!(top, [12.0, 0.0, 0.0, -24.0, 0.0, 24.0]);
    let moved = empirical_image_transform(page, 50, 100, point(1, 1)).unwrap();
    close(moved[4], 240.0 / 2473.0);
    close(moved[5], 24.0 - 240.0 / 2473.0);
    assert_eq!(&top[..4], &moved[..4]);
    // Coordinates remain unrounded; four-decimal precision is a comparison
    // policy, rather than a loss of precision during field evaluation.
    assert_ne!(moved[4], (moved[4] * 10_000.0).round() / 10_000.0);
}

#[test]
fn type0_page_uses_padded_bits_and_checks_the_public_info() {
    let info = Type0Info {
        width: 33,
        height: 100,
        dib_stride: 8,
        visible_bytes: 5,
    };
    let padded = empirical_page_from_type0(info, [0.5, -1.25]).unwrap();
    close(padded.size.width_points, 64.0 * 0.24);
    assert_eq!(padded.size.height_points, 24.0);
    assert_ne!(padded.size.width_points, 33.0 * 0.24);
    let visible = empirical_page_from_pixels(33, 100, [0.5, -1.25]).unwrap();
    assert_ne!(
        padded.media_box().unwrap()[2],
        visible.media_box().unwrap()[2]
    );
    let ctm = empirical_image_transform(padded, 64, 100, point(0, 0)).unwrap();
    assert_eq!(ctm[0], padded.size.width_points);
    assert_eq!(ctm[5], 22.75);
    let aligned = empirical_page_from_type0(
        Type0Info {
            width: 32,
            height: 1,
            dib_stride: 4,
            visible_bytes: 4,
        },
        [0.0; 2],
    )
    .unwrap();
    assert_eq!(aligned.size.width_points, 32.0 * 0.24);
    for invalid in [
        Type0Info { width: 0, ..info },
        Type0Info { height: 0, ..info },
        Type0Info {
            dib_stride: 7,
            ..info
        },
        Type0Info {
            dib_stride: usize::MAX,
            ..info
        },
        Type0Info {
            visible_bytes: 4,
            ..info
        },
    ] {
        assert!(matches!(
            empirical_page_from_type0(invalid, [0.0; 2]),
            Err(Error::InvalidInput { .. })
        ));
    }
}

#[test]
fn type0_padded_width_conversion_is_checked_at_the_u32_boundary() {
    let width = u32::MAX - 7;
    let stride = u64::from(width).div_ceil(32) * 4;
    let visible = u64::from(width).div_ceil(8);
    let info = Type0Info {
        width,
        height: 1,
        dib_stride: stride as usize,
        visible_bytes: visible as usize,
    };
    assert!(matches!(
        empirical_page_from_type0(info, [0.0; 2]),
        Err(Error::InvalidInput {
            reason: "empirical type-0 display width exceeds the supported pixel range"
        })
    ));
}

#[test]
fn source_order_repeats_and_pixel_dimensions_do_not_change_coordinate_roles() {
    let page = page();
    let coordinates = [point(2, 4), point(7, 3), point(2, 4)];
    let transforms =
        coordinates.map(|coordinate| empirical_image_transform(page, 3, 7, coordinate).unwrap());
    assert_eq!(transforms[0], transforms[2]);
    assert!(transforms[1][4] > transforms[0][4]);
    assert!(transforms[1][5] > transforms[0][5]);
    let resized = empirical_image_transform(page, 11, 2, coordinates[0]).unwrap();
    // Compare returned binary64 values, not x87 extended intermediates.
    // Independently rounded exact coordinates are -13/4 + 480/2473 and
    // 489/8 - 960/2473. Pixel dimensions must not change either translation.
    for transform in [transforms[0], transforms[2], resized] {
        assert_eq!(transform[4].to_bits(), 0xc008727dabbc819b);
        assert_eq!(transform[5].to_bits(), 0x404e5e4fb5779033);
    }
    close(resized[0], 2.64);
    close(resized[3], -0.48);
}

#[test]
fn positive_pixel_range_has_no_integer_product_or_float_overflow() {
    let page = empirical_page_from_pixels(u32::MAX, u32::MAX, [0.0; 2]).unwrap();
    assert_eq!(page.size.width_points, f64::from(u32::MAX) * 0.24);
    let ctm = empirical_image_transform(page, u32::MAX, u32::MAX, point(1, 2)).unwrap();
    assert!(ctm.into_iter().all(f64::is_finite));
    for (width, height) in [(0, 1), (1, 0), (0, 0)] {
        assert!(matches!(
            empirical_page_from_pixels(width, height, [0.0; 2]),
            Err(Error::InvalidInput { .. })
        ));
        assert!(matches!(
            empirical_image_transform(page, width, height, point(0, 0)),
            Err(Error::InvalidInput { .. })
        ));
    }
}

#[test]
fn forged_page_origins_sizes_overflow_and_precision_collapse_are_rejected() {
    let valid = page();
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for origin in [[value, 0.0], [0.0, value]] {
            assert!(matches!(
                empirical_page_from_pixels(1, 1, origin),
                Err(Error::InvalidInput { .. })
            ));
        }
    }
    for value in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for size in [
            PageSpec {
                width_points: value,
                ..valid.size
            },
            PageSpec {
                height_points: value,
                ..valid.size
            },
        ] {
            let bad = EmpiricalPageGeometry { size, ..valid };
            assert!(matches!(bad.media_box(), Err(Error::InvalidInput { .. })));
            assert!(matches!(
                empirical_image_transform(bad, 1, 1, point(0, 0)),
                Err(Error::InvalidInput { .. })
            ));
        }
    }
    for (origin_points, size) in [
        (
            [f64::MAX, 0.0],
            PageSpec {
                width_points: f64::MAX,
                height_points: 1.0,
            },
        ),
        (
            [0.0, f64::MAX],
            PageSpec {
                width_points: 1.0,
                height_points: f64::MAX,
            },
        ),
        (
            [f64::MAX, 0.0],
            PageSpec {
                width_points: 1.0,
                height_points: 1.0,
            },
        ),
        (
            [0.0, f64::MAX],
            PageSpec {
                width_points: 1.0,
                height_points: 1.0,
            },
        ),
    ] {
        let bad = EmpiricalPageGeometry {
            origin_points,
            size,
        };
        assert!(matches!(bad.media_box(), Err(Error::InvalidInput { .. })));
    }
}

#[test]
fn noncollapsed_extreme_origins_must_preserve_dimension_and_coordinate_precision() {
    for page in [
        EmpiricalPageGeometry {
            origin_points: [1.0e18, 0.0],
            size: PageSpec {
                width_points: 1000.0,
                height_points: 24.0,
            },
        },
        EmpiricalPageGeometry {
            origin_points: [0.0, 1.0e18],
            size: PageSpec {
                width_points: 24.0,
                height_points: 1000.0,
            },
        },
    ] {
        assert!(matches!(
            page.media_box(),
            Err(Error::InvalidInput {
                reason: "empirical PDF origin cannot preserve the page dimension precision"
            })
        ));
    }
    for (page, coordinate) in [
        (
            EmpiricalPageGeometry {
                origin_points: [1.0e18, 0.0],
                size: PageSpec {
                    width_points: 1024.0,
                    height_points: 24.0,
                },
            },
            point(1, 0),
        ),
        (
            EmpiricalPageGeometry {
                origin_points: [0.0, 1.0e18],
                size: PageSpec {
                    width_points: 24.0,
                    height_points: 1024.0,
                },
            },
            point(0, 1),
        ),
    ] {
        assert!(page.media_box().is_ok());
        assert!(matches!(
            empirical_image_transform(page, 1, 1, coordinate),
            Err(Error::InvalidInput {
                reason: "empirical PDF origin cannot preserve the selected coordinate precision"
            })
        ));
        // An exact zero offset still has its intended meaning at that origin.
        assert!(empirical_image_transform(page, 1, 1, point(0, 0)).is_ok());
    }
}

#[test]
fn page_and_offset_rounding_must_not_accumulate_beyond_translation_tolerance() {
    let page = empirical_page_from_pixels(1, 9, [0.0, 274_877_906_944.0]).unwrap();
    let [_, bottom, _, top] = page.media_box().unwrap();
    let offset = 14.0 * EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
    let rounded_y = top - offset;
    assert!(
        ((top - bottom) - page.size.height_points).abs() <= EMPIRICAL_PLACEMENT_TOLERANCE_POINTS
    );
    assert!(((top - rounded_y) - offset).abs() <= EMPIRICAL_PLACEMENT_TOLERANCE_POINTS);
    assert!(
        ((rounded_y - bottom) - (page.size.height_points - offset)).abs()
            > EMPIRICAL_PLACEMENT_TOLERANCE_POINTS
    );
    match empirical_image_transform(page, 1, 1, point(0, 14)) {
        // x87 may retain enough intermediate precision to avoid the two
        // accumulated binary64 rounding errors demonstrated above. Accept
        // success only for the one binary64 y within the existing tolerance
        // of the exact result: 2^38 + 54/25 - 3360/2473. Its neighbors miss by
        // -0.0000567930524666 and +0.0000652772600334 points respectively.
        Ok(ctm) => assert_eq!(ctm[5].to_bits(), 0x4250_0000_0000_3349),
        Err(error) => assert!(matches!(
            error,
            Error::InvalidInput {
                reason: "empirical PDF origin cannot preserve the selected coordinate precision"
            }
        )),
    }
}

#[test]
fn full_raw_word_domain_is_unsigned_and_preserves_off_page_positions() {
    let page = page();
    for coordinate in [
        point(0, 0),
        point(32767, 32767),
        point(32768, 0),
        point(0, 32768),
        point(u16::MAX, u16::MAX),
    ] {
        let ctm = empirical_image_transform(page, 1, 1, coordinate).unwrap();
        close(ctm[4], -3.25 + f64::from(coordinate.x) * 240.0 / 2473.0);
        close(ctm[5], 61.125 - f64::from(coordinate.y) * 240.0 / 2473.0);
        if coordinate.x >= 32768 {
            assert!(ctm[4] > page.media_box().unwrap()[2]);
        }
        if coordinate.y >= 32768 {
            assert!(ctm[5] < page.media_box().unwrap()[1]);
        }
    }
}

#[test]
fn type0_display_width_preserves_partial_bits_when_there_are_no_padding_bytes() {
    for width in 1_u32..=96 {
        let info = Type0Info {
            width,
            height: 3,
            dib_stride: width.div_ceil(32) as usize * 4,
            visible_bytes: width.div_ceil(8) as usize,
        };
        let expected = if width.div_ceil(8) % 4 == 0 {
            width
        } else {
            width.div_ceil(32) * 32
        };
        assert_eq!(type0_display_width(info), u64::from(expected));
        assert_eq!(
            empirical_page_from_type0(info, [0.0; 2])
                .unwrap()
                .size
                .width_points,
            f64::from(expected) * 72.0 / 300.0
        );
    }
}

fn point(x: u16, y: u16) -> RawTextCoordinate {
    RawTextCoordinate {
        x,
        y,
        ..Default::default()
    }
}

#[test]
fn c8_glyph_sizes_predict_independent_original_controls() {
    let origin = [4652, 4274];
    for (field, expected_pixels) in [
        (2, 318),
        (3, 352),
        (4, 397),
        (5, 477),
        (6, 545),
        (7, 636),
        (8, 715),
    ] {
        let style = 0x1000 | field << 5 | field;
        let cjk =
            empirical_c8_glyph_transform(page(), origin, [4672, 4294], style, C8GlyphClass::Cjk)
                .unwrap();
        let latin =
            empirical_c8_glyph_transform(page(), origin, [4672, 4294], style, C8GlyphClass::Latin)
                .unwrap();
        // Original square fonts, 96 DPI, displayed 3420%. Field 7 was held out
        // from model calibration. These are measurements, not size-table copies.
        assert_eq!(
            (cjk[3] * 96.0 / 72.0 * 34.2).floor(),
            f64::from(expected_pixels)
        );
        assert_eq!(cjk[..4], latin[..4]);
        assert!(latin[4] > cjk[4]);
        assert!(latin[5] < cjk[5]);
        let shifted = empirical_c8_glyph_transform(
            page(),
            [4672, 4294],
            [4692, 4314],
            style,
            C8GlyphClass::Cjk,
        )
        .unwrap();
        assert_eq!(shifted, cjk);
    }
    // Independent horizontal/vertical fields preserve each axis's size.
    let matrix =
        empirical_c8_glyph_transform(page(), origin, [4672, 4294], 0x1065, C8GlyphClass::Cjk)
            .unwrap();
    assert!(matrix[0] < matrix[3]);
    close(matrix[0], 7.724252491694352);
    close(matrix[3], 10.465116279069768);
}

#[test]
fn c8_glyph_origins_are_signed_and_unknown_styles_are_errors() {
    let origin = [4652, 4274];
    let a = empirical_c8_glyph_transform(page(), origin, [4652, 4274], 0x1084, C8GlyphClass::Cjk)
        .unwrap();
    let b = empirical_c8_glyph_transform(page(), origin, [4632, 4254], 0x1084, C8GlyphClass::Cjk)
        .unwrap();
    close(b[4] - a[4], -20.0 * EMPIRICAL_COORDINATE_POINTS_PER_UNIT);
    close(b[5] - a[5], 20.0 * EMPIRICAL_COORDINATE_POINTS_PER_UNIT);
    for style in [
        0x0485, 0x9c85, 0x1485, 0x9084, 0x1004, 0x1080, 0x1024, 0x1089,
    ] {
        assert!(
            empirical_c8_glyph_transform(page(), origin, origin, style, C8GlyphClass::Cjk).is_err()
        );
    }
    let mut invalid = page();
    invalid.size.height_points = f64::NAN;
    assert!(
        empirical_c8_glyph_transform(invalid, origin, origin, 0x1084, C8GlyphClass::Cjk).is_err()
    );
}

#[test]
fn c8_horizontal_decoration_preserves_partial_marks_and_independent_axes() {
    let page = source_page_geometry([600, 600]).unwrap();
    let origin = [4652, 4274];
    for (length, count) in [(10, 1), (50, 1), (89, 1), (91, 2), (180, 3), (430, 5)] {
        let d = empirical_c8_horizontal_decoration(
            page,
            origin,
            [[4712, 4334], [4712 + length, 4334]],
            0x1084,
        )
        .unwrap();
        // Counts independently observed at 486% and 993%, including partial tails.
        assert_eq!(d.glyph_count, count);
        close(d.first_glyph[4], 5.822887181560857);
        close(d.first_glyph[5], 48.045519517768646);
        close(
            d.clip[2],
            f64::from(length) * EMPIRICAL_COORDINATE_POINTS_PER_UNIT,
        );
        assert_eq!(d.clip[1], 0.0);
        assert_eq!(d.clip[3], page.size.height_points);
    }
    let a = empirical_c8_horizontal_decoration(page, origin, [[4712, 4334], [5142, 4334]], 0x1048)
        .unwrap();
    let b = empirical_c8_horizontal_decoration(page, origin, [[4712, 4334], [5142, 4334]], 0x1102)
        .unwrap();
    assert!(a.first_glyph[0] < b.first_glyph[0]);
    assert!(a.first_glyph[3] > b.first_glyph[3]);
    assert!(a.glyph_count > b.glyph_count);
    assert_eq!(a.clip, b.clip);
    let shifted = empirical_c8_horizontal_decoration(
        page,
        [4672, 4294],
        [[4732, 4354], [5162, 4354]],
        0x1048,
    )
    .unwrap();
    assert_eq!(a, shifted);
    let maximum =
        empirical_c8_horizontal_decoration(page, origin, [[0, 0], [u16::MAX, 0]], 0x1042).unwrap();
    assert!(maximum.first_glyph[4] < 0.0);
    assert!(maximum.glyph_count < 1000);
}

#[test]
fn c8_horizontal_decoration_rejects_unverified_geometry_and_styles() {
    for points in [
        [[1, 1], [1, 1]],
        [[2, 1], [1, 1]],
        [[1, 1], [1, 2]],
        [[1, 1], [2, 2]],
    ] {
        assert!(empirical_c8_horizontal_decoration(page(), [0, 0], points, 0x1084).is_err());
    }
    assert!(empirical_c8_horizontal_decoration(page(), [0, 0], [[0, 0], [1, 0]], 0x1080).is_err());
    let mut invalid = page();
    invalid.size.width_points = f64::NAN;
    assert!(empirical_c8_horizontal_decoration(invalid, [0, 0], [[0, 0], [1, 0]], 0x1084).is_err());
}

#[test]
fn c8_segments_reproduce_independent_axes_and_preserve_endpoint_order() {
    let page = source_page_geometry([600, 600]).unwrap();
    for style in [0xa381, 0xa383, 0xa38b] {
        let horizontal =
            empirical_c8_segment(page, [4652, 4274], [[4682, 4304], [4832, 4304]], style).unwrap();
        // Original segment-axes control: relative endpoints (30,30)/(180,30).
        close(horizontal[0][0], 12000.0 / 2473.0);
        close(horizontal[1][0], 48000.0 / 2473.0);
        close(horizontal[0][1], 132000.0 / 2473.0);
        assert_eq!(horizontal[0][1], horizontal[1][1]);
        let reverse =
            empirical_c8_segment(page, [4652, 4274], [[4832, 4304], [4682, 4304]], style).unwrap();
        assert_eq!(reverse, [horizontal[1], horizontal[0]]);
        let vertical =
            empirical_c8_segment(page, [4652, 4274], [[4902, 4524], [4902, 4704]], style).unwrap();
        assert_eq!(vertical[0][0], vertical[1][0]);
        assert!(vertical[0][1] > vertical[1][1]);
        let shifted =
            empirical_c8_segment(page, [4672, 4294], [[4702, 4324], [4852, 4324]], style).unwrap();
        assert_eq!(shifted, horizontal);
    }
    let off_page =
        empirical_c8_segment(page, [100, 100], [[0, 0], [u16::MAX, u16::MAX]], 0xa381).unwrap();
    assert!(off_page[0][0] < 0.0);
    assert!(off_page[1][1] < 0.0);
    for style in [0xa384, 0xa382, 1] {
        assert!(empirical_c8_segment(page, [0, 0], [[0, 0], [1, 1]], style).is_err());
    }
    let mut invalid = page;
    invalid.origin_points[0] = f64::INFINITY;
    assert!(empirical_c8_segment(invalid, [0, 0], [[0, 0], [1, 1]], 0xa381).is_err());
}

#[test]
fn observed_glyph_style_prefixes_share_geometry_without_admitting_other_records() {
    for field in [2, 8] {
        for class in [C8GlyphClass::Cjk, C8GlyphClass::Latin] {
            let style = 0x1000 | field << 5 | field;
            let expected =
                empirical_c8_glyph_transform(page(), [4652, 4274], [4672, 4294], style, class)
                    .unwrap();
            for flags in [0x0800, 0x0c00] {
                let actual = empirical_c8_glyph_transform(
                    page(),
                    [4652, 4274],
                    [4672, 4294],
                    flags | field << 5 | field,
                    class,
                )
                .unwrap();
                // i586 may retain x87 intermediate precision across the two
                // evaluations. Compare arithmetic with the existing point-scale
                // tolerance; this is not a source-raster fidelity tolerance.
                for (actual, expected) in actual.into_iter().zip(expected) {
                    close(actual, expected);
                }
            }
        }
    }
    // The independent glyph controls do not admit these decoration states.
    assert!(
        empirical_c8_horizontal_decoration(page(), [0, 0], [[0, 0], [100, 0]], 0x0884).is_err()
    );
}

#[test]
fn independently_controlled_variants_preserve_both_glyph_classes() {
    for (reference, styles) in [
        (0x10e7, &[0x04e7, 0x14e7][..]),
        (0x1084, &[0x0484, 0x1484, 0x9c84][..]),
    ] {
        for class in [C8GlyphClass::Cjk, C8GlyphClass::Latin] {
            let expected =
                empirical_c8_glyph_transform(page(), [4652, 4274], [5200, 4700], reference, class)
                    .unwrap();
            for &style in styles {
                let actual =
                    empirical_c8_glyph_transform(page(), [4652, 4274], [5200, 4700], style, class)
                        .unwrap();
                for (actual, expected) in actual.into_iter().zip(expected) {
                    close(actual, expected);
                }
                assert!(
                    empirical_c8_horizontal_decoration(page(), [0, 0], [[0, 0], [100, 0]], style,)
                        .is_err()
                );
            }
        }
    }
}

#[test]
fn large_cjk_control_uses_verified_em_and_existing_signed_origin() {
    for (style, expected_em) in [(0xe58c, 27.159468438538206), (0x154a, 20.930232558139537)] {
        let actual = empirical_c8_glyph_transform(
            page(),
            [4652, 4274],
            [4672, 4294],
            style,
            C8GlyphClass::Cjk,
        )
        .unwrap();
        close(actual[0], expected_em);
        close(actual[3], expected_em);
        close(
            actual[4],
            page().origin_points[0] + 40.0 * EMPIRICAL_COORDINATE_POINTS_PER_UNIT,
        );
        let shifted = empirical_c8_glyph_transform(
            page(),
            [4672, 4294],
            [4692, 4314],
            style,
            C8GlyphClass::Cjk,
        )
        .unwrap();
        assert_eq!(actual, shifted);
        assert!(
            empirical_c8_glyph_transform(page(), [0, 0], [0, 0], style, C8GlyphClass::Latin,)
                .is_err()
        );
    }
    for style in [0x118c, 0xe58b, 0xe56c] {
        assert!(
            empirical_c8_glyph_transform(page(), [0, 0], [0, 0], style, C8GlyphClass::Cjk,)
                .is_err()
        );
    }
}

#[test]
fn a385_line_marker_preserves_independently_checked_endpoints() {
    for points in [[[4757, 4800], [6300, 4800]], [[4690, 4350], [4900, 4500]]] {
        let expected = empirical_c8_segment(page(), [4652, 4274], points, 0xa381).unwrap();
        let mut marked = points;
        marked[0][0] |= 0xc000;
        // x87 targets may retain different intermediate precision. Use the
        // existing point tolerance, as for the other geometry controls.
        for input in [points, marked] {
            let actual = empirical_c8_segment(page(), [4652, 4274], input, 0xa385).unwrap();
            for (actual, expected) in actual
                .into_iter()
                .flatten()
                .zip(expected.into_iter().flatten())
            {
                close(actual, expected);
            }
        }
        assert_ne!(
            empirical_c8_segment(page(), [4652, 4274], marked, 0xa381).unwrap(),
            expected
        );
    }
}

#[test]
fn mode_zero_geometry_uses_its_measured_origins_and_size_zero() {
    let page = source_page_geometry([700, 500]).unwrap();
    let unit = EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
    for (style, class, size, baseline) in [
        (0x1084, C8GlyphClass::Cjk, 35.0, 0.0),
        (0x1084, C8GlyphClass::Latin, 35.0, 10.0),
        (0, C8GlyphClass::Cjk, 21.0, 0.0),
        (0x1000, C8GlyphClass::Latin, 21.0, 10.0),
        (0x154a, C8GlyphClass::Cjk, 84.0, 0.0),
    ] {
        let m =
            mode_zero_glyph_transform(page, [30, 41], [100, 140], style, class, [None; 2]).unwrap();
        close(m[0], size * 75.0 / 301.0);
        close(m[3], size * 75.0 / 301.0);
        close(m[4], 90.0 * unit);
        close(m[5], (401.0 - baseline) * unit - size * 75.0 / 301.0);
    }
    let bad_page = EmpiricalPageGeometry {
        origin_points: [f64::NAN, 0.0],
        ..page
    };
    assert!(
        mode_zero_glyph_transform(bad_page, [0; 2], [100; 2], 0, C8GlyphClass::Cjk, [None; 2])
            .is_err()
    );
    assert!(
        mode_zero_glyph_transform(
            page,
            [0; 2],
            [100; 2],
            0x154a,
            C8GlyphClass::Latin,
            [None; 2]
        )
        .is_err()
    );
}

#[test]
fn mode_zero_digits_have_measured_height_specific_offsets() {
    let page = source_page_geometry([300, 300]).unwrap();
    let unit = EMPIRICAL_COORDINATE_POINTS_PER_UNIT;
    for (style, axes, em, raw_x, raw_y) in [
        (0x1000, [None; 2], 21.0, 19.0, 64.0),
        (0x1084, [None; 2], 35.0, 18.0, 67.0),
        (0x10a5, [None; 2], 42.0, 18.0, 69.0),
        (0, [Some(36); 2], 36.0, 18.0, 67.0),
    ] {
        let m = mode_zero_digit_transform(page, [0, 1], [20, 50], style, axes).unwrap();
        close(m[0], em * 75.0 / 301.0);
        close(m[3], em * 75.0 / 301.0);
        close(m[4], raw_x * unit);
        close(m[5], (300.0 - raw_y) * unit - em * 75.0 / 301.0);
    }
    for (style, axes) in [
        (0x154a, [None; 2]),
        (0x1084, [Some(36), None]),
        (0xe58c, [Some(36); 2]),
    ] {
        assert!(mode_zero_digit_transform(page, [0; 2], [20, 50], style, axes).is_err());
    }
}

#[test]
fn four_unit_axes_preserve_measured_em_and_latin_baseline() {
    let cjk = native_glyph_transform(
        page(),
        [4652, 4274],
        [4672, 4334],
        0x1084,
        C8GlyphClass::Cjk,
        [Some(4); 2],
    )
    .unwrap();
    let latin = native_glyph_transform(
        page(),
        [4652, 4274],
        [4672, 4334],
        0x10a5,
        C8GlyphClass::Latin,
        [Some(4); 2],
    )
    .unwrap();
    close(cjk[0], 4.0 * 75.0 / 301.0);
    close(cjk[3], cjk[0]);
    close(latin[0], cjk[0]);
    close(latin[3], cjk[3]);
    close(latin[4] - cjk[4], cjk[0] / 8.0);
    close(
        cjk[5] - latin[5],
        15.0 * EMPIRICAL_COORDINATE_POINTS_PER_UNIT,
    );
}
