// SPDX-License-Identifier: MIT

//! End-to-end CAJ conversion using independently authored container bytes.
//! The header, twelve-byte page rows, and 308-byte TOC records follow the
//! public observations registered in `docs/caj-format.md`.

use caj2pdf_core::{
    ConversionOptions, Error, Limits, NeverCancel, RangedSource,
    caj::convert_caj,
    native::{SeekableSource, WriteSink},
    pdf::{PdfIndex, PdfRange, PdfRef},
};
use std::{
    fs::{OpenOptions, remove_file},
    future::Future,
    io::{Cursor, Write},
    path::PathBuf,
    pin::pin,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    task::{Context, Poll, Waker},
};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn run_native<F: Future>(future: F) -> F::Output {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = pin!(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("native I/O unexpectedly yielded"),
    }
}

struct TempPdf(PathBuf);

impl TempPdf {
    fn write(label: &str, bytes: &[u8]) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "caj2pdf-caj-{label}-{}-{id}.pdf",
            std::process::id()
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap_or_else(|error| panic!("create {}: {error}", path.display()));
        file.write_all(bytes).expect("write synthetic output PDF");
        file.flush().expect("flush synthetic output PDF");
        Self(path)
    }
}

impl Drop for TempPdf {
    fn drop(&mut self) {
        let _ = remove_file(&self.0);
    }
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn object(body: &mut Vec<u8>, number: u32, dictionary: &str) {
    body.extend_from_slice(format!("{number} 0 obj\n{dictionary}\nendobj\n").as_bytes());
}

fn toc_record(title: &[u8], page: u8, level: u32) -> [u8; 308] {
    let mut record = [0_u8; 308];
    record[..title.len()].copy_from_slice(title);
    record[280] = page;
    record[304..308].copy_from_slice(&level.to_le_bytes());
    record
}

struct TinyCaj {
    bytes: Vec<u8>,
    table_start: usize,
}

fn tiny_caj() -> TinyCaj {
    // The physical Page objects appear as 9, 3, 4. The page table and page
    // tree request 9, 4, 3. The missing parent (8) is reconstructed.
    let mut body = Vec::new();
    object(
        &mut body,
        9,
        "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 200 100] /Resources << >> >>",
    );
    object(
        &mut body,
        3,
        "<< /Type /Page /Parent 6 0 R /MediaBox [0 0 400 250] /Resources << >> >>",
    );
    object(
        &mut body,
        4,
        "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 300 150] /Resources << >> >>",
    );
    object(
        &mut body,
        5,
        "<< /Type /Pages /Parent 8 0 R /Count 2 /Kids [9 0 R 4 0 R] >>",
    );
    object(
        &mut body,
        6,
        "<< /Type /Pages /Parent 8 0 R /Count 1 /Kids [3 0 R] >>",
    );

    const TABLE_START: usize = 0x600;
    let body_start = TABLE_START + 3 * 12;
    let mut bytes = vec![0_u8; body_start];
    bytes[..4].copy_from_slice(b"CAJ\0");
    bytes[4..8].copy_from_slice(&[1, 0, 2, 0]);
    put_u32(&mut bytes, 0x10, 3);
    put_u32(&mut bytes, 0x14, TABLE_START as u32);
    put_u32(&mut bytes, 0x110, 3);
    // U+20000 encodes as these four GB18030 bytes. This title is deliberately
    // outside the two-byte GBK repertoire observed in the external corpus.
    let records = [
        toc_record(&[0x95, 0x32, 0x82, 0x36], b'1', 1),
        toc_record(b"Nested", b'2', 2),
        toc_record(b"Third", b'3', 1),
    ];
    for (index, record) in records.iter().enumerate() {
        let start = 0x114 + index * 308;
        bytes[start..start + 308].copy_from_slice(record);
    }
    let body_end = body_start + body.len();
    let page_rows = [
        (body_start as u32, body.len() as u32, 9),
        (body_end as u32, 0, 4),
        (body_end as u32, 0, 3),
    ];
    for (index, (offset, length, page_object)) in page_rows.into_iter().enumerate() {
        let start = TABLE_START + index * 12;
        put_u32(&mut bytes, start, offset);
        put_u32(&mut bytes, start + 4, length);
        put_u32(&mut bytes, start + 8, page_object);
    }
    bytes.extend_from_slice(&body);
    TinyCaj {
        bytes,
        table_start: TABLE_START,
    }
}

fn convert(
    input: &[u8],
    options: ConversionOptions,
    limits: &Limits,
) -> Result<(Vec<u8>, caj2pdf_core::ConversionReport), Error> {
    let mut source = SeekableSource::new(Cursor::new(input))?;
    let mut output = Vec::new();
    let report = run_native(convert_caj(
        &mut source,
        &mut WriteSink::new(&mut output),
        options,
        limits,
        &NeverCancel,
    ))?;
    Ok((output, report))
}

fn inspect(output: &[u8]) -> PdfIndex {
    let mut source = SeekableSource::new(Cursor::new(output)).unwrap();
    let size = source.size();
    run_native(PdfIndex::open(
        &mut source,
        PdfRange {
            offset: 0,
            length: size,
        },
        &Limits::default(),
        &NeverCancel,
    ))
    .unwrap()
}

fn checked_command(command: &mut Command, label: &str) -> String {
    let result = command
        .output()
        .unwrap_or_else(|error| panic!("{label} is required in CI: {error}"));
    assert!(
        result.status.success(),
        "{label} failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).expect("validator output is UTF-8")
}

#[test]
fn converts_scrambled_pages_with_missing_root_and_gb18030_outline() {
    let tiny = tiny_caj();
    let limits = Limits {
        io_chunk_bytes: 1,
        ..Limits::default()
    };
    let (output, report) = convert(&tiny.bytes, ConversionOptions::default(), &limits).unwrap();
    assert_eq!(report.pages_converted, 3);
    assert_eq!(report.bookmarks_written, 3);
    assert_eq!(report.output_bytes_written, output.len() as u64);
    let pdf = inspect(&output);
    assert_eq!(
        pdf.pages(),
        &[
            PdfRef {
                number: 9,
                generation: 0,
            },
            PdfRef {
                number: 4,
                generation: 0,
            },
            PdfRef {
                number: 3,
                generation: 0,
            },
        ]
    );
    assert!(pdf.has_outlines());
    let file = TempPdf::write("outline", &output);
    checked_command(Command::new("qpdf").arg("--check").arg(&file.0), "qpdf");
    let outline = checked_command(
        Command::new("mutool")
            .arg("show")
            .arg(&file.0)
            .arg("outline"),
        "mutool outline",
    );
    assert!(
        outline.contains("𠀀") && outline.contains("#page=1"),
        "{outline}"
    );
    assert!(
        outline.contains("Nested") && outline.contains("#page=2"),
        "{outline}"
    );
    assert!(
        outline.contains("Third") && outline.contains("#page=3"),
        "{outline}"
    );
    let info = checked_command(
        Command::new("pdfinfo")
            .args(["-f", "1", "-l", "3", "-box"])
            .arg(&file.0),
        "pdfinfo",
    );
    let compact = info.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(compact.contains("Pages: 3"), "{info}");
    assert!(compact.contains("Page 1 size: 200 x 100 pts"), "{info}");
    assert!(compact.contains("Page 2 size: 300 x 150 pts"), "{info}");
    assert!(compact.contains("Page 3 size: 400 x 250 pts"), "{info}");
}

#[test]
fn bookmark_option_excludes_outline_without_changing_pages() {
    let tiny = tiny_caj();
    let (output, report) = convert(
        &tiny.bytes,
        ConversionOptions {
            include_bookmarks: false,
        },
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(report.pages_converted, 3);
    assert_eq!(report.bookmarks_written, 0);
    let pdf = inspect(&output);
    assert_eq!(pdf.pages().len(), 3);
    assert!(!pdf.has_outlines());
}

#[test]
fn missing_page_object_is_reported_before_writing() {
    let mut tiny = tiny_caj();
    put_u32(&mut tiny.bytes, tiny.table_start + 2 * 12 + 8, 777);
    let mut source = SeekableSource::new(Cursor::new(tiny.bytes.as_slice())).unwrap();
    let mut output = Vec::new();
    let result = run_native(convert_caj(
        &mut source,
        &mut WriteSink::new(&mut output),
        ConversionOptions::default(),
        &Limits::default(),
        &NeverCancel,
    ));
    assert!(
        matches!(&result, Err(Error::Caj { offset, .. }) if *offset == tiny.bytes.len() as u64),
        "{result:?}"
    );
    assert!(output.is_empty());
}
