// SPDX-License-Identifier: MIT

//! Adversarial, generated PDF inputs for the bounded reader contract.

use caj2pdf_core::{
    Cancellation, Error, Limits, NeverCancel, PdfErrorKind, RangedSource, Result,
    native::{SeekableSource, WriteSink},
    pdf::{PdfIndex, PdfRange, PdfRef, copy_pdf},
};
use std::{
    cell::Cell,
    future::Future,
    io::Cursor,
    pin::pin,
    task::{Context, Poll, Waker},
};

fn run<F: Future>(future: F) -> F::Output {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = pin!(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("immediate test source unexpectedly yielded"),
    }
}

struct Fixture {
    bytes: Vec<u8>,
    offsets: Vec<usize>,
    xref: usize,
}

fn fixture(stream_body: &[u8], length_object: Option<&[u8]>) -> Fixture {
    fixture_with(
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << >> /Contents 4 0 R >>",
        stream_body,
        length_object,
    )
}

fn fixture_with(
    catalog: &[u8],
    pages: &[u8],
    page: &[u8],
    stream_body: &[u8],
    length_object: Option<&[u8]>,
) -> Fixture {
    let mut objects: Vec<&[u8]> = vec![catalog, pages, page, stream_body];
    if let Some(length) = length_object {
        objects.push(length);
    }
    let mut bytes = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        bytes.extend_from_slice(body);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    bytes.extend_from_slice(b"0000000000 65535 f \n");
    for offset in &offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    Fixture {
        bytes,
        offsets,
        xref,
    }
}

fn stream(data: &[u8], length: &str, ending: &[u8]) -> Vec<u8> {
    let mut body = format!("<< /Length {length} >>\nstream\n").into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(ending);
    body
}

fn ordinary_fixture() -> Fixture {
    fixture(&stream(b"q Q", "3", b"\nendstream"), None)
}

fn inspect(bytes: Vec<u8>, limits: &Limits) -> Result<PdfIndex> {
    let size = bytes.len() as u64;
    let mut source = SeekableSource::new(Cursor::new(bytes))?;
    run(PdfIndex::open(
        &mut source,
        PdfRange {
            offset: 0,
            length: size,
        },
        limits,
        &NeverCancel,
    ))
}

fn assert_pdf_error(bytes: Vec<u8>, kind: PdfErrorKind) -> Error {
    let error = match inspect(bytes, &Limits::default()) {
        Err(error) => error,
        Ok(_) => panic!("invalid generated PDF was accepted"),
    };
    assert!(
        matches!(error, Error::Pdf { kind: actual, .. } if actual == kind),
        "{error}"
    );
    error
}

fn replace_once(bytes: &mut Vec<u8>, old: &[u8], new: &[u8]) {
    let at = bytes
        .windows(old.len())
        .position(|window| window == old)
        .expect("expected mutation target");
    assert!(
        bytes[at + old.len()..]
            .windows(old.len())
            .all(|window| window != old)
    );
    bytes.splice(at..at + old.len(), new.iter().copied());
}

#[test]
fn one_byte_ranged_reads_preserve_binary_stream_markers_and_absolute_offset() {
    let payload = b"q\nendstream\nendobj\nxref\nstartxref\n%%EOF\nQ";
    let pdf = fixture(
        &stream(payload, &payload.len().to_string(), b"\rendstream"),
        None,
    );
    let prefix = b"CAJ container prefix";
    let mut bytes = prefix.to_vec();
    bytes.extend_from_slice(&pdf.bytes);
    let mut source = SmallReads {
        bytes,
        max_read: 1,
        calls: 0,
        max_request: 0,
        fail_after: None,
    };
    let index = run(PdfIndex::open(
        &mut source,
        PdfRange {
            offset: prefix.len() as u64,
            length: pdf.bytes.len() as u64,
        },
        &Limits::default(),
        &NeverCancel,
    ))
    .unwrap();
    assert_eq!(
        index.pages(),
        &[PdfRef {
            number: 3,
            generation: 0
        }]
    );
    assert_eq!(
        index
            .object_location(PdfRef {
                number: 4,
                generation: 0
            })
            .unwrap()
            .offset,
        pdf.offsets[3] as u64
    );
    assert!(
        source.calls > pdf.bytes.len(),
        "reader did not exercise short reads"
    );
    assert!(source.max_request <= Limits::default().io_chunk_bytes);
}

