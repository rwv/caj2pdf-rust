// SPDX-License-Identifier: MIT

//! End-to-end CAJ conversion using independently authored container bytes.
//! The header, twelve-byte page rows, and 308-byte TOC records follow the
//! public observations registered in `docs/caj-format.md`.

use caj2pdf_core::{
    ConversionOptions, Error, Limits, NeverCancel, RangedSource, SequentialSink,
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
type CajFieldCase = (&'static str, Vec<u8>, u64, Option<u32>, &'static str);

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

fn fragment_caj(body: &[u8], page_objects: &[u32]) -> Vec<u8> {
    const TABLE_START: usize = 0x400;
    assert!(!page_objects.is_empty());
    let body_start = TABLE_START + page_objects.len() * 12;
    let mut bytes = vec![0_u8; body_start];
    bytes[..4].copy_from_slice(b"CAJ\0");
    bytes[4..8].copy_from_slice(&[1, 0, 2, 0]);
    put_u32(&mut bytes, 0x10, page_objects.len() as u32);
    put_u32(&mut bytes, 0x14, TABLE_START as u32);
    for (index, page_object) in page_objects.iter().copied().enumerate() {
        let row = TABLE_START + index * 12;
        put_u32(
            &mut bytes,
            row,
            body_start as u32 + if index == 0 { 0 } else { body.len() as u32 },
        );
        put_u32(
            &mut bytes,
            row + 4,
            if index == 0 { body.len() as u32 } else { 0 },
        );
        put_u32(&mut bytes, row + 8, page_object);
    }
    bytes.extend_from_slice(body);
    bytes
}

fn one_page_body(annotation: Option<&str>, declared_stream_length: Option<usize>) -> Vec<u8> {
    const CONTENT: &str = "0 0 0 rg 10 10 30 30 re f";
    let mut body = Vec::new();
    let annots = if annotation.is_some() {
        " /Annots [12 0 R]"
    } else {
        ""
    };
    object(
        &mut body,
        9,
        &format!(
            "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 72 72] /Resources << >> /Contents 11 0 R{annots} >>"
        ),
    );
    object(&mut body, 5, "<< /Type /Pages /Count 1 /Kids [9 0 R] >>");
    object(
        &mut body,
        11,
        &format!(
            "<< /Length {} >>\nstream\n{CONTENT}\nendstream",
            declared_stream_length.unwrap_or(CONTENT.len())
        ),
    );
    if let Some(annotation) = annotation {
        object(&mut body, 12, annotation);
    }
    body
}

#[test]
fn a_synthetic_pages_node_cannot_satisfy_an_annotation_destination() {
    let mut body = Vec::new();
    object(
        &mut body,
        9,
        "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 72 72] /Resources << >> /Annots [12 0 R] >>",
    );
    object(
        &mut body,
        12,
        "<< /Type /Annot /Subtype /Link /Rect [0 0 1 1] /Dest [5 0 R /Fit] >>",
    );
    let error = rejected_without_output(&fragment_caj(&body, &[9]), &Limits::default());
    assert!(matches!(
        error,
        Error::Pdf {
            object: Some((12, 0)),
            reason: "indirect reference targets a missing object",
            ..
        }
    ));
}

#[test]
fn a_shared_indirect_link_destination_cannot_rewrite_an_appearance() {
    let mut body = one_page_body(
        Some("<< /Type /Annot /Subtype /Link /Rect [0 0 1 1] /Dest 10 0 R /AP 10 0 R >>"),
        None,
    );
    object(&mut body, 10, "[22 0 R /Fit]");
    let error = rejected_without_output(&fragment_caj(&body, &[9]), &Limits::default());
    assert!(matches!(
        error,
        Error::Pdf {
            object: Some((12, 0)),
            reason: "indirect reference targets a missing object",
            ..
        }
    ));
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

#[test]
fn conversion_uses_the_platform_neutral_short_io_contract() {
    struct ShortSource {
        bytes: Vec<u8>,
        largest_request: usize,
    }

    impl RangedSource for ShortSource {
        fn size(&self) -> u64 {
            self.bytes.len() as u64
        }

        async fn read_at(
            &mut self,
            offset: u64,
            destination: &mut [u8],
        ) -> caj2pdf_core::Result<usize> {
            self.largest_request = self.largest_request.max(destination.len());
            let start = offset as usize;
            let count = destination
                .len()
                .min(2)
                .min(self.bytes.len().saturating_sub(start));
            destination[..count].copy_from_slice(&self.bytes[start..start + count]);
            Ok(count)
        }
    }

    #[derive(Default)]
    struct ShortSink {
        bytes: Vec<u8>,
        largest_request: usize,
        flushed: bool,
    }

    impl SequentialSink for ShortSink {
        async fn write(&mut self, bytes: &[u8]) -> caj2pdf_core::Result<usize> {
            self.largest_request = self.largest_request.max(bytes.len());
            let count = bytes.len().min(2);
            self.bytes.extend_from_slice(&bytes[..count]);
            Ok(count)
        }

        async fn flush(&mut self) -> caj2pdf_core::Result<()> {
            self.flushed = true;
            Ok(())
        }
    }

    let mut source = ShortSource {
        bytes: tiny_caj().bytes,
        largest_request: 0,
    };
    let mut sink = ShortSink::default();
    let limits = Limits {
        io_chunk_bytes: 3,
        ..Limits::default()
    };
    let report = run_native(convert_caj(
        &mut source,
        &mut sink,
        ConversionOptions::default(),
        &limits,
        &NeverCancel,
    ))
    .expect("short ranged reads and sequential writes must convert CAJ");
    assert!(source.largest_request <= 3);
    assert!(sink.largest_request <= 3);
    assert!(sink.flushed);
    assert_eq!(report.pages_converted, 3);
    assert_eq!(report.output_bytes_written, sink.bytes.len() as u64);
    assert_eq!(inspect(&sink.bytes).pages().len(), 3);
}

fn rejected_without_output(input: &[u8], limits: &Limits) -> Error {
    let mut source = SeekableSource::new(Cursor::new(input)).unwrap();
    let mut output = Vec::new();
    let error = run_native(convert_caj(
        &mut source,
        &mut WriteSink::new(&mut output),
        ConversionOptions::default(),
        limits,
        &NeverCancel,
    ))
    .expect_err("invalid input must be rejected");
    assert!(output.is_empty(), "partial PDF was written before: {error}");
    error
}

struct OverreportingSource {
    size: u64,
}

impl RangedSource for OverreportingSource {
    fn size(&self) -> u64 {
        self.size
    }

    async fn read_at(
        &mut self,
        _offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        Ok(destination.len() + 1)
    }
}

fn assert_caj_error(
    label: &str,
    input: &[u8],
    expected_offset: u64,
    expected_record: Option<u32>,
    expected_reason: &str,
) {
    let error = rejected_without_output(input, &Limits::default());
    assert!(
        matches!(&error, Error::Caj { offset, record, reason }
            if *offset == expected_offset
                && *record == expected_record
                && reason.contains(expected_reason)),
        "{label}: {error}"
    );
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

fn rendered_page(path: &PathBuf) -> Vec<u8> {
    let result = Command::new("mutool")
        .args([
            "draw", "-q", "-F", "pnm", "-c", "gray", "-r", "36", "-o", "-",
        ])
        .arg(path)
        .arg("1")
        .output()
        .expect("mutool draw is required in CI");
    assert!(
        result.status.success(),
        "mutool draw failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    result.stdout
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

#[test]
fn joins_multiple_missing_page_tree_groups_in_table_order() {
    let mut body = Vec::new();
    object(
        &mut body,
        3,
        "<< /Type /Page /Parent 6 0 R /MediaBox [0 0 90 120] /Resources << >> >>",
    );
    object(
        &mut body,
        9,
        "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 72 144] /Resources << >> >>",
    );
    object(
        &mut body,
        5,
        "<< /Type /Pages /Parent 20 0 R /Count 1 /Kids [9 0 R] >>",
    );
    object(
        &mut body,
        6,
        "<< /Type /Pages /Parent 21 0 R /Count 1 /Kids [3 0 R] >>",
    );
    let input = fragment_caj(&body, &[9, 3]);
    let (output, report) = convert(&input, ConversionOptions::default(), &Limits::default())
        .expect("join two missing /Pages ancestor groups");
    assert_eq!(report.pages_converted, 2);
    assert_eq!(
        inspect(&output).pages(),
        &[
            PdfRef {
                number: 9,
                generation: 0
            },
            PdfRef {
                number: 3,
                generation: 0
            },
        ]
    );
    let pdf = String::from_utf8_lossy(&output);
    assert!(
        pdf.contains("20 0 obj\n<< /Type /Pages /Parent 22 0 R"),
        "{pdf}"
    );
    assert!(
        pdf.contains("21 0 obj\n<< /Type /Pages /Parent 22 0 R"),
        "{pdf}"
    );
    assert!(pdf.contains("22 0 obj\n<< /Type /Pages /Count 2"), "{pdf}");
    let file = TempPdf::write("multi-roots", &output);
    checked_command(Command::new("qpdf").arg("--check").arg(&file.0), "qpdf");
    let info = checked_command(Command::new("pdfinfo").arg("-box").arg(&file.0), "pdfinfo");
    assert!(info.contains("Pages:           2"), "{info}");
    assert!(!rendered_page(&file.0).is_empty());
}

#[test]
fn converts_many_pages_under_a_deep_shared_missing_parent_chain() {
    const DEPTH: u32 = 128;
    const PAGE_COUNT: u32 = 128;
    const FIRST_GROUP: u32 = 1000;
    const FIRST_PAGE: u32 = 2000;
    const MISSING_ROOT: u32 = 5000;

    let page_ids: Vec<u32> = (FIRST_PAGE..FIRST_PAGE + PAGE_COUNT).collect();
    let mut body = Vec::new();
    for page in &page_ids {
        object(
            &mut body,
            *page,
            &format!(
                "<< /Type /Page /Parent {} 0 R /MediaBox [0 0 72 72] /Resources << >> >>",
                FIRST_GROUP + DEPTH - 1
            ),
        );
    }
    for level in 0..DEPTH {
        let number = FIRST_GROUP + level;
        let parent = if level == 0 { MISSING_ROOT } else { number - 1 };
        let kids = if level + 1 == DEPTH {
            page_ids
                .iter()
                .map(|page| format!("{page} 0 R "))
                .collect::<String>()
        } else {
            format!("{} 0 R ", number + 1)
        };
        object(
            &mut body,
            number,
            &format!("<< /Type /Pages /Parent {parent} 0 R /Count {PAGE_COUNT} /Kids [{kids}] >>"),
        );
    }

    let input = fragment_caj(&body, &page_ids);
    let (output, report) = convert(&input, ConversionOptions::default(), &Limits::default())
        .expect("convert a deep shared page-tree chain");
    assert_eq!(report.pages_converted, PAGE_COUNT);
    let expected_pages: Vec<PdfRef> = page_ids
        .into_iter()
        .map(|number| PdfRef {
            number,
            generation: 0,
        })
        .collect();
    assert_eq!(inspect(&output).pages(), expected_pages);
    let pdf = String::from_utf8_lossy(&output);
    assert!(
        pdf.contains(&format!(
            "{MISSING_ROOT} 0 obj\n<< /Type /Pages /Count {PAGE_COUNT} /Kids [{FIRST_GROUP} 0 R ]"
        )),
        "missing root should count every page but list the shared direct child once"
    );
}

#[test]
fn synthetic_root_never_satisfies_an_unrelated_missing_reference() {
    // The two omitted /Pages parents are 20 and 21. Object 22 is absent but
    // referenced as a page resource; an automatically assigned root must not
    // turn that broken resource into a seemingly valid reference.
    let mut body = Vec::new();
    object(
        &mut body,
        9,
        "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 72 72] /Resources << /XObject << /Im0 22 0 R >> >> >>",
    );
    object(
        &mut body,
        3,
        "<< /Type /Page /Parent 6 0 R /MediaBox [0 0 72 72] /Resources << >> >>",
    );
    object(
        &mut body,
        5,
        "<< /Type /Pages /Parent 20 0 R /Count 1 /Kids [9 0 R] >>",
    );
    object(
        &mut body,
        6,
        "<< /Type /Pages /Parent 21 0 R /Count 1 /Kids [3 0 R] >>",
    );
    let input = fragment_caj(&body, &[9, 3]);
    let error = rejected_without_output(&input, &Limits::default());
    assert!(
        matches!(&error, Error::Pdf { object: Some((9, 0)), reason, .. }
            if reason.contains("missing object")),
        "{error}"
    );
}

#[test]
fn synthetic_root_id_stays_clear_of_repairable_link_targets() {
    let mut body = Vec::new();
    object(
        &mut body,
        9,
        "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 72 72] /Resources << >> /Annots [12 0 R] >>",
    );
    object(
        &mut body,
        3,
        "<< /Type /Page /Parent 6 0 R /MediaBox [0 0 72 72] /Resources << >> >>",
    );
    object(
        &mut body,
        5,
        "<< /Type /Pages /Parent 20 0 R /Count 1 /Kids [9 0 R] >>",
    );
    object(
        &mut body,
        6,
        "<< /Type /Pages /Parent 21 0 R /Count 1 /Kids [3 0 R] >>",
    );
    object(
        &mut body,
        12,
        "<< /Type /Annot /Subtype /Link /Rect [0 0 20 20] /Border [0 0 0] /Dest [22 0 R /Fit] >>",
    );
    let input = fragment_caj(&body, &[9, 3]);
    let (output, report) = convert(&input, ConversionOptions::default(), &Limits::default())
        .expect("repair link without colliding with synthetic page-tree root");
    assert_eq!(report.pages_converted, 2);
    assert_eq!(inspect(&output).pages().len(), 2);
    let pdf = String::from_utf8_lossy(&output);
    assert!(pdf.contains("23 0 obj\n<< /Type /Pages /Count 2"), "{pdf}");
    assert!(!pdf.contains("/Dest [22 0 R"), "{pdf}");
    let file = TempPdf::write("root-link-collision", &output);
    checked_command(Command::new("qpdf").arg("--check").arg(&file.0), "qpdf");
}

#[test]
fn repairs_direct_and_indirect_broken_link_destinations_without_changing_render() {
    let baseline_input = fragment_caj(&one_page_body(None, None), &[9]);
    let (baseline_output, _) = convert(
        &baseline_input,
        ConversionOptions::default(),
        &Limits::default(),
    )
    .unwrap();
    let baseline_file = TempPdf::write("link-baseline", &baseline_output);
    let baseline_render = rendered_page(&baseline_file.0);
    assert!(!baseline_render.is_empty());

    for (label, destination, indirect) in [
        ("direct", "[99 0 R /Fit]", false),
        ("indirect", "13 0 R", true),
    ] {
        let mut body = one_page_body(
            Some(&format!(
                "<< /Type /Annot /Subtype /Link /Rect [0 0 20 20] /Border [0 0 0] /Dest {destination} >>"
            )),
            None,
        );
        if indirect {
            object(&mut body, 13, "[99 0 R /Fit]");
        }
        let input = fragment_caj(&body, &[9]);
        let (output, report) = convert(&input, ConversionOptions::default(), &Limits::default())
            .unwrap_or_else(|error| panic!("repair {label} link: {error}"));
        assert_eq!(report.pages_converted, 1);
        assert_eq!(inspect(&output).pages()[0].number, 9);
        assert!(!output.windows(5).any(|window| window == b"/Dest"));
        if indirect {
            assert!(
                output
                    .windows(b"13 0 obj\nnull\nendobj".len())
                    .any(|window| { window == b"13 0 obj\nnull\nendobj" }),
                "indirect destination array must become null"
            );
        }
        let file = TempPdf::write(label, &output);
        checked_command(Command::new("qpdf").arg("--check").arg(&file.0), "qpdf");
        assert_eq!(
            rendered_page(&file.0),
            baseline_render,
            "{label} render changed"
        );
    }
}

#[test]
fn unrelated_missing_resource_reference_fails_before_sink_output() {
    let mut body = Vec::new();
    object(
        &mut body,
        9,
        "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 72 72] /Resources << /XObject << /Im0 99 0 R >> >> >>",
    );
    object(&mut body, 5, "<< /Type /Pages /Count 1 /Kids [9 0 R] >>");
    let input = fragment_caj(&body, &[9]);
    let mut source = SeekableSource::new(Cursor::new(input)).unwrap();
    let mut output = Vec::new();
    let result = run_native(convert_caj(
        &mut source,
        &mut WriteSink::new(&mut output),
        ConversionOptions::default(),
        &Limits::default(),
        &NeverCancel,
    ));
    assert!(
        matches!(
            &result,
            Err(Error::Pdf {
                object: Some((9, 0)),
                ..
            })
        ),
        "{result:?}"
    );
    assert!(output.is_empty());
}

#[test]
fn repairs_nearby_stream_length_without_changing_page_render() {
    const CONTENT_LEN: usize = "0 0 0 rg 10 10 30 30 re f".len();
    let valid_input = fragment_caj(&one_page_body(None, None), &[9]);
    let near_input = fragment_caj(&one_page_body(None, Some(CONTENT_LEN - 1)), &[9]);
    let (valid_output, _) = convert(
        &valid_input,
        ConversionOptions::default(),
        &Limits::default(),
    )
    .unwrap();
    let (repaired_output, report) = convert(
        &near_input,
        ConversionOptions::default(),
        &Limits::default(),
    )
    .expect("repair near-stream-length mismatch");
    assert_eq!(report.pages_converted, 1);
    assert_eq!(inspect(&repaired_output).pages()[0].number, 9);
    assert!(
        repaired_output
            .windows(format!("/Length {CONTENT_LEN}").len())
            .any(|window| window == format!("/Length {CONTENT_LEN}").as_bytes())
    );
    let valid_file = TempPdf::write("stream-valid", &valid_output);
    let repaired_file = TempPdf::write("stream-repaired", &repaired_output);
    checked_command(
        Command::new("qpdf").arg("--check").arg(&repaired_file.0),
        "qpdf",
    );
    assert_eq!(
        rendered_page(&repaired_file.0),
        rendered_page(&valid_file.0)
    );
}

#[test]
fn rejects_invalid_page_tree_relationships_before_writing() {
    let cases: [(&str, &str, &[u32], &str); 4] = [
        (
            "cycle",
            "9 0 obj\n<< /Type /Page /Parent 5 0 R /MediaBox [0 0 72 72] >>\nendobj\n\
             5 0 obj\n<< /Type /Pages /Parent 6 0 R /Count 1 /Kids [9 0 R] >>\nendobj\n\
             6 0 obj\n<< /Type /Pages /Parent 5 0 R /Count 1 /Kids [5 0 R] >>\nendobj\n",
            &[9],
            "parent cycle",
        ),
        (
            "non-page parent",
            "9 0 obj\n<< /Type /Page /Parent 11 0 R /MediaBox [0 0 72 72] >>\nendobj\n\
             11 0 obj\n<< /Producer (not a Pages node) >>\nendobj\n",
            &[9],
            "parent is not a Pages object",
        ),
        (
            "two existing roots",
            "9 0 obj\n<< /Type /Page /Parent 5 0 R /MediaBox [0 0 72 72] >>\nendobj\n\
             3 0 obj\n<< /Type /Page /Parent 6 0 R /MediaBox [0 0 72 72] >>\nendobj\n\
             5 0 obj\n<< /Type /Pages /Count 1 /Kids [9 0 R] >>\nendobj\n\
             6 0 obj\n<< /Type /Pages /Count 1 /Kids [3 0 R] >>\nendobj\n",
            &[9, 3],
            "multiple existing roots",
        ),
        (
            "existing and missing roots",
            "9 0 obj\n<< /Type /Page /Parent 5 0 R /MediaBox [0 0 72 72] >>\nendobj\n\
             3 0 obj\n<< /Type /Page /Parent 7 0 R /MediaBox [0 0 72 72] >>\nendobj\n\
             5 0 obj\n<< /Type /Pages /Count 1 /Kids [9 0 R] >>\nendobj\n",
            &[9, 3],
            "mixed page-tree roots",
        ),
    ];
    for (label, body, pages, expected) in cases {
        let input = fragment_caj(body.as_bytes(), pages);
        let error = rejected_without_output(&input, &Limits::default());
        assert!(
            matches!(&error, Error::Caj { reason, .. } if reason.contains(expected)),
            "{label}: {error}"
        );
    }
}

#[test]
fn duplicate_page_tree_object_numbers_are_rejected_before_output() {
    let mut body = Vec::new();
    object(
        &mut body,
        9,
        "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 72 72] >>",
    );
    object(
        &mut body,
        9,
        "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 72 72] >>",
    );
    object(&mut body, 5, "<< /Type /Pages /Count 1 /Kids [9 0 R] >>");
    let error = rejected_without_output(&fragment_caj(&body, &[9]), &Limits::default());
    assert!(matches!(
        error,
        Error::Caj {
            reason: "duplicate PDF page-tree object",
            ..
        }
    ));
}

#[test]
fn rejects_unsafe_link_repairs_without_writing() {
    let cases: [(&str, &str, Option<&str>); 5] = [
        (
            "missing indirect destination object",
            "<< /Type /Annot /Subtype /Link /Rect [0 0 20 20] /Dest 99 0 R >>",
            None,
        ),
        (
            "destination and action together",
            "<< /Type /Annot /Subtype /Link /Rect [0 0 20 20] /Dest [99 0 R /Fit] /A << /S /URI /URI (https://example.invalid) >> >>",
            None,
        ),
        (
            "multiple dangling targets",
            "<< /Type /Annot /Subtype /Link /Rect [0 0 20 20] /Dest [99 0 R /Fit] /P 98 0 R >>",
            None,
        ),
        (
            "unreferenced destination array",
            "<< /Type /Annot /Subtype /Link /Rect [0 0 20 20] >>",
            Some("[99 0 R /Fit]"),
        ),
        (
            "non-link shares destination array",
            "<< /Type /Annot /Subtype /Link /Rect [0 0 20 20] /Dest 13 0 R >>",
            Some("[99 0 R /Fit]"),
        ),
    ];
    for (label, annotation, destination) in cases {
        let mut body = if label == "non-link shares destination array" {
            let mut body = Vec::new();
            object(
                &mut body,
                9,
                "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 72 72] /Resources << >> /Annots [12 0 R 14 0 R] >>",
            );
            object(&mut body, 5, "<< /Type /Pages /Count 1 /Kids [9 0 R] >>");
            object(&mut body, 12, annotation);
            body
        } else {
            one_page_body(Some(annotation), None)
        };
        if let Some(destination) = destination {
            object(&mut body, 13, destination);
        }
        if label == "non-link shares destination array" {
            object(
                &mut body,
                14,
                "<< /Type /Annot /Subtype /Text /Rect [0 0 20 20] /Dest 13 0 R >>",
            );
        }
        let input = fragment_caj(&body, &[9]);
        let error = rejected_without_output(&input, &Limits::default());
        assert!(
            matches!(
                &error,
                Error::Pdf {
                    kind: caj2pdf_core::PdfErrorKind::Malformed,
                    ..
                }
            ),
            "{label}: {error}"
        );
    }
}

#[test]
fn bounded_page_tree_repair_and_output_preflight_leave_sink_empty() {
    let mut body = Vec::new();
    let page_objects: Vec<u32> = (1000..1161).collect();
    for number in &page_objects {
        object(
            &mut body,
            *number,
            "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 72 72] /Resources << >> >>",
        );
    }
    let missing_parent = fragment_caj(&body, &page_objects);
    let error = rejected_without_output(
        &missing_parent,
        &Limits {
            io_chunk_bytes: 64,
            max_allocation_bytes: 4000,
            ..Limits::default()
        },
    );
    assert!(
        matches!(
            &error,
            Error::LimitExceeded {
                resource: "allocation bytes",
                ..
            }
        ),
        "{error}"
    );

    let valid = fragment_caj(&one_page_body(None, None), &[9]);
    let error = rejected_without_output(
        &valid,
        &Limits {
            max_output_bytes: 128,
            ..Limits::default()
        },
    );
    assert!(
        matches!(
            &error,
            Error::PdfLimitExceeded {
                resource: "output bytes",
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn rejects_a_ranged_source_that_reports_more_bytes_than_requested() {
    let mut source = OverreportingSource { size: 4096 };
    let mut output = Vec::new();
    let error = run_native(convert_caj(
        &mut source,
        &mut WriteSink::new(&mut output),
        ConversionOptions::default(),
        &Limits::default(),
        &NeverCancel,
    ))
    .expect_err("overreporting source must be rejected");
    assert!(
        matches!(&error, Error::InvalidInput { reason } if reason.contains("more bytes than requested")),
        "{error}"
    );
    assert!(output.is_empty());
}

#[test]
fn repairs_shared_indirect_destination_for_every_link() {
    let mut body = Vec::new();
    object(
        &mut body,
        9,
        "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 72 72] /Resources << >> /Annots [12 0 R 14 0 R] >>",
    );
    object(&mut body, 5, "<< /Type /Pages /Count 1 /Kids [9 0 R] >>");
    for number in [12, 14] {
        object(
            &mut body,
            number,
            "<< /Type /Annot /Subtype /Link /Rect [0 0 20 20] /Border [0 0 0] /Dest 13 0 R >>",
        );
    }
    object(&mut body, 13, "[99 0 R /Fit]");
    let input = fragment_caj(&body, &[9]);
    let (output, report) = convert(&input, ConversionOptions::default(), &Limits::default())
        .expect("all links sharing the omitted page destination should be repaired");
    assert_eq!(report.pages_converted, 1);
    assert_eq!(inspect(&output).pages()[0].number, 9);
    assert!(!output.windows(5).any(|window| window == b"/Dest"));
    let file = TempPdf::write("shared-dest", &output);
    checked_command(Command::new("qpdf").arg("--check").arg(&file.0), "qpdf");
}

#[test]
fn rejects_corrupt_caj_header_and_page_rows_with_locations() {
    let tiny = tiny_caj();
    let mut cases: Vec<CajFieldCase> = Vec::new();

    let mut zero_pages = tiny.bytes.clone();
    put_u32(&mut zero_pages, 0x10, 0);
    cases.push(("zero pages", zero_pages, 0x10, None, "page count"));

    let mut negative_toc_count = tiny.bytes.clone();
    put_u32(&mut negative_toc_count, 0x110, u32::MAX);
    cases.push((
        "negative TOC count",
        negative_toc_count,
        0x110,
        None,
        "TOC count",
    ));

    let mut overlapping_table = tiny.bytes.clone();
    put_u32(&mut overlapping_table, 0x14, 0x300);
    cases.push((
        "overlapping table",
        overlapping_table,
        0x14,
        None,
        "overlaps",
    ));

    let mut truncated_table = tiny.bytes.clone();
    put_u32(&mut truncated_table, 0x14, tiny.bytes.len() as u32 - 4);
    cases.push((
        "truncated table",
        truncated_table,
        0x14,
        None,
        "extends beyond",
    ));

    let mut zero_page_id = tiny.bytes.clone();
    put_u32(&mut zero_page_id, tiny.table_start + 8, 0);
    cases.push((
        "zero page object",
        zero_page_id,
        (tiny.table_start + 8) as u64,
        Some(1),
        "page object number",
    ));

    let mut disjoint_spans = tiny.bytes.clone();
    put_u32(
        &mut disjoint_spans,
        tiny.table_start + 12,
        tiny.bytes.len() as u32 - 1,
    );
    cases.push((
        "disjoint page spans",
        disjoint_spans,
        (tiny.table_start + 12) as u64,
        Some(2),
        "not contiguous",
    ));

    let mut overlapping_body = tiny.bytes.clone();
    put_u32(
        &mut overlapping_body,
        tiny.table_start,
        tiny.table_start as u32,
    );
    cases.push((
        "body overlaps table",
        overlapping_body,
        tiny.table_start as u64,
        Some(1),
        "overlaps the page table",
    ));

    let mut overflowing_span = tiny.bytes.clone();
    let first_body_length = u32::from_le_bytes(
        overflowing_span[tiny.table_start + 4..tiny.table_start + 8]
            .try_into()
            .unwrap(),
    );
    put_u32(
        &mut overflowing_span,
        tiny.table_start + 4,
        first_body_length + 1,
    );
    cases.push((
        "page span extends past source",
        overflowing_span,
        (tiny.table_start + 4) as u64,
        Some(1),
        "extends beyond source",
    ));

    let mut duplicate_page_id = tiny.bytes.clone();
    put_u32(&mut duplicate_page_id, tiny.table_start + 12 + 8, 9);
    cases.push((
        "duplicate page ID",
        duplicate_page_id,
        (tiny.table_start + 12 + 8) as u64,
        Some(2),
        "duplicate CAJ page object",
    ));

    for (label, input, offset, record, reason) in cases {
        assert_caj_error(label, &input, offset, record, reason);
    }

    let mut unsupported = tiny.bytes.clone();
    unsupported[0] = b'X';
    assert!(matches!(
        rejected_without_output(&unsupported, &Limits::default()),
        Error::UnsupportedFormat
    ));

    let empty_body = fragment_caj(&one_page_body(None, None), &[9]);
    let mut empty_body = empty_body;
    put_u32(&mut empty_body, 0x400 + 4, 0);
    assert_caj_error("empty PDF body", &empty_body, 0x400, None, "body is empty");

    assert_caj_error("short CAJ header", b"CAJ", 0, None, "extends beyond source");
}

#[test]
fn rejects_invalid_caj_outline_fields_with_record_locations() {
    let tiny = tiny_caj();
    let first = 0x114;
    let second = first + 308;
    let mut cases: Vec<(&str, Vec<u8>, u64, u32, &str)> = Vec::new();

    let mut empty_title = tiny.bytes.clone();
    empty_title[first] = 0;
    cases.push((
        "empty title",
        empty_title,
        first as u64,
        1,
        "empty CAJ TOC title",
    ));

    let mut invalid_encoding = tiny.bytes.clone();
    invalid_encoding[first] = 0xff;
    invalid_encoding[first + 1] = 0;
    cases.push((
        "invalid GB18030",
        invalid_encoding,
        first as u64,
        1,
        "GB18030",
    ));

    let mut malformed_page = tiny.bytes.clone();
    malformed_page[first + 280] = b'x';
    cases.push((
        "nondecimal TOC page",
        malformed_page,
        (first + 280) as u64,
        1,
        "ASCII decimal",
    ));

    let mut overflowing_page = tiny.bytes.clone();
    overflowing_page[first + 280..first + 292].copy_from_slice(b"42949672960\0");
    cases.push((
        "overflowing TOC page",
        overflowing_page,
        (first + 289) as u64,
        1,
        "overflows",
    ));

    let mut out_of_range_page = tiny.bytes.clone();
    out_of_range_page[first + 280] = b'4';
    cases.push((
        "out-of-range TOC page",
        out_of_range_page,
        (first + 280) as u64,
        1,
        "outside the document",
    ));

    let mut zero_level = tiny.bytes.clone();
    put_u32(&mut zero_level, first + 304, 0);
    cases.push((
        "zero outline level",
        zero_level,
        (first + 304) as u64,
        1,
        "level must be positive",
    ));

    let mut skipped_level = tiny.bytes.clone();
    put_u32(&mut skipped_level, second + 304, 3);
    cases.push((
        "skipped outline level",
        skipped_level,
        (second + 304) as u64,
        2,
        "skips a parent",
    ));

    for (label, input, offset, record, reason) in cases {
        assert_caj_error(label, &input, offset, Some(record), reason);
    }
}

#[test]
fn enforces_caj_metadata_limits_before_writing() {
    let tiny = tiny_caj();
    let cases = [
        (
            "page count",
            Limits {
                max_pages: 2,
                ..Limits::default()
            },
            "CAJ pages",
        ),
        (
            "bookmark count",
            Limits {
                max_bookmarks: 2,
                ..Limits::default()
            },
            "CAJ bookmarks",
        ),
        (
            "body bytes",
            Limits {
                max_input_bytes: 100,
                ..Limits::default()
            },
            "CAJ PDF input bytes",
        ),
        (
            "page metadata",
            Limits {
                io_chunk_bytes: 1,
                max_allocation_bytes: 64,
                ..Limits::default()
            },
            "CAJ page metadata",
        ),
        (
            "bookmark metadata",
            Limits {
                io_chunk_bytes: 1,
                max_allocation_bytes: 80,
                ..Limits::default()
            },
            "CAJ bookmarks",
        ),
    ];
    for (label, limits, expected_resource) in cases {
        let error = rejected_without_output(&tiny.bytes, &limits);
        assert!(
            matches!(&error, Error::CajLimitExceeded { resource, .. } if *resource == expected_resource),
            "{label}: {error}"
        );
    }

    let mut large_title = tiny.bytes.clone();
    large_title[0x114..0x114 + 200].fill(b'A');
    let limits = Limits {
        io_chunk_bytes: 1,
        max_allocation_bytes: 256,
        ..Limits::default()
    };
    let error = rejected_without_output(&large_title, &limits);
    assert!(
        matches!(
            &error,
            Error::CajLimitExceeded {
                resource: "CAJ title allocation bytes",
                record: Some(1),
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn retained_link_repair_budget_is_checked_before_output() {
    let pages: Vec<u32> = (1000..1032).collect();
    let mut body = Vec::new();
    for (index, page) in pages.iter().copied().enumerate() {
        let link = 2000 + index as u32;
        object(
            &mut body,
            page,
            &format!(
                "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 72 72] /Resources << >> /Annots [{link} 0 R] >>"
            ),
        );
        object(
            &mut body,
            link,
            "<< /Type /Annot /Subtype /Link /Rect [0 0 1 1] /Dest [9999 0 R /Fit] >>",
        );
    }
    let input = fragment_caj(&body, &pages);
    let error = rejected_without_output(
        &input,
        &Limits {
            io_chunk_bytes: 64,
            max_allocation_bytes: 5000,
            ..Limits::default()
        },
    );
    assert!(
        matches!(&error, Error::CajLimitExceeded { resource: "retained link repairs", limit: 5000, attempted, .. } if *attempted > 5000),
        "{error}"
    );

    let (output, report) = convert(&input, ConversionOptions::default(), &Limits::default())
        .expect("same fragment should convert when repair budget is sufficient");
    assert_eq!(report.pages_converted, pages.len() as u32);
    assert_eq!(inspect(&output).pages().len(), pages.len());
    assert!(!output.windows(5).any(|window| window == b"/Dest"));
    let file = TempPdf::write("many-link-repairs", &output);
    checked_command(Command::new("qpdf").arg("--check").arg(&file.0), "qpdf");
}

#[test]
fn missing_reference_index_budget_is_checked_before_output() {
    let mut body = Vec::new();
    object(
        &mut body,
        9,
        "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 1 1] >>",
    );
    for number in 100..180 {
        object(
            &mut body,
            number,
            "<< /A 9001 0 R /B 9002 0 R /C 9003 0 R >>",
        );
    }
    let input = fragment_caj(&body, &[9]);
    let error = rejected_without_output(
        &input,
        &Limits {
            io_chunk_bytes: 64,
            max_allocation_bytes: 4000,
            ..Limits::default()
        },
    );
    assert!(
        matches!(&error, Error::CajLimitExceeded { resource: "missing PDF references", limit: 4000, attempted, .. } if *attempted > 4000),
        "{error}"
    );
}

fn object_offset(input: &[u8], number: u32) -> u64 {
    let header = format!("{number} 0 obj\n");
    input
        .windows(header.len())
        .position(|window| window == header.as_bytes())
        .expect("object header present") as u64
}

#[test]
fn synthetic_root_number_cannot_overflow_past_the_highest_object() {
    // Two pages under different absent parents need a new joining root, but
    // the largest possible object number is already occupied.
    let mut body = Vec::new();
    object(
        &mut body,
        9,
        "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 72 72] >>",
    );
    object(
        &mut body,
        u32::MAX,
        "<< /Type /Page /Parent 6 0 R /MediaBox [0 0 72 72] >>",
    );
    let input = fragment_caj(&body, &[9, u32::MAX]);
    let body_start = 0x400 + 2 * 12;
    assert_caj_error(
        "root number overflow",
        &input,
        body_start,
        None,
        "CAJ synthetic page-tree object number overflows",
    );

    // The same layout converts when a joining root number is available.
    let mut body = Vec::new();
    object(
        &mut body,
        9,
        "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 72 72] >>",
    );
    object(
        &mut body,
        10,
        "<< /Type /Page /Parent 6 0 R /MediaBox [0 0 72 72] >>",
    );
    let (output, report) = convert(
        &fragment_caj(&body, &[9, 10]),
        ConversionOptions::default(),
        &Limits::default(),
    )
    .expect("two absent parents should be joined under a new root");
    assert_eq!(report.pages_converted, 2);
    let pages: Vec<u32> = inspect(&output)
        .pages()
        .iter()
        .map(|page| page.number)
        .collect();
    assert_eq!(pages, [9, 10]);
}

#[test]
fn joining_root_allocation_is_checked_after_every_group_node() {
    // Each page has its own absent parent. The joining root is generated
    // last, and its estimate covers every group node plus one slot per group.
    const GROUPS: u32 = 35;
    const LIMIT: u64 = 3000;
    let pages: Vec<u32> = (1000..1000 + GROUPS).collect();
    let root = 2000 + GROUPS;
    let mut body = Vec::new();
    let mut group_nodes = 0;
    for (index, page) in pages.iter().copied().enumerate() {
        let parent = 2000 + index as u32;
        object(
            &mut body,
            page,
            &format!("<< /Type /Page /Parent {parent} 0 R /MediaBox [0 0 72 72] >>"),
        );
        group_nodes += format!(
            "{parent} 0 obj\n<< /Type /Pages /Parent {root} 0 R /Count 1 /Kids [{page} 0 R ] >>\nendobj\n"
        )
        .len() as u64;
    }
    let input = fragment_caj(&body, &pages);
    let error = rejected_without_output(
        &input,
        &Limits {
            io_chunk_bytes: 64,
            max_allocation_bytes: LIMIT,
            ..Limits::default()
        },
    );
    let root_estimate = group_nodes + 24 * u64::from(GROUPS) + 160;
    assert!(
        matches!(
            &error,
            Error::LimitExceeded {
                resource: "allocation bytes",
                limit: LIMIT,
                attempted,
            } if *attempted == root_estimate
        ),
        "{error}"
    );

    let (output, report) = convert(&input, ConversionOptions::default(), &Limits::default())
        .expect("groups join under one root with the default allocation budget");
    assert_eq!(report.pages_converted, GROUPS);
    let order: Vec<u32> = inspect(&output)
        .pages()
        .iter()
        .map(|page| page.number)
        .collect();
    assert_eq!(order, pages);
}

#[test]
fn one_object_cannot_share_two_repaired_destination_arrays() {
    let mut body = one_page_body(
        Some("<< /Type /Annot /Subtype /Link /Rect [0 0 20 20] /Dest 13 0 R >>"),
        None,
    );
    object(&mut body, 13, "[99 0 R /Fit]");
    object(&mut body, 15, "[98 0 R /Fit]");
    object(&mut body, 20, "<< /First 13 0 R /Second 15 0 R >>");
    let input = fragment_caj(&body, &[9]);
    let error = rejected_without_output(&input, &Limits::default());
    assert!(
        matches!(
            &error,
            Error::Pdf {
                object: Some((20, 0)),
                reason: "indirect reference targets a missing object",
                offset,
                ..
            } if *offset == object_offset(&input, 20)
        ),
        "{error}"
    );
}

#[test]
fn link_repair_budget_includes_every_link_sharing_a_destination() {
    // One destination array is retained first; each link that shares it is
    // retained afterwards, so the budget is exhausted on a link annotation.
    let mut body = Vec::new();
    object(
        &mut body,
        9,
        "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 72 72] /Resources << >> >>",
    );
    object(&mut body, 5, "<< /Type /Pages /Count 1 /Kids [9 0 R] >>");
    object(&mut body, 13, "[99 0 R /Fit]");
    let links: Vec<u32> = (20..40).collect();
    for number in &links {
        object(
            &mut body,
            *number,
            "<< /Type /Annot /Subtype /Link /Rect [0 0 20 20] /Dest 13 0 R >>",
        );
    }
    let input = fragment_caj(&body, &[9]);
    let link_offsets: Vec<u64> = links
        .iter()
        .map(|number| object_offset(&input, *number))
        .collect();
    let error = rejected_without_output(
        &input,
        &Limits {
            io_chunk_bytes: 64,
            max_allocation_bytes: 3500,
            ..Limits::default()
        },
    );
    assert!(
        matches!(
            &error,
            Error::CajLimitExceeded {
                resource: "retained link repairs",
                limit: 3500,
                attempted,
                offset,
                record: None,
            } if *attempted > 3500 && link_offsets.contains(offset)
        ),
        "{error}"
    );

    let (output, report) = convert(&input, ConversionOptions::default(), &Limits::default())
        .expect("links sharing an omitted destination convert with enough budget");
    assert_eq!(report.pages_converted, 1);
    assert!(!output.windows(5).any(|window| window == b"/Dest"));
}
