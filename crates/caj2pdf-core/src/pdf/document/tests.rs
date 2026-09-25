// SPDX-License-Identifier: MIT

use super::*;
use crate::pdf::{MAX_CLASSIC_PDF_BYTES, PdfIndex, PdfRange};
use crate::test_support::{CancelAfter, NEVER, run};
use std::io;

/// A source whose bytes are all `0x5A`, optionally claiming to have read
/// one byte more than the caller requested.
struct FilledSource {
    size: u64,
    over_report: bool,
}

impl FilledSource {
    fn new(size: u64) -> Self {
        Self {
            size,
            over_report: false,
        }
    }
}

impl RangedSource for FilledSource {
    fn size(&self) -> u64 {
        self.size
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        let available = self.size.saturating_sub(offset);
        let copied = destination.len().min(available as usize);
        destination[..copied].fill(0x5a);
        Ok(copied + usize::from(self.over_report))
    }
}

#[derive(Default)]
struct VecSink {
    bytes: Vec<u8>,
    writes: usize,
    fail_at_write: Option<usize>,
}

impl SequentialSink for VecSink {
    async fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        self.writes += 1;
        if self.fail_at_write == Some(self.writes) {
            return Err(Error::Io(io::Error::other("injected sink failure")));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    async fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

fn page() -> PageSpec {
    PageSpec {
        width_points: 100.0,
        height_points: 50.0,
    }
}

fn gray_pixels(pixel_width: u32) -> ImageSpec {
    ImageSpec {
        pixel_width,
        pixel_height: 1,
        encoding: ImageEncoding::Gray8,
    }
}

fn bookmark(depth: u32, title: &str) -> Bookmark {
    Bookmark {
        depth,
        title: title.into(),
        page_index: 0,
    }
}

fn index_pdf(bytes: Vec<u8>) -> Result<PdfIndex> {
    let length = bytes.len() as u64;
    let mut source = crate::native::SeekableSource::new(io::Cursor::new(bytes))?;
    run(PdfIndex::open(
        &mut source,
        PdfRange { offset: 0, length },
        &Limits::default(),
        &NEVER,
    ))
}

async fn write_sample<W: SequentialSink, C: Cancellation>(
    sink: &mut W,
    limits: &Limits,
    cancellation: &C,
) -> Result<ConversionReport> {
    let mut source = FilledSource::new(3);
    let mut document = PdfDocument::new(sink, limits, cancellation).await?;
    for _ in 0..2 {
        document
            .add_image_page(&mut source, 0, 3, page(), gray_pixels(3))
            .await?;
    }
    document.add_bookmark(bookmark(0, "Root")).await?;
    document.add_bookmark(bookmark(1, "Child")).await?;
    document.add_bookmark(bookmark(0, "Next")).await?;
    document.finish().await
}

#[test]
fn image_source_that_over_reports_is_rejected() {
    let mut source = FilledSource {
        over_report: true,
        ..FilledSource::new(4)
    };
    let mut sink = VecSink::default();
    let limits = Limits::default();
    let error = run(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await?;
        document
            .add_image_page(&mut source, 0, 4, page(), gray_pixels(4))
            .await
    })
    .unwrap_err();
    assert!(matches!(
        error,
        Error::InvalidInput {
            reason: "image source reported more bytes than requested"
        }
    ));
}

#[test]
fn jpeg_stream_longer_than_a_pdf_integer_is_rejected_before_reading() {
    let length = MAX_PDF_INTEGER + 1;
    let mut source = FilledSource::new(length);
    let mut sink = VecSink::default();
    let limits = Limits::default();
    let (error, header_bytes) = run(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await?;
        let header_bytes = document.writer.position();
        let image = ImageSpec {
            pixel_width: 10,
            pixel_height: 10,
            encoding: ImageEncoding::JpegRgb8,
        };
        let error = document
            .add_image_page(&mut source, 0, length, page(), image)
            .await
            .unwrap_err();
        assert_eq!(document.input_bytes_read, 0);
        Ok::<_, Error>((error, header_bytes))
    })
    .unwrap();
    assert!(matches!(
        error,
        Error::LimitExceeded {
            resource: "PDF image stream bytes",
            limit: MAX_PDF_INTEGER,
            attempted,
        } if attempted == length
    ));
    assert_eq!(sink.bytes.len() as u64, header_bytes);
}

