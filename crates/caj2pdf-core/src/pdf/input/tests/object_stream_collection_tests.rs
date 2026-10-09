// SPDX-License-Identifier: MIT

use super::*;

// Wholly authored independent object streams: each of the first three streams
// owns one page-tree member. Remaining streams have no live members, but their
// collection links must still be checked. No external document bytes are used.
fn collection(extras: &[&str], edit_rows: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    assert!(extras.len() >= 3);
    let members = minimal_objects();
    let xref_number = 4 + extras.len() as u32;
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, extra) in extras.iter().enumerate() {
        let (n, first, decoded) = if let Some((number, value)) = members.get(i) {
            let header = format!("{number} 0 ");
            (1, header.len(), format!("{header}{value}"))
        } else {
            (0, 0, String::new())
        };
        let encoded = zlib(decoded.as_bytes());
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n<</Type/ObjStm/N {n}/First {first}/Filter/FlateDecode/Length {} {extra}>>stream\n", 4 + i, encoded.len()).as_bytes());
        pdf.extend_from_slice(&encoded);
        pdf.extend_from_slice(b"\nendstream\nendobj\n");
    }
    let xref_at = pdf.len();
    let mut rows = Vec::new();
    for number in 0..=xref_number {
        let (kind, at, generation) = match number {
            0 => (0, 0, u16::MAX),
            1..=3 => (2, number + 3, 0),
            n if n == xref_number => (1, xref_at as u32, 0),
            n => (1, offsets[n as usize - 4] as u32, 0),
        };
        rows.push(kind);
        rows.extend_from_slice(&at.to_be_bytes());
        rows.extend_from_slice(&generation.to_be_bytes());
    }
    edit_rows(&mut rows);
    let encoded = zlib(&rows);
    pdf.extend_from_slice(format!("{xref_number} 0 obj\n<</Type/XRef/Size {}/Root 1 0 R/W[1 4 2]/Filter/FlateDecode/Length {}>>stream\n", xref_number + 1, encoded.len()).as_bytes());
    pdf.extend_from_slice(&encoded);
    pdf.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
    pdf
}

#[test]
fn independent_members_and_shared_collection_ancestors_are_preserved() {
    // Forward chain 4 -> 6 -> 7, backward/shared link 5 -> 4; 7 has no members.
    let doc = collection(
        &["/Ext#65nds 6 0 R", "/Extends 4 0 R", "/Extends 7 0 R", ""],
        |_| {},
    );
    let expected = open(collection(&["", "", "", ""], |_| {})).unwrap();
    let actual = open_with(
        doc.clone(),
        &Limits {
            io_chunk_bytes: 1,
            ..Limits::default()
        },
    )
    .unwrap();
    assert_eq!(actual.pages(), expected.pages());
    assert_eq!(actual.catalog(), expected.catalog());
    for number in 1..=3 {
        let reference = PdfRef {
            number,
            generation: 0,
        };
        assert!(actual.compressed_object(reference).is_some());
        assert!(actual.metadata_location(reference).unwrap().length > 0);
    }
    // All cancellation checkpoints, including the collection traversal, remain
    // observable; allowing one more checkpoint eventually completes the parse.
    for allowed in 0..10000 {
        match open_cancellable(doc.clone(), &Limits::default(), &CancelAfter::new(allowed)) {
            Ok(_) => return,
            Err(Error {
                kind: ErrorKind::Cancelled,
                ..
            }) => {}
            Err(error) => panic!("checkpoint {allowed}: {error}"),
        }
    }
    panic!("collection traversal did not terminate");
}

#[test]
fn malformed_collection_references_types_and_cycles_are_rejected() {
    for value in [
        "4 0 R", "1 0 R", "8 0 R", "99 0 R", "0 0 R", "5 1 R", "null", "5", "[]", "<<>>", "(5 0 R)",
    ] {
        let extra = format!("/Extends {value}");
        let error = pdf_error(open(collection(&[&extra, "", "", ""], |_| {})));
        assert!(
            matches!(error.kind, ErrorKind::Malformed),
            "{value}: {error}"
        );
    }
    for extras in [
        ["/Extends 5 0 R", "/Extends 4 0 R", "", ""],
        ["/Extends 7 0 R", "", "", "/Extends 4 0 R"],
        ["", "", "", "/Extends 7 0 R"],
    ] {
        let error = pdf_error(open(collection(&extras, |_| {})));
        assert_eq!(error.reason, "object stream Extends contains a cycle");
    }
    let free = collection(&["/Extends 7 0 R", "", "", ""], |rows| rows[7 * 7] = 0);
    assert!(open(free).is_err());
    let stale = collection(&["/Extends 7 0 R", "", "", ""], |rows| rows[7 * 7 + 6] = 1);
    assert!(open(stale).is_err());
}

#[test]
fn collection_index_is_bounded() {
    let extras = vec![""; 100];
    let error = pdf_error(open_with(
        collection(&extras, |_| {}),
        &Limits {
            io_chunk_bytes: 64,
            max_allocation_bytes: 16384,
            ..Limits::default()
        },
    ));
    assert!(
        matches!(
            error.kind,
            ErrorKind::LimitExceeded {
                resource: "PDF object stream collections",
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn object_stream_type_requires_a_standalone_generation_zero_stream() {
    let mut objects = minimal_objects().to_vec();
    objects.push((4, "<< /Type /ObjStm /N 0 /First 0 >>"));
    let error = pdf_error(open(build_pdf(&objects, "")));
    assert_eq!(error.reason, "ObjStm must be a generation-zero stream");

    let mut doc = collection(&["", "", "", ""], |rows| rows[7 * 7 + 6] = 1);
    replace_once(&mut doc, b"7 0 obj", b"7 1 obj");
    let error = pdf_error(open(doc));
    assert_eq!(error.reason, "ObjStm must be a generation-zero stream");
}
