// SPDX-License-Identifier: MIT

//! Independent reopen tests for PDFs emitted by the forward-only writer.
//! These test-only tools are required in CI; missing executables fail clearly.

use caj2pdf_core::{
    Bookmark, Limits, NeverCancel, RangedSource,
    native::{SeekableSource, WriteSink},
    pdf::{ImageEncoding, ImageSpec, PageSpec, PdfDocument, PdfWriter},
};
use std::{
    fs::{File, OpenOptions, read, remove_file},
    future::Future,
    io::{Cursor, Write},
    path::{Path, PathBuf},
    pin::pin,
    process::{Command, Output, Stdio},
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
            "caj2pdf-{label}-{}-{sequence}.pdf",
            std::process::id()
        ));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap_or_else(|error| panic!("create temporary PDF {}: {error}", path.display()));
        Self { path, file }
    }
}

impl Drop for TempPdf {
    fn drop(&mut self) {
        // The file handle is dropped after this method returns. Unix permits
        // unlinking an open test file, and Windows is not a CI target here.
        let _ = remove_file(&self.path);
    }
}

fn tool_output(command: &mut Command, name: &str) -> Output {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("{name} is required for PDF validation tests: {error}"));
    assert!(
        output.status.success(),
        "{name} rejected generated PDF (status {}):\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn validate_pdf(path: &Path, pages: u32) -> (String, String) {
    tool_output(
        Command::new("qpdf").arg("--check").arg(path),
        "qpdf --check",
    );
    let mutool = tool_output(Command::new("mutool").arg("info").arg(path), "mutool info");
    let pdfinfo = tool_output(
        Command::new("pdfinfo")
            .arg("-f")
            .arg("1")
            .arg("-l")
            .arg(pages.to_string())
            .arg("-box")
            .arg(path),
        "pdfinfo -box",
    );
    (
        String::from_utf8(mutool.stdout).expect("MuPDF info is UTF-8"),
        String::from_utf8(pdfinfo.stdout).expect("Poppler info is UTF-8"),
    )
}

