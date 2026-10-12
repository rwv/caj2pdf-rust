// SPDX-License-Identifier: MIT

use super::*;
use crate::pdf::font::tests::{band_font, table, tables};
use crate::{ErrorKind, NeverCancel};

struct Source {
    bytes: Vec<u8>,
    fail: bool,
    largest: usize,
}
impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize> {
        if self.fail {
            return Err(Error::invalid("outline source failed"));
        }
        self.largest = self.largest.max(out.len());
        let at = offset as usize;
        let length = out.len().min(2).min(self.bytes.len().saturating_sub(at));
        out[..length].copy_from_slice(&self.bytes[at..at + length]);
        Ok(length)
    }
}

#[test]
fn horizontal_bands_use_bounded_short_reads_for_both_location_formats() {
    for long in [false, true] {
        for inset in [false, true] {
            let mut source = Source {
                bytes: band_font(inset, long),
                fail: false,
                largest: 0,
            };
            let limits = Limits {
                io_chunk_bytes: 7,
                ..Limits::default()
            };
            let mut font = OpenTypeFont::read(&mut source, 0, &limits, &NeverCancel).unwrap();
            let before = font.subset_bytes_read();
            assert_eq!(
                font.horizontal_bands('A', &limits, &NeverCancel).unwrap(),
                !inset
            );
            assert!(font.subset_bytes_read() - before <= 520);
            assert!(!font.horizontal_bands('中', &limits, &NeverCancel).unwrap());
            drop(font);
            assert!(source.largest <= 7);
        }
    }
}

#[test]
fn horizontal_bands_propagate_source_failure_and_cancellation() {
    struct Cancel;
    impl Cancellation for Cancel {
        fn is_cancelled(&self) -> bool {
            true
        }
    }
    let limits = Limits::default();
    let mut source = Source {
        bytes: band_font(false, false),
        fail: false,
        largest: 0,
    };
    let mut font = OpenTypeFont::read(&mut source, 0, &limits, &NeverCancel).unwrap();
    assert!(matches!(
        font.horizontal_bands('A', &limits, &Cancel)
            .unwrap_err()
            .kind,
        ErrorKind::Cancelled
    ));
    font.source.fail = true;
    assert!(
        font.horizontal_bands('A', &limits, &NeverCancel)
            .unwrap_err()
            .reason
            .contains("outline source failed")
    );
}

#[test]
fn horizontal_bands_refuse_unproven_geometry_and_truncated_data() {
    let mut font = tables(&band_font(false, false));
    let glyph = table(&mut font, b"glyf").clone();
    assert_eq!(rectangles(&glyph, 1000, -10), Some(true));
    for length in 0..glyph.len() {
        assert_ne!(rectangles(&glyph[..length], 1000, -10), Some(true));
    }
    for (at, value) in [
        (0, 255),
        (1, 9),
        (3, 0),
        (11, 2),
        (15, 1),
        (16, 0),
        (16, 129),
    ] {
        let mut changed = glyph.clone();
        changed[at] = value;
        assert_ne!(rectangles(&changed, 1000, -10), Some(true), "byte {at}");
    }
    assert_eq!(rectangles(&glyph, 1024, -10), Some(false));
    assert_eq!(rectangles(&glyph, 1000, 0), Some(false));
    // Move the upper band downward so its extent overlaps the lower one.
    let mut overlap = glyph.clone();
    overlap[48..50].copy_from_slice(&(-40i16).to_be_bytes());
    overlap[8..10].copy_from_slice(&620i16.to_be_bytes());
    assert_eq!(rectangles(&overlap, 1000, -10), Some(false));
    // Compressed/repeated flags with short signed coordinates: one 100-unit box.
    let mut compressed = vec![];
    for value in [1i16, 0, 0, 100, 100, 3, 0] {
        compressed.extend(value.to_be_bytes());
    }
    compressed.extend([0x31, 0x33, 0x35, 0x23, 100, 100, 100]);
    assert_eq!(rectangles(&compressed, 100, 0), Some(true));
    let mut repeated = glyph[..16].to_vec();
    repeated.extend([9, 7]);
    repeated.extend(&glyph[24..]);
    assert_eq!(rectangles(&repeated, 1000, -10), Some(true));
    repeated[17] = 8;
    assert_eq!(rectangles(&repeated, 1000, -10), Some(false));
}
