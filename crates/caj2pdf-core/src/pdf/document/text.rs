// SPDX-License-Identifier: MIT

//! Embedded TrueType resources and incremental mixed-page content.

use super::*;
use crate::pdf::TrueTypeFont;

const BMP_BITMAP_BYTES: usize = 8192;
const MAX_PAGE_FONTS: usize = 128;

/// A completed embedded font, usable only in the document that created it.
///
/// A fixed 8 KiB bitmap records which BMP Unicode characters can be drawn.
/// The font program and metric tables are not retained by this handle.
pub struct FontObject {
    object: ObjectId,
    document_id: usize,
    characters: Vec<u8>,
}

impl FontObject {
    pub fn supports(&self, character: char) -> bool {
        let code = character as usize;
        code <= 0xffff && self.characters[code / 8] & (1 << (code % 8)) != 0
    }
}

impl<'a, W: SequentialSink, C: Cancellation> PdfDocument<'a, W, C> {
    /// Embed a complete static TrueType font through ranged reads.
    ///
    /// The initial profile uses BMP Unicode character codes with explicit
    /// CID-to-glyph and ToUnicode maps. No font subsetting, system lookup or
    /// glyph substitution is performed. Input must remain stable throughout
    /// metadata reading and embedding. The caller supplies a valid font;
    /// metadata validation is not a sanitizer for every embedded outline.
    /// A failure after emission starts poisons the document.
    pub async fn add_font<S: RangedSource>(
        &mut self,
        font: &mut TrueTypeFont<'_, S>,
    ) -> Result<FontObject> {
        self.ensure_image_page_intact()?;
        self.writer.ensure_idle()?;
        let name = font.postscript_name()?.replace('#', "#23");
        let length = font.source_bytes();
        if length > MAX_PDF_INTEGER {
            return Err(Error::LimitExceeded {
                resource: "PDF font program bytes",
                limit: MAX_PDF_INTEGER,
                attempted: length,
            });
        }
        let total = self
            .input_bytes_read
            .checked_add(length)
            .ok_or(Error::InvalidInput {
                reason: "font input byte count overflows",
            })?;
        self.limits.check_input_size(total)?;
        self.limits.check_allocation(BMP_BITMAP_BYTES as u64)?;
        let mut characters = Vec::new();
        let refused = self
            .limits
            .allocation_refused("PDF font character bitmap", BMP_BITMAP_BYTES as u64);
        reserve_exact(&mut characters, BMP_BITMAP_BYTES, refused)?;
        characters.resize(BMP_BITMAP_BYTES, 0);
        self.writer.prepare_objects(9)?;
        self.image_page_failed = true;
        let file = self.writer.reserve_object()?;
        let file_length = self.writer.reserve_object()?;
        let mapping = self.writer.reserve_object()?;
        let mapping_length = self.writer.reserve_object()?;
        let unicode = self.writer.reserve_object()?;
        let unicode_length = self.writer.reserve_object()?;
        let descriptor = self.writer.reserve_object()?;
        let descendant = self.writer.reserve_object()?;
        let object = self.writer.reserve_object()?;

        self.writer
            .begin_stream(file, file_length, format!("/Length1 {length}").as_bytes())
            .await?;
        self.copy_resource(font.source, 0, length).await?;
        self.writer.end_stream().await?;

        let face = font.face()?;
        self.writer
            .begin_stream(mapping, mapping_length, b"")
            .await?;
        for first in (0..65536_u32).step_by(256) {
            let mut bytes = [0; 512];
            for delta in 0..256_u32 {
                let code = first + delta;
                if let Some(glyph) = bmp_glyph(&face, code)? {
                    bytes[delta as usize * 2..delta as usize * 2 + 2]
                        .copy_from_slice(&glyph.id.to_be_bytes());
                    characters[code as usize / 8] |= 1 << (code % 8);
                }
            }
            self.writer.write_stream_bytes(&bytes).await?;
        }
        self.writer.end_stream().await?;
        self.writer
            .begin_stream(unicode, unicode_length, b"")
            .await?;
        self.writer.write_stream_bytes(b"/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /CajUnicode def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n").await?;
        // Each range increments only the final byte; UTF-16 surrogates are
        // excluded. At most 32 entries per block (the CMap limit is 100).
        for first in (0..256_u32).step_by(32) {
            let blocks = (first..first + 32).filter(|high| !(0xd8..=0xdf).contains(high));
            self.writer
                .write_stream_bytes(format!("{} beginbfrange\n", blocks.clone().count()).as_bytes())
                .await?;
            for high in blocks {
                self.writer
                    .write_stream_bytes(
                        format!("<{high:02X}00> <{high:02X}FF> <{high:02X}00>\n").as_bytes(),
                    )
                    .await?;
            }
            self.writer.write_stream_bytes(b"endbfrange\n").await?;
        }
        self.writer
            .write_stream_bytes(
                b"endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n",
            )
            .await?;
        self.writer.end_stream().await?;

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
        // Keep both the outer /W array and each inner width array below
        // PDF's recommended array-size limit, even for dense CJK fonts.
        for first in (0..65536_u32).step_by(256) {
            if characters[first as usize / 8..first as usize / 8 + 32]
                .iter()
                .all(|byte| *byte == 0)
            {
                continue;
            }
            self.writer
                .write_bytes(format!(" {first} [").as_bytes())
                .await?;
            for code in first..first + 256 {
                let width = bmp_glyph(&face, code)?
                    .map_or(1000.0, |glyph| f64::from(glyph.advance) * scale);
                self.writer
                    .write_bytes(format!(" {width}").as_bytes())
                    .await?;
            }
            self.writer.write_bytes(b" ]").await?;
        }
        self.writer.write_bytes(b" ] >>").await?;
        self.writer.end_object().await?;
        self.writer.write_object(object, format!("<< /Type /Font /Subtype /Type0 /BaseFont /{name} /Encoding /Identity-H /DescendantFonts [{} 0 R] /ToUnicode {} 0 R >>", descendant.number(), unicode.number()).as_bytes()).await?;
        self.image_page_failed = false;
        Ok(FontObject {
            object,
            document_id: self.document_id,
            characters,
        })
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
        self.draw_glyph(font, character, transform, None).await
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
        self.draw_glyph(font, character, transform, Some(gray))
            .await
    }

    async fn draw_glyph(
        &mut self,
        font: usize,
        character: char,
        transform: [f64; 6],
        gray: Option<u8>,
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
        let matrix = DecimalMatrix::new(transform)?;
        if let Some(gray) = gray {
            self.document
                .writer
                .write_stream_bytes(format!("q {:.6} g\n", f64::from(gray) / 255.0).as_bytes())
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
        if gray.is_some() {
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
