// SPDX-License-Identifier: MIT

//! Versioned CAJViewer font resource facts and measured native-text presets.
//! Adapters supply the bytes; the core never downloads or opens files.
mod data;

use crate::hnc8::{
    C8PageFonts, NativeDecorationGlyph, NativeFontGlyphAlias, NativeSymbolGlyph, SymbolFontIdentity,
};
use crate::{Cancellation, Error, Fonts, Limits, RangedSource, Result, read_exact_at};
pub use data::{PACK_SHA256, PACK_SIZE, RESOURCES};
use sha2::{Digest, Sha256};

/// Verify a complete font pack with bounded reads before trusting cached bytes.
pub fn verify<S: RangedSource, C: Cancellation>(
    source: &mut S,
    limits: &Limits,
    cancel: &C,
) -> Result<()> {
    limits.validate()?;
    if source.size() != PACK_SIZE {
        return Err(Error::invalid("bundled font pack has the wrong size"));
    }
    let chunk = limits.io_chunk_bytes.min(65536);
    limits.check_allocation(chunk as u64)?;
    let mut buffer = vec![0; chunk];
    let mut hash = Sha256::new();
    let mut offset = 0;
    while offset < PACK_SIZE {
        let count = (PACK_SIZE - offset).min(chunk as u64) as usize;
        read_exact_at(source, offset, &mut buffer[..count], limits, cancel)?;
        hash.update(&buffer[..count]);
        offset += count as u64;
    }
    if hash.finalize().as_slice() != PACK_SHA256 {
        return Err(Error::invalid("bundled font pack checksum mismatch"));
    }
    Ok(())
}

/// Apply the measured preset to eleven sources in [`RESOURCES`] order.
/// The Latin and decoration roles intentionally have separate source IDs,
/// while adapters may serve their identical bytes from one pack range.
/// Full programs preserve the viewer's original outlines and metrics.
pub fn configure(fonts: &mut Fonts<'_>, mode: u32, limits: &Limits) -> Result<()> {
    let aliases = match mode {
        0 => data::MODE0,
        2 => data::MODE2,
        _ => return Err(Error::invalid("unknown bundled font mode")),
    };
    if fonts.sources.len() != RESOURCES.len()
        || fonts
            .sources
            .iter()
            .zip(RESOURCES)
            .any(|(font, (_, size))| font.face != 0 || font.source.size() != size)
    {
        return Err(Error::invalid(
            "bundled font resources do not match the preset",
        ));
    }
    limits.check_allocation((aliases.len() * size_of::<NativeFontGlyphAlias>() + 2048) as u64)?;
    fonts.roles = Some(C8PageFonts {
        cjk: 0,
        latin: 1,
        alternate_latin: Some(2),
        decoration: Some((
            3,
            if mode == 2 {
                '\u{5d41}'
            } else {
                crate::hnc8::C8_DEFAULT_DECORATION_ALIAS
            },
        )),
        symbols: Some(4),
        latin_state3: Some(5),
        latin_state28: Some(6),
        latin_state31: Some(7),
        cjk_sans: Some(8),
        cjk_kai: Some(9),
        cjk_system: Some(10),
    });
    fonts.embedding = crate::pdf::FontEmbedding::Full;
    fonts.glyph_aliases = aliases
        .iter()
        .map(|&(font, character, glyph)| NativeFontGlyphAlias {
            font,
            character: char::from_u32(character).unwrap(),
            glyph: char::from_u32(glyph).unwrap(),
        })
        .collect();
    fonts.symbol_glyphs.clear();
    fonts.symbol_font = None;
    fonts.decoration_glyphs.clear();
    fonts.decoration_font = None;
    if mode == 0 {
        fonts.symbol_glyphs = data::SYMBOLS
            .iter()
            .map(|&(code, glyph)| NativeSymbolGlyph {
                code,
                glyph: char::from_u32(glyph).unwrap(),
            })
            .collect();
        fonts.symbol_font = Some(SymbolFontIdentity {
            postscript_name: "HGFX_CNKI".into(),
            checksum_adjustment: 0x9a2b73d3,
        });
    } else {
        fonts.decoration_glyphs = data::ORNAMENTS
            .iter()
            .map(|&(value, glyph)| NativeDecorationGlyph {
                value,
                glyph: char::from_u32(glyph).unwrap(),
            })
            .collect();
        fonts.decoration_font = Some(SymbolFontIdentity {
            postscript_name: "HGBZ_CNKI".into(),
            checksum_adjustment: 2632420189,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NeverCancel, hnc8::C8FontSource};

    struct Unread(u64);
    impl RangedSource for Unread {
        fn size(&self) -> u64 {
            self.0
        }
        fn read_at(&mut self, _: u64, _: &mut [u8]) -> Result<usize> {
            panic!("preset must not read font bytes")
        }
    }
    fn fonts() -> Fonts<'static> {
        Fonts {
            sources: RESOURCES
                .iter()
                .map(|&(_, size)| C8FontSource {
                    source: Box::new(Unread(size)) as Box<dyn RangedSource>,
                    face: 0,
                })
                .collect(),
            ..Fonts::default()
        }
    }
    #[test]
    fn measured_modes_replace_maps_without_reading_programs() {
        let mut fonts = fonts();
        configure(&mut fonts, 0, &Limits::default()).unwrap();
        assert_eq!(fonts.glyph_aliases.len(), 12);
        assert_eq!(fonts.symbol_glyphs.len(), 21);
        assert!(fonts.decoration_glyphs.is_empty());
        assert_eq!(
            fonts.symbol_font.as_ref().unwrap().postscript_name,
            "HGFX_CNKI"
        );
        configure(&mut fonts, 2, &Limits::default()).unwrap();
        assert_eq!(fonts.glyph_aliases.len(), 441);
        assert!(fonts.symbol_glyphs.is_empty());
        assert!(fonts.symbol_font.is_none());
        assert_eq!(fonts.decoration_glyphs.len(), 4);
        assert_eq!(fonts.roles.unwrap().decoration, Some((3, '\u{5d41}')));
        assert_eq!(fonts.embedding, crate::pdf::FontEmbedding::Full);
        assert!(
            fonts
                .glyph_aliases
                .windows(2)
                .all(|p| (p[0].font, p[0].character) < (p[1].font, p[1].character))
        );
        assert!(configure(&mut fonts, 3, &Limits::default()).is_err());
        fonts.sources.pop();
        assert!(configure(&mut fonts, 0, &Limits::default()).is_err());
    }
    #[test]
    fn preset_limits_and_pack_failures_are_checked_before_reads() {
        let limits = Limits {
            max_allocation_bytes: 32,
            ..Limits::default()
        };
        assert!(configure(&mut fonts(), 2, &limits).is_err());
        assert!(verify(&mut Unread(PACK_SIZE - 1), &Limits::default(), &NeverCancel).is_err());
        assert!(verify(&mut Unread(PACK_SIZE), &limits, &NeverCancel).is_err());
        struct Stop;
        impl Cancellation for Stop {
            fn is_cancelled(&self) -> bool {
                true
            }
        }
        assert!(matches!(
            verify(&mut Unread(PACK_SIZE), &Limits::default(), &Stop)
                .unwrap_err()
                .kind,
            crate::ErrorKind::Cancelled
        ));
    }
}
