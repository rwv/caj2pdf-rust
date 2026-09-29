// SPDX-License-Identifier: MIT

//! Process-level tests of the `caj2pdf` executable. Inputs are synthetic:
//! small CAJ, KDH, C8, and HN containers are built here from the layouts in
//! `docs/caj-format.md`, `docs/kdh-format.md`, and `docs/hnc8-container.md`,
//! and PDFs come from the MIT fixtures in `tests/fixtures`.

#![cfg(unix)]

use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT_DIR: AtomicU32 = AtomicU32::new(0);

/// A scratch directory holding the test files and a private `TMPDIR`.
struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "caj2pdf-cli-{label}-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(path.join("tmp")).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.path(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    /// Entries other than `tmp`, which must stay empty.
    fn entries(&self) -> Vec<String> {
        assert_eq!(fs::read_dir(self.path("tmp")).unwrap().count(), 0);
        let mut names: Vec<_> = fs::read_dir(&self.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name != "tmp")
            .collect();
        names.sort();
        names
    }

    fn command<I, S>(&self, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = Command::new(env!("CARGO_BIN_EXE_caj2pdf"));
        command
            .args(args)
            .current_dir(&self.0)
            .env("TMPDIR", self.path("tmp"))
            .stdin(Stdio::null());
        command
    }

    fn run<I, S>(&self, args: I) -> Output
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.command(args).output().unwrap()
    }

    fn run_with_stdin(&self, args: &[&str], input: &[u8]) -> Output {
        let mut child = self
            .command(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // The child may exit before consuming all input; ignore EPIPE.
        let _ = child.stdin.take().unwrap().write_all(input);
        child.wait_with_output().unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[track_caller]
fn assert_success(output: &Output) {
    assert_eq!(output.status.code(), Some(0), "{}", stderr(output));
    assert!(output.stderr.is_empty(), "{}", stderr(output));
}

#[track_caller]
fn assert_failure(output: &Output, code: i32, message: &str) {
    assert_eq!(output.status.code(), Some(code), "{}", stderr(output));
    assert!(output.stdout.is_empty());
    let text = stderr(output);
    assert!(text.starts_with("caj2pdf: error: "), "{text}");
    assert!(text.contains(message), "expected {message:?} in {text}");
}

fn fixture(name: &str) -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name),
    )
    .unwrap()
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

/// A three-page CAJ whose outline is `(title, one-based page, level)`.
fn caj(outline: &[(&[u8], u8, u32)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (number, dictionary) in [
        (
            9,
            "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 200 100] /Resources << >> >>",
        ),
        (
            3,
            "<< /Type /Page /Parent 6 0 R /MediaBox [0 0 400 250] /Resources << >> >>",
        ),
        (
            4,
            "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 300 150] /Resources << >> >>",
        ),
        (
            5,
            "<< /Type /Pages /Parent 8 0 R /Count 2 /Kids [9 0 R 4 0 R] >>",
        ),
        (6, "<< /Type /Pages /Parent 8 0 R /Count 1 /Kids [3 0 R] >>"),
    ] {
        body.extend_from_slice(format!("{number} 0 obj\n{dictionary}\nendobj\n").as_bytes());
    }
    let table = (0x114 + outline.len() * 308).max(0x600);
    let body_start = table + 3 * 12;
    let mut bytes = vec![0_u8; body_start];
    bytes[..4].copy_from_slice(b"CAJ\0");
    put_u32(&mut bytes, 0x10, 3);
    put_u32(&mut bytes, 0x14, table as u32);
    put_u32(&mut bytes, 0x110, outline.len() as u32);
    for (index, (title, page, level)) in outline.iter().enumerate() {
        let record = 0x114 + index * 308;
        bytes[record..record + title.len()].copy_from_slice(title);
        bytes[record + 280] = b'0' + page;
        put_u32(&mut bytes, record + 304, *level);
    }
    let body_end = body_start + body.len();
    for (index, (offset, length, object)) in [
        (body_start, body.len(), 9),
        (body_end, 0, 4),
        (body_end, 0, 3),
    ]
    .into_iter()
    .enumerate()
    {
        let row = table + index * 12;
        put_u32(&mut bytes, row, offset as u32);
        put_u32(&mut bytes, row + 4, length as u32);
        put_u32(&mut bytes, row + 8, object);
    }
    bytes.extend_from_slice(&body);
    bytes
}

const OUTLINE: &[(&[u8], u8, u32)] = &[
    // "中文" in GB18030, then a nested entry and a second root.
    (b"\xd6\xd0\xce\xc4", 1, 1),
    (b"Nested \"q\"", 2, 2),
    (b"Third", 3, 1),
];

fn kdh(pdf: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0_u8; 254];
    bytes[..32].copy_from_slice(b"KDH 2.00 Copyright(C) 2000 CAJCD");
    bytes[0x28..0x2c].copy_from_slice(&[0, 0, 2, 0]);
    for (index, byte) in pdf.iter().enumerate() {
        bytes.push(byte ^ b"FZHMEI"[index % 6]);
    }
    bytes
}

