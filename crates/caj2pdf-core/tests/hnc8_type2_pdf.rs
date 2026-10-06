// SPDX-License-Identifier: MIT

//! Original synthetic PGM/PPM pixels encoded as tiny baseline JPEGs at test
//! runtime, then composed from HN-A/HN-B/C8 documents into PDF pages.

mod common;

use caj2pdf_core::hnc8::convert_source_pages_pdf as compose;
use caj2pdf_core::{
    Cancellation, Error, Limits, NeverCancel, RangedSource,
    hnc8::{
        ComposeError, ComposeErrorKind, ComposeOptions, ComposeReport, ComposeStage, ErrorKind,
        Variant,
    },
};
use common::{
    CancelAfter,
    hnc8_document::{Image, RENDER_DPI, document},
};
use std::io::Write;
use std::{
    error::Error as _,
    fs, io,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

fn segment(marker: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![0xff, marker];
    out.extend_from_slice(&u16::try_from(body.len() + 2).unwrap().to_be_bytes());
    out.extend_from_slice(body);
    out
}

/// Four nonidentical, asymmetric 8x8 tiles in an original ASCII PGM or PPM.
fn pnm(channels: usize) -> Vec<u8> {
    let mut out = format!("P{}\n16 16\n255\n", if channels == 1 { 2 } else { 3 }).into_bytes();
    let gray = [15, 75, 190, 245];
    let rgb = [[245, 15, 15], [15, 210, 35], [25, 45, 225], [240, 210, 20]];
    for y in 0..16 {
        for x in 0..16 {
            let tile = (y / 8) * 2 + x / 8;
            let colors: &[u8] = if channels == 1 {
                &gray[tile..tile + 1]
            } else {
                &rgb[tile]
            };
            for sample in colors {
                out.extend_from_slice(format!("{sample} ").as_bytes());
            }
        }
        out.push(b'\n');
    }
    out
}

struct Bits {
    bytes: Vec<u8>,
    current: u8,
    used: u8,
}

impl Bits {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            current: 0,
            used: 0,
        }
    }

    fn put(&mut self, value: u16, length: u8) {
        for shift in (0..length).rev() {
            self.current = (self.current << 1) | ((value >> shift) as u8 & 1);
            self.used += 1;
            if self.used == 8 {
                self.bytes.push(self.current);
                if self.current == 0xff {
                    self.bytes.push(0);
                }
                self.current = 0;
                self.used = 0;
            }
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.used != 0 {
            self.put((1_u16 << (8 - self.used)) - 1, 8 - self.used);
        }
        self.bytes
    }
}

fn category(value: i32) -> u8 {
    if value == 0 {
        0
    } else {
        (32 - value.unsigned_abs().leading_zeros()) as u8
    }
}

fn amplitude(value: i32, bits: u8) -> u16 {
    if value >= 0 {
        value as u16
    } else {
        (value + (1 << bits) - 1) as u16
    }
}

