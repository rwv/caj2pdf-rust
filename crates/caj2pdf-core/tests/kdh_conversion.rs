// SPDX-License-Identifier: MIT

//! Synthetic KDH wrappers around repository-owned PDF test input.

use caj2pdf_core::{
    Cancellation, Error, Limits, NeverCancel, RangedSource, SequentialSink,
    kdh::{KdhPdfSource, convert_kdh},
    native::WriteSink,
};
use std::{
    cell::Cell,
    fs::read,
    future::Future,
    io,
    path::Path,
    pin::pin,
    rc::Rc,
    task::{Context, Poll, Waker},
};

const PDF_START: usize = 254;
const KEY: &[u8; 6] = b"FZHMEI";

fn run<F: Future>(future: F) -> F::Output {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = pin!(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("native test source unexpectedly yielded"),
    }
}

fn fixture_pdf() -> Vec<u8> {
    read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/valid_out_of_order_objects.pdf"),
    )
    .unwrap()
}

fn kdh_bytes(pdf: &[u8], decoded_tail: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0_u8; PDF_START];
    bytes[..32].copy_from_slice(b"KDH 2.00 Copyright(C) 2000 CAJCD");
    bytes[0x28..0x2c].copy_from_slice(&[0, 0, 2, 0]);
    bytes.extend_from_slice(pdf);
    bytes.extend_from_slice(decoded_tail);
    for (index, byte) in bytes[PDF_START..].iter_mut().enumerate() {
        *byte ^= KEY[index % KEY.len()];
    }
    bytes
}

struct MeasuredSource {
    bytes: Vec<u8>,
    zero_tail: u64,
    max_request: usize,
    max_return: usize,
    reads: Rc<Cell<usize>>,
    fail_reads: Rc<Cell<bool>>,
}

impl MeasuredSource {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            zero_tail: 0,
            max_request: 0,
            max_return: usize::MAX,
            reads: Rc::new(Cell::new(0)),
            fail_reads: Rc::new(Cell::new(false)),
        }
    }
}

impl RangedSource for MeasuredSource {
    fn size(&self) -> u64 {
        self.bytes.len() as u64 + self.zero_tail
    }

    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        self.max_request = self.max_request.max(destination.len());
        self.reads.set(self.reads.get() + 1);
        if self.fail_reads.get() {
            return Ok(0);
        }
        let count = (self.size() - offset)
            .min(destination.len() as u64)
            .min(self.max_return as u64) as usize;
        let from_bytes = (self.bytes.len() as u64)
            .saturating_sub(offset)
            .min(count as u64) as usize;
        if from_bytes != 0 {
            destination[..from_bytes]
                .copy_from_slice(&self.bytes[offset as usize..offset as usize + from_bytes]);
        }
        destination[from_bytes..count].fill(0);
        Ok(count)
    }
}