fn c8() -> Vec<u8> {
    let mut bytes = vec![0_u8; 0x50 + 20];
    bytes[..4].copy_from_slice(b"\xc8\0\0\0");
    put_u32(&mut bytes, 0x08, 1);
    bytes
}

fn hn() -> Vec<u8> {
    let mut bytes = vec![0_u8; 0xd8 + 2 * 20];
    bytes[..8].copy_from_slice(b"HN\0\0\xc8\0\0\0");
    put_u32(&mut bytes, 0x90, 2);
    bytes
}

fn tool(program: &str, args: &[&OsStr]) -> String {
    let output = Command::new(program).args(args).output().unwrap();
    assert!(output.status.success(), "{program}: {}", stderr(&output));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Check a PDF with qpdf and return its page count and MuPDF outline.
fn validate_pdf(path: &Path) -> (u32, String) {
    tool("qpdf", &[OsStr::new("--check"), path.as_os_str()]);
    let pages = tool("qpdf", &[OsStr::new("--show-npages"), path.as_os_str()]);
    let outline = tool(
        "mutool",
        &[OsStr::new("show"), path.as_os_str(), OsStr::new("outline")],
    );
    (pages.trim().parse().unwrap(), outline)
}

#[test]
fn help_and_version_print_to_stdout() {
    let scratch = Scratch::new("help");
    for args in [
        &["--help"][..],
        &["-h"],
        &["inspect", "--help"],
        &["add-bookmarks", "-h"],
    ] {
        let output = scratch.run(args);
        assert_success(&output);
        assert!(stdout(&output).contains("Usage:\n  caj2pdf "));
    }
    let output = scratch.run(["--version"]);
    assert_success(&output);
    assert_eq!(
        stdout(&output),
        format!("caj2pdf {}\n", env!("CARGO_PKG_VERSION"))
    );
    let output = scratch
        .command(["-V"])
        .stdout(File::create("/dev/full").unwrap())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("cannot write standard output"));
}

#[test]
fn argument_errors_exit_with_status_two() {
    let scratch = Scratch::new("usage");
    for (args, message) in [
        (&[][..], "missing INPUT"),
        (&["--bogus", "a.caj"], "unrecognized option '--bogus'"),
        (&["a.caj", "b.caj"], "unexpected argument 'b.caj'"),
        (
            &["add-bookmarks", "a.caj", "b.pdf"],
            "requires -o OUTPUT_PDF",
        ),
        (&["paper.pdf"], "already has a .pdf name"),
    ] {
        let output = scratch.run(args);
        assert_failure(&output, 2, message);
        assert!(stderr(&output).ends_with("\nTry 'caj2pdf --help' for more information.\n"));
    }
    // An unwritable standard error does not change the exit status.
    let status = scratch
        .command(["--bogus"])
        .stderr(File::create("/dev/full").unwrap())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(2));
    assert!(scratch.entries().is_empty());
}

