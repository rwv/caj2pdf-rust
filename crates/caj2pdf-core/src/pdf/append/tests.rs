// SPDX-License-Identifier: MIT

use super::*;
use crate::test_support::{CancelAfter, NEVER, run};
use crate::{
    native::{SeekableSource, WriteSink},
    pdf::{ImageEncoding, ImageSpec, PageSpec, PdfDocument},
};
use std::io::{self, Cursor};

fn unoutlined_pdf() -> Result<Vec<u8>> {
    let mut image = SeekableSource::new(Cursor::new(vec![0x7f]))?;
    let mut output = WriteSink::new(Vec::<u8>::new());
    let limits = Limits::default();
    run(async {
        let mut pdf = PdfDocument::new(&mut output, &limits, &NEVER).await?;
        pdf.add_image_page(
            &mut image,
            0,
            1,
            PageSpec {
                width_points: 72.0,
                height_points: 72.0,
            },
            ImageSpec {
                pixel_width: 1,
                pixel_height: 1,
                encoding: ImageEncoding::Gray8,
            },
        )
        .await?;
        pdf.finish().await?;
        Ok::<(), Error>(())
    })?;
    Ok(output.into_inner())
}

fn with_id(mut pdf: Vec<u8>) -> Vec<u8> {
    let needle = b" >>\nstartxref\n";
    let marker = pdf
        .windows(needle.len())
        .rposition(|window| window == needle)
        .expect("generated trailer has a closing dictionary");
    pdf.splice(
        marker..marker + needle.len(),
        b" /ID [<00112233445566778899AABBCCDDEEFF> <00112233445566778899AABBCCDDEEFF>] >>\nstartxref\n"
            .iter()
            .copied(),
    );
    pdf
}

fn import_one(pdf: &[u8], title: &str) -> Result<Vec<u8>> {
    let mut source = SeekableSource::new(Cursor::new(pdf))?;
    let mut output = WriteSink::new(Vec::<u8>::new());
    let limits = Limits::default();
    run(async {
        let index = PdfIndex::open(
            &mut source,
            PdfRange {
                offset: 0,
                length: pdf.len() as u64,
            },
            &limits,
            &NEVER,
        )
        .await?;
        let mut appender =
            PdfOutlineAppender::begin(&mut source, &mut output, &index, &limits, &NEVER).await?;
        appender
            .add_bookmark(Bookmark {
                depth: 0,
                title: title.into(),
                page_index: 0,
            })
            .await?;
        let report = appender.finish().await?;
        assert_eq!(report.pages_converted, 1);
        assert_eq!(report.bookmarks_written, 1);
        Ok::<(), Error>(())
    })?;
    Ok(output.into_inner())
}

fn final_id(pdf: &[u8]) -> &[u8] {
    let trailer = pdf
        .windows(b"trailer\n".len())
        .rposition(|window| window == b"trailer\n")
        .expect("output trailer exists");
    let tail = &pdf[trailer..];
    let id = tail
        .windows(b" /ID [".len())
        .position(|window| window == b" /ID [")
        .expect("output ID exists");
    &tail[id..]
}

#[test]
fn direct_id_parser_accepts_hex_literal_and_comments() {
    assert_eq!(
        first_id_string(b" [ % comment\r\n <0123> (second) ] ", 0).unwrap(),
        b"<0123>"
    );
    assert_eq!(
        first_id_string(br"[(first\)id) <ABCD>]", 0).unwrap(),
        br"(first\)id)"
    );
}

#[test]
fn direct_id_parser_rejects_indirect_and_unterminated_values() {
    for raw in [
        b"[1 0 R <00>]".as_slice(),
        b"[<00> <11>".as_slice(),
        b"[(unterminated <11>]".as_slice(),
    ] {
        assert!(matches!(
            first_id_string(raw, 123),
            Err(Error::Pdf {
                offset: 123,
                kind: PdfErrorKind::UnsupportedFeature,
                ..
            })
        ));
    }
}

#[test]
fn update_id_hash_changes_with_update_bytes() {
    let first = fnv128(FNV128_BASIS, b"[<first> <old>]");
    assert_ne!(fnv128(first, b"outline A"), fnv128(first, b"outline B"));
}

