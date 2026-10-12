// SPDX-License-Identifier: MIT

//! Font registration: host font resources and the roles JavaScript assigns
//! them.

use super::{Host, HostSource};
use caj2pdf_core::{
    Fonts as FontResources, RangedSource,
    hnc8::{
        C8_MAX_FONTS, C8FontSource, C8PageFonts, MAX_NATIVE_FONT_GLYPHS, NativeDecorationGlyph,
        NativeFontGlyphAlias, NativeSymbolGlyph, SymbolFontIdentity,
    },
    pdf::FontEmbedding,
};
use std::cell::RefCell;

/// One entry per HN-B mode-0 symbol code; a longer map cannot be valid.
const MAX_SYMBOL_GLYPHS: usize = 21;

/// Explicitly registered font resources and their roles.
#[derive(Default)]
pub(super) struct Fonts {
    bundled_mode: Option<u32>,
    sizes: [u64; C8_MAX_FONTS],
    faces: [u32; C8_MAX_FONTS],
    count: usize,
    roles: Option<C8PageFonts>,
    embedding: Option<FontEmbedding>,
    glyph_aliases: Vec<NativeFontGlyphAlias>,
    /// At most [`MAX_SYMBOL_GLYPHS`]; the core validates the codes.
    symbol_glyphs: Vec<NativeSymbolGlyph>,
    symbol_font: Option<SymbolFontIdentity>,
    decoration_glyphs: Vec<NativeDecorationGlyph>,
    decoration_font: Option<SymbolFontIdentity>,
}

impl Fonts {
    pub(super) fn use_bundled(&mut self, mode: u32) -> bool {
        if !matches!(mode, 0 | 2) || self.count != 0 {
            return false;
        }
        self.bundled_mode = Some(mode);
        for (_, size) in caj2pdf_core::bundled_fonts::RESOURCES {
            self.add(size, 0);
        }
        true
    }

    pub(super) fn set_embedding(&mut self, mode: u32) -> bool {
        let embedding = match mode {
            0 => FontEmbedding::Subset,
            1 => FontEmbedding::Full,
            _ => return false,
        };
        if self.roles.is_none() || self.embedding.is_some() {
            return false;
        }
        self.embedding = Some(embedding);
        true
    }

    pub(super) fn add_glyph_alias(&mut self, font: u32, character: u32, glyph: u32) -> bool {
        let (Some(character), Some(glyph)) = (char::from_u32(character), char::from_u32(glyph))
        else {
            return false;
        };
        let font = font as usize;
        if self.roles.is_none()
            || font >= self.count
            || character > '\u{ffff}'
            || glyph > '\u{ffff}'
            || self.glyph_aliases.len() == MAX_NATIVE_FONT_GLYPHS
            || self
                .glyph_aliases
                .last()
                .is_some_and(|entry| (entry.font, entry.character) >= (font, character))
            || self.glyph_aliases.try_reserve(1).is_err()
        {
            return false;
        }
        self.glyph_aliases.push(NativeFontGlyphAlias {
            font,
            character,
            glyph,
        });
        true
    }

    pub(super) fn count(&self) -> usize {
        self.count
    }

    pub(super) fn add(&mut self, size: u64, face: u32) -> u32 {
        if self.count == self.sizes.len() || self.roles.is_some() {
            return 0;
        }
        self.sizes[self.count] = size;
        self.faces[self.count] = face;
        self.count += 1;
        self.count as u32
    }

    pub(super) fn set_latin_state(&mut self, state: u32, index: u32) -> bool {
        let Some(roles) = &mut self.roles else {
            return false;
        };
        let role = match state {
            3 => &mut roles.latin_state3,
            28 => &mut roles.latin_state28,
            31 => &mut roles.latin_state31,
            _ => return false,
        };
        if index as usize >= self.count || role.is_some() {
            return false;
        }
        *role = Some(index as usize);
        true
    }

    pub(super) fn set_cjk_style(&mut self, style: u32, index: u32) -> bool {
        let Some(roles) = &mut self.roles else {
            return false;
        };
        let role = match style {
            0x0800 => &mut roles.cjk_sans,
            0x0c00 => &mut roles.cjk_kai,
            0x0400 => &mut roles.cjk_system,
            _ => return false,
        };
        if index as usize >= self.count || role.is_some() {
            return false;
        }
        *role = Some(index as usize);
        true
    }

    /// Map one 16-bit code to a BMP glyph of the symbols role. Like the CLI,
    /// leave code and duplicate checks to the core's native preflight.
    pub(super) fn add_symbol_glyph(&mut self, code: u32, glyph: u32) -> bool {
        let (Ok(code), Some(glyph)) = (u16::try_from(code), char::from_u32(glyph)) else {
            return false;
        };
        if self.roles.is_none_or(|roles| roles.symbols.is_none())
            || glyph > '\u{ffff}'
            || self.symbol_glyphs.len() == MAX_SYMBOL_GLYPHS
        {
            return false;
        }
        self.symbol_glyphs.push(NativeSymbolGlyph { code, glyph });
        true
    }

