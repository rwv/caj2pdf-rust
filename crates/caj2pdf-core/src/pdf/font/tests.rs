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

fn put16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}
fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

// Original minimal metadata. Glyph slots have empty outlines; this fixture
// proves character/metric access, not rendered font validity or C8 fidelity.
fn fixture() -> Source {
    let mut head = vec![0; 54];
    put32(&mut head, 0, 0x10000);
    put32(&mut head, 12, 0x5f0f3cf5);
    put16(&mut head, 18, 1000);
    let mut hhea = vec![0; 36];
    put32(&mut hhea, 0, 0x10000);
    put16(&mut hhea, 4, 800);
    put16(&mut hhea, 6, (-200_i16) as u16);
    put16(&mut hhea, 34, 3);
    let mut maxp = vec![0; 32];
    put32(&mut maxp, 0, 0x10000);
    put16(&mut maxp, 4, 3);
    let mut cmap = vec![0; 52];
    put16(&mut cmap, 2, 1);
    put16(&mut cmap, 4, 3);
    put16(&mut cmap, 6, 10);
    put32(&mut cmap, 8, 12);
    put16(&mut cmap, 12, 12);
    put32(&mut cmap, 16, 40);
    put32(&mut cmap, 24, 2);
    for (index, code) in [65, 0x4e2d].into_iter().enumerate() {
        let at = 28 + index * 12;
        put32(&mut cmap, at, code);
        put32(&mut cmap, at + 4, code);
        put32(&mut cmap, at + 8, index as u32 + 1);
    }
    let mut hmtx = vec![0; 12];
    for (index, advance) in [500, 600, 1000].into_iter().enumerate() {
        put16(&mut hmtx, index * 4, advance);
    }
    let mut post = vec![0; 32];
    put32(&mut post, 0, 0x30000);
    let mut os2 = vec![0; 96];
    put16(&mut os2, 0, 3);
    let mut tables = vec![
        (*b"head", head),
        (*b"hhea", hhea),
        (*b"maxp", maxp),
        (*b"cmap", cmap),
        (*b"hmtx", hmtx),
        (*b"OS/2", os2),
        (*b"post", post),
        (*b"loca", vec![0; 8]),
        (*b"glyf", vec![]),
    ];
    tables.sort_by_key(|table| table.0);
    let mut bytes = vec![0; 12 + 16 * tables.len()];
    put32(&mut bytes, 0, 0x10000);
    put16(&mut bytes, 4, tables.len() as u16);
    for (index, (tag, payload)) in tables.iter().enumerate() {
        bytes.resize(bytes.len().next_multiple_of(4), 0);
        let offset = bytes.len() as u32;
        bytes[12 + index * 16..16 + index * 16].copy_from_slice(tag);
        put32(&mut bytes, 20 + index * 16, offset);
        put32(&mut bytes, 24 + index * 16, payload.len() as u32);
        bytes.extend(payload);
    }
    bytes.resize(bytes.len().next_multiple_of(4), 0);
    let outline = bytes.len() as u64;
    let glyf = entry(&bytes, b"glyf");
    put32(&mut bytes, glyf + 8, outline as u32);
    put32(&mut bytes, glyf + 12, 2 * 1024 * 1024);
    bytes.resize(outline as usize + 2 * 1024 * 1024, 0);
    Source {
        bytes,
        short: 2,
        requested: 0,
        outline,
    }
}

fn entry(bytes: &[u8], tag: &[u8; 4]) -> usize {
    let n = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    (0..n)
        .map(|i| 12 + i * 16)
        .find(|&i| &bytes[i..i + 4] == tag)
        .unwrap()
}

#[test]
fn ranged_metadata_maps_unicode_without_reading_outlines() {
    for chunk in [1, 3, 256] {
        let mut source = fixture();
        let limits = Limits {
            io_chunk_bytes: chunk,
            ..Limits::default()
        };
        let font = run(TrueTypeFont::read(&mut source, &limits, &NeverCancel)).unwrap();
        assert!(font.source_bytes() > 2 * 1024 * 1024);
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
