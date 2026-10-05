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

#[test]
fn info_dictionary_holds_only_present_values_as_utf16_text() {
    let write = |info: &[(&'static str, Option<&str>)]| {
        let limits = Limits::default();
        let mut sink = VecSink::default();
        let mut source = FilledSource::new(1);
        let result = run(async {
            let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await?;
            document
                .add_image_page(&mut source, 0, 1, page(), gray_pixels(1))
                .await?;
            document.finish_with_info(info).await
        });
        (result, sink.bytes)
    };
    let (_, plain) = write(&[]);
    let (_, absent) = write(&[("Subject", None)]);
    assert_eq!(absent, plain);
    let (report, pdf) = write(&[
        ("Subject", Some("A\u{4e2d}")),
        ("Empty", None),
        ("X_1", Some("")),
    ]);
    let text = String::from_utf8_lossy(&pdf).into_owned();
    assert!(
        text.contains("<< /Subject <FEFF00414E2D> /X_1 <FEFF> >>"),
        "{text}"
    );
    assert!(text.contains(" /Info "), "{text}");
    assert_eq!(report.unwrap().output_bytes_written, pdf.len() as u64);
    assert_eq!(index_pdf(pdf).unwrap().pages().len(), 1);
    for key in ["", "Sub ject", "/Subject"] {
        let (error, pdf) = write(&[(key, Some("x"))]);
        assert!(matches!(
            error,
            Err(Error::InvalidInput {
                reason: "PDF Info key must be a plain ASCII name"
            })
        ));
        assert!(!String::from_utf8_lossy(&pdf).contains("trailer"));
    }
}

#[test]
fn decimal_matrices_preserve_small_values_and_signed_bounds_without_exponents() {
    let values = [
        f64::from_bits(1),
        -f64::from_bits(1),
        f64::MIN_POSITIVE,
        -(MAX_PDF_INTEGER as f64),
        MAX_PDF_INTEGER as f64,
        -0.0,
    ];
    let matrix = DecimalMatrix::new(values).unwrap();
    let text = std::str::from_utf8(matrix.as_bytes()).unwrap();
    assert!(text.len() <= MATRIX_TEXT_BYTES);
    assert!(
        text.bytes()
            .all(|b| b.is_ascii_digit() || b"-. ".contains(&b))
    );
    let words: Vec<_> = text.split_ascii_whitespace().collect();
    assert_eq!(words.len(), 6);
    for (word, value) in words.iter().zip(values) {
        assert_eq!(word.parse::<f64>().unwrap(), value);
    }
    assert_eq!(words[5], "0");
    let small = DecimalMatrix::new([1.0e-20; 6]).unwrap();
    assert!(
        std::str::from_utf8(small.as_bytes())
            .unwrap()
            .contains("0.00000000000000000001")
    );
    let worst = DecimalMatrix::new([-f64::from_bits(1); 6]).unwrap();
    assert_eq!(
        std::str::from_utf8(worst.as_bytes())
            .unwrap()
            .split_ascii_whitespace()
            .count(),
        6
    );
}

#[test]
fn decimal_matrices_reject_nonfinite_and_out_of_profile_components() {
    for bad in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        MAX_PDF_INTEGER as f64 + 1.0,
        -(MAX_PDF_INTEGER as f64 + 1.0),
    ] {
        // Invalid late components must be rejected as well as the first one.
        for index in [0, 5] {
            let mut values = [1.0; 6];
            values[index] = bad;
            assert!(matches!(
                DecimalMatrix::new(values),
                Err(Error::InvalidInput { .. })
            ));
        }
    }
    // Singular matrices are deliberately allowed, with no rounding or epsilon
    // determinant rule. PDF consumers decide their visibility.
    assert_eq!(
        DecimalMatrix::new([0.0; 6]).unwrap().as_bytes(),
        b"0 0 0 0 0 0"
    );
}

