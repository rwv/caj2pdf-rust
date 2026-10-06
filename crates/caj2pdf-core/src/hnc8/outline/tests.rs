// SPDX-License-Identifier: MIT

//! Original synthetic HN-A outlines with one defect class per test. Each
//! checks the written hierarchy and the bounded defect record.

use super::*;
use crate::{Limits, NeverCancel, native::SeekableSource};
use std::io::Cursor;

const RECORDS: u64 = 0x15c;

/// A three-page HN-A header followed by `(title, page, level)` records.
fn source(records: &[(&[u8], &[u8], u32)]) -> Vec<u8> {
    let end = RECORDS as usize + records.len() * 308 + 3 * 20;
    let mut bytes = vec![0; end];
    bytes[..8].copy_from_slice(b"HN\0\0\x90\x01\0\0");
    bytes[144..148].copy_from_slice(&3_i32.to_le_bytes());
    bytes[0x158..0x15c].copy_from_slice(&(records.len() as i32).to_le_bytes());
    for (index, &(title, page, level)) in records.iter().enumerate() {
        let start = RECORDS as usize + index * 308;
        bytes[start..start + title.len()].copy_from_slice(title);
        bytes[start + 280..start + 280 + page.len()].copy_from_slice(page);
        bytes[start + 304..start + 308].copy_from_slice(&level.to_le_bytes());
    }
    bytes
}

#[derive(Default)]
struct Entries(Vec<(u32, String, u32)>);
impl BookmarkVisitor for Entries {
    fn visit(&mut self, bookmark: Bookmark) -> crate::Result<()> {
        self.0
            .push((bookmark.depth, bookmark.title, bookmark.page_index));
        Ok(())
    }
}

type Written = Vec<(u32, String, u32)>;

fn visit(
    bytes: Vec<u8>,
    depth: u32,
    map: impl FnMut(u32) -> Option<u32>,
) -> crate::Result<(Written, OutlineReport)> {
    (|| {
        let mut input = SeekableSource::new(Cursor::new(bytes)).unwrap();
        let limits = Limits::default();
        let mut reader = Hnc8Reader::open(&mut input, &limits, &NeverCancel)?;
        let mut entries = Entries::default();
        let report = reader.visit_bookmarks(depth, 3, map, &mut entries)?;
        Ok((entries.0, report))
    })()
}

fn identity(page: u32) -> Option<u32> {
    Some(page - 1)
}

fn tree(entries: &[(u32, &str, u32)]) -> Written {
    entries
        .iter()
        .map(|&(depth, title, page)| (depth, title.to_owned(), page))
        .collect()
}

fn record_at(ordinal: u64, field: u64) -> u64 {
    RECORDS + ordinal * 308 + field
}

fn skipped(offset: u64, reason: &'static str) -> OutlineDefect {
    OutlineDefect {
        offset,
        reason,
        repair: OutlineRepair::Skipped,
    }
}

fn clamped(offset: u64, reason: &'static str) -> OutlineDefect {
    OutlineDefect {
        offset,
        reason,
        repair: OutlineRepair::Clamped,
    }
}

#[test]
fn a_clean_outline_has_no_defects() {
    let bytes = source(&[(b"Root", b"1", 1), (b"Leaf", b"2", 2), (b"Next", b"3", 1)]);
    let (written, report) = visit(bytes, 64, identity).unwrap();
    assert_eq!(
        written,
        tree(&[(0, "Root", 0), (1, "Leaf", 1), (0, "Next", 2)])
    );
    assert_eq!((report.declared, report.written, report.defects), (3, 3, 0));
    assert!(report.recorded_defects().is_empty());
}

#[test]
fn invalid_titles_are_skipped_and_their_children_re_parented() {
    let unterminated = [b'x'; 256];
    let bytes = source(&[
        (b"Root", b"1", 1),
        (b"ok\x81", b"1", 2),
        (b"Orphan", b"2", 3),
        (b"Sibling", b"2", 2),
        (&unterminated, b"3", 1),
        (b"Second orphan", b"3", 2),
    ]);
    let (written, report) = visit(bytes, 64, identity).unwrap();
    assert_eq!(
        written,
        tree(&[
            (0, "Root", 0),
            (1, "Orphan", 1),
            (1, "Sibling", 1),
            (1, "Second orphan", 2),
        ])
    );
    assert_eq!((report.declared, report.written, report.defects), (6, 4, 2));
    assert_eq!(
        report.recorded_defects(),
        [
            skipped(record_at(1, 2), "title is not valid GB18030"),
            skipped(record_at(4, 0), "title has no NUL terminator"),
        ]
    );
}

