// SPDX-License-Identifier: MIT

use super::*;
use crate::NeverCancel;
use crate::test_support::page_image;
use crate::test_support::variant_of;
use crate::test_support::{field_of, kind_name};
use flate2::{Compression, write::ZlibEncoder};
use std::{cell::Cell, io::Write};

// Format tags with invented payload words, not a copied document header.
const INVENTED_PREFIX: [u8; 20] = *b"\x03\x80\x01\x00\x03\x80\x02\x00COMPRESSTEXT";

#[derive(Clone, Copy)]
enum Fault {
    None,
    Zero,
    Overreport,
    Error,
}

struct Source {
    bytes: Vec<u8>,
    size: u64,
    short: usize,
    fault: Fault,
    fault_at: u64,
    requests: usize,
    max_request: usize,
}

impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.size
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> crate::Result<usize> {
        self.requests += 1;
        self.max_request = self.max_request.max(destination.len());
        if offset >= self.fault_at {
            match self.fault {
                Fault::Zero => return Ok(0),
                Fault::Overreport => return Ok(destination.len() + 1),
                Fault::Error => {
                    return Err(Error::invalid("synthetic source fault"));
                }
                Fault::None => {}
            }
        }
        let offset = offset as usize;
        let available = self.bytes.len().saturating_sub(offset);
        let count = available.min(destination.len()).min(self.short);
        if count > 0 {
            destination[..count].copy_from_slice(&self.bytes[offset..offset + count]);
        }
        Ok(count)
    }
}

struct Fixture {
    source: Source,
    header: Header,
    page: PageRecord,
    plain: Vec<u8>,
}

impl Fixture {
    fn new(records: usize, coordinates: &[RawTextCoordinate]) -> Self {
        let mut plain = vec![0x19; 8 + records * 16 + 4 + coordinates.len() * 28];
        for record in 0..records {
            let at = 8 + record * 16;
            for (slot, marker) in [(0, 0x8070_u16), (4, 0x8071), (8, 0x8001)] {
                plain[at + slot..at + slot + 2].copy_from_slice(&marker.to_le_bytes());
            }
        }
        let tail = 12 + records * 16;
        for (index, coordinate) in coordinates.iter().enumerate() {
            let at = tail + index * 28;
            plain[at..at + 2].copy_from_slice(&coordinate.x.to_le_bytes());
            plain[at + 2..at + 4].copy_from_slice(&coordinate.y.to_le_bytes());
            plain[at + 4..at + 6].copy_from_slice(&coordinate.width.to_le_bytes());
            plain[at + 6..at + 8].copy_from_slice(&coordinate.height.to_le_bytes());
        }
        let mut result = Self {
            source: Source {
                bytes: vec![0; 512],
                size: 512,
                short: usize::MAX,
                fault: Fault::None,
                fault_at: u64::MAX,
                requests: 0,
                max_request: 0,
            },
            header: Header {
                native_mode: Some(2),
                native_origin: Some([0, 0]),
                page_size: None,
                variant: Variant::C8,
                page_count: 1,
                page_index: Span {
                    offset: 0x50,
                    length: 20,
                },
            },
            page: PageRecord {
                page_number: 1,
                row_offset: 0x50,
                text: Span {
                    offset: 512,
                    length: 0,
                },
                image_count: coordinates.len() as u32,
                unknown: [0; 10],
            },
            plain,
        };
        result.recompress();
        result
    }

    fn recompress(&mut self) {
        self.recompress_level(Compression::default());
    }

    fn recompress_level(&mut self, level: Compression) {
        let mut encoder = ZlibEncoder::new(Vec::new(), level);
        encoder.write_all(&self.plain).unwrap();
        self.set_frame(encoder.finish().unwrap());
        self.set_declared(self.plain.len() as u32);
    }

    fn set_frame(&mut self, frame: Vec<u8>) {
        self.source.bytes.truncate(512);
        self.source.bytes.extend(INVENTED_PREFIX);
        self.source
            .bytes
            .extend((self.plain.len() as u32).to_le_bytes());
        self.source.bytes.extend(frame);
        self.source.size = self.source.bytes.len() as u64;
        self.page.text.length = self.source.size - self.page.text.offset;
    }

    fn set_declared(&mut self, length: u32) {
        self.source.bytes[532..536].copy_from_slice(&length.to_le_bytes());
    }

    fn frame(&self) -> &[u8] {
        &self.source.bytes[536..]
    }

    fn parse<C: Cancellation>(&mut self, limits: Limits, cancel: &C) -> Result<TextCoordinates> {
        read_coordinates(&mut self.source, self.header, self.page, &limits, cancel)
    }

    fn normal(&mut self) -> Result<TextCoordinates> {
        self.parse(Limits::default(), &NeverCancel)
    }
}

fn point(x: u16, y: u16) -> RawTextCoordinate {
    RawTextCoordinate {
        x,
        y,
        width: 0,
        height: 0,
    }
}
fn ordinary() -> Fixture {
    Fixture::new(3, &[point(10, 20), point(0x8123, 0xffff), point(10, 20)])
}

fn hna_region_frame(regions: u8) -> Fixture {
    let placement = RawTextCoordinate {
        x: 0,
        y: 0,
        width: 1500,
        height: 2100,
    };
    let mut fixture = Fixture::new(2, &[placement]);
    fixture.header.variant = Variant::HnA;
    fixture.header.page_index.offset = 0x15c;
    fixture.page.row_offset = 0x15c;
    fixture.plain[..8].copy_from_slice(&[0x1c, 0x80, 0, 0, 0xce, 0x80, 0, 0]);
    fixture.plain.truncate(40);
    for record in 0..=regions {
        let mut bytes = [0; 28];
        bytes[..2].copy_from_slice(&0x800a_u16.to_le_bytes());
        let words = if record == 0 {
            [0, 0, 1500, 2100]
        } else {
            // Region coordinates are opaque metadata, including an extent
            // beyond the page. They must not create or resize another image.
            [100, 200, 1800, 700]
        };
        for (at, word) in words.into_iter().enumerate() {
            bytes[4 + at * 2..6 + at * 2].copy_from_slice(&(word as u16).to_le_bytes());
        }
        bytes[12] = if record == 0 { regions - 1 } else { record - 1 };
        fixture.plain.extend_from_slice(&bytes);
    }
    fixture.plain.extend_from_slice(&[4, 0x80, 0, 0]);
    frame_with_jpeg_descriptor(&mut fixture);
    fixture
}

