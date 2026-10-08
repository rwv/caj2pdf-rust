// SPDX-License-Identifier: MIT

use super::*;
use crate::native::SeekableSource;
use crate::test_support::pdf_class;
use crate::test_support::{CancelAfter, NEVER};
use recovery::blank_fragment_page;
use std::io::Cursor;

/// Parse one planned object span and frame it with `resolve_length`.
fn inspect_fragment_object<S: RangedSource, C: Cancellation>(
    source: &mut S,
    range: PdfRange,
    expected: PdfRef,
    limits: &Limits,
    cancellation: &C,
    resolve_length: impl Fn(PdfRef) -> Option<u64>,
) -> Result<FragmentInspection> {
    let planned = parse_planned_object(source, range, expected, limits, cancellation)?;
    finish_planned_object(
        source,
        range,
        expected,
        planned,
        limits,
        cancellation,
        resolve_length,
    )
}

/// The integer value of one planned object span.
fn inspect_fragment_scalar<S: RangedSource, C: Cancellation>(
    source: &mut S,
    range: PdfRange,
    expected: PdfRef,
    limits: &Limits,
    cancellation: &C,
) -> Result<Option<u64>> {
    let planned = parse_planned_object(source, range, expected, limits, cancellation)?;
    Ok(planned.scalar)
}

/// A classic-xref PDF with a binary-marker header comment and a trailer of
/// `/Size`, `/Root 1 0 R`, and `trailer_extra`.
fn build_pdf(objects: &[(u32, &str)], trailer_extra: &str) -> Vec<u8> {
    let highest = objects.iter().map(|(number, _)| *number).max().unwrap_or(0);
    let trailer = format!("<< /Size {} /Root 1 0 R {trailer_extra} >>", highest + 1);
    pdf_with_header(
        b"%PDF-1.7\n%\x80\x81\x82\x83\n",
        &ungapped(objects),
        &trailer,
    )
}

fn open_cancellable(
    bytes: Vec<u8>,
    limits: &Limits,
    cancellation: &impl Cancellation,
) -> Result<PdfIndex> {
    let len = bytes.len() as u64;
    let mut source = SeekableSource::new(Cursor::new(bytes))?;
    PdfIndex::open(
        &mut source,
        PdfRange {
            offset: 0,
            length: len,
        },
        limits,
        cancellation,
    )
}

fn open_with(bytes: Vec<u8>, limits: &Limits) -> Result<PdfIndex> {
    open_cancellable(bytes, limits, &NEVER)
}

fn open(bytes: Vec<u8>) -> Result<PdfIndex> {
    open_with(bytes, &Limits::default())
}

fn base_pdf(page: &str, pages: &str, catalog: &str) -> Vec<u8> {
    build_pdf(&[(1, catalog), (2, pages), (3, page)], "")
}

#[test]
fn stream_crlf_across_object_head_reads_preserves_payload_extent() {
    for boundary in [512, 1024] {
        for separator in ["\r\n", "\r"] {
            let prefix = "<< /Length 3 >>";
            let padding = boundary - "4 0 obj\n".len() - prefix.len() - "stream\r".len();
            let stream = format!(
                "{prefix}{}stream{separator}abc\nendstream",
                " ".repeat(padding)
            );
            let bytes = build_pdf(
                &[
                    (1, "<< /Type /Catalog /Pages 2 0 R >>"),
                    (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
                    (
                        3,
                        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Contents 4 0 R >>",
                    ),
                    (4, &stream),
                ],
                "",
            );
            assert_eq!(open(bytes.clone()).unwrap().pages.len(), 1);
            let mut invalid = bytes;
            replace_once(&mut invalid, b"/Length 3", b"/Length 2");
            expect_pdf_error(invalid, "malformed");
        }
    }
}

fn expect_pdf_error(bytes: Vec<u8>, kind: &str) {
    let error = pdf_error(open(bytes));
    assert!(pdf_class(&error) == Some(kind), "{error:?}");
}

fn indirect_flate_fragment(encoded: &[u8], integer: &str) -> Vec<u8> {
    let mut bytes = b"1 0 obj\n<< /Length 2 0 R /Filter /FlateDecode >>\nstream\n".to_vec();
    bytes.extend_from_slice(encoded);
    bytes.extend_from_slice(format!("\nendstream\nendobj\n2 0 obj\n{integer}\nendobj").as_bytes());
    bytes
}

fn scan_indirect(
    bytes: Vec<u8>,
    limits: &Limits,
    cancellation: &CancelAfter,
) -> Result<fragment_scan::FragmentScan> {
    let size = bytes.len() as u64;
    let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
    scan_fragment_with_candidates(&mut source, 0, size, limits, cancellation, &mut [])
}

#[test]
fn indirect_flate_length_resolves_forward_and_backward_integer_objects() {
    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::none());
    encoder
        .write_all(b"original payload with endstream endobj 2 0 obj 0 endobj markers")
        .unwrap();
    let encoded = encoder.finish().unwrap();
    assert!(encoded.windows(9).any(|bytes| bytes == b"endstream"));
    let integer = encoded.len().to_string();
    let forward = indirect_flate_fragment(&encoded, &integer);
    let boundary = forward
        .windows(7)
        .rposition(|bytes| bytes == b"2 0 obj")
        .unwrap();
    let mut backward = forward[boundary..].to_vec();
    backward.push(b'\n');
    backward.extend_from_slice(&forward[..boundary]);
    for bytes in [forward, backward] {
        let scan = scan_indirect(
            bytes,
            &Limits {
                io_chunk_bytes: 1,
                ..Limits::default()
            },
            &NEVER,
        )
        .unwrap();
        assert_eq!(scan.objects.len(), 2);
        assert!(scan.patches.is_empty());
        let stream = scan
            .objects
            .iter()
            .find(|scanned| scanned.object.reference.number == 1)
            .unwrap();
        assert!(stream.object.range.length > encoded.len() as u64);
    }
}

#[test]
fn indirect_flate_length_rejects_missing_cyclic_wrong_and_noninteger_targets() {
    let encoded = zlib(b"original");
    for integer in ["1 0 R", "2 0 R", "null", "-1", "3.5", "0", "<<>>"] {
        let error = scan_indirect(
            indirect_flate_fragment(&encoded, integer),
            &Limits::default(),
            &NEVER,
        )
        .err()
        .unwrap();
        assert!(
            matches!(
                error,
                Error {
                    kind: ErrorKind::Malformed,
                    context: Context::Pdf { repair: false, .. },
                    ..
                }
            ),
            "{error:?}"
        );
    }
    let mut missing = indirect_flate_fragment(&encoded, &encoded.len().to_string());
    missing.truncate(find(&missing, b"2 0 obj") as usize);
    assert!(scan_indirect(missing, &Limits::default(), &NEVER).is_err());
    let mut bad_value = indirect_flate_fragment(&encoded, &encoded.len().to_string());
    replace_once(&mut bad_value, b"/Length 2 0 R", b"/Length null ");
    assert!(scan_indirect(bad_value, &Limits::default(), &NEVER).is_err());
    let mut duplicate = indirect_flate_fragment(&encoded, &encoded.len().to_string());
    duplicate.extend_from_slice(b"\n2 0 obj 1 endobj");
    assert!(scan_indirect(duplicate, &Limits::default(), &NEVER).is_err());
}

#[test]
fn indirect_lengths_frame_streams_without_decoding_them() {
    let encoded = zlib(&vec![b'x'; 20_000]);
    let bytes = indirect_flate_fragment(&encoded, &encoded.len().to_string());
    // No inflate work is done, so no output budget is spent on framing.
    let tight = Limits {
        max_output_bytes: 1,
        ..Limits::default()
    };
    assert_eq!(
        scan_indirect(bytes.clone(), &tight, &NEVER)
            .unwrap()
            .objects
            .len(),
        2
    );
    let mut two_streams = bytes.clone();
    let mut second = bytes.clone();
    replace_once(&mut second, b"1 0 obj", b"3 0 obj");
    replace_once(&mut second, b"2 0 R", b"4 0 R");
    replace_once(&mut second, b"2 0 obj", b"4 0 obj");
    two_streams.push(b'\n');
    two_streams.extend_from_slice(&second);
    assert_eq!(
        scan_indirect(two_streams, &tight, &NEVER)
            .unwrap()
            .objects
            .len(),
        4
    );
    // Payload bytes are opaque: a bad checksum, a truncated codec stream or
    // non-zlib bytes frame when the Length object agrees with endstream.
    let mut bad_checksum = encoded.clone();
    *bad_checksum.last_mut().unwrap() ^= 1;
    for payload in [
        bad_checksum.as_slice(),
        &encoded[..encoded.len() - 1],
        b"invalid zlib bytes".as_slice(),
    ] {
        let bytes = indirect_flate_fragment(payload, &payload.len().to_string());
        assert!(scan_indirect(bytes, &Limits::default(), &NEVER).is_ok());
    }
    // A Length that disagrees with every endstream, or no endstream, fails.
    let disagreeing = indirect_flate_fragment(&encoded, &(encoded.len() - 2).to_string());
    assert!(scan_indirect(disagreeing, &Limits::default(), &NEVER).is_err());
    let mut truncated = b"1 0 obj << /Length 2 0 R /Filter /FlateDecode >> stream\n".to_vec();
    truncated.extend_from_slice(&encoded[..encoded.len() - 1]);
    assert!(scan_indirect(truncated, &Limits::default(), &NEVER).is_err());
}

#[test]
fn indirect_flate_length_preserves_cancellation_at_every_checkpoint() {
    let encoded = zlib(&vec![b'x'; 9000]);
    let bytes = indirect_flate_fragment(&encoded, &encoded.len().to_string());
    let counter = CancelAfter::never();
    scan_indirect(bytes.clone(), &Limits::default(), &counter).unwrap();
    for allowed in 0..counter.queries() {
        assert!(matches!(
            scan_indirect(
                bytes.clone(),
                &Limits::default(),
                &CancelAfter::new(allowed)
            ),
            Err(Error {
                kind: ErrorKind::Cancelled,
                ..
            })
        ));
    }
}

#[test]
fn fragment_scanner_skips_binary_markers_repairs_unique_short_length_and_excludes_tail() {
    let payload = b"binary endobj 17 0 obj and endstream marker";
    let actual = payload.len();
    let declared = actual - 2;
    assert_eq!(declared.to_string().len(), actual.to_string().len());
    let mut bytes = format!("1 0 obj\n<< /Length {declared} >>\nstream\n").into_bytes();
    bytes.extend_from_slice(payload);
    bytes.extend_from_slice(b"\r\nendstream\rendobj\n2 0 obj\n42\nendobj");
    let hint = bytes.len() as u64;
    bytes.extend_from_slice(b"<container-tail>");
    let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
    let scan = scan_fragment_with_candidates(
        &mut source,
        0,
        hint - 3,
        &Limits::default(),
        &NEVER,
        &mut [],
    )
    .unwrap();
    assert_eq!(scan.objects.len(), 2);
    assert_eq!(scan.objects[0].object.reference.number, 1);
    assert_eq!(scan.objects[1].object.reference.number, 2);
    assert_eq!(scan.objects.last().unwrap().object.range.end(), Some(hint));
    assert_eq!(scan.patches.len(), 1);
    let mut patched = PatchedSource::new(&mut source, &scan.patches);
    let mut length = vec![0; actual.to_string().len()];
    read_exact_at(
        &mut patched,
        scan.patches[0].offset,
        &mut length,
        &Limits::default(),
        &NEVER,
    )
    .unwrap();
    assert_eq!(length, actual.to_string().as_bytes());
}

#[test]
fn fragment_scanner_rejects_ambiguous_nearby_stream_terminators() {
    let payload = b"x\r\nendstream\rendobj\r\ny";
    let mut bytes = b"1 0 obj\n<< /Length 0 >>\nstream\n".to_vec();
    bytes.extend_from_slice(payload);
    bytes.extend_from_slice(b"\r\nendstream\rendobj");
    let hint = bytes.len() as u64;
    let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
    let result =
        scan_fragment_with_candidates(&mut source, 0, hint, &Limits::default(), &NEVER, &mut []);
    assert!(matches!(
        result,
        Err(Error {
            kind: ErrorKind::Malformed,
            context: Context::Pdf { repair: true, .. },
            ..
        })
    ));
}

#[test]
fn fragment_scanner_does_not_stop_at_a_fake_final_stream_terminator() {
    let mut bytes = b"1 0 obj\n<< /Length 0 >>\nstream\nabc\r\nendstream\rendobj".to_vec();
    let hint = bytes.len() as u64 - 1;
    bytes.extend(std::iter::repeat_n(b'x', 100));
    bytes.extend_from_slice(b"\r\nendstream\rendobj");
    let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
    let error =
        scan_fragment_with_candidates(&mut source, 0, hint, &Limits::default(), &NEVER, &mut [])
            .err()
            .expect("fake final stream terminator was accepted");
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Malformed,
            context: Context::Pdf { repair: true, .. },
            reason: "repaired final stream has a later stream terminator",
            ..
        }
    ));
}

#[test]
fn fragment_scanner_grows_syntax_window_for_split_endobj_keyword() {
    let mut bytes = b"1 0 obj\n[0".to_vec();
    bytes.resize(508, b' ');
    bytes.extend_from_slice(b"]\nendobj");
    assert_eq!(&bytes[510..512], b"en");
    let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
    let scan = scan_fragment_with_candidates(
        &mut source,
        0,
        bytes.len() as u64,
        &Limits::default(),
        &NEVER,
        &mut [],
    )
    .unwrap();
    assert_eq!(scan.objects.len(), 1);
    assert_eq!(scan.objects[0].object.range.length, bytes.len() as u64);
}

#[test]
fn fragment_scanner_uses_declared_stream_extent_even_with_complete_fake_terminator() {
    let payload = b"prefix\rendstream\rendobj\nsecond half";
    let mut bytes = format!("1 0 obj\n<< /Length {} >>\nstream\n", payload.len()).into_bytes();
    bytes.extend_from_slice(payload);
    bytes.extend_from_slice(b"\r\nendstream\rendobj");
    let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
    let scan = scan_fragment_with_candidates(
        &mut source,
        0,
        bytes.len() as u64,
        &Limits::default(),
        &NEVER,
        &mut [],
    )
    .unwrap();
    assert_eq!(scan.objects.len(), 1);
    assert!(scan.patches.is_empty());
    assert_eq!(scan.objects[0].object.range.length, bytes.len() as u64);
}

