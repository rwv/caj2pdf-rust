// SPDX-License-Identifier: MIT

//! Native C8 document orchestration using the shared image codecs and writer.

use super::*;
use crate::hnc8::{
    C8PageFonts, NativeSymbolGlyph, SymbolFontIdentity, is_mode_zero_symbol, write_c8_native_page,
};
use crate::pdf::{FontObject, ImageObject, OpenTypeFont};
use std::io::Write;

/// Explicit ranged font sources, embedded once per document.
/// Multiple roles may reference one source index. At most eight distinct
/// resources are needed by the admitted profile; no system fonts are searched.
/// Absent optional roles and unmapped characters follow the
/// [`C8PageFonts`] fallback rule.
pub struct C8FontSources<'a, F> {
    pub sources: &'a mut [C8FontSource<F>],
    pub roles: C8PageFonts,
    /// Explicit HN-B mode-0 symbol glyphs from the `symbols` font. Each code
    /// appears once and draws exactly its glyph; other codes keep the
    /// [`C8PageFonts`] rules. Must be empty for any other native profile.
    /// A shared `symbols` source is embedded as one mapped font for every
    /// role that uses it.
    pub symbol_glyphs: &'a [NativeSymbolGlyph],
    /// The font `symbol_glyphs` was measured on; requires a nonempty map.
    pub symbol_font: Option<&'a SymbolFontIdentity>,
}

/// One font resource: a ranged source and its face index, zero for a
/// standalone font or the selected face of a font collection.
pub struct C8FontSource<F> {
    pub source: F,
    pub face: u32,
}