#[test]
fn clean_existing_outline_is_copied_byte_for_byte() -> Result<()> {
    let original = include_bytes!("../../../../../tests/fixtures/valid_nested_outline.pdf");
    let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
    let mut output = WriteSink::new(Vec::<u8>::new());
    let report = run(copy_pdf(
        &mut source,
        &mut output,
        &Limits::default(),
        &NEVER,
    ))?;
    assert_eq!(output.into_inner(), original);
    assert_eq!(report.output_bytes_written, original.len() as u64);
    assert!(report.input_bytes_read >= original.len() as u64);
    assert_eq!(report.pages_converted, 2);
    assert_eq!(report.bookmarks_written, 0);
    Ok(())
}

#[test]
fn importing_into_existing_outline_preserves_original_navigation() -> Result<()> {
    let original = include_bytes!("../../../../../tests/fixtures/valid_nested_outline.pdf");
    let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
    let mut output = WriteSink::new(Vec::<u8>::new());
    let limits = Limits::default();
    let report = run(async {
        let index = PdfIndex::open(
            &mut source,
            PdfRange {
                offset: 0,
                length: original.len() as u64,
            },
            &limits,
            &NEVER,
        )
        .await?;
        let mut appender =
            PdfOutlineAppender::begin(&mut source, &mut output, &index, &limits, &NEVER).await?;
        assert!(appender.preserves_existing_outlines());
        appender
            .add_bookmark(Bookmark {
                depth: 0,
                title: "New title".into(),
                page_index: 999,
            })
            .await?;
        appender.finish().await
    })?;
    assert_eq!(report.bookmarks_written, 0);
    assert_eq!(output.into_inner(), original);
    Ok(())
}

#[test]
fn embedded_pdf_range_is_copied_without_container_bytes() -> Result<()> {
    let original = include_bytes!("../../../../../tests/fixtures/valid_nested_outline.pdf");
    let prefix = b"container bytes before embedded PDF";
    let mut container = prefix.to_vec();
    container.extend_from_slice(original);
    container.extend_from_slice(b"container suffix");
    let mut source = SeekableSource::new(Cursor::new(container))?;
    let mut output = WriteSink::new(Vec::<u8>::new());
    let report = run(copy_pdf_range(
        &mut source,
        &mut output,
        PdfRange {
            offset: prefix.len() as u64,
            length: original.len() as u64,
        },
        &Limits::default(),
        &NEVER,
    ))?;
    assert_eq!(output.into_inner(), original);
    assert_eq!(report.pages_converted, 2);
    Ok(())
}

#[test]
fn new_outline_update_preserves_pdf_prefix_and_changes_id_by_title() -> Result<()> {
    let original = with_id(unoutlined_pdf()?);
    let one = import_one(&original, "AA")?;
    let two = import_one(&original, "BB")?;
    assert!(one.starts_with(&original));
    assert!(two.starts_with(&original));
    assert_ne!(final_id(&one), final_id(&two));
    assert!(String::from_utf8_lossy(&one).contains("/Title <FEFF00410041>"));
    assert!(String::from_utf8_lossy(&two).contains("/Title <FEFF00420042>"));
    Ok(())
}

#[test]
fn nested_siblings_and_long_title_reopen_as_one_outline_tree() -> Result<()> {
    let original = unoutlined_pdf()?;
    let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
    let mut output = WriteSink::new(Vec::<u8>::new());
    let limits = Limits::default();
    let report = run(async {
        let index = PdfIndex::open(
            &mut source,
            PdfRange {
                offset: 0,
                length: original.len() as u64,
            },
            &limits,
            &NEVER,
        )
        .await?;
        let mut appender =
            PdfOutlineAppender::begin(&mut source, &mut output, &index, &limits, &NEVER).await?;
        for (depth, title) in [
            (0, "Root".to_owned()),
            (1, "A".repeat(3000)),
            (1, "第二章".to_owned()),
            (0, "After".to_owned()),
        ] {
            appender
                .add_bookmark(Bookmark {
                    depth,
                    title,
                    page_index: 0,
                })
                .await?;
        }
        appender.finish().await
    })?;
    assert_eq!(report.bookmarks_written, 4);
    let pdf = output.into_inner();
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.contains("/Title <FEFF7B2C4E8C7AE0>"));
    assert_eq!(text.matches(" /Next ").count(), 2);
    assert_eq!(text.matches(" /Prev ").count(), 3); // two item links + trailer
    let mut source = SeekableSource::new(Cursor::new(pdf.as_slice()))?;
    let index = run(PdfIndex::open(
        &mut source,
        PdfRange {
            offset: 0,
            length: pdf.len() as u64,
        },
        &limits,
        &NEVER,
    ))?;
    assert!(index.has_outlines());
    assert_eq!(index.pages().len(), 1);
    Ok(())
}