#[test]
fn header_and_tail_errors_are_located_before_any_output() {
    let valid = ordinary_fixture().bytes;
    for bytes in [b"%PDF-1.".to_vec(), b"garbage at header".to_vec()] {
        let error = assert_pdf_error(bytes, PdfErrorKind::Malformed);
        assert!(matches!(error, Error::Pdf { offset: 0, .. }));
    }
    let mut invalid_version = valid.clone();
    invalid_version[..8].copy_from_slice(b"%PDF-1.9");
    assert_pdf_error(invalid_version, PdfErrorKind::Malformed);

    let mut missing_eof = valid.clone();
    missing_eof.truncate(missing_eof.len() - b"%%EOF\n".len());
    assert_pdf_error(missing_eof, PdfErrorKind::Malformed);

    let mut overflow = valid.clone();
    let start = overflow
        .windows(b"startxref\n".len())
        .rposition(|w| w == b"startxref\n")
        .unwrap();
    let old = overflow[start..].to_vec();
    overflow.splice(
        start..,
        b"startxref\n184467440737095516160\n%%EOF\n".iter().copied(),
    );
    assert_ne!(old, overflow[start..]);
    let error = assert_pdf_error(overflow, PdfErrorKind::Malformed);
    assert!(
        error.to_string().contains("startxref offset overflows"),
        "{error}"
    );

    let mut nonclassic = valid;
    let xref = ordinary_fixture().xref;
    nonclassic[xref..xref + 4].copy_from_slice(b"1234");
    assert_pdf_error(nonclassic, PdfErrorKind::UnsupportedFeature);
}

#[test]
fn source_range_and_index_access_are_checked() {
    let pdf = ordinary_fixture();
    let size = pdf.bytes.len() as u64;
    for (range, expected) in [
        (
            PdfRange {
                offset: u64::MAX,
                length: 2,
            },
            "PDF source range overflows",
        ),
        (
            PdfRange {
                offset: 0,
                length: size + 1,
            },
            "truncated input",
        ),
    ] {
        let mut source = SeekableSource::new(Cursor::new(pdf.bytes.clone())).unwrap();
        let error = run(PdfIndex::open(
            &mut source,
            range,
            &Limits::default(),
            &NeverCancel,
        ))
        .err()
        .expect("out-of-bounds range should fail");
        assert!(error.to_string().contains(expected), "{error}");
    }
    let input_limit = Limits {
        max_input_bytes: size - 1,
        ..Limits::default()
    };
    let error = inspect(pdf.bytes.clone(), &input_limit).err().unwrap();
    assert!(matches!(
        error,
        Error::PdfLimitExceeded {
            resource: "input bytes",
            offset: 0,
            ..
        }
    ));

    let index = inspect(pdf.bytes, &Limits::default()).unwrap();
    assert_eq!(
        index.range(),
        PdfRange {
            offset: 0,
            length: size
        }
    );
    assert_eq!(index.logical_end(), size);
    assert_eq!(index.xref_offset(), pdf.xref as u64);
    assert_eq!(index.trailer_size(), 5);
    assert_eq!(
        index.catalog(),
        PdfRef {
            number: 1,
            generation: 0
        }
    );
    assert_eq!(index.next_free_object_number().unwrap(), 5);
    assert_eq!(index.max_referenced_object(), 4);
    for invalid in [
        PdfRef {
            number: 2,
            generation: 1,
        },
        PdfRef {
            number: 5,
            generation: 0,
        },
    ] {
        let error = index.object_location(invalid).unwrap_err();
        assert!(
            matches!(
                error,
                Error::Pdf {
                    kind: PdfErrorKind::Malformed,
                    object: Some(_),
                    ..
                }
            ),
            "{error}"
        );
    }
}

