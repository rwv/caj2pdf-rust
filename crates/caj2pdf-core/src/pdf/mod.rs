// SPDX-License-Identifier: MIT

//! Bounded PDF input, fragment repair, and forward-only PDF 1.7 output.

mod append;
mod document;
mod fragment;
pub(crate) mod input;
mod types;
mod writer;

pub use append::{PdfOutlineAppender, copy_pdf, copy_pdf_range};
pub use document::{ImageEncoding, ImageSpec, PageSpec, PdfDocument};
pub use fragment::{
    FragmentObject, FragmentPlan, reconstruct_fragment, reconstruct_fragment_with_bookmarks,
};
pub use input::{PdfIndex, RepairObject};
pub use types::{PdfRange, PdfRef};
pub use writer::{MAX_CLASSIC_PDF_BYTES, ObjectId, PdfWriter};
