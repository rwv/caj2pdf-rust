// SPDX-License-Identifier: MIT

use caj2pdf_core::{
    Bookmark, BookmarkVisitor, Error, Limits, NeverCancel, RangedSource,
    hnc8::{Budget, ErrorKind, Hnc8Reader, OutlineRepair, OutlineReport},
    native::SeekableSource,
};
use std::{
    future::Future,
    io::Cursor,
    pin::pin,
    task::{Context, Poll, Waker},
};

fn run_native<F: Future>(future: F) -> F::Output {
    match pin!(future)
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("in-memory adapters must be ready"),
    }
}

fn source(records: &[(&[u8], &[u8], u32)]) -> Vec<u8> {
    let end = 348 + records.len() * 308 + 3 * 20;
    let mut bytes = vec![0; end];
    bytes[..8].copy_from_slice(b"HN\0\0\x90\x01\0\0");
    bytes[144..148].copy_from_slice(&3_i32.to_le_bytes());
    bytes[344..348].copy_from_slice(&(records.len() as i32).to_le_bytes());
    for (index, &(title, page, level)) in records.iter().enumerate() {
        let start = 348 + index * 308;
        bytes[start..start + title.len()].copy_from_slice(title);
        bytes[start + 280..start + 280 + page.len()].copy_from_slice(page);
        bytes[start + 304..start + 308].copy_from_slice(&level.to_le_bytes());
    }
    bytes
}

#[derive(Default)]
struct Entries(Vec<Bookmark>);
impl BookmarkVisitor for Entries {
    async fn visit(&mut self, bookmark: Bookmark) -> caj2pdf_core::Result<()> {
        self.0.push(bookmark);
        Ok(())
    }
}

fn read(
    bytes: Vec<u8>,
    limits: Limits,
    depth: u32,
    map: impl FnMut(u32) -> Option<u32>,
) -> Result<(Vec<Bookmark>, OutlineReport), caj2pdf_core::hnc8::Hnc8Error> {
    run_native(async {
        let mut input = SeekableSource::new(Cursor::new(bytes)).unwrap();
        let mut reader =
            Hnc8Reader::open(&mut input, &limits, &NeverCancel, Budget::default()).await?;
        let mut entries = Entries::default();
        let report = reader.visit_bookmarks(depth, 3, map, &mut entries).await?;
        Ok((entries.0, report))
    })
}

#[test]
fn ranged_outlines_preserve_empty_unicode_hierarchy_and_mapped_pages() {
    let bytes = source(&[
        (b"Root\0ignored", b"00000000003", 1),
        (&[0xd6, 0xd0, 0x94, 0x39, 0xfc, 0x36], b"2", 2),
        (b"", b"2", 1),
    ]);
    let (entries, report) = read(
        bytes,
        Limits {
            io_chunk_bytes: 1,
            ..Limits::default()
        },
        64,
        |page| Some(3 - page),
    )
    .unwrap();
    assert_eq!(
        entries.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(),
        ["Root", "中😀", ""]
    );
    assert_eq!(
        entries
            .iter()
            .map(|e| (e.depth, e.page_index))
            .collect::<Vec<_>>(),
        [(0, 0), (1, 1), (0, 1)]
    );
    assert_eq!((report.declared, report.written, report.defects), (3, 3, 0));
}

#[test]
fn malformed_entries_and_omitted_destinations_are_located_defects_not_errors() {
    let title = [b'x'; 256];
    for (record, relative, repair) in [
        (
            (title.as_slice(), b"1".as_slice(), 1),
            0,
            OutlineRepair::Skipped,
        ),
        (
            (b"\x81".as_slice(), b"1".as_slice(), 1),
            0,
            OutlineRepair::Skipped,
        ),
        (
            (b"x".as_slice(), b"".as_slice(), 1),
            280,
            OutlineRepair::Skipped,
        ),
        (
            (b"x".as_slice(), b"-1".as_slice(), 1),
            280,
            OutlineRepair::Skipped,
        ),
        (
            (b"x".as_slice(), b"000000000001".as_slice(), 1),
            280,
            OutlineRepair::Skipped,
        ),
        (
            (b"x".as_slice(), b"0".as_slice(), 1),
            280,
            OutlineRepair::Skipped,
        ),
        (
            (b"x".as_slice(), b"4".as_slice(), 1),
            280,
            OutlineRepair::Skipped,
        ),
        (
            (b"x".as_slice(), b"1".as_slice(), 0),
            304,
            OutlineRepair::Skipped,
        ),
        (
            (b"x".as_slice(), b"1".as_slice(), 65),
            304,
            OutlineRepair::Clamped,
        ),
        (
            (b"x".as_slice(), b"1".as_slice(), 2),
            304,
            OutlineRepair::Clamped,
        ),
    ] {
        let (entries, report) =
            read(source(&[record]), Limits::default(), 64, |p| Some(p - 1)).unwrap();
        let written = usize::from(repair == OutlineRepair::Clamped);
        assert_eq!(entries.len(), written);
        assert!(entries.iter().all(|entry| entry.depth == 0));
        let [defect] = report.recorded_defects() else {
            panic!("expected one defect: {report:?}");
        };
        assert_eq!((defect.offset, defect.repair), (348 + relative, repair));
    }
    for missing in [None, Some(3)] {
        let (entries, report) = read(source(&[(b"x", b"1", 1)]), Limits::default(), 64, |_| {
            missing
        })
        .unwrap();
        assert!(entries.is_empty());
        assert_eq!(report.recorded_defects()[0].offset, 628);
    }
}