fn compact_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn an_empty_stream_pdf_passes_independent_reopen_checks() {
    let mut output = TempPdf::new("empty-stream");
    let limits = Limits::default();
    let mut sink = WriteSink::new(&mut output.file);
    let bytes_written = run_native(async {
        let mut writer = PdfWriter::new(&mut sink, &limits, &NeverCancel).await?;
        let catalog = writer.reserve_object()?;
        let pages = writer.reserve_object()?;
        let page = writer.reserve_object()?;
        let content = writer.reserve_object()?;
        let length = writer.reserve_object()?;
        writer
            .write_object(catalog, b"<< /Type /Catalog /Pages 2 0 R >>")
            .await?;
        writer
            .write_object(pages, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>")
            .await?;
        writer
            .write_object(
                page,
                b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Resources << >> /Contents 4 0 R >>",
            )
            .await?;
        writer.begin_stream(content, length, b"").await?;
        writer.end_stream().await?;
        writer.finish(catalog).await
    })
    .unwrap();
    output.file.flush().unwrap();
    assert_eq!(bytes_written, output.file.metadata().unwrap().len());

    let (mutool, pdfinfo) = validate_pdf(&output.path, 1);
    assert!(compact_whitespace(&mutool).contains("Pages: 1"));
    assert!(compact_whitespace(&pdfinfo).contains("Pages: 1"));
    assert!(compact_whitespace(&pdfinfo).contains("Page 1 size: 200 x 100 pts"));
}

#[test]
fn image_pages_and_unicode_outlines_reopen_with_correct_order_and_dimensions() {
    // These delimiter-like bytes are valid 8-bit grayscale pixels. The
    // independent parsers must use /Length rather than textual delimiter scans.
    const BINARY_GRAY: &[u8] = b"\x00\xffendstream\nendobj\nxref\n";
    const RGB: [u8; 18] = [
        255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0, 0, 255, 255, 255, 0, 255,
    ];

    let mut output = TempPdf::new("image-outlines");
    let limits = Limits::default();
    let mut sink = WriteSink::new(&mut output.file);
    let mut gray_source = SeekableSource::new(Cursor::new(BINARY_GRAY)).unwrap();
    let mut rgb_source = SeekableSource::new(Cursor::new(RGB.as_slice())).unwrap();
    let report = run_native(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NeverCancel).await?;
        let first = document
            .add_image_page(
                &mut gray_source,
                0,
                BINARY_GRAY.len() as u64,
                PageSpec {
                    width_points: 200.0,
                    height_points: 300.0,
                },
                ImageSpec {
                    pixel_width: BINARY_GRAY.len() as u32,
                    pixel_height: 1,
                    encoding: ImageEncoding::Gray8,
                },
            )
            .await?;
        let second = document
            .add_image_page(
                &mut rgb_source,
                0,
                RGB.len() as u64,
                PageSpec {
                    width_points: 400.0,
                    height_points: 250.0,
                },
                ImageSpec {
                    pixel_width: 2,
                    pixel_height: 3,
                    encoding: ImageEncoding::Rgb8,
                },
            )
            .await?;
        assert_eq!((first, second), (0, 1));
        document
            .add_bookmark(Bookmark {
                depth: 0,
                title: "First".into(),
                page_index: first,
            })
            .await?;
        document
            .add_bookmark(Bookmark {
                depth: 1,
                title: "章节😀".into(),
                page_index: second,
            })
            .await?;
        document
            .add_bookmark(Bookmark {
                depth: 0,
                title: "Last".into(),
                page_index: second,
            })
            .await?;
        document.finish().await
    })
    .unwrap();
    output.file.flush().unwrap();
    assert_eq!(report.pages_converted, 2);
    assert_eq!(report.bookmarks_written, 3);
    assert_eq!(
        report.input_bytes_read,
        (BINARY_GRAY.len() + RGB.len()) as u64
    );
    assert_eq!(
        report.output_bytes_written,
        output.file.metadata().unwrap().len()
    );

    let (mutool, pdfinfo) = validate_pdf(&output.path, 2);
    let mutool = compact_whitespace(&mutool);
    let pdfinfo = compact_whitespace(&pdfinfo);
    assert!(mutool.contains("Pages: 2"), "{mutool}");
    assert!(pdfinfo.contains("Page 1 size: 200 x 300 pts"), "{pdfinfo}");
    assert!(pdfinfo.contains("Page 2 size: 400 x 250 pts"), "{pdfinfo}");

    let outline = tool_output(
        Command::new("mutool")
            .arg("show")
            .arg(&output.path)
            .arg("outline"),
        "mutool show outline",
    );
    let outline = String::from_utf8(outline.stdout).expect("MuPDF outline is UTF-8");
    let first = outline
        .lines()
        .find(|line| line.contains("\"First\""))
        .unwrap();
    let nested = outline
        .lines()
        .find(|line| line.contains("\"章节😀\""))
        .unwrap();
    let last = outline
        .lines()
        .find(|line| line.contains("\"Last\""))
        .unwrap();
    assert!(first.contains("#page=1"), "{outline}");
    assert!(nested.contains("#page=2"), "{outline}");
    assert!(
        nested.contains('|'),
        "outline child is not nested: {outline}"
    );
    assert!(last.contains("#page=2"), "{outline}");

    // Poppler independently decodes the first raw grayscale image. Some
    // versions emit PGM; others emit PPM with each gray sample expanded to
    // equal red, green, and blue samples. Either must preserve the pixels.
    let image_root = output.path.with_extension("image");
    let images = tool_output(
        Command::new("pdfimages")
            .arg("-f")
            .arg("1")
            .arg("-l")
            .arg("1")
            .arg("-print-filenames")
            .arg(&output.path)
            .arg(&image_root),
        "pdfimages",
    );
    let filenames = String::from_utf8(images.stdout).expect("pdfimages filenames are UTF-8");
    let filenames: Vec<_> = filenames.lines().filter(|line| !line.is_empty()).collect();
    assert_eq!(filenames.len(), 1, "expected one decoded grayscale image");
    let extracted = PathBuf::from(filenames[0]);
    let image_bytes = read(&extracted).expect("read image decoded by pdfimages");
    remove_file(&extracted).expect("remove decoded test image");
    let ppm_header = format!("P6\n{} 1\n255\n", BINARY_GRAY.len());
    let pgm_header = format!("P5\n{} 1\n255\n", BINARY_GRAY.len());
    if image_bytes.starts_with(ppm_header.as_bytes()) {
        let pixels = &image_bytes[ppm_header.len()..];
        assert_eq!(pixels.len(), BINARY_GRAY.len() * 3);
        for (rgb, gray) in pixels.chunks_exact(3).zip(BINARY_GRAY) {
            assert_eq!(rgb, [*gray; 3], "decoded grayscale pixel differs");
        }
    } else if image_bytes.starts_with(pgm_header.as_bytes()) {
        assert_eq!(&image_bytes[pgm_header.len()..], BINARY_GRAY);
    } else {
        panic!(
            "unexpected PPM/PGM header in {}: first bytes {:?}",
            extracted.display(),
            &image_bytes[..image_bytes.len().min(32)]
        );
    }
}

