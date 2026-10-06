// SPDX-License-Identifier: MIT

use caj2pdf_core::{
    Bookmark, Error, Limits, NeverCancel, RangedSource, Result, SequentialSink,
    native::{SeekableSource, WriteSink},
    pdf::{ImageEncoding, ImageSpec, PageSpec, PdfDocument},
};
use std::io::{self, Cursor};

fn page() -> PageSpec {
    PageSpec {
        width_points: 144.0,
        height_points: 72.5,
    }
}

fn gray(width: u32) -> ImageSpec {
    ImageSpec {
        pixel_width: width,
        pixel_height: 1,
        encoding: ImageEncoding::Gray8,
    }
}

fn one_byte_source() -> SeekableSource<Cursor<Vec<u8>>> {
    SeekableSource::new(Cursor::new(vec![0x7f])).unwrap()
}

fn outline_for_title<'a>(pdf: &'a str, title: &str) -> (u32, &'a str) {
    let encoded = title
        .encode_utf16()
        .map(|unit| format!("{unit:04X}"))
        .collect::<String>();
    let marker = format!("/Title <FEFF{encoded}>");
    let title_start = pdf.find(&marker).expect("outline title is present");
    let object_marker = pdf[..title_start]
        .rfind(" 0 obj\n")
        .expect("outline object header is present");
    let id_start = pdf[..object_marker]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let id = pdf[id_start..object_marker]
        .parse()
        .expect("outline object ID is numeric");
    let body_end = title_start + pdf[title_start..].find("endobj").unwrap();
    (id, &pdf[title_start..body_end])
}

#[test]
fn two_pages_and_nested_unicode_outline_have_checked_report_and_stream_bytes() -> Result<()> {
    let raw = b"endstream endobj xref".to_vec();
    let mut gray_source = SeekableSource::new(Cursor::new(raw.clone()))?;
    let mut rgb_source = SeekableSource::new(Cursor::new(vec![255, 0, 0, 0, 255, 0]))?;
    let mut output = WriteSink::new(Vec::<u8>::new());
    let limits = Limits::default();
    let report = (|| {
        let mut document = PdfDocument::new(&mut output, &limits, &NeverCancel)?;
        assert_eq!(
            document.add_image_page(
                &mut gray_source,
                0,
                raw.len() as u64,
                page(),
                gray(raw.len() as u32)
            )?,
            0
        );
        assert_eq!(
            document.add_image_page(
                &mut rgb_source,
                0,
                6,
                page(),
                ImageSpec {
                    pixel_width: 2,
                    pixel_height: 1,
                    encoding: ImageEncoding::Rgb8,
                }
            )?,
            1
        );
        for (depth, title, page_index) in [
            (0, "文😀", 0),
            (1, "First child", 1),
            (2, "Grandchild", 0),
            (2, "Sibling", 1),
            (1, "Second child", 0),
            (0, "Last root", 1),
        ] {
            document.add_bookmark(Bookmark {
                depth,
                title: title.into(),
                page_index,
            })?;
        }
        document.finish()
    })()?;
    let pdf = output.into_inner();
    assert_eq!(report.input_bytes_read, raw.len() as u64 + 6);
    assert_eq!(report.output_bytes_written, pdf.len() as u64);
    assert_eq!(report.pages_converted, 2);
    assert_eq!(report.bookmarks_written, 6);
    assert!(pdf.windows(raw.len()).any(|window| window == raw));
    assert!(
        pdf.windows(b"<FEFF6587D83DDE00>".len())
            .any(|window| window == b"<FEFF6587D83DDE00>")
    );
    let text = String::from_utf8_lossy(&pdf);
    let (first_root_id, first_root) = outline_for_title(&text, "文😀");
    let (first_child_id, first_child) = outline_for_title(&text, "First child");
    let (grandchild_id, grandchild) = outline_for_title(&text, "Grandchild");
    let (sibling_id, sibling) = outline_for_title(&text, "Sibling");
    let (second_child_id, second_child) = outline_for_title(&text, "Second child");
    let (last_root_id, last_root) = outline_for_title(&text, "Last root");
    assert!(first_root.contains(&format!("/First {first_child_id} 0 R")));
    assert!(first_root.contains(&format!("/Last {second_child_id} 0 R")));
    assert!(first_root.contains("/Count 4"));
    assert!(first_root.contains(&format!("/Next {last_root_id} 0 R")));
    assert!(last_root.contains(&format!("/Prev {first_root_id} 0 R")));
    assert!(first_child.contains(&format!("/First {grandchild_id} 0 R")));
    assert!(first_child.contains(&format!("/Last {sibling_id} 0 R")));
    assert!(first_child.contains("/Count 2"));
    assert!(first_child.contains(&format!("/Next {second_child_id} 0 R")));
    assert!(second_child.contains(&format!("/Prev {first_child_id} 0 R")));
    assert!(grandchild.contains(&format!("/Next {sibling_id} 0 R")));
    assert!(sibling.contains(&format!("/Prev {grandchild_id} 0 R")));
    Ok(())
}

