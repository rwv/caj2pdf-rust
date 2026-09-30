// SPDX-License-Identifier: MIT

//! Original runtime-only image fixtures checked by independent PDF readers.
//! No corpus, reference converter, stored JPEG or rendered fixture is used.

use caj2pdf_core::{
    Error, Limits, NeverCancel, RangedSource, Result, SequentialSink,
    native::WriteSink,
    pdf::{BilevelImageSpec, ImageEncoding, ImagePlacement, ImageSpec, PageSpec, PdfDocument},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    future::Future,
    path::{Path, PathBuf},
    pin::pin,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
    task::{Context, Poll, Waker},
};

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);
const MAX_TOOL_OUTPUT: usize = 1024 * 1024;

fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("native test adapters must finish immediately"),
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "caj2pdf-placement-render-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create synthetic PDF test directory");
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

fn tool(command: &mut Command, name: &str) -> Output {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("{name} is mandatory for these tests: {error}"));
    assert!(output.stdout.len() <= MAX_TOOL_OUTPUT, "{name} stdout cap");
    assert!(output.stderr.len() <= MAX_TOOL_OUTPUT, "{name} stderr cap");
    assert!(
        output.status.success(),
        "{name} failed with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// Prefix/suffix bytes are excluded by the requested range. Short reads force
/// streaming; sealing after image emission makes any placement reread fail.
struct Source {
    bytes: Vec<u8>,
    read_bytes: u64,
    max_request: usize,
    sealed: bool,
}

impl Source {
    fn new(image: &[u8]) -> Self {
        let mut bytes = b"MIT!".to_vec();
        bytes.extend_from_slice(image);
        bytes.extend_from_slice(b"tail");
        Self {
            bytes,
            read_bytes: 0,
            max_request: 0,
            sealed: false,
        }
    }
}

impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        assert!(!self.sealed, "placing a completed image reread its source");
        self.max_request = self.max_request.max(destination.len());
        let offset = usize::try_from(offset).map_err(|_| Error::InvalidInput {
            reason: "synthetic source offset exceeds usize",
        })?;
        let remaining = self.bytes.len().saturating_sub(offset);
        let count = remaining.min(destination.len()).min(3);
        destination[..count].copy_from_slice(&self.bytes[offset..offset + count]);
        self.read_bytes += count as u64;
        Ok(count)
    }
}

fn page(width: f64, height: f64) -> PageSpec {
    PageSpec {
        width_points: width,
        height_points: height,
    }
}

fn spec(width: u32, height: u32, encoding: ImageEncoding) -> ImageSpec {
    ImageSpec {
        pixel_width: width,
        pixel_height: height,
        encoding,
    }
}

#[derive(Debug)]
struct Raster {
    width: usize,
    height: usize,
    channels: usize,
    pixels: Vec<u8>,
}

impl Raster {
    fn parse(bytes: &[u8]) -> Self {
        let mut at = 0;
        let mut token = || {
            loop {
                while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
                    at += 1;
                }
                if bytes.get(at) != Some(&b'#') {
                    break;
                }
                while bytes.get(at).is_some_and(|byte| *byte != b'\n') {
                    at += 1;
                }
            }
            let start = at;
            while bytes
                .get(at)
                .is_some_and(|byte| !byte.is_ascii_whitespace())
            {
                at += 1;
            }
            std::str::from_utf8(&bytes[start..at])
                .expect("PNM header is ASCII")
                .to_owned()
        };
        let channels = match token().as_str() {
            "P5" => 1,
            "P6" => 3,
            magic => panic!("unexpected PNM magic {magic}"),
        };
        let width = token().parse::<usize>().expect("PNM width");
        let height = token().parse::<usize>().expect("PNM height");
        assert_eq!(token(), "255");
        assert!(bytes[at].is_ascii_whitespace());
        at += 1;
        if bytes[at - 1] == b'\r' && bytes.get(at) == Some(&b'\n') {
            at += 1;
        }
        assert_eq!(bytes.len() - at, width * height * channels);
        Self {
            width,
            height,
            channels,
            pixels: bytes[at..].to_vec(),
        }
    }

    fn pixel(&self, x: usize, y: usize) -> [u8; 3] {
        assert!(x < self.width && y < self.height);
        let at = (y * self.width + x) * self.channels;
        if self.channels == 1 {
            [self.pixels[at]; 3]
        } else {
            self.pixels[at..at + 3].try_into().unwrap()
        }
    }

    fn point(&self, x: f64, y: f64, width: u32, height: u32) -> [u8; 3] {
        let scale = self.width as f64 / f64::from(width);
        assert_eq!(self.height as f64, f64::from(height) * scale);
        self.pixel(
            (x * scale).floor() as usize,
            ((f64::from(height) - y) * scale).floor() as usize,
        )
    }
}