#[test]
fn count_depth_and_title_budgets_are_checked_before_visiting() {
    let bytes = source(&[(b"title", b"1", 1)]);
    for limits in [
        Limits {
            max_bookmarks: 0,
            ..Limits::default()
        },
        Limits {
            io_chunk_bytes: 1,
            max_allocation_bytes: 19,
            ..Limits::default()
        },
    ] {
        let error = read(bytes.clone(), limits, 64, |p| Some(p - 1)).unwrap_err();
        assert!(matches!(error.kind, ErrorKind::LimitExceeded { .. }));
    }
    assert!(read(bytes, Limits::default(), 0, |p| Some(p - 1)).is_err());
    assert!(
        read(source(&[]), Limits::default(), 64, |_| None)
            .unwrap()
            .0
            .is_empty()
    );
}

struct FailedVisitor(bool);
impl BookmarkVisitor for FailedVisitor {
    async fn visit(&mut self, _: Bookmark) -> caj2pdf_core::Result<()> {
        Err(if self.0 {
            Error::Cancelled
        } else {
            Error::InvalidInput {
                reason: "injected visitor failure",
            }
        })
    }
}

#[test]
fn visitor_failure_poisoning_prevents_replaying_partial_output() {
    for cancelled in [false, true] {
        run_native(async {
            let mut input = SeekableSource::new(Cursor::new(source(&[(b"x", b"1", 1)]))).unwrap();
            let limits = Limits::default();
            let mut reader = Hnc8Reader::open(&mut input, &limits, &NeverCancel, Budget::default())
                .await
                .unwrap();
            let error = reader
                .visit_bookmarks(64, 3, |p| Some(p - 1), &mut FailedVisitor(cancelled))
                .await
                .unwrap_err();
            assert_eq!(error.offset, 348);
            assert_eq!(
                error.kind.field(),
                if cancelled {
                    "cancellation"
                } else {
                    "outline visitor"
                }
            );
            let mut entries = Entries::default();
            let again = reader
                .visit_bookmarks(64, 3, |p| Some(p - 1), &mut entries)
                .await
                .unwrap_err();
            assert!(matches!(again.kind, ErrorKind::Poisoned));
            assert!(entries.0.is_empty());
        });
    }
}

#[test]
fn unsupported_outline_profiles_are_not_empty_successes() {
    let mut bytes = vec![0; 216 + 20];
    bytes[..8].copy_from_slice(b"HN\0\0\xc8\0\0\0");
    bytes[144..148].copy_from_slice(&1_i32.to_le_bytes());
    let error = read(bytes, Limits::default(), 64, |p| Some(p - 1)).unwrap_err();
    assert!(matches!(
        error.kind,
        ErrorKind::Unsupported {
            field: "outline variant",
            ..
        }
    ));
}

struct TruncatedRecord(Vec<u8>);
impl RangedSource for TruncatedRecord {
    fn size(&self) -> u64 {
        self.0.len() as u64
    }
    async fn read_at(&mut self, offset: u64, bytes: &mut [u8]) -> caj2pdf_core::Result<usize> {
        if offset >= 348 {
            return Ok(0);
        }
        let n = bytes.len().min(348 - offset as usize);
        bytes[..n].copy_from_slice(&self.0[offset as usize..offset as usize + n]);
        Ok(n)
    }
}
#[test]
fn a_short_record_read_is_not_a_partial_bookmark() {
    run_native(async {
        let mut input = TruncatedRecord(source(&[(b"x", b"1", 1)]));
        let limits = Limits::default();
        let mut reader = Hnc8Reader::open(&mut input, &limits, &NeverCancel, Budget::default())
            .await
            .unwrap();
        let mut entries = Entries::default();
        let error = reader
            .visit_bookmarks(64, 3, |p| Some(p - 1), &mut entries)
            .await
            .unwrap_err();
        assert_eq!(error.offset, 348);
        assert!(entries.0.is_empty());
        assert!(matches!(error.kind, ErrorKind::Truncated { .. }));
    });
}

#[test]
fn cancellation_and_output_page_limit_are_checked_even_before_records() {
    use std::cell::Cell;
    struct Flag(Cell<bool>);
    impl caj2pdf_core::Cancellation for Flag {
        fn is_cancelled(&self) -> bool {
            self.0.get()
        }
    }
    run_native(async {
        let flag = Flag(Cell::new(false));
        let mut input = SeekableSource::new(Cursor::new(source(&[]))).unwrap();
        let limits = Limits {
            max_pages: 3,
            ..Limits::default()
        };
        let mut reader = Hnc8Reader::open(&mut input, &limits, &flag, Budget::default())
            .await
            .unwrap();
        let mut entries = Entries::default();
        flag.0.set(true);
        assert!(matches!(
            reader
                .visit_bookmarks(64, 3, |p| Some(p - 1), &mut entries)
                .await
                .unwrap_err()
                .kind,
            ErrorKind::Cancelled
        ));
        flag.0.set(false);
        let mut reader = Hnc8Reader::open(&mut input, &limits, &flag, Budget::default())
            .await
            .unwrap();
        assert!(matches!(
            reader
                .visit_bookmarks(64, 4, |p| Some(p - 1), &mut entries)
                .await
                .unwrap_err()
                .kind,
            ErrorKind::LimitExceeded {
                resource: "outline output pages",
                ..
            }
        ));
    });
}