#[test]
fn a_named_input_converts_to_its_sibling_pdf() {
    let scratch = Scratch::new("sibling");
    let input = scratch.write("paper.caj", &caj(OUTLINE));
    let output = scratch.run(["paper.caj"]);
    assert_success(&output);
    assert!(output.stdout.is_empty());
    assert_eq!(scratch.entries(), ["paper.caj", "paper.pdf"]);
    let (pages, outline) = validate_pdf(&scratch.path("paper.pdf"));
    assert_eq!(pages, 3);
    assert!(outline.contains("中文"), "{outline}");
    assert!(outline.contains("Nested"), "{outline}");
    assert_eq!(fs::read(input).unwrap(), caj(OUTLINE));

    // An explicit relative output and an existing target.
    let output = scratch.run(["paper.caj", "-o", "paper.pdf"]);
    assert_failure(&output, 1, "output 'paper.pdf' already exists; use --force");
    let before = fs::metadata(scratch.path("paper.pdf")).unwrap();
    let output = scratch.run(["--force", "paper.caj", "--output=paper.pdf"]);
    assert_success(&output);
    let after = fs::metadata(scratch.path("paper.pdf")).unwrap();
    assert_ne!(
        std::os::unix::fs::MetadataExt::ino(&before),
        std::os::unix::fs::MetadataExt::ino(&after),
        "--force must replace the file by rename"
    );
    assert_eq!(scratch.entries(), ["paper.caj", "paper.pdf"]);
}

#[test]
fn pdf_and_kdh_inputs_convert_to_pipes() {
    let scratch = Scratch::new("pipe");
    let pdf = fixture("valid_nested_outline.pdf");
    scratch.write("doc.pdf", &pdf);
    scratch.write("doc.kdh", &kdh(&pdf));
    for args in [&["doc.pdf", "-o", "-"][..], &["doc.kdh", "-o", "-"]] {
        let output = scratch.run(args);
        assert_success(&output);
        assert_eq!(output.stdout, pdf, "{args:?}");
    }
    assert_eq!(scratch.entries(), ["doc.kdh", "doc.pdf"]);
}

#[test]
fn stdin_is_spooled_and_removed() {
    let scratch = Scratch::new("stdin");
    let output = scratch.run_with_stdin(&["-"], &caj(OUTLINE));
    assert_success(&output);
    assert!(output.stdout.starts_with(b"%PDF-1.7"));
    let converted = scratch.write("converted.pdf", &output.stdout);
    assert_eq!(validate_pdf(&converted).0, 3);

    let output = scratch.run_with_stdin(&["inspect", "-", "--json"], &caj(OUTLINE));
    assert_success(&output);
    assert!(stdout(&output).contains("\"format\":\"CAJ\""));

    // A pipe given by path is spooled too.
    let output = scratch.run_with_stdin(&["/dev/stdin", "-o", "piped.pdf"], &caj(OUTLINE));
    assert_success(&output);
    assert_eq!(
        fs::read(scratch.path("piped.pdf")).unwrap(),
        fs::read(&converted).unwrap()
    );
    assert_eq!(scratch.entries(), ["converted.pdf", "piped.pdf"]);
}

#[test]
fn regular_file_stdin_is_read_in_place() {
    let scratch = Scratch::new("stdin-file");
    let input = scratch.write("paper.caj", &caj(OUTLINE));
    let output = scratch
        .command(["-", "-o", "out.pdf"])
        .stdin(File::open(&input).unwrap())
        .output()
        .unwrap();
    assert_success(&output);
    assert_eq!(validate_pdf(&scratch.path("out.pdf")).0, 3);
}

#[test]
fn inputs_are_never_overwritten() {
    let scratch = Scratch::new("same");
    let input = scratch.write("paper.caj", &caj(OUTLINE));
    std::os::unix::fs::symlink("paper.caj", scratch.path("link.pdf")).unwrap();
    fs::hard_link(&input, scratch.path("hard.pdf")).unwrap();
    for target in ["paper.caj", "./paper.caj", "link.pdf", "hard.pdf"] {
        let output = scratch.run(["paper.caj", "-o", target, "--force"]);
        assert_failure(&output, 1, "is the same file as input 'paper.caj'");
    }
    // Standard output appending to the input file is refused as well.
    let append = OpenOptions::new().append(true).open(&input).unwrap();
    let output = scratch
        .command(["paper.caj", "-o", "-"])
        .stdout(append)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("standard output is the same file as input"));
    assert_eq!(fs::read(&input).unwrap(), caj(OUTLINE));
    assert_eq!(scratch.entries(), ["hard.pdf", "link.pdf", "paper.caj"]);
}

