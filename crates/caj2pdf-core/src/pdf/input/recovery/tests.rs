// SPDX-License-Identifier: MIT
use super::*;
use crate::native::SeekableSource;
use crate::pdf::input::scan_damaged_fragment;
use crate::test_support::{CancelAfter, NEVER};
use std::io::Cursor;

fn exercise(
    body: &[u8],
    limits: &Limits,
    cancellation: &impl Cancellation,
) -> Result<(Vec<u8>, Vec<OmittedPage>)> {
    let metadata = crate::caj::CajMetadata {
        page_count: 1,
        body_start: 0,
        body_end_hint: body.len() as u64,
        page_rows: vec![crate::caj::CajPageRow {
            offset: 0,
            length: body.len() as u64,
            page_object_id: 1,
        }],
        bookmarks: Vec::new(),
    };
    let mut source = SeekableSource::new(Cursor::new(body)).unwrap();
    let mut scan = scan_damaged_fragment(
        &mut source,
        &metadata.page_rows,
        body.len() as u64,
        &Limits::default(),
        &NEVER,
        &mut [],
    )?;
    scan.damaged.push((
        Some(PdfRef {
            number: 999,
            generation: 0,
        }),
        0,
    ));
    // Deliberately malformed object 7 is structurally indexed before the
    // page-content check. Resource 6 and the page depend on it transitively.
    substitute_damaged_pages(&mut source, &metadata, &mut scan, limits, cancellation)
}

const BODY: &[u8] = b"1 0 obj<</Type/Page /Parent 9 0 R /MediaBox[0 0 20 30] /Resources 6 0 R>>endobj\n6 0 obj<</Font<</F1 7 0 R>>>>endobj\n7 0 obj<</Type/Page /Parent 9 0 R /Contents 42>>endobj\n100 0 obj 1 endobj\n101 0 obj<</Length 100 0 R>>stream\nx\nendstream\nendobj\n";

#[test]
fn damaged_dependencies_are_transitive_and_honor_cancellation() {
    let (suffix, report) = exercise(BODY, &Limits::default(), &NEVER).unwrap();
    assert_eq!(report.len(), 1);
    assert!(
        String::from_utf8(suffix)
            .unwrap()
            .contains("/Resources << >>")
    );
    let checkpoints = CancelAfter::never();
    let short = Limits {
        io_chunk_bytes: 1,
        ..Limits::default()
    };
    exercise(BODY, &short, &checkpoints).unwrap();
    for allowed in 0..=checkpoints.queries() {
        let result = exercise(
            BODY,
            &Limits {
                io_chunk_bytes: 1,
                ..Limits::default()
            },
            &CancelAfter::new(allowed),
        );
        assert!(matches!(result, Ok(_) | Err(Error::Cancelled)));
    }
    let unsupported = String::from_utf8(BODY.to_vec())
        .unwrap()
        .replace("/MediaBox[0 0 20 30]", "/MediaBox 20 0 R");
    assert!(matches!(
        exercise(unsupported.as_bytes(), &Limits::default(), &NEVER),
        Err(Error::Pdf {
            kind: PdfErrorKind::UnsupportedFeature,
            ..
        })
    ));
    let unknown = b"7 0 obj null endobj";
    assert!(matches!(
        exercise(unknown, &Limits::default(), &NEVER),
        Err(Error::Caj {
            reason: "damaged page has no validated geometry",
            ..
        })
    ));
}
