// SPDX-License-Identifier: MIT

//! Installed-font discovery for native C8/HN-B text when no font option is
//! given. See `docs/cli.md#installed-fonts`.
//!
//! The platform font directories are walked once, to a bounded depth and
//! number of directory entries, in sorted order and without following
//! symbolic links to directories. Only files whose names are listed for a
//! known face are opened; each is accepted only when the core font reader
//! validates the face and its PostScript name matches. The first match in
//! each ordered list wins.

use crate::document::block_on;
use crate::signals::ProcessCancellation;
use caj2pdf_core::{Limits, native::SeekableSource, pdf::OpenTypeFont};
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::path::{Path, PathBuf};

/// Directory levels searched below each root.
pub const MAX_DEPTH: usize = 6;
/// Directory entries read in one search, over all roots.
pub const MAX_ENTRIES: usize = 20_000;
/// Collection faces checked in one file.
pub const MAX_FACES: u32 = 64;
/// Replaces the platform font directories with a path list (`:`-separated
/// on Unix, `;` on Windows). An empty value searches nothing.
pub const DIRS_VARIABLE: &str = "CAJ2PDF_FONT_DIRS";

/// One known face: its PostScript name and the file names it ships in.
#[derive(Debug, Eq, PartialEq)]
pub struct Face {
    pub postscript: &'static str,
    pub files: &'static [&'static str],
}

macro_rules! face {
    ($postscript:literal, [$($file:literal),+ $(,)?] $(,)?) => {
        Face {
            postscript: $postscript,
            files: &[$($file),+],
        }
    };
}

/// CJK faces in preference order: serif (Song/Ming) faces first, which
/// match the printed documents, then sans faces.
pub const CJK: [Face; 10] = [
    face!(
        "NotoSerifCJKsc-Regular",
        ["NotoSerifCJK-Regular.ttc", "NotoSerifCJKsc-Regular.otf"],
    ),
    face!(
        "SourceHanSerifSC-Regular",
        ["SourceHanSerif-Regular.ttc", "SourceHanSerifSC-Regular.otf"],
    ),
    face!("SimSun", ["simsun.ttc"]),
    face!("STSongti-SC-Regular", ["Songti.ttc"]),
    face!(
        "NotoSansCJKsc-Regular",
        ["NotoSansCJK-Regular.ttc", "NotoSansCJKsc-Regular.otf"],
    ),
    face!(
        "SourceHanSansSC-Regular",
        ["SourceHanSans-Regular.ttc", "SourceHanSansSC-Regular.otf"],
    ),
    face!("MicrosoftYaHei", ["msyh.ttc", "msyh.ttf"]),
    face!("PingFangSC-Regular", ["PingFang.ttc"]),
    face!("WenQuanYiZenHei", ["wqy-zenhei.ttc"]),
    face!(
        "DroidSansFallback",
        ["DroidSansFallbackFull.ttf", "DroidSansFallback.ttf"],
    ),
];

/// Latin faces in measured order: faces covering every Latin-font glyph of
/// the pinned documents first, then faces that miss some symbols.
pub const LATIN: [Face; 7] = [
    face!("FreeSerif", ["FreeSerif.ttf", "FreeSerif.otf"]),
    face!("DejaVuSans", ["DejaVuSans.ttf"]),
    face!("NimbusRoman-Regular", ["NimbusRoman-Regular.otf"]),
    face!("DejaVuSerif", ["DejaVuSerif.ttf"]),
    face!("LiberationSerif", ["LiberationSerif-Regular.ttf"]),
    face!("TimesNewRomanPSMT", ["times.ttf", "Times New Roman.ttf"]),
    face!("Times-Roman", ["Times.ttc"]),
];

/// The platform whose font directories are searched. A build constructs
/// only its own platform; the tests construct each.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))]
pub enum Platform {
    /// Linux and other Unix systems: the XDG base directories.
    Unix,
    MacOs,
    Windows,
}

#[cfg(target_os = "macos")]
pub const PLATFORM: Platform = Platform::MacOs;
#[cfg(windows)]
pub const PLATFORM: Platform = Platform::Windows;
#[cfg(not(any(target_os = "macos", windows)))]
pub const PLATFORM: Platform = Platform::Unix;

