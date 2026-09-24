// SPDX-License-Identifier: MIT

use super::*;
use crate::NeverCancel;
use crate::native::SeekableSource;
use std::future::Future;
use std::io::Cursor;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

fn run<F: Future>(future: F) -> F::Output {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = pin!(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("in-memory native source yielded unexpectedly"),
    }
}

fn build_pdf(objects: &[(u32, &str)], trailer_extra: &str) -> Vec<u8> {
    let highest = objects.iter().map(|(number, _)| *number).max().unwrap_or(0);
    let mut bytes = b"%PDF-1.7\n%\x80\x81\x82\x83\n".to_vec();
    let mut offsets = vec![None; highest as usize + 1];
    for &(number, body) in objects {
        offsets[number as usize] = Some(bytes.len());
        bytes.extend_from_slice(format!("{number} 0 obj\n{body}\nendobj\n").as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", highest + 1).as_bytes());
    for offset in offsets.into_iter().skip(1) {
        if let Some(offset) = offset {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        } else {
            bytes.extend_from_slice(b"0000000000 00000 f \n");
        }
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R {trailer_extra} >>\nstartxref\n{xref}\n%%EOF\n",
            highest + 1
        )
        .as_bytes(),
    );
    bytes
}

fn open_with(bytes: Vec<u8>, limits: &Limits) -> Result<PdfIndex> {
    let len = bytes.len() as u64;
    let mut source = SeekableSource::new(Cursor::new(bytes))?;
    run(PdfIndex::open(
        &mut source,
        PdfRange {
            offset: 0,
            length: len,
        },
        limits,
        &NeverCancel,
    ))
}

fn open(bytes: Vec<u8>) -> Result<PdfIndex> {
    open_with(bytes, &Limits::default())
}

fn base_pdf(page: &str, pages: &str, catalog: &str) -> Vec<u8> {
    build_pdf(&[(1, catalog), (2, pages), (3, page)], "")
}

fn expect_pdf_error(bytes: Vec<u8>, kind: PdfErrorKind) {
    let error = match open(bytes) {
        Err(error) => error,
        Ok(_) => panic!("malformed PDF was accepted"),
    };
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
