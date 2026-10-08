// SPDX-License-Identifier: MIT

//! Original controls for proved interruptions; no external document bytes.

use super::*;
use crate::pdf::input::fragment_scan::{FragmentScan, scan_fragment_with_candidates};
use crate::test_support::{CancelAfter, NEVER};

struct Source {
    bytes: Vec<u8>,
    maximum: usize,
    largest: usize,
}
impl Source {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            maximum: 7,
            largest: 0,
        }
    }
}
impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> Result<usize> {
        self.largest = self.largest.max(out.len());
        let at = offset as usize;
        let n = out
            .len()
            .min(self.maximum)
            .min(self.bytes.len().saturating_sub(at));
        out[..n].copy_from_slice(&self.bytes[at..at + n]);
        Ok(n)
    }
}

fn scan(
    bytes: Vec<u8>,
    candidates: &mut [FragmentCandidate],
    limits: &Limits,
    cancel: &impl Cancellation,
) -> Result<FragmentScan> {
    let size = bytes.len() as u64;
    scan_fragment_with_candidates(&mut Source::new(bytes), 0, size, limits, cancel, candidates)
}

struct Fixture {
    bytes: Vec<u8>,
    start: u64,
    next: u64,
    header: u64,
    original: FragmentObject,
}

fn fixture(earlier: bool, prefix: usize, gap: usize) -> Fixture {
    let header = b"7 0 obj<</Length 70000>>stream\n";
    let mut complete = header.to_vec();
    complete.extend_from_slice(&[b'Z'; 70000]);
    // Object-like bytes within the payload must never become a boundary.
    complete[100..128].copy_from_slice(b"99 0 obj null endobj\nstream\n");
    complete.extend_from_slice(b"\nendstream\nendobj");
    let mut bytes = Vec::new();
    let mut at = 0;
    if earlier {
        bytes.extend_from_slice(&complete);
        bytes.push(b'\n');
    }
    let start = bytes.len() as u64;
    bytes.extend_from_slice(&complete[..prefix]);
    bytes.extend_from_slice(b"\r\n");
    let next = bytes.len() as u64;
    bytes.extend_from_slice(b"8 0 obj<</Use 7 0 R>>endobj\n");
    if !earlier {
        bytes.extend_from_slice(format!("9 0 obj<</Length {gap}>>stream\n").as_bytes());
        bytes.extend(std::iter::repeat_n(b'P', gap));
        bytes.extend_from_slice(b"\nendstream\nendobj\n");
        at = bytes.len() as u64;
        bytes.extend_from_slice(&complete);
    }
    Fixture {
        bytes,
        start,
        next,
        header: header.len() as u64,
        original: FragmentObject {
            reference: PdfRef {
                number: 7,
                generation: 0,
            },
            range: PdfRange {
                offset: at,
                length: complete.len() as u64,
            },
        },
    }
}

fn candidate(f: &Fixture) -> FragmentCandidate {
    FragmentCandidate {
        object: f.original,
        used: false,
    }
}

fn probe(
    f: &Fixture,
    source: &mut impl RangedSource,
    limits: &Limits,
    cancel: &impl Cancellation,
) -> Result<Option<u64>> {
    let size = source.size();
    let mut reader = Reader::new(
        source,
        PdfRange {
            offset: 0,
            length: size,
        },
        limits,
        cancel,
    )?;
    copy_prefix(&mut reader, f.start, f.original, f.header)
        .map(|found| found.map(|(resume, _)| resume))
}

#[test]
fn earlier_and_distant_later_streams_keep_the_complete_copy() {
    for earlier in [true, false] {
        for prefix in [227, 487, 977] {
            let f = fixture(earlier, prefix, 75000);
            for chunk in [1, 256, 4096] {
                let limits = Limits {
                    io_chunk_bytes: chunk,
                    ..Limits::default()
                };
                let mut candidates = if earlier { vec![] } else { vec![candidate(&f)] };
                let result = scan(f.bytes.clone(), &mut candidates, &limits, &NEVER).unwrap();
                assert!(result.patches.is_empty() && result.damaged.is_empty());
                assert_eq!(result.objects.len(), if earlier { 2 } else { 3 });
                assert_eq!(
                    result
                        .objects
                        .iter()
                        .find(|o| o.object.reference.number == 7)
                        .unwrap()
                        .object,
                    f.original
                );
                assert!(candidates.iter().all(|c| c.used));
            }
        }
    }
}

