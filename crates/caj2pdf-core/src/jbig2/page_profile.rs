// SPDX-License-Identifier: MIT

//! Preflight for the five-segment HN/C8 JBIG2 page profile observed by #43.
//!
//! One check binds the directory, the page information, and the region
//! headers to that profile. It reads no source bytes, decodes no bitmap, and
//! does not establish compatibility with JBIG2 streams outside the profile.

use super::{
    SegmentDirectory, SegmentSpan,
    generic::GenericRegionHeader,
    page_info::PageInfo,
    text::{TextHeaderAnomaly, TextRegionHeader},
};
use crate::{Error, ErrorKind, Result};

/// Number, type, and references of each segment: the page information, the
/// direct and the refinement dictionary, the text region, the generic region.
const OBSERVED_SEGMENTS: [(u32, u8, &[u32]); 5] = [
    (0, 48, &[]),
    (1, 0, &[]),
    (2, 0, &[1]),
    (3, 6, &[2]),
    (4, 38, &[]),
];

/// Metadata validated before any page bytes are sent to the final sink.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageProfile {
    pub(super) page: PageInfo,
    pub(super) text_header: TextRegionHeader,
    pub(super) generic_segment: u32,
    pub(super) generic_header: GenericRegionHeader,
}

impl PageProfile {
    pub fn page(self) -> PageInfo {
        self.page
    }

    pub fn text_segment(self) -> u32 {
        self.text_header.segment
    }

    pub fn text_page_association(self) -> u32 {
        self.text_header.page_association
    }

    pub fn text_dictionary_segment(self) -> u32 {
        self.text_header.dictionary_segment
    }

    pub fn text_body(self) -> SegmentSpan {
        self.text_header.body
    }

    /// The exact parsed header accepted by page preflight, including region
    /// geometry, text mode, instance count, and source identity.
    pub fn text_header(self) -> TextRegionHeader {
        self.text_header
    }

    pub fn generic_segment(self) -> u32 {
        self.generic_segment
    }

    pub fn text_flags_raw(self) -> u16 {
        self.text_header.flags.raw
    }

    pub fn text_header_anomaly(self) -> Option<TextHeaderAnomaly> {
        self.text_header.anomaly
    }

    pub fn generic_header(self) -> GenericRegionHeader {
        self.generic_header
    }
}

/// A metadata rejection at the first offending segment, when one is known.
fn malformed(segment: Option<u32>, reason: &'static str) -> Error {
    Error::invalid(reason).in_jbig2(segment)
}

fn unsupported(segment: Option<u32>, feature: &'static str) -> Error {
    Error::from(ErrorKind::UnsupportedFormat)
        .because(feature)
        .in_jbig2(segment)
}