    /// Bind the symbol glyph map to a PostScript name (`length` bytes,
    /// little-endian in `words`) and `head` checkSumAdjustment, once.
    pub(super) fn set_symbol_font(&mut self, checksum: u32, length: u32, words: [u64; 8]) -> bool {
        if self.roles.is_none_or(|roles| roles.symbols.is_none())
            || self.symbol_font.is_some()
            || length > 64
        {
            return false;
        }
        let Some(identity) = font_identity(checksum, length, words) else {
            return false;
        };
        self.symbol_font = Some(identity);
        true
    }

    /// Register at most four ornament aliases. The core validates values and duplicates.
    pub(super) fn add_decoration_glyph(&mut self, value: u32, glyph: u32) -> bool {
        let (Ok(value), Some(glyph)) = (u16::try_from(value), char::from_u32(glyph)) else {
            return false;
        };
        if self.roles.is_none_or(|roles| roles.decoration.is_none())
            || glyph > '\u{ffff}'
            || self.decoration_glyphs.len() == 4
        {
            return false;
        }
        self.decoration_glyphs
            .push(NativeDecorationGlyph { value, glyph });
        true
    }

    /// Bind ornament aliases to an explicitly supplied font, once after roles.
    pub(super) fn set_decoration_font(
        &mut self,
        checksum: u32,
        length: u32,
        words: [u64; 8],
    ) -> bool {
        if self.roles.is_none_or(|roles| roles.decoration.is_none())
            || self.decoration_font.is_some()
        {
            return false;
        }
        let Some(identity) = font_identity(checksum, length, words) else {
            return false;
        };
        self.decoration_font = Some(identity);
        true
    }

    pub(super) fn set(
        &mut self,
        cjk: u32,
        latin: u32,
        alternate: u32,
        decoration: u32,
        alias: u32,
        symbols: u32,
    ) -> bool {
        // `u32::MAX` marks an absent optional role; core applies its fallback.
        if self.bundled_mode.is_some()
            || self.roles.is_some()
            || [cjk, latin]
                .into_iter()
                .any(|index| index as usize >= self.count)
            || (alternate != u32::MAX && alternate as usize >= self.count)
        {
            return false;
        }
        let alternate_latin = (alternate != u32::MAX).then_some(alternate as usize);
        let decoration = if decoration == u32::MAX {
            None
        } else {
            let Some(character) = char::from_u32(alias) else {
                return false;
            };
            if decoration as usize >= self.count || alias > 0xffff {
                return false;
            }
            Some((decoration as usize, character))
        };
        let symbols = if symbols == u32::MAX {
            None
        } else if symbols as usize >= self.count {
            return false;
        } else {
            Some(symbols as usize)
        };
        self.roles = Some(C8PageFonts {
            cjk: cjk as usize,
            latin: latin as usize,
            alternate_latin,
            decoration,
            symbols,
            latin_state3: None,
            latin_state28: None,
            latin_state31: None,
            cjk_sans: None,
            cjk_kai: None,
            cjk_system: None,
        });
        true
    }
}

impl Fonts {
    /// The registered fonts as host resources 1..=11 for one conversion;
    /// `None` when none are registered.
    pub(super) fn resources<'h, H: Host + 'h>(
        self,
        host: &'h RefCell<&'h mut H>,
        limits: &caj2pdf_core::Limits,
    ) -> caj2pdf_core::Result<Option<FontResources<'h>>> {
        let mut resources = (self.count != 0).then(|| FontResources {
            embedding: self.embedding.unwrap_or_default(),
            glyph_aliases: self.glyph_aliases,
            sources: (0..self.count)
                .map(|index| {
                    let source: Box<dyn RangedSource + 'h> =
                        Box::new(HostSource::new(host, index as u32 + 1, self.sizes[index]));
                    C8FontSource {
                        source,
                        face: self.faces[index],
                    }
                })
                .collect(),
            roles: self.roles,
            symbol_glyphs: self.symbol_glyphs,
            symbol_font: self.symbol_font,
            decoration_glyphs: self.decoration_glyphs,
            decoration_font: self.decoration_font,
        });
        if let (Some(mode), Some(fonts)) = (self.bundled_mode, resources.as_mut()) {
            caj2pdf_core::bundled_fonts::configure(fonts, mode, limits)?;
        }
        Ok(resources)
    }
}

/// Decode the bounded ABI representation shared by explicit font identities.
fn font_identity(checksum: u32, length: u32, words: [u64; 8]) -> Option<SymbolFontIdentity> {
    if length > 64 {
        return None;
    }
    let mut bytes = [0u8; 64];
    for (chunk, word) in bytes.as_chunks_mut::<8>().0.iter_mut().zip(words) {
        chunk.copy_from_slice(&word.to_le_bytes());
    }
    let name = std::str::from_utf8(&bytes[..length as usize]).ok()?;
    SymbolFontIdentity::is_valid_postscript_name(name).then(|| SymbolFontIdentity {
        checksum_adjustment: checksum,
        postscript_name: name.to_owned(),
    })
}
