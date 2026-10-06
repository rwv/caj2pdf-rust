// SPDX-License-Identifier: MIT

//! Original synthetic containers for every accepted text framing.

use super::*;
use crate::hnc8::Hnc8Error;
use crate::test_support::NEVER;
use crate::{Error, Limits};
use flate2::{Compression, write::ZlibEncoder};
use std::io::Write;

/// An in-memory source that fails every read at or after `fail_at`.
struct Memory {
    bytes: Vec<u8>,
    fail_at: u64,
}

impl RangedSource for Memory {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> crate::Result<usize> {
        if offset >= self.fail_at {
            return Err(Error::InvalidInput {
                reason: "synthetic read failure",
            });
        }
        let start = offset as usize;
        let count = destination.len().min(self.bytes.len() - start);
        destination[..count].copy_from_slice(&self.bytes[start..start + count]);
        Ok(count)
    }
}

fn put_u32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn words(pairs: &[[u16; 2]]) -> Vec<u8> {
    pairs
        .iter()
        .flatten()
        .flat_map(|w| w.to_le_bytes())
        .collect()
}

/// A container whose pages are `(text, image payloads)`; every image is an
/// invented type-2 descriptor that is never decoded here.
fn container(variant: Variant, pages: &[(Vec<u8>, usize)]) -> Vec<u8> {
    let (count_at, index) = match variant {
        Variant::C8 => (0x08, 0x50),
        Variant::HnA => (0x90, 0x15c),
        Variant::HnB => (0x90, 0xd8),
    };
    let mut bytes = vec![0; index + pages.len() * 20];
    match variant {
        Variant::C8 => bytes[0] = 0xc8,
        Variant::HnA => bytes[..8].copy_from_slice(&[b'H', b'N', 0, 0, 0x90, 1, 0, 0]),
        Variant::HnB => {
            bytes[..8].copy_from_slice(&[b'H', b'N', 0, 0, 0xc8, 0, 0, 0]);
            put_u32(&mut bytes, 0x88, 0xc8);
        }
    }
    put_u32(&mut bytes, count_at, pages.len() as u32);
    put_u32(&mut bytes, count_at + 4, 2);
    for (number, (text, images)) in pages.iter().enumerate() {
        let row = index + number * 20;
        let start = bytes.len() as u32;
        put_u32(&mut bytes, row, start);
        put_u32(&mut bytes, row + 4, text.len() as u32);
        bytes[row + 8] = *images as u8;
        bytes.extend(text);
        for _ in 0..*images {
            let payload = bytes.len() as u32 + 12;
            bytes.extend(2_u32.to_le_bytes());
            bytes.extend(payload.to_le_bytes());
            bytes.extend(4_u32.to_le_bytes());
            bytes.extend(b"JPEG");
        }
    }
    bytes
}

fn zlib(plain: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(plain).unwrap();
    encoder.finish().unwrap()
}

/// The 24-byte tagged header with invented payload words, glyph records and
/// one image coordinate.
fn legacy_text(glyphs: usize) -> Vec<u8> {
    let mut plain = vec![0x19; 8 + glyphs * 16 + 4 + 28];
    for glyph in 0..glyphs {
        let at = 8 + glyph * 16;
        for (slot, tag) in [(0, 0x8070_u16), (4, 0x8071), (8, 0x8001)] {
            plain[at + slot..at + slot + 2].copy_from_slice(&tag.to_le_bytes());
        }
    }
    let mut text = b"\x03\x80\x07\x00\x03\x80\x09\x00COMPRESSTEXT".to_vec();
    text.extend((plain.len() as u32).to_le_bytes());
    text.extend(zlib(&plain));
    text
}

/// The 16-byte direct header with one image record and an end record.
fn direct_text() -> Vec<u8> {
    let mut plain = vec![0; 28];
    plain[..2].copy_from_slice(&0x800a_u16.to_le_bytes());
    plain[4..12].copy_from_slice(&words(&[[10, 20], [30, 40]]));
    plain.extend(words(&[[0x8004, 0]]));
    let mut text = b"COMPRESSTEXT".to_vec();
    text.extend((plain.len() as u32).to_le_bytes());
    text.extend(zlib(&plain));
    text
}

/// Uncompressed HN-A records for one image, optionally after the paired
/// `8003` page-size prefix.
fn raw_text(paired: bool) -> Vec<u8> {
    let mut text = if paired {
        words(&[[0x8003, 100], [0x8003, 200]])
    } else {
        Vec::new()
    };
    text.extend(words(&[[0x800a, 0], [0, 0], [80, 40], [0, 0]]));
    text.extend([0; 12]);
    text.extend(words(&[[0x8004, 0]]));
    text
}

fn native_text() -> Vec<u8> {
    words(&[[0x8001, 60], [0x8002, 0x1084], [30, 0xa0c1], [0x8004, 40]])
}

type Outcome = std::result::Result<TextStructure, Hnc8Error>;

/// Inspect every page with a fresh cursor, after reading its descriptors.
fn inspect(bytes: Vec<u8>) -> Vec<Outcome> {
    let mut source = Memory {
        bytes,
        fail_at: u64::MAX,
    };
    let limits = Limits::default();
    {
        let pages = Hnc8Reader::open(&mut source, &limits, &NEVER)
            .unwrap()
            .header()
            .page_count;
        let mut outcomes = Vec::new();
        for number in 1..=pages {
            let mut reader =
                Hnc8Reader::probe_at_page(&mut source, &limits, &NEVER, number).unwrap();
            reader.next_page().unwrap().unwrap();
            while reader.next_image().unwrap().is_some() {}
            outcomes.push(reader.inspect_text());
        }
        outcomes
    }
}

