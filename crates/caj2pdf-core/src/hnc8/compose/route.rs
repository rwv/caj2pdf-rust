// SPDX-License-Identifier: MIT

//! One routing rule for documents converted with optional native fonts.

use super::*;
use crate::hnc8::TextFraming;
use crate::hnc8::native_page::admits_native_mode;
use std::io::Write;

/// Whether native composition is chosen for this document.
///
/// Native composition draws only C8 and HN-B documents in an admitted
/// rendering mode (mode 2, or HN-B mode 0), so every other document, HN-A
/// included, uses image composition after its header is read. Otherwise the
/// first page with a nonempty text span decides; pages are walked with one
/// cursor, reading only page rows and image descriptors before it:
///
/// - HN-B text is only ever native records, which image composition never
///   reads, so such a page selects native composition. A defect in that text
///   is then reported by native composition instead of being dropped.
/// - C8 text is classified by [`Hnc8Reader::inspect_text`]. Native records,
///   or text neither reader accepts, select native composition, which
///   reports the located defect; compressed or raw text selects image
///   composition.
///
/// A document without text uses image composition. Image composition refuses
/// every native C8 page, so a mixed document fails in either composer, at its
/// first page the composer cannot draw. A malformed container, page row or
/// descriptor uses image composition, which reports it. A failed source read
/// or cancellation is returned. Reads are ranged and bounded by `limits`; no
/// image payload is read and no text is retained.
pub fn uses_native_text<S, C>(source: &mut S, limits: &Limits, cancellation: &C) -> Result<bool>
where
    S: RangedSource,
    C: Cancellation,
{
    validate(limits)?;
    let mut reader = match Hnc8Reader::open(source, limits, cancellation) {
        Ok(reader) if !admits_native_mode(reader.header()) => return Ok(false),
        Ok(reader) => reader,
        Err(error) => return image_unless_fatal(error, Hnc8Stage::Container),
    };
    loop {
        let page = match reader.next_page() {
            Ok(Some(page)) => page,
            Ok(None) => return Ok(false),
            Err(error) => return image_unless_fatal(error, Hnc8Stage::Container),
        };
        loop {
            match reader.next_image() {
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(error) => return image_unless_fatal(error, Hnc8Stage::Container),
            }
        }
        if page.text.length == 0 {
            continue;
        }
        if reader.header().variant == Variant::HnB {
            return Ok(true);
        }
        return match reader.inspect_text() {
            Ok(text) => Ok(text.framing == TextFraming::Native),
            Err(error) => image_unless_fatal(error, Hnc8Stage::Text).map(|_| true),
        };
    }
}

/// A located document defect selects image composition, which reports it;
/// only failures independent of the document bytes are returned.
fn image_unless_fatal(error: Error, stage: Hnc8Stage) -> Result<bool> {
    match error.kind {
        ErrorKind::Cancelled | ErrorKind::Io(_) => Err(container(error, stage)),
        _ => Ok(false),
    }
}

/// Convert an HN/C8 document, with native text when fonts are supplied.
///
/// Without `fonts` this is exactly [`convert_source_pages_pdf`]. With fonts,
/// [`uses_native_text`] decides once per document: native documents use
/// `convert_c8_native_pdf` and all others use image composition, leaving the
/// fonts unread and unvalidated, so the PDF is byte-identical to a conversion
/// without fonts. `visitor` receives image-composition pages only. The
/// routing reads are added to `conversion.input_bytes_read`. The CLI, Node
/// and browser adapters all route through this function.
#[allow(clippy::too_many_arguments)]
pub fn convert_document_pdf<S, F, W, V, C>(
    source: &mut S,
    sink: &mut W,
    fonts: Option<C8FontSources<'_, F>>,
    table: Option<&QmTable>,
    visitor: &mut V,
    options: ComposeOptions,
    limits: &Limits,
    cancellation: &C,
) -> Result<ComposeReport>
where
    S: RangedSource,
    F: RangedSource,
    W: Write,
    V: ComposeVisitor,
    C: Cancellation,
{
    let mut routing = 0;
    let mut counted = CountingSource::new(&mut *source, &mut routing);
    let native = match fonts {
        Some(fonts) if uses_native_text(&mut counted, limits, cancellation)? => Some(fonts),
        _ => None,
    };
    let mut report = match native {
        Some(fonts) => {
            convert_c8_native_pdf(source, sink, fonts, table, options, limits, cancellation)?
        }
        None => {
            convert_source_pages_pdf(source, sink, table, visitor, options, limits, cancellation)?
        }
    };
    report.conversion.input_bytes_read = report.conversion.input_bytes_read.saturating_add(routing);
    Ok(report)
}
