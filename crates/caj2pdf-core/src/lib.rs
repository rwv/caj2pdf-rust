// SPDX-License-Identifier: MIT

//! Platform-neutral, bounded I/O contracts for CAJ-family conversion.
//!
//! Format implementations are separate from these contracts. A source must
//! expose a stable size and positioned reads; a sink receives bytes in order
//! and may apply backpressure through its returned future. Forward-only input
//! must be spooled by a platform adapter or rejected explicitly.

#![forbid(unsafe_code)]

pub mod caj;
mod error;
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
pub use io::{
    Cancellation, NeverCancel, RangedSource, SequentialSink, copy_range, read_exact_at, write_all,
};
pub use limits::{DEFAULT_IO_CHUNK, Limits, MAX_IO_CHUNK};
pub use operations::{
    Bookmark, BookmarkVisitor, ConversionOptions, ConversionReport, DocumentInfo,
    DocumentOperations, InputFormat,
};
