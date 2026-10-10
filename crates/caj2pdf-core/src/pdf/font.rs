// SPDX-License-Identifier: MIT

//! Ranged metadata access for explicitly supplied static OpenType fonts:
//! TrueType or CFF outlines, standalone or in a collection.

mod cff;
mod encoding;
mod subset;
pub(crate) use encoding::{CharacterMap, Characters, GlyphCharacters};
pub(crate) use subset::{Subset, SubsetOutput};

use crate::fallible::reserve_exact;
use crate::{Cancellation, Error, Limits, RangedSource, Result, read_exact_at};
use sha2::Digest;
use std::rc::Rc;
use xberg_ttf_parser::{Face, RawFaceTables};

const MAX_TABLES: usize = 128;
/// Maximum combined retained font metadata, independent of outline size.
pub const MAX_FONT_METADATA_BYTES: u64 = 1024 * 1024;
const METADATA_TAGS: [[u8; 4]; 8] = [
    *b"head", *b"hhea", *b"maxp", *b"cmap", *b"hmtx", *b"OS/2", *b"post", *b"name",
];
/// Error reason when a font read again differs from its first read.
pub(crate) const CHANGED: &str = "font source changed after its metadata was read";

/// Whether BMP `code` is set in a 65,536-bit character bitmap.
pub(crate) fn has_code(bitmap: &[u8], code: usize) -> bool {
    bitmap[code / 8] & (1 << (code % 8)) != 0
}

/// Set BMP `code` in a 65,536-bit character bitmap.
pub(crate) fn mark_code(bitmap: &mut [u8], code: usize) {
    bitmap[code / 8] |= 1 << (code % 8);
}

/// Tables whose bytes stay in the source and are read again when subsetting.
/// A font has either `glyf` and `loca` (with optional hinting tables) or
/// `CFF `.
const OUTLINE_TAGS: [[u8; 4]; 6] = [*b"glyf", *b"loca", *b"cvt ", *b"fpgm", *b"prep", *b"CFF "];
const CFF: usize = 5;

/// A Unicode glyph's ID and horizontal advance in font units.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FontGlyph {
    pub id: u16,
    pub advance: u16,
}

/// Whether an OS/2 table of `version` with `fs_type` permits embedding a
/// subset of the outlines. When a font sets several licensing bits, the
/// least restrictive applies, as OS/2 versions 0-2 specify and readers
/// apply to later versions too: a restricted license (bit 1) alone forbids
/// embedding. A subset is always embedded, so from version 2, which defines
/// them, the no-subsetting (bit 8) and bitmap-only (bit 9) bits forbid it as
/// well; versions 0 and 1 reserve those bits.
fn permits_subset_embedding(version: u16, fs_type: u16) -> bool {
    let licensing = fs_type & 0xf;
    (licensing == 0 || licensing & 0xc != 0) && (version < 2 || fs_type & 0x300 == 0)
}

/// Metadata and a borrowed source for one face of a static OpenType font or
/// collection, with TrueType (`glyf`/`loca`) or CFF outlines.
///
/// Only small metric/character tables and the CFF structures are retained;
/// outline bytes remain in the ranged source and only drawn glyphs are later
/// read again for a subset. This validates metadata, not every glyph
/// outline. Variable fonts and `CFF2` are not supported.
pub struct OpenTypeFont<'a, S> {
    pub(super) source: &'a mut S,
    face: u32,
    tables: [Vec<u8>; 8],
    /// `(offset, length)` of each present [`OUTLINE_TAGS`] table.
    outlines: [Option<(u64, u64)>; 6],
    /// Parsed CFF structures of a CFF-flavoured OpenType font.
    cff: Option<Rc<cff::Cff>>,
    /// Outline bytes read again for subsets, counted toward input limits.
    subset_bytes_read: u64,
}

