// SPDX-License-Identifier: MIT

//! Preflight for the five-segment HN/C8 JBIG2 page profile observed by #43.
//!
//! This checks composition metadata only. It does not decode any bitmap or
//! establish compatibility with JBIG2 streams outside this observed profile.

use super::{
    SegmentDirectory, SegmentSpan,
    generic::GenericRegionHeader,
    page_info::PageInfo,
    text::{RegionCombination, TextHeaderAnomaly, TextRegionHeader},
};
use std::{error, fmt};

const SEGMENT_NUMBERS: [u32; 5] = [0, 1, 2, 3, 4];
const SEGMENT_TYPES: [u8; 5] = [48, 0, 0, 6, 38];
const REFERENCES: [&[u32]; 5] = [&[], &[], &[1], &[2], &[]];

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageProfileErrorKind {
    Malformed(&'static str),
    Unsupported { feature: &'static str, value: u64 },
}

/// Metadata rejection; `segment` identifies the first offending segment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageProfileError {
    pub segment: Option<u32>,
    pub kind: PageProfileErrorKind,
}

pub type PageProfileResult<T> = Result<T, PageProfileError>;

impl fmt::Display for PageProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("unsupported HN/C8 JBIG2 page profile")?;
        if let Some(segment) = self.segment {
            write!(f, " at segment {segment}")?;
        }
        match self.kind {
            PageProfileErrorKind::Malformed(reason) => write!(f, ": malformed {reason}"),
            PageProfileErrorKind::Unsupported { feature, value } => {
                write!(f, ": unsupported {feature} ({value})")
            }
        }
    }
}

impl error::Error for PageProfileError {}

fn malformed(segment: Option<u32>, reason: &'static str) -> PageProfileError {
    PageProfileError {
        segment,
        kind: PageProfileErrorKind::Malformed(reason),
    }
}

fn unsupported(segment: Option<u32>, feature: &'static str, value: u64) -> PageProfileError {
    PageProfileError {
        segment,
        kind: PageProfileErrorKind::Unsupported { feature, value },
    }
}

