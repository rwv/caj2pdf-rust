// SPDX-License-Identifier: MIT

use super::*;
use crate::{
    native::SeekableSource,
    pdf::font::tests::drawing_font,
    test_support::{NEVER, inflated_pdf, inflated_stream, run},
};
use std::io::Cursor;

#[derive(Default)]
struct Sink {
    bytes: Vec<u8>,
    max_request: usize,
    fail_after: Option<usize>,
    fail_now: std::rc::Rc<std::cell::Cell<bool>>,
    pending: std::rc::Rc<std::cell::Cell<bool>>,
}
impl SequentialSink for Sink {
    async fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        if self.pending.get() {
            std::future::pending::<()>().await;
        }
        self.max_request = self.max_request.max(bytes.len());
        if self.fail_now.get()
            || self
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
fn fonts_must_be_embedded_before_finishing() {
    let mut source = SeekableSource::new(Cursor::new(drawing_font())).unwrap();
    let limits = Limits::default();
    let font = run(TrueTypeFont::read(&mut source, &limits, &NEVER)).unwrap();
    let mut sink = Sink::default();
    run(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await.unwrap();
        let handle = document.add_font(&font).unwrap();
        let fonts = [&handle];
        let mut content = document
            .begin_content_page(page(), &fonts, &[])
            .await
            .unwrap();
        content.glyph(0, 'A', matrix(10.0)).await.unwrap();
        content.finish().await.unwrap();
        assert!(matches!(
            document.finish().await,
            Err(Error::InvalidInput {
                reason: "PDF font was added but its subset was not embedded"
            })
        ));
    });
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
        let handle = document.add_font(&font).unwrap();
        assert!(handle.supports('A'));
        assert!(handle.supports('中'));
        assert!(!handle.supports('B'));
        assert!(!handle.supports('😀'));
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
        let fonts = [&handle];
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
        content
            .glyph_with_gray(0, 'A', matrix(60.0), 68)
            .await
            .unwrap();
        content
            .glyph_with_clip(0, 'A', matrix(30.0), [30.0, 50.0, 5.0, 20.0])
            .await
            .unwrap();
        content
            .decoration_glyph(0, 'A', matrix(45.0), [45.0, 50.0, 5.0, 20.0])
            .await
            .unwrap();
        content.glyph(0, 'A', matrix(90.0)).await.unwrap();
        assert_eq!(content.finish().await.unwrap(), 1);
        document.embed_font(&handle, &mut font).await.unwrap();
        assert!(matches!(
            document.embed_font(&handle, &mut font).await,
            Err(Error::InvalidInput {
                reason: "PDF font subset is already embedded"
            })
        ));
        document.finish().await.unwrap()
    });
    assert_eq!(report.pages_converted, 2);
    assert_eq!(report.input_bytes_read, 3 + font.subset_bytes_read());
    assert!(sink.max_request <= 31);
    let inflated = inflated_pdf(&sink.bytes);
    let text = String::from_utf8_lossy(&inflated);
    assert!(text.contains("/Subtype /CIDFontType2"));
    assert!(text.contains("/DW 1000 /W [ 65 [ 600 ] 20013 [ 1000 ] ]"));
    let tagged = text.split("/FontName /").nth(1).unwrap();
    assert_eq!(&tagged[6..18], "+CajFixture ");
    assert!(tagged[..6].bytes().all(|byte| byte.is_ascii_uppercase()));
    let program = inflated_stream(&sink.bytes, b"/Length1 ");
    let length = text.split("/Length1 ").nth(1).unwrap().split('\n').next();
    assert_eq!(length, Some(program.len().to_string().as_str()));
    let subset = xberg_ttf_parser::Face::parse(&program, 0).unwrap();
    assert_eq!(subset.number_of_glyphs(), 3);
    assert_eq!(
        subset.glyph_hor_advance(xberg_ttf_parser::GlyphId(1)),
        Some(600)
    );
    assert_eq!(
        subset.glyph_hor_advance(xberg_ttf_parser::GlyphId(2)),
        Some(1000)
    );
    let mapping = text
        .split("/CIDToGIDMap ")
        .nth(1)
        .unwrap()
        .split(' ')
        .next()
        .unwrap();
    let map = inflated_stream(&sink.bytes, format!("\n{mapping} 0 obj").as_bytes());
    assert_eq!(map.len(), 2 * (0x4e2d + 1));
    assert_eq!(&map[0x82..0x84], &[0, 1]);
    assert_eq!(&map[0x4e2d * 2..], &[0, 2]);
    assert_eq!(map.iter().filter(|byte| **byte != 0).count(), 2);
    let unicode = text
        .split("/ToUnicode ")
        .nth(1)
        .unwrap()
        .split(' ')
        .next()
        .unwrap();
    let unicode = inflated_stream(&sink.bytes, format!("\n{unicode} 0 obj").as_bytes());
    assert!(String::from_utf8_lossy(&unicode).contains("<4E00> <4EFF> <4E00>"));
    let a = text.find("<0041> Tj").unwrap();
    let image = text.find("/Im0 Do").unwrap();
    let chinese = text.find("<4E2D> Tj").unwrap();
    assert!(a < image && image < chinese);
    let gray = text.split("q 0.266667 g\n").nth(1).unwrap();
    let (gray, following) = gray.split_once("Q\n").unwrap();
    assert!(gray.contains("60 50 Tm <0041> Tj ET"));
    assert!(following.contains("90 50 Tm <0041> Tj ET"));
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
fn bounded_filled_polygons_preserve_concavity_and_close_paths() {
    let limits = Limits {
        io_chunk_bytes: 31,
        ..Limits::default()
    };
    let mut sink = Sink::default();
    run(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await.unwrap();
        let mut content = document.begin_content_page(page(), &[], &[]).await.unwrap();
        content
            .fill_polygon(&[
                [10.0, 10.0],
                [30.0, 10.0],
                [45.0, 25.0],
                [30.0, 40.0],
                [10.0, 40.0],
                [25.0, 25.0],
            ])
            .await
            .unwrap();
        content
            .fill_polygon(&[[70.0, 10.0], [90.0, 10.0], [70.0, 30.0]])
            .await
            .unwrap();
        content
            .fill_polygon(&[
                [60.0, 50.0],
                [80.0, 50.0],
                [90.0, 60.0],
                [90.0, 80.0],
                [80.0, 90.0],
                [60.0, 90.0],
                [50.0, 80.0],
                [50.0, 60.0],
            ])
            .await
            .unwrap();
        content.finish().await.unwrap();
        document.finish().await.unwrap();
    });
    assert!(sink.max_request <= 31);
    let inflated = inflated_pdf(&sink.bytes);
    let text = String::from_utf8_lossy(&inflated);
    assert!(text.contains("q 0 g\n10 10 m\n30 10 l\n45 25 l\n30 40 l\n10 40 l\n25 25 l\nh f Q\n"));
    assert_eq!(text.matches("h f Q").count(), 3);
    let mut source = SeekableSource::new(Cursor::new(sink.bytes.clone())).unwrap();
    let length = source.size();
    let index = run(crate::pdf::PdfIndex::open(
        &mut source,
        crate::pdf::PdfRange { offset: 0, length },
        &limits,
        &NEVER,
    ))
    .unwrap();
    assert_eq!(index.pages().len(), 1);
    if let Some(path) = std::env::var_os("CAJ2PDF_POLYGON_TEST_OUTPUT") {
        std::fs::write(path, sink.bytes).unwrap();
    }
}

