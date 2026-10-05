// SPDX-License-Identifier: MIT

//! One routing rule for documents converted with optional native fonts.

use super::*;
use crate::hnc8::{ErrorKind, TextFraming};

/// Whether native composition is chosen for this document.
///
/// HN-A has no native text, so only its header is read. For C8 and HN-B the
/// pages are walked in order with one cursor: each page's image descriptors
/// are read, then its text is classified by [`Hnc8Reader::inspect_text`] with
/// `options.text`, the budget native composition uses. The first page framed
/// as raw C8/HN-B native records selects native composition and ends the walk;
/// a document without such a page uses image composition.
///
/// **Mixed documents.** Image composition rejects every C8 page framed as
/// native records, so a C8 document with any native page can only convert
/// natively. A document that mixes native and compressed or raw C8 text is
/// therefore routed to native composition, which reports any page it cannot
/// draw. A C8 document image composition can convert has no native page and
/// is never routed away from it. HN-B text is only ever framed as native
/// records, so an HN-B document with text uses native composition.
///
/// A page that neither text reader accepts, or a malformed container,
/// descriptor or limit, ends the walk with image composition; the composer
/// then reports its own located error. A dropped source read or cancellation
/// is returned. Reads are ranged and bounded by `options.container` and
/// `options.text`; no image payload is read and no text is retained.
pub async fn uses_native_text<S, C>(
    source: &mut S,
    options: ComposeOptions,
    limits: &Limits,
    cancellation: &C,
) -> Result<bool, ComposeError>
where
    S: RangedSource,
    C: Cancellation,
{
    validate(options, limits)?;
    let mut reader = match Hnc8Reader::open(source, limits, cancellation, options.container).await {
        Ok(reader) if reader.header().variant == Variant::HnA => return Ok(false),
        Ok(reader) => reader,
        Err(error) => return image_unless_fatal(error, ComposeStage::Container),
    };
    loop {
        match reader.next_page().await {
            Ok(Some(_)) => {}
            Ok(None) => return Ok(false),
            Err(error) => return image_unless_fatal(error, ComposeStage::Container),
        }
        loop {
            match reader.next_image().await {
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(error) => return image_unless_fatal(error, ComposeStage::Container),
            }
        }
        match reader.inspect_text(options.text).await {
            Ok(text) if text.framing == TextFraming::Native => return Ok(true),
            Ok(_) => {}
            Err(error) => return image_unless_fatal(error, ComposeStage::Text),
        }
    }
}

/// A located document defect selects image composition, which reports it;
/// only failures independent of the document bytes are returned.
fn image_unless_fatal(error: Hnc8Error, stage: ComposeStage) -> Result<bool, ComposeError> {
    match error.kind {
        ErrorKind::Cancelled | ErrorKind::Source { .. } => Err(container(error, stage)),
        _ => Ok(false),
    }
}

/// Convert an HN/C8 document, with native text when fonts are supplied.
///
/// Without `fonts` this is exactly [`convert_source_pages_pdf`]. With fonts,
/// [`uses_native_text`] decides once per document: native documents use
/// [`convert_c8_native_pdf`] and all others use image composition, leaving the
/// fonts unread and unvalidated, so the PDF is byte-identical to a conversion
/// without fonts. `visitor` receives image-composition pages only. The
/// routing reads are added to `conversion.input_bytes_read`. The CLI, Node
/// and browser adapters all route through this function.
#[allow(clippy::too_many_arguments)]
pub async fn convert_document_pdf<'a, S, F, W, T, V, C>(
    source: &mut S,
    sink: &mut W,
    fonts: Option<C8FontSources<'_, F>>,
    table: Option<&QmTable>,
    workspaces: impl Into<ComposeWorkspaces<'a, T>>,
    visitor: &mut V,
    options: ComposeOptions,
    limits: &Limits,
    cancellation: &C,
) -> Result<ComposeReport, ComposeError>
where
    S: RangedSource,
    F: RangedSource,
    W: SequentialSink,
    T: RandomAccessScratch + 'a,
    V: ComposeVisitor,
    C: Cancellation,
{
    let mut counted = CountingSource { source, bytes: 0 };
    let native = match fonts {
        Some(fonts) if uses_native_text(&mut counted, options, limits, cancellation).await? => {
            Some(fonts)
        }
        _ => None,
    };
    let routing = counted.bytes;
    let source = counted.source;
    let mut report = match native {
        Some(fonts) => {
            convert_c8_native_pdf(
                source,
                sink,
                fonts,
                table,
                workspaces,
                options,
                limits,
                cancellation,
            )
            .await?
        }
        None => {
            convert_source_pages_pdf(
                source,
                sink,
                table,
                workspaces,
                visitor,
                options,
                limits,
                cancellation,
            )
            .await?
        }
    };
    report.conversion.input_bytes_read = report.conversion.input_bytes_read.saturating_add(routing);
    Ok(report)
}
