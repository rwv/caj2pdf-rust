// SPDX-License-Identifier: MIT

//! Structure-only diagnostics for unseen HN/C8 profiles.
//!
//! These helpers reuse the existing page-text readers and report which one
//! accepts a page, with counts and lengths only. They never return text,
//! titles or image payload bytes, and they add no format interpretation.

use super::{
    ErrorKind, Hnc8Reader, Location, NativeRecord, NativeRecordVisitor, Result, TextBudget,
    Variant, appinfo::Trailer, text::read_coordinates,
};
use crate::{Cancellation, RangedSource};

/// The page-text framing accepted by the existing readers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextFraming {
    /// The indexed text span is empty.
    None,
    /// Uncompressed HN-A records.
    Raw,
    /// Uncompressed HN-A records after the paired `8003` page-size prefix.
    RawPaired,
    /// A 16-byte direct `COMPRESSTEXT` header and one zlib frame.
    CompressText,
    /// The 24-byte paired-`8003` and `COMPRESSTEXT` header and one zlib frame.
    Legacy24,
    /// Raw C8/HN-B native records.
    Native,
}

impl TextFraming {
    /// A stable lowercase label for reports.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Raw => "raw",
            Self::RawPaired => "raw-paired",
            Self::CompressText => "compresstext",
            Self::Legacy24 => "legacy-24",
            Self::Native => "native",
        }
    }
}

/// How a page's text span is framed, without any of its content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TextStructure {
    pub framing: TextFraming,
    /// Glyph, raw or native records counted by the accepting reader.
    pub records: u32,
    /// The inflated length of a compressed frame; `None` otherwise.
    pub decoded_length: Option<u32>,
}

/// The trailing application-info section; its contents are not parsed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApplicationInfoTail {
    /// The decimal start offset declared by the trailer.
    pub offset: u64,
    /// Bytes from `offset` to the end of the source, including the trailer;
    /// `None` when the declared offset is not inside the source.
    pub length: Option<u64>,
}

struct Discard;

impl NativeRecordVisitor for Discard {
    async fn visit(&mut self, _offset: u64, _record: NativeRecord) -> crate::Result<()> {
        Ok(())
    }
}

impl<S: RangedSource, C: Cancellation> Hnc8Reader<'_, S, C> {
    /// Bytes per page-index row: 20, or 12 for the compact HN-B layout
    /// selected by a zero word at `0x88`.
    pub fn page_row_bytes(&self) -> u64 {
        self.page_row_bytes
    }

    /// Classify the current page's text with the reader that accepts it.
    ///
    /// Call after `next_page` and after reading its image descriptors. HN-A
    /// and C8 use the compressed or raw page-text reader with the same
    /// coordinate-group rule as composition. C8 spans whose compressed header
    /// is absent, and all HN-B spans, are framed as native records. Errors are
    /// the first located error of the deciding reader. Like
    /// [`Hnc8Reader::visit_native_records`], a failed native framing poisons
    /// the reader; open a fresh cursor to continue with later pages.
    pub async fn inspect_text(&mut self, budget: TextBudget) -> Result<TextStructure> {
        let header = self.header;
        let loc = Location {
            variant: Some(header.variant),
            offset: header.page_index.offset,
            page: None,
            image: None,
        };
        if self.poisoned {
            return Err(loc.error(ErrorKind::Poisoned));
        }
        let page = self
            .current
            .ok_or_else(|| loc.error(ErrorKind::NoCurrentPage))?
            .page;
        if page.text.length == 0 {
            return Ok(TextStructure {
                framing: TextFraming::None,
                records: 0,
                decoded_length: None,
            });
        }
        if header.variant != Variant::HnB {
            match read_coordinates(
                self.source,
                header,
                page,
                self.limits,
                self.cancellation,
                budget,
            )
            .await
            {
                Ok(text) => {
                    let framing = match text.zlib_frame {
                        None if text.page_size.is_some() => TextFraming::RawPaired,
                        None => TextFraming::Raw,
                        Some(frame) if frame.offset - page.text.offset == 16 => {
                            TextFraming::CompressText
                        }
                        Some(_) => TextFraming::Legacy24,
                    };
                    return Ok(TextStructure {
                        framing,
                        records: text.record_count,
                        decoded_length: text.zlib_frame.map(|_| text.decoded_length),
                    });
                }
                Err(error)
                    if header.variant == Variant::C8
                        && matches!(
                            error.kind.field(),
                            "page text prefix" | "page text header"
                        ) => {}
                Err(error) => return Err(error),
            }
        }
        let records = self.visit_native_records(budget, &mut Discard).await?;
        Ok(TextStructure {
            framing: TextFraming::Native,
            records,
            decoded_length: None,
        })
    }

    /// Find a trailing `APPINFOSIGN <decimal offset>` marker that ends the
    /// source, located like the C8 package reader does. Only its presence and
    /// extent are reported; the section itself is neither validated nor
    /// decoded, and a marker without a usable offset is reported as absent.
    /// This does not affect the cursor.
    pub async fn application_info_tail(&mut self) -> Result<Option<ApplicationInfoTail>> {
        let size = self.source.size();
        match self.application_info_trailer().await {
            Ok(trailer) => Ok(trailer.map(|Trailer { start, .. }| ApplicationInfoTail {
                offset: start,
                length: (start < size).then(|| size - start),
            })),
            Err(error) if matches!(error.kind, ErrorKind::Malformed { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests;
