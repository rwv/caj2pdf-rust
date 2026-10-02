// SPDX-License-Identifier: MIT

//! Original synthetic PGM/PPM pixels encoded as tiny baseline JPEGs at test
//! runtime, then selected from HN-A/HN-B/C8 into one-page PDFs.

mod common;

use caj2pdf_core::{
    Error, Limits, NeverCancel, RangedSource, SequentialSink,
    hnc8::{
        ErrorKind, JpegColor, Type2ImageSelection, Type2PdfError, Type2PdfErrorKind,
        Type2PdfOptions, Type2SelectedPdfReport, Variant, convert_type2_image_pdf,
    },
};
use common::CancelAfter;
use std::{
    error::Error as _,
    fs,
    future::Future,
    io,
    path::PathBuf,
    pin::pin,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
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

#[derive(Clone, Copy)]
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
    if matches!(layout, Layout::HnB) {
        bytes[0x88..0x8c].copy_from_slice(&0xc8_u32.to_le_bytes());
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

struct Source {
    bytes: Vec<u8>,
    max_read: usize,
    max_request: usize,
    payload_start: Option<u64>,
    payload_passes: usize,
    mutate_at_pass: Option<(usize, usize)>,
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
            mutate_at_pass: None,
            zero_at_pass: None,
            overreport_at_pass: None,
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
        self.max_request = self.max_request.max(destination.len());
        if self.payload_start == Some(offset) {
            self.payload_passes += 1;
            if let Some((pass, index)) = self.mutate_at_pass
                && self.payload_passes == pass
            {
                self.bytes[index] ^= 1;
            }
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

fn selection(page_number: u32, image_number: u32) -> Type2ImageSelection {
    Type2ImageSelection {
        page_number,
        image_number,
    }
}

fn options() -> Type2PdfOptions {
    Type2PdfOptions {
        pixels_per_inch: 72.0,
        ..Type2PdfOptions::default()
    }
}

fn run(
    source: &mut Source,
    sink: &mut Sink,
    selected: Type2ImageSelection,
    options: Type2PdfOptions,
    limits: &Limits,
    cancel: &impl caj2pdf_core::Cancellation,
) -> Result<Type2SelectedPdfReport, Type2PdfError> {
    ready(convert_type2_image_pdf(
        source, sink, selected, options, limits, cancel,
    ))
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

fn render_jpeg_and_pdf(jpeg: &[u8], pdf: &[u8], color: bool) -> (Vec<u8>, Vec<u8>) {
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
            "72",
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
fn original_asymmetric_pgm_ppm_pixels_render_in_their_direct_jpeg_orientation_and_color() {
    for channels in [1, 3] {
        let jpeg = jpeg_from_pnm(&pnm(channels));
        let built = container(
            Layout::HnA,
            &[vec![Record {
                kind: 1,
                payload: jpeg.clone(),
            }]],
        );
        let mut source = Source::new(built.bytes);
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
        let (direct, rendered) = render_jpeg_and_pdf(&jpeg, &sink.bytes, channels == 3);
        let worst = direct
            .iter()
            .zip(&rendered)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(
            worst <= 5,
            "direct JPEG and PDF rendered pixels differ by {worst}"
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
            assert!(upper_right[1] > upper_right[0] + 100 && upper_right[1] > upper_right[2] + 100);
            assert!(lower_left[2] > lower_left[0] + 100 && lower_left[2] > lower_left[1] + 100);
            assert!(lower_right[0] > 180 && lower_right[1] > 150 && lower_right[2] < 80);
        }
    }
}

#[test]
fn selected_first_middle_last_and_multirecord_pages_stream_exact_gray_and_color_jpegs() {
    let gray = jpeg_from_pnm(&pnm(1));
    let color = jpeg_from_pnm(&pnm(3));
    for layout in [Layout::HnA, Layout::HnB, Layout::C8] {
        let built = container(
            layout,
            &[
                vec![Record {
                    kind: 2,
                    payload: gray.clone(),
                }],
                vec![
                    Record {
                        kind: 2,
                        payload: gray.clone(),
                    },
                    Record {
                        kind: 1,
                        payload: vec![0x11],
                    },
                    Record {
                        kind: 2,
                        payload: color.clone(),
                    },
                    Record {
                        kind: 2,
                        payload: gray.clone(),
                    },
                ],
                vec![Record {
                    kind: 2,
                    payload: color.clone(),
                }],
            ],
        );
        for (selected, expected, rgb) in [
            (selection(1, 1), &gray, false),
            (selection(2, 3), &color, true),
            (selection(2, 4), &gray, false),
            (selection(3, 1), &color, true),
        ] {
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
                selected,
                options(),
                &limits,
                &NeverCancel,
            )
            .unwrap();
            assert_eq!(report.source_variant, layout.variant());
            assert_eq!(report.source_pages, 3);
            assert_eq!(report.image.page_number, selected.page_number);
            assert_eq!(report.image.image_number, selected.image_number);
            let page = selected.page_number as usize - 1;
            let image = selected.image_number as usize - 1;
            assert_eq!(
                report.image.descriptor_offset,
                built.descriptors[page][image]
            );
            assert_eq!(report.image.payload.offset, built.payloads[page][image]);
            assert_eq!(report.image.payload.length, expected.len() as u64);
            assert_eq!(report.jpeg.payload, report.image.payload);
            assert_eq!(
                (report.jpeg.width, report.jpeg.height, report.jpeg.precision),
                (16, 16, 8)
            );
            assert_eq!(
                report.jpeg.color,
                if rgb {
                    JpegColor::Ycbcr
                } else {
                    JpegColor::Gray
                }
            );
            assert_eq!(report.conversion.pages_converted, 1);
            assert_eq!(
                report.conversion.output_bytes_written,
                sink.bytes.len() as u64
            );
            assert!(report.conversion.input_bytes_read >= expected.len() as u64 * 2);
            assert_eq!(embedded_jpeg(&sink.bytes), expected);
            assert_eq!(
                sink.bytes
                    .windows(b"/Type /Page /Parent".len())
                    .filter(|w| *w == b"/Type /Page /Parent")
                    .count(),
                1
            );
            assert!(find(&sink.bytes, b"/MediaBox [0 0 16.000000 16.000000]").is_some());
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
fn default_300_ppi_scales_sixteen_pixels_to_3_84_points() {
    let built = container(
        Layout::C8,
        &[vec![Record {
            kind: 2,
            payload: jpeg_from_pnm(&pnm(1)),
        }]],
    );
    let mut source = Source::new(built.bytes);
    let mut sink = Sink::default();
    let report = run(
        &mut source,
        &mut sink,
        selection(1, 1),
        Type2PdfOptions::default(),
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap();
    assert_eq!(report.conversion.pages_converted, 1);
    assert!(find(&sink.bytes, b"/MediaBox [0 0 3.840000 3.840000]").is_some());
}

#[test]
fn selection_skips_bad_earlier_page_and_rejects_invalid_id_and_wrong_type() {
    let jpeg = jpeg_from_pnm(&pnm(1));
    let mut built = container(
        Layout::C8,
        &[
            vec![Record {
                kind: 2,
                payload: jpeg.clone(),
            }],
            vec![
                Record {
                    kind: 0,
                    payload: vec![0xaa],
                },
                Record {
                    kind: 2,
                    payload: jpeg,
                },
            ],
        ],
    );
    built.bytes[0x50 + 8..0x50 + 10].copy_from_slice(&(-1_i16).to_le_bytes());
    let mut source = Source::new(built.bytes.clone());
    let mut sink = Sink::default();
    let report = run(
        &mut source,
        &mut sink,
        selection(2, 2),
        options(),
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap();
    assert_eq!(
        (report.image.page_number, report.image.image_number),
        (2, 2)
    );
    for selected in [
        selection(0, 1),
        selection(1, 0),
        selection(3, 1),
        selection(2, 3),
    ] {
        let mut source = Source::new(built.bytes.clone());
        let mut sink = Sink::default();
        let error = run(
            &mut source,
            &mut sink,
            selected,
            options(),
            &Limits::default(),
            &NeverCancel,
        )
        .unwrap_err();
        assert!(matches!(
            error.kind,
            Type2PdfErrorKind::InvalidSelection(_) | Type2PdfErrorKind::Container(_)
        ));
        if selected.page_number != 0 && selected.image_number != 0 {
            assert_eq!(
                (error.page, error.image),
                (Some(selected.page_number), Some(selected.image_number))
            );
            assert!(error.offset.is_some());
        }
        assert!(error.to_string().contains("conversion"));
        assert!(sink.bytes.is_empty());
    }
    let mut source = Source::new(built.bytes);
    let mut sink = Sink::default();
    let error = run(
        &mut source,
        &mut sink,
        selection(2, 1),
        options(),
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert!(matches!(
        error.kind,
        Type2PdfErrorKind::UnsupportedImageType(0)
    ));
    assert!(
        error
            .to_string()
            .contains("unsupported image record type 0")
    );
    assert!(error.source().is_none());
    assert_eq!(
        (error.page, error.image, error.offset),
        (Some(2), Some(1), Some(built.descriptors[1][0]))
    );
    assert!(sink.bytes.is_empty());
}

#[test]
fn malformed_descriptor_and_jpeg_errors_preserve_absolute_location() {
    let jpeg = jpeg_from_pnm(&pnm(3));
    let built = container(
        Layout::HnB,
        &[vec![
            Record {
                kind: 2,
                payload: jpeg.clone(),
            },
            Record {
                kind: 2,
                payload: jpeg.clone(),
            },
        ]],
    );
    let mut corrupt = built.bytes.clone();
    let descriptor = built.descriptors[0][1] as usize;
    corrupt[descriptor + 4..descriptor + 8].copy_from_slice(&(-1_i32).to_le_bytes());
    let mut source = Source::new(corrupt);
    let mut sink = Sink::default();
    let error = run(
        &mut source,
        &mut sink,
        selection(1, 2),
        options(),
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert!(matches!(error.kind, Type2PdfErrorKind::Container(_)));
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
        selection(1, 1),
        options(),
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert!(
        matches!(&error.kind, Type2PdfErrorKind::Jpeg(inner) if matches!(inner.kind, ErrorKind::Malformed { field: "JPEG SOI", .. }))
    );
    assert_eq!(
        (error.page, error.image, error.offset),
        (Some(1), Some(1), Some(payload as u64))
    );
    assert!(error.source().is_some());
    assert!(sink.bytes.is_empty());
}

#[test]
fn source_mutation_during_pdf_copy_is_detected_even_when_length_is_unchanged() {
    let jpeg = jpeg_from_pnm(&pnm(3));
    let built = container(
        Layout::HnA,
        &[vec![Record {
            kind: 2,
            payload: jpeg,
        }]],
    );
    let payload = built.payloads[0][0];
    let mut source = Source::new(built.bytes);
    source.payload_start = Some(payload);
    source.mutate_at_pass = Some((2, payload as usize + 12)); // JFIF field, same length
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
    assert!(matches!(error.kind, Type2PdfErrorKind::SourceChanged));
    assert!(
        error
            .to_string()
            .contains("changed between preflight and PDF copy")
    );
    assert!(error.source().is_none());
    assert_eq!(
        (error.page, error.image, error.offset),
        (Some(1), Some(1), Some(payload))
    );
    assert_eq!(source.payload_passes, 2);
    assert!(!sink.bytes.is_empty()); // Must be discarded by the caller.
}

#[test]
fn short_reads_sink_faults_cancellation_and_limits_are_typed() {
    let jpeg = jpeg_from_pnm(&pnm(1));
    let built = container(
        Layout::C8,
        &[vec![Record {
            kind: 2,
            payload: jpeg,
        }]],
    );
    let payload = built.payloads[0][0];
    let mut source = Source::new(built.bytes.clone());
    source.payload_start = Some(payload);
    source.zero_at_pass = Some(2);
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
    assert!(matches!(
        error.kind,
        Type2PdfErrorKind::Pdf(Error::TruncatedInput { .. })
    ));
    assert!(error.to_string().contains("PDF output"));
    assert!(error.source().is_some());
    assert_eq!(
        (error.page, error.image, error.offset),
        (Some(1), Some(1), Some(payload))
    );

    let mut source = Source::new(built.bytes.clone());
    let mut sink = Sink {
        fail_at: Some(3),
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
    assert!(matches!(error.kind, Type2PdfErrorKind::Pdf(Error::Io(_))));
    assert_eq!((error.page, error.image), (Some(1), Some(1)));

    let mut source = Source::new(built.bytes.clone());
    let mut sink = Sink::default();
    let error = run(
        &mut source,
        &mut sink,
        selection(1, 1),
        options(),
        &Limits::default(),
        &CancelAfter::new(0),
    )
    .unwrap_err();
    assert!(
        matches!(&error.kind, Type2PdfErrorKind::Container(inner) if matches!(inner.kind, ErrorKind::Cancelled))
    );
    assert!(error.to_string().contains("cancelled"));
    assert!(error.source().is_some());

    let mut source = Source::new(built.bytes.clone());
    let mut sink = Sink::default();
    let mut opts = options();
    opts.jpeg.max_payload_bytes = 1;
    let error = run(
        &mut source,
        &mut sink,
        selection(1, 1),
        opts,
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert!(
        matches!(&error.kind, Type2PdfErrorKind::Jpeg(inner) if matches!(inner.kind, ErrorKind::LimitExceeded { .. }))
    );
    assert!(error.to_string().contains("limit"));

    let mut source = Source::new(built.bytes.clone());
    let mut sink = Sink::default();
    let limits = Limits {
        max_output_bytes: 100,
        ..Limits::default()
    };
    let error = run(
        &mut source,
        &mut sink,
        selection(1, 1),
        options(),
        &limits,
        &NeverCancel,
    )
    .unwrap_err();
    assert!(matches!(
        error.kind,
        Type2PdfErrorKind::Pdf(Error::LimitExceeded { .. })
    ));

    let mut source = Source::new(built.bytes.clone());
    let mut sink = Sink::default();
    let mut opts = options();
    opts.pixels_per_inch = 0.0;
    let error = run(
        &mut source,
        &mut sink,
        selection(1, 1),
        opts,
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert!(matches!(error.kind, Type2PdfErrorKind::InvalidOptions(_)));
    assert!(error.to_string().contains("invalid options"));
    assert!(error.source().is_none());
    assert!(sink.bytes.is_empty());
    assert_eq!(source.max_request, 0);

    let mut source = Source::new(built.bytes.clone());
    let mut sink = Sink::default();
    let limits = Limits {
        max_input_bytes: 1,
        ..Limits::default()
    };
    let error = run(
        &mut source,
        &mut sink,
        selection(1, 1),
        options(),
        &limits,
        &NeverCancel,
    )
    .unwrap_err();
    assert!(
        matches!(&error.kind, Type2PdfErrorKind::Container(inner) if matches!(inner.kind, ErrorKind::LimitExceeded { .. }))
    );

    let mut source = Source::new(built.bytes);
    source.payload_start = Some(payload);
    source.overreport_at_pass = Some(1);
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
        matches!(&error.kind, Type2PdfErrorKind::Jpeg(inner) if matches!(inner.kind, ErrorKind::Source { .. }))
    );
    assert!(sink.bytes.is_empty());
}
