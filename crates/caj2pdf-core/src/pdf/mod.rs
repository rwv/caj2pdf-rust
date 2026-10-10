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
pub(crate) use font::is_valid_postscript_name;
pub use font::{FontGlyph, MAX_FONT_METADATA_BYTES, OpenTypeFont};
pub use fragment::{FragmentObject, FragmentPlan, reconstruct_fragment_with_bookmarks};
pub(crate) use fragment::{
    InspectedObject, InspectedPlan, append_replacement, reconstruct_inspected,
};
pub use input::ttkn::TtknResponse;
pub(crate) use input::ttkn::convert as convert_ttkn;
pub use input::{PdfIndex, RepairObject};
pub use outline::BookmarkView;
pub use types::{PdfRange, PdfRef};
pub use writer::{MAX_CLASSIC_PDF_BYTES, ObjectId, PdfWriter};

impl crate::Error {
    /// A located PDF problem of `kind`.
    pub(crate) fn pdf(
        kind: crate::ErrorKind,
        offset: u64,
        object: Option<(u32, u16)>,
        reason: &'static str,
    ) -> Self {
        Self::from(kind).at(offset).because(reason).in_pdf(object)
    }

    /// Mark a located PDF problem as a damaged structure whose repair
    /// would be ambiguous.
    pub(crate) fn ambiguous_repair(self) -> Self {
        match self.context {
            crate::Context::Pdf { object, .. } => self.within(crate::Context::Pdf {
                object,
                repair: true,
            }),
            _ => self,
        }
    }

    /// Whether this is a located PDF structure problem: malformed, encrypted,
    /// unsupported or an ambiguous repair, not a limit, I/O failure or
    /// cancellation.
    pub(crate) fn is_pdf_problem(&self) -> bool {
        matches!(self.context, crate::Context::Pdf { .. })
            && matches!(
                self.kind,
                crate::ErrorKind::Malformed
                    | crate::ErrorKind::Encrypted
                    | crate::ErrorKind::UnsupportedFormat
            )
    }

    /// Whether this is a located malformed PDF structure, not an ambiguous
    /// repair.
    pub(crate) fn is_malformed_pdf(&self) -> bool {
        matches!(self.kind, crate::ErrorKind::Malformed)
            && matches!(self.context, crate::Context::Pdf { repair: false, .. })
    }
}