#[test]
fn fragment_scanner_rejects_unbounded_or_width_changing_stream_repairs() {
    let make = |declared: usize, actual: usize| {
        let mut bytes = format!("1 0 obj\n<< /Length {declared} >>\nstream\n").into_bytes();
        bytes.extend(std::iter::repeat_n(b'x', actual));
        bytes.extend_from_slice(b"\r\nendstream\rendobj");
        bytes
    };
    for (declared, actual, kind) in [(1, 90, "malformed"), (9, 10, "unsupported")] {
        let bytes = make(declared, actual);
        let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
        let error = scan_fragment_with_candidates(
            &mut source,
            0,
            bytes.len() as u64,
            &Limits::default(),
            &NEVER,
            &mut [],
        )
        .err()
        .expect("unsafe stream repair was accepted");
        assert!(pdf_class(&error) == Some(kind));
    }
}

#[test]
fn fragment_scanner_rejects_unresolved_length_and_body_budget_before_output() {
    let bytes = b"1 0 obj\n<< /Length 9 0 R >>\nstream\nabc\r\nendstream\rendobj";
    let mut source = SeekableSource::new(Cursor::new(bytes.to_vec())).unwrap();
    let result = scan_fragment_with_candidates(
        &mut source,
        0,
        bytes.len() as u64,
        &Limits::default(),
        &NEVER,
        &mut [],
    );
    assert!(matches!(
        result,
        Err(Error {
            kind: ErrorKind::Malformed,
            context: Context::Pdf {
                object: Some((9, 0)),
                repair: false
            },
            reason: "indirect stream Length does not match its integer object",
            ..
        })
    ));

    let limits = Limits {
        max_input_bytes: bytes.len() as u64 - 1,
        ..Limits::default()
    };
    let result =
        scan_fragment_with_candidates(&mut source, 0, bytes.len() as u64, &limits, &NEVER, &mut []);
    assert!(matches!(
        result,
        Err(Error {
            kind: ErrorKind::LimitExceeded {
                resource: "input bytes",
                ..
            },
            offset: Some(0),
            context: Context::Caj { .. },
            ..
        })
    ));
}

#[test]
fn fragment_scanner_rejects_invalid_ranges_and_unfinished_objects() {
    let mut source = SeekableSource::new(Cursor::new(b"1 0 obj\nnull\nendobj".to_vec())).unwrap();
    for (start, end) in [(0, 0), (10, 10), (0, source.size() + 1)] {
        assert!(matches!(
            scan_fragment_with_candidates(
                &mut source,
                start,
                end,
                &Limits::default(),
                &NEVER,
                &mut [],
            ),
            Err(Error {
                kind: ErrorKind::Malformed,
                context: Context::Caj { .. },
                reason: "CAJ PDF fragment body range is invalid",
                ..
            })
        ));
    }

    for bytes in [
        b"   ".as_slice(),
        b"1 0 obj\nnull\nendobj\n2 0 obj\ntrue".as_slice(),
        b"1 0 obj\nnull\nendobj\n<unexpected tail>".as_slice(),
    ] {
        let mut source = SeekableSource::new(Cursor::new(bytes.to_vec())).unwrap();
        assert!(matches!(
            scan_fragment_with_candidates(
                &mut source,
                0,
                bytes.len() as u64,
                &Limits::default(),
                &NEVER,
                &mut [],
            ),
            Err(Error {
                kind: ErrorKind::Malformed,
                context: Context::Pdf { repair: false, .. },
                ..
            })
        ));
    }
}

#[test]
fn fragment_scanner_rejects_unsupported_generation_and_stream_framing() {
    for (bytes, kind) in [
        (b"1 1 obj\nnull\nendobj".as_slice(), "unsupported"),
        (
            b"1 0 obj\n<< >>\nstream\nabc\nendstream\nendobj".as_slice(),
            "malformed",
        ),
        (
            b"1 0 obj\n<< /Length 18446744073709551615 >>\nstream\nabc\nendstream\nendobj"
                .as_slice(),
            "malformed",
        ),
    ] {
        let mut source = SeekableSource::new(Cursor::new(bytes.to_vec())).unwrap();
        let error = scan_fragment_with_candidates(
            &mut source,
            0,
            bytes.len() as u64,
            &Limits::default(),
            &NEVER,
            &mut [],
        )
        .err()
        .expect("invalid fragment was accepted");
        assert!(pdf_class(&error) == Some(kind), "{error:?}");
    }
}

#[test]
fn fragment_scanner_requires_a_dictionary_for_stream_payloads() {
    let bytes = b"1 0 obj\nnull\nstream\nabc\nendstream\nendobj";
    let mut source = SeekableSource::new(Cursor::new(bytes.to_vec())).unwrap();
    let error = scan_fragment_with_candidates(
        &mut source,
        0,
        bytes.len() as u64,
        &Limits::default(),
        &NEVER,
        &mut [],
    )
    .err()
    .expect("a stream without a dictionary was accepted");
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Malformed,
            context: Context::Pdf { repair: false, .. },
            reason: "stream has no dictionary",
            ..
        }
    ));
}

#[test]
fn patched_stream_length_rejects_source_mutation_after_scan() {
    let mut bytes = b"1 0 obj\n<< /Length 10 >>\nstream\n".to_vec();
    bytes.extend_from_slice(b"123456789012\r\nendstream\rendobj");
    let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
    let length = source.size();
    let scan =
        scan_fragment_with_candidates(&mut source, 0, length, &Limits::default(), &NEVER, &mut [])
            .unwrap();
    assert_eq!(scan.patches.len(), 1);
    let mut bytes = source.into_inner().into_inner();
    bytes[scan.patches[0].offset as usize] = b'7';
    let mut changed = SeekableSource::new(Cursor::new(bytes)).unwrap();
    let mut patched = PatchedSource::new(&mut changed, &scan.patches);
    let mut one = [0u8; 1];
    let result = read_exact_at(
        &mut patched,
        scan.patches[0].offset,
        &mut one,
        &Limits::default(),
        &NEVER,
    );
    assert!(matches!(
        result,
        Err(Error {
            kind: ErrorKind::Malformed,
            context: Context::Pdf { repair: false, .. },
            reason: "source changed after stream Length validation",
            ..
        })
    ));
}

#[test]
fn patched_source_rejects_a_ranged_adapter_that_overreports() {
    struct Overreporting;
    impl RangedSource for Overreporting {
        fn size(&self) -> u64 {
            1
        }

        fn read_at(&mut self, _offset: u64, destination: &mut [u8]) -> Result<usize> {
            Ok(destination.len() + 1)
        }
    }

    let mut source = Overreporting;
    let mut patched = PatchedSource::new(&mut source, &[]);
    let mut byte = [0u8; 1];
    assert!(matches!(
        patched.read_at(0, &mut byte),
        Err(Error {
            kind: ErrorKind::Malformed,
            reason: "source reported more bytes than requested",
            ..
        })
    ));
}

#[test]
fn fragment_scanner_caps_its_indexes_by_object_count() {
    let mut items = vec![1_u8, 2];
    assert!(matches!(
        fragment_scan::push_capped(&mut items, 3, 2, "test index"),
        Err(Error {
            kind: ErrorKind::LimitExceeded {
                resource: "test index",
                limit: 2,
                attempted: 3,
                ..
            },
            ..
        })
    ));
    fragment_scan::push_capped(&mut items, 3, 3, "test index").unwrap();
    assert_eq!(items, [1, 2, 3]);
    // Without a byte budget, many small objects index under a tiny
    // allocation limit: only the object count is capped.
    let mut bytes = Vec::new();
    for number in 1..=200 {
        bytes.extend_from_slice(format!("{number} 0 obj\nnull\nendobj\n").as_bytes());
    }
    let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
    let limits = Limits {
        io_chunk_bytes: 1,
        max_allocation_bytes: 4096,
        ..Limits::default()
    };
    let size = source.size();
    let scan =
        scan_fragment_with_candidates(&mut source, 0, size, &limits, &NEVER, &mut []).unwrap();
    assert_eq!(scan.objects.len(), 200);
}

#[test]
fn patched_source_applies_multiple_sorted_lengths_across_split_reads() {
    let prefix = b"CAJ container bytes:";
    let mut bytes = prefix.to_vec();
    let body_start = bytes.len() as u64;
    for (number, declared, payload) in [
        (1, 10, b"abcdefghijkl".as_slice()),
        (2, 20, b"ABCDEFGHIJKLMNOPQRSTUV".as_slice()),
        (3, 30, b"12345678901234567890123456789012".as_slice()),
    ] {
        bytes.extend_from_slice(
            format!("{number} 0 obj\n<< /Length {declared} >>\nstream\n").as_bytes(),
        );
        bytes.extend_from_slice(payload);
        bytes.extend_from_slice(b"\r\nendstream\nendobj\n");
    }
    let body_end = bytes.len() as u64;
    let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
    let scan = scan_fragment_with_candidates(
        &mut source,
        body_start,
        body_end,
        &Limits::default(),
        &NEVER,
        &mut [],
    )
    .unwrap();
    assert_eq!(scan.objects.len(), 3);
    assert_eq!(scan.patches.len(), 3);
    assert!(
        scan.patches
            .windows(2)
            .all(|pair| pair[0].offset < pair[1].offset)
    );
    assert_eq!(
        scan.patches
            .iter()
            .map(|patch| patch.replacement.as_slice())
            .collect::<Vec<_>>(),
        [b"12".as_slice(), b"22".as_slice(), b"32".as_slice()]
    );

    let mut expected = bytes;
    for patch in &scan.patches {
        let at = patch.offset as usize;
        assert_eq!(&expected[at..at + patch.original.len()], patch.original);
        expected[at..at + patch.replacement.len()].copy_from_slice(&patch.replacement);
    }
    let mut patched = PatchedSource::new(&mut source, &scan.patches);
    for patch in &scan.patches {
        let at = patch.offset as usize;
        let mut before_and_first_digit = [0; 2];
        assert_eq!(
            patched
                .read_at(patch.offset - 1, &mut before_and_first_digit)
                .unwrap(),
            2
        );
        assert_eq!(before_and_first_digit, expected[at - 1..at + 1]);
        let mut last_digit_and_after = [0; 2];
        assert_eq!(
            patched
                .read_at(patch.offset + 1, &mut last_digit_and_after)
                .unwrap(),
            2
        );
        assert_eq!(last_digit_and_after, expected[at + 1..at + 3]);
    }
    let mut observed = vec![0; expected.len()];
    let one_byte_reads = Limits {
        io_chunk_bytes: 1,
        ..Limits::default()
    };
    for (offset, byte) in observed.iter_mut().enumerate() {
        read_exact_at(
            &mut patched,
            offset as u64,
            std::slice::from_mut(byte),
            &one_byte_reads,
            &NEVER,
        )
        .unwrap();
    }
    assert_eq!(observed, expected);
}

#[test]
fn repaired_final_stream_must_be_the_only_terminator() {
    let mut body = b"1 0 obj\n<< /Length 10 >>\nstream\n".to_vec();
    body.extend_from_slice(b"abcdefghijkl\r\nendstream\nendobj");
    let body_end = body.len() as u64;
    let understated_hint = body_end - 3;

    // Any trailer is accepted after the repaired stream, unless it holds a
    // later complete stream terminator that could end the stream instead.
    for tail in [
        b"\r\n<?xml version=\"1.0\"?><Doc/>".as_slice(),
        b"\r\n<unexpected/>",
        b"\r\nendstream junk",
    ] {
        let mut bytes = body.clone();
        bytes.extend_from_slice(tail);
        let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
        let scan = scan_fragment_with_candidates(
            &mut source,
            0,
            understated_hint,
            &Limits::default(),
            &NEVER,
            &mut [],
        )
        .unwrap();
        assert_eq!(scan.objects.len(), 1);
        assert_eq!(scan.objects[0].object.range.end(), Some(body_end));
        assert_eq!(scan.patches.len(), 1);
        assert_eq!(scan.patches[0].replacement, b"12");
    }

    let mut later_terminator = body;
    later_terminator.extend_from_slice(b"\r\n");
    later_terminator.extend(std::iter::repeat_n(b'x', 100));
    later_terminator.extend_from_slice(b"\nendstream\nendobj");
    let mut source = SeekableSource::new(Cursor::new(later_terminator)).unwrap();
    let error = scan_fragment_with_candidates(
        &mut source,
        0,
        understated_hint,
        &Limits::default(),
        &NEVER,
        &mut [],
    )
    .err()
    .expect("a repaired stream with a later terminator was accepted");
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Malformed,
            context: Context::Pdf { repair: true, .. },
            reason: "repaired final stream has a later stream terminator",
            ..
        }
    ));
}

#[test]
fn nested_page_order_indirect_contents_and_ranged_offsets_are_indexed() {
    let doc = build_pdf(
        &[
            (1, "<< /Type /Catalog /Pages 2 0 R >>"),
            (
                2,
                "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 3 /MediaBox [0 0 100 200] >>",
            ),
            (
                3,
                "<< /Type /Pages /Parent 2 0 R /Kids [5 0 R 6 0 R] /Count 2 >>",
            ),
            (4, "<< /Type /Page /Parent 2 0 R /Contents 8 0 R >>"),
            (5, "<< /Type /Page /Parent 3 0 R >>"),
            (6, "<< /Type /Page /Parent 3 0 R >>"),
            (7, "<< /Length 9 0 R >>\nstream\nabc\nendstream"),
            (8, "[7 0 R]"),
            (9, "3"),
            (10, "<< /Producer (test) >>"),
        ],
        "/ID [<01> (two)] /Info 10 0 R",
    );
    let mut container = b"CAJ prefix".to_vec();
    let offset = container.len() as u64;
    container.extend_from_slice(&doc);
    container.extend_from_slice(b"opaque container suffix");
    let mut source = SeekableSource::new(Cursor::new(container)).unwrap();
    let index = PdfIndex::open(
        &mut source,
        PdfRange {
            offset,
            length: doc.len() as u64,
        },
        &Limits::default(),
        &NEVER,
    )
    .unwrap();
    assert_eq!(
        index
            .pages()
            .iter()
            .map(|page| page.number)
            .collect::<Vec<_>>(),
        [5, 6, 4]
    );
    assert_eq!(
        index.trailer_info(),
        Some(PdfRef {
            number: 10,
            generation: 0
        })
    );
    assert_eq!(index.trailer_id(), Some(b"[<01> (two)]".as_slice()));
    assert_eq!(index.next_free_object_number().unwrap(), 11);
    assert!(
        index
            .object_location(PdfRef {
                number: 7,
                generation: 0
            })
            .unwrap()
            .length
            > 20
    );
    assert!(matches!(
        index.object_location(PdfRef {
            number: 11,
            generation: 0
        }),
        Err(Error {
            kind: ErrorKind::Malformed,
            context: Context::Pdf { repair: false, .. },
            ..
        })
    ));
}

