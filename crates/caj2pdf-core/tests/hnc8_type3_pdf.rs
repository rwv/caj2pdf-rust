// SPDX-License-Identifier: MIT

//! Original synthetic HN/C8 type-3 records and caller-supplied invented MQ
//! states. No normative table, private document, or oracle pixel is embedded.

mod common;

use caj2pdf_core::{
    Cancellation, Error, Limits, NeverCancel, RangedSource, SequentialSink,
    hnc8::{
        Type3ImageSelection, Type3PdfError, Type3PdfErrorKind, Type3PdfOptions, Type3RefinedStore,
        Type3SelectedPdfReport, Type3Stage, Type3Store, Type3Workspaces, Variant,
        convert_type3_image_pdf,
    },
    jbig2::{
        mq::{MQ_STATE_COUNT, MqState, MqTable},
        text::{TextHeaderAnomaly, TextHeaderPolicy},
        text_composer::RandomAccessScratch,
    },
    pdf::{BilevelImageSpec, PageSpec, PdfDocument},
};
use common::CancelAfter;
use std::{
    cell::RefCell,
    fs,
    future::Future,
    io,
    path::PathBuf,
    pin::pin,
    process::Command,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
    task::{Context, Poll, Waker},
};

fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("test adapters complete immediately"),
    }
}

#[derive(Clone, Copy, Debug)]
enum Layout {
    C8,
    HnA,
    HnB,
}

impl Layout {
    fn variant(self) -> Variant {
        match self {
            Self::C8 => Variant::C8,
            Self::HnA => Variant::HnA,
            Self::HnB => Variant::HnB,
        }
    }
}

struct Record {
    kind: i32,
    payload: Vec<u8>,
}

struct Built {
    bytes: Vec<u8>,
    descriptors: Vec<Vec<u64>>,
    payloads: Vec<Vec<u64>>,
}

/// The three header/page-index layouts measured in docs/hnc8-container.md.
fn container(layout: Layout, pages: &[Vec<Record>]) -> Built {
    let (count_at, index_at) = match layout {
        Layout::C8 => (0x08, 0x50),
        Layout::HnA => (0x90, 0x15c + 308),
        Layout::HnB => (0x90, 0xd8),
    };
    let mut bytes = vec![0_u8; index_at + 20 * pages.len()];
    match layout {
        Layout::C8 => bytes[..4].copy_from_slice(&[0xc8, 0, 0, 0]),
        Layout::HnA | Layout::HnB => {
            bytes[..4].copy_from_slice(b"HN\0\0");
            bytes[4..8].copy_from_slice(if matches!(layout, Layout::HnA) {
                &[0x90, 1, 0, 0]
            } else {
                &[0xc8, 0, 0, 0]
            });
        }
    }
    bytes[count_at..count_at + 4].copy_from_slice(&(pages.len() as i32).to_le_bytes());
    if matches!(layout, Layout::HnA) {
        bytes[0x158..0x15c].copy_from_slice(&1_i32.to_le_bytes());
        bytes[0x15c..0x15c + 308].fill(0xa5);
    }
    let mut descriptors = Vec::new();
    let mut payloads = Vec::new();
    for (number, records) in pages.iter().enumerate() {
        let text = bytes.len();
        bytes.extend_from_slice(b"tx");
        let row = index_at + 20 * number;
        bytes[row..row + 4].copy_from_slice(&(text as i32).to_le_bytes());
        bytes[row + 4..row + 8].copy_from_slice(&2_i32.to_le_bytes());
        bytes[row + 8..row + 10].copy_from_slice(&(records.len() as i16).to_le_bytes());
        let mut page_descriptors = Vec::new();
        let mut page_payloads = Vec::new();
        for record in records {
            let descriptor = bytes.len();
            let payload = descriptor + 16;
            bytes.extend_from_slice(&record.kind.to_le_bytes());
            bytes.extend_from_slice(&(payload as i32).to_le_bytes());
            bytes.extend_from_slice(&(record.payload.len() as i32).to_le_bytes());
            bytes.extend_from_slice(&[0xee; 4]);
            bytes.extend_from_slice(&record.payload);
            page_descriptors.push(descriptor as u64);
            page_payloads.push(payload as u64);
        }
        descriptors.push(page_descriptors);
        payloads.push(page_payloads);
    }
    Built {
        bytes,
        descriptors,
        payloads,
    }
}

#[path = "common/type3_fixture.rs"]
mod type3_fixture;
use type3_fixture::*;

fn type3(width: u32, height: u32, text_flags: u16) -> Record {
    Record {
        kind: 3,
        payload: payload(width, height, text_flags),
    }
}

fn invented_table() -> MqTable {
    let states = vec![
        MqState {
            qe: 1,
            next_mps: 0,
            next_lps: 0,
            switch_mps: false,
        };
        MQ_STATE_COUNT
    ];
    MqTable::new(states, &Limits::default()).unwrap()
}

struct Source {
    bytes: Vec<u8>,
    max_read: usize,
    max_request: usize,
    calls: usize,
    advertised_size: Option<u64>,
    stop_at: Option<u64>,
    overreport_at: Option<u64>,
    fail_at: Option<u64>,
    fail_on_offset_visit: Option<(u64, usize)>,
    failed_offset_visits: usize,
    mutate_at_call: Option<(usize, usize)>,
    mutate_at_offset_visit: Option<(u64, usize, usize)>,
    offset_visits: usize,
}

impl Source {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            max_read: usize::MAX,
            max_request: 0,
            calls: 0,
            advertised_size: None,
            stop_at: None,
            overreport_at: None,
            fail_at: None,
            fail_on_offset_visit: None,
            failed_offset_visits: 0,
            mutate_at_call: None,
            mutate_at_offset_visit: None,
            offset_visits: 0,
        }
    }
}

impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.advertised_size.unwrap_or(self.bytes.len() as u64)
    }

    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        self.calls += 1;
        self.max_request = self.max_request.max(destination.len());
        if let Some((call, index)) = self.mutate_at_call {
            if self.calls == call {
                self.bytes[index] ^= 1;
            }
        }
        if let Some((watched, visit, index)) = self.mutate_at_offset_visit {
            if offset == watched {
                self.offset_visits += 1;
                if self.offset_visits == visit {
                    self.bytes[index] ^= 1;
                }
            }
        }
        if self.fail_at == Some(offset) {
            return Err(Error::Io(io::Error::other("injected source failure")));
        }
        if let Some((watched, visit)) = self.fail_on_offset_visit {
            if offset == watched {
                self.failed_offset_visits += 1;
                if self.failed_offset_visits == visit {
                    return Err(Error::Io(io::Error::other(
                        "injected staged source failure",
                    )));
                }
            }
        }
        if self.overreport_at == Some(offset) {
            return Ok(destination.len() + 1);
        }
        if self.stop_at.is_some_and(|end| offset >= end) {
            return Ok(0);
        }
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let count = self
            .bytes
            .len()
            .saturating_sub(start)
            .min(destination.len())
            .min(self.max_read);
        destination[..count].copy_from_slice(&self.bytes[start..start + count]);
        Ok(count)
    }
}

#[derive(Default)]
struct Sink {
    bytes: Vec<u8>,
    writes: usize,
    fail_at: Option<usize>,
    max_write: usize,
}

impl SequentialSink for Sink {
    async fn write(&mut self, bytes: &[u8]) -> caj2pdf_core::Result<usize> {
        self.writes += 1;
        if self.fail_at == Some(self.writes) {
            return Err(Error::Io(io::Error::other("injected sink failure")));
        }
        let count = if self.max_write == 0 {
            bytes.len()
        } else {
            bytes.len().min(self.max_write)
        };
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }

    async fn flush(&mut self) -> caj2pdf_core::Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct Scratch {
    bytes: Vec<u8>,
    fail_read_after: Option<usize>,
    read_calls: usize,
}

impl RandomAccessScratch for Scratch {
    fn size(&self) -> caj2pdf_core::Result<u64> {
        Ok(self.bytes.len() as u64)
    }

    async fn set_len(&mut self, length: u64) -> caj2pdf_core::Result<()> {
        self.bytes.resize(length as usize, 0);
        Ok(())
    }

    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        self.read_calls += 1;
        if self
            .fail_read_after
            .is_some_and(|count| self.read_calls > count)
        {
            return Err(Error::Io(io::Error::other("injected scratch read failure")));
        }
        let start = offset as usize;
        let count = destination
            .len()
            .min(self.bytes.len().saturating_sub(start));
        destination[..count].copy_from_slice(&self.bytes[start..start + count]);
        Ok(count)
    }

    async fn write_at(&mut self, offset: u64, bytes: &[u8]) -> caj2pdf_core::Result<usize> {
        let start = offset as usize;
        self.bytes[start..start + bytes.len()].copy_from_slice(bytes);
        Ok(bytes.len())
    }

    async fn flush(&mut self) -> caj2pdf_core::Result<()> {
        Ok(())
    }
}

/// A reader sees every byte appended through its paired writer.
#[derive(Clone)]
struct SharedStore(Rc<RefCell<Vec<u8>>>);

impl SharedStore {
    fn handles() -> (Self, Self, Self) {
        let shared = Rc::new(RefCell::new(Vec::new()));
        (
            Self(Rc::clone(&shared)),
            Self(Rc::clone(&shared)),
            Self(shared),
        )
    }
}

impl RangedSource for SharedStore {
    fn size(&self) -> u64 {
        self.0.borrow().len() as u64
    }

    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        let source = self.0.borrow();
        let start = offset as usize;
        let count = destination.len().min(source.len().saturating_sub(start));
        destination[..count].copy_from_slice(&source[start..start + count]);
        Ok(count)
    }
}

