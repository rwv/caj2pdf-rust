// SPDX-License-Identifier: MIT

//! Opt-in acceptance check: the six pinned native C8/HN-B documents of the
//! external corpus convert with no font option, using the fonts installed by
//! the packages in `docs/cli.md` (Installed fonts). A normal test run reads
//! no document and no installed font and reports zero conversions.

#![cfg(any(unix, windows))]

use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::process::Command;

/// Corpus-relative path and SHA-256 of each pinned document.
const PINNED: [(&str, &str); 6] = [
    (
        "issue-66/IDL编译器的实现_词法分析部分_于埴尧.caj",
        "90e7b47716c32ef7a67cde8094f312e0ee7f1a7e0ed50a25830e8ba64a84f6a6",
    ),
    (
        "issue-90/4-[21].caj",
        "1b9aa912e0bc9dcce4dfbefa07e08a24665d6db624a02f9a30a8c404df5a9ef7",
    ),
    (
        "issue-90/4-[24].caj",
        "03770ea1cdebc9856d1443438b296860fa11f3e283717d256ea0af57ecae1c89",
    ),
    (
        "issue-100/中国金融体制改革阶段研究_李卉.caj",
        "3f3b9b57d6925df811247dced47fd7fb74cf0f678ab9bfda0827c827258ab39b",
    ),
    (
        "issue-63/对任务型教学法的理论基础与课堂实践的思考.caj",
        "63870d12f2069d3d6c663dc2813d04a38a632c4c319f20b6f8dd1b8b8bc589c2",
    ),
    (
        "issue-65/伽利略的原子论思想_近代科学革命的形而上学基础.caj",
        "e1b17805a87f62097987c41f2821836d6b774caaf035c9846be966c965f08a49",
    ),
];

#[test]
fn optional_installed_font_corpus_is_not_run_by_default() {
    println!("NOT_RUN\tchecked=0\tconverted=0\tfailed=0");
}

#[test]
#[ignore = "requires explicit private CAJ2PDF_CORPUS_DIR and installed fonts"]
fn pinned_native_documents_convert_with_installed_fonts() {
    let corpus = PathBuf::from(
        std::env::var_os("CAJ2PDF_CORPUS_DIR").expect("explicit run requires CAJ2PDF_CORPUS_DIR"),
    );
    let output_dir = std::env::temp_dir().join(format!("caj2pdf-installed-{}", std::process::id()));
    std::fs::create_dir_all(&output_dir).unwrap();
    let mut failed = Vec::new();
    for (index, (path, sha256)) in PINNED.iter().enumerate() {
        let input = corpus.join(path);
        let digest = Sha256::digest(std::fs::read(&input).unwrap());
        let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(&hex, sha256, "{path} is not the pinned document");
        let pdf = output_dir.join(format!("{index}.pdf"));
        let output = Command::new(env!("CARGO_BIN_EXE_caj2pdf"))
            .arg(&input)
            .args(["--no-bookmarks", "--force", "-o"])
            .arg(&pdf)
            .env_remove("CAJ2PDF_FONT_DIRS")
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        let bytes = std::fs::read(&pdf).unwrap_or_default();
        if output.status.success()
            && stderr.matches("caj2pdf: using installed ").count() == 2
            && bytes.starts_with(b"%PDF-")
        {
            println!(
                "{path}\t{} bytes\t{}",
                bytes.len(),
                stderr.trim().replace('\n', "\t")
            );
        } else {
            println!("{path}\tFAILED\t{}", stderr.trim());
            failed.push(*path);
        }
    }
    let _ = std::fs::remove_dir_all(&output_dir);
    let status = if failed.is_empty() { "PASS" } else { "FAIL" };
    println!(
        "{status}\tchecked={}\tconverted={}\tfailed={}",
        PINNED.len(),
        PINNED.len() - failed.len(),
        failed.len()
    );
    assert!(failed.is_empty(), "{failed:?}");
}
