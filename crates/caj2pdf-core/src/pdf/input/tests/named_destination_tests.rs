// SPDX-License-Identifier: MIT

use super::*;
use crate::pdf::copy_pdf;
use named_destinations::name_bytes;
use std::io::{self, Write};

fn objects() -> Vec<(u32, &'static str)> {
    vec![
        (
            1,
            "<</Type/Catalog/Pages 2 0 R/Outlines 4 0 R/Names 6 0 R>>",
        ),
        (2, "<</Type/Pages/Kids[3 0 R 13 0 R]/Count 2>>"),
        (3, "<</Type/Page/Parent 2 0 R/MediaBox[0 0 20 20]>>"),
        (4, "<</First 5 0 R/Last 5 0 R>>"),
        (5, "<</Title (Example)/Parent 4 0 R/A<</S/GoTo/D(a)>>>>"),
        (6, "<</Dests 7 0 R>>"),
        (7, "<</Kids[12 0 R]>>"),
        (8, "<</Limits[(a)(a)]/Names[(a)9 0 R]>>"),
        (9, "[3 0 R/XYZ 0 20 null]"),
        (10, "<</Limits[(b)(b)]/Names[(b)11 0 R]>>"),
        (11, "[13 0 R/XYZ null -2.5 0]"),
        (12, "<</Limits[(a)(b)]/Kids[8 0 R 10 0 R]>>"),
        (13, "<</Type/Page/Parent 2 0 R/MediaBox[0 0 20 20]>>"),
    ]
}

fn fixture(changes: &[(u32, &str)]) -> Vec<u8> {
    let mut data = objects();
    for &(number, value) in changes {
        data.iter_mut().find(|(n, _)| *n == number).unwrap().1 = value;
    }
    build_pdf(&data, "")
}

fn reject(changes: &[(u32, &str)], expected: &str) {
    let bytes = fixture(changes);
    let mut output = Vec::new();
    let error = copy_pdf(
        &mut bytes.as_slice(),
        &mut output,
        &Limits::default(),
        &NEVER,
    )
    .unwrap_err();
    assert!(error.to_string().contains(expected), "{changes:?}: {error}");
    assert!(output.is_empty());
}

#[test]
fn named_destinations_preserve_root_leaf_internal_nodes_and_byte_keys() {
    for root in ["<</Kids[12 0 R]>>", "<</Names[(a)9 0 R <62>11 0 R]>>"] {
        for dest in [
            "/Dest (a)",
            "/A<</Type/Action/S/GoTo/D <61>>>",
            "/Dest (\\142)",
        ] {
            let item = format!("<</Title (Example)/Parent 4 0 R{dest}>>");
            let bytes = fixture(&[(7, root), (5, &item)]);
            for chunk in [1, 7, 256, 4096] {
                let limits = Limits {
                    io_chunk_bytes: chunk,
                    ..Limits::default()
                };
                let mut output = Vec::new();
                let report = copy_pdf(&mut bytes.as_slice(), &mut output, &limits, &NEVER).unwrap();
                assert_eq!(report.pages_converted, 2);
                assert_eq!(output, bytes);
                let index = open(bytes.clone()).unwrap();
                assert!(index.has_outlines());
                assert!(index.repair_objects().is_empty());
            }
        }
    }
    let bytes = compressed_fixture(&objects(), "", |_| {}, |_| {});
    let mut output = Vec::new();
    copy_pdf(
        &mut bytes.as_slice(),
        &mut output,
        &Limits::default(),
        &NEVER,
    )
    .unwrap();
    assert_eq!(output, bytes);
}

#[test]
fn named_destination_strings_follow_byte_string_escapes_without_unicode_folding() {
    for (raw, expected) in [
        (b"(a\\142)".as_slice(), b"ab".as_slice()),
        (b"<6 162>", b"ab"),
        (b"<6>", b"\x60"),
        (b"(a\\\r\nb\\\nc)", b"abc"),
        (b"(a\r\nb\rc\nd)", b"a\nb\nc\nd"),
        (b"(\\n\\r\\t\\b\\f\\(\\)\\\\\\z)", b"\n\r\t\x08\x0c()\\z"),
        (b"(\\1\\12\\1234\\777)", b"\x01\nS4\xff"),
        (b"(a(b)c)", b"a(b)c"),
        (b"()", b""),
        (b"%before\n<feff0061> %after", b"\xfe\xff\0a"),
    ] {
        assert_eq!(name_bytes(raw, 100).unwrap().unwrap(), expected);
    }
    for raw in [
        b"/a".as_slice(),
        b"(a",
        b"<XX>",
        b"<<>>",
        b"(a)(b)",
        b"(a\\)",
    ] {
        assert!(name_bytes(raw, 100).unwrap().is_none(), "{raw:?}");
    }
    assert!(matches!(
        name_bytes(b"(ab)", 3),
        Err(Error {
            kind: ErrorKind::LimitExceeded { .. },
            ..
        })
    ));
    assert_eq!(name_bytes(b"(ab)", 4).unwrap().unwrap(), b"ab");
}

