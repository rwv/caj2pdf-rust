// SPDX-License-Identifier: MIT

use super::*;
use crate::{
    native::SeekableSource,
    pdf::font::tests::drawing_font,
    test_support::{NEVER, run},
};
use std::io::Cursor;

#[derive(Default)]
struct Sink {
    bytes: Vec<u8>,
    max_request: usize,
    fail_after: Option<usize>,
    pending: std::rc::Rc<std::cell::Cell<bool>>,
}
impl SequentialSink for Sink {
    async fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        if self.pending.get() {
            std::future::pending::<()>().await;
        }
        self.max_request = self.max_request.max(bytes.len());
        if self
            .fail_after
            .is_some_and(|limit| self.bytes.len() >= limit)
        {
            return Err(std::io::Error::other("injected output failure").into());
        }
        let count = bytes.len().min(7);
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }
    async fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

fn page() -> PageSpec {
    PageSpec {
        width_points: 120.0,
        height_points: 100.0,
    }
}
fn matrix(x: f64) -> [f64; 6] {
    [20.0, 0.0, 0.0, 20.0, x, 50.0]
}

#[test]
fn embedded_font_and_ordered_mixed_page_reopen() {
    let bytes = drawing_font();
    assert!(bytes.len() < 1024);
    let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
    let limits = Limits {
        io_chunk_bytes: 31,
        ..Limits::default()
    };
    let mut font = run(TrueTypeFont::read(&mut source, &limits, &NEVER)).unwrap();
    let mut sink = Sink::default();
    let report = run(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await.unwrap();
        let font = document.add_font(&mut font).await.unwrap();
        assert!(font.supports('A'));
        assert!(font.supports('中'));
        assert!(!font.supports('B'));
        assert!(!font.supports('😀'));
        let mut image = SeekableSource::new(Cursor::new(vec![255_u8, 0, 0])).unwrap();
        let image = document
            .add_image(
                &mut image,
                0,
                3,
                ImageSpec {
                    pixel_width: 1,
                    pixel_height: 1,
                    encoding: ImageEncoding::Rgb8,
                },
            )
            .await
            .unwrap();
        let fonts = [&font];
        let images = [image];
        let mut content = document
            .begin_content_page(page(), &fonts, &images)
            .await
            .unwrap();
        content.glyph(0, 'A', matrix(10.0)).await.unwrap();
        content
            .image(0, [6.0, 0.0, 0.0, 6.0, 14.0, 52.0])
            .await
            .unwrap();
        content.glyph(0, '中', matrix(30.0)).await.unwrap();
        content
            .segment([60.0, 50.0], [90.0, 50.0], 2.0)
            .await
            .unwrap();
        assert_eq!(content.finish().await.unwrap(), 0);
        let mut content = document
            .begin_content_page(page(), &fonts, &[])
            .await
            .unwrap();
        content.glyph(0, '中', matrix(10.0)).await.unwrap();
        assert_eq!(content.finish().await.unwrap(), 1);
        document.finish().await.unwrap()
    });
    assert_eq!(report.pages_converted, 2);
    assert_eq!(report.input_bytes_read, bytes.len() as u64 + 3);
    assert!(sink.max_request <= 31);
    let text = String::from_utf8_lossy(&sink.bytes);
    assert!(text.contains("/Subtype /CIDFontType2"));
    let widths = text
        .split("/W [ 0 [")
        .nth(1)
        .unwrap()
        .split(']')
        .next()
        .unwrap();
    assert_eq!(widths.split_whitespace().nth(65), Some("600"));
    assert!(text.contains(" 19968 ["));
    assert!(text.contains("/FontName /CajFixture"));
    let a = text.find("<0041> Tj").unwrap();
    let image = text.find("/Im0 Do").unwrap();
    let chinese = text.find("<4E2D> Tj").unwrap();
    assert!(a < image && image < chinese);
    let mut pdf = SeekableSource::new(Cursor::new(sink.bytes.clone())).unwrap();
    let size = pdf.size();
    let index = run(crate::pdf::PdfIndex::open(
        &mut pdf,
        crate::pdf::PdfRange {
            offset: 0,
            length: size,
        },
        &Limits::default(),
        &NEVER,
    ))
    .unwrap();
    assert_eq!(index.pages().len(), 2);
    // Optional export of this original fixture for independent validators;
    // no external data or test pass is inferred from an absent export.
    if let Some(root) = std::env::var_os("CAJ2PDF_FONT_TEST_OUTPUT") {
        let root = std::path::PathBuf::from(root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("original.ttf"), bytes).unwrap();
        std::fs::write(root.join("mixed.pdf"), sink.bytes).unwrap();
    }
}

