// SPDX-License-Identifier: MIT

//! Unit tests for argument parsing, output selection, JSON encoding, report
//! rendering, and the file helpers. Process behavior is in `tests/cli.rs`.

use crate::CliError;
use crate::cli::default_output;
use crate::command::{Command, Endpoint, parse};
use crate::document::unsupported;
use crate::files::{
    Input, NEXT_TEMP, Output, SpoolError, TEMP_ATTEMPTS, open_input, open_input_spooling_in,
    open_output, refuse_terminal, spool,
};
use crate::report::{
    Pages, write_application_info_warning, write_json, write_text, write_warnings,
};
use caj2pdf_core::{
    Bookmark, DocumentInfo as Inspection, InputFormat, Structure,
    hnc8::{
        ApplicationInfo, ApplicationInfoDefect, ApplicationInfoReport, ApplicationInfoStatus,
        ApplicationInfoTail, Header, ImageRecord, OutlineReport, PageRecord, Span, TextFraming,
        TextStructure, Variant,
    },
};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn path(value: &str) -> Endpoint {
    Endpoint::Path(PathBuf::from(value))
}

fn parse_str(values: &[&str]) -> Result<Command, String> {
    parse(args(values))
}

/// A temporary directory removed on drop; shared by the module tests.
pub(crate) struct TempDir(pub(crate) PathBuf);

impl TempDir {
    pub(crate) fn new(label: &str) -> Self {
        let counter = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "caj2pdf-cli-unit-{label}-{}-{counter}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    /// Write `bytes` to `name` below the directory, creating parents.
    pub(crate) fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }

    fn entries(&self) -> Vec<OsString> {
        let mut names: Vec<_> = fs::read_dir(&self.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        names
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn input(path: &Path) -> Input {
    let Ok(input) = open_input(&Endpoint::Path(path.to_owned()), u64::MAX) else {
        panic!("cannot open {}", path.display());
    };
    input
}

#[test]
fn conversion_arguments_accept_every_output_spelling() {
    assert_eq!(
        parse_str(&["paper.caj"]),
        Ok(Command::Convert {
            input: path("paper.caj"),
            output: None,
            force: false,
            options: Default::default()
        })
    );
    for spelling in [
        &["-o", "out.pdf", "paper.caj"][..],
        &["paper.caj", "--output", "out.pdf"],
        &["--output=out.pdf", "paper.caj"],
    ] {
        assert_eq!(
            parse_str(spelling),
            Ok(Command::Convert {
                input: path("paper.caj"),
                output: Some(path("out.pdf")),
                force: false,
                options: Default::default()
            })
        );
    }
    assert_eq!(
        parse_str(&["-", "-o", "-", "-f", "--force"]),
        Ok(Command::Convert {
            input: Endpoint::Std,
            output: Some(Endpoint::Std),
            force: true,
            options: Default::default()
        })
    );
    assert_eq!(
        parse_str(&["--", "-name.caj"]),
        Ok(Command::Convert {
            input: path("-name.caj"),
            output: None,
            force: false,
            options: Default::default()
        })
    );
    assert_eq!(
        parse_str(&["./inspect"]),
        Ok(Command::Convert {
            input: path("./inspect"),
            output: None,
            force: false,
            options: Default::default()
        })
    );
}

#[test]
fn subcommands_parse_their_own_options() {
    assert_eq!(
        parse_str(&["inspect", "--bookmarks", "a.caj", "--json"]),
        Ok(Command::Inspect {
            input: path("a.caj"),
            json: true,
            bookmarks: true,
            pages: false
        })
    );
    assert_eq!(
        parse_str(&["inspect", "-"]),
        Ok(Command::Inspect {
            input: Endpoint::Std,
            json: false,
            bookmarks: false,
            pages: false
        })
    );
    assert_eq!(
        parse_str(&["inspect", "--pages", "a.hn"]),
        Ok(Command::Inspect {
            input: path("a.hn"),
            json: false,
            bookmarks: false,
            pages: true
        })
    );
    assert_eq!(
        parse_str(&["add-bookmarks", "a.caj", "-", "-o", "b.pdf", "--force"]),
        Ok(Command::AddBookmarks {
            outline: path("a.caj"),
            pdf: Endpoint::Std,
            output: path("b.pdf"),
            force: true
        })
    );
}

fn help(values: &[&str]) -> String {
    match parse_str(values) {
        Ok(Command::Help(text)) => text,
        other => panic!("{values:?}: expected help, got {other:?}"),
    }
}

#[test]
fn help_and_version_win_after_valid_arguments() {
    assert_eq!(help(&["--help"]), help(&["x", "-h"]));
    assert!(help(&["inspect", "--help"]).contains("caj2pdf inspect INPUT"));
    assert!(help(&["add-bookmarks", "-h"]).contains("caj2pdf add-bookmarks SOURCE_CAJ"));
    assert_eq!(parse_str(&["-V"]), Ok(Command::Version));
    assert_eq!(parse_str(&["inspect", "--version"]), Ok(Command::Version));
    assert_eq!(parse_str(&["add-bookmarks", "-V"]), Ok(Command::Version));
}

/// Every option and positional argument documented in `docs/cli.md`
/// appears in the help of its command.
#[test]
fn help_lists_every_documented_option() {
    let cases: &[(&[&str], &[&str])] = &[
        (
            &["--help"],
            &[
                "Usage: caj2pdf INPUT [-o OUTPUT] [--force]",
                "caj2pdf inspect INPUT [--json] [--bookmarks] [--pages]",
                "caj2pdf add-bookmarks SOURCE_CAJ INPUT_PDF -o OUTPUT_PDF [--force]",
                "inspect",
                "add-bookmarks",
                "<INPUT>",
                "-o, --output <OUTPUT>",
                "-f, --force",
                "-q, --quiet",
                "--allow-damaged",
                "--no-bookmarks",
                "--fonts <DIR>",
                "--font-cjk <FILE>",
                "--font-latin <FILE>",
                "--font-alternate-latin <FILE>",
                "--font-latin-state3 <FILE>",
                "--font-latin-state28 <FILE>",
                "--font-latin-state31 <FILE>",
                "--font-symbols <FILE>",
                "--font-decoration <FILE>",
                "--decoration-char <CHAR>",
                "--no-system-fonts",
                "-h, --help",
                "-V, --version",
                "Exit status: 0 on success, 3 for a partial PDF with blank pages,\n\
                 2 for invalid arguments, 1 for other failures.",
            ],
        ),
        (
            &["inspect", "--help"],
            &["<INPUT>", "--json", "--bookmarks", "--pages", "-h, --help"],
        ),
        (
            &["add-bookmarks", "--help"],
            &[
                "<SOURCE_CAJ>",
                "<INPUT_PDF>",
                "-o, --output <OUTPUT_PDF>",
                "-f, --force",
                "-h, --help",
                "At most one input may be -.",
            ],
        ),
    ];
    for (command, expected) in cases {
        let text = help(command);
        for option in *expected {
            assert!(
                text.contains(option),
                "{command:?} lacks {option:?}:\n{text}"
            );
        }
    }
    // Conversion options belong to conversion only.
    let inspect = help(&["inspect", "--help"]);
    assert!(!inspect.contains("--force") && !inspect.contains("--fonts"));
}

#[test]
fn invalid_arguments_are_usage_errors() {
    let cases: &[(&[&str], &str)] = &[
        (&[], "missing INPUT"),
        (&["inspect"], "missing INPUT"),
        (&["a", "b"], "unexpected argument 'b'"),
        (&["add-bookmarks", "a"], "requires SOURCE_CAJ and INPUT_PDF"),
        (&["add-bookmarks", "a", "b", "c"], "unexpected argument 'c'"),
        (&["add-bookmarks", "a", "b"], "requires -o OUTPUT_PDF"),
        (&["add-bookmarks", "-", "-", "-o", "x"], "only one input"),
        (&["a", "-o"], "option '--output' requires a value"),
        (&["a", "-o", "x", "--output=y"], "more than once"),
        (&["a", "--json"], "unrecognized option '--json'"),
        (&["inspect", "a", "-o", "x"], "unrecognized option '-o'"),
        (
            &["inspect", "a", "--output=x"],
            "unrecognized option '--output'",
        ),
        (&["inspect", "a", "b"], "unexpected argument 'b'"),
        (&["-q", "inspect", "a"], "unexpected argument 'a'"),
        (
            &["a", "--force=yes"],
            "unexpected value 'yes' for '--force'",
        ),
        (
            &["inspect", "a", "--force"],
            "unrecognized option '--force'",
        ),
        (&["-x", "a"], "unrecognized option '-x'"),
        (&[""], "empty path argument"),
        (&["a", "-o", ""], "empty path argument"),
    ];
    for (values, message) in cases {
        let error = parse_str(values).unwrap_err();
        assert!(error.contains(message), "{values:?}: {error}");
    }
    let invalid = OsString::from_vec(b"--\xff".to_vec());
    assert_eq!(
        parse(vec![invalid]),
        Err("unrecognized option '--\u{fffd}'".to_owned())
    );
}

#[test]
fn non_utf8_positional_paths_are_preserved() {
    let name = OsString::from_vec(b"caj\xff.caj".to_vec());
    let Ok(Command::Convert { input, .. }) = parse(vec![name.clone()]) else {
        panic!("expected a conversion");
    };
    assert_eq!(input, Endpoint::Path(PathBuf::from(name)));
    assert_eq!(
        default_output(&input),
        Ok(Endpoint::Path(PathBuf::from(OsString::from_vec(
            b"caj\xff.pdf".to_vec()
        ))))
    );
}

#[test]
fn default_output_is_a_distinct_sibling_pdf() {
    assert_eq!(default_output(&Endpoint::Std), Ok(Endpoint::Std));
    assert_eq!(default_output(&path("dir/a.caj")), Ok(path("dir/a.pdf")));
    assert_eq!(default_output(&path("a")), Ok(path("a.pdf")));
    assert_eq!(default_output(&path("a.PDF")), Ok(path("a.pdf")));
    let error = default_output(&path("dir/a.pdf")).unwrap_err();
    assert_eq!(error.code, 2);
    assert!(error.message.contains("-o OUTPUT"));
}

#[test]
fn json_strings_escape_quotes_backslashes_and_controls() {
    let mut info = caj_inspection();
    let json_title = |info: &Inspection| {
        let text = render(true, info, true);
        let start = text.find("\"title\":").unwrap() + 8;
        let end = text.find(",\"page\":").unwrap();
        text[start..end].to_owned()
    };
    for (title, expected) in [
        ("", "\"\""),
        ("plain", "\"plain\""),
        (
            "a\"b\\c\nd\re\tf\u{1}g\u{1f}h\u{7f}",
            "\"a\\\"b\\\\c\\nd\\re\\tf\\u0001g\\u001fh\u{7f}\"",
        ),
        // Backspace and form feed keep the \u escapes of schema version 1.
        ("\u{8}\u{c}/", "\"\\u0008\\u000c/\""),
        ("中文\u{20000}", "\"中文\u{20000}\""),
    ] {
        info.bookmarks = Some(vec![bookmark(0, title, 0)]);
        assert_eq!(json_title(&info), expected, "{title:?}");
    }
}

fn bookmark(depth: u32, title: &str, page_index: u32) -> Bookmark {
    Bookmark {
        depth,
        title: title.to_owned(),
        page_index,
    }
}

fn caj_inspection() -> Inspection {
    Inspection {
        format: InputFormat::Caj,
        variant: None,
        bookmark_count: None,
        input_bytes_read: 0,
        page_count: Some(3),
        has_outline: Some(true),
        bookmarks: Some(vec![
            bookmark(0, "One", 0),
            bookmark(1, "One.A", 1),
            bookmark(2, "One.A.i", 1),
            bookmark(0, "Two \"q\"\u{1b}", 2),
            bookmark(1, "Two.A", 2),
            bookmark(1, "Two.B", 2),
        ]),
        outline: OutlineReport::default(),
        application_info: ApplicationInfoReport::default(),
        structure: None,
    }
}

/// A report sink that keeps what it accepts and fails once `remaining`
/// bytes were written. Rendering and failure tests share this one type.
struct FailAfter {
    bytes: Vec<u8>,
    remaining: usize,
}

impl FailAfter {
    fn new(remaining: usize) -> Self {
        Self {
            bytes: Vec::new(),
            remaining,
        }
    }
}

impl Write for FailAfter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Err(io::Error::other("sink failed"));
        }
        let accepted = bytes.len().min(self.remaining);
        self.remaining -= accepted;
        self.bytes.extend_from_slice(&bytes[..accepted]);
        Ok(accepted)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn write_report(out: &mut FailAfter, json: bool, info: &Inspection, list: bool) -> io::Result<()> {
    if json {
        write_json(out, info, list, false)
    } else {
        write_text(out, info, list, false)
    }
}

fn render(json: bool, info: &Inspection, list: bool) -> String {
    let mut out = FailAfter::new(usize::MAX);
    write_report(&mut out, json, info, list).unwrap();
    String::from_utf8(out.bytes).unwrap()
}

#[test]
fn json_report_nests_bookmarks_by_depth() {
    let info = caj_inspection();
    let head = r#"{"schema_version":1,"format":"CAJ","variant":null,"conversion_supported":true,"page_count":3,"has_outline":true,"bookmark_count":6"#;
    assert_eq!(
        render(true, &info, false),
        format!("{head},\"outline_warnings\":0}}\n")
    );
    let tree = concat!(
        r#"[{"title":"One","page":1,"children":[{"title":"One.A","page":2,"children":["#,
        r#"{"title":"One.A.i","page":2,"children":[]}]}]},"#,
        r#"{"title":"Two \"q\"\u001b","page":3,"children":["#,
        r#"{"title":"Two.A","page":3,"children":[]},{"title":"Two.B","page":3,"children":[]}]}]"#
    );
    assert_eq!(
        render(true, &info, true),
        format!("{head},\"bookmarks\":{tree},\"outline_warnings\":0}}\n")
    );
}

#[test]
fn json_tree_clamps_a_skipped_parent_and_handles_empty_outlines() {
    let mut info = caj_inspection();
    info.bookmarks = Some(vec![bookmark(2, "Deep", 0)]);
    assert!(
        render(true, &info, true).ends_with(
            "[{\"title\":\"Deep\",\"page\":1,\"children\":[]}],\"outline_warnings\":0}\n"
        )
    );
    info.bookmarks = Some(Vec::new());
    info.has_outline = Some(false);
    assert!(
        render(true, &info, true)
            .ends_with("\"bookmark_count\":0,\"bookmarks\":[],\"outline_warnings\":0}\n")
    );
}

#[test]
fn json_report_uses_null_for_unknown_fields() {
    let info = Inspection {
        format: InputFormat::C8,
        variant: Some(Variant::C8),
        bookmark_count: None,
        input_bytes_read: 0,
        page_count: Some(1),
        has_outline: None,
        bookmarks: None,
        outline: OutlineReport::default(),
        application_info: ApplicationInfoReport::default(),
        structure: None,
    };
    assert_eq!(
        render(true, &info, true),
        "{\"schema_version\":1,\"format\":\"C8\",\"variant\":\"C8\",\"conversion_supported\":true,\
         \"page_count\":1,\"has_outline\":null,\"bookmark_count\":null,\"bookmarks\":null,\
         \"outline_warnings\":null}\n"
    );
}

#[test]
fn text_report_lists_an_indented_outline() {
    let info = caj_inspection();
    let head = "Format: CAJ\nConversion: supported\nPages: 3\nOutline: yes\nBookmarks: 6\n";
    assert_eq!(render(false, &info, false), head);
    assert_eq!(
        render(false, &info, true),
        format!(
            "{head}  - One (page 1)\n    - One.A (page 2)\n      - One.A.i (page 2)\n  \
             - Two \"q\"\\u{{1b}} (page 3)\n    - Two.A (page 3)\n    - Two.B (page 3)\n"
        )
    );
}

#[test]
fn text_report_describes_unknown_fields() {
    let mut info = Inspection {
        format: InputFormat::Teb,
        variant: None,
        bookmark_count: None,
        input_bytes_read: 0,
        page_count: None,
        has_outline: None,
        bookmarks: None,
        outline: OutlineReport::default(),
        application_info: ApplicationInfoReport::default(),
        structure: None,
    };
    let unknown = "Format: TEB\nConversion: not supported (DRM-encrypted container)\nPages: unknown\nOutline: unknown\n";
    assert_eq!(render(false, &info, false), unknown);
    assert_eq!(
        render(false, &info, true),
        format!("{unknown}Bookmarks: listing is not available for TEB input\n")
    );
    info.format = InputFormat::Nh;
    assert!(render(false, &info, false).contains("Conversion: not supported\n"));
    assert!(!render(true, &info, false).contains("unsupported_reason"));
    assert_eq!(
        unsupported(InputFormat::Nh),
        "NH input is recognized, but NH conversion is not supported"
    );
    assert!(unsupported(InputFormat::Teb).contains("DRM-encrypted"));
    info.format = InputFormat::Hn;
    info.variant = Some(Variant::HnA);
    info.has_outline = Some(false);
    let text = render(false, &info, false);
    assert!(text.starts_with("Format: HN\nVariant: HN-A\n"), "{text}");
    assert!(text.contains("Outline: no\n"), "{text}");
}

#[test]
fn outline_warnings_list_bounded_locations_then_a_summary() {
    use caj2pdf_core::{InspectOptions, Limits, NeverCancel};
    // One valid HN-A root followed by 18 entries whose page 0 is outside
    // the single source page.
    let count = 19;
    let mut bytes = vec![0; 0x15c + count * 308 + 20];
    bytes[..8].copy_from_slice(&[72, 78, 0, 0, 0x90, 1, 0, 0]);
    bytes[0x90] = 1;
    bytes[0x158] = count as u8;
    for number in 0..count {
        let at = 0x15c + number * 308;
        bytes[at] = b'T';
        bytes[at + 280] = if number == 0 { b'1' } else { b'0' };
        bytes[at + 304] = 1;
    }
    let options = InspectOptions {
        bookmarks: true,
        ..InspectOptions::default()
    };
    let inspected = caj2pdf_core::inspect(
        &mut &bytes[..],
        &options,
        &Limits::default(),
        &mut NeverCancel,
    )
    .unwrap();
    assert_eq!(inspected.bookmarks.unwrap().len(), 1);
    assert_eq!(inspected.application_info, ApplicationInfoReport::default());
    let outline = inspected.outline;
    let mut out = FailAfter::new(usize::MAX);
    write_warnings(&mut out, &outline).unwrap();
    let text = String::from_utf8(out.bytes).unwrap();
    let lines = text.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 17);
    for (number, line) in (1..).zip(&lines[..16]) {
        assert_eq!(
            *line,
            format!(
                "caj2pdf: warning: skipped HN-A bookmark at byte {}: destination is outside source pages",
                0x15c + number * 308 + 280
            )
        );
    }
    assert_eq!(
        lines[16],
        "caj2pdf: warning: 2 more HN-A bookmark defects were not listed"
    );
    let mut info = caj_inspection();
    info.outline = outline;
    assert!(render(true, &info, false).ends_with(",\"outline_warnings\":18}\n"));
    assert!(render(false, &info, false).ends_with("Bookmarks: 6\nOutline warnings: 18\n"));
    for remaining in 0..text.len() {
        assert!(write_warnings(&mut FailAfter::new(remaining), &outline).is_err());
    }
    let mut unverified = OutlineReport::default();
    unverified.unverified = true;
    let mut out = FailAfter::new(usize::MAX);
    write_warnings(&mut out, &unverified).unwrap();
    assert_eq!(
        String::from_utf8(out.bytes).unwrap(),
        "caj2pdf: warning: C8/HN-B bookmarks are not verified; wrote no outline \
         (--no-bookmarks silences this)\n"
    );
    assert!(write_warnings(&mut FailAfter::new(0), &unverified).is_err());
    let mut empty = FailAfter::new(0);
    write_warnings(&mut empty, &OutlineReport::default()).unwrap();
}

struct Chunks(Vec<io::Result<Vec<u8>>>);

impl Read for Chunks {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.0.is_empty() {
            return Ok(0);
        }
        let bytes = self.0.remove(0)?;
        buffer[..bytes.len()].copy_from_slice(&bytes);
        Ok(bytes.len())
    }
}