#[test]
fn page_tree_crosses_256_page_leaf_boundary() -> Result<()> {
    let mut source = one_byte_source();
    let mut output = WriteSink::new(Vec::<u8>::new());
    let limits = Limits::default();
    let report = (|| {
        let mut document = PdfDocument::new(&mut output, &limits, &NeverCancel)?;
        for expected in 0..257 {
            let page_index = document.add_image_page(&mut source, 0, 1, page(), gray(1))?;
            assert_eq!(page_index, expected);
        }
        document.finish()
    })()?;
    let pdf = output.into_inner();
    let text = String::from_utf8_lossy(&pdf);
    assert_eq!(report.pages_converted, 257);
    assert_eq!(text.matches("/Type /Page /Parent").count(), 257);
    assert!(text.contains("/Type /Pages /Count 257 /Kids ["));
    Ok(())
}

#[test]
fn rejects_invalid_page_and_image_specs_before_reading() -> Result<()> {
    let mut source = CountingBytes::new(vec![0; 4]);
    let mut output = WriteSink::new(Vec::<u8>::new());
    let limits = Limits::default();
    (|| {
        let mut document = PdfDocument::new(&mut output, &limits, &NeverCancel)?;
        for width in [0.0, f64::NAN, f64::INFINITY, 14_401.0] {
            let error = document
                .add_image_page(
                    &mut source,
                    0,
                    1,
                    PageSpec {
                        width_points: width,
                        height_points: 72.0,
                    },
                    gray(1),
                )
                .unwrap_err();
            assert!(matches!(error, Error::InvalidInput { .. }));
        }
        let error = document
            .add_image_page(&mut source, 0, 3, page(), gray(4))
            .unwrap_err();
        assert!(matches!(error, Error::InvalidInput { .. }));
        let error = document
            .add_image_page(
                &mut source,
                0,
                1,
                page(),
                ImageSpec {
                    pixel_width: u32::MAX,
                    pixel_height: 1,
                    encoding: ImageEncoding::JpegGray8,
                },
            )
            .unwrap_err();
        assert!(matches!(
            error,
            Error::LimitExceeded {
                resource: "PDF image width",
                ..
            }
        ));
        let error = document
            .add_image_page(
                &mut source,
                0,
                1,
                page(),
                ImageSpec {
                    pixel_width: 1,
                    pixel_height: u32::MAX,
                    encoding: ImageEncoding::JpegRgb8,
                },
            )
            .unwrap_err();
        assert!(matches!(
            error,
            Error::LimitExceeded {
                resource: "PDF image height",
                ..
            }
        ));
        Ok::<_, Error>(())
    })()?;
    assert_eq!(source.reads, 0);
    Ok(())
}

#[test]
fn rejects_unavailable_image_ranges_zero_pixels_and_empty_jpeg_before_reading() -> Result<()> {
    let mut source = CountingBytes::new(vec![0x7f]);
    let mut output = WriteSink::new(Vec::<u8>::new());
    let limits = Limits::default();
    (|| {
        let mut document = PdfDocument::new(&mut output, &limits, &NeverCancel)?;
        for image in [
            ImageSpec {
                pixel_width: 0,
                pixel_height: 1,
                encoding: ImageEncoding::Gray8,
            },
            ImageSpec {
                pixel_width: 1,
                pixel_height: 0,
                encoding: ImageEncoding::Gray8,
            },
        ] {
            let error = document
                .add_image_page(&mut source, 0, 1, page(), image)
                .unwrap_err();
            assert!(matches!(error, Error::InvalidInput { .. }));
        }
        let empty_jpeg = document
            .add_image_page(
                &mut source,
                0,
                0,
                page(),
                ImageSpec {
                    pixel_width: 1,
                    pixel_height: 1,
                    encoding: ImageEncoding::JpegGray8,
                },
            )
            .unwrap_err();
        assert!(matches!(empty_jpeg, Error::InvalidInput { .. }));
        let beyond_source = document
            .add_image_page(&mut source, 2, 1, page(), gray(1))
            .unwrap_err();
        assert!(matches!(beyond_source, Error::InvalidInput { .. }));
        let short_range = document
            .add_image_page(&mut source, 0, 2, page(), gray(2))
            .unwrap_err();
        assert!(matches!(short_range, Error::TruncatedInput { .. }));
        // The PDF integer ceiling is checked before the source range.
        let too_long = document
            .add_image_page(
                &mut source,
                0,
                i32::MAX as u64 + 1,
                page(),
                ImageSpec {
                    pixel_width: 1,
                    pixel_height: 1,
                    encoding: ImageEncoding::JpegGray8,
                },
            )
            .unwrap_err();
        assert!(matches!(
            too_long,
            Error::LimitExceeded {
                resource: "PDF image stream bytes",
                limit: 2_147_483_647,
                attempted: 2_147_483_648,
            }
        ));
        Ok::<_, Error>(())
    })()?;
    assert_eq!(source.reads, 0);
    Ok(())
}