impl SequentialSink for SharedStore {
    async fn write(&mut self, bytes: &[u8]) -> caj2pdf_core::Result<usize> {
        self.0.borrow_mut().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    async fn flush(&mut self) -> caj2pdf_core::Result<()> {
        Ok(())
    }
}

fn selection(page_number: u32, image_number: u32) -> Type3ImageSelection {
    Type3ImageSelection {
        page_number,
        image_number,
    }
}

fn options() -> Type3PdfOptions {
    Type3PdfOptions {
        pixels_per_inch: 72.0,
        ..Type3PdfOptions::default()
    }
}

#[derive(Clone, Copy)]
enum WorkspaceFault {
    None,
    DirtyFirst,
    DirtySecond,
    DirtyRefined,
    DirtyText,
    ScratchRead,
}

fn run<C: Cancellation>(
    source: &mut Source,
    sink: &mut Sink,
    selected: Type3ImageSelection,
    options: Type3PdfOptions,
    limits: &Limits,
    cancellation: &C,
) -> Result<Type3SelectedPdfReport, Type3PdfError> {
    run_with_workspace_fault(
        source,
        sink,
        selected,
        options,
        limits,
        cancellation,
        WorkspaceFault::None,
    )
}

fn run_with_workspace_fault<C: Cancellation>(
    source: &mut Source,
    sink: &mut Sink,
    selected: Type3ImageSelection,
    options: Type3PdfOptions,
    limits: &Limits,
    cancellation: &C,
    fault: WorkspaceFault,
) -> Result<Type3SelectedPdfReport, Type3PdfError> {
    let table = invented_table();
    let (mut first_reader, mut first_compose_reader, mut first_writer) = SharedStore::handles();
    let (mut second_reader, mut second_compose_reader, mut second_writer) = SharedStore::handles();
    let (mut refined_reader, _refined_compose_reader, mut refined_writer) = SharedStore::handles();
    let mut scratch = Scratch::default();
    match fault {
        WorkspaceFault::None => {}
        WorkspaceFault::DirtyFirst => first_writer.0.borrow_mut().push(1),
        WorkspaceFault::DirtySecond => second_writer.0.borrow_mut().push(1),
        WorkspaceFault::DirtyRefined => refined_writer.0.borrow_mut().push(1),
        WorkspaceFault::DirtyText => scratch.bytes.push(1),
        WorkspaceFault::ScratchRead => scratch.fail_read_after = Some(2),
    }
    let mut workspaces = Type3Workspaces {
        first: Type3Store {
            reader: &mut first_reader,
            compose_reader: &mut first_compose_reader,
            writer: &mut first_writer,
        },
        second: Type3Store {
            reader: &mut second_reader,
            compose_reader: &mut second_compose_reader,
            writer: &mut second_writer,
        },
        refined: Type3RefinedStore {
            reader: &mut refined_reader,
            writer: &mut refined_writer,
        },
        text: &mut scratch,
    };
    ready(convert_type3_image_pdf(
        source,
        sink,
        &table,
        &mut workspaces,
        selected,
        options,
        limits,
        cancellation,
    ))
}

fn find(bytes: &[u8], needle: &[u8]) -> Option<usize> {
    bytes
        .windows(needle.len())
        .position(|window| window == needle)
}

fn embedded_image(pdf: &[u8]) -> &[u8] {
    let image = find(pdf, b"/Subtype /Image").unwrap();
    let start = image + find(&pdf[image..], b"stream\n").unwrap() + 7;
    let end = start + find(&pdf[start..], b"\nendstream").unwrap();
    &pdf[start..end]
}

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("caj2pdf-type3-test-{}-{id}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn tool(command: &mut Command) {
    let output = command
        .output()
        .expect("independent PDF test tool installed");
    assert!(
        output.status.success(),
        "tool failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.len() <= 1024 * 1024);
    assert!(output.stderr.len() <= 1024 * 1024);
}

fn validate_one_image_pdf(pdf_bytes: &[u8], width: u32, height: u32) {
    assert!(find(pdf_bytes, b"/Subtype /Image").is_some());
    assert!(find(pdf_bytes, b"/DeviceGray").is_some());
    assert!(find(pdf_bytes, b"/BitsPerComponent 1").is_some());
    assert!(find(pdf_bytes, b"/Decode [1 0]").is_some());
    let temp = TempDir::new();
    let pdf = temp.file("one.pdf");
    fs::write(&pdf, pdf_bytes).unwrap();
    tool(Command::new("qpdf").arg("--check").arg(&pdf));
    let info = Command::new("pdfinfo").arg(&pdf).output().unwrap();
    assert!(info.status.success());
    let text = String::from_utf8(info.stdout).unwrap();
    assert!(text.contains("Pages:           1"), "{text}");
    let prefix = temp.file("extract");
    tool(Command::new("pdfimages").arg(&pdf).arg(&prefix));
    let pbm = fs::read(temp.file("extract-000.pbm")).unwrap();
    let header = format!("P4\n{width} {height}\n");
    assert!(pbm.starts_with(header.as_bytes()));
    let expected = width.div_ceil(8) as usize * height as usize;
    assert_eq!(pbm.len() - header.len(), expected);
}

fn pnm_payload<'a>(bytes: &'a [u8], magic: &str, width: u32, height: u32) -> &'a [u8] {
    let mut at = 0;
    let mut next = || {
        loop {
            while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
                at += 1;
            }
            if bytes.get(at) == Some(&b'#') {
                while bytes.get(at).is_some_and(|byte| *byte != b'\n') {
                    at += 1;
                }
            } else {
                break;
            }
        }
        let start = at;
        while bytes
            .get(at)
            .is_some_and(|byte| !byte.is_ascii_whitespace())
        {
            at += 1;
        }
        std::str::from_utf8(&bytes[start..at]).unwrap().to_owned()
    };
    assert_eq!(next(), magic);
    assert_eq!(next(), width.to_string());
    assert_eq!(next(), height.to_string());
    if magic == "P5" {
        assert_eq!(next(), "255");
    }
    assert_eq!(bytes[at], b'\n');
    &bytes[at + 1..]
}

#[test]
fn selected_first_middle_and_last_hn_c8_images_produce_one_checked_page() {
    let limits = Limits::default();
    for layout in [Layout::C8, Layout::HnA, Layout::HnB] {
        let built = container(
            layout,
            &[
                vec![type3(3, 2, 0x10)],
                vec![
                    Record {
                        kind: 0,
                        payload: vec![0; 48],
                    },
                    type3(9, 3, 0x10),
                    type3(8, 2, 0x10),
                ],
                vec![type3(7, 4, 0x10)],
            ],
        );
        for (page, image, width, height) in [(1, 1, 3, 2), (2, 2, 9, 3), (2, 3, 8, 2), (3, 1, 7, 4)]
        {
            let mut source = Source::new(built.bytes.clone());
            source.max_read = 3;
            let mut sink = Sink {
                max_write: 5,
                ..Sink::default()
            };
            let report = run(
                &mut source,
                &mut sink,
                selection(page, image),
                options(),
                &limits,
                &NeverCancel,
            )
            .unwrap_or_else(|error| panic!("{layout:?} {page}/{image}: {error}"));
            assert_eq!(report.image.page_number, page);
            assert_eq!(report.image.image_number, image);
            assert_eq!(report.image.record_type, 3);
            assert_eq!(
                report.image.descriptor_offset,
                built.descriptors[(page - 1) as usize][(image - 1) as usize]
            );
            assert_eq!(
                report.image.payload.offset,
                built.payloads[(page - 1) as usize][(image - 1) as usize]
            );
            assert_eq!(report.source_variant, layout.variant());
            assert_eq!(report.source_pages, 3);
            assert_eq!((report.page.width, report.page.height), (width, height));
            assert_eq!(report.text_header_anomaly, None);
            assert_eq!(report.conversion.pages_converted, 1);
            assert_eq!(
                report.conversion.output_bytes_written,
                sink.bytes.len() as u64
            );
            assert!(report.conversion.input_bytes_read > 0);
            assert!(source.max_request <= 65_536);
            validate_one_image_pdf(&sink.bytes, width, height);
        }
    }
}