#[test]
fn failed_or_abandoned_content_cannot_be_finished() {
    for case in 0..8 {
        let mut source = SeekableSource::new(Cursor::new(drawing_font())).unwrap();
        let limits = Limits::default();
        let mut font = run(TrueTypeFont::read(&mut source, &limits, &NEVER)).unwrap();
        let mut sink = Sink::default();
        run(async {
            let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await.unwrap();
            let font = document.add_font(&mut font).await.unwrap();
            let fonts = [&font];
            let mut content = document
                .begin_content_page(page(), &fonts, &[])
                .await
                .unwrap();
            let result = match case {
                0 => content.glyph(1, 'A', matrix(0.0)).await,
                1 => content.glyph(0, 'B', matrix(0.0)).await,
                2 => content.glyph(0, 'A', matrix(f64::NAN)).await,
                3 => content.image(0, matrix(0.0)).await,
                4 => {
                    drop(content);
                    assert!(document.finish().await.is_err());
                    return;
                }
                5 => content.segment([0.0, 0.0], [1.0, 1.0], -1.0).await,
                6 => content.segment([f64::INFINITY, 0.0], [1.0, 1.0], 1.0).await,
                _ => {
                    content.failed = true;
                    content.glyph(0, 'A', matrix(0.0)).await
                }
            };
            assert!(result.is_err());
            assert!(content.finish().await.is_err());
            assert!(document.finish().await.is_err());
        });
    }
}

#[test]
fn failed_font_copy_poisons_document() {
    let mut source = SeekableSource::new(Cursor::new(drawing_font())).unwrap();
    let limits = Limits::default();
    let mut font = run(TrueTypeFont::read(&mut source, &limits, &NEVER)).unwrap();
    let mut sink = Sink {
        fail_after: Some(100),
        ..Sink::default()
    };
    run(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await.unwrap();
        assert!(document.add_font(&mut font).await.is_err());
        assert!(document.add_font(&mut font).await.is_err());
        assert!(document.begin_content_page(page(), &[], &[]).await.is_err());
        assert!(document.finish().await.is_err());
    });
}

#[test]
fn foreign_resources_and_page_preflight_do_not_poison() {
    let limits = Limits::default();
    let mut source = SeekableSource::new(Cursor::new(drawing_font())).unwrap();
    let mut font = run(TrueTypeFont::read(&mut source, &limits, &NEVER)).unwrap();
    let mut first = Sink::default();
    let foreign = run(async {
        let mut document = PdfDocument::new(&mut first, &limits, &NEVER).await.unwrap();
        document.add_font(&mut font).await.unwrap()
    });
    let mut sink = Sink::default();
    run(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await.unwrap();
        assert!(
            document
                .begin_content_page(page(), &[&foreign], &[])
                .await
                .is_err()
        );
        let many = vec![&foreign; MAX_PAGE_FONTS + 1];
        assert!(
            document
                .begin_content_page(page(), &many, &[])
                .await
                .is_err()
        );
        assert!(
            document
                .begin_content_page(
                    PageSpec {
                        width_points: 0.0,
                        height_points: 1.0
                    },
                    &[],
                    &[]
                )
                .await
                .is_err()
        );
        let content = document.begin_content_page(page(), &[], &[]).await.unwrap();
        assert_eq!(content.finish().await.unwrap(), 0);
        document.finish().await.unwrap();
    });
}

struct FontSource {
    bytes: Vec<u8>,
    size: u64,
    fail: bool,
    pending: bool,
}
impl FontSource {
    fn new() -> Self {
        let bytes = drawing_font();
        Self {
            size: bytes.len() as u64,
            bytes,
            fail: false,
            pending: false,
        }
    }
}
impl RangedSource for FontSource {
    fn size(&self) -> u64 {
        self.size
    }
    async fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize> {
        if self.pending {
            std::future::pending::<()>().await;
        }
        if self.fail {
            return Err(std::io::Error::other("injected font read failure").into());
        }
        let at = offset as usize;
        let count = out.len().min(3).min(self.bytes.len().saturating_sub(at));
        out[..count].copy_from_slice(&self.bytes[at..at + count]);
        Ok(count)
    }
}