#[test]
fn rejects_empty_document_and_missing_bookmark_links() -> Result<()> {
    let mut output = WriteSink::new(Vec::<u8>::new());
    let limits = Limits::default();
    let error = (|| PdfDocument::new(&mut output, &limits, &NeverCancel)?.finish())().unwrap_err();
    assert!(matches!(error, Error::InvalidInput { .. }));

    let mut source = one_byte_source();
    let mut output = WriteSink::new(Vec::<u8>::new());
    (|| {
        let mut document = PdfDocument::new(&mut output, &limits, &NeverCancel)?;
        document.add_image_page(&mut source, 0, 1, page(), gray(1))?;
        let missing_page = document
            .add_bookmark(Bookmark {
                depth: 0,
                title: "future page".into(),
                page_index: 1,
            })
            .unwrap_err();
        assert!(matches!(missing_page, Error::InvalidInput { .. }));
        let missing_parent = document
            .add_bookmark(Bookmark {
                depth: 1,
                title: "orphan".into(),
                page_index: 0,
            })
            .unwrap_err();
        assert!(matches!(missing_parent, Error::InvalidInput { .. }));
        document.finish()
    })()?;
    Ok(())
}

#[test]
fn applies_page_bookmark_and_retained_title_limits() -> Result<()> {
    let mut source = one_byte_source();
    let mut output = WriteSink::new(Vec::<u8>::new());
    let limits = Limits {
        max_pages: 1,
        max_bookmarks: 1,
        ..Limits::default()
    };
    (|| {
        let mut document = PdfDocument::new(&mut output, &limits, &NeverCancel)?;
        document.add_image_page(&mut source, 0, 1, page(), gray(1))?;
        let second = document
            .add_image_page(&mut source, 0, 1, page(), gray(1))
            .unwrap_err();
        assert!(matches!(
            second,
            Error::LimitExceeded {
                resource: "pages",
                ..
            }
        ));
        document.add_bookmark(Bookmark {
            depth: 0,
            title: "first".into(),
            page_index: 0,
        })?;
        let second = document
            .add_bookmark(Bookmark {
                depth: 0,
                title: "second".into(),
                page_index: 0,
            })
            .unwrap_err();
        assert!(matches!(
            second,
            Error::LimitExceeded {
                resource: "bookmarks",
                ..
            }
        ));
        document.finish()
    })()?;

    let tiny = Limits {
        io_chunk_bytes: 1,
        max_allocation_bytes: 7,
        ..Limits::default()
    };
    let mut output = WriteSink::new(Vec::<u8>::new());
    let error = PdfDocument::new(&mut output, &tiny, &NeverCancel)
        .err()
        .unwrap();
    assert!(matches!(
        error,
        Error::LimitExceeded {
            resource: "allocation bytes",
            ..
        }
    ));
    let bounded = Limits {
        io_chunk_bytes: 1,
        max_allocation_bytes: 4096,
        ..Limits::default()
    };
    let mut source = one_byte_source();
    let mut output = WriteSink::new(Vec::<u8>::new());
    (|| {
        let mut document = PdfDocument::new(&mut output, &bounded, &NeverCancel)?;
        document.add_image_page(&mut source, 0, 1, page(), gray(1))?;
        let mut first = String::with_capacity(3000);
        first.push_str("first");
        document.add_bookmark(Bookmark {
            depth: 0,
            title: first,
            page_index: 0,
        })?;
        // The previous sibling is emitted before the replacement title is
        // retained, so the two large capacities need not coexist.
        let mut second = String::with_capacity(3000);
        second.push_str("second");
        document.add_bookmark(Bookmark {
            depth: 0,
            title: second,
            page_index: 0,
        })?;
        let mut title = String::with_capacity(8192);
        title.push('x');
        let error = document
            .add_bookmark(Bookmark {
                depth: 0,
                title,
                page_index: 0,
            })
            .unwrap_err();
        assert!(matches!(
            error,
            Error::LimitExceeded {
                resource: "allocation bytes",
                ..
            }
        ));
        let report = document.finish()?;
        assert_eq!(report.bookmarks_written, 2);
        Ok::<_, Error>(())
    })()?;
    let pdf = output.into_inner();
    let text = String::from_utf8_lossy(&pdf);
    let (first_id, first) = outline_for_title(&text, "first");
    let (second_id, second) = outline_for_title(&text, "second");
    assert!(first.contains(&format!("/Next {second_id} 0 R")));
    assert!(second.contains(&format!("/Prev {first_id} 0 R")));
    assert!(!text.contains("/Title <FEFF0078>"));
    Ok(())
}

