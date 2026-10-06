// SPDX-License-Identifier: MIT

//! Discovery over temporary directory trees of original synthetic fonts; the
//! host's installed fonts are never read.

use super::*;
use crate::files::NEXT_TEMP;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;
use std::sync::atomic::Ordering;

const GEOMETRIC: &[u8] = include_bytes!("../../../../tests/fonts/geometric.ttf");

struct Tree(PathBuf);

impl Tree {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "caj2pdf-cli-fonts-{}-{}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.path(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn get32(bytes: &[u8], at: usize) -> usize {
    u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize
}

/// The original geometric fixture with PostScript name `postscript`: its
/// tables are copied and the `name` table is replaced.
fn named(postscript: &str) -> Vec<u8> {
    let count = u16::from_be_bytes([GEOMETRIC[4], GEOMETRIC[5]]) as usize;
    let mut tables: Vec<([u8; 4], Vec<u8>)> = (0..count)
        .map(|index| {
            let entry = 12 + 16 * index;
            let (offset, length) = (get32(GEOMETRIC, entry + 8), get32(GEOMETRIC, entry + 12));
            let tag = GEOMETRIC[entry..entry + 4].try_into().unwrap();
            (tag, GEOMETRIC[offset..offset + length].to_vec())
        })
        .collect();
    let mut name = Vec::new();
    for value in [0, 1, 18, 3, 1, 0x409, 6, 2 * postscript.len() as u16, 0] {
        name.extend(value.to_be_bytes());
    }
    name.extend(postscript.encode_utf16().flat_map(u16::to_be_bytes));
    tables
        .iter_mut()
        .find(|table| &table.0 == b"name")
        .unwrap()
        .1 = name;
    let mut font = GEOMETRIC[..12 + 16 * count].to_vec();
    for (index, (_, bytes)) in tables.iter().enumerate() {
        let entry = 12 + 16 * index;
        let offset = font.len() as u32;
        font[entry + 8..entry + 12].copy_from_slice(&offset.to_be_bytes());
        font[entry + 12..entry + 16].copy_from_slice(&(bytes.len() as u32).to_be_bytes());
        font.extend(bytes);
        font.resize(font.len().next_multiple_of(4), 0);
    }
    font
}

/// A collection whose face `i` is `fonts[order[i]]`; faces may share bytes.
fn collection(fonts: &[Vec<u8>], order: &[usize]) -> Vec<u8> {
    let mut bytes = b"ttcf\0\x01\0\0".to_vec();
    bytes.extend((order.len() as u32).to_be_bytes());
    bytes.resize(12 + 4 * order.len(), 0);
    let mut bases = Vec::new();
    for font in fonts {
        let base = bytes.len();
        bases.push(base);
        let mut font = font.clone();
        let count = u16::from_be_bytes([font[4], font[5]]) as usize;
        for table in 0..count {
            let at = 12 + 16 * table + 8;
            let offset = (get32(&font, at) + base) as u32;
            font[at..at + 4].copy_from_slice(&offset.to_be_bytes());
        }
        bytes.extend(font);
    }
    for (face, font) in order.iter().enumerate() {
        bytes[12 + 4 * face..16 + 4 * face].copy_from_slice(&(bases[*font] as u32).to_be_bytes());
    }
    bytes
}

fn wanted(names: &[&str]) -> HashSet<String> {
    names.iter().map(|name| name.to_ascii_lowercase()).collect()
}

fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
    move |name| {
        pairs
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| OsString::from(value))
    }
}

fn paths(values: &[&str]) -> Vec<PathBuf> {
    values.iter().map(PathBuf::from).collect()
}

#[test]
fn platform_roots_follow_the_environment_in_a_fixed_order() {
    assert_eq!(
        roots(Platform::Unix, env(&[("HOME", "/home/u")])),
        paths(&[
            "/home/u/.local/share/fonts",
            "/home/u/.fonts",
            "/usr/local/share/fonts",
            "/usr/share/fonts",
        ])
    );
    // Relative XDG values are ignored and repeated directories are dropped.
    let unix = [
        ("HOME", "relative"),
        ("XDG_DATA_HOME", "/data"),
        ("XDG_DATA_DIRS", "/a:relative:/data:/a"),
    ];
    assert_eq!(
        roots(Platform::Unix, env(&unix)),
        paths(&["/data/fonts", "/a/fonts"])
    );
    assert_eq!(
        roots(Platform::Unix, env(&[("XDG_DATA_DIRS", "")])),
        paths(&["/usr/local/share/fonts", "/usr/share/fonts"])
    );
    assert_eq!(
        roots(Platform::MacOs, env(&[("HOME", "/Users/u")])),
        paths(&[
            "/Users/u/Library/Fonts",
            "/Library/Fonts",
            "/System/Library/Fonts"
        ])
    );
    assert_eq!(
        roots(Platform::MacOs, env(&[])),
        paths(&["/Library/Fonts", "/System/Library/Fonts"])
    );
    let windows = [("LOCALAPPDATA", "/local"), ("WINDIR", "/win")];
    assert_eq!(
        roots(Platform::Windows, env(&windows)),
        [
            Path::new("/local").join("Microsoft\\Windows\\Fonts"),
            Path::new("/win").join("Fonts")
        ]
    );
    assert_eq!(
        roots(Platform::Windows, env(&[("SystemRoot", "/root")])),
        [Path::new("/root").join("Fonts")]
    );
    assert_eq!(
        roots(Platform::Windows, env(&[])),
        [Path::new("C:\\Windows").join("Fonts")]
    );
    // The override replaces every platform directory; empty searches none.
    for platform in [Platform::Unix, Platform::MacOs, Platform::Windows] {
        let custom = [(DIRS_VARIABLE, "/x:relative:/y:/x"), ("HOME", "/home/u")];
        assert_eq!(roots(platform, env(&custom)), paths(&["/x", "/y"]));
        assert!(roots(platform, env(&[(DIRS_VARIABLE, "")])).is_empty());
    }
}