#[test]
fn font_limits_and_invalid_glyphs_are_explicit() {
    for case in 0..4 {
        let mut source = FontSource::new();
        if case == 0 {
            source.size = MAX_PDF_INTEGER + 1;
        }
        if case == 3 {
            let table = source.bytes[12..]
                .chunks_exact(16)
                .find(|entry| &entry[..4] == b"cmap")
                .unwrap();
            let offset = u32::from_be_bytes(table[8..12].try_into().unwrap()) as usize;
            source.bytes[offset + 36..offset + 40].copy_from_slice(&9_u32.to_be_bytes());
        }
        let limits = Limits::default();
        let mut font = run(TrueTypeFont::read(&mut source, &limits, &NEVER)).unwrap();
        let document_limits = match case {
            1 => Limits {
                max_input_bytes: 1,
                ..limits
            },
            2 => Limits {
                io_chunk_bytes: 31,
                max_allocation_bytes: 4096,
                ..limits
            },
            _ => limits,
        };
        let mut sink = Sink::default();
        run(async {
            let mut document = PdfDocument::new(&mut sink, &document_limits, &NEVER)
                .await
                .unwrap();
            let before = document.writer.position();
            assert!(document.add_font(&mut font).await.is_err());
            if case == 3 {
                assert!(document.writer.position() > before);
                assert!(document.finish().await.is_err());
            } else {
                assert_eq!(document.writer.position(), before);
                document
                    .begin_content_page(page(), &[], &[])
                    .await
                    .unwrap()
                    .finish()
                    .await
                    .unwrap();
                document.finish().await.unwrap();
            }
        });
    }
}

#[test]
fn failed_or_abandoned_font_read_poisons_document() {
    use std::{
        future::Future,
        task::{Context, Poll, Waker},
    };
    for pending in [false, true] {
        let mut source = FontSource::new();
        let limits = Limits::default();
        let mut font = run(TrueTypeFont::read(&mut source, &limits, &NEVER)).unwrap();
        font.source.pending = pending;
        font.source.fail = !pending;
        let mut sink = Sink::default();
        let mut document = run(PdfDocument::new(&mut sink, &limits, &NEVER)).unwrap();
        if pending {
            let mut future = std::pin::pin!(document.add_font(&mut font));
            assert!(matches!(
                future
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop())),
                Poll::Pending
            ));
        } else {
            assert!(run(document.add_font(&mut font)).is_err());
        }
        assert!(run(document.finish()).is_err());
    }
}

#[test]
fn cancellation_and_abandoned_draws_cannot_publish_a_partial_page() {
    use std::{
        cell::Cell,
        future::Future,
        rc::Rc,
        task::{Context, Poll, Waker},
    };
    struct Cancel(Rc<Cell<bool>>);
    impl Cancellation for Cancel {
        fn is_cancelled(&self) -> bool {
            self.0.get()
        }
    }
    for pending in [false, true] {
        let mut source = FontSource::new();
        let limits = Limits::default();
        let mut font = run(TrueTypeFont::read(&mut source, &limits, &NEVER)).unwrap();
        let mut sink = Sink::default();
        let suspend = sink.pending.clone();
        let cancelled = Rc::new(Cell::new(false));
        let cancellation = Cancel(cancelled.clone());
        let mut document = run(PdfDocument::new(&mut sink, &limits, &cancellation)).unwrap();
        let font = run(document.add_font(&mut font)).unwrap();
        let fonts = [&font];
        let mut page = run(document.begin_content_page(page(), &fonts, &[])).unwrap();
        if pending {
            suspend.set(true);
            let mut future = std::pin::pin!(page.glyph(0, 'A', matrix(10.0)));
            assert!(matches!(
                future
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop())),
                Poll::Pending
            ));
        } else {
            cancelled.set(true);
            assert!(matches!(
                run(page.glyph(0, 'A', matrix(10.0))),
                Err(Error::Cancelled)
            ));
        }
        suspend.set(false);
        cancelled.set(false);
        assert!(run(page.finish()).is_err());
        assert!(run(document.finish()).is_err());
    }
}

#[test]
fn notdef_is_missing_and_postscript_hash_is_escaped() {
    let mut source = FontSource::new();
    let offset = |bytes: &[u8], tag: &[u8; 4]| {
        let table = bytes[12..]
            .chunks_exact(16)
            .find(|entry| &entry[..4] == tag)
            .unwrap();
        u32::from_be_bytes(table[8..12].try_into().unwrap()) as usize
    };
    let cmap = offset(&source.bytes, b"cmap");
    source.bytes[cmap + 36..cmap + 40].copy_from_slice(&0_u32.to_be_bytes());
    let name = offset(&source.bytes, b"name");
    source.bytes[name + 18..name + 20].copy_from_slice(&u16::from(b'#').to_be_bytes());
    let limits = Limits::default();
    let mut font = run(TrueTypeFont::read(&mut source, &limits, &NEVER)).unwrap();
    let mut sink = Sink::default();
    run(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await.unwrap();
        let font = document.add_font(&mut font).await.unwrap();
        assert!(!font.supports('A'));
        assert!(font.supports('中'));
        document
            .begin_content_page(page(), &[&font], &[])
            .await
            .unwrap()
            .finish()
            .await
            .unwrap();
        document.finish().await.unwrap();
    });
    assert!(String::from_utf8_lossy(&sink.bytes).contains("/FontName /#23ajFixture"));
}
