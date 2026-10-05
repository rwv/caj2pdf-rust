// SPDX-License-Identifier: MIT

//! Subset TrueType resources and incremental mixed-page content.

use super::*;
use crate::pdf::TrueTypeFont;
use crate::pdf::font::{CHANGED, SubsetOutput, has_code, mark_code};

const BMP_BITMAP_BYTES: usize = 8192;
const MAX_PAGE_FONTS: usize = 128;
const FONT_STREAM_CHUNK: usize = 4096;

/// A font added to a document, usable only in the document that created it.
///
/// A fixed 8 KiB bitmap records which BMP Unicode characters can be drawn.
/// The font program and metric tables are not retained by this handle; the
/// subset is written by [`PdfDocument::embed_font`] after the last page.
pub struct FontObject {
    object: ObjectId,
    document_id: usize,
    slot: usize,
    characters: Vec<u8>,
}

impl FontObject {
    pub fn supports(&self, character: char) -> bool {
        let code = character as usize;
        code <= 0xffff && has_code(&self.characters, code)
    }
}

/// Document-owned state of an added font until its subset is embedded.
pub(super) struct PendingFont {
    /// Font file, CID-to-GID map, descriptor, descendant and the Type 0
    /// font, each stream followed by its length object.
    ids: [ObjectId; 7],
    name: String,
    /// Identifies the font when its source is read again for embedding.
    fingerprint: [u8; 32],
    /// BMP characters drawn with this font; only these glyphs are embedded.
    used: Vec<u8>,
    embedded: bool,
}

/// Feeds bytes through a zlib stream, gathering small writes into fixed
/// chunks first.
struct FontStream<'w, 'a, W: SequentialSink, C: Cancellation> {
    writer: &'w mut PdfWriter<'a, W, C>,
    deflate: &'w mut Deflate,
    buffer: [u8; FONT_STREAM_CHUNK],
    length: usize,
}

impl<'w, 'a, W: SequentialSink, C: Cancellation> FontStream<'w, 'a, W, C> {
    fn new(writer: &'w mut PdfWriter<'a, W, C>, deflate: &'w mut Deflate) -> Self {
        deflate.reset();
        Self {
            writer,
            deflate,
            buffer: [0; FONT_STREAM_CHUNK],
            length: 0,
        }
    }

    async fn finish(self) -> Result<()> {
        let length = self.length;
        self.deflate
            .write(self.writer, &self.buffer[..length], true)
            .await?;
        self.writer.end_stream().await
    }
}

impl<W: SequentialSink, C: Cancellation> SubsetOutput for FontStream<'_, '_, W, C> {
    async fn put(&mut self, mut bytes: &[u8]) -> Result<()> {
        while !bytes.is_empty() {
            let count = bytes.len().min(FONT_STREAM_CHUNK - self.length);
            self.buffer[self.length..self.length + count].copy_from_slice(&bytes[..count]);
            self.length += count;
            bytes = &bytes[count..];
            if self.length == FONT_STREAM_CHUNK {
                self.length = 0;
                self.deflate.write(self.writer, &self.buffer, false).await?;
            }
        }
        Ok(())
    }
}

pub(super) fn ensure_fonts_embedded(fonts: &[PendingFont]) -> Result<()> {
    if fonts.iter().any(|font| !font.embedded) {
        return Err(Error::InvalidInput {
            reason: "PDF font was added but its subset was not embedded",
        });
    }
    Ok(())
}

fn changed() -> Error {
    Error::InvalidInput { reason: CHANGED }
}

impl<'a, W: SequentialSink, C: Cancellation> PdfDocument<'a, W, C> {
    /// Register a static TrueType font for content pages.
    ///
    /// Characters use BMP Unicode CIDs with an explicit CID-to-glyph map and
    /// ToUnicode. Nothing is written yet: draws record the characters used,
    /// and [`Self::embed_font`] must embed the subset before [`Self::finish`].
    /// No system lookup or glyph substitution is performed.
    pub fn add_font<S: RangedSource>(&mut self, font: &TrueTypeFont<'_, S>) -> Result<FontObject> {
        self.ensure_image_page_intact()?;
        let name = font.postscript_name()?.replace('#', "#23");
        let face = font.face()?;
        let mut characters = self.bitmap("PDF font character bitmap")?;
        for code in 0..65536_u32 {
            if bmp_glyph(&face, code)?.is_some() {
                mark_code(&mut characters, code as usize);
            }
        }
        let used = self.bitmap("PDF font used-character bitmap")?;
        let refused = self
            .limits
            .allocation_refused("PDF fonts", size_of::<PendingFont>() as u64);
        reserve(&mut self.fonts, 1, refused)?;
        self.writer.prepare_objects(7)?;
        let mut next = || self.writer.reserve_object();
        let ids = [
            next()?,
            next()?,
            next()?,
            next()?,
            next()?,
            next()?,
            next()?,
        ];
        self.fonts.push(PendingFont {
            ids,
            name,
            fingerprint: font.fingerprint(),
            used,
            embedded: false,
        });
        Ok(FontObject {
            object: ids[6],
            document_id: self.document_id,
            slot: self.fonts.len() - 1,
            characters,
        })
    }

