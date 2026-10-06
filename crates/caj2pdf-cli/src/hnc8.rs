// SPDX-License-Identifier: MIT

//! Experimental HN/C8 CLI routing with the standard codec states.

use crate::signals::ProcessCancellation;
use crate::{
    CliError,
    args::{ConvertOptions, Endpoint, FONT_EXTENSIONS, FONT_FILES},
    files::{Input, open_input},
};
use caj2pdf_core::{
    Error, Limits, RangedSource,
    hnc8::{
        ApplicationInfoReport, ApplicationInfoStatus, C8_DEFAULT_DECORATION_ALIAS, C8FontSource,
        C8FontSources, ComposeOptions, ComposePage, ComposeVisitor, OutlineReport,
        convert_document_pdf,
    },
    jbig2::text::TextHeaderPolicy,
    qm::QmTable,
};
use std::io::Write;
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

struct CompletePages;
impl ComposeVisitor for CompletePages {
    fn page(&mut self, page: ComposePage<'_>) -> caj2pdf_core::Result<()> {
        if page.output_page.is_none() {
            return Err(Error::InvalidInput {
                reason: "HN/C8 conversion cannot omit source pages without image content",
            });
        }
        Ok(())
    }
}

pub fn compose_options(include_bookmarks: bool) -> ComposeOptions {
    ComposeOptions {
        // The HN/C8 profile explicitly admits the measured unused-template
        // anomaly; general JBIG2 APIs and all other malformed flags stay strict.
        text_header_policy: TextHeaderPolicy::HnC8UnusedRefinementTemplate,
        include_bookmarks,
    }
}

pub fn convert<S: RangedSource, W: Write>(
    source: &mut S,
    sink: &mut W,
    resources: &mut Resources,
    include_bookmarks: bool,
    limits: &Limits,
) -> Result<(OutlineReport, ApplicationInfoStatus), String> {
    let options = compose_options(include_bookmarks);
    // Empty unless fonts were supplied.
    let mut fonts = resources
        .inputs
        .iter_mut()
        .zip(resources.font_faces)
        .map(|(input, face)| {
            Ok(C8FontSource {
                source: caj2pdf_core::native::SeekableSource::new(&mut input.file)?,
                face,
            })
        })
        .collect::<caj2pdf_core::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    // The core routes by text framing: image documents ignore the fonts.
    convert_document_pdf(
        source,
        sink,
        resources.font_roles.map(|roles| C8FontSources {
            sources: &mut fonts,
            roles,
        }),
        Some(&QmTable::standard()),
        &mut CompletePages,
        options,
        limits,
        &ProcessCancellation,
    )
    .map(|report| (report.outline, report.application_info))
    .map_err(|e| e.to_string())
}

/// The bounded HN/C8 document-level inspection; image payloads are never read.
#[derive(Debug)]
pub struct Inspected {
    pub header: caj2pdf_core::hnc8::Header,
    pub bookmarks: Option<Vec<caj2pdf_core::Bookmark>>,
    pub outline: OutlineReport,
    pub structure: Option<crate::document::Structure>,
    /// The C8 application-info package; a defect is a warning.
    pub application_info: ApplicationInfoReport,
}

/// Keep only bounded outline and application-info metadata; image payloads
/// are never read here. `structure` also reports the page-index layout and
/// application-info tail.
pub fn inspect<S: RangedSource>(
    source: &mut S,
    limits: &Limits,
    structure: bool,
) -> Result<Inspected, String> {
    use caj2pdf_core::hnc8::Hnc8Reader;
    let mut reader =
        Hnc8Reader::open(source, limits, &ProcessCancellation).map_err(|e| e.to_string())?;
    let header = reader.header();
    let structure = if structure {
        Some(crate::document::Structure::Hnc8 {
            header,
            page_row_bytes: reader.page_row_bytes(),
            application_info: reader.application_info_tail().map_err(|e| e.to_string())?,
        })
    } else {
        None
    };
    let application_info = reader
        .application_info_report()
        .map_err(|e| e.to_string())?;
    let mut inspected = Inspected {
        header,
        bookmarks: None,
        outline: OutlineReport::default(),
        structure,
        application_info,
    };
    let Some(count) = reader.declared_bookmark_count() else {
        return Ok(inspected);
    };
    if count > limits.max_bookmarks {
        return Err("HN-A bookmark count exceeds the configured limit".into());
    }
    let bytes = u64::from(count) * std::mem::size_of::<caj2pdf_core::Bookmark>() as u64;
    limits.check_allocation(bytes).map_err(|e| e.to_string())?;
    let mut collected = CollectedBookmarks {
        items: Vec::new(),
        limits: *limits,
        bytes,
    };
    collected
        .items
        .try_reserve_exact(count as usize)
        .map_err(|_| "cannot allocate HN-A outline metadata")?;
    inspected.outline = reader
        .visit_bookmarks(64, header.page_count, |page| Some(page - 1), &mut collected)
        .map_err(|e| e.to_string())?;
    inspected.bookmarks = Some(collected.items);
    Ok(inspected)
}