#[test]
fn page_tree_capacity_is_checked_before_any_page_output() {
    let mut source = FilledSource::new(1);
    let mut sink = VecSink::default();
    let limits = Limits {
        max_pages: u32::MAX,
        ..Limits::default()
    };
    let error = run(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await?;
        document.pages_written = MAX_TREE_PAGES as u32;
        document
            .add_image_page(&mut source, 0, 1, page(), gray_pixels(1))
            .await
    })
    .unwrap_err();
    assert!(matches!(
        error,
        Error::LimitExceeded {
            resource: "PDF page-tree capacity",
            limit: MAX_TREE_PAGES,
            attempted,
        } if attempted == MAX_TREE_PAGES + 1
    ));
}

/// Counts output bytes and keeps only page-tree node objects, so a very
/// large document can be checked without retaining or rescanning it.
#[derive(Default)]
struct PageTreeSink {
    written: u64,
    object: Vec<u8>,
    page_nodes: Vec<Vec<u8>>,
}

impl SequentialSink for PageTreeSink {
    async fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        // Page-tree nodes are small; larger runs (the xref table) are
        // never page-tree objects and need not be retained.
        if self.object.len() > 64 * 1024 {
            self.object.clear();
        }
        self.written += bytes.len() as u64;
        self.object.extend_from_slice(bytes);
        if self.object.ends_with(b"\nendobj\n") {
            let body = self
                .object
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(&[][..], |header| &self.object[header + 1..]);
            if body.starts_with(b"<< /Type /Pages ") {
                self.page_nodes.push(body.to_vec());
            }
            self.object.clear();
        }
        Ok(bytes.len())
    }

    async fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

/// A one-pixel image that many pages can share, which keeps documents with
/// full page-tree nodes cheap to build.
async fn shared_image<W: SequentialSink>(
    document: &mut PdfDocument<'_, W, CancelAfter>,
) -> Result<ImageObject> {
    let mut writer = document
        .begin_bilevel_image(BilevelImageSpec {
            pixel_width: 1,
            pixel_height: 1,
            row_stride: 1,
        })
        .await?;
    writer.write(&[0]).await?;
    writer.finish().await
}

#[test]
fn full_middle_node_starts_a_second_root_kid() -> Result<()> {
    const PER_MIDDLE: usize = PAGE_TREE_FANOUT * PAGE_TREE_FANOUT;
    const PAGES: u32 = PER_MIDDLE as u32 + 1;
    let mut sink = PageTreeSink::default();
    let limits = Limits::default();
    let report = run(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await?;
        let image = shared_image(&mut document).await?;
        for expected in 0..PAGES {
            let index = document.add_page(page(), &[image]).await?;
            assert_eq!(index, expected);
        }
        assert_eq!(document.root_children.len(), 1);
        document.finish().await
    })?;
    assert_eq!(report.pages_converted, PAGES);
    assert_eq!(report.output_bytes_written, sink.written);
    // 257 leaves, two middle nodes, and the root.
    assert_eq!(sink.page_nodes.len(), 257 + 2 + 1);
    let root_prefix = format!("<< /Type /Pages /Count {PAGES} /Kids [");
    let roots: Vec<_> = sink
        .page_nodes
        .iter()
        .filter(|node| node.starts_with(root_prefix.as_bytes()))
        .collect();
    assert_eq!(roots.len(), 1);
    let root_kids = roots[0]
        .windows(b" 0 R".len())
        .filter(|window| *window == b" 0 R")
        .count();
    assert_eq!(root_kids, 2);
    let full_middle = format!("/Count {PER_MIDDLE} /Kids [");
    let full_middles = sink
        .page_nodes
        .iter()
        .filter(|node| {
            node.windows(full_middle.len())
                .any(|window| window == full_middle.as_bytes())
        })
        .count();
    assert_eq!(full_middles, 1);
    Ok(())
}