impl<'a, S: RangedSource> OpenTypeFont<'a, S> {
    /// Read face `face` of a TrueType font or TrueType collection (`ttcf`).
    /// A standalone font has only face 0. Collection table offsets are
    /// file-relative, so faces may share tables.
    pub fn read<C: Cancellation>(
        source: &'a mut S,
        face: u32,
        limits: &Limits,
        cancellation: &C,
    ) -> Result<Self> {
        limits.validate()?;
        limits.check_input_size(source.size())?;
        let mut header = [0; 12];
        read(source, 0, &mut header, limits, cancellation)?;
        // The collection header and this face's directory hold no tables.
        let (base, collection_end) = if let Some((faces, dsig)) = collection(&header)? {
            if face >= faces {
                return Err(invalid("TrueType collection face index is out of range"));
            }
            let mut offset = [0; 4];
            read(
                source,
                12 + 4 * u64::from(face),
                &mut offset,
                limits,
                cancellation,
            )?;
            let base = u64::from(u32::from_be_bytes(offset));
            read(source, base, &mut header, limits, cancellation)?;
            (base, 12 + 4 * u64::from(faces) + dsig)
        } else if face != 0 {
            return Err(invalid("a standalone font has only face 0"));
        } else {
            (0, 0)
        };
        if !is_font_tag(&header) {
            return Err(invalid("font must be an OpenType font or collection face"));
        }
        let count = usize::from(u16::from_be_bytes([header[4], header[5]]));
        if count == 0 || count > MAX_TABLES {
            return Err(invalid("unsupported TrueType table count"));
        }
        let directory_end = base + 12 + 16 * count as u64;
        let mut directory = [[0_u8; 16]; MAX_TABLES];
        let mut metadata_bytes = 0_u64;
        let mut outlines = [None; 6];
        for i in 0..count {
            read(
                source,
                base + 12 + 16 * i as u64,
                &mut directory[i],
                limits,
                cancellation,
            )?;
            let entry = directory[i];
            let tag = &entry[..4];
            if i != 0 && directory[i - 1][..4] >= *tag {
                return Err(invalid("TrueType table tags must be unique and sorted"));
            }
            if tag == b"fvar" || tag == b"CFF2" {
                return Err(invalid("variable fonts are not supported"));
            }
            let (offset, length) = span(&entry);
            // Tables should be 4-byte aligned, but installed fonts such as
            // WenQuanYi Zen Hei are not; readers accept any offset.
            if (offset < directory_end && base < offset + length.max(1))
                || offset < collection_end
                || offset + length > source.size()
            {
                return Err(invalid("TrueType table range is outside the source"));
            }
            for previous in &directory[..i] {
                let (start, len) = span(previous);
                if length != 0 && len != 0 && offset < start + len && start < offset + length {
                    return Err(invalid("TrueType table ranges overlap"));
                }
            }
            if let Some(slot) = OUTLINE_TAGS.iter().position(|wanted| wanted == tag) {
                outlines[slot] = Some((offset, length));
            }
            if METADATA_TAGS.iter().any(|wanted| wanted == tag) {
                metadata_bytes += length;
            }
        }
        let truetype = outlines[0].is_some() && outlines[1].is_some();
        if truetype == outlines[CFF].is_some() {
            return Err(invalid("font needs either glyf and loca or CFF outlines"));
        }
        if metadata_bytes > MAX_FONT_METADATA_BYTES {
            return Err(Error::limit(
                "font metadata bytes",
                MAX_FONT_METADATA_BYTES,
                metadata_bytes,
            ));
        }
        limits.check_allocation(metadata_bytes)?;
        let mut tables: [Vec<u8>; 8] = Default::default();
        for (tag, table) in METADATA_TAGS.iter().zip(&mut tables) {
            let entry = directory[..count]
                .iter()
                .find(|entry| entry[..4] == *tag)
                .ok_or(invalid("required TrueType metadata table is missing"))?;
            let (offset, length) = span(entry);
            let refused = limits.allocation_refused("font metadata bytes", length);
            reserve_exact(table, length as usize, refused)?;
            table.resize(length as usize, 0);
            read(source, offset, table, limits, cancellation)?;
        }
        let mut font = Self {
            source,
            face,
            tables,
            outlines,
            cff: None,
            subset_bytes_read: 0,
        };
        let face = font.face()?;
        if face.tables().cmap.is_none()
            || face.tables().hmtx.is_none()
            || face.tables().os2.is_none()
            || face.tables().post.is_none()
        {
            return Err(invalid("invalid required TrueType metadata table"));
        }
        // A BMP mapping pass performs a fixed number of character lookups.
        // Bound their per-character table search independently of byte size.
        let maps = face
            .tables()
            .cmap
            .as_ref()
            .map_or(0, |cmap| cmap.subtables.len());
        if maps > 16 {
            return Err(Error::limit("font character maps", 16, u64::from(maps)));
        }
        // `version` and `fsType` of the required OS/2 table.
        let field = |at: usize| {
            font.tables[5]
                .get(at..at + 2)
                .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
        };
        let permitted = field(0)
            .zip(field(8))
            .is_some_and(|(version, fs_type)| permits_subset_embedding(version, fs_type));
        if !permitted {
            return Err(invalid("font metadata does not permit subset embedding"));
        }
        if let Some(span) = font.outlines[CFF] {
            let glyphs = face.number_of_glyphs();
            font.cff = Some(Rc::new(cff::Cff::read(
                font.source,
                span,
                glyphs,
                limits,
                cancellation,
            )?));
        }
        Ok(font)
    }