#[test]
fn stream_copy_requires_payload_and_a_bounded_proper_prefix() {
    for (prefix, accepted) in [
        (31, false),
        (32, true),
        (65536, true),
        (65537, false),
        (70029, false),
    ] {
        let f = fixture(true, prefix, 0);
        let mut source = Source::new(f.bytes.clone());
        let result = probe(&f, &mut source, &Limits::default(), &NEVER).unwrap();
        assert_eq!(
            result.is_some(),
            accepted,
            "prefix {prefix}, header {}",
            f.header
        );
        if accepted {
            assert_eq!(result, Some(f.next));
        }
        assert!(source.largest <= 8192);
    }
    for padding in [64, 65] {
        let mut f = fixture(true, 500, 0);
        let boundary = f.start as usize + 500;
        f.bytes.splice(
            boundary..f.next as usize,
            std::iter::repeat_n(b' ', padding),
        );
        f.next = boundary as u64 + padding as u64;
        let result = probe(
            &f,
            &mut Source::new(f.bytes.clone()),
            &Limits::default(),
            &NEVER,
        )
        .unwrap();
        assert_eq!(result.is_some(), padding == 64);
    }
    for offset in [0, 100, 499, 500, 502] {
        let mut f = fixture(true, 500, 0);
        f.bytes[f.start as usize + offset] = b'?';
        assert!(
            probe(
                &f,
                &mut Source::new(f.bytes.clone()),
                &Limits::default(),
                &NEVER
            )
            .unwrap()
            .is_none()
        );
    }
}

#[test]
fn later_copy_requires_unique_candidates_reached_by_the_scan() {
    let f = fixture(false, 500, 75000);
    assert!(
        scan(
            f.bytes.clone(),
            &mut [candidate(&f), candidate(&f)],
            &Limits::default(),
            &NEVER
        )
        .is_err()
    );
    let mut f = f;
    let at = f.original.range.offset as usize;
    let tail = f.bytes.split_off(at);
    f.bytes
        .extend_from_slice(format!("10 0 obj<</Length {}>>stream\n", tail.len()).as_bytes());
    f.original.range.offset = f.bytes.len() as u64;
    f.bytes.extend_from_slice(&tail);
    f.bytes.extend_from_slice(b"\nendstream\nendobj");
    let error = scan(
        f.bytes.clone(),
        &mut [candidate(&f)],
        &Limits::default(),
        &NEVER,
    )
    .err()
    .unwrap();
    assert_eq!(
        error.reason,
        "recovery candidate is not a complete fragment object"
    );
    // A second complete object with the same ID but different bytes is not a replay.
    let mut f = fixture(true, 500, 0);
    f.bytes.extend_from_slice(b"\n7 0 obj null endobj");
    assert!(scan(f.bytes, &mut [], &Limits::default(), &NEVER).is_err());
}

#[test]
fn copy_comparison_propagates_cancellation_limits_and_io() {
    let f = fixture(true, 500, 0);
    let checkpoints = CancelAfter::never();
    probe(
        &f,
        &mut Source::new(f.bytes.clone()),
        &Limits::default(),
        &checkpoints,
    )
    .unwrap();
    for allowed in 0..checkpoints.queries() {
        let e = probe(
            &f,
            &mut Source::new(f.bytes.clone()),
            &Limits::default(),
            &CancelAfter::new(allowed),
        )
        .unwrap_err();
        assert!(matches!(e.kind, ErrorKind::Cancelled), "{e:?}");
    }
    let limits = Limits {
        max_allocation_bytes: 511,
        ..Limits::default()
    };
    assert!(matches!(
        probe(&f, &mut Source::new(f.bytes.clone()), &limits, &NEVER)
            .unwrap_err()
            .kind,
        ErrorKind::LimitExceeded { .. }
    ));
    struct Failing;
    impl RangedSource for Failing {
        fn size(&self) -> u64 {
            200000
        }
        fn read_at(&mut self, _: u64, _: &mut [u8]) -> Result<usize> {
            Err(ErrorKind::Io(std::io::Error::other("injected copy read failure")).into())
        }
    }
    assert!(matches!(
        probe(&f, &mut Failing, &Limits::default(), &NEVER)
            .unwrap_err()
            .kind,
        ErrorKind::Io(_)
    ));
}