#[test]
fn invalid_polygons_and_output_failure_poison_the_content_page() {
    for case in 0..7 {
        let limits = Limits::default();
        let mut sink = Sink::default();
        let fail_now = sink.fail_now.clone();
        run(async {
            let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await.unwrap();
            let mut content = document.begin_content_page(page(), &[], &[]).await.unwrap();
            let mut points = vec![[0.0, 0.0], [20.0, 0.0], [10.0, 20.0]];
            match case {
                0 => points.clear(),
                1 => points.truncate(2),
                2 => points.resize(9, [0.0, 0.0]),
                3 => points[1][0] = f64::NAN,
                4 => points[2][1] = MAX_PDF_INTEGER as f64 + 1.0,
                5 => fail_now.set(true),
                _ => content.failed = true,
            }
            // Output is compressed in chunks, so an output failure can
            // surface when the page finishes instead of at this draw.
            let drawn = content.fill_polygon(&points).await;
            assert!(drawn.is_err() || case == 5);
            assert!(content.finish().await.is_err());
            assert!(document.finish().await.is_err());
        });
    }
}

#[test]
fn failed_or_abandoned_content_cannot_be_finished() {
    for case in 0..16 {
        let mut source = SeekableSource::new(Cursor::new(drawing_font())).unwrap();
        let limits = Limits::default();
        let font = run(TrueTypeFont::read(&mut source, &limits, &NEVER)).unwrap();
        let mut sink = Sink::default();
        let fail = sink.fail_now.clone();
        // Output is compressed in chunks: an output failure during these
        // draws can surface only when the page finishes.
        let late_failure = matches!(case, 8 | 9 | 14 | 15);
        run(async {
            let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await.unwrap();
            let font = document.add_font(&font).unwrap();
            let fonts = [&font];
            let mut content = document
                .begin_content_page(page(), &fonts, &[])
                .await
                .unwrap();
            fail.set(late_failure);
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
                14 | 15 => {
                    content
                        .decoration_glyph(0, 'A', matrix(0.0), [0.0, 0.0, 1.0, 1.0])
                        .await
                }
                8 => content.glyph_with_gray(0, 'A', matrix(0.0), 68).await,
                9..=13 => {
                    let clip = match case {
                        10 => [0.0, 0.0, 0.0, 1.0],
                        11 => [0.0, 0.0, 1.0, -1.0],
                        12 => [f64::NAN, 0.0, 1.0, 1.0],
                        13 => [MAX_PDF_INTEGER as f64, 0.0, 1.0, 1.0],
                        _ => [0.0, 0.0, 1.0, 1.0],
                    };
                    content.glyph_with_clip(0, 'A', matrix(0.0), clip).await
                }
                _ => {
                    content.failed = true;
                    content.glyph(0, 'A', matrix(0.0)).await
                }
            };
            assert!(result.is_err() || late_failure);
            assert!(content.finish().await.is_err());
            assert!(document.finish().await.is_err());
        });
    }
}

