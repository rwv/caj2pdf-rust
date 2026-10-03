// SPDX-License-Identifier: MIT

//! Native C8 document orchestration using the shared image codecs and writer.

use super::*;
use crate::hnc8::{C8PageFonts, write_c8_native_page};
use crate::pdf::{FontObject, ImageObject, TrueTypeFont};

/// Explicit ranged font sources, embedded once per document.
/// Multiple roles may reference one source index. At most four distinct
/// resources are needed by the admitted profile; no system fonts are searched.
pub struct C8FontSources<'a, F> {
    pub sources: &'a mut [F],
    pub roles: C8PageFonts,
}

/// Convert every page of the admitted raw C8 native profile.
///
/// Fonts are caller-owned stable ranged resources. Images use the same
/// preflight, codecs and reusable scratch as image-only composition. Only
/// current-page image handles are retained; native content streams in source
/// order. Unknown required records and missing glyphs fail explicitly.
/// The caller must discard partial output on error or a dropped future.
/// Bookmarks remain unsupported. CLI/JavaScript transport is separate.
#[allow(clippy::too_many_arguments)]
pub async fn convert_c8_native_pdf<'a, S, F, W, T, C>(
    source: &mut S,
    sink: &mut W,
    fonts: C8FontSources<'_, F>,
    table: Option<&QmTable>,
    workspaces: impl Into<ComposeWorkspaces<'a, T>>,
    options: ComposeOptions,
    limits: &Limits,
    cancellation: &C,
) -> Result<ComposeReport, ComposeError>
where
    S: RangedSource,
    F: RangedSource,
    W: SequentialSink,
    T: RandomAccessScratch + 'a,
    C: Cancellation,
{
    validate(options, limits)?;
    let roles = fonts.roles;
    let count = fonts.sources.len();
    if !(1..=4).contains(&count)
        || [roles.cjk, roles.latin, roles.alternate_latin]
            .into_iter()
            .chain(roles.decoration.map(|(index, _)| index))
            .any(|index| index >= count)
    {
        return Err(At::NONE.error(
            ComposeStage::Preflight,
            ComposeErrorKind::InvalidOptions(
                "C8 font roles require 1..=4 explicit resources with valid indices",
            ),
        ));
    }
    let mut counted = CountingSource { source, bytes: 0 };
    let mut reader = Hnc8Reader::open(&mut counted, limits, cancellation, options.container)
        .await
        .map_err(|error| container(error, ComposeStage::Container))?;
    let header = reader.header();
    let at = At {
        variant: Some(header.variant),
        offset: Some(0),
        ..At::NONE
    };
    if header.variant != Variant::C8 || options.include_bookmarks {
        return Err(at.error(
            ComposeStage::Preflight,
            ComposeErrorKind::Unsupported("native composition requires C8 with bookmarks omitted"),
        ));
    }
    let mut document = PdfDocument::new(sink, limits, cancellation)
        .await
        .map_err(at.io(ComposeStage::Pdf))?;
    let mut handles: Vec<FontObject> =
        page_vector(count, limits, "C8 font handles").map_err(at.io(ComposeStage::Preflight))?;
    let mut font_bytes = 0u64;
    for source in fonts.sources {
        let mut counted_font = CountingSource { source, bytes: 0 };
        let mut font = TrueTypeFont::read(&mut counted_font, limits, cancellation)
            .await
            .map_err(at.io(ComposeStage::Preflight))?;
        handles.push(
            document
                .add_font(&mut font)
                .await
                .map_err(at.io(ComposeStage::Pdf))?,
        );
        font_bytes = font_bytes.saturating_add(counted_font.bytes);
    }
    // Fixed profile bound avoids a second allocation for references.
    let references = [
        &handles[roles.cjk],
        &handles[roles.latin],
        &handles[roles.alternate_latin],
        &handles[roles.decoration.map_or(roles.cjk, |(index, _)| index)],
    ];
    let page_roles = C8PageFonts {
        cjk: 0,
        latin: 1,
        alternate_latin: 2,
        decoration: roles.decoration.map(|(_, alias)| (3, alias)),
    };
    let mut report = ComposeReport::new(header);
    let mut workspaces = workspaces.into();
    let mut contexts = None;
    while let Some(page) = reader
        .next_page()
        .await
        .map_err(|error| container(error, ComposeStage::Container))?
    {
        let at = At::page(header, page);
        let count = usize_from_u32(page.image_count);
        if count > MAX_PAGE_IMAGE_PLACEMENTS {
            return Err(at.io(ComposeStage::Preflight)(Error::LimitExceeded {
                resource: "PDF image placements per page",
                limit: MAX_PAGE_IMAGE_PLACEMENTS as u64,
                attempted: u64::from(page.image_count),
            }));
        }
        let bytes = metadata_bytes(
            u64::from(page.image_count),
            (size_of::<ImageObject>() + size_of::<bool>()) as u64,
        )
        .map_err(at.io(ComposeStage::Preflight))?;
        check_metadata(bytes, options.budget).map_err(at.io(ComposeStage::Preflight))?;
        let mut images = page_vector(count, limits, "native page image handles")
            .map_err(at.io(ComposeStage::Preflight))?;
        let mut top_first = page_vector(count, limits, "native page image orientation")
            .map_err(at.io(ComposeStage::Preflight))?;
        let bytes = capacity_bytes::<ImageObject>(images.capacity())
            + capacity_bytes::<bool>(top_first.capacity());
        check_metadata(bytes, options.budget).map_err(at.io(ComposeStage::Preflight))?;
        report.peak_page_metadata_bytes = report.peak_page_metadata_bytes.max(bytes);
        while let Some(record) = reader
            .next_image()
            .await
            .map_err(|error| container(error, ComposeStage::Container))?
        {
            let image_at = at.image(record);
            let (checked, visible_width, display_width, height) = preflight_image(
                reader.source_mut(),
                record,
                header.variant,
                image_at,
                table,
                workspaces.type3.is_some(),
                options,
                limits,
                cancellation,
            )
            .await?;
            top_first.push(matches!(checked, CheckedImage::Type3 { .. }));
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
            images.push(
                emit_image(
                    reader.source_mut(),
                    &mut document,
                    &mut image,
                    image_at,
                    &mut contexts,
                    &mut workspaces,
                    table,
                    options,
                    limits,
                    cancellation,
                    &mut report,
                )
                .await?,
            );
        }
        report.output_pages = write_c8_native_page(
            &mut reader,
            &mut document,
            &references,
            page_roles,
            &images,
            &top_first,
            options.text,
        )
        .await
        .map_err(|error| container(error, ComposeStage::Text))?
            + 1;
        report.no_image_pages += u32::from(count == 0);
    }
    report.conversion = document.finish().await.map_err(at.io(ComposeStage::Pdf))?;
    report.conversion.input_bytes_read = counted.bytes.saturating_add(font_bytes);
    Ok(report)
}