#[test]
fn walks_are_sorted_bounded_and_never_follow_directory_links() {
    let tree = Tree::new();
    let names = wanted(&["a.ttf", "b.ttf"]);
    let root = tree.path("root");
    // Entries are visited in name order, files of a directory before the
    // contents of later subdirectories; names match without case.
    tree.write("root/z/a.ttf", b"");
    tree.write("root/B.TTF", b"");
    tree.write("root/a.ttf", b"");
    tree.write("root/m/b.ttf", b"");
    tree.write("root/other.ttf", b"");
    // A wanted name that is a directory or a dangling link is not a file;
    // a link to a file is.
    fs::create_dir_all(tree.path("root/dir/b.ttf")).unwrap();
    fs::create_dir_all(tree.path("root/dangling")).unwrap();
    symlink(tree.path("missing"), tree.path("root/dangling/a.ttf")).unwrap();
    tree.write("outside/b.ttf", b"");
    fs::create_dir_all(tree.path("root/link")).unwrap();
    symlink(tree.path("outside/b.ttf"), tree.path("root/link/b.ttf")).unwrap();
    // A directory link (here a cycle back to the root) is not followed.
    symlink(&root, tree.path("root/m/loop")).unwrap();
    symlink(tree.path("outside"), tree.path("root/m/out")).unwrap();
    // A name that is not Unicode never matches.
    let raw = root.join(OsStr::from_bytes(b"\xff.ttf"));
    fs::write(&raw, b"").unwrap();
    let expected = paths(&[
        "root/B.TTF",
        "root/a.ttf",
        "root/link/b.ttf",
        "root/m/b.ttf",
        "root/z/a.ttf",
    ]);
    let expected: Vec<_> = expected.iter().map(|p| tree.0.join(p)).collect();
    // A missing root, a file root and a repeated root (also through a link)
    // add nothing.
    symlink(&root, tree.path("alias")).unwrap();
    let roots = [
        tree.path("absent"),
        tree.path("root/a.ttf"),
        root.clone(),
        tree.path("alias"),
    ];
    let found = walk(&roots, &names, MAX_ENTRIES);
    assert_eq!(
        found,
        Walk {
            files: expected.clone(),
            truncated: false
        }
    );
    // The entry bound stops the walk deterministically, counting every
    // entry read, wanted or not.
    let bounded = walk(std::slice::from_ref(&root), &names, 11);
    assert!(bounded.truncated);
    assert_eq!(bounded.files, expected[..2]);
    assert_eq!(
        walk(std::slice::from_ref(&root), &names, 13).files,
        expected[..3]
    );
    // A root and MAX_DEPTH levels below it are entered; deeper ones are not.
    let top = tree.path("deep");
    let mut deep = top.clone();
    let mut entered = Vec::new();
    for level in 0..=MAX_DEPTH + 1 {
        fs::create_dir(&deep).unwrap();
        fs::write(deep.join("a.ttf"), b"").unwrap();
        if level <= MAX_DEPTH {
            entered.push(deep.join("a.ttf"));
        }
        deep = deep.join("d");
    }
    assert_eq!(walk(&[top], &names, MAX_ENTRIES).files, entered);
}

