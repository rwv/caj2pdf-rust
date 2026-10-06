// SPDX-License-Identifier: MIT

use super::*;
use crate::pdf::MAX_CLASSIC_PDF_BYTES;
use crate::test_support::{CancelAfter, NEVER};
use crate::{Context, ErrorKind};
use crate::{
    native::SeekableSource,
    pdf::{ImageEncoding, ImageSpec, PageSpec, PdfDocument},
};
use std::io::Write;
use std::io::{self, Cursor};

fn unoutlined_pdf() -> Result<Vec<u8>> {
    let mut image = SeekableSource::new(Cursor::new(vec![0x7f]))?;
    let mut output = Vec::<u8>::new();
    let limits = Limits::default();
    (|| {
        let mut pdf = PdfDocument::new(&mut output, &limits, &NEVER)?;
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
        )?;
        pdf.finish()?;
        Ok::<(), Error>(())
    })()?;
    Ok(output)
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
    let mut output = Vec::<u8>::new();
    let limits = Limits::default();
    (|| {
        let index = PdfIndex::open(
            &mut source,
            PdfRange {
                offset: 0,
                length: pdf.len() as u64,
            },
            &limits,
            &NEVER,
        )?;
        let mut appender =
            PdfOutlineAppender::begin(&mut source, &mut output, &index, &limits, &NEVER)?;
        appender.add_bookmark(Bookmark {
            depth: 0,
            title: title.into(),
            page_index: 0,
        })?;
        let report = appender.finish()?;
        assert_eq!(report.pages_converted, 1);
        assert_eq!(report.bookmarks_written, 1);
        Ok::<(), Error>(())
    })()?;
    Ok(output)
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
fn update_id_changes_with_the_old_id_and_the_update_position() {
    let id = update_id(b"[<first> <old>]", 100, 200);
    assert_eq!(id, update_id(b"[<first> <old>]", 100, 200));
    assert_ne!(id, update_id(b"[<first> <new>]", 100, 200));
    assert_ne!(id, update_id(b"[<first> <old>]", 101, 200));
    assert_ne!(id, update_id(b"[<first> <old>]", 100, 201));
}

#[test]
fn clean_existing_outline_is_copied_byte_for_byte() -> Result<()> {
    let original = include_bytes!("../../../../../tests/fixtures/valid_nested_outline.pdf");
    let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
    let mut output = Vec::<u8>::new();
    let report = copy_pdf(&mut source, &mut output, &Limits::default(), &NEVER)?;
    assert_eq!(output, original);
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
    let mut output = Vec::<u8>::new();
    let limits = Limits::default();
    let report = (|| {
        let index = PdfIndex::open(
            &mut source,
            PdfRange {
                offset: 0,
                length: original.len() as u64,
            },
            &limits,
            &NEVER,
        )?;
        let mut appender =
            PdfOutlineAppender::begin(&mut source, &mut output, &index, &limits, &NEVER)?;
        assert!(appender.preserves_existing_outlines());
        appender.add_bookmark(Bookmark {
            depth: 0,
            title: "New title".into(),
            page_index: 999,
        })?;
        appender.finish()
    })()?;
    assert_eq!(report.bookmarks_written, 0);
    assert_eq!(output, original);
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
    let mut output = Vec::<u8>::new();
    let report = copy_pdf_range(
        &mut source,
        &mut output,
        PdfRange {
            offset: prefix.len() as u64,
            length: original.len() as u64,
        },
        &Limits::default(),
        &NEVER,
    )?;
    assert_eq!(output, original);
    assert_eq!(report.pages_converted, 2);
    Ok(())
}

#[test]
fn new_outline_update_preserves_pdf_prefix_and_changes_id_with_its_size() -> Result<()> {
    let original = with_id(unoutlined_pdf()?);
    let one = import_one(&original, "A")?;
    let two = import_one(&original, "BB")?;
    assert!(one.starts_with(&original));
    assert!(two.starts_with(&original));
    assert_ne!(final_id(&one), final_id(&two));
    assert!(final_id(&one).starts_with(b" /ID [<00112233445566778899AABBCCDDEEFF> <"));
    assert!(String::from_utf8_lossy(&one).contains("/Title <FEFF0041>"));
    assert!(String::from_utf8_lossy(&two).contains("/Title <FEFF00420042>"));
    Ok(())
}

