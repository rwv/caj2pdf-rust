// SPDX-License-Identifier: MIT

use std::process::Command;

#[test]
fn unfinished_cli_fails_without_emitting_document_bytes() {
    let output = Command::new(env!("CARGO_BIN_EXE_caj2pdf"))
        .output()
        .expect("caj2pdf binary should start");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"caj2pdf conversion is not implemented yet\n"
    );
}
