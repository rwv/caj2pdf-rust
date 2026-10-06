// SPDX-License-Identifier: MIT

//! Bounded PDF input, fragment repair, and forward-only PDF 1.7 output.

mod append;
mod document;
mod font;
#[cfg(test)]
pub(crate) use font::tests::{drawing_font, symbol_font};
mod fragment;
pub(crate) mod input;
mod outline;
mod page_walk;
mod types;
mod writer;
mod xref;

pub use append::{PdfOutlineAppender, copy_pdf, copy_pdf_range};
pub use document::{
    BilevelImageSpec, BilevelImageWriter, ContentPageWriter, FontObject, ImageEncoding,
    ImageObject, ImagePlacement, ImageSpec, MAX_PAGE_IMAGE_PLACEMENTS, PageSpec, PdfDocument,
};
pub use font::{FontGlyph, MAX_FONT_METADATA_BYTES, OpenTypeFont};
pub use fragment::{FragmentObject, FragmentPlan, reconstruct_fragment_with_bookmarks};
pub use input::{PdfIndex, RepairObject};
pub use outline::BookmarkView;
pub use types::{PdfRange, PdfRef};
pub use writer::{MAX_CLASSIC_PDF_BYTES, ObjectId, PdfWriter};