#[test]
fn a_failing_stdout_sink_exits_with_status_one() {
    let scratch = Scratch::new("full");
    scratch.write("paper.caj", &caj(OUTLINE));
    let output = scratch
        .command(["paper.caj", "-o", "-"])
        .stdout(File::create("/dev/full").unwrap())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("No space left on device"),
        "{}",
        stderr(&output)
    );
    assert_eq!(scratch.entries(), ["paper.caj"]);
}

#[test]
fn malformed_and_unsupported_inputs_leave_no_output() {
    let scratch = Scratch::new("malformed");
    let mut truncated = caj(OUTLINE);
    truncated.truncate(0x700);
    scratch.write("truncated.caj", &truncated);
    scratch.write("empty.caj", b"");
    scratch.write("unknown.caj", b"GIF89a");
    scratch.write("c8.c8", &c8());
    scratch.write("hn.hn", &hn());
    scratch.write("short.hn", &fixture("truncated_hn.hn"));
    scratch.write("teb.teb", &fixture("truncated_teb.teb"));
    scratch.write("broken.pdf", &fixture("invalid_xref_offset.pdf"));
    fs::create_dir(scratch.path("folder.caj")).unwrap();
    for (input, message) in [
        ("truncated.caj", "cannot convert 'truncated.caj': "),
        ("empty.caj", "input is empty"),
        ("unknown.caj", "unrecognized input format"),
        ("c8.c8", "outlines are only validated for HN-A"),
        ("hn.hn", "outlines are only validated for HN-A"),
        // A malformed container is reported as such, not as unsupported.
        ("short.hn", "HN/C8 at byte 0: truncated"),
        (
            "teb.teb",
            "TEB input is recognized, but TEB conversion is not supported",
        ),
        ("broken.pdf", "PDF at byte"),
        ("folder.caj", "'folder.caj' is a directory"),
        ("missing.caj", "cannot read 'missing.caj'"),
    ] {
        let output = scratch.run([input, "-o", "out.pdf"]);
        assert_failure(&output, 1, message);
    }
    assert!(!scratch.path("out.pdf").exists());
    assert_eq!(
        scratch.entries(),
        [
            "broken.pdf",
            "c8.c8",
            "empty.caj",
            "folder.caj",
            "hn.hn",
            "short.hn",
            "teb.teb",
            "truncated.caj",
            "unknown.caj"
        ]
    );
    let output = scratch.run_with_stdin(&["-"], b"");
    assert_failure(&output, 1, "cannot convert standard input: input is empty");
    let output = scratch.run(["folder.caj/x", "-o", "missing/out.pdf"]);
    assert_failure(&output, 1, "cannot read");
}

#[test]
fn output_errors_are_reported() {
    let scratch = Scratch::new("output-errors");
    scratch.write("paper.caj", &caj(OUTLINE));
    fs::create_dir(scratch.path("dir.pdf")).unwrap();
    let output = scratch.run(["paper.caj", "-o", "dir.pdf", "--force"]);
    assert_failure(&output, 1, "output 'dir.pdf' is a directory");
    let output = scratch.run(["paper.caj", "-o", "missing/out.pdf"]);
    assert_failure(&output, 1, "cannot create a temporary file in 'missing'");
    assert_eq!(scratch.entries(), ["dir.pdf", "paper.caj"]);
}

#[test]
fn non_utf8_paths_are_used_unchanged() {
    let scratch = Scratch::new("bytes");
    let name = OsStr::from_bytes(b"caj\xff.caj");
    fs::write(scratch.0.join(name), caj(OUTLINE)).unwrap();
    let output = scratch.run([name]);
    assert_success(&output);
    let pdf = scratch.0.join(OsStr::from_bytes(b"caj\xff.pdf"));
    assert_eq!(validate_pdf(&pdf).0, 3);
    let output = scratch.run([OsString::from("inspect"), name.to_owned()]);
    assert_success(&output);
    let missing = OsStr::from_bytes(b"missing\xfe.caj");
    let output = scratch.run([missing]);
    assert_failure(&output, 1, "cannot read 'missing\u{fffd}.caj'");
}

