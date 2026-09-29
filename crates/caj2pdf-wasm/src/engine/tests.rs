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
    stores: [Vec<u8>; 4],
}

/// Drive an engine as the JavaScript host does, with optional short I/O.
fn drive(engine: &mut Engine, input: &[u8], short: Option<usize>) -> Run {
    drive_until(engine, input, short, None)
}

fn drive_until(
    engine: &mut Engine,
    input: &[u8],
    short: Option<usize>,
    stop: Option<Status>,
) -> Run {
    let mut run = Run::default();
    loop {
        let status = engine.poll();
        if Some(status) == stop {
            return run;
        }
        match status {
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
            Status::ScratchResize => {
                let Some(Request::ScratchResize { store, bytes }) = engine.request() else {
                    panic!("resize request");
                };
                run.stores[store as usize - 1].resize(bytes as usize, 0);
                assert!(engine.complete_resize());
            }
            Status::ScratchRead => {
                let Some(Request::ScratchRead {
                    store,
                    offset,
                    length,
                }) = engine.request()
                else {
                    panic!("scratch read request");
                };
                let count = short.unwrap_or(length).min(length);
                engine.with_staging(|staging| {
                    staging[..count].copy_from_slice(
                        &run.stores[store as usize - 1][offset as usize..offset as usize + count],
                    )
                });
                assert!(engine.complete_read(count));
            }
            Status::ScratchWrite => {
                let Some(Request::ScratchWrite {
                    store,
                    offset,
                    length,
                }) = engine.request()
                else {
                    panic!("scratch write request");
                };
                let count = short.unwrap_or(length).min(length);
                engine.with_staging(|staging| {
                    run.stores[store as usize - 1][offset as usize..offset as usize + count]
                        .copy_from_slice(&staging[..count])
                });
                assert!(engine.complete_write(count));
            }
            Status::ScratchFlush => {
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
fn malformed_known_formats_and_unsupported_inputs_have_distinct_errors() {
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
        if matches!(format, Some(InputFormat::Hn | InputFormat::C8)) {
            assert!(matches!(failure(&engine), Error::Hnc8(_)));
            assert!(engine.message().contains("HN/C8"));
        } else {
            assert!(matches!(failure(&engine), Error::UnsupportedFormat));
            assert_eq!(engine.message(), "unsupported input format");
        }
        assert_eq!(engine.format(), format);
        assert!(engine.request().is_none());
    }
    let inspect_hn = Operation::Inspect {
        format: Some(InputFormat::Hn),
    };
    let mut engine = Engine::start(4, limits(64), inspect_hn).unwrap();
    drive(&mut engine, b"HN\0\0", None);
    assert!(matches!(failure(&engine), Error::Hnc8Metadata(_)));
    assert_eq!(error_code(failure(&engine)), 16);
    assert!(std::error::Error::source(failure(&engine)).is_some());
    let mut teb = Engine::start(3, limits(1), Operation::Inspect { format: None }).unwrap();
    drive(&mut teb, b"TEB", None);
    assert!(matches!(failure(&teb), Error::UnsupportedFormat));
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

#[test]
fn source_reads_past_the_end_are_clamped_before_reaching_the_host() {
    let shared = Rc::new(RefCell::new(Shared {
        staging: vec![0; 8],
        request: None,
        response: None,
        cancelled: false,
        format: None,
        tables: hnc8::Tables::default(),
    }));
    let mut source = BridgeSource {
        shared: Rc::clone(&shared),
        size: 10,
    };
    let mut context = Context::from_waker(Waker::noop());
    let mut destination = [0_u8; 6];

    let mut read = Box::pin(source.read_at(7, &mut destination));
    assert!(read.as_mut().poll(&mut context).is_pending());
    assert_eq!(
        shared.borrow().request,
        Some(Request::Read {
            offset: 7,
            length: 3
        })
    );
    drop(read);

    for offset in [10, 11] {
        let result = Box::pin(source.read_at(offset, &mut destination))
            .as_mut()
            .poll(&mut context);
        match (offset, result) {
            (10, Poll::Ready(Ok(0))) => {}
            (11, Poll::Ready(Err(Error::InvalidInput { .. }))) => {}
            (_, other) => panic!("unexpected read at {offset}: {other:?}"),
        }
    }
}

#[test]
fn a_task_pending_without_a_request_reports_idle_until_it_completes() {
    // The bridged operations always leave a request when they wait, so a
    // hand-built task stands in for one that yields without host I/O.
    let copy = Operation::Copy {
        offset: 0,
        length: 0,
    };
    let mut engine = Engine::start(0, limits(512), copy).unwrap();
    let mut yielded = false;
    engine.task = Box::pin(poll_fn(move |_| {
        if std::mem::replace(&mut yielded, true) {
            Poll::Ready(Ok(Outcome {
                report: ConversionReport::default(),
                info: None,
            }))
        } else {
            Poll::Pending
        }
    }));
    assert_eq!(engine.poll(), Status::Idle);
    assert_eq!(engine.request(), None);
    assert_eq!(engine.poll(), Status::Done);
}

fn synthetic_hn() -> Vec<u8> {
    let text = 0x15c + 20;
    let descriptor = text + 32;
    let payload = descriptor + 12;
    let mut bytes = vec![0; payload + 49];
    bytes[..8].copy_from_slice(&[72, 78, 0, 0, 0x90, 1, 0, 0]);
    for (at, value) in [
        (0x90, 1),
        (0x15c, text as u32),
        (0x160, 32),
        (descriptor + 4, payload as u32),
        (descriptor + 8, 49),
        (payload, 40),
        (payload + 4, 3),
        (payload + 8, 2),
        (payload + 32, 2),
    ] {
        put_u32(&mut bytes, at, value);
    }
    for (at, value) in [
        (0x164, 1_u16),
        (text, 0x800a),
        (text + 28, 0x8004),
        (payload + 12, 1),
        (payload + 14, 1),
    ] {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
    bytes[payload + 40..payload + 43].fill(255);
    bytes[payload + 48] = 0x92; // Original 101 / 010 rows under the invented table.
    bytes
}

fn add_table(engine: &mut Engine, table: u32, count: usize) {
    for _ in 0..count {
        assert!(engine.add_hnc8_state(table, 0x4000, 0, 0, 0));
    }
}

#[test]
fn hnc8_type0_converts_through_short_scratch_io_and_clears_stores() {
    let input = synthetic_hn();
    for (chunk, with_mq) in [(1, false), (3, false), (7, true)] {
        let mut engine =
            Engine::start(input.len() as u64, limits(chunk), convert_op(None)).unwrap();
        add_table(&mut engine, 0, 113);
        if with_mq {
            add_table(&mut engine, 1, 47);
        }
        let run = drive(&mut engine, &input, Some(1));
        assert_eq!(outcome(&engine).report.pages_converted, 1);
        assert!(run.output.starts_with(b"%PDF-1.7"));
        assert!(run.output.ends_with(b"%%EOF\n"));
        assert!(run.stores.iter().all(Vec::is_empty));
        assert!(!engine.add_hnc8_state(0, 0x4000, 0, 0, 0));
    }
}

#[test]
fn hnc8_configuration_and_located_failures_are_explicit() {
    let input = synthetic_hn();
    for (table, count) in [(0, 0), (0, 1), (1, 1)] {
        let mut engine = Engine::start(input.len() as u64, limits(8), convert_op(None)).unwrap();
        add_table(&mut engine, table, count);
        drive(&mut engine, &input, None);
        let error = failure(&engine);
        if count == 0 {
            assert_eq!(error_code(error), 16);
            assert!(engine.message().contains("page 1"));
            assert!(engine.message().contains("image 1"));
            assert!(std::error::Error::source(error).is_some());
        } else {
            assert!(matches!(error, Error::InvalidInput { .. }));
        }
    }
    let mut engine = Engine::start(0, limits(8), convert_op(None)).unwrap();
    for args in [
        (2, 1, 0, 0, 0),
        (0, 0, 0, 0, 0),
        (0, 0x8000, 0, 0, 0),
        (0, 1, 113, 0, 0),
        (0, 1, 0, 113, 0),
        (0, 1, 0, 0, 2),
        (1, 1, 47, 0, 0),
    ] {
        assert!(!engine.add_hnc8_state(args.0, args.1, args.2, args.3, args.4));
    }
    add_table(&mut engine, 0, 113);
    add_table(&mut engine, 1, 47);
    assert!(!engine.add_hnc8_state(0, 1, 0, 0, 0));
    assert!(!engine.add_hnc8_state(1, 1, 0, 0, 0));
    let mut tiny = limits(1);
    tiny.max_allocation_bytes = 1;
    let mut engine = Engine::start(0, tiny, convert_op(None)).unwrap();
    assert!(!engine.add_hnc8_state(0, 1, 0, 0, 0));
    assert!(!engine.add_hnc8_state(1, 1, 0, 0, 0));
}

fn scratch_engine() -> Engine {
    use caj2pdf_core::jbig2::text_composer::RandomAccessScratch;
    let mut engine = Engine::start(
        0,
        limits(3),
        Operation::Copy {
            offset: 0,
            length: 0,
        },
    )
    .unwrap();
    let shared = Rc::clone(&engine.shared);
    engine.task = Box::pin(async move {
        for id in 1..=4 {
            let mut store = scratch::Scratch::new(Rc::clone(&shared), id, 8);
            assert_eq!(store.size()?, 0);
            assert!(matches!(
                store.set_len(9).await,
                Err(Error::LimitExceeded { .. })
            ));
            store.set_len(8).await?;
            assert!(store.read_at(u64::MAX, &mut [0]).await.is_err());
            assert!(store.write_at(7, &[0; 2]).await.is_err());
            assert_eq!(store.read_at(8, &mut []).await?, 0);
            assert_eq!(store.write_at(8, &[]).await?, 0);
            assert_eq!(store.write_at(2, &[1, 2, 3, 4]).await?, 3);
            let mut bytes = [0; 4];
            assert_eq!(store.read_at(2, &mut bytes).await?, 3);
            assert_eq!(bytes, [1, 2, 3, 0]);
            store.flush().await?;
            store.set_len(0).await?;
        }
        Ok(Outcome::default())
    });
    engine
}

#[test]
fn all_four_scratch_stores_use_bounded_acknowledged_requests() {
    let mut engine = scratch_engine();
    assert!(!engine.complete_resize());
    assert_eq!(engine.poll(), Status::ScratchResize);
    assert!(!engine.complete_read(0));
    assert!(!engine.complete_write(0));
    assert!(!engine.complete_flush());
    let run = drive(&mut engine, &[], None);
    outcome(&engine);
    assert!(run.stores.iter().all(Vec::is_empty));
}

#[test]
fn each_pending_scratch_operation_cancels_without_another_host_request() {
    for stop in [
        Status::ScratchResize,
        Status::ScratchRead,
        Status::ScratchWrite,
        Status::ScratchFlush,
    ] {
        let mut engine = scratch_engine();
        drive_until(&mut engine, &[], None, Some(stop));
        engine.cancel();
        assert_eq!(engine.poll(), Status::Failed);
        assert_eq!(error_code(failure(&engine)), 6);
        assert!(engine.request().is_none());
    }
}

#[test]
fn hnb_empty_source_rows_are_not_silently_omitted() {
    let mut input = vec![0; 0xd8 + 20];
    input[..8].copy_from_slice(&[72, 78, 0, 0, 0xc8, 0, 0, 0]);
    put_u32(&mut input, 0x90, 1);
    put_u32(&mut input, 0xd8, 0xd8 + 20);
    let mut engine = Engine::start(
        input.len() as u64,
        limits(7),
        Operation::Convert {
            format: None,
            options: ConversionOptions {
                include_bookmarks: false,
            },
        },
    )
    .unwrap();
    drive(&mut engine, &input, None);
    assert!(matches!(failure(&engine), Error::Hnc8(_)));
    assert!(
        engine.message().contains("cannot omit source pages"),
        "{}",
        engine.message()
    );
    assert!(engine.message().contains("page 1"));
}

fn hn_metadata() -> Vec<u8> {
    let mut bytes = vec![0; 0x15c + 2 * 308 + 2 * 20];
    bytes[..8].copy_from_slice(&[72, 78, 0, 0, 0x90, 1, 0, 0]);
    put_u32(&mut bytes, 0x90, 2);
    put_u32(&mut bytes, 0x158, 2);
    for index in 0..2 {
        let at = 0x15c + index * 308;
        bytes[at..at + 4].copy_from_slice(b"Root");
        bytes[at + 280] = b'1' + index as u8;
        put_u32(&mut bytes, at + 304, index as u32 + 1);
    }
    bytes
}

#[test]
fn hnc8_inspection_streams_outline_validation_without_codec_or_scratch() {
    let mut c8 = vec![0; 0x50 + 20];
    c8[0] = 0xc8;
    put_u32(&mut c8, 8, 1);
    let mut hnb = vec![0; 0xd8 + 20];
    hnb[..8].copy_from_slice(&[72, 78, 0, 0, 0xc8, 0, 0, 0]);
    put_u32(&mut hnb, 0x90, 1);
    for (input, pages, bookmarks) in [
        (hn_metadata(), 2, Some(2)),
        (synthetic_hn(), 1, Some(0)),
        (c8, 1, None),
        (hnb, 1, None),
    ] {
        let mut engine = Engine::start(
            input.len() as u64,
            limits(1),
            Operation::Inspect { format: None },
        )
        .unwrap();
        let run = drive(&mut engine, &input, Some(1));
        let info = outcome(&engine).info.as_ref().unwrap();
        assert_eq!((info.page_count, info.bookmark_count), (pages, bookmarks));
        assert!(run.output.is_empty());
        assert_eq!(run.flushes, 0);
        assert_eq!(run.max_read, 1);
        assert!(run.stores.iter().all(Vec::is_empty));
    }
}

#[test]
fn hna_inspection_rejects_invalid_records_and_resource_limits() {
    for case in 0..5 {
        let mut input = hn_metadata();
        let mut bounded = limits(1);
        match case {
            0 => put_u32(&mut input, 0x15c + 308 + 304, 4),
            1 => bounded.max_bookmarks = 1,
            2 => bounded.max_pages = 1,
            3 => bounded.max_allocation_bytes = 8,
            _ => {
                input.pop();
            }
        }
        let mut engine = Engine::start(
            input.len() as u64,
            bounded,
            Operation::Inspect { format: None },
        )
        .unwrap();
        drive(&mut engine, &input, None);
        assert_eq!(error_code(failure(&engine)), 16);
        assert!(engine.message().contains("byte"));
        assert!(std::error::Error::source(failure(&engine)).is_some());
    }
}