#[test]
fn page_nodes_lost_to_failed_writes_make_finish_fail() -> Result<()> {
    const PER_MIDDLE: usize = PAGE_TREE_FANOUT * PAGE_TREE_FANOUT;
    let mut sink = PageTreeSink::default();
    let limits = Limits::default();
    let (leaf_failure, middle_failure, finish) = run(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await?;
        let image = shared_image(&mut document).await?;
        for _ in 0..PER_MIDDLE {
            document.add_page(page(), &[image]).await?;
        }
        // Writing a closed node would pass the classic xref limit, which
        // fails before the sink sees a byte and leaves the writer usable.
        let position = document.writer.position();
        document.writer.set_position_for_test(MAX_CLASSIC_PDF_BYTES);
        // The full leaf is detached before its write fails, and on the
        // next page the full middle node is.
        let leaf_failure = document.add_page(page(), &[image]).await;
        let middle_failure = document.add_page(page(), &[image]).await;
        document.writer.set_position_for_test(position);
        Ok::<_, Error>((leaf_failure, middle_failure, document.finish().await))
    })?;
    for failure in [leaf_failure.map(|_| ()), middle_failure.map(|_| ())] {
        assert!(matches!(
            failure,
            Err(Error::LimitExceeded {
                resource: "classic PDF file bytes",
                ..
            })
        ));
    }
    // Neither lost node is written, so no PDF with dangling kids completes.
    assert!(matches!(
        finish,
        Err(Error::InvalidInput {
            reason: "a reserved PDF object has not been written"
        })
    ));
    Ok(())
}

#[test]
fn outline_titles_are_chunked_and_empty_titles_stay_well_formed() -> Result<()> {
    let long_title = "\u{4E2D}".repeat(3000);
    let mut source = FilledSource::new(1);
    let mut sink = VecSink::default();
    let limits = Limits::default();
    let report = run(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await?;
        document
            .add_image_page(&mut source, 0, 1, page(), gray_pixels(1))
            .await?;
        document.add_bookmark(bookmark(0, &long_title)).await?;
        BookmarkVisitor::visit(&mut document, bookmark(1, "")).await?;
        document.finish().await
    })?;
    assert_eq!(report.bookmarks_written, 2);
    let text = String::from_utf8_lossy(&sink.bytes);
    assert!(text.contains(&format!("/Title <FEFF{}>", "4E2D".repeat(3000))));
    assert!(text.contains("/Title <FEFF> /Parent"));
    assert!(text.contains(" /Count 1 >>"));
    let index = index_pdf(sink.bytes)?;
    assert!(index.has_outlines());
    Ok(())
}

#[test]
fn every_sink_failure_point_returns_the_io_error() {
    let limits = Limits::default();
    let mut clean = VecSink::default();
    run(write_sample(&mut clean, &limits, &NEVER)).unwrap();
    assert!(clean.writes > 20);
    for fail_at in 1..=clean.writes {
        let mut sink = VecSink {
            fail_at_write: Some(fail_at),
            ..VecSink::default()
        };
        let result = run(write_sample(&mut sink, &limits, &NEVER));
        assert!(
            matches!(&result, Err(Error::Io(error)) if error.to_string() == "injected sink failure"),
            "write {fail_at}: {result:?}"
        );
        assert_eq!(sink.writes, fail_at);
        assert!(clean.bytes.starts_with(&sink.bytes));
    }
}

#[test]
fn cancellation_at_every_checkpoint_never_reports_success() {
    let limits = Limits::default();
    let mut allowed = 0;
    loop {
        let mut sink = VecSink::default();
        let cancellation = CancelAfter::new(allowed);
        let report = match run(write_sample(&mut sink, &limits, &cancellation)) {
            Err(Error::Cancelled) => {
                allowed += 1;
                continue;
            }
            result => result.expect("only cancellation may stop the run"),
        };
        assert!(allowed >= 20, "only {allowed} cancellation checks");
        assert_eq!(report.output_bytes_written, sink.bytes.len() as u64);
        break;
    }
}

