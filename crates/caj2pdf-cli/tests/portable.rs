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
        let binary = std::env::var_os("CAJ2PDF_TEST_BINARY")
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_caj2pdf").into());
        let runner = std::env::var("CAJ2PDF_TEST_RUNNER").unwrap_or_default();
        let mut words = runner.split_whitespace();
        let mut command = if let Some(program) = words.next() {
            let mut command = Command::new(program);
            command.args(words).arg(&binary);
            command
        } else {
            Command::new(&binary)
        };
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

#[test]
fn native_c8_fonts_are_ranged_reused_and_protected() {
    let dir = Directory::new();
    let font = include_bytes!("../../../tests/fonts/geometric.ttf");
    fs::write(dir.0.join("字体.ttf"), font).unwrap();
    let mut input = vec![0u8; 100];
    input[0] = 0xc8;
    input[12] = 2;
    input[8] = 1;
    input[32] = 100;
    input[34] = 200;
    input[80] = 100;
    input[84] = 16;
    input[96] = 116;
    for word in [0x8001u16, 60, 0x8002, 0x1084, 30, 0xa0c1, 0x8004, 39] {
        input.extend(word.to_le_bytes());
    }
    fs::write(dir.0.join("input.c8"), &input).unwrap();
    let flags = [
        "--font-cjk=字体.ttf",
        "--font-latin=字体.ttf",
        "--font-alternate-latin=字体.ttf",
        "--no-bookmarks",
    ];
    let mut args = vec!["input.c8", "-o", "native.pdf"];
    args.extend(flags);
    success(&dir.run(&args));
    let pdf = fs::read(dir.0.join("native.pdf")).unwrap();
    assert!(pdf.ends_with(b"%%EOF\n"));
    assert_eq!(
        String::from_utf8_lossy(&pdf).matches("/FontFile2 ").count(),
        1
    );
    assert!(String::from_utf8_lossy(&page_content(&pdf)).contains("<0041> Tj"));
    for target in ["字体.ttf", "native.pdf"] {
        input[110..112].copy_from_slice(&0xa0c2u16.to_le_bytes());
        fs::write(dir.0.join("input.c8"), &input).unwrap();
        let mut args = vec!["input.c8", "-o", target, "--force"];
        args.extend(flags);
        assert!(!dir.run(&args).status.success());
        assert_eq!(fs::read(dir.0.join("字体.ttf")).unwrap(), font);
        assert_eq!(fs::read(dir.0.join("native.pdf")).unwrap(), pdf);
    }
    dir.clean();
}

#[test]
fn late_hnb_error_preserves_destination_and_removes_staging() {
    let dir = Directory::new();
    fs::write(
        dir.0.join("font.ttf"),
        include_bytes!("../../../tests/fonts/geometric.ttf"),
    )
    .unwrap();
    let mut input = vec![0u8; 240];
    input[..4].copy_from_slice(b"HN\0\0");
    for (at, value) in [(4, 200u32), (8, 136), (144, 2), (148, 2)] {
        input[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    input[168..170].copy_from_slice(&100u16.to_le_bytes());
    input[170..172].copy_from_slice(&200u16.to_le_bytes());
    for page in 0..2 {
        let offset = input.len() as u32;
        let row = 216 + page * 12;
        input[row..row + 4].copy_from_slice(&offset.to_le_bytes());
        input[row + 4..row + 8].copy_from_slice(&14u32.to_le_bytes());
        for word in [0x8001u16, 60, 0x8002, 0x1084, 30, 0xa0c1, 0x8004] {
            input.extend(word.to_le_bytes());
        }
    }
    fs::write(dir.0.join("input.hn"), &input).unwrap();
    let args = [
        "input.hn",
        "-o",
        "output.pdf",
        "--force",
        "--no-bookmarks",
        "--font-cjk=font.ttf",
        "--font-latin=font.ttf",
        "--font-alternate-latin=font.ttf",
    ];
    success(&dir.run(&args));
    let original = fs::read(dir.0.join("output.pdf")).unwrap();
    assert!(String::from_utf8_lossy(&original).contains("/Count 2"));
    input[254..256].copy_from_slice(&0x8099u16.to_le_bytes());
    fs::write(dir.0.join("input.hn"), input).unwrap();
    let failed = dir.run(&args);
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("page 2"));
    assert_eq!(fs::read(dir.0.join("output.pdf")).unwrap(), original);
    dir.clean();
}

/// Inflate the first page's compressed content stream.
fn page_content(pdf: &[u8]) -> Vec<u8> {
    use std::io::Read;
    let marker = b"/Filter /FlateDecode\n>>\nstream\n";
    let at = pdf
        .windows(marker.len())
        .position(|part| part == marker)
        .unwrap();
    let mut content = Vec::new();
    flate2::read::ZlibDecoder::new(&pdf[at + marker.len()..])
        .read_to_end(&mut content)
        .unwrap();
    content
}
