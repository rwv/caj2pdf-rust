// SPDX-License-Identifier: MIT

use super::*;
use crate::NeverCancel;
use crate::native::SeekableSource;
use crate::test_support::{CancelAfter, run};
use std::io::Cursor;

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
    run(PdfIndex::open(
        &mut source,
        PdfRange {
            offset: 0,
            length: len,
        },
        limits,
        cancellation,
    ))
}

fn open_with(bytes: Vec<u8>, limits: &Limits) -> Result<PdfIndex> {
    open_cancellable(bytes, limits, &NeverCancel)
}

fn open(bytes: Vec<u8>) -> Result<PdfIndex> {
    open_with(bytes, &Limits::default())
}

fn base_pdf(page: &str, pages: &str, catalog: &str) -> Vec<u8> {
    build_pdf(&[(1, catalog), (2, pages), (3, page)], "")
}

fn expect_pdf_error(bytes: Vec<u8>, kind: PdfErrorKind) {
    let error = pdf_error(open(bytes));
    assert!(
        matches!(error, Error::Pdf { kind: found, .. } if found == kind),
        "{error:?}"
    );
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
    let scan = run(scan_fragment_objects(
        &mut source,
        0,
        hint - 3,
        &Limits::default(),
        &NeverCancel,
    ))
    .unwrap();
    assert_eq!(scan.objects.len(), 2);
    assert_eq!(scan.objects[0].reference.number, 1);
    assert_eq!(scan.objects[1].reference.number, 2);
    assert_eq!(scan.objects.last().unwrap().range.end(), Some(hint));
    assert_eq!(scan.patches.len(), 1);
    let mut patched = PatchedSource::new(&mut source, &scan.patches);
    let mut length = vec![0; actual.to_string().len()];
    run(read_exact_at(
        &mut patched,
        scan.patches[0].offset,
        &mut length,
        &Limits::default(),
        &NeverCancel,
    ))
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
    let result = run(scan_fragment_objects(
        &mut source,
        0,
        hint,
        &Limits::default(),
        &NeverCancel,
    ));
    assert!(matches!(
        result,
        Err(Error::Pdf {
            kind: PdfErrorKind::AmbiguousRepair,
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
    let error = run(scan_fragment_objects(
        &mut source,
        0,
        hint,
        &Limits::default(),
        &NeverCancel,
    ))
    .err()
    .expect("fake final stream terminator was accepted");
    assert!(matches!(
        error,
        Error::Pdf {
            kind: PdfErrorKind::AmbiguousRepair,
            reason: "repaired final stream has an unrecognized CAJ trailer",
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
    let scan = run(scan_fragment_objects(
        &mut source,
        0,
        bytes.len() as u64,
        &Limits::default(),
        &NeverCancel,
    ))
    .unwrap();
    assert_eq!(scan.objects.len(), 1);
    assert_eq!(scan.objects[0].range.length, bytes.len() as u64);
}

#[test]
fn fragment_scanner_uses_declared_stream_extent_even_with_complete_fake_terminator() {
    let payload = b"prefix\rendstream\rendobj\nsecond half";
    let mut bytes = format!("1 0 obj\n<< /Length {} >>\nstream\n", payload.len()).into_bytes();
    bytes.extend_from_slice(payload);
    bytes.extend_from_slice(b"\r\nendstream\rendobj");
    let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
    let scan = run(scan_fragment_objects(
        &mut source,
        0,
        bytes.len() as u64,
        &Limits::default(),
        &NeverCancel,
    ))
    .unwrap();
    assert_eq!(scan.objects.len(), 1);
    assert!(scan.patches.is_empty());
    assert_eq!(scan.objects[0].range.length, bytes.len() as u64);
}

#[test]
fn fragment_scanner_rejects_unbounded_or_width_changing_stream_repairs() {
    let make = |declared: usize, actual: usize| {
        let mut bytes = format!("1 0 obj\n<< /Length {declared} >>\nstream\n").into_bytes();
        bytes.extend(std::iter::repeat_n(b'x', actual));
        bytes.extend_from_slice(b"\r\nendstream\rendobj");
        bytes
    };
    for (declared, actual, kind) in [
        (1, 90, PdfErrorKind::Malformed),
        (9, 10, PdfErrorKind::UnsupportedFeature),
    ] {
        let bytes = make(declared, actual);
        let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
        let error = run(scan_fragment_objects(
            &mut source,
            0,
            bytes.len() as u64,
            &Limits::default(),
            &NeverCancel,
        ))
        .err()
        .expect("unsafe stream repair was accepted");
        assert!(matches!(error, Error::Pdf { kind: found, .. } if found == kind));
    }
}

#[test]
fn fragment_scanner_rejects_unresolved_length_and_body_budget_before_output() {
    let bytes = b"1 0 obj\n<< /Length 9 0 R >>\nstream\nabc\r\nendstream\rendobj";
    let mut source = SeekableSource::new(Cursor::new(bytes.to_vec())).unwrap();
    let result = run(scan_fragment_objects(
        &mut source,
        0,
        bytes.len() as u64,
        &Limits::default(),
        &NeverCancel,
    ));
    assert!(matches!(
        result,
        Err(Error::Pdf {
            kind: PdfErrorKind::UnsupportedFeature,
            ..
        })
    ));

    let limits = Limits {
        max_input_bytes: bytes.len() as u64 - 1,
        ..Limits::default()
    };
    let result = run(scan_fragment_objects(
        &mut source,
        0,
        bytes.len() as u64,
        &limits,
        &NeverCancel,
    ));
    assert!(matches!(
        result,
        Err(Error::CajLimitExceeded {
            offset: 0,
            resource: "input bytes",
            ..
        })
    ));
}

#[test]
fn fragment_scanner_rejects_invalid_ranges_and_unfinished_objects() {
    let mut source = SeekableSource::new(Cursor::new(b"1 0 obj\nnull\nendobj".to_vec())).unwrap();
    for (start, end) in [(0, 0), (10, 10), (0, source.size() + 1)] {
        assert!(matches!(
            run(scan_fragment_objects(
                &mut source,
                start,
                end,
                &Limits::default(),
                &NeverCancel
            )),
            Err(Error::Caj {
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
            run(scan_fragment_objects(
                &mut source,
                0,
                bytes.len() as u64,
                &Limits::default(),
                &NeverCancel
            )),
            Err(Error::Pdf {
                kind: PdfErrorKind::Malformed,
                ..
            })
        ));
    }
}

#[test]
fn fragment_scanner_rejects_unsupported_generation_and_stream_framing() {
    for (bytes, kind) in [
        (
            b"1 1 obj\nnull\nendobj".as_slice(),
            PdfErrorKind::UnsupportedFeature,
        ),
        (
            b"1 0 obj\n<< >>\nstream\nabc\nendstream\nendobj".as_slice(),
            PdfErrorKind::Malformed,
        ),
        (
            b"1 0 obj\n<< /Length 18446744073709551615 >>\nstream\nabc\nendstream\nendobj"
                .as_slice(),
            PdfErrorKind::Malformed,
        ),
    ] {
        let mut source = SeekableSource::new(Cursor::new(bytes.to_vec())).unwrap();
        let error = run(scan_fragment_objects(
            &mut source,
            0,
            bytes.len() as u64,
            &Limits::default(),
            &NeverCancel,
        ))
        .err()
        .expect("invalid fragment was accepted");
        assert!(
            matches!(&error, Error::Pdf { kind: found, .. } if *found == kind),
            "{error:?}"
        );
    }
}

#[test]
fn fragment_scanner_requires_a_dictionary_for_stream_payloads() {
    let bytes = b"1 0 obj\nnull\nstream\nabc\nendstream\nendobj";
    let mut source = SeekableSource::new(Cursor::new(bytes.to_vec())).unwrap();
    let error = run(scan_fragment_objects(
        &mut source,
        0,
        bytes.len() as u64,
        &Limits::default(),
        &NeverCancel,
    ))
    .err()
    .expect("a stream without a dictionary was accepted");
    assert!(matches!(
        error,
        Error::Pdf {
            kind: PdfErrorKind::Malformed,
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
    let scan = run(scan_fragment_objects(
        &mut source,
        0,
        length,
        &Limits::default(),
        &NeverCancel,
    ))
    .unwrap();
    assert_eq!(scan.patches.len(), 1);
    let mut bytes = source.into_inner().into_inner();
    bytes[scan.patches[0].offset as usize] = b'7';
    let mut changed = SeekableSource::new(Cursor::new(bytes)).unwrap();
    let mut patched = PatchedSource::new(&mut changed, &scan.patches);
    let mut one = [0u8; 1];
    let result = run(read_exact_at(
        &mut patched,
        scan.patches[0].offset,
        &mut one,
        &Limits::default(),
        &NeverCancel,
    ));
    assert!(matches!(
        result,
        Err(Error::Pdf {
            kind: PdfErrorKind::Malformed,
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

        async fn read_at(&mut self, _offset: u64, destination: &mut [u8]) -> Result<usize> {
            Ok(destination.len() + 1)
        }
    }

    let mut source = Overreporting;
    let mut patched = PatchedSource::new(&mut source, &[]);
    let mut byte = [0u8; 1];
    assert!(matches!(
        run(patched.read_at(0, &mut byte)),
        Err(Error::InvalidInput {
            reason: "source reported more bytes than requested"
        })
    ));
}

#[test]
fn fragment_scanner_caps_the_total_object_index_before_allocating_it() {
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
    let error = run(scan_fragment_objects(
        &mut source,
        0,
        size,
        &limits,
        &NeverCancel,
    ))
    .err()
    .expect("an unbounded object index was accepted");
    assert!(matches!(
        error,
        Error::PdfLimitExceeded {
            resource: "allocation bytes",
            ..
        }
    ));
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
    let scan = run(scan_fragment_objects(
        &mut source,
        body_start,
        body_end,
        &Limits::default(),
        &NeverCancel,
    ))
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
            run(patched.read_at(patch.offset - 1, &mut before_and_first_digit)).unwrap(),
            2
        );
        assert_eq!(before_and_first_digit, expected[at - 1..at + 1]);
        let mut last_digit_and_after = [0; 2];
        assert_eq!(
            run(patched.read_at(patch.offset + 1, &mut last_digit_and_after)).unwrap(),
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
        run(read_exact_at(
            &mut patched,
            offset as u64,
            std::slice::from_mut(byte),
            &one_byte_reads,
            &NeverCancel,
        ))
        .unwrap();
    }
    assert_eq!(observed, expected);
}

#[test]
fn repaired_final_stream_accepts_xml_trailer_but_rejects_unknown_tail() {
    let mut body = b"1 0 obj\n<< /Length 10 >>\nstream\n".to_vec();
    body.extend_from_slice(b"abcdefghijkl\r\nendstream\nendobj");
    let body_end = body.len() as u64;
    let understated_hint = body_end - 3;

    let mut with_xml = body.clone();
    with_xml.extend_from_slice(b"\r\n<?xml version=\"1.0\"?><Doc/>");
    let mut source = SeekableSource::new(Cursor::new(with_xml)).unwrap();
    let scan = run(scan_fragment_objects(
        &mut source,
        0,
        understated_hint,
        &Limits::default(),
        &NeverCancel,
    ))
    .unwrap();
    assert_eq!(scan.objects.len(), 1);
    assert_eq!(scan.objects[0].range.end(), Some(body_end));
    assert_eq!(scan.patches.len(), 1);
    assert_eq!(scan.patches[0].replacement, b"12");

    let mut with_unknown_tail = body;
    with_unknown_tail.extend_from_slice(b"\r\n<unexpected/>");
    let mut source = SeekableSource::new(Cursor::new(with_unknown_tail)).unwrap();
    let error = run(scan_fragment_objects(
        &mut source,
        0,
        understated_hint,
        &Limits::default(),
        &NeverCancel,
    ))
    .err()
    .expect("unknown trailer after repaired stream was accepted");
    assert!(matches!(
        error,
        Error::Pdf {
            kind: PdfErrorKind::AmbiguousRepair,
            reason: "repaired final stream has an unrecognized CAJ trailer",
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
    let index = run(PdfIndex::open(
        &mut source,
        PdfRange {
            offset,
            length: doc.len() as u64,
        },
        &Limits::default(),
        &NeverCancel,
    ))
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
        Err(Error::Pdf {
            kind: PdfErrorKind::Malformed,
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
        expect_pdf_error(base_pdf(page, pages, catalog), PdfErrorKind::Malformed);
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
            PdfErrorKind::Malformed,
        ),
        (
            "<< /Type /Outlines /First 5 0 R /Last 5 0 R >>",
            first,
            second,
            PdfErrorKind::Malformed,
        ),
        (
            root,
            "<< /Title (A) /Parent 4 0 R /Next 5 0 R >>",
            second,
            PdfErrorKind::Malformed,
        ),
        (
            root,
            "<< /Title (A) /Parent 4 0 R /Next 6 0 R /Dest /named >>",
            second,
            PdfErrorKind::UnsupportedFeature,
        ),
        (
            root,
            "<< /Title (A) /Parent 4 0 R /Next 6 0 R /A << /S /URI /URI (x) >> >>",
            second,
            PdfErrorKind::UnsupportedFeature,
        ),
        (
            root,
            first,
            "<< /Title (B) /Parent 4 0 R /Dest [3 0 R /Fit] >>",
            PdfErrorKind::Malformed,
        ),
        (
            root,
            first,
            "<< /Title (B) /Parent 1 0 R /Prev 5 0 R >>",
            PdfErrorKind::Malformed,
        ),
        (
            root,
            first,
            "<< /Title (B) /Parent 4 0 R /Prev 5 0 R /Dest [2 0 R /Fit] >>",
            PdfErrorKind::Malformed,
        ),
    ] {
        expect_pdf_error(make(root, first, second), kind);
    }
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
        (
            "<< /Type /Outlines /Count 2 >>",
            parent,
            child,
            PdfErrorKind::Malformed,
        ),
        (
            "<< /Type /Outlines /First 5 0 R /Last 5 0 R /Count 0 >>",
            parent,
            child,
            PdfErrorKind::Malformed,
        ),
        (
            "<< /Type /Wrong /First 5 0 R /Last 5 0 R >>",
            parent,
            child,
            PdfErrorKind::Malformed,
        ),
        (
            root,
            "<< /Title 7 /Parent 4 0 R /First 6 0 R /Last 6 0 R >>",
            child,
            PdfErrorKind::Malformed,
        ),
        (
            root,
            "<< /Title (Parent) /Parent 4 0 R /First 6 0 R >>",
            child,
            PdfErrorKind::Malformed,
        ),
        (
            root,
            "<< /Title (Parent) /Parent 4 0 R /Last 6 0 R >>",
            child,
            PdfErrorKind::Malformed,
        ),
        (
            root,
            "<< /Title (Parent) /Parent 4 0 R /First 5 0 R /Last 5 0 R >>",
            child,
            PdfErrorKind::Malformed,
        ),
        (
            root,
            parent,
            "<< /Title (Child) /Parent 4 0 R >>",
            PdfErrorKind::Malformed,
        ),
        (
            root,
            parent,
            "<< /Title (Child) /Parent 5 0 R /Prev /Bad >>",
            PdfErrorKind::Malformed,
        ),
        (
            root,
            parent,
            "<< /Title (Child) /Parent 5 0 R /Dest [3 0 R null] >>",
            PdfErrorKind::UnsupportedFeature,
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
        Err(Error::PdfLimitExceeded {
            object: Some((6, 0)),
            resource: "bookmarks",
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
            "/Encrypt 1 0 R" => PdfErrorKind::Encrypted,
            "/XRefStm 50" => PdfErrorKind::UnsupportedFeature,
            _ => PdfErrorKind::Malformed,
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
    expect_pdf_error(bad_entry, PdfErrorKind::Malformed);
    let mut no_eof = valid;
    no_eof.truncate(no_eof.len() - 6);
    expect_pdf_error(no_eof, PdfErrorKind::Malformed);

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
        Err(Error::PdfLimitExceeded {
            resource: "PDF object index",
            attempted: 8_388_609,
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
            PdfErrorKind::UnsupportedFeature,
        ),
        (form_catalog, "null", PdfErrorKind::Malformed),
        (
            form_catalog,
            "<< /SigFlags /Bad >>",
            PdfErrorKind::Malformed,
        ),
        (
            form_catalog,
            "<< /SigFlags 1 >>",
            PdfErrorKind::UnsupportedFeature,
        ),
        (
            "<< /Type /Catalog /Pages 2 0 R /Outlines /Bad >>",
            "<< >>",
            PdfErrorKind::Malformed,
        ),
        (
            "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>",
            "null",
            PdfErrorKind::Malformed,
        ),
    ] {
        expect_pdf_error(make(catalog, object4, ""), kind);
    }
    let plain_catalog = "<< /Type /Catalog /Pages 2 0 R >>";
    expect_pdf_error(
        make(plain_catalog, "3", "/Info 4 0 R"),
        PdfErrorKind::Malformed,
    );
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
        expect_pdf_error(make(root, page, object4), PdfErrorKind::Malformed);
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
    expect_pdf_error(make("null"), PdfErrorKind::Malformed);
    expect_pdf_error(make("[0 0 0 200]"), PdfErrorKind::Malformed);
}

#[test]
fn fragment_catalog_with_unvalidated_outline_tree_is_unsupported() {
    let bytes = b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>\nendobj".to_vec();
    let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
    let error = run(inspect_fragment_object(
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
        &NeverCancel,
        |_| None,
    ))
    .err()
    .unwrap();
    assert!(matches!(
        error,
        Error::Pdf {
            kind: PdfErrorKind::UnsupportedFeature,
            object: Some((1, 0)),
            ..
        }
    ));
}

fn inspect_raw_fragment(raw: &[u8]) -> Result<FragmentInspection> {
    let mut source = SeekableSource::new(Cursor::new(raw.to_vec()))?;
    run(inspect_fragment_object(
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
        &NeverCancel,
        |_| None,
    ))
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
            PdfErrorKind::Malformed,
        ),
        (
            b"1 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 0 10] >> endobj",
            PdfErrorKind::Malformed,
        ),
        (
            b"1 0 obj << /Type /Page /Parent 2 0 R /MediaBox 4 0 R >> endobj",
            PdfErrorKind::UnsupportedFeature,
        ),
        (
            b"1 0 obj << /Type /Page /Parent 2 0 R /Contents /Bad >> endobj",
            PdfErrorKind::Malformed,
        ),
        (
            b"1 0 obj << /Type /Pages /Parent /Bad /Count 1 /Kids [2 0 R] >> endobj",
            PdfErrorKind::Malformed,
        ),
        (
            b"1 0 obj << /Type /Pages /Count /Bad /Kids [2 0 R] >> endobj",
            PdfErrorKind::Malformed,
        ),
        (
            b"1 0 obj << /Type /Pages /Count 1 /Kids [2 0 R 0] >> endobj",
            PdfErrorKind::Malformed,
        ),
        (
            b"1 0 obj << /Type /Catalog >> endobj",
            PdfErrorKind::Malformed,
        ),
        (
            b"1 0 obj << /Title (A) /A << /S /URI /URI (x) >> >> endobj",
            PdfErrorKind::UnsupportedFeature,
        ),
        (
            b"1 0 obj << /Title (A) /Dest /named >> endobj",
            PdfErrorKind::UnsupportedFeature,
        ),
        (
            b"1 0 obj << /Type /Page /Parent 2 0 R >> endobj trailing",
            PdfErrorKind::Malformed,
        ),
        (
            b"1 0 obj 3 stream\nabc\nendstream\nendobj",
            PdfErrorKind::Malformed,
        ),
        (
            b"1 0 obj << >>\nstream\nabc\nendstream\nendobj",
            PdfErrorKind::Malformed,
        ),
        (
            b"1 0 obj << /Length /Bad >>\nstream\nabc\nendstream\nendobj",
            PdfErrorKind::Malformed,
        ),
        (
            b"1 0 obj << /Length 4 0 R >>\nstream\nabc\nendstream\nendobj",
            PdfErrorKind::Malformed,
        ),
    ] {
        let error = inspect_raw_fragment(raw)
            .err()
            .unwrap_or_else(|| panic!("accepted malformed fragment: {raw:?}"));
        assert!(
            matches!(error, Error::Pdf { kind: found, .. } if found == kind),
            "{error:?}"
        );
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
        Error::Pdf {
            kind: PdfErrorKind::UnsupportedFeature,
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
    let too_long = run(PdfIndex::open(
        &mut source,
        PdfRange {
            offset: 0,
            length: doc.len() as u64 + 1,
        },
        &Limits::default(),
        &NeverCancel,
    ))
    .err()
    .unwrap();
    assert!(matches!(too_long, Error::TruncatedInput { .. }));

    let mut limits = Limits {
        max_input_bytes: doc.len() as u64 - 1,
        ..Limits::default()
    };
    let error = open_with(doc.clone(), &limits).err().unwrap();
    assert!(matches!(
        error,
        Error::PdfLimitExceeded {
            offset: 0,
            object: None,
            resource: "input bytes",
            ..
        }
    ));

    let mut index = open(doc).unwrap();
    index.max_referenced_object = MAX_PDF_OBJECTS;
    assert!(matches!(
        index.next_free_object_number(),
        Err(Error::PdfLimitExceeded {
            resource: "PDF object number",
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
        Error::PdfLimitExceeded {
            object: Some((4, 0)),
            resource: "PDF syntax depth",
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
            Error::PdfLimitExceeded {
                object: Some((4, 0)),
                resource: "PDF object syntax bytes",
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
        expect_pdf_error(with_stream(stream), PdfErrorKind::Malformed);
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
        PdfErrorKind::Malformed,
    );
    expect_pdf_error(
        base_pdf(page, "<< /Type /Pages /Kids /Bad /Count 1 >>", catalog),
        PdfErrorKind::Malformed,
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
        PdfErrorKind::Malformed,
    );
    expect_pdf_error(make(outline_root, "null"), PdfErrorKind::Malformed);
    expect_pdf_error(
        make(outline_root, "<< /Title (A) /Parent 4 0 R /Next /Bad >>"),
        PdfErrorKind::Malformed,
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
        let result = run(inspect_fragment_scalar(
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
            &NeverCancel,
        ));
        if expected_none {
            assert!(matches!(result, Ok(None)));
        } else {
            assert!(matches!(
                result,
                Err(Error::Pdf {
                    kind: PdfErrorKind::Malformed,
                    ..
                })
            ));
        }
    }
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
fn eof_marker_glued_to_other_bytes_is_not_the_document_end() {
    let mut doc = build_pdf(&minimal_objects(), "");
    let real_end = doc.len() as u64;
    doc.extend_from_slice(b"junk%%EOF\n");
    let error = pdf_error(open(doc));
    assert!(
        matches!(
            error,
            Error::Pdf {
                offset,
                kind: PdfErrorKind::AmbiguousRepair,
                reason: "bytes after PDF EOF are not a recognized CAJ footer",
                ..
            } if offset == real_end
        ),
        "{error:?}"
    );
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
                Error::Pdf {
                    offset,
                    object: None,
                    kind: PdfErrorKind::Malformed,
                    reason: found,
                } if offset == xref_at + 5 && found == reason
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
            Error::Pdf {
                offset,
                kind: PdfErrorKind::Malformed,
                reason: "PDF dictionary is truncated",
                ..
            } if offset == length
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
            Error::Pdf {
                offset,
                object: None,
                kind: PdfErrorKind::Malformed,
                ..
            } if offset > dictionary_at
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
            matches!(error, Error::Pdf { object: None, reason: found, .. } if found == reason),
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
            Error::Pdf {
                offset: 9,
                object: Some((1, 0)),
                kind: PdfErrorKind::Malformed,
                reason: "xref stream lacks a dictionary",
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
            Err(Error::Cancelled) => cancelled += 1,
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
            Error::Pdf {
                offset,
                object: Some((4, 0)),
                kind: PdfErrorKind::Malformed,
                reason: "live PDF object begins after logical EOF",
            } if offset == footer_at
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
            Error::PdfLimitExceeded {
                object: Some((6, 0)),
                resource: "pages",
                limit: 2,
                attempted: 3,
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
            Error::PdfLimitExceeded {
                object: None,
                resource: "PDF orphan gap repair bytes",
                limit: 8192,
                attempted: 8256,
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
            Error::PdfLimitExceeded {
                object: Some((3, 0)),
                resource: "PDF repair object bytes",
                limit,
                attempted,
                ..
            } if limit == orphan_bytes && attempted == orphan_bytes + page_bytes
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
            Error::PdfLimitExceeded {
                object: Some((number, 0)),
                resource: "PDF repair object bytes",
                attempted,
                ..
            } if number == 9 + orphans && attempted == orphan_bytes
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
            Error::Pdf {
                offset: 0,
                object: Some((1, 0)),
                kind: PdfErrorKind::Malformed,
                reason: "stream extent overflows",
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
        inflate_xref(&encoded, decoded.len(), 7, &NeverCancel).ok(),
        Some(decoded.clone())
    );
    assert!(matches!(
        inflate_xref(&encoded, decoded.len(), 7, &CancelAfter::always()),
        Err(InflateXrefError::Cancelled)
    ));
    assert!(matches!(
        inflate_xref(&encoded, decoded.len() - 1, 7, &NeverCancel),
        Err(InflateXrefError::TooLong)
    ));
    assert!(matches!(
        inflate_xref(&encoded, decoded.len() + 1, 7, &NeverCancel),
        Err(InflateXrefError::Malformed)
    ));
    let mut trailing = encoded.clone();
    trailing.push(0);
    assert!(matches!(
        inflate_xref(&trailing, decoded.len(), 7, &NeverCancel),
        Err(InflateXrefError::Malformed)
    ));
    assert!(matches!(
        inflate_xref(
            &encoded[..encoded.len() - 6],
            decoded.len(),
            7,
            &NeverCancel
        ),
        Err(InflateXrefError::Malformed)
    ));
    assert!(matches!(
        inflate_xref(b"not zlib", decoded.len(), 7, &NeverCancel),
        Err(InflateXrefError::Malformed)
    ));
}

#[test]
fn bounded_index_growth_reports_the_attempted_capacity() {
    let mut items: Vec<u64> = Vec::new();
    assert!(matches!(
        push_bounded(&mut items, 1, 16, "test index"),
        Err(Error::LimitExceeded {
            resource: "test index",
            limit: 16,
            attempted: 32,
        })
    ));
    assert!(items.is_empty());
    for value in 0..8 {
        push_bounded(&mut items, value, 64, "test index").unwrap();
    }
    assert_eq!(items, [0, 1, 2, 3, 4, 5, 6, 7]);
    assert!(matches!(
        push_bounded(&mut items, 8, 64, "test index"),
        Err(Error::LimitExceeded { attempted: 128, .. })
    ));
    assert_eq!(items.len(), 8);
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
        Error::PdfLimitExceeded {
            offset,
            object,
            resource,
            limit,
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
    objects.push((67, "null"));
    let stream = xref_stream_pdf(&objects, [1, 2, 1], 0, false);
    let (_, object, resource, _) = index_limit(stream, 2944);
    assert_eq!((object, resource), (Some((68, 0)), "PDF xref records"));

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
        (
            b"%PDF-1".to_vec(),
            PdfErrorKind::Malformed,
            "PDF header is truncated",
        ),
        (
            version,
            PdfErrorKind::Malformed,
            "PDF 1.0 through 1.7 header is required",
        ),
        (
            second,
            PdfErrorKind::UnsupportedFeature,
            "PDF 2.x is outside the supported input profile",
        ),
        (token, PdfErrorKind::Malformed, "PDF token is too long"),
        (
            build_pdf(&orphan, ""),
            PdfErrorKind::Malformed,
            "stale page Parent is not reachable through validated Kids",
        ),
        (
            duplicate,
            PdfErrorKind::AmbiguousRepair,
            "duplicate PDF dictionary keys have undefined value",
        ),
        (escape, PdfErrorKind::Malformed, "invalid PDF name escape"),
        (
            with_bytes_before_xref(minimal(), b"4 0 obj\n0\nendobj\n"),
            PdfErrorKind::Malformed,
            "unindexed bytes between PDF objects",
        ),
        (
            base_pdf(&page("/Parent 2 0 R /Foo 9 0 R"), PAGES, CATALOG),
            PdfErrorKind::Malformed,
            "PDF object contains a dangling indirect reference",
        ),
        (
            base_pdf(&page(""), PAGES, CATALOG),
            PdfErrorKind::Malformed,
            "page tree Parent link disagrees with Kids",
        ),
        (
            base_pdf(&page("/Parent 1 0 R"), PAGES, CATALOG),
            PdfErrorKind::Malformed,
            "page tree Parent link disagrees with Kids",
        ),
        (
            base_pdf(
                &page("/Parent 2 0 R"),
                "<< /Type /Pages /Parent 3 0 R /Kids [3 0 R] /Count 1 >>",
                CATALOG,
            ),
            PdfErrorKind::Malformed,
            "page tree Parent link disagrees with Kids",
        ),
        (
            base_pdf(
                &page("/Parent 2 0 R"),
                PAGES,
                "<< /Type /Catalog /Pages 2 0 R /Perms << >> >>",
            ),
            PdfErrorKind::UnsupportedFeature,
            "signed PDF edits are unsupported",
        ),
    ];
    for (bytes, kind, reason) in cases {
        let error = pdf_error(open(bytes));
        assert!(
            matches!(
                error,
                Error::Pdf { kind: found, reason: actual, .. } if found == kind && actual == reason
            ),
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
            Error::PdfLimitExceeded {
                object: Some((2, 0)),
                resource: "pages",
                limit: 0,
                attempted: 1,
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
    let error = pdf_error(run(PdfIndex::open(
        &mut source,
        range,
        &Limits::default(),
        &NeverCancel,
    )));
    assert!(
        matches!(
            error,
            Error::TruncatedInput {
                offset: 1,
                expected,
                available,
            } if expected == length && available == length - 1
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
            Error::PdfLimitExceeded {
                offset,
                object: None,
                resource: "PDF dictionary syntax bytes",
                limit: 300,
                attempted: 301,
            } if offset == trailer
        ),
        "{error:?}"
    );
}
