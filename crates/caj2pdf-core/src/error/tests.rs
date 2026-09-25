// SPDX-License-Identifier: MIT

use super::Error;

fn unlocated() -> Error {
    Error::LimitExceeded {
        resource: "resource",
        limit: 1,
        attempted: 2,
    }
}

#[test]
fn limit_errors_receive_a_caj_location() {
    assert!(matches!(
        unlocated().locate_caj_limit(7, Some(3)),
        Error::CajLimitExceeded {
            offset: 7,
            record: Some(3),
            resource: "resource",
            limit: 1,
            attempted: 2,
        }
    ));
}

#[test]
fn limit_errors_receive_a_pdf_location() {
    assert!(matches!(
        unlocated().locate_pdf_limit(9, Some((4, 0))),
        Error::PdfLimitExceeded {
            offset: 9,
            object: Some((4, 0)),
            resource: "resource",
            limit: 1,
            attempted: 2,
        }
    ));
}

#[test]
fn other_errors_are_not_relocated() {
    assert!(matches!(
        Error::Cancelled.locate_caj_limit(7, None),
        Error::Cancelled
    ));
    assert!(matches!(
        Error::Cancelled.locate_pdf_limit(9, None),
        Error::Cancelled
    ));
}
