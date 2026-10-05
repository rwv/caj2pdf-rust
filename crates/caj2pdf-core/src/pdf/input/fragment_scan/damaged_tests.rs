// SPDX-License-Identifier: MIT
use super::*;
use crate::native::SeekableSource;
use crate::test_support::{NEVER, run};
use std::io::Cursor;

fn anchor(bytes: &[u8], limits: &Limits) -> Result<u64> {
    let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
    let mut reader = Reader::new(
        &mut source,
        PdfRange {
            offset: 0,
            length: bytes.len() as u64,
        },
        limits,
        &NEVER,
    )?;
    run(damaged_page_anchor(
        &mut reader,
        &crate::caj::CajPageRow {
            offset: 0,
            length: bytes.len() as u64,
            page_object_id: 1,
        },
    ))
}

#[test]
fn partial_boundaries_require_one_complete_table_named_page() {
    let limits = Limits::default();
    let page = "1 0 obj<</Type/Page>>endobj\n";
    assert_eq!(
        anchor(format!("tail\n{page}").as_bytes(), &limits).unwrap(),
        5
    );
    for bytes in [
        "garbage".to_owned(),
        "11 0 obj<</Type/Page>>endobj".to_owned(),
        format!("{}{}", page, page),
        "1 0 obj<</Type/Other>>endobj".to_owned(),
        "1 0 obj<</Type @>>endobj".to_owned(),
        format!("{}{}", "x".repeat(65), page),
    ] {
        assert!(anchor(bytes.as_bytes(), &limits).is_err());
    }
    // Parsing the page itself must not hide a depth limit behind partial mode.
    let nested = format!(
        "1 0 obj<</Type/Page /Deep {}0{}>>endobj",
        "[".repeat(300),
        "]".repeat(300)
    );
    assert!(matches!(
        anchor(nested.as_bytes(), &limits),
        Err(Error::PdfLimitExceeded { .. })
    ));
}

#[test]
fn discarded_stream_boundaries_use_only_parsed_lengths_and_existing_tail_rules() {
    let limits = Limits {
        io_chunk_bytes: 1,
        ..Limits::default()
    };
    for bytes in [
        "1 0 obj null endobj",
        "1 0 obj null stream\nX\nendstream\nendobj",
        "1 0 obj << >> stream\nX\nendstream\nendobj",
        "1 0 obj << /Length 2 0 R >> stream\nX\nendstream\nendobj",
        "1 0 obj << /Length 18446744073709551615 >> stream\nX\nendstream\nendobj",
        "1 0 obj << /Length 1 >> stream\nXX\nendstream\nendobj",
    ] {
        let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
        let mut reader = Reader::new(
            &mut source,
            PdfRange {
                offset: 0,
                length: bytes.len() as u64,
            },
            &limits,
            &NEVER,
        )
        .unwrap();
        let result = run(damaged_stream_end(&mut reader, 0, &[])).unwrap();
        assert_eq!(result.is_some(), bytes.contains("\nXX\n"));
    }
    let header = b"1 0 obj << /Length 10000 >> stream\n";
    let bytes = [
        header.as_slice(),
        &vec![b'X'; 10000],
        b"\nendstream\nendobj",
    ]
    .concat();
    let boundary = header.len() + 10000;
    let mut source = super::tests::UnreadableTail {
        bytes: bytes.to_vec(),
        unreadable_from: boundary as u64,
    };
    let mut reader = Reader::new(
        &mut source,
        PdfRange {
            offset: 0,
            length: bytes.len() as u64,
        },
        &limits,
        &NEVER,
    )
    .unwrap();
    assert!(matches!(
        run(damaged_stream_end(&mut reader, 0, &[])),
        Err(Error::Io(_))
    ));
}

#[test]
fn partial_scan_resumes_at_pages_and_tracks_unresolved_prefixes_and_lengths() {
    let first = b"1 0 obj<</Type/Page /Parent 9 0 R /MediaBox[0 0 20 30]>>endobj\nBAD\n";
    let last = b"tail\n2 0 obj<</Type/Page /Parent 9 0 R /MediaBox[0 0 30 40]>>endobj\n";
    let bytes = [first.as_slice(), last.as_slice()].concat();
    let rows = [
        crate::caj::CajPageRow {
            offset: 0,
            length: first.len() as u64,
            page_object_id: 1,
        },
        crate::caj::CajPageRow {
            offset: first.len() as u64,
            length: last.len() as u64,
            page_object_id: 2,
        },
    ];
    let mut source = SeekableSource::new(Cursor::new(&bytes)).unwrap();
    let scan = run(scan_damaged_fragment(
        &mut source,
        &rows,
        bytes.len() as u64,
        &Limits::default(),
        &NEVER,
        &mut [FragmentCandidate {
            object: FragmentObject {
                reference: PdfRef {
                    number: 99,
                    generation: 0,
                },
                range: PdfRange {
                    offset: 0,
                    length: 1,
                },
            },
            used: true,
        }],
        &mut 0,
    ))
    .unwrap();
    assert_eq!(scan.objects.len(), 2);
    assert_eq!(scan.damaged.len(), 1);
    let prefix = b"7 0 obj << /Box [1 3\n8 0 obj 42 endobj\n";
    let mut source = SeekableSource::new(Cursor::new(prefix)).unwrap();
    let rows = [crate::caj::CajPageRow {
        offset: 0,
        length: prefix.len() as u64,
        page_object_id: 8,
    }];
    let scan = run(scan_damaged_fragment(
        &mut source,
        &rows,
        prefix.len() as u64,
        &Limits::default(),
        &NEVER,
        &mut [],
        &mut 0,
    ))
    .unwrap();
    assert_eq!(scan.damaged[0].0.unwrap().number, 7);
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut encoder, b"hello").unwrap();
    let payload = encoder.finish().unwrap();
    let bytes = [
        b"1 0 obj<</Length 2 0 R /Filter/FlateDecode>>stream\n".as_slice(),
        payload.as_slice(),
        b"\nendstream\nendobj\n2 0 obj 999 endobj",
    ]
    .concat();
    let rows = [crate::caj::CajPageRow {
        offset: 0,
        length: bytes.len() as u64,
        page_object_id: 1,
    }];
    let mut source = SeekableSource::new(Cursor::new(&bytes)).unwrap();
    let scan = run(scan_damaged_fragment(
        &mut source,
        &rows,
        bytes.len() as u64,
        &Limits::default(),
        &NEVER,
        &mut [],
        &mut 0,
    ))
    .unwrap();
    assert_eq!(scan.damaged[0].0.unwrap().number, 2);
    assert_eq!(scan.objects.len(), 1);
}

#[test]
fn discarded_final_object_does_not_reuse_prior_stream_tail_state() {
    let body = b"1 0 obj<</Length 1>>stream\nXX\nendstream\nendobj\n2 0 obj<</Bad @>>endobj\n";
    let bytes = [body.as_slice(), b"unrecognized external trailer"].concat();
    let rows = [crate::caj::CajPageRow {
        offset: 0,
        length: body.len() as u64,
        page_object_id: 1,
    }];
    let mut source = SeekableSource::new(Cursor::new(&bytes)).unwrap();
    let scan = run(scan_damaged_fragment(
        &mut source,
        &rows,
        body.len() as u64,
        &Limits::default(),
        &NEVER,
        &mut [],
        &mut 0,
    ))
    .unwrap();
    assert_eq!(scan.objects.len(), 1);
    assert_eq!(scan.damaged.len(), 1);
}
