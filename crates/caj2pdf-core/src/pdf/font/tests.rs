// SPDX-License-Identifier: MIT

use super::*;
use crate::NeverCancel;
use std::{
    future::Future,
    task::{Context, Poll, Waker},
};

fn run<F: Future>(future: F) -> F::Output {
    match std::pin::pin!(future)
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("unexpected pending fixture"),
    }
}

struct Source {
    bytes: Vec<u8>,
    short: usize,
    requested: usize,
    outline: u64,
}
impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }
    async fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize> {
        assert!(
            offset + out.len() as u64 <= self.outline,
            "outline payload must remain unread"
        );
        self.requested = self.requested.max(out.len());
        let start = offset as usize;
        let count = out
            .len()
            .min(self.short)
            .min(self.bytes.len().saturating_sub(start));
        out[..count].copy_from_slice(&self.bytes[start..start + count]);
        Ok(count)
    }
}

#[allow(dead_code)]
mod original_font {
    include!("../../../tests/common/font_fixture.rs");
}
pub(crate) use original_font::{
    CALLSUBR, CffOptions, ENDCHAR, RETURN, RLINETO, RMOVETO, Tables, build, charstrings,
    collection_font, drawing_font, entry, get32, num, ops, otf, symbol_font, table, tables,
};
use original_font::{put16, put32};

fn fixture() -> Source {
    let (bytes, outline) = original_font::metadata_font();
    Source {
        bytes,
        short: 2,
        requested: 0,
        outline,
    }
}

#[test]
fn postscript_names_are_bounded_and_validated_before_embedding() {
    for case in 0..6 {
        let mut source = fixture();
        let mut font = run(OpenTypeFont::read(
            &mut source,
            0,
            &Limits::default(),
            &NeverCancel,
        ))
        .unwrap();
        assert_eq!(font.postscript_name().unwrap(), "CajFixture");
        let name = &mut font.tables[7];
        match case {
            0 => put16(name, 6, 1), // unsupported name encoding
            1 => put16(name, 14, 0),
            2 => put16(name, 14, 1),
            3 => {
                name.resize(146, 0);
                put16(name, 14, 128);
            }
            4 => put16(name, 18, 0xd800),
            _ => put16(name, 18, u16::from(b'/')),
        }
        assert!(font.postscript_name().is_err(), "case {case}");
    }
}

#[test]
fn ranged_metadata_maps_unicode_without_reading_outlines() {
    for chunk in [1, 3, 256] {
        let mut source = fixture();
        let outline = source.outline;
        let limits = Limits {
            io_chunk_bytes: chunk,
            ..Limits::default()
        };
        let font = run(OpenTypeFont::read(&mut source, 0, &limits, &NeverCancel)).unwrap();
        assert_eq!(font.outlines[0], Some((outline, 2 * 1024 * 1024)));
        assert_eq!(font.outlines[2..], [None; 4]);
        assert_eq!(font.units_per_em().unwrap(), 1000);
        assert_eq!(
            font.glyph('A').unwrap(),
            FontGlyph {
                id: 1,
                advance: 600
            }
        );
        assert_eq!(
            font.glyph('中').unwrap(),
            FontGlyph {
                id: 2,
                advance: 1000
            }
        );
        assert!(font.glyph('B').is_err());
        assert!(font.tables.iter().map(Vec::len).sum::<usize>() < 1024);
        drop(font);
        assert!(source.requested <= chunk);
    }
}

#[test]
fn metadata_and_directory_fail_closed() {
    let mutations: &[fn(&mut Source)] = &[
        |s| s.bytes[0] = 1,
        |s| put16(&mut s.bytes, 4, 0),
        |s| put16(&mut s.bytes, 4, 129),
        |s| {
            let tag = s.bytes[12..16].to_vec();
            s.bytes[28..32].copy_from_slice(&tag);
        },
        |s| put32(&mut s.bytes, 20, 0),
        |s| put32(&mut s.bytes, 20, 201),
        |s| put32(&mut s.bytes, 24, u32::MAX),
        |s| {
            let offset = s.bytes[20..24].to_vec();
            s.bytes[36..40].copy_from_slice(&offset);
        },
        |s| {
            let i = entry(&s.bytes, b"glyf");
            s.bytes[i..i + 4].copy_from_slice(b"gxxx");
        },
        |s| {
            let i = entry(&s.bytes, b"glyf");
            s.bytes[i..i + 4].copy_from_slice(b"fvar");
        },
        |s| {
            let i = entry(&s.bytes, b"post");
            s.bytes[i..i + 4].copy_from_slice(b"pxxx");
        },
        |s| {
            let i = entry(&s.bytes, b"head");
            put32(&mut s.bytes, i + 12, 0);
        },
        |s| {
            let i = entry(&s.bytes, b"cmap");
            put32(&mut s.bytes, i + 12, 0);
        },
        |s| {
            let i = entry(&s.bytes, b"OS/2");
            let (at, _) = span(s.bytes[i..i + 16].try_into().unwrap());
            put16(&mut s.bytes, at as usize + 8, 2);
        },
        |s| {
            let i = entry(&s.bytes, b"OS/2");
            let (at, _) = span(s.bytes[i..i + 16].try_into().unwrap());
            put16(&mut s.bytes, at as usize + 8, 0x200);
        },
    ];
    for (index, mutation) in mutations.iter().enumerate() {
        let mut source = fixture();
        mutation(&mut source);
        assert!(
            run(OpenTypeFont::read(
                &mut source,
                0,
                &Limits::default(),
                &NeverCancel
            ))
            .is_err(),
            "mutation {index}"
        );
    }
}