#[test]
fn tail_requires_an_unambiguous_final_revision() {
    let original = ordinary_fixture().bytes;
    for replacement in [
        b"startxreX\n".as_slice(),
        b"startxref\nx\n".as_slice(),
        b"startxref\n9999999999\n".as_slice(),
        b"startxref\n0x\n".as_slice(),
    ] {
        let mut bytes = original.clone();
        let marker = b"startxref\n";
        let at = bytes
            .windows(marker.len())
            .rposition(|part| part == marker)
            .unwrap();
        bytes.splice(
            at..bytes.len() - b"%%EOF\n".len(),
            replacement.iter().copied(),
        );
        let error = assert_pdf_error(bytes, PdfErrorKind::Malformed);
        assert!(error.to_string().contains("startxref and EOF"), "{error}");
    }

    let mut unsafe_footer = original.clone();
    unsafe_footer.extend_from_slice(b"WebFastLoadW untrusted xref 0 1");
    assert_pdf_error(unsafe_footer, PdfErrorKind::AmbiguousRepair);

    let mut safe_footer = original.clone();
    safe_footer.extend_from_slice(b"WebFastLoadW\x00binary\xff");
    let index = inspect(safe_footer, &Limits::default()).unwrap();
    assert_eq!(index.logical_end(), original.len() as u64);

    let mut out_of_window = original;
    out_of_window.extend(std::iter::repeat_n(b'A', 65_536));
    assert_pdf_error(out_of_window, PdfErrorKind::Malformed);
}