#[test]
fn spooling_retries_interrupts_and_leaves_no_name() {
    let dir = TempDir::new("spool");
    let interrupted = io::Error::from(io::ErrorKind::Interrupted);
    let reader = Chunks(vec![
        Ok(b"abc".to_vec()),
        Err(interrupted),
        Ok(b"de".to_vec()),
    ]);
    let mut file = spool(reader, 5, &dir.0).unwrap();
    assert!(dir.entries().is_empty());
    let mut text = String::new();
    file.read_to_string(&mut text).unwrap();
    assert_eq!(text, "abcde");
}

#[test]
fn spooling_enforces_the_limit_and_reports_read_errors() {
    let dir = TempDir::new("spool-limit");
    let reader = Chunks(vec![Ok(b"abc".to_vec()), Ok(b"de".to_vec())]);
    assert!(matches!(
        spool(reader, 4, &dir.0),
        Err(SpoolError::TooLarge)
    ));
    let reader = Chunks(vec![Err(io::Error::other("boom"))]);
    assert!(matches!(spool(reader, 4, &dir.0), Err(SpoolError::Io(_))));
    assert!(dir.entries().is_empty());
    let missing = dir.0.join("missing");
    assert!(matches!(
        spool(Chunks(Vec::new()), 4, &missing),
        Err(SpoolError::Io(_))
    ));
}