#[test]
fn nested_siblings_and_long_title_reopen_as_one_outline_tree() -> Result<()> {
    let original = unoutlined_pdf()?;
    let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
    let mut output = Vec::<u8>::new();
    let limits = Limits::default();
    let report = (|| {
        let index = PdfIndex::open(
            &mut source,
            PdfRange {
                offset: 0,
                length: original.len() as u64,
            },
            &limits,
            &NEVER,
        )?;
        let mut appender =
            PdfOutlineAppender::begin(&mut source, &mut output, &index, &limits, &NEVER)?;
        for (depth, title) in [
            (0, "Root".to_owned()),
            (1, "A".repeat(3000)),
            (1, "第二章".to_owned()),
            (0, "After".to_owned()),
        ] {
            appender.add_bookmark(Bookmark {
                depth,
                title,
                page_index: 0,
            })?;
        }
        appender.finish()
    })()?;
    assert_eq!(report.bookmarks_written, 4);
    let pdf = output;
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.contains("/Title <FEFF7B2C4E8C7AE0>"));
    assert_eq!(text.matches(" /Next ").count(), 2);
    assert_eq!(text.matches(" /Prev ").count(), 3); // two item links + trailer
    let mut source = SeekableSource::new(Cursor::new(pdf.as_slice()))?;
    let index = PdfIndex::open(
        &mut source,
        PdfRange {
            offset: 0,
            length: pdf.len() as u64,
        },
        &limits,
        &NEVER,
    )?;
    assert!(index.has_outlines());
    assert_eq!(index.pages().len(), 1);
    Ok(())
}

#[test]
fn bookmark_limits_reject_input_before_new_objects() -> Result<()> {
    let original = unoutlined_pdf()?;
    let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
    let mut output = Vec::<u8>::new();
    let limits = Limits {
        max_bookmarks: 1,
        max_allocation_bytes: 256 * 1024,
        ..Limits::default()
    };
    (|| {
        let index = PdfIndex::open(
            &mut source,
            PdfRange {
                offset: 0,
                length: original.len() as u64,
            },
            &limits,
            &NEVER,
        )?;
        let mut appender =
            PdfOutlineAppender::begin(&mut source, &mut output, &index, &limits, &NEVER)?;
        assert!(matches!(
            appender.add_bookmark(Bookmark {
                depth: 0,
                title: "too far".into(),
                page_index: 1,
            }),
            Err(Error {
                kind: ErrorKind::Malformed,
                ..
            })
        ));
        assert!(matches!(
            appender.add_bookmark(Bookmark {
                depth: 0,
                title: "".into(),
                page_index: 0,
            }),
            Err(Error {
                kind: ErrorKind::Malformed,
                ..
            })
        ));
        assert!(matches!(
            appender.add_bookmark(Bookmark {
                depth: MAX_OUTLINE_DEPTH as u32,
                title: "deep".into(),
                page_index: 0,
            }),
            Err(Error {
                kind: ErrorKind::LimitExceeded {
                    resource: "PDF outline depth",
                    ..
                },
                ..
            })
        ));
        assert!(matches!(
            appender.add_bookmark(Bookmark {
                depth: 0,
                title: "X".repeat(300_000),
                page_index: 0,
            }),
            Err(Error {
                kind: ErrorKind::LimitExceeded { .. },
                ..
            })
        ));
        appender.add_bookmark(Bookmark {
            depth: 0,
            title: "Allowed".into(),
            page_index: 0,
        })?;
        assert!(matches!(
            appender.add_bookmark(Bookmark {
                depth: 0,
                title: "second".into(),
                page_index: 0,
            }),
            Err(Error {
                kind: ErrorKind::LimitExceeded {
                    resource: "bookmarks",
                    ..
                },
                ..
            })
        ));
        Ok::<(), Error>(())
    })()
}

#[test]
fn copy_output_limit_is_checked_before_sink_writes() -> Result<()> {
    let original = include_bytes!("../../../../../tests/fixtures/valid_nested_outline.pdf");
    let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
    let mut output = Vec::<u8>::new();
    let limits = Limits {
        max_output_bytes: original.len() as u64 - 1,
        ..Limits::default()
    };
    assert!(matches!(
        copy_pdf(&mut source, &mut output, &limits, &NEVER),
        Err(Error {
            kind: ErrorKind::LimitExceeded {
                resource: "output bytes",
                ..
            },
            context: Context::Pdf {
                object: Some((1, 0)),
                ..
            },
            ..
        })
    ));
    assert!(output.is_empty());
    Ok(())
}