#[test]
fn budget_and_cancellation_precede_payload_reads() {
    struct Cancel;
    impl Cancellation for Cancel {
        fn is_cancelled(&self) -> bool {
            true
        }
    }
    let mut source = fixture();
    assert!(matches!(
        run(OpenTypeFont::read(
            &mut source,
            0,
            &Limits::default(),
            &Cancel
        )),
        Err(Error::Cancelled)
    ));
    assert_eq!(source.requested, 0);
    let limits = Limits {
        max_input_bytes: 12,
        ..Limits::default()
    };
    assert!(matches!(
        run(OpenTypeFont::read(&mut source, 0, &limits, &NeverCancel)),
        Err(Error::LimitExceeded {
            resource: "input bytes",
            ..
        })
    ));
    let limits = Limits {
        io_chunk_bytes: 1,
        max_allocation_bytes: 32,
        ..Limits::default()
    };
    assert!(matches!(
        run(OpenTypeFont::read(&mut source, 0, &limits, &NeverCancel)),
        Err(Error::LimitExceeded {
            resource: "allocation bytes",
            ..
        })
    ));
    source.bytes.truncate(11);
    assert!(matches!(
        run(OpenTypeFont::read(
            &mut source,
            0,
            &Limits::default(),
            &NeverCancel
        )),
        Err(Error::TruncatedInput { .. })
    ));
}

#[test]
fn oversized_metadata_is_rejected_without_reading_or_allocating_it() {
    let mut source = fixture();
    let glyf = entry(&source.bytes, b"glyf");
    put32(&mut source.bytes, glyf + 12, 0);
    let post = entry(&source.bytes, b"post");
    put32(&mut source.bytes, post + 8, source.outline as u32);
    put32(
        &mut source.bytes,
        post + 12,
        MAX_FONT_METADATA_BYTES as u32 + 1,
    );
    assert!(matches!(
        run(OpenTypeFont::read(
            &mut source,
            0,
            &Limits::default(),
            &NeverCancel
        )),
        Err(Error::LimitExceeded {
            resource: "font metadata bytes",
            ..
        })
    ));
}

#[test]
fn many_character_maps_cannot_multiply_mapping_work_without_a_bound() {
    let mut source = fixture();
    let cmap = entry(&source.bytes, b"cmap");
    let (old, _) = span(source.bytes[cmap..cmap + 16].try_into().unwrap());
    let mapping = source.bytes[old as usize + 12..old as usize + 52].to_vec();
    let mut table = vec![0; 4 + 17 * 8];
    put16(&mut table, 2, 17);
    for i in 0..17 {
        put16(&mut table, 4 + i * 8, 3);
        put16(&mut table, 6 + i * 8, 10);
        put32(&mut table, 8 + i * 8, 4 + 17 * 8);
    }
    table.extend(mapping);
    let offset = source.outline as usize;
    source.bytes[offset..offset + table.len()].copy_from_slice(&table);
    put32(&mut source.bytes, cmap + 8, offset as u32);
    put32(&mut source.bytes, cmap + 12, table.len() as u32);
    let glyf = entry(&source.bytes, b"glyf");
    put32(&mut source.bytes, glyf + 12, 0);
    source.outline += table.len() as u64;
    assert!(matches!(
        run(OpenTypeFont::read(
            &mut source,
            0,
            &Limits::default(),
            &NeverCancel
        )),
        Err(Error::LimitExceeded {
            resource: "font character maps",
            limit: 16,
            attempted: 17
        })
    ));
}

#[test]
fn shared_cross_runtime_font_matches_original_generator() {
    assert_eq!(
        symbol_font(),
        include_bytes!("../../../../../tests/fonts/symbols.ttf")
    );
    assert_eq!(
        drawing_font(),
        include_bytes!("../../../../../tests/fonts/geometric.ttf")
    );
    assert_eq!(
        collection_font(),
        include_bytes!("../../../../../tests/fonts/collection.ttc")
    );
    assert_eq!(
        otf(&CffOptions::default()),
        include_bytes!("../../../../../tests/fonts/geometric.otf")
    );
}

