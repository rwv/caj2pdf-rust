// SPDX-License-Identifier: MIT

//! Native tests of the poll/resume engine used by the WASM exports.

use super::*;
use caj2pdf_core::{Limits, PdfErrorKind};
use std::{fs::read, io, path::Path};

const KDH_PDF_START: usize = 254;

fn fixture(name: &str) -> Vec<u8> {
    read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name),
    )
    .unwrap()
}

fn kdh_bytes(pdf: &[u8]) -> Vec<u8> {
    let key = b"FZHMEI";
    let mut bytes = vec![0_u8; KDH_PDF_START];
    bytes[..32].copy_from_slice(b"KDH 2.00 Copyright(C) 2000 CAJCD");
    bytes[0x28..0x2c].copy_from_slice(&[0, 0, 2, 0]);
    bytes.extend(
        pdf.iter()
            .enumerate()
            .map(|(index, byte)| byte ^ key[index % key.len()]),
    );
    bytes
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

/// A two-page, one-bookmark CAJ whose page-tree root is written by the core.
/// Layout facts are those registered in `docs/caj-format.md`.
fn caj_bytes() -> Vec<u8> {
    let objects = [
        (
            3,
            "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 200 100] /Resources << >> >>",
        ),
        (
            4,
            "<< /Type /Page /Parent 5 0 R /MediaBox [0 0 300 150] /Resources << >> >>",
        ),
        (5, "<< /Type /Pages /Count 2 /Kids [3 0 R 4 0 R] >>"),
    ];
    let body: Vec<u8> = objects
        .iter()
        .flat_map(|(number, dictionary)| {
            format!("{number} 0 obj\n{dictionary}\nendobj\n").into_bytes()
        })
        .collect();
    const TABLE: usize = 0x400;
    let body_start = TABLE + 2 * 12;
    let mut bytes = vec![0_u8; body_start];
    bytes[..4].copy_from_slice(b"CAJ\0");
    put_u32(&mut bytes, 0x10, 2);
    put_u32(&mut bytes, 0x14, TABLE as u32);
    put_u32(&mut bytes, 0x110, 1);
    bytes[0x114..0x114 + 5].copy_from_slice(b"Intro");
    bytes[0x114 + 280] = b'1';
    put_u32(&mut bytes, 0x114 + 304, 1);
    put_u32(&mut bytes, TABLE, body_start as u32);
    put_u32(&mut bytes, TABLE + 4, body.len() as u32);
    put_u32(&mut bytes, TABLE + 8, 3);
    put_u32(&mut bytes, TABLE + 12, (body_start + body.len()) as u32);
    put_u32(&mut bytes, TABLE + 20, 4);
    bytes.extend_from_slice(&body);
    bytes
}

fn limits(chunk: usize) -> Limits {
    Limits {
        io_chunk_bytes: chunk,
        ..Limits::default()
    }
}

fn convert_op(format: Option<InputFormat>) -> Operation {
    Operation::Convert {
        format,
        options: ConversionOptions::default(),
    }
}

/// Host-side record of one driven operation.
#[derive(Default)]
struct Run {
    output: Vec<u8>,
    flushes: usize,
    max_read: usize,
    max_write: usize,
}

/// Drive an engine as the JavaScript host does, with optional short I/O.
fn drive(engine: &mut Engine, input: &[u8], short: Option<usize>) -> Run {
    let mut run = Run::default();
    loop {
        match engine.poll() {
            Status::Read => {
                let Some(Request::Read { offset, length }) = engine.request() else {
                    panic!("read status without a read request");
                };
                run.max_read = run.max_read.max(length);
                let start = offset as usize;
                let count = short.unwrap_or(length).min(length).min(input.len() - start);
                engine.with_staging(|staging| {
                    staging[..count].copy_from_slice(&input[start..start + count]);
                });
                assert!(engine.complete_read(count));
            }
            Status::Write => {
                let Some(Request::Write { length }) = engine.request() else {
                    panic!("write status without a write request");
                };
                run.max_write = run.max_write.max(length);
                let accepted = short.unwrap_or(length).min(length);
                engine.with_staging(|staging| run.output.extend_from_slice(&staging[..accepted]));
                assert!(engine.complete_write(accepted));
            }
            Status::Flush => {
                run.flushes += 1;
                assert!(engine.complete_flush());
            }
            Status::Done | Status::Failed => return run,
            Status::Idle => panic!("engine yielded without a request"),
        }
    }
}

fn outcome(engine: &Engine) -> &Outcome {
    match engine.result() {
        Some(Ok(outcome)) => outcome,
        Some(Err(error)) => panic!("operation failed: {error}"),
        None => panic!("operation incomplete"),
    }
}

fn failure(engine: &Engine) -> &Error {
    match engine.result() {
        Some(Err(error)) => error,
        _ => panic!("operation did not fail"),
    }
}

#[test]
fn detects_observed_signatures_only_at_the_start() {
    let cases: [(&[u8], Option<InputFormat>); 10] = [
        (b"%PDF-1.7", Some(InputFormat::Pdf)),
        (b"CAJ\0", Some(InputFormat::Caj)),
        (b"KDH 2", Some(InputFormat::Kdh)),
        (b"HN\0\0", Some(InputFormat::Hn)),
        (&[0xc8, 0, 0, 0, 1], Some(InputFormat::C8)),
        (b"TEB", Some(InputFormat::Teb)),
        (b"%PDF", None),
        (&[0xc8, 0, 0], None),
        (b" %PDF-", None),
        (b"", None),
    ];
    for (prefix, expected) in cases {
        assert_eq!(detect_format(prefix), expected, "{prefix:?}");
    }
}

#[test]
fn format_codes_round_trip_and_reject_unknown_codes() {
    for code in 0..=7 {
        let format = format_from_code(code).expect("known code");
        assert_eq!(format_code(format), code);
    }
    assert_eq!(format_from_code(8), None);
}

#[test]
fn error_codes_are_stable() {
    let cases = [
        (Error::UnsupportedFormat, 1),
        (Error::InvalidInput { reason: "x" }, 2),
        (
            Error::TruncatedInput {
                offset: 0,
                expected: 1,
                available: 0,
            },
            3,
        ),
        (
            Error::LimitExceeded {
                resource: "x",
                limit: 0,
                attempted: 1,
            },
            4,
        ),
        (Error::Io(io::Error::other("x")), 5),
        (Error::Cancelled, 6),
        (Error::RandomAccessRequired, 7),
        (
            Error::PdfLimitExceeded {
                offset: 0,
                object: None,
                resource: "x",
                limit: 0,
                attempted: 1,
            },
            12,
        ),
        (
            Error::Caj {
                offset: 0,
                record: None,
                reason: "x",
            },
            13,
        ),
        (
            Error::CajLimitExceeded {
                offset: 0,
                record: None,
                resource: "x",
                limit: 0,
                attempted: 1,
            },
            14,
        ),
        (
            Error::Kdh {
                offset: 0,
                reason: "x",
            },
            15,
        ),
    ];
    for (error, code) in cases {
        assert_eq!(error_code(&error), code, "{error}");
    }
    let kinds = [
        (PdfErrorKind::Malformed, 8),
        (PdfErrorKind::Encrypted, 9),
        (PdfErrorKind::UnsupportedFeature, 10),
        (PdfErrorKind::AmbiguousRepair, 11),
    ];
    for (kind, code) in kinds {
        let error = Error::Pdf {
            offset: 0,
            object: None,
            kind,
            reason: "x",
        };
        assert_eq!(error_code(&error), code);
    }
}

#[test]
fn rejects_invalid_limits_before_allocating() {
    for chunk in [0, caj2pdf_core::MAX_IO_CHUNK + 1] {
        assert!(Engine::start(1, limits(chunk), convert_op(None)).is_err());
    }
    let oversized = Limits {
        max_allocation_bytes: MAX_ALLOCATION_LIMIT + 1,
        ..Limits::default()
    };
    assert!(matches!(
        Engine::start(1, oversized, convert_op(None)),
        Err(Error::LimitExceeded {
            resource: "WASM allocation limit",
            ..
        })
    ));
}

#[test]
fn auto_detected_pdf_is_copied_through_bounded_chunks_and_flushed() {
    let pdf = fixture("valid_out_of_order_objects.pdf");
    let mut engine = Engine::start(pdf.len() as u64, limits(512), convert_op(None)).unwrap();
    let run = drive(&mut engine, &pdf, None);
    assert_eq!(run.output, pdf);
    assert_eq!(run.flushes, 1);
    assert!(run.max_read <= 512 && run.max_write <= 512);
    assert_eq!(engine.format(), Some(InputFormat::Pdf));
    let report = outcome(&engine).report;
    assert_eq!(report.output_bytes_written, pdf.len() as u64);
    assert_eq!(report.pages_converted, 2);
    assert!(report.input_bytes_read >= pdf.len() as u64);
}

#[test]
fn kdh_and_caj_conversions_use_the_core_engines_with_short_io() {
    let pdf = fixture("valid_out_of_order_objects.pdf");
    let kdh = kdh_bytes(&pdf);
    let mut engine = Engine::start(kdh.len() as u64, limits(4096), convert_op(None)).unwrap();
    let run = drive(&mut engine, &kdh, Some(333));
    assert_eq!(run.output, pdf);
    assert_eq!(engine.format(), Some(InputFormat::Kdh));
    assert_eq!(outcome(&engine).report.pages_converted, 2);

    let caj = caj_bytes();
    let mut engine = Engine::start(caj.len() as u64, limits(256), convert_op(None)).unwrap();
    let run = drive(&mut engine, &caj, Some(100));
    assert!(run.output.starts_with(b"%PDF-"));
    assert!(run.output.trim_ascii_end().ends_with(b"%%EOF"));
    assert!(run.max_read <= 256 && run.max_write <= 256);
    let report = outcome(&engine).report;
    assert_eq!(engine.format(), Some(InputFormat::Caj));
    assert_eq!(report.pages_converted, 2);
    assert_eq!(report.bookmarks_written, 1);
    assert_eq!(report.output_bytes_written, run.output.len() as u64);
}

#[test]
fn caj_conversion_honors_disabled_bookmarks_and_explicit_format() {
    let caj = caj_bytes();
    let operation = Operation::Convert {
        format: Some(InputFormat::Caj),
        options: ConversionOptions {
            include_bookmarks: false,
        },
    };
    let mut engine = Engine::start(caj.len() as u64, limits(4096), operation).unwrap();
    drive(&mut engine, &caj, None);
    assert_eq!(outcome(&engine).report.bookmarks_written, 0);
}

#[test]
fn recognized_image_formats_and_unknown_inputs_are_unsupported() {
    let cases: [(&[u8], Option<InputFormat>); 5] = [
        (b"HN\0\0\x90\x01\0\0", Some(InputFormat::Hn)),
        (&[0xc8, 0, 0, 0, 0, 0], Some(InputFormat::C8)),
        (b"TEB....", Some(InputFormat::Teb)),
        (b"unknown", None),
        (b"", None),
    ];
    for (input, format) in cases {
        let mut engine = Engine::start(input.len() as u64, limits(64), convert_op(None)).unwrap();
        let run = drive(&mut engine, input, None);
        assert!(run.output.is_empty());
        assert!(matches!(failure(&engine), Error::UnsupportedFormat));
        assert_eq!(engine.format(), format);
        assert_eq!(engine.message(), "unsupported input format");
        assert!(engine.request().is_none());
    }
    let inspect_hn = Operation::Inspect {
        format: Some(InputFormat::Hn),
    };
    let mut engine = Engine::start(4, limits(64), inspect_hn).unwrap();
    drive(&mut engine, b"HN\0\0", None);
    assert!(matches!(failure(&engine), Error::UnsupportedFormat));
}

#[test]
fn inspects_pdf_caj_and_kdh_without_output() {
    let pdf = fixture("valid_nested_outline.pdf");
    let cases = [
        (pdf.clone(), InputFormat::Pdf, 2, None),
        (caj_bytes(), InputFormat::Caj, 2, Some(1)),
        (kdh_bytes(&pdf), InputFormat::Kdh, 2, None),
    ];
    for (input, format, pages, bookmarks) in cases {
        let operation = Operation::Inspect { format: None };
        let mut engine = Engine::start(input.len() as u64, limits(1024), operation).unwrap();
        let run = drive(&mut engine, &input, None);
        assert!(run.output.is_empty() && run.flushes == 0);
        let outcome = outcome(&engine);
        assert_eq!(
            outcome.info,
            Some(DocumentInfo {
                format,
                page_count: pages,
                bookmark_count: bookmarks,
            })
        );
        assert!(outcome.report.input_bytes_read > 0);
    }
}

#[test]
fn engine_errors_carry_typed_errors_and_messages() {
    let truncated = fixture("truncated_kdh.kdh");
    let mut engine = Engine::start(truncated.len() as u64, limits(64), convert_op(None)).unwrap();
    drive(&mut engine, &truncated, None);
    assert!(matches!(failure(&engine), Error::TruncatedInput { .. }));
    assert!(engine.message().starts_with("truncated input"));

    let small = Limits {
        max_input_bytes: 3,
        ..limits(64)
    };
    let mut engine = Engine::start(4, small, convert_op(None)).unwrap();
    assert_eq!(engine.poll(), Status::Failed);
    assert!(matches!(failure(&engine), Error::LimitExceeded { .. }));
    // A completed engine keeps reporting its result.
    assert_eq!(engine.poll(), Status::Failed);
}

#[test]
fn rejects_invalid_completions_without_corrupting_the_request() {
    let input = [1_u8, 2, 3, 4];
    let copy = Operation::Copy {
        offset: 0,
        length: 4,
    };
    let mut engine = Engine::start(4, limits(2), copy).unwrap();
    assert!(!engine.complete_read(0), "no request is pending yet");
    assert_eq!(engine.poll(), Status::Read);
    assert_eq!(
        engine.request(),
        Some(Request::Read {
            offset: 0,
            length: 2
        })
    );
    assert!(!engine.complete_read(3));
    assert!(!engine.complete_write(1));
    assert!(!engine.complete_flush());
    assert_eq!(engine.poll(), Status::Read);
    engine.with_staging(|staging| staging[..2].copy_from_slice(&input[..2]));
    assert!(engine.complete_read(2));
    assert!(!engine.complete_read(2), "the request was consumed");
    assert_eq!(engine.poll(), Status::Write);
    assert!(!engine.complete_write(3));
    assert!(!engine.complete_read(1));
    assert!(engine.complete_write(2));
    let run = drive(&mut engine, &input, None);
    assert_eq!(run.output, [3, 4], "the first chunk was accepted by hand");
    assert_eq!(outcome(&engine).report.output_bytes_written, 4);
}

#[test]
fn cancellation_resolves_the_pending_request_with_a_typed_error() {
    let pdf = fixture("valid_out_of_order_objects.pdf");
    let mut engine = Engine::start(pdf.len() as u64, limits(64), convert_op(None)).unwrap();
    assert_eq!(engine.poll(), Status::Read);
    engine.cancel();
    assert_eq!(engine.poll(), Status::Failed);
    assert!(matches!(failure(&engine), Error::Cancelled));
    assert!(engine.request().is_none());

    let copy = Operation::Copy {
        offset: 0,
        length: 2,
    };
    for status in [Status::Write, Status::Flush] {
        let mut engine = Engine::start(2, limits(2), copy).unwrap();
        loop {
            let current = engine.poll();
            if current == status {
                break;
            }
            match current {
                Status::Read => assert!(engine.complete_read(2)),
                Status::Write => assert!(engine.complete_write(2)),
                other => panic!("unexpected {other:?}"),
            }
        }
        engine.cancel();
        assert_eq!(engine.poll(), Status::Failed);
        assert!(matches!(failure(&engine), Error::Cancelled));
    }
}

#[test]
fn error_messages_are_bounded_on_a_character_boundary() {
    let prefix = "I/O error: ".len();
    let long = format!("{}é", "x".repeat(MAX_MESSAGE_BYTES - prefix - 1));
    let message = bounded_message(&Error::Io(io::Error::other(long)));
    assert_eq!(message.len(), MAX_MESSAGE_BYTES - 1);
    assert!(message.ends_with('x'));
    assert_eq!(bounded_message(&Error::Cancelled), "operation cancelled");
}