fn frame_with_jpeg_descriptor(fixture: &mut Fixture) {
    fixture.recompress();
    fixture.source.bytes[514..516].copy_from_slice(&1500_u16.to_le_bytes());
    fixture.source.bytes[518..520].copy_from_slice(&2100_u16.to_le_bytes());
    // This coordinate-reader test needs only the descriptor type. Complete
    // descriptor/payload validation belongs to the container/composer tests.
    fixture.source.bytes.extend_from_slice(&2_u32.to_le_bytes());
    fixture.source.size = fixture.source.bytes.len() as u64;
}

#[test]
fn hna_full_page_jpeg_regions_do_not_add_image_placements() {
    for regions in [1, 2] {
        for chunk in [1, 2, 3, 11, 28, 65_536] {
            let mut fixture = hna_region_frame(regions);
            fixture.source.short = 3;
            let result = fixture
                .parse(
                    Limits {
                        io_chunk_bytes: chunk,
                        ..Limits::default()
                    },
                    &NeverCancel,
                )
                .unwrap();
            assert_eq!(result.page_size, Some([1500, 2100]));
            assert_eq!(result.record_count, 2);
            assert_eq!(
                result.coordinates,
                [RawTextCoordinate {
                    x: 0,
                    y: 0,
                    width: 1500,
                    height: 2100,
                }]
            );
            assert!(fixture.source.max_request <= chunk);
        }
    }
}

#[test]
fn hna_image_regions_require_complete_records_and_the_measured_profile() {
    for at in [0, 4, 8, 40, 44, 48, 68, 70, 80, 84, 96, 108, 124, 126] {
        let mut fixture = hna_region_frame(2);
        fixture.plain[at] ^= 1;
        frame_with_jpeg_descriptor(&mut fixture);
        assert_eq!(kind_name(&fixture.normal().unwrap_err()), "malformed");
    }
    let mut fixture = hna_region_frame(1);
    fixture.plain[52] = 2;
    frame_with_jpeg_descriptor(&mut fixture);
    assert!(fixture.normal().is_err());
    for removed in [1, 4, 12, 27] {
        let mut fixture = hna_region_frame(2);
        fixture.plain.truncate(fixture.plain.len() - removed);
        frame_with_jpeg_descriptor(&mut fixture);
        assert!(fixture.normal().is_err());
    }
    let mut fixture = hna_region_frame(1);
    let at = (fixture.page.text.offset + fixture.page.text.length) as usize;
    fixture.source.bytes[at] = 0; // A type-0 image is outside this JPEG profile.
    assert_eq!(kind_name(&fixture.normal().unwrap_err()), "malformed");
    let mut fixture = hna_region_frame(1);
    fixture.source.bytes.pop();
    fixture.source.size -= 1;
    assert!(fixture.normal().is_err());
    let mut fixture = hna_region_frame(1);
    fixture.header.variant = Variant::C8;
    fixture.header.page_index.offset = 0x50;
    fixture.page.row_offset = 0x50;
    assert_eq!(kind_name(&fixture.normal().unwrap_err()), "malformed");
}

#[test]
fn invented_frames_preserve_raw_words_order_and_repeats() {
    let expected = [point(10, 20), point(0x8123, 0xffff), point(10, 20)];
    let mut fixture = ordinary();
    let result = fixture.normal().unwrap();
    assert_eq!(result.coordinates, expected);
    assert_eq!(result.text, fixture.page.text);
    assert_eq!(
        result.zlib_frame.unwrap(),
        Span {
            offset: 536,
            length: fixture.page.text.length - 24
        }
    );
    assert_eq!((result.decoded_length, result.record_count), (144, 3));
    assert_eq!(result.max_source_request_bytes, fixture.source.max_request);
}

#[test]
fn chunk_boundaries_and_short_reads_do_not_change_metadata() {
    for chunk in [
        1, 2, 3, 4, 7, 15, 16, 17, 23, 24, 25, 27, 28, 29, 31, 64, 4096, 65_536,
    ] {
        for short in [1, 3, usize::MAX] {
            let mut fixture = ordinary();
            fixture.source.short = short;
            let result = fixture
                .parse(
                    Limits {
                        io_chunk_bytes: chunk,
                        ..Limits::default()
                    },
                    &NeverCancel,
                )
                .unwrap();
            assert_eq!(
                result.coordinates,
                [point(10, 20), point(0x8123, 0xffff), point(10, 20)]
            );
            assert_eq!(result.max_source_request_bytes, fixture.source.max_request);
            assert!(result.max_source_request_bytes <= chunk);
            assert!(result.max_decoder_output_chunk_bytes <= chunk);
        }
    }
}

#[test]
fn empty_records_and_coordinates_are_structurally_valid() {
    let mut fixture = Fixture::new(0, &[]);
    let report = fixture.normal().unwrap();
    assert_eq!((report.decoded_length, report.record_count), (12, 0));
    assert!(report.coordinates.is_empty());
    assert_eq!(report.owned_buffer_bytes, fixture.frame().len() as u64 + 13);
}

#[test]
fn large_decoded_text_is_discarded_in_bounded_chunks() {
    let mut fixture = Fixture::new(20_000, &[point(0, 65535)]);
    let result = fixture.normal().unwrap();
    assert_eq!(result.record_count, 20_000);
    assert_eq!(result.coordinates, [point(0, 65535)]);
    assert_eq!(result.max_decoder_output_chunk_bytes, CHUNK_BYTES);
    assert!(result.owned_buffer_bytes < 70 * 1024);
    assert!(fixture.plain.len() > result.owned_buffer_bytes as usize);
}