/// Convert every page of the admitted raw C8 or text/vector HN-B profile.
///
/// Fonts are caller-owned stable ranged resources. Images use the same
/// preflight, codecs and reusable buffers as image-only composition. Only
/// current-page image handles are retained; native content streams in source
/// order. Unknown required records and missing glyphs fail explicitly.
/// The caller must discard partial output on error.
/// Bookmarks remain unsupported. CLI/JavaScript transport is separate.
pub fn convert_c8_native_pdf<S, F, W, C>(
    source: &mut S,
    sink: &mut W,
    fonts: C8FontSources<'_, F>,
    table: Option<&QmTable>,
    options: ComposeOptions,
    limits: &Limits,
    cancellation: &C,
) -> Result<ComposeReport>
where
    S: RangedSource,
    F: RangedSource,
    W: Write,
    C: Cancellation,
{
    validate(limits)?;
    let roles = fonts.roles;
    let count = fonts.sources.len();
    if !(1..=8).contains(&count)
        || [roles.cjk, roles.latin]
            .into_iter()
            .chain(roles.alternate_latin)
            .chain(roles.decoration.map(|(index, _)| index))
            .chain(roles.symbols)
            .chain(roles.latin_state3)
            .chain(roles.latin_state28)
            .chain(roles.latin_state31)
            .any(|index| index >= count)
    {
        return Err(At::NONE.error(
            Hnc8Stage::Preflight,
            Error::invalid("C8 font roles require 1..=8 explicit resources with valid indices"),
        ));
    }
    let symbol_glyphs = fonts.symbol_glyphs;
    let mapped_symbols = if symbol_glyphs.is_empty() {
        None
    } else {
        Some(roles.symbols.ok_or_else(|| {
            At::NONE.error(
                Hnc8Stage::Preflight,
                Error::invalid("native symbol glyphs require a symbols font role"),
            )
        })?)
    };
    if fonts.symbol_font.is_some() && mapped_symbols.is_none() {
        return Err(At::NONE.error(
            Hnc8Stage::Preflight,
            Error::invalid("symbol font identity requires native symbol glyphs"),
        ));
    }
    for (index, entry) in symbol_glyphs.iter().enumerate() {
        let refuse = |reason| Err(At::NONE.error(Hnc8Stage::Preflight, Error::invalid(reason)));
        if !is_mode_zero_symbol(entry.code) {
            return refuse("native symbol glyph code is not an HN-B mode-0 symbol");
        }
        if symbol_glyphs[..index].iter().any(|e| e.code == entry.code) {
            return refuse("native symbol glyph code is mapped more than once");
        }
    }
    let mut input_bytes_read = 0;
    let mut counted = CountingSource::new(source, &mut input_bytes_read);
    let mut reader = Hnc8Reader::open(&mut counted, limits, cancellation)
        .map_err(|error| container(error, Hnc8Stage::Container))?;
    let header = reader.header();
    let at = At {
        variant: Some(header.variant),
        offset: Some(0),
        ..At::NONE
    };
    if !matches!(header.variant, Variant::C8 | Variant::HnB) {
        return Err(at.error(
            Hnc8Stage::Preflight,
            unsupported("native composition requires C8 or HN-B"),
        ));
    }
    if mapped_symbols.is_some() && (header.variant, header.native_mode) != (Variant::HnB, Some(0)) {
        return Err(at.error(
            Hnc8Stage::Preflight,
            Error::invalid("native symbol glyphs apply only to HN-B mode-0 text"),
        ));
    }
    let mut document =
        PdfDocument::new(sink, limits, cancellation).map_err(at.locator(Hnc8Stage::Pdf))?;
    let mut handles: Vec<FontObject> =
        page_vector(count, limits, "C8 font handles").map_err(at.locator(Hnc8Stage::Preflight))?;
    let mut font_bytes = 0u64;
    for (index, C8FontSource { source, face }) in fonts.sources.iter_mut().enumerate() {
        let mut counted_font = CountingSource::new(source, &mut font_bytes);
        let font = OpenTypeFont::read(&mut counted_font, *face, limits, cancellation)
            .map_err(at.locator(Hnc8Stage::Preflight))?;
        let mapped = mapped_symbols == Some(index);
        let handle = if mapped {
            document.add_mapped_font(&font)
        } else {
            document.add_font(&font)
        }
        .map_err(at.locator(Hnc8Stage::Pdf))?;
        if mapped
            && let Some(identity) = fonts.symbol_font
            && (font.checksum_adjustment().ok() != Some(identity.checksum_adjustment)
                || font.postscript_name().ok().as_deref()
                    != Some(identity.postscript_name.as_str()))
        {
            return Err(at.error(
                Hnc8Stage::Preflight,
                Error::invalid("symbols font does not match the expected identity"),
            ));
        }
        // A missing source glyph is refused, never drawn by fallback.
        if mapped
            && !symbol_glyphs
                .iter()
                .all(|entry| handle.supports(entry.glyph))
        {
            return Err(at.error(
                Hnc8Stage::Preflight,
                Error::invalid("symbols font does not map a native symbol glyph"),
            ));
        }
        handles.push(handle);
    }
    // Fixed profile bound avoids a second allocation for references.
    let references = [
        &handles[roles.cjk],
        &handles[roles.latin],
        &handles[roles.alternate_latin.unwrap_or(roles.cjk)],
        &handles[roles.decoration.map_or(roles.cjk, |(index, _)| index)],
        &handles[roles.symbols.unwrap_or(roles.cjk)],
        &handles[roles.latin_state3.unwrap_or(roles.cjk)],
        &handles[roles.latin_state28.unwrap_or(roles.cjk)],
        &handles[roles.latin_state31.unwrap_or(roles.cjk)],
    ];
    let page_roles = C8PageFonts {
        cjk: 0,
        latin: 1,
        alternate_latin: roles.alternate_latin.map(|_| 2),
        decoration: roles.decoration.map(|(_, alias)| (3, alias)),
        symbols: roles.symbols.map(|_| 4),
        latin_state3: roles.latin_state3.map(|_| 5),
        latin_state28: roles.latin_state28.map(|_| 6),
        latin_state31: roles.latin_state31.map(|_| 7),
    };
    let mut report = ComposeReport::new(header);
    // C8/HN-B outlines are unverified; a request writes none and is reported.
    report.outline.unverified = options.include_bookmarks;
    let mut buffers = ImageBuffers::default();
    while let Some(page) = reader
        .next_page()
        .map_err(|error| container(error, Hnc8Stage::Container))?
    {
        let at = At::page(header, page);
        let count = admit_page_images(
            page,
            &[size_of::<ImageObject>(), size_of::<bool>()],
            at,
            limits,
        )?;
        let mut images = page_vector(count, limits, "native page image handles")
            .map_err(at.locator(Hnc8Stage::Preflight))?;
        let mut top_first = page_vector(count, limits, "native page image orientation")
            .map_err(at.locator(Hnc8Stage::Preflight))?;
        while let Some(record) = reader
            .next_image()
            .map_err(|error| container(error, Hnc8Stage::Container))?
        {
            let image_at = at.image(record);
            let (checked, plan, visible_width, display_width, height) = preflight_image(
                reader.source_mut(),
                record,
                image_at,
                table,
                options,
                limits,
                cancellation,
            )?;
            top_first.push(!matches!(checked, CheckedImage::Jpeg(_)));
            // Native placement comes from the record visitor, after resources
            // are emitted. The codec emitter does not consume this transform.
            let mut image = ComposedImage {
                record,
                visible_width,
                display_width,
                height,
                checked,
                transform: [0.0; 6],
                duplicate_of: None,
                type3_text_header_anomaly: None,
            };
            images.push(emit_image(
                reader.source_mut(),
                &mut document,
                &mut image,
                plan,
                image_at,
                &mut buffers,
                table,
                limits,
                cancellation,
                &mut report,
            )?);
        }
        report.output_pages = write_c8_native_page(
            &mut reader,
            &mut document,
            &references,
            page_roles,
            symbol_glyphs,
            &images,
            &top_first,
        )
        .map_err(|error| container(error, Hnc8Stage::Text))?
            + 1;
        report.no_image_pages += u32::from(count == 0);
    }
    // Metadata is read again rather than retained for every font; only the
    // drawn glyphs' outlines are then read for each subset.
    for (handle, C8FontSource { source, face }) in handles.iter().zip(fonts.sources.iter_mut()) {
        let mut counted_font = CountingSource::new(source, &mut font_bytes);
        let mut font = OpenTypeFont::read(&mut counted_font, *face, limits, cancellation)
            .map_err(at.locator(Hnc8Stage::Pdf))?;
        document
            .embed_font(handle, &mut font)
            .map_err(at.locator(Hnc8Stage::Pdf))?;
    }
    report.conversion = finish_document(&mut reader, document, &mut report, at)?;
    report.conversion.input_bytes_read = input_bytes_read.saturating_add(font_bytes);
    Ok(report)
}
