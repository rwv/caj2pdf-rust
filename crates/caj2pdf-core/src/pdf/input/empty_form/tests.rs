// SPDX-License-Identifier: MIT

//! Original synthetic Form dictionaries and CAJ pages, without corpus bytes.

use super::*;
use crate::caj::convert_caj;
use crate::pdf::input::fragment_scan::scan_fragment_with_candidates;
use crate::pdf::input::inspect_head;
use crate::test_support::{CancelAfter, NEVER};
use crate::{ConversionOptions, Error, ErrorKind, Limits};

const OUTER: &[u8] = b"8 0 obj\r<< /Type /XObject /Subtype /Form /Length 0 /BBox [0 0 3 4] /Matrix [1 0 0 1 0 0] >>\r\nstream\r\n";
const INNER: &[u8] =
    b"8 0 obj<</BBox[0 0 3 4]/Matrix[1 0 0 1 0 0]/Length 0/Subtype/Form/Type/XObject>>stream\n";
const TAIL: &[u8] = b"\r\nendstream\rendobj\r";

fn wrapped() -> Vec<u8> {
    [OUTER, INNER, TAIL, TAIL].concat()
}

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
    fn read_at(&mut self, at: u64, out: &mut [u8]) -> Result<usize> {
        self.largest = self.largest.max(out.len());
        let at = at as usize;
        let n = out
            .len()
            .min(self.maximum)
            .min(self.bytes.len().saturating_sub(at));
        out[..n].copy_from_slice(&self.bytes[at..at + n]);
        Ok(n)
    }
}

fn probe<S: RangedSource>(
    source: &mut S,
    limits: &Limits,
    cancel: &impl Cancellation,
) -> Result<Option<(FragmentObject, u64)>> {
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
    let head = reader.load_head(0, None)?;
    let ObjectTail::Stream { data_start } = head.tail else {
        panic!("stream fixture")
    };
    let dict = head.dictionary.as_ref().unwrap();
    let length = dict.value(b"Length").and_then(exact_unsigned);
    let stream = StreamFailure {
        reference: head.reference,
        data_start: data_start as u64,
        length,
        direct: length.map(|_| (0, b"0".to_vec())),
        inspection: inspect_head(&head, reader.range, 0, limits),
        simple_flate: false,
    };
    candidate(&mut reader, 0, &stream)
}

fn replace(bytes: &[u8], needle: &[u8], new: &[u8]) -> Vec<u8> {
    let at = bytes
        .windows(needle.len())
        .position(|v| v == needle)
        .unwrap();
    [&bytes[..at], new, &bytes[at + needle.len()..]].concat()
}

