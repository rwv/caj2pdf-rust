// SPDX-License-Identifier: MIT

use super::*;
use crate::pdf::font::tests::{CffOptions, collection_font, otf};

/// Change an opaque trailing byte only when the emission pass reaches it.
/// Metadata and the planning pass still return the original font.
struct ChangingFont {
    bytes: Vec<u8>,
    starts: usize,
}

impl RangedSource for ChangingFont {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize> {
        if offset == 0 {
            self.starts += 1;
        }
        let start = offset as usize;
        let count = out.len().min(self.bytes.len().saturating_sub(start));
        out[..count].copy_from_slice(&self.bytes[start..start + count]);
        if self.starts >= 3 && count != 0 && start + count == self.bytes.len() {
            out[count - 1] ^= 1;
        }
        Ok(count)
    }
}

#[test]
fn changed_complete_font_program_prevents_finishing_document() {
    let mut bytes = drawing_font();
    bytes.push(0);
    let mut source = ChangingFont { bytes, starts: 0 };
    let limits = Limits {
        io_chunk_bytes: 31,
        ..Limits::default()
    };
    let mut font = OpenTypeFont::read(&mut source, 0, &limits, &NEVER).unwrap();
    let mut sink = Sink::default();
    let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).unwrap();
    let handle = document.add_font(&font).unwrap();
    let fonts = [&handle];
    let mut content = document.begin_content_page(page(), &fonts, &[]).unwrap();
    content.glyph(0, 'A', matrix(10.0)).unwrap();
    content.finish().unwrap();
    let error = document
        .embed_font_with_embedding(&handle, &mut font, FontEmbedding::Full)
        .unwrap_err();
    assert_eq!(error.reason, CHANGED);
    assert!(document.image_page_failed);
    assert!(document.finish().is_err());
}

#[test]
fn full_program_preserves_original_gids_and_independent_semantic_text() {
    for collection in [false, true] {
        for mapped in [false, true] {
            let bytes = if collection {
                collection_font()
            } else {
                drawing_font()
            };
            let mut source = &bytes[..];
            let limits = Limits {
                io_chunk_bytes: 31,
                ..Limits::default()
            };
            let mut font = OpenTypeFont::read(&mut source, 0, &limits, &NEVER).unwrap();
            let gid = font.glyph('中').unwrap().id;
            let mut sink = Sink::default();
            let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).unwrap();
            let handle = if mapped {
                document.add_mapped_font(&font)
            } else {
                document.add_font(&font)
            }
            .unwrap();
            let fonts = [&handle];
            let mut content = document.begin_content_page(page(), &fonts, &[]).unwrap();
            if mapped {
                // One source glyph, two meanings, including a non-BMP scalar.
                content.mapped_glyph(0, '中', 'A', matrix(10.0), 0).unwrap();
                content
                    .mapped_glyph(0, '中', '😀', matrix(40.0), 0)
                    .unwrap();
            } else {
                content.glyph(0, '中', matrix(10.0)).unwrap();
            }
            content.finish().unwrap();
            document
                .embed_font_with_embedding(&handle, &mut font, FontEmbedding::Full)
                .unwrap();
            let report = document.finish().unwrap();
            assert!(report.input_bytes_read > 0);
            let text = pdf_text(&sink.bytes);
            assert!(text.contains("/BaseFont /CajFixture /"));
            assert!(!text.contains("+CajFixture"));
            let program = inflated_stream(&sink.bytes, b"/Length1 ");
            if !collection {
                assert_eq!(program, bytes);
            }
            let face = xberg_ttf_parser::Face::parse(&program, 0).unwrap();
            assert_eq!(face.number_of_glyphs(), 3);
            assert_eq!(face.glyph_index('中').unwrap().0, gid);
            let mapping = text
                .split("/CIDToGIDMap ")
                .nth(1)
                .unwrap()
                .split(' ')
                .next()
                .unwrap();
            let map = inflated_stream(&sink.bytes, format!("\n{mapping} 0 obj").as_bytes());
            if mapped {
                assert_eq!(map, [0, 0, 0, gid as u8, 0, gid as u8]);
                let cmap = text
                    .split("/ToUnicode ")
                    .nth(1)
                    .unwrap()
                    .split(' ')
                    .next()
                    .unwrap();
                let unicode = inflated_stream(&sink.bytes, format!("\n{cmap} 0 obj").as_bytes());
                let unicode = String::from_utf8(unicode).unwrap();
                assert!(unicode.contains("<0001> <0041>"));
                assert!(unicode.contains("<0002> <D83DDE00>"));
            } else {
                assert_eq!(&map[2 * '中' as usize..], gid.to_be_bytes());
            }
            if let Some(root) = std::env::var_os("CAJ2PDF_FONT_TEST_OUTPUT") {
                let root = std::path::PathBuf::from(root);
                std::fs::create_dir_all(&root).unwrap();
                let path = root.join(format!(
                    "complete-collection-{collection}-mapped-{mapped}.pdf"
                ));
                std::fs::write(path, &sink.bytes).unwrap();
            }
        }
    }
}

