// SPDX-License-Identifier: MIT

//! Original synthetic indexed PDFs. No external document bytes.

use super::*;
use crate::pdf::{PdfOutlineAppender, copy_pdf};
use crate::test_support::{CancelAfter, NEVER};
use crate::{ErrorKind, Limits};
use std::io::{self, Write};

const OUTER: &str = "4 0 obj\n<</Type/XObject/Subtype/Form/Length 0/BBox[0 0 3.2500 4.500]/Matrix[2.00 0 0 3.0 0 0]>>stream\n";
const INNER: &str =
    "6 0 obj<</BBox[0 0 3.25 4.5]/Matrix[2 0 0 3 0 0]/Length 0/Subtype/Form/Type/XObject>>stream\n";
const TAIL: &str = "\nendstream\nendobj\n";

fn wrapped() -> String {
    format!("{OUTER}{INNER}{TAIL}{TAIL}")
}

struct Fixture {
    bytes: Vec<u8>,
    offsets: [usize; 7],
    xref: usize,
}

fn pdf(form: &str, live_before: bool) -> Fixture {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = [0; 7];
    let content = "q /Fm Do Q";
    let stream = format!(
        "5 0 obj<</Length {}>>stream\n{content}\nendstream\nendobj\n",
        content.len()
    );
    let objects = [
        (1, "1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n"),
        (2, "2 0 obj<</Type/Pages/Count 1/Kids[3 0 R]>>endobj\n"),
        (
            3,
            "3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 20 20]/Resources<</XObject<</Fm 4 0 R>>>>/Contents 5 0 R>>endobj\n",
        ),
        (4, form),
        (5, stream.as_str()),
        (6, "6 0 obj 12345 endobj\n"),
    ];
    let order = if live_before {
        [0, 1, 2, 5, 3, 4]
    } else {
        [0, 1, 2, 3, 4, 5]
    };
    for i in order {
        let (id, body) = objects[i];
        offsets[id] = bytes.len();
        bytes.extend_from_slice(body.as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 7\n0000000000 65535 f \n");
    for offset in &offsets[1..] {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<</Size 7/Root 1 0 R>>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    Fixture {
        bytes,
        offsets,
        xref,
    }
}

fn open(bytes: &[u8], limits: &Limits) -> Result<PdfIndex> {
    PdfIndex::open(
        &mut &*bytes,
        PdfRange {
            offset: 0,
            length: bytes.len() as u64,
        },
        limits,
        &NEVER,
    )
}

fn reject(form: &str) {
    let input = pdf(form, false).bytes;
    let mut output = Vec::new();
    assert!(
        copy_pdf(
            &mut input.as_slice(),
            &mut output,
            &Limits::default(),
            &NEVER
        )
        .is_err(),
        "{form:?}"
    );
    assert!(output.is_empty());
}

#[test]
fn appends_outer_identity_preserving_real_inner_id_and_original_prefix() {
    for live_before in [false, true] {
        for chunk in [1, 7, 256, 4096] {
            let fixture = pdf(&wrapped(), live_before);
            let limits = Limits {
                io_chunk_bytes: chunk,
                ..Limits::default()
            };
            let index = open(&fixture.bytes, &limits).unwrap();
            assert_eq!(index.repair_objects().len(), 1);
            assert_eq!(index.repair_objects()[0].reference.number, 4);
            assert!(
                index.repair_objects()[0]
                    .body
                    .ends_with(b"\nstream\n\nendstream")
            );
            assert_eq!(index.empty_form_checks.len(), 1);
            assert_eq!(
                index.empty_form_checks[0].original,
                wrapped().trim_end().as_bytes()
            );
            let mut output = Vec::new();
            let report =
                copy_pdf(&mut fixture.bytes.as_slice(), &mut output, &limits, &NEVER).unwrap();
            assert_eq!(report.pages_converted, 1);
            assert!(output.starts_with(&fixture.bytes));
            let reopened = open(&output, &limits).unwrap();
            assert!(reopened.repair_objects().is_empty());
            assert_eq!(reopened.pages(), index.pages());
            assert_eq!(reopened.trailer_size(), 7);
            for id in [1, 2, 3, 5, 6] {
                let reference = PdfRef {
                    number: id,
                    generation: 0,
                };
                assert_eq!(
                    reopened.object_location(reference).unwrap(),
                    index.object_location(reference).unwrap()
                );
            }
            assert!(
                reopened
                    .object_location(PdfRef {
                        number: 4,
                        generation: 0
                    })
                    .unwrap()
                    .offset
                    >= fixture.bytes.len() as u64
            );
        }
    }
}

#[test]
fn only_the_measured_dictionary_geometry_and_two_empty_tails_qualify() {
    for (needle, replacement) in [
        ("/Length 0", "/Length 1"),
        ("/Length 0", "/Length 6 0 R"),
        ("/Form", "/Image"),
        ("/XObject", "/Something"),
        ("/Length 0", "/Length 0/Resources<<>>"),
        ("/Length 0", "/Length 0/Filter/FlateDecode"),
        ("/Length 0", "/Length 0/Length 0"),
        ("6 0 obj", "4 0 obj"),
        ("6 0 obj", "6 1 obj"),
        ("[0 0 3.25 4.5]", "[0 0 3.250000000000000000001 4.5]"),
        ("[0 0 3.25 4.5]", "[0 0 03.25 4.5]"),
        ("[0 0 3.25 4.5]", "[0 0 -3.25 4.5]"),
        ("[0 0 3.25 4.5]", "[0 0 null 4.5]"),
        ("[0 0 3.25 4.5]", "[00 3.25 4.5]"),
        ("[0 0 3.25 4.5]", "[0 0 3.25%comment\n4.5]"),
        ("[2 0 0 3 0 0]", "[2 0 0 3 0 0.000000000000000001]"),
    ] {
        reject(&format!(
            "{OUTER}{}{TAIL}{TAIL}",
            INNER.replacen(needle, replacement, 1)
        ));
    }
    for outer in [
        OUTER.replace("/Length 0", "/Length 1"),
        OUTER.replace("4 0 obj", "4 1 obj"),
        OUTER.replace("/BBox", "/Unknown"),
    ] {
        reject(&format!("{outer}{INNER}{TAIL}{TAIL}"));
    }
    for form in [
        format!("{OUTER} {INNER}{TAIL}{TAIL}"),
        format!("{OUTER}{INNER}q{TAIL}{TAIL}"),
        format!("{OUTER}{INNER}{TAIL}junk{TAIL}"),
        format!("{OUTER}{INNER}{TAIL}"),
        format!("{OUTER}{INNER}{INNER}{TAIL}{TAIL}{TAIL}"),
        format!("{OUTER}{INNER}\nendstream\nendobjX\n{TAIL}"),
        format!("{OUTER}{INNER}{TAIL}\nendstream\nendobjX\n"),
        format!("{OUTER}{INNER}{}{TAIL}{TAIL}", " ".repeat(65)),
        format!("{OUTER}{INNER}{TAIL}{}{TAIL}", " ".repeat(65)),
    ] {
        reject(&form);
    }
}

#[test]
fn inner_identity_requires_a_separate_generation_zero_live_xref() {
    for entry in [
        "0000000000 00000 f \n".to_owned(),
        "0000000000 00001 n \n".to_owned(),
        "0000000000 00000 n \n".to_owned(),
    ] {
        let mut f = pdf(&wrapped(), false);
        let at = f.xref + b"xref\n0 7\n".len() + 6 * 20;
        let entry = if entry == "0000000000 00000 n \n" {
            format!("{:010} 00000 n \n", f.offsets[4] + OUTER.len())
        } else {
            entry
        };
        f.bytes[at..at + 20].copy_from_slice(entry.as_bytes());
        assert!(open(&f.bytes, &Limits::default()).is_err());
    }
}

#[test]
fn changed_header_or_tail_is_rejected_at_proof_capture_and_sequential_copy() {
    let f = pdf(&wrapped(), false);
    let limits = Limits {
        io_chunk_bytes: 4096,
        ..Limits::default()
    };
    let index = open(&f.bytes, &limits).unwrap();
    let proof = &index.empty_form_checks[0];
    struct Changing {
        bytes: Vec<u8>,
        start: u64,
        len: usize,
        change: usize,
        changed: bool,
    }
    impl RangedSource for Changing {
        fn size(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn read_at(&mut self, at: u64, out: &mut [u8]) -> Result<usize> {
            if at == self.start && out.len() == self.len {
                self.bytes[self.change] ^= 1;
                self.changed = true;
            }
            self.bytes.as_slice().read_at(at, out)
        }
    }
    for relative in 0..proof.original.len() {
        let mut source = Changing {
            bytes: f.bytes.clone(),
            start: proof.offset,
            len: proof.original.len(),
            change: proof.offset as usize + relative,
            changed: false,
        };
        let error = PdfIndex::open(&mut source, index.range(), &limits, &NEVER)
            .err()
            .unwrap();
        assert!(source.changed);
        assert_eq!(error.reason, "nested empty Form changed while reading");
        for chunk in [1, 7, 256, 4096] {
            let mut changed = f.bytes.clone();
            changed[proof.offset as usize + relative] ^= 1;
            let limits = Limits {
                io_chunk_bytes: chunk,
                ..limits
            };
            let error = PdfOutlineAppender::begin(
                &mut changed.as_slice(),
                &mut Vec::new(),
                &index,
                &limits,
                &NEVER,
            )
            .err()
            .unwrap();
            assert_eq!(error.reason, "PDF empty Form changed after inspection");
        }
    }
}

#[test]
fn short_io_errors_cancellation_and_allocation_limits_propagate() {
    struct ShortSource {
        bytes: Vec<u8>,
        maximum: usize,
        stop: Option<usize>,
        largest: usize,
    }
    impl RangedSource for ShortSource {
        fn size(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn read_at(&mut self, at: u64, out: &mut [u8]) -> Result<usize> {
            self.largest = self.largest.max(out.len());
            if self
                .stop
                .is_some_and(|stop| at as usize <= stop && at as usize + out.len() > stop)
            {
                return Ok(0);
            }
            let n = out.len().min(self.maximum);
            self.bytes.as_slice().read_at(at, &mut out[..n])
        }
    }
    struct ShortSink {
        bytes: Vec<u8>,
        stop: usize,
    }
    impl Write for ShortSink {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let n = bytes
                .len()
                .min(3)
                .min(self.stop.saturating_sub(self.bytes.len()));
            self.bytes.extend_from_slice(&bytes[..n]);
            Ok(n)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let input = pdf(&wrapped(), false).bytes;
    let limits = Limits {
        io_chunk_bytes: 256,
        ..Limits::default()
    };
    let signal = CancelAfter::never();
    let mut output = Vec::new();
    copy_pdf(&mut input.as_slice(), &mut output, &limits, &signal).unwrap();
    for allowed in 0..signal.queries() {
        let error = copy_pdf(
            &mut input.as_slice(),
            &mut Vec::new(),
            &limits,
            &CancelAfter::new(allowed),
        )
        .unwrap_err();
        assert!(matches!(error.kind, ErrorKind::Cancelled));
    }
    let mut source = ShortSource {
        bytes: input.clone(),
        maximum: 3,
        stop: None,
        largest: 0,
    };
    let mut sink = ShortSink {
        bytes: Vec::new(),
        stop: usize::MAX,
    };
    copy_pdf(&mut source, &mut sink, &limits, &NEVER).unwrap();
    assert_eq!(sink.bytes, output);
    assert!(source.largest <= limits.io_chunk_bytes);
    for stop in [0, input.len() / 2, input.len(), output.len() - 1] {
        let mut sink = ShortSink {
            bytes: Vec::new(),
            stop,
        };
        assert!(copy_pdf(&mut input.as_slice(), &mut sink, &limits, &NEVER).is_err());
        assert_eq!(sink.bytes.len(), stop);
    }
    source.stop = Some(pdf(&wrapped(), false).offsets[4] + OUTER.len());
    assert!(matches!(
        copy_pdf(&mut source, &mut Vec::new(), &limits, &NEVER)
            .unwrap_err()
            .kind,
        ErrorKind::Truncated { .. }
    ));
    let limits = Limits {
        max_allocation_bytes: 512,
        ..limits
    };
    assert!(matches!(
        copy_pdf(&mut input.as_slice(), &mut Vec::new(), &limits, &NEVER)
            .unwrap_err()
            .kind,
        ErrorKind::LimitExceeded { .. }
    ));
}

#[test]
fn both_headers_obey_the_fixed_byte_bound() {
    for size in [255, 256, 257] {
        for inner in [false, true] {
            let header = if inner { INNER } else { OUTER };
            let long =
                header.replacen("obj", &format!("obj{}", " ".repeat(size - header.len())), 1);
            assert_eq!(long.len(), size);
            let form = if inner {
                format!("{OUTER}{long}{TAIL}{TAIL}")
            } else {
                format!("{long}{INNER}{TAIL}{TAIL}")
            };
            let result = open(&pdf(&form, false).bytes, &Limits::default());
            assert_eq!(result.is_ok(), size <= 256, "header={size}, inner={inner}");
        }
    }
}

#[test]
fn retained_proofs_have_an_aggregate_budget() {
    let f = pdf(&wrapped(), false);
    for count in [30, 400] {
        let mut bytes = f.bytes[..f.xref].to_vec();
        let mut offsets = f.offsets.to_vec();
        for id in 7..7 + count {
            offsets.push(bytes.len());
            bytes.extend_from_slice(
                wrapped()
                    .replacen("4 0 obj", &format!("{id} 0 obj"), 1)
                    .as_bytes(),
            );
        }
        let xref = bytes.len();
        bytes.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes(),
        );
        for offset in &offsets[1..] {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!(
                "trailer\n<</Size {}/Root 1 0 R>>\nstartxref\n{xref}\n%%EOF\n",
                offsets.len()
            )
            .as_bytes(),
        );
        let result = open(&bytes, &Limits::default());
        if count == 30 {
            assert_eq!(result.unwrap().repair_objects().len(), count + 1);
        } else {
            assert!(matches!(
                result.err().unwrap().kind,
                ErrorKind::LimitExceeded {
                    resource: "PDF empty Form proof bytes",
                    ..
                }
            ));
        }
    }
}