    /// Count font outline bytes read for a subset toward the input limit.
    fn count_font_input(&mut self, bytes: u64) -> Result<()> {
        let total = self.input_bytes_read.saturating_add(bytes);
        self.limits.check_input_size(total)?;
        self.input_bytes_read = total;
        Ok(())
    }

    fn bitmap(&self, resource: &'static str) -> Result<Vec<u8>> {
        self.limits.check_allocation(BMP_BITMAP_BYTES as u64)?;
        let mut bitmap = Vec::new();
        let refused = self
            .limits
            .allocation_refused(resource, BMP_BITMAP_BYTES as u64);
        reserve_exact(&mut bitmap, BMP_BITMAP_BYTES, refused)?;
        bitmap.resize(BMP_BITMAP_BYTES, 0);
        Ok(bitmap)
    }

    /// Embed the subset of glyphs drawn with `handle` from the same font.
    ///
    /// Call once per added font after its last draw. `font` must read the
    /// same unchanged source. Selected outlines are read again by range and
    /// Flate-compressed; no font program is held in memory. Planning only
    /// reads, so a failure there can be retried; a failure after emission
    /// starts poisons the document.
    pub async fn embed_font<S: RangedSource>(
        &mut self,
        handle: &FontObject,
        font: &mut TrueTypeFont<'_, S>,
    ) -> Result<()> {
        self.ensure_image_page_intact()?;
        self.writer.ensure_idle()?;
        if handle.document_id != self.document_id {
            return Err(Error::InvalidInput {
                reason: "PDF font belongs to another document",
            });
        }
        let pending = &self.fonts[handle.slot];
        if pending.embedded {
            return Err(Error::InvalidInput {
                reason: "PDF font subset is already embedded",
            });
        }
        if font.fingerprint() != pending.fingerprint {
            return Err(changed());
        }
        let [
            file,
            file_length,
            mapping,
            mapping_length,
            descriptor,
            descendant,
            object,
        ] = pending.ids;
        let before = font.subset_bytes_read();
        let plan = font
            .plan_subset(
                &pending.used,
                MAX_PDF_INTEGER,
                self.limits,
                self.cancellation,
            )
            .await?;
        self.count_font_input(font.subset_bytes_read() - before)?;
        let pending = &self.fonts[handle.slot];
        let name = format!("{}+{}", plan.tag(&pending.name), pending.name);
        let mut deflate = Deflate::new(self.limits)?;
        let before = font.subset_bytes_read();
        self.image_page_failed = true;
        let unicode = self.unicode_cmap(&mut deflate).await?;
        self.writer
            .begin_stream(
                file,
                file_length,
                format!("/Length1 {}\n/Filter /FlateDecode", plan.length()).as_bytes(),
            )
            .await?;
        let mut stream = FontStream::new(&mut self.writer, &mut deflate);
        font.write_subset(&plan, &mut stream, self.limits, self.cancellation)
            .await?;
        stream.finish().await?;
        self.count_font_input(font.subset_bytes_read() - before)?;

        let face = font.face()?;
        self.writer
            .begin_stream(mapping, mapping_length, b"/Filter /FlateDecode")
            .await?;
        let mut stream = FontStream::new(&mut self.writer, &mut deflate);
        let used = &self.fonts[handle.slot].used;
        // CID-to-glyph entries up to the highest drawn CID.
        let last = (0..65536).rev().find(|code| has_code(used, *code));
        for code in last.map_or(0..0, |last| 0..last + 1) {
            let glyph = char::from_u32(code as u32)
                .filter(|_| has_code(used, code))
                .map_or(0, |character| plan.glyph(&face, character));
            stream.put(&glyph.to_be_bytes()).await?;
        }
        stream.finish().await?;

        let scale = 1000.0 / f64::from(face.units_per_em());
        let bbox = face.global_bounding_box();
        let flags = 4 | u32::from(face.is_monospaced()) | if face.is_italic() { 64 } else { 0 };
        let desc = format!(
            "<< /Type /FontDescriptor /FontName /{name} /Flags {flags} /FontBBox [{} {} {} {}] /ItalicAngle {} /Ascent {} /Descent {} /CapHeight {} /StemV 0 /FontFile2 {} 0 R >>",
            f64::from(bbox.x_min) * scale,
            f64::from(bbox.y_min) * scale,
            f64::from(bbox.x_max) * scale,
            f64::from(bbox.y_max) * scale,
            face.italic_angle(),
            f64::from(face.ascender()) * scale,
            f64::from(face.descender()) * scale,
            f64::from(face.capital_height().unwrap_or(face.ascender())) * scale,
            file.number()
        );
        self.writer
            .write_object(descriptor, desc.as_bytes())
            .await?;
        self.writer.begin_object(descendant).await?;
        self.writer.write_bytes(format!("<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{name} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {} 0 R /CIDToGIDMap {} 0 R /DW 1000 /W [", descriptor.number(), mapping.number()).as_bytes()).await?;
        // One width array per 256-character block that contains drawn
        // characters, spanning its first to last drawn character. Both the
        // outer and inner arrays stay below PDF's recommended size limit.
        let used = &self.fonts[handle.slot].used;
        for first in (0..65536).step_by(256) {
            let mut drawn = (first..first + 256).filter(|code| has_code(used, *code));
            let Some(low) = drawn.next() else {
                continue;
            };
            let high = drawn.next_back().unwrap_or(low);
            let mut widths = format!(" {low} [");
            for code in low..=high {
                // Undrawn characters inside the span are never shown.
                let glyph = has_code(used, code)
                    .then(|| bmp_glyph(&face, code as u32))
                    .transpose()?
                    .flatten();
                let width = glyph.map_or(1000.0, |glyph| f64::from(glyph.advance) * scale);
                widths += &format!(" {width}");
            }
            widths.push_str(" ]");
            self.writer.write_bytes(widths.as_bytes()).await?;
        }
        self.writer.write_bytes(b" ] >>").await?;
        self.writer.end_object().await?;
        self.writer.write_object(object, format!("<< /Type /Font /Subtype /Type0 /BaseFont /{name} /Encoding /Identity-H /DescendantFonts [{} 0 R] /ToUnicode {} 0 R >>", descendant.number(), unicode.number()).as_bytes()).await?;
        self.fonts[handle.slot].embedded = true;
        self.image_page_failed = false;
        Ok(())
    }

