// SPDX-License-Identifier: MIT

use caj2pdf_core::{
    Cancellation, Limits, RangedSource,
    hnc8::{
        ErrorKind, Hnc8Error, Hnc8Reader, ImageRecord, JpegColor, JpegInfo, Span, Variant,
        read_type2_jpeg_info,
    },
};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Source {
    bytes: Vec<u8>,
    size: u64,
    max_read: usize,
    zero_at: Option<u64>,
    overreport: bool,
    max_request: usize,
    reads: usize,
}

impl Source {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            size: bytes.len() as u64,
            bytes,
            max_read: usize::MAX,
            zero_at: None,
            overreport: false,
            max_request: 0,
            reads: 0,
        }
    }
}

impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.size
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> caj2pdf_core::Result<usize> {
        self.reads += 1;
        self.max_request = self.max_request.max(destination.len());
        if self.overreport {
            return Ok(destination.len() + 1);
        }
        if self.zero_at.is_some_and(|at| offset >= at) {
            return Ok(0);
        }
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let count = self
            .bytes
            .len()
            .saturating_sub(start)
            .min(destination.len())
            .min(self.max_read)
            .min(self.zero_at.map_or(usize::MAX, |at| {
                usize::try_from(at.saturating_sub(offset)).unwrap_or(usize::MAX)
            }));
        if count > 0 {
            destination[..count].copy_from_slice(&self.bytes[start..start + count]);
        }
        Ok(count)
    }
}

struct Flag {
    queries: AtomicUsize,
    cancel_at_query: usize,
}
impl Flag {
    const fn new(cancel_at_query: usize) -> Self {
        Self {
            queries: AtomicUsize::new(0),
            cancel_at_query,
        }
    }
}
impl Cancellation for Flag {
    fn is_cancelled(&self) -> bool {
        self.queries.fetch_add(1, Ordering::Relaxed) + 1 >= self.cancel_at_query
    }
}
static NEVER: Flag = Flag::new(usize::MAX);

fn segment(marker: u8, body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0xff, marker];
    bytes.extend_from_slice(&u16::try_from(body.len() + 2).unwrap().to_be_bytes());
    bytes.extend_from_slice(body);
    bytes
}

fn base_jpeg(components: u8) -> Vec<u8> {
    let mut bytes = vec![0xff, 0xd8];
    bytes.extend(segment(0xe0, b"JFIF\0\x01\x01\x00\x00\x01\x00\x01\x00\x00"));
    bytes.extend(segment(0xfe, b"original synthetic JPEG profile"));
    bytes.extend(segment(0xdb, &[&[0_u8][..], &[1_u8; 64][..]].concat()));
    let mut frame = vec![8, 0, 7, 0, 9, components];
    for id in 1..=components {
        frame.extend([id, if id == 1 { 0x21 } else { 0x11 }, 0]);
    }
    bytes.extend(segment(0xc0, &frame));
    let mut huffman = Vec::new();
    for selector in [0x00, 0x10] {
        huffman.push(selector);
        huffman.extend([1]);
        huffman.extend([0; 15]);
        huffman.push(0);
    }
    bytes.extend(segment(0xc4, &huffman));
    let mut scan = vec![components];
    for id in 1..=components {
        scan.extend([id, 0]);
    }
    scan.extend([0, 63, 0]);
    bytes.extend(segment(0xda, &scan));
    bytes.extend([0x51, 0xff, 0x00, 0x2b, 0xff, 0xd9]);
    bytes
}

fn marker_offset(bytes: &[u8], marker: u8) -> usize {
    bytes
        .windows(2)
        .position(|pair| pair == [0xff, marker])
        .unwrap()
}

fn record_for(payload: &[u8]) -> (Source, ImageRecord) {
    let mut bytes = vec![0; 32];
    bytes.extend_from_slice(payload);
    let record = ImageRecord {
        page_number: 1,
        image_number: 1,
        descriptor_offset: 0,
        record_type: 2,
        payload: Span {
            offset: 32,
            length: payload.len() as u64,
        },
    };
    (Source::new(bytes), record)
}

fn parse_with(
    source: &mut Source,
    record: ImageRecord,
    limits: Limits,
    cancel: &Flag,
) -> Result<JpegInfo, Hnc8Error> {
    read_type2_jpeg_info(source, record, &limits, cancel)
}

fn parse(payload: &[u8]) -> Result<JpegInfo, Hnc8Error> {
    let (mut source, record) = record_for(payload);
    parse_with(&mut source, record, Limits::default(), &NEVER)
}