#[test]
fn temporary_names_are_bounded_retries() {
    let dir = TempDir::new("names");
    // Occupy every name the next attempts can pick, including counter values
    // consumed concurrently by other tests.
    let first = NEXT_TEMP.load(Ordering::Relaxed);
    for counter in first..first + 256 {
        let name = format!(".caj2pdf-spool.{}-{counter}.tmp", std::process::id());
        File::create(dir.0.join(name)).unwrap();
    }
    let error = spool(Chunks(Vec::new()), 1, &dir.0).unwrap_err();
    let SpoolError::Io(error) = error else {
        panic!("expected an I/O error");
    };
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    const { assert!(TEMP_ATTEMPTS < 256) };
}

#[test]
fn terminal_stdout_is_refused_before_writing() {
    let error = refuse_terminal(&Endpoint::Std, true).unwrap_err();
    assert_eq!(error.code, 1);
    assert!(error.message.contains("terminal"));
    assert_eq!(refuse_terminal(&Endpoint::Std, false), Ok(()));
    assert_eq!(refuse_terminal(&path("out.pdf"), true), Ok(()));
}

fn open_error(endpoint: &Endpoint, force: bool, inputs: &[&Input]) -> CliError {
    match open_output(endpoint, force, inputs) {
        Err(error) => error,
        Ok(_) => panic!("output accepted"),
    }
}

#[test]
fn output_paths_are_checked_before_staging() {
    let dir = TempDir::new("output");
    let source = dir.0.join("in.caj");
    fs::write(&source, b"CAJ").unwrap();
    let source_input = input(&source);
    let link = dir.0.join("link.pdf");
    std::os::unix::fs::symlink(&source, &link).unwrap();
    let hard = dir.0.join("hard.pdf");
    fs::hard_link(&source, &hard).unwrap();
    for target in [&source, &link, &hard] {
        let error = open_error(&Endpoint::Path(target.clone()), true, &[&source_input]);
        assert!(
            error.message.contains("same file as input"),
            "{}",
            error.message
        );
    }
    let error = open_error(&Endpoint::Path(dir.0.clone()), true, &[]);
    assert!(error.message.ends_with("is a directory"));
    let error = open_error(&Endpoint::Path(link.clone()), false, &[]);
    assert!(error.message.contains("already exists; use --force"));
    let dangling = dir.0.join("dangling.pdf");
    std::os::unix::fs::symlink(dir.0.join("nowhere"), &dangling).unwrap();
    let error = open_error(&Endpoint::Path(dangling), false, &[]);
    assert!(error.message.contains("already exists"));
    let error = open_error(&Endpoint::Path(dir.0.join("missing/..")), true, &[]);
    assert!(error.message.contains("does not name a file"));
    let error = open_error(&Endpoint::Path(dir.0.join("missing/out.pdf")), false, &[]);
    assert!(error.message.contains("cannot create a temporary file"));
}

fn staged(target: &Path, force: bool) -> Output {
    staged_for(target, force, &[])
}

fn staged_for(target: &Path, force: bool, inputs: &[&Input]) -> Output {
    let mut output = open_output(&Endpoint::Path(target.to_owned()), force, inputs).unwrap();
    output.writer().write_all(b"%PDF-").unwrap();
    output
}

#[test]
fn staged_output_is_committed_only_on_success() {
    let dir = TempDir::new("stage");
    let target = dir.0.join("out.pdf");
    drop(staged(&target, false));
    assert!(dir.entries().is_empty());

    let output = staged(&target, false);
    assert_eq!(dir.entries().len(), 1);
    output.commit().unwrap();
    assert_eq!(dir.entries(), vec![OsString::from("out.pdf")]);
    assert_eq!(fs::read(&target).unwrap(), b"%PDF-");

    // A file that appears during conversion is not replaced without --force.
    fs::remove_file(&target).unwrap();
    let output = staged(&target, false);
    fs::write(&target, b"other").unwrap();
    let error = output.commit().unwrap_err();
    assert!(error.message.contains("already exists"));
    assert_eq!(fs::read(&target).unwrap(), b"other");
    assert_eq!(dir.entries(), vec![OsString::from("out.pdf")]);

    let output = staged(&target, true);
    output.commit().unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"%PDF-");

    // A rename failure removes the staged file.
    fs::remove_file(&target).unwrap();
    let output = staged(&target, true);
    fs::create_dir(&target).unwrap();
    fs::write(target.join("occupied"), b"").unwrap();
    let error = output.commit().unwrap_err();
    assert!(
        error.message.starts_with("cannot write"),
        "{}",
        error.message
    );
    assert_eq!(dir.entries(), vec![OsString::from("out.pdf")]);
}

/// The single hidden temporary name in `dir`.
fn temp_name(dir: &TempDir) -> PathBuf {
    let names: Vec<_> = dir
        .entries()
        .into_iter()
        .filter(|name| name.as_encoded_bytes().starts_with(b"."))
        .collect();
    let [name] = names.as_slice() else {
        panic!("expected one temporary file, found {names:?}");
    };
    dir.0.join(name)
}