#[test]
fn invalid_bookmark_rejects_without_claiming_success() -> Result<()> {
    let original = unoutlined_pdf()?;
    let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
    let mut output = Vec::<u8>::new();
    let limits = Limits::default();
    (|| {
        let index = PdfIndex::open(
            &mut source,
            PdfRange {
                offset: 0,
                length: original.len() as u64,
            },
            &limits,
            &NEVER,
        )?;
        let mut appender =
            PdfOutlineAppender::begin(&mut source, &mut output, &index, &limits, &NEVER)?;
        assert!(matches!(
            appender.add_bookmark(Bookmark {
                depth: 1,
                title: "orphan".into(),
                page_index: 0,
            }),
            Err(Error {
                kind: ErrorKind::Malformed,
                ..
            })
        ));
        Ok::<(), Error>(())
    })()
}

#[test]
fn new_outline_objects_stop_at_the_pdf_object_number_limit() -> Result<()> {
    let original = unoutlined_pdf()?;
    let limits = Limits::default();
    let index = open_index(&original, &limits)?;
    let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
    let mut output = Vec::<u8>::new();
    let result = (|| {
        let mut appender =
            PdfOutlineAppender::begin(&mut source, &mut output, &index, &limits, &NEVER)?;
        appender.writer.next_number = Some(MAX_PDF_OBJECTS + 1);
        Ok::<_, Error>(appender.add_bookmark(Bookmark {
            depth: 0,
            title: "late".into(),
            page_index: 0,
        }))
    })()?;
    assert!(matches!(
        result,
        Err(Error { kind: ErrorKind::LimitExceeded { resource: "PDF object number", attempted, .. }, .. }) if attempted == u64::from(MAX_PDF_OBJECTS) + 1
    ));
    Ok(())
}

/// The one custom sink type of these tests, so failure, short-write, and
/// flush-recording runs share an appender instantiation. Writes accept at
/// most `max_write` bytes and fail once `remaining` is exhausted.
struct TestSink {
    bytes: Vec<u8>,
    remaining: usize,
    max_write: usize,
    fail_flush: bool,
    flushes: u32,
}

impl TestSink {
    /// Accepts `remaining` bytes in short writes, then fails.
    fn failing(remaining: usize, fail_flush: bool) -> Self {
        Self {
            bytes: Vec::new(),
            remaining,
            max_write: 7,
            fail_flush,
            flushes: 0,
        }
    }

    /// Accepts every write whole and counts flushes.
    fn recording() -> Self {
        Self {
            max_write: usize::MAX,
            ..Self::failing(usize::MAX, false)
        }
    }
}

impl Write for TestSink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.remaining == 0 {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "injected sink failure",
            ));
        }
        let count = bytes.len().min(self.remaining).min(self.max_write);
        self.remaining -= count;
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if self.fail_flush {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "injected flush failure",
            ));
        }
        self.flushes += 1;
        Ok(())
    }
}

#[test]
fn sink_failure_has_no_success_report() -> Result<()> {
    let original = include_bytes!("../../../../../tests/fixtures/valid_nested_outline.pdf");
    let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
    let mut sink = TestSink::failing(35, false);
    let result = copy_pdf(&mut source, &mut sink, &Limits::default(), &NEVER);
    assert!(matches!(
        result,
        Err(Error {
            kind: ErrorKind::Io(_),
            ..
        })
    ));
    assert_eq!(sink.bytes.len(), 35);
    Ok(())
}