#[test]
fn bookmark_limits_reject_input_before_new_objects() -> Result<()> {
    let original = unoutlined_pdf()?;
    let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
    let mut output = WriteSink::new(Vec::<u8>::new());
    let limits = Limits {
        max_bookmarks: 1,
        max_allocation_bytes: 256 * 1024,
        ..Limits::default()
    };
    run(async {
        let index = PdfIndex::open(
            &mut source,
            PdfRange {
                offset: 0,
                length: original.len() as u64,
            },
            &limits,
            &NEVER,
        )
        .await?;
        let mut appender =
            PdfOutlineAppender::begin(&mut source, &mut output, &index, &limits, &NEVER).await?;
        assert!(matches!(
            appender
                .add_bookmark(Bookmark {
                    depth: 0,
                    title: "too far".into(),
                    page_index: 1,
                })
                .await,
            Err(Error::InvalidInput { .. })
        ));
        assert!(matches!(
            appender
                .add_bookmark(Bookmark {
                    depth: 0,
                    title: "".into(),
                    page_index: 0,
                })
                .await,
            Err(Error::InvalidInput { .. })
        ));
        assert!(matches!(
            appender
                .add_bookmark(Bookmark {
                    depth: MAX_OUTLINE_DEPTH as u32,
                    title: "deep".into(),
                    page_index: 0,
                })
                .await,
            Err(Error::LimitExceeded {
                resource: "PDF outline depth",
                ..
            })
        ));
        assert!(matches!(
            appender
                .add_bookmark(Bookmark {
                    depth: 0,
                    title: "X".repeat(300_000),
                    page_index: 0,
                })
                .await,
            Err(Error::LimitExceeded { .. })
        ));
        appender
            .add_bookmark(Bookmark {
                depth: 0,
                title: "Allowed".into(),
                page_index: 0,
            })
            .await?;
        assert!(matches!(
            appender
                .add_bookmark(Bookmark {
                    depth: 0,
                    title: "second".into(),
                    page_index: 0,
                })
                .await,
            Err(Error::LimitExceeded {
                resource: "bookmarks",
                ..
            })
        ));
        Ok::<(), Error>(())
    })
}

#[test]
fn copy_output_limit_is_checked_before_sink_writes() -> Result<()> {
    let original = include_bytes!("../../../../../tests/fixtures/valid_nested_outline.pdf");
    let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
    let mut output = WriteSink::new(Vec::<u8>::new());
    let limits = Limits {
        max_output_bytes: original.len() as u64 - 1,
        ..Limits::default()
    };
    assert!(matches!(
        run(copy_pdf(&mut source, &mut output, &limits, &NEVER)),
        Err(Error::PdfLimitExceeded {
            resource: "output bytes",
            object: Some((1, 0)),
            ..
        })
    ));
    assert!(output.into_inner().is_empty());
    Ok(())
}

#[test]
fn invalid_bookmark_rejects_without_claiming_success() -> Result<()> {
    let original = unoutlined_pdf()?;
    let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
    let mut output = WriteSink::new(Vec::<u8>::new());
    let limits = Limits::default();
    run(async {
        let index = PdfIndex::open(
            &mut source,
            PdfRange {
                offset: 0,
                length: original.len() as u64,
            },
            &limits,
            &NEVER,
        )
        .await?;
        let mut appender =
            PdfOutlineAppender::begin(&mut source, &mut output, &index, &limits, &NEVER).await?;
        assert!(matches!(
            appender
                .add_bookmark(Bookmark {
                    depth: 1,
                    title: "orphan".into(),
                    page_index: 0,
                })
                .await,
            Err(Error::InvalidInput { .. })
        ));
        Ok::<(), Error>(())
    })
}