    /// The number of faces in `source`: a collection's declared face count,
    /// or 1 for a standalone font. Only the 12-byte header is read, so the
    /// faces themselves are not validated; [`Self::read`] validates one.
    pub fn face_count<C: Cancellation>(
        source: &mut S,
        limits: &Limits,
        cancellation: &C,
    ) -> Result<u32> {
        limits.validate()?;
        limits.check_input_size(source.size())?;
        let mut header = [0; 12];
        read(source, 0, &mut header, limits, cancellation)?;
        match collection(&header)? {
            Some((faces, _)) => Ok(faces),
            None if is_font_tag(&header) => Ok(1),
            None => Err(invalid("font must be an OpenType font or collection face")),
        }
    }

    pub fn units_per_em(&self) -> Result<u16> {
        Ok(self.face()?.units_per_em())
    }

    /// Reject missing glyphs instead of silently selecting `.notdef`.
    pub fn glyph(&self, character: char) -> Result<FontGlyph> {
        let face = self.face()?;
        let id = face
            .glyph_index(character)
            .filter(|id| id.0 != 0 && id.0 < face.number_of_glyphs())
            .ok_or(invalid("font has no glyph for the requested character"))?;
        let advance = face
            .glyph_hor_advance(id)
            .ok_or(invalid("font glyph has no horizontal advance"))?;
        Ok(FontGlyph { id: id.0, advance })
    }

