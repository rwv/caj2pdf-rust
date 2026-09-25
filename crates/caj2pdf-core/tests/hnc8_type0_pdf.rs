// SPDX-License-Identifier: MIT

//! Synthetic HN/C8 type-0 containers converted to PDF.
//!
//! The coded images are produced at test runtime by an original, test-only
//! arithmetic encoder written from the T.82 interval description in
//! `docs/t82-arithmetic-core.md` and the observed row rule in
//! `docs/jbig1-type0-rows.md`. The probability table is invented; it is not
//! the T.82 Table 24. Nothing here is corpus data or a decoder oracle.

mod common;

use caj2pdf_core::{
    Error, Limits, MAX_BUDGET_COUNT, NeverCancel, RangedSource, SequentialSink,
    hnc8::{
        ErrorKind, MultipleImages, Type0PdfError, Type0PdfErrorKind, Type0PdfOptions,
        Type0PdfReport, convert_type0_pdf,
    },
    jbig1::Type0ErrorKind,
    pdf::{BilevelImageSpec, PageSpec, PdfDocument},
    qm::{QM_STATE_COUNT, QmState, QmTable},
};
use common::CancelAfter;
use std::{
    error::Error as _,
    fs::{read, remove_file, write},
    future::Future,
    io,
    path::{Path, PathBuf},
    pin::pin,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
    task::{Context, Poll, Waker},
};

fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("test adapters complete immediately"),
    }
}

// ---------------------------------------------------------------------------
// Invented probability table and a test-only arithmetic encoder.

/// Invented adaptive states: varied Qe values, forward MPS steps, backward
/// LPS steps, and occasional MPS switches. Not a standard table.
fn invented_states() -> Vec<QmState> {
    (0..QM_STATE_COUNT)
        .map(|i| QmState {
            qe: (0x0400 + (i * 0x0137) % 0x3c00) as u16,
            next_lps: (i / 3) as u8,
            next_mps: ((i + 1) % QM_STATE_COUNT) as u8,
            switch_mps: i % 7 == 0,
        })
        .collect()
}

fn table() -> QmTable {
    QmTable::new(invented_states()).unwrap()
}

/// Encodes decisions so that the decoder's lower subinterval `[0, A - Qe)`
/// carries the MPS unless `A - Qe < Qe` (conditional exchange). The code
/// value is kept exactly as a bit vector; its final lower bound is emitted,
/// and the decoder supplies zero bytes after the end.
struct Encoder {
    states: Vec<QmState>,
    contexts: Vec<(u8, bool)>,
    low: Vec<u8>,
    interval: u32,
    shift: usize,
}

impl Encoder {
    fn new() -> Self {
        Self {
            states: invented_states(),
            contexts: vec![(0, false); 1024],
            low: Vec::new(),
            interval: 0x10000,
            shift: 0,
        }
    }

    fn add_to_low(&mut self, value: u32) {
        // Bit j of `value` has weight 2^(j - 16 - shift): index 15 + shift - j.
        if self.low.len() < 16 + self.shift {
            self.low.resize(16 + self.shift, 0);
        }
        for j in 0..16 {
            if value & (1 << j) == 0 {
                continue;
            }
            let mut index = 15 + self.shift - j;
            loop {
                self.low[index] += 1;
                if self.low[index] < 2 {
                    break;
                }
                self.low[index] = 0;
                index -= 1;
            }
        }
    }

    fn encode(&mut self, context: usize, bit: bool) {
        let (index, mps) = self.contexts[context];
        let state = self.states[usize::from(index)];
        let qe = u32::from(state.qe);
        let narrowed = self.interval - qe;
        let lower_is_mps = narrowed >= qe;
        if (bit == mps) == lower_is_mps {
            self.interval = narrowed;
        } else {
            self.add_to_low(narrowed);
            self.interval = qe;
        }
        if bit != mps {
            self.contexts[context] = (state.next_lps, mps ^ state.switch_mps);
        } else if self.interval < 0x8000 {
            self.contexts[context] = (state.next_mps, mps);
        }
        while self.interval < 0x8000 {
            self.interval <<= 1;
            self.shift += 1;
        }
    }

    fn finish(self) -> Vec<u8> {
        let mut bytes: Vec<u8> = self
            .low
            .chunks(8)
            .map(|bits| {
                bits.iter()
                    .enumerate()
                    .fold(0, |byte, (i, bit)| byte | (bit << (7 - i)))
            })
            .collect();
        while bytes.last() == Some(&0) {
            bytes.pop();
        }
        if bytes.is_empty() {
            bytes.push(0);
        }
        bytes
    }
}

type Pixels = Vec<Vec<bool>>;

fn at(rows: &Pixels, y: isize, x: isize) -> usize {
    if y < 0 || x < 0 {
        return 0;
    }
    rows.get(y as usize)
        .and_then(|row| row.get(x as usize))
        .map_or(0, |&bit| usize::from(bit))
}