#[test]
fn reports_truncated_source_and_failing_sink_without_success() -> Result<()> {
    let mut short = ShortSource { calls: 0 };
    let mut output = WriteSink::new(Vec::<u8>::new());
    let limits = Limits::default();
    let error = (|| {
        let mut document = PdfDocument::new(&mut output, &limits, &NeverCancel)?;
        document.add_image_page(&mut short, 0, 4, page(), gray(4))
    })()
    .unwrap_err();
    assert!(matches!(error, Error::TruncatedInput { .. }));
    assert_eq!(short.calls, 2);

    let mut failing_source = FailingSource { calls: 0 };
    let mut output = WriteSink::new(Vec::<u8>::new());
    let error = (|| {
        let mut document = PdfDocument::new(&mut output, &limits, &NeverCancel)?;
        document.add_image_page(&mut failing_source, 0, 4, page(), gray(4))
    })()
    .unwrap_err();
    assert!(matches!(error, Error::Io(_)));
    assert_eq!(failing_source.calls, 2);

    let mut source = one_byte_source();
    let mut sink = FailingSink { budget: 60 };
    let error = (|| {
        let mut document = PdfDocument::new(&mut sink, &limits, &NeverCancel)?;
        document.add_image_page(&mut source, 0, 1, page(), gray(1))
    })()
    .unwrap_err();
    assert!(matches!(error, Error::Io(_)));
    Ok(())
}

#[test]
fn large_image_reads_remain_bounded_and_do_not_require_output_vec() -> Result<()> {
    const SIZE: u64 = 1_048_593;
    let mut source = CountingBytes::generated(SIZE);
    let mut sink = CountingSink::default();
    let limits = Limits::default();
    let report = (|| {
        let mut document = PdfDocument::new(&mut sink, &limits, &NeverCancel)?;
        document.add_image_page(&mut source, 0, SIZE, page(), gray(SIZE as u32))?;
        document.finish()
    })()?;
    assert_eq!(report.input_bytes_read, SIZE);
    assert_eq!(report.output_bytes_written, sink.bytes);
    assert!(source.max_request <= limits.io_chunk_bytes);
    assert!(source.reads >= 5);
    Ok(())
}

struct CountingBytes {
    bytes: Option<Vec<u8>>,
    size: u64,
    reads: usize,
    max_request: usize,
}

impl CountingBytes {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            size: bytes.len() as u64,
            bytes: Some(bytes),
            reads: 0,
            max_request: 0,
        }
    }
    fn generated(size: u64) -> Self {
        Self {
            bytes: None,
            size,
            reads: 0,
            max_request: 0,
        }
    }
}

impl RangedSource for CountingBytes {
    fn size(&self) -> u64 {
        self.size
    }
    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        self.reads += 1;
        self.max_request = self.max_request.max(destination.len());
        let available = (self.size - offset) as usize;
        let count = destination.len().min(available);
        match &self.bytes {
            Some(bytes) => destination[..count]
                .copy_from_slice(&bytes[offset as usize..offset as usize + count]),
            None => destination[..count].fill(0x55),
        }
        Ok(count)
    }
}

struct ShortSource {
    calls: usize,
}

impl RangedSource for ShortSource {
    fn size(&self) -> u64 {
        4
    }
    fn read_at(&mut self, _: u64, destination: &mut [u8]) -> Result<usize> {
        self.calls += 1;
        if self.calls == 1 {
            destination[0] = 1;
            Ok(1)
        } else {
            Ok(0)
        }
    }
}

struct FailingSource {
    calls: usize,
}

impl RangedSource for FailingSource {
    fn size(&self) -> u64 {
        4
    }
    fn read_at(&mut self, _: u64, destination: &mut [u8]) -> Result<usize> {
        self.calls += 1;
        if self.calls == 1 {
            destination[0] = 1;
            Ok(1)
        } else {
            Err(Error::Io(io::Error::other("injected source failure")))
        }
    }
}

struct FailingSink {
    budget: usize,
}

impl SequentialSink for FailingSink {
    fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        if self.budget == 0 {
            return Err(Error::Io(io::Error::other("injected sink failure")));
        }
        let accepted = bytes.len().min(self.budget);
        self.budget -= accepted;
        Ok(accepted)
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct CountingSink {
    bytes: u64,
}

impl SequentialSink for CountingSink {
    fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        self.bytes += bytes.len() as u64;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}