#[test]
fn invalid_or_unmapped_pages_are_skipped_and_their_children_re_parented() {
    for (page, reason) in [
        (b"".as_slice(), "page is not a decimal number"),
        (b"-1", "page is not a decimal number"),
        (b"000000000001", "page has no NUL terminator"),
        (b"0", "destination is outside source pages"),
        (b"4", "destination is outside source pages"),
        (b"999", "destination is outside source pages"),
        (b"3", "destination page was not emitted"),
    ] {
        let bytes = source(&[
            (b"Root", b"1", 1),
            (b"Bad", page, 2),
            (b"Child", b"2", 3),
            (b"Grandchild", b"2", 4),
            (b"Sibling", b"1", 2),
        ]);
        // Page 3 is treated as omitted from the output.
        let (written, report) = visit(bytes, 64, |page| (page != 3).then(|| page - 1)).unwrap();
        assert_eq!(
            written,
            tree(&[
                (0, "Root", 0),
                (1, "Child", 1),
                (2, "Grandchild", 1),
                (1, "Sibling", 0),
            ]),
            "{reason}"
        );
        assert_eq!(
            report.recorded_defects(),
            [skipped(record_at(1, 280), reason)]
        );
    }
    // An index at or beyond the emitted page count is also unmapped.
    let (written, report) = visit(source(&[(b"Late", b"1", 1)]), 64, |_| Some(3)).unwrap();
    assert!(written.is_empty());
    assert_eq!(
        report.recorded_defects(),
        [skipped(
            record_at(0, 280),
            "destination page was not emitted"
        )]
    );
}

#[test]
fn zero_levels_are_skipped_without_reporting_their_children() {
    let bytes = source(&[
        (b"Root", b"1", 1),
        (b"Zero", b"1", 0),
        (b"Child", b"2", 3),
        (b"Grandchild", b"2", 4),
        (b"Next", b"3", 1),
    ]);
    let (written, report) = visit(bytes, 64, identity).unwrap();
    assert_eq!(
        written,
        tree(&[
            (0, "Root", 0),
            (1, "Child", 1),
            (2, "Grandchild", 1),
            (0, "Next", 2),
        ])
    );
    assert_eq!(
        report.recorded_defects(),
        [skipped(record_at(1, 304), "level is zero")]
    );
}

#[test]
fn level_skips_are_clamped_below_the_previous_written_entry() {
    let bytes = source(&[
        (b"Starts deep", b"1", 2),
        (b"Child", b"1", 3),
        (b"Gap", b"2", 5),
        (b"Below gap", b"2", 6),
        (b"Back", b"3", 2),
    ]);
    let (written, report) = visit(bytes, 64, identity).unwrap();
    assert_eq!(
        written,
        tree(&[
            (0, "Starts deep", 0),
            (1, "Child", 0),
            (2, "Gap", 1),
            (3, "Below gap", 1),
            (1, "Back", 2),
        ])
    );
    assert_eq!((report.declared, report.written, report.defects), (5, 5, 2));
    assert_eq!(
        report.recorded_defects(),
        [
            clamped(record_at(0, 304), "level skips a parent"),
            clamped(record_at(2, 304), "level skips a parent"),
        ]
    );
}

#[test]
fn levels_beyond_the_depth_limit_are_clamped() {
    let bytes = source(&[
        (b"Root", b"1", 1),
        (b"Child", b"1", 2),
        (b"Deep", b"2", 3),
        (b"Huge", b"2", u32::MAX),
        (b"Next", b"3", 1),
    ]);
    let (written, report) = visit(bytes, 2, identity).unwrap();
    assert_eq!(
        written,
        tree(&[
            (0, "Root", 0),
            (1, "Child", 0),
            (1, "Deep", 1),
            (1, "Huge", 1),
            (0, "Next", 2),
        ])
    );
    assert_eq!(
        report.recorded_defects(),
        [
            clamped(record_at(2, 304), "level exceeds the configured depth"),
            clamped(record_at(3, 304), "level exceeds the configured depth"),
        ]
    );
}

#[test]
fn a_skipped_entry_can_lower_the_source_parent_without_a_false_skip() {
    // The skipped root at level 1 makes the next level-3 entry look like a
    // source skip, but the written parent still allows it, so no defect.
    let bytes = source(&[
        (b"Root", b"1", 1),
        (b"Child", b"1", 2),
        (b"Grandchild", b"1", 3),
        (b"Skipped root", b"9", 1),
        (b"Cousin", b"2", 3),
    ]);
    let (written, report) = visit(bytes, 64, identity).unwrap();
    assert_eq!(
        written,
        tree(&[
            (0, "Root", 0),
            (1, "Child", 0),
            (2, "Grandchild", 0),
            (2, "Cousin", 1),
        ])
    );
    assert_eq!(
        report.recorded_defects(),
        [skipped(
            record_at(3, 280),
            "destination is outside source pages"
        )]
    );
}

#[test]
fn defect_locations_are_bounded_but_every_defect_is_counted() {
    let mut records = vec![(b"Kept".as_slice(), b"1".as_slice(), 1)];
    records.extend([(b"Bad".as_slice(), b"0".as_slice(), 1); 20]);
    let (written, report) = visit(source(&records), 64, identity).unwrap();
    assert_eq!(written, tree(&[(0, "Kept", 0)]));
    assert_eq!(
        (report.declared, report.written, report.defects),
        (21, 1, 20)
    );
    let recorded = report.recorded_defects();
    assert_eq!(recorded.len(), MAX_RECORDED_OUTLINE_DEFECTS);
    for (ordinal, defect) in (1..).zip(recorded) {
        assert_eq!(
            *defect,
            skipped(
                record_at(ordinal, 280),
                "destination is outside source pages"
            )
        );
    }
}

#[test]
fn defects_describe_their_repair_and_location() {
    assert_eq!(
        skipped(640, "level is zero").to_string(),
        "skipped HN-A bookmark at byte 640: level is zero"
    );
    assert_eq!(
        clamped(1260, "level skips a parent").to_string(),
        "re-parented HN-A bookmark at byte 1260: level skips a parent"
    );
}