/// Code rows with the documented type-0 model: a row-control decision at
/// context 457 (one copies the preceding row, blank above row 0), else each
/// pixel with ten neighbors in the documented order.
fn encode_rows(rows: &Pixels) -> Vec<u8> {
    let width = rows[0].len();
    let mut encoder = Encoder::new();
    for y in 0..rows.len() {
        let previous = if y == 0 {
            vec![false; width]
        } else {
            rows[y - 1].clone()
        };
        let copy = rows[y] == previous;
        encoder.encode(457, copy);
        if copy {
            continue;
        }
        let yi = y as isize;
        for x in 0..width {
            let xi = x as isize;
            let mut context = 0;
            for (dy, dx) in [
                (0, -2),
                (0, -1),
                (-1, -2),
                (-1, -1),
                (-1, 0),
                (-1, 1),
                (-1, 2),
                (-2, -1),
                (-2, 0),
                (-2, 1),
            ] {
                context = (context << 1) | at(rows, yi + dy, xi + dx);
            }
            encoder.encode(context, rows[y][x]);
        }
    }
    encoder.finish()
}

/// A deterministic, nonuniform pattern with copied rows and edge pixels.
fn pattern(width: usize, height: usize, seed: usize) -> Pixels {
    let mut rows: Pixels = Vec::new();
    for y in 0..height {
        if y % 4 == 2 {
            let copy = rows[y - 1].clone();
            rows.push(copy);
            continue;
        }
        rows.push(
            (0..width)
                .map(|x| (x * 7 + y * 13 + seed * 5 + (x * y) % 3) % 5 < 2 || x + 1 == width)
                .collect(),
        );
    }
    rows
}

/// Packed PDF rows: `ceil(width / 8)` bytes, MSB first, 1 = black.
fn packed(rows: &Pixels) -> Vec<u8> {
    let mut bytes = Vec::new();
    for row in rows {
        for chunk in row.chunks(8) {
            bytes.push(
                chunk
                    .iter()
                    .enumerate()
                    .fold(0, |byte, (i, &bit)| byte | (u8::from(bit) << (7 - i))),
            );
        }
    }
    bytes
}

fn dib(width: i32, height: i32) -> Vec<u8> {
    let mut bytes = vec![0; 48];
    bytes[0..4].copy_from_slice(&40_u32.to_le_bytes());
    bytes[4..8].copy_from_slice(&width.to_le_bytes());
    bytes[8..12].copy_from_slice(&height.to_le_bytes());
    bytes[12..14].copy_from_slice(&1_u16.to_le_bytes());
    bytes[14..16].copy_from_slice(&1_u16.to_le_bytes());
    bytes[32..36].copy_from_slice(&2_u32.to_le_bytes());
    bytes[40..43].fill(0xff);
    bytes
}

fn type0_payload(rows: &Pixels) -> Vec<u8> {
    let mut bytes = dib(rows[0].len() as i32, rows.len() as i32);
    bytes.extend(encode_rows(rows));
    bytes
}

// ---------------------------------------------------------------------------
// Synthetic containers built from docs/hnc8-container.md.

#[derive(Clone, Copy, Debug, PartialEq)]
enum Layout {
    C8,
    HnA,
    HnB,
}

const LAYOUTS: [Layout; 3] = [Layout::C8, Layout::HnA, Layout::HnB];

struct Record {
    kind: i32,
    payload: Vec<u8>,
}

fn type0(rows: &Pixels) -> Record {
    Record {
        kind: 0,
        payload: type0_payload(rows),
    }
}

/// Descriptor offsets and payload offsets of every image, per page.
struct Built {
    bytes: Vec<u8>,
    descriptors: Vec<Vec<u64>>,
    payloads: Vec<Vec<u64>>,
}

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
            let marker: [u8; 4] = if layout == Layout::HnA {
                [0x90, 1, 0, 0]
            } else {
                [0xc8, 0, 0, 0]
            };
            bytes[4..8].copy_from_slice(&marker);
        }
    }
    bytes[count_at..count_at + 4].copy_from_slice(&(pages.len() as i32).to_le_bytes());
    if layout == Layout::HnA {
        // One opaque outline-like record precedes the page index.
        bytes[0x158..0x15c].copy_from_slice(&1_i32.to_le_bytes());
        bytes[0x15c..0x15c + 308].fill(0xa5);
    }
    let mut descriptors = Vec::new();
    let mut payloads = Vec::new();
    for (number, records) in pages.iter().enumerate() {
        // Two opaque text bytes, then chained descriptors, each followed by
        // a four-byte gap and its payload.
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

// ---------------------------------------------------------------------------
// I/O doubles.

struct Source {
    bytes: Vec<u8>,
    max_read: usize,
    largest_request: usize,
    /// After a read starting at `.0`, set byte `.1` to `.2`.
    rewrite: Option<(u64, usize, u8)>,
}

impl Source {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            max_read: usize::MAX,
            largest_request: 0,
            rewrite: None,
        }
    }
}

impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }

    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        self.largest_request = self.largest_request.max(destination.len());
        let start = (offset as usize).min(self.bytes.len());
        let count = (self.bytes.len() - start)
            .min(destination.len())
            .min(self.max_read);
        destination[..count].copy_from_slice(&self.bytes[start..start + count]);
        if let Some((trigger, index, value)) = self.rewrite {
            if trigger == offset {
                self.bytes[index] = value;
            }
        }
        Ok(count)
    }
}

#[derive(Default)]
struct Sink {
    bytes: Vec<u8>,
    writes: usize,
    fail_at: Option<usize>,
}

