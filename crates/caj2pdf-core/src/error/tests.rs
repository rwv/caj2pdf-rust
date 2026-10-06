// SPDX-License-Identifier: MIT

use super::{Context, Error, ErrorKind, Hnc8Stage};
use crate::hnc8::Variant;
use std::io;

#[test]
fn display_names_the_kind_context_offset_location_and_reason() {
    let cases = [
        (
            Error::malformed(7, "bad table").in_caj(Some(3)),
            "malformed CAJ at byte 7, record 3: bad table",
        ),
        (
            Error::malformed(9, "bad xref").in_pdf(Some((4, 0))),
            "malformed PDF at byte 9, object 4 0: bad xref",
        ),
        (
            Error::malformed(9, "two candidates").within(Context::Pdf {
                object: None,
                repair: true,
            }),
            "ambiguous repair PDF at byte 9: two candidates",
        ),
        (
            Error::truncated(0, 4, 2)
                .because("signature")
                .within(Context::Hnc8 {
                    variant: Some(Variant::HnA),
                    page: Some(2),
                    image: Some(1),
                    segment: None,
                    stage: Some(Hnc8Stage::Decode),
                }),
            "truncated HN/C8 HN-A at byte 0, page 2, image 1: signature: \
             expected 4 bytes, available 2",
        ),
        (
            Error::limit("pages", 1, 2).at(5).in_caj(None),
            "CAJ pages limit exceeded at byte 5: maximum 1, attempted 2",
        ),
        (
            Error::limit("pages", 1, 2),
            "pages limit exceeded: maximum 1, attempted 2",
        ),
        (Error::invalid("empty range"), "invalid input: empty range"),
        (
            Error::truncated(0, 254, 3),
            "truncated input at byte 0: expected 254 bytes, available 3",
        ),
        (
            ErrorKind::UnsupportedFormat.into(),
            "unsupported input format",
        ),
        (ErrorKind::Cancelled.into(), "operation cancelled"),
        (
            Error::from(ErrorKind::Cancelled).in_jbig2(Some(4)),
            "operation cancelled in JBIG2, segment 4",
        ),
    ];
    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
    }
}

#[test]
fn only_unlocated_errors_take_a_location() {
    let error = Error::limit("resource", 1, 2).or_at(7, Context::Caj { record: Some(3) });
    assert_eq!(error.offset, Some(7));
    assert_eq!(error.context, Context::Caj { record: Some(3) });
    let located = Error::malformed(9, "x")
        .in_pdf(None)
        .or_at(7, Context::Caj { record: None });
    assert_eq!(located.offset, Some(9));
    assert_eq!(
        located.context,
        Context::Pdf {
            object: None,
            repair: false
        }
    );
}

#[test]
fn io_errors_round_trip_through_io_write_adapters() {
    let carried = io::Error::from(Error::from(ErrorKind::Cancelled));
    assert!(matches!(Error::from(carried).kind, ErrorKind::Cancelled));
    let error = Error::from(io::Error::new(io::ErrorKind::BrokenPipe, "x"));
    assert!(std::error::Error::source(&error).is_some());
    assert_eq!(io::Error::from(error).kind(), io::ErrorKind::BrokenPipe);
    let located = Error::from(io::Error::new(io::ErrorKind::BrokenPipe, "x")).at(3);
    let carried = io::Error::from(located);
    assert_eq!(carried.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(Error::from(carried).offset, Some(3));
}