fn structure(framing: TextFraming, records: u32, decoded: Option<u32>) -> TextStructure {
    TextStructure {
        framing,
        records,
        decoded_length: decoded,
    }
}

#[test]
fn every_accepted_framing_is_named_with_counts_only() {
    let outcomes = inspect(container(
        Variant::C8,
        &[
            (legacy_text(2), 1),
            (direct_text(), 1),
            (native_text(), 0),
            (Vec::new(), 0),
        ],
    ));
    let outcomes: Vec<_> = outcomes.into_iter().map(Result::unwrap).collect();
    assert_eq!(
        outcomes,
        [
            structure(TextFraming::Legacy24, 2, Some(8 + 32 + 4 + 28)),
            structure(TextFraming::CompressText, 2, Some(32)),
            structure(TextFraming::Native, 4, None),
            structure(TextFraming::None, 0, None),
        ]
    );
    let outcomes = inspect(container(
        Variant::HnA,
        &[(raw_text(false), 1), (raw_text(true), 1)],
    ));
    let framings: Vec<_> = outcomes
        .into_iter()
        .map(|outcome| outcome.unwrap().framing)
        .collect();
    assert_eq!(framings, [TextFraming::Raw, TextFraming::RawPaired]);
    let outcomes = inspect(container(Variant::HnB, &[(native_text(), 0)]));
    assert_eq!(
        outcomes[0].as_ref().unwrap(),
        &structure(TextFraming::Native, 4, None)
    );
    let labels = [
        TextFraming::None,
        TextFraming::Raw,
        TextFraming::RawPaired,
        TextFraming::CompressText,
        TextFraming::Legacy24,
        TextFraming::Native,
    ]
    .map(TextFraming::as_str);
    assert_eq!(
        labels,
        [
            "none",
            "raw",
            "raw-paired",
            "compresstext",
            "legacy-24",
            "native"
        ]
    );
}

#[test]
fn rejected_text_reports_the_deciding_reader_error() {
    let mut corrupt = legacy_text(1);
    let last = corrupt.len() - 1;
    corrupt[last] ^= 0xff; // Adler-32 mismatch after an accepted header.
    let short_native = words(&[[0xffff, 0xffff]]);
    let outcomes = inspect(container(
        Variant::C8,
        &[(corrupt, 1), (short_native.clone(), 0)],
    ));
    let errors: Vec<_> = outcomes.into_iter().map(Result::unwrap_err).collect();
    // A compressed error past the header is not retried as native records.
    assert_eq!(errors[0].kind.field(), "text zlib frame");
    // A span without the compressed header is framed as native records.
    assert!(errors[1].to_string().contains("native"), "{}", errors[1]);
    // HN-A never falls back to native framing.
    let error = inspect(container(Variant::HnA, &[(short_native.repeat(8), 0)]))
        .pop()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.kind.field(), "page text prefix");
}

#[test]
fn inspect_text_requires_a_current_page() {
    let mut source = Memory {
        bytes: container(Variant::C8, &[(words(&[[0xffff, 0xffff]]), 0)]),
        fail_at: u64::MAX,
    };
    let limits = Limits::default();
    {
        let mut reader = Hnc8Reader::open(&mut source, &limits, &NEVER).unwrap();
        assert_eq!(reader.page_row_bytes(), 20);
        let error = reader.inspect_text().unwrap_err();
        assert!(matches!(error.kind, ErrorKind::NoCurrentPage));
        reader.next_page().unwrap();
        reader.inspect_text().unwrap_err();
    };
}

#[test]
fn compact_hn_b_rows_are_reported() {
    let mut bytes = container(Variant::HnB, &[(Vec::new(), 0)]);
    put_u32(&mut bytes, 0x88, 0);
    let mut source = Memory {
        bytes,
        fail_at: u64::MAX,
    };
    let limits = Limits::default();
    let rows = {
        Hnc8Reader::open(&mut source, &limits, &NEVER)
            .unwrap()
            .page_row_bytes()
    };
    assert_eq!(rows, 12);
}

fn tail(trailer: &[u8], fail_at: u64) -> crate::hnc8::Result<Option<ApplicationInfoTail>> {
    let mut bytes = container(Variant::C8, &[(Vec::new(), 0)]);
    bytes.extend(trailer);
    let mut source = Memory { bytes, fail_at };
    let limits = Limits::default();
    {
        Hnc8Reader::open(&mut source, &limits, &NEVER)
            .unwrap()
            .application_info_tail()
    }
}

#[test]
fn application_info_tail_reports_presence_and_extent_only() {
    let size = 0x50 + 20;
    let trailer = b"\x01\x02APPINFOSIGN 100";
    assert_eq!(
        tail(trailer, u64::MAX).unwrap(),
        Some(ApplicationInfoTail {
            offset: 100,
            length: Some((size + trailer.len() - 100) as u64),
        })
    );
    assert_eq!(
        tail(b"APPINFOSIGN 99999", u64::MAX).unwrap(),
        Some(ApplicationInfoTail {
            offset: 99999,
            length: None,
        })
    );
    for trailer in [
        &b""[..],
        b"APPINFOSIGN ",
        b"APPINFOSIGN 12 ",
        b"APPINFOSIGN x1",
        b"APPINFOSIGN 99999999999999999999",
    ] {
        assert_eq!(tail(trailer, u64::MAX).unwrap(), None, "{trailer:?}");
    }
    // Every header field precedes the page index at 0x50; the trailer does not.
    let error = tail(b"APPINFOSIGN 100", 0x50).unwrap_err();
    assert_eq!(error.kind.field(), "application-info trailer");
}