impl SequentialSink for Sink {
    async fn write(&mut self, bytes: &[u8]) -> caj2pdf_core::Result<usize> {
        self.writes += 1;
        if self.fail_at == Some(self.writes) {
            return Err(Error::Io(io::Error::other("injected sink failure")));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    async fn flush(&mut self) -> caj2pdf_core::Result<()> {
        Ok(())
    }
}

fn options() -> Type0PdfOptions {
    Type0PdfOptions {
        pixels_per_inch: 72.0,
        ..Type0PdfOptions::default()
    }
}

fn convert_with(
    source: &mut Source,
    sink: &mut Sink,
    options: Type0PdfOptions,
    limits: &Limits,
) -> Result<Type0PdfReport, Type0PdfError> {
    ready(convert_type0_pdf(
        source,
        sink,
        &table(),
        options,
        limits,
        &NeverCancel,
    ))
}

fn convert(
    bytes: Vec<u8>,
    options: Type0PdfOptions,
) -> Result<(Type0PdfReport, Vec<u8>), Type0PdfError> {
    let mut source = Source::new(bytes);
    let mut sink = Sink::default();
    let report = convert_with(&mut source, &mut sink, options, &Limits::default())?;
    assert_eq!(
        report.conversion.output_bytes_written,
        sink.bytes.len() as u64
    );
    Ok((report, sink.bytes))
}

fn convert_error(bytes: Vec<u8>, options: Type0PdfOptions) -> Type0PdfError {
    convert(bytes, options).map(|_| ()).unwrap_err()
}

// ---------------------------------------------------------------------------
// PDF inspection helpers.

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|position| position + from)
}

/// The exact payloads of every bilevel image stream, in output order.
fn image_streams(pdf: &[u8]) -> Vec<(u32, u32, Vec<u8>)> {
    let mut images = Vec::new();
    let mut from = 0;
    while let Some(start) = find(pdf, b"/Subtype /Image\n/Width ", from) {
        let text = String::from_utf8_lossy(&pdf[start..start + 120]).into_owned();
        let number = |key: &str| -> u32 {
            let rest = &text[text.find(key).unwrap() + key.len()..];
            rest[..rest.find('\n').unwrap()].parse().unwrap()
        };
        let (width, height) = (number("/Width "), number("/Height "));
        assert!(
            text.contains(
                "/ColorSpace /DeviceGray\n/BitsPerComponent 1\n/Decode [1 0]\n>>\nstream\n"
            )
        );
        let data = find(pdf, b">>\nstream\n", start).unwrap() + b">>\nstream\n".len();
        let length = width.div_ceil(8) as usize * height as usize;
        assert_eq!(&pdf[data + length..data + length + 11], b"\nendstream\n");
        images.push((width, height, pdf[data..data + length].to_vec()));
        from = data + length;
    }
    images
}

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);

impl Temp {
    fn new(extension: &str) -> Self {
        Self(std::env::temp_dir().join(format!(
            "caj2pdf-hnc8-type0-{}-{}.{extension}",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        )))
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = remove_file(&self.0);
    }
}