#[test]
fn trailer_and_object_syntax_budgets_are_located() {
    let base = ordinary_fixture();
    let mut trailer = base.bytes;
    let pad = vec![b'A'; 700];
    let mut expanded = b"/Pad (".to_vec();
    expanded.extend_from_slice(&pad);
    expanded.extend_from_slice(b") >>");
    replace_once(
        &mut trailer,
        b"/Root 1 0 R >>",
        &[b"/Root 1 0 R ".as_slice(), &expanded].concat(),
    );
    assert_eq!(
        inspect(trailer.clone(), &Limits::default())
            .unwrap()
            .pages()
            .len(),
        1
    );

    let limited = Limits {
        io_chunk_bytes: 64,
        max_allocation_bytes: 4096,
        ..Limits::default()
    };
    let error = inspect(trailer, &limited).err().unwrap();
    assert!(
        matches!(
            error,
            Error::PdfLimitExceeded {
                resource: "PDF dictionary syntax bytes",
                ..
            }
        ),
        "{error}"
    );

    let mut catalog = b"<< /Type /Catalog /Pages 2 0 R /Pad (".to_vec();
    catalog.extend_from_slice(&pad);
    catalog.extend_from_slice(b") >>");
    let large_object = fixture_with(
        &catalog,
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R >>",
        &stream(b"q Q", "3", b"\nendstream"),
        None,
    );
    assert_eq!(
        inspect(large_object.bytes.clone(), &Limits::default())
            .unwrap()
            .pages()
            .len(),
        1
    );
    let error = inspect(large_object.bytes, &limited).err().unwrap();
    assert!(
        matches!(
            error,
            Error::PdfLimitExceeded {
                resource: "PDF object syntax bytes",
                object: Some((1, 0)),
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn malformed_xref_rows_and_trailer_fields_are_rejected() {
    let original = ordinary_fixture();
    let mut invalid_row = original.bytes.clone();
    invalid_row[original.xref + b"xref\n0 5\n".len() + 20 + 17] = b'x';
    assert_pdf_error(invalid_row, PdfErrorKind::Malformed);

    let mut duplicate = original.bytes.clone();
    replace_once(
        &mut duplicate,
        b"trailer\n",
        b"0 1\n0000000000 65535 f \ntrailer\n",
    );
    let duplicate_error = assert_pdf_error(duplicate, PdfErrorKind::Malformed);
    assert!(duplicate_error.to_string().contains("duplicate xref entry"));

    for trailer in [
        b"/Size 0 /Root 1 0 R".as_slice(),
        b"/Size 5 /Root 0 0 R".as_slice(),
        b"/Size 5 /Root 1 0 R /Prev /Wrong".as_slice(),
    ] {
        let mut bytes = original.bytes.clone();
        replace_once(&mut bytes, b"/Size 5 /Root 1 0 R", trailer);
        assert_pdf_error(bytes, PdfErrorKind::Malformed);
    }
}

#[test]
fn xref_membership_and_object_headers_must_agree() {
    let original = ordinary_fixture();
    let first_row = original.xref + b"xref\n0 5\n".len() + 20;

    let mut too_small = original.bytes.clone();
    replace_once(&mut too_small, b"/Size 5 /Root", b"/Size 4 /Root");
    let error = assert_pdf_error(too_small, PdfErrorKind::Malformed);
    assert!(
        error.to_string().contains("exceeds trailer Size"),
        "{error}"
    );

    let mut wrong_object = original.bytes.clone();
    wrong_object[first_row..first_row + 10]
        .copy_from_slice(format!("{:010}", original.offsets[1]).as_bytes());
    let error = assert_pdf_error(wrong_object, PdfErrorKind::Malformed);
    assert!(
        matches!(
            error,
            Error::Pdf {
                object: Some((1, 0)),
                ..
            }
        ),
        "{error}"
    );
    assert!(
        error.to_string().contains("different object header"),
        "{error}"
    );

    let mut wrong_generation = original.bytes.clone();
    wrong_generation[first_row + 11..first_row + 16].copy_from_slice(b"00001");
    let error = assert_pdf_error(wrong_generation, PdfErrorKind::Malformed);
    assert!(
        matches!(
            error,
            Error::Pdf {
                object: Some((1, 1)),
                ..
            }
        ),
        "{error}"
    );

    let mut freed_root = original.bytes;
    freed_root[first_row + 17] = b'f';
    let error = assert_pdf_error(freed_root, PdfErrorKind::Malformed);
    assert!(
        matches!(
            error,
            Error::Pdf {
                object: Some((1, 0)),
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn missing_and_duplicate_trailer_keys_are_not_inferred() {
    let original = ordinary_fixture().bytes;
    for (replacement, reason) in [
        (b"/Root 1 0 R".as_slice(), "lacks Size"),
        (b"/Size 5".as_slice(), "lacks Root"),
        (
            b"/Size 5 /Root 1 0 R /Root 1 0 R".as_slice(),
            "duplicate PDF dictionary keys",
        ),
    ] {
        let mut bytes = original.clone();
        replace_once(&mut bytes, b"/Size 5 /Root 1 0 R", replacement);
        let kind = if reason.starts_with("duplicate") {
            PdfErrorKind::AmbiguousRepair
        } else {
            PdfErrorKind::Malformed
        };
        let error = assert_pdf_error(bytes, kind);
        assert!(error.to_string().contains(reason), "{error}");
    }
}

#[test]
fn xref_subsection_numbers_and_offsets_are_checked() {
    let original = ordinary_fixture();
    for (new, reason) in [
        (b"0 nope\n".as_slice(), "expected nonnegative PDF integer"),
        (
            b"123456789012345678901 1\n".as_slice(),
            "PDF token is too long",
        ),
        (b"0 0\n".as_slice(), "invalid PDF xref subsection range"),
    ] {
        let mut bytes = original.bytes.clone();
        replace_once(&mut bytes, b"0 5\n", new);
        let error = assert_pdf_error(bytes, PdfErrorKind::Malformed);
        assert!(error.to_string().contains(reason), "{error}");
    }

    let mut oversized = original.bytes.clone();
    replace_once(&mut oversized, b"0 5\n", b"8388607 2\n");
    let error = inspect(oversized, &Limits::default()).err().unwrap();
    assert!(
        matches!(
            error,
            Error::PdfLimitExceeded {
                resource: "PDF object index",
                ..
            }
        ),
        "{error}"
    );

    let mut bad_offset = original.bytes;
    let first_row = original.xref + b"xref\n0 5\n".len() + 20;
    bad_offset[first_row..first_row + 10].copy_from_slice(b"9999999999");
    let error = assert_pdf_error(bad_offset, PdfErrorKind::Malformed);
    assert!(
        matches!(
            error,
            Error::Pdf {
                object: Some((1, 0)),
                ..
            }
        ),
        "{error}"
    );
    assert!(
        error.to_string().contains("offset exceeds PDF range"),
        "{error}"
    );
}

#[test]
fn comments_in_xref_are_accepted_but_unindexed_object_bytes_are_not() {
    let mut commented = ordinary_fixture().bytes;
    replace_once(
        &mut commented,
        b"xref\n0 5\n",
        b"xref\n% generated comment\r\n0 5\n",
    );
    assert_eq!(
        inspect(commented, &Limits::default())
            .unwrap()
            .pages()
            .len(),
        1
    );

    let fixture = ordinary_fixture();
    let mut unindexed = fixture.bytes;
    let stray = b"5 0 obj\n0\nendobj\n";
    unindexed.splice(fixture.xref..fixture.xref, stray.iter().copied());
    replace_once(
        &mut unindexed,
        format!("startxref\n{}\n", fixture.xref).as_bytes(),
        format!("startxref\n{}\n", fixture.xref + stray.len()).as_bytes(),
    );
    let error = assert_pdf_error(unindexed, PdfErrorKind::Malformed);
    assert!(
        error
            .to_string()
            .contains("unindexed bytes between PDF objects"),
        "{error}"
    );
}

#[test]
fn previous_xref_must_move_backward_and_revision_chain_is_bounded() {
    let original = ordinary_fixture();
    let mut self_reference = original.bytes.clone();
    let xref = self_reference.len();
    append_revision(&mut self_reference, xref);
    let error = assert_pdf_error(self_reference, PdfErrorKind::Malformed);
    assert!(
        error
            .to_string()
            .contains("Prev must point to an earlier section"),
        "{error}"
    );

    let mut many = original.bytes;
    let mut previous = original.xref;
    for _ in 0..65 {
        let here = many.len();
        append_revision(&mut many, previous);
        previous = here;
    }
    let error = assert_pdf_error(many, PdfErrorKind::Malformed);
    assert!(
        error.to_string().contains("xref revision limit exceeded"),
        "{error}"
    );
}

fn append_revision(bytes: &mut Vec<u8>, previous: usize) {
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!(
            "xref\n0 1\n0000000000 65535 f \ntrailer\n<< /Size 5 /Root 1 0 R /Prev {previous} >>\nstartxref\n{xref}\n%%EOF\n"
        )
        .as_bytes(),
    );
}

#[test]
fn stream_length_and_framing_fail_at_the_stream_object() {
    let cases: Vec<(Vec<u8>, Option<&[u8]>)> = vec![
        (stream(b"q Q", "5", b"\nendstream"), None),
        (stream(b"q Q", "-1", b"\nendstream"), None),
        (stream(b"q Q", "5 0 R", b"\nendstream"), None),
        (stream(b"q Q", "5 0 R", b"\nendstream"), Some(b"(three)")),
        (stream(b"q Q", "5 0 R", b"\nendstream"), Some(b"-3")),
        (stream(b"q Q", "3", b"\nendstreaX"), None),
        (stream(b"q Q", "3", b"\nendstream\njunk"), None),
        (stream(b"q Q", "3.5", b"\nendstream"), None),
        (b"<< >>\nstream\nq Q\nendstream".to_vec(), None),
    ];
    for (body, scalar) in cases {
        let fixture = fixture(&body, scalar);
        let error = assert_pdf_error(fixture.bytes, PdfErrorKind::Malformed);
        assert!(
            matches!(
                error,
                Error::Pdf {
                    object: Some((4, 0)) | Some((5, 0)),
                    ..
                }
            ),
            "{error}"
        );
    }
}

#[test]
fn indirect_stream_length_can_resolve_and_must_be_scalar() {
    let payload = b"q\n%%EOF\nQ";
    let valid = fixture(
        &stream(payload, "5 0 R", b"\r\nendstream"),
        Some(payload.len().to_string().as_bytes()),
    );
    assert_eq!(
        inspect(valid.bytes, &Limits::default())
            .unwrap()
            .pages()
            .len(),
        1
    );

    let nested_stream = stream(b"0", "1", b"\nendstream");
    let invalid = fixture(
        &stream(payload, "5 0 R", b"\nendstream"),
        Some(&nested_stream),
    );
    let error = assert_pdf_error(invalid.bytes, PdfErrorKind::Malformed);
    assert!(
        matches!(
            error,
            Error::Pdf {
                object: Some((5, 0)),
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn stream_extents_and_indirect_references_cannot_escape_the_document() {
    let enormous = fixture(
        &stream(b"q Q", "18446744073709551615", b"\nendstream"),
        None,
    );
    let error = assert_pdf_error(enormous.bytes, PdfErrorKind::Malformed);
    assert!(
        error.to_string().contains("stream extent overflows"),
        "{error}"
    );

    let past_eof = fixture(&stream(b"q Q", "9999999999", b"\nendstream"), None);
    let error = assert_pdf_error(past_eof.bytes, PdfErrorKind::Malformed);
    assert!(
        error
            .to_string()
            .contains("PDF range ends inside required syntax"),
        "{error}"
    );

    let dangling = fixture(
        b"<< /Length 3 /Filter 99 0 R >>\nstream\nq Q\nendstream",
        None,
    );
    let error = assert_pdf_error(dangling.bytes, PdfErrorKind::Malformed);
    assert!(
        matches!(
            error,
            Error::Pdf {
                object: Some((4, 0)),
                ..
            }
        ),
        "{error}"
    );
    assert!(
        error.to_string().contains("dangling indirect reference"),
        "{error}"
    );
}

#[test]
fn cancelled_and_stalled_ranged_sources_never_produce_a_copy() {
    let bytes = ordinary_fixture().bytes;
    let mut source = SmallReads {
        bytes: bytes.clone(),
        max_read: 1,
        calls: 0,
        max_request: 0,
        fail_after: Some(3),
    };
    let mut sink = WriteSink::new(Vec::<u8>::new());
    let error = run(copy_pdf(
        &mut source,
        &mut sink,
        &Limits::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(matches!(error, Error::TruncatedInput { .. }), "{error}");
    assert!(sink.into_inner().is_empty());

    let mut source = SmallReads {
        bytes,
        max_read: 1,
        calls: 0,
        max_request: 0,
        fail_after: None,
    };
    let signal = CancelAfter {
        polls: Cell::new(0),
        limit: 4,
    };
    let mut sink = WriteSink::new(Vec::<u8>::new());
    let error = run(copy_pdf(
        &mut source,
        &mut sink,
        &Limits::default(),
        &signal,
    ))
    .unwrap_err();
    assert!(matches!(error, Error::Cancelled));
    assert!(sink.into_inner().is_empty());
}

#[test]
fn xref_index_allocation_limit_has_pdf_location() {
    let mut pdf = ordinary_fixture().bytes;
    replace_once(&mut pdf, b"/Size 5 /Root", b"/Size 1000 /Root");
    let limits = Limits {
        io_chunk_bytes: 64,
        max_allocation_bytes: 1024,
        ..Limits::default()
    };
    let error = inspect(pdf, &limits).err().unwrap();
    assert!(
        matches!(
            error,
            Error::PdfLimitExceeded {
                resource: "allocation bytes",
                ..
            }
        ),
        "{error}"
    );
}

struct SmallReads {
    bytes: Vec<u8>,
    max_read: usize,
    calls: usize,
    max_request: usize,
    fail_after: Option<usize>,
}

impl RangedSource for SmallReads {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        self.calls += 1;
        self.max_request = self.max_request.max(destination.len());
        if self.fail_after.is_some_and(|after| self.calls > after) {
            return Ok(0);
        }
        let start = offset as usize;
        if start >= self.bytes.len() {
            return Ok(0);
        }
        let count = destination
            .len()
            .min(self.max_read)
            .min(self.bytes.len() - start);
        destination[..count].copy_from_slice(&self.bytes[start..start + count]);
        Ok(count)
    }
}

struct CancelAfter {
    polls: Cell<usize>,
    limit: usize,
}

impl Cancellation for CancelAfter {
    fn is_cancelled(&self) -> bool {
        let polls = self.polls.get() + 1;
        self.polls.set(polls);
        polls >= self.limit
    }
}