/// Why the per-page report stopped.
#[derive(Debug)]
pub enum PagesError {
    Input(String),
    Output(std::io::Error),
}

/// A failure that would repeat on every later page ends the report.
fn fatal(error: caj2pdf_core::hnc8::Hnc8Error) -> Result<String, PagesError> {
    use caj2pdf_core::hnc8::ErrorKind;
    match error.kind {
        ErrorKind::Cancelled | ErrorKind::Source { .. } => {
            Err(PagesError::Input(error.to_string()))
        }
        _ => Ok(error.to_string()),
    }
}

/// Stream one structural record per page. Each page uses a fresh cursor, so
/// a malformed page is reported and later pages are still inspected; only
/// one page's row, the current descriptor and bounded text-reader state are
/// held at a time. Image payloads and text content are never reported.
pub fn write_pages<S: RangedSource, W: std::io::Write>(
    source: &mut S,
    limits: &Limits,
    page_count: u32,
    out: &mut crate::report::Pages<'_, W>,
) -> Result<(), PagesError> {
    use caj2pdf_core::hnc8::Hnc8Reader;
    out.begin().map_err(PagesError::Output)?;
    for number in 1..=page_count {
        let mut reader = Hnc8Reader::probe_at_page(source, limits, &ProcessCancellation, number)
            .map_err(|e| PagesError::Input(e.to_string()))?;
        let row = match reader.next_page() {
            Ok(row) => row.expect("a probe opens at a declared page"),
            Err(error) => {
                let message = fatal(error)?;
                out.page(number, None).map_err(PagesError::Output)?;
                out.end_page(None, None, Some(&message))
                    .map_err(PagesError::Output)?;
                continue;
            }
        };
        out.page(number, Some(&row)).map_err(PagesError::Output)?;
        let error = loop {
            match reader.next_image() {
                Ok(Some(image)) => out.image(&image).map_err(PagesError::Output)?,
                Ok(None) => break None,
                Err(error) => break Some(fatal(error)?),
            }
        };
        if error.is_some() {
            out.end_page(None, None, error.as_deref())
                .map_err(PagesError::Output)?;
            continue;
        }
        match reader.inspect_text() {
            Ok(text) => out.end_page(Some(&text), None, None),
            Err(error) => out.end_page(None, Some(&fatal(error)?), None),
        }
        .map_err(PagesError::Output)?;
    }
    out.finish().map_err(PagesError::Output)
}

struct CollectedBookmarks {
    items: Vec<caj2pdf_core::Bookmark>,
    limits: Limits,
    bytes: u64,
}

impl caj2pdf_core::BookmarkVisitor for CollectedBookmarks {
    fn visit(&mut self, bookmark: caj2pdf_core::Bookmark) -> caj2pdf_core::Result<()> {
        self.bytes += bookmark.title.capacity() as u64;
        self.limits.check_allocation(self.bytes)?;
        self.items.push(bookmark);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_repeating_page_failures_end_the_page_report() {
        use caj2pdf_core::hnc8::{ErrorKind, Hnc8Error};
        let error = |kind| Hnc8Error {
            variant: None,
            offset: 7,
            page: Some(2),
            image: None,
            kind,
        };
        let source = ErrorKind::Source {
            field: "page row",
            source: Error::Cancelled,
        };
        for kind in [ErrorKind::Cancelled, source] {
            assert!(matches!(fatal(error(kind)), Err(PagesError::Input(_))));
        }
        let page = fatal(error(ErrorKind::IncompletePage)).unwrap();
        assert_eq!(
            page,
            "HN/C8 at byte 7, page 2: page has unread image records"
        );
    }

    #[test]
    fn outline_collection_accounts_for_records_and_retained_titles() {
        use caj2pdf_core::native::SeekableSource;
        let mut bytes = vec![0; 0x15c + 308 + 20];
        bytes[..8].copy_from_slice(&[72, 78, 0, 0, 0x90, 1, 0, 0]);
        bytes[0x90] = 1;
        bytes[0x158] = 1;
        bytes[0x15c..0x160].copy_from_slice(b"Root");
        bytes[0x15c + 280] = b'1';
        bytes[0x15c + 304] = 1;
        for (max_bookmarks, max_allocation_bytes) in [
            (0, 4096),
            (1, 1),
            (1, std::mem::size_of::<caj2pdf_core::Bookmark>() as u64),
        ] {
            let mut source = SeekableSource::new(std::io::Cursor::new(&bytes)).unwrap();
            let limits = Limits {
                max_bookmarks,
                max_allocation_bytes,
                io_chunk_bytes: 1,
                ..Limits::default()
            };
            let error = inspect(&mut source, &limits, false).unwrap_err();
            assert!(error.contains("limit"), "{error}");
        }
    }
}