fn close_pixel(actual: [u8; 3], expected: [u8; 3], tolerance: u8, label: &str) {
    for (channel, (&actual, &expected)) in actual.iter().zip(&expected).enumerate() {
        assert!(
            actual.abs_diff(expected) <= tolerance,
            "{label} channel {channel}: {actual} != {expected} (+/-{tolerance})"
        );
    }
}

/// Both independent readers are mandatory. Poppler is sampled far from image
/// edges at 10 device pixels per point, avoiding its 1:1 edge interpolation.
fn check_renderers(
    temporary: &TempDir,
    pdf: &Path,
    page_number: usize,
    width: u32,
    height: u32,
    mut check: impl FnMut(&str, &Raster),
) {
    let mupdf = temporary.file(&format!("mupdf-{page_number}.pnm"));
    tool(
        Command::new("mutool")
            .args([
                "draw", "-q", "-F", "pnm", "-c", "rgb", "-r", "72", "-A", "0", "-o",
            ])
            .arg(&mupdf)
            .arg(pdf)
            .arg(page_number.to_string()),
        "MuPDF",
    );
    let raster = Raster::parse(&fs::read(mupdf).unwrap());
    assert_eq!(
        (raster.width, raster.height, raster.channels),
        (width as usize, height as usize, 3)
    );
    check("MuPDF", &raster);

    let poppler = temporary.file(&format!("poppler-{page_number}"));
    tool(
        Command::new("pdftoppm")
            .args([
                "-r",
                "720",
                "-singlefile",
                "-aa",
                "no",
                "-aaVector",
                "no",
                "-f",
            ])
            .arg(page_number.to_string())
            .arg("-l")
            .arg(page_number.to_string())
            .arg(pdf)
            .arg(&poppler),
        "Poppler",
    );
    let raster = Raster::parse(&fs::read(poppler.with_extension("ppm")).unwrap());
    assert_eq!(
        (raster.width, raster.height, raster.channels),
        (width as usize * 10, height as usize * 10, 3)
    );
    check("Poppler", &raster);
}

struct ExpectedImage<'a> {
    bytes: &'a [u8],
    width: u32,
    height: u32,
    bits: u32,
    color: &'static str,
}

#[derive(Debug)]
struct Draw {
    image: usize,
    transform: [f64; 6],
}

#[derive(Default)]
struct QpdfPage {
    object: u32,
    content: u32,
    images: BTreeMap<String, u32>,
}

fn qpdf_object(pdf: &Path, object: u32, stream: bool) -> Vec<u8> {
    let mut command = Command::new("qpdf");
    command.arg(format!("--show-object={object}"));
    if stream {
        command.arg("--raw-stream-data");
    }
    tool(command.arg(pdf), "qpdf object inspection").stdout
}

fn dictionary_number(dictionary: &str, name: &str) -> u32 {
    let words: Vec<_> = dictionary.split_ascii_whitespace().collect();
    let at = words.iter().position(|word| *word == name).unwrap();
    words[at + 1].parse().unwrap()
}

