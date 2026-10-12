// SPDX-License-Identifier: MIT

//! HN/C8 font resources: the files named by the font options or found
//! installed, opened once each and mapped to the core's font roles.

use crate::{
    CliError,
    command::{ConvertOptions, Endpoint, FONT_EXTENSIONS, FONT_FILES},
    files::{Input, open_input},
};
use caj2pdf_core::{
    Fonts, Limits, RangedSource,
    hnc8::{C8_DEFAULT_DECORATION_ALIAS, C8_MAX_FONTS, C8FontSource},
    native::SeekableSource,
};
use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

static FONT_PACK: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/fonts.bin"));

#[derive(Default)]
pub struct Resources {
    // Retain opened input identities so --force cannot overwrite a font.
    pub inputs: Vec<Input>,
    pub bundled_mode: Option<u32>,
    pub font_roles: Option<caj2pdf_core::hnc8::C8PageFonts>,
    /// Collection face of each opened font source.
    pub font_faces: [u32; C8_MAX_FONTS],
    pub symbol_glyphs: Vec<caj2pdf_core::hnc8::NativeSymbolGlyph>,
    pub symbol_font: Option<caj2pdf_core::hnc8::SymbolFontIdentity>,
    pub decoration_glyphs: Vec<caj2pdf_core::hnc8::NativeDecorationGlyph>,
    pub decoration_font: Option<caj2pdf_core::hnc8::SymbolFontIdentity>,
    pub embedding: caj2pdf_core::pdf::FontEmbedding,
    pub glyph_aliases: Vec<caj2pdf_core::hnc8::NativeFontGlyphAlias>,
}

impl Resources {
    pub fn has_fonts(&self) -> bool {
        self.bundled_mode.is_some() || self.font_roles.is_some()
    }

    /// The opened fonts and their roles, for one conversion.
    pub fn fonts(&mut self) -> caj2pdf_core::Result<Option<Fonts<'_>>> {
        if let Some(mode) = self.bundled_mode {
            let sources = caj2pdf_core::bundled_fonts::RESOURCES
                .iter()
                .map(|&(offset, size)| {
                    let source: Box<dyn RangedSource> =
                        Box::new(&FONT_PACK[offset as usize..(offset + size) as usize]);
                    C8FontSource { source, face: 0 }
                })
                .collect();
            let mut fonts = Fonts {
                sources,
                ..Fonts::default()
            };
            caj2pdf_core::bundled_fonts::configure(&mut fonts, mode, &Limits::default())?;
            return Ok(Some(fonts));
        }
        let Some(roles) = self.font_roles else {
            return Ok(None);
        };
        let sources = self
            .inputs
            .iter_mut()
            .zip(self.font_faces)
            .map(|(input, face)| {
                let source: Box<dyn RangedSource + '_> =
                    Box::new(SeekableSource::new(&mut input.file)?);
                Ok(C8FontSource { source, face })
            })
            .collect::<caj2pdf_core::Result<_>>()?;
        Ok(Some(Fonts {
            embedding: self.embedding,
            glyph_aliases: self.glyph_aliases.clone(),
            sources,
            roles: Some(roles),
            symbol_glyphs: self.symbol_glyphs.clone(),
            symbol_font: self.symbol_font.clone(),
            decoration_glyphs: self.decoration_glyphs.clone(),
            decoration_font: self.decoration_font.clone(),
        }))
    }
    pub fn load(options: &ConvertOptions, limits: &Limits) -> Result<Self, CliError> {
        let mut resources = Self::default();
        let mut fonts: [Option<(PathBuf, u32)>; C8_MAX_FONTS] = Default::default();
        for (font, path) in fonts.iter_mut().zip(font_paths(options)?) {
            *font = path.as_deref().map(font_face).transpose()?;
        }
        resources.open_fonts(&fonts, options.decoration_char, limits)?;
        resources.embedding = options.font_embedding;
        if let Some(roles) = resources.font_roles {
            let indices = [
                Some(roles.cjk),
                Some(roles.latin),
                roles.alternate_latin,
                roles.decoration.map(|(font, _)| font),
                roles.symbols,
                roles.latin_state3,
                roles.latin_state28,
                roles.latin_state31,
                roles.cjk_sans,
                roles.cjk_kai,
                roles.cjk_system,
            ];
            limits
                .check_allocation(
                    (options.font_glyphs.len()
                        * size_of::<caj2pdf_core::hnc8::NativeFontGlyphAlias>())
                        as u64,
                )
                .map_err(|error| CliError::runtime(error.to_string()))?;
            resources
                .glyph_aliases
                .try_reserve_exact(options.font_glyphs.len())
                .map_err(|_| CliError::runtime("native font glyph alias allocation failed"))?;
            for entry in &options.font_glyphs {
                let font = indices[entry.font].ok_or_else(|| {
                    CliError::usage(format!(
                        "--font-glyph requires the {} font role",
                        FONT_FILES[entry.font]
                    ))
                })?;
                resources
                    .glyph_aliases
                    .push(caj2pdf_core::hnc8::NativeFontGlyphAlias { font, ..*entry });
            }
            resources
                .glyph_aliases
                .sort_unstable_by_key(|entry| (entry.font, entry.character));
            if resources
                .glyph_aliases
                .windows(2)
                .any(|pair| (pair[0].font, pair[0].character) == (pair[1].font, pair[1].character))
            {
                return Err(CliError::usage(
                    "--font-glyph maps a font character more than once",
                ));
            }
        }
        resources.symbol_glyphs.clone_from(&options.symbol_glyphs);
        resources.symbol_font.clone_from(&options.symbol_font);
        resources
            .decoration_glyphs
            .clone_from(&options.decoration_glyphs);
        resources
            .decoration_font
            .clone_from(&options.decoration_font);
        Ok(resources)
    }

    /// Use installed fonts found by [`crate::system_fonts::discover`] for
    /// the CJK and Latin roles; every optional role stays absent.
    pub fn use_installed(
        &mut self,
        fonts: &crate::system_fonts::Installed,
        limits: &Limits,
    ) -> Result<(), CliError> {
        let mut roles: [Option<(PathBuf, u32)>; C8_MAX_FONTS] = Default::default();
        for (role, choice) in roles.iter_mut().zip(&fonts.choices) {
            *role = Some((choice.path.clone(), choice.face));
        }
        self.open_fonts(&roles, None, limits)
    }

    /// Open each distinct `(file, face)` once and assign the roles.
    fn open_fonts(
        &mut self,
        fonts: &[Option<(PathBuf, u32)>; C8_MAX_FONTS],
        decoration_char: Option<char>,
        limits: &Limits,
    ) -> Result<(), CliError> {
        if fonts.iter().all(Option::is_none) {
            return Ok(());
        }
        let mut faces = Vec::new();
        let mut indices = [0; C8_MAX_FONTS];
        for (role, face) in fonts.iter().enumerate() {
            if let Some(face) = face {
                indices[role] = if let Some(index) = faces.iter().position(|f| f == face) {
                    index
                } else {
                    let index = faces.len();
                    self.inputs.push(open_input(
                        &Endpoint::Path(face.0.clone()),
                        limits.max_input_bytes,
                    )?);
                    self.font_faces[index] = face.1;
                    faces.push(face.clone());
                    index
                };
            }
        }
        let role = |index: usize| fonts[index].as_ref().map(|_| indices[index]);
        self.font_roles = Some(caj2pdf_core::hnc8::C8PageFonts {
            cjk: indices[0],
            latin: indices[1],
            alternate_latin: role(2),
            symbols: role(4),
            latin_state3: role(5),
            latin_state28: role(6),
            latin_state31: role(7),
            cjk_sans: role(8),
            cjk_kai: role(9),
            cjk_system: role(10),
            decoration: role(3).map(|index| {
                (
                    index,
                    decoration_char.unwrap_or(C8_DEFAULT_DECORATION_ALIAS),
                )
            }),
        });
        Ok(())
    }
}

