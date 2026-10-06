// SPDX-License-Identifier: MIT

//! Synthetic HN/C8 type-0 images converted through the document pipeline.
//!
//! The coded images are produced at test runtime by an original, test-only
//! arithmetic encoder written from the T.82 interval description, for the
//! standard T.82 Table 24 states and the observed type-0 row rule. Nothing
//! here is corpus data or a decoder oracle.

mod common;

use caj2pdf_core::{
    Cancellation, Error, Limits, NeverCancel, RangedSource, SequentialSink,
    hnc8::{
        ComposeError, ComposeErrorKind, ComposeOptions, ComposeReport, ComposeStage, ErrorKind,
        Variant,
    },
    jbig1::Type0ErrorKind,
    pdf::{BilevelImageSpec, PageSpec, PdfDocument},
    qm::{ArithmeticErrorKind, QmTable},
};
use common::{
    CancelAfter,
    hnc8_document::{Image, RENDER_DPI, convert as compose, document, ready},
};
use std::{
    error::Error as _,
    fs::{read, remove_file, write},
    io,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

fn table() -> QmTable {
    QmTable::standard()
}

type Pixels = Vec<Vec<bool>>;

/// Code rows with the documented type-0 model for the standard T.82 states:
/// a row-control decision (one copies the preceding row, blank above row
/// 0), else each pixel with ten neighbors.
fn encode_rows(rows: &Pixels) -> Vec<u8> {
    let mut encoder = common::qm_encoder();
    encoder.type0_rows(rows, true);
    let mut bytes = encoder.finish();
    if bytes.is_empty() {
        bytes.push(0);
    }
    bytes
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
// One-image documents for the composition pipeline. HN-B admits only JPEG
// pages, so type-0 images are composed from C8 and HN-A documents.

const LAYOUTS: [Variant; 2] = [Variant::C8, Variant::HnA];

fn type0(rows: &Pixels) -> Image {
    Image {
        kind: 0,
        payload: type0_payload(rows),
        width: rows[0].len() as u32,
        height: rows.len() as u32,
    }
}

// ---------------------------------------------------------------------------
// I/O doubles.

struct Source {
    bytes: Vec<u8>,
    max_read: usize,
    largest_request: usize,
}

impl Source {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            max_read: usize::MAX,
            largest_request: 0,
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

fn convert_with<C: Cancellation>(
    source: &mut Source,
    sink: &mut Sink,
    options: ComposeOptions,
    limits: &Limits,
    cancellation: &C,
) -> Result<ComposeReport, ComposeError> {
    compose(
        source,
        sink,
        Some(&table()),
        &mut Default::default(),
        &mut (),
        options,
        limits,
        cancellation,
    )
}

fn convert(bytes: Vec<u8>) -> Result<(ComposeReport, Vec<u8>), ComposeError> {
    let mut source = Source::new(bytes);
    let mut sink = Sink::default();
    let report = convert_with(
        &mut source,
        &mut sink,
        ComposeOptions::default(),
        &Limits::default(),
        &NeverCancel,
    )?;
    assert_eq!(
        report.conversion.output_bytes_written,
        sink.bytes.len() as u64
    );
    Ok((report, sink.bytes))
}

fn convert_error(bytes: Vec<u8>, options: ComposeOptions, limits: &Limits) -> ComposeError {
    let mut source = Source::new(bytes);
    convert_with(
        &mut source,
        &mut Sink::default(),
        options,
        limits,
        &NeverCancel,
    )
    .unwrap_err()
}

/// Every cancellation surface of the type-0 composition path.
fn cancelled(error: &ComposeError) -> bool {
    match &error.kind {
        ComposeErrorKind::Container(inner) => matches!(inner.kind, ErrorKind::Cancelled),
        ComposeErrorKind::Image(inner) => matches!(
            inner.kind,
            Type0ErrorKind::Cancelled | Type0ErrorKind::Sink(Error::Cancelled)
        ),
        ComposeErrorKind::Contexts(inner) => matches!(inner.kind, ArithmeticErrorKind::Cancelled),
        ComposeErrorKind::Io(Error::Cancelled) => true,
        ComposeErrorKind::Cleanup { primary, .. } => cancelled(primary),
        _ => false,
    }
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
        let text = String::from_utf8_lossy(&pdf[start..start + 145]).into_owned();
        let number = |key: &str| -> u32 {
            let rest = &text[text.find(key).unwrap() + key.len()..];
            rest[..rest.find('\n').unwrap()].parse().unwrap()
        };
        let (width, height) = (number("/Width "), number("/Height "));
        assert!(
            text.contains(
                "/ColorSpace /DeviceGray\n/BitsPerComponent 1\n/Decode [1 0]\n/Filter /FlateDecode\n>>\nstream\n"
            )
        );
        let data = find(pdf, b">>\nstream\n", start).unwrap() + b">>\nstream\n".len();
        use std::io::Read;
        let mut decoder = flate2::read::ZlibDecoder::new(&pdf[data..]);
        let mut pixels = Vec::new();
        decoder.read_to_end(&mut pixels).unwrap();
        assert_eq!(pixels.len(), width.div_ceil(8) as usize * height as usize);
        let length = decoder.total_in() as usize;
        assert_eq!(&pdf[data + length..data + length + 11], b"\nendstream\n");
        images.push((width, height, pixels));
        from = data + length;
    }
    images
}

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

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
/// Check the one-page PDF with qpdf, then render it with Poppler and MuPDF
/// and compare the black pixels with the expected rows. The page is the
/// image's declared extent, one device pixel per image pixel at
/// [`RENDER_DPI`].
fn check_renders(pdf: &[u8], rows: &Pixels) {
    let file = Temp::new("pdf");
    write(&file.0, pdf).unwrap();
    tool(
        Command::new("qpdf").arg("--check").arg(&file.0),
        "qpdf --check",
    );
    let info = tool(Command::new("pdfinfo").arg(&file.0), "pdfinfo");
    let info = String::from_utf8(info.stdout).unwrap();
    assert!(
        info.split_whitespace()
            .collect::<Vec<_>>()
            .windows(2)
            .any(|pair| pair == ["Pages:", "1"]),
        "{info}"
    );
    let want = (rows[0].len() as u32, rows.len() as u32, packed(rows));
    // Poppler smooths a 1:1 image blit, so render it at ten device pixels
    // per image pixel and sample each block's centre instead.
    let poppler = Temp::new("poppler");
    let dpi: f64 = RENDER_DPI.parse().unwrap();
    tool(
        Command::new("pdftoppm")
            .args(["-mono", "-r", &(dpi * 10.0).to_string(), "-singlefile"])
            .arg(&file.0)
            .arg(&poppler.0),
        "pdftoppm -mono",
    );
    let poppler_pbm = Path::new(&poppler.0).with_extension("poppler.pbm");
    let rendered = read(&poppler_pbm).unwrap();
    let _ = remove_file(&poppler_pbm);
    assert_eq!(block_centres(pbm(&rendered), 10), want, "Poppler");
    let mupdf = Temp::new("pbm");
    tool(
        Command::new("mutool")
            .args(["draw", "-q", "-r", RENDER_DPI, "-o"])
            .arg(&mupdf.0)
            .arg(&file.0)
            .arg("1"),
        "mutool draw",
    );
    assert_eq!(pbm(&read(&mupdf.0).unwrap()), want, "MuPDF");
}

// ---------------------------------------------------------------------------
// Successful conversions.

#[test]
fn boundary_widths_have_exact_packed_rows_in_every_layout() {
    for layout in LAYOUTS {
        for (seed, width) in [7, 8, 9, 31, 32, 33].into_iter().enumerate() {
            for height in [1, 6] {
                let rows = pattern(width, height, seed);
                let built = document(layout, &[vec![type0(&rows)]]);
                let (report, pdf) = convert(built.bytes.clone()).unwrap();
                assert_eq!(report.source_variant, layout);
                assert_eq!((report.source_pages, report.output_pages), (1, 1));
                assert_eq!(report.type0_images, 1);
                assert_eq!(report.conversion.pages_converted, 1);
                assert_eq!(report.conversion.bookmarks_written, 0);
                assert!(report.conversion.input_bytes_read <= 2 * built.bytes.len() as u64);
                assert_eq!(
                    image_streams(&pdf),
                    [(width as u32, height as u32, packed(&rows))],
                    "{layout:?} width {width} height {height}"
                );
            }
        }
    }
}

#[test]
fn padding_bits_and_row_order_are_exact_for_known_rows() {
    // Width 9: the first byte is full and the second keeps only its MSB.
    // The PDF stream drops the two DIB padding bytes of each 4-byte row and
    // holds the rows top-first, drawn upright by a positive-height matrix.
    let rows: Pixels = vec![
        vec![true, false, false, false, false, false, false, false, true],
        vec![false; 9],
        vec![false, true, true, true, true, true, true, true, false],
    ];
    let built = document(Variant::C8, &[vec![type0(&rows)]]);
    let (_, pdf) = convert(built.bytes).unwrap();
    assert_eq!(
        image_streams(&pdf),
        [(9, 3, vec![0x80, 0x80, 0x00, 0x00, 0x7f, 0x00])]
    );
    check_renders(&pdf, &rows);
    let rows = pattern(33, 5, 1);
    let built = document(Variant::HnA, &[vec![type0(&rows)]]);
    check_renders(&convert(built.bytes).unwrap().1, &rows);
}

#[test]
fn one_byte_ranged_reads_produce_identical_output() {
    let rows = pattern(33, 7, 8);
    let built = document(Variant::C8, &[vec![type0(&rows)], vec![type0(&rows)]]);
    let (_, expected) = convert(built.bytes.clone()).unwrap();
    let limits = Limits {
        io_chunk_bytes: 1,
        ..Limits::default()
    };
    let mut source = Source::new(built.bytes);
    source.max_read = 1;
    let mut sink = Sink::default();
    let report = convert_with(
        &mut source,
        &mut sink,
        ComposeOptions::default(),
        &limits,
        &NeverCancel,
    )
    .unwrap();
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
fn container_errors_keep_their_own_location() {
    let rows = pattern(9, 2, 11);
    let mut built = document(Variant::C8, &[vec![type0(&rows)], vec![type0(&rows)]]);
    // An unmeasured positive type on page 2's descriptor.
    let descriptor = built.descriptors[1][0] as usize;
    built.bytes[descriptor..descriptor + 4].copy_from_slice(&9_i32.to_le_bytes());
    let error = convert_error(built.bytes, ComposeOptions::default(), &Limits::default());
    let ComposeErrorKind::Container(inner) = &error.kind else {
        panic!("{error}");
    };
    assert!(matches!(
        inner.kind,
        ErrorKind::Unsupported {
            field: "image type",
            value: 9
        }
    ));
    assert_eq!(error.stage, ComposeStage::Container);
    assert_eq!((error.page, error.image), (Some(2), Some(1)));
    assert_eq!(error.offset, Some(descriptor as u64));
    assert!(error.source().is_some());
    assert!(
        error.to_string().contains("unsupported image type: 9"),
        "{error}"
    );

    let error = convert_error(
        b"KDH ".to_vec(),
        ComposeOptions::default(),
        &Limits::default(),
    );
    assert!(
        matches!(error.kind, ComposeErrorKind::Container(_)),
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
    let built = document(Variant::HnA, &[vec![type0(&rows)]]);
    let payload = built.payloads[0][0];
    // Cutting inside the payload leaves the declared span outside the source.
    let mut short = built.bytes.clone();
    short.truncate(payload as usize + 50);
    let error = convert_error(short, ComposeOptions::default(), &Limits::default());
    let ComposeErrorKind::Container(inner) = &error.kind else {
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
    let mut built = document(Variant::HnA, &[vec![type0(&rows)]]);
    let descriptor = built.descriptors[0][0] as usize;
    built.bytes[descriptor + 8..descriptor + 12].copy_from_slice(&48_i32.to_le_bytes());
    let error = convert_error(built.bytes, ComposeOptions::default(), &Limits::default());
    assert!(
        matches!(&error.kind, ComposeErrorKind::Image(e) if matches!(e.kind, Type0ErrorKind::Truncated(_))),
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
        let mut built = document(Variant::C8, &[vec![type0(&rows)]]);
        let payload = built.payloads[0][0];
        let at = payload as usize + field;
        built.bytes[at..at + value.len()].copy_from_slice(value);
        let mut source = Source::new(built.bytes);
        let mut sink = Sink::default();
        let error = convert_with(
            &mut source,
            &mut sink,
            ComposeOptions::default(),
            &Limits::default(),
            &NeverCancel,
        )
        .unwrap_err();
        assert!(matches!(error.kind, ComposeErrorKind::Image(_)), "{error}");
        assert_eq!(error.stage, ComposeStage::Headers);
        assert_eq!((error.page, error.image), (Some(1), Some(1)));
        assert_eq!(error.offset, Some(payload + relative), "{error}");
        assert!(error.to_string().ends_with(message), "{error}");
        assert!(error.source().is_some());
        assert!(find(&sink.bytes, b"/Subtype /Image", 0).is_none());
    }
}

#[test]
fn every_sink_failure_is_reported_and_leaves_a_prefix() {
    let rows = pattern(9, 3, 14);
    let built = document(Variant::C8, &[vec![type0(&rows)], vec![type0(&rows)]]);
    let mut clean_source = Source::new(built.bytes.clone());
    let mut clean = Sink::default();
    convert_with(
        &mut clean_source,
        &mut clean,
        ComposeOptions::default(),
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap();
    assert!(clean.writes > 30);
    for fail_at in 1..=clean.writes {
        let mut source = Source::new(built.bytes.clone());
        let mut sink = Sink {
            fail_at: Some(fail_at),
            ..Sink::default()
        };
        let error = convert_with(
            &mut source,
            &mut sink,
            ComposeOptions::default(),
            &Limits::default(),
            &NeverCancel,
        )
        .unwrap_err();
        let io = match &error.kind {
            ComposeErrorKind::Io(Error::Io(io)) => io,
            other => panic!("write {fail_at}: {other:?}"),
        };
        assert_eq!(io.to_string(), "injected sink failure");
        assert_eq!(error.stage, ComposeStage::Pdf, "write {fail_at}");
        assert!(error.source().is_some(), "write {fail_at}");
        assert_eq!(sink.writes, fail_at, "no write follows the failure");
        assert!(clean.bytes.starts_with(&sink.bytes));
    }
}

#[test]
fn cancellation_at_every_check_never_reports_success() {
    let rows = pattern(9, 3, 15);
    let built = document(Variant::HnA, &[vec![type0(&rows)], vec![type0(&rows)]]);
    let mut allowed = 0;
    let mut located = false;
    loop {
        let mut source = Source::new(built.bytes.clone());
        let mut sink = Sink::default();
        let cancellation = CancelAfter::new(allowed);
        match convert_with(
            &mut source,
            &mut sink,
            ComposeOptions::default(),
            &Limits::default(),
            &cancellation,
        ) {
            Ok(report) => {
                assert!(allowed > 50, "only {allowed} checks");
                assert_eq!(report.type0_images, 2);
                break;
            }
            Err(error) => {
                assert!(cancelled(&error), "check {allowed}: {error}");
                located |= error.image.is_some();
                allowed += 1;
            }
        }
    }
    assert!(located, "some cancellation is located at its image");
}

#[test]
fn shared_and_format_limits_fail_with_their_resource() {
    let rows = pattern(33, 4, 16);
    let built = document(Variant::C8, &[vec![type0(&rows)], vec![type0(&rows)]]);
    let run = |limits: Limits, options: ComposeOptions| {
        convert_error(built.bytes.clone(), options, &limits)
    };

    let error = run(
        Limits {
            max_pages: 1,
            ..Limits::default()
        },
        ComposeOptions::default(),
    );
    assert!(
        matches!(&error.kind, ComposeErrorKind::Container(e) if matches!(e.kind, ErrorKind::LimitExceeded { resource: "pages", .. })),
        "{error}"
    );

    let error = run(
        Limits {
            max_output_bytes: 600,
            ..Limits::default()
        },
        ComposeOptions::default(),
    );
    assert!(
        matches!(
            &error.kind,
            ComposeErrorKind::Io(Error::LimitExceeded {
                resource: "output bytes",
                ..
            })
        ),
        "{error}"
    );

    let mut small = ComposeOptions::default();
    small.image.max_pixels = 100;
    let error = run(Limits::default(), small);
    assert_eq!(error.stage, ComposeStage::Headers);
    assert!(
        error
            .to_string()
            .ends_with("image pixels limit 100 exceeded by 132"),
        "{error}"
    );

    let mut small = ComposeOptions::default();
    small.arithmetic.max_work = 10;
    let error = run(Limits::default(), small);
    assert!(
        matches!(&error.kind, ComposeErrorKind::Image(e) if matches!(e.kind, Type0ErrorKind::Arithmetic(_))),
        "{error}"
    );
    assert_eq!((error.page, error.image), (Some(1), Some(1)));

    let mut small = ComposeOptions::default();
    small.container.max_images_per_page = 0;
    let error = run(Limits::default(), small);
    assert!(
        matches!(&error.kind, ComposeErrorKind::Container(e) if matches!(e.kind, ErrorKind::LimitExceeded { resource: "images per page", .. })),
        "{error}"
    );
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