#[test]
fn inspect_prints_text_and_stable_json() {
    let scratch = Scratch::new("inspect");
    scratch.write("paper.caj", &caj(OUTLINE));
    let output = scratch.run(["inspect", "paper.caj", "--bookmarks"]);
    assert_success(&output);
    assert_eq!(
        stdout(&output),
        "Format: CAJ\nConversion: supported\nPages: 3\nOutline: yes\nBookmarks: 3\n  \
         - 中文 (page 1)\n    - Nested \"q\" (page 2)\n  - Third (page 3)\n"
    );
    let output = scratch.run(["inspect", "--json", "--bookmarks", "paper.caj"]);
    assert_success(&output);
    assert_eq!(
        stdout(&output),
        concat!(
            r#"{"schema_version":1,"format":"CAJ","variant":null,"conversion_supported":true,"#,
            r#""page_count":3,"has_outline":true,"bookmark_count":3,"bookmarks":["#,
            r#"{"title":"中文","page":1,"children":[{"title":"Nested \"q\"","page":2,"children":[]}]},"#,
            r#"{"title":"Third","page":3,"children":[]}]}"#,
            "\n"
        )
    );
}

#[test]
fn inspect_reports_every_recognized_format() {
    let scratch = Scratch::new("inspect-formats");
    let pdf = fixture("valid_nested_outline.pdf");
    scratch.write("doc.pdf", &pdf);
    scratch.write("doc.kdh", &kdh(&pdf));
    scratch.write("doc.c8", &c8());
    scratch.write("doc.hn", &hn());
    scratch.write("doc.teb", &fixture("truncated_teb.teb"));
    let common = r#""conversion_supported":"#;
    for (input, expected) in [
        (
            "doc.pdf",
            format!(
                r#"{{"schema_version":1,"format":"PDF","variant":null,{common}true,"page_count":2,"has_outline":true,"bookmark_count":null,"bookmarks":null}}"#
            ),
        ),
        (
            "doc.kdh",
            format!(
                r#"{{"schema_version":1,"format":"KDH","variant":null,{common}true,"page_count":2,"has_outline":true,"bookmark_count":null,"bookmarks":null}}"#
            ),
        ),
        (
            "doc.c8",
            format!(
                r#"{{"schema_version":1,"format":"C8","variant":"C8",{common}true,"page_count":1,"has_outline":null,"bookmark_count":null,"bookmarks":null}}"#
            ),
        ),
        (
            "doc.hn",
            format!(
                r#"{{"schema_version":1,"format":"HN","variant":"HN-B",{common}true,"page_count":2,"has_outline":null,"bookmark_count":null,"bookmarks":null}}"#
            ),
        ),
        (
            "doc.teb",
            format!(
                r#"{{"schema_version":1,"format":"TEB","variant":null,{common}false,"page_count":null,"has_outline":null,"bookmark_count":null,"bookmarks":null}}"#
            ),
        ),
    ] {
        let output = scratch.run(["inspect", "--json", "--bookmarks", input]);
        assert_success(&output);
        assert_eq!(stdout(&output), expected + "\n");
    }
    let output = scratch.run(["inspect", "doc.hn"]);
    assert_success(&output);
    assert_eq!(
        stdout(&output),
        "Format: HN\nVariant: HN-B\nConversion: experimental (caller codec states may be required)\nPages: 2\nOutline: unknown\n"
    );
    scratch.write("bad.kdh", &kdh(b"not a pdf"));
    scratch.write("broken.kdh", &kdh(&fixture("truncated_xref.pdf")));
    scratch.write("empty", b"");
    scratch.write("bad.hn", &fixture("truncated_hn.hn"));
    scratch.write("bad.caj", b"CAJ\0");
    scratch.write("bad.pdf", &fixture("truncated_xref.pdf"));
    for (input, message) in [
        ("bad.kdh", "cannot inspect 'bad.kdh': "),
        ("broken.kdh", "cannot inspect 'broken.kdh': "),
        ("empty", "cannot inspect 'empty': input is empty"),
        ("bad.hn", "truncated signature"),
        ("bad.caj", "malformed CAJ"),
        ("bad.pdf", "cannot inspect 'bad.pdf': "),
        ("missing", "cannot read 'missing'"),
    ] {
        let output = scratch.run(["inspect", input]);
        assert_failure(&output, 1, message);
    }
}