#[test]
fn selected_nonblank_asymmetric_pixels_keep_top_left_black_with_two_renderers() {
    let built = container(Layout::HnB, &[vec![type3(9, 3, 0x10)]]);
    let mut source = Source::new(built.bytes);
    let mut sink = Sink::default();
    let report = run(
        &mut source,
        &mut sink,
        selection(1, 1),
        options(),
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap();
    assert_eq!((report.page.width, report.page.height), (9, 3));
    const PACKED: [u8; 6] = [0x80, 0, 0, 0, 0, 0];
    assert_eq!(embedded_image(&sink.bytes), PACKED);

    let temp = TempDir::new();
    let pdf = temp.file("selected.pdf");
    fs::write(&pdf, &sink.bytes).unwrap();
    tool(Command::new("qpdf").arg("--check").arg(&pdf));
    let extracted = temp.file("extracted");
    tool(Command::new("pdfimages").arg(&pdf).arg(&extracted));
    let pbm = fs::read(temp.file("extracted-000.pbm")).unwrap();
    assert_eq!(pnm_payload(&pbm, "P4", 9, 3), PACKED);

    let poppler = temp.file("poppler");
    tool(
        Command::new("pdftoppm")
            .arg("-mono")
            .arg("-r")
            .arg("72")
            .arg("-singlefile")
            .arg(&pdf)
            .arg(&poppler),
    );
    let pbm = fs::read(temp.file("poppler.pbm")).unwrap();
    let poppler = pnm_payload(&pbm, "P4", 9, 3);
    assert_eq!(poppler.len(), PACKED.len());
    // Poppler's page raster can expand a 1x1 pixel into an adjacent pixel.
    // Require the black mark to remain at the top left and every rendered
    // black pixel to lie in its clipped 3x3 neighborhood, with no color slack.
    assert_ne!(poppler[0] & 0x80, 0);
    for (y, row) in poppler.chunks_exact(2).enumerate() {
        for x in 0..9 {
            if row[x / 8] & (0x80 >> (x % 8)) != 0 {
                assert!(x <= 1 && y <= 1, "Poppler moved a black pixel to ({x},{y})");
            }
        }
    }

    let mupdf = temp.file("mupdf.pgm");
    tool(
        Command::new("mutool")
            .arg("draw")
            .arg("-q")
            .arg("-F")
            .arg("pgm")
            .arg("-r")
            .arg("72")
            .arg("-o")
            .arg(&mupdf)
            .arg(&pdf),
    );
    let pgm = fs::read(mupdf).unwrap();
    let pixels = pnm_payload(&pgm, "P5", 9, 3);
    assert_eq!(pixels.len(), 27);
    assert_eq!(pixels[0], 0);
    assert!(pixels[1..].iter().all(|value| *value == 255));
}

#[test]
fn selected_page_probe_skips_earlier_unreadable_page() {
    let mut built = container(
        Layout::HnB,
        &[vec![type3(3, 2, 0x10)], vec![type3(9, 2, 0x10)]],
    );
    built.bytes[0xd8..0xdc].copy_from_slice(&i32::MAX.to_le_bytes());
    let mut source = Source::new(built.bytes);
    let mut sink = Sink::default();
    let report = run(
        &mut source,
        &mut sink,
        selection(2, 1),
        options(),
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap();
    assert_eq!(report.image.page_number, 2);
    validate_one_image_pdf(&sink.bytes, 9, 2);
}

#[test]
fn malformed_dib_palette_segments_and_page_geometry_are_rejected_before_pdf() {
    let built = container(Layout::C8, &[vec![type3(9, 2, 0x10)]]);
    let base = built.payloads[0][0] as usize;
    for relative in [
        0,           // DIB header length
        40,          // white palette entry
        48 + 4,      // page-information segment type
        48 + 11 + 3, // page-information width differs from DIB
    ] {
        let mut damaged = built.bytes.clone();
        damaged[base + relative] ^= 1;
        let mut source = Source::new(damaged);
        let mut sink = Sink::default();
        let error = run(
            &mut source,
            &mut sink,
            selection(1, 1),
            options(),
            &Limits::default(),
            &NeverCancel,
        )
        .unwrap_err();
        assert!(
            sink.bytes.is_empty(),
            "failed preflight emitted PDF bytes at +{relative}"
        );
        assert_eq!((error.page, error.image), (Some(1), Some(1)));
        assert!(error.offset.is_some());
    }
}

#[test]
fn selected_span_mutation_between_checked_passes_is_refused() {
    let built = container(Layout::HnA, &[vec![type3(9, 3, 0x10)]]);
    let payload = built.payloads[0][0];
    for visit in [2, 3, 4] {
        let mut source = Source::new(built.bytes.clone());
        source.mutate_at_offset_visit = Some((payload, visit, payload as usize + 24));
        let mut sink = Sink::default();
        let error = run(
            &mut source,
            &mut sink,
            selection(1, 1),
            options(),
            &Limits::default(),
            &NeverCancel,
        )
        .unwrap_err();
        assert_eq!((error.page, error.image), (Some(1), Some(1)));
        assert!(
            matches!(error.kind, Type3PdfErrorKind::SourceChanged),
            "visit {visit}: {error:?}"
        );
        // A post-decode failure may already have written an image stream.
        // The caller must discard this sink and never expose it as a PDF.
    }
}

#[test]
fn located_stage_errors_cover_each_checked_metadata_boundary() {
    let built = container(Layout::C8, &[vec![type3(9, 2, 0x10)]]);
    let base = built.payloads[0][0] as usize;
    // Five original segments are 30, 25, 26, 42, and 36 bytes long.
    let segment = [48, 78, 103, 129, 171];
    for (name, relative, value, expected) in [
        (
            "page flags",
            segment[0] + 11 + 16,
            0xff,
            Type3Stage::PageInfo,
        ),
        (
            "text flags",
            segment[3] + 12 + 17,
            0xff,
            Type3Stage::TextHeader,
        ),
        (
            "generic flags",
            segment[4] + 11 + 17,
            5,
            Type3Stage::GenericHeader,
        ),
        ("text x", segment[3] + 12 + 11, 1, Type3Stage::Profile),
        (
            "unexpected segment number",
            segment[4] + 3,
            200,
            Type3Stage::Profile,
        ),
        (
            "first dictionary mode",
            segment[1] + 11,
            0xff,
            Type3Stage::FirstDictionary,
        ),
        (
            "second dictionary AT",
            segment[2] + 12 + 2,
            3,
            Type3Stage::SecondDictionary,
        ),
    ] {
        let mut bytes = built.bytes.clone();
        bytes[base + relative] = value;
        let mut source = Source::new(bytes);
        let mut sink = Sink::default();
        let error = run(
            &mut source,
            &mut sink,
            selection(1, 1),
            options(),
            &Limits::default(),
            &NeverCancel,
        )
        .unwrap_err();
        assert!(sink.bytes.is_empty(), "{name} unexpectedly wrote PDF bytes");
        assert_eq!((error.page, error.image), (Some(1), Some(1)), "{name}");
        let Type3PdfErrorKind::Stage { stage, .. } = error.kind else {
            panic!("{name}: {error:?}");
        };
        assert_eq!(stage, expected, "{name}");
    }

    let mut record = type3(9, 2, 0x10);
    record.payload.truncate(segment[4]); // omit the entire fifth segment
    let built = container(Layout::C8, &[vec![record]]);
    let mut source = Source::new(built.bytes);
    let mut sink = Sink::default();
    let error = run(
        &mut source,
        &mut sink,
        selection(1, 1),
        options(),
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert!(sink.bytes.is_empty());
    assert!(matches!(
        error.kind,
        Type3PdfErrorKind::Stage {
            stage: Type3Stage::Profile,
            ..
        }
    ));
}

#[test]
fn dirty_workspaces_and_scratch_read_failure_are_typed() {
    let built = container(Layout::HnB, &[vec![type3(9, 2, 0x10)]]);
    for fault in [
        WorkspaceFault::DirtyFirst,
        WorkspaceFault::DirtySecond,
        WorkspaceFault::DirtyRefined,
        WorkspaceFault::DirtyText,
    ] {
        let mut source = Source::new(built.bytes.clone());
        let mut sink = Sink::default();
        let error = run_with_workspace_fault(
            &mut source,
            &mut sink,
            selection(1, 1),
            options(),
            &Limits::default(),
            &NeverCancel,
            fault,
        )
        .unwrap_err();
        assert!(matches!(error.kind, Type3PdfErrorKind::Workspace(_)));
        assert!(sink.bytes.is_empty());
    }

    let mut source = Source::new(built.bytes);
    let mut sink = Sink::default();
    let error = run_with_workspace_fault(
        &mut source,
        &mut sink,
        selection(1, 1),
        options(),
        &Limits::default(),
        &NeverCancel,
        WorkspaceFault::ScratchRead,
    )
    .unwrap_err();
    assert!(
        matches!(
            error.kind,
            Type3PdfErrorKind::Stage {
                stage: Type3Stage::PageCompose,
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn pdf_scale_and_page_budget_are_checked_at_selected_geometry() {
    let built = container(Layout::C8, &[vec![type3(9, 2, 0x10)]]);
    for ppi in [f64::NAN, f64::INFINITY, -1.0, 1e-10, 1e10] {
        let mut source = Source::new(built.bytes.clone());
        let mut sink = Sink::default();
        let error = run(
            &mut source,
            &mut sink,
            selection(1, 1),
            Type3PdfOptions {
                pixels_per_inch: ppi,
                ..options()
            },
            &Limits::default(),
            &NeverCancel,
        )
        .unwrap_err();
        assert!(matches!(error.kind, Type3PdfErrorKind::InvalidOptions(_)));
        assert!(sink.bytes.is_empty());
    }
    let mut source = Source::new(built.bytes.clone());
    let mut sink = Sink::default();
    let page_budget_options = Type3PdfOptions {
        page_compose: caj2pdf_core::jbig2::page_compose::PageComposeBudget {
            max_packed_bytes: 0,
            ..Default::default()
        },
        ..options()
    };
    let error = run(
        &mut source,
        &mut sink,
        selection(1, 1),
        page_budget_options,
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert!(matches!(
        error.kind,
        Type3PdfErrorKind::Stage {
            stage: Type3Stage::PageCompose,
            ..
        }
    ));

    let mut source = Source::new(built.bytes);
    let mut sink = Sink::default();
    let context_budget_options = Type3PdfOptions {
        mq: caj2pdf_core::jbig2::mq::MqBudget {
            max_contexts: 1024,
            ..Default::default()
        },
        ..options()
    };
    let error = run(
        &mut source,
        &mut sink,
        selection(1, 1),
        context_budget_options,
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert!(
        matches!(
            error.kind,
            Type3PdfErrorKind::Stage {
                stage: Type3Stage::Contexts,
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn second_dictionary_context_growth_and_text_scratch_budget_are_checked() {
    let built = container(Layout::HnB, &[vec![type3(9, 2, 0x10)]]);
    let mut bytes = built.bytes.clone();
    let second_data = built.payloads[0][0] as usize + 103 + 12;
    bytes[second_data + 8..second_data + 12].copy_from_slice(&2_u32.to_be_bytes());
    let mut source = Source::new(bytes);
    let mut sink = Sink::default();
    let context_options = Type3PdfOptions {
        mq: caj2pdf_core::jbig2::mq::MqBudget {
            max_contexts: caj2pdf_core::jbig2::integer::INTEGER_CONTEXT_COUNT + 1024,
            ..Default::default()
        },
        ..options()
    };
    let error = run(
        &mut source,
        &mut sink,
        selection(1, 1),
        context_options,
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert!(
        matches!(
            error.kind,
            Type3PdfErrorKind::Stage {
                stage: Type3Stage::Contexts,
                ..
            }
        ),
        "{error:?}"
    );
    assert!(sink.bytes.is_empty());

    let mut source = Source::new(built.bytes);
    let mut sink = Sink::default();
    let compose_options = Type3PdfOptions {
        text_compose: caj2pdf_core::jbig2::text_composer::TextComposeBudget {
            max_scratch_bytes: 0,
            ..Default::default()
        },
        ..options()
    };
    let error = run(
        &mut source,
        &mut sink,
        selection(1, 1),
        compose_options,
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert!(
        matches!(
            error.kind,
            Type3PdfErrorKind::Stage {
                stage: Type3Stage::TextCompose,
                ..
            }
        ),
        "{error:?}"
    );
    assert!(sink.bytes.is_empty());
}

#[test]
fn row_width_boundaries_keep_zero_low_padding_in_selected_pdf() {
    let limits = Limits::default();
    for width in [7, 8, 9, 31, 32, 33] {
        let built = container(Layout::C8, &[vec![type3(width, 3, 0x10)]]);
        let mut source = Source::new(built.bytes);
        let mut sink = Sink::default();
        let report = run(
            &mut source,
            &mut sink,
            selection(1, 1),
            options(),
            &limits,
            &NeverCancel,
        )
        .unwrap_or_else(|error| panic!("width {width}: {error}"));
        let image = embedded_image(&sink.bytes);
        let stride = width.div_ceil(8) as usize;
        assert_eq!(image.len(), stride * 3);
        let remainder = width % 8;
        if remainder != 0 {
            let low_mask = (1_u8 << (8 - remainder)) - 1;
            for row in image.chunks_exact(stride) {
                assert_eq!(row[stride - 1] & low_mask, 0);
            }
        }
        assert_eq!(report.page.width, width);
        validate_one_image_pdf(&sink.bytes, width, 3);
    }
}

#[test]
fn selected_identity_type_and_dib_errors_are_located() {
    let limits = Limits::default();
    let built = container(
        Layout::HnA,
        &[vec![
            Record {
                kind: 2,
                payload: vec![0; 48],
            },
            type3(9, 2, 0x10),
        ]],
    );
    for selected in [
        selection(0, 1),
        selection(1, 0),
        selection(1, 3),
        selection(2, 1),
    ] {
        let mut source = Source::new(built.bytes.clone());
        let mut sink = Sink::default();
        let error = run(
            &mut source,
            &mut sink,
            selected,
            options(),
            &limits,
            &NeverCancel,
        )
        .unwrap_err();
        assert!(sink.bytes.is_empty());
        assert_eq!(
            (error.page, error.image),
            (Some(selected.page_number), Some(selected.image_number))
        );
        assert!(error.offset.is_some());
        if selected.page_number == 0 || selected.image_number == 0 {
            assert_eq!(error.offset, Some(0));
        }
        assert!(matches!(
            error.kind,
            Type3PdfErrorKind::InvalidSelection(_) | Type3PdfErrorKind::Container(_)
        ));
    }
    let mut source = Source::new(built.bytes.clone());
    let mut sink = Sink::default();
    let error = run(
        &mut source,
        &mut sink,
        selection(1, 1),
        options(),
        &limits,
        &NeverCancel,
    )
    .unwrap_err();
    assert_eq!(
        (error.page, error.image, error.offset),
        (Some(1), Some(1), Some(built.descriptors[0][0]))
    );
    assert!(matches!(
        error.kind,
        Type3PdfErrorKind::UnsupportedImageType(2)
    ));

    let mut corrupt = built.bytes.clone();
    corrupt[built.payloads[0][1] as usize + 14] = 8; // 8 bpp, not observed 1 bpp
    let mut source = Source::new(corrupt);
    let mut sink = Sink::default();
    let error = run(
        &mut source,
        &mut sink,
        selection(1, 2),
        options(),
        &limits,
        &NeverCancel,
    )
    .unwrap_err();
    assert!(sink.bytes.is_empty());
    assert_eq!((error.page, error.image), (Some(1), Some(2)));
    assert!(matches!(
        error.kind,
        Type3PdfErrorKind::DibMalformed(_) | Type3PdfErrorKind::Stage { .. }
    ));
}

#[test]
fn empty_coded_span_and_nonpositive_dib_dimensions_are_refused() {
    let built = container(
        Layout::C8,
        &[vec![Record {
            kind: 3,
            payload: dib(9, 2).to_vec(),
        }]],
    );
    let mut source = Source::new(built.bytes);
    let mut sink = Sink::default();
    let error = run(
        &mut source,
        &mut sink,
        selection(1, 1),
        options(),
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert!(matches!(error.kind, Type3PdfErrorKind::DibMalformed(_)));
    assert!(sink.bytes.is_empty());

    let built = container(Layout::C8, &[vec![type3(9, 2, 0x10)]]);
    for dimension in [4, 8] {
        let mut bytes = built.bytes.clone();
        bytes[built.payloads[0][0] as usize + dimension
            ..built.payloads[0][0] as usize + dimension + 4]
            .copy_from_slice(&0_i32.to_le_bytes());
        let mut source = Source::new(bytes);
        let mut sink = Sink::default();
        let error = run(
            &mut source,
            &mut sink,
            selection(1, 1),
            options(),
            &Limits::default(),
            &NeverCancel,
        )
        .unwrap_err();
        assert!(matches!(error.kind, Type3PdfErrorKind::DibMalformed(_)));
        assert!(sink.bytes.is_empty());
    }
}

#[test]
fn per_image_directory_and_page_limits_refuse_before_pdf() {
    let built = container(Layout::HnB, &[vec![type3(9, 2, 0x10)]]);
    for (options, expected) in [
        (
            Type3PdfOptions {
                container: caj2pdf_core::hnc8::Budget {
                    max_image_span_bytes: 100,
                    ..Default::default()
                },
                ..options()
            },
            None,
        ),
        (
            Type3PdfOptions {
                directory: caj2pdf_core::jbig2::DirectoryLimits {
                    max_segments: 4,
                    ..Default::default()
                },
                ..options()
            },
            Some(Type3Stage::Directory),
        ),
        (
            Type3PdfOptions {
                page: caj2pdf_core::jbig2::page_info::PageInfoBudget {
                    max_width: 8,
                    ..Default::default()
                },
                ..options()
            },
            Some(Type3Stage::PageInfo),
        ),
    ] {
        let mut source = Source::new(built.bytes.clone());
        let mut sink = Sink::default();
        let error = run(
            &mut source,
            &mut sink,
            selection(1, 1),
            options,
            &Limits::default(),
            &NeverCancel,
        )
        .unwrap_err();
        assert!(sink.bytes.is_empty());
        match expected {
            None => assert!(matches!(error.kind, Type3PdfErrorKind::Container(_))),
            Some(stage) => assert!(
                matches!(error.kind, Type3PdfErrorKind::Stage { stage: actual, .. } if actual == stage),
                "{error:?}"
            ),
        }
    }
}

#[test]
fn generic_marker_and_pdf_sink_faults_propagate_without_success() {
    let built = container(Layout::C8, &[vec![type3(9, 3, 0x10)]]);
    let mut damaged = built.bytes.clone();
    let generic_tail = damaged.len() - 1;
    damaged[generic_tail] = 0xab; // not the MQ terminal marker 0xac
    let mut source = Source::new(damaged);
    let mut sink = Sink::default();
    let error = run(
        &mut source,
        &mut sink,
        selection(1, 1),
        options(),
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert!(
        matches!(
            error.kind,
            Type3PdfErrorKind::Stage {
                stage: Type3Stage::GenericRegion,
                ..
            }
        ),
        "{error:?}"
    );
    assert!(!sink.bytes.is_empty()); // caller discards this partial stream

    let mut source = Source::new(built.bytes.clone());
    let mut sink = Sink::default();
    run(
        &mut source,
        &mut sink,
        selection(1, 1),
        options(),
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap();
    let writes = sink.writes;
    assert!(writes > 10);
    let mut saw_pdf = false;
    let mut saw_page_compose = false;
    for fail_at in 1..=writes {
        let mut source = Source::new(built.bytes.clone());
        let mut sink = Sink {
            fail_at: Some(fail_at),
            ..Sink::default()
        };
        let error = run(
            &mut source,
            &mut sink,
            selection(1, 1),
            options(),
            &Limits::default(),
            &NeverCancel,
        )
        .unwrap_err();
        assert_eq!((error.page, error.image), (Some(1), Some(1)));
        match error.kind {
            Type3PdfErrorKind::Pdf(_) => saw_pdf = true,
            Type3PdfErrorKind::Stage {
                stage: Type3Stage::PageCompose,
                ..
            } => saw_page_compose = true,
            other => panic!("sink failure at write {fail_at}: {other:?}"),
        }
    }
    assert!(saw_pdf && saw_page_compose);
}

#[test]
fn located_decoder_input_failures_cover_dictionary_text_and_generic_stages() {
    let built = container(Layout::HnB, &[vec![type3(9, 3, 0x10)]]);
    let base = built.payloads[0][0];
    for (name, relative, stage) in [
        (
            "first dictionary header",
            78 + 11,
            Type3Stage::FirstDictionary,
        ),
        (
            "second dictionary count header",
            103 + 12,
            Type3Stage::SecondDictionary,
        ),
        (
            "generic coded bytes",
            171 + 11 + 20,
            Type3Stage::GenericRegion,
        ),
    ] {
        let mut source = Source::new(built.bytes.clone());
        source.fail_at = Some(base + relative);
        let mut sink = Sink::default();
        let error = run(
            &mut source,
            &mut sink,
            selection(1, 1),
            options(),
            &Limits::default(),
            &NeverCancel,
        )
        .unwrap_err();
        assert_eq!((error.page, error.image), (Some(1), Some(1)), "{name}");
        assert!(
            matches!(error.kind, Type3PdfErrorKind::Stage { stage: actual, .. } if actual == stage),
            "{name}: {error:?}"
        );
    }

    for (name, relative, stage) in [
        (
            "first dictionary body",
            78 + 11 + 12,
            Type3Stage::FirstDictionary,
        ),
        (
            "second dictionary body",
            103 + 12 + 12,
            Type3Stage::SecondDictionary,
        ),
        (
            "text body terminal",
            129 + 12 + 23 + 5,
            Type3Stage::TextCompose,
        ),
    ] {
        let mut damaged = built.bytes.clone();
        let at = base as usize + relative as usize;
        damaged[at..at + 2].fill(0);
        let mut source = Source::new(damaged);
        let mut sink = Sink::default();
        let error = match run(
            &mut source,
            &mut sink,
            selection(1, 1),
            options(),
            &Limits::default(),
            &NeverCancel,
        ) {
            Ok(report) => panic!("{name} unexpectedly converted: {report:?}"),
            Err(error) => error,
        };
        assert!(
            matches!(error.kind, Type3PdfErrorKind::Stage { stage: actual, .. } if actual == stage),
            "{name}: {error:?}"
        );
    }

    let mut source = Source::new(built.bytes);
    source.fail_on_offset_visit = Some((base + 129 + 12, 2));
    let mut sink = Sink::default();
    let error = run(
        &mut source,
        &mut sink,
        selection(1, 1),
        options(),
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert!(
        matches!(
            error.kind,
            Type3PdfErrorKind::Stage {
                stage: Type3Stage::TextInstances,
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn strict_text_header_refuses_anomaly_but_named_opt_in_records_it() {
    let built = container(Layout::C8, &[vec![type3(9, 2, 0xa40c)]]);
    let mut source = Source::new(built.bytes.clone());
    let mut sink = Sink::default();
    let strict = run(
        &mut source,
        &mut sink,
        selection(1, 1),
        options(),
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert!(sink.bytes.is_empty());
    assert_eq!((strict.page, strict.image), (Some(1), Some(1)));
    assert!(matches!(strict.kind, Type3PdfErrorKind::Stage { .. }));

    let mut source = Source::new(built.bytes);
    let mut sink = Sink::default();
    let report = run(
        &mut source,
        &mut sink,
        selection(1, 1),
        Type3PdfOptions {
            text_header_policy: TextHeaderPolicy::HnC8UnusedRefinementTemplate,
            ..options()
        },
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap();
    assert_eq!(
        report.text_header_anomaly,
        Some(TextHeaderAnomaly::UnusedRefinementTemplate)
    );
    validate_one_image_pdf(&sink.bytes, 9, 2);
}

#[test]
fn invalid_options_limits_cancellation_and_io_faults_never_report_success() {
    let built = container(Layout::HnB, &[vec![type3(9, 2, 0x10)]]);
    let selected = selection(1, 1);
    let mut source = Source::new(built.bytes.clone());
    let mut sink = Sink::default();
    let error = run(
        &mut source,
        &mut sink,
        selected,
        Type3PdfOptions {
            pixels_per_inch: 0.0,
            ..options()
        },
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert!(sink.bytes.is_empty());
    assert!(matches!(error.kind, Type3PdfErrorKind::InvalidOptions(_)));

    let mut source = Source::new(built.bytes.clone());
    let mut sink = Sink::default();
    let limits = Limits {
        max_input_bytes: built.bytes.len() as u64 - 1,
        ..Limits::default()
    };
    assert!(
        run(
            &mut source,
            &mut sink,
            selected,
            options(),
            &limits,
            &NeverCancel
        )
        .is_err()
    );
    assert!(sink.bytes.is_empty());

    let mut source = Source::new(built.bytes.clone());
    let mut sink = Sink::default();
    let error = run(
        &mut source,
        &mut sink,
        selected,
        options(),
        &Limits::default(),
        &CancelAfter::new(0),
    )
    .unwrap_err();
    assert_eq!((error.page, error.image), (Some(1), Some(1)));
    assert!(sink.bytes.is_empty());

    for kind in 0..4 {
        let mut source = Source::new(built.bytes.clone());
        match kind {
            0 => source.stop_at = Some(0),
            1 => source.overreport_at = Some(0),
            2 => source.fail_at = Some(0),
            _ => source.stop_at = Some(built.payloads[0][0]),
        }
        let mut sink = Sink::default();
        assert!(
            run(
                &mut source,
                &mut sink,
                selected,
                options(),
                &Limits::default(),
                &NeverCancel
            )
            .is_err()
        );
        assert!(sink.bytes.is_empty());
    }

    let mut source = Source::new(built.bytes);
    let mut sink = Sink {
        fail_at: Some(1),
        ..Sink::default()
    };
    let error = run(
        &mut source,
        &mut sink,
        selected,
        options(),
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert_eq!((error.page, error.image), (Some(1), Some(1)));
    assert!(matches!(error.kind, Type3PdfErrorKind::Pdf(_)));
}

#[test]
fn bilevel_pdf_rows_are_top_down_black_one_and_drop_low_padding() {
    let expected = [0x81, 0x80, 0x42, 0x00, 0x24, 0x80];
    let mut output = Sink::default();
    let limits = Limits::default();
    let report = ready(async {
        let mut document = PdfDocument::new(&mut output, &limits, &NeverCancel).await?;
        let mut image = document
            .begin_bilevel_image(BilevelImageSpec {
                pixel_width: 9,
                pixel_height: 3,
                row_stride: 2,
            })
            .await?;
        image.write(&expected).await?;
        let image = image.finish().await?;
        document
            .add_page(
                PageSpec {
                    width_points: 9.0,
                    height_points: 3.0,
                },
                &[image],
            )
            .await?;
        document.finish().await
    })
    .unwrap();
    assert_eq!(report.pages_converted, 1);
    assert_eq!(embedded_image(&output.bytes), expected);
    assert!(find(&output.bytes, b"/DeviceGray").is_some());
    assert!(find(&output.bytes, b"/BitsPerComponent 1").is_some());
    assert!(find(&output.bytes, b"/Decode [1 0]").is_some());

    // The PDF must preserve the source's asymmetric top/right/bottom marks.
    let temp = TempDir::new();
    let pdf = temp.file("one.pdf");
    fs::write(&pdf, &output.bytes).unwrap();
    tool(Command::new("qpdf").arg("--check").arg(&pdf));
    let prefix = temp.file("extract");
    tool(
        Command::new("pdfimages")
            .arg("-f")
            .arg("1")
            .arg("-l")
            .arg("1")
            .arg(&pdf)
            .arg(&prefix),
    );
    let pbm = fs::read(temp.file("extract-000.pbm")).unwrap();
    assert!(pbm.starts_with(b"P4\n9 3\n"));
    assert_eq!(&pbm[b"P4\n9 3\n".len()..], expected);
}