#[test]
fn flush_failure_has_no_success_report() -> Result<()> {
    let original = include_bytes!("../../../../../tests/fixtures/valid_nested_outline.pdf");
    let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
    let mut sink = TestSink::failing(original.len(), true);
    let result = copy_pdf(&mut source, &mut sink, &Limits::default(), &NEVER);
    assert!(matches!(
        result,
        Err(Error {
            kind: ErrorKind::Io(_),
            ..
        })
    ));
    assert_eq!(sink.bytes.len(), original.len());

    // The same short-write sink succeeds once its flush does.
    let mut sink = TestSink::failing(original.len(), false);
    let report = copy_pdf(&mut source, &mut sink, &Limits::default(), &NEVER)?;
    assert_eq!(report.output_bytes_written, original.len() as u64);
    assert_eq!(sink.bytes.len(), original.len());
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
    PdfIndex::open(
        &mut source,
        PdfRange {
            offset: 0,
            length: pdf.len() as u64,
        },
        limits,
        &NEVER,
    )
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
    let mut output = Vec::<u8>::new();
    let report = (|| {
        let mut appender =
            PdfOutlineAppender::begin(&mut source, &mut output, &index, &limits, &NEVER)?;
        assert!(!appender.preserves_existing_outlines());
        appender.add_bookmark(Bookmark {
            depth: 0,
            title: "Only".into(),
            page_index: 0,
        })?;
        appender.finish()
    })()?;
    assert_eq!(report.bookmarks_written, 1);
    assert_eq!(report.input_bytes_read, original.len() as u64);
    let pdf = output;
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
    let mut output = Vec::<u8>::new();
    let report = copy_pdf(&mut source, &mut output, &limits, &NEVER)?;
    let mut expected = original.clone();
    expected[start..end].fill(b' ');
    let copied = output;
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
    let mut sink = TestSink::failing(original.len() + 3, false);
    let poisoned = (|| {
        let mut appender =
            PdfOutlineAppender::begin(&mut source, &mut sink, &index, &limits, &NEVER)?;
        let bookmark = |title: &str| Bookmark {
            depth: 0,
            title: title.into(),
            page_index: 0,
        };
        appender.add_bookmark(bookmark("first"))?;
        // The second sibling emits the first item and exhausts the sink.
        assert!(matches!(
            appender.add_bookmark(bookmark("second")),
            Err(Error {
                kind: ErrorKind::Io(_),
                ..
            })
        ));
        let retry = appender.add_bookmark(bookmark("third"));
        let finish = appender.finish();
        Ok::<_, Error>((retry, finish))
    })()?;
    for result in [poisoned.0.map(|_| ()), poisoned.1.map(|_| ())] {
        assert!(matches!(
            result,
            Err(Error {
                kind: ErrorKind::Malformed,
                reason: "PDF writer cannot continue after a sink failure",
                ..
            })
        ));
    }
    assert_eq!(sink.bytes.len(), original.len() + 3);
    Ok(())
}

#[test]
fn cancellation_around_the_final_flush_prevents_a_success_report() -> Result<()> {
    let original = include_bytes!("../../../../../tests/fixtures/valid_nested_outline.pdf");
    let copy = |allowed: u64| -> (Result<ConversionReport>, TestSink, u64) {
        let mut source = SeekableSource::new(Cursor::new(original.as_slice())).unwrap();
        let mut sink = TestSink::recording();
        let cancellation = CancelAfter::new(allowed);
        let result = copy_pdf(&mut source, &mut sink, &Limits::default(), &cancellation);
        (result, sink, cancellation.queries())
    };
    let (result, sink, checks) = copy(u64::MAX);
    result?;
    assert_eq!(sink.bytes, original);
    assert_eq!(sink.flushes, 1);

    // The penultimate check precedes the flush; the last one follows it.
    let (before_flush, sink, _) = copy(checks - 2);
    assert!(matches!(
        before_flush,
        Err(Error {
            kind: ErrorKind::Cancelled,
            ..
        })
    ));
    assert_eq!(sink.bytes, original);
    assert_eq!(sink.flushes, 0);

    let (after_flush, sink, _) = copy(checks - 1);
    assert!(matches!(
        after_flush,
        Err(Error {
            kind: ErrorKind::Cancelled,
            ..
        })
    ));
    assert_eq!(sink.flushes, 1);
    Ok(())
}

fn invalid(reason: &'static str) -> impl Fn(&Result<()>) -> bool {
    move |result| matches!(result, Err(Error { kind: ErrorKind::Malformed, reason: actual, .. }) if *actual == reason)
}