#[test]
fn page_tree_structure_errors_are_rejected() {
    let catalog = "<< /Type /Catalog /Pages 2 0 R >>";
    let root = "<< /Type /Pages /Kids [3 0 R] /Count 1 >>";
    let page = "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 200] >>";
    let bad = [
        (page, root, "null"),
        (page, root, "<< /Type /Xatalog /Pages 2 0 R >>"),
        (page, root, "<< /Type /Catalog >>"),
        (page, "<< /Type /Page /Kids [3 0 R] /Count 1 >>", catalog),
        (page, "<< /Type /Pages /Kids [3 0 R] /Count 2 >>", catalog),
        (page, "<< /Type /Pages /Kids [] /Count 0 >>", catalog),
        (
            page,
            "<< /Type /Pages /Kids [3 0 R 3 0 R] /Count 2 >>",
            catalog,
        ),
        (
            page,
            "<< /Type /Pages /Kids [3 0 R bad] /Count 1 >>",
            catalog,
        ),
        (
            "<< /Type /Other /Parent 2 0 R /MediaBox [0 0 100 200] >>",
            root,
            catalog,
        ),
        (
            "<< /Type /Page /Parent 1 0 R /MediaBox [0 0 100 200] >>",
            root,
            catalog,
        ),
        (
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 0 200] >>",
            root,
            catalog,
        ),
        ("<< /Type /Page /Parent 2 0 R >>", root, catalog),
    ];
    for (page, pages, catalog) in bad {
        expect_pdf_error(base_pdf(page, pages, catalog), "malformed");
    }
}

#[test]
fn outline_sibling_links_and_destinations_are_checked() {
    let catalog = "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>";
    let pages = "<< /Type /Pages /Kids [3 0 R] /Count 1 >>";
    let page = "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 200] >>";
    let root = "<< /Type /Outlines /First 5 0 R /Last 6 0 R /Count 2 >>";
    let first = "<< /Title (A) /Parent 4 0 R /Next 6 0 R /Dest [3 0 R /Fit] >>";
    let second = "<< /Title (B) /Parent 4 0 R /Prev 5 0 R /Dest [3 0 R /Fit] >>";
    let make = |root: &str, first: &str, second: &str| {
        build_pdf(
            &[
                (1, catalog),
                (2, pages),
                (3, page),
                (4, root),
                (5, first),
                (6, second),
            ],
            "",
        )
    };
    assert!(open(make(root, first, second)).unwrap().has_outlines());
    for (root, first, second, kind) in [
        (
            "<< /Type /Outlines /First 5 0 R >>",
            first,
            second,
            "malformed",
        ),
        (
            "<< /Type /Outlines /First 5 0 R /Last 5 0 R >>",
            first,
            second,
            "malformed",
        ),
        (
            root,
            "<< /Title (A) /Parent 4 0 R /Next 5 0 R >>",
            second,
            "malformed",
        ),
        (
            root,
            "<< /Title (A) /Parent 4 0 R /Next 6 0 R /Dest /named >>",
            second,
            "unsupported",
        ),
        (
            root,
            "<< /Title (A) /Parent 4 0 R /Next 6 0 R /A << /S /URI /URI (x) >> >>",
            second,
            "unsupported",
        ),
        (
            root,
            first,
            "<< /Title (B) /Parent 4 0 R /Prev 4 0 R /Dest [3 0 R /Fit] >>",
            "malformed",
        ),
        (
            root,
            first,
            "<< /Title (B) /Parent 1 0 R /Prev 5 0 R >>",
            "malformed",
        ),
        (
            root,
            first,
            "<< /Title (B) /Parent 4 0 R /Prev 5 0 R /Dest [2 0 R /Fit] >>",
            "malformed",
        ),
    ] {
        expect_pdf_error(make(root, first, second), kind);
    }
}

#[test]
fn missing_outline_backlinks_and_descendant_last_are_repaired_together() {
    let objects = [
        (1, "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>"),
        (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        (3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 200] >>"),
        // Both root and item 6 point to their final descendant, rather than
        // their direct last child. Item 6 also lacks Prev.
        (4, "<< /Type /Outlines /First 5 0 R /Last 9 0 R >>"),
        (
            5,
            "<< /Title (First) /Parent 4 0 R /Next 6 0 R /Dest [3 0 R /Fit] >>",
        ),
        (
            6,
            "<< /Title (Second) /Parent 4 0 R /First 7 0 R /Last 9 0 R >>",
        ),
        (7, "<< /Title (Child A) /Parent 6 0 R /Next 8 0 R >>"),
        (
            8,
            "<< /Title (Child B) /Parent 6 0 R /First 9 0 R /Last 9 0 R >>",
        ),
        (
            9,
            "<< /Title (Grandchild) /Parent 8 0 R /Dest [3 0 R /XYZ 0 80 null] >>",
        ),
    ];
    let input = build_pdf(&objects, "");
    let index = open(input.clone()).unwrap();
    assert_eq!(index.repair_objects().len(), 3);
    let repaired = index
        .repair_objects()
        .iter()
        .find(|r| r.reference.number == 6)
        .unwrap();
    let text = std::str::from_utf8(&repaired.body).unwrap();
    assert!(text.contains("/Prev 5 0 R") && text.contains("/Last 8 0 R"));
    assert!(!text.contains("/Last 9 0 R"));
    let mut source = SeekableSource::new(Cursor::new(input.clone())).unwrap();
    let mut output = Vec::new();
    crate::pdf::copy_pdf(&mut source, &mut output, &Limits::default(), &NEVER).unwrap();
    assert!(output.starts_with(&input));
    let reopened = open(output).unwrap();
    assert!(reopened.has_outlines());
    assert!(reopened.repair_objects().is_empty());

    for (at, replacement) in [
        (4, "<< /Type /Outlines /First 5 0 R /Last 7 0 R >>"),
        (
            6,
            "<< /Title (Second) /Parent 4 0 R /Prev null /First 7 0 R /Last 9 0 R >>",
        ),
        (8, "<< /Title (Child B) /Parent 6 0 R /Next 7 0 R >>"),
        (8, "<< /Title (Child B) /Parent 4 0 R >>"),
        (8, "<< /Type /Pages /Title (Child B) /Parent 6 0 R >>"),
    ] {
        let mut invalid = objects;
        invalid.iter_mut().find(|(n, _)| *n == at).unwrap().1 = replacement;
        expect_pdf_error(build_pdf(&invalid, ""), "malformed");
    }
}

#[test]
fn outline_local_goto_actions_preserve_destinations_without_executing_actions() {
    let make = |item: &str| {
        build_pdf(
            &[
                (1, "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>"),
                (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
                (3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 200] >>"),
                (4, "<< /First 5 0 R /Last 5 0 R >>"),
                (5, item),
            ],
            "",
        )
    };
    for action in [
        "<< /S /GoTo /D [3 0 R /FitH 150] >>",
        "<< /Type /Action /D [3 0 R /XYZ 0 150 null] /S /GoTo >>",
    ] {
        let input = make(&format!("<< /Title (Local) /Parent 4 0 R /A {action} >>"));
        let mut source = SeekableSource::new(Cursor::new(input.clone())).unwrap();
        let mut output = Vec::new();
        crate::pdf::copy_pdf(&mut source, &mut output, &Limits::default(), &NEVER).unwrap();
        assert_eq!(output, input);
    }
    for action in [
        "<< /S /URI /URI (https://example.invalid) >>",
        "<< /S /GoToR /D [3 0 R /Fit] >>",
        "<< /S /GoTo /D [3 0 R /Fit] /Next << /S /JavaScript /JS (test) >> >>",
        "<< /S /GoTo /D /named >>",
        "<< /Type /Wrong /S /GoTo /D [3 0 R /Fit] >>",
        "<< /S /GoTo >>",
        "<< /D [3 0 R /Fit] >>",
        "4 0 R",
    ] {
        expect_pdf_error(
            make(&format!("<< /Title (Local) /Parent 4 0 R /A {action} >>")),
            "unsupported",
        );
    }
    for item in [
        "<< /Title (Local) /Parent 4 0 R /A << /S /GoTo /D [2 0 R /Fit] >> >>",
        "<< /Title (Local) /Parent 4 0 R /Dest [3 0 R /Fit] /A << /S /GoTo /D [3 0 R /Fit] >> >>",
    ] {
        expect_pdf_error(make(item), "malformed");
    }
}

fn forward_xref_pdf() -> (Vec<u8>, u64, u64) {
    let base = build_pdf(&minimal_objects(), "");
    let first_at = find(&base, b"xref\n");
    let page_at = find(&base, b"3 0 obj");
    let mut first = format!(
        "xref\n3 1\n{page_at:010} 00000 n \ntrailer\n<< /Size 4 /Root 1 0 R /Prev 0000000000 >>\n"
    )
    .into_bytes();
    let main_at = first_at + first.len() as u64;
    replace_once(
        &mut first,
        b"/Prev 0000000000",
        format!("/Prev {main_at:010}").as_bytes(),
    );
    let mut main = base[first_at as usize..].to_vec();
    replace_once(&mut main, b"/Root 1 0 R", b"");
    // An older entry must not replace the logically newer first-table entry.
    replace_once(
        &mut main,
        format!("{page_at:010} 00000 n").as_bytes(),
        b"0000000001 00000 n",
    );
    let mut doc = base[..first_at as usize].to_vec();
    doc.extend_from_slice(&first);
    doc.extend_from_slice(&main);
    (doc, first_at, main_at)
}

#[test]
fn forward_xref_links_follow_logical_precedence_with_cycle_and_extent_checks() {
    let (doc, first_at, main_at) = forward_xref_pdf();
    assert_eq!(open(doc.clone()).unwrap().pages().len(), 1);
    let mut source = SeekableSource::new(Cursor::new(doc.clone())).unwrap();
    let mut output = Vec::new();
    crate::pdf::copy_pdf(&mut source, &mut output, &Limits::default(), &NEVER).unwrap();
    assert!(output.starts_with(&doc));
    assert!(output.len() > doc.len());
    assert!(open(output).unwrap().repair_objects().is_empty());
    for previous in [first_at, doc.len() as u64, u64::MAX] {
        let mut invalid = doc.clone();
        replace_once(
            &mut invalid,
            format!("/Prev {main_at:010}").as_bytes(),
            format!("/Prev {previous}").as_bytes(),
        );
        let error = pdf_error(open(invalid));
        assert!(matches!(
            error.reason,
            "PDF xref chain contains a cycle" | "xref Prev exceeds PDF range"
        ));
    }
    let mut cycle = doc;
    let trailer = main_at as usize + find(&cycle[main_at as usize..], b"trailer\n<<") as usize;
    cycle.splice(
        trailer + 10..trailer + 10,
        format!(" /Prev {first_at}").bytes(),
    );
    assert_eq!(
        pdf_error(open(cycle)).reason,
        "PDF xref chain contains a cycle"
    );
}

#[test]
fn nested_outline_children_and_empty_roots_are_checked() {
    let catalog = "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>";
    let pages = "<< /Type /Pages /Kids [3 0 R] /Count 1 >>";
    let page = "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 200] >>";
    let root = "<< /Type /Outlines /First 5 0 R /Last 5 0 R /Count 2 >>";
    let parent =
        "<< /Title (Parent) /Parent 4 0 R /First 6 0 R /Last 6 0 R /Count 1 /Dest [3 0 R /Fit] >>";
    let child = "<< /Title <FEFF0043> /Parent 5 0 R /Dest [3 0 R /XYZ 0 80 null] >>";
    let make = |root: &str, parent: &str, child: &str| {
        build_pdf(
            &[
                (1, catalog),
                (2, pages),
                (3, page),
                (4, root),
                (5, parent),
                (6, child),
            ],
            "",
        )
    };
    assert!(open(make(root, parent, child)).unwrap().has_outlines());
    assert!(
        !open(make("<< /Type /Outlines /Count 0 >>", parent, child))
            .unwrap()
            .has_outlines()
    );
    for (root, parent, child, kind) in [
        ("<< /Type /Outlines /Count 2 >>", parent, child, "malformed"),
        (
            "<< /Type /Outlines /First 5 0 R /Last 5 0 R /Count 0 >>",
            parent,
            child,
            "malformed",
        ),
        (
            "<< /Type /Wrong /First 5 0 R /Last 5 0 R >>",
            parent,
            child,
            "malformed",
        ),
        (
            root,
            "<< /Title 7 /Parent 4 0 R /First 6 0 R /Last 6 0 R >>",
            child,
            "malformed",
        ),
        (
            root,
            "<< /Title (Parent) /Parent 4 0 R /First 6 0 R >>",
            child,
            "malformed",
        ),
        (
            root,
            "<< /Title (Parent) /Parent 4 0 R /Last 6 0 R >>",
            child,
            "malformed",
        ),
        (
            root,
            "<< /Title (Parent) /Parent 4 0 R /First 5 0 R /Last 5 0 R >>",
            child,
            "malformed",
        ),
        (
            root,
            parent,
            "<< /Title (Child) /Parent 4 0 R >>",
            "malformed",
        ),
        (
            root,
            parent,
            "<< /Title (Child) /Parent 5 0 R /Prev /Bad >>",
            "malformed",
        ),
        (
            root,
            parent,
            "<< /Title (Child) /Parent 5 0 R /Dest [3 0 R null] >>",
            "unsupported",
        ),
    ] {
        expect_pdf_error(make(root, parent, child), kind);
    }
    let limits = Limits {
        max_bookmarks: 1,
        ..Limits::default()
    };
    assert!(matches!(
        open_with(make(root, parent, child), &limits),
        Err(Error {
            kind: ErrorKind::LimitExceeded {
                resource: "bookmarks",
                ..
            },
            context: Context::Pdf {
                object: Some((6, 0)),
                ..
            },
            ..
        })
    ));
}

#[test]
fn malformed_trailer_and_xref_entries_are_located() {
    let objects = [
        (1, "<< /Type /Catalog /Pages 2 0 R >>"),
        (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        (3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 200] >>"),
    ];
    for extra in [
        "/ID /Bogus",
        "/ID [<01>]",
        "/Info 9 0 R",
        "/Info 3 0 R",
        "/Encrypt 1 0 R",
        "/XRefStm 50",
    ] {
        let kind = match extra {
            "/Encrypt 1 0 R" => "encrypted",
            "/XRefStm 50" => "unsupported",
            _ => "malformed",
        };
        expect_pdf_error(build_pdf(&objects, extra), kind);
    }
    let valid = build_pdf(&objects, "");
    let mut bad_entry = valid.clone();
    let at = bad_entry
        .windows(20)
        .position(|slice| slice == b"0000000000 65535 f \n")
        .unwrap();
    bad_entry[at + 17] = b'z';
    expect_pdf_error(bad_entry, "malformed");
    let mut no_eof = valid;
    no_eof.truncate(no_eof.len() - 6);
    expect_pdf_error(no_eof, "malformed");

    let mut oversized_index = build_pdf(&objects, "");
    let size_at = oversized_index
        .windows(b"/Size 4".len())
        .position(|bytes| bytes == b"/Size 4")
        .unwrap();
    oversized_index.splice(
        size_at..size_at + b"/Size 4".len(),
        b"/Size 8388609".iter().copied(),
    );
    assert!(matches!(
        open(oversized_index),
        Err(Error {
            kind: ErrorKind::LimitExceeded {
                resource: "PDF object index",
                attempted: 8_388_609,
                ..
            },
            context: Context::Pdf { .. },
            ..
        })
    ));
}

#[test]
fn catalog_forms_info_and_outline_roots_have_valid_object_roles() {
    let pages = "<< /Type /Pages /Kids [3 0 R] /Count 1 >>";
    let page = "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 200] >>";
    let make = |catalog: &str, object4: &str, trailer: &str| {
        build_pdf(
            &[(1, catalog), (2, pages), (3, page), (4, object4)],
            trailer,
        )
    };
    let form_catalog = "<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>";
    assert_eq!(
        open(make(form_catalog, "<< /SigFlags 0 >>", ""))
            .unwrap()
            .pages()
            .len(),
        1
    );
    for (catalog, object4, kind) in [
        (
            "<< /Type /Catalog /Pages 2 0 R /AcroForm /Bad >>",
            "<< >>",
            "unsupported",
        ),
        (form_catalog, "null", "malformed"),
        (form_catalog, "<< /SigFlags /Bad >>", "malformed"),
        (form_catalog, "<< /SigFlags 1 >>", "unsupported"),
        (
            "<< /Type /Catalog /Pages 2 0 R /Outlines /Bad >>",
            "<< >>",
            "malformed",
        ),
        (
            "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>",
            "null",
            "malformed",
        ),
    ] {
        expect_pdf_error(make(catalog, object4, ""), kind);
    }
    let plain_catalog = "<< /Type /Catalog /Pages 2 0 R >>";
    expect_pdf_error(make(plain_catalog, "3", "/Info 4 0 R"), "malformed");
}

#[test]
fn page_contents_and_page_node_types_cannot_be_guessed() {
    let catalog = "<< /Type /Catalog /Pages 2 0 R >>";
    let root = "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 100 200] >>";
    let stream = "<< /Length 3 >>\nstream\nabc\nendstream";
    let make = |root: &str, page: &str, object4: &str| {
        build_pdf(&[(1, catalog), (2, root), (3, page), (4, object4)], "")
    };
    let valid = "<< /Type /Page /Parent 2 0 R /Contents [4 0 R] >>";
    assert_eq!(open(make(root, valid, stream)).unwrap().pages().len(), 1);
    for (root, page, object4) in [
        ("null", valid, stream),
        (
            "<< /Kids [3 0 R] /Count 1 /MediaBox [0 0 100 200] >>",
            valid,
            stream,
        ),
        (
            "<< /Type /Pages /Kids [3 0 R] /Count /Bad /MediaBox [0 0 100 200] >>",
            valid,
            stream,
        ),
        (root, "null", stream),
        (root, "<< /Parent 2 0 R >>", stream),
        (root, "<< /Type /Page /Parent /Bad >>", stream),
        (
            root,
            "<< /Type /Page /Parent 2 0 R /Contents /Bad >>",
            stream,
        ),
        (
            root,
            "<< /Type /Page /Parent 2 0 R /Contents [1 0 R] >>",
            stream,
        ),
        (
            root,
            "<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>",
            "[1 0 R]",
        ),
        (root, "<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>", "3"),
    ] {
        expect_pdf_error(make(root, page, object4), "malformed");
    }
}

#[test]
fn indirect_media_box_resolves_to_a_valid_rectangle() {
    let catalog = "<< /Type /Catalog /Pages 2 0 R >>";
    let root = "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox 4 0 R >>";
    let page = "<< /Type /Page /Parent 2 0 R >>";
    let make =
        |box_object: &str| build_pdf(&[(1, catalog), (2, root), (3, page), (4, box_object)], "");
    assert_eq!(open(make("[0 0 100 200]")).unwrap().pages().len(), 1);
    expect_pdf_error(make("null"), "malformed");
    expect_pdf_error(make("[0 0 0 200]"), "malformed");
}

#[test]
fn fragment_catalog_with_unvalidated_outline_tree_is_unsupported() {
    let bytes = b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>\nendobj".to_vec();
    let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
    let error = inspect_fragment_object(
        &mut source,
        PdfRange {
            offset: 0,
            length: bytes.len() as u64,
        },
        PdfRef {
            number: 1,
            generation: 0,
        },
        &Limits::default(),
        &NEVER,
        |_| None,
    )
    .err()
    .unwrap();
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::UnsupportedFormat,
            context: Context::Pdf {
                object: Some((1, 0)),
                repair: false
            },
            ..
        }
    ));
}