fn tool(command: &mut Command, name: &str) -> Output {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("{name} is required for PDF render tests: {error}"));
    assert!(
        output.status.success(),
        "{name} failed ({}):\n{}{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// Parse a binary PBM (P4) into width, height, and packed rows.
fn pbm(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut fields = Vec::new();
    let mut position = 0;
    while fields.len() < 3 {
        while bytes[position].is_ascii_whitespace() {
            position += 1;
        }
        if bytes[position] == b'#' {
            while bytes[position] != b'\n' {
                position += 1;
            }
            continue;
        }
        let start = position;
        while !bytes[position].is_ascii_whitespace() {
            position += 1;
        }
        fields.push(String::from_utf8(bytes[start..position].to_vec()).unwrap());
    }
    assert_eq!(fields[0], "P4");
    (
        fields[1].parse().unwrap(),
        fields[2].parse().unwrap(),
        bytes[position + 1..].to_vec(),
    )
}

/// Downsample a PBM by `factor`, keeping the centre pixel of each block.
fn block_centres((width, height, data): (u32, u32, Vec<u8>), factor: u32) -> (u32, u32, Vec<u8>) {
    let stride = width.div_ceil(8) as usize;
    let rows: Pixels = (0..height / factor)
        .map(|y| {
            let row = (y * factor + factor / 2) as usize * stride;
            (0..width / factor)
                .map(|x| {
                    let x = (x * factor + factor / 2) as usize;
                    data[row + x / 8] & (0x80 >> (x % 8)) != 0
                })
                .collect()
        })
        .collect();
    (width / factor, height / factor, packed(&rows))
}

/// Check the PDF with qpdf, then render each page with Poppler and MuPDF and
/// compare the black pixels with the expected packed rows.
fn check_renders(pdf: &[u8], expected: &[&Pixels]) {
    let file = Temp::new("pdf");
    write(&file.0, pdf).unwrap();
    tool(
        Command::new("qpdf").arg("--check").arg(&file.0),
        "qpdf --check",
    );
    let info = tool(Command::new("pdfinfo").arg(&file.0), "pdfinfo");
    let info = String::from_utf8(info.stdout).unwrap();
    let pages = expected.len().to_string();
    assert!(
        info.split_whitespace()
            .collect::<Vec<_>>()
            .windows(2)
            .any(|pair| pair == ["Pages:", pages.as_str()]),
        "{info}"
    );
    for (index, rows) in expected.iter().enumerate() {
        let page = (index + 1).to_string();
        let want = packed(rows);
        // Poppler smooths a 1:1 image blit, so render it at ten device
        // pixels per image pixel and sample each block's centre instead.
        let poppler = Temp::new("poppler");
        tool(
            Command::new("pdftoppm")
                .args([
                    "-mono",
                    "-r",
                    "720",
                    "-singlefile",
                    "-f",
                    &page,
                    "-l",
                    &page,
                ])
                .arg(&file.0)
                .arg(&poppler.0),
            "pdftoppm -mono",
        );
        let poppler_pbm = Path::new(&poppler.0).with_extension("poppler.pbm");
        let rendered = read(&poppler_pbm).unwrap();
        let _ = remove_file(&poppler_pbm);
        assert_eq!(
            block_centres(pbm(&rendered), 10),
            (rows[0].len() as u32, rows.len() as u32, want.clone()),
            "Poppler page {page}"
        );
        let mupdf = Temp::new("pbm");
        tool(
            Command::new("mutool")
                .args(["draw", "-q", "-r", "72", "-o"])
                .arg(&mupdf.0)
                .arg(&file.0)
                .arg(&page),
            "mutool draw",
        );
        assert_eq!(
            pbm(&read(&mupdf.0).unwrap()),
            (rows[0].len() as u32, rows.len() as u32, want),
            "MuPDF page {page}"
        );
    }
}

// ---------------------------------------------------------------------------
// Successful conversions.

#[test]
fn boundary_widths_have_exact_packed_rows_in_every_layout() {
    for layout in LAYOUTS {
        for (seed, width) in [7, 8, 9, 31, 32, 33].into_iter().enumerate() {
            for height in [1, 6] {
                let rows = pattern(width, height, seed);
                let built = container(layout, &[vec![type0(&rows)]]);
                let (report, pdf) = convert(built.bytes.clone(), options()).unwrap();
                assert_eq!(report.source_pages, 1);
                assert_eq!(report.images, 1);
                assert_eq!(report.conversion.pages_converted, 1);
                assert_eq!(report.conversion.bookmarks_written, 0);
                assert!(report.conversion.input_bytes_read <= built.bytes.len() as u64 + 48);
                assert_eq!(
                    image_streams(&pdf),
                    [(width as u32, height as u32, packed(&rows))],
                    "{layout:?} width {width} height {height}"
                );
                let media_box = format!("/MediaBox [0 0 {width}.000000 {height}.000000]");
                assert!(find(&pdf, media_box.as_bytes(), 0).is_some());
            }
        }
    }
}

#[test]
fn padding_bits_and_row_order_are_exact_for_known_rows() {
    // Width 9: the first byte is full and the second keeps only its MSB.
    // The PDF stream drops the two DIB padding bytes of each 4-byte row.
    let rows: Pixels = vec![
        vec![true, false, false, false, false, false, false, false, true],
        vec![false; 9],
        vec![false, true, true, true, true, true, true, true, false],
    ];
    let built = container(Layout::C8, &[vec![type0(&rows)]]);
    let (_, pdf) = convert(built.bytes, options()).unwrap();
    assert_eq!(
        image_streams(&pdf),
        [(9, 3, vec![0x80, 0x80, 0x00, 0x00, 0x7f, 0x00])]
    );
    check_renders(&pdf, &[&rows]);
}

#[test]
fn pages_keep_order_size_and_pixels_under_independent_renderers() {
    let first = pattern(33, 5, 1);
    let second = pattern(7, 1, 2);
    let third = pattern(16, 9, 3);
    for layout in LAYOUTS {
        let built = container(
            layout,
            &[
                vec![type0(&first)],
                vec![type0(&second)],
                vec![type0(&third)],
            ],
        );
        let (report, pdf) = convert(built.bytes, options()).unwrap();
        assert_eq!((report.source_pages, report.images), (3, 3));
        assert_eq!(report.conversion.pages_converted, 3);
        assert_eq!(
            image_streams(&pdf),
            [
                (33, 5, packed(&first)),
                (7, 1, packed(&second)),
                (16, 9, packed(&third)),
            ]
        );
        check_renders(&pdf, &[&first, &second, &third]);
    }
}

#[test]
fn separate_page_policy_splits_multi_image_pages_in_record_order() {
    let a = pattern(9, 4, 4);
    let b = pattern(31, 2, 5);
    let c = pattern(8, 3, 6);
    let built = container(Layout::HnA, &[vec![type0(&a), type0(&b)], vec![type0(&c)]]);
    let (report, pdf) = convert(
        built.bytes,
        Type0PdfOptions {
            multiple_images: MultipleImages::SeparatePages,
            ..options()
        },
    )
    .unwrap();
    assert_eq!((report.source_pages, report.images), (2, 3));
    assert_eq!(report.conversion.pages_converted, 3);
    check_renders(&pdf, &[&a, &b, &c]);
}

#[test]
fn separate_pages_stop_at_the_page_limit_before_reading_the_image() {
    let rows = pattern(9, 2, 18);
    let built = container(
        Layout::C8,
        &[vec![type0(&rows), type0(&rows)], vec![type0(&rows)]],
    );
    let descriptor = built.descriptors[1][0];
    let limits = Limits {
        max_pages: 2,
        ..Limits::default()
    };
    let mut source = Source::new(built.bytes);
    // Reading page 2's image wrapper would trip this rewrite of its width.
    source.rewrite = Some((built.payloads[1][0], built.payloads[1][0] as usize + 4, 0));
    let mut sink = Sink::default();
    let error = convert_with(
        &mut source,
        &mut sink,
        Type0PdfOptions {
            multiple_images: MultipleImages::SeparatePages,
            ..options()
        },
        &limits,
    )
    .unwrap_err();
    assert!(
        matches!(
            error.kind,
            Type0PdfErrorKind::Pdf(Error::LimitExceeded {
                resource: "pages",
                limit: 2,
                attempted: 3
            })
        ),
        "{error}"
    );
    assert_eq!(
        (error.page, error.image, error.offset),
        (Some(2), Some(1), Some(descriptor))
    );
    assert_eq!(image_streams(&sink.bytes).len(), 2);
    assert_eq!(source.bytes[built.payloads[1][0] as usize + 4], 9);
}

#[test]
fn a_wrapper_that_changes_between_reads_is_refused() {
    // Width 9 -> 17 keeps the 4-byte DIB stride, so only this check can
    // notice that the image dictionary no longer matches the rows.
    let rows = pattern(9, 3, 19);
    let built = container(Layout::HnB, &[vec![type0(&rows)]]);
    let payload = built.payloads[0][0];
    let mut source = Source::new(built.bytes);
    source.rewrite = Some((payload, payload as usize + 4, 17));
    let error = convert_with(
        &mut source,
        &mut Sink::default(),
        options(),
        &Limits::default(),
    )
    .unwrap_err();
    assert!(
        matches!(&error.kind, Type0PdfErrorKind::Image(e) if matches!(e.kind, Type0ErrorKind::Malformed(_))),
        "{error}"
    );
    assert_eq!(
        (error.page, error.image, error.offset),
        (Some(1), Some(1), Some(payload))
    );
    assert!(
        error
            .to_string()
            .ends_with("malformed DIB wrapper that changed between reads"),
        "{error}"
    );
}

#[test]
fn resolution_scales_page_geometry_only() {
    let rows = pattern(32, 2, 7);
    let built = container(Layout::HnB, &[vec![type0(&rows)]]);
    let (_, pdf) = convert(
        built.bytes,
        Type0PdfOptions {
            pixels_per_inch: 288.0,
            ..options()
        },
    )
    .unwrap();
    assert!(find(&pdf, b"/MediaBox [0 0 8.000000 0.500000]", 0).is_some());
    assert!(find(&pdf, b"q\n8.000000 0 0 0.500000 0 0 cm\n/Im0 Do\nQ\n", 0).is_some());
    assert_eq!(image_streams(&pdf), [(32, 2, packed(&rows))]);
}

#[test]
fn one_byte_ranged_reads_produce_identical_output() {
    let rows = pattern(33, 7, 8);
    let built = container(Layout::C8, &[vec![type0(&rows)], vec![type0(&rows)]]);
    let (_, expected) = convert(built.bytes.clone(), options()).unwrap();
    let limits = Limits {
        io_chunk_bytes: 1,
        ..Limits::default()
    };
    let mut source = Source::new(built.bytes);
    source.max_read = 1;
    let mut sink = Sink::default();
    let report = convert_with(&mut source, &mut sink, options(), &limits).unwrap();
    assert_eq!(sink.bytes, expected);
    assert_eq!(source.largest_request, 1);
    assert_eq!(
        report.conversion.output_bytes_written,
        expected.len() as u64
    );
}

// ---------------------------------------------------------------------------
// Explicit refusals with page, image, and source context.

#[test]
fn pages_without_images_or_with_several_are_refused_at_their_row() {
    let rows = pattern(8, 2, 9);
    let built = container(Layout::C8, &[vec![type0(&rows)], vec![]]);
    let error = convert_error(built.bytes, options());
    assert!(matches!(error.kind, Type0PdfErrorKind::NoImages), "{error}");
    assert_eq!((error.page, error.image), (Some(2), None));
    assert_eq!(error.offset, Some(0x50 + 20 + 8));
    assert_eq!(
        error.to_string(),
        "HN/C8 type-0 PDF conversion, page 2, source byte 108: page declares no images"
    );

    let built = container(Layout::HnB, &[vec![type0(&rows), type0(&rows)]]);
    let error = convert_error(built.bytes, options());
    assert!(
        matches!(error.kind, Type0PdfErrorKind::MultipleImages(2)),
        "{error}"
    );
    assert_eq!((error.page, error.image), (Some(1), None));
    assert!(
        error
            .to_string()
            .ends_with("page declares 2 images; placement is not measured")
    );
}

#[test]
fn unassigned_image_types_are_typed_and_located() {
    let rows = pattern(9, 2, 10);
    for kind in 1..=3 {
        let built = container(
            Layout::HnA,
            &[
                vec![type0(&rows)],
                vec![
                    type0(&rows),
                    Record {
                        kind,
                        payload: vec![1, 2, 3],
                    },
                ],
            ],
        );
        let descriptor = built.descriptors[1][1];
        let error = convert_error(
            built.bytes,
            Type0PdfOptions {
                multiple_images: MultipleImages::SeparatePages,
                ..options()
            },
        );
        assert!(
            matches!(error.kind, Type0PdfErrorKind::UnsupportedImageType(k) if k == kind as u32),
            "{error}"
        );
        assert_eq!((error.page, error.image), (Some(2), Some(2)));
        assert_eq!(error.offset, Some(descriptor));
        assert!(error.source().is_none());
        assert!(
            error
                .to_string()
                .ends_with(&format!("unsupported image record type {kind}"))
        );
    }
}

#[test]
fn container_errors_keep_their_own_location() {
    let rows = pattern(9, 2, 11);
    let mut built = container(Layout::C8, &[vec![type0(&rows)], vec![type0(&rows)]]);
    // An unmeasured positive type on page 2's descriptor.
    let descriptor = built.descriptors[1][0] as usize;
    built.bytes[descriptor..descriptor + 4].copy_from_slice(&9_i32.to_le_bytes());
    let error = convert_error(built.bytes, options());
    let Type0PdfErrorKind::Container(inner) = &error.kind else {
        panic!("{error}");
    };
    assert!(matches!(
        inner.kind,
        ErrorKind::Unsupported {
            field: "image type",
            value: 9
        }
    ));
    assert_eq!((error.page, error.image), (Some(2), Some(1)));
    assert_eq!(error.offset, Some(descriptor as u64));
    assert!(error.source().is_some());
    assert!(
        error.to_string().contains("unsupported image type: 9"),
        "{error}"
    );

    let error = convert_error(b"KDH ".to_vec(), options());
    assert!(
        matches!(error.kind, Type0PdfErrorKind::Container(_)),
        "{error}"
    );
    assert_eq!(
        (error.page, error.image, error.offset),
        (None, None, Some(0))
    );
}

#[test]
fn truncated_sources_fail_at_the_missing_bytes() {
    let rows = pattern(31, 4, 12);
    let built = container(Layout::HnB, &[vec![type0(&rows)]]);
    let payload = built.payloads[0][0];
    // Cutting inside the payload leaves the declared span outside the source.
    let mut short = built.bytes.clone();
    short.truncate(payload as usize + 50);
    let error = convert_error(short, options());
    let Type0PdfErrorKind::Container(inner) = &error.kind else {
        panic!("{error}");
    };
    assert!(matches!(
        inner.kind,
        ErrorKind::Truncated {
            field: "image payload",
            ..
        }
    ));
    assert_eq!((error.page, error.image), (Some(1), Some(1)));

    // A declared DIB-only payload has no coded bytes.
    let mut built = container(Layout::HnB, &[vec![type0(&rows)]]);
    let descriptor = built.descriptors[0][0] as usize;
    built.bytes[descriptor + 8..descriptor + 12].copy_from_slice(&48_i32.to_le_bytes());
    let error = convert_error(built.bytes, options());
    assert!(
        matches!(&error.kind, Type0PdfErrorKind::Image(e) if matches!(e.kind, Type0ErrorKind::Truncated(_))),
        "{error}"
    );
    assert_eq!((error.page, error.image), (Some(1), Some(1)));
    assert_eq!(error.offset, Some(payload + 48));
}

#[test]
fn corrupt_wrappers_and_impossible_dimensions_are_located() {
    let rows = pattern(9, 3, 13);
    let cases: [(usize, &[u8], u64, &str); 5] = [
        (
            14,
            &8_u16.to_le_bytes(),
            14,
            "unsupported DIB bit count (8)",
        ),
        (
            4,
            &0_i32.to_le_bytes(),
            4,
            "malformed nonpositive DIB dimensions",
        ),
        (
            8,
            &(-3_i32).to_le_bytes(),
            4,
            "malformed nonpositive DIB dimensions",
        ),
        (
            4,
            &40_000_i32.to_le_bytes(),
            4,
            "image width limit 32768 exceeded by 40000",
        ),
        (40, &[0, 0, 0], 40, "unsupported DIB palette (0)"),
    ];
    for (field, value, relative, message) in cases {
        let mut built = container(Layout::C8, &[vec![type0(&rows)]]);
        let payload = built.payloads[0][0];
        let at = payload as usize + field;
        built.bytes[at..at + value.len()].copy_from_slice(value);
        let error = convert_error(built.bytes, options());
        assert!(matches!(error.kind, Type0PdfErrorKind::Image(_)), "{error}");
        assert_eq!((error.page, error.image), (Some(1), Some(1)));
        assert_eq!(error.offset, Some(payload + relative), "{error}");
        assert!(error.to_string().ends_with(message), "{error}");
        assert!(error.source().is_some());
    }
}

#[test]
fn every_sink_failure_is_reported_and_leaves_a_prefix() {
    let rows = pattern(9, 3, 14);
    let built = container(Layout::C8, &[vec![type0(&rows)], vec![type0(&rows)]]);
    let mut clean_source = Source::new(built.bytes.clone());
    let mut clean = Sink::default();
    convert_with(&mut clean_source, &mut clean, options(), &Limits::default()).unwrap();
    assert!(clean.writes > 30);
    for fail_at in 1..=clean.writes {
        let mut source = Source::new(built.bytes.clone());
        let mut sink = Sink {
            fail_at: Some(fail_at),
            ..Sink::default()
        };
        let error =
            convert_with(&mut source, &mut sink, options(), &Limits::default()).unwrap_err();
        let io = match &error.kind {
            Type0PdfErrorKind::Pdf(Error::Io(io)) => io,
            Type0PdfErrorKind::Image(image) => match &image.kind {
                Type0ErrorKind::Sink(Error::Io(io)) => io,
                other => panic!("write {fail_at}: {other:?}"),
            },
            other => panic!("write {fail_at}: {other:?}"),
        };
        assert_eq!(io.to_string(), "injected sink failure");
        assert!(error.source().is_some(), "write {fail_at}");
        assert_eq!(sink.writes, fail_at, "no write follows the failure");
        assert!(clean.bytes.starts_with(&sink.bytes));
    }
}

#[test]
fn cancellation_at_every_check_never_reports_success() {
    let rows = pattern(9, 3, 15);
    let built = container(Layout::HnA, &[vec![type0(&rows)], vec![type0(&rows)]]);
    let table = table();
    let mut allowed = 0;
    loop {
        let mut source = Source::new(built.bytes.clone());
        let mut sink = Sink::default();
        let cancellation = CancelAfter::new(allowed);
        match ready(convert_type0_pdf(
            &mut source,
            &mut sink,
            &table,
            options(),
            &Limits::default(),
            &cancellation,
        )) {
            Ok(report) => {
                assert!(allowed > 50, "only {allowed} checks");
                assert_eq!(report.images, 2);
                break;
            }
            Err(error) => {
                let cancelled = match &error.kind {
                    Type0PdfErrorKind::Container(inner) => {
                        matches!(inner.kind, ErrorKind::Cancelled)
                    }
                    Type0PdfErrorKind::Image(inner) => {
                        matches!(inner.kind, Type0ErrorKind::Cancelled)
                    }
                    Type0PdfErrorKind::Pdf(Error::Cancelled) => true,
                    _ => false,
                };
                assert!(cancelled, "check {allowed}: {error}");
                allowed += 1;
            }
        }
    }
}

#[test]
fn shared_and_format_limits_fail_with_their_resource() {
    let rows = pattern(33, 4, 16);
    let built = container(Layout::C8, &[vec![type0(&rows)], vec![type0(&rows)]]);
    let run = |limits: Limits, options: Type0PdfOptions| {
        let mut source = Source::new(built.bytes.clone());
        convert_with(&mut source, &mut Sink::default(), options, &limits).unwrap_err()
    };

    let error = run(
        Limits {
            max_pages: 1,
            ..Limits::default()
        },
        options(),
    );
    assert!(
        matches!(&error.kind, Type0PdfErrorKind::Container(e) if matches!(e.kind, ErrorKind::LimitExceeded { resource: "pages", .. })),
        "{error}"
    );

    let error = run(
        Limits {
            max_output_bytes: 600,
            ..Limits::default()
        },
        options(),
    );
    assert!(
        matches!(
            &error.kind,
            Type0PdfErrorKind::Pdf(Error::LimitExceeded {
                resource: "output bytes",
                ..
            }) | Type0PdfErrorKind::Image(_)
        ),
        "{error}"
    );

    let error = run(
        Limits {
            io_chunk_bytes: 256,
            max_allocation_bytes: 1024,
            ..Limits::default()
        },
        options(),
    );
    assert!(
        matches!(error.kind, Type0PdfErrorKind::Contexts(_)),
        "{error}"
    );
    assert_eq!((error.page, error.image, error.offset), (None, None, None));
    assert!(error.source().is_some());
    assert!(error.to_string().contains("arithmetic contexts"), "{error}");

    let mut small = options();
    small.image.max_pixels = 100;
    let error = run(Limits::default(), small);
    assert!(
        error
            .to_string()
            .ends_with("image pixels limit 100 exceeded by 132"),
        "{error}"
    );

    let mut small = options();
    small.arithmetic.max_work = 10;
    let error = run(Limits::default(), small);
    assert!(
        matches!(&error.kind, Type0PdfErrorKind::Image(e) if matches!(e.kind, Type0ErrorKind::Arithmetic(_))),
        "{error}"
    );
    assert_eq!((error.page, error.image), (Some(1), Some(1)));

    let mut small = options();
    small.container.max_images_per_page = 0;
    let error = run(Limits::default(), small);
    assert!(
        matches!(&error.kind, Type0PdfErrorKind::Container(e) if matches!(e.kind, ErrorKind::LimitExceeded { resource: "images per page", .. })),
        "{error}"
    );
}

#[test]
fn page_geometry_and_resolution_are_validated() {
    let rows = pattern(8, 1, 17);
    let built = container(Layout::C8, &[vec![type0(&rows)]]);
    for pixels_per_inch in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let error = convert_error(
            built.bytes.clone(),
            Type0PdfOptions {
                pixels_per_inch,
                ..options()
            },
        );
        assert!(
            matches!(error.kind, Type0PdfErrorKind::InvalidOptions(_)),
            "{error}"
        );
        assert_eq!(
            error.to_string(),
            "HN/C8 type-0 PDF conversion: invalid options: pixels per inch must be finite and positive"
        );
    }
    let mut accepted = options();
    accepted.arithmetic.max_symbols = MAX_BUDGET_COUNT;
    accepted.arithmetic.max_work = MAX_BUDGET_COUNT;
    convert(built.bytes.clone(), accepted).unwrap();
    for (max_symbols, max_work) in [
        (0, 1),
        (1, 0),
        (MAX_BUDGET_COUNT + 1, MAX_BUDGET_COUNT),
        (MAX_BUDGET_COUNT, MAX_BUDGET_COUNT + 1),
    ] {
        let mut invalid = options();
        invalid.arithmetic.max_symbols = max_symbols;
        invalid.arithmetic.max_work = max_work;
        let mut source = Source::new(built.bytes.clone());
        let mut sink = Sink::default();
        let error = convert_with(&mut source, &mut sink, invalid, &Limits::default()).unwrap_err();
        assert_eq!((error.page, error.image, error.offset), (None, None, None));
        assert_eq!(
            error.to_string(),
            "HN/C8 type-0 PDF conversion: invalid options: arithmetic budget fields must be in 1..=MAX_BUDGET_COUNT"
        );
        assert!(sink.bytes.is_empty());
    }
    // Eight pixels at 0.01 ppi is 57,600 points, beyond the page profile.
    let error = convert_error(
        built.bytes,
        Type0PdfOptions {
            pixels_per_inch: 0.01,
            ..options()
        },
    );
    assert!(
        matches!(
            error.kind,
            Type0PdfErrorKind::Pdf(Error::InvalidInput { .. })
        ),
        "{error}"
    );
    assert_eq!((error.page, error.image), (Some(1), Some(1)));
    assert!(error.to_string().contains("PDF output: "), "{error}");
}