    /// The document's identity ToUnicode CMap for BMP CIDs, written once
    /// and shared by every font.
    async fn unicode_cmap(&mut self, deflate: &mut Deflate) -> Result<ObjectId> {
        if let Some(unicode) = self.to_unicode {
            return Ok(unicode);
        }
        self.writer.prepare_objects(2)?;
        let unicode = self.writer.reserve_object()?;
        let length = self.writer.reserve_object()?;
        self.writer
            .begin_stream(unicode, length, b"/Filter /FlateDecode")
            .await?;
        let mut stream = FontStream::new(&mut self.writer, deflate);
        let mut cmap = String::from(
            "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /CajUnicode def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
        );
        // Each range increments only the final byte; UTF-16 surrogates are
        // excluded. At most 32 entries per block (the CMap limit is 100).
        for first in (0..256_u32).step_by(32) {
            let blocks = (first..first + 32).filter(|high| !(0xd8..=0xdf).contains(high));
            cmap += &format!("{} beginbfrange\n", blocks.clone().count());
            for high in blocks {
                cmap += &format!("<{high:02X}00> <{high:02X}FF> <{high:02X}00>\n");
            }
            cmap += "endbfrange\n";
        }
        cmap += "endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n";
        stream.put(cmap.as_bytes()).await?;
        stream.finish().await?;
        self.to_unicode = Some(unicode);
        Ok(unicode)
    }

