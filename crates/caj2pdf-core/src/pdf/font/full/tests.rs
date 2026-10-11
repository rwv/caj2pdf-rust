// SPDX-License-Identifier: MIT

use super::*;
use crate::pdf::font::tests::{build, collection_font, drawing_font, table, tables};
use crate::test_support::NEVER;
use std::cell::Cell;
use std::rc::Rc;

struct Source {
    bytes: Vec<u8>,
    flip: Rc<Cell<Option<usize>>>,
    requested: usize,
}
impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize> {
        self.requested = self.requested.max(out.len());
        let at = offset as usize;
        let count = out.len().min(7).min(self.bytes.len().saturating_sub(at));
        out[..count].copy_from_slice(&self.bytes[at..at + count]);
        if let Some(flip) = self.flip.get()
            && at <= flip
            && flip < at + count
        {
            out[flip - at] ^= 1;
        }
        Ok(count)
    }
}
fn source(bytes: Vec<u8>) -> Source {
    Source {
        bytes,
        flip: Rc::new(Cell::new(None)),
        requested: 0,
    }
}
#[derive(Default)]
struct Bytes(Vec<u8>);
impl SubsetOutput for Bytes {
    fn put(&mut self, bytes: &[u8]) -> Result<()> {
        self.0.extend(bytes);
        Ok(())
    }
}

#[test]
fn standalone_program_is_exact_and_larger_than_its_allocation_bound() {
    let mut data = drawing_font();
    data.resize(65537, 0x5a); // Opaque trailing bytes must also be streamed exactly.
    let mut source = source(data.clone());
    let limits = Limits {
        io_chunk_bytes: 31,
        max_allocation_bytes: 2048,
        ..Limits::default()
    };
    let mut font = OpenTypeFont::read(&mut source, 0, &limits, &NEVER).unwrap();
    let plan = font.plan_full(u64::MAX, &limits, &NEVER).unwrap();
    assert_eq!(plan.length(), data.len() as u64);
    let mut output = Bytes::default();
    font.write_full(&plan, &mut output, &limits, &NEVER)
        .unwrap();
    assert_eq!(output.0, data);
    assert_eq!(font.subset_bytes_read(), 2 * data.len() as u64);
    drop(font);
    assert!(source.requested <= 31);
}

#[test]
fn collection_preserves_selected_face_tables_glyph_ids_and_checksum() {
    for face in 0..2 {
        for chunk in [1, 3, 127] {
            let mut source = source(collection_font());
            let limits = Limits {
                io_chunk_bytes: chunk,
                ..Limits::default()
            };
            let mut font = OpenTypeFont::read(&mut source, face, &limits, &NEVER).unwrap();
            let character = if face == 0 { 'A' } else { ' ' };
            let glyph = font.glyph(character).unwrap();
            let plan = font.plan_full(u64::MAX, &limits, &NEVER).unwrap();
            let mut output = Bytes::default();
            font.write_full(&plan, &mut output, &limits, &NEVER)
                .unwrap();
            assert_eq!(output.0.len() as u64, plan.length());
            let mut checksum = Measure::default();
            checksum.put(&output.0).unwrap();
            assert_eq!(checksum.sum, 0xb1b0_afba);
            let mut emitted = tables(&output.0);
            for entry in &font.directory[..font.table_count] {
                let tag: [u8; 4] = entry[..4].try_into().unwrap();
                let (at, len) = span(entry);
                let mut expected = font.source.bytes[at as usize..(at + len) as usize].to_vec();
                let actual = table(&mut emitted, &tag);
                if tag == *b"head" {
                    expected[8..12].fill(0);
                    actual[8..12].fill(0);
                }
                assert_eq!(actual, &expected);
            }
            let mut standalone = &output.0[..];
            let copied = OpenTypeFont::read(&mut standalone, 0, &limits, &NEVER).unwrap();
            assert_eq!(copied.glyph(character).unwrap(), glyph);
            drop(font);
            assert!(source.requested <= chunk);
        }
    }
}

#[test]
fn collection_keeps_unknown_tables_and_omits_stale_signature() {
    use crate::pdf::font::tests::collection;
    let mut entries = tables(&drawing_font());
    entries.push((*b"ZZZZ", vec![1, 2, 3, 4, 5]));
    entries.push((*b"DSIG", vec![0; 8]));
    let original = build(entries);
    let mut source = source(collection(&[original], &[0]));
    let limits = Limits::default();
    let mut font = OpenTypeFont::read(&mut source, 0, &limits, &NEVER).unwrap();
    let plan = font.plan_full(u64::MAX, &limits, &NEVER).unwrap();
    let mut output = Bytes::default();
    font.write_full(&plan, &mut output, &limits, &NEVER)
        .unwrap();
    let mut emitted = tables(&output.0);
    assert_eq!(table(&mut emitted, b"ZZZZ"), &[1, 2, 3, 4, 5]);
    assert!(!emitted.iter().any(|(tag, _)| tag == b"DSIG"));
}

#[test]
fn source_changes_are_detected_in_metadata_and_unused_payload() {
    for bytes in [drawing_font(), collection_font()] {
        for metadata in [true, false] {
            let mut source = source(bytes.clone());
            let flip = source.flip.clone();
            let limits = Limits {
                io_chunk_bytes: 3,
                ..Limits::default()
            };
            let mut font = OpenTypeFont::read(&mut source, 0, &limits, &NEVER).unwrap();
            let entry = font.directory[..font.table_count]
                .iter()
                .find(|e| e[..4] == if metadata { *b"cmap" } else { *b"glyf" })
                .unwrap();
            let at = span(entry).0 as usize;
            if metadata {
                flip.set(Some(at));
                assert_eq!(
                    font.plan_full(u64::MAX, &limits, &NEVER)
                        .err()
                        .unwrap()
                        .reason,
                    CHANGED
                );
            } else {
                let plan = font.plan_full(u64::MAX, &limits, &NEVER).unwrap();
                flip.set(Some(at));
                assert_eq!(
                    font.write_full(&plan, &mut Bytes::default(), &limits, &NEVER)
                        .unwrap_err()
                        .reason,
                    CHANGED
                );
            }
        }
    }
}

#[test]
fn planned_program_length_is_bounded_before_payload_reads() {
    for bytes in [drawing_font(), collection_font()] {
        let mut source = source(bytes);
        let limits = Limits::default();
        let mut font = OpenTypeFont::read(&mut source, 0, &limits, &NEVER).unwrap();
        assert!(font.plan_full(1, &limits, &NEVER).is_err());
        assert_eq!(font.subset_bytes_read(), 0);
    }
}

#[test]
fn complete_program_planning_and_writing_honor_cancellation() {
    use crate::{ErrorKind, test_support::CancelAfter};
    for bytes in [drawing_font(), collection_font()] {
        let mut source = source(bytes);
        let limits = Limits {
            io_chunk_bytes: 31,
            ..Limits::default()
        };
        let mut font = OpenTypeFont::read(&mut source, 0, &limits, &NEVER).unwrap();
        // Interrupt within a table/program, not just before the first read.
        let cancelled = CancelAfter::new(3);
        assert!(matches!(
            font.plan_full(u64::MAX, &limits, &cancelled)
                .err()
                .unwrap()
                .kind,
            ErrorKind::Cancelled
        ));
        let plan = font.plan_full(u64::MAX, &limits, &NEVER).unwrap();
        let cancelled = CancelAfter::new(3);
        assert!(matches!(
            font.write_full(&plan, &mut Bytes::default(), &limits, &cancelled)
                .unwrap_err()
                .kind,
            ErrorKind::Cancelled
        ));
    }
}