#[test]
fn named_destination_tree_rejects_conflicting_bounds_keys_and_graphs() {
    for (number, value, message) in [
        (
            7,
            "<</Kids[12 0 R]/Names[]>>",
            "invalid destination name-tree",
        ),
        (
            7,
            "<</Kids[12 0 R]/Limits[(a)(b)]>>",
            "invalid destination name-tree",
        ),
        (7, "<</Kids[]>>", "invalid destination name-tree"),
        (7, "<</Kids[12 0 R 12 0 R]>>", "cycle or shared"),
        (7, "<</Kids[7 0 R]>>", "cycle or shared"),
        (
            7,
            "<</Names[(a)9 0 R (a)11 0 R]>>",
            "duplicated or out of order",
        ),
        (
            7,
            "<</Names[(b)11 0 R (a)9 0 R]>>",
            "duplicated or out of order",
        ),
        (7, "<</Names[(a)]>>", "invalid destination name-tree"),
        (7, "<</Names[]>>", "leaf is empty"),
        (7, "<</Names[/a 9 0 R]>>", "invalid destination name-tree"),
        (
            7,
            "<</Names[(a)9 0 R (ab)11 0 R (a)9 0 R]>>",
            "duplicated or out of order",
        ),
        (8, "<</Names[(a)9 0 R]>>", "invalid destination name-tree"),
        (
            8,
            "<</Limits[(b)(a)]/Names[(a)9 0 R]>>",
            "invalid destination name-tree",
        ),
        (
            8,
            "<</Limits[(a)]/Names[(a)9 0 R]>>",
            "invalid destination name-tree",
        ),
        (
            8,
            "<</Limits[(a)(a)(a)]/Names[(a)9 0 R]>>",
            "invalid destination name-tree",
        ),
        (8, "<</Limits[(a)(b)]/Names[(a)9 0 R]>>", "Limits disagree"),
        (
            12,
            "<</Limits[(a)(a)]/Kids[8 0 R 10 0 R]>>",
            "Limits disagree",
        ),
        (
            12,
            "<</Limits[(a)(b)]/Kids[10 0 R 8 0 R]>>",
            "duplicated or out of order",
        ),
        (
            12,
            "<</Limits[(a)(b)]/Kids[8 0 R 8 0 R]>>",
            "cycle or shared",
        ),
        (12, "123", "node is not a dictionary"),
        (
            6,
            "<</Length 0/Dests 7 0 R>>stream\n\nendstream",
            "must not be a stream",
        ),
        (
            7,
            "<</Length 0/Names[(a)9 0 R]>>stream\n\nendstream",
            "must not be a stream",
        ),
    ] {
        reject(&[(number, value)], message);
    }
}

#[test]
fn named_destinations_keep_missing_targets_views_and_actions_strict() {
    for destination in [
        "null",
        "(alias)",
        "9 0 R",
        "<</D[3 0 R/XYZ 0 20 null]>>",
        "[3 0 R/Fit]",
        "[3 0 R/XYZ 0 20]",
        "[3 0 R/XYZ 0 20 null 0]",
        "[3 0 R/XYZ (0) 20 null]",
        "[3 0 R/XYZ true 20 null]",
        "[3 0 R/XYZ 9 0 R 20 null]",
    ] {
        reject(&[(9, destination)], "indirect XYZ array");
    }
    reject(&[(9, "[2 0 R/XYZ 0 20 null]")], "does not target a page");
    reject(
        &[(7, "<</Names[(a)[3 0 R/XYZ 0 20 null]]>>")],
        "indirect XYZ array",
    );
    reject(&[(6, "<<>>")], "indirect Dests name tree");
    reject(
        &[(1, "<</Type/Catalog/Pages 2 0 R/Outlines 4 0 R>>")],
        "indirect Catalog Names",
    );
    for action in [
        "/Dest (missing)",
        "/A<</S/GoToR/D(a)>>",
        "/A<</S/GoTo/D(a)/Next<</S/GoTo/D(b)>>>>",
        "/A 9 0 R",
        "/Dest /a",
        "/Dest(a)/A<</S/GoTo/D(a)>>",
    ] {
        let item = format!("<</Title(Test)/Parent 4 0 R{action}>>");
        reject(
            &[(5, &item)],
            if action == "/Dest (missing)" {
                "name is absent"
            } else if action == "/A 9 0 R" {
                "action is not a dictionary object"
            } else if action.starts_with("/Dest(a)") {
                "both Dest and A"
            } else {
                "unsupported"
            },
        );
    }
}