#[test]
fn reserve_bounded_enforces_item_and_byte_ceilings() {
    let limits = Limits {
        io_chunk_bytes: 16,
        max_allocation_bytes: 64,
        ..Limits::default()
    };
    let mut values: Vec<u64> = Vec::new();
    for _ in 0..3 {
        reserve_bounded(&mut values, 3, &limits, "test items").unwrap();
        values.push(0);
    }
    assert!(matches!(
        reserve_bounded(&mut values, 3, &limits, "test items"),
        Err(Error::LimitExceeded {
            resource: "test items",
            limit: 3,
            attempted: 4,
        })
    ));

    let mut wide: Vec<[u8; 40]> = vec![[0; 40]];
    assert!(matches!(
        reserve_bounded(&mut wide, 10, &limits, "wide items"),
        Err(Error::LimitExceeded {
            resource: "allocation bytes",
            limit: 64,
            attempted: 80,
        })
    ));
    assert_eq!(wide.len(), 1);
}

#[test]
fn a_refused_bookmark_leaves_earlier_outline_items_writable() {
    // Walk the allocation ceiling across every refusal point of a nested
    // outline. Whenever a bookmark is refused, the items accepted before it
    // must still finish as a valid outline tree.
    let titles = [(0, "A"), (1, "B"), (2, "C"), (1, "D"), (0, "E"), (1, "F")];
    let mut refusals_checked = 0;
    for max_allocation_bytes in (64..4096).step_by(8) {
        let limits = Limits {
            io_chunk_bytes: 16,
            max_allocation_bytes,
            ..Limits::default()
        };
        let mut source = FilledSource::new(1);
        let mut sink = VecSink::default();
        let result = run(async {
            let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await?;
            document
                .add_image_page(&mut source, 0, 1, page(), gray_pixels(1))
                .await?;
            let mut accepted = 0;
            for (depth, title) in titles {
                if document.add_bookmark(bookmark(depth, title)).await.is_err() {
                    break;
                }
                accepted += 1;
            }
            Ok::<_, Error>((accepted, document.finish().await))
        });
        let Ok((accepted, Ok(report))) = result else {
            continue;
        };
        if (1..titles.len()).contains(&accepted) {
            assert_eq!(report.bookmarks_written, accepted as u32);
            assert!(index_pdf(sink.bytes).unwrap().has_outlines());
            refusals_checked += 1;
        }
    }
    assert!(refusals_checked > 0);
}

#[test]
fn a_bookmark_that_fails_while_closing_items_stops_the_outline() -> Result<()> {
    let limits = Limits::default();
    let mut source = FilledSource::new(1);
    let mut sink = VecSink::default();
    let (failed, retry, finish) = run(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await?;
        document
            .add_image_page(&mut source, 0, 1, page(), gray_pixels(1))
            .await?;
        document.add_bookmark(bookmark(0, "A")).await?;
        document.add_bookmark(bookmark(1, "B")).await?;
        // Writing the closed items would pass the classic xref limit, which
        // fails before the sink sees a byte and so leaves the writer usable.
        let position = document.writer.position();
        document.writer.set_position_for_test(MAX_CLASSIC_PDF_BYTES);
        let failed = document.add_bookmark(bookmark(0, "C")).await;
        document.writer.set_position_for_test(position);
        let retry = document.add_bookmark(bookmark(0, "D")).await;
        Ok::<_, Error>((failed, retry, document.finish().await))
    })?;
    assert!(matches!(
        failed,
        Err(Error::LimitExceeded {
            resource: "classic PDF file bytes",
            ..
        })
    ));
    for result in [retry, finish.map(|_| ())] {
        assert!(matches!(
            result,
            Err(Error::InvalidInput {
                reason: "PDF outline cannot continue after a failed bookmark operation"
            })
        ));
    }
    assert!(!sink.bytes.ends_with(b"%%EOF\n"));
    Ok(())
}
