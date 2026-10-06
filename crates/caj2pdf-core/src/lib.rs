// SPDX-License-Identifier: MIT

//! Platform-neutral, bounded I/O contracts for CAJ-family conversion.
//!
//! Format implementations are separate from these contracts. A source must
//! expose a stable size and positioned reads; the output is any
//! [`std::io::Write`], which receives bytes in order. Forward-only input must
//! be spooled by a platform adapter or rejected explicitly.

#![forbid(unsafe_code)]

pub mod arith;
pub mod caj;
mod error;
mod fallible;
mod gb18030;
pub mod hnc8;
mod io;
pub mod jbig1;
pub mod jbig2;
pub mod kdh;
mod limits;
pub mod native;
mod operations;
pub mod pdf;
pub mod qm;
#[cfg(test)]
pub(crate) mod test_support;

pub use error::{Error, PdfErrorKind, Result};
pub(crate) use io::write_counted;
pub use io::{
    Cancellation, CountingSource, NeverCancel, Payload, RangedSource, read_exact_at, read_payload,
};
pub use limits::{DEFAULT_IO_CHUNK, Limits, MAX_BUDGET_COUNT, MAX_IO_CHUNK};
pub use operations::{
    Bookmark, BookmarkVisitor, ConversionOptions, ConversionReport, Detection, DocumentInfo,
    InputFormat, OmittedPage, PDF_HEADER_SEARCH_BYTES, SIGNATURE_BYTES, detect_format,
    detect_source,
};