#[test]
fn failed_font_embedding_poisons_document() {
    let mut source = SeekableSource::new(Cursor::new(drawing_font())).unwrap();
    let limits = Limits::default();
    let mut font = run(TrueTypeFont::read(&mut source, &limits, &NEVER)).unwrap();
    let mut sink = Sink {
        fail_after: Some(100),
        ..Sink::default()
    };
    run(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await.unwrap();
        let handle = document.add_font(&font).unwrap();
        assert!(document.embed_font(&handle, &mut font).await.is_err());
        assert!(document.embed_font(&handle, &mut font).await.is_err());
        assert!(document.add_font(&font).is_err());
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
        document.add_font(&font).unwrap()
    });
    let mut sink = Sink::default();
    run(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await.unwrap();
        assert!(matches!(
            document.embed_font(&foreign, &mut font).await,
            Err(Error::InvalidInput {
                reason: "PDF font belongs to another document"
            })
        ));
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

fn mutate_cmap_glyph(bytes: &mut [u8], glyph: u32) {
    let table = bytes[12..]
        .as_chunks::<16>()
        .0
        .iter()
        .find(|entry| &entry[..4] == b"cmap")
        .unwrap();
    let offset = u32::from_be_bytes(table[8..12].try_into().unwrap()) as usize;
    bytes[offset + 36..offset + 40].copy_from_slice(&glyph.to_be_bytes());
}

#[test]
fn font_limits_and_invalid_glyphs_are_explicit_before_output() {
    for case in 0..2 {
        let mut source = FontSource::new();
        if case == 1 {
            mutate_cmap_glyph(&mut source.bytes, 9);
        }
        let limits = Limits::default();
        let font = run(TrueTypeFont::read(&mut source, &limits, &NEVER)).unwrap();
        let document_limits = Limits {
            io_chunk_bytes: 31,
            max_allocation_bytes: if case == 0 {
                4096
            } else {
                limits.max_allocation_bytes
            },
            ..limits
        };
        let mut sink = Sink::default();
        run(async {
            let mut document = PdfDocument::new(&mut sink, &document_limits, &NEVER)
                .await
                .unwrap();
            let before = document.writer.position();
            assert!(document.add_font(&font).is_err());
            assert_eq!(document.writer.position(), before);
            assert!(!document.image_page_failed);
            if case == 1 {
                // A refused font leaves the document able to finish. (A
                // content page needs its compressor, which case 0's
                // 4 KiB allocation limit refuses.)
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
fn width_spans_cover_unused_characters_between_used_ones() {
    let mut source = FontSource::new();
    let table = source.bytes[12..]
        .as_chunks::<16>()
        .0
        .iter()
        .find(|entry| &entry[..4] == b"cmap")
        .unwrap();
    let offset = u32::from_be_bytes(table[8..12].try_into().unwrap()) as usize;
    // Relabel the triangle from U+4E2D to C, two codes after A.
    for at in [offset + 40, offset + 44] {
        source.bytes[at..at + 4].copy_from_slice(&0x43_u32.to_be_bytes());
    }
    let limits = Limits::default();
    let mut font = run(TrueTypeFont::read(&mut source, &limits, &NEVER)).unwrap();
    let mut sink = Sink::default();
    run(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await.unwrap();
        let handle = document.add_font(&font).unwrap();
        let fonts = [&handle];
        let mut content = document
            .begin_content_page(page(), &fonts, &[])
            .await
            .unwrap();
        content.glyph(0, 'C', matrix(10.0)).await.unwrap();
        content.glyph(0, 'A', matrix(30.0)).await.unwrap();
        content.finish().await.unwrap();
        document.embed_font(&handle, &mut font).await.unwrap();
        document.finish().await.unwrap();
    });
    let text = String::from_utf8_lossy(&sink.bytes);
    assert!(text.contains("/W [ 65 [ 600 1000 1000 ] ]"));
}

#[test]
fn changed_font_sources_are_rejected_when_embedding() {
    for case in 0..3 {
        let limits = Limits::default();
        let mut original = FontSource::new();
        let font = run(TrueTypeFont::read(&mut original, &limits, &NEVER)).unwrap();
        // Any metadata change, including a remap to another existing glyph,
        // differs from the fingerprint recorded when the font was added.
        let mut changed = FontSource::new();
        match case {
            0 => {
                let table = changed.bytes[12..]
                    .as_chunks::<16>()
                    .0
                    .iter()
                    .find(|entry| &entry[..4] == b"name")
                    .unwrap();
                let offset = u32::from_be_bytes(table[8..12].try_into().unwrap()) as usize;
                changed.bytes[offset + 19] = b'D';
            }
            1 => mutate_cmap_glyph(&mut changed.bytes, 0),
            _ => mutate_cmap_glyph(&mut changed.bytes, 2),
        }
        let mut sink = Sink::default();
        run(async {
            let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await.unwrap();
            let handle = document.add_font(&font).unwrap();
            let fonts = [&handle];
            let mut content = document
                .begin_content_page(page(), &fonts, &[])
                .await
                .unwrap();
            content.glyph(0, 'A', matrix(10.0)).await.unwrap();
            content.finish().await.unwrap();
            let mut font = TrueTypeFont::read(&mut changed, &limits, &NEVER)
                .await
                .unwrap();
            let result = document.embed_font(&handle, &mut font).await;
            assert!(matches!(
                result,
                Err(Error::InvalidInput {
                    reason: "font source changed after its metadata was read"
                })
            ));
            // Changes are detected before output; the document survives.
            document.add_font(&font).unwrap();
        });
    }
}

#[test]
fn failed_or_abandoned_subset_planning_can_be_retried() {
    use std::{
        future::Future,
        task::{Context, Poll, Waker},
    };
    for pending in [false, true] {
        let mut source = FontSource::new();
        let limits = Limits::default();
        let mut font = run(TrueTypeFont::read(&mut source, &limits, &NEVER)).unwrap();
        let mut sink = Sink::default();
        let mut document = run(PdfDocument::new(&mut sink, &limits, &NEVER)).unwrap();
        let handle = document.add_font(&font).unwrap();
        let content = run(document.begin_content_page(page(), &[], &[])).unwrap();
        run(content.finish()).unwrap();
        font.source.pending = pending;
        font.source.fail = !pending;
        if pending {
            let mut future = std::pin::pin!(document.embed_font(&handle, &mut font));
            assert!(matches!(
                future
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop())),
                Poll::Pending
            ));
        } else {
            assert!(run(document.embed_font(&handle, &mut font)).is_err());
        }
        // Planning wrote nothing, so the same embedding can be retried.
        font.source.pending = false;
        font.source.fail = false;
        run(document.embed_font(&handle, &mut font)).unwrap();
        run(document.finish()).unwrap();
    }
}

#[test]
fn subset_reads_count_toward_the_input_limit() {
    let limits = Limits::default();
    let mut source = FontSource::new();
    let mut font = run(TrueTypeFont::read(&mut source, &limits, &NEVER)).unwrap();
    let mut sink = Sink::default();
    let total = run(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await.unwrap();
        let handle = document.add_font(&font).unwrap();
        let fonts = [&handle];
        let mut content = document
            .begin_content_page(page(), &fonts, &[])
            .await
            .unwrap();
        content.glyph(0, '中', matrix(10.0)).await.unwrap();
        content.finish().await.unwrap();
        document.embed_font(&handle, &mut font).await.unwrap();
        assert!(matches!(
            document.begin_content_page(page(), &fonts, &[]).await,
            Err(Error::InvalidInput {
                reason: "PDF font subset is already embedded"
            })
        ));
        document.finish().await.unwrap().input_bytes_read
    });
    assert_eq!(total, font.subset_bytes_read());
    let mut used = vec![0; 8192];
    used[0x4e2d / 8] |= 1 << (0x4e2d % 8);
    let mut source = FontSource::new();
    let mut font = run(TrueTypeFont::read(&mut source, &limits, &NEVER)).unwrap();
    run(font.plan_subset(&used, u64::MAX, &limits, &NEVER)).unwrap();
    let planned = font.subset_bytes_read();
    assert!(planned < total);
    // Exceeding the limit while planning leaves the document usable;
    // exceeding it while writing poisons it.
    for (limit, poisoned) in [(planned - 1, false), (planned, true)] {
        let mut source = FontSource::new();
        let mut font = run(TrueTypeFont::read(&mut source, &limits, &NEVER)).unwrap();
        let document_limits = Limits {
            max_input_bytes: limit,
            ..limits
        };
        let mut sink = Sink::default();
        run(async {
            let mut document = PdfDocument::new(&mut sink, &document_limits, &NEVER)
                .await
                .unwrap();
            let handle = document.add_font(&font).unwrap();
            let fonts = [&handle];
            let mut content = document
                .begin_content_page(page(), &fonts, &[])
                .await
                .unwrap();
            content.glyph(0, '中', matrix(10.0)).await.unwrap();
            content.finish().await.unwrap();
            assert!(matches!(
                document.embed_font(&handle, &mut font).await,
                Err(Error::LimitExceeded {
                    resource: "input bytes",
                    ..
                })
            ));
            assert_eq!(document.add_font(&font).is_err(), poisoned);
        });
    }
}

#[test]
fn subset_tags_distinguish_programs_with_one_name() {
    let tags: Vec<_> = [600_u16, 610]
        .into_iter()
        .map(|advance| {
            let mut source = FontSource::new();
            let table = source.bytes[12..]
                .as_chunks::<16>()
                .0
                .iter()
                .find(|entry| &entry[..4] == b"hmtx")
                .unwrap();
            let offset = u32::from_be_bytes(table[8..12].try_into().unwrap()) as usize;
            source.bytes[offset + 4..offset + 6].copy_from_slice(&advance.to_be_bytes());
            let limits = Limits::default();
            let mut font = run(TrueTypeFont::read(&mut source, &limits, &NEVER)).unwrap();
            let mut sink = Sink::default();
            run(async {
                let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await.unwrap();
                let handle = document.add_font(&font).unwrap();
                let fonts = [&handle];
                let mut content = document
                    .begin_content_page(page(), &fonts, &[])
                    .await
                    .unwrap();
                content.glyph(0, 'A', matrix(10.0)).await.unwrap();
                content.finish().await.unwrap();
                document.embed_font(&handle, &mut font).await.unwrap();
                document.finish().await.unwrap();
            });
            let text = String::from_utf8_lossy(&sink.bytes).into_owned();
            text.split("/FontName /").nth(1).unwrap()[..6].to_owned()
        })
        .collect();
    assert_ne!(tags[0], tags[1]);
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
    for (pending, kind) in [
        (false, 0),
        (true, 0),
        (false, 1),
        (true, 1),
        (false, 2),
        (true, 2),
        (false, 3),
        (true, 3),
        (false, 4),
        (true, 4),
        (false, 5),
        (true, 5),
    ] {
        let mut source = FontSource::new();
        let limits = Limits::default();
        let font = run(TrueTypeFont::read(&mut source, &limits, &NEVER)).unwrap();
        let mut sink = Sink::default();
        let suspend = sink.pending.clone();
        let cancelled = Rc::new(Cell::new(false));
        let cancellation = Cancel(cancelled.clone());
        let mut document = run(PdfDocument::new(&mut sink, &limits, &cancellation)).unwrap();
        let font = document.add_font(&font).unwrap();
        let fonts = [&font];
        let mut page = run(document.begin_content_page(page(), &fonts, &[])).unwrap();
        let draw = async {
            if kind == 5 {
                page.stroke_polyline(&[[10.0, 10.0], [30.0, 20.0]], 2.0, 68)
                    .await
            } else if kind == 1 {
                page.fill_polygon(&[[10.0, 10.0], [30.0, 10.0], [20.0, 30.0]])
                    .await
            } else if kind == 4 {
                page.decoration_glyph(0, 'A', matrix(10.0), [10.0, 50.0, 5.0, 20.0])
                    .await
            } else if kind == 3 {
                page.glyph_with_clip(0, 'A', matrix(10.0), [10.0, 50.0, 5.0, 20.0])
                    .await
            } else if kind == 2 {
                page.glyph_with_gray(0, 'A', matrix(10.0), 68).await
            } else {
                page.glyph(0, 'A', matrix(10.0)).await
            }
        };
        if pending {
            // Draws are buffered; the page's output is written, and here
            // abandoned while pending, when it finishes.
            suspend.set(true);
            run(draw).unwrap();
            let mut future = std::pin::pin!(page.finish());
            assert!(matches!(
                future
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop())),
                Poll::Pending
            ));
        } else {
            cancelled.set(true);
            assert!(matches!(run(draw), Err(Error::Cancelled)));
            cancelled.set(false);
            assert!(run(page.finish()).is_err());
        }
        suspend.set(false);
        assert!(run(document.finish()).is_err());
    }
}