#[test]
fn jpeg_gray_image_is_passed_through_and_renders() {
    // The PGM is original test input. cjpeg produces a real JPEG at test
    // runtime, so no binary fixture or codec implementation enters the repo.
    let mut pgm = b"P5\n8 8\n255\n".to_vec();
    pgm.extend_from_slice(&[96_u8; 64]);
    let encoder_program = std::env::var_os("CAJ2PDF_TEST_CJPEG").unwrap_or_else(|| "cjpeg".into());
    let mut encoder = Command::new(encoder_program)
        .arg("-grayscale")
        .arg("-quality")
        .arg("100")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("cjpeg is required for JPEG PDF tests: {error}"));
    encoder
        .stdin
        .take()
        .expect("cjpeg stdin was piped")
        .write_all(&pgm)
        .expect("write original PGM to cjpeg");
    let encoded = encoder.wait_with_output().expect("wait for cjpeg");
    assert!(
        encoded.status.success(),
        "cjpeg failed: {}",
        String::from_utf8_lossy(&encoded.stderr)
    );
    let jpeg = encoded.stdout;
    assert!(jpeg.starts_with(&[0xff, 0xd8]));
    assert!(jpeg.ends_with(&[0xff, 0xd9]));

    let mut output = TempPdf::new("jpeg-gray");
    let limits = Limits::default();
    let mut sink = WriteSink::new(&mut output.file);
    let mut source = SeekableSource::new(Cursor::new(jpeg.as_slice())).unwrap();
    let report = run_native(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NeverCancel).await?;
        document
            .add_image_page(
                &mut source,
                0,
                jpeg.len() as u64,
                PageSpec {
                    width_points: 8.0,
                    height_points: 8.0,
                },
                ImageSpec {
                    pixel_width: 8,
                    pixel_height: 8,
                    encoding: ImageEncoding::JpegGray8,
                },
            )
            .await?;
        document.finish().await
    })
    .unwrap();
    output.file.flush().unwrap();
    assert_eq!(report.input_bytes_read, jpeg.len() as u64);
    assert_eq!(report.pages_converted, 1);

    let (mutool, pdfinfo) = validate_pdf(&output.path, 1);
    assert!(compact_whitespace(&mutool).contains("Pages: 1"));
    assert!(compact_whitespace(&pdfinfo).contains("Page 1 size: 8 x 8 pts"));

    // Poppler's JPEG extraction must be byte-for-byte equal to the input.
    let image_root = output.path.with_extension("jpeg-extracted");
    let images = tool_output(
        Command::new("pdfimages")
            .arg("-f")
            .arg("1")
            .arg("-l")
            .arg("1")
            .arg("-j")
            .arg("-print-filenames")
            .arg(&output.path)
            .arg(&image_root),
        "pdfimages -j",
    );
    let filenames = String::from_utf8(images.stdout).expect("pdfimages filenames are UTF-8");
    let filenames: Vec<_> = filenames.lines().filter(|line| !line.is_empty()).collect();
    assert_eq!(filenames.len(), 1, "expected one extracted JPEG");
    let extracted = PathBuf::from(filenames[0]);
    let extracted_jpeg = read(&extracted).expect("read extracted JPEG");
    remove_file(&extracted).expect("remove extracted JPEG");
    assert_eq!(
        extracted_jpeg, jpeg,
        "JPEG bytes changed during PDF writing"
    );

    // MuPDF independently decodes and draws the JPEG XObject. A constant
    // source level leaves enough headroom for ordinary JPEG rounding.
    let rendered = output.path.with_extension("render.pgm");
    tool_output(
        Command::new("mutool")
            .arg("draw")
            .arg("-q")
            .arg("-F")
            .arg("pnm")
            .arg("-c")
            .arg("gray")
            .arg("-r")
            .arg("72")
            .arg("-A")
            .arg("0")
            .arg("-o")
            .arg(&rendered)
            .arg(&output.path)
            .arg("1"),
        "mutool draw",
    );
    let raster = read(&rendered).expect("read MuPDF rendering");
    remove_file(&rendered).expect("remove MuPDF rendering");
    let header = b"P5\n8 8\n255\n";
    assert!(
        raster.starts_with(header),
        "unexpected PGM rendering: first bytes {:?}",
        &raster[..raster.len().min(32)]
    );
    let pixels = &raster[header.len()..];
    assert_eq!(pixels.len(), 64);
    assert!(
        pixels.iter().all(|pixel| pixel.abs_diff(96) <= 5),
        "rendered JPEG pixels differ from original gray level: {pixels:?}"
    );
}