fn format_source(variant: Variant, first: &[u8], last: &[u8]) -> Source {
    let (index, count_offset) = match variant {
        Variant::C8 => (0x50, 8),
        Variant::HnA => (0x15c, 0x90),
        Variant::HnB => (0xd8, 0x90),
    };
    let mut bytes = vec![0; 640];
    match variant {
        Variant::C8 => bytes[..4].copy_from_slice(&[0xc8, 0, 0, 0]),
        Variant::HnA => {
            bytes[..8].copy_from_slice(&[b'H', b'N', 0, 0, 0x90, 1, 0, 0]);
        }
        Variant::HnB => {
            bytes[..8].copy_from_slice(&[b'H', b'N', 0, 0, 0xc8, 0, 0, 0]);
            bytes[0x88..0x8c].copy_from_slice(&0xc8_u32.to_le_bytes());
        }
    }
    bytes[count_offset..count_offset + 4].copy_from_slice(&1_i32.to_le_bytes());
    bytes[index..index + 4].copy_from_slice(&620_i32.to_le_bytes());
    bytes[index + 4..index + 8].copy_from_slice(&0_i32.to_le_bytes());
    bytes[index + 8..index + 10].copy_from_slice(&2_i16.to_le_bytes());
    bytes[620..624].copy_from_slice(&2_i32.to_le_bytes());
    bytes[624..628].copy_from_slice(&640_i32.to_le_bytes());
    bytes[628..632].copy_from_slice(&(first.len() as i32).to_le_bytes());
    bytes.extend_from_slice(first);
    let second_descriptor = bytes.len();
    bytes.extend_from_slice(&2_i32.to_le_bytes());
    bytes.extend_from_slice(&((second_descriptor + 12) as i32).to_le_bytes());
    bytes.extend_from_slice(&(last.len() as i32).to_le_bytes());
    bytes.extend_from_slice(last);
    Source::new(bytes)
}

#[test]
fn checked_first_and_last_images_in_each_container_profile_are_bounded() {
    for variant in [Variant::HnA, Variant::HnB, Variant::C8] {
        let gray = base_jpeg(1);
        let color = base_jpeg(3);
        let mut source = format_source(variant, &gray, &color);
        source.max_read = 1;
        let limits = Limits {
            io_chunk_bytes: 7,
            ..Limits::default()
        };
        let mut reader = Hnc8Reader::open(&mut source, &limits, &NEVER).unwrap();
        assert_eq!(reader.header().variant, variant);
        assert_eq!(reader.next_page().unwrap().unwrap().image_count, 2);
        let first = reader.next_image().unwrap().unwrap();
        let first_info = read_type2_jpeg_info(reader.source_mut(), first, &limits, &NEVER).unwrap();
        assert_eq!((first_info.width, first_info.height), (9, 7));
        assert_eq!(
            (first_info.components, first_info.color),
            (1, JpegColor::Gray)
        );
        assert!(first_info.app0_jfif);
        assert_eq!(first_info.scans, 1);
        assert_eq!(first_info.payload, first.payload);
        let last = reader.next_image().unwrap().unwrap();
        let last_info = read_type2_jpeg_info(reader.source_mut(), last, &limits, &NEVER).unwrap();
        assert_eq!(
            (last_info.components, last_info.color),
            (3, JpegColor::Ycbcr)
        );
        assert_eq!(reader.next_image().unwrap(), None);
        assert!(source.max_request <= 7);
        assert!(source.reads > gray.len() + color.len());
    }
}

#[test]
fn restart_markers_are_recognized_and_app14_is_unsupported() {
    let mut bytes = base_jpeg(3);
    let dri = segment(0xdd, &[0, 1]);
    let sos = marker_offset(&bytes, 0xda);
    bytes.splice(sos..sos, dri);
    let eoi = marker_offset(&bytes, 0xd9);
    bytes.splice(eoi..eoi, [0xff, 0xd0, 0x44, 0xff, 0xd1]);
    let info = parse(&bytes).unwrap();
    assert_eq!(info.color, JpegColor::Ycbcr);
    assert_eq!(info.restart_interval, Some(1));
    assert!(info.app0_jfif);
    bytes.splice(20..20, segment(0xee, b"test"));
    let error = parse(&bytes).unwrap_err();
    assert!(matches!(
        error.kind,
        ErrorKind::Unsupported {
            field: "JPEG APP14 marker",
            value: 0xee
        }
    ));
}