fn read_face(bytes: Vec<u8>, face: u32) -> Result<(char, u16)> {
    let mut source = crate::native::SeekableSource::new(std::io::Cursor::new(bytes)).unwrap();
    let font = run(OpenTypeFont::read(
        &mut source,
        face,
        &Limits::default(),
        &NeverCancel,
    ))?;
    let face = font.face()?;
    let mapped = [' ', 'A']
        .into_iter()
        .find(|character| face.glyph_index(*character).is_some())
        .unwrap();
    Ok((mapped, face.number_of_glyphs()))
}

#[test]
fn collection_faces_are_selected_by_index() {
    assert_eq!(read_face(collection_font(), 0).unwrap(), ('A', 3));
    assert_eq!(read_face(collection_font(), 1).unwrap(), (' ', 3));
    // Apple's `true` tag names the same TrueType outlines.
    let mut apple = drawing_font();
    apple[..4].copy_from_slice(b"true");
    assert_eq!(read_face(apple, 0).unwrap(), ('A', 3));
    // Faces of one collection have distinct identities.
    let limits = Limits::default();
    let fingerprint = |face| {
        let mut source =
            crate::native::SeekableSource::new(std::io::Cursor::new(collection_font())).unwrap();
        run(OpenTypeFont::read(&mut source, face, &limits, &NeverCancel))
            .unwrap()
            .fingerprint()
    };
    assert_ne!(fingerprint(0), fingerprint(1));
}

#[test]
fn collection_headers_and_face_indices_fail_closed() {
    let reason = |bytes: Vec<u8>, face| match read_face(bytes, face) {
        Err(Error::InvalidInput { reason }) => reason,
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(
        reason(collection_font(), 2),
        "TrueType collection face index is out of range"
    );
    assert_eq!(
        reason(drawing_font(), 1),
        "a standalone font has only face 0"
    );
    let mut version = collection_font();
    put16(&mut version, 4, 3);
    assert_eq!(
        reason(version, 0),
        "unsupported TrueType collection version"
    );
    let mut unknown = collection_font();
    let base = u32::from_be_bytes(unknown[12..16].try_into().unwrap()) as usize;
    unknown[base..base + 4].copy_from_slice(b"wOFF");
    assert_eq!(
        reason(unknown, 0),
        "font must be an OpenType font or collection face"
    );
    // No table may overlap the collection header (with version 2's DSIG
    // fields) or its face's directory.
    let face_one = get32(&collection_font(), 16);
    for (version, face, offset, length) in [
        (1, 0, 24, 4),
        (1, 0, 8, 4),
        (1, 1, face_one - 4, 64),
        (2, 1, 28, 4),
    ] {
        let mut bytes = collection_font();
        put16(&mut bytes, 4, version);
        let entry = get32(&bytes, 12 + 4 * face as usize) as usize + 12;
        put32(&mut bytes, entry + 8, offset);
        put32(&mut bytes, entry + 12, length);
        assert_eq!(
            reason(bytes, face),
            "TrueType table range is outside the source",
            "{version} {face} {offset}"
        );
    }
}

#[test]
fn face_counts_read_only_the_header() {
    let count = |bytes: Vec<u8>| {
        let mut source = crate::native::SeekableSource::new(std::io::Cursor::new(bytes)).unwrap();
        run(OpenTypeFont::face_count(
            &mut source,
            &Limits::default(),
            &NeverCancel,
        ))
    };
    let mut apple = drawing_font();
    apple[..4].copy_from_slice(b"true");
    for (bytes, faces) in [
        (collection_font(), 2),
        (drawing_font(), 1),
        (otf(&CffOptions::default()), 1),
        (apple, 1),
    ] {
        assert_eq!(count(bytes).unwrap(), faces);
    }
    // The header alone is read: a face beyond the source is still counted.
    let mut header = collection_font()[..12].to_vec();
    put32(&mut header, 8, 70_000);
    assert_eq!(count(header).unwrap(), 70_000);
    let mut version = collection_font();
    put16(&mut version, 4, 3);
    let reason = |bytes| match count(bytes) {
        Err(Error::InvalidInput { reason }) => reason,
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(reason(version), "unsupported TrueType collection version");
    assert_eq!(
        reason(b"wOFF\0\0\0\0\0\0\0\0".to_vec()),
        "font must be an OpenType font or collection face"
    );
    assert!(count(b"ttcf".to_vec()).is_err());
    let mut source = fixture();
    let limits = Limits {
        max_input_bytes: 1,
        ..Limits::default()
    };
    assert!(matches!(
        run(OpenTypeFont::face_count(&mut source, &limits, &NeverCancel)),
        Err(Error::LimitExceeded { .. })
    ));
}