#[test]
fn named_destination_depth_and_repeated_metadata_work_are_bounded() {
    for depth in [64, 65] {
        let mut data: Vec<_> = objects().iter().map(|(n, s)| (*n, s.to_string())).collect();
        data[6].1 = "<</Kids[14 0 R]>>".into();
        for i in 0..depth - 1 {
            let id = 14 + i;
            let next = if i == depth - 2 { 8 } else { id + 1 };
            data.push((id, format!("<</Limits[(a)(a)]/Kids[{next} 0 R]>>")));
        }
        let refs: Vec<_> = data.iter().map(|(n, s)| (*n, s.as_str())).collect();
        let result = open(build_pdf(&refs, ""));
        if depth == 64 {
            result.unwrap();
        } else {
            assert!(
                pdf_error(result)
                    .to_string()
                    .contains("destination tree depth")
            );
        }
    }
    let entries: String = (0..200).map(|i| format!("(k{i:03})9 0 R ")).collect();
    let root = format!("<</Names[{entries}]>>");
    let target = format!("[3 0 R/XYZ 0 20 null]{}", " ".repeat(2000));
    let bytes = fixture(&[
        (7, &root),
        (9, &target),
        (5, "<</Title(Test)/Parent 4 0 R/Dest(k000)>>"),
    ]);
    let error = pdf_error(open_with(
        bytes.clone(),
        &Limits {
            max_allocation_bytes: 131072,
            io_chunk_bytes: 256,
            ..Limits::default()
        },
    ));
    assert!(
        error.to_string().contains("destination metadata work"),
        "{error}"
    );
    open_with(
        bytes,
        &Limits {
            max_allocation_bytes: 1048576,
            ..Limits::default()
        },
    )
    .unwrap();
}

#[test]
fn named_destination_aggregate_name_and_index_allocations_are_bounded() {
    let key = "a".repeat(1000);
    let mut data: Vec<_> = objects().iter().map(|(n, s)| (*n, s.to_string())).collect();
    data[4].1 = format!("<</Title(Test)/Parent 4 0 R/Dest({key})>>");
    data[6].1 = "<</Kids[14 0 R]>>".into();
    data[7].1 = format!("<</Limits[({key})({key})]/Names[({key})9 0 R]>>");
    for id in 14..24 {
        let next = if id == 23 { 8 } else { id + 1 };
        data.push((id, format!("<</Limits[({key})({key})]/Kids[{next} 0 R]>>")));
    }
    let refs: Vec<_> = data.iter().map(|(n, s)| (*n, s.as_str())).collect();
    let bytes = build_pdf(&refs, "");
    let limited = Limits {
        io_chunk_bytes: 256,
        max_allocation_bytes: 131072,
        ..Limits::default()
    };
    let error = pdf_error(open_with(bytes.clone(), &limited));
    assert!(
        error.to_string().contains("destination name bytes"),
        "{error}"
    );
    open(bytes).unwrap();

    // Compressed metadata makes the cumulative work smaller than the separate
    // destination index budget. Multiple leaves stay within each syntax bound.
    let mut data: Vec<_> = objects().iter().map(|(n, s)| (*n, s.to_string())).collect();
    data[4].1 = "<</Title(Test)/Parent 4 0 R/Dest(k0000)>>".into();
    data[6].1 = "<</Kids[14 0 R 15 0 R 16 0 R 17 0 R 18 0 R]>>".into();
    for leaf in 0..5 {
        let first = leaf * 500;
        let last = first + 499;
        let entries: String = (first..=last).map(|i| format!("(k{i:04})9 0 R ")).collect();
        data.push((
            14 + leaf,
            format!("<</Limits[(k{first:04})(k{last:04})]/Names[{entries}]>>"),
        ));
    }
    let refs: Vec<_> = data.iter().map(|(n, s)| (*n, s.as_str())).collect();
    let bytes = separate_object_streams(&refs);
    let error = pdf_error(open_with(
        bytes.clone(),
        &Limits {
            max_allocation_bytes: 524288,
            ..Limits::default()
        },
    ));
    assert!(
        error.to_string().contains("named destination index"),
        "{error}"
    );
    open(bytes).unwrap();
}