#[test]
fn commit_rechecks_the_target_and_never_clobbers_without_force() {
    let dir = TempDir::new("recheck");
    let source = dir.0.join("in.caj");
    fs::write(&source, b"CAJ").unwrap();
    let source_input = input(&source);
    let target = dir.0.join("out.pdf");

    // A target replaced by a link to an input is refused, even with --force.
    let output = staged_for(&target, true, &[&source_input]);
    fs::hard_link(&source, &target).unwrap();
    let error = output.commit().unwrap_err();
    assert!(
        error.message.contains("same file as input"),
        "{}",
        error.message
    );
    assert_eq!(fs::read(&source).unwrap(), b"CAJ");
    fs::remove_file(&target).unwrap();

    // A dangling symbolic link that appears is not replaced either.
    let output = staged(&target, false);
    std::os::unix::fs::symlink("nowhere", &target).unwrap();
    assert!(
        output
            .commit()
            .unwrap_err()
            .message
            .contains("already exists")
    );
    assert!(fs::symlink_metadata(&target).unwrap().is_symlink());
    fs::remove_file(&target).unwrap();

    // When linking fails for another reason, an existing target is still
    // refused, and otherwise the rename reports the failure.
    let output = staged(&target, false);
    fs::remove_file(temp_name(&dir)).unwrap();
    fs::write(&target, b"other").unwrap();
    assert!(
        output
            .commit()
            .unwrap_err()
            .message
            .contains("already exists")
    );
    fs::remove_file(&target).unwrap();
    let output = staged(&target, false);
    fs::remove_file(temp_name(&dir)).unwrap();
    assert!(
        output
            .commit()
            .unwrap_err()
            .message
            .starts_with("cannot write")
    );
    assert_eq!(dir.entries(), vec![OsString::from("in.caj")]);
}

#[test]
fn long_output_names_get_a_bounded_temporary_name() {
    let dir = TempDir::new("long");
    let name = format!("{}.pdf", "n".repeat(251));
    let target = dir.0.join(&name);
    staged(&target, false).commit().unwrap();
    assert_eq!(dir.entries(), vec![OsString::from(name)]);
}

fn c8_package(doi: Option<&str>, url: Option<&str>) -> Inspection {
    Inspection {
        format: InputFormat::C8,
        variant: Some(Variant::C8),
        bookmark_count: None,
        input_bytes_read: 0,
        page_count: Some(1),
        has_outline: None,
        bookmarks: None,
        outline: OutlineReport::default(),
        application_info: ApplicationInfoReport {
            info: Some(ApplicationInfo {
                doi: doi.map(str::to_owned),
                url: url.map(str::to_owned),
                note_count: 2,
            }),
            status: ApplicationInfoStatus::Read,
        },
        structure: None,
    }
}

#[test]
fn application_info_is_reported_only_when_read() {
    let info = c8_package(Some("ID\u{1b}"), Some("http://example.invalid/\"q\""));
    assert!(render(true, &info, false).ends_with(
        ",\"outline_warnings\":null,\"application_info\":{\"doi\":\"ID\\u001b\",\
         \"url\":\"http://example.invalid/\\\"q\\\"\",\"note_count\":2}}\n"
    ));
    assert!(render(false, &info, false).ends_with(
        "Outline: unknown\nDOI: ID\\u{1b}\nURL: http://example.invalid/\"q\"\nNotes: 2\n"
    ));
    let empty = c8_package(None, None);
    assert!(
        render(true, &empty, false)
            .ends_with("\"application_info\":{\"doi\":null,\"url\":null,\"note_count\":2}}\n")
    );
    assert!(render(false, &empty, false).ends_with("Outline: unknown\nNotes: 2\n"));
    // The parsed values precede the --pages structure, which only locates the tail.
    let both = Inspection {
        structure: hnc8_structure(None).structure,
        ..c8_package(Some("I"), None)
    };
    let mut out = FailAfter::new(usize::MAX);
    write_json(&mut out, &both, false, true).unwrap();
    write_text(&mut out, &both, false, true).unwrap();
    let text = String::from_utf8(out.bytes).unwrap();
    assert!(text.contains(
        "\"outline_warnings\":null,\"application_info\":{\"doi\":\"I\",\"url\":null,\
         \"note_count\":2},\"structure\":{\"page_index_offset\":80,"
    ));
    assert!(text.contains("\"application_info\":null}"));
    assert!(text.ends_with(
        "DOI: I\nNotes: 2\nPage index: 80+40 (20-byte rows)\nNative mode: 23112\n\
         Native origin: unknown\nPage size: 5901 8354\nApplication info: none\n"
    ));
    let defect = ApplicationInfoDefect {
        offset: 7,
        field: "application-info XML",
        reason: "malformed start tag",
    };
    let mut out = FailAfter::new(usize::MAX);
    write_application_info_warning(&mut out, ApplicationInfoStatus::Ignored(defect)).unwrap();
    write_application_info_warning(&mut out, ApplicationInfoStatus::Read).unwrap();
    write_application_info_warning(&mut out, ApplicationInfoStatus::Absent).unwrap();
    assert_eq!(
        String::from_utf8(out.bytes).unwrap(),
        "caj2pdf: warning: ignored C8 application-info package at byte 7: \
         application-info XML: malformed start tag\n"
    );
    assert!(
        write_application_info_warning(
            &mut FailAfter::new(0),
            ApplicationInfoStatus::Ignored(defect)
        )
        .is_err()
    );
}

#[test]
fn report_writers_propagate_every_sink_failure() {
    let hn = Inspection {
        format: InputFormat::Hn,
        variant: Some(Variant::HnB),
        bookmark_count: None,
        input_bytes_read: 0,
        page_count: None,
        has_outline: None,
        bookmarks: None,
        outline: OutlineReport::default(),
        application_info: ApplicationInfoReport::default(),
        structure: None,
    };
    for info in [
        caj_inspection(),
        hn,
        c8_package(Some("I"), Some("U")),
        c8_package(None, None),
    ] {
        for json in [false, true] {
            let full = render(json, &info, true).len();
            for remaining in 0..full {
                let result = write_report(&mut FailAfter::new(remaining), json, &info, true);
                assert!(result.is_err(), "{remaining} of {full}");
            }
        }
    }
}

fn hnc8_structure(application_info: Option<ApplicationInfoTail>) -> Inspection {
    Inspection {
        format: InputFormat::C8,
        variant: Some(Variant::C8),
        bookmark_count: None,
        input_bytes_read: 0,
        page_count: Some(2),
        has_outline: None,
        bookmarks: None,
        outline: OutlineReport::default(),
        application_info: ApplicationInfoReport::default(),
        structure: Some(Structure::Hnc8 {
            header: Header {
                variant: Variant::C8,
                native_mode: Some(23112),
                native_origin: None,
                page_size: Some([5901, 8354]),
                page_count: 2,
                page_index: Span {
                    offset: 80,
                    length: 40,
                },
            },
            page_row_bytes: 20,
            application_info,
        }),
    }
}

