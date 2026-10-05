// SPDX-License-Identifier: MIT

//! Ranged metadata access for explicitly supplied static TrueType fonts.

mod subset;
pub(crate) use subset::SubsetOutput;

use crate::fallible::reserve_exact;
use crate::{Cancellation, Error, Limits, RangedSource, Result, read_exact_at};
use sha2::Digest;
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
/// `glyf` and `loca` are required; the hinting tables are optional.
const OUTLINE_TAGS: [[u8; 4]; 5] = [*b"glyf", *b"loca", *b"cvt ", *b"fpgm", *b"prep"];

/// A Unicode glyph's ID and horizontal advance in font units.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FontGlyph {
    pub id: u16,
    pub advance: u16,
}

/// Metadata and a borrowed source for one standalone static TrueType font.
///
/// Only small metric/character tables are retained; outline bytes remain in
/// the ranged source and only drawn glyphs are later read again for a
/// subset. This validates metadata, not every glyph outline. Font
/// collections, CFF and variable fonts are outside this initial profile.
pub struct TrueTypeFont<'a, S> {
    pub(super) source: &'a mut S,
    tables: [Vec<u8>; 8],
    /// `(offset, length)` of each present [`OUTLINE_TAGS`] table.
    outlines: [Option<(u64, u64)>; 5],
    /// Outline bytes read again for subsets, counted toward input limits.
    subset_bytes_read: u64,
}

impl<'a, S: RangedSource> TrueTypeFont<'a, S> {
    pub async fn read<C: Cancellation>(
        source: &'a mut S,
        limits: &Limits,
        cancellation: &C,
    ) -> Result<Self> {
        limits.validate()?;
        limits.check_input_size(source.size())?;
        let mut header = [0; 12];
        read(source, 0, &mut header, limits, cancellation).await?;
        if header[..4] != [0, 1, 0, 0] {
            return Err(invalid("font must be a standalone TrueType SFNT"));
        }
        let count = usize::from(u16::from_be_bytes([header[4], header[5]]));
        if count == 0 || count > MAX_TABLES {
            return Err(invalid("unsupported TrueType table count"));
        }
        let directory_end = 12 + 16 * count as u64;
        let mut directory = [[0_u8; 16]; MAX_TABLES];
        let mut metadata_bytes = 0_u64;
        let mut outlines = [None; 5];
        for i in 0..count {
            read(
                source,
                12 + 16 * i as u64,
                &mut directory[i],
                limits,
                cancellation,
            )
            .await?;
            let entry = directory[i];
            let tag = &entry[..4];
            if i != 0 && directory[i - 1][..4] >= *tag {
                return Err(invalid("TrueType table tags must be unique and sorted"));
            }
            if tag == b"fvar" || tag == b"CFF " || tag == b"CFF2" {
                return Err(invalid("only static TrueType outlines are supported"));
            }
            let (offset, length) = span(&entry);
            if offset < directory_end || offset % 4 != 0 || offset + length > source.size() {
                return Err(invalid(
                    "TrueType table range is outside the source or unaligned",
                ));
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
        if outlines[0].is_none() || outlines[1].is_none() {
            return Err(invalid("TrueType outline tables are missing"));
        }
        if metadata_bytes > MAX_FONT_METADATA_BYTES {
            return Err(Error::LimitExceeded {
                resource: "font metadata bytes",
                limit: MAX_FONT_METADATA_BYTES,
                attempted: metadata_bytes,
            });
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
            read(source, offset, table, limits, cancellation).await?;
        }
        let font = Self {
            source,
            tables,
            outlines,
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
            return Err(Error::LimitExceeded {
                resource: "font character maps",
                limit: 16,
                attempted: u64::from(maps),
            });
        }
        if matches!(
            face.permissions(),
            None | Some(xberg_ttf_parser::Permissions::Restricted)
        ) || !face.is_outline_embedding_allowed()
        {
            return Err(invalid("font metadata does not permit outline embedding"));
        }
        Ok(font)
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

    /// Digest of the retained metadata and outline table ranges, identifying
    /// the font when its source is read again.
    pub(crate) fn fingerprint(&self) -> [u8; 32] {
        let mut hash = sha2::Sha256::new();
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

    pub(super) fn postscript_name(&self) -> Result<String> {
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

fn span(entry: &[u8; 16]) -> (u64, u64) {
    (
        u64::from(u32::from_be_bytes(entry[8..12].try_into().unwrap())),
        u64::from(u32::from_be_bytes(entry[12..16].try_into().unwrap())),
    )
}

fn invalid(reason: &'static str) -> Error {
    Error::InvalidInput { reason }
}

async fn read<S: RangedSource, C: Cancellation>(
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
        )
        .await?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests;