struct FailingSink {
    accepted: usize,
    remaining: usize,
    fail_flush: bool,
}

impl SequentialSink for FailingSink {
    async fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        if self.remaining == 0 {
            return Err(Error::Io(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "injected sink failure",
            )));
        }
        let count = bytes.len().min(self.remaining).min(7);
        self.remaining -= count;
        self.accepted += count;
        Ok(count)
    }

    async fn flush(&mut self) -> Result<()> {
        if self.fail_flush {
            return Err(Error::Io(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "injected flush failure",
            )));
        }
        Ok(())
    }
}

#[test]
fn sink_failure_has_no_success_report() -> Result<()> {
    let original = include_bytes!("../../../../../tests/fixtures/valid_nested_outline.pdf");
    let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
    let mut sink = FailingSink {
        accepted: 0,
        remaining: 35,
        fail_flush: false,
    };
    let result = run(copy_pdf(&mut source, &mut sink, &Limits::default(), &NEVER));
    assert!(matches!(result, Err(Error::Io(_))));
    assert_eq!(sink.accepted, 35);
    Ok(())
}

#[test]
fn flush_failure_has_no_success_report() -> Result<()> {
    let original = include_bytes!("../../../../../tests/fixtures/valid_nested_outline.pdf");
    let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
    let mut sink = FailingSink {
        accepted: 0,
        remaining: original.len(),
        fail_flush: true,
    };
    let result = run(copy_pdf(&mut source, &mut sink, &Limits::default(), &NEVER));
    assert!(matches!(result, Err(Error::Io(_))));
    assert_eq!(sink.accepted, original.len());

    // The same short-write sink succeeds once its flush does.
    let mut sink = FailingSink {
        accepted: 0,
        remaining: original.len(),
        fail_flush: false,
    };
    let report = run(copy_pdf(&mut source, &mut sink, &Limits::default(), &NEVER))?;
    assert_eq!(report.output_bytes_written, original.len() as u64);
    assert_eq!(sink.accepted, original.len());
    Ok(())
}

/// A classic-xref PDF whose objects are numbered `1..=objects.len()`.
/// `gap` is inserted immediately before object `gap.0`.
fn classic_pdf(objects: &[&str], gap: Option<(u32, &[u8])>, trailer_extra: &str) -> Vec<u8> {
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        let number = index as u32 + 1;
        if let Some((_, bytes)) = gap.filter(|(before, _)| *before == number) {
            pdf.extend_from_slice(bytes);
        }
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{number} 0 obj\n{body}\nendobj\n").as_bytes());
    }
    let xref = pdf.len();
    let size = objects.len() + 1;
    pdf.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {size} /Root 1 0 R {trailer_extra} >>\nstartxref\n{xref}\n%%EOF\n"
        )
        .as_bytes(),
    );
    pdf
}

const CATALOG: &str = "<< /Type /Catalog /Pages 2 0 R >>";
const PAGES: &str = "<< /Type /Pages /Kids [3 0 R] /Count 1 >>";
const PAGE: &str = "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 200] >>";

fn open_index(pdf: &[u8], limits: &Limits) -> Result<PdfIndex> {
    let mut source = SeekableSource::new(Cursor::new(pdf))?;
    run(PdfIndex::open(
        &mut source,
        PdfRange {
            offset: 0,
            length: pdf.len() as u64,
        },
        limits,
        &NEVER,
    ))
}

#[test]
fn id_parser_rejects_non_arrays_trailing_values_and_bad_strings() {
    for raw in [
        b"<00> <11>".as_slice(),
        b"[<00> <11>] junk",
        b"[<0G> <11>]",
        b"[<00> <11",
        br"[(ends with escape\",
        b"[(unbalanced (nested) <11>]",
        b"[<< >> <11>]",
    ] {
        assert!(
            matches!(
                first_id_string(raw, 77),
                Err(Error::Pdf {
                    offset: 77,
                    object: None,
                    kind: PdfErrorKind::UnsupportedFeature,
                    reason: "unsupported PDF trailer ID syntax",
                })
            ),
            "accepted {raw:?}"
        );
    }
    assert_eq!(
        first_id_string(b"[(a (nested) b) < 0a 1B >]", 0).unwrap(),
        b"(a (nested) b)"
    );
    assert_eq!(
        first_id_string(b"[<00 11\n22> (x)] % trailing comment", 0).unwrap(),
        b"<00 11\n22>"
    );
}