/// The directories searched, in order, from the environment `var`.
pub fn roots(platform: Platform, var: impl Fn(&'static str) -> Option<OsString>) -> Vec<PathBuf> {
    // Relative values are ignored, as the XDG specification requires.
    let absolute = |value: Option<OsString>| value.map(PathBuf::from).filter(|p| p.is_absolute());
    let mut roots = Vec::new();
    if let Some(list) = var(DIRS_VARIABLE) {
        roots.extend(std::env::split_paths(&list).filter(|p| p.is_absolute()));
    } else {
        let home = absolute(var("HOME"));
        match platform {
            Platform::Unix => {
                let data = absolute(var("XDG_DATA_HOME"))
                    .or_else(|| home.as_ref().map(|home| home.join(".local/share")));
                roots.extend(data.map(|data| data.join("fonts")));
                roots.extend(home.map(|home| home.join(".fonts")));
                let dirs = var("XDG_DATA_DIRS")
                    .filter(|dirs| !dirs.is_empty())
                    .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
                roots.extend(
                    std::env::split_paths(&dirs)
                        .filter(|p| p.is_absolute())
                        .map(|dir| dir.join("fonts")),
                );
            }
            Platform::MacOs => {
                roots.extend(home.map(|home| home.join("Library/Fonts")));
                roots.extend(["/Library/Fonts", "/System/Library/Fonts"].map(PathBuf::from));
            }
            Platform::Windows => {
                let local = absolute(var("LOCALAPPDATA"));
                roots.extend(local.map(|local| local.join("Microsoft\\Windows\\Fonts")));
                let windows = absolute(var("WINDIR"))
                    .or_else(|| absolute(var("SystemRoot")))
                    .unwrap_or_else(|| PathBuf::from("C:\\Windows"));
                roots.push(windows.join("Fonts"));
            }
        }
    }
    let mut seen = HashSet::new();
    roots.retain(|root| seen.insert(root.clone()));
    roots
}

/// Files with listed names, in walk order.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct Walk {
    pub files: Vec<PathBuf>,
    /// The entry bound stopped the walk.
    pub truncated: bool,
}

#[cfg(unix)]
type DirectoryKey = (u64, u64);
#[cfg(not(unix))]
type DirectoryKey = PathBuf;

/// The identity of a directory, following a symbolic link only for a root.
#[cfg(unix)]
fn directory_key(path: &Path) -> Option<DirectoryKey> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::metadata(path).ok().filter(fs::Metadata::is_dir)?;
    Some((metadata.dev(), metadata.ino()))
}

/// Without symbolic links to directories, only roots can repeat a path.
#[cfg(not(unix))]
fn directory_key(path: &Path) -> Option<DirectoryKey> {
    let metadata = fs::metadata(path).ok().filter(fs::Metadata::is_dir);
    metadata.map(|_| path.to_owned())
}

/// Whether `name` is a file name listed for one of `faces`, ignoring ASCII
/// case.
fn listed(faces: &[Face], name: &OsStr) -> bool {
    name.to_str().is_some_and(|name| {
        faces
            .iter()
            .flat_map(|face| face.files)
            .any(|file| file.eq_ignore_ascii_case(name))
    })
}

/// Walk `roots` depth-first, each directory's entries in byte order of
/// their names, collecting regular files (or symbolic links to them) whose
/// names are `wanted`. Symbolic links to directories are not followed, a
/// directory reached twice is walked once, unreadable entries are skipped,
/// and at most `max_entries` entries are read.
pub fn walk(roots: &[PathBuf], wanted: impl Fn(&OsStr) -> bool, max_entries: usize) -> Walk {
    let mut walk = Walk::default();
    let mut budget = max_entries;
    let mut visited = HashSet::new();
    for root in roots {
        let mut stack = vec![(root.clone(), 0)];
        while let Some((directory, depth)) = stack.pop() {
            let fresh = directory_key(&directory).is_some_and(|key| visited.insert(key));
            let Some(entries) = fresh.then(|| fs::read_dir(&directory).ok()).flatten() else {
                continue;
            };
            let mut names = Vec::new();
            for entry in entries {
                if budget == 0 {
                    walk.truncated = true;
                    return walk;
                }
                budget -= 1;
                if let Ok((name, kind)) = entry.and_then(|e| Ok((e.file_name(), e.file_type()?))) {
                    names.push((name, kind));
                }
            }
            names.sort_by(|a, b| a.0.cmp(&b.0));
            let mut subdirectories = Vec::new();
            for (name, kind) in names {
                if kind.is_dir() {
                    if depth < MAX_DEPTH {
                        subdirectories.push((directory.join(name), depth + 1));
                    }
                } else if wanted(&name) {
                    let path = directory.join(name);
                    if fs::metadata(&path).is_ok_and(|m| m.is_file()) {
                        walk.files.push(path);
                    }
                }
            }
            stack.extend(subdirectories.into_iter().rev());
        }
    }
    walk
}