/// qpdf resolves the page tree/resources and reads stream lengths itself.
/// The test parses only its small inspection protocol, never PDF bytes.
fn inspect_pdf(
    temporary: &TempDir,
    bytes: &[u8],
    dimensions: &[(u32, u32)],
    expected_images: &[ExpectedImage<'_>],
) -> (PathBuf, Vec<Vec<Draw>>) {
    let pdf = temporary.file("document.pdf");
    fs::write(&pdf, bytes).unwrap();
    tool(
        Command::new("qpdf").arg("--check").arg(&pdf),
        "qpdf --check",
    );
    let pages = tool(
        Command::new("qpdf")
            .args(["--show-pages", "--with-images"])
            .arg(&pdf),
        "qpdf pages/images",
    );
    let pages = String::from_utf8(pages.stdout).unwrap();
    let mut parsed = Vec::<QpdfPage>::new();
    for line in pages.lines() {
        let words: Vec<_> = line.split_ascii_whitespace().collect();
        if words.first() == Some(&"page") {
            assert_eq!(words[1], format!("{}:", parsed.len() + 1));
            parsed.push(QpdfPage {
                object: words[2].parse().unwrap(),
                ..QpdfPage::default()
            });
        } else if words.first().is_some_and(|word| word.starts_with('/')) {
            let current = parsed.last_mut().unwrap();
            let name = words[0].trim_end_matches(':').to_owned();
            assert!(
                current
                    .images
                    .insert(name, words[1].parse().unwrap())
                    .is_none()
            );
        } else if words.len() == 3 && words[1..] == ["0", "R"] {
            let current = parsed.last_mut().unwrap();
            assert_eq!(current.content, 0, "one content stream per generated page");
            current.content = words[0].parse().unwrap();
        }
    }
    assert_eq!(parsed.len(), dimensions.len(), "qpdf page count");
    let ids: BTreeSet<_> = parsed
        .iter()
        .flat_map(|page| page.images.values().copied())
        .collect();
    assert_eq!(
        ids.len(),
        expected_images.len(),
        "one referenced XObject per image handle"
    );

    // Also count every image object, including unreferenced duplicates. qpdf's
    // JSON object dictionaries contain no image stream bytes by default.
    let json = tool(
        Command::new("qpdf")
            .args(["--json", "--json-key=qpdf"])
            .arg(&pdf),
        "qpdf image-object inventory",
    );
    let compact: String = String::from_utf8(json.stdout)
        .unwrap()
        .split_ascii_whitespace()
        .collect();
    assert_eq!(
        compact.matches("\"/Subtype\":\"/Image\"").count(),
        expected_images.len()
    );

    let mut fixture_by_object = BTreeMap::new();
    for object in ids {
        let dictionary = String::from_utf8(qpdf_object(&pdf, object, false)).unwrap();
        let stream = if dictionary.contains("/FlateDecode") {
            tool(
                Command::new("qpdf")
                    .arg(format!("--show-object={object}"))
                    .arg("--filtered-stream-data")
                    .arg(&pdf),
                "qpdf decoded bilevel pixels",
            )
            .stdout
        } else {
            qpdf_object(&pdf, object, true)
        };
        let fixture = expected_images
            .iter()
            .position(|image| image.bytes == stream)
            .expect("qpdf stream equals one original fixture");
        assert!(
            !fixture_by_object.values().any(|&value| value == fixture),
            "image payload was embedded twice"
        );
        fixture_by_object.insert(object, fixture);
        let expected = &expected_images[fixture];
        assert_eq!(dictionary_number(&dictionary, "/Width"), expected.width);
        assert_eq!(dictionary_number(&dictionary, "/Height"), expected.height);
        assert_eq!(
            dictionary_number(&dictionary, "/BitsPerComponent"),
            expected.bits
        );
        let words: Vec<_> = dictionary.split_ascii_whitespace().collect();
        let color = words
            .iter()
            .position(|word| *word == "/ColorSpace")
            .unwrap();
        assert_eq!(words[color + 1], expected.color);
    }
    let mut draws = Vec::new();
    for (observed, &(width, height)) in parsed.iter().zip(dimensions) {
        let dictionary = String::from_utf8(qpdf_object(&pdf, observed.object, false)).unwrap();
        let words: Vec<_> = dictionary.split_ascii_whitespace().collect();
        let media = words.iter().position(|word| *word == "/MediaBox").unwrap();
        assert_eq!(words[media + 1], "[");
        let actual: Vec<f64> = words[media + 2..media + 6]
            .iter()
            .map(|word| word.parse().unwrap())
            .collect();
        assert_eq!(actual, [0.0, 0.0, f64::from(width), f64::from(height)]);
        let content = tool(
            Command::new("qpdf")
                .arg(format!("--show-object={}", observed.content))
                .arg("--filtered-stream-data")
                .arg(&pdf),
            "qpdf decoded page content",
        );
        let content = String::from_utf8(content.stdout).unwrap();
        let words: Vec<_> = content.split_ascii_whitespace().collect();
        assert_eq!(words.len() % 11, 0, "only isolated affine image draws");
        let mut page_draws = Vec::new();
        for operation in words.chunks_exact(11) {
            assert_eq!(
                [operation[0], operation[7], operation[9], operation[10]],
                ["q", "cm", "Do", "Q"]
            );
            let mut transform = [0.0_f64; 6];
            for (value, word) in transform.iter_mut().zip(&operation[1..7]) {
                assert!(
                    word.bytes()
                        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'-' | b'+' | b'.')),
                    "PDF content contains a nondecimal number: {word}"
                );
                *value = word.parse().unwrap();
                assert!(value.is_finite());
            }
            let object = observed
                .images
                .get(operation[8])
                .expect("Do name resolves through qpdf resources");
            page_draws.push(Draw {
                image: fixture_by_object[object],
                transform,
            });
        }
        draws.push(page_draws);
    }
    (pdf, draws)
}