#[test]
fn notdef_is_missing_and_postscript_hash_is_escaped() {
    let mut source = FontSource::new();
    let offset = |bytes: &[u8], tag: &[u8; 4]| {
        let table = bytes[12..]
            .as_chunks::<16>()
            .0
            .iter()
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
        let handle = document.add_font(&font).unwrap();
        assert!(!handle.supports('A'));
        assert!(handle.supports('中'));
        document
            .begin_content_page(page(), &[&handle], &[])
            .await
            .unwrap()
            .finish()
            .await
            .unwrap();
        // An unused font still embeds a valid `.notdef`-only subset.
        document.embed_font(&handle, &mut font).await.unwrap();
        document.finish().await.unwrap();
    });
    let text = String::from_utf8_lossy(&sink.bytes);
    assert!(text.contains("+#23ajFixture /Flags"));
    assert!(text.contains("/DW 1000 /W [ ] >>"));
    let program = inflated_stream(&sink.bytes, b"/Length1 ");
    let subset = xberg_ttf_parser::Face::parse(&program, 0).unwrap();
    assert_eq!(subset.number_of_glyphs(), 1);
    let mapping = text
        .split("/CIDToGIDMap ")
        .nth(1)
        .unwrap()
        .split(' ')
        .next()
        .unwrap();
    let map = inflated_stream(&sink.bytes, format!("\n{mapping} 0 obj").as_bytes());
    assert!(map.is_empty());
}

