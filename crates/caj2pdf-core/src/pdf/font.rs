// SPDX-License-Identifier: MIT

//! Ranged metadata access for explicitly supplied static TrueType fonts.

use crate::fallible::reserve_exact;
use crate::{Cancellation, Error, Limits, RangedSource, Result, read_exact_at};
use xberg_ttf_parser::{Face, RawFaceTables};

const MAX_TABLES: usize = 128;
/// Maximum combined retained font metadata, independent of outline size.
pub const MAX_FONT_METADATA_BYTES: u64 = 1024 * 1024;
const METADATA_TAGS: [[u8; 4]; 8] = [
    *b"head", *b"hhea", *b"maxp", *b"cmap", *b"hmtx", *b"OS/2", *b"post", *b"name",
];

/// A Unicode glyph's ID and horizontal advance in font units.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FontGlyph {
    pub id: u16,
    pub advance: u16,
}

/// Metadata and a borrowed source for one standalone static TrueType font.
///
/// Only small metric/character tables are retained; outline bytes remain in
/// the ranged source. This validates metadata, not every glyph outline. Font
/// collections, CFF and variable fonts are outside this initial profile.
/// Loading this resource does not enable native C8 conversion by itself.
pub struct TrueTypeFont<'a, S> {
    pub(super) source: &'a mut S,
    tables: [Vec<u8>; 8],
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
        let mut has_glyf = false;
        let mut has_loca = false;
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
            has_glyf |= tag == b"glyf";
            has_loca |= tag == b"loca";
            if METADATA_TAGS.iter().any(|wanted| wanted == tag) {
                metadata_bytes += length;
            }
        }
        if !has_glyf || !has_loca {
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
        let font = Self { source, tables };
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

    /// Size of the original font program; its bytes have not been retained.
    pub fn source_bytes(&self) -> u64 {
        self.source.size()
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