/// The first face of `path` (at most [`MAX_FACES`] are checked) that the
/// core reader accepts with PostScript name `postscript`.
fn find_face(path: &Path, postscript: &str, limits: &Limits) -> Option<u32> {
    let file = File::open(path).ok()?;
    let mut source = SeekableSource::new(file).ok()?;
    let count = block_on(OpenTypeFont::face_count(
        &mut source,
        limits,
        &ProcessCancellation,
    ))
    .ok()?;
    (0..count.min(MAX_FACES)).find(|&face| {
        block_on(OpenTypeFont::read(
            &mut source,
            face,
            limits,
            &ProcessCancellation,
        ))
        .and_then(|font| font.postscript_name())
        .is_ok_and(|name| name == postscript)
    })
}

/// A chosen installed face.
#[derive(Debug, Eq, PartialEq)]
pub struct Choice {
    pub path: PathBuf,
    pub face: u32,
    pub postscript: &'static str,
}

/// The first listed face found in `files`, in list order, then file order.
fn choose(files: &[PathBuf], faces: &'static [Face], limits: &Limits) -> Option<Choice> {
    faces.iter().find_map(|known| {
        files
            .iter()
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| listed(std::slice::from_ref(known), name))
            })
            .find_map(|path| {
                find_face(path, known.postscript, limits).map(|face| Choice {
                    path: path.clone(),
                    face,
                    postscript: known.postscript,
                })
            })
    })
}

/// The installed CJK and Latin faces, in role order.
#[derive(Debug, Eq, PartialEq)]
pub struct Installed {
    pub choices: [Choice; 2],
    /// The entry bound that stopped the search, if it did.
    pub stopped_after: Option<usize>,
}

const ROLES: [&str; 2] = ["CJK", "Latin"];

impl Installed {
    /// The standard-error report naming each chosen file and face. A face
    /// other than 0 uses the `FILE#N` spelling the font options accept.
    pub fn report(&self) -> String {
        let mut text = String::new();
        for (role, choice) in ROLES.iter().zip(&self.choices) {
            let face = if choice.face == 0 {
                String::new()
            } else {
                format!("#{}", choice.face)
            };
            text += &format!(
                "caj2pdf: using installed {role} font {}{face} ({})\n",
                choice.path.display(),
                choice.postscript
            );
        }
        if let Some(entries) = self.stopped_after {
            text += &format!(
                "caj2pdf: note: the font search stopped after {entries} directory entries\n"
            );
        }
        text
    }
}

/// Search `roots` for a CJK and a Latin face. The error names what was
/// searched and the alternatives.
pub fn discover(roots: &[PathBuf], limits: &Limits) -> Result<Installed, String> {
    discover_with(roots, limits, MAX_ENTRIES)
}

/// [`discover`] reading at most `max_entries` directory entries.
fn discover_with(
    roots: &[PathBuf],
    limits: &Limits,
    max_entries: usize,
) -> Result<Installed, String> {
    let walk = walk(
        roots,
        |name| listed(&CJK, name) || listed(&LATIN, name),
        max_entries,
    );
    let stopped_after = walk.truncated.then_some(max_entries);
    let cjk = choose(&walk.files, &CJK, limits);
    let latin = choose(&walk.files, &LATIN, limits);
    match (cjk, latin) {
        (Some(cjk), Some(latin)) => Ok(Installed {
            choices: [cjk, latin],
            stopped_after,
        }),
        (cjk, latin) => {
            let missing: Vec<_> = [cjk.is_none(), latin.is_none()]
                .into_iter()
                .zip(ROLES)
                .filter_map(|(missing, role)| missing.then_some(role))
                .collect();
            let searched = if roots.is_empty() {
                "no directories".to_owned()
            } else {
                roots
                    .iter()
                    .map(|root| format!("'{}'", root.display()))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let stopped = stopped_after
                .map(|entries| format!(" (stopped after {entries} entries)"))
                .unwrap_or_default();
            Err(format!(
                "this document has native C8/HN-B text, and no known installed {} font \
                 was found in {searched}{stopped}; install one listed in docs/cli.md \
                 (Installed fonts), or pass --fonts DIR or --font-cjk FILE --font-latin FILE \
                 (with --no-system-fonts, an HN-B document whose pages all have images \
                 converts as images without its text)",
                missing.join(" or ")
            ))
        }
    }
}

#[cfg(all(test, unix))]
mod tests;