#[test]
fn append_writer_rejects_misordered_object_calls() -> Result<()> {
    let limits = Limits::default();
    let index = open_index(&classic_pdf(&[CATALOG, PAGES, PAGE], None, ""), &limits)?;
    let mut sink = TestSink::recording();
    let reference = PdfRef {
        number: 7,
        generation: 0,
    };
    (|| {
        let mut writer = AppendWriter::new(&mut sink, &index, &limits, &NEVER);
        let misuse = invalid("invalid PDF append object state or number");
        assert!(misuse(&writer.begin_object(PdfRef {
            number: 0,
            generation: 0,
        })));
        assert!(invalid("no PDF append object is open")(
            &writer.end_object()
        ));
        writer.begin_object(reference)?;
        assert!(misuse(&writer.begin_object(reference)));
        let open = invalid("PDF append object remains open");
        assert!(open(&writer.finish_copy()));
        writer.end_object()
    })()?;
    assert_eq!(sink.bytes, b"\n7 0 obj\n\nendobj\n");
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
    let mut sink = Vec::new();
    (|| {
        let mut writer = AppendWriter::new(&mut sink, &index, &limits, &NEVER);
        writer.begin_object(reference)?;
        assert!(invalid("PDF append object remains open")(
            &writer.finish_update()
        ));
        writer.end_object()?;
        writer.begin_object(reference)?;
        writer.end_object()?;
        assert!(invalid("PDF update defines an object twice")(
            &writer.finish_update()
        ));
        Ok::<_, Error>(())
    })()?;

    let mut sink = Vec::new();
    let oversized = {
        let mut writer = AppendWriter::new(&mut sink, &index, &limits, &NEVER);
        writer.out.position = MAX_CLASSIC_PDF_BYTES;
        writer.begin_object(reference)
    };
    assert!(matches!(
        oversized,
        Err(Error { kind: ErrorKind::LimitExceeded { resource: "classic PDF file bytes", attempted, .. }, .. }) if attempted == MAX_CLASSIC_PDF_BYTES + 1
    ));

    let mut sink = Vec::new();
    let far_object = {
        let mut writer = AppendWriter::new(&mut sink, &index, &limits, &NEVER);
        writer.entries.push(XrefEntry {
            reference,
            offset: MAX_CLASSIC_PDF_BYTES + 1,
        });
        writer.finish_update()
    };
    assert!(matches!(
        far_object,
        Err(Error {
            kind: ErrorKind::LimitExceeded {
                resource: "classic PDF object offset",
                ..
            },
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
fn an_empty_gap_patch_is_consumed_without_touching_the_chunk() -> Result<()> {
    let gaps = [GapPatch {
        offset: 1,
        original: Vec::new(),
    }];
    let mut patches = copy_patches(&[], &gaps);
    let mut chunk = *b"abc";
    patches.apply(&mut chunk, 0)?;
    assert_eq!(&chunk, b"abc");
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

#[test]
fn a_bookmark_that_fails_while_closing_items_stops_the_outline() -> Result<()> {
    let original = unoutlined_pdf()?;
    let limits = Limits::default();
    let index = open_index(&original, &limits)?;
    let mut source = SeekableSource::new(Cursor::new(original.as_slice()))?;
    let mut sink = TestSink::recording();
    let (failed, retry, finish) = (|| {
        let mut appender =
            PdfOutlineAppender::begin(&mut source, &mut sink, &index, &limits, &NEVER)?;
        let bookmark = |depth: u32, title: &str| Bookmark {
            depth,
            title: title.into(),
            page_index: 0,
        };
        appender.add_bookmark(bookmark(0, "A"))?;
        appender.add_bookmark(bookmark(1, "B"))?;
        // Writing the closed items would pass the classic xref limit, which
        // fails before the sink sees a byte and so leaves the writer usable.
        // Restoring the position afterwards makes the failure transient, as
        // an allocator refusal would be.
        let position = appender.writer.out.position;
        appender.writer.out.position = MAX_CLASSIC_PDF_BYTES;
        let failed = appender.add_bookmark(bookmark(0, "C"));
        appender.writer.out.position = position;
        let retry = appender.add_bookmark(bookmark(0, "D"));
        Ok::<_, Error>((failed, retry, appender.finish()))
    })()?;
    assert!(matches!(
        failed,
        Err(Error {
            kind: ErrorKind::LimitExceeded {
                resource: "classic PDF file bytes",
                ..
            },
            ..
        })
    ));
    for result in [retry, finish.map(|_| ())] {
        assert!(matches!(
            result,
            Err(Error {
                kind: ErrorKind::Malformed,
                reason: "PDF outline cannot continue after a failed bookmark operation",
                ..
            })
        ));
    }
    assert_eq!(sink.flushes, 0);
    assert_eq!(sink.bytes, original);
    Ok(())
}
