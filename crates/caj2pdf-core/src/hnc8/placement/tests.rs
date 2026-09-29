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
    assert_eq!(&resized[4..], &transforms[0][4..]);
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
    assert!(matches!(
        empirical_image_transform(page, 1, 1, point(0, 14)),
        Err(Error::InvalidInput {
            reason: "empirical PDF origin cannot preserve the selected coordinate precision"
        })
    ));
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