fn inspect_raw_fragment(raw: &[u8]) -> Result<FragmentInspection> {
    let mut source = SeekableSource::new(Cursor::new(raw.to_vec()))?;
    inspect_fragment_object(
        &mut source,
        PdfRange {
            offset: 0,
            length: raw.len() as u64,
        },
        PdfRef {
            number: 1,
            generation: 0,
        },
        &Limits::default(),
        &NEVER,
        |_| None,
    )
}

#[test]
fn fragment_inspection_rejects_an_empty_object_range() {
    let error = inspect_raw_fragment(b"").expect_err("an empty object range was inspected");
    assert!(
        matches!(
            error,
            Error {
                kind: ErrorKind::Malformed,
                offset: Some(0),
                context: Context::Pdf {
                    object: Some((1, 0)),
                    repair: false
                },
                reason: "indirect object is truncated",
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn page_walk_rejects_more_leaves_than_the_page_limit_before_counts_are_checked() {
    // Every Pages node is within the limit, but the leaves under the root's
    // two kids exceed it before the root's Count is compared on exit.
    let doc = build_pdf(
        &[
            (1, "<< /Type /Catalog /Pages 2 0 R >>"),
            (2, "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>"),
            (
                3,
                "<< /Type /Pages /Parent 2 0 R /Kids [5 0 R 6 0 R] /Count 2 /MediaBox [0 0 10 10] >>",
            ),
            (
                4,
                "<< /Type /Pages /Parent 2 0 R /Kids [7 0 R 8 0 R] /Count 2 /MediaBox [0 0 10 10] >>",
            ),
            (5, "<< /Type /Page /Parent 3 0 R >>"),
            (6, "<< /Type /Page /Parent 3 0 R >>"),
            (7, "<< /Type /Page /Parent 4 0 R >>"),
            (8, "<< /Type /Page /Parent 4 0 R >>"),
        ],
        "",
    );
    let limits = Limits {
        max_pages: 2,
        ..Limits::default()
    };
    let error = open_with(doc, &limits)
        .err()
        .expect("a page tree over the page limit was accepted");
    assert!(
        matches!(
            error,
            Error {
                kind: ErrorKind::LimitExceeded {
                    resource: "pages",
                    limit: 2,
                    attempted: 3,
                    ..
                },
                context: Context::Pdf {
                    object: Some((7, 0)),
                    ..
                },
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn fragment_inspection_rejects_invalid_roles_streams_and_tail_bytes() {
    let valid_page =
        b"1 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Contents [4 0 R] >>\nendobj";
    let inspected = inspect_raw_fragment(valid_page).unwrap();
    assert!(matches!(
        inspected.kind,
        FragmentKind::Page {
            has_media_box: true,
            ..
        }
    ));
    assert!(inspected.contents_is_direct_array);
    assert_eq!(
        inspected.contents.unwrap(),
        [PdfRef {
            number: 4,
            generation: 0
        }]
    );
    for (raw, kind) in [
        (
            b"1 0 obj << /Type /Page /MediaBox [0 0 10 10] >> endobj".as_slice(),
            "malformed",
        ),
        (
            b"1 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 0 10] >> endobj",
            "malformed",
        ),
        (
            b"1 0 obj << /Type /Page /Parent 2 0 R /MediaBox 4 0 R >> endobj",
            "unsupported",
        ),
        (
            b"1 0 obj << /Type /Page /Parent 2 0 R /Contents /Bad >> endobj",
            "malformed",
        ),
        (
            b"1 0 obj << /Type /Pages /Parent /Bad /Count 1 /Kids [2 0 R] >> endobj",
            "malformed",
        ),
        (
            b"1 0 obj << /Type /Pages /Count /Bad /Kids [2 0 R] >> endobj",
            "malformed",
        ),
        (
            b"1 0 obj << /Type /Pages /Count 1 /Kids [2 0 R 0] >> endobj",
            "malformed",
        ),
        (b"1 0 obj << /Type /Catalog >> endobj", "malformed"),
        (
            b"1 0 obj << /Title (A) /A << /S /URI /URI (x) >> >> endobj",
            "unsupported",
        ),
        (
            b"1 0 obj << /Title (A) /Dest /named >> endobj",
            "unsupported",
        ),
        (
            b"1 0 obj << /Type /Page /Parent 2 0 R >> endobj trailing",
            "malformed",
        ),
        (b"1 0 obj 3 stream\nabc\nendstream\nendobj", "malformed"),
        (
            b"1 0 obj << >>\nstream\nabc\nendstream\nendobj",
            "malformed",
        ),
        (
            b"1 0 obj << /Length /Bad >>\nstream\nabc\nendstream\nendobj",
            "malformed",
        ),
        (
            b"1 0 obj << /Length 4 0 R >>\nstream\nabc\nendstream\nendobj",
            "malformed",
        ),
        (b"", "malformed"),
    ] {
        let error = inspect_raw_fragment(raw)
            .err()
            .unwrap_or_else(|| panic!("accepted malformed fragment: {raw:?}"));
        assert!(pdf_class(&error) == Some(kind), "{error:?}");
    }
}

#[test]
fn fragment_inspection_classifies_streams_outline_items_and_other_dictionaries() {
    let page_ref = PdfRef {
        number: 4,
        generation: 0,
    };
    let stream =
        inspect_raw_fragment(b"1 0 obj << /Length 3 >>\nstream\nabc\nendstream\nendobj").unwrap();
    assert!(stream.is_stream);
    assert!(matches!(stream.kind, FragmentKind::Other));

    let page =
        inspect_raw_fragment(b"1 0 obj << /Type /Page /Parent 2 0 R /Contents 4 0 R >> endobj")
            .unwrap();
    assert!(!page.contents_is_direct_array);
    assert_eq!(page.contents.unwrap(), [page_ref]);
    assert!(matches!(
        page.kind,
        FragmentKind::Page {
            has_media_box: false,
            ..
        }
    ));

    let array = inspect_raw_fragment(b"1 0 obj [4 0 R] endobj").unwrap();
    assert!(matches!(array.kind, FragmentKind::Other));
    assert_eq!(array.destination, None);
    assert_eq!(array.scalar_reference_array, Some(vec![page_ref]));

    let item = inspect_raw_fragment(b"1 0 obj << /Title (A) /Parent 2 0 R >> endobj").unwrap();
    assert_eq!(item.destination, None);
    assert!(matches!(item.kind, FragmentKind::Other));

    let error =
        inspect_raw_fragment(b"1 0 obj << /Type /Catalog /Pages 2 0 R /Outlines 3 0 R >> endobj")
            .err()
            .unwrap();
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::UnsupportedFormat,
            context: Context::Pdf { repair: false, .. },
            reason: "preexisting outline trees in PDF fragments are unsupported",
            ..
        }
    ));
}

#[test]
fn source_ranges_and_parser_limits_have_precise_error_types() {
    let doc = base_pdf(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Catalog /Pages 2 0 R >>",
    );
    let mut source = SeekableSource::new(Cursor::new(doc.clone())).unwrap();
    let too_long = PdfIndex::open(
        &mut source,
        PdfRange {
            offset: 0,
            length: doc.len() as u64 + 1,
        },
        &Limits::default(),
        &NEVER,
    )
    .err()
    .unwrap();
    assert!(matches!(
        too_long,
        Error {
            kind: ErrorKind::Truncated { .. },
            ..
        }
    ));

    let mut limits = Limits {
        max_input_bytes: doc.len() as u64 - 1,
        ..Limits::default()
    };
    let error = open_with(doc.clone(), &limits).err().unwrap();
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::LimitExceeded {
                resource: "input bytes",
                ..
            },
            offset: Some(0),
            context: Context::Pdf { object: None, .. },
            ..
        }
    ));

    let mut index = open(doc).unwrap();
    index.max_referenced_object = MAX_PDF_OBJECTS;
    assert!(matches!(
        index.next_free_object_number(),
        Err(Error {
            kind: ErrorKind::LimitExceeded {
                resource: "PDF object number",
                ..
            },
            context: Context::Pdf { .. },
            ..
        })
    ));

    let nested = format!("{}0{}", "[".repeat(65), "]".repeat(65));
    let nested_doc = build_pdf(
        &[
            (1, "<< /Type /Catalog /Pages 2 0 R >>"),
            (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
            (3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>"),
            (4, &nested),
        ],
        "",
    );
    let error = open(nested_doc).err().unwrap();
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::LimitExceeded {
                resource: "PDF syntax depth",
                ..
            },
            context: Context::Pdf {
                object: Some((4, 0)),
                ..
            },
            ..
        }
    ));

    let large_value = format!("<< /Long ({}) >>", "A".repeat(1_024));
    let large_doc = build_pdf(
        &[
            (1, "<< /Type /Catalog /Pages 2 0 R >>"),
            (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
            (3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>"),
            (4, &large_value),
        ],
        "",
    );
    limits = Limits {
        io_chunk_bytes: 1024,
        max_allocation_bytes: 16 * 1024,
        ..Limits::default()
    };
    let error = open_with(large_doc, &limits).err().unwrap();
    assert!(
        matches!(
            error,
            Error {
                kind: ErrorKind::LimitExceeded {
                    resource: "PDF object syntax bytes",
                    ..
                },
                context: Context::Pdf {
                    object: Some((4, 0)),
                    ..
                },
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn distinct_stream_page_and_outline_failure_paths_keep_object_context() {
    let catalog = "<< /Type /Catalog /Pages 2 0 R >>";
    let pages = "<< /Type /Pages /Kids [3 0 R] /Count 1 >>";
    let page = "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>";
    let with_stream =
        |stream: &str| build_pdf(&[(1, catalog), (2, pages), (3, page), (4, stream)], "");
    for stream in [
        "3 stream\nabc\nendstream",
        "<< /Length 18446744073709551615 >>\nstream\nabc\nendstream",
    ] {
        expect_pdf_error(with_stream(stream), "malformed");
    }
    expect_pdf_error(
        build_pdf(
            &[
                (1, catalog),
                (2, pages),
                (3, page),
                (4, "<< /Length 5 0 R >>\nstream\nabc\nendstream"),
                (5, "<< >>"),
            ],
            "",
        ),
        "malformed",
    );
    expect_pdf_error(
        base_pdf(page, "<< /Type /Pages /Kids /Bad /Count 1 >>", catalog),
        "malformed",
    );

    let outline_catalog = "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>";
    let outline_root = "<< /Type /Outlines /First 5 0 R /Last 5 0 R /Count 1 >>";
    let make = |root: &str, item: &str| {
        build_pdf(
            &[
                (1, outline_catalog),
                (2, pages),
                (3, page),
                (4, root),
                (5, item),
            ],
            "",
        )
    };
    expect_pdf_error(
        make(
            "<< /Type /Outlines /First 5 0 R /Last 5 0 R /Count /Bad >>",
            "<< /Title (A) /Parent 4 0 R >>",
        ),
        "malformed",
    );
    expect_pdf_error(make(outline_root, "null"), "malformed");
    expect_pdf_error(
        make(outline_root, "<< /Title (A) /Parent 4 0 R /Next /Bad >>"),
        "malformed",
    );
}

#[test]
fn scalar_fragment_requires_exact_framing() {
    for (raw, expected_none) in [
        (b"".as_slice(), false),
        (b"1 0 obj 3 endobj trailing".as_slice(), false),
        (
            b"1 0 obj << /Length 0 >> stream\n\nendstream\nendobj".as_slice(),
            true,
        ),
    ] {
        let mut source = SeekableSource::new(Cursor::new(raw.to_vec())).unwrap();
        let result = inspect_fragment_scalar(
            &mut source,
            PdfRange {
                offset: 0,
                length: raw.len() as u64,
            },
            PdfRef {
                number: 1,
                generation: 0,
            },
            &Limits::default(),
            &NEVER,
        );
        if expected_none {
            assert!(matches!(result, Ok(None)));
        } else {
            assert!(matches!(
                result,
                Err(Error {
                    kind: ErrorKind::Malformed,
                    context: Context::Pdf { repair: false, .. },
                    ..
                })
            ));
        }
    }
    // Whitespace after `endobj` is framing, not trailing content.
    let raw = b"1 0 obj 3 endobj \n";
    let mut source = SeekableSource::new(Cursor::new(raw.to_vec())).unwrap();
    let length = inspect_fragment_scalar(
        &mut source,
        PdfRange {
            offset: 0,
            length: raw.len() as u64,
        },
        PdfRef {
            number: 1,
            generation: 0,
        },
        &Limits::default(),
        &NEVER,
    );
    assert_eq!(length.unwrap(), Some(3));
}

/// Assemble a classic-xref PDF from `(number, gap_before, body)` parts with a
/// caller-supplied trailer dictionary. Missing numbers become free entries.
fn assemble_pdf(objects: &[(u32, &str, &str)], trailer: &str) -> Vec<u8> {
    pdf_with_header(b"%PDF-1.7\n", objects, trailer)
}

fn pdf_with_header(header: &[u8], objects: &[(u32, &str, &str)], trailer: &str) -> Vec<u8> {
    let highest = objects
        .iter()
        .map(|(number, ..)| *number)
        .max()
        .unwrap_or(0);
    let mut bytes = header.to_vec();
    let mut offsets = vec![None; highest as usize + 1];
    for &(number, gap, body) in objects {
        bytes.extend_from_slice(gap.as_bytes());
        offsets[number as usize] = Some(bytes.len());
        bytes.extend_from_slice(format!("{number} 0 obj\n{body}\nendobj\n").as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", highest + 1).as_bytes());
    for offset in offsets.into_iter().skip(1) {
        match offset {
            Some(offset) => bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes()),
            None => bytes.extend_from_slice(b"0000000000 00000 f \n"),
        }
    }
    bytes.extend_from_slice(format!("trailer\n{trailer}\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    bytes
}

/// `(number, body)` objects with no bytes before each object.
fn ungapped<'a>(objects: &[(u32, &'a str)]) -> Vec<(u32, &'static str, &'a str)> {
    objects
        .iter()
        .map(|&(number, body)| (number, "", body))
        .collect()
}

fn replace_once(bytes: &mut Vec<u8>, from: &[u8], to: &[u8]) {
    let at = find(bytes, from) as usize;
    bytes.splice(at..at + from.len(), to.iter().copied());
}

fn find(bytes: &[u8], needle: &[u8]) -> u64 {
    bytes
        .windows(needle.len())
        .position(|window| window == needle)
        .unwrap_or_else(|| panic!("fixture lacks {:?}", String::from_utf8_lossy(needle))) as u64
}

fn minimal_objects() -> [(u32, &'static str); 3] {
    [
        (1, "<< /Type /Catalog /Pages 2 0 R >>"),
        (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        (3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>"),
    ]
}

/// Build a PDF whose only cross-reference section is an xref stream. Rows are
/// written for `index_start..size`; the stream object is numbered last.
fn xref_stream_pdf(
    objects: &[(u32, &str)],
    widths: [usize; 3],
    index_start: u32,
    flate: bool,
) -> Vec<u8> {
    let highest = objects.iter().map(|(number, _)| *number).max().unwrap_or(0);
    let stream_number = highest + 1;
    let size = stream_number + 1;
    let mut bytes = b"%PDF-1.5\n".to_vec();
    let mut offsets = vec![None; size as usize];
    for &(number, body) in objects {
        offsets[number as usize] = Some(bytes.len() as u64);
        bytes.extend_from_slice(format!("{number} 0 obj\n{body}\nendobj\n").as_bytes());
    }
    let xref_at = bytes.len() as u64;
    offsets[stream_number as usize] = Some(xref_at);
    let mut decoded = Vec::new();
    let mut field = |value: u64, width: usize| {
        decoded.extend_from_slice(&value.to_be_bytes()[8 - width..]);
    };
    for number in index_start..size {
        let (kind, offset, generation) = match offsets[number as usize] {
            Some(offset) => (1, offset, 0),
            None if number == 0 => (0, 0, 65535),
            None => (0, 0, 0),
        };
        if widths[0] != 0 {
            field(kind, widths[0]);
        }
        field(offset, widths[1]);
        field(generation, widths[2]);
    }
    let (encoded, filter) = if flate {
        (zlib(&decoded), " /Filter /FlateDecode")
    } else {
        (decoded, "")
    };
    bytes.extend_from_slice(
        format!(
            "{stream_number} 0 obj\n<< /Type /XRef /Size {size} /Root 1 0 R /W [{} {} {}] \
             /Index [{index_start} {}] /Length {}{filter} >>\nstream\n",
            widths[0],
            widths[1],
            widths[2],
            size - index_start,
            encoded.len()
        )
        .as_bytes(),
    );
    bytes.extend_from_slice(&encoded);
    bytes.extend_from_slice(
        format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n").as_bytes(),
    );
    bytes
}

fn zlib(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

fn pdf_error(result: Result<PdfIndex>) -> Error {
    match result {
        Err(error) => error,
        Ok(_) => panic!("malformed PDF was accepted"),
    }
}

#[test]
fn incomplete_trailing_eof_markers_are_not_the_document_end() {
    // Each suffix ends in `%%EOF` without a usable `startxref` before it: the
    // marker is glued, has no `startxref` within 128 bytes, has no offset, or
    // has bytes after its offset.
    let far = format!("\n{}\n%%EOF\n", " ".repeat(200));
    for suffix in [
        b"junk%%EOF\n".as_slice(),
        far.as_bytes(),
        b"\nstartxref\n\n%%EOF\n",
        b"\nstartxref\n12 x\n%%EOF\n",
    ] {
        let mut doc = build_pdf(&minimal_objects(), "");
        let real_end = doc.len() as u64;
        doc.extend_from_slice(suffix);
        let error = pdf_error(open(doc));
        let whitespace = suffix
            .iter()
            .take_while(|byte| byte.is_ascii_whitespace())
            .count();
        assert!(
            matches!(
                error,
                Error { kind: ErrorKind::Malformed, offset: Some(offset), context: Context::Pdf { repair: true, .. }, reason: "bytes after PDF EOF are not a recognized CAJ footer", .. } if offset == real_end + whitespace as u64
            ),
            "{:?}: {error:?}",
            String::from_utf8_lossy(suffix)
        );
    }
}

#[test]
fn download_footers_preserve_the_pdf_logical_end() {
    let property = "\u{feff}<FileProperty><Doi /><FileName>original-test</FileName>\
                    <TableName>TEST</TableName><Type>1</Type></FileProperty>";
    for suffix in [
        "WebFastLoad".to_owned(),
        property.to_owned(),
        format!("WebFastLoad{property}"),
        format!(
            "WebFastLoad{}",
            property.replace("<Doi />", "<Doi>10.test/example</Doi>")
        ),
    ] {
        let mut doc = build_pdf(&minimal_objects(), "");
        let logical_end = doc.len() as u64;
        doc.extend_from_slice(suffix.as_bytes());
        let index = open(doc).unwrap();
        assert_eq!(index.logical_end(), logical_end);
        assert_eq!(index.pages().len(), 1);
    }
}

#[test]
fn download_footers_reject_unknown_or_ambiguous_neighbors() {
    let property = "\u{feff}<FileProperty><Doi /><FileName>test</FileName>\
                    <TableName>TEST</TableName><Type>1</Type></FileProperty>";
    for suffix in [
        "WebFastLoadgarbage".to_owned(),
        "WebFastLoad\0".to_owned(),
        "WebFastLoad\0<right-meta></right-meta>startrights 12,25".to_owned(),
        "WebFastLoad\n4 0 obj null endobj".to_owned(),
        format!("{property}junk"),
        format!("{property}{property}"),
        property.replace("<Doi />", "<DoiX />"),
        property.replace("</Type>", "</Other>"),
        property.replace("<Doi />", ""),
        property.replace("<Doi />", "<Doi /><Doi />"),
        property.replace("<Doi />", "<!DOCTYPE Doi><Doi />"),
        property.replace("<Doi />", "<Doi>&external;</Doi>"),
        property.replace("<Doi />", "<Doi><Nested /></Doi>"),
        property.replace("test", "startxref"),
        property.replace("test", "\0"),
        property.replace('\u{feff}', ""),
    ] {
        let mut doc = build_pdf(&minimal_objects(), "");
        doc.extend_from_slice(suffix.as_bytes());
        let error = pdf_error(open(doc));
        assert!(
            matches!(error.context, Context::Pdf { repair: true, .. }),
            "{error:?}"
        );
        assert_eq!(
            error.reason,
            "bytes after PDF EOF are not a recognized CAJ footer"
        );
    }
}

#[test]
fn xref_subsection_tokens_must_be_bounded_integers() {
    for (replacement, reason) in [
        (b"xref\n<0 4\n".as_slice(), "expected PDF token"),
        (
            b"xref\n99999999999999999999 4\n".as_slice(),
            "PDF integer overflows",
        ),
        (
            b"xref\n0x 4\n".as_slice(),
            "expected nonnegative PDF integer",
        ),
    ] {
        let mut doc = build_pdf(&minimal_objects(), "");
        let xref_at = find(&doc, b"xref\n0 4\n");
        replace_once(&mut doc, b"xref\n0 4\n", replacement);
        let error = pdf_error(open(doc));
        assert!(
            matches!(
                error,
                Error { kind: ErrorKind::Malformed, offset: Some(offset), context: Context::Pdf { object: None, repair: false }, reason: found, .. } if offset == xref_at + 5 && found == reason
            ),
            "{error:?}"
        );
    }
}

#[test]
fn trailer_keyword_at_range_end_reports_a_truncated_dictionary() {
    // The startxref line hides inside a comment, so only whitespace and
    // comments follow `trailer` before the end of the PDF range.
    let doc = b"%PDF-1.7\nxref\n0 1\n0000000000 65535 f \ntrailer\n%startxref 9\n%%EOF\n".to_vec();
    let length = doc.len() as u64;
    let error = pdf_error(open(doc));
    assert!(
        matches!(
            error,
            Error { kind: ErrorKind::Malformed, offset: Some(offset), context: Context::Pdf { repair: false, .. }, reason: "PDF dictionary is truncated", .. } if offset == length
        ),
        "{error:?}"
    );
}

#[test]
fn malformed_trailer_dictionary_syntax_is_located() {
    let mut doc = build_pdf(&minimal_objects(), "");
    replace_once(&mut doc, b"trailer\n<< /Size", b"trailer\n<< 7 /Size");
    let dictionary_at = find(&doc, b"trailer\n") + 8;
    let error = pdf_error(open(doc));
    assert!(
        matches!(
            error,
            Error { kind: ErrorKind::Malformed, offset: Some(offset), context: Context::Pdf { object: None, repair: false }, .. } if offset > dictionary_at
        ),
        "{error:?}"
    );
}

#[test]
fn trailer_size_root_info_and_prev_values_must_have_the_right_types() {
    let objects: Vec<_> = ungapped(&minimal_objects());
    for (trailer, reason) in [
        ("<< /Size /Bad /Root 1 0 R >>", "invalid PDF trailer Size"),
        ("<< /Size 0 /Root 1 0 R >>", "invalid PDF trailer Size"),
        ("<< /Root 1 0 R >>", "PDF trailer lacks Size"),
        ("<< /Size 4 >>", "PDF trailer lacks Root"),
        ("<< /Size 4 /Root 1 >>", "invalid PDF trailer Root"),
        (
            "<< /Size 4 /Root 1 0 R /Info /Bad >>",
            "invalid PDF trailer Info",
        ),
        (
            "<< /Size 4 /Root 1 0 R /Prev -1 >>",
            "invalid PDF trailer Prev",
        ),
        (
            "<< /Size 4 /Root 1 0 R /Size 4 >>",
            "duplicate PDF dictionary keys have undefined value",
        ),
    ] {
        let error = pdf_error(open(assemble_pdf(&objects, trailer)));
        assert!(
            matches!(error, Error { kind: _, context: Context::Pdf { object: None, .. }, reason: found, .. } if found == reason),
            "{trailer}: {error:?}"
        );
    }
}

#[test]
fn startxref_to_a_non_dictionary_object_is_not_an_xref_stream() {
    let doc = b"%PDF-1.7\n1 0 obj\n42\nendobj\nstartxref\n9\n%%EOF\n".to_vec();
    let error = pdf_error(open(doc));
    assert!(
        matches!(
            error,
            Error {
                kind: ErrorKind::Malformed,
                offset: Some(9),
                context: Context::Pdf {
                    object: Some((1, 0)),
                    repair: false
                },
                reason: "xref stream lacks a dictionary",
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn xref_stream_without_a_type_field_defaults_rows_to_in_use() {
    let doc = xref_stream_pdf(&minimal_objects(), [0, 4, 1], 1, false);
    let page_at = find(&doc, b"3 0 obj\n");
    let index = open(doc).unwrap();
    assert_eq!(index.trailer_size(), 5);
    let page = PdfRef {
        number: 3,
        generation: 0,
    };
    assert_eq!(index.pages(), [page]);
    assert_eq!(index.object_location(page).unwrap().offset, page_at);
}

#[test]
fn cancellation_at_every_checkpoint_of_a_flate_xref_stream_is_reported() {
    let doc = xref_stream_pdf(&minimal_objects(), [1, 4, 2], 0, true);
    let mut cancelled = 0;
    for allowed in 0.. {
        assert!(allowed < 10_000, "cancellation checkpoints never ended");
        let cancellation = CancelAfter::new(allowed);
        let result = open_cancellable(doc.clone(), &Limits::default(), &cancellation);
        match result {
            Ok(index) => {
                assert_eq!(index.pages().len(), 1);
                break;
            }
            Err(Error {
                kind: ErrorKind::Cancelled,
                ..
            }) => cancelled += 1,
            Err(other) => panic!("checkpoint {allowed} failed with {other:?}"),
        }
    }
    assert!(cancelled > 3, "only {cancelled} checkpoints observed");
}

#[test]
fn live_object_inside_the_caj_footer_is_rejected() {
    let mut objects = minimal_objects().to_vec();
    objects.push((4, "null"));
    let mut doc = build_pdf(&objects, "");
    let object4_at = find(&doc, b"4 0 obj\n");
    let footer_at = doc.len() as u64;
    doc.extend_from_slice(b"WebFastLoadP 4 0 null");
    replace_once(
        &mut doc,
        format!("{object4_at:010} 00000 n").as_bytes(),
        format!("{footer_at:010} 00000 n").as_bytes(),
    );
    let error = pdf_error(open(doc));
    assert!(
        matches!(
            error,
            Error { kind: ErrorKind::Malformed, offset: Some(offset), context: Context::Pdf { object: Some((4, 0)), repair: false }, reason: "live PDF object begins after logical EOF", .. } if offset == footer_at
        ),
        "{error:?}"
    );
}

/// Serves `first` until a read starts at `marker`, then `then` for that and
/// every later read: a source whose bytes change after the tail scan.
struct MutatesAfterMarker {
    first: Vec<u8>,
    then: Vec<u8>,
    marker: u64,
    switched: bool,
}

impl RangedSource for MutatesAfterMarker {
    fn size(&self) -> u64 {
        self.first.len() as u64
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        self.switched |= offset == self.marker;
        let bytes = if self.switched {
            &self.then
        } else {
            &self.first
        };
        let start = offset as usize;
        let count = destination.len().min(bytes.len().saturating_sub(start));
        destination[..count].copy_from_slice(&bytes[start..start + count]);
        Ok(count)
    }
}

#[test]
fn live_object_rewritten_across_the_logical_eof_after_the_tail_scan_is_rejected() {
    let mut objects = minimal_objects().to_vec();
    objects.push((4, "null"));
    let mut first = build_pdf(&objects, "");
    let object4_at = find(&first, b"4 0 obj\n");
    let startxref_at = find(&first, b"startxref");
    replace_once(
        &mut first,
        format!("{object4_at:010} 00000 n").as_bytes(),
        format!("{startxref_at:010} 00000 n").as_bytes(),
    );
    first.extend_from_slice(b"WebFastLoadP");
    first.extend_from_slice(&[b'Z'; 64]);
    let logical_end = find(&first, b"WebFastLoadP");
    // After the tail scan fixed the logical EOF, object 4 appears where
    // `startxref` was and runs on into the footer.
    let object4 = format!("4 0 obj\nnull{}\nendobj\n", " ".repeat(40));
    let mut then = first.clone();
    let at = startxref_at as usize;
    then[at..at + object4.len()].copy_from_slice(object4.as_bytes());
    assert!(startxref_at + object4.len() as u64 > logical_end);
    let length = first.len() as u64;
    let mut source = MutatesAfterMarker {
        marker: find(&first, b"xref\n"),
        first,
        then,
        switched: false,
    };
    let error = PdfIndex::open(
        &mut source,
        PdfRange { offset: 0, length },
        &Limits::default(),
        &NEVER,
    )
    .err()
    .expect("an object past the scanned logical EOF was accepted");
    assert!(
        matches!(
            error,
            Error { kind: ErrorKind::Malformed, offset: Some(offset), context: Context::Pdf { object: Some((4, 0)), repair: false }, reason: "live PDF object extends past logical EOF", .. } if offset == startxref_at
        ),
        "{error:?}"
    );
}

#[test]
fn acroform_without_signature_flags_is_accepted() {
    let doc = build_pdf(
        &[
            (1, "<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>"),
            (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
            (3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>"),
            (4, "<< /Fields [] >>"),
        ],
        "",
    );
    let index = open(doc).unwrap();
    assert_eq!(index.pages().len(), 1);
    assert!(
        index
            .catalog_entries()
            .iter()
            .any(|entry| entry.name == b"AcroForm")
    );
}

#[test]
fn nested_leaves_are_counted_against_the_page_limit() {
    let doc = build_pdf(
        &[
            (1, "<< /Type /Catalog /Pages 2 0 R >>"),
            (
                2,
                "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 10 10] >>",
            ),
            (3, "<< /Type /Page /Parent 2 0 R >>"),
            (
                4,
                "<< /Type /Pages /Parent 2 0 R /Kids [5 0 R 6 0 R] /Count 2 >>",
            ),
            (5, "<< /Type /Page /Parent 4 0 R >>"),
            (6, "<< /Type /Page /Parent 4 0 R >>"),
        ],
        "",
    );
    let limits = Limits {
        max_pages: 2,
        ..Limits::default()
    };
    let error = pdf_error(open_with(doc, &limits));
    assert!(
        matches!(
            error,
            Error {
                kind: ErrorKind::LimitExceeded {
                    resource: "pages",
                    limit: 2,
                    attempted: 3,
                    ..
                },
                context: Context::Pdf {
                    object: Some((6, 0)),
                    ..
                },
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn pages_sharing_one_content_stream_are_both_indexed() {
    let doc = build_pdf(
        &[
            (1, "<< /Type /Catalog /Pages 2 0 R >>"),
            (
                2,
                "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 10 10] >>",
            ),
            (3, "<< /Type /Page /Parent 2 0 R /Contents 5 0 R >>"),
            (4, "<< /Type /Page /Parent 2 0 R /Contents 5 0 R >>"),
            (5, "<< /Length 3 >>\nstream\nabc\nendstream"),
        ],
        "",
    );
    let index = open(doc).unwrap();
    assert_eq!(
        index
            .pages()
            .iter()
            .map(|page| page.number)
            .collect::<Vec<_>>(),
        [3, 4]
    );
}

#[test]
fn comments_between_objects_are_not_orphan_repairs() {
    let [catalog, pages, page] = minimal_objects();
    let objects = [
        (catalog.0, "", catalog.1),
        (pages.0, "", pages.1),
        (page.0, "% producer note\n", page.1),
    ];
    let index = open(assemble_pdf(&objects, "<< /Size 4 /Root 1 0 R >>")).unwrap();
    assert!(index.gap_patches().is_empty());
    assert_eq!(index.pages().len(), 1);
}

#[test]
fn gaps_that_are_not_free_object_prefixes_stay_unindexed() {
    // Arbitrary bytes, a conflicting live-object prefix, and an oversized
    // free-object prefix remain errors under both gap recovery rules.
    let long = format!("{}4 0 obj\n", " ".repeat(64));
    for gap in ["junk\n", "2 0 obj << /Wrong\n", &long] {
        let error = pdf_error(open(gapped_pdf(2, gap)));
        assert!(
            matches!(
                error,
                Error {
                    kind: ErrorKind::Malformed,
                    context: Context::Pdf { repair: false, .. },
                    reason: "unindexed bytes between PDF objects",
                    ..
                }
            ),
            "{gap:?}: {error:?}"
        );
    }
}

#[test]
fn duplicate_nested_page_keys_are_not_guessed() {
    let [catalog, pages, _] = minimal_objects();
    let page = (
        3,
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Resources << /A 1 /A 2 >> >>",
    );
    let bytes = build_pdf(&[catalog, pages, page], "");
    expect_pdf_error(bytes, "ambiguous");
}

/// Objects 1-3 form a one-page tree, object 4 is free, and objects 5.. are
/// inert fillers, each preceded by `gap`.
fn gapped_pdf(fillers: u32, gap: &str) -> Vec<u8> {
    let mut objects: Vec<(u32, &str, &str)> = ungapped(&minimal_objects());
    for number in 5..5 + fillers {
        objects.push((number, gap, "null"));
    }
    assemble_pdf(
        &objects,
        &format!("<< /Size {} /Root 1 0 R >>", 5 + fillers),
    )
}

#[test]
fn orphan_prefixes_of_a_free_object_are_retained_up_to_a_budget() {
    let gap = "4 0 obj\n";
    let doc = gapped_pdf(2, gap);
    let first_gap = find(&doc, b"4 0 obj\n");
    let index = open(doc).unwrap();
    assert_eq!(index.gap_patches().len(), 2);
    // A gap starts right after the previous `endobj`, so it retains the
    // newline that ended that object as well.
    assert_eq!(index.gap_patches()[0].offset, first_gap - 1);
    for patch in index.gap_patches() {
        assert_eq!(patch.original, b"\n4 0 obj\n");
    }

    // 64 KiB / 8 = 8192 retained bytes. Padding each gap to the 64-byte
    // maximum makes the 129th gap the first one over budget.
    let padded = format!("{}4 0 obj\n", " ".repeat(55));
    let limits = Limits {
        io_chunk_bytes: 4096,
        max_allocation_bytes: 64 * 1024,
        ..Limits::default()
    };
    let error = pdf_error(open_with(gapped_pdf(130, &padded), &limits));
    assert!(
        matches!(
            error,
            Error {
                kind: ErrorKind::LimitExceeded {
                    resource: "PDF orphan gap repair bytes",
                    limit: 8192,
                    attempted: 8256,
                    ..
                },
                context: Context::Pdf { object: None, .. },
                ..
            }
        ),
        "{error:?}"
    );
}

/// A tree whose leaf 3 has a stale Parent (free object 9), plus `orphans`
/// unreferenced Pages nodes that repeat an identical MediaBox.
fn repair_budget_pdf(orphans: u32) -> Vec<u8> {
    let padding = "x".repeat(300);
    let orphan =
        format!("<< /Type /Pages /MediaBox [0 0 1 1] /MediaBox [0 0 1 1] /Pad ({padding}) >>");
    let mut objects: Vec<(u32, String)> = vec![
        (1, "<< /Type /Catalog /Pages 2 0 R >>".into()),
        (
            2,
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 10 10] >>".into(),
        ),
        (3, "<< /Type /Page /Parent 9 0 R >>".into()),
    ];
    for number in 10..10 + orphans {
        objects.push((number, orphan.clone()));
    }
    let borrowed: Vec<_> = objects
        .iter()
        .map(|(number, body)| (*number, body.as_str()))
        .collect();
    build_pdf(&borrowed, "")
}

#[test]
fn duplicate_media_box_and_stale_parent_repairs_share_one_budget() {
    let orphans = 30;
    let index = open(repair_budget_pdf(orphans)).unwrap();
    let repairs = index.repair_objects();
    assert_eq!(repairs.len(), orphans as usize + 1);
    let page_repair = repairs.last().unwrap();
    assert_eq!(page_repair.reference.number, 3);
    assert!(page_repair.body.ends_with(b"/Parent 2 0 R\n>>"));
    let page_bytes = page_repair.body.len() as u64;
    let orphan_bytes: u64 = repairs[..orphans as usize]
        .iter()
        .map(|repair| {
            assert_eq!(
                repair
                    .body
                    .windows(b"/MediaBox".len())
                    .filter(|window| *window == b"/MediaBox")
                    .count(),
                1
            );
            repair.body.len() as u64
        })
        .sum();

    // A budget that fits every MediaBox repair leaves no room for the page.
    let limits = Limits {
        io_chunk_bytes: 4096,
        max_allocation_bytes: orphan_bytes * 2,
        ..Limits::default()
    };
    let error = pdf_error(open_with(repair_budget_pdf(orphans), &limits));
    assert!(
        matches!(
            error,
            Error { kind: ErrorKind::LimitExceeded { resource: "PDF repair object bytes", limit, attempted, .. }, context: Context::Pdf { object: Some((3, 0)), .. }, .. } if limit == orphan_bytes && attempted == orphan_bytes + page_bytes
        ),
        "{error:?}"
    );

    // The page repair charges exactly its body length: that budget suffices.
    let limits = Limits {
        io_chunk_bytes: 4096,
        max_allocation_bytes: (orphan_bytes + page_bytes) * 2,
        ..Limits::default()
    };
    let index = open_with(repair_budget_pdf(orphans), &limits).unwrap();
    assert_eq!(index.repair_objects().len(), orphans as usize + 1);

    // One byte less and the last MediaBox repair itself is refused.
    let limits = Limits {
        io_chunk_bytes: 4096,
        max_allocation_bytes: (orphan_bytes - 1) * 2,
        ..Limits::default()
    };
    let error = pdf_error(open_with(repair_budget_pdf(orphans), &limits));
    assert!(
        matches!(
            error,
            Error { kind: ErrorKind::LimitExceeded { resource: "PDF repair object bytes", attempted, .. }, context: Context::Pdf { object: Some((number, 0)), .. }, .. } if number == 9 + orphans && attempted == orphan_bytes
        ),
        "{error:?}"
    );
}

#[test]
fn fragment_stream_length_that_overflows_is_rejected() {
    let error = inspect_raw_fragment(
        b"1 0 obj << /Length 18446744073709551615 >>\nstream\nabc\nendstream\nendobj",
    )
    .err()
    .unwrap();
    assert!(
        matches!(
            error,
            Error {
                kind: ErrorKind::Malformed,
                offset: Some(0),
                context: Context::Pdf {
                    object: Some((1, 0)),
                    repair: false
                },
                reason: "stream extent overflows",
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn fragment_outline_item_without_destination_has_none() {
    let inspected =
        inspect_raw_fragment(b"1 0 obj << /Title (A) /Parent 2 0 R /Next 3 0 R >> endobj").unwrap();
    assert!(matches!(inspected.kind, FragmentKind::Other));
    assert_eq!(inspected.destination, None);
    assert!(!inspected.is_stream);
    assert_eq!(inspected.max_referenced_object, 3);
}

#[test]
fn fixed_width_xref_entries_require_exact_spacing() {
    assert!(matches!(
        parse_xref_entry(b"0000000017 00002 n \n"),
        Some(XrefSlot {
            generation: 2,
            kind: XrefKind::InUse(17)
        })
    ));
    assert!(matches!(
        parse_xref_entry(b"0000000000 65535 f\r\n"),
        Some(XrefSlot {
            generation: 65535,
            kind: XrefKind::Free
        })
    ));
    for line in [
        b"0000000017 00002 n \n\n".as_slice(),
        b"000000001a 00002 n \n",
        b"0000000017_00002 n \n",
        b"0000000017 0000x n \n",
        b"0000000017 00002_n \n",
        b"0000000017 00002 n  \n",
        b"0000000017 99999 n \n",
        b"0000000017 00002 x \n",
    ] {
        assert!(parse_xref_entry(line).is_none(), "{line:?}");
    }
}

#[test]
fn orphan_gap_grammar_accepts_only_aborted_object_prefixes() {
    for gap in [
        b" 7 0\n".as_slice(),
        b"7 0 obj\n",
        b"7 0 obj\r<\r\n",
        b"7 0 obj 12\n",
        b"7 0 obj 12 e",
    ] {
        assert_eq!(parse_orphan_gap(gap), Some(7), "{gap:?}");
    }
    for gap in [
        b"obj".as_slice(),
        b"7",
        b"7x 0 obj",
        b"0 0 obj",
        b"99999999 0 obj",
        b"7 1 obj",
        b"7 00 obj",
        b"7 0 objx",
        b"7 0 obj /Name",
        b"7 0 obj 12 endobj",
    ] {
        assert_eq!(parse_orphan_gap(gap), None, "{gap:?}");
    }
}

#[test]
fn xref_inflation_requires_exact_complete_streams() {
    let decoded = (0..=255).collect::<Vec<u8>>();
    let encoded = zlib(&decoded);
    assert_eq!(
        inflate_xref(&encoded, decoded.len(), 7, &NEVER).ok(),
        Some(decoded.clone())
    );
    assert!(matches!(
        inflate_xref(&encoded, decoded.len(), 7, &CancelAfter::always()),
        Err(Error {
            kind: ErrorKind::Cancelled,
            ..
        })
    ));
    assert!(matches!(
        inflate_xref(&encoded, decoded.len() - 1, 7, &NEVER),
        Err(Error {
            kind: ErrorKind::LimitExceeded { .. },
            ..
        })
    ));
    assert!(matches!(
        inflate_xref(&encoded, decoded.len() + 1, 7, &NEVER),
        Err(Error {
            kind: ErrorKind::Malformed,
            ..
        })
    ));
    let mut trailing = encoded.clone();
    trailing.push(0);
    assert!(matches!(
        inflate_xref(&trailing, decoded.len(), 7, &NEVER),
        Err(Error {
            kind: ErrorKind::Malformed,
            ..
        })
    ));
    assert!(matches!(
        inflate_xref(&encoded[..encoded.len() - 6], decoded.len(), 7, &NEVER),
        Err(Error {
            kind: ErrorKind::Malformed,
            ..
        })
    ));
    assert!(matches!(
        inflate_xref(b"not zlib", decoded.len(), 7, &NEVER),
        Err(Error {
            kind: ErrorKind::Malformed,
            ..
        })
    ));
}

/// Open `bytes` with a small allocation limit and return the typed limit.
fn index_limit(
    bytes: Vec<u8>,
    max_allocation_bytes: u64,
) -> (u64, Option<(u32, u16)>, &'static str, u64) {
    let limits = Limits {
        io_chunk_bytes: 64,
        max_allocation_bytes,
        ..Limits::default()
    };
    match pdf_error(open_with(bytes, &limits)) {
        Error {
            kind: ErrorKind::LimitExceeded {
                resource, limit, ..
            },
            offset: Some(offset),
            context: Context::Pdf { object, .. },
            ..
        } => (offset, object, resource, limit),
        other => panic!("unexpected error: {other:?}"),
    }
}

/// The minimal document plus `count` copies of `body` numbered from 4.
fn with_copies(body: &'static str, count: u32) -> Vec<(u32, &'static str)> {
    let mut objects = minimal_objects().to_vec();
    objects.extend((4..4 + count).map(|number| (number, body)));
    objects
}

#[test]
fn index_allocations_obey_the_allocation_limit() {
    // Each limit below still admits every object's syntax window
    // (`max_allocation_bytes / 32`), so the named index is what fails.
    let mut objects = minimal_objects().to_vec();
    objects.push((35, "null"));
    let (_, object, resource, limit) = index_limit(build_pdf(&objects, ""), 1024);
    assert_eq!((object, resource, limit), (None, "PDF xref records", 1024));

    let mut objects = minimal_objects().to_vec();
    // The 129th record grows capacity to 256 even on i686, where records
    // are smaller. Three-byte rows keep decoded bytes within their own cap.
    objects.push((127, "null"));
    let stream = xref_stream_pdf(&objects, [1, 2, 0], 0, false);
    let (_, object, resource, _) = index_limit(stream, 4096);
    assert_eq!((object, resource), (Some((128, 0)), "PDF xref records"));

    // The xref records fit, but the combined per-object indexes do not.
    let mut objects = minimal_objects().to_vec();
    objects.push((103, "null"));
    let classic = build_pdf(&objects, "");
    let xref = find(&classic, b"xref\n");
    assert_eq!(
        index_limit(classic, 4096),
        (xref, None, "allocation bytes", 4096)
    );

    let separators = with_copies("<< /Length 1 >>\nstream\rx\nendstream", 34);
    let (_, object, resource, limit) = index_limit(build_pdf(&separators, ""), 2752);
    assert_eq!(
        (object, resource, limit),
        (Some((36, 0)), "PDF stream separator patches", 2752 / 8)
    );

    // Orphan pages whose Parent is a free object below the highest number.
    let mut stale = with_copies("<< /Type /Page /Parent 30 0 R >>", 20);
    stale.push((31, "null"));
    let (_, object, resource, limit) = index_limit(build_pdf(&stale, ""), 2176);
    assert_eq!(
        (object, resource, limit),
        (Some((20, 0)), "PDF stale page parent candidates", 2176 / 8)
    );
}

/// Insert `stray` immediately before the classic xref and move `startxref`.
fn with_bytes_before_xref(mut bytes: Vec<u8>, stray: &[u8]) -> Vec<u8> {
    let xref = find(&bytes, b"xref\n");
    bytes.splice(xref as usize..xref as usize, stray.iter().copied());
    replace_once(
        &mut bytes,
        format!("startxref\n{xref}\n").as_bytes(),
        format!("startxref\n{}\n", xref + stray.len() as u64).as_bytes(),
    );
    bytes
}

#[test]
fn document_structure_errors_have_typed_reasons() {
    const PAGES: &str = "<< /Type /Pages /Kids [3 0 R] /Count 1 >>";
    const CATALOG: &str = "<< /Type /Catalog /Pages 2 0 R >>";
    let page = |extra: &str| format!("<< /Type /Page /MediaBox [0 0 1 1] {extra} >>");
    let minimal = || build_pdf(&minimal_objects(), "");
    let mut version = minimal();
    replace_once(&mut version, b"%PDF-1.7", b"%PDF-1.8");
    let mut second = minimal();
    replace_once(&mut second, b"%PDF-1.7", b"%PDF-2.0");
    // An orphan page whose Parent is a free object is a repair candidate
    // only while the validated Kids also reach it.
    let mut orphan = with_copies("<< /Type /Page /Parent 5 0 R >>", 1);
    orphan.push((6, "null"));
    let mut token = minimal();
    replace_once(
        &mut token,
        b"xref\n0 4\n",
        b"xref\n123456789012345678901 1\n",
    );
    let mut duplicate = minimal();
    replace_once(&mut duplicate, b"/Root 1 0 R", b"/Root 1 0 R /Root 1 0 R");
    let mut escape = minimal();
    replace_once(&mut escape, b"/Root 1 0 R", b"/Root 1 0 R /Bad#GG 1");
    let cases = [
        (b"%PDF-1".to_vec(), "malformed", "PDF header is truncated"),
        (
            version,
            "malformed",
            "PDF 1.0 through 1.7 header is required",
        ),
        (
            second,
            "unsupported",
            "PDF 2.x is outside the supported input profile",
        ),
        (token, "malformed", "PDF token is too long"),
        (
            build_pdf(&orphan, ""),
            "malformed",
            "stale page Parent is not reachable through validated Kids",
        ),
        (
            duplicate,
            "ambiguous",
            "duplicate PDF dictionary keys have undefined value",
        ),
        (escape, "malformed", "invalid PDF name escape"),
        (
            with_bytes_before_xref(minimal(), b"4 0 obj\n0\nendobj\n"),
            "malformed",
            "unindexed bytes between PDF objects",
        ),
        (
            base_pdf(&page("/Parent 2 0 R /Foo 9 0 R"), PAGES, CATALOG),
            "malformed",
            "PDF object contains a dangling indirect reference",
        ),
        (
            base_pdf(&page(""), PAGES, CATALOG),
            "malformed",
            "page tree Parent link disagrees with Kids",
        ),
        (
            base_pdf(&page("/Parent 1 0 R"), PAGES, CATALOG),
            "malformed",
            "page tree Parent link disagrees with Kids",
        ),
        (
            base_pdf(
                &page("/Parent 2 0 R"),
                "<< /Type /Pages /Parent 3 0 R /Kids [3 0 R] /Count 1 >>",
                CATALOG,
            ),
            "malformed",
            "page tree Parent link disagrees with Kids",
        ),
        (
            base_pdf(
                &page("/Parent 2 0 R"),
                PAGES,
                "<< /Type /Catalog /Pages 2 0 R /Perms << >> >>",
            ),
            "unsupported",
            "signed PDF edits are unsupported",
        ),
    ];
    for (bytes, kind, reason) in cases {
        let error = pdf_error(open(bytes));
        assert!(
            (pdf_class(&error) == Some(kind) && error.reason == reason),
            "{reason}: {error:?}"
        );
    }
}

#[test]
fn declared_page_count_is_checked_against_the_page_limit() {
    let limits = Limits {
        max_pages: 0,
        ..Limits::default()
    };
    let error = pdf_error(open_with(build_pdf(&minimal_objects(), ""), &limits));
    assert!(
        matches!(
            error,
            Error {
                kind: ErrorKind::LimitExceeded {
                    resource: "pages",
                    limit: 0,
                    attempted: 1,
                    ..
                },
                context: Context::Pdf {
                    object: Some((2, 0)),
                    ..
                },
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn range_beyond_the_source_is_truncated_input() {
    let bytes = build_pdf(&minimal_objects(), "");
    let length = bytes.len() as u64;
    let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
    let range = PdfRange { offset: 1, length };
    let error = pdf_error(PdfIndex::open(
        &mut source,
        range,
        &Limits::default(),
        &NEVER,
    ));
    assert!(
        matches!(
            error,
            Error { kind: ErrorKind::Truncated { expected, available, .. }, offset: Some(1), .. } if expected == length && available == length - 1
        ),
        "{error:?}"
    );
}

#[test]
fn long_trailer_dictionary_grows_its_window_up_to_the_syntax_limit() {
    let padding = format!("/Pad ({})", "x".repeat(600));
    let doc = build_pdf(&minimal_objects(), &padding);
    assert_eq!(open(doc.clone()).unwrap().pages().len(), 1);

    // A 300-byte syntax window still fits every object but not the trailer.
    let limits = Limits {
        io_chunk_bytes: 64,
        max_allocation_bytes: 300 * 32,
        ..Limits::default()
    };
    let trailer = find(&doc, b"<< /Size");
    let error = pdf_error(open_with(doc, &limits));
    assert!(
        matches!(
            error,
            Error { kind: ErrorKind::LimitExceeded { resource: "PDF dictionary syntax bytes", limit: 300, attempted: 301, .. }, offset: Some(offset), context: Context::Pdf { object: None, .. }, .. } if offset == trailer
        ),
        "{error:?}"
    );
}

#[test]
fn blank_substitutes_require_valid_page_geometry() {
    for (body, valid) in [
        (
            "<</Type/Page /Parent 9 0 R /MediaBox[0 0 20 30] /Rotate 90 /UserUnit 2 /BleedBox[0 0 20 30] /TrimBox[0 0 20 30] /ArtBox[0 0 20 30]>>",
            true,
        ),
        ("<</Type/Page /Parent 9 0 R /CropBox[0 0 0 0]>>", false),
        ("<</Type/Page /Parent 9 0 R /Rotate /Wrong>>", false),
        ("<</Type/Other>>", false),
        ("<</Type/Page>>", false),
        ("null", false),
    ] {
        let bytes = format!("1 0 obj {body} endobj");
        let mut source = SeekableSource::new(Cursor::new(&bytes)).unwrap();
        let object = FragmentObject {
            reference: PdfRef {
                number: 1,
                generation: 0,
            },
            range: PdfRange {
                offset: 0,
                length: bytes.len() as u64,
            },
        };
        assert_eq!(
            blank_fragment_page(&mut source, object, &Limits::default(), &NEVER).is_ok(),
            valid
        );
    }
}

/// Original object-stream fixture. Members are metadata only; callbacks mutate
/// the uncompressed stream or xref rows before independent zlib encoding.
fn compressed_fixture(
    members: &[(u32, &str)],
    extra: &str,
    change_data: impl FnOnce(&mut Vec<u8>),
    change_xref: impl FnOnce(&mut Vec<u8>),
) -> Vec<u8> {
    let mut header = String::new();
    let mut body = Vec::new();
    for &(number, value) in members {
        header.push_str(&format!("{number} {} ", body.len()));
        body.extend_from_slice(value.as_bytes());
        body.push(b'\n');
    }
    let first = header.len();
    let mut decoded = header.into_bytes();
    decoded.extend_from_slice(&body);
    change_data(&mut decoded);
    let encoded = zlib(&decoded);
    let mut pdf = b"%PDF-1.5\n".to_vec();
    let object_at = pdf.len();
    pdf.extend_from_slice(format!("4 0 obj\n<< /Type /ObjStm /N {} /First {first} /Length {} /Filter /FlateDecode {extra} >>\nstream\n", members.len(), encoded.len()).as_bytes());
    pdf.extend_from_slice(&encoded);
    pdf.extend_from_slice(b"\nendstream\nendobj\n");
    let xref_at = pdf.len();
    let mut rows = Vec::new();
    for number in 0..6 {
        let (kind, offset, ordinal) = match number {
            0 => (0_u8, 0_u32, u16::MAX),
            4 => (1, object_at as u32, 0),
            5 => (1, xref_at as u32, 0),
            _ => (
                2,
                4,
                members.iter().position(|(n, _)| *n == number).unwrap_or(0) as u16,
            ),
        };
        rows.push(kind);
        rows.extend_from_slice(&offset.to_be_bytes());
        rows.extend_from_slice(&ordinal.to_be_bytes());
    }
    change_xref(&mut rows);
    let xref = zlib(&rows);
    pdf.extend_from_slice(format!("5 0 obj\n<< /Type /XRef /Size 6 /Root 1 0 R /W [1 4 2] /Length {} /Filter /FlateDecode >>\nstream\n", xref.len()).as_bytes());
    pdf.extend_from_slice(&xref);
    pdf.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
    pdf
}

fn with_up_xref(
    mut doc: Vec<u8>,
    row_width: usize,
    parameters: &str,
    edit: impl FnOnce(&mut Vec<u8>),
) -> Vec<u8> {
    let data_at = find(&doc, b"stream\n") as usize + 7;
    let data_end = find(&doc, b"\nendstream") as usize;
    let raw = doc[data_at..data_end].to_vec();
    let mut predicted = Vec::new();
    for (number, row) in raw.chunks_exact(row_width).enumerate() {
        predicted.push(2);
        for (column, &byte) in row.iter().enumerate() {
            let previous = if number == 0 {
                0
            } else {
                raw[(number - 1) * row_width + column]
            };
            predicted.push(byte.wrapping_sub(previous));
        }
    }
    edit(&mut predicted);
    let encoded = zlib(&predicted);
    doc.splice(data_at..data_end, encoded.iter().copied());
    replace_once(
        &mut doc,
        format!("/Length {}", raw.len()).as_bytes(),
        format!(
            "/Length {} /Filter /FlateDecode /DecodeParms {parameters}",
            encoded.len()
        )
        .as_bytes(),
    );
    doc
}

#[test]
fn png_up_xref_rows_preserve_object_offsets_and_index_subsections() {
    for widths in [[1, 2, 2], [1, 4, 2], [8, 8, 8]] {
        let width = widths.iter().sum();
        let raw = xref_stream_pdf(&minimal_objects(), widths, 0, false);
        let expected = open(raw.clone()).unwrap();
        let mut doc = with_up_xref(
            raw,
            width,
            &format!("<< /Predictor 12 /Columns {width} >>"),
            |_| {},
        );
        // Prediction must not reset when Index advances to another subsection.
        replace_once(&mut doc, b"/Index [0 5]", b"/Index [0 2 2 3]");
        let actual = open_with(
            doc,
            &Limits {
                io_chunk_bytes: 1,
                ..Limits::default()
            },
        )
        .unwrap();
        assert_eq!(actual.pages(), expected.pages());
        for number in 1..4 {
            let reference = PdfRef {
                number,
                generation: 0,
            };
            assert_eq!(
                actual.object_location(reference).unwrap(),
                expected.object_location(reference).unwrap()
            );
        }
    }
}

#[test]
fn png_up_xref_parameters_rows_and_lengths_remain_strict() {
    for parameters in [
        "<< >>",
        "null",
        "[]",
        "1 0 R",
        "<< /Predictor 11 /Columns 7 >>",
        "<< /Predictor 12 /Columns 8 >>",
        "<< /Predictor 12 /Columns 7 /Colors 3 >>",
        "<< /Predictor 12 /Columns 7 /BitsPerComponent 16 >>",
        "<< /Predictor 12 /Columns -7 >>",
        "<< /Predictor 12 /Columns (7) >>",
        "<< /Predictor 12 /Columns 7 /EarlyChange 1 >>",
    ] {
        let doc = with_up_xref(
            xref_stream_pdf(&minimal_objects(), [1, 4, 2], 0, false),
            7,
            parameters,
            |_| {},
        );
        assert!(
            matches!(
                open(doc),
                Err(Error {
                    kind: ErrorKind::UnsupportedFormat,
                    ..
                })
            ),
            "{parameters}"
        );
    }
    for algorithm in [0, 1, 3, 4, 5, 255] {
        let doc = with_up_xref(
            xref_stream_pdf(&minimal_objects(), [1, 4, 2], 0, false),
            7,
            "<< /Predictor 12 /Columns 7 >>",
            |rows| rows[8] = algorithm,
        );
        let error = pdf_error(open(doc));
        assert_eq!(
            pdf_class(&error),
            Some(if algorithm <= 4 {
                "unsupported"
            } else {
                "malformed"
            })
        );
    }
    for extra in [false, true] {
        let doc = with_up_xref(
            xref_stream_pdf(&minimal_objects(), [1, 4, 2], 0, false),
            7,
            "<< /Predictor 12 /Columns 7 >>",
            |rows| {
                if extra {
                    rows.push(0);
                } else {
                    rows.pop();
                }
            },
        );
        assert!(open(doc).is_err());
    }
    assert!(xref_up_parameters(
        b"<< /Pr#65dictor +12 /Columns 7 /Colors 1 /BitsPerComponent 8 >>",
        7
    ));
    assert!(!xref_up_parameters(b"<< /Predictor 12 /Columns 7 >> 0", 7));
    assert!(!xref_up_parameters(
        b"<< /Predictor 12 /Columns 7 /Columns 7 >>",
        7
    ));
}

#[test]
fn compressed_metadata_preserves_references_and_has_no_invented_source_span() {
    let mut members = minimal_objects();
    members.swap(0, 2); // Header order need not equal object-number order.
    let doc = compressed_fixture(&members, "", |_| {}, |_| {});
    let index = open(doc).unwrap();
    assert_eq!(
        index.pages(),
        &[PdfRef {
            number: 3,
            generation: 0
        }]
    );
    assert!(matches!(
        index.object_location(index.catalog()),
        Err(Error {
            kind: ErrorKind::UnsupportedFormat,
            ..
        })
    ));
    assert_eq!(
        index
            .object_location(PdfRef {
                number: 4,
                generation: 0
            })
            .unwrap()
            .offset,
        9
    );
}

#[test]
fn compressed_metadata_rejects_bad_headers_members_and_xref_ordinals() {
    for mutation in 0..8 {
        let doc = compressed_fixture(
            &minimal_objects(),
            "",
            |data| match mutation {
                0 => data[0] = b'0', // Reserved object zero.
                1 => data[0] = b'2', // Duplicate number.
                2 => data[2] = b'9', // Nonzero first offset.
                3 => {
                    data.pop();
                    data.pop();
                } // Truncated final dictionary.
                4 => data.extend_from_slice(b" 0"), // More than one member value.
                _ => {}
            },
            |rows| match mutation {
                5 => rows[7 + 6] = 2,   // Object 1 is not member 2.
                6 => rows[7 + 4] = 1,   // Container itself is compressed.
                7 => rows[7 + 6] = 250, // Missing ordinal.
                _ => {}
            },
        );
        assert!(open(doc).is_err(), "mutation {mutation}");
    }
    for extra in ["/Extends 4 0 R", "/DecodeParms << >>", "/F (external)"] {
        assert!(
            matches!(
                open(compressed_fixture(
                    &minimal_objects(),
                    extra,
                    |_| {},
                    |_| {}
                )),
                Err(Error {
                    kind: ErrorKind::UnsupportedFormat,
                    ..
                })
            ),
            "{extra}"
        );
    }
}

#[test]
fn later_standalone_revision_supersedes_compressed_member() {
    let mut doc = compressed_fixture(&minimal_objects(), "", |_| {}, |_| {});
    let marker = find(&doc, b"startxref\n") as usize + 10;
    let previous = std::str::from_utf8(&doc[marker..])
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_owned();
    let page_at = doc.len();
    doc.extend_from_slice(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 20 30] >>\nendobj\n",
    );
    let xref_at = doc.len();
    doc.extend_from_slice(format!("xref\n3 1\n{page_at:010} 00000 n \ntrailer\n<< /Size 6 /Root 1 0 R /Prev {previous} >>\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
    let index = open(doc).unwrap();
    assert_eq!(
        index
            .object_location(PdfRef {
                number: 3,
                generation: 0
            })
            .unwrap()
            .offset,
        page_at as u64
    );
    assert!(
        index
            .compressed_object(PdfRef {
                number: 3,
                generation: 0
            })
            .is_none()
    );
}

#[test]
fn cancellation_covers_up_xref_and_compressed_metadata_loading() {
    let fixtures = [
        with_up_xref(
            xref_stream_pdf(&minimal_objects(), [1, 4, 2], 0, false),
            7,
            "<< /Predictor 12 /Columns 7 >>",
            |_| {},
        ),
        compressed_fixture(&minimal_objects(), "", |_| {}, |_| {}),
    ];
    for doc in fixtures {
        for allowed in 0..10000 {
            match open_cancellable(doc.clone(), &Limits::default(), &CancelAfter::new(allowed)) {
                Ok(_) => {
                    assert!(allowed > 5);
                    break;
                }
                Err(Error {
                    kind: ErrorKind::Cancelled,
                    ..
                }) => {}
                Err(error) => panic!("checkpoint {allowed}: {error}"),
            }
            assert!(allowed < 9999);
        }
    }
}

#[test]
fn compressed_metadata_limits_and_integer_overflow_are_located() {
    let limits = Limits {
        io_chunk_bytes: 64,
        max_allocation_bytes: 512 * 32,
        ..Limits::default()
    };
    let make = |size| {
        compressed_fixture(
            &minimal_objects(),
            "",
            |data| data.resize(size, b' '),
            |_| {},
        )
    };
    open_with(make(512), &limits).unwrap();
    let error = pdf_error(open_with(make(513), &limits));
    assert!(
        matches!(
            error,
            Error {
                kind: ErrorKind::LimitExceeded {
                    resource: "PDF object stream decoded bytes",
                    limit: 512,
                    attempted: 513
                },
                offset: Some(9),
                context: Context::Pdf {
                    object: Some((4, 0)),
                    ..
                },
                ..
            }
        ),
        "{error:?}"
    );
    for field in ["18446744073709551616 0 ", "1 18446744073709551616 "] {
        let doc = compressed_fixture(
            &minimal_objects(),
            "",
            |data| {
                data.splice(..4, field.bytes());
            },
            |_| {},
        );
        let error = pdf_error(open(doc));
        assert!(
            matches!(
                error,
                Error {
                    kind: ErrorKind::Malformed,
                    offset: Some(9),
                    context: Context::Pdf {
                        object: Some((4, 0)),
                        ..
                    },
                    ..
                }
            ),
            "{error:?}"
        );
    }
    let mut doc = compressed_fixture(&minimal_objects(), "", |_| {}, |_| {});
    replace_once(&mut doc, b"/N 3 ", b"/N 0 ");
    assert!(matches!(
        open(doc),
        Err(Error {
            kind: ErrorKind::Malformed,
            ..
        })
    ));
}

#[test]
fn object_stream_inflation_requires_complete_zlib_and_exact_encoded_extent() {
    let encoded = zlib(b"1 0 null");
    for mutated in [
        encoded[..encoded.len() - 1].to_vec(),
        [encoded.as_slice(), b"x"].concat(),
    ] {
        assert!(matches!(
            inflate_pdf_stream(&mutated, 100, false, 1, &NEVER),
            Err(Error {
                kind: ErrorKind::Malformed,
                ..
            })
        ));
    }
    assert_eq!(
        inflate_pdf_stream(&encoded, 8, false, 1, &NEVER).unwrap(),
        b"1 0 null"
    );
}

#[test]
fn object_stream_members_cannot_contain_streams_or_dangling_references() {
    for member in [
        "<< /Type /Page /Parent 2 1 R /MediaBox [0 0 20 30] >>",
        "<< /Length 1 >> stream\nx\nendstream",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 20 30] /Contents 1 0 R >>",
    ] {
        let mut members = minimal_objects();
        members[2].1 = member;
        assert!(open(compressed_fixture(&members, "", |_| {}, |_| {})).is_err());
    }
}

#[test]
fn fragment_page_box_repair_keeps_conflicts_and_other_duplicates_strict() {
    for body in [
        "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 10 20] /MediaBox [0 0 11 20] >>",
        "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 10 20] /MediaBox [0 0 10 20] /MediaBox [0 0 10 20] >>",
        "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 0 20] /MediaBox [0 0 0 20] >>",
        "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 10 20] /MediaBox 6 0 R >>",
        "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 10 20] /MediaBox [0 0 10 20] /Other 0 /Other 0 >>",
        "<< /Type /Font /MediaBox [0 0 10 20] /MediaBox [0 0 10 20] >>",
        "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 10 20] /MediaBox [0 0 10 20] /Length 0 >> stream\n\nendstream",
    ] {
        let bytes = format!("3 0 obj {body} endobj");
        assert!(
            inspect_generated_object(bytes.as_bytes(), &Limits::default()).is_err(),
            "{body}"
        );
    }
    let bytes = b"3 0 obj << /Type /Page /Parent 5 0 R /MediaBox [0 0 10 20] /Media#42ox [0 0 10.0 +20] >> endobj";
    let inspection = inspect_generated_object(bytes, &Limits::default()).unwrap();
    let (start, end) = inspection.blank_media_box.unwrap();
    assert_eq!(&bytes[start..end], b"/Media#42ox [0 0 10.0 +20]");
    assert!(matches!(
        inspect_generated_object(
            bytes,
            &Limits {
                max_allocation_bytes: 1,
                ..Limits::default()
            }
        ),
        Err(Error {
            kind: ErrorKind::LimitExceeded { .. },
            ..
        })
    ));
}