#[test]
fn source_changes_during_copy_boundary_validation_are_errors() {
    struct Changing {
        bytes: Vec<u8>,
        trigger: usize,
        change: usize,
        fired: bool,
    }
    impl RangedSource for Changing {
        fn size(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn read_at(&mut self, at: u64, out: &mut [u8]) -> Result<usize> {
            let at = at as usize;
            if at == self.trigger && !self.fired {
                self.bytes[self.change] ^= 1;
                self.fired = true;
            }
            let n = out.len().min(self.bytes.len().saturating_sub(at));
            out[..n].copy_from_slice(&self.bytes[at..at + n]);
            Ok(n)
        }
    }
    for earlier in [true, false] {
        let f = fixture(earlier, 500, 0);
        for change in [f.start + 200, f.original.range.offset + 200, f.start + 500] {
            let mut source = Changing {
                bytes: f.bytes.clone(),
                trigger: f.next as usize,
                change: change as usize,
                fired: false,
            };
            let result = probe(&f, &mut source, &Limits::default(), &NEVER);
            assert!(source.fired);
            assert!(result.is_err(), "{earlier} change at {change}: {result:?}");
        }
    }
}

#[test]
fn syntax_interruption_requires_the_same_boolean_number_or_terminal_header() {
    for (original, prefix) in [
        ("7 0 obj<</OP false>>endobj\n", "7 0 obj<</OP fa"),
        (
            "7 0 obj<</FontBBox[-558 -12 42 50]>>endobj\n",
            "7 0 obj<</FontBBox[-558 -",
        ),
    ] {
        let bytes = format!("{original}{prefix}\r\n8 0 obj null endobj");
        assert_eq!(
            scan(bytes.into_bytes(), &mut [], &Limits::default(), &NEVER)
                .unwrap()
                .objects
                .len(),
            2
        );
        let bytes = format!("{original}{prefix}?\r\n8 0 obj null endobj");
        assert!(scan(bytes.into_bytes(), &mut [], &Limits::default(), &NEVER).is_err());
    }
    let original = b"7 0 obj<</FontName/Test>>endobj\n";
    let bytes = [original.as_slice(), b"7 0\r\n"].concat();
    assert_eq!(
        scan(bytes.clone(), &mut [], &Limits::default(), &NEVER)
            .unwrap()
            .objects
            .len(),
        1
    );
    for prefix in [
        b"7 1\r\n".as_slice(),
        b"7 0 obj<</Font",
        b"8 0\r\n",
        b"7 0 x",
    ] {
        assert!(
            scan(
                [original.as_slice(), prefix].concat(),
                &mut [],
                &Limits::default(),
                &NEVER
            )
            .is_err()
        );
    }
    let end = bytes.len() as u64;
    let mut source = Source::new([bytes, b"outside fragment".to_vec()].concat());
    assert!(
        scan_fragment_with_candidates(&mut source, 0, end, &Limits::default(), &NEVER, &mut [])
            .is_err()
    );
}

const DECLARATION: &[u8] = b"7 0 obj<</Length 123/Filter/FlateDecod\r\n";
const NEXT_STREAM: &[u8] = b"8 0 obj<</Length 3>>stream\nabc\nendstream\nendobj\n";

#[test]
fn unfinished_flate_declaration_requires_a_complete_unreferenced_graph() {
    let bytes = [DECLARATION, NEXT_STREAM].concat();
    assert_eq!(
        scan(bytes.clone(), &mut [], &Limits::default(), &NEVER)
            .unwrap()
            .objects
            .len(),
        1
    );
    for extra in [
        "9 0 obj<</Use [<< /Nested 7 0 R >>]>>endobj",
        "7 0 obj null endobj",
        "9 0 obj<</Length 0/Type/ObjStm>>stream\nendstream\nendobj",
        "9 0 obj<</Length 0/Type/XRef>>stream\nendstream\nendobj",
        "9 0 obj<</Length 0/Type 8 0 R>>stream\nendstream\nendobj",
        "9 0 obj<</Key 1/Key 2>>endobj",
        "9 0 obj<</Type/Page>>endobj",
        "9 0 obj<</Unfinished",
    ] {
        assert!(
            scan(
                [bytes.as_slice(), extra.as_bytes()].concat(),
                &mut [],
                &Limits::default(),
                &NEVER
            )
            .is_err(),
            "{extra}"
        );
    }
    // Text resembling a reference is not an indirect graph edge.
    assert!(
        scan(
            [bytes.as_slice(), b"9 0 obj<</Text(7 0 R)>>endobj"].concat(),
            &mut [],
            &Limits::default(),
            &NEVER
        )
        .is_ok()
    );
    for declaration in [
        "7 1 obj<</Length 123/Filter/FlateDecod\r\n",
        "7 0 obj<</Length 0/Filter/FlateDecod\r\n",
        "7 0 obj<</Length 123/Filter/FlateDecode\r\n",
        "7 0 obj<</Length 123/Filter/FlateDecod\n",
        "7 0 obj<</Length 123/Filter/FlateDecod X\r\n",
        "7 0 obj<</Length 123/Private true/Filter/FlateDecod\r\n",
        "7 0 obj<</Length 123/Filter/FlateDecod>>stream\r\n",
    ] {
        assert!(
            scan(
                [declaration.as_bytes(), NEXT_STREAM].concat(),
                &mut [],
                &Limits::default(),
                &NEVER
            )
            .is_err(),
            "{declaration}"
        );
    }
}

#[test]
fn reverse_page_collection_proves_a_chain_then_requires_full_confirmation() {
    use crate::ConversionOptions;
    use crate::caj::convert_caj;
    fn stream(id: u32) -> Vec<u8> {
        let content = b"q Q\n".repeat(300);
        [
            format!("{id} 0 obj<</Length {}>>stream\n", content.len()).into_bytes(),
            content,
            b"\nendstream\nendobj\n".to_vec(),
        ]
        .concat()
    }
    fn page(id: u32, contents: u32) -> Vec<u8> {
        format!("{id} 0 obj<</Type/Page/Parent 99 0 R/MediaBox[0 0 32 32]/Resources<<>>/Contents {contents} 0 R>>endobj\n").into_bytes()
    }
    let a = stream(7);
    let b = stream(8);
    let rows = [
        [page(1, 7), a[..350].to_vec(), b"\r\n".to_vec()].concat(),
        [page(2, 8), b[..400].to_vec(), b"\r\n".to_vec(), a.clone()].concat(),
        // The first full copy needed by row 2 is over 64 KiB beyond its end.
        [
            page(3, 8),
            b"9 0 obj<</Length 75000>>stream\n".to_vec(),
            vec![b'Z'; 75000],
            b"\nendstream\nendobj\n".to_vec(),
            b.clone(),
        ]
        .concat(),
    ];
    let mut bytes = vec![0; 0x424];
    bytes[..4].copy_from_slice(b"CAJ\0");
    bytes[16..20].copy_from_slice(&3_u32.to_le_bytes());
    bytes[20..24].copy_from_slice(&0x400_u32.to_le_bytes());
    for (i, row) in rows.iter().enumerate() {
        let at = 0x400 + i * 12;
        let offset = bytes.len() as u32;
        bytes[at..at + 4].copy_from_slice(&offset.to_le_bytes());
        bytes[at + 4..at + 8].copy_from_slice(&(row.len() as u32).to_le_bytes());
        bytes[at + 8..at + 12].copy_from_slice(&(i as u32 + 1).to_le_bytes());
        bytes.extend_from_slice(row);
    }
    for chunk in [1, 4096] {
        let mut output = Vec::new();
        let limits = Limits {
            io_chunk_bytes: chunk,
            ..Limits::default()
        };
        let report = convert_caj(
            &mut Source::new(bytes.clone()),
            &mut output,
            &ConversionOptions::default(),
            &limits,
            &NEVER,
        )
        .unwrap();
        assert_eq!(report.pages_converted, 3);
        assert!(output.windows(a.len() - 1).any(|v| v == &a[..a.len() - 1]));
        assert!(output.windows(b.len() - 1).any(|v| v == &b[..b.len() - 1]));
        crate::pdf::copy_pdf(&mut output.as_slice(), &mut Vec::new(), &limits, &NEVER).unwrap();
    }
    // The table's third anchor can appear in an opaque payload. Local proofs
    // cannot substitute for reaching its objects in the complete source scan.
    let row2_start = 0x424 + rows[0].len();
    let row3_start = row2_start + rows[1].len();
    let header = format!("10 0 obj<</Length {}>>stream\n", rows[2].len());
    let mut hidden = bytes[..row3_start].to_vec();
    hidden.extend_from_slice(header.as_bytes());
    hidden.extend_from_slice(&rows[2]);
    hidden.extend_from_slice(b"\nendstream\nendobj\n");
    let table = 0x400 + 2 * 12;
    hidden[table..table + 4].copy_from_slice(&((row3_start + header.len()) as u32).to_le_bytes());
    hidden[0x410..0x414].copy_from_slice(&((rows[1].len() + header.len()) as u32).to_le_bytes());
    let mut output = Vec::new();
    assert!(
        convert_caj(
            &mut Source::new(hidden),
            &mut output,
            &ConversionOptions::default(),
            &Limits::default(),
            &NEVER
        )
        .is_err()
    );
    assert!(output.is_empty());
}

#[test]
fn indexed_pdf_does_not_admit_the_new_long_copy_gap() {
    let f = fixture(true, 500, 0);
    let objects=[
        (1,b"1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n".to_vec()),
        (2,b"2 0 obj<</Type/Pages/Count 1/Kids[3 0 R]>>endobj\n".to_vec()),
        (3,b"3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 32 32]/Resources<<>>/Contents 7 0 R>>endobj\n".to_vec()),
        (7,f.bytes[..f.start as usize].to_vec()),
    ];
    let mut pdf = b"%PDF-1.7\n".to_vec();
    let mut offsets = [0; 9];
    for (id, bytes) in objects {
        offsets[id] = pdf.len();
        pdf.extend_from_slice(&bytes);
    }
    pdf.extend_from_slice(&f.bytes[f.start as usize..f.next as usize]);
    offsets[8] = pdf.len();
    pdf.extend_from_slice(b"8 0 obj null endobj\n");
    let xref = pdf.len();
    pdf.extend_from_slice(b"xref\n0 9\n0000000000 65535 f \n");
    for at in &offsets[1..] {
        pdf.extend_from_slice(
            if *at == 0 {
                "0000000000 00000 f \n".into()
            } else {
                format!("{at:010} 00000 n \n")
            }
            .as_bytes(),
        );
    }
    pdf.extend_from_slice(
        format!("trailer\n<</Size 9/Root 1 0 R>>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    assert!(
        crate::pdf::copy_pdf(
            &mut pdf.as_slice(),
            &mut Vec::new(),
            &Limits::default(),
            &NEVER
        )
        .is_err()
    );
}

#[test]
fn same_row_replays_have_a_finite_distance_and_bounded_reads() {
    for distance in [600_000, MAX_REPLAY_DISTANCE, MAX_REPLAY_DISTANCE + 1] {
        let mut gap = distance as usize;
        let f = loop {
            let f = fixture(false, 500, gap);
            let actual = f.original.range.offset - f.start;
            if actual == distance {
                break f;
            }
            gap = (gap as i64 + distance as i64 - actual as i64) as usize;
        };
        let mut source = Source::new(f.bytes.clone());
        source.maximum = 4096;
        let size = source.size();
        let limits = Limits {
            io_chunk_bytes: 8192,
            max_allocation_bytes: 128 * 1024,
            ..Limits::default()
        };
        let result = scan_fragment_with_candidates(&mut source, 0, size, &limits, &NEVER, &mut []);
        if distance <= MAX_REPLAY_DISTANCE {
            let result = result.unwrap();
            assert_eq!(result.objects.len(), 3);
            assert!(result.objects.iter().any(|s| s.object == f.original));
            assert!(result.patches.is_empty() && result.damaged.is_empty());
        } else {
            assert!(result.is_err());
        }
        assert!(source.largest <= 8192, "{} byte read", source.largest);
    }
}

#[test]
fn a_marker_derived_copy_must_be_reached_even_with_another_matching_prefix() {
    let mut f = fixture(false, 500, 75000);
    let at = f.original.range.offset as usize;
    let complete = f.bytes.split_off(at);
    // The first apparent same-ID frame is inside an opaque stream. A real
    // later same-ID frame does not validate that marker-derived candidate.
    f.bytes
        .extend_from_slice(format!("10 0 obj<</Length {}>>stream\n", complete.len()).as_bytes());
    f.bytes.extend_from_slice(&complete);
    f.bytes.extend_from_slice(b"\nendstream\nendobj\n");
    f.bytes.extend_from_slice(&complete);
    let error = scan(f.bytes, &mut [], &Limits::default(), &NEVER)
        .err()
        .unwrap();
    assert_eq!(
        error.reason,
        "recovery candidate is not a complete fragment object"
    );
}

#[test]
fn distant_marker_recovery_keeps_strict_prefix_header_and_tail_proofs() {
    let f = fixture(false, 500, 75000);
    for offset in [
        100,
        499,
        500,
        f.original.range.offset as usize,
        f.bytes.len() - 3,
    ] {
        let mut bytes = f.bytes.clone();
        bytes[offset] = b'?';
        assert!(
            scan(bytes, &mut [], &Limits::default(), &NEVER).is_err(),
            "mutation {offset}"
        );
    }
    for prefix in [31, 65537] {
        let f = fixture(false, prefix, 75000);
        assert!(scan(f.bytes, &mut [], &Limits::default(), &NEVER).is_err());
    }
}
