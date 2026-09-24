// SPDX-License-Identifier: MIT

//! Narrow, forward-only PDF 1.7 document construction.

mod document;
mod writer;

pub use document::{ImageEncoding, ImageSpec, PageSpec, PdfDocument};
pub use writer::{MAX_CLASSIC_PDF_BYTES, ObjectId, PdfWriter};