// Separate original metadata streams keep each decoded stream below its own
// syntax limit, allowing the independent destination-index limit to be tested.
fn separate_object_streams(objects: &[(u32, &str)]) -> Vec<u8> {
    let count = objects.len() as u32;
    let xref_number = 2 * count + 1;
    let mut bytes = b"%PDF-1.5\n".to_vec();
    let mut offsets = Vec::new();
    for &(number, value) in objects {
        let header = format!("{number} 0 ");
        let encoded = zlib(format!("{header}{value}").as_bytes());
        offsets.push(bytes.len());
        bytes.extend_from_slice(
            format!(
                "{} 0 obj<</Type/ObjStm/N 1/First {}/Length {}/Filter/FlateDecode>>stream\n",
                count + number,
                header.len(),
                encoded.len()
            )
            .as_bytes(),
        );
        bytes.extend_from_slice(&encoded);
        bytes.extend_from_slice(b"\nendstream\nendobj\n");
    }
    let xref = bytes.len();
    let mut rows = Vec::new();
    for number in 0..=xref_number {
        let (kind, at, ordinal) = if number == 0 {
            (0, 0, u16::MAX)
        } else if number <= count {
            (2, count + number, 0)
        } else if number == xref_number {
            (1, xref as u32, 0)
        } else {
            (1, offsets[(number - count - 1) as usize] as u32, 0)
        };
        rows.push(kind);
        rows.extend_from_slice(&at.to_be_bytes());
        rows.extend_from_slice(&ordinal.to_be_bytes());
    }
    let encoded = zlib(&rows);
    bytes.extend_from_slice(format!("{xref_number} 0 obj<</Type/XRef/Size {}/Root 1 0 R/W[1 4 2]/Length {}/Filter/FlateDecode>>stream\n", xref_number + 1, encoded.len()).as_bytes());
    bytes.extend_from_slice(&encoded);
    bytes.extend_from_slice(format!("\nendstream\nendobj\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    bytes
}
struct ShortSource<'a>(&'a [u8]);

impl RangedSource for ShortSource<'_> {
    fn size(&self) -> u64 {
        self.0.len() as u64
    }
    fn read_at(&mut self, at: u64, destination: &mut [u8]) -> Result<usize> {
        let end = destination
            .len()
            .min(1)
            .min(self.0.len().saturating_sub(at as usize));
        destination[..end].copy_from_slice(&self.0[at as usize..at as usize + end]);
        Ok(end)
    }
}
struct ShortSink(Vec<u8>);
impl Write for ShortSink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = bytes.len().min(1);
        self.0.extend_from_slice(&bytes[..count]);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn named_destination_short_io_cancellation_and_changed_target() {
    let bytes = fixture(&[]);
    let mut output = ShortSink(Vec::new());
    copy_pdf(
        &mut ShortSource(&bytes),
        &mut output,
        &Limits::default(),
        &NEVER,
    )
    .unwrap();
    assert_eq!(output.0, bytes);
    let mut reached = false;
    for allowed in 0..10000 {
        match open_cancellable(
            bytes.clone(),
            &Limits::default(),
            &CancelAfter::new(allowed),
        ) {
            Ok(_) => {
                reached = true;
                break;
            }
            Err(error) => assert!(matches!(error.kind, ErrorKind::Cancelled)),
        }
    }
    assert!(reached);
    let mut changed = bytes.clone();
    replace_once(&mut changed, b"[3 0 R/XYZ", b"[2 0 R/XYZ");
    let mut source = MutatesAfterMarker {
        marker: find(&bytes, b"xref\n"),
        first: bytes,
        then: changed,
        switched: false,
    };
    let mut output = Vec::new();
    let error = copy_pdf(
        &mut source,
        &mut output,
        &Limits {
            io_chunk_bytes: 1,
            ..Limits::default()
        },
        &NEVER,
    )
    .unwrap_err();
    assert!(source.switched);
    assert!(
        error.to_string().contains("does not target a page"),
        "{error}"
    );
    assert!(output.is_empty());
}