#[test]
fn update_keeps_trailer_info_and_replaces_an_empty_outline_root() -> Result<()> {
    let original = classic_pdf(
        &[
            "<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R /PageMode /UseNone >>",
            PAGES,
            PAGE,
            "<< /Type /Outlines /Count 0 >>",
            "<< /Producer (unit test) >>",
        ],
        None,
        "/Info 5 0 R",
    );
    let limits = Limits::default();
    let index = open_index(&original, &limits)?;
    assert!(!index.has_outlines());
    let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
    let mut output = WriteSink::new(Vec::<u8>::new());
    let report = run(async {
        let mut appender =
            PdfOutlineAppender::begin(&mut source, &mut output, &index, &limits, &NEVER).await?;
        assert!(!appender.preserves_existing_outlines());
        appender
            .add_bookmark(Bookmark {
                depth: 0,
                title: "Only".into(),
                page_index: 0,
            })
            .await?;
        appender.finish().await
    })?;
    assert_eq!(report.bookmarks_written, 1);
    assert_eq!(report.input_bytes_read, original.len() as u64);
    let pdf = output.into_inner();
    assert_eq!(report.output_bytes_written, pdf.len() as u64);
    assert!(pdf.starts_with(&original));
    let update = String::from_utf8_lossy(&pdf[original.len()..]);
    assert!(
        update.contains(
            "1 0 obj\n<< /Type /Catalog /Pages 2 0 R /PageMode /UseNone /Outlines 6 0 R >>"
        ),
        "{update}"
    );
    assert!(!update.contains("/Outlines 4 0 R"), "{update}");
    assert!(update.contains("6 0 obj\n<< /Type /Outlines /First 7 0 R /Last 7 0 R /Count 1 >>"));
    let xref = original
        .windows(5)
        .position(|window| window == b"xref\n")
        .unwrap();
    assert!(
        update.contains(&format!(
            "trailer\n<< /Size 8 /Root 1 0 R /Prev {xref} /Info 5 0 R >>"
        )),
        "{update}"
    );
    let reopened = open_index(&pdf, &limits)?;
    assert!(reopened.has_outlines());
    assert_eq!(reopened.trailer_info(), index.trailer_info());
    Ok(())
}

#[test]
fn orphan_gap_scrubbing_spans_small_copy_chunks() -> Result<()> {
    let gap = b"4 0 obj\r<\r\n";
    let original = classic_pdf(
        &[CATALOG, PAGES, PAGE, "(live but unreferenced)"],
        Some((4, gap)),
        "",
    );
    let at = original
        .windows(gap.len())
        .position(|window| window == gap)
        .unwrap();
    let limits = Limits {
        io_chunk_bytes: 4,
        ..Limits::default()
    };
    let index = open_index(&original, &limits)?;
    assert_eq!(
        index.gap_patches().len(),
        1,
        "expected one orphan gap patch"
    );
    let patch = &index.gap_patches()[0];
    // The inactive span may include separator whitespace before the
    // aborted header; it must end at the next live object.
    let start = patch.offset as usize;
    let end = start + patch.original.len();
    assert!(start <= at && patch.original.ends_with(gap));
    assert!(original[end..].starts_with(b"4 0 obj\n("));
    // More than two 4-byte chunks, so the scrub crosses chunk boundaries.
    assert!(patch.original.len() > 8);
    let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
    let mut output = WriteSink::new(Vec::<u8>::new());
    let report = run(copy_pdf(&mut source, &mut output, &limits, &NEVER))?;
    let mut expected = original.clone();
    expected[start..end].fill(b' ');
    let copied = output.into_inner();
    assert_eq!(copied, expected);
    assert_eq!(report.output_bytes_written, expected.len() as u64);
    assert_eq!(report.bookmarks_written, 0);
    assert!(
        open_index(&copied, &Limits::default())?
            .gap_patches()
            .is_empty()
    );
    Ok(())
}