    /// Begin an incrementally drawn page with previously emitted resources.
    ///
    /// Resources are borrowed and bounded to 128 fonts and 8192 images. Draws
    /// are awaited in source order; no page-sized content list is retained.
    /// An abandoned or failed page prevents finishing the document.
    pub async fn begin_content_page<'d, 'r>(
        &'d mut self,
        page: PageSpec,
        fonts: &'r [&'r FontObject],
        images: &'r [ImageObject],
    ) -> Result<ContentPageWriter<'d, 'a, 'r, W, C>> {
        self.ensure_image_page_intact()?;
        self.writer.ensure_idle()?;
        let width = pdf_page_number(page.width_points)?;
        let height = pdf_page_number(page.height_points)?;
        if fonts.len() > MAX_PAGE_FONTS || images.len() > MAX_PAGE_IMAGE_PLACEMENTS {
            return Err(Error::InvalidInput {
                reason: "too many resources for a PDF content page",
            });
        }
        for font in fonts {
            if font.document_id != self.document_id {
                return Err(Error::InvalidInput {
                    reason: "PDF font belongs to another document",
                });
            }
            // Its glyph set is final once the subset is written.
            if self.fonts[font.slot].embedded {
                return Err(Error::InvalidInput {
                    reason: "PDF font subset is already embedded",
                });
            }
        }
        for image in images {
            self.check_image_owner(*image)?;
        }
        self.check_next_page()?;
        self.reserve_page_index_slot()?;
        self.prepare_page_objects()?;
        self.image_page_failed = true;
        self.ensure_leaf().await?;
        let content = self.writer.reserve_object()?;
        let length = self.writer.reserve_object()?;
        let page = self.writer.reserve_object()?;
        self.writer.begin_stream(content, length, b"").await?;
        Ok(ContentPageWriter {
            document: self,
            fonts,
            images,
            width,
            height,
            content,
            page,
            failed: false,
        })
    }
}

fn bmp_glyph(
    face: &xberg_ttf_parser::Face<'_>,
    code: u32,
) -> Result<Option<crate::pdf::FontGlyph>> {
    let Some(character) = char::from_u32(code) else {
        return Ok(None);
    };
    let Some(id) = face.glyph_index(character).filter(|id| id.0 != 0) else {
        return Ok(None);
    };
    if id.0 >= face.number_of_glyphs() {
        return Err(Error::InvalidInput {
            reason: "font character map references an invalid glyph",
        });
    }
    let advance = face.glyph_hor_advance(id).ok_or(Error::InvalidInput {
        reason: "font glyph has no horizontal advance",
    })?;
    Ok(Some(crate::pdf::FontGlyph { id: id.0, advance }))
}

/// One open mixed-content page. Call `finish` before using its document again.
pub struct ContentPageWriter<'d, 'a, 'r, W: SequentialSink, C: Cancellation> {
    document: &'d mut PdfDocument<'a, W, C>,
    fonts: &'r [&'r FontObject],
    images: &'r [ImageObject],
    width: String,
    height: String,
    content: ObjectId,
    page: ObjectId,
    failed: bool,
}