fn caj(form: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; 0x40c];
    bytes[..4].copy_from_slice(b"CAJ\0");
    bytes[16..20].copy_from_slice(&1_u32.to_le_bytes());
    bytes[20..24].copy_from_slice(&0x400_u32.to_le_bytes());
    bytes.extend_from_slice(b"7 0 obj<</Type/Page/Parent 99 0 R/MediaBox[0 0 32 32]/Resources<</XObject<</Fm8 8 0 R>>>>/Contents 10 0 R>>endobj\n");
    bytes.extend_from_slice(form);
    let content = b"q 1 0 0 1 2 2 cm /Fm8 Do Q";
    bytes.extend_from_slice(format!("10 0 obj<</Length {}>>stream\n", content.len()).as_bytes());
    bytes.extend_from_slice(content);
    bytes.extend_from_slice(b"\nendstream\nendobj\n");
    let length = bytes.len() - 0x40c;
    for (at, value) in [(0x400, 0x40c), (0x404, length as u32), (0x408, 7)] {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn converted(bytes: Vec<u8>, limits: &Limits, cancel: &impl Cancellation) -> Result<Vec<u8>> {
    let mut source = Source::new(bytes);
    let mut pdf = Vec::new();
    let report = convert_caj(
        &mut source,
        &mut pdf,
        &ConversionOptions::default(),
        limits,
        cancel,
    )?;
    assert_eq!(report.pages_converted, 1);
    assert!(source.largest <= limits.io_chunk_bytes);
    Ok(pdf)
}

#[test]
fn keeps_the_complete_inner_form_and_ordinary_graph_reconstruction() {
    for chunk in [1, 7, 256, 4096] {
        let limits = Limits {
            io_chunk_bytes: chunk,
            ..Limits::default()
        };
        let bytes = wrapped();
        let size = bytes.len() as u64;
        let mut source = Source::new(bytes.clone());
        let result =
            scan_fragment_with_candidates(&mut source, 0, size, &limits, &NEVER, &mut []).unwrap();
        assert_eq!(result.objects.len(), 1);
        assert!(result.patches.is_empty());
        let object = result.objects[0].object;
        assert_eq!(object.reference.number, 8);
        assert_eq!(object.range.offset, OUTER.len() as u64);
        assert_eq!(
            &bytes[object.range.offset as usize
                ..(object.range.offset + object.range.length) as usize],
            &[INNER, &TAIL[..TAIL.len() - 1]].concat()
        );
        assert_eq!(
            converted(caj(&bytes), &limits, &NEVER).unwrap(),
            converted(caj(&[INNER, TAIL].concat()), &limits, &NEVER).unwrap()
        );
        assert!(source.largest <= chunk);
    }
}

#[test]
fn nonempty_conflicting_and_unmeasured_dictionaries_do_not_qualify() {
    let changes: &[(&[u8], &[u8])] = &[
        (b"/Length 0", b"/Length 1"),
        (b"/Length 0", b"/Length 21 0 R"),
        (b"/Form", b"/Image"),
        (b"/XObject", b"/Something"),
        (b"[0 0 3 4]", b"[0 0 3.0 4]"),
        (b"[1 0 0 1 0 0]", b"[1 0 0 1 0 0.00000000000000000000001]"),
        (b"/BBox[0 0 3 4]", b"/BBox[0 0 3 4]/Resources<<>>"),
        (b"/BBox[0 0 3 4]", b"/BBox[0 0 3 4]/Filter/FlateDecode"),
        (b"/BBox[0 0 3 4]", b"/BBox[0 0 3 4]/BBox[0 0 3 4]"),
        (b"8 0 obj", b"9 0 obj"),
        (b"8 0 obj", b"8 1 obj"),
    ];
    for &(needle, new) in changes {
        let inner = replace(INNER, needle, new);
        let bytes = [OUTER, inner.as_slice(), TAIL, TAIL].concat();
        assert!(
            probe(&mut Source::new(bytes), &Limits::default(), &NEVER)
                .unwrap()
                .is_none(),
            "{new:?}"
        );
    }
    for head in [
        replace(OUTER, b"/Length 0", b"/Length 1"),
        replace(OUTER, b"8 0 obj", b"8 1 obj"),
    ] {
        assert!(
            probe(
                &mut Source::new([head.as_slice(), INNER, TAIL, TAIL].concat()),
                &Limits::default(),
                &NEVER
            )
            .unwrap()
            .is_none()
        );
    }
}

#[test]
fn exact_adjacency_two_tails_and_nonrecursive_framing_are_required() {
    for bytes in [
        [OUTER, b" ", INNER, TAIL, TAIL].concat(),
        [OUTER, INNER, b"q ", TAIL, TAIL].concat(),
        [OUTER, INNER, TAIL, b"junk", TAIL].concat(),
        [OUTER, INNER, TAIL].concat(),
        [OUTER, OUTER, INNER, TAIL, TAIL, TAIL].concat(),
        [OUTER, INNER, b"\nendstream\nendobjX\n", TAIL].concat(),
        [OUTER, INNER, TAIL, b"\nendstream\nendobjX\n"].concat(),
        [OUTER, INNER, &[b' '; 65], TAIL, TAIL].concat(),
        [OUTER, INNER, TAIL, &[b' '; 65], TAIL].concat(),
    ] {
        assert!(
            probe(&mut Source::new(bytes), &Limits::default(), &NEVER)
                .unwrap()
                .is_none()
        );
    }
}

#[test]
fn headers_are_bounded_and_numeric_tokens_cannot_merge() {
    let long = replace(INNER, b"obj", &[b"obj".as_slice(), &[b' '; 256]].concat());
    assert!(
        probe(
            &mut Source::new([OUTER, long.as_slice(), TAIL, TAIL].concat()),
            &Limits::default(),
            &NEVER
        )
        .unwrap()
        .is_none()
    );
    for (a, b) in [
        (b"[0 0 3 4]".as_slice(), b"[00 3 4]".as_slice()),
        (b"[0 0 3 4]", b"[0 0 null 4]"),
        (b"[0 0 3 4]", b"[0 0 3%comment\n4]"),
    ] {
        let inner = replace(INNER, a, b);
        assert!(
            probe(
                &mut Source::new([OUTER, inner.as_slice(), TAIL, TAIL].concat()),
                &Limits::default(),
                &NEVER
            )
            .unwrap()
            .is_none()
        );
    }
}

#[test]
fn cancellation_and_allocation_limits_are_preserved() {
    let bytes = caj(&wrapped());
    let limits = Limits {
        io_chunk_bytes: 256,
        ..Limits::default()
    };
    let signal = CancelAfter::never();
    converted(bytes.clone(), &limits, &signal).unwrap();
    for allowed in 0..signal.queries() {
        assert!(
            matches!(
                converted(bytes.clone(), &limits, &CancelAfter::new(allowed)),
                Err(Error {
                    kind: ErrorKind::Cancelled,
                    ..
                })
            ),
            "checkpoint {allowed}"
        );
    }
    let tight = Limits {
        max_allocation_bytes: 512,
        ..limits
    };
    assert!(matches!(
        converted(bytes, &tight, &NEVER),
        Err(Error {
            kind: ErrorKind::LimitExceeded { .. },
            ..
        })
    ));
}

#[test]
fn rechecks_both_headers_and_tails_before_selecting_the_inner_object() {
    struct Changing {
        bytes: Vec<u8>,
        change: usize,
        fail: bool,
        changed: bool,
    }
    impl RangedSource for Changing {
        fn size(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn read_at(&mut self, at: u64, out: &mut [u8]) -> Result<usize> {
            // The final exact outer-header recheck follows both parsed heads
            // and the two-tail probe; change a selected or omitted byte then.
            if at == 0 && out.len() == OUTER.len() && !self.changed {
                self.changed = true;
                if self.fail {
                    return Err(Error::truncated(17, 5, 0));
                }
                self.bytes[self.change] ^= 1;
            }
            self.bytes.as_slice().read_at(at, out)
        }
    }
    for change in [
        OUTER.len() - 4,
        OUTER.len() + 4,
        OUTER.len() + INNER.len() + 4,
        OUTER.len() + INNER.len() + TAIL.len() + 4,
    ] {
        let mut source = Changing {
            bytes: wrapped(),
            change,
            fail: false,
            changed: false,
        };
        let error =
            probe(&mut source, &Limits::default(), &NEVER).expect_err("changed source accepted");
        assert!(source.changed);
        assert_eq!(error.reason, "nested empty Form changed while reading");
        assert_eq!(error.offset, Some(0));
    }
    let mut source = Changing {
        bytes: wrapped(),
        change: 0,
        fail: true,
        changed: false,
    };
    let error = probe(&mut source, &Limits::default(), &NEVER)
        .err()
        .unwrap();
    assert!(matches!(error.kind, ErrorKind::Truncated { .. }));
    assert_eq!(error.offset, Some(17));
}

#[test]
fn a_conflicting_complete_copy_still_fails_before_emission() {
    let other = replace(INNER, b"[0 0 3 4]", b"[0 0 4 4]");
    let body = [wrapped().as_slice(), other.as_slice(), TAIL].concat();
    let mut source = Source::new(caj(&body));
    let mut output = Vec::new();
    assert!(
        convert_caj(
            &mut source,
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
fn indexed_pdf_stream_framing_is_unchanged() {
    fn pdf(form: &[u8]) -> Vec<u8> {
        let objects: [(usize,Vec<u8>);4]=[
            (1,b"1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n".to_vec()),
            (2,b"2 0 obj<</Type/Pages/Count 1/Kids[7 0 R]>>endobj\n".to_vec()),
            (7,b"7 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 32 32]/Resources<</XObject<</Fm8 8 0 R>>>>>>endobj\n".to_vec()),
            (8,form.to_vec()),
        ];
        let mut pdf = b"%PDF-1.7\n".to_vec();
        let mut offsets = [0; 9];
        for (id, bytes) in objects {
            offsets[id] = pdf.len();
            pdf.extend_from_slice(&bytes);
        }
        let xref = pdf.len();
        pdf.extend_from_slice(b"xref\n0 9\n0000000000 65535 f \n");
        for at in &offsets[1..] {
            pdf.extend_from_slice(
                if *at == 0 {
                    "0000000000 00000 f \n".to_owned()
                } else {
                    format!("{at:010} 00000 n \n")
                }
                .as_bytes(),
            );
        }
        pdf.extend_from_slice(
            format!("trailer\n<</Size 9/Root 1 0 R>>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        pdf
    }
    let limits = Limits::default();
    let clean = pdf(&[INNER, TAIL].concat());
    let malformed = pdf(&wrapped());
    crate::pdf::copy_pdf(&mut clean.as_slice(), &mut Vec::new(), &limits, &NEVER).unwrap();
    assert!(
        crate::pdf::copy_pdf(&mut malformed.as_slice(), &mut Vec::new(), &limits, &NEVER).is_err()
    );
}