fn assert_draws(actual: &[Draw], expected: &[(usize, [f64; 6])]) {
    assert_eq!(actual.len(), expected.len());
    for (draw, &(image, transform)) in actual.iter().zip(expected) {
        assert_eq!(draw.image, image, "source draw order/handle");
        assert_eq!(draw.transform, transform, "exact six-component CTM");
    }
}

#[test]
fn asymmetric_bilevel_and_raw_rgb_keep_rows_or_flip_only_with_negative_ctm() {
    let temporary = TempDir::new();
    let colors = [
        [240, 10, 10],
        [10, 220, 30],
        [240, 220, 20],
        [10, 210, 210],
        [0, 0, 0],
        [230, 10, 220],
        [20, 40, 230],
        [255, 255, 255],
        [230, 110, 10],
    ];
    let raw: Vec<u8> = colors.iter().flatten().copied().collect();
    let packed = [0x80, 0x40, 0x60];
    let padded = [0x80, 0xe1, 0x40, 0xe2, 0x60, 0xe3];
    let mut source = Source::new(&raw);
    let limits = Limits {
        io_chunk_bytes: 4,
        ..Limits::default()
    };
    let mut output = WriteSink::new(Vec::new());
    let matrices = [
        [
            [12.0, 0.0, 0.0, 12.0, 0.0, 2.0],
            [12.0, 0.0, 0.0, 12.0, 16.0, 2.0],
        ],
        [
            [12.0, 0.0, 0.0, -12.0, 0.0, 14.0],
            [12.0, 0.0, 0.0, -12.0, 16.0, 14.0],
        ],
    ];
    let report = ready(async {
        let mut document = PdfDocument::new(&mut output, &limits, &NeverCancel).await?;
        let mut bilevel = document
            .begin_bilevel_image(BilevelImageSpec {
                pixel_width: 3,
                pixel_height: 3,
                row_stride: 2,
            })
            .await?;
        assert_eq!(bilevel.write(&padded[..1]).await?, 1);
        assert_eq!(bilevel.write(&padded[1..5]).await?, 4);
        assert_eq!(bilevel.write(&padded[5..]).await?, 1);
        let bilevel = bilevel.finish().await?;
        let rgb = document
            .add_image(
                &mut source,
                4,
                raw.len() as u64,
                spec(3, 3, ImageEncoding::Rgb8),
            )
            .await?;
        source.sealed = true;
        for transforms in matrices {
            document
                .add_placed_page(
                    page(32.0, 16.0),
                    &[
                        ImagePlacement {
                            image: bilevel,
                            transform: transforms[0],
                        },
                        ImagePlacement {
                            image: rgb,
                            transform: transforms[1],
                        },
                    ],
                )
                .await?;
        }
        document.finish().await
    })
    .unwrap();
    assert_eq!(source.read_bytes, raw.len() as u64);
    assert!(source.max_request <= 4);
    assert_eq!(report.input_bytes_read, raw.len() as u64);
    assert_eq!(report.pages_converted, 2);
    let (pdf, draws) = inspect_pdf(
        &temporary,
        &output.into_inner(),
        &[(32, 16); 2],
        &[
            ExpectedImage {
                bytes: &packed,
                width: 3,
                height: 3,
                bits: 1,
                color: "/DeviceGray",
            },
            ExpectedImage {
                bytes: &raw,
                width: 3,
                height: 3,
                bits: 8,
                color: "/DeviceRGB",
            },
        ],
    );
    for (index, transforms) in matrices.iter().enumerate() {
        assert_draws(&draws[index], &[(0, transforms[0]), (1, transforms[1])]);
        check_renderers(&temporary, &pdf, index + 1, 32, 16, |reader, raster| {
            for row in 0..3 {
                let source_row = if index == 0 { row } else { 2 - row };
                for column in 0..3 {
                    let y = 12.0 - row as f64 * 4.0;
                    let bilevel = if packed[source_row] & (0x80 >> column) != 0 {
                        [0; 3]
                    } else {
                        [255; 3]
                    };
                    close_pixel(
                        raster.point(2.0 + column as f64 * 4.0, y, 32, 16),
                        bilevel,
                        0,
                        reader,
                    );
                    close_pixel(
                        raster.point(18.0 + column as f64 * 4.0, y, 32, 16),
                        colors[source_row * 3 + column],
                        1,
                        reader,
                    );
                }
            }
            for (x, y) in [(14.0, 8.0), (30.0, 8.0), (6.0, 15.0), (22.0, 1.0)] {
                close_pixel(raster.point(x, y, 32, 16), [255; 3], 0, reader);
            }
        });
    }
}