#[test]
fn writer_refuses_new_bookmarks_after_an_output_failure() -> Result<()> {
    let original = unoutlined_pdf()?;
    let limits = Limits::default();
    let index = open_index(&original, &limits)?;
    let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
    let mut sink = FailingSink {
        accepted: 0,
        remaining: original.len() + 3,
        fail_flush: false,
    };
    let poisoned = run(async {
        let mut appender =
            PdfOutlineAppender::begin(&mut source, &mut sink, &index, &limits, &NEVER).await?;
        let bookmark = |title: &str| Bookmark {
            depth: 0,
            title: title.into(),
            page_index: 0,
        };
        appender.add_bookmark(bookmark("first")).await?;
        // The second sibling emits the first item and exhausts the sink.
        assert!(matches!(
            appender.add_bookmark(bookmark("second")).await,
            Err(Error::Io(_))
        ));
        let retry = appender.add_bookmark(bookmark("third")).await;
        let finish = appender.finish().await;
        Ok::<_, Error>((retry, finish))
    })?;
    for result in [poisoned.0.map(|_| ()), poisoned.1.map(|_| ())] {
        assert!(matches!(
            result,
            Err(Error::InvalidInput {
                reason: "PDF append writer cannot continue after an output failure"
            })
        ));
    }
    assert_eq!(sink.accepted, original.len() + 3);
    Ok(())
}

#[derive(Default)]
struct RecordingSink {
    bytes: Vec<u8>,
    flushes: u32,
}

impl SequentialSink for RecordingSink {
    async fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    async fn flush(&mut self) -> Result<()> {
        self.flushes += 1;
        Ok(())
    }
}

#[test]
fn cancellation_around_the_final_flush_prevents_a_success_report() -> Result<()> {
    let original = include_bytes!("../../../../../tests/fixtures/valid_nested_outline.pdf");
    let copy = |allowed: u64| -> (Result<ConversionReport>, RecordingSink, u64) {
        let mut source = SeekableSource::new(Cursor::new(original.as_slice())).unwrap();
        let mut sink = RecordingSink::default();
        let cancellation = CancelAfter::new(allowed);
        let result = run(copy_pdf(
            &mut source,
            &mut sink,
            &Limits::default(),
            &cancellation,
        ));
        (result, sink, cancellation.queries())
    };
    let (result, sink, checks) = copy(u64::MAX);
    result?;
    assert_eq!(sink.bytes, original);
    assert_eq!(sink.flushes, 1);

    // The penultimate check precedes the flush; the last one follows it.
    let (before_flush, sink, _) = copy(checks - 2);
    assert!(matches!(before_flush, Err(Error::Cancelled)));
    assert_eq!(sink.bytes, original);
    assert_eq!(sink.flushes, 0);

    let (after_flush, sink, _) = copy(checks - 1);
    assert!(matches!(after_flush, Err(Error::Cancelled)));
    assert_eq!(sink.flushes, 1);
    Ok(())
}

fn invalid(reason: &'static str) -> impl Fn(&Result<()>) -> bool {
    move |result| matches!(result, Err(Error::InvalidInput { reason: actual }) if *actual == reason)
}

#[test]
fn append_writer_rejects_misordered_object_calls() -> Result<()> {
    let limits = Limits::default();
    let mut sink = WriteSink::new(Vec::new());
    let reference = PdfRef {
        number: 7,
        generation: 0,
    };
    run(async {
        let mut writer = AppendWriter::new(&mut sink, &limits, &NEVER);
        let misuse = invalid("invalid PDF append object state or number");
        assert!(misuse(
            &writer
                .begin_object(PdfRef {
                    number: 0,
                    generation: 0,
                })
                .await
        ));
        assert!(invalid("no PDF append object is open")(
            &writer.end_object().await
        ));
        writer.begin_object(reference).await?;
        assert!(misuse(&writer.begin_object(reference).await));
        let open = invalid("PDF append object remains open");
        assert!(open(&writer.finish_copy().await));
        writer.end_object().await
    })?;
    assert_eq!(sink.into_inner(), b"\n7 0 obj\n\nendobj\n");
    Ok(())
}