#[test]
fn dropping_a_pending_visitor_poisoned_the_reader() {
    struct Pending;
    impl BookmarkVisitor for Pending {
        async fn visit(&mut self, _: Bookmark) -> caj2pdf_core::Result<()> {
            std::future::pending().await
        }
    }
    run_native(async {
        let mut input = SeekableSource::new(Cursor::new(source(&[(b"x", b"1", 1)]))).unwrap();
        let limits = Limits::default();
        let mut reader = Hnc8Reader::open(&mut input, &limits, &NeverCancel, Budget::default())
            .await
            .unwrap();
        {
            let mut visitor = Pending;
            let mut future = pin!(reader.visit_bookmarks(64, 3, |p| Some(p - 1), &mut visitor));
            assert!(
                future
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop()))
                    .is_pending()
            );
        }
        let error = reader
            .visit_bookmarks(64, 3, |p| Some(p - 1), &mut Entries::default())
            .await
            .unwrap_err();
        assert!(matches!(error.kind, ErrorKind::Poisoned));
    });
}

#[test]
fn independent_pdf_reader_checks_native_outline_titles_hierarchy_and_targets() {
    use caj2pdf_core::{
        native::WriteSink,
        pdf::{BookmarkView, ImageEncoding, ImageSpec, PageSpec, PdfDocument},
    };
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    struct Xyz<'a, 'b>(&'a mut PdfDocument<'b, WriteSink<Vec<u8>>, NeverCancel>);
    impl BookmarkVisitor for Xyz<'_, '_> {
        async fn visit(&mut self, bookmark: Bookmark) -> caj2pdf_core::Result<()> {
            self.0
                .add_bookmark_with_view(bookmark, BookmarkView::Xyz)
                .await
        }
    }
    let mut input = SeekableSource::new(Cursor::new(source(&[
        (b"Root", b"3", 1),
        (&[0xd6, 0xd0, 0x94, 0x39, 0xfc, 0x36], b"2", 2),
        (b"", b"2", 1),
    ])))
    .unwrap();
    let limits = Limits::default();
    let mut sink = WriteSink::new(Vec::new());
    run_native(async {
        let mut reader = Hnc8Reader::open(&mut input, &limits, &NeverCancel, Budget::default())
            .await
            .unwrap();
        let mut doc = PdfDocument::new(&mut sink, &limits, &NeverCancel)
            .await
            .unwrap();
        let mut white = SeekableSource::new(Cursor::new([255_u8])).unwrap();
        let image = doc
            .add_image(
                &mut white,
                0,
                1,
                ImageSpec {
                    pixel_width: 1,
                    pixel_height: 1,
                    encoding: ImageEncoding::Gray8,
                },
            )
            .await
            .unwrap();
        for _ in 0..3 {
            doc.add_page(
                PageSpec {
                    width_points: 72.,
                    height_points: 72.,
                },
                &[image],
            )
            .await
            .unwrap();
        }
        reader
            .visit_bookmarks(64, 3, |page| Some(3 - page), &mut Xyz(&mut doc))
            .await
            .unwrap();
        let report = doc.finish().await.unwrap();
        assert_eq!(report.bookmarks_written, 3);
    });
    struct Temp(std::path::PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let file =
        Temp(std::env::temp_dir().join(format!("caj2pdf-hna-outlines-{}.pdf", std::process::id())));
    std::fs::File::create_new(&file.0)
        .unwrap()
        .write_all(&sink.into_inner())
        .unwrap();
    let check = Command::new("qpdf")
        .arg("--check")
        .arg(&file.0)
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    let query = Command::new("qpdf")
        .args(["--json", "--json-key=outlines", "--json-stream-data=none"])
        .arg(&file.0)
        .output()
        .unwrap();
    assert!(query.status.success());
    let mut verify = Command::new("python3")
        .args([
            "-c",
            r#"
import json,sys
roots=json.load(sys.stdin)['outlines']
assert len(roots)==2
entries=[roots[0],roots[0]['kids'][0],roots[1]]
assert [e['title'] for e in entries]==['Root','中😀','']
assert [e['destpageposfrom1'] for e in entries]==[1,2,2]
assert all(e['dest'][1:]==['/XYZ',None,None,None] for e in entries)
assert entries[1]['kids']==[] and entries[2]['kids']==[]
"#,
        ])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    verify
        .stdin
        .take()
        .unwrap()
        .write_all(&query.stdout)
        .unwrap();
    assert!(verify.wait().unwrap().success());
}