#[test]
fn decimal_scratch_refuses_overflow_without_modifying_existing_text() {
    let mut matrix = DecimalMatrix::new([1.0; 6]).unwrap();
    let before = matrix.as_bytes().to_vec();
    assert!(matrix.write_str(&"0".repeat(MATRIX_TEXT_BYTES)).is_err());
    assert_eq!(matrix.as_bytes(), before);

    // Six minimum-subnormal numbers fit. A seventh exceeds the fixed byte
    // capacity: exercise the append helper's actual typed failure, without
    // changing its capacity or injecting a formatting error.
    let mut matrix = DecimalMatrix::new([-f64::from_bits(1); 6]).unwrap();
    let prefix = matrix.as_bytes().to_vec();
    let error = matrix.push(-f64::from_bits(1)).unwrap_err();
    assert!(matches!(
        error,
        Error::InvalidInput {
            reason: "PDF matrix decimal representation exceeds fixed scratch capacity",
        }
    ));
    assert!(matrix.as_bytes().starts_with(&prefix));
    assert!(matrix.as_bytes().len() <= MATRIX_TEXT_BYTES);
}

#[test]
fn affine_preflight_at_leaf_rollover_preserves_existing_pages_and_object_ids() {
    // 256 placed pages reserve 774 objects. Their successor needs a new leaf
    // plus three page objects, exceeding this 777-offset allocation ceiling.
    let limits = Limits {
        io_chunk_bytes: 8,
        max_allocation_bytes: 777 * 8,
        ..Limits::default()
    };
    let mut sink = VecSink::default();
    let mut source = FilledSource::new(1);
    let report = run(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await?;
        let image = document
            .add_image(&mut source, 0, 1, gray_pixels(1))
            .await?;
        let placement = ImagePlacement {
            image,
            transform: [100.0, 0.0, 0.0, -50.0, 0.25, 50.125],
        };
        for index in 0..256 {
            assert_eq!(document.add_placed_page(page(), &[placement]).await?, index);
        }
        let before = document.writer.position();
        let mut invalid = placement;
        invalid.transform[5] = f64::INFINITY;
        assert!(matches!(
            document
                .add_placed_page(page(), &[placement, invalid])
                .await,
            Err(Error::InvalidInput { .. })
        ));
        assert_eq!(document.writer.position(), before);
        assert!(matches!(
            document.add_placed_page(page(), &[placement]).await,
            Err(Error::LimitExceeded {
                resource: "allocation bytes",
                limit: 6216,
                attempted: 6224,
            })
        ));
        assert_eq!(document.writer.position(), before);
        document.finish().await
    })
    .unwrap();
    assert_eq!(report.pages_converted, 256);
    assert_eq!(report.input_bytes_read, 1);
    assert_eq!(report.output_bytes_written, sink.bytes.len() as u64);
    index_pdf(sink.bytes).expect("refused rollover leaves a valid completed page tree");
}

#[test]
fn document_identities_are_unique_and_never_wrap_or_reuse() {
    let counter = AtomicUsize::new(1);
    assert_eq!(next_document_id(&counter).unwrap(), 1);
    assert_eq!(next_document_id(&counter).unwrap(), 2);
    let last = AtomicUsize::new(usize::MAX - 1);
    assert_eq!(next_document_id(&last).unwrap(), usize::MAX - 1);
    for _ in 0..2 {
        assert!(matches!(
            next_document_id(&last),
            Err(Error::LimitExceeded {
                resource: "PDF document identities",
                ..
            })
        ));
        assert_eq!(last.load(Ordering::Relaxed), usize::MAX);
    }
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

#[test]
fn bilevel_compressor_reservation_is_checked_before_opening_an_image() {
    let limits = Limits {
        io_chunk_bytes: 8,
        max_allocation_bytes: DEFLATE_RESERVATION_BYTES - 1,
        ..Limits::default()
    };
    let mut sink = VecSink::default();
    run(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await.unwrap();
        let before = document.writer.position();
        let result = document
            .begin_bilevel_image(BilevelImageSpec {
                pixel_width: 8,
                pixel_height: 1,
                row_stride: 1,
            })
            .await;
        assert!(matches!(
            result,
            Err(Error::LimitExceeded {
                resource: "allocation bytes",
                attempted: DEFLATE_RESERVATION_BYTES,
                ..
            })
        ));
        assert_eq!(document.writer.position(), before);
        assert!(document.writer.ensure_idle().is_ok());
    });
}