#[test]
fn joined_stroke_preserves_vertices_gray_and_failure_state() {
    let limits = Limits::default();
    let mut sink = Sink::default();
    run(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await.unwrap();
        let mut content = document.begin_content_page(page(), &[], &[]).await.unwrap();
        content
            .stroke_polyline(&[[10.0, 10.0], [20.0, 30.0], [40.0, 30.0]], 2.0, 68)
            .await
            .unwrap();
        content.finish().await.unwrap();
        document.finish().await.unwrap();
    });
    let text = String::from_utf8_lossy(&inflated_pdf(&sink.bytes))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(text.contains("q 0 J 0 j 10 M 2 w 0.266667 G 10 10 m 20 30 l 40 30 l S Q"));
    for case in 0..9 {
        let mut sink = Sink::default();
        let fail = sink.fail_now.clone();
        run(async {
            let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).await.unwrap();
            let mut content = document.begin_content_page(page(), &[], &[]).await.unwrap();
            let mut points = vec![[0.0, 0.0], [20.0, 30.0]];
            let mut width = 2.0;
            match case {
                0 => points.clear(),
                1 => points.resize(9, [0.0, 0.0]),
                2 => width = -1.0,
                3 => width = f64::NAN,
                4 => points[1][0] = f64::INFINITY,
                5 => points[1][1] = f64::NAN,
                6 => fail.set(true),
                7 => content.failed = true,
                _ => width = MAX_PDF_INTEGER as f64 + 1.0,
            }
            let drawn = content.stroke_polyline(&points, width, 0).await;
            assert!(drawn.is_err() || case == 6);
            assert!(content.finish().await.is_err());
            assert!(document.finish().await.is_err());
        });
    }
}

