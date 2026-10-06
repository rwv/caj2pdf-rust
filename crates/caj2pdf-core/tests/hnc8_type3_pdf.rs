// SPDX-License-Identifier: MIT

//! Original synthetic HN/C8 type-3 records composed through the document
//! pipeline, MQ-coded for the standard T.88 states by the test-only encoder.
//! No private document or oracle pixel is embedded.

mod common;

use caj2pdf_core::hnc8::convert_source_pages_pdf as compose;
use caj2pdf_core::{
    Cancellation, Error, Limits, NeverCancel, RangedSource,
    hnc8::{
        ComposeError, ComposeErrorKind, ComposeOptions, ComposePage, ComposeReport, ComposeStage,
        ComposeVisitor, Type3Stage, Variant,
    },
    jbig2::text::{TextHeaderAnomaly, TextHeaderPolicy},
    pdf::{BilevelImageSpec, PageSpec, PdfDocument},
};
use common::{
    CancelAfter,
    hnc8_document::{Image, RENDER_DPI, document},
    mq_encoder,
};
use std::io::Write;
use std::{
    fs, io,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

/// HN-B admits only JPEG pages, so type-3 images compose from C8 and HN-A.
const LAYOUTS: [Variant; 2] = [Variant::C8, Variant::HnA];

#[path = "common/type3_fixture.rs"]
mod type3_fixture;
use type3_fixture::*;

/// Payload offsets of the five segments of `type3(width, height, _)` and
/// the payload end; each header has a one-byte page association.
fn segment_starts(payload: &[u8]) -> [usize; 6] {
    let mut starts = [0; 6];
    let mut at = 48;
    for start in &mut starts[..5] {
        *start = at;
        let references = usize::from(payload[at + 5] >> 5);
        let header = 6 + references + 1 + 4;
        let length = u32::from_be_bytes(payload[at + header - 4..at + header].try_into().unwrap());
        at += header + length as usize;
    }
    starts[5] = at;
    starts
}

fn type3(width: u32, height: u32, text_flags: u16) -> Image {
    Image {
        kind: 3,
        payload: payload(width, height, text_flags),
        width,
        height,
    }
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
    /// Absolute `[start, end)` span whose returned bytes are counted.
    payload: Option<(u64, u64)>,
    payload_bytes_read: u64,
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
            payload: None,
            payload_bytes_read: 0,
        }
    }
}

impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.advertised_size.unwrap_or(self.bytes.len() as u64)
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> caj2pdf_core::Result<usize> {
        self.calls += 1;
        self.max_request = self.max_request.max(destination.len());
        if self.fail_at == Some(offset) {
            return Err(Error::Io(io::Error::other("injected source failure")));
        }
        if let Some((watched, visit)) = self.fail_on_offset_visit
            && offset == watched
        {
            self.failed_offset_visits += 1;
            if self.failed_offset_visits == visit {
                return Err(Error::Io(io::Error::other(
                    "injected staged source failure",
                )));
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
        if let Some((first, end)) = self.payload {
            let overlap = (offset + count as u64)
                .min(end)
                .saturating_sub(offset.max(first));
            self.payload_bytes_read += overlap;
        }
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

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.writes += 1;
        if self.fail_at == Some(self.writes) {
            return Err(io::Error::other("injected sink failure"));
        }
        let count = if self.max_write == 0 {
            bytes.len()
        } else {
            bytes.len().min(self.max_write)
        };
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Collects each drawn image's reported text-header anomaly.
#[derive(Default)]
struct Anomalies(Vec<Option<TextHeaderAnomaly>>);

impl ComposeVisitor for Anomalies {
    fn page(&mut self, page: ComposePage<'_>) -> caj2pdf_core::Result<()> {
        self.0.extend(
            page.images
                .iter()
                .map(|image| image.type3_text_header_anomaly),
        );
        Ok(())
    }
}

fn run_with<C: Cancellation>(
    source: &mut Source,
    sink: &mut Sink,
    visitor: &mut Anomalies,
    options: ComposeOptions,
    limits: &Limits,
    cancellation: &C,
) -> Result<ComposeReport, ComposeError> {
    compose(source, sink, None, visitor, options, limits, cancellation)
}

fn run<C: Cancellation>(
    source: &mut Source,
    sink: &mut Sink,
    options: ComposeOptions,
    limits: &Limits,
    cancellation: &C,
) -> Result<ComposeReport, ComposeError> {
    run_with(
        source,
        sink,
        &mut Anomalies::default(),
        options,
        limits,
        cancellation,
    )
}

fn run_error(bytes: Vec<u8>, options: ComposeOptions) -> (ComposeError, Vec<u8>) {
    let mut source = Source::new(bytes);
    let mut sink = Sink::default();
    let error = run(
        &mut source,
        &mut sink,
        options,
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    (error, sink.bytes)
}

fn stage(error: &ComposeError) -> Option<Type3Stage> {
    match error.kind {
        ComposeErrorKind::Type3 { stage, .. } => Some(stage),
        _ => None,
    }
}

/// No image object was started before the failure.
fn no_image(pdf: &[u8]) -> bool {
    find(pdf, b"/Subtype /Image").is_none()
}

fn find(bytes: &[u8], needle: &[u8]) -> Option<usize> {
    bytes
        .windows(needle.len())
        .position(|window| window == needle)
}

fn embedded_image(pdf: &[u8]) -> Vec<u8> {
    let image = find(pdf, b"/Subtype /Image").unwrap();
    let start = image + find(&pdf[image..], b"stream\n").unwrap() + 7;
    use std::io::Read;
    let mut pixels = Vec::new();
    flate2::read::ZlibDecoder::new(&pdf[start..])
        .read_to_end(&mut pixels)
        .unwrap();
    pixels
}

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

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
fn one_image_documents_produce_checked_pages_in_every_layout() {
    let limits = Limits::default();
    for layout in LAYOUTS {
        for (width, height) in [(3, 2), (9, 3), (8, 2), (7, 4)] {
            let built = document(layout, &[vec![type3(width, height, 0x10)]]);
            let mut source = Source::new(built.bytes);
            source.max_read = 3;
            let mut sink = Sink {
                max_write: 5,
                ..Sink::default()
            };
            let mut anomalies = Anomalies::default();
            let report = run_with(
                &mut source,
                &mut sink,
                &mut anomalies,
                ComposeOptions::default(),
                &limits,
                &NeverCancel,
            )
            .unwrap_or_else(|error| panic!("{layout:?} {width}x{height}: {error}"));
            assert_eq!(report.source_variant, layout);
            assert_eq!((report.source_pages, report.output_pages), (1, 1));
            assert_eq!(report.type3_images, 1);
            assert_eq!(anomalies.0, [None]);
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
fn nonblank_asymmetric_pixels_keep_top_left_black_with_two_renderers() {
    let built = document(Variant::C8, &[vec![type3(9, 3, 0x10)]]);
    let mut source = Source::new(built.bytes);
    let mut sink = Sink::default();
    run(
        &mut source,
        &mut sink,
        ComposeOptions::default(),
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap();
    const PACKED: [u8; 6] = [0x80, 0, 0, 0, 0, 0];
    assert_eq!(embedded_image(&sink.bytes), PACKED);

    let temp = TempDir::new();
    let pdf = temp.file("composed.pdf");
    fs::write(&pdf, &sink.bytes).unwrap();
    tool(Command::new("qpdf").arg("--check").arg(&pdf));
    let extracted = temp.file("extracted");
    tool(Command::new("pdfimages").arg(&pdf).arg(&extracted));
    let pbm = fs::read(temp.file("extracted-000.pbm")).unwrap();
    assert_eq!(pnm_payload(&pbm, "P4", 9, 3), PACKED);

    // Poppler smooths a 1:1 image blit, so render it at ten device pixels
    // per image pixel and sample each block's centre.
    let poppler = temp.file("poppler");
    let dpi: f64 = RENDER_DPI.parse().unwrap();
    tool(
        Command::new("pdftoppm")
            .arg("-mono")
            .arg("-r")
            .arg((dpi * 10.0).to_string())
            .arg("-singlefile")
            .arg(&pdf)
            .arg(&poppler),
    );
    let pbm = fs::read(temp.file("poppler.pbm")).unwrap();
    let poppler = pnm_payload(&pbm, "P4", 90, 30);
    for y in 0..3 {
        for x in 0..9 {
            let (row, column) = (y * 10 + 5, x * 10 + 5);
            let black = poppler[row * 12 + column / 8] & (0x80 >> (column % 8)) != 0;
            assert_eq!(black, (x, y) == (0, 0), "Poppler pixel ({x},{y})");
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
            .arg(RENDER_DPI)
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
fn malformed_dib_palette_segments_and_page_geometry_are_rejected_before_image_output() {
    let built = document(Variant::C8, &[vec![type3(9, 2, 0x10)]]);
    let base = built.payloads[0][0] as usize;
    for relative in [
        0,           // DIB header length
        40,          // white palette entry
        48 + 4,      // page-information segment type
        48 + 11 + 3, // page-information width differs from DIB
    ] {
        let mut damaged = built.bytes.clone();
        damaged[base + relative] ^= 1;
        let (error, pdf) = run_error(damaged, ComposeOptions::default());
        assert!(
            no_image(&pdf),
            "failed preflight emitted an image at +{relative}"
        );
        assert_eq!(error.stage, ComposeStage::Headers, "{error}");
        assert_eq!((error.page, error.image), (Some(1), Some(1)));
        assert!(error.offset.is_some());
    }
}

#[test]
fn located_stage_errors_cover_each_checked_metadata_boundary() {
    let built = document(Variant::C8, &[vec![type3(9, 2, 0x10)]]);
    let base = built.payloads[0][0] as usize;
    let segment = segment_starts(&type3(9, 2, 0x10).payload);
    for (name, relative, value, expected) in [
        (
            "page flags",
            segment[0] + 11 + 16,
            0xff,
            Type3Stage::Profile,
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
        let (error, pdf) = run_error(bytes, ComposeOptions::default());
        assert!(no_image(&pdf), "{name} unexpectedly started an image");
        assert_eq!((error.page, error.image), (Some(1), Some(1)), "{name}");
        assert_eq!(stage(&error), Some(expected), "{name}: {error}");
    }

    let mut record = type3(9, 2, 0x10);
    record.payload.truncate(segment[4]); // omit the entire fifth segment
    let built = document(Variant::C8, &[vec![record]]);
    let (error, pdf) = run_error(built.bytes, ComposeOptions::default());
    assert!(no_image(&pdf));
    assert_eq!(stage(&error), Some(Type3Stage::Profile), "{error}");
    assert_eq!(error.stage, ComposeStage::Headers);
}

#[test]
fn row_width_boundaries_keep_zero_low_padding() {
    for width in [7, 8, 9, 31, 32, 33] {
        let built = document(Variant::C8, &[vec![type3(width, 3, 0x10)]]);
        let mut source = Source::new(built.bytes);
        let mut sink = Sink::default();
        let report = run(
            &mut source,
            &mut sink,
            ComposeOptions::default(),
            &Limits::default(),
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
        assert_eq!(report.type3_images, 1);
        validate_one_image_pdf(&sink.bytes, width, 3);
    }
}

#[test]
fn dib_bit_count_empty_span_and_nonpositive_dimensions_are_refused() {
    let built = document(Variant::HnA, &[vec![type3(9, 2, 0x10)]]);
    let mut corrupt = built.bytes.clone();
    corrupt[built.payloads[0][0] as usize + 14] = 8; // 8 bpp, not observed 1 bpp
    let (error, pdf) = run_error(corrupt, ComposeOptions::default());
    assert!(no_image(&pdf));
    assert_eq!((error.page, error.image), (Some(1), Some(1)));
    assert!(
        matches!(error.kind, ComposeErrorKind::Type3Dib(_)),
        "{error}"
    );
    assert_eq!(error.offset, Some(built.payloads[0][0] + 12));

    let empty = document(
        Variant::C8,
        &[vec![Image {
            kind: 3,
            payload: dib(9, 2).to_vec(),
            width: 9,
            height: 2,
        }]],
    );
    let (error, pdf) = run_error(empty.bytes, ComposeOptions::default());
    assert!(
        matches!(error.kind, ComposeErrorKind::Type3Dib(_)),
        "{error}"
    );
    assert!(no_image(&pdf));

    for dimension in [4, 8] {
        let mut bytes = built.bytes.clone();
        let at = built.payloads[0][0] as usize + dimension;
        bytes[at..at + 4].copy_from_slice(&0_i32.to_le_bytes());
        let (error, pdf) = run_error(bytes, ComposeOptions::default());
        assert!(
            matches!(error.kind, ComposeErrorKind::Type3Dib(_)),
            "{error}"
        );
        assert_eq!(error.stage, ComposeStage::Headers);
        assert!(no_image(&pdf));
    }
}

#[test]
fn symbol_limit_refuses_the_first_dictionary_before_image_output() {
    let built = document(Variant::C8, &[vec![type3(9, 2, 0x10)]]);
    let limits = Limits {
        max_symbols: 0,
        ..Limits::default()
    };
    let mut sink = Sink::default();
    let error = run(
        &mut Source::new(built.bytes),
        &mut sink,
        ComposeOptions::default(),
        &limits,
        &NeverCancel,
    )
    .unwrap_err();
    assert!(no_image(&sink.bytes));
    assert_eq!((error.page, error.image), (Some(1), Some(1)));
    assert_eq!(stage(&error), Some(Type3Stage::FirstDictionary), "{error}");
    assert!(
        error
            .to_string()
            .ends_with("export runs limit 0 exceeded by 1"),
        "{error}"
    );
}

#[test]
fn page_pixel_limit_refuses_before_image_output() {
    let built = document(Variant::C8, &[vec![type3(9, 2, 0x10)]]);
    let limits = Limits {
        max_image_pixels: 17,
        ..Limits::default()
    };
    let mut sink = Sink::default();
    let error = run(
        &mut Source::new(built.bytes),
        &mut sink,
        ComposeOptions::default(),
        &limits,
        &NeverCancel,
    )
    .unwrap_err();
    assert!(no_image(&sink.bytes));
    assert_eq!((error.page, error.image), (Some(1), Some(1)));
    assert_eq!(stage(&error), Some(Type3Stage::PageInfo), "{error}");
    assert!(
        error
            .to_string()
            .ends_with("page pixels limit 17 exceeded by 18"),
        "{error}"
    );
}

#[test]
fn generic_marker_and_pdf_sink_faults_propagate_without_success() {
    let built = document(Variant::C8, &[vec![type3(9, 3, 0x10)]]);
    let mut damaged = built.bytes.clone();
    let generic_tail = damaged.len() - 1;
    damaged[generic_tail] = 0xab; // not the MQ terminal marker 0xac
    let (error, pdf) = run_error(damaged, ComposeOptions::default());
    assert_eq!(stage(&error), Some(Type3Stage::GenericRegion), "{error}");
    assert!(!no_image(&pdf)); // the caller discards this partial stream

    let mut sink = Sink::default();
    run(
        &mut Source::new(built.bytes.clone()),
        &mut sink,
        ComposeOptions::default(),
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap();
    let writes = sink.writes;
    assert!(writes > 10);
    let mut saw_image = false;
    for fail_at in 1..=writes {
        let mut sink = Sink {
            fail_at: Some(fail_at),
            ..Sink::default()
        };
        let error = run(
            &mut Source::new(built.bytes.clone()),
            &mut sink,
            ComposeOptions::default(),
            &Limits::default(),
            &NeverCancel,
        )
        .unwrap_err();
        assert_eq!(error.variant, Some(Variant::C8));
        match (&error.kind, error.stage) {
            (ComposeErrorKind::Io(Error::Io(_)), ComposeStage::Pdf) => {}
            (
                ComposeErrorKind::Type3 {
                    stage: Type3Stage::PageCompose,
                    ..
                },
                _,
            ) => {}
            _ => panic!("sink failure at write {fail_at}: {error:?}"),
        }
        if error.image.is_some() {
            assert_eq!((error.page, error.image), (Some(1), Some(1)));
            saw_image = true;
        }
    }
    // A small image may remain entirely inside the compressor until its
    // stream closes, so its failures need not occur during PageCompose.
    assert!(saw_image);
}

#[test]
fn located_payload_read_and_decoder_input_failures_cover_each_stage() {
    let built = document(Variant::HnA, &[vec![type3(9, 3, 0x10)]]);
    let base = built.payloads[0][0];
    let segment = segment_starts(&type3(9, 3, 0x10).payload).map(|at| at as u64);
    // Preflight reads the headers through the source; the decoder then reads
    // the whole payload into memory once. Either failure keeps the image.
    let mut decode_failure = None;
    for visit in 1..16 {
        let mut source = Source::new(built.bytes.clone());
        source.fail_on_offset_visit = Some((base, visit));
        match run(
            &mut source,
            &mut Sink::default(),
            ComposeOptions::default(),
            &Limits::default(),
            &NeverCancel,
        ) {
            Err(error) => {
                assert_eq!((error.page, error.image), (Some(1), Some(1)), "{error}");
                if error.stage == ComposeStage::Decode {
                    decode_failure = Some(error);
                    break;
                }
            }
            Ok(_) => break,
        }
    }
    let error = decode_failure.expect("the payload read must fail during decode");
    assert!(
        matches!(error.kind, ComposeErrorKind::Io(Error::Io(_))),
        "{error}"
    );

    for (name, relative, expected) in [
        (
            "first dictionary body",
            segment[1] + 11 + 12,
            Type3Stage::FirstDictionary,
        ),
        (
            "second dictionary body",
            segment[2] + 12 + 12,
            Type3Stage::SecondDictionary,
        ),
        (
            "text body terminal",
            segment[4] - 2,
            Type3Stage::TextCompose,
        ),
    ] {
        let mut damaged = built.bytes.clone();
        let at = base as usize + relative as usize;
        damaged[at..at + 2].fill(0);
        let (error, _) = run_error(damaged, ComposeOptions::default());
        assert_eq!(stage(&error), Some(expected), "{name}: {error}");
    }
}

#[test]
fn strict_text_header_refuses_anomaly_but_named_opt_in_records_it() {
    let built = document(Variant::C8, &[vec![type3(9, 2, 0xa40c)]]);
    let (strict, pdf) = run_error(built.bytes.clone(), ComposeOptions::default());
    assert!(no_image(&pdf));
    assert_eq!((strict.page, strict.image), (Some(1), Some(1)));
    assert!(stage(&strict).is_some(), "{strict}");

    let mut sink = Sink::default();
    let mut anomalies = Anomalies::default();
    run_with(
        &mut Source::new(built.bytes),
        &mut sink,
        &mut anomalies,
        ComposeOptions {
            text_header_policy: TextHeaderPolicy::HnC8UnusedRefinementTemplate,
            ..Default::default()
        },
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap();
    assert_eq!(
        anomalies.0,
        [Some(TextHeaderAnomaly::UnusedRefinementTemplate)]
    );
    validate_one_image_pdf(&sink.bytes, 9, 2);
}

#[test]
fn limits_cancellation_and_io_faults_never_report_success() {
    let built = document(Variant::HnA, &[vec![type3(9, 2, 0x10)]]);
    let mut sink = Sink::default();
    let limits = Limits {
        max_input_bytes: built.bytes.len() as u64 - 1,
        ..Limits::default()
    };
    assert!(
        run(
            &mut Source::new(built.bytes.clone()),
            &mut sink,
            ComposeOptions::default(),
            &limits,
            &NeverCancel
        )
        .is_err()
    );
    assert!(no_image(&sink.bytes));

    for allowed in [0, 40] {
        let mut sink = Sink::default();
        let error = run(
            &mut Source::new(built.bytes.clone()),
            &mut sink,
            ComposeOptions::default(),
            &Limits::default(),
            &CancelAfter::new(allowed),
        )
        .unwrap_err();
        assert!(error.to_string().contains("cancelled"), "{error}");
    }

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
                ComposeOptions::default(),
                &Limits::default(),
                &NeverCancel
            )
            .is_err()
        );
        assert!(no_image(&sink.bytes));
    }

    let mut sink = Sink {
        fail_at: Some(1),
        ..Sink::default()
    };
    let error = run(
        &mut Source::new(built.bytes),
        &mut sink,
        ComposeOptions::default(),
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert!(
        matches!(error.kind, ComposeErrorKind::Io(Error::Io(_))),
        "{error}"
    );
    assert_eq!(error.stage, ComposeStage::Pdf);
}

#[test]
fn bilevel_pdf_rows_are_top_down_black_one_and_drop_low_padding() {
    let expected = [0x81, 0x80, 0x42, 0x00, 0x24, 0x80];
    let mut output = Sink::default();
    let limits = Limits::default();
    let report = (|| {
        let mut document = PdfDocument::new(&mut output, &limits, &NeverCancel)?;
        let mut image = document.begin_bilevel_image(BilevelImageSpec {
            pixel_width: 9,
            pixel_height: 3,
            row_stride: 2,
        })?;
        image.write_all(&expected)?;
        let image = image.finish()?;
        document.add_page(
            PageSpec {
                width_points: 9.0,
                height_points: 3.0,
            },
            &[image],
        )?;
        document.finish()
    })()
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
