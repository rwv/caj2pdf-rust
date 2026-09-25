// SPDX-License-Identifier: MIT

//! Independent validator and renderer checks for inspected and updated PDFs.

mod common;

use caj2pdf_core::{
    Bookmark, Cancellation, Error, Limits, PdfErrorKind, RangedSource, SequentialSink,
    native::{SeekableSource, WriteSink},
    pdf::{
        FragmentObject, FragmentPlan, PdfIndex, PdfOutlineAppender, PdfRange, PdfRef, PdfWriter,
        copy_pdf, copy_pdf_range, reconstruct_fragment,
    },
};
use common::CancelAfter;
use flate2::{Compression, write::ZlibEncoder};
use std::{
    cell::{Cell, RefCell},
    fs::{File, OpenOptions, read, remove_file},
    future::Future,
    io::{Cursor, Write},
    path::{Path, PathBuf},
    pin::pin,
    process::Command,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
    task::{Context, Poll, Waker},
};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn run_native<F: Future>(future: F) -> F::Output {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = pin!(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("native PDF adapters unexpectedly yielded"),
    }
}

struct TempPdf {
    path: PathBuf,
    file: File,
}

impl TempPdf {
    fn new(label: &str) -> Self {
        let sequence = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "caj2pdf-input-{label}-{}-{sequence}.pdf",
            std::process::id()
        ));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap_or_else(|error| panic!("create {}: {error}", path.display()));
        Self { path, file }
    }
}

impl Drop for TempPdf {
    fn drop(&mut self) {
        let _ = remove_file(&self.path);
    }
}

fn check_command(command: &mut Command, name: &str) -> Vec<u8> {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("{name} is required in CI: {error}"));
    assert!(
        output.status.success(),
        "{name} rejected PDF (status {}):\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn check_pdf(path: &Path, pages: u32) {
    check_command(
        Command::new("qpdf").arg("--check").arg(path),
        "qpdf --check",
    );
    let info = check_command(Command::new("mutool").arg("info").arg(path), "mutool info");
    assert!(
        String::from_utf8_lossy(&info).contains(&format!("Pages: {pages}")),
        "{}",
        String::from_utf8_lossy(&info)
    );
}

fn render_page(path: &Path, page: u32) -> Vec<u8> {
    let render = path.with_extension(format!("page-{page}.pnm"));
    check_command(
        Command::new("mutool")
            .args([
                "draw", "-q", "-F", "pnm", "-c", "gray", "-r", "72", "-A", "0", "-o",
            ])
            .arg(&render)
            .arg(path)
            .arg(page.to_string()),
        "mutool draw",
    );
    let bytes = read(&render).expect("read rendered page");
    remove_file(render).expect("remove rendered page");
    bytes
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn synthetic_xref_stream_pdf(
    duplicate_box: bool,
    bomb: bool,
    compressed_object: bool,
    gap: &[u8],
) -> Vec<u8> {
    synthetic_xref_stream_pdf_with_filter(duplicate_box, bomb, compressed_object, gap, true)
}

fn synthetic_xref_stream_pdf_with_filter(
    duplicate_box: bool,
    bomb: bool,
    compressed_object: bool,
    gap: &[u8],
    flate: bool,
) -> Vec<u8> {
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = [0_u32; 5];
    let pages = if duplicate_box {
        b"<< /Type /Pages /Count 1 /Kids [2 0 R] /MediaBox [0 0 612 792] /MediaBox [0 0 612 792] >>"
            .as_slice()
    } else {
        b"<< /Type /Pages /Count 1 /Kids [2 0 R] /MediaBox [0 0 612 792] >>".as_slice()
    };
    for (number, body) in [
        (1, pages),
        (2, b"<< /Type /Page /Parent 1 0 R >>".as_slice()),
        (3, b"<< /Type /Catalog /Pages 1 0 R >>".as_slice()),
    ] {
        offsets[number] = pdf.len() as u32;
        pdf.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
        pdf.extend_from_slice(body);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    pdf.extend_from_slice(gap);
    offsets[4] = pdf.len() as u32;
    let mut decoded = Vec::new();
    for (number, &offset) in offsets.iter().enumerate() {
        let kind = if number == 0 {
            0
        } else if compressed_object && number == 2 {
            2
        } else {
            1
        };
        let field2 = if number == 0 {
            0
        } else if compressed_object && number == 2 {
            1
        } else {
            offset
        };
        let generation = if number == 0 { u16::MAX } else { 0 };
        decoded.push(kind);
        decoded.extend_from_slice(&field2.to_be_bytes());
        decoded.extend_from_slice(&generation.to_be_bytes());
    }
    if bomb {
        decoded.extend_from_slice(&[0; 10_000]);
    }
    let encoded = if flate {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&decoded).unwrap();
        encoder.finish().unwrap()
    } else {
        decoded
    };
    let filter = if flate { " /Filter /FlateDecode" } else { "" };
    pdf.extend_from_slice(format!("4 0 obj\n<< /Type /XRef /Size 5 /Root 3 0 R /W [1 4 2] /Index [0 5] /Length {}{filter} >>\nstream\n", encoded.len()).as_bytes());
    pdf.extend_from_slice(&encoded);
    pdf.extend_from_slice(
        format!("\nendstream\nendobj\nstartxref\n{}\n%%EOF\n", offsets[4]).as_bytes(),
    );
    pdf
}

fn synthetic_stale_parent_pdf(page_body: &[u8]) -> Vec<u8> {
    synthetic_stale_parent_pdf_with_tree(page_body, b"2 0 R", b"(unused but live)", b"")
}

fn synthetic_stale_parent_pdf_with_tree(
    page_body: &[u8],
    kids: &[u8],
    other_body: &[u8],
    gap: &[u8],
) -> Vec<u8> {
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = [0_usize; 6];
    let pages = [
        b"<< /Type /Pages /Count 1 /Kids [".as_slice(),
        kids,
        b"] /MediaBox [0 0 100 100] >>".as_slice(),
    ]
    .concat();
    for (number, body) in [
        (1, pages.as_slice()),
        (2, page_body),
        (4, b"<< /Type /Catalog /Pages 1 0 R >>".as_slice()),
        (5, other_body),
    ] {
        if number == 4 {
            pdf.extend_from_slice(gap);
        }
        offsets[number] = pdf.len();
        pdf.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
        pdf.extend_from_slice(body);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let xref_at = pdf.len();
    pdf.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for (number, offset) in offsets.iter().enumerate().skip(1) {
        if number == 3 {
            pdf.extend_from_slice(b"0000000000 00000 f \n");
        } else {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size 6 /Root 4 0 R >>\nstartxref\n{xref_at}\n%%EOF\n").as_bytes(),
    );
    pdf
}

struct OneBytePdfSource(Vec<u8>);

impl RangedSource for OneBytePdfSource {
    fn size(&self) -> u64 {
        self.0.len() as u64
    }

    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        let Some(byte) = self.0.get(offset as usize) else {
            return Ok(0);
        };
        if destination.is_empty() {
            return Ok(0);
        }
        destination[0] = *byte;
        Ok(1)
    }
}

struct CancelAfterXrefRead {
    armed: Cell<bool>,
    checks_after_read: Cell<u32>,
}

impl Cancellation for CancelAfterXrefRead {
    fn is_cancelled(&self) -> bool {
        if !self.armed.get() {
            return false;
        }
        let checks = self.checks_after_read.get() + 1;
        self.checks_after_read.set(checks);
        checks >= 3
    }
}

struct ArmAtPdfOffset {
    bytes: Vec<u8>,
    offset: u64,
    cancellation: Rc<CancelAfterXrefRead>,
}

struct SharedPdfSource(Rc<RefCell<Vec<u8>>>);

impl RangedSource for SharedPdfSource {
    fn size(&self) -> u64 {
        self.0.borrow().len() as u64
    }

    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        let bytes = self.0.borrow();
        let remaining = bytes.get(offset as usize..).unwrap_or_default();
        let count = remaining.len().min(destination.len());
        destination[..count].copy_from_slice(&remaining[..count]);
        Ok(count)
    }
}

struct MutateOnFirstWrite {
    bytes: Rc<RefCell<Vec<u8>>>,
    at: usize,
    replacement: u8,
    written: Vec<u8>,
}

impl SequentialSink for MutateOnFirstWrite {
    async fn write(&mut self, bytes: &[u8]) -> caj2pdf_core::Result<usize> {
        if self.written.is_empty() {
            self.bytes.borrow_mut()[self.at] = self.replacement;
        }
        self.written.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    async fn flush(&mut self) -> caj2pdf_core::Result<()> {
        Ok(())
    }
}

impl RangedSource for ArmAtPdfOffset {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }

    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        let start = offset as usize;
        let Some(remaining) = self.bytes.get(start..) else {
            return Ok(0);
        };
        let count = remaining.len().min(destination.len());
        destination[..count].copy_from_slice(&remaining[..count]);
        if offset == self.offset {
            self.cancellation.armed.set(true);
        }
        Ok(count)
    }
}

#[test]
fn flate_xref_stream_copy_handles_one_byte_reads() {
    let input = synthetic_xref_stream_pdf(false, false, false, b"");
    let mut source = OneBytePdfSource(input.clone());
    let mut sink = WriteSink::new(Vec::<u8>::new());
    let report = run_native(copy_pdf(
        &mut source,
        &mut sink,
        &Limits::default(),
        &CancelAfter::Never,
    ))
    .unwrap();
    assert_eq!(report.pages_converted, 1);
    assert_eq!(sink.into_inner(), input);
}

