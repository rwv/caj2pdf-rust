// SPDX-License-Identifier: MIT

//! HN/C8 font resources: the files named by the font options or found
//! installed, opened once each and mapped to the core's font roles.

use crate::{
    CliError,
    args::{ConvertOptions, Endpoint, FONT_EXTENSIONS, FONT_FILES},
    files::{Input, open_input},
};
use caj2pdf_core::{
    Fonts, Limits, RangedSource,
    hnc8::{C8_DEFAULT_DECORATION_ALIAS, C8FontSource},
    native::SeekableSource,
};
use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

#[derive(Default)]
pub struct Resources {
    // Retain opened input identities so --force cannot overwrite a font.
    pub inputs: Vec<Input>,
    pub font_roles: Option<caj2pdf_core::hnc8::C8PageFonts>,
    /// Collection face of each opened font source.
    pub font_faces: [u32; 8],
}

impl Resources {
    pub fn has_fonts(&self) -> bool {
        self.font_roles.is_some()
    }

    /// The opened fonts and their roles, for one conversion.
    pub fn fonts(&mut self) -> caj2pdf_core::Result<Option<Fonts<'_>>> {
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
            sources,
            roles: Some(roles),
        }))
    }
    pub fn load(options: &ConvertOptions, limits: &Limits) -> Result<Self, CliError> {
        let mut resources = Self::default();
        let mut fonts: [Option<(PathBuf, u32)>; 8] = Default::default();
        for (font, path) in fonts.iter_mut().zip(font_paths(options)?) {
            *font = path.as_deref().map(font_face).transpose()?;
        }
        resources.open_fonts(&fonts, options.decoration_char, limits)?;
        Ok(resources)
    }

    /// Use installed fonts found by [`crate::system_fonts::discover`] for
    /// the CJK and Latin roles; every optional role stays absent.
    pub fn use_installed(
        &mut self,
        fonts: &crate::system_fonts::Installed,
        limits: &Limits,
    ) -> Result<(), CliError> {
        let mut roles: [Option<(PathBuf, u32)>; 8] = Default::default();
        for (role, choice) in roles.iter_mut().zip(&fonts.choices) {
            *role = Some((choice.path.clone(), choice.face));
        }
        self.open_fonts(&roles, None, limits)
    }

    /// Open each distinct `(file, face)` once and assign the roles.
    fn open_fonts(
        &mut self,
        fonts: &[Option<(PathBuf, u32)>; 8],
        decoration_char: Option<char>,
        limits: &Limits,
    ) -> Result<(), CliError> {
        if fonts.iter().all(Option::is_none) {
            return Ok(());
        }
        let mut faces = Vec::new();
        let mut indices = [0; 8];
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
fn font_paths(options: &ConvertOptions) -> Result<[Option<PathBuf>; 8], CliError> {
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