/// Require the exact immediate, full-page OR composition topology measured in
/// the SHA-pinned HN/C8 corpus. `page`, `text`, and `generic` must come from
/// their corresponding `directory` segments; callers then pass this profile
/// to the output adapter before starting any generic-region row.
pub fn validate_observed_page_profile(
    directory: &SegmentDirectory,
    page: PageInfo,
    text: &TextRegionHeader,
    generic: GenericRegionHeader,
) -> PageProfileResult<PageProfile> {
    if directory.segments.len() != SEGMENT_TYPES.len() {
        return Err(unsupported(
            None,
            "segment count",
            directory.segments.len() as u64,
        ));
    }
    for (index, segment) in directory.segments.iter().enumerate() {
        if segment.number != SEGMENT_NUMBERS[index] {
            return Err(unsupported(
                Some(segment.number),
                "segment number/order",
                u64::from(segment.number),
            ));
        }
        if segment.segment_type != SEGMENT_TYPES[index] {
            return Err(unsupported(
                Some(segment.number),
                "segment type/order",
                u64::from(segment.segment_type),
            ));
        }
        if segment.page_association != 1 {
            return Err(unsupported(
                Some(segment.number),
                "page association",
                u64::from(segment.page_association),
            ));
        }
        if segment.referred_to.as_slice() != REFERENCES[index] {
            return Err(malformed(
                Some(segment.number),
                "unexpected segment references",
            ));
        }
    }

    if page.data != directory.segments[0].data || page.data.length != 19 {
        return Err(malformed(
            Some(0),
            "page information does not match segment data",
        ));
    }

    let text_segment = &directory.segments[3];
    if text.segment != text_segment.number
        || text.page_association != text_segment.page_association
        || text.dictionary_segment != text_segment.referred_to[0]
    {
        return Err(malformed(
            Some(3),
            "text header does not match segment directory",
        ));
    }
    let text_data = text_segment.data;
    if text_data.offset.checked_add(text.header_bytes) != Some(text.body.offset)
        || text_data.length.checked_sub(text.header_bytes) != Some(text.body.length)
    {
        return Err(malformed(Some(3), "text body does not match segment data"));
    }
    let generic_segment = &directory.segments[4];
    if generic.segment != generic_segment.number
        || generic.page_association != generic_segment.page_association
        || generic.reference_count != generic_segment.referred_to.len()
        || generic.data != generic_segment.data
        || generic.data.offset.checked_add(20) != Some(generic.mq_span.offset)
        || generic.data.length.checked_sub(20) != Some(generic.mq_span.length)
    {
        return Err(malformed(
            Some(4),
            "generic header does not match segment directory",
        ));
    }

    if page.width == 0 || page.height == 0 {
        return Err(malformed(Some(0), "zero page dimension"));
    }
    if page.flags_raw != 0x01 || page.striping_raw != 0 {
        return Err(unsupported(
            Some(0),
            "page flags or striping",
            (u64::from(page.flags_raw) << 16) | u64::from(page.striping_raw),
        ));
    }
    if page.default_pixel != 0 || page.combination_operator != 0 {
        return Err(unsupported(
            Some(0),
            "page default pixel or operator",
            (u64::from(page.default_pixel) << 8) | u64::from(page.combination_operator),
        ));
    }
    let stride = u64::from(page.width).div_ceil(8);
    let packed = stride
        .checked_mul(u64::from(page.height))
        .ok_or(malformed(Some(0), "page size overflows"))?;
    if page.row_stride as u64 != stride || page.packed_bytes != packed {
        return Err(malformed(Some(0), "page packed geometry differs"));
    }
    if generic.pixels != u64::from(page.width) * u64::from(page.height) {
        return Err(malformed(Some(4), "generic pixel count differs"));
    }

    let generic_header = generic;
    let generic = generic.info;
    for (feature, value, expected) in [
        ("text region width", text.region.width, page.width),
        ("text region height", text.region.height, page.height),
        ("text region x", text.region.x, 0),
        ("text region y", text.region.y, 0),
    ] {
        if value != expected {
            return Err(unsupported(Some(3), feature, u64::from(value)));
        }
    }
    if text.region.combination != RegionCombination::Or {
        return Err(unsupported(
            Some(3),
            "text external operator",
            text.region.combination as u64,
        ));
    }
    if let Some((feature, value)) = text.unsupported_feature() {
        return Err(unsupported(Some(3), feature, value));
    }
    for (feature, value, expected) in [
        ("generic region width", generic.width, page.width),
        ("generic region height", generic.height, page.height),
        ("generic region x", generic.x, 0),
        ("generic region y", generic.y, 0),
    ] {
        if value != expected {
            return Err(unsupported(Some(4), feature, u64::from(value)));
        }
    }
    if generic.row_stride != page.row_stride {
        return Err(unsupported(
            Some(4),
            "generic row stride",
            generic.row_stride as u64,
        ));
    }
    if generic.combination_operator != 0 {
        return Err(unsupported(
            Some(4),
            "generic external operator",
            u64::from(generic.combination_operator),
        ));
    }

    Ok(PageProfile {
        page,
        text_header: *text,
        generic_segment: 4,
        generic_header,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jbig2::{
        SegmentHeader, SegmentSpan,
        generic::GenericRegionInfo,
        mq::MqSpan,
        text::{ReferenceCorner, RegionInfo, SymbolCombination, TextRegionFlags},
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
            default_pixel: 0,
            combination_operator: 0,
            striping_raw: 0,
            row_stride: 2,
            packed_bytes: 4,
            source_bytes_fetched: 19,
            source_read_calls: 1,
            max_source_request_bytes: 19,
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
            mq_span: MqSpan {
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
            Err(PageProfileError {
                kind: PageProfileErrorKind::Unsupported {
                    feature: "segment count",
                    ..
                },
                ..
            })
        ));

        let mut wrong_order = directory();
        wrong_order.segments.swap(3, 4);
        assert!(matches!(
            validate_observed_page_profile(&wrong_order, page(), &text(), generic()),
            Err(PageProfileError {
                kind: PageProfileErrorKind::Unsupported {
                    feature: "segment number/order",
                    ..
                },
                ..
            })
        ));

        let mut wrong_reference = directory();
        wrong_reference.segments[3].referred_to = vec![1];
        assert!(matches!(
            validate_observed_page_profile(&wrong_reference, page(), &text(), generic()),
            Err(PageProfileError {
                segment: Some(3),
                kind: PageProfileErrorKind::Malformed("unexpected segment references"),
            })
        ));

        let mut mismatched_body = text();
        mismatched_body.body.offset += 1;
        assert!(matches!(
            validate_observed_page_profile(&directory(), page(), &mismatched_body, generic()),
            Err(PageProfileError {
                segment: Some(3),
                kind: PageProfileErrorKind::Malformed("text body does not match segment data"),
            })
        ));

        let mut unrelated_generic = generic();
        unrelated_generic.data.offset += 1;
        assert!(matches!(
            validate_observed_page_profile(&directory(), page(), &text(), unrelated_generic),
            Err(PageProfileError {
                segment: Some(4),
                kind: PageProfileErrorKind::Malformed(
                    "generic header does not match segment directory"
                ),
            })
        ));

        let mut wrong_mq_span = generic();
        wrong_mq_span.mq_span.offset += 1;
        assert!(matches!(
            validate_observed_page_profile(&directory(), page(), &text(), wrong_mq_span),
            Err(PageProfileError {
                segment: Some(4),
                kind: PageProfileErrorKind::Malformed(
                    "generic header does not match segment directory"
                ),
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
        assert_eq!(
            error,
            PageProfileError {
                segment: Some(0),
                kind: PageProfileErrorKind::Malformed(
                    "page information does not match segment data"
                ),
            }
        );

        let mut extra_body_directory = directory();
        extra_body_directory.segments[0].data.length = 20;
        let mut extra_body_page = page();
        extra_body_page.data.length = 20;
        assert_eq!(
            validate_observed_page_profile(
                &extra_body_directory,
                extra_body_page,
                &text(),
                generic(),
            )
            .expect_err("a matching but oversized page body must be refused"),
            error
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
            assert_eq!(
                error,
                PageProfileError {
                    segment: Some(4),
                    kind: PageProfileErrorKind::Malformed(
                        "generic header does not match segment directory"
                    ),
                }
            );
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
            assert_eq!(
                error,
                PageProfileError {
                    segment: Some(3),
                    kind: PageProfileErrorKind::Malformed(
                        "text header does not match segment directory"
                    ),
                }
            );
        }
    }

    #[test]
    fn profile_rejects_cross_page_and_non_or_regions_before_output() {
        let mut wrong_page = directory();
        wrong_page.segments[4].page_association = 2;
        assert!(matches!(
            validate_observed_page_profile(&wrong_page, page(), &text(), generic()),
            Err(PageProfileError {
                segment: Some(4),
                kind: PageProfileErrorKind::Unsupported {
                    feature: "page association",
                    ..
                },
            })
        ));

        let mut wrong_text = text();
        wrong_text.region.combination = RegionCombination::Xor;
        assert!(matches!(
            validate_observed_page_profile(&directory(), page(), &wrong_text, generic()),
            Err(PageProfileError {
                segment: Some(3),
                kind: PageProfileErrorKind::Unsupported {
                    feature: "text external operator",
                    ..
                },
            })
        ));

        let mut wrong_generic = generic();
        wrong_generic.info.x = 1;
        assert!(matches!(
            validate_observed_page_profile(&directory(), page(), &text(), wrong_generic),
            Err(PageProfileError {
                segment: Some(4),
                kind: PageProfileErrorKind::Unsupported {
                    feature: "generic region x",
                    ..
                },
            })
        ));
    }

    #[test]
    fn profile_rejects_forged_page_packing_and_unsupported_flags() {
        let mut wrong_packing = page();
        wrong_packing.packed_bytes = 3;
        assert!(matches!(
            validate_observed_page_profile(&directory(), wrong_packing, &text(), generic()),
            Err(PageProfileError {
                segment: Some(0),
                kind: PageProfileErrorKind::Malformed("page packed geometry differs"),
            })
        ));

        let mut wrong_pixels = generic();
        wrong_pixels.pixels -= 1;
        assert!(matches!(
            validate_observed_page_profile(&directory(), page(), &text(), wrong_pixels),
            Err(PageProfileError {
                segment: Some(4),
                kind: PageProfileErrorKind::Malformed("generic pixel count differs"),
            })
        ));

        let mut wrong_flags = page();
        wrong_flags.flags_raw = 0x41;
        assert!(matches!(
            validate_observed_page_profile(&directory(), wrong_flags, &text(), generic()),
            Err(PageProfileError {
                segment: Some(0),
                kind: PageProfileErrorKind::Unsupported {
                    feature: "page flags or striping",
                    ..
                },
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
            Err(PageProfileError {
                segment: Some(3),
                kind: PageProfileErrorKind::Unsupported {
                    feature: "Huffman text region",
                    ..
                },
            })
        ));
    }

    #[test]
    fn typed_rejections_name_the_offending_segment_and_field() {
        let mut wrong_type = directory();
        wrong_type.segments[4].segment_type = 36;
        let error = validate_observed_page_profile(&wrong_type, page(), &text(), generic())
            .expect_err("refinement region is outside the observed profile");
        assert_eq!(error.segment, Some(4));
        assert!(
            error
                .to_string()
                .contains("segment 4: unsupported segment type/order (36)")
        );

        let mut bad_page = page();
        bad_page.width = 0;
        let error = validate_observed_page_profile(&directory(), bad_page, &text(), generic())
            .expect_err("forged zero width must be refused");
        assert_eq!(
            error.to_string(),
            "unsupported HN/C8 JBIG2 page profile at segment 0: malformed zero page dimension"
        );

        let mut bad_page = page();
        bad_page.default_pixel = 1;
        let error = validate_observed_page_profile(&directory(), bad_page, &text(), generic())
            .expect_err("nonzero page default changes OR semantics");
        assert!(matches!(
            error.kind,
            PageProfileErrorKind::Unsupported {
                feature: "page default pixel or operator",
                value: 256,
            }
        ));
        assert!(
            unsupported(None, "segment count", 6)
                .to_string()
                .contains("profile: unsupported segment count (6)")
        );
    }

    #[test]
    fn region_and_row_metadata_must_match_the_full_page() {
        let mut bad_text = text();
        bad_text.region.y = 1;
        let error = validate_observed_page_profile(&directory(), page(), &bad_text, generic())
            .expect_err("text placed below page origin");
        assert!(matches!(
            error.kind,
            PageProfileErrorKind::Unsupported {
                feature: "text region y",
                value: 1,
            }
        ));

        let mut bad_generic = generic();
        bad_generic.info.row_stride = 1;
        let error = validate_observed_page_profile(&directory(), page(), &text(), bad_generic)
            .expect_err("generic row width differs");
        assert!(matches!(
            error.kind,
            PageProfileErrorKind::Unsupported {
                feature: "generic row stride",
                value: 1,
            }
        ));

        let mut bad_generic = generic();
        bad_generic.info.combination_operator = 2;
        let error = validate_observed_page_profile(&directory(), page(), &text(), bad_generic)
            .expect_err("XOR changes page pixels");
        assert!(matches!(
            error.kind,
            PageProfileErrorKind::Unsupported {
                feature: "generic external operator",
                value: 2,
            }
        ));
    }
}
