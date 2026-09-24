// SPDX-License-Identifier: MIT

//! Optional PDF-body check against independently decoded KDH corpus files.
//! Normal test runs report this as ignored (NOT_RUN), never as a pass.

use caj2pdf_core::{
    Limits, NeverCancel,
    native::{SeekableSource, WriteSink},
    pdf::copy_pdf,
};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, remove_file},
    future::Future,
    io::Write,
    path::Path,
    pin::pin,
    process::Command,
    task::{Context, Poll, Waker},
};

fn run_native<F: Future>(future: F) -> F::Output {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = pin!(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("native PDF adapters unexpectedly yielded"),
    }
}

fn render_hash(path: &Path, page: u32) -> [u8; 32] {
    let output = Command::new("mutool")
        .args([
            "draw", "-q", "-F", "pnm", "-c", "gray", "-r", "36", "-A", "0", "-o", "-",
        ])
        .arg(path)
        .arg(page.to_string())
        .output()
        .expect("mutool is required for the explicit external corpus check");
    assert!(
        output.status.success(),
        "mutool failed for {} page {page}: {}",
        path.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    Sha256::digest(output.stdout).into()
}

#[test]
#[ignore = "NOT_RUN unless CAJ2PDF_KDH_PDF_DIR supplies independently decoded PDF bodies"]
fn independently_decoded_kdh_pdf_bodies_normalize_and_preserve_74_pages() {
    let directory = std::env::var_os("CAJ2PDF_KDH_PDF_DIR")
        .expect("set CAJ2PDF_KDH_PDF_DIR to the external decoded PDF directory");
    let directory = Path::new(&directory);
    let mut pages_checked = 0_u32;
    for (issue, pages) in [(21, 6), (34, 67), (48, 1)] {
        let input = directory.join(format!("issue-{issue}.pdf"));
        assert!(
            input.is_file(),
            "missing external corpus file: {}",
            input.display()
        );
        let output = std::env::temp_dir().join(format!(
            "caj2pdf-kdh-pdf-{}-{issue}.pdf",
            std::process::id()
        ));
        let mut source = SeekableSource::new(File::open(&input).unwrap()).unwrap();
        let mut file = File::create(&output).unwrap();
        let report = run_native(copy_pdf(
            &mut source,
            &mut WriteSink::new(&mut file),
            &Limits::default(),
            &NeverCancel,
        ))
        .unwrap();
        file.flush().unwrap();
        assert_eq!(report.pages_converted, pages, "issue-{issue} page count");
        let check = Command::new("qpdf")
            .arg("--check")
            .arg(&output)
            .output()
            .unwrap();
        assert!(
            check.status.success(),
            "qpdf warnings/errors for issue-{issue}: {}",
            String::from_utf8_lossy(&check.stderr)
        );
        for page in 1..=pages {
            assert_eq!(
                render_hash(&input, page),
                render_hash(&output, page),
                "issue-{issue} page {page} render changed"
            );
            pages_checked += 1;
        }
        remove_file(&output).unwrap();
    }
    assert_eq!(pages_checked, 74);
    eprintln!(
        "KDH external PDF-body compatibility: 74/74 rendered pages match; qpdf --check exit 0 for 3/3 outputs"
    );
}
