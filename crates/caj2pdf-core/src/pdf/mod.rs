// SPDX-License-Identifier: MIT

//! Bounded PDF input, fragment repair, and forward-only PDF 1.7 output.

mod append;
mod document;
mod fragment;
pub(crate) mod input;
mod types;
mod writer;

pub use append::{PdfOutlineAppender, copy_pdf, copy_pdf_range};
pub use document::{
    BilevelImageSpec, BilevelImageWriter, ImageEncoding, ImageObject, ImageSpec, PageSpec,
    PdfDocument,
};
pub use fragment::{
    FragmentObject, FragmentPlan, reconstruct_fragment, reconstruct_fragment_with_bookmarks,
};
pub use input::{PdfIndex, RepairObject};
pub use types::{PdfRange, PdfRef};
pub use writer::{MAX_CLASSIC_PDF_BYTES, ObjectId, PdfWriter};

/// Pop the top of an open-outline stack while it holds more than `depth`
/// items, closing every item deeper than a new item at `depth`.
fn pop_deeper_than<T>(stack: &mut Vec<T>, depth: usize) -> Option<T> {
    if stack.len() > depth {
        stack.pop()
    } else {
        None
    }
}