// ---------------------------------------------------------------------------
// The streamed bilevel image API on its own.

fn bilevel(width: u32, height: u32, row_stride: usize) -> BilevelImageSpec {
    BilevelImageSpec {
        pixel_width: width,
        pixel_height: height,
        row_stride,
    }
}

#[test]
fn bilevel_writer_drops_stride_padding_across_split_writes() {
    let limits = Limits::default();
    let mut sink = Sink::default();
    ready(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NeverCancel).await?;
        let mut image = document.begin_bilevel_image(bilevel(12, 2, 5)).await?;
        // Row 1 = aa bb | pad x3, row 2 = cc dd | pad x3, split unevenly.
        for chunk in [&[0xaa][..], &[0xbb, 1, 2], &[3, 0xcc, 0xdd, 4], &[5, 6]] {
            assert_eq!(image.write(chunk).await?, chunk.len());
        }
        image.flush().await?;
        let object = image.finish().await?;
        let page = PageSpec {
            width_points: 12.0,
            height_points: 2.0,
        };
        assert_eq!(document.add_page(page, &[object, object]).await?, 0);
        document.finish().await
    })
    .unwrap();
    assert_eq!(
        image_streams(&sink.bytes),
        [(12, 2, vec![0xaa, 0xbb, 0xcc, 0xdd])]
    );
    // Two placements of one XObject, drawn in order.
    assert!(find(&sink.bytes, b"/Im0 3 0 R /Im1 3 0 R", 0).is_some());
}