#[test]
fn add_bookmarks_writes_a_new_pdf_and_keeps_the_input() {
    let scratch = Scratch::new("add");
    scratch.write("paper.caj", &caj(OUTLINE));
    scratch.write("plain.caj", &caj(&[]));
    assert_success(&scratch.run(["plain.caj"]));
    let plain = fs::read(scratch.path("plain.pdf")).unwrap();
    assert_eq!(validate_pdf(&scratch.path("plain.pdf")).1.trim(), "");

    let add = |output: &str, force: &[&str]| {
        let args = ["add-bookmarks", "paper.caj", "plain.pdf", "-o", output];
        scratch.run(args.iter().chain(force))
    };
    assert_success(&add("marked.pdf", &[]));
    assert_eq!(fs::read(scratch.path("plain.pdf")).unwrap(), plain);
    let (pages, outline) = validate_pdf(&scratch.path("marked.pdf"));
    assert_eq!(pages, 3);
    assert!(
        outline.contains("中文") && outline.contains("Third"),
        "{outline}"
    );

    assert_failure(&add("marked.pdf", &[]), 1, "already exists");
    assert_failure(
        &add("plain.pdf", &["--force"]),
        1,
        "is the same file as input 'plain.pdf'",
    );
    assert_failure(
        &add("paper.caj", &["-f"]),
        1,
        "is the same file as input 'paper.caj'",
    );

    // The PDF may come from standard input, and the result may go to stdout.
    let output = scratch
        .command(["add-bookmarks", "paper.caj", "-", "-o", "-"])
        .stdin(File::open(scratch.path("plain.pdf")).unwrap())
        .output()
        .unwrap();
    assert_success(&output);
    assert_eq!(output.stdout, fs::read(scratch.path("marked.pdf")).unwrap());
    assert_eq!(
        scratch.entries(),
        ["marked.pdf", "paper.caj", "plain.caj", "plain.pdf"]
    );
}

#[test]
fn add_bookmarks_rejects_unusable_inputs() {
    let scratch = Scratch::new("add-errors");
    scratch.write("paper.caj", &caj(OUTLINE));
    scratch.write("plain.caj", &caj(&[]));
    scratch.write("bad.caj", b"CAJ\0");
    scratch.write("hn.hn", &hn());
    scratch.write("outlined.pdf", &fixture("valid_nested_outline.pdf"));
    scratch.write("broken.pdf", &fixture("truncated_xref.pdf"));
    assert_success(&scratch.run(["plain.caj"]));
    // The appender rejects an outline deeper than 256 levels after it has
    // started writing, so the staged output must be removed.
    let levels: Vec<u32> = (1..=257).collect();
    let deep: Vec<(&[u8], u8, u32)> = levels
        .iter()
        .map(|level| (&b"Deep"[..], 1, *level))
        .collect();
    scratch.write("deep.caj", &caj(&deep));
    for (args, message) in [
        (
            ["hn.hn", "plain.pdf"],
            "'hn.hn': expected a CAJ outline source, found HN",
        ),
        (
            ["bad.caj", "plain.pdf"],
            "cannot read 'bad.caj': malformed CAJ",
        ),
        (
            ["plain.caj", "plain.pdf"],
            "'plain.caj' has no bookmarks to import",
        ),
        (
            ["paper.caj", "paper.caj"],
            "'paper.caj': expected a PDF, found CAJ",
        ),
        (["paper.caj", "broken.pdf"], "cannot read 'broken.pdf': "),
        (
            ["paper.caj", "outlined.pdf"],
            "'outlined.pdf' already has an outline",
        ),
        (
            ["deep.caj", "plain.pdf"],
            "cannot add bookmarks from 'deep.caj' to 'plain.pdf': ",
        ),
        (["missing.caj", "plain.pdf"], "cannot read 'missing.caj'"),
        (["paper.caj", "missing.pdf"], "cannot read 'missing.pdf'"),
        (
            ["/dev/null", "plain.pdf"],
            "cannot read '/dev/null': input is empty",
        ),
    ] {
        let output = scratch.run(["add-bookmarks", args[0], args[1], "-o", "out.pdf"]);
        assert_failure(&output, 1, message);
    }
    assert!(!scratch.path("out.pdf").exists());
    assert_eq!(
        scratch.entries(),
        [
            "bad.caj",
            "broken.pdf",
            "deep.caj",
            "hn.hn",
            "outlined.pdf",
            "paper.caj",
            "plain.caj",
            "plain.pdf"
        ]
    );
}