fn jpeg(temporary: &TempDir, gray: bool) -> (Vec<u8>, Raster) {
    let channels = if gray { 1 } else { 3 };
    let name = if gray { "gray" } else { "rgb" };
    let mut pixels = format!("P{}\n16 16\n255\n", if gray { 5 } else { 6 }).into_bytes();
    let gray_tiles = [15, 75, 190, 245];
    let rgb_tiles = [[245, 15, 15], [15, 210, 35], [25, 45, 225], [240, 210, 20]];
    for y in 0..16 {
        for x in 0..16 {
            let tile = y / 8 * 2 + x / 8;
            if channels == 1 {
                pixels.push(gray_tiles[tile]);
            } else {
                pixels.extend_from_slice(&rgb_tiles[tile]);
            }
        }
    }
    let pnm = temporary.file(&format!("{name}.pnm"));
    fs::write(&pnm, pixels).unwrap();
    let mut encoder = Command::new("cjpeg");
    encoder.args([
        "-quality",
        "100",
        "-sample",
        "1x1",
        "-baseline",
        "-dct",
        "int",
    ]);
    if gray {
        encoder.arg("-grayscale");
    }
    let bytes = tool(encoder.arg(&pnm), "cjpeg original synthetic pixels").stdout;
    let file = temporary.file(&format!("{name}.jpg"));
    fs::write(&file, &bytes).unwrap();
    let direct = tool(
        Command::new("djpeg").arg("-pnm").arg(&file),
        "djpeg independent baseline",
    );
    let direct = Raster::parse(&direct.stdout);
    assert_eq!(
        (direct.width, direct.height, direct.channels),
        (16, 16, channels)
    );
    (bytes, direct)
}