#[test]
fn bilevel_writer_checks_geometry_and_row_counts() {
    let limits = Limits::default();
    let page = PageSpec {
        width_points: 1.0,
        height_points: 1.0,
    };
    let mut sink = Sink::default();
    ready(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NeverCancel).await?;
        for (spec, message) in [
            (bilevel(0, 1, 1), "image width and height must be nonzero"),
            (bilevel(1, 0, 1), "image width and height must be nonzero"),
            (
                bilevel(9, 1, 1),
                "bilevel row stride is shorter than the packed row",
            ),
            (
                bilevel(8, 2, usize::MAX),
                "bilevel input byte count overflows",
            ),
        ] {
            let Err(Error::InvalidInput { reason }) = document.begin_bilevel_image(spec).await
            else {
                panic!("{spec:?} was accepted");
            };
            assert_eq!(reason, message);
        }
        for (spec, resource) in [
            (bilevel(u32::MAX, 1, 1 << 29), "PDF image width"),
            (bilevel(1, u32::MAX, 1), "PDF image height"),
            (bilevel(1 << 30, 1 << 5, 1 << 27), "PDF image stream bytes"),
        ] {
            let Err(Error::LimitExceeded { resource: got, .. }) =
                document.begin_bilevel_image(spec).await
            else {
                panic!("{spec:?} was accepted");
            };
            assert_eq!(got, resource);
        }
        let Err(Error::InvalidInput { reason }) = document.add_page(page, &[]).await else {
            panic!("an empty page was accepted");
        };
        assert_eq!(reason, "PDF page requires at least one image");

        let mut image = document.begin_bilevel_image(bilevel(8, 2, 1)).await?;
        image.write(&[1]).await?;
        let Err(Error::InvalidInput { reason }) = image.write(&[2, 3]).await else {
            panic!("an extra row was accepted");
        };
        assert_eq!(reason, "bilevel image rows exceed the declared height");
        let Err(Error::InvalidInput { reason }) = image.finish().await else {
            panic!("a short image was accepted");
        };
        assert_eq!(reason, "bilevel image ended before its declared height");
        // The unfinished stream blocks later objects instead of corrupting them.
        assert!(
            document
                .begin_bilevel_image(bilevel(8, 1, 1))
                .await
                .is_err()
        );
        Ok::<_, Error>(())
    })
    .unwrap();
}

#[test]
fn bilevel_padding_writes_still_observe_cancellation() {
    let limits = Limits::default();
    for allowed in 0.. {
        let cancellation = CancelAfter::new(allowed);
        let mut sink = Sink::default();
        let result = ready(async {
            let mut document = PdfDocument::new(&mut sink, &limits, &cancellation).await?;
            let mut image = document.begin_bilevel_image(bilevel(8, 1, 4)).await?;
            image.write(&[0x81]).await?;
            // The next check is reached only by this padding-only write.
            Ok::<_, Error>(image.write(&[0, 0, 0]).await)
        });
        if let Ok(padding) = result {
            assert!(matches!(padding, Err(Error::Cancelled)), "{padding:?}");
            break;
        }
    }
}