/// Resolve explicit role paths, then fill absent roles from `--fonts DIR`.
/// A missing optional file leaves its role to the core fallback; any other
/// metadata failure keeps the path so opening it reports the actual error.
fn font_paths(options: &ConvertOptions) -> Result<[Option<PathBuf>; C8_MAX_FONTS], CliError> {
    let mut paths = options.fonts.clone();
    let Some(directory) = &options.font_dir else {
        return Ok(paths);
    };
    if !directory.is_dir() {
        return Err(CliError::runtime(format!(
            "font directory '{}' is not a readable directory",
            directory.display()
        )));
    }
    for (path, stem) in paths.iter_mut().zip(FONT_FILES) {
        if path.is_none() {
            *path = FONT_EXTENSIONS
                .iter()
                .map(|extension| directory.join(format!("{stem}.{extension}")))
                .find(|candidate| {
                    !matches!(std::fs::metadata(candidate), Err(e) if e.kind() == ErrorKind::NotFound)
                });
        }
    }
    let missing = |role: usize| {
        let names = FONT_EXTENSIONS.map(|extension| format!("{}.{extension}", FONT_FILES[role]));
        format!(
            "font directory '{}' has no {}",
            directory.display(),
            names.join(" or ")
        )
    };
    for (role, flag) in [(0, "--font-cjk"), (1, "--font-latin")] {
        if paths[role].is_none() {
            return Err(CliError::runtime(format!(
                "{}; add one or pass {flag}",
                missing(role)
            )));
        }
    }
    if options.decoration_char.is_some() && paths[3].is_none() {
        return Err(CliError::runtime(format!(
            "--decoration-char requires a decoration font; {}",
            missing(3)
        )));
    }
    if !options.decoration_glyphs.is_empty() && paths[3].is_none() {
        return Err(CliError::usage(
            "decoration glyphs require an explicit decoration font",
        ));
    }
    if !options.symbol_glyphs.is_empty() && paths[4].is_none() {
        return Err(CliError::runtime(format!(
            "--symbol-glyph requires a symbols font; {}",
            missing(4)
        )));
    }
    Ok(paths)
}

/// Split a font argument into its file and collection face. `FILE#N`
/// selects face `N` when `FILE#N` itself is not an existing file. The
/// suffix requires a Unicode path.
fn font_face(path: &Path) -> Result<(PathBuf, u32), CliError> {
    let split = path
        .to_str()
        .and_then(|text| text.rsplit_once('#'))
        .filter(|(file, digits)| {
            !file.is_empty()
                && !digits.is_empty()
                && digits.bytes().all(|byte| byte.is_ascii_digit())
        });
    match split {
        Some((file, digits)) if !path.exists() => {
            let face = digits.parse().map_err(|_| {
                CliError::runtime(format!(
                    "font face index in '{}' is too large",
                    path.display()
                ))
            })?;
            Ok((PathBuf::from(file), face))
        }
        _ => Ok((path.to_owned(), 0)),
    }
}