#[test]
fn faces_are_matched_by_postscript_name_in_list_order() {
    let tree = Tree::new();
    let limits = Limits::default();
    let jp = named("NotoSerifCJKjp-Regular");
    let sc = named("NotoSerifCJKsc-Regular");
    tree.write(
        "a/NotoSerifCJK-Regular.ttc",
        &collection(&[jp.clone(), sc.clone()], &[0, 1]),
    );
    tree.write(
        "b/wqy-zenhei.ttc",
        &collection(&[named("WenQuanYiZenHei")], &[0]),
    );
    // A listed file name with another face or invalid bytes is skipped.
    tree.write("a/FreeSerif.ttf", &named("NotFreeSerif"));
    tree.write("b/DejaVuSans.ttf", b"not a font");
    tree.write("c/LiberationSerif-Regular.ttf", &named("LiberationSerif"));
    let roots = [tree.path("a"), tree.path("b"), tree.path("c")];
    let installed = discover(&roots, &limits).unwrap();
    assert_eq!(
        installed,
        Installed {
            choices: [
                Choice {
                    path: tree.path("a/NotoSerifCJK-Regular.ttc"),
                    face: 1,
                    postscript: "NotoSerifCJKsc-Regular",
                },
                Choice {
                    path: tree.path("c/LiberationSerif-Regular.ttf"),
                    face: 0,
                    postscript: "LiberationSerif",
                },
            ],
            truncated: false,
        }
    );
    assert_eq!(
        installed.report(),
        format!(
            "caj2pdf: using installed CJK font {}#1 (NotoSerifCJKsc-Regular)\n\
             caj2pdf: using installed Latin font {} (LiberationSerif)\n",
            tree.path("a/NotoSerifCJK-Regular.ttc").display(),
            tree.path("c/LiberationSerif-Regular.ttf").display()
        )
    );
    // List order, not root order, picks between installed faces.
    tree.write("a/FreeSerif.ttf", &named("FreeSerif"));
    let installed = discover(&[tree.path("c"), tree.path("a")], &limits).unwrap();
    assert_eq!(installed.choices[1].postscript, "FreeSerif");
    fs::remove_file(tree.path("a/NotoSerifCJK-Regular.ttc")).unwrap();
    let installed = discover(&roots, &limits).unwrap();
    assert_eq!(installed.choices[0].postscript, "WenQuanYiZenHei");
    assert_eq!(installed.choices[0].face, 0);
    // At most MAX_FACES faces of one collection are checked.
    let faces = |count: usize| {
        let mut order = vec![0; count];
        order[count - 1] = 1;
        collection(&[jp.clone(), sc.clone()], &order)
    };
    let file = tree.write("d/NotoSerifCJK-Regular.ttc", &faces(MAX_FACES as usize));
    assert_eq!(
        find_face(&file, "NotoSerifCJKsc-Regular", &limits),
        Some(MAX_FACES - 1)
    );
    fs::write(&file, faces(MAX_FACES as usize + 1)).unwrap();
    assert_eq!(find_face(&file, "NotoSerifCJKsc-Regular", &limits), None);
    assert_eq!(find_face(&tree.path("d/absent.ttc"), "x", &limits), None);
}

#[test]
fn missing_roles_name_the_searched_directories_and_the_options() {
    let tree = Tree::new();
    let limits = Limits::default();
    let message = discover(&[], &limits).unwrap_err();
    assert!(message.contains("no known installed CJK or Latin font was found in no directories"));
    assert!(message.contains("--fonts DIR or --font-cjk FILE --font-latin FILE"));
    tree.write("fonts/FreeSerif.ttf", &named("FreeSerif"));
    let message = discover(&[tree.path("fonts"), tree.path("none")], &limits).unwrap_err();
    assert!(
        message.contains(&format!(
            "no known installed CJK font was found in '{}', '{}';",
            tree.path("fonts").display(),
            tree.path("none").display()
        )),
        "{message}"
    );
    tree.write("fonts/simsun.ttc", &named("SimSun"));
    discover(&[tree.path("fonts")], &limits).unwrap();
    // The entry bound is reported with the result.
    for index in 0..MAX_ENTRIES {
        fs::write(tree.path(&format!("fonts/{index:05}")), b"").unwrap();
    }
    let message = discover(&[tree.path("fonts")], &limits).unwrap_err();
    assert!(
        message.contains(&format!("(stopped after {MAX_ENTRIES} entries)")),
        "{message}"
    );
    let installed = Installed {
        choices: [
            Choice {
                path: "/c.ttf".into(),
                face: 0,
                postscript: "SimSun",
            },
            Choice {
                path: "/l.ttf".into(),
                face: 0,
                postscript: "FreeSerif",
            },
        ],
        truncated: true,
    };
    assert!(installed.report().ends_with(&format!(
        "caj2pdf: note: the font search stopped after {MAX_ENTRIES} directory entries\n"
    )));
}

#[test]
fn every_listed_file_name_is_a_font_file_name() {
    for face in CJK.iter().chain(&LATIN) {
        assert!(!face.files.is_empty());
        for file in face.files {
            let extension = file.rsplit_once('.').unwrap().1.to_ascii_lowercase();
            assert!(
                ["ttf", "otf", "ttc", "otc"].contains(&extension.as_str()),
                "{file}"
            );
        }
    }
}
