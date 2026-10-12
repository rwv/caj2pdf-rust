// SPDX-License-Identifier: MIT

use super::C8_MAX_FONTS;
use crate::{Error, Limits, Result};

/// Maximum ordinary glyph aliases: one per BMP character for each resource.
pub const MAX_NATIVE_FONT_GLYPHS: usize = C8_MAX_FONTS * 65536;

/// An ordinary native-text character and the caller font's original cmap entry.
///
/// `font` indexes the supplied sources (or handles for the page writer).
/// Both characters must be BMP scalars. Entries must be strictly sorted by
/// `(font, character)`; duplicate keys are refused. The source program is not
/// modified, and extracted text retains the original decoded character.
/// Explicit symbol-code maps and nonsemantic decorations bypass these aliases.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeFontGlyphAlias {
    pub font: usize,
    pub character: char,
    pub glyph: char,
}

pub(crate) fn validate(
    aliases: &[NativeFontGlyphAlias],
    fonts: usize,
    limits: &Limits,
) -> Result<()> {
    if aliases.len() > MAX_NATIVE_FONT_GLYPHS {
        return Err(Error::limit(
            "native font glyph aliases",
            MAX_NATIVE_FONT_GLYPHS as u64,
            aliases.len() as u64,
        ));
    }
    limits.check_allocation(std::mem::size_of_val(aliases) as u64)?;
    let mut previous = None;
    for entry in aliases {
        let key = (entry.font, entry.character);
        if entry.font >= fonts
            || entry.character > '\u{ffff}'
            || entry.glyph > '\u{ffff}'
            || previous.is_some_and(|previous| previous >= key)
        {
            return Err(Error::invalid(
                "font glyph aliases require valid font indices, BMP characters and strictly sorted unique keys",
            ));
        }
        previous = Some(key);
    }
    Ok(())
}

pub(crate) fn for_font(aliases: &[NativeFontGlyphAlias], font: usize) -> &[NativeFontGlyphAlias] {
    let start = aliases.partition_point(|entry| entry.font < font);
    let end = aliases.partition_point(|entry| entry.font <= font);
    &aliases[start..end]
}

pub(crate) fn lookup(
    aliases: &[NativeFontGlyphAlias],
    font: usize,
    character: char,
) -> Option<char> {
    aliases
        .binary_search_by_key(&(font, character), |entry| (entry.font, entry.character))
        .ok()
        .map(|index| aliases[index].glyph)
}