#[test]
fn a_later_zero_read_reports_the_absolute_kdh_offset() {
    let pdf = fixture_pdf();
    let mut source = MeasuredSource::new(kdh_bytes(&pdf, b""));
    let fail_reads = Rc::clone(&source.fail_reads);
    let mut decoded = run(KdhPdfSource::open(
        &mut source,
        &Limits::default(),
        &NeverCancel,
    ))
    .unwrap();
    fail_reads.set(true);
    let mut output = Vec::new();
    let error = run(caj2pdf_core::pdf::copy_pdf(
        &mut decoded,
        &mut WriteSink::new(&mut output),
        &Limits::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(matches!(
        error,
        Error::TruncatedInput {
            offset: 254,
            expected: 8,
            available: 0,
        }
    ));
    assert!(output.is_empty());
}

struct CancelAfterReads {
    reads: Rc<Cell<usize>>,
    threshold: usize,
}

impl Cancellation for CancelAfterReads {
    fn is_cancelled(&self) -> bool {
        self.reads.get() >= self.threshold
    }
}

struct FailingSink {
    remaining: usize,
}

impl SequentialSink for FailingSink {
    async fn write(&mut self, bytes: &[u8]) -> caj2pdf_core::Result<usize> {
        if self.remaining == 0 {
            return Err(Error::Io(io::Error::other("injected KDH sink failure")));
        }
        let count = bytes.len().min(self.remaining);
        self.remaining -= count;
        Ok(count)
    }

    async fn flush(&mut self) -> caj2pdf_core::Result<()> {
        Ok(())
    }
}

#[test]
fn bounded_kdh_source_discards_a_large_tail_and_false_eof() {
    let pdf = fixture_pdf();
    let tail = format!("\n%%EOF\nstartxref\n{}\n%%EOF\n", pdf.len() + 8);
    let mut source = MeasuredSource::new(kdh_bytes(&pdf, tail.as_bytes()));
    source.zero_tail = 2 * 1024 * 1024;
    let limits = Limits {
        io_chunk_bytes: 257,
        ..Limits::default()
    };
    {
        let mut decoded = run(KdhPdfSource::open(&mut source, &limits, &NeverCancel)).unwrap();
        assert_eq!(decoded.pdf_len(), pdf.len() as u64);
        assert_eq!(decoded.trailing_len(), tail.len() as u64 + 2 * 1024 * 1024);
        let mut actual = vec![0; pdf.len()];
        for (offset, chunk) in actual.chunks_mut(97).enumerate() {
            let count = run(decoded.read_at((offset * 97) as u64, chunk)).unwrap();
            assert_eq!(count, chunk.len());
        }
        assert_eq!(actual, pdf);
    }
    assert!(source.max_request <= limits.io_chunk_bytes);
}

#[test]
fn a_second_plausible_xref_end_is_rejected_as_ambiguous() {
    let pdf = fixture_pdf();
    let tail = format!(
        "xref\n0 1\n0000000000 65535 f \nstartxref\n{}\n%%EOF\n",
        pdf.len()
    );
    let mut source = MeasuredSource::new(kdh_bytes(&pdf, tail.as_bytes()));
    let error = run(KdhPdfSource::open(
        &mut source,
        &Limits::default(),
        &NeverCancel,
    ))
    .err()
    .unwrap();
    assert!(matches!(
        error,
        Error::Kdh {
            reason: "ambiguous PDF end in KDH trailer",
            ..
        }
    ));
}

#[test]
fn full_kdh_input_size_limit_includes_trailing_bytes() {
    let pdf = fixture_pdf();
    let mut source = MeasuredSource::new(kdh_bytes(&pdf, b""));
    source.zero_tail = 2 * 1024 * 1024;
    let limits = Limits {
        max_input_bytes: source.bytes.len() as u64 + source.zero_tail - 1,
        ..Limits::default()
    };
    assert!(matches!(
        run(KdhPdfSource::open(&mut source, &limits, &NeverCancel)),
        Err(Error::LimitExceeded {
            resource: "input bytes",
            ..
        })
    ));
    assert_eq!(source.max_request, 0);
}

#[test]
fn kdh_conversion_uses_the_shared_pdf_reader_and_sink() {
    let pdf = fixture_pdf();
    let mut source = MeasuredSource::new(kdh_bytes(&pdf, b"metadata"));
    let mut output = Vec::new();
    let report = run(convert_kdh(
        &mut source,
        &mut WriteSink::new(&mut output),
        &Limits::default(),
        &NeverCancel,
    ))
    .unwrap();
    assert_eq!(output, pdf);
    assert_eq!(report.pages_converted, 2);
    assert_eq!(report.output_bytes_written, pdf.len() as u64);
}

#[test]
fn kdh_conversion_accepts_short_positioned_reads() {
    let pdf = fixture_pdf();
    let mut source = MeasuredSource::new(kdh_bytes(&pdf, b""));
    source.max_return = 7;
    let limits = Limits {
        io_chunk_bytes: 97,
        ..Limits::default()
    };
    let mut output = Vec::new();
    let report = run(convert_kdh(
        &mut source,
        &mut WriteSink::new(&mut output),
        &limits,
        &NeverCancel,
    ))
    .unwrap();
    assert_eq!(output, pdf);
    assert_eq!(report.pages_converted, 2);
    assert!(source.reads.get() > 100);
    assert!(source.max_request <= limits.io_chunk_bytes);
}

#[test]
fn kdh_scan_observes_cancellation_before_writing_output() {
    let pdf = fixture_pdf();
    let mut source = MeasuredSource::new(kdh_bytes(&pdf, b""));
    source.zero_tail = 2 * 1024 * 1024;
    let cancellation = CancelAfterReads {
        reads: Rc::clone(&source.reads),
        threshold: 10,
    };
    let limits = Limits {
        io_chunk_bytes: 97,
        ..Limits::default()
    };
    let mut output = Vec::new();
    let result = run(convert_kdh(
        &mut source,
        &mut WriteSink::new(&mut output),
        &limits,
        &cancellation,
    ));
    assert!(matches!(result, Err(Error::Cancelled)));
    assert!(output.is_empty());
    assert!(source.max_request <= limits.io_chunk_bytes);
}

#[test]
fn kdh_conversion_propagates_sink_failure() {
    let pdf = fixture_pdf();
    let mut source = MeasuredSource::new(kdh_bytes(&pdf, b""));
    let limits = Limits {
        io_chunk_bytes: 97,
        ..Limits::default()
    };
    let error = run(convert_kdh(
        &mut source,
        &mut FailingSink { remaining: 128 },
        &limits,
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(matches!(error, Error::Io(_)));
}

#[test]
fn kdh_header_payload_and_eof_fail_with_locations() {
    let pdf = fixture_pdf();
    let limits = Limits::default();
    let mut truncated = MeasuredSource::new(vec![0; 40]);
    assert!(matches!(
        run(KdhPdfSource::open(&mut truncated, &limits, &NeverCancel)),
        Err(Error::TruncatedInput {
            offset: 0,
            expected: 254,
            available: 40
        })
    ));

    let mut bad_signature = kdh_bytes(&pdf, b"");
    bad_signature[0] = b'!';
    assert!(matches!(
        run(KdhPdfSource::open(
            &mut MeasuredSource::new(bad_signature),
            &limits,
            &NeverCancel
        )),
        Err(Error::Kdh {
            offset: 0,
            reason: "KDH signature is invalid"
        })
    ));

    let short_payload = kdh_bytes(&pdf, b"")[..PDF_START + 7].to_vec();
    assert!(matches!(
        run(KdhPdfSource::open(
            &mut MeasuredSource::new(short_payload),
            &limits,
            &NeverCancel
        )),
        Err(Error::TruncatedInput {
            offset: 254,
            expected: 8,
            available: 7
        })
    ));

    let mut bad_version = kdh_bytes(&pdf, b"");
    bad_version[0x28] = 1;
    assert!(matches!(
        run(KdhPdfSource::open(
            &mut MeasuredSource::new(bad_version),
            &limits,
            &NeverCancel
        )),
        Err(Error::Kdh { offset: 0x28, .. })
    ));

    let mut bad_payload = pdf.clone();
    bad_payload[0] = b'!';
    assert!(matches!(
        run(KdhPdfSource::open(
            &mut MeasuredSource::new(kdh_bytes(&bad_payload, b"")),
            &limits,
            &NeverCancel
        )),
        Err(Error::Kdh { offset: 254, .. })
    ));

    let mut missing_eof = pdf;
    let at = missing_eof
        .windows(5)
        .rposition(|part| part == b"%%EOF")
        .unwrap();
    missing_eof[at] = b'!';
    assert!(matches!(
        run(KdhPdfSource::open(
            &mut MeasuredSource::new(kdh_bytes(&missing_eof, b"")),
            &limits,
            &NeverCancel
        )),
        Err(Error::Kdh {
            reason: "decoded PDF startxref and EOF were not found",
            ..
        })
    ));
}

#[test]
fn corrupt_pdf_object_is_reported_at_kdh_absolute_offset() {
    let mut pdf = fixture_pdf();
    let at = pdf
        .windows(12)
        .position(|part| part == b"/Pages 2 0 R")
        .unwrap();
    pdf[at + 7] = b'8';
    let mut source = MeasuredSource::new(kdh_bytes(&pdf, b""));
    let mut output = Vec::new();
    let error = run(convert_kdh(
        &mut source,
        &mut WriteSink::new(&mut output),
        &Limits::default(),
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(matches!(error, Error::Pdf { offset, .. } if offset >= PDF_START as u64));
}

#[test]
fn pdf_output_limit_preserves_kdh_absolute_error_location() {
    let pdf = fixture_pdf();
    let mut source = MeasuredSource::new(kdh_bytes(&pdf, b""));
    let mut output = Vec::new();
    let limits = Limits {
        max_output_bytes: pdf.len() as u64 - 1,
        ..Limits::default()
    };
    let error = run(convert_kdh(
        &mut source,
        &mut WriteSink::new(&mut output),
        &limits,
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(
        matches!(error, Error::PdfLimitExceeded { offset, resource: "output bytes", .. } if offset >= PDF_START as u64),
        "{error}"
    );
    assert!(output.is_empty());
}
