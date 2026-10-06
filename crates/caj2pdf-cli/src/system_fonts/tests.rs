// SPDX-License-Identifier: MIT

//! Discovery over temporary directory trees of original synthetic fonts; the
//! host's installed fonts are never read.

use super::*;
use crate::tests::TempDir;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;

#[allow(dead_code)]
mod original_font {
    include!("../../../caj2pdf-core/tests/common/font_fixture.rs");
}
use original_font::{collection, named_font as named};

fn wanted(names: &'static [&'static str]) -> impl Fn(&OsStr) -> bool {
    move |name| {
        name.to_str()
            .is_some_and(|name| names.iter().any(|wanted| wanted.eq_ignore_ascii_case(name)))
    }
}

fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&'static str) -> Option<OsString> + 'a {
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
    let tree = TempDir::new("fonts");
    let names = wanted(&["a.ttf", "b.ttf"]);
    let root = tree.0.join("root");
    // Entries are visited in name order, files of a directory before the
    // contents of later subdirectories; names match without case.
    tree.write("root/z/a.ttf", b"");
    tree.write("root/B.TTF", b"");
    tree.write("root/a.ttf", b"");
    tree.write("root/m/b.ttf", b"");
    tree.write("root/other.ttf", b"");
    // A wanted name that is a directory or a dangling link is not a file;
    // a link to a file is.
    fs::create_dir_all(tree.0.join("root/dir/b.ttf")).unwrap();
    fs::create_dir_all(tree.0.join("root/dangling")).unwrap();
    symlink(tree.0.join("missing"), tree.0.join("root/dangling/a.ttf")).unwrap();
    tree.write("outside/b.ttf", b"");
    fs::create_dir_all(tree.0.join("root/link")).unwrap();
    symlink(tree.0.join("outside/b.ttf"), tree.0.join("root/link/b.ttf")).unwrap();
    // A directory link (here a cycle back to the root) is not followed.
    symlink(&root, tree.0.join("root/m/loop")).unwrap();
    symlink(tree.0.join("outside"), tree.0.join("root/m/out")).unwrap();
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
    symlink(&root, tree.0.join("alias")).unwrap();
    let roots = [
        tree.0.join("absent"),
        tree.0.join("root/a.ttf"),
        root.clone(),
        tree.0.join("alias"),
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
    let top = tree.0.join("deep");
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
    let tree = TempDir::new("fonts");
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
    let roots = [tree.0.join("a"), tree.0.join("b"), tree.0.join("c")];
    let installed = discover(&roots, &limits).unwrap();
    assert_eq!(
        installed,
        Installed {
            choices: [
                Choice {
                    path: tree.0.join("a/NotoSerifCJK-Regular.ttc"),
                    face: 1,
                    postscript: "NotoSerifCJKsc-Regular",
                },
                Choice {
                    path: tree.0.join("c/LiberationSerif-Regular.ttf"),
                    face: 0,
                    postscript: "LiberationSerif",
                },
            ],
            stopped_after: None,
        }
    );
    assert_eq!(
        installed.report(),
        format!(
            "caj2pdf: using installed CJK font {}#1 (NotoSerifCJKsc-Regular)\n\
             caj2pdf: using installed Latin font {} (LiberationSerif)\n",
            tree.0.join("a/NotoSerifCJK-Regular.ttc").display(),
            tree.0.join("c/LiberationSerif-Regular.ttf").display()
        )
    );
    // List order, not root order, picks between installed faces.
    tree.write("a/FreeSerif.ttf", &named("FreeSerif"));
    let installed = discover(&[tree.0.join("c"), tree.0.join("a")], &limits).unwrap();
    assert_eq!(installed.choices[1].postscript, "FreeSerif");
    fs::remove_file(tree.0.join("a/NotoSerifCJK-Regular.ttc")).unwrap();
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
    assert_eq!(find_face(&tree.0.join("d/absent.ttc"), "x", &limits), None);
}

#[test]
fn missing_roles_name_the_searched_directories_and_the_options() {
    let tree = TempDir::new("fonts");
    let limits = Limits::default();
    let message = discover(&[], &limits).unwrap_err();
    assert!(message.contains("no known installed CJK or Latin font was found in no directories"));
    assert!(message.contains("--fonts DIR or --font-cjk FILE --font-latin FILE"));
    tree.write("fonts/FreeSerif.ttf", &named("FreeSerif"));
    let message = discover(&[tree.0.join("fonts"), tree.0.join("none")], &limits).unwrap_err();
    assert!(
        message.contains(&format!(
            "no known installed CJK font was found in '{}', '{}';",
            tree.0.join("fonts").display(),
            tree.0.join("none").display()
        )),
        "{message}"
    );
    tree.write("fonts/simsun.ttc", &named("SimSun"));
    discover(&[tree.0.join("fonts")], &limits).unwrap();
    // The entry bound is reported with the result: the directory holds
    // two fonts and three other entries, read in name order.
    for name in ["0", "1", "2"] {
        tree.write(&format!("fonts/{name}"), b"");
    }
    let message = discover_with(&[tree.0.join("fonts")], &limits, 4).unwrap_err();
    assert!(message.contains("(stopped after 4 entries)"), "{message}");
    assert!(
        message.contains("with --no-system-fonts, an HN-B document"),
        "{message}"
    );
    let installed = discover_with(&[tree.0.join("fonts")], &limits, 5).unwrap();
    assert_eq!(installed.stopped_after, None);
    let installed = Installed {
        stopped_after: Some(4),
        ..installed
    };
    assert!(
        installed
            .report()
            .ends_with("caj2pdf: note: the font search stopped after 4 directory entries\n")
    );
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