#[test]
fn an_unusable_spool_directory_is_reported() {
    let scratch = Scratch::new("spool-dir");
    let output = scratch
        .command(["-", "-o", "out.pdf"])
        .env("TMPDIR", scratch.path("missing"))
        .stdin(Stdio::piped())
        .output()
        .unwrap();
    assert_failure(&output, 1, "cannot read standard input: ");
    assert!(scratch.entries().is_empty());
}

#[test]
fn pdf_bytes_are_not_written_to_a_terminal() {
    // util-linux `script` runs the command with a pseudo-terminal as stdout.
    let scratch = Scratch::new("tty");
    scratch.write("paper.caj", &caj(OUTLINE));
    // `paper.pdf` does not exist: the refusal precedes opening any input.
    for args in ["paper.caj -o -", "add-bookmarks paper.caj paper.pdf -o -"] {
        let command = format!("'{}' {args}", env!("CARGO_BIN_EXE_caj2pdf"));
        let output = Command::new("script")
            .args(["-qec", &command, "/dev/null"])
            .current_dir(&scratch.0)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        let transcript = String::from_utf8_lossy(&output.stdout);
        assert!(
            transcript.contains("refusing to write binary PDF data to a terminal"),
            "{transcript}"
        );
        assert!(!transcript.contains("%PDF"));
    }
    assert_eq!(scratch.entries(), ["paper.caj"]);
}

/// Original 3x2 HN-A image shared in shape with the WASM synthetic fixture.
/// Its invented constant probability model emits 101/010, not normative states.
fn image_hn() -> Vec<u8> {
    let index = 0x15c;
    let text = index + 20;
    let descriptor = text + 32;
    let payload = descriptor + 12;
    let mut bytes = vec![0; payload + 49];
    bytes[..8].copy_from_slice(&[72, 78, 0, 0, 0x90, 1, 0, 0]);
    put_u32(&mut bytes, 0x90, 1);
    put_u32(&mut bytes, index, text as u32);
    put_u32(&mut bytes, index + 4, 32);
    bytes[index + 8] = 1;
    bytes[text..text + 2].copy_from_slice(&0x800au16.to_le_bytes());
    bytes[text + 28..text + 30].copy_from_slice(&0x8004u16.to_le_bytes());
    put_u32(&mut bytes, descriptor + 4, payload as u32);
    put_u32(&mut bytes, descriptor + 8, 49);
    put_u32(&mut bytes, payload, 40);
    put_u32(&mut bytes, payload + 4, 3);
    put_u32(&mut bytes, payload + 8, 2);
    bytes[payload + 12] = 1;
    bytes[payload + 14] = 1;
    put_u32(&mut bytes, payload + 32, 2);
    bytes[payload + 40..payload + 43].fill(255);
    bytes[payload + 48] = 0x92;
    bytes
}

#[test]
fn hn_converts_from_files_and_pipes_with_exact_pixels_and_no_named_scratch() {
    let scratch = Scratch::new("hn-image");
    let input = image_hn();
    scratch.write("input.hn", &input);
    scratch.write("qm.txt", "16384 0 0 0\n".repeat(113).as_bytes());
    scratch.write("mq.txt", "16384 0 0 0\n".repeat(47).as_bytes());
    assert_success(&scratch.run([
        "input.hn",
        "--qm-states",
        "qm.txt",
        "--mq-states=mq.txt",
        "-o",
        "out.pdf",
    ]));
    assert_eq!(validate_pdf(&scratch.path("out.pdf")).0, 1);
    let objects = tool(
        "mutool",
        &[
            OsStr::new("show"),
            scratch.path("out.pdf").as_os_str(),
            OsStr::new("pages/1/Resources/XObject"),
        ],
    );
    let object = objects
        .split_whitespace()
        .find_map(|word| word.parse::<u32>().ok())
        .unwrap();
    let pixels = Command::new("qpdf")
        .arg(format!("--show-object={object}"))
        .arg("--filtered-stream-data")
        .arg(scratch.path("out.pdf"))
        .output()
        .unwrap();
    assert_success(&pixels);
    assert_eq!(pixels.stdout, [0x40, 0, 0, 0, 0xa0, 0, 0, 0]);
    let pipe = scratch.run_with_stdin(&["-", "--qm-states=qm.txt", "--no-bookmarks"], &input);
    assert_success(&pipe);
    assert_eq!(pipe.stdout, fs::read(scratch.path("out.pdf")).unwrap());
    assert_eq!(
        scratch.entries(),
        ["input.hn", "mq.txt", "out.pdf", "qm.txt"]
    );
}