#[test]
fn bilevel_compression_is_independent_of_row_and_output_chunk_boundaries() {
    let (visible, stride, height) = (8193, 8196, 7);
    let mut raw = vec![0; stride * height];
    let mut random = 0x12345678_u32;
    for byte in &mut raw {
        random ^= random << 13;
        random ^= random >> 17;
        random ^= random << 5;
        *byte = random as u8;
    }
    let expected: Vec<_> = raw
        .chunks(stride)
        .flat_map(|row| row[..visible].iter().copied())
        .collect();
    let mut reference = None;
    for (chunk, split) in [(1, 1), (7, 13), (16 * 1024, raw.len())] {
        let limits = Limits {
            io_chunk_bytes: chunk,
            ..Limits::default()
        };
        let mut sink = VecSink::default();
        run(async {
            let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await.unwrap();
            let mut image = document
                .begin_bilevel_image(BilevelImageSpec {
                    pixel_width: 65537,
                    pixel_height: height as u32,
                    row_stride: stride,
                })
                .await
                .unwrap();
            assert!(image.deflate.encoded.len() <= DEFLATE_CHUNK_BYTES);
            for bytes in raw.chunks(split) {
                image.write(bytes).await.unwrap();
            }
            assert_eq!(image.deflate.encoder.total_in(), expected.len() as u64);
            let object = image.finish().await.unwrap();
            document.add_page(page(), &[object]).await.unwrap();
            document.finish().await.unwrap();
        });
        assert_eq!(
            crate::test_support::bilevel_pixels(&sink.bytes).as_slice(),
            std::slice::from_ref(&expected)
        );
        if let Some(bytes) = &reference {
            assert_eq!(&sink.bytes, bytes);
        } else {
            reference = Some(sink.bytes);
        }
    }
}

#[test]
fn bilevel_compression_failure_poisons_the_image_and_leaves_the_stream_open() {
    for mode in 0..2 {
        let limits = Limits::default();
        let mut sink = VecSink::default();
        run(async {
            let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await.unwrap();
            let mut image = document
                .begin_bilevel_image(BilevelImageSpec {
                    pixel_width: 8,
                    pixel_height: 1,
                    row_stride: 1,
                })
                .await
                .unwrap();
            if mode == 0 {
                // A broken compressor/output-buffer contract must not spin.
                image.deflate.encoded.clear();
            } else {
                // Simulate a backend entering finalization before row submission.
                assert_eq!(
                    image
                        .deflate
                        .encoder
                        .compress(&[], &mut image.deflate.encoded[..1], FlushCompress::Finish)
                        .unwrap(),
                    Status::Ok
                );
            }
            let error = image.write(&[0x80]).await.unwrap_err();
            let expected = if mode == 0 {
                "zlib compression made no progress"
            } else {
                "zlib compression failed"
            };
            assert!(matches!(error, Error::InvalidInput { reason } if reason == expected));
            assert!(image.failed);
            assert!(matches!(
                image.write(&[0x80]).await,
                Err(Error::InvalidInput {
                    reason: "bilevel image cannot continue after compression or output failure"
                })
            ));
            assert!(image.finish().await.is_err());
            assert!(document.finish().await.is_err());
        });
    }
}

#[test]
fn bilevel_finish_observes_output_limits() {
    let limits = Limits {
        max_output_bytes: 4096,
        ..Limits::default()
    };
    let mut sink = VecSink::default();
    run(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await.unwrap();
        let mut image = document
            .begin_bilevel_image(BilevelImageSpec {
                pixel_width: 8,
                pixel_height: 1,
                row_stride: 1,
            })
            .await
            .unwrap();
        image.write(&[0]).await.unwrap();
        // The final zlib bytes must pass through the PDF output limit.
        image
            .document
            .writer
            .set_position_for_test(limits.max_output_bytes);
        assert!(matches!(
            image.finish().await,
            Err(Error::LimitExceeded {
                resource: "output bytes",
                ..
            })
        ));
        assert!(document.finish().await.is_err());
    });
}

#[test]
fn bilevel_finish_observes_cancellation_while_draining() {
    for allowed in 0.. {
        let limits = Limits {
            io_chunk_bytes: 1,
            ..Limits::default()
        };
        let mut sink = VecSink::default();
        let cancellation = CancelAfter::new(allowed);
        let result = run(async {
            let mut document = PdfDocument::new(&mut sink, &limits, &cancellation).await?;
            let mut image = document
                .begin_bilevel_image(BilevelImageSpec {
                    pixel_width: 8,
                    pixel_height: 1,
                    row_stride: 1,
                })
                .await?;
            image.write(&[0]).await?;
            Ok::<_, Error>(image.finish().await)
        });
        match result {
            Ok(Err(Error::Cancelled)) => return,
            Err(Error::Cancelled) => {}
            other => panic!("expected a cancellation checkpoint during finish: {other:?}"),
        }
    }
}