    pub(super) fn face(&self) -> Result<Face<'_>> {
        Face::from_raw_tables(RawFaceTables {
            head: &self.tables[0],
            hhea: &self.tables[1],
            maxp: &self.tables[2],
            cmap: Some(&self.tables[3]),
            hmtx: Some(&self.tables[4]),
            os2: Some(&self.tables[5]),
            post: Some(&self.tables[6]),
            name: Some(&self.tables[7]),
            ..Default::default()
        })
        .map_err(|_| invalid("invalid required TrueType face metadata"))
    }

    /// Whether the font has CFF rather than TrueType outlines.
    pub(crate) fn is_cff(&self) -> bool {
        self.cff.is_some()
    }

    /// Digest of the retained metadata and outline table ranges, identifying
    /// the font when its source is read again.
    pub(crate) fn fingerprint(&self) -> [u8; 32] {
        let mut hash = sha2::Sha256::new().chain_update(self.face.to_be_bytes());
        for table in &self.tables {
            hash.update((table.len() as u64).to_be_bytes());
            hash.update(table);
        }
        for (offset, length) in self.outlines.iter().flatten() {
            hash.update(offset.to_be_bytes());
            hash.update(length.to_be_bytes());
        }
        hash.finalize().into()
    }

    /// The `(glyph, CID)` of each used character, in CID order.
    /// A character the font no longer maps means its source
    /// changed after the metadata was read.
    pub(super) fn used_glyphs(
        &self,
        used: Characters<'_>,
        limits: &Limits,
    ) -> Result<Vec<(u16, u16)>> {
        let face = self.face()?;
        let count = face.number_of_glyphs();
        let length = used.codes().count();
        let bytes = length as u64 * size_of::<(u16, u16)>() as u64;
        limits.check_allocation(bytes)?;
        let mut glyphs = Vec::new();
        reserve_exact(
            &mut glyphs,
            length,
            limits.allocation_refused("font used glyphs", bytes),
        )?;
        for code in used.codes() {
            let glyph = used
                .glyph(usize::from(code))
                .and_then(|character| face.glyph_index(character))
                .filter(|id| id.0 != 0 && id.0 < count)
                .map(|id| (id.0, code))
                .ok_or(invalid(CHANGED))?;
            glyphs.push(glyph);
        }
        Ok(glyphs)
    }

    /// The face's PostScript name (name ID 6), validated as 1 to 63
    /// printable ASCII characters without PDF delimiters.
    pub fn postscript_name(&self) -> Result<String> {
        let face = self.face()?;
        let name = face
            .names()
            .into_iter()
            .find(|name| name.name_id == 6 && name.is_unicode())
            .ok_or(invalid("font requires a Unicode PostScript name"))?;
        if name.name.is_empty() || name.name.len() > 126 || name.name.len() % 2 != 0 {
            return Err(invalid(
                "font PostScript name must have 1 to 63 ASCII characters",
            ));
        }
        let name = name
            .to_string()
            .ok_or(invalid("font PostScript name is invalid UTF-16"))?;
        if !name
            .bytes()
            .all(|byte| (33..=126).contains(&byte) && !b"[](){}<>/%".contains(&byte))
        {
            return Err(invalid("font PostScript name contains invalid characters"));
        }
        Ok(name)
    }
}

/// The face count and DSIG header bytes of a TrueType collection header, or
/// `None` for any other font header.
fn collection(header: &[u8; 12]) -> Result<Option<(u32, u64)>> {
    if header[..4] != *b"ttcf" {
        return Ok(None);
    }
    // Version 2 adds three 32-bit DSIG fields to the header.
    let dsig = match u16::from_be_bytes([header[4], header[5]]) {
        1 => 0,
        2 => 12,
        _ => return Err(invalid("unsupported TrueType collection version")),
    };
    Ok(Some((
        u32::from_be_bytes(header[8..12].try_into().unwrap()),
        dsig,
    )))
}

/// Whether a font header starts with a TrueType or CFF OpenType tag.
fn is_font_tag(header: &[u8; 12]) -> bool {
    header[..4] == [0, 1, 0, 0] || header[..4] == *b"true" || header[..4] == *b"OTTO"
}

fn span(entry: &[u8; 16]) -> (u64, u64) {
    (
        u64::from(u32::from_be_bytes(entry[8..12].try_into().unwrap())),
        u64::from(u32::from_be_bytes(entry[12..16].try_into().unwrap())),
    )
}

fn invalid(reason: &'static str) -> Error {
    Error::invalid(reason)
}

fn read<S: RangedSource, C: Cancellation>(
    source: &mut S,
    offset: u64,
    bytes: &mut [u8],
    limits: &Limits,
    cancellation: &C,
) -> Result<()> {
    for (index, chunk) in bytes.chunks_mut(limits.io_chunk_bytes).enumerate() {
        read_exact_at(
            source,
            offset + (index * limits.io_chunk_bytes) as u64,
            chunk,
            limits,
            cancellation,
        )?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests;