#[test]
fn stored_and_compressed_blocks_respect_request_and_output_caps() {
    for compression in [
        Compression::none(),
        Compression::fast(),
        Compression::best(),
    ] {
        let mut fixture = Fixture::new(5000, &[point(17, 31)]);
        fixture.recompress_level(compression);
        let result = fixture.normal().unwrap();
        assert_eq!(result.coordinates, [point(17, 31)]);
        assert_eq!(result.record_count, 5000);
        assert!(result.max_source_request_bytes <= CHUNK_BYTES);
        assert!(result.max_decoder_output_chunk_bytes <= CHUNK_BYTES);
        if compression == Compression::none() {
            assert!(fixture.frame().len() > CHUNK_BYTES);
            assert_eq!(result.max_source_request_bytes, CHUNK_BYTES);
            assert_eq!(
                result.owned_buffer_bytes,
                (2 * CHUNK_BYTES + size_of::<RawTextCoordinate>()) as u64
            );
        }
    }
}

#[test]
fn both_variants_reject_unknown_compression_markers() {
    for variant in [Variant::C8, Variant::HnA] {
        let mut fixture = ordinary();
        fixture.header.variant = variant;
        if variant == Variant::HnA {
            fixture.header.page_index.offset = 0x15c;
            fixture.page.row_offset = 0x15c;
        }
        fixture.source.bytes[520] ^= 1;
        let error = read_coordinates(
            &mut fixture.source,
            fixture.header,
            fixture.page,
            &Limits::default(),
            &NeverCancel,
        )
        .unwrap_err();
        assert_eq!(field_of(&error), "page text prefix");
        assert_eq!(variant_of(&error), Some(variant));
    }
}