/// Test-only JPEG encoder for constant 8x8 tiles. Custom canonical Huffman
/// tables use four-bit codes 0..11 for DC categories and a one-bit AC EOB.
/// The encoder is independent of production and external converter code.
fn jpeg_from_pnm(source: &[u8]) -> Vec<u8> {
    let text = std::str::from_utf8(source).unwrap();
    let mut words = text.split_ascii_whitespace();
    let channels = match words.next().unwrap() {
        "P2" => 1,
        "P3" => 3,
        _ => panic!("PGM/PPM"),
    };
    assert_eq!(words.next(), Some("16"));
    assert_eq!(words.next(), Some("16"));
    assert_eq!(words.next(), Some("255"));
    let pixels: Vec<u8> = words.map(|word| word.parse().unwrap()).collect();
    assert_eq!(pixels.len(), 16 * 16 * channels);

    let mut out = vec![0xff, 0xd8];
    out.extend(segment(0xe0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0"));
    out.extend(segment(0xfe, b"original PNM tiles"));
    out.extend(segment(0xdb, &[&[0_u8][..], &[1_u8; 64][..]].concat()));
    let mut frame = vec![8, 0, 16, 0, 16, channels as u8];
    for id in 1..=channels {
        frame.extend([id as u8, 0x11, 0]);
    }
    out.extend(segment(0xc0, &frame));
    let mut dc_table = vec![0x00];
    dc_table.extend([0_u8, 0, 0, 12]);
    dc_table.extend([0_u8; 12]);
    dc_table.extend(0_u8..12);
    out.extend(segment(0xc4, &dc_table));
    let mut ac_table = vec![0x10, 1];
    ac_table.extend([0_u8; 15]);
    ac_table.push(0);
    out.extend(segment(0xc4, &ac_table));
    let mut scan = vec![channels as u8];
    for id in 1..=channels {
        scan.extend([id as u8, 0]);
    }
    scan.extend([0, 63, 0]);
    out.extend(segment(0xda, &scan));

    let mut bits = Bits::new();
    let mut previous = [0_i32; 3];
    for tile_y in 0..2 {
        for tile_x in 0..2 {
            let pixel = ((tile_y * 8) * 16 + tile_x * 8) * channels;
            let samples = if channels == 1 {
                [pixels[pixel] as i32, 0, 0]
            } else {
                let r = pixels[pixel] as f64;
                let g = pixels[pixel + 1] as f64;
                let b = pixels[pixel + 2] as f64;
                [
                    (0.299 * r + 0.587 * g + 0.114 * b).round() as i32,
                    (128.0 - 0.168736 * r - 0.331264 * g + 0.5 * b).round() as i32,
                    (128.0 + 0.5 * r - 0.418688 * g - 0.081312 * b).round() as i32,
                ]
            };
            for channel in 0..channels {
                // Constant samples have only an exact DC coefficient when
                // quantization is one: 8 * (sample - 128).
                let dc = 8 * (samples[channel] - 128);
                let difference = dc - previous[channel];
                previous[channel] = dc;
                let width = category(difference);
                assert!(width <= 11);
                bits.put(u16::from(width), 4);
                if width != 0 {
                    bits.put(amplitude(difference, width), width);
                }
                bits.put(0, 1); // AC end-of-block
            }
        }
    }
    out.extend(bits.finish());
    out.extend([0xff, 0xd9]);
    out
}

fn jpeg_image(kind: i32, payload: Vec<u8>) -> Image {
    Image {
        kind,
        payload,
        width: 16,
        height: 16,
    }
}

struct Source {
    bytes: Vec<u8>,
    max_read: usize,
    max_request: usize,
    payload_start: Option<u64>,
    payload_passes: usize,
    zero_at_pass: Option<usize>,
    overreport_at_pass: Option<usize>,
}

impl Source {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            max_read: usize::MAX,
            max_request: 0,
            payload_start: None,
            payload_passes: 0,
            zero_at_pass: None,
            overreport_at_pass: None,
        }
    }
}

impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> caj2pdf_core::Result<usize> {
        self.max_request = self.max_request.max(destination.len());
        if self.payload_start == Some(offset) {
            self.payload_passes += 1;
            if self.zero_at_pass == Some(self.payload_passes) {
                return Ok(0);
            }
            if self.overreport_at_pass == Some(self.payload_passes) {
                return Ok(destination.len() + 1);
            }
        }
        let start = usize::try_from(offset).unwrap();
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
}

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.writes += 1;
        if self.fail_at == Some(self.writes) {
            return Err(io::Error::other("injected sink failure"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn run(
    source: &mut Source,
    sink: &mut Sink,
    options: ComposeOptions,
    limits: &Limits,
    cancel: &impl Cancellation,
) -> Result<ComposeReport, ComposeError> {
    compose(source, sink, None, &mut (), options, limits, cancel)
}

fn find(bytes: &[u8], needle: &[u8]) -> Option<usize> {
    bytes.windows(needle.len()).position(|part| part == needle)
}

fn embedded_jpeg(pdf: &[u8]) -> &[u8] {
    let image = find(pdf, b"/Subtype /Image").unwrap();
    let stream = image + find(&pdf[image..], b"stream\n").unwrap() + b"stream\n".len();
    let end = stream + find(&pdf[stream..], b"\nendstream").unwrap();
    &pdf[stream..end]
}

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("caj2pdf-type2-test-{}-{id}", std::process::id()));
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
        .expect("independent JPEG/PDF test tool is installed");
    assert!(
        output.status.success(),
        "tool failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.len() <= 1024 * 1024);
    assert!(output.stderr.len() <= 1024 * 1024);
}

fn pnm_pixels(bytes: &[u8]) -> (usize, Vec<u8>) {
    let mut at = 0;
    let mut token = || {
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
    let channels = match token().as_str() {
        "P5" => 1,
        "P6" => 3,
        other => panic!("unexpected PNM magic {other}"),
    };
    assert_eq!(token(), "16");
    assert_eq!(token(), "16");
    assert_eq!(token(), "255");
    assert!(bytes[at].is_ascii_whitespace());
    at += 1;
    if bytes[at - 1] == b'\r' && bytes.get(at) == Some(&b'\n') {
        at += 1;
    }
    let pixels = bytes[at..].to_vec();
    assert_eq!(pixels.len(), 16 * 16 * channels);
    (channels, pixels)
}

fn render_jpeg_and_pdf(jpeg: &[u8], pdf: &[u8], color: bool, dpi: &str) -> (Vec<u8>, Vec<u8>) {
    let temporary = TempDir::new();
    let jpeg_path = temporary.file("selected.jpg");
    let pdf_path = temporary.file("selected.pdf");
    let raster_path = temporary.file("render.pnm");
    fs::write(&jpeg_path, jpeg).unwrap();
    fs::write(&pdf_path, pdf).unwrap();
    let direct = Command::new("djpeg")
        .arg("-pnm")
        .arg(&jpeg_path)
        .output()
        .expect("djpeg is installed for native test");
    assert!(
        direct.status.success(),
        "djpeg: {}",
        String::from_utf8_lossy(&direct.stderr)
    );
    assert!(direct.stdout.len() <= 1024 * 1024);
    assert!(direct.stderr.len() <= 1024 * 1024);
    let direct = pnm_pixels(&direct.stdout);
    assert_eq!(direct.0, if color { 3 } else { 1 });

    let mut command = Command::new("mutool");
    command
        .args([
            "draw",
            "-q",
            "-F",
            "pnm",
            "-c",
            if color { "rgb" } else { "gray" },
            "-r",
            dpi,
            "-A",
            "0",
            "-o",
        ])
        .arg(&raster_path)
        .arg(&pdf_path)
        .arg("1");
    tool(&mut command);
    let rendered = pnm_pixels(&fs::read(&raster_path).unwrap());
    assert_eq!(rendered.0, direct.0);
    (direct.1, rendered.1)
}

#[test]
fn original_asymmetric_pgm_ppm_pixels_render_flipped_by_the_measured_matrix_in_their_color() {
    // Type 1 is admitted on HN-A/C8 pages; HN-B admits type 2 only.
    for (layout, kind) in [(Variant::HnA, 1), (Variant::HnB, 2)] {
        for channels in [1, 3] {
            let jpeg = jpeg_from_pnm(&pnm(channels));
            let built = document(layout, &[vec![jpeg_image(kind, jpeg.clone())]]);
            let mut source = Source::new(built.bytes);
            let mut sink = Sink::default();
            let report = run(
                &mut source,
                &mut sink,
                ComposeOptions::default(),
                &Limits::default(),
                &NeverCancel,
            )
            .unwrap();
            assert_eq!(report.jpeg_images, 1);
            // HN-B pages are the JPEG at 300 pixels per inch; HN-A/C8 pages
            // use the declared extents, one device pixel per image pixel.
            let dpi = if layout == Variant::HnB {
                "300"
            } else {
                RENDER_DPI
            };
            let (direct, rendered) = render_jpeg_and_pdf(&jpeg, &sink.bytes, channels == 3, dpi);
            // The composer draws every JPEG with the measured negative-height
            // matrix, so its first coded row is the page's bottom row.
            let row = 16 * channels;
            let flipped: Vec<u8> = direct.rchunks(row).flatten().copied().collect();
            let worst = flipped
                .iter()
                .zip(&rendered)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            assert!(
                worst <= 5,
                "{layout:?}: flipped JPEG and PDF rendered pixels differ by {worst}"
            );
            if channels == 1 {
                for (x, y, expected) in [(0, 0, 15), (15, 0, 75), (0, 15, 190), (15, 15, 245)] {
                    assert!(direct[y * 16 + x].abs_diff(expected) <= 5);
                }
            } else {
                let pixel = |x: usize, y: usize| &direct[(y * 16 + x) * 3..(y * 16 + x) * 3 + 3];
                let upper_left = pixel(0, 0);
                let upper_right = pixel(15, 0);
                let lower_left = pixel(0, 15);
                let lower_right = pixel(15, 15);
                assert!(upper_left[0] > upper_left[1] + 100 && upper_left[0] > upper_left[2] + 100);
                assert!(
                    upper_right[1] > upper_right[0] + 100 && upper_right[1] > upper_right[2] + 100
                );
                assert!(lower_left[2] > lower_left[0] + 100 && lower_left[2] > lower_left[1] + 100);
                assert!(lower_right[0] > 180 && lower_right[1] > 150 && lower_right[2] < 80);
            }
        }
    }
}

#[test]
fn every_layout_streams_the_exact_gray_and_color_jpeg_with_short_reads() {
    let gray = jpeg_from_pnm(&pnm(1));
    let color = jpeg_from_pnm(&pnm(3));
    for layout in [Variant::HnA, Variant::HnB, Variant::C8] {
        for (expected, rgb) in [(&gray, false), (&color, true)] {
            let built = document(layout, &[vec![jpeg_image(2, expected.clone())]]);
            let mut source = Source::new(built.bytes.clone());
            source.max_read = 3;
            let mut sink = Sink::default();
            let limits = Limits {
                io_chunk_bytes: 11,
                ..Limits::default()
            };
            let report = run(
                &mut source,
                &mut sink,
                ComposeOptions::default(),
                &limits,
                &NeverCancel,
            )
            .unwrap();
            assert_eq!(report.source_variant, layout);
            assert_eq!((report.source_pages, report.output_pages), (1, 1));
            assert_eq!(report.jpeg_images, 1);
            assert_eq!(report.conversion.pages_converted, 1);
            assert_eq!(
                report.conversion.output_bytes_written,
                sink.bytes.len() as u64
            );
            assert!(report.conversion.input_bytes_read >= expected.len() as u64 * 2);
            assert_eq!(embedded_jpeg(&sink.bytes), expected.as_slice());
            assert_eq!(
                sink.bytes
                    .windows(b"/Type /Page /Parent".len())
                    .filter(|w| *w == b"/Type /Page /Parent")
                    .count(),
                1
            );
            assert!(find(&sink.bytes, b"/Width 16\n/Height 16\n").is_some());
            assert!(find(&sink.bytes, b"/BitsPerComponent 8\n/Filter /DCTDecode\n").is_some());
            if rgb {
                assert!(find(&sink.bytes, b"/ColorSpace /DeviceRGB").is_some());
                assert!(find(&sink.bytes, b"/DecodeParms << /ColorTransform 1 >>").is_some());
            } else {
                assert!(find(&sink.bytes, b"/ColorSpace /DeviceGray").is_some());
                assert!(find(&sink.bytes, b"/DecodeParms").is_none());
            }
            assert!(source.max_request <= limits.io_chunk_bytes);
        }
    }
}

#[test]
fn malformed_descriptor_and_jpeg_errors_preserve_absolute_location() {
    let jpeg = jpeg_from_pnm(&pnm(3));
    let built = document(
        Variant::HnA,
        &[vec![jpeg_image(2, jpeg.clone()), jpeg_image(2, jpeg)]],
    );
    let mut corrupt = built.bytes.clone();
    let descriptor = built.descriptors[0][1] as usize;
    corrupt[descriptor + 4..descriptor + 8].copy_from_slice(&(-1_i32).to_le_bytes());
    let mut source = Source::new(corrupt);
    let mut sink = Sink::default();
    let error = run(
        &mut source,
        &mut sink,
        ComposeOptions::default(),
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert!(
        matches!(error.kind, ComposeErrorKind::Container(_)),
        "{error}"
    );
    assert_eq!((error.page, error.image), (Some(1), Some(2)));
    assert!(error.offset.is_some());

    let mut corrupt = built.bytes;
    let payload = built.payloads[0][0] as usize;
    corrupt[payload + 1] = 0xd9;
    let mut source = Source::new(corrupt);
    let mut sink = Sink::default();
    let error = run(
        &mut source,
        &mut sink,
        ComposeOptions::default(),
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert!(
        matches!(&error.kind, ComposeErrorKind::Jpeg(inner) if matches!(inner.kind, ErrorKind::Malformed { field: "JPEG SOI", .. })),
        "{error}"
    );
    assert_eq!(error.stage, ComposeStage::Headers);
    assert_eq!(
        (error.page, error.image, error.offset),
        (Some(1), Some(1), Some(payload as u64))
    );
    assert!(error.source().is_some());
    assert!(find(&sink.bytes, b"/Subtype /Image").is_none());
}

#[test]
fn jpeg_payload_is_read_once_for_markers_and_once_for_copy() {
    for layout in [Variant::HnA, Variant::HnB] {
        let built = document(layout, &[vec![jpeg_image(2, jpeg_from_pnm(&pnm(3)))]]);
        let payload = built.payloads[0][0];
        let mut source = Source::new(built.bytes);
        source.payload_start = Some(payload);
        let mut sink = Sink::default();
        run(
            &mut source,
            &mut sink,
            ComposeOptions::default(),
            &Limits::default(),
            &NeverCancel,
        )
        .unwrap();
        assert_eq!(source.payload_passes, 2, "{layout:?}");
    }
}

#[test]
fn short_reads_sink_faults_cancellation_and_limits_are_typed() {
    let jpeg = jpeg_from_pnm(&pnm(1));
    let built = document(Variant::C8, &[vec![jpeg_image(2, jpeg)]]);
    let payload = built.payloads[0][0];
    let run_with = |source: &mut Source, sink: &mut Sink, options, limits: &Limits| {
        run(source, sink, options, limits, &NeverCancel).unwrap_err()
    };

    // The marker walk succeeds; the PDF copy pass then reads nothing.
    let mut source = Source::new(built.bytes.clone());
    source.payload_start = Some(payload);
    source.zero_at_pass = Some(2);
    let error = run_with(
        &mut source,
        &mut Sink::default(),
        ComposeOptions::default(),
        &Limits::default(),
    );
    assert!(
        matches!(
            error.kind,
            ComposeErrorKind::Io(Error::TruncatedInput { .. })
        ),
        "{error}"
    );
    assert_eq!(error.stage, ComposeStage::Pdf);
    assert!(error.source().is_some());
    assert_eq!(
        (error.page, error.image, error.offset),
        (Some(1), Some(1), Some(payload))
    );

    let mut sink = Sink {
        fail_at: Some(3),
        ..Sink::default()
    };
    let error = run_with(
        &mut Source::new(built.bytes.clone()),
        &mut sink,
        ComposeOptions::default(),
        &Limits::default(),
    );
    assert!(
        matches!(error.kind, ComposeErrorKind::Io(Error::Io(_))),
        "{error}"
    );
    assert_eq!(error.stage, ComposeStage::Pdf);

    let error = run(
        &mut Source::new(built.bytes.clone()),
        &mut Sink::default(),
        ComposeOptions::default(),
        &Limits::default(),
        &CancelAfter::new(0),
    )
    .unwrap_err();
    assert!(error.to_string().contains("cancelled"), "{error}");
    assert!(error.source().is_some());

    let limits = Limits {
        max_output_bytes: 100,
        ..Limits::default()
    };
    let error = run_with(
        &mut Source::new(built.bytes.clone()),
        &mut Sink::default(),
        ComposeOptions::default(),
        &limits,
    );
    assert!(
        matches!(
            error.kind,
            ComposeErrorKind::Io(Error::LimitExceeded { .. })
        ),
        "{error}"
    );

    let limits = Limits {
        max_input_bytes: 1,
        ..Limits::default()
    };
    let error = run_with(
        &mut Source::new(built.bytes.clone()),
        &mut Sink::default(),
        ComposeOptions::default(),
        &limits,
    );
    assert!(
        matches!(&error.kind, ComposeErrorKind::Container(inner) if matches!(inner.kind, ErrorKind::LimitExceeded { .. })),
        "{error}"
    );

    let mut source = Source::new(built.bytes);
    source.payload_start = Some(payload);
    source.overreport_at_pass = Some(1);
    let mut sink = Sink::default();
    let error = run_with(
        &mut source,
        &mut sink,
        ComposeOptions::default(),
        &Limits::default(),
    );
    assert!(
        matches!(&error.kind, ComposeErrorKind::Jpeg(inner) if matches!(inner.kind, ErrorKind::Source { .. })),
        "{error}"
    );
    assert!(find(&sink.bytes, b"/Subtype /Image").is_none());
}