#[test]
fn runtime_gray_and_rgb_jpeg_streams_are_unchanged_and_both_ctm_signs_render() {
    let temporary = TempDir::new();
    let (gray, gray_pixels) = jpeg(&temporary, true);
    let (rgb, rgb_pixels) = jpeg(&temporary, false);
    let mut gray_source = Source::new(&gray);
    let mut rgb_source = Source::new(&rgb);
    let mut output = WriteSink::new(Vec::new());
    let limits = Limits {
        io_chunk_bytes: 31,
        ..Limits::default()
    };
    let matrices = [
        [16.0, 0.0, 0.0, 16.0, 1.0, 2.0],
        [16.0, 0.0, 0.0, -16.0, 22.0, 18.0],
    ];
    let report = ready(async {
        let mut document = PdfDocument::new(&mut output, &limits, &NeverCancel).await?;
        let gray_image = document
            .add_image(
                &mut gray_source,
                4,
                gray.len() as u64,
                spec(16, 16, ImageEncoding::JpegGray8),
            )
            .await?;
        let rgb_image = document
            .add_image(
                &mut rgb_source,
                4,
                rgb.len() as u64,
                spec(16, 16, ImageEncoding::JpegRgb8),
            )
            .await?;
        gray_source.sealed = true;
        rgb_source.sealed = true;
        for image in [gray_image, rgb_image] {
            document
                .add_placed_page(
                    page(40.0, 20.0),
                    &[
                        ImagePlacement {
                            image,
                            transform: matrices[0],
                        },
                        ImagePlacement {
                            image,
                            transform: matrices[1],
                        },
                    ],
                )
                .await?;
        }
        document.finish().await
    })
    .unwrap();
    assert_eq!(report.input_bytes_read, (gray.len() + rgb.len()) as u64);
    assert_eq!(gray_source.read_bytes, gray.len() as u64);
    assert_eq!(rgb_source.read_bytes, rgb.len() as u64);
    assert!(gray_source.max_request <= 31 && rgb_source.max_request <= 31);
    let (pdf, draws) = inspect_pdf(
        &temporary,
        &output.into_inner(),
        &[(40, 20); 2],
        &[
            ExpectedImage {
                bytes: &gray,
                width: 16,
                height: 16,
                bits: 8,
                color: "/DeviceGray",
            },
            ExpectedImage {
                bytes: &rgb,
                width: 16,
                height: 16,
                bits: 8,
                color: "/DeviceRGB",
            },
        ],
    );
    for (index, direct) in [&gray_pixels, &rgb_pixels].into_iter().enumerate() {
        assert_draws(&draws[index], &[(index, matrices[0]), (index, matrices[1])]);
        check_renderers(&temporary, &pdf, index + 1, 40, 20, |reader, raster| {
            for row in 0..2 {
                for column in 0..2 {
                    let x = 4 + column * 8;
                    let y = 4 + row * 8;
                    close_pixel(
                        raster.point(1.0 + x as f64, 18.0 - y as f64, 40, 20),
                        direct.pixel(x, y),
                        5,
                        reader,
                    );
                    close_pixel(
                        raster.point(22.0 + x as f64, 18.0 - y as f64, 40, 20),
                        direct.pixel(x, 15 - y),
                        5,
                        reader,
                    );
                }
            }
            close_pixel(raster.point(19.0, 10.0, 40, 20), [255; 3], 0, reader);
        });
    }
}