#[test]
fn full_option_keeps_unused_resources_small_and_refuses_used_cff_before_writing() {
    for cff in [false, true] {
        for drawn in [false, true] {
            let bytes = if cff {
                otf(&CffOptions::default())
            } else {
                drawing_font()
            };
            let mut source = &bytes[..];
            let limits = Limits::default();
            let mut font = OpenTypeFont::read(&mut source, 0, &limits, &NEVER).unwrap();
            let mut sink = Sink::default();
            let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).unwrap();
            let handle = document.add_font(&font).unwrap();
            let fonts = [&handle];
            let mut content = document.begin_content_page(page(), &fonts, &[]).unwrap();
            if drawn {
                content.glyph(0, '中', matrix(10.0)).unwrap();
            }
            content.finish().unwrap();
            let before = document.writer.position();
            let result =
                document.embed_font_with_embedding(&handle, &mut font, FontEmbedding::Full);
            if cff && drawn {
                assert_eq!(
                    result.unwrap_err().reason,
                    "complete font embedding requires TrueType outlines"
                );
                assert_eq!(document.writer.position(), before);
                assert!(!document.image_page_failed);
                document.embed_font(&handle, &mut font).unwrap();
            } else {
                result.unwrap();
            }
            document.finish().unwrap();
            if !cff && !drawn {
                let program = inflated_stream(&sink.bytes, b"/Length1 ");
                let face = xberg_ttf_parser::Face::parse(&program, 0).unwrap();
                assert_eq!(face.number_of_glyphs(), 1);
            }
        }
    }
}

#[test]
fn full_font_read_failure_can_retry_but_output_failure_poisons_document() {
    let mut source = FontSource::new();
    let limits = Limits::default();
    let mut font = OpenTypeFont::read(&mut source, 0, &limits, &NEVER).unwrap();
    let mut sink = Sink::default();
    let failed = sink.fail_now.clone();
    let mut document = PdfDocument::new(&mut sink, &limits, &NEVER).unwrap();
    let handle = document.add_font(&font).unwrap();
    let fonts = [&handle];
    let mut content = document.begin_content_page(page(), &fonts, &[]).unwrap();
    content.glyph(0, 'A', matrix(10.0)).unwrap();
    content.finish().unwrap();
    font.source.fail = true;
    let before = document.writer.position();
    assert!(
        document
            .embed_font_with_embedding(&handle, &mut font, FontEmbedding::Full)
            .is_err()
    );
    assert_eq!(document.writer.position(), before);
    assert!(!document.image_page_failed);
    font.source.fail = false;
    failed.set(true);
    assert!(
        document
            .embed_font_with_embedding(&handle, &mut font, FontEmbedding::Full)
            .is_err()
    );
    assert!(document.image_page_failed);
    failed.set(false);
    assert!(
        document
            .embed_font_with_embedding(&handle, &mut font, FontEmbedding::Full)
            .is_err()
    );
    assert!(document.finish().is_err());
}