#[test]
fn hn_a_outline_aligned_index_is_supported_and_hn_b_is_not() {
    let mut fixture = ordinary();
    fixture.header.variant = Variant::HnA;
    fixture.header.page_index.offset = 0x15c;
    fixture.page.row_offset = 0x15c;
    assert_eq!(fixture.normal().unwrap().coordinates.len(), 3);
    fixture.header.variant = Variant::HnB;
    let error = read_coordinates(
        &mut fixture.source,
        fixture.header,
        fixture.page,
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert_eq!(kind_name(&error), "unsupported");
    assert_eq!(field_of(&error), "text framing variant");
    assert_eq!(
        field_of(&fixture.normal().unwrap_err()),
        "text framing variant"
    );
}

#[test]
fn later_rows_and_hn_a_outline_prefixes_are_checked_consistently() {
    let mut fixture = ordinary();
    fixture.header.page_count = 2;
    fixture.header.page_index.length = 40;
    fixture.page.page_number = 2;
    fixture.page.row_offset = 100;
    assert_eq!(fixture.normal().unwrap().coordinates.len(), 3);

    let text = fixture.source.bytes[512..].to_vec();
    fixture.source.bytes = vec![0; 1024];
    fixture.source.bytes.extend(text);
    fixture.source.size = fixture.source.bytes.len() as u64;
    fixture.header.variant = Variant::HnA;
    fixture.header.page_index.offset = 0x15c + 308;
    fixture.page.row_offset = 0x15c + 308 + 20;
    fixture.page.text.offset = 1024;
    assert_eq!(fixture.normal().unwrap().coordinates.len(), 3);
}

#[test]
fn fabricated_metadata_is_revalidated_before_reads() {
    type Change = fn(&mut Fixture);
    let cases: &[(Change, &str)] = &[
        (|f| f.header.page_count = 0, "page count"),
        (|f| f.header.page_count = u32::MAX, "page count"),
        (|f| f.page.page_number = 0, "page number"),
        (|f| f.page.page_number = 2, "page number"),
        (|f| f.header.page_index.length = 19, "page index"),
        (|f| f.header.page_index.offset = 84, "page index"),
        (|f| f.page.row_offset = u64::MAX, "page row"),
        (|f| f.page.text.offset = 99, "page text span"),
        (|f| f.page.text.length = 23, "page text header"),
        (|f| f.page.text.offset = f.source.size + 1, "page text span"),
        (|f| f.page.text.length = f.source.size, "page text span"),
        (
            |f| {
                f.page.text = Span {
                    offset: u64::MAX - 10,
                    length: 24,
                }
            },
            "page text span",
        ),
    ];
    for (change, field) in cases {
        let mut fixture = ordinary();
        change(&mut fixture);
        assert_eq!(field_of(&fixture.normal().unwrap_err()), *field);
        assert_eq!(fixture.source.requests, 0);
    }
    let cases: &[Change] = &[
        |f| f.header.page_index.offset = u64::MAX - 10,
        |f| f.page.text.offset = i32::MAX as u64 + 1,
        |f| f.page.text.length = i32::MAX as u64 + 1,
        |f| {
            f.header.variant = Variant::HnA;
            f.header.page_index.offset = 347;
        },
        |f| {
            f.header.variant = Variant::HnA;
            f.header.page_index.offset = 349;
        },
        |f| {
            f.header.variant = Variant::HnA;
            f.header.page_index.offset = 0x15c + (i32::MAX as u64 + 1) * 308;
        },
    ];
    for change in cases {
        let mut fixture = ordinary();
        fixture.source.size = u64::MAX;
        change(&mut fixture);
        let error = fixture
            .parse(
                Limits {
                    max_input_bytes: u64::MAX,
                    ..Limits::default()
                },
                &NeverCancel,
            )
            .unwrap_err();
        assert_eq!(kind_name(&error), "malformed");
        assert_eq!(fixture.source.requests, 0);
    }
    let mut fixture = ordinary();
    fixture.source.size = 90;
    assert_eq!(field_of(&fixture.normal().unwrap_err()), "page index");
}

#[test]
fn limits_refuse_work_before_unbounded_allocation() {
    for (limits, field) in [
        (
            Limits {
                max_input_bytes: 1,
                ..Limits::default()
            },
            "source bytes",
        ),
        (
            Limits {
                max_pages: 0,
                ..Limits::default()
            },
            "pages",
        ),
        (
            Limits {
                max_output_bytes: 0,
                ..Limits::default()
            },
            "text decoded output bytes",
        ),
        (
            Limits {
                io_chunk_bytes: 1,
                max_allocation_bytes: TEXT_DECODER_RESERVATION_BYTES - 1,
                ..Limits::default()
            },
            "text decoder allocation reservation",
        ),
    ] {
        assert_eq!(
            field_of(&ordinary().parse(limits, &NeverCancel).unwrap_err()),
            field
        );
    }
    let f = ordinary();
    let error = allocate(
        3,
        RawTextCoordinate::default(),
        &Limits {
            max_allocation_bytes: 8,
            ..Limits::default()
        },
        location(f.header, f.page),
    )
    .unwrap_err();
    assert_eq!(field_of(&error), "text allocation bytes");
    let error = allocate(
        usize::MAX,
        0_u8,
        &Limits {
            max_allocation_bytes: u64::MAX,
            ..Limits::default()
        },
        location(f.header, f.page),
    )
    .unwrap_err();
    assert_eq!(field_of(&error), "text allocation bytes");
}

#[test]
fn layout_and_declared_lengths_are_strict() {
    for length in [0, 11, 95, 97] {
        let mut fixture = ordinary();
        fixture.set_declared(length);
        assert_eq!(
            field_of(&fixture.normal().unwrap_err()),
            "decoded text layout"
        );
    }
    let mut too_long = Fixture::new(1, &[]);
    too_long.set_declared(12);
    assert_eq!(
        field_of(&too_long.normal().unwrap_err()),
        "decoded text length"
    );
    let mut too_short = Fixture::new(0, &[]);
    too_short.plain[8..10].copy_from_slice(&0x8070_u16.to_le_bytes());
    too_short.recompress();
    too_short.set_declared(28);
    assert_eq!(
        field_of(&too_short.normal().unwrap_err()),
        "text zlib frame"
    );
}

#[test]
fn every_marker_slot_is_validated_while_opaque_bytes_are_ignored() {
    for slot in [0, 1, 4, 5, 8, 9] {
        let mut fixture = ordinary();
        fixture.plain[8 + 16 + slot] ^= 1;
        fixture.recompress();
        assert_eq!(
            field_of(&fixture.normal().unwrap_err()),
            "decoded text marker"
        );
    }
    let mut fixture = ordinary();
    fixture.plain[0] = 255;
    fixture.plain[8 + 2] = 250;
    fixture.plain[8 + 3] = 251;
    fixture.plain[8 + 16 * 3] = 254;
    let last = fixture.plain.len() - 1;
    fixture.plain[last] = 252;
    fixture.recompress();
    assert_eq!(
        fixture.normal().unwrap().coordinates,
        [point(10, 20), point(0x8123, 0xffff), point(10, 20)]
    );
}

#[test]
fn malformed_checksum_dictionary_truncation_and_trailing_frames_are_rejected() {
    for length in 0..6 {
        let mut fixture = ordinary();
        fixture.set_frame(vec![0; length]);
        assert_eq!(kind_name(&fixture.normal().unwrap_err()), "truncated");
    }
    let mut fixture = ordinary();
    let mut frame = fixture.frame().to_vec();
    frame[0] = 0;
    fixture.set_frame(frame);
    assert_eq!(field_of(&fixture.normal().unwrap_err()), "text zlib frame");
    let mut fixture = ordinary();
    let mut frame = fixture.frame().to_vec();
    let last = frame.len() - 1;
    frame[last] ^= 1;
    fixture.set_frame(frame);
    assert_eq!(field_of(&fixture.normal().unwrap_err()), "text zlib frame");
    let mut fixture = ordinary();
    let mut frame = fixture.frame().to_vec();
    let base = 0x20_u16;
    frame[1] = (base + (31 - ((u16::from(frame[0]) * 256 + base) % 31)) % 31) as u8;
    frame.splice(2..2, [0, 0, 0, 1]);
    fixture.set_frame(frame);
    assert_eq!(field_of(&fixture.normal().unwrap_err()), "text zlib frame");
    for drop in [1, 2, 4, 6] {
        let mut fixture = ordinary();
        let mut frame = fixture.frame().to_vec();
        frame.truncate(frame.len() - drop);
        fixture.set_frame(frame);
        assert_eq!(field_of(&fixture.normal().unwrap_err()), "text zlib frame");
    }
    for concatenate in [false, true] {
        let mut fixture = ordinary();
        let mut frame = fixture.frame().to_vec();
        if concatenate {
            frame.extend(fixture.frame());
        } else {
            frame.push(0);
        }
        fixture.set_frame(frame);
        assert_eq!(field_of(&fixture.normal().unwrap_err()), "text zlib frame");
    }
}

#[test]
fn zero_overreported_and_error_reads_are_located() {
    // A source failure keeps its own reason; a short read names the field.
    for (fault, kind, field) in [
        (Fault::Zero, "truncated", "page text read"),
        (
            Fault::Overreport,
            "malformed",
            "source reported more bytes than requested",
        ),
        (Fault::Error, "malformed", "synthetic source fault"),
    ] {
        for offset in [512, 536] {
            let mut fixture = ordinary();
            fixture.source.fault = fault;
            fixture.source.fault_at = offset;
            let error = fixture.normal().unwrap_err();
            assert_eq!(kind_name(&error), kind);
            assert_eq!(field_of(&error), field);
            assert_eq!(error.offset, Some(offset));
            assert_eq!(page_image(&error).0, Some(1));
        }
    }
    let mut fixture = ordinary();
    fixture.source.bytes.truncate(540);
    let error = fixture.normal().unwrap_err();
    assert_eq!(error.offset, Some(540));
    assert_eq!(kind_name(&error), "truncated");
}

struct CancelAfter(Cell<u64>);
impl Cancellation for CancelAfter {
    fn is_cancelled(&self) -> bool {
        let remaining = self.0.get();
        self.0.set(remaining.saturating_sub(1));
        remaining == 0
    }
}

#[test]
fn every_cancellation_checkpoint_returns_no_partial_coordinates() {
    let mut completed = false;
    for polls in 0..1000 {
        let mut fixture = ordinary();
        let result = fixture.parse(
            Limits {
                io_chunk_bytes: 7,
                ..Limits::default()
            },
            &CancelAfter(Cell::new(polls)),
        );
        match result {
            Err(error) => assert_eq!(kind_name(&error), "cancelled"),
            Ok(report) => {
                assert_eq!(report.coordinates.len(), 3);
                completed = true;
                break;
            }
        }
    }
    assert!(completed);
}

#[test]
fn compressed_header_payload_words_vary_without_changing_coordinates() {
    for word in [0_u16, 1, 0x8000, u16::MAX] {
        for at in [514, 518] {
            let mut fixture = ordinary();
            fixture.source.bytes[at..at + 2].copy_from_slice(&word.to_le_bytes());
            fixture.source.short = 1;
            assert_eq!(
                fixture.normal().unwrap().coordinates,
                [point(10, 20), point(0x8123, 0xffff), point(10, 20)]
            );
        }
    }
}

#[test]
fn compressed_header_requires_both_tags_and_the_complete_marker() {
    for relative in [0, 1, 4, 5].into_iter().chain(8..20) {
        let mut fixture = ordinary();
        fixture.source.bytes[512 + relative] ^= 1;
        let error = fixture.normal().unwrap_err();
        assert_eq!(field_of(&error), "page text prefix");
        assert_eq!(error.offset, Some(512));
    }
}

fn raw_fixture() -> Fixture {
    let mut fixture = ordinary();
    fixture.header.variant = Variant::HnA;
    fixture.header.page_index.offset = 0x15c;
    fixture.page.row_offset = 0x15c;
    fixture.page.image_count = 2;
    let mut bytes = Vec::new();
    for (tag, value) in [
        (0x8001_u16, 7_u16),
        (0x8070, 10),
        (0x8071, 20),
        (33, 0x800a),
        (44, 0x8004),
    ] {
        bytes.extend(tag.to_le_bytes());
        bytes.extend(value.to_le_bytes());
    }
    for (x, y) in [(12_u16, 34_u16), (0xffff, 0x8000)] {
        bytes.extend(0x800a_u16.to_le_bytes());
        bytes.extend(99_u16.to_le_bytes());
        bytes.extend(x.to_le_bytes());
        bytes.extend(y.to_le_bytes());
        bytes.extend([0; 4]);
        bytes.extend([0x5a; 16]);
    }
    bytes.extend(0x8004_u16.to_le_bytes());
    bytes.extend(7_u16.to_le_bytes());
    bytes.extend([0xff; 80]); // Opaque tail is read/hashed, never scanned for images.
    fixture.source.bytes.truncate(512);
    fixture.source.bytes.extend(bytes);
    fixture.source.size = fixture.source.bytes.len() as u64;
    fixture.page.text.length = fixture.source.size - 512;
    fixture
}

#[test]
fn raw_records_cross_every_small_chunk_boundary_without_false_image_markers() {
    for chunk in 1..=33 {
        let mut fixture = raw_fixture();
        fixture.source.short = 1;
        let result = fixture
            .parse(
                Limits {
                    io_chunk_bytes: chunk,
                    ..Limits::default()
                },
                &NeverCancel,
            )
            .unwrap();
        assert_eq!(result.coordinates, [point(12, 34), point(0xffff, 0x8000)]);
        assert_eq!(result.record_count, 8);
        assert_eq!(result.zlib_frame, None);
        assert_eq!(result.max_decoder_output_chunk_bytes, 0);
        assert!(result.max_source_request_bytes <= chunk);
    }
}

#[test]
fn raw_record_order_counts_and_truncation_are_checked() {
    for relative in [4, 8, 12, 20, 48, 76] {
        let mut fixture = raw_fixture();
        fixture.source.bytes[512 + relative..514 + relative]
            .copy_from_slice(&0x80ff_u16.to_le_bytes());
        let error = fixture.normal().unwrap_err();
        assert_eq!(field_of(&error), "raw text record");
        assert_eq!(error.offset, Some(512 + relative as u64));
    }
    for count in [0, 1, 3] {
        let mut fixture = raw_fixture();
        fixture.page.image_count = count;
        assert_eq!(field_of(&fixture.normal().unwrap_err()), "raw text record");
    }
    for length in [24, 40, 76, 79] {
        let mut fixture = raw_fixture();
        fixture.page.text.length = length;
        assert_eq!(field_of(&fixture.normal().unwrap_err()), "raw text records");
    }
    let mut fixture = raw_fixture();
    fixture.source.bytes.truncate(540);
    assert_eq!(kind_name(&fixture.normal().unwrap_err()), "truncated");
}

#[test]
fn raw_limits_and_source_failures_do_not_return_partial_coordinates() {
    for (limits, field) in [
        (
            Limits {
                max_output_bytes: 1,
                ..Limits::default()
            },
            "raw text bytes",
        ),
        (
            Limits {
                max_allocation_bytes: 1,
                io_chunk_bytes: 1,
                ..Limits::default()
            },
            "text allocation bytes",
        ),
    ] {
        assert_eq!(
            field_of(&raw_fixture().parse(limits, &NeverCancel).unwrap_err()),
            field
        );
    }
    let mut fixture = raw_fixture();
    fixture.source.fault = Fault::Error;
    fixture.source.fault_at = 516;
    assert!(
        fixture
            .parse(
                Limits {
                    io_chunk_bytes: 4,
                    ..Limits::default()
                },
                &NeverCancel
            )
            .is_err()
    );
    let mut fixture = raw_fixture();
    fixture.header.variant = Variant::C8;
    fixture.header.page_index.offset = 0x50;
    fixture.page.row_offset = 0x50;
    assert_eq!(field_of(&fixture.normal().unwrap_err()), "page text prefix");
}

#[test]
fn raw_zero_image_stream_still_requires_a_terminator() {
    let mut fixture = raw_fixture();
    fixture.page.image_count = 0;
    fixture.source.bytes[512..514].copy_from_slice(&0x8004_u16.to_le_bytes());
    let result = fixture.normal().unwrap();
    assert!(result.coordinates.is_empty());
    assert_eq!(result.record_count, 1);
}

#[test]
fn raw_cancellation_including_opaque_tail_returns_no_partial_coordinates() {
    let mut completed = false;
    for polls in 0..200 {
        let mut fixture = raw_fixture();
        match fixture.parse(
            Limits {
                io_chunk_bytes: 7,
                ..Limits::default()
            },
            &CancelAfter(Cell::new(polls)),
        ) {
            Err(error) => assert_eq!(kind_name(&error), "cancelled"),
            Ok(report) => {
                assert_eq!(report.coordinates.len(), 2);
                completed = true;
                break;
            }
        }
    }
    assert!(completed);
}

#[test]
fn raw_image_only_records_need_no_glyph_run() {
    let mut fixture = raw_fixture();
    fixture.source.bytes.drain(512..532);
    fixture.source.size -= 20;
    fixture.page.text.length -= 20;
    let report = fixture.normal().unwrap();
    assert_eq!(report.coordinates, [point(12, 34), point(0xffff, 0x8000)]);
    assert_eq!(report.record_count, 3);
}

fn direct_fixture(plain: Vec<u8>, images: usize) -> Fixture {
    let mut fixture = Fixture::new(0, &vec![point(0, 0); images]);
    fixture.plain = plain;
    fixture.recompress();
    fixture.source.bytes.drain(512..520); // Direct marker has no tagged prefix.
    fixture.source.size -= 8;
    fixture.page.text.length -= 8;
    fixture
}

fn direct_record(tag: u16, value: u16) -> [u8; 4] {
    let [a, b] = tag.to_le_bytes();
    let [c, d] = value.to_le_bytes();
    [a, b, c, d]
}

fn direct_image(coordinate: RawTextCoordinate) -> Vec<u8> {
    let mut bytes = direct_record(0x800a, 73).to_vec();
    bytes.extend(coordinate.x.to_le_bytes());
    bytes.extend(coordinate.y.to_le_bytes());
    // Image payload deliberately contains tags that are not record starts.
    bytes.extend(coordinate.width.to_le_bytes());
    bytes.extend(coordinate.height.to_le_bytes());
    for tag in [0x800a, 0x8004, 0x8071, 0xffff] {
        bytes.extend(direct_record(tag, 42));
    }
    bytes
}

#[test]
fn direct_compressed_records_cross_chunks_without_scanning_image_or_tail_payloads() {
    let expected = [point(0x8004, 0xffff), point(17, 39)];
    let mut plain = Vec::new();
    for tag in [0x8001, 0x801c, 0x801d, 0x80ff, 0x8070, 0x8071] {
        plain.extend(direct_record(tag, 99));
    }
    plain.extend(direct_image(expected[0]));
    plain.extend(direct_record(0x1234, 0x800a));
    plain.extend(direct_image(expected[1]));
    plain.extend(direct_record(0x8004, 201));
    plain.extend([0x0a, 0x80, 0x04, 0x80, 0xff]); // Indexed opaque tail.
    for chunk in 1..=31 {
        let mut fixture = direct_fixture(plain.clone(), 2);
        fixture.source.short = 3;
        if chunk % 2 == 0 {
            fixture.header.variant = Variant::HnA;
            fixture.header.page_index.offset = 0x15c;
            fixture.page.row_offset = 0x15c;
        }
        let output = fixture
            .parse(
                Limits {
                    io_chunk_bytes: chunk,
                    ..Limits::default()
                },
                &NeverCancel,
            )
            .unwrap();
        assert_eq!(output.coordinates, expected);
        assert_eq!(output.record_count, 10);
        assert_eq!(output.zlib_frame.unwrap().offset, 528);
        assert_eq!(output.decoded_length, plain.len() as u32);
        assert!(output.max_source_request_bytes <= chunk);
        assert!(output.max_decoder_output_chunk_bytes <= chunk);
    }
}

#[test]
fn direct_record_counts_unknown_tags_and_incomplete_records_are_refused() {
    let image = direct_image(point(4, 8));
    let end = direct_record(0x8004, 0);
    let valid = [image.as_slice(), &end].concat();
    for length in 0..valid.len() {
        assert!(
            direct_fixture(valid[..length].to_vec(), 1)
                .normal()
                .is_err(),
            "accepted prefix {length}"
        );
    }
    for tag in [0x8000, 0x8002, 0x8003, 0xffff] {
        let bytes = [&direct_record(tag, 0)[..], &valid].concat();
        let error = direct_fixture(bytes, 1).normal().unwrap_err();
        assert!(error.to_string().contains("unknown control tag"));
        assert_eq!(
            error.offset,
            Some(528),
            "compressed diagnostics use a source anchor"
        );
    }
    let no_images = direct_fixture(end.to_vec(), 0).normal().unwrap();
    assert!(no_images.coordinates.is_empty());
    assert_eq!(no_images.record_count, 1);
    assert_eq!(direct_fixture(valid, 1).normal().unwrap().record_count, 2);
}

#[test]
fn direct_opaque_tail_stays_bounded_and_requires_the_complete_checksum() {
    let mut plain = direct_image(point(7, 11));
    plain.extend(direct_record(0x8004, 0));
    plain.extend(vec![0xaa; 128 * 1024]);
    let mut fixture = direct_fixture(plain.clone(), 1);
    let output = fixture
        .parse(
            Limits {
                io_chunk_bytes: 17,
                ..Limits::default()
            },
            &NeverCancel,
        )
        .unwrap();
    assert_eq!(output.coordinates, [point(7, 11)]);
    assert!(output.owned_buffer_bytes <= 17 * 2 + size_of::<RawTextCoordinate>() as u64);
    assert_eq!(output.record_count, 2);
    let last = fixture.source.bytes.len() - 1;
    fixture.source.bytes[last] ^= 1;
    assert!(
        fixture
            .normal()
            .unwrap_err()
            .to_string()
            .contains("checksum")
    );
    let mut fixture = direct_fixture(plain, 1);
    fixture.source.bytes.push(0);
    fixture.source.size += 1;
    fixture.page.text.length += 1;
    assert!(
        fixture
            .normal()
            .unwrap_err()
            .to_string()
            .contains("end differs")
    );
}

#[test]
fn direct_frame_cancellation_and_source_faults_return_no_partial_coordinates() {
    let plain = [
        direct_image(point(3, 7)),
        direct_record(0x8004, 0).to_vec(),
        vec![0xab; 97],
    ]
    .concat();
    let mut completed = false;
    for polls in 0..1000 {
        let result = direct_fixture(plain.clone(), 1).parse(
            Limits {
                io_chunk_bytes: 7,
                ..Limits::default()
            },
            &CancelAfter(Cell::new(polls)),
        );
        match result {
            Err(error) => assert_eq!(kind_name(&error), "cancelled"),
            Ok(output) => {
                assert_eq!(output.coordinates, [point(3, 7)]);
                completed = true;
                break;
            }
        }
    }
    assert!(completed);
    for fault in [Fault::Zero, Fault::Overreport, Fault::Error] {
        let mut fixture = direct_fixture(plain.clone(), 1);
        fixture.source.fault = fault;
        fixture.source.fault_at = 536;
        assert!(
            fixture
                .parse(
                    Limits {
                        io_chunk_bytes: 7,
                        ..Limits::default()
                    },
                    &NeverCancel
                )
                .is_err()
        );
    }
}

#[test]
fn raw_image_first_records_reuse_compact_controls() {
    let mut plain = direct_image(point(0xffff, 17));
    for tag in [0x8001, 0x801c, 0x801d, 0x80ff, 0x8071, 0x8070, 0x0042] {
        plain.extend(direct_record(tag, 42));
    }
    plain.extend(direct_image(point(19, 0x8004)));
    plain.extend(direct_record(0x8004, 0));
    plain.extend([0xff; 31]);
    for chunk in 1..=31 {
        let mut f = direct_fixture(plain.clone(), 2);
        f.header.variant = Variant::HnA;
        f.header.page_index.offset = 0x15c;
        f.page.row_offset = 0x15c;
        f.source.bytes.truncate(512);
        f.source.bytes.extend(&plain);
        f.source.size = f.source.bytes.len() as u64;
        f.page.text.length = plain.len() as u64;
        let out = f
            .parse(
                Limits {
                    io_chunk_bytes: chunk,
                    ..Default::default()
                },
                &NeverCancel,
            )
            .unwrap();
        assert_eq!(out.coordinates, [point(0xffff, 17), point(19, 0x8004)]);
        assert_eq!(out.record_count, 10);
        assert_eq!(out.zlib_frame, None);
        assert_eq!(out.max_decoder_output_chunk_bytes, 0);
    }
}

#[test]
fn declared_extents_survive_each_text_framing_and_single_byte_reads() {
    let expected = RawTextCoordinate {
        x: 17,
        y: 29,
        width: 0x8123,
        height: 0x4567,
    };
    let mut raw = direct_image(expected);
    raw.extend(direct_record(0x8004, 0));
    let mut direct = direct_fixture(raw.clone(), 1);
    direct.source.short = 1;
    let mut tagged = Fixture::new(1, &[expected]);
    tagged.source.short = 1;
    assert_eq!(tagged.normal().unwrap().coordinates, [expected]);
    assert_eq!(direct.normal().unwrap().coordinates, [expected]);
    direct.header.variant = Variant::HnA;
    direct.header.page_index.offset = 0x15c;
    direct.page.row_offset = 0x15c;
    direct.source.bytes.truncate(512);
    direct.source.bytes.extend(raw);
    direct.source.size = direct.source.bytes.len() as u64;
    direct.page.text.length = direct.source.size - 512;
    assert_eq!(direct.normal().unwrap().coordinates, [expected]);
}

fn prefixed_raw_fixture() -> Fixture {
    let mut plain = Vec::new();
    for (tag, payload) in [
        (0x8003, 701),
        (0x8003, 907),
        (0x801c, 0),
        (0x80ce, 0),
        (0x8070, 0),
        (0x8071, 0),
        (0x8001, 0),
        (23, 0x800a),
    ] {
        plain.extend(direct_record(tag, payload));
    }
    plain.extend(direct_image(RawTextCoordinate {
        x: 17,
        y: 31,
        width: 701,
        height: 907,
    }));
    plain.extend(direct_record(0x8004, 51));
    plain.extend([0xff; 9]);
    let mut f = direct_fixture(plain.clone(), 1);
    f.header.variant = Variant::HnA;
    f.header.page_index.offset = 0x15c;
    f.page.row_offset = 0x15c;
    f.source.bytes.truncate(512);
    f.source.bytes.extend(&plain);
    f.source.size = f.source.bytes.len() as u64;
    f.page.text.length = plain.len() as u64;
    f
}

#[test]
fn paired_prefix_raw_records_preserve_extents_hashes_and_chunk_bounds() {
    for chunk in 1..=31 {
        let mut f = prefixed_raw_fixture();
        f.source.short = 3;
        let result = f
            .parse(
                Limits {
                    io_chunk_bytes: chunk,
                    ..Default::default()
                },
                &NeverCancel,
            )
            .unwrap();
        assert_eq!(result.page_size, Some([701, 907]));
        assert_eq!(
            result.coordinates,
            [RawTextCoordinate {
                x: 17,
                y: 31,
                width: 701,
                height: 907
            }]
        );
        assert_eq!(result.record_count, 10);
        assert_eq!(result.zlib_frame, None);
        assert!(f.source.max_request <= chunk);
        assert_eq!(result.max_decoder_output_chunk_bytes, 0);
    }
}

#[test]
fn paired_prefix_does_not_admit_unknown_controls_or_other_variants() {
    for (offset, tag) in [(4, 0x8001), (8, 0x8003), (12, 0x80cd), (14, 1)] {
        let mut f = prefixed_raw_fixture();
        f.source.bytes[512 + offset..514 + offset].copy_from_slice(&u16::to_le_bytes(tag));
        assert!(f.normal().is_err(), "offset {offset}");
    }
    let mut f = prefixed_raw_fixture();
    f.header.variant = Variant::C8;
    f.header.page_index.offset = 0x50;
    f.page.row_offset = 0x50;
    assert!(
        f.normal()
            .unwrap_err()
            .to_string()
            .contains("compressed text header")
    );
    for end in [24, 32, 40, 59, 63] {
        let mut f = prefixed_raw_fixture();
        f.source.bytes.truncate(512 + end);
        f.source.size = (512 + end) as u64;
        f.page.text.length = end as u64;
        assert!(f.normal().is_err(), "end {end}");
    }
    let mut f = prefixed_raw_fixture();
    f.source.fault_at = 516;
    f.source.fault = Fault::Error;
    assert!(f.normal().is_err());
}

#[test]
fn prefixed_image_only_and_cancellation_keep_the_same_bounded_contract() {
    let mut image_only = prefixed_raw_fixture();
    image_only.source.bytes.drain(520..544);
    image_only.source.size -= 24;
    image_only.page.text.length -= 24;
    let result = image_only.normal().unwrap();
    assert_eq!(result.record_count, 4);
    assert_eq!(result.coordinates[0].x, 17);
    let mut completed = false;
    for polls in 0..200 {
        let mut fixture = prefixed_raw_fixture();
        match fixture.parse(
            Limits {
                io_chunk_bytes: 3,
                ..Default::default()
            },
            &CancelAfter(Cell::new(polls)),
        ) {
            Err(error) => assert_eq!(kind_name(&error), "cancelled"),
            Ok(report) => {
                assert_eq!(report.coordinates.len(), 1);
                completed = true;
                break;
            }
        }
    }
    assert!(completed);
}

#[test]
fn raw_hna_composition_decodes_only_verified_image_markers() {
    for (marker, x, width, expected_x, expected_width) in [
        (0xd300_u16, 0xc014_u16, 0xc118_u16, 20, 280),
        (0xd300, 20, 280, 20, 280),
        (0xd301, 0xc014, 0xc118, 0xc014, 0xc118),
        (0xd300, 0x8014, 0xc118, 0x8014, 0xc118),
        (0xd300, 0xc014, 0x8118, 0xc014, 0x8118),
    ] {
        for chunk in [1, 3, 31] {
            let mut f = prefixed_raw_fixture();
            let image = 512 + 32;
            f.source.bytes[image + 2..image + 4].copy_from_slice(&marker.to_le_bytes());
            f.source.bytes[image + 4..image + 6].copy_from_slice(&x.to_le_bytes());
            f.source.bytes[image + 8..image + 10].copy_from_slice(&width.to_le_bytes());
            f.source.short = 2;
            let composed = read_coordinates(
                &mut f.source,
                f.header,
                f.page,
                &Limits {
                    io_chunk_bytes: chunk,
                    ..Limits::default()
                },
                &NeverCancel,
            )
            .unwrap();
            assert_eq!(
                composed.coordinates,
                [RawTextCoordinate {
                    x: expected_x,
                    y: 31,
                    width: expected_width,
                    height: 907,
                }]
            );
            assert!(composed.max_source_request_bytes <= chunk);
        }
    }
}

#[test]
fn compressed_hna_markers_preserve_other_profiles() {
    for (tag, marker, x, width, expected_x, expected_width) in [
        (0x800a_u16, 0xd300_u16, 0xc014_u16, 0xc118_u16, 20, 280),
        (0x800a, 0xd300, 20, 280, 20, 280),
        (0x800b, 0xd300, 0xc014, 0xc118, 0xc014, 0xc118),
        (0x800a, 0xd301, 0xc014, 0xc118, 0xc014, 0xc118),
        (0x800a, 0xd300, 0x8014, 0xc118, 0x8014, 0xc118),
        (0x800a, 0xd300, 0xc014, 0x8118, 0xc014, 0x8118),
    ] {
        for variant in [Variant::HnA, Variant::C8] {
            for chunk in [1, 3, 31] {
                let raw = RawTextCoordinate {
                    x,
                    y: 31,
                    width,
                    height: 907,
                };
                let mut f = Fixture::new(1, &[raw, raw]);
                f.header.variant = variant;
                if variant == Variant::HnA {
                    f.header.page_index.offset = 0x15c;
                    f.page.row_offset = 0x15c;
                }
                for start in [24, 52] {
                    f.plain[start..start + 2].copy_from_slice(&tag.to_le_bytes());
                    f.plain[start + 2..start + 4].copy_from_slice(&marker.to_le_bytes());
                }
                f.recompress();
                f.source.short = 2;
                let composed = read_coordinates(
                    &mut f.source,
                    f.header,
                    f.page,
                    &Limits {
                        io_chunk_bytes: chunk,
                        ..Limits::default()
                    },
                    &NeverCancel,
                )
                .unwrap();
                let expected = if variant == Variant::HnA {
                    RawTextCoordinate {
                        x: expected_x,
                        width: expected_width,
                        ..raw
                    }
                } else {
                    raw
                };
                assert_eq!(composed.coordinates, [expected, expected]);
                assert!(composed.max_source_request_bytes <= chunk);
            }
        }
    }
}