#[test]
fn append_update_rejects_open_duplicate_and_oversized_state() -> Result<()> {
    let limits = Limits::default();
    let index = open_index(&classic_pdf(&[CATALOG, PAGES, PAGE], None, ""), &limits)?;
    let reference = PdfRef {
        number: 4,
        generation: 0,
    };
    let mut sink = WriteSink::new(Vec::new());
    run(async {
        let mut writer = AppendWriter::new(&mut sink, &limits, &NEVER);
        writer.begin_object(reference).await?;
        assert!(invalid("PDF append object remains open")(
            &writer.finish_update(&index).await
        ));
        writer.end_object().await?;
        writer.begin_object(reference).await?;
        writer.end_object().await?;
        assert!(invalid("PDF update defines an object twice")(
            &writer.finish_update(&index).await
        ));
        Ok::<_, Error>(())
    })?;

    let mut sink = WriteSink::new(Vec::new());
    let oversized = run(async {
        let mut writer = AppendWriter::new(&mut sink, &limits, &NEVER);
        writer.position = MAX_CLASSIC_PDF_BYTES + 1;
        writer.finish_update(&index).await
    });
    assert!(matches!(
        oversized,
        Err(Error::LimitExceeded {
            resource: "classic PDF file bytes",
            attempted,
            ..
        }) if attempted == MAX_CLASSIC_PDF_BYTES + 1
    ));

    let mut sink = WriteSink::new(Vec::new());
    let far_object = run(async {
        let mut writer = AppendWriter::new(&mut sink, &limits, &NEVER);
        writer.entries.push(XrefEntry {
            reference,
            offset: MAX_CLASSIC_PDF_BYTES + 1,
        });
        writer.finish_update(&index).await
    });
    assert!(matches!(
        far_object,
        Err(Error::LimitExceeded {
            resource: "classic PDF object offset",
            ..
        })
    ));
    Ok(())
}

fn copy_patches<'a>(separators: &'a [u64], gaps: &'a [GapPatch]) -> CopyPatches<'a> {
    CopyPatches {
        separators,
        gaps,
        next_separator: 0,
        next_gap: 0,
    }
}

#[test]
fn copy_patches_rewrite_verified_bytes_across_chunks() -> Result<()> {
    let gaps = [GapPatch {
        offset: 3,
        original: b"1 0".to_vec(),
    }];
    let mut patches = copy_patches(&[1], &gaps);
    let mut first = *b"a\rx1";
    patches.apply(&mut first, 0)?;
    assert_eq!(&first, b"a\nx ");
    // The gap continues into the next chunk and must not be skipped.
    assert!(invalid("PDF orphan gap patch exceeds copied prefix")(
        &patches.check_consumed()
    ));
    let mut second = *b" 0z";
    patches.apply(&mut second, 4)?;
    assert_eq!(&second, b"  z");
    patches.check_consumed()
}

#[test]
fn copy_patches_refuse_changed_or_uncopied_bytes() {
    let expect = |result: Result<()>, reason: &'static str| {
        assert!(invalid(reason)(&result), "{result:?}");
    };
    let mut patches = copy_patches(&[1], &[]);
    expect(
        patches.apply(&mut b"a\n".to_owned(), 0),
        "PDF stream separator changed after inspection",
    );
    let mut patches = copy_patches(&[4], &[]);
    patches.apply(&mut b"abc".to_owned(), 0).unwrap();
    expect(
        patches.check_consumed(),
        "PDF stream separator patch exceeds copied prefix",
    );

    let gaps = [GapPatch {
        offset: 1,
        original: b"xy".to_vec(),
    }];
    let mut patches = copy_patches(&[], &gaps);
    expect(
        patches.apply(&mut b"axz".to_owned(), 0),
        "PDF orphan gap changed after inspection",
    );
    let gaps = [GapPatch {
        offset: 8,
        original: b"xy".to_vec(),
    }];
    let mut patches = copy_patches(&[], &gaps);
    patches.apply(&mut b"abc".to_owned(), 0).unwrap();
    expect(
        patches.check_consumed(),
        "PDF orphan gap patch exceeds copied prefix",
    );
}