const IMAGE_MARKER: &[u8] = b"\x00\xffendstream\nendobj\nxref\n";

struct GeneratedGrayImage {
    size: u64,
    max_request: usize,
    reads: usize,
}

impl RangedSource for GeneratedGrayImage {
    fn size(&self) -> u64 {
        self.size
    }

    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        self.max_request = self.max_request.max(destination.len());
        self.reads += 1;
        let count = (self.size.saturating_sub(offset)).min(destination.len() as u64) as usize;
        for (index, byte) in destination[..count].iter_mut().enumerate() {
            let position = offset + index as u64;
            *byte = if position < IMAGE_MARKER.len() as u64 {
                IMAGE_MARKER[position as usize]
            } else {
                (position % 251) as u8
            };
        }
        Ok(count)
    }
}

#[test]
fn a_large_image_stream_reopens_without_whole_image_input_allocation() {
    const WIDTH: u32 = 1024;
    const HEIGHT: u32 = 1025;
    const BYTES: u64 = WIDTH as u64 * HEIGHT as u64;

    let mut source = GeneratedGrayImage {
        size: BYTES,
        max_request: 0,
        reads: 0,
    };
    let mut output = TempPdf::new("large-image");
    let limits = Limits::default();
    let mut sink = WriteSink::new(&mut output.file);
    let report = run_native(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NeverCancel).await?;
        document
            .add_image_page(
                &mut source,
                0,
                BYTES,
                PageSpec {
                    width_points: 1024.0,
                    height_points: 1025.0,
                },
                ImageSpec {
                    pixel_width: WIDTH,
                    pixel_height: HEIGHT,
                    encoding: ImageEncoding::Gray8,
                },
            )
            .await?;
        document.finish().await
    })
    .unwrap();
    output.file.flush().unwrap();
    assert_eq!(report.input_bytes_read, BYTES);
    assert!(report.input_bytes_read > 1024 * 1024);
    assert_eq!(report.pages_converted, 1);
    assert_eq!(
        report.output_bytes_written,
        output.file.metadata().unwrap().len()
    );
    assert!(source.reads >= 5, "expected multiple bounded input reads");
    assert!(source.max_request <= limits.io_chunk_bytes);

    let (mutool, pdfinfo) = validate_pdf(&output.path, 1);
    assert!(compact_whitespace(&mutool).contains("Pages: 1"));
    assert!(compact_whitespace(&pdfinfo).contains("Page 1 size: 1024 x 1025 pts"));
}

#[test]
fn page_tree_rollover_preserves_page_order_at_257_pages() {
    let mut output = TempPdf::new("page-tree-rollover");
    let limits = Limits::default();
    let mut sink = WriteSink::new(&mut output.file);
    let mut pixel = SeekableSource::new(Cursor::new([0x7f_u8])).unwrap();
    let report = run_native(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NeverCancel).await?;
        for page_index in 0..257_u32 {
            let index = document
                .add_image_page(
                    &mut pixel,
                    0,
                    1,
                    PageSpec {
                        width_points: 100.0 + f64::from(page_index),
                        height_points: 200.0,
                    },
                    ImageSpec {
                        pixel_width: 1,
                        pixel_height: 1,
                        encoding: ImageEncoding::Gray8,
                    },
                )
                .await?;
            assert_eq!(index, page_index);
        }
        document.finish().await
    })
    .unwrap();
    output.file.flush().unwrap();
    assert_eq!(report.pages_converted, 257);
    assert_eq!(report.input_bytes_read, 257);
    assert_eq!(
        report.output_bytes_written,
        output.file.metadata().unwrap().len()
    );

    let (mutool, pdfinfo) = validate_pdf(&output.path, 257);
    assert!(compact_whitespace(&mutool).contains("Pages: 257"));
    let pdfinfo = compact_whitespace(&pdfinfo);
    assert!(
        pdfinfo.contains("Page 256 size: 355 x 200 pts"),
        "{pdfinfo}"
    );
    assert!(
        pdfinfo.contains("Page 257 size: 356 x 200 pts"),
        "{pdfinfo}"
    );
}