/// Write the document report, then two pages: one compressed page with two
/// images and one whose second descriptor failed.
fn render_pages(out: &mut FailAfter, json: bool, info: &Inspection) -> io::Result<()> {
    if json {
        write_json(out, info, false, true)?;
    } else {
        write_text(out, info, false, true)?;
    }
    let row = |page_number, offset| PageRecord {
        page_number,
        row_offset: 60 + 20 * u64::from(page_number),
        text: Span { offset, length: 9 },
        image_count: 2,
        unknown: [0; 10],
    };
    let image = |record_type, offset| ImageRecord {
        page_number: 1,
        image_number: 1,
        descriptor_offset: offset - 12,
        record_type,
        payload: Span { offset, length: 7 },
    };
    let mut pages = Pages::new(out, json);
    pages.begin()?;
    pages.page(1, Some(&row(1, 120)))?;
    pages.image(&image(3, 141))?;
    pages.image(&image(2, 160))?;
    let text = TextStructure {
        framing: TextFraming::CompressText,
        records: 5,
        decoded_length: Some(64),
    };
    pages.end_page(Some(&text), None, None)?;
    pages.page(2, Some(&row(2, 167)))?;
    pages.image(&image(0, 188))?;
    pages.end_page(None, None, Some("bad descriptor"))?;
    pages.finish()
}

fn rendered_pages(json: bool, info: &Inspection) -> String {
    let mut out = FailAfter::new(usize::MAX);
    render_pages(&mut out, json, info).unwrap();
    String::from_utf8(out.bytes).unwrap()
}

#[test]
fn page_reports_stream_spans_counts_and_errors() {
    let tail = ApplicationInfoTail {
        offset: 99,
        length: None,
    };
    let info = hnc8_structure(Some(tail));
    assert_eq!(
        rendered_pages(false, &info),
        "Format: C8\nVariant: C8\nConversion: experimental\n\
         Pages: 2\nOutline: unknown\nPage index: 80+40 (20-byte rows)\nNative mode: 23112\n\
         Native origin: unknown\nPage size: 5901 8354\n\
         Application info: declared at 99, outside the input\n\
         Page 1: text 120+9, images [type 3 at 141+7, type 2 at 160+7], \
         framing compresstext (5 records, 64 decoded bytes)\n\
         Page 2: text 167+9, images [type 0 at 188+7], error: bad descriptor\n"
    );
    assert_eq!(
        rendered_pages(true, &info),
        concat!(
            r#"{"schema_version":1,"format":"C8","variant":"C8","conversion_supported":true,"#,
            r#""page_count":2,"has_outline":null,"bookmark_count":null,"outline_warnings":null,"#,
            r#""structure":{"page_index_offset":80,"page_index_length":40,"page_row_bytes":20,"#,
            r#""native_mode":23112,"native_origin":null,"page_size":[5901,8354],"#,
            r#""application_info":{"offset":99,"length":null}},"pages":["#,
            r#"{"page":1,"text_offset":120,"text_length":9,"image_count":2,"images":["#,
            r#"{"type":3,"offset":141,"length":7},{"type":2,"offset":160,"length":7}],"#,
            r#""text_framing":"compresstext","text_records":5,"text_decoded_length":64,"#,
            r#""text_error":null,"error":null},"#,
            r#"{"page":2,"text_offset":167,"text_length":9,"image_count":2,"#,
            r#""images":[{"type":0,"offset":188,"length":7}],"text_framing":null,"#,
            r#""text_records":null,"text_decoded_length":null,"text_error":null,"#,
            r#""error":"bad descriptor"}]}"#,
            "\n"
        )
    );
    // Without --pages the structure is neither printed nor listed.
    assert!(!render(false, &info, false).contains("Page index"));
    assert!(!render(true, &info, false).contains("structure"));
    for json in [false, true] {
        let full = rendered_pages(json, &info).len();
        for remaining in 0..full {
            let result = render_pages(&mut FailAfter::new(remaining), json, &info);
            assert!(result.is_err(), "{remaining} of {full}");
        }
    }
}

#[test]
fn kdh_signatures_escape_bytes_outside_printable_ascii() {
    let info = Inspection {
        format: InputFormat::Kdh,
        variant: None,
        bookmark_count: None,
        input_bytes_read: 0,
        page_count: None,
        has_outline: None,
        bookmarks: None,
        outline: OutlineReport::default(),
        application_info: ApplicationInfoReport::default(),
        structure: Some(Structure::Kdh {
            signature: b"KDH \"9\\\x00\xff~".to_vec(),
            supported: false,
        }),
    };
    let mut out = FailAfter::new(usize::MAX);
    write_text(&mut out, &info, false, true).unwrap();
    let text = String::from_utf8(out.bytes).unwrap();
    assert!(
        text.ends_with("KDH signature: \"KDH \\x229\\x5c\\x00\\xff~\" (unsupported)\n"),
        "{text}"
    );
    let mut out = FailAfter::new(usize::MAX);
    write_json(&mut out, &info, false, true).unwrap();
    let json = String::from_utf8(out.bytes).unwrap();
    assert!(
        json.ends_with(
            r#""structure":{"kdh_signature":"KDH \\x229\\x5c\\x00\\xff~","kdh_signature_supported":false}"#
        ),
        "{json}"
    );
    let info = hnc8_structure(Some(ApplicationInfoTail {
        offset: 40,
        length: Some(1449),
    }));
    let text = render_text_structure(&info);
    assert!(
        text.contains("Application info: 1449 bytes at 40\n"),
        "{text}"
    );
    let text = render_text_structure(&hnc8_structure(None));
    assert!(text.contains("Application info: none\n"), "{text}");
}

fn render_text_structure(info: &Inspection) -> String {
    let mut out = FailAfter::new(usize::MAX);
    write_text(&mut out, info, false, true).unwrap();
    String::from_utf8(out.bytes).unwrap()
}

#[test]
fn page_reports_without_records_say_so() {
    for (json, expected) in [
        (false, "Page structure: not available for PDF input\n"),
        (true, ",\"pages\":null}\n"),
    ] {
        let mut out = FailAfter::new(usize::MAX);
        Pages::new(&mut out, json)
            .unavailable(InputFormat::Pdf)
            .unwrap();
        assert_eq!(String::from_utf8(out.bytes).unwrap(), expected);
    }
}

#[test]
fn an_input_that_shrinks_during_the_page_report_is_an_inspection_error() {
    let dir = TempDir::new("pages-shrink");
    let path = dir.0.join("two.hn");
    let mut bytes = vec![0; 0xd8 + 2 * 20];
    bytes[..8].copy_from_slice(b"HN\0\0\xc8\0\0\0");
    bytes[0x88] = 0xc8;
    bytes[0x90] = 2;
    fs::write(&path, &bytes).unwrap();
    let mut input = input(&path);
    let limits = caj2pdf_core::Limits::default();
    let info = crate::document::inspect(&mut input, &limits, true).unwrap();
    // The page index no longer fits when the per-page cursor reopens it.
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(0xd8 + 20)
        .unwrap();
    let mut out = FailAfter::new(usize::MAX);
    let error =
        crate::document::write_pages(&mut input, &limits, &info, &mut Pages::new(&mut out, true))
            .unwrap_err();
    assert_eq!(error.code, 1);
    assert!(
        error.message.starts_with("cannot inspect ") && error.message.contains("page index"),
        "{}",
        error.message
    );
}

