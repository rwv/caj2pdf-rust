// SPDX-License-Identifier: MIT

//! Native-platform regressions: execute the actual CLI with original fixtures.
#![cfg(any(unix, windows))]

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "caj2pdf-portable-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::create_dir(path.join("temp")).unwrap();
        fs::write(
            path.join("文献.pdf"),
            include_bytes!("../../../tests/fixtures/valid_nested_outline.pdf"),
        )
        .unwrap();
        Self(path)
    }
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_caj2pdf"));
        command.current_dir(&self.0).args(args);
        for key in ["TMPDIR", "TMP", "TEMP"] {
            command.env(key, self.0.join("temp"));
        }
        command
    }
    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }
    fn clean(&self) {
        assert_eq!(fs::read_dir(self.0.join("temp")).unwrap().count(), 0);
        assert!(
            !fs::read_dir(&self.0).unwrap().any(|p| p
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp"))
        );
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn unicode_paths_conversion_inspection_and_overwrite_policy() {
    let dir = Directory::new();
    success(&dir.run(&["文献.pdf", "-o", "output.pdf"]));
    let bytes = fs::read(dir.0.join("output.pdf")).unwrap();
    assert!(bytes.starts_with(b"%PDF-"));
    let info = dir.run(&["inspect", "output.pdf", "--json"]);
    success(&info);
    assert!(
        String::from_utf8(info.stdout)
            .unwrap()
            .contains("\"page_count\":2")
    );
    assert!(!dir.run(&["文献.pdf", "-o", "output.pdf"]).status.success());
    assert_eq!(fs::read(dir.0.join("output.pdf")).unwrap(), bytes);
    success(&dir.run(&["文献.pdf", "-o", "output.pdf", "--force"]));
    dir.clean();
}

#[test]
fn same_file_and_hard_link_inputs_are_protected() {
    let dir = Directory::new();
    let before = fs::read(dir.0.join("文献.pdf")).unwrap();
    fs::hard_link(dir.0.join("文献.pdf"), dir.0.join("alias.pdf")).unwrap();
    for target in ["文献.pdf", "alias.pdf"] {
        let output = dir.run(&["文献.pdf", "-o", target, "--force"]);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("same file"));
    }
    assert_eq!(fs::read(dir.0.join("文献.pdf")).unwrap(), before);
    dir.clean();
}

#[test]
fn pipe_input_and_output_match_path_conversion() {
    let dir = Directory::new();
    success(&dir.run(&["文献.pdf", "-o", "output.pdf"]));
    let mut child = dir
        .command(&["-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(include_bytes!(
            "../../../tests/fixtures/valid_nested_outline.pdf"
        ))
        .unwrap();
    let output = child.wait_with_output().unwrap();
    success(&output);
    assert_eq!(output.stdout, fs::read(dir.0.join("output.pdf")).unwrap());
    dir.clean();
}

#[test]
fn malformed_input_preserves_destination_and_removes_staging() {
    let dir = Directory::new();
    fs::write(dir.0.join("bad.caj"), b"not a document").unwrap();
    fs::write(dir.0.join("output.pdf"), b"keep me").unwrap();
    let output = dir.run(&["bad.caj", "-o", "output.pdf", "--force"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(fs::read(dir.0.join("output.pdf")).unwrap(), b"keep me");
    dir.clean();
}