impl<W: SequentialSink, C: Cancellation> ContentPageWriter<'_, '_, '_, W, C> {
    fn start_draw(&mut self) -> Result<()> {
        if self.failed {
            return Err(Error::InvalidInput {
                reason: "PDF content page cannot continue after a failed draw",
            });
        }
        self.failed = true;
        Ok(())
    }

    /// Draw one BMP character using a font resource index and text matrix.
    /// Font size is one; matrix scales are in PDF points per em. For example,
    /// `[12, 0, 0, 12, x, y]` draws an upright 12-point glyph at its baseline.
    pub async fn glyph(&mut self, font: usize, character: char, transform: [f64; 6]) -> Result<()> {
        self.draw_glyph(font, character, transform, None, None, false)
            .await
    }

    /// Draw a glyph in DeviceGray: zero is black and 255 is white.
    /// The color is local to this glyph; later content retains its prior color.
    pub async fn glyph_with_gray(
        &mut self,
        font: usize,
        character: char,
        transform: [f64; 6],
        gray: u8,
    ) -> Result<()> {
        self.draw_glyph(font, character, transform, Some(gray), None, false)
            .await
    }

    /// Draw one glyph clipped to `[left, bottom, width, height]` in PDF points.
    /// Extents must be positive. Clipping is local to this draw and does not
    /// affect later content. This retains the ordinary glyph's Unicode mapping;
    /// it does not by itself mark a font alias as nonsemantic decoration.
    pub async fn glyph_with_clip(
        &mut self,
        font: usize,
        character: char,
        transform: [f64; 6],
        clip: [f64; 4],
    ) -> Result<()> {
        self.draw_glyph(font, character, transform, None, Some(clip), false)
            .await
    }

    /// Draw a clipped decorative font glyph without semantic replacement text.
    /// `character` selects a glyph from the font's map, not document text.
    /// The draw is an Artifact containing a Span with empty ActualText. This
    /// preserves visible outlines while excluding the alias from extractors
    /// that honor ActualText. It does not make the document PDF/UA conformant.
    pub async fn decoration_glyph(
        &mut self,
        font: usize,
        character: char,
        transform: [f64; 6],
        clip: [f64; 4],
    ) -> Result<()> {
        self.draw_glyph(font, character, transform, None, Some(clip), true)
            .await
    }

    async fn draw_glyph(
        &mut self,
        font: usize,
        character: char,
        transform: [f64; 6],
        gray: Option<u8>,
        clip: Option<[f64; 4]>,
        decorative: bool,
    ) -> Result<()> {
        self.start_draw()?;
        let resource = self.fonts.get(font).ok_or(Error::InvalidInput {
            reason: "PDF page font index is out of range",
        })?;
        if !resource.supports(character) {
            return Err(Error::InvalidInput {
                reason: "PDF font has no supported BMP glyph for the character",
            });
        }
        mark_code(
            &mut self.document.fonts[resource.slot].used,
            character as usize,
        );
        let matrix = DecimalMatrix::new(transform)?;
        if gray.is_some() || clip.is_some() {
            self.document.writer.write_stream_bytes(b"q ").await?;
        }
        if let Some([left, bottom, width, height]) = clip {
            DecimalMatrix::new([left, bottom, width, height, left + width, bottom + height])?;
            if width <= 0.0 || height <= 0.0 {
                return Err(Error::InvalidInput {
                    reason: "PDF glyph clipping extents must be positive",
                });
            }
            self.document
                .writer
                .write_stream_bytes(format!("{left} {bottom} {width} {height} re W n\n").as_bytes())
                .await?;
        }
        if let Some(gray) = gray {
            self.document
                .writer
                .write_stream_bytes(format!("{:.6} g\n", f64::from(gray) / 255.0).as_bytes())
                .await?;
        }
        if decorative {
            self.document
                .writer
                .write_stream_bytes(b"/Artifact BMC\n/Span << /ActualText () >> BDC\n")
                .await?;
        }
        self.document
            .writer
            .write_stream_bytes(format!("BT /F{font} 1 Tf\n").as_bytes())
            .await?;
        self.document
            .writer
            .write_stream_bytes(matrix.as_bytes())
            .await?;
        self.document
            .writer
            .write_stream_bytes(format!(" Tm <{:04X}> Tj ET\n", character as u32).as_bytes())
            .await?;
        if decorative {
            self.document
                .writer
                .write_stream_bytes(b"EMC\nEMC\n")
                .await?;
        }
        if gray.is_some() || clip.is_some() {
            self.document.writer.write_stream_bytes(b"Q\n").await?;
        }
        self.failed = false;
        Ok(())
    }

    pub async fn image(&mut self, index: usize, transform: [f64; 6]) -> Result<()> {
        self.start_draw()?;
        if index >= self.images.len() {
            return Err(Error::InvalidInput {
                reason: "PDF page image index is out of range",
            });
        }
        let matrix = DecimalMatrix::new(transform)?;
        self.document.writer.write_stream_bytes(b"q\n").await?;
        self.document
            .writer
            .write_stream_bytes(matrix.as_bytes())
            .await?;
        self.document
            .writer
            .write_stream_bytes(format!(" cm /Im{index} Do Q\n").as_bytes())
            .await?;
        self.failed = false;
        Ok(())
    }

    /// Stroke one black segment in page coordinates. Zero width is PDF's
    /// device-dependent hairline; negative or nonfinite widths are rejected.
    pub async fn segment(&mut self, from: [f64; 2], to: [f64; 2], width: f64) -> Result<()> {
        self.start_draw()?;
        DecimalMatrix::new([from[0], from[1], to[0], to[1], width, 0.0])?;
        if width < 0.0 {
            return Err(Error::InvalidInput {
                reason: "PDF stroke width must be nonnegative",
            });
        }
        let mut command = DecimalMatrix {
            bytes: [0; MATRIX_TEXT_BYTES],
            length: 0,
        };
        let overflow = Error::InvalidInput {
            reason: "PDF segment exceeds fixed scratch capacity",
        };
        writeln!(
            command,
            "q {width} w {} {} m {} {} l S Q",
            from[0], from[1], to[0], to[1]
        )
        .map_err(|_| overflow)?;
        self.document
            .writer
            .write_stream_bytes(command.as_bytes())
            .await?;
        self.failed = false;
        Ok(())
    }

    /// Stroke a bounded continuous path with butt caps and miter joins.
    /// Fixed decimal scratch and sequential writes avoid retaining page paths.
    pub(crate) async fn stroke_polyline(
        &mut self,
        points: &[[f64; 2]],
        width: f64,
        gray: u8,
    ) -> Result<()> {
        self.start_draw()?;
        if !(2..=8).contains(&points.len()) || width < 0.0 {
            return Err(Error::InvalidInput {
                reason: "PDF polyline requires two to eight points and nonnegative width",
            });
        }
        let mut command = DecimalMatrix {
            bytes: [0; MATRIX_TEXT_BYTES],
            length: 0,
        };
        command.push(width)?;
        self.document
            .writer
            .write_stream_bytes(b"q 0 J 0 j 10 M ")
            .await?;
        self.document
            .writer
            .write_stream_bytes(command.as_bytes())
            .await?;
        self.document
            .writer
            .write_stream_bytes(format!(" w {:.6} G\n", f64::from(gray) / 255.0).as_bytes())
            .await?;
        for (index, point) in points.iter().enumerate() {
            let mut command = DecimalMatrix {
                bytes: [0; MATRIX_TEXT_BYTES],
                length: 0,
            };
            command.push(point[0])?;
            command.push(point[1])?;
            self.document
                .writer
                .write_stream_bytes(command.as_bytes())
                .await?;
            let operator = if index == 0 { b" m\n" } else { b" l\n" };
            self.document.writer.write_stream_bytes(operator).await?;
        }
        self.document.writer.write_stream_bytes(b"S Q\n").await?;
        self.failed = false;
        Ok(())
    }

    /// Fill a closed black polygon using PDF's nonzero winding rule.
    /// Three to eight vertices cover small native decorations without retaining
    /// an unbounded path. Points are streamed through fixed decimal scratch;
    /// failure or cancellation invalidates this content page.
    pub async fn fill_polygon(&mut self, points: &[[f64; 2]]) -> Result<()> {
        self.start_draw()?;
        if !(3..=8).contains(&points.len()) {
            return Err(Error::InvalidInput {
                reason: "PDF polygon must have three to eight vertices",
            });
        }
        self.document.writer.write_stream_bytes(b"q 0 g\n").await?;
        for (index, point) in points.iter().enumerate() {
            let mut command = DecimalMatrix {
                bytes: [0; MATRIX_TEXT_BYTES],
                length: 0,
            };
            command.push(point[0])?;
            command.push(point[1])?;
            self.document
                .writer
                .write_stream_bytes(command.as_bytes())
                .await?;
            let operator = if index == 0 { b" m\n" } else { b" l\n" };
            self.document.writer.write_stream_bytes(operator).await?;
        }
        self.document.writer.write_stream_bytes(b"h f Q\n").await?;
        self.failed = false;
        Ok(())
    }

    pub async fn finish(self) -> Result<u32> {
        if self.failed {
            return Err(Error::InvalidInput {
                reason: "PDF content page cannot finish after a failed draw",
            });
        }
        let document = self.document;
        document.writer.end_stream().await?;
        let parent = document
            .leaf
            .as_ref()
            .ok_or(Error::InvalidInput {
                reason: "PDF page-tree leaf is missing",
            })?
            .id;
        document.writer.begin_object(self.page).await?;
        document
            .writer
            .write_bytes(
                format!(
                    "<< /Type /Page /Parent {} 0 R /MediaBox [0 0 {} {}] /Resources << /Font <<",
                    parent.number(),
                    self.width,
                    self.height
                )
                .as_bytes(),
            )
            .await?;
        for (index, font) in self.fonts.iter().enumerate() {
            document
                .writer
                .write_bytes(format!(" /F{index} {} 0 R", font.object.number()).as_bytes())
                .await?;
        }
        document.writer.write_bytes(b" >> /XObject <<").await?;
        for (index, image) in self.images.iter().enumerate() {
            document
                .writer
                .write_bytes(format!(" /Im{index} {} 0 R", image.object.number()).as_bytes())
                .await?;
        }
        document
            .writer
            .write_bytes(format!(" >> >> /Contents {} 0 R >>", self.content.number()).as_bytes())
            .await?;
        document.writer.end_object().await?;
        let index = document.register_page(self.page)?;
        document.image_page_failed = false;
        Ok(index)
    }
}

#[cfg(test)]
mod tests;