#[test]
fn unfiltered_xref_stream_copy_reopens() {
    let input = synthetic_xref_stream_pdf_with_filter(false, false, false, b"", false);
    let mut source = SeekableSource::new(Cursor::new(&input)).unwrap();
    let mut output = TempPdf::new("unfiltered-xref-stream");
    let report = run_native(copy_pdf(
        &mut source,
        &mut WriteSink::new(&mut output.file),
        &Limits::default(),
        &CancelAfter::Never,
    ))
    .unwrap();
    output.file.flush().unwrap();
    assert_eq!(report.pages_converted, 1);
    assert_eq!(read(&output.path).unwrap(), input);
    check_pdf(&output.path, 1);
    inspect_bytes(read(&output.path).unwrap()).unwrap();
}

#[test]
fn xref_stream_row_parse_observes_cancellation() {
    // Width-one rows are cheap to inflate but can require many parse steps.
    // The deliberately free self-entry would fail later if parsing completed.
    let decoded = vec![0; 10_000];
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&decoded).unwrap();
    let encoded = encoder.finish().unwrap();
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let xref_at = bytes.len();
    bytes.extend_from_slice(format!("1 0 obj\n<< /Type /XRef /Size 10000 /Root 1 0 R /W [1 0 0] /Length {} /Filter /FlateDecode >>\nstream\n", encoded.len()).as_bytes());
    let data_at = bytes.len() as u64;
    bytes.extend_from_slice(&encoded);
    bytes.extend_from_slice(
        format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n").as_bytes(),
    );
    let cancellation = Rc::new(CancelAfterXrefRead {
        armed: Cell::new(false),
        checks_after_read: Cell::new(0),
    });
    let mut source = ArmAtPdfOffset {
        bytes,
        offset: data_at,
        cancellation: cancellation.clone(),
    };
    let length = source.size();
    let error = run_native(PdfIndex::open(
        &mut source,
        PdfRange { offset: 0, length },
        &Limits::default(),
        cancellation.as_ref(),
    ))
    .err()
    .unwrap();
    assert!(matches!(error, Error::Cancelled), "{error}");
    assert_eq!(cancellation.checks_after_read.get(), 3);
}

#[test]
fn duplicate_page_box_in_xref_stream_pdf_is_repaired_and_reopens() {
    let input = synthetic_xref_stream_pdf(true, false, false, b"");
    let mut source = SeekableSource::new(Cursor::new(input)).unwrap();
    let mut output = TempPdf::new("xref-stream-repair");
    let report = run_native(copy_pdf(
        &mut source,
        &mut WriteSink::new(&mut output.file),
        &Limits::default(),
        &CancelAfter::Never,
    ))
    .unwrap();
    output.file.flush().unwrap();
    assert_eq!(report.pages_converted, 1);
    check_pdf(&output.path, 1);
    let reopened = inspect_bytes(read(&output.path).unwrap()).unwrap();
    assert!(reopened.repair_objects().is_empty());
}