#[test]
fn malformed_and_unsupported_markers_have_absolute_offsets() {
    let original = base_jpeg(3);
    let mut cases: Vec<(Vec<u8>, usize, &'static str)> = Vec::new();

    let mut bad = original.clone();
    bad[1] = 0xd9;
    cases.push((bad, 0, "malformed"));
    let mut bad = original.clone();
    bad[2] = 0x11;
    cases.push((bad, 2, "malformed"));
    let mut bad = original.clone();
    bad[3] = 0;
    cases.push((bad, 2, "malformed"));
    let mut bad = original.clone();
    let at = marker_offset(&bad, 0xc0);
    bad[at + 2..at + 4].copy_from_slice(&1_u16.to_be_bytes());
    cases.push((bad, at + 2, "malformed"));
    let mut bad = original.clone();
    let at = marker_offset(&bad, 0xc0);
    bad[at + 1] = 0xc2;
    cases.push((bad, at, "unsupported"));
    let mut bad = original.clone();
    let at = marker_offset(&bad, 0xc0);
    bad[at + 1] = 0xc9;
    cases.push((bad, at, "unsupported"));
    let mut bad = original.clone();
    let at = marker_offset(&bad, 0xc0);
    bad[at + 1] = 0xdc;
    cases.push((bad, at, "unsupported"));
    let mut bad = original.clone();
    let at = marker_offset(&bad, 0xc0);
    bad[at + 1] = 0x01;
    cases.push((bad, at, "unsupported"));
    let mut bad = original.clone();
    let at = marker_offset(&bad, 0xc0);
    bad.splice(at..at, [0xff, 0xd0]);
    cases.push((bad, at, "malformed"));
    let mut bad = original.clone();
    let at = marker_offset(&bad, 0xda);
    bad.splice(at..at, [0xff, 0xd9]);
    cases.push((bad, at, "malformed"));
    let mut bad = original.clone();
    let at = marker_offset(&bad, 0xda);
    bad.splice(at..at, [0xff, 0xda, 0, 2]);
    cases.push((bad, at + 4, "truncated"));
    let mut bad = original.clone();
    let at = marker_offset(&bad, 0xd9);
    bad.splice(at..at, [0xff, 0xda]);
    cases.push((bad, at, "unsupported"));
    let mut bad = original.clone();
    bad.push(0);
    cases.push((bad, original.len(), "malformed"));
    let mut bad = original.clone();
    bad.pop();
    cases.push((bad.clone(), bad.len(), "truncated"));

    for (index, (bytes, offset, expected)) in cases.into_iter().enumerate() {
        let error = parse(&bytes).unwrap_err();
        assert_eq!(error.offset, offset as u64 + 32, "case {index}: {error}");
        assert_eq!(error.kind.as_str(), expected, "case {index}: {error}");
        assert_eq!((error.page, error.image), (Some(1), Some(1)));
    }
}

#[test]
fn frame_and_scan_fields_reject_unsupported_or_conflicting_profiles() {
    let original = base_jpeg(3);
    let sof = marker_offset(&original, 0xc0);
    let sos = marker_offset(&original, 0xda);
    let mut cases: Vec<(Vec<u8>, usize, &'static str)> = Vec::new();
    for (index, replacement, kind) in [
        (4, 12, "unsupported"),
        (6, 0, "unsupported"),
        (8, 0, "malformed"),
        (9, 4, "malformed"),
        (13, 1, "malformed"),
        (11, 0, "malformed"),
        (12, 4, "unsupported"),
    ] {
        let mut bad = original.clone();
        bad[sof + index] = replacement;
        let at = if index == 6 || index == 8 {
            sof + index - 1
        } else {
            sof + index
        };
        cases.push((bad, at, kind));
    }
    let mut bad = original.clone();
    bad[sof + 11] = 0x44;
    cases.push((bad, sof, "malformed"));
    let mut bad = original.clone();
    bad[sos + 4] = 2;
    cases.push((bad, sos + 4, "malformed"));
    let mut bad = original.clone();
    bad[sos + 2..sos + 4].copy_from_slice(&10_u16.to_be_bytes());
    bad[sos + 4] = 2;
    bad.drain(sos + 9..sos + 11);
    cases.push((bad, sos + 4, "unsupported"));
    let mut bad = original.clone();
    bad[sos + 5] = 2;
    cases.push((bad, sos + 5, "malformed"));
    let mut bad = original.clone();
    bad[sos + 6] = 0x40;
    cases.push((bad, sos + 6, "unsupported"));
    let mut bad = original.clone();
    bad[sos + 11] = 1;
    cases.push((bad, sos + 11, "unsupported"));
    let mut bad = original.clone();
    bad[sof + 12] = 1;
    cases.push((bad, sos + 5, "malformed"));

    for (index, (bytes, offset, kind)) in cases.into_iter().enumerate() {
        let error = parse(&bytes).unwrap_err();
        assert_eq!(error.kind.as_str(), kind, "case {index}: {error}");
        assert_eq!(error.offset, offset as u64 + 32, "case {index}: {error}");
    }
}

#[test]
fn color_ambiguity_and_duplicate_application_markers_are_explicit() {
    let original = base_jpeg(3);
    let app0 = marker_offset(&original, 0xe0);
    let mut unknown = original.clone();
    unknown.drain(app0..app0 + 18);
    let error = parse(&unknown).unwrap_err();
    assert!(matches!(
        error.kind,
        ErrorKind::Unsupported {
            field: "JPEG color transform",
            ..
        }
    ));
    let mut duplicate = original.clone();
    duplicate.splice(2..2, segment(0xe0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0"));
    let error = parse(&duplicate).unwrap_err();
    assert!(matches!(
        error.kind,
        ErrorKind::Malformed {
            field: "JFIF APP0",
            ..
        }
    ));
    let mut misplaced = unknown.clone();
    let jfif = segment(0xe0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
    misplaced.splice(2..2, segment(0xfe, b"first"));
    misplaced.splice(11..11, jfif);
    let error = parse(&misplaced).unwrap_err();
    assert!(matches!(
        error.kind,
        ErrorKind::Unsupported {
            field: "JFIF APP0 placement",
            ..
        }
    ));
    let mut app14 = original.clone();
    app14.splice(20..20, segment(0xee, b"test"));
    let error = parse(&app14).unwrap_err();
    assert!(matches!(
        error.kind,
        ErrorKind::Unsupported {
            field: "JPEG APP14 marker",
            ..
        }
    ));
}

#[test]
fn checked_span_identity_limits_cancellation_and_disrupted_reads() {
    let jpeg = base_jpeg(1);
    let (mut source, record) = record_for(&jpeg);
    let mut bad = record;
    bad.page_number = 0;
    assert_eq!(
        parse_with(&mut source, bad, Limits::default(), &NEVER)
            .unwrap_err()
            .kind
            .field(),
        "image identity"
    );
    bad = record;
    bad.image_number = 0;
    assert_eq!(
        parse_with(&mut source, bad, Limits::default(), &NEVER)
            .unwrap_err()
            .kind
            .field(),
        "image identity"
    );
    bad = record;
    bad.record_type = 0;
    assert!(matches!(
        parse_with(&mut source, bad, Limits::default(), &NEVER)
            .unwrap_err()
            .kind,
        ErrorKind::Unsupported {
            field: "image type",
            ..
        }
    ));
    bad = record;
    bad.descriptor_offset = u64::MAX;
    assert_eq!(
        parse_with(&mut source, bad, Limits::default(), &NEVER)
            .unwrap_err()
            .kind
            .field(),
        "image descriptor"
    );
    bad = record;
    bad.payload.offset = 11;
    assert_eq!(
        parse_with(&mut source, bad, Limits::default(), &NEVER)
            .unwrap_err()
            .kind
            .field(),
        "image payload"
    );
    bad = record;
    bad.payload.length = 0;
    assert_eq!(
        parse_with(&mut source, bad, Limits::default(), &NEVER)
            .unwrap_err()
            .kind
            .field(),
        "image payload"
    );
    bad = record;
    bad.payload.length = u64::MAX;
    assert_eq!(
        parse_with(&mut source, bad, Limits::default(), &NEVER)
            .unwrap_err()
            .kind
            .field(),
        "image payload"
    );
    bad = record;
    bad.payload.length += 1;
    assert!(matches!(
        parse_with(&mut source, bad, Limits::default(), &NEVER)
            .unwrap_err()
            .kind,
        ErrorKind::Truncated { .. }
    ));

    let limits = Limits {
        max_input_bytes: source.size() - 1,
        ..Limits::default()
    };
    assert!(matches!(
        parse_with(&mut source, record, limits, &NEVER)
            .unwrap_err()
            .kind,
        ErrorKind::LimitExceeded {
            resource: "source bytes",
            ..
        }
    ));
    let limits = Limits {
        io_chunk_bytes: 1,
        max_allocation_bytes: 1,
        ..Limits::default()
    };
    assert!(matches!(
        parse_with(&mut source, record, limits, &NEVER)
            .unwrap_err()
            .kind,
        ErrorKind::LimitExceeded {
            resource: "JPEG payload bytes",
            limit: 1,
            ..
        }
    ));
    assert!(matches!(
        parse_with(&mut source, record, Limits::default(), &Flag::new(0))
            .unwrap_err()
            .kind,
        ErrorKind::Cancelled
    ));
    source.zero_at = Some(36);
    let error = parse_with(&mut source, record, Limits::default(), &NEVER).unwrap_err();
    assert_eq!(error.offset, 36);
    assert!(matches!(error.kind, ErrorKind::Truncated { .. }));
    source.zero_at = None;
    source.overreport = true;
    assert!(matches!(
        parse_with(&mut source, record, Limits::default(), &NEVER)
            .unwrap_err()
            .kind,
        ErrorKind::Source { .. }
    ));
    source.overreport = false;
    assert!(matches!(
        parse_with(&mut source, record, Limits::default(), &Flag::new(3))
            .unwrap_err()
            .kind,
        ErrorKind::Cancelled
    ));
}

#[test]
fn application_fields_and_repeated_legal_markers_are_checked() {
    let original = base_jpeg(3);
    let app = marker_offset(&original, 0xe0);
    let mut version_102 = original.clone();
    version_102[app + 10] = 2;
    assert_eq!(parse(&version_102).unwrap().color, JpegColor::Ycbcr);

    let mut thumb = original.clone();
    thumb[app + 16] = 1;
    thumb[app + 17] = 1;
    thumb[app + 2..app + 4].copy_from_slice(&19_u16.to_be_bytes());
    thumb.splice(app + 18..app + 18, [1, 2, 3]);
    assert_eq!(parse(&thumb).unwrap().color, JpegColor::Ycbcr);

    let mut fill = original.clone();
    fill.splice(2..2, [0xff]);
    assert_eq!(parse(&fill).unwrap().color, JpegColor::Ycbcr);

    let mut repeated = original.clone();
    let sos = marker_offset(&repeated, 0xda);
    repeated.splice(sos..sos, segment(0xdd, &[0, 0]));
    let sos = marker_offset(&repeated, 0xda);
    repeated.splice(sos..sos, segment(0xdd, &[0, 1]));
    assert_eq!(parse(&repeated).unwrap().restart_interval, Some(1));

    let mut no_jfif_gray = base_jpeg(1);
    no_jfif_gray.drain(app..app + 18);
    no_jfif_gray.splice(2..2, segment(0xe0, b"OTHER"));
    no_jfif_gray.splice(2..2, segment(0xed, b"OTHER"));
    no_jfif_gray.splice(2..2, segment(0xe1, b"tiny"));
    assert_eq!(parse(&no_jfif_gray).unwrap().color, JpegColor::Gray);

    let mut invalid_cases = Vec::new();
    let mut bad = original.clone();
    bad[app + 9] = 2;
    invalid_cases.push((bad, "JFIF version or density unit"));
    let mut bad = original.clone();
    bad[app + 11] = 3;
    invalid_cases.push((bad, "JFIF version or density unit"));
    let mut bad = original.clone();
    bad[app + 13] = 0;
    bad[app + 14] = 0;
    invalid_cases.push((bad, "JFIF density"));
    let mut bad = original.clone();
    bad[app + 16] = 1;
    bad[app + 17] = 1;
    invalid_cases.push((bad, "JFIF thumbnail"));
    let mut bad = original.clone();
    bad[app + 2..app + 4].copy_from_slice(&7_u16.to_be_bytes());
    invalid_cases.push((bad, "JFIF APP0 fields"));
    for (index, (bytes, field)) in invalid_cases.into_iter().enumerate() {
        let error = parse(&bytes).unwrap_err();
        assert_eq!(error.kind.field(), field, "case {index}: {error}");
    }
}

#[test]
fn table_segments_frame_order_and_entropy_failures_are_located() {
    let original = base_jpeg(3);
    let sof = marker_offset(&original, 0xc0);
    let dqt = marker_offset(&original, 0xdb);
    let dht = marker_offset(&original, 0xc4);
    let sos = marker_offset(&original, 0xda);
    let eoi = marker_offset(&original, 0xd9);
    let mut cases: Vec<(Vec<u8>, &'static str)> = Vec::new();

    let mut bad = original.clone();
    bad[dqt + 2..dqt + 4].copy_from_slice(&2_u16.to_be_bytes());
    cases.push((bad, "JPEG DQT"));
    let mut bad = original.clone();
    bad[dqt + 4] = 0x10;
    cases.push((bad, "JPEG DQT selector"));
    let mut bad = original.clone();
    bad[dqt + 2..dqt + 4].copy_from_slice(&3_u16.to_be_bytes());
    cases.push((bad, "JPEG DQT values"));
    let mut bad = original.clone();
    bad[dht + 2..dht + 4].copy_from_slice(&2_u16.to_be_bytes());
    cases.push((bad, "JPEG DHT"));
    let mut bad = original.clone();
    bad[dht + 4] = 0x20;
    cases.push((bad, "JPEG DHT selector"));
    let mut bad = original.clone();
    bad[dht + 2..dht + 4].copy_from_slice(&3_u16.to_be_bytes());
    cases.push((bad, "JPEG DHT counts"));
    let mut bad = original.clone();
    bad[dht + 5] = 255;
    bad[dht + 6] = 2;
    cases.push((bad, "JPEG DHT"));
    let mut bad = original.clone();
    bad.splice(
        sos..sos,
        segment(0xc4, &[0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
    );
    cases.push((bad, "JPEG DHT symbols"));
    let mut bad = original.clone();
    bad.splice(sos..sos, segment(0xdd, &[0]));
    cases.push((bad, "JPEG DRI"));
    let mut bad = original.clone();
    bad.splice(sof..sof, segment(0xda, &[1, 1, 0, 0, 63, 0]));
    cases.push((bad, "JPEG SOS"));
    let mut bad = original.clone();
    let frame = bad[sof..sof + 19].to_vec();
    bad.splice(sos..sos, frame);
    cases.push((bad, "JPEG frame"));
    let mut bad = base_jpeg(4);
    cases.push((bad.clone(), "JPEG components"));
    bad.splice(eoi..eoi, [0xff, 0xdc]);
    cases.push((bad, "JPEG components"));
    let mut bad = original.clone();
    bad.splice(eoi..eoi, [0xff, 0xdc]);
    cases.push((bad, "JPEG DNL marker"));
    let mut bad = original.clone();
    bad.splice(eoi..eoi, [0xff, 0xd8]);
    cases.push((bad, "JPEG entropy marker"));
    let mut bad = original.clone();
    bad.splice(eoi..eoi, [0xff, 0xd0]);
    cases.push((bad, "JPEG restart"));
    let mut bad = original.clone();
    bad.splice(sos..sos, segment(0xdd, &[0, 1]));
    let insertion = marker_offset(&bad, 0xd9);
    bad.splice(insertion..insertion, [0xff, 0xd1]);
    cases.push((bad, "JPEG restart"));
    let mut bad = original.clone();
    bad.splice(sos..sos, segment(0xdd, &[0, 0]));
    let insertion = marker_offset(&bad, 0xd9);
    bad.splice(insertion..insertion, [0xff, 0xd0]);
    cases.push((bad, "JPEG restart"));
    let mut bad = original.clone();
    bad.splice(eoi..eoi, [0xff, 0xff, 0x42]);
    cases.push((bad, "JPEG entropy marker"));
    let mut bad = original.clone();
    bad.splice(eoi..eoi, [0xff, 0xff, 0x00]);
    cases.push((bad, "JPEG entropy marker"));
    let mut bad = original.clone();
    bad.splice(eoi..eoi, [0xff, 0xe1]);
    cases.push((bad, "JPEG multiple scans"));
    let mut bad = original.clone();
    bad.splice(eoi..eoi, [0xff, 0xfe]);
    cases.push((bad, "JPEG multiple scans"));

    for (index, (bytes, field)) in cases.into_iter().enumerate() {
        let error = parse(&bytes).unwrap_err();
        assert_eq!(error.kind.field(), field, "case {index}: {error}");
    }

    let mut truncated_segment = original.clone();
    truncated_segment.truncate(app_boundary(&original));
    let error = parse(&truncated_segment).unwrap_err();
    assert!(matches!(error.kind, ErrorKind::Truncated { .. }));
}

fn app_boundary(jpeg: &[u8]) -> usize {
    marker_offset(jpeg, 0xe0) + 7
}
