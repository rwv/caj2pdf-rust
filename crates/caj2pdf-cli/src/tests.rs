// SPDX-License-Identifier: MIT

//! Unit tests for argument parsing, output selection, JSON encoding, report
//! rendering, and the file helpers. Process behavior is in `tests/cli.rs`.

use crate::CliError;
use crate::args::{Command, Endpoint, Topic, parse};
use crate::cli::default_output;
use crate::document::{Inspection, block_on, format_name};
use crate::files::{
    Input, NEXT_TEMP, Output, SpoolError, TEMP_ATTEMPTS, open_input, open_output, refuse_terminal,
    spool,
};
use crate::json::write_string;
use crate::report::{write_json, write_text};
use caj2pdf_core::{Bookmark, InputFormat};
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

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let counter = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "caj2pdf-cli-unit-{label}-{}-{counter}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
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
            force: false
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
                force: false
            })
        );
    }
    assert_eq!(
        parse_str(&["-", "-o", "-", "-f", "--force"]),
        Ok(Command::Convert {
            input: Endpoint::Std,
            output: Some(Endpoint::Std),
            force: true
        })
    );
    assert_eq!(
        parse_str(&["--", "-name.caj"]),
        Ok(Command::Convert {
            input: path("-name.caj"),
            output: None,
            force: false
        })
    );
    assert_eq!(
        parse_str(&["./inspect"]),
        Ok(Command::Convert {
            input: path("./inspect"),
            output: None,
            force: false
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
            bookmarks: true
        })
    );
    assert_eq!(
        parse_str(&["inspect", "-"]),
        Ok(Command::Inspect {
            input: Endpoint::Std,
            json: false,
            bookmarks: false
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

#[test]
fn help_and_version_win_after_valid_arguments() {
    assert_eq!(parse_str(&["--help"]), Ok(Command::Help(Topic::Convert)));
    assert_eq!(parse_str(&["x", "-h"]), Ok(Command::Help(Topic::Convert)));
    assert_eq!(
        parse_str(&["inspect", "--help"]),
        Ok(Command::Help(Topic::Inspect))
    );
    assert_eq!(
        parse_str(&["add-bookmarks", "-h"]),
        Ok(Command::Help(Topic::AddBookmarks))
    );
    assert_eq!(parse_str(&["-V"]), Ok(Command::Version));
    assert_eq!(parse_str(&["inspect", "--version"]), Ok(Command::Version));
    for topic in [Topic::Convert, Topic::Inspect, Topic::AddBookmarks] {
        assert!(topic.help().contains("Usage:"));
    }
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
        (&["a", "-o"], "option '-o' requires a value"),
        (&["a", "-o", "x", "--output=y"], "more than once"),
        (&["a", "--json"], "unrecognized option '--json'"),
        (&["inspect", "a", "-o", "x"], "unrecognized option '-o'"),
        (
            &["inspect", "a", "--output=x"],
            "unrecognized option '--output=x'",
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
fn block_on_returns_a_ready_value() {
    assert_eq!(block_on(async { 7 }), 7);
}

fn json_string(value: &str) -> String {
    let mut out = Vec::new();
    write_string(&mut out, value).unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn json_strings_escape_quotes_backslashes_and_controls() {
    assert_eq!(json_string(""), "\"\"");
    assert_eq!(json_string("plain"), "\"plain\"");
    assert_eq!(
        json_string("a\"b\\c\nd\re\tf\u{1}g\u{1f}h\u{7f}"),
        "\"a\\\"b\\\\c\\nd\\re\\tf\\u0001g\\u001fh\u{7f}\""
    );
    assert_eq!(json_string("中文\u{20000}"), "\"中文\u{20000}\"");
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
        write_json(out, info, list)
    } else {
        write_text(out, info, list)
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
    assert_eq!(render(true, &info, false), format!("{head}}}\n"));
    let tree = concat!(
        r#"[{"title":"One","page":1,"children":[{"title":"One.A","page":2,"children":["#,
        r#"{"title":"One.A.i","page":2,"children":[]}]}]},"#,
        r#"{"title":"Two \"q\"\u001b","page":3,"children":["#,
        r#"{"title":"Two.A","page":3,"children":[]},{"title":"Two.B","page":3,"children":[]}]}]"#
    );
    assert_eq!(
        render(true, &info, true),
        format!("{head},\"bookmarks\":{tree}}}\n")
    );
}

#[test]
fn json_tree_clamps_a_skipped_parent_and_handles_empty_outlines() {
    let mut info = caj_inspection();
    info.bookmarks = Some(vec![bookmark(2, "Deep", 0)]);
    assert!(
        render(true, &info, true)
            .ends_with("\"bookmarks\":[{\"title\":\"Deep\",\"page\":1,\"children\":[]}]}\n")
    );
    info.bookmarks = Some(Vec::new());
    info.has_outline = Some(false);
    assert!(
        render(true, &info, true)
            .ends_with("\"has_outline\":false,\"bookmark_count\":0,\"bookmarks\":[]}\n")
    );
}

#[test]
fn json_report_uses_null_for_unknown_fields() {
    let info = Inspection {
        format: InputFormat::C8,
        variant: Some("C8"),
        page_count: Some(1),
        has_outline: None,
        bookmarks: None,
    };
    assert_eq!(
        render(true, &info, true),
        "{\"schema_version\":1,\"format\":\"C8\",\"variant\":\"C8\",\"conversion_supported\":false,\
         \"page_count\":1,\"has_outline\":null,\"bookmark_count\":null,\"bookmarks\":null}\n"
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
        page_count: None,
        has_outline: None,
        bookmarks: None,
    };
    let unknown = "Format: TEB\nConversion: not supported\nPages: unknown\nOutline: unknown\n";
    assert_eq!(render(false, &info, false), unknown);
    assert_eq!(
        render(false, &info, true),
        format!("{unknown}Bookmarks: listing is not available for TEB input\n")
    );
    info.format = InputFormat::Hn;
    info.variant = Some("HN-A");
    info.has_outline = Some(false);
    let text = render(false, &info, false);
    assert!(text.starts_with("Format: HN\nVariant: HN-A\n"), "{text}");
    assert!(text.contains("Outline: no\n"), "{text}");
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
    for counter in first..first + 4096 {
        let name = format!(".caj2pdf-spool.{}-{counter}.tmp", std::process::id());
        File::create(dir.0.join(name)).unwrap();
    }
    let error = spool(Chunks(Vec::new()), 1, &dir.0).unwrap_err();
    let SpoolError::Io(error) = error else {
        panic!("expected an I/O error");
    };
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    const { assert!(TEMP_ATTEMPTS < 4096) };
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

/// Accepts `remaining` bytes, then fails every write.

#[test]
fn report_writers_propagate_every_sink_failure() {
    let hn = Inspection {
        format: InputFormat::Hn,
        variant: Some("HN-B"),
        page_count: None,
        has_outline: None,
        bookmarks: None,
    };
    for info in [caj_inspection(), hn] {
        for json in [false, true] {
            let full = render(json, &info, true).len();
            for remaining in 0..full {
                let result = write_report(&mut FailAfter::new(remaining), json, &info, true);
                assert!(result.is_err(), "{remaining} of {full}");
            }
        }
    }
}

#[test]
fn every_format_has_a_name() {
    assert_eq!(format_name(InputFormat::Nh), "NH");
}

/// A future that is pending once before it completes.
struct PendingOnce(bool);

impl std::future::Future for PendingOnce {
    type Output = u8;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<u8> {
        if std::mem::replace(&mut self.0, true) {
            std::task::Poll::Ready(1)
        } else {
            std::task::Poll::Pending
        }
    }
}

#[test]
fn block_on_polls_again_after_pending() {
    assert_eq!(block_on(PendingOnce(false)), 1);
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
fn stdout_commit_reports_a_failed_flush() {
    let mut output = Output::Stdout(io::BufWriter::new(File::create("/dev/full").unwrap()));
    output.writer().write_all(b"%PDF-").unwrap();
    let error = output.commit().unwrap_err();
    assert!(error.message.starts_with("cannot write standard output"));
}
