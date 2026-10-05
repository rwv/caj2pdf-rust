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
pub(crate) use original_font::{drawing_font, symbol_font};
use original_font::{entry, put16, put32};

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
        let mut font = run(TrueTypeFont::read(
            &mut source,
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
        let font = run(TrueTypeFont::read(&mut source, &limits, &NeverCancel)).unwrap();
        assert_eq!(font.outlines[0], Some((outline, 2 * 1024 * 1024)));
        assert_eq!(font.outlines[2..], [None; 3]);
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
            run(TrueTypeFont::read(
                &mut source,
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
        run(TrueTypeFont::read(&mut source, &Limits::default(), &Cancel)),
        Err(Error::Cancelled)
    ));
    assert_eq!(source.requested, 0);
    let limits = Limits {
        max_input_bytes: 12,
        ..Limits::default()
    };
    assert!(matches!(
        run(TrueTypeFont::read(&mut source, &limits, &NeverCancel)),
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
        run(TrueTypeFont::read(&mut source, &limits, &NeverCancel)),
        Err(Error::LimitExceeded {
            resource: "allocation bytes",
            ..
        })
    ));
    source.bytes.truncate(11);
    assert!(matches!(
        run(TrueTypeFont::read(
            &mut source,
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
        run(TrueTypeFont::read(
            &mut source,
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
        run(TrueTypeFont::read(
            &mut source,
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
}