#[test]
fn every_format_has_a_name() {
    for (format, name) in [
        (InputFormat::Pdf, "PDF"),
        (InputFormat::Caj, "CAJ"),
        (InputFormat::Kdh, "KDH"),
        (InputFormat::Nh, "NH"),
        (InputFormat::Hn, "HN"),
        (InputFormat::C8, "C8"),
        (InputFormat::Teb, "TEB"),
    ] {
        assert_eq!(format.name(), name);
    }
}

#[test]
fn a_forward_only_input_is_bounded_while_spooling() {
    let error = match open_input(&path("/dev/zero"), 10) {
        Err(error) => error,
        Ok(_) => panic!("unbounded input accepted"),
    };
    assert_eq!(error.message, "'/dev/zero' exceeds the 10-byte input limit");
}

#[test]
fn an_unusable_spool_directory_is_a_read_failure() {
    let scratch = TempDir::new("missing-spool");
    let missing = scratch.0.join("missing");
    let error = match open_input_spooling_in(&path("/dev/zero"), 10, &missing) {
        Err(error) => error,
        Ok(_) => panic!("spooled into a missing directory"),
    };
    assert!(
        error.message.starts_with("cannot read '/dev/zero': "),
        "{}",
        error.message
    );
    assert!(scratch.entries().is_empty());
}

#[test]
fn a_staged_output_whose_final_flush_fails_is_not_committed() {
    let scratch = TempDir::new("flush");
    let target = scratch.0.join("out.pdf");
    let mut output = open_output(&Endpoint::Path(target.clone()), false, &[]).unwrap();
    // Buffered bytes reach the file only when commit flushes them.
    *output.writer() = io::BufWriter::new(File::options().write(true).open("/dev/full").unwrap());
    output.writer().write_all(b"%PDF-").unwrap();
    let error = output.commit().unwrap_err();
    assert!(
        error
            .message
            .starts_with(&format!("cannot write '{}': ", target.display())),
        "{}",
        error.message
    );
    assert!(scratch.entries().is_empty());
}

#[test]
fn stdout_commit_reports_a_failed_flush() {
    let mut output = Output::Stdout(io::BufWriter::new(File::create("/dev/full").unwrap()));
    output.writer().write_all(b"%PDF-").unwrap();
    let error = output.commit().unwrap_err();
    assert!(error.message.starts_with("cannot write standard output"));
}

#[test]
fn experimental_conversion_options_are_scoped_and_unambiguous() {
    for args in [
        vec!["paper.hn", "--no-bookmarks"],
        vec!["--no-bookmarks", "paper.hn"],
    ] {
        let Command::Convert { options, .. } = parse_str(&args).unwrap() else {
            panic!()
        };
        assert!(options.no_bookmarks);
        assert!(!options.quiet);
    }
    for flag in ["-q", "--quiet"] {
        let Command::Convert { options, .. } = parse_str(&["paper.caj", flag]).unwrap() else {
            panic!()
        };
        assert!(options.quiet);
        assert!(!options.no_system_fonts);
    }
    let Command::Convert { options, .. } =
        parse_str(&["paper.c8", "--no-system-fonts", "--fonts", "dir"]).unwrap()
    else {
        panic!()
    };
    assert!(options.no_system_fonts);
    for args in [
        vec!["inspect", "paper.caj", "--quiet"],
        vec!["inspect", "paper.c8", "--no-system-fonts"],
        vec!["paper.hn", "--qm-states", "qm.txt"],
        vec!["paper.hn", "--mq-states=mq.txt"],
        vec!["inspect", "paper.hn", "--no-bookmarks"],
    ] {
        assert!(parse_str(&args).is_err(), "{args:?}");
    }
}

#[test]
fn native_font_options_preserve_paths_and_validate_roles() {
    let Command::Convert { options, .. } = parse_str(&[
        "input.c8",
        "--font-cjk=a",
        "--font-latin",
        "a",
        "--font-alternate-latin=b",
        "--font-decoration=c",
        "--font-symbols=d",
        "--font-latin-state3=e",
        "--font-latin-state28=f",
        "--font-latin-state31=g",
        "--decoration-char",
        "A",
    ])
    .unwrap() else {
        panic!()
    };
    assert_eq!(
        options.fonts,
        [
            Some("a".into()),
            Some("a".into()),
            Some("b".into()),
            Some("c".into()),
            Some("d".into()),
            Some("e".into()),
            Some("f".into()),
            Some("g".into())
        ]
    );
    assert_eq!(options.decoration_char, Some('A'));
    for args in [
        vec!["input.c8", "--font-cjk"],
        vec!["input.c8", "--font-cjk="],
        vec!["input.c8", "--font-cjk", "-"],
        vec!["input.c8", "--font-cjk=a"],
        vec!["input.c8", "--font-cjk=a", "--font-cjk=b"],
        vec!["input.c8", "--decoration-char"],
        vec!["input.c8", "--decoration-char", ""],
        vec!["input.c8", "--decoration-char", "😀"],
        vec!["input.c8", "--decoration-char", "AB"],
        vec!["input.c8", "--decoration-char", "A"],
        vec![
            "input.c8",
            "--decoration-char",
            "A",
            "--decoration-char",
            "A",
        ],
        vec!["inspect", "input.c8", "--font-cjk=a"],
    ] {
        assert!(parse_str(&args).is_err(), "{args:?}");
    }
    let unusual = OsString::from_vec(b"font-\xff".to_vec());
    let Command::Convert { options, .. } = parse(vec![
        "input.c8".into(),
        "--font-cjk".into(),
        unusual.clone(),
        "--font-latin".into(),
        unusual.clone(),
        "--font-alternate-latin".into(),
        unusual.clone(),
    ])
    .unwrap() else {
        panic!()
    };
    assert_eq!(options.fonts[0], Some(unusual.clone().into()));
    assert!(parse(vec!["input.c8".into(), "--decoration-char".into(), unusual]).is_err());
}

#[test]
fn font_directory_and_two_required_roles_parse_without_optional_roles() {
    let Command::Convert { options, .. } =
        parse_str(&["input.c8", "--font-cjk=a", "--font-latin", "b"]).unwrap()
    else {
        panic!()
    };
    assert_eq!(options.fonts[..2], [Some("a".into()), Some("b".into())]);
    assert!(options.fonts[2..].iter().all(Option::is_none));
    for (args, directory) in [
        (vec!["input.c8", "--fonts", "dir"], "dir"),
        (vec!["input.c8", "--fonts=dir", "--font-latin=b"], "dir"),
        (vec!["input.c8", "--fonts=d", "--decoration-char", "A"], "d"),
    ] {
        let Command::Convert { options, .. } = parse_str(&args).unwrap() else {
            panic!()
        };
        assert_eq!(options.font_dir, Some(directory.into()), "{args:?}");
    }
    for args in [
        vec!["input.c8", "--fonts"],
        vec!["input.c8", "--fonts="],
        vec!["input.c8", "--fonts", "-"],
        vec!["input.c8", "--fonts=a", "--fonts", "b"],
        vec!["input.c8", "--font-latin=b"],
        vec!["inspect", "input.c8", "--fonts=dir"],
    ] {
        assert!(parse_str(&args).is_err(), "{args:?}");
    }
}