#[test]
fn malformed_and_unsupported_xref_streams_are_typed() {
    for (old, replacement, kind) in [
        (
            b"/W [1 4 2]".as_slice(),
            b"/W [1 4 9]".as_slice(),
            PdfErrorKind::Malformed,
        ),
        (
            b"/Index [0 5]".as_slice(),
            b"/Index [1 5]".as_slice(),
            PdfErrorKind::Malformed,
        ),
        (
            b"/FlateDecode".as_slice(),
            b"/FlateDecodX".as_slice(),
            PdfErrorKind::UnsupportedFeature,
        ),
    ] {
        let mut input = synthetic_xref_stream_pdf(false, false, false, b"");
        replace_once_same_len(&mut input, old, replacement);
        let error = inspect_bytes(input)
            .err()
            .expect("bad xref stream must fail");
        assert!(
            matches!(error, Error::Pdf { kind: actual, .. } if actual == kind),
            "{error}"
        );
    }
    let error = inspect_bytes(synthetic_xref_stream_pdf(false, false, true, b""))
        .err()
        .unwrap();
    assert!(
        matches!(
            error,
            Error::Pdf {
                kind: PdfErrorKind::UnsupportedFeature,
                ..
            }
        ),
        "{error}"
    );
    let error = inspect_bytes(synthetic_xref_stream_pdf(false, true, false, b""))
        .err()
        .unwrap();
    assert!(
        matches!(
            error,
            Error::PdfLimitExceeded {
                resource: "PDF xref decoded bytes",
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn xref_stream_metadata_limits_and_unsupported_forms_are_located() {
    for (before, after, kind) in [
        (
            b"/Type /XRef".as_slice(),
            b"/Type /Page".as_slice(),
            PdfErrorKind::Malformed,
        ),
        (
            b"/W [1 4 2]".as_slice(),
            b"/W [0 0 0]".as_slice(),
            PdfErrorKind::Malformed,
        ),
        (
            b"/W [1 4 2]".as_slice(),
            b"/W [1 4]".as_slice(),
            PdfErrorKind::Malformed,
        ),
        (
            b"/Index [0 5]".as_slice(),
            b"/Index [0]".as_slice(),
            PdfErrorKind::Malformed,
        ),
        (
            b"/Index [0 5]".as_slice(),
            b"/Index [18446744073709551615 1]".as_slice(),
            PdfErrorKind::Malformed,
        ),
        (
            b"/Length ".as_slice(),
            b"/Length 1 0 R /Unused ".as_slice(),
            PdfErrorKind::UnsupportedFeature,
        ),
        (
            b"/Filter /FlateDecode".as_slice(),
            b"/Filter [/FlateDecode]".as_slice(),
            PdfErrorKind::UnsupportedFeature,
        ),
        (
            b"/Filter /FlateDecode".as_slice(),
            b"/Filter /FlateDecode /DecodeParms << /Predictor 12 >>".as_slice(),
            PdfErrorKind::UnsupportedFeature,
        ),
    ] {
        let mut input = synthetic_xref_stream_pdf(false, false, false, b"");
        replace_once(&mut input, before, after);
        let error = inspect_bytes(input).err().expect("invalid xref metadata");
        assert!(
            matches!(error, Error::Pdf { kind: actual, .. } if actual == kind),
            "{error}"
        );
    }

    for (before, after, resource) in [
        (
            b"/Size 5 /Root".as_slice(),
            b"/Size 600000 /Root".as_slice(),
            "PDF xref decoded bytes",
        ),
        (
            b"/Length ".as_slice(),
            b"/Length 9000000 /Unused ".as_slice(),
            "PDF xref encoded bytes",
        ),
    ] {
        let mut input = synthetic_xref_stream_pdf(false, false, false, b"");
        if resource == "PDF xref decoded bytes" {
            replace_once(&mut input, b"/Index [0 5]", b"/Index [0 600000]");
        }
        replace_once(&mut input, before, after);
        let error = inspect_bytes(input).err().expect("xref resource limit");
        assert!(
            matches!(error, Error::PdfLimitExceeded { resource: actual, .. } if actual == resource),
            "{error}"
        );
    }
}

#[test]
fn xref_stream_data_corruption_and_unsupported_rows_are_located() {
    let mut input = synthetic_xref_stream_pdf(false, false, false, b"");
    let data_at = input
        .windows(7)
        .position(|part| part == b"stream\n")
        .unwrap()
        + 7;
    input[data_at] = 0;
    let error = inspect_bytes(input).err().unwrap();
    assert!(
        matches!(
            error,
            Error::Pdf {
                kind: PdfErrorKind::Malformed,
                reason: "xref stream Flate data is invalid",
                ..
            }
        ),
        "{error}"
    );

    let mut unfiltered = synthetic_xref_stream_pdf_with_filter(false, false, false, b"", false);
    replace_once(&mut unfiltered, b"/W [1 4 2]", b"/W [1 4 1]");
    let error = inspect_bytes(unfiltered).err().unwrap();
    assert!(
        matches!(
            error,
            Error::Pdf {
                kind: PdfErrorKind::Malformed,
                reason: "xref stream length disagrees with W and Index",
                ..
            }
        ),
        "{error}"
    );

    let mut unknown_kind = synthetic_xref_stream_pdf_with_filter(false, false, false, b"", false);
    let data_at = unknown_kind
        .windows(7)
        .position(|part| part == b"stream\n")
        .unwrap()
        + 7;
    unknown_kind[data_at] = 3;
    let error = inspect_bytes(unknown_kind).err().unwrap();
    assert!(
        matches!(
            error,
            Error::Pdf {
                kind: PdfErrorKind::UnsupportedFeature,
                reason: "xref entry type is unsupported",
                ..
            }
        ),
        "{error}"
    );

    let mut bad_offset = synthetic_xref_stream_pdf_with_filter(false, false, false, b"", false);
    let data_at = bad_offset
        .windows(7)
        .position(|part| part == b"stream\n")
        .unwrap()
        + 7;
    bad_offset[data_at + 8..data_at + 12].fill(0xff);
    let error = inspect_bytes(bad_offset).err().unwrap();
    assert!(
        matches!(
            error,
            Error::Pdf {
                kind: PdfErrorKind::Malformed,
                reason: "xref object offset exceeds PDF range",
                ..
            }
        ),
        "{error}"
    );

    let mut bad_generation = synthetic_xref_stream_pdf_with_filter(false, false, false, b"", false);
    replace_once(&mut bad_generation, b"/W [1 4 2]", b"/W [1 4 3]");
    replace_once(&mut bad_generation, b"/Length 35", b"/Length 40");
    let data_at = bad_generation
        .windows(7)
        .position(|part| part == b"stream\n")
        .unwrap()
        + 7;
    let mut wider_rows = Vec::new();
    for row in bad_generation[data_at..data_at + 35].chunks_exact(7) {
        wider_rows.extend_from_slice(&row[..5]);
        wider_rows.push(0);
        wider_rows.extend_from_slice(&row[5..]);
    }
    wider_rows[13..16].copy_from_slice(&[1, 0, 0]);
    bad_generation.splice(data_at..data_at + 35, wider_rows);
    let error = inspect_bytes(bad_generation).err().unwrap();
    assert!(
        matches!(
            error,
            Error::Pdf {
                kind: PdfErrorKind::Malformed,
                reason: "xref generation exceeds 16 bits",
                ..
            }
        ),
        "{error}"
    );

    let mut missing_self = synthetic_xref_stream_pdf_with_filter(false, false, false, b"", false);
    let data_at = missing_self
        .windows(7)
        .position(|part| part == b"stream\n")
        .unwrap()
        + 7;
    missing_self[data_at + 4 * 7] = 0;
    let error = inspect_bytes(missing_self).err().unwrap();
    assert!(
        matches!(
            error,
            Error::Pdf {
                kind: PdfErrorKind::Malformed,
                reason: "xref stream has no valid self entry",
                ..
            }
        ),
        "{error}"
    );
}

/// A three-object document indexed by an unfiltered xref stream with
/// `/W [0 4 2]`: rows carry no type field, so each listed object defaults to
/// in use. Returns the document and the offset of the page object.
fn untyped_xref_stream_pdf() -> (Vec<u8>, u64) {
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut rows = Vec::new();
    for (number, body) in [
        (
            1,
            "<< /Type /Pages /Count 1 /Kids [2 0 R] /MediaBox [0 0 612 792] >>",
        ),
        (2, "<< /Type /Page /Parent 1 0 R >>"),
        (3, "<< /Type /Catalog /Pages 1 0 R >>"),
    ] {
        rows.extend_from_slice(&(pdf.len() as u32).to_be_bytes());
        rows.extend_from_slice(&0_u16.to_be_bytes());
        pdf.extend_from_slice(format!("{number} 0 obj\n{body}\nendobj\n").as_bytes());
    }
    let page_at = u64::from(u32::from_be_bytes(rows[6..10].try_into().unwrap()));
    let xref_at = pdf.len() as u32;
    rows.extend_from_slice(&xref_at.to_be_bytes());
    rows.extend_from_slice(&0_u16.to_be_bytes());
    pdf.extend_from_slice(
        format!(
            "4 0 obj\n<< /Type /XRef /Size 5 /Root 3 0 R /W [0 4 2] /Index [1 4] /Length {} >>\nstream\n",
            rows.len()
        )
        .as_bytes(),
    );
    pdf.extend_from_slice(&rows);
    pdf.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref_at}\n%%EOF\n").as_bytes());
    (pdf, page_at)
}

#[test]
fn xref_stream_rows_without_a_type_field_are_in_use() {
    let (bytes, page_at) = untyped_xref_stream_pdf();
    let index = inspect_bytes(bytes).unwrap();
    let page = PdfRef {
        number: 2,
        generation: 0,
    };
    assert_eq!(index.pages(), [page]);
    assert_eq!(index.object_location(page).unwrap().offset, page_at);
}

#[test]
fn cancellation_at_every_flate_xref_stream_checkpoint_is_reported() {
    let input = synthetic_xref_stream_pdf(false, false, false, b"");
    let mut cancelled = 0;
    for allowed in 0.. {
        assert!(allowed < 10_000, "cancellation checkpoints never ended");
        match inspect_bytes_with(input.clone(), &CancelAfter::new(allowed)) {
            Ok(index) => {
                assert_eq!(index.pages().len(), 1);
                break;
            }
            Err(Error::Cancelled) => cancelled += 1,
            Err(other) => panic!("checkpoint {allowed} failed with {other:?}"),
        }
    }
    assert!(cancelled > 3, "only {cancelled} checkpoints observed");
}

#[test]
fn xref_stream_default_index_and_nonstream_body_are_distinguished() {
    let mut implicit_index = synthetic_xref_stream_pdf(false, false, false, b"");
    replace_once(&mut implicit_index, b" /Index [0 5]", b"");
    inspect_bytes(implicit_index).unwrap();

    let mut regular_object = synthetic_xref_stream_pdf(false, false, false, b"");
    replace_once(&mut regular_object, b">>\nstream\n", b">>\nendobj\n");
    let error = inspect_bytes(regular_object).err().unwrap();
    assert!(
        matches!(
            error,
            Error::Pdf {
                kind: PdfErrorKind::Malformed,
                reason: "xref object is not a stream",
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn lone_cr_stream_separator_is_normalized_without_moving_offsets() {
    let mut input = write_pdf_without_outlines();
    let marker = b"stream\n";
    let at = input
        .windows(marker.len())
        .position(|part| part == marker)
        .unwrap()
        + marker.len()
        - 1;
    input[at] = b'\r';
    let mut source = SeekableSource::new(Cursor::new(input.clone())).unwrap();
    let mut sink = WriteSink::new(Vec::<u8>::new());
    run_native(copy_pdf(
        &mut source,
        &mut sink,
        &Limits::default(),
        &CancelAfter::Never,
    ))
    .unwrap();
    let mut expected = input;
    expected[at] = b'\n';
    assert_eq!(sink.into_inner(), expected);
    inspect_bytes(expected).unwrap();
}

#[test]
fn stale_page_parent_is_repaired_only_from_validated_kids() {
    let input = synthetic_stale_parent_pdf(b"<< /Type /Page /Parent 3 0 R >>");
    let mut source = SeekableSource::new(Cursor::new(input)).unwrap();
    let mut output = TempPdf::new("stale-page-parent");
    let report = run_native(copy_pdf(
        &mut source,
        &mut WriteSink::new(&mut output.file),
        &Limits::default(),
        &CancelAfter::Never,
    ))
    .unwrap();
    output.file.flush().unwrap();
    assert_eq!(report.pages_converted, 1);
    check_pdf(&output.path, 1);
    assert!(
        inspect_bytes(read(&output.path).unwrap())
            .unwrap()
            .repair_objects()
            .is_empty()
    );

    let mut wrong_live = synthetic_stale_parent_pdf(b"<< /Type /Page /Parent 3 0 R >>");
    replace_once_same_len(&mut wrong_live, b"/Parent 3 0 R", b"/Parent 4 0 R");
    let error = inspect_bytes(wrong_live).err().unwrap();
    assert!(
        matches!(
            error,
            Error::Pdf {
                kind: PdfErrorKind::Malformed,
                ..
            }
        ),
        "{error}"
    );

    let unrelated = synthetic_stale_parent_pdf(b"<< /Type /Page /Parent 3 0 R /Resources 3 0 R >>");
    let error = inspect_bytes(unrelated).err().unwrap();
    assert!(
        matches!(
            error,
            Error::Pdf {
                kind: PdfErrorKind::Malformed,
                ..
            }
        ),
        "{error}"
    );

    let unreachable = synthetic_stale_parent_pdf_with_tree(
        b"<< /Type /Page /Parent 3 0 R >>",
        b"5 0 R",
        b"<< /Type /Page /Parent 1 0 R >>",
        b"",
    );
    let error = inspect_bytes(unreachable).err().unwrap();
    assert!(
        matches!(
            error,
            Error::Pdf {
                kind: PdfErrorKind::Malformed,
                reason: "stale page Parent is not reachable through validated Kids",
                ..
            }
        ),
        "{error}"
    );

    let missing_parent = synthetic_stale_parent_pdf(b"<< /Type /Page >>");
    let error = inspect_bytes(missing_parent).err().unwrap();
    assert!(
        matches!(
            error,
            Error::Pdf {
                kind: PdfErrorKind::Malformed,
                reason: "page tree Parent link disagrees with Kids",
                ..
            }
        ),
        "{error}"
    );

    let mut parent_on_root = synthetic_stale_parent_pdf(b"<< /Type /Page /Parent 1 0 R >>");
    replace_once_same_len(&mut parent_on_root, b"/Kids [2 0 R]", b"/Parent 4 0 R");
    let error = inspect_bytes(parent_on_root).err().unwrap();
    assert!(
        matches!(
            error,
            Error::Pdf {
                kind: PdfErrorKind::Malformed,
                reason: "page tree Parent link disagrees with Kids",
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn short_aborted_object_prefix_is_scrubbed_but_other_gap_content_fails() {
    let input = synthetic_xref_stream_pdf(false, false, false, b"4 0 obj\r<\r\n");
    let mut source = SeekableSource::new(Cursor::new(input)).unwrap();
    let mut output = TempPdf::new("orphan-gap");
    run_native(copy_pdf(
        &mut source,
        &mut WriteSink::new(&mut output.file),
        &Limits::default(),
        &CancelAfter::Never,
    ))
    .unwrap();
    output.file.flush().unwrap();
    check_pdf(&output.path, 1);
    let free_gap = synthetic_stale_parent_pdf_with_tree(
        b"<< /Type /Page /Parent 1 0 R >>",
        b"2 0 R",
        b"(unused but live)",
        b"3 0 obj\r<\r\n",
    );
    let mut source = SeekableSource::new(Cursor::new(free_gap)).unwrap();
    let mut output = TempPdf::new("free-orphan-gap");
    run_native(copy_pdf(
        &mut source,
        &mut WriteSink::new(&mut output.file),
        &Limits::default(),
        &CancelAfter::Never,
    ))
    .unwrap();
    output.file.flush().unwrap();
    check_pdf(&output.path, 1);
    let invalid = synthetic_xref_stream_pdf(false, false, false, b"arbitrary gap text\n");
    let error = inspect_bytes(invalid).err().unwrap();
    assert!(
        matches!(
            error,
            Error::Pdf {
                kind: PdfErrorKind::Malformed,
                ..
            }
        ),
        "{error}"
    );

    let active_but_not_next = synthetic_xref_stream_pdf(false, false, false, b"2 0 obj\r<\r\n");
    let error = inspect_bytes(active_but_not_next).err().unwrap();
    assert!(
        matches!(
            error,
            Error::Pdf {
                kind: PdfErrorKind::Malformed,
                reason: "unindexed bytes between PDF objects",
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn copy_rechecks_stream_and_gap_patch_bytes_after_inspection() {
    let mut lone_cr = write_pdf_without_outlines();
    let separator = lone_cr
        .windows(7)
        .position(|part| part == b"stream\n")
        .unwrap()
        + 6;
    lone_cr[separator] = b'\r';
    let marker = b"4 0 obj\r<\r\n";
    let orphan = synthetic_xref_stream_pdf(false, false, false, marker);
    let gap = orphan
        .windows(marker.len())
        .position(|part| part == marker)
        .unwrap();
    for (bytes, at, reason) in [
        (
            lone_cr,
            separator,
            "PDF stream separator changed after inspection",
        ),
        (orphan, gap, "PDF orphan gap changed after inspection"),
    ] {
        let shared = Rc::new(RefCell::new(bytes));
        let mut source = SharedPdfSource(shared.clone());
        let mut sink = MutateOnFirstWrite {
            bytes: shared,
            at,
            replacement: b'X',
            written: Vec::new(),
        };
        let limits = Limits {
            io_chunk_bytes: 16,
            ..Limits::default()
        };
        let error = run_native(copy_pdf(
            &mut source,
            &mut sink,
            &limits,
            &CancelAfter::Never,
        ))
        .expect_err("changed source must fail copy");
        assert!(matches!(error, Error::InvalidInput { reason: actual } if actual == reason));
        assert!(!sink.written.is_empty());
    }
}

#[test]
fn clean_pdf_copy_is_exact_and_preserves_binary_stream_and_outlines() {
    for name in ["valid_nested_outline.pdf", "valid_out_of_order_objects.pdf"] {
        let input = fixture(name);
        let mut source = SeekableSource::new(File::open(&input).unwrap()).unwrap();
        let mut output = TempPdf::new("clean-copy");
        let report = run_native(copy_pdf(
            &mut source,
            &mut WriteSink::new(&mut output.file),
            &Limits::default(),
            &CancelAfter::Never,
        ))
        .unwrap();
        output.file.flush().unwrap();
        assert_eq!(report.pages_converted, 2);
        assert_eq!(report.bookmarks_written, 0);
        assert_eq!(read(&input).unwrap(), read(&output.path).unwrap());
        check_pdf(&output.path, 2);
    }
}

#[test]
fn indirect_media_box_is_a_valid_page_geometry_for_copy() {
    let input = write_pdf_with_catalog_page(
        b"<< /Type /Catalog /Pages 2 0 R >>",
        Some(b"[0 0 200 100]"),
        b"<< /Type /Page /Parent 2 0 R /MediaBox 6 0 R /Resources << >> /Contents 4 0 R >>",
    );
    let mut source = SeekableSource::new(Cursor::new(&input)).unwrap();
    let mut output = TempPdf::new("indirect-mediabox");
    let report = run_native(copy_pdf(
        &mut source,
        &mut WriteSink::new(&mut output.file),
        &Limits::default(),
        &CancelAfter::Never,
    ))
    .unwrap();
    output.file.flush().unwrap();
    assert_eq!(report.pages_converted, 1);
    assert_eq!(read(&output.path).unwrap(), input);
    check_pdf(&output.path, 1);
    let info = check_command(
        Command::new("pdfinfo")
            .args(["-box", "-f", "1", "-l", "1"])
            .arg(&output.path),
        "pdfinfo -box",
    );
    assert!(
        String::from_utf8_lossy(&info).contains("200 x 100 pts"),
        "{}",
        String::from_utf8_lossy(&info)
    );
}

#[test]
fn known_duplicate_page_box_and_long_opaque_tail_are_normalized() {
    let input = fixture("repairable_duplicate_mediabox_tail.pdf");
    let mut source = SeekableSource::new(File::open(&input).unwrap()).unwrap();
    let mut output = TempPdf::new("normalized");
    let report = run_native(copy_pdf(
        &mut source,
        &mut WriteSink::new(&mut output.file),
        &Limits::default(),
        &CancelAfter::Never,
    ))
    .unwrap();
    output.file.flush().unwrap();
    assert_eq!(report.pages_converted, 2);
    assert!(report.output_bytes_written < source.size());
    check_pdf(&output.path, 2);
    for page in 1..=2 {
        assert_eq!(render_page(&input, page), render_page(&output.path, page));
    }
}

#[test]
fn a_valid_pdf_with_known_webfastload_suffix_copies_its_logical_bytes() {
    let clean = read(fixture("valid_nested_outline.pdf")).unwrap();
    for marker in [b"WebFastLoadP".as_slice(), b"WebFastLoadW".as_slice()] {
        let mut tailed = clean.clone();
        tailed.extend_from_slice(marker);
        tailed.extend_from_slice(&[0x7f, 0x00, 0xff, b'X'].repeat(3_000));
        let mut source = SeekableSource::new(Cursor::new(tailed)).unwrap();
        let mut sink = WriteSink::new(Vec::<u8>::new());
        let report = run_native(copy_pdf(
            &mut source,
            &mut sink,
            &Limits::default(),
            &CancelAfter::Never,
        ))
        .unwrap();
        assert_eq!(report.pages_converted, 2);
        assert_eq!(report.output_bytes_written, clean.len() as u64);
        assert_eq!(sink.into_inner(), clean);
    }
}

#[test]
fn unknown_suffix_and_incomplete_incremental_revision_are_not_silently_dropped() {
    for suffix in [
        b"unknown opaque extension".as_slice(),
        b"\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\nxref\n1 1\n0000000000 00000 n \ntrailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n12345\n".as_slice(),
    ] {
        let mut input = write_pdf_without_outlines();
        input.extend_from_slice(suffix);
        let mut source = SeekableSource::new(Cursor::new(input)).unwrap();
        let mut sink = WriteSink::new(Vec::<u8>::new());
        let error = run_native(copy_pdf(
            &mut source,
            &mut sink,
            &Limits::default(),
            &CancelAfter::Never,
        ))
        .expect_err("unknown trailing data must fail");
        assert!(matches!(error, Error::Pdf { kind: PdfErrorKind::Malformed | PdfErrorKind::AmbiguousRepair, .. }), "{error}");
        assert!(sink.into_inner().is_empty());
    }
}

#[test]
fn incremental_xref_cannot_activate_a_live_object_after_the_pdf_end() {
    let mut input = write_pdf_without_outlines();
    let marker = b"startxref\n";
    let marker_at = input
        .windows(marker.len())
        .rposition(|window| window == marker)
        .unwrap();
    let old_xref = std::str::from_utf8(
        &input[marker_at + marker.len()..]
            .iter()
            .copied()
            .take_while(u8::is_ascii_digit)
            .collect::<Vec<_>>(),
    )
    .unwrap()
    .parse::<u64>()
    .unwrap();
    let latest_xref = input.len() as u64;
    let header = b"xref\n3 1\n";
    let trailer = format!(
        "trailer\n<< /Size 6 /Root 1 0 R /Prev {old_xref} >>\nstartxref\n{latest_xref}\n%%EOF\n"
    );
    let footer = b"WebFastLoadP\n";
    let future_object =
        latest_xref + header.len() as u64 + 20 + trailer.len() as u64 + footer.len() as u64;
    input.extend_from_slice(header);
    input.extend_from_slice(format!("{future_object:010} 00000 n \n").as_bytes());
    input.extend_from_slice(trailer.as_bytes());
    input.extend_from_slice(footer);
    assert_eq!(input.len() as u64, future_object);
    input.extend_from_slice(
        b"3 0\tobj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] >>\nendobj\n",
    );
    let mut source = SeekableSource::new(Cursor::new(input)).unwrap();
    let mut sink = WriteSink::new(Vec::<u8>::new());
    let error = run_native(copy_pdf(
        &mut source,
        &mut sink,
        &Limits::default(),
        &CancelAfter::Never,
    ))
    .expect_err("live object beyond PDF end must not be dropped");
    assert!(
        matches!(
            error,
            Error::Pdf {
                kind: PdfErrorKind::Malformed | PdfErrorKind::AmbiguousRepair,
                ..
            }
        ),
        "{error}"
    );
    assert!(sink.into_inner().is_empty());
}

#[test]
fn permitted_whitespace_after_eof_stays_byte_identical() {
    let mut input = read(fixture("valid_nested_outline.pdf")).unwrap();
    input.extend_from_slice(b" \t\r\n  \n");
    let mut source = SeekableSource::new(Cursor::new(&input)).unwrap();
    let mut sink = WriteSink::new(Vec::<u8>::new());
    run_native(copy_pdf(
        &mut source,
        &mut sink,
        &Limits::default(),
        &CancelAfter::Never,
    ))
    .unwrap();
    assert_eq!(sink.into_inner(), input);
}

#[test]
fn identical_duplicate_page_box_without_tail_is_repaired() {
    let mut input = read(fixture("repairable_duplicate_mediabox_tail.pdf")).unwrap();
    let marker = b"%%EOF\n";
    let at = input
        .windows(marker.len())
        .rposition(|part| part == marker)
        .unwrap();
    input.truncate(at + marker.len());
    let mut source = SeekableSource::new(Cursor::new(input)).unwrap();
    let mut output = TempPdf::new("duplicate-only");
    run_native(copy_pdf(
        &mut source,
        &mut WriteSink::new(&mut output.file),
        &Limits::default(),
        &CancelAfter::Never,
    ))
    .unwrap();
    output.file.flush().unwrap();
    check_pdf(&output.path, 2);
}

fn write_pdf_without_outlines() -> Vec<u8> {
    write_pdf_with_catalog(b"<< /Type /Catalog /Pages 2 0 R >>", None)
}

fn write_pdf_with_catalog(catalog_body: &[u8], extra_body: Option<&[u8]>) -> Vec<u8> {
    write_pdf_with_catalog_page(
        catalog_body,
        extra_body,
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Resources << >> /Contents 4 0 R >>",
    )
}

fn write_pdf_with_catalog_page(
    catalog_body: &[u8],
    extra_body: Option<&[u8]>,
    page_body: &[u8],
) -> Vec<u8> {
    write_pdf_with_catalog_page_and_content(
        catalog_body,
        extra_body,
        page_body,
        b"0 0 0 rg 10 10 40 20 re f\n",
    )
}

fn write_pdf_with_catalog_page_and_content(
    catalog_body: &[u8],
    extra_body: Option<&[u8]>,
    page_body: &[u8],
    content_bytes: &[u8],
) -> Vec<u8> {
    let limits = Limits::default();
    let mut sink = WriteSink::new(Vec::<u8>::new());
    run_native(async {
        let mut writer = PdfWriter::new(&mut sink, &limits, &CancelAfter::Never).await?;
        let catalog = writer.reserve_object()?;
        let pages = writer.reserve_object()?;
        let page = writer.reserve_object()?;
        let content = writer.reserve_object()?;
        let length = writer.reserve_object()?;
        let extra = extra_body.map(|_| writer.reserve_object()).transpose()?;
        writer.write_object(catalog, catalog_body).await?;
        writer
            .write_object(pages, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
            .await?;
        writer.write_object(page, page_body).await?;
        writer.begin_stream(content, length, b"").await?;
        writer.write_stream_bytes(content_bytes).await?;
        writer.end_stream().await?;
        if let (Some(extra), Some(body)) = (extra, extra_body) {
            writer.write_object(extra, body).await?;
        }
        writer.finish(catalog).await
    })
    .unwrap();
    sink.into_inner()
}

#[test]
fn outline_import_keeps_pages_and_rendered_content() {
    let input = write_pdf_without_outlines();
    let mut source = SeekableSource::new(Cursor::new(&input)).unwrap();
    let limits = Limits::default();
    let index = run_native(PdfIndex::open(
        &mut source,
        PdfRange {
            offset: 0,
            length: input.len() as u64,
        },
        &limits,
        &CancelAfter::Never,
    ))
    .unwrap();
    let mut output = TempPdf::new("outline-import");
    let report = run_native(async {
        let mut sink = WriteSink::new(&mut output.file);
        let mut appender =
            PdfOutlineAppender::begin(&mut source, &mut sink, &index, &limits, &CancelAfter::Never)
                .await?;
        appender
            .add_bookmark(Bookmark {
                depth: 0,
                title: "Part One".into(),
                page_index: 0,
            })
            .await?;
        appender
            .add_bookmark(Bookmark {
                depth: 1,
                title: "章节😀".into(),
                page_index: 0,
            })
            .await?;
        appender.finish().await
    })
    .unwrap();
    output.file.flush().unwrap();
    assert_eq!(report.pages_converted, 1);
    assert_eq!(report.bookmarks_written, 2);
    check_pdf(&output.path, 1);
    let outline = check_command(
        Command::new("mutool")
            .arg("show")
            .arg(&output.path)
            .arg("outline"),
        "mutool show outline",
    );
    let outline = String::from_utf8(outline).unwrap();
    assert!(outline.contains("Part One"), "{outline}");
    assert!(outline.contains("章节😀"), "{outline}");
    assert!(outline.contains("#page=1"), "{outline}");

    let mut input_file = TempPdf::new("outline-original");
    input_file.file.write_all(&input).unwrap();
    input_file.file.flush().unwrap();
    assert_eq!(
        render_page(&input_file.path, 1),
        render_page(&output.path, 1)
    );
}

fn fragment_ref(number: u32) -> PdfRef {
    PdfRef {
        number,
        generation: 0,
    }
}

fn add_fragment_object(bytes: &mut Vec<u8>, number: u32, body: &[u8]) -> FragmentObject {
    let offset = bytes.len() as u64;
    bytes.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
    bytes.extend_from_slice(body);
    bytes.extend_from_slice(b"\nendobj\n");
    FragmentObject {
        reference: fragment_ref(number),
        range: PdfRange {
            offset,
            length: bytes.len() as u64 - offset,
        },
    }
}

#[test]
fn fragment_rebuild_preserves_explicit_page_order_and_binary_stream() {
    let mut bytes = b"CAJ\0unrelated metadata\n".to_vec();
    let second = add_fragment_object(
        &mut bytes,
        9,
        b"<< /Type /Page /Parent 5 0 R /MediaBox [0 0 400 250] /Resources << >> >>",
    );
    let first = add_fragment_object(
        &mut bytes,
        3,
        b"<< /Type /Page /Parent 5 0 R /MediaBox [0 0 200 300] /Resources << >> >>",
    );
    let fake = b"endobj\nxref\nstartxref\n%%EOF\n";
    let scalar = add_fragment_object(&mut bytes, 4, fake.len().to_string().as_bytes());
    let mut body = b"<< /Length 4 0 R >>\nstream\n".to_vec();
    body.extend_from_slice(fake);
    body.extend_from_slice(b"\nendstream");
    let stream = add_fragment_object(&mut bytes, 6, &body);
    let objects = [second, first, scalar, stream];
    let pages = [fragment_ref(9), fragment_ref(3)];
    let plan = FragmentPlan {
        objects: &objects,
        pages: &pages,
        pages_root: fragment_ref(5),
        catalog: None,
    };
    let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
    let mut output = TempPdf::new("fragment-rebuild");
    let report = run_native(reconstruct_fragment(
        &mut source,
        &mut WriteSink::new(&mut output.file),
        &plan,
        &Limits::default(),
        &CancelAfter::Never,
    ))
    .unwrap();
    output.file.flush().unwrap();
    assert_eq!(report.pages_converted, 2);
    check_pdf(&output.path, 2);
    let info = check_command(
        Command::new("pdfinfo")
            .args(["-box", "-f", "1", "-l", "2"])
            .arg(&output.path),
        "pdfinfo -box",
    );
    let info = String::from_utf8(info).unwrap();
    let compact = info.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(compact.contains("Page 1 size: 400 x 250 pts"), "{info}");
    assert!(compact.contains("Page 2 size: 200 x 300 pts"), "{info}");
}

#[test]
fn fragment_with_unchecked_existing_outline_is_rejected_before_output() {
    let mut bytes = Vec::new();
    let objects = [
        add_fragment_object(
            &mut bytes,
            1,
            b"<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>",
        ),
        add_fragment_object(
            &mut bytes,
            2,
            b"<< /Type /Pages /Count 1 /Kids [3 0 R] /MediaBox [0 0 100 100] >>",
        ),
        add_fragment_object(
            &mut bytes,
            3,
            b"<< /Type /Page /Parent 2 0 R /Resources << >> >>",
        ),
        add_fragment_object(
            &mut bytes,
            4,
            b"<< /Type /Outlines /First 5 0 R /Last 5 0 R /Count 1 >>",
        ),
        add_fragment_object(
            &mut bytes,
            5,
            b"<< /Title (X) /Parent 3 0 R /Dest [3 0 R /Fit] >>",
        ),
    ];
    let pages = [fragment_ref(3)];
    let plan = FragmentPlan {
        objects: &objects,
        pages: &pages,
        pages_root: fragment_ref(2),
        catalog: Some(fragment_ref(1)),
    };
    let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
    let mut sink = WriteSink::new(Vec::<u8>::new());
    let error = run_native(reconstruct_fragment(
        &mut source,
        &mut sink,
        &plan,
        &Limits::default(),
        &CancelAfter::Never,
    ))
    .expect_err("fragment outlines require checked import");
    assert!(
        matches!(
            error,
            Error::Pdf {
                object: Some((1, 0)),
                kind: PdfErrorKind::UnsupportedFeature,
                ..
            }
        ),
        "{error}"
    );
    assert!(sink.into_inner().is_empty());
}

#[test]
fn fragment_page_tree_root_rejects_a_present_non_reference_parent() {
    for parent in [b"999".as_slice(), b"[1 0 R]".as_slice()] {
        let mut bytes = Vec::new();
        let catalog = add_fragment_object(&mut bytes, 1, b"<< /Type /Catalog /Pages 2 0 R >>");
        let mut root_body = b"<< /Type /Pages /Parent ".to_vec();
        root_body.extend_from_slice(parent);
        root_body.extend_from_slice(b" /Count 1 /Kids [3 0 R] /MediaBox [0 0 100 100] >>");
        let root = add_fragment_object(&mut bytes, 2, &root_body);
        let page = add_fragment_object(
            &mut bytes,
            3,
            b"<< /Type /Page /Parent 2 0 R /Resources << >> >>",
        );
        let objects = [catalog, root, page];
        let pages = [fragment_ref(3)];
        let plan = FragmentPlan {
            objects: &objects,
            pages: &pages,
            pages_root: fragment_ref(2),
            catalog: Some(fragment_ref(1)),
        };
        let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
        let mut sink = WriteSink::new(Vec::<u8>::new());
        let error = run_native(reconstruct_fragment(
            &mut source,
            &mut sink,
            &plan,
            &Limits::default(),
            &CancelAfter::Never,
        ))
        .expect_err("present Pages Parent must be a reference");
        assert!(
            matches!(
                error,
                Error::Pdf {
                    object: Some((2, 0)),
                    kind: PdfErrorKind::Malformed,
                    ..
                }
            ),
            "{error}"
        );
        assert!(sink.into_inner().is_empty());
    }
}

#[test]
fn malformed_fixture_is_located_and_never_partially_copied() {
    for name in [
        "invalid_stream_length.pdf",
        "invalid_xref_offset.pdf",
        "invalid_page_count.pdf",
        "duplicate_object.pdf",
        "truncated_xref.pdf",
    ] {
        let mut source = SeekableSource::new(File::open(fixture(name)).unwrap()).unwrap();
        let mut sink = WriteSink::new(Vec::<u8>::new());
        let error = run_native(copy_pdf(
            &mut source,
            &mut sink,
            &Limits::default(),
            &CancelAfter::Never,
        ))
        .expect_err(name);
        assert!(
            matches!(
                error,
                Error::Pdf {
                    kind: PdfErrorKind::Malformed,
                    ..
                }
            ),
            "{name}: {error}"
        );
        assert!(
            error.to_string().contains("PDF at byte "),
            "{name}: {error}"
        );
        assert!(sink.into_inner().is_empty(), "{name} wrote output");
    }
}

#[test]
fn conflicting_duplicate_page_box_is_not_guessed() {
    let mut bytes = read(fixture("repairable_duplicate_mediabox_tail.pdf")).unwrap();
    let marker = b"/MediaBox [0 0 612 792]";
    let second = bytes
        .windows(marker.len())
        .enumerate()
        .filter_map(|(i, v)| (v == marker).then_some(i))
        .nth(1)
        .unwrap();
    bytes[second + marker.len() - 4] = b'8';
    let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
    let mut sink = WriteSink::new(Vec::<u8>::new());
    let error = run_native(copy_pdf(
        &mut source,
        &mut sink,
        &Limits::default(),
        &CancelAfter::Never,
    ))
    .unwrap_err();
    assert!(
        matches!(
            error,
            Error::Pdf {
                kind: PdfErrorKind::AmbiguousRepair,
                ..
            }
        ),
        "{error}"
    );
    assert!(error.to_string().contains("object 2 0"), "{error}");
    assert!(sink.into_inner().is_empty());
}

fn inspect_bytes(bytes: Vec<u8>) -> caj2pdf_core::Result<PdfIndex> {
    inspect_bytes_with(bytes, &CancelAfter::Never)
}

fn inspect_bytes_with(
    bytes: Vec<u8>,
    cancellation: &CancelAfter,
) -> caj2pdf_core::Result<PdfIndex> {
    let length = bytes.len() as u64;
    let mut source = SeekableSource::new(Cursor::new(bytes))?;
    run_native(PdfIndex::open(
        &mut source,
        PdfRange { offset: 0, length },
        &Limits::default(),
        cancellation,
    ))
}

fn replace_once_same_len(bytes: &mut [u8], old: &[u8], new: &[u8]) {
    assert_eq!(old.len(), new.len());
    let positions: Vec<_> = bytes
        .windows(old.len())
        .enumerate()
        .filter_map(|(index, window)| (window == old).then_some(index))
        .collect();
    assert_eq!(positions.len(), 1, "test marker must be unique");
    bytes[positions[0]..positions[0] + old.len()].copy_from_slice(new);
}

fn replace_once(bytes: &mut Vec<u8>, old: &[u8], new: &[u8]) {
    let positions: Vec<_> = bytes
        .windows(old.len())
        .enumerate()
        .filter_map(|(index, window)| (window == old).then_some(index))
        .collect();
    assert_eq!(positions.len(), 1, "test marker must be unique");
    bytes.splice(positions[0]..positions[0] + old.len(), new.iter().copied());
}

#[test]
fn higher_pdf_version_and_encryption_are_typed_unsupported_inputs() {
    let mut version = write_pdf_without_outlines();
    version[..8].copy_from_slice(b"%PDF-2.0");
    let version_error = inspect_bytes(version).err().unwrap();
    assert!(matches!(
        version_error,
        Error::Pdf {
            offset: 0,
            kind: PdfErrorKind::UnsupportedFeature,
            ..
        }
    ));
    assert!(
        version_error
            .to_string()
            .contains("unsupported feature PDF at byte 0")
    );

    let mut encrypted = write_pdf_without_outlines();
    let marker = b"/Root 1 0 R >>";
    let at = encrypted
        .windows(marker.len())
        .rposition(|part| part == marker)
        .unwrap();
    encrypted.splice(
        at..at + marker.len(),
        b"/Root 1 0 R /Encrypt << /Filter /Standard >> >>"
            .iter()
            .copied(),
    );
    let encrypted_error = inspect_bytes(encrypted).err().unwrap();
    assert!(matches!(
        encrypted_error,
        Error::Pdf {
            kind: PdfErrorKind::Encrypted,
            ..
        }
    ));
    assert!(
        encrypted_error
            .to_string()
            .contains("encrypted PDF at byte")
    );
}

#[test]
fn recognized_signature_indicators_block_modification() {
    for (catalog, extra) in [
        (
            b"<< /Type /Catalog /Pages 2 0 R /Perms << >> >>".as_slice(),
            None,
        ),
        (
            b"<< /Type /Catalog /Pages 2 0 R /AcroForm 6 0 R >>".as_slice(),
            Some(b"<< /SigFlags 1 >>".as_slice()),
        ),
    ] {
        let error = inspect_bytes(write_pdf_with_catalog(catalog, extra))
            .err()
            .unwrap();
        assert!(
            matches!(
                error,
                Error::Pdf {
                    kind: PdfErrorKind::UnsupportedFeature,
                    ..
                }
            ),
            "{error}"
        );
    }
}

#[test]
fn classic_incremental_prev_and_crlf_xref_are_accepted() {
    let mut crlf = write_pdf_without_outlines();
    let marker = b"xref\n0 6\n";
    let header = crlf
        .windows(marker.len())
        .position(|part| part == marker)
        .unwrap();
    for row in crlf[header + marker.len()..header + marker.len() + 6 * 20].chunks_exact_mut(20) {
        assert_eq!(&row[18..], b" \n");
        row[18] = b'\r';
    }
    let crlf_index = inspect_bytes(crlf.clone()).unwrap();
    assert_eq!(crlf_index.pages().len(), 1);
    let mut crlf_pdf = TempPdf::new("crlf-xref");
    crlf_pdf.file.write_all(&crlf).unwrap();
    crlf_pdf.file.flush().unwrap();
    check_pdf(&crlf_pdf.path, 1);

    let mut revised = write_pdf_without_outlines();
    let marker = b"startxref\n";
    let marker_at = revised
        .windows(marker.len())
        .rposition(|part| part == marker)
        .unwrap();
    let digits = revised[marker_at + marker.len()..]
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .copied()
        .collect::<Vec<_>>();
    let previous_xref = std::str::from_utf8(&digits)
        .unwrap()
        .parse::<u64>()
        .unwrap();
    revised.push(b'\n');
    let catalog_offset = revised.len();
    revised.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    let xref_offset = revised.len();
    revised.extend_from_slice(
        format!(
            "xref\n1 1\n{catalog_offset:010} 00000 n \ntrailer\n<< /Size 6 /Root 1 0 R /Prev {previous_xref} >>\nstartxref\n{xref_offset}\n%%EOF\n"
        )
        .as_bytes(),
    );
    let index = inspect_bytes(revised.clone()).unwrap();
    assert_eq!(index.pages().len(), 1);
    let mut revised_pdf = TempPdf::new("incremental-prev");
    revised_pdf.file.write_all(&revised).unwrap();
    revised_pdf.file.flush().unwrap();
    check_pdf(&revised_pdf.path, 1);
}

#[test]
fn incremental_xref_cannot_activate_an_object_inside_a_live_stream() {
    let fake = b"6 0 obj\n0\nendobj\n";
    let mut bytes = write_pdf_with_catalog_page_and_content(
        b"<< /Type /Catalog /Pages 2 0 R >>",
        None,
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Resources << >> /Contents 4 0 R >>",
        fake,
    );
    let fake_at = bytes
        .windows(fake.len())
        .position(|part| part == fake)
        .unwrap();
    let marker = b"startxref\n";
    let marker_at = bytes
        .windows(marker.len())
        .rposition(|part| part == marker)
        .unwrap();
    let digits = bytes[marker_at + marker.len()..]
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .copied()
        .collect::<Vec<_>>();
    let previous_xref = std::str::from_utf8(&digits)
        .unwrap()
        .parse::<u64>()
        .unwrap();
    let xref_at = bytes.len();
    bytes.extend_from_slice(
        format!(
            "xref\n6 1\n{fake_at:010} 00000 n \ntrailer\n<< /Size 7 /Root 1 0 R /Prev {previous_xref} >>\nstartxref\n{xref_at}\n%%EOF\n"
        )
        .as_bytes(),
    );
    let error = inspect_bytes(bytes)
        .err()
        .expect("live PDF objects must not overlap");
    assert!(
        matches!(
            error,
            Error::Pdf {
                kind: PdfErrorKind::Malformed,
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn malformed_outline_links_and_unsupported_named_destinations_are_located() {
    let original = read(fixture("valid_nested_outline.pdf")).unwrap();
    let mut wrong_parent = original.clone();
    replace_once_same_len(&mut wrong_parent, b"/Parent 7 0 R", b"/Parent 6 0 R");
    assert!(matches!(
        inspect_bytes(wrong_parent),
        Err(Error::Pdf {
            kind: PdfErrorKind::Malformed,
            ..
        })
    ));

    let mut wrong_destination = original.clone();
    replace_once_same_len(
        &mut wrong_destination,
        b"/Dest [3 0 R /Fit]",
        b"/Dest [6 0 R /Fit]",
    );
    assert!(matches!(
        inspect_bytes(wrong_destination),
        Err(Error::Pdf {
            kind: PdfErrorKind::Malformed,
            ..
        })
    ));

    let mut named_destination = original;
    let old = b"/Dest [3 0 R /Fit]";
    let mut named = b"/Dest (named)".to_vec();
    named.resize(old.len(), b' ');
    replace_once_same_len(&mut named_destination, old, &named);
    assert!(matches!(
        inspect_bytes(named_destination),
        Err(Error::Pdf {
            kind: PdfErrorKind::UnsupportedFeature,
            ..
        })
    ));
}

#[test]
fn page_contents_must_resolve_to_a_stream() {
    let mut bytes = read(fixture("valid_nested_outline.pdf")).unwrap();
    replace_once_same_len(&mut bytes, b"/Contents 5 0 R", b"/Contents 1 0 R");
    let error = inspect_bytes(bytes)
        .err()
        .expect("Catalog is not a content stream");
    assert!(
        matches!(
            error,
            Error::Pdf {
                kind: PdfErrorKind::Malformed,
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn embedded_pdf_error_offset_is_absolute_in_its_source() {
    let prefix = b"CAJ\0source header\n";
    let pdf = read(fixture("invalid_page_count.pdf")).unwrap();
    let mut bytes = prefix.to_vec();
    bytes.extend_from_slice(&pdf);
    bytes.extend_from_slice(b"unrelated container suffix");
    let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
    let error = run_native(PdfIndex::open(
        &mut source,
        PdfRange {
            offset: prefix.len() as u64,
            length: pdf.len() as u64,
        },
        &Limits::default(),
        &CancelAfter::Never,
    ))
    .err()
    .unwrap();
    assert!(
        matches!(error, Error::Pdf { offset, kind: PdfErrorKind::Malformed, .. } if offset >= prefix.len() as u64),
        "{error}"
    );
}

#[test]
fn nested_duplicate_keys_and_bad_trailer_id_are_rejected() {
    let nested = write_pdf_with_catalog_page(
        b"<< /Type /Catalog /Pages 2 0 R >>",
        None,
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Resources << /ProcSet [/PDF] /ProcSet [/Text] >> /Contents 4 0 R >>",
    );
    let nested_error = inspect_bytes(nested)
        .err()
        .expect("nested duplicate is invalid");
    assert!(
        matches!(
            nested_error,
            Error::Pdf {
                kind: PdfErrorKind::AmbiguousRepair,
                ..
            }
        ),
        "{nested_error}"
    );

    let mut invalid_id = write_pdf_without_outlines();
    let marker = b"/Root 1 0 R >>";
    let at = invalid_id
        .windows(marker.len())
        .rposition(|part| part == marker)
        .unwrap();
    invalid_id.splice(
        at..at + marker.len(),
        b"/Root 1 0 R /ID /Bogus >>".iter().copied(),
    );
    let id_error = inspect_bytes(invalid_id)
        .err()
        .expect("trailer ID needs two strings");
    assert!(
        matches!(
            id_error,
            Error::Pdf {
                kind: PdfErrorKind::Malformed,
                ..
            }
        ),
        "{id_error}"
    );

    let mut dangling_info = write_pdf_without_outlines();
    let at = dangling_info
        .windows(marker.len())
        .rposition(|part| part == marker)
        .unwrap();
    dangling_info.splice(
        at..at + marker.len(),
        b"/Root 1 0 R /Info 9 0 R >>".iter().copied(),
    );
    let info_error = inspect_bytes(dangling_info)
        .err()
        .expect("trailer Info must resolve");
    assert!(
        matches!(
            info_error,
            Error::Pdf {
                kind: PdfErrorKind::Malformed,
                ..
            }
        ),
        "{info_error}"
    );
}

#[test]
fn fragment_without_page_media_box_fails_before_output() {
    let mut bytes = b"CAJ\0".to_vec();
    let page = add_fragment_object(
        &mut bytes,
        1,
        b"<< /Type /Page /Parent 2 0 R /Resources << >> >>",
    );
    let pages = [fragment_ref(1)];
    let objects = [page];
    let plan = FragmentPlan {
        objects: &objects,
        pages: &pages,
        pages_root: fragment_ref(2),
        catalog: None,
    };
    let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
    let mut sink = WriteSink::new(Vec::<u8>::new());
    let error = run_native(reconstruct_fragment(
        &mut source,
        &mut sink,
        &plan,
        &Limits::default(),
        &CancelAfter::Never,
    ))
    .expect_err("undefined MediaBox must fail");
    assert!(
        matches!(
            error,
            Error::Pdf {
                kind: PdfErrorKind::Malformed,
                ..
            }
        ),
        "{error}"
    );
    assert!(sink.into_inner().is_empty());
}

#[test]
fn fragment_page_contents_must_resolve_to_stream() {
    let mut bytes = b"CAJ\0".to_vec();
    let page = add_fragment_object(
        &mut bytes,
        3,
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Resources << >> /Contents 4 0 R >>",
    );
    let scalar = add_fragment_object(&mut bytes, 4, b"0");
    let pages = [fragment_ref(3)];
    let objects = [page, scalar];
    let plan = FragmentPlan {
        objects: &objects,
        pages: &pages,
        pages_root: fragment_ref(2),
        catalog: None,
    };
    let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
    let mut sink = WriteSink::new(Vec::<u8>::new());
    let error = run_native(reconstruct_fragment(
        &mut source,
        &mut sink,
        &plan,
        &Limits::default(),
        &CancelAfter::Never,
    ))
    .expect_err("page contents must be stream data");
    assert!(
        matches!(
            error,
            Error::Pdf {
                kind: PdfErrorKind::Malformed,
                ..
            }
        ),
        "{error}"
    );
    assert!(sink.into_inner().is_empty());
}

#[test]
fn fragment_top_level_duplicate_dictionary_key_is_rejected() {
    let mut bytes = b"CAJ\0".to_vec();
    let page = add_fragment_object(
        &mut bytes,
        3,
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /MediaBox [0 0 200 100] /Resources << >> >>",
    );
    let pages = [fragment_ref(3)];
    let objects = [page];
    let plan = FragmentPlan {
        objects: &objects,
        pages: &pages,
        pages_root: fragment_ref(2),
        catalog: None,
    };
    let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
    let mut sink = WriteSink::new(Vec::<u8>::new());
    let error = run_native(reconstruct_fragment(
        &mut source,
        &mut sink,
        &plan,
        &Limits::default(),
        &CancelAfter::Never,
    ))
    .expect_err("ambiguous fragment dictionary must fail");
    assert!(
        matches!(
            error,
            Error::Pdf {
                kind: PdfErrorKind::AmbiguousRepair,
                ..
            }
        ),
        "{error}"
    );
    assert!(sink.into_inner().is_empty());
}

#[test]
fn page_limit_reports_the_pdf_object_and_source_offset() {
    let input = read(fixture("valid_nested_outline.pdf")).unwrap();
    let mut source = SeekableSource::new(Cursor::new(input.clone())).unwrap();
    let limits = Limits {
        max_pages: 1,
        ..Limits::default()
    };
    let error = run_native(PdfIndex::open(
        &mut source,
        PdfRange {
            offset: 0,
            length: input.len() as u64,
        },
        &limits,
        &CancelAfter::Never,
    ))
    .err()
    .unwrap();
    assert!(
        matches!(
            error,
            Error::PdfLimitExceeded {
                offset,
                object: Some((2, 0)),
                resource: "pages",
                limit: 1,
                attempted: 2,
            } if offset > 0
        ),
        "{error}"
    );
    let message = error.to_string();
    assert!(
        message.contains("PDF pages limit exceeded at byte "),
        "{message}"
    );
    assert!(
        message.contains("object 2 0: maximum 1, attempted 2"),
        "{message}"
    );
}

#[test]
fn pdf_size_limits_keep_source_location_in_each_entry_point() {
    assert_eq!(
        Error::PdfLimitExceeded {
            offset: 7,
            object: None,
            resource: "bytes",
            limit: 1,
            attempted: 2,
        }
        .to_string(),
        "PDF bytes limit exceeded at byte 7: maximum 1, attempted 2"
    );
    let input = write_pdf_without_outlines();
    let input_limits = Limits {
        max_input_bytes: input.len() as u64 - 1,
        ..Limits::default()
    };
    let mut source = SeekableSource::new(Cursor::new(&input)).unwrap();
    let mut sink = WriteSink::new(Vec::<u8>::new());
    let error = run_native(copy_pdf(
        &mut source,
        &mut sink,
        &input_limits,
        &CancelAfter::Never,
    ))
    .expect_err("oversized PDF input must fail before writing");
    assert!(
        matches!(
            error,
            Error::PdfLimitExceeded {
                offset: 0,
                resource: "input bytes",
                ..
            }
        ),
        "{error}"
    );
    assert!(sink.into_inner().is_empty());

    let mut source = SeekableSource::new(Cursor::new(&input)).unwrap();
    let index = run_native(PdfIndex::open(
        &mut source,
        PdfRange {
            offset: 0,
            length: input.len() as u64,
        },
        &Limits::default(),
        &CancelAfter::Never,
    ))
    .unwrap();
    let mut sink = WriteSink::new(Vec::<u8>::new());
    let error = run_native(PdfOutlineAppender::begin(
        &mut source,
        &mut sink,
        &index,
        &input_limits,
        &CancelAfter::Never,
    ))
    .err()
    .unwrap();
    assert!(
        matches!(
            error,
            Error::PdfLimitExceeded {
                offset: 0,
                object: Some((1, 0)),
                resource: "input bytes",
                ..
            }
        ),
        "{error}"
    );
    assert!(sink.into_inner().is_empty());
    let output_limits = Limits {
        max_output_bytes: input.len() as u64 - 1,
        ..Limits::default()
    };
    let mut sink = WriteSink::new(Vec::<u8>::new());
    let error = PdfOutlineAppender::begin(
        &mut source,
        &mut sink,
        &index,
        &output_limits,
        &CancelAfter::Never,
    );
    let error = run_native(error).err().unwrap();
    assert!(
        matches!(
            error,
            Error::PdfLimitExceeded {
                offset,
                object: Some((1, 0)),
                resource: "output bytes",
                ..
            } if offset > 0
        ),
        "{error}"
    );
    assert!(sink.into_inner().is_empty());

    let mut fragment_bytes = Vec::new();
    let page = add_fragment_object(
        &mut fragment_bytes,
        3,
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] >>",
    );
    let objects = [page];
    let pages = [fragment_ref(3)];
    let plan = FragmentPlan {
        objects: &objects,
        pages: &pages,
        pages_root: fragment_ref(2),
        catalog: None,
    };
    let limits = Limits {
        max_input_bytes: fragment_bytes.len() as u64 - 1,
        ..Limits::default()
    };
    let mut source = SeekableSource::new(Cursor::new(fragment_bytes)).unwrap();
    let mut sink = WriteSink::new(Vec::<u8>::new());
    let error = run_native(reconstruct_fragment(
        &mut source,
        &mut sink,
        &plan,
        &limits,
        &CancelAfter::Never,
    ))
    .expect_err("oversized fragment source must fail before writing");
    assert!(
        matches!(
            error,
            Error::PdfLimitExceeded {
                offset: 0,
                object: Some((3, 0)),
                resource: "input bytes",
                ..
            }
        ),
        "{error}"
    );
    assert!(sink.into_inner().is_empty());
}

#[test]
fn embedded_pdf_limits_count_only_the_selected_range_or_fragment_spans() {
    let pdf = write_pdf_without_outlines();
    let prefix = vec![b'X'; 4096];
    let mut container = prefix.clone();
    container.extend_from_slice(&pdf);
    container.extend_from_slice(&vec![b'Y'; 4096]);
    let limits = Limits {
        max_input_bytes: pdf.len() as u64,
        ..Limits::default()
    };
    let mut source = SeekableSource::new(Cursor::new(container)).unwrap();
    let mut sink = WriteSink::new(Vec::<u8>::new());
    let report = run_native(copy_pdf_range(
        &mut source,
        &mut sink,
        PdfRange {
            offset: prefix.len() as u64,
            length: pdf.len() as u64,
        },
        &limits,
        &CancelAfter::Never,
    ))
    .unwrap();
    assert_eq!(report.pages_converted, 1);
    assert_eq!(sink.into_inner(), pdf);

    let mut fragment_container = vec![b'X'; 4096];
    let page = add_fragment_object(
        &mut fragment_container,
        3,
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] >>",
    );
    fragment_container.extend_from_slice(&vec![b'Y'; 4096]);
    let objects = [page];
    let pages = [fragment_ref(3)];
    let plan = FragmentPlan {
        objects: &objects,
        pages: &pages,
        pages_root: fragment_ref(2),
        catalog: None,
    };
    let limits = Limits {
        max_input_bytes: page.range.length,
        ..Limits::default()
    };
    let mut source = SeekableSource::new(Cursor::new(fragment_container)).unwrap();
    let mut output = TempPdf::new("embedded-fragment-range");
    let report = run_native(reconstruct_fragment(
        &mut source,
        &mut WriteSink::new(&mut output.file),
        &plan,
        &limits,
        &CancelAfter::Never,
    ))
    .unwrap();
    output.file.flush().unwrap();
    assert_eq!(report.pages_converted, 1);
    check_pdf(&output.path, 1);
}

#[test]
fn appender_rejects_a_source_that_shrank_after_inspection() {
    let input = write_pdf_without_outlines();
    let mut original = SeekableSource::new(Cursor::new(&input)).unwrap();
    let index = run_native(PdfIndex::open(
        &mut original,
        PdfRange {
            offset: 0,
            length: input.len() as u64,
        },
        &Limits::default(),
        &CancelAfter::Never,
    ))
    .unwrap();
    let mut shortened = SeekableSource::new(Cursor::new(&input[..input.len() - 1])).unwrap();
    let mut sink = WriteSink::new(Vec::<u8>::new());
    let error = run_native(PdfOutlineAppender::begin(
        &mut shortened,
        &mut sink,
        &index,
        &Limits::default(),
        &CancelAfter::Never,
    ))
    .err()
    .unwrap();
    assert!(matches!(error, Error::InvalidInput { .. }), "{error}");
    assert!(sink.into_inner().is_empty());
}

struct OverreportingPdfSource;

impl RangedSource for OverreportingPdfSource {
    fn size(&self) -> u64 {
        128
    }

    async fn read_at(
        &mut self,
        _offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        Ok(destination.len() + 1)
    }
}

struct HugeNoReadSource;

impl RangedSource for HugeNoReadSource {
    fn size(&self) -> u64 {
        u64::MAX
    }

    async fn read_at(
        &mut self,
        _offset: u64,
        _destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        panic!("overflowing fragment spans must fail before any read")
    }
}

#[test]
fn fragment_span_total_cannot_overflow_before_preflight_reads() {
    let objects = [
        FragmentObject {
            reference: fragment_ref(1),
            range: PdfRange {
                offset: 0,
                length: u64::MAX,
            },
        },
        FragmentObject {
            reference: fragment_ref(3),
            range: PdfRange {
                offset: 0,
                length: 2,
            },
        },
    ];
    let pages = [fragment_ref(3)];
    let plan = FragmentPlan {
        objects: &objects,
        pages: &pages,
        pages_root: fragment_ref(2),
        catalog: None,
    };
    let limits = Limits {
        max_input_bytes: u64::MAX,
        ..Limits::default()
    };
    let mut source = HugeNoReadSource;
    let mut sink = WriteSink::new(Vec::<u8>::new());
    let error = run_native(reconstruct_fragment(
        &mut source,
        &mut sink,
        &plan,
        &limits,
        &CancelAfter::Never,
    ))
    .expect_err("sum of fragment spans overflows 64 bits");
    assert!(
        matches!(
            error,
            Error::PdfLimitExceeded {
                object: Some((3, 0)),
                resource: "input bytes",
                attempted: u64::MAX,
                ..
            }
        ),
        "{error}"
    );
    assert!(sink.into_inner().is_empty());
}

#[test]
fn pdf_copy_rejects_a_source_that_overreports_a_read() {
    let mut source = OverreportingPdfSource;
    let mut sink = WriteSink::new(Vec::<u8>::new());
    let error = run_native(copy_pdf(
        &mut source,
        &mut sink,
        &Limits::default(),
        &CancelAfter::Never,
    ))
    .expect_err("overreported source bytes must fail");
    assert!(matches!(error, Error::InvalidInput { .. }), "{error}");
    assert!(sink.into_inner().is_empty());
}