#[test]
fn a_draw_abandoned_while_flushing_cannot_publish_its_page() {
    use std::{
        future::Future,
        task::{Context, Poll, Waker},
    };
    let limits = Limits::default();
    let mut source = SeekableSource::new(Cursor::new(drawing_font())).unwrap();
    let font = run(TrueTypeFont::read(&mut source, &limits, &NEVER)).unwrap();
    let mut sink = Sink::default();
    let suspend = sink.pending.clone();
    let mut document = run(PdfDocument::new(&mut sink, &limits, &NEVER)).unwrap();
    let handle = document.add_font(&font).unwrap();
    let fonts = [&handle];
    let mut page = run(document.begin_content_page(page(), &fonts, &[])).unwrap();
    suspend.set(true);
    // Buffered draws complete until one fills the buffer and must write.
    let mut draws = 0;
    loop {
        draws += 1;
        let mut draw = std::pin::pin!(page.glyph(0, 'A', matrix(10.0)));
        match draw.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(result) => result.unwrap(),
            Poll::Pending => break,
        }
    }
    assert!(draws > 1);
    suspend.set(false);
    assert!(run(page.glyph(0, 'A', matrix(10.0))).is_err());
    assert!(run(page.finish()).is_err());
    assert!(run(document.finish()).is_err());
}