#[test]
fn hn_failures_preserve_inputs_and_existing_output_and_remove_temporary_files() {
    let scratch = Scratch::new("hn-fail");
    scratch.write("input.hn", &image_hn());
    let states = "16384 0 0 0\n".repeat(113);
    scratch.write("qm.txt", states.as_bytes());
    scratch.write("out.pdf", b"keep original output");
    let missing = scratch.run(["input.hn", "-o", "out.pdf", "--force"]);
    assert_failure(&missing, 1, "table");
    assert_eq!(
        fs::read(scratch.path("out.pdf")).unwrap(),
        b"keep original output"
    );
    for target in ["qm.txt", "state-hardlink"] {
        if target == "state-hardlink" {
            fs::hard_link(scratch.path("qm.txt"), scratch.path(target)).unwrap();
        }
        assert_failure(
            &scratch.run(["input.hn", "--qm-states=qm.txt", "-o", target, "--force"]),
            1,
            "input",
        );
        assert_eq!(fs::read(scratch.path("qm.txt")).unwrap(), states.as_bytes());
    }
    let failed = scratch
        .command(["input.hn", "--qm-states=qm.txt", "-o", "new.pdf"])
        .env("TMPDIR", scratch.path("missing"))
        .output()
        .unwrap();
    assert_failure(&failed, 1, "cannot create HN/C8 scratch");
    assert!(!scratch.path("new.pdf").exists());
    let mut empty = hn();
    put_u32(&mut empty, 0x90, 1);
    put_u32(&mut empty, 0xd8, 0xd8 + 40);
    scratch.write("empty.hn", &empty);
    assert_failure(
        &scratch.run(["empty.hn", "--no-bookmarks", "-o", "new.pdf"]),
        1,
        "cannot omit source pages",
    );
    assert_eq!(fs::read_dir(scratch.path("tmp")).unwrap().count(), 0);
}

#[test]
fn invalid_caller_tables_are_rejected_before_output_creation() {
    let scratch = Scratch::new("state-files");
    scratch.write("input.hn", &image_hn());
    for (flag, bytes) in [
        ("--qm-states", vec![255]),
        ("--qm-states", b"1 0\n".to_vec()),
        ("--qm-states", "0 0 0 0\n".repeat(113).into_bytes()),
        ("--mq-states", "32768 0 0 0\n".repeat(47).into_bytes()),
        ("--qm-states", vec![b' '; 16385]),
    ] {
        scratch.write("states.txt", &bytes);
        assert_failure(
            &scratch.run(["input.hn", flag, "states.txt", "-o", "out.pdf"]),
            1,
            "state",
        );
        assert!(!scratch.path("out.pdf").exists());
    }
    assert_failure(
        &scratch.run(["input.hn", "--mq-states=missing", "-o", "out.pdf"]),
        1,
        "cannot read",
    );
    assert_eq!(fs::read_dir(scratch.path("tmp")).unwrap().count(), 0);
}

#[test]
fn no_bookmarks_skips_caj_outline_import() {
    let scratch = Scratch::new("no-bookmarks");
    scratch.write("input.caj", &caj(OUTLINE));
    assert_success(&scratch.run(["input.caj", "--no-bookmarks"]));
    let (pages, outline) = validate_pdf(&scratch.path("input.pdf"));
    assert_eq!(pages, 3);
    assert!(outline.trim().is_empty(), "{outline}");
    assert_eq!(scratch.entries(), ["input.caj", "input.pdf"]);
}