#[test]
fn ordered_overlaps_repeated_handles_and_affine_pages_reuse_only_two_streams() {
    let temporary = TempDir::new();
    let red = [240, 10, 10];
    let blue = [10, 20, 230];
    let mut red_source = Source::new(&red);
    let mut blue_source = Source::new(&blue);
    let mut output = WriteSink::new(Vec::new());
    let limits = Limits {
        io_chunk_bytes: 2,
        ..Limits::default()
    };
    let overlap = [
        [16.0, 0.0, 0.0, 16.0, 2.0, 2.0],
        [12.0, 0.0, 0.0, 12.0, 10.0, 6.0],
        [4.0, 0.0, 0.0, 4.0, 14.0, 10.0],
    ];
    let affine = [
        [0.0, 6.0, -5.0, 0.0, 25.5, 3.25],
        [8.5, 1.25, 2.75, 7.5, -3.25, 15.125],
        [3.5, 0.0, 0.0, 2.75, 6.125, -1.5],
        [3.0, 0.0, 0.0, -3.0, 40.0, 50.0],
    ];
    let report = ready(async {
        let mut document = PdfDocument::new(&mut output, &limits, &NeverCancel).await?;
        let red_image = document
            .add_image(&mut red_source, 4, 3, spec(1, 1, ImageEncoding::Rgb8))
            .await?;
        let blue_image = document
            .add_image(&mut blue_source, 4, 3, spec(1, 1, ImageEncoding::Rgb8))
            .await?;
        red_source.sealed = true;
        blue_source.sealed = true;
        for handles in [
            [red_image, blue_image, red_image],
            [blue_image, red_image, blue_image],
        ] {
            let placements: Vec<_> = handles
                .into_iter()
                .zip(overlap)
                .map(|(image, transform)| ImagePlacement { image, transform })
                .collect();
            document
                .add_placed_page(page(32.0, 24.0), &placements)
                .await?;
        }
        let placements: Vec<_> = [red_image, blue_image, red_image, blue_image]
            .into_iter()
            .zip(affine)
            .map(|(image, transform)| ImagePlacement { image, transform })
            .collect();
        document
            .add_placed_page(page(32.0, 24.0), &placements)
            .await?;
        document.finish().await
    })
    .unwrap();
    assert_eq!(report.input_bytes_read, 6);
    assert_eq!(red_source.read_bytes + blue_source.read_bytes, 6);
    assert!(red_source.max_request <= 2 && blue_source.max_request <= 2);
    assert_eq!(report.pages_converted, 3);
    let (pdf, draws) = inspect_pdf(
        &temporary,
        &output.into_inner(),
        &[(32, 24); 3],
        &[
            ExpectedImage {
                bytes: &red,
                width: 1,
                height: 1,
                bits: 8,
                color: "/DeviceRGB",
            },
            ExpectedImage {
                bytes: &blue,
                width: 1,
                height: 1,
                bits: 8,
                color: "/DeviceRGB",
            },
        ],
    );
    for (index, identities) in [[0, 1, 0], [1, 0, 1]].into_iter().enumerate() {
        let expected: Vec<_> = identities.into_iter().zip(overlap).collect();
        assert_draws(&draws[index], &expected);
        let (first, second) = if index == 0 { (red, blue) } else { (blue, red) };
        check_renderers(&temporary, &pdf, index + 1, 32, 24, |reader, raster| {
            for (x, y, want) in [
                (4.0, 4.0, first),
                (12.0, 8.0, second),
                (16.0, 12.0, first),
                (20.0, 16.0, second),
                (30.0, 2.0, [255; 3]),
            ] {
                close_pixel(raster.point(x, y, 32, 24), want, 1, reader);
            }
        });
    }
    let expected: Vec<_> = [0, 1, 0, 1].into_iter().zip(affine).collect();
    assert_draws(&draws[2], &expected);
    check_renderers(&temporary, &pdf, 3, 32, 24, |reader, raster| {
        for (x, y, want) in [
            (23.0, 6.0, red),
            (2.0, 19.0, blue),
            (7.0, 0.5, red),
            (16.0, 20.0, [255; 3]),
        ] {
            close_pixel(raster.point(x, y, 32, 24), want, 1, reader);
        }
    });
}