/// Require the exact immediate, full-page OR composition topology measured in
/// the SHA-pinned HN/C8 corpus: segment 0 is an unstriped, lossless page with
/// no page-level options, followed by two dictionaries, the text region, and
/// the generic region. `page`, `text`, and `generic` must come from their
/// corresponding `directory` segments; callers then pass this profile to the
/// output adapter before starting any generic-region row.
pub fn validate_observed_page_profile(
    directory: &SegmentDirectory,
    page: PageInfo,
    text: &TextRegionHeader,
    generic: GenericRegionHeader,
) -> Result<PageProfile> {
    let segments = &directory.segments;
    if segments.len() != OBSERVED_SEGMENTS.len() {
        return Err(unsupported(None, "segment count"));
    }
    for (segment, &(number, segment_type, references)) in segments.iter().zip(&OBSERVED_SEGMENTS) {
        let at = Some(segment.number);
        if segment.number != number {
            return Err(unsupported(at, "segment number/order"));
        }
        if segment.segment_type != segment_type {
            return Err(unsupported(at, "segment type/order"));
        }
        if segment.page_association != 1 {
            return Err(unsupported(at, "page association"));
        }
        if segment.referred_to.as_slice() != references {
            return Err(malformed(at, "unexpected segment references"));
        }
    }
    // Each parsed header must describe its own directory segment.
    let (text_data, generic_segment) = (segments[3].data, &segments[4]);
    for (segment, matches, reason) in [
        (
            0,
            page.data == segments[0].data && page.data.length == 19,
            "page information does not match segment data",
        ),
        (
            3,
            text.segment == 3 && text.page_association == 1 && text.dictionary_segment == 2,
            "text header does not match segment directory",
        ),
        (
            3,
            text_data.offset.checked_add(text.header_bytes) == Some(text.body.offset)
                && text_data.length.checked_sub(text.header_bytes) == Some(text.body.length),
            "text body does not match segment data",
        ),
        (
            4,
            generic.segment == 4
                && generic.page_association == 1
                && generic.reference_count == 0
                && generic.data == generic_segment.data
                && generic.data.offset.checked_add(20) == Some(generic.mq_span.offset)
                && generic.data.length.checked_sub(20) == Some(generic.mq_span.length),
            "generic header does not match segment directory",
        ),
    ] {
        if !matches {
            return Err(malformed(Some(segment), reason));
        }
    }
    if page.width == 0 || page.height == 0 {
        return Err(malformed(Some(0), "zero page dimension"));
    }
    // Only the eventually-lossless flag: default pixel 0, OR, no auxiliary
    // buffers or refinements.
    if page.flags_raw != 0x01 || page.striping_raw != 0 {
        return Err(unsupported(Some(0), "page flags or striping"));
    }
    if page.height == u32::MAX {
        return Err(unsupported(Some(0), "unknown page height"));
    }
    // `PageInfo` fields are public, so its derived geometry is rechecked.
    let stride = u64::from(page.width).div_ceil(8);
    if page.row_stride as u64 != stride || page.packed_bytes != stride * u64::from(page.height) {
        return Err(malformed(Some(0), "page packed geometry differs"));
    }
    if generic.pixels != u64::from(page.width) * u64::from(page.height) {
        return Err(malformed(Some(4), "generic pixel count differs"));
    }
    let (region, info) = (text.region, generic.info);
    for (segment, feature, value, expected) in [
        (3, "text region width", region.width, page.width),
        (3, "text region height", region.height, page.height),
        (3, "text region x", region.x, 0),
        (3, "text region y", region.y, 0),
        (3, "text external operator", region.combination as u32, 0),
        (4, "generic region width", info.width, page.width),
        (4, "generic region height", info.height, page.height),
        (4, "generic region x", info.x, 0),
        (4, "generic region y", info.y, 0),
        (
            4,
            "generic row stride",
            info.row_stride as u32,
            page.row_stride as u32,
        ),
        (
            4,
            "generic external operator",
            u32::from(info.combination_operator),
            0,
        ),
    ] {
        if value != expected {
            return Err(unsupported(Some(segment), feature));
        }
    }
    if let Some((feature, _)) = text.unsupported_feature() {
        return Err(unsupported(Some(3), feature));
    }
    Ok(PageProfile {
        page,
        text_header: *text,
        generic_segment: 4,
        generic_header: generic,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Context;
    use crate::jbig2::{
        SegmentHeader, SegmentSpan,
        generic::GenericRegionInfo,
        mq::CodedSpan,
        text::{
            ReferenceCorner, RegionCombination, RegionInfo, SymbolCombination, TextRegionFlags,
        },
    };

    fn segment(number: u32, segment_type: u8, references: &[u32]) -> SegmentHeader {
        SegmentHeader {
            number,
            segment_type,
            deferred_non_retain: false,
            page_association: 1,
            referred_to: references.to_vec(),
            data: if number == 0 {
                SegmentSpan {
                    offset: 11,
                    length: 19,
                }
            } else {
                SegmentSpan {
                    offset: u64::from(number) * 64,
                    length: 32,
                }
            },
            header_length: if number == 0 { 11 } else { 8 },
            retention: vec![0],
        }
    }

    fn directory() -> SegmentDirectory {
        SegmentDirectory {
            span: SegmentSpan {
                offset: 0,
                length: 320,
            },
            segments: vec![
                segment(0, 48, &[]),
                segment(1, 0, &[]),
                segment(2, 0, &[1]),
                segment(3, 6, &[2]),
                segment(4, 38, &[]),
            ],
        }
    }

    fn page() -> PageInfo {
        PageInfo {
            data: SegmentSpan {
                offset: 11,
                length: 19,
            },
            width: 9,
            height: 2,
            x_resolution: 3000,
            y_resolution: 4000,
            flags_raw: 1,
            striping_raw: 0,
            row_stride: 2,
            packed_bytes: 4,
        }
    }

    fn text() -> TextRegionHeader {
        TextRegionHeader {
            segment: 3,
            page_association: 1,
            dictionary_segment: 2,
            region: RegionInfo {
                width: 9,
                height: 2,
                x: 0,
                y: 0,
                combination: RegionCombination::Or,
            },
            flags: TextRegionFlags {
                raw: 0,
                huffman: false,
                refine: false,
                log_strips: 0,
                reference_corner: ReferenceCorner::TopLeft,
                transposed: false,
                combination: SymbolCombination::Or,
                default_pixel: false,
                ds_offset: 0,
                refinement_template: 0,
            },
            anomaly: None,
            huffman_flags: None,
            refinement_at: None,
            instances: 0,
            header_bytes: 23,
            body: SegmentSpan {
                offset: 215,
                length: 9,
            },
        }
    }

    fn generic() -> GenericRegionHeader {
        GenericRegionHeader {
            segment: 4,
            page_association: 1,
            reference_count: 0,
            data: SegmentSpan {
                offset: 256,
                length: 32,
            },
            info: GenericRegionInfo {
                width: 9,
                height: 2,
                x: 0,
                y: 0,
                combination_operator: 0,
                row_stride: 2,
            },
            mq_span: CodedSpan {
                offset: 276,
                length: 12,
            },
            pixels: 18,
        }
    }

    #[test]
    fn observed_page_profile_retains_dimensions_resolution_and_policy() {
        let mut parsed_text = text();
        parsed_text.instances = 7;
        let profile = validate_observed_page_profile(&directory(), page(), &parsed_text, generic())
            .expect("observed profile");
        assert_eq!((profile.page().width, profile.page().height), (9, 2));
        assert_eq!(
            (profile.page().x_resolution, profile.page().y_resolution),
            (3000, 4000)
        );
        assert_eq!((profile.text_segment(), profile.generic_segment()), (3, 4));
        assert_eq!(profile.text_header(), parsed_text);
        assert_eq!(profile.text_header().instances, 7);
        assert_eq!(
            (
                profile.text_page_association(),
                profile.text_dictionary_segment()
            ),
            (1, 2)
        );
        assert_eq!(profile.text_body(), text().body);
        assert_eq!(profile.generic_header().data.offset, 256);
        assert_eq!(profile.text_flags_raw(), 0);
        assert_eq!(profile.text_header_anomaly(), None);
    }

    #[test]
    fn profile_rejects_additional_or_reordered_segments_and_wrong_references() {
        let mut extra = directory();
        extra.segments.push(segment(5, 38, &[]));
        assert!(matches!(
            validate_observed_page_profile(&extra, page(), &text(), generic()),
            Err(Error {
                kind: ErrorKind::UnsupportedFormat,
                reason: "segment count",
                ..
            })
        ));

        let mut wrong_order = directory();
        wrong_order.segments.swap(3, 4);
        assert!(matches!(
            validate_observed_page_profile(&wrong_order, page(), &text(), generic()),
            Err(Error {
                kind: ErrorKind::UnsupportedFormat,
                reason: "segment number/order",
                ..
            })
        ));

        let mut wrong_reference = directory();
        wrong_reference.segments[3].referred_to = vec![1];
        assert!(matches!(
            validate_observed_page_profile(&wrong_reference, page(), &text(), generic()),
            Err(Error {
                kind: ErrorKind::Malformed,
                context: Context::Jbig2 { segment: Some(3) },
                reason: "unexpected segment references",
                ..
            })
        ));

        let mut mismatched_body = text();
        mismatched_body.body.offset += 1;
        assert!(matches!(
            validate_observed_page_profile(&directory(), page(), &mismatched_body, generic()),
            Err(Error {
                kind: ErrorKind::Malformed,
                context: Context::Jbig2 { segment: Some(3) },
                reason: "text body does not match segment data",
                ..
            })
        ));

        let mut unrelated_generic = generic();
        unrelated_generic.data.offset += 1;
        assert!(matches!(
            validate_observed_page_profile(&directory(), page(), &text(), unrelated_generic),
            Err(Error {
                kind: ErrorKind::Malformed,
                context: Context::Jbig2 { segment: Some(4) },
                reason: "generic header does not match segment directory",
                ..
            })
        ));

        let mut wrong_mq_span = generic();
        wrong_mq_span.mq_span.offset += 1;
        assert!(matches!(
            validate_observed_page_profile(&directory(), page(), &text(), wrong_mq_span),
            Err(Error {
                kind: ErrorKind::Malformed,
                context: Context::Jbig2 { segment: Some(4) },
                reason: "generic header does not match segment directory",
                ..
            })
        ));
    }

    #[test]
    fn page_info_must_bind_to_the_exact_nineteen_byte_segment_zero() {
        let mut unrelated_page = page();
        unrelated_page.data.offset += 1;
        let error =
            validate_observed_page_profile(&directory(), unrelated_page, &text(), generic())
                .expect_err("page parsed from another source span must be refused");
        assert!(matches!(
            error,
            Error {
                kind: ErrorKind::Malformed,
                context: Context::Jbig2 { segment: Some(0) },
                reason: "page information does not match segment data",
                ..
            }
        ));

        let mut extra_body_directory = directory();
        extra_body_directory.segments[0].data.length = 20;
        let mut extra_body_page = page();
        extra_body_page.data.length = 20;
        let oversized = validate_observed_page_profile(
            &extra_body_directory,
            extra_body_page,
            &text(),
            generic(),
        )
        .expect_err("a matching but oversized page body must be refused");
        assert_eq!(
            (oversized.context, oversized.reason),
            (error.context, error.reason)
        );
    }

    #[test]
    fn generic_preflight_identity_must_match_segment_four() {
        for forged in [
            GenericRegionHeader {
                segment: 5,
                ..generic()
            },
            GenericRegionHeader {
                page_association: 2,
                ..generic()
            },
            GenericRegionHeader {
                reference_count: 1,
                ..generic()
            },
        ] {
            let error = validate_observed_page_profile(&directory(), page(), &text(), forged)
                .expect_err("generic metadata from another segment must be refused");
            assert!(matches!(
                error,
                Error {
                    kind: ErrorKind::Malformed,
                    context: Context::Jbig2 { segment: Some(4) },
                    reason: "generic header does not match segment directory",
                    ..
                }
            ));
        }
    }

    #[test]
    fn text_preflight_identity_must_match_segment_three() {
        for forged in [
            TextRegionHeader {
                segment: 4,
                ..text()
            },
            TextRegionHeader {
                page_association: 2,
                ..text()
            },
            TextRegionHeader {
                dictionary_segment: 1,
                ..text()
            },
        ] {
            let error = validate_observed_page_profile(&directory(), page(), &forged, generic())
                .expect_err("text metadata from another segment must be refused");
            assert!(matches!(
                error,
                Error {
                    kind: ErrorKind::Malformed,
                    context: Context::Jbig2 { segment: Some(3) },
                    reason: "text header does not match segment directory",
                    ..
                }
            ));
        }
    }

    #[test]
    fn profile_rejects_cross_page_and_non_or_regions_before_output() {
        let mut wrong_page = directory();
        wrong_page.segments[4].page_association = 2;
        assert!(matches!(
            validate_observed_page_profile(&wrong_page, page(), &text(), generic()),
            Err(Error {
                kind: ErrorKind::UnsupportedFormat,
                context: Context::Jbig2 { segment: Some(4) },
                reason: "page association",
                ..
            })
        ));

        let mut wrong_text = text();
        wrong_text.region.combination = RegionCombination::Xor;
        assert!(matches!(
            validate_observed_page_profile(&directory(), page(), &wrong_text, generic()),
            Err(Error {
                kind: ErrorKind::UnsupportedFormat,
                context: Context::Jbig2 { segment: Some(3) },
                reason: "text external operator",
                ..
            })
        ));

        let mut wrong_generic = generic();
        wrong_generic.info.x = 1;
        assert!(matches!(
            validate_observed_page_profile(&directory(), page(), &text(), wrong_generic),
            Err(Error {
                kind: ErrorKind::UnsupportedFormat,
                context: Context::Jbig2 { segment: Some(4) },
                reason: "generic region x",
                ..
            })
        ));
    }

    #[test]
    fn profile_rejects_forged_page_packing_and_unsupported_flags() {
        let mut wrong_packing = page();
        wrong_packing.packed_bytes = 3;
        assert!(matches!(
            validate_observed_page_profile(&directory(), wrong_packing, &text(), generic()),
            Err(Error {
                kind: ErrorKind::Malformed,
                context: Context::Jbig2 { segment: Some(0) },
                reason: "page packed geometry differs",
                ..
            })
        ));

        let mut wrong_pixels = generic();
        wrong_pixels.pixels -= 1;
        assert!(matches!(
            validate_observed_page_profile(&directory(), page(), &text(), wrong_pixels),
            Err(Error {
                kind: ErrorKind::Malformed,
                context: Context::Jbig2 { segment: Some(4) },
                reason: "generic pixel count differs",
                ..
            })
        ));

        let mut wrong_flags = page();
        wrong_flags.flags_raw = 0x41;
        assert!(matches!(
            validate_observed_page_profile(&directory(), wrong_flags, &text(), generic()),
            Err(Error {
                kind: ErrorKind::UnsupportedFormat,
                context: Context::Jbig2 { segment: Some(0) },
                reason: "page flags or striping",
                ..
            })
        ));
    }

    #[test]
    fn anomaly_is_explicit_and_unimplemented_text_mode_is_refused() {
        let mut compatible = text();
        compatible.flags.raw = 0xa40c;
        compatible.flags.refinement_template = 1;
        compatible.anomaly = Some(TextHeaderAnomaly::UnusedRefinementTemplate);
        let profile = validate_observed_page_profile(&directory(), page(), &compatible, generic())
            .expect("already accepted explicit policy");
        assert_eq!(profile.text_flags_raw(), 0xa40c);
        assert_eq!(
            profile.text_header_anomaly(),
            Some(TextHeaderAnomaly::UnusedRefinementTemplate)
        );

        compatible.huffman_flags = Some(1);
        assert!(matches!(
            validate_observed_page_profile(&directory(), page(), &compatible, generic()),
            Err(Error {
                kind: ErrorKind::UnsupportedFormat,
                context: Context::Jbig2 { segment: Some(3) },
                reason: "Huffman text region",
                ..
            })
        ));
    }

    #[test]
    fn typed_rejections_name_the_offending_segment_and_field() {
        let mut wrong_type = directory();
        wrong_type.segments[4].segment_type = 36;
        let error = validate_observed_page_profile(&wrong_type, page(), &text(), generic())
            .expect_err("refinement region is outside the observed profile");
        assert_eq!(error.context, Context::Jbig2 { segment: Some(4) });
        assert!(error.to_string().contains("segment 4: segment type/order"));

        let mut bad_page = page();
        bad_page.width = 0;
        let error = validate_observed_page_profile(&directory(), bad_page, &text(), generic())
            .expect_err("forged zero width must be refused");
        assert_eq!(
            error.to_string(),
            "malformed JBIG2, segment 0: zero page dimension"
        );

        let mut bad_page = page();
        bad_page.flags_raw = 0x05;
        let error = validate_observed_page_profile(&directory(), bad_page, &text(), generic())
            .expect_err("nonzero page default changes OR semantics");
        assert!(matches!(
            error,
            Error {
                kind: ErrorKind::UnsupportedFormat,
                reason: "page flags or striping",
                ..
            }
        ));
        assert_eq!(
            unsupported(None, "segment count").to_string(),
            "unsupported JBIG2: segment count"
        );
    }

    #[test]
    fn every_page_flag_striping_and_unknown_height_is_outside_the_profile() {
        let refused = |page: PageInfo| {
            validate_observed_page_profile(&directory(), page, &text(), generic())
                .expect_err("outside the observed page profile")
        };
        // Reserved, lossy, refinements, default pixel, operators, auxiliary
        // buffers, and operator override.
        for flags in [0x80, 0x00, 0x03, 0x05, 0x09, 0x11, 0x21, 0x41] {
            let flagged = PageInfo {
                flags_raw: flags,
                ..page()
            };
            assert!(
                matches!(
                    refused(flagged),
                    Error {
                        kind: ErrorKind::UnsupportedFormat,
                        context: Context::Jbig2 { segment: Some(0) },
                        reason: "page flags or striping",
                        ..
                    }
                ),
                "flags {flags:#04x}"
            );
        }
        for striping in [1_u16, 0x8000, 0x8001] {
            let striped = PageInfo {
                striping_raw: striping,
                ..page()
            };
            assert!(matches!(
                refused(striped),
                Error {
                    kind: ErrorKind::UnsupportedFormat,
                    context: Context::Jbig2 { segment: Some(0) },
                    reason: "page flags or striping",
                    ..
                }
            ));
        }
        let unknown = PageInfo {
            height: u32::MAX,
            ..page()
        };
        assert!(matches!(
            refused(unknown),
            Error {
                kind: ErrorKind::UnsupportedFormat,
                context: Context::Jbig2 { segment: Some(0) },
                reason: "unknown page height",
                ..
            }
        ));
        let mut detached = directory();
        detached.segments[0].page_association = 0;
        assert!(matches!(
            validate_observed_page_profile(&detached, page(), &text(), generic()),
            Err(Error {
                kind: ErrorKind::UnsupportedFormat,
                context: Context::Jbig2 { segment: Some(0) },
                reason: "page association",
                ..
            })
        ));
    }

    #[test]
    fn region_and_row_metadata_must_match_the_full_page() {
        let mut bad_text = text();
        bad_text.region.y = 1;
        let error = validate_observed_page_profile(&directory(), page(), &bad_text, generic())
            .expect_err("text placed below page origin");
        assert!(matches!(
            error,
            Error {
                kind: ErrorKind::UnsupportedFormat,
                reason: "text region y",
                ..
            }
        ));

        let mut bad_generic = generic();
        bad_generic.info.row_stride = 1;
        let error = validate_observed_page_profile(&directory(), page(), &text(), bad_generic)
            .expect_err("generic row width differs");
        assert!(matches!(
            error,
            Error {
                kind: ErrorKind::UnsupportedFormat,
                reason: "generic row stride",
                ..
            }
        ));

        let mut bad_generic = generic();
        bad_generic.info.combination_operator = 2;
        let error = validate_observed_page_profile(&directory(), page(), &text(), bad_generic)
            .expect_err("XOR changes page pixels");
        assert!(matches!(
            error,
            Error {
                kind: ErrorKind::UnsupportedFormat,
                reason: "generic external operator",
                ..
            }
        ));
    }
}