fn font_options(directory: &Path) -> crate::command::ConvertOptions {
    crate::command::ConvertOptions {
        font_dir: Some(directory.to_owned()),
        ..Default::default()
    }
}

fn load_fonts(
    options: &crate::command::ConvertOptions,
) -> Result<crate::hnc8::Resources, CliError> {
    crate::hnc8::Resources::load(options, &caj2pdf_core::Limits::default())
}

#[test]
fn font_directory_maps_fixed_names_and_leaves_missing_optional_roles_to_fallback() {
    let dir = TempDir::new("fonts");
    let missing = load_fonts(&font_options(&dir.0.join("absent")))
        .err()
        .unwrap();
    assert_eq!(missing.code, 1);
    assert!(missing.message.contains("not a readable directory"));
    let file = dir.0.join("file");
    fs::write(&file, b"").unwrap();
    assert!(load_fonts(&font_options(&file)).is_err());
    for (present, absent) in [("latin.ttf", "cjk.ttf"), ("cjk.ttf", "latin.ttf")] {
        let roles = TempDir::new("font-roles");
        fs::write(roles.0.join(present), b"font").unwrap();
        let error = load_fonts(&font_options(&roles.0)).err().unwrap();
        assert!(error.message.contains(absent), "{}", error.message);
    }
    fs::write(dir.0.join("cjk.ttf"), b"cjk").unwrap();
    fs::write(dir.0.join("latin.ttf"), b"latin").unwrap();
    let resources = load_fonts(&font_options(&dir.0)).unwrap();
    let roles = resources.font_roles.unwrap();
    assert_eq!((roles.cjk, roles.latin), (0, 1));
    assert!(roles.alternate_latin.is_none() && roles.decoration.is_none());
    assert!(roles.symbols.is_none() && roles.latin_state3.is_none());
    assert!(roles.latin_state28.is_none() && roles.latin_state31.is_none());
    assert_eq!(resources.inputs.len(), 2);
    // A decoration alias needs a decoration font from the directory or a flag.
    let mut options = font_options(&dir.0);
    options.decoration_char = Some('A');
    assert!(
        load_fonts(&options)
            .err()
            .unwrap()
            .message
            .contains("decoration.ttf")
    );
    for name in crate::command::FONT_FILES {
        fs::write(dir.0.join(format!("{name}.ttf")), name).unwrap();
    }
    let resources = load_fonts(&options).unwrap();
    let roles = resources.font_roles.unwrap();
    assert_eq!(roles.alternate_latin, Some(2));
    assert_eq!(roles.decoration, Some((3, 'A')));
    assert_eq!(roles.symbols, Some(4));
    assert_eq!(roles.latin_state3, Some(5));
    assert_eq!(roles.latin_state28, Some(6));
    assert_eq!(roles.latin_state31, Some(7));
    assert_eq!(resources.inputs.len(), 8);
}

#[test]
fn explicit_font_flags_override_the_directory_and_entries_are_opened() {
    let dir = TempDir::new("font-override");
    fs::write(dir.0.join("cjk.ttf"), b"cjk").unwrap();
    fs::write(dir.0.join("latin.ttf"), b"latin").unwrap();
    // A non-file entry is not treated as absent: opening it reports the error.
    fs::create_dir(dir.0.join("symbols.ttf")).unwrap();
    assert!(load_fonts(&font_options(&dir.0)).is_err());
    fs::remove_dir(dir.0.join("symbols.ttf")).unwrap();
    // Explicit Latin reuses the directory's CJK path, so it is embedded once;
    // an explicit decoration replaces the absent directory entry.
    let mut options = font_options(&dir.0);
    options.fonts[1] = Some(dir.0.join("cjk.ttf"));
    options.fonts[3] = Some(dir.0.join("latin.ttf"));
    let resources = load_fonts(&options).unwrap();
    let roles = resources.font_roles.unwrap();
    assert_eq!((roles.cjk, roles.latin), (0, 0));
    assert_eq!(
        roles.decoration,
        Some((1, caj2pdf_core::hnc8::C8_DEFAULT_DECORATION_ALIAS))
    );
    assert_eq!(resources.inputs.len(), 2);
    // Without a directory, explicit flags behave as before.
    options.font_dir = None;
    let resources = load_fonts(&options).unwrap();
    assert_eq!(resources.font_roles.unwrap().latin, 0);
}

#[test]
fn font_collections_are_found_in_directories_and_selected_by_face_suffix() {
    let dir = TempDir::new("font-faces");
    fs::write(dir.0.join("cjk.ttc"), b"cjk").unwrap();
    fs::write(dir.0.join("latin.otf"), b"latin").unwrap();
    // A literal file name ending in `#digits` is used whole.
    fs::write(dir.0.join("odd#7"), b"odd").unwrap();
    let mut options = font_options(&dir.0);
    options.fonts[2] = Some(dir.0.join("cjk.ttc#2"));
    options.fonts[4] = Some(dir.0.join("odd#7"));
    options.fonts[5] = Some(dir.0.join("cjk.ttc#x"));
    let resources = load_fonts(&options);
    // `cjk.ttc#x` is not a face suffix and names no file.
    assert!(resources.is_err());
    options.fonts[5] = None;
    let resources = load_fonts(&options).unwrap();
    let roles = resources.font_roles.unwrap();
    assert_eq!((roles.cjk, roles.latin), (0, 1));
    // The same collection opened again as face 2 is a distinct source.
    assert_eq!(roles.alternate_latin, Some(2));
    assert_eq!(roles.symbols, Some(3));
    assert_eq!(resources.font_faces[..4], [0, 0, 2, 0]);
    options.fonts[2] = Some(dir.0.join("cjk.ttc#4294967296"));
    let error = load_fonts(&options).err().unwrap();
    assert!(error.message.contains("too large"), "{}", error.message);
}

#[test]
fn progress_reports_the_furthest_input_byte_once_per_percent() {
    use crate::progress::Progress;
    use caj2pdf_core::Progress as _;
    let mut out = Vec::new();
    let mut progress = Progress::new(Some(&mut out));
    for done in [100, 101, 200] {
        progress.input_read(done, 200);
    }
    progress.format(None);
    assert_eq!(progress.format, Some(None));
    assert!(!progress.is_cancelled());
    progress.finish();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        format!(
            "\rcaj2pdf: reading input  50%\rcaj2pdf: reading input 100%\r{:30}\r",
            ""
        )
    );
}

#[test]
fn progress_without_a_terminal_or_reads_writes_nothing() {
    use crate::progress::Progress;
    use caj2pdf_core::Progress as _;
    let mut out = Vec::new();
    Progress::new(Some(&mut out)).finish();
    assert!(out.is_empty());
    let mut progress = Progress::new(None);
    progress.input_read(4, 4);
    progress.finish();
}
