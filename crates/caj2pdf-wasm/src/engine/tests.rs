// SPDX-License-Identifier: MIT

//! Native tests of the session behind the WASM exports, over an in-memory
//! host.

use super::*;
use caj2pdf_core::Limits;
use std::{fs::read, path::Path};

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
/// Layout facts are those registered in `docs/research/caj-format.md`.
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
        options: ConversionOptions {
            format,
            ..ConversionOptions::default()
        },
    }
}

/// An in-memory host with optional short I/O, failures and cancellation.
#[derive(Default)]
struct Memory<'a> {
    input: &'a [u8],
    fonts: Vec<&'a [u8]>,
    short: Option<usize>,
    output: Vec<u8>,
    flushes: usize,
    reads: usize,
    max_read: usize,
    max_write: usize,
    progress: Vec<u32>,
    cancel_after_reads: Option<usize>,
    cancel_after_writes: Option<usize>,
    cancel_checks: usize,
    fail_reads: bool,
    fail_writes: bool,
}

impl<'a> Memory<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            ..Self::default()
        }
    }

    fn short(mut self, short: usize) -> Self {
        self.short = Some(short);
        self
    }
}

impl Host for Memory<'_> {
    fn read(&mut self, resource: u32, offset: u64, destination: &mut [u8]) -> io::Result<usize> {
        if self.fail_reads {
            return Err(io::Error::other("host read failure"));
        }
        let input = if resource == 0 {
            self.input
        } else {
            self.fonts[resource as usize - 1]
        };
        self.reads += 1;
        self.max_read = self.max_read.max(destination.len());
        let start = offset as usize;
        assert!(
            start + destination.len() <= input.len(),
            "the host was asked for bytes beyond the resource"
        );
        let count = self.short.unwrap_or(usize::MAX).min(destination.len());
        destination[..count].copy_from_slice(&input[start..start + count]);
        Ok(count)
    }

    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.fail_writes {
            return Err(io::Error::other("host write failure"));
        }
        self.max_write = self.max_write.max(bytes.len());
        let count = self.short.unwrap_or(usize::MAX).min(bytes.len());
        self.output.extend_from_slice(&bytes[..count]);
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        Ok(())
    }

    fn progress(&mut self, done: u32, total: u32) {
        assert_eq!(total, PROGRESS_TOTAL);
        self.progress.push(done);
    }

    fn cancelled(&mut self) -> bool {
        self.cancel_checks += 1;
        self.cancel_after_reads
            .is_some_and(|reads| self.reads >= reads)
            || self
                .cancel_after_writes
                .is_some_and(|writes| self.output.len() >= writes)
    }
}

/// Run `operation` over `host` in a fresh session.
fn run(host: &mut Memory<'_>, limits: Limits, operation: Operation) -> Session {
    let mut session = Session::default();
    let size = host.input.len() as u64;
    let status = session.run(host, size, limits, operation);
    assert_eq!(
        status == Status::Done,
        matches!(session.result(), Some(Ok(_))),
        "{status:?}"
    );
    session
}

fn outcome(session: &Session) -> &Outcome {
    match session.result() {
        Some(Ok(outcome)) => outcome,
        Some(Err(error)) => panic!("operation failed: {error}"),
        None => panic!("operation did not run"),
    }
}

fn failure(session: &Session) -> &Error {
    match session.result() {
        Some(Err(error)) => error,
        _ => panic!("operation did not fail"),
    }
}

#[test]
fn format_codes_round_trip_and_reject_unknown_codes() {
    for code in 0..=8 {
        let format = format_from_code(code).expect("known code");
        assert_eq!(format_code(format), code);
    }
    assert_eq!(format_code(Some(InputFormat::Caa)), 8);
    assert_eq!(format_from_code(9), None);
}

#[test]
fn caa_can_be_inspected_but_conversion_refuses_without_output() {
    let bytes = fixture("target_descriptor.caa");
    let mut host = Memory::new(&bytes).short(2);
    let session = run(&mut host, limits(3), Operation::Inspect { format: None });
    let info = outcome(&session).info.as_ref().unwrap();
    assert_eq!(info.format, InputFormat::Caa);
    assert_eq!(info.page_count, None);
    assert_eq!(info.bookmark_count, None);
    assert!(host.output.is_empty());
    assert!(host.max_read <= 3);
    for format in [None, Some(InputFormat::Caa)] {
        let session = run(&mut host, limits(3), convert_op(format));
        assert!(matches!(
            failure(&session).kind,
            ErrorKind::UnsupportedFormat
        ));
        assert_eq!(session.format(), Some(InputFormat::Caa));
        assert!(host.output.is_empty());
    }
}

#[test]
fn error_codes_are_stable() {
    let cases = [
        (Error::from(ErrorKind::UnsupportedFormat), 1),
        (Error::invalid("x"), 2),
        (Error::truncated(0, 1, 0), 3),
        (Error::limit("x", 0, 1), 4),
        (Error::from(ErrorKind::Io(io::Error::other("x"))), 5),
        (Error::cancelled(), 6),
        (Error::limit("x", 0, 1).at(0).in_pdf(None), 12),
        (Error::malformed(0, "x").in_caj(None), 13),
        (Error::limit("x", 0, 1).at(0).in_caj(None), 14),
        (Error::malformed(0, "x").within(Context::Kdh), 15),
    ];
    for (error, code) in cases {
        assert_eq!(error_code(&error), code, "{error}");
    }
    let kinds = [
        (ErrorKind::Malformed, false, 8),
        (ErrorKind::Encrypted, false, 9),
        (ErrorKind::UnsupportedFormat, false, 10),
        (ErrorKind::Malformed, true, 11),
    ];
    for (kind, repair, code) in kinds {
        let error = Error::from(kind).at(0).because("x").within(Context::Pdf {
            object: None,
            repair,
        });
        assert_eq!(error_code(&error), code, "{error}");
    }
    let hnc8 = [
        Error::malformed(0, "x").within(Context::Hnc8 {
            variant: None,
            page: Some(1),
            image: None,
            segment: None,
            stage: None,
        }),
        Error::limit("x", 0, 1).in_jbig2(Some(2)),
        Error::truncated(0, 1, 0).in_jbig2(None),
    ];
    for error in hnc8 {
        assert_eq!(error_code(&error), 16, "{error}");
    }
}

#[test]
fn invalid_limits_are_refused_before_any_host_call() {
    let oversized = Limits {
        max_allocation_bytes: MAX_ALLOCATION_LIMIT + 1,
        ..Limits::default()
    };
    for limits in [limits(0), limits(caj2pdf_core::MAX_IO_CHUNK + 1), oversized] {
        let mut host = Memory::new(b"%PDF-1.7");
        let mut session = Session::default();
        assert_eq!(
            session.run(&mut host, 8, limits, convert_op(None)),
            Status::Invalid
        );
        assert!(session.result().is_none());
        assert_eq!((host.reads, host.cancel_checks), (0, 0));
    }
}

#[test]
fn auto_detected_pdf_is_copied_through_bounded_chunks_and_flushed() {
    let pdf = fixture("valid_out_of_order_objects.pdf");
    let mut host = Memory::new(&pdf);
    let session = run(&mut host, limits(512), convert_op(None));
    assert_eq!(host.output, pdf);
    assert_eq!(host.flushes, 1);
    assert!(host.max_read <= 512 && host.max_write <= 512);
    assert_eq!(session.format(), Some(InputFormat::Pdf));
    let report = &outcome(&session).report;
    assert_eq!(report.output_bytes_written, pdf.len() as u64);
    assert_eq!(report.pages_converted, 2);
    assert!(report.input_bytes_read >= pdf.len() as u64);
    // Progress is the furthest byte read, in increasing thousandths.
    assert!(host.progress.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(host.progress.last(), Some(&PROGRESS_TOTAL));
}

#[test]
fn an_auto_detected_pdf_header_after_leading_bytes_converts_and_inspects() {
    let pdf = fixture("valid_nested_outline.pdf");
    let mut junk = vec![b'x'; 100];
    junk.push(b'\n');
    for prefix in [b"\n".to_vec(), b"\xef\xbb\xbf".to_vec(), junk] {
        let input = [prefix.as_slice(), &pdf].concat();
        let mut host = Memory::new(&input).short(100);
        let session = run(&mut host, limits(256), convert_op(None));
        assert_eq!(host.output, pdf);
        assert!(host.max_read <= 256);
        assert_eq!(session.format(), Some(InputFormat::Pdf));
        assert_eq!(outcome(&session).report.pages_converted, 2);

        let mut host = Memory::new(&input);
        let session = run(&mut host, limits(256), Operation::Inspect { format: None });
        let outcome = outcome(&session);
        assert_eq!(
            outcome.info.as_ref().and_then(|info| info.page_count),
            Some(2)
        );
        // The detection prefix is counted once, beside the PDF reads.
        assert!(outcome.report.input_bytes_read > 1024);

        // An explicit format skips detection, so the header must be at byte 0.
        let mut host = Memory::new(&input);
        let session = run(&mut host, limits(256), convert_op(Some(InputFormat::Pdf)));
        assert!(host.output.is_empty());
        assert!(matches!(
            failure(&session),
            Error {
                kind: _,
                offset: Some(0),
                context: Context::Pdf { .. },
                ..
            }
        ));
    }
    let late = [vec![b' '; 1020].as_slice(), &pdf].concat();
    let mut host = Memory::new(&late);
    let session = run(&mut host, limits(256), convert_op(None));
    assert!(matches!(
        failure(&session),
        Error {
            kind: ErrorKind::UnsupportedFormat,
            ..
        }
    ));
}

#[test]
fn kdh_and_caj_conversions_use_the_core_engines_with_short_io() {
    let pdf = fixture("valid_out_of_order_objects.pdf");
    let kdh = kdh_bytes(&pdf);
    let mut host = Memory::new(&kdh).short(333);
    let session = run(&mut host, limits(4096), convert_op(None));
    assert_eq!(host.output, pdf);
    assert_eq!(session.format(), Some(InputFormat::Kdh));
    assert_eq!(outcome(&session).report.pages_converted, 2);

    let caj = caj_bytes();
    let mut host = Memory::new(&caj).short(100);
    let session = run(&mut host, limits(256), convert_op(None));
    assert!(host.output.starts_with(b"%PDF-"));
    assert!(host.output.trim_ascii_end().ends_with(b"%%EOF"));
    assert!(host.max_read <= 256 && host.max_write <= 256);
    let report = &outcome(&session).report;
    assert_eq!(session.format(), Some(InputFormat::Caj));
    assert_eq!(report.pages_converted, 2);
    assert_eq!(report.bookmarks_written, 1);
    assert_eq!(report.output_bytes_written, host.output.len() as u64);
}

#[test]
fn caj_conversion_honors_disabled_bookmarks_and_explicit_format() {
    let caj = caj_bytes();
    let operation = Operation::Convert {
        options: ConversionOptions {
            format: Some(InputFormat::Caj),
            include_bookmarks: false,
            ..ConversionOptions::default()
        },
    };
    let session = run(&mut Memory::new(&caj), limits(4096), operation);
    assert_eq!(outcome(&session).report.bookmarks_written, 0);
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
        let mut host = Memory::new(input);
        let session = run(&mut host, limits(64), convert_op(None));
        assert!(host.output.is_empty());
        if matches!(format, Some(InputFormat::Hn | InputFormat::C8)) {
            assert!(matches!(failure(&session).context, Context::Hnc8 { .. }));
            assert!(session.message().contains("HN/C8"));
        } else {
            assert!(matches!(
                failure(&session),
                Error {
                    kind: ErrorKind::UnsupportedFormat,
                    ..
                }
            ));
            assert_eq!(session.message(), "unsupported input format");
        }
        assert_eq!(session.format(), format);
    }
    let inspect_hn = Operation::Inspect {
        format: Some(InputFormat::Hn),
    };
    let session = run(&mut Memory::new(b"HN\0\0"), limits(64), inspect_hn);
    assert!(matches!(failure(&session).context, Context::Hnc8 { .. }));
    assert_eq!(error_code(failure(&session)), 16);
    let teb = run(
        &mut Memory::new(b"TEB"),
        limits(1),
        Operation::Inspect { format: None },
    );
    assert!(matches!(
        failure(&teb),
        Error {
            kind: ErrorKind::UnsupportedFormat,
            ..
        }
    ));
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
        let mut host = Memory::new(&input);
        let session = run(&mut host, limits(1024), Operation::Inspect { format: None });
        assert!(host.output.is_empty() && host.flushes == 0);
        let outcome = outcome(&session);
        let info = outcome.info.as_ref().unwrap();
        assert_eq!(
            (info.format, info.page_count, info.bookmark_count),
            (format, Some(pages), bookmarks)
        );
        assert_eq!(info.bookmarks, None);
        assert!(outcome.report.input_bytes_read > 0);
    }
}

#[test]
fn failures_carry_typed_errors_and_messages_and_a_session_runs_once() {
    let truncated = fixture("truncated_kdh.kdh");
    let session = run(&mut Memory::new(&truncated), limits(64), convert_op(None));
    assert!(matches!(
        failure(&session),
        Error {
            kind: ErrorKind::Truncated { .. },
            ..
        }
    ));
    assert!(session.message().starts_with("truncated input"));

    let small = Limits {
        max_input_bytes: 3,
        ..limits(64)
    };
    let mut host = Memory::new(b"%PDF");
    let mut session = run(&mut host, small, convert_op(None));
    assert!(matches!(
        failure(&session),
        Error {
            kind: ErrorKind::LimitExceeded { .. },
            ..
        }
    ));
    assert_eq!(host.reads, 0);
    // A completed session keeps its result until it is replaced.
    assert_eq!(
        session.run(&mut host, 4, limits(64), convert_op(None)),
        Status::Busy
    );
    assert!(matches!(
        failure(&session),
        Error {
            kind: ErrorKind::LimitExceeded { .. },
            ..
        }
    ));
}

#[test]
fn host_failures_and_cancellation_end_the_operation_with_typed_errors() {
    let pdf = fixture("valid_out_of_order_objects.pdf");
    for (fail_reads, fail_writes) in [(true, false), (false, true)] {
        let mut host = Memory {
            fail_reads,
            fail_writes,
            ..Memory::new(&pdf)
        };
        let session = run(&mut host, limits(64), convert_op(None));
        assert!(matches!(
            failure(&session),
            Error {
                kind: ErrorKind::Io(_),
                ..
            }
        ));
        assert_eq!(error_code(failure(&session)), 5);
        assert_eq!(host.flushes, 0);
    }
    for (reads, writes) in [(Some(0), None), (Some(2), None), (None, Some(64))] {
        let mut host = Memory {
            cancel_after_reads: reads,
            cancel_after_writes: writes,
            ..Memory::new(&pdf)
        };
        let session = run(&mut host, limits(64), convert_op(None));
        assert!(matches!(
            failure(&session),
            Error {
                kind: ErrorKind::Cancelled,
                ..
            }
        ));
        assert_eq!(error_code(failure(&session)), 6);
        assert_eq!(host.flushes, 0);
        assert!(host.output.len() < pdf.len());
    }
}

#[test]
fn error_messages_are_bounded_on_a_character_boundary() {
    let prefix = "I/O error: ".len();
    let long = format!("{}é", "x".repeat(MAX_MESSAGE_BYTES - prefix - 1));
    let message = bounded_message(&Error::from(ErrorKind::Io(io::Error::other(long))));
    assert_eq!(message.len(), MAX_MESSAGE_BYTES - 1);
    assert!(message.ends_with('x'));
    assert_eq!(bounded_message(&Error::cancelled()), "operation cancelled");
}

#[test]
fn source_reads_past_the_end_are_clamped_before_reaching_the_host() {
    let input = [7_u8; 10];
    let mut memory = Memory::new(&input);
    {
        let host = RefCell::new(&mut memory);
        let mut source = HostSource::new(&host, 0, 10);
        let mut destination = [0_u8; 6];
        assert_eq!(source.read_at(7, &mut destination).unwrap(), 3);
        assert_eq!(destination[..3], [7; 3]);
        assert_eq!(source.read_at(10, &mut destination).unwrap(), 0);
        assert!(matches!(
            source.read_at(11, &mut destination),
            Err(Error {
                kind: ErrorKind::Malformed,
                ..
            })
        ));
    }
    assert_eq!((memory.reads, memory.max_read), (1, 3));
    assert!(memory.progress.is_empty());
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
        (0xa8, 100_u16),
        (0xaa, 200),
        (text + 8, 80),
        (text + 10, 40),
        (0x164, 1_u16),
        (text, 0x800a),
        (text + 28, 0x8004),
        (payload + 12, 1),
        (payload + 14, 1),
    ] {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
    bytes[payload + 40..payload + 43].fill(255);
    bytes[payload + 48] = 0x39; // Rows 101 / 010 under the standard QM states.
    bytes
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

/// A stored (uncompressed) zlib stream: the framing without a compressor.
fn stored_zlib(data: &[u8]) -> Vec<u8> {
    let (mut a, mut b) = (1_u32, 0_u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    let length = data.len() as u16;
    let mut stream = vec![0x78, 0x01, 0x01];
    stream.extend(length.to_le_bytes());
    stream.extend((!length).to_le_bytes());
    stream.extend(data);
    stream.extend(((b << 16) | a).to_be_bytes());
    stream
}

/// A one-page C8 index followed by an invented application-info package
/// whose declared decoded length is off by `decoded_extra` bytes.
fn c8_with_application_info(decoded_extra: u32) -> Vec<u8> {
    let xml = b"<Package><Note-Package><NoteItems><Item/><Item/></NoteItems>\
</Note-Package><FileProperty-Package><DOI>INVENTED:1</DOI>\
<DURL>http://example.invalid/x</DURL></FileProperty-Package></Package>";
    let mut bytes = vec![0; 0x50 + 20];
    bytes[0] = 0xc8;
    put_u32(&mut bytes, 8, 1);
    let start = bytes.len();
    let stream = stored_zlib(xml);
    bytes.extend((xml.len() as u32 + decoded_extra).to_le_bytes());
    bytes.extend((stream.len() as u32).to_le_bytes());
    bytes.extend(stream);
    bytes.extend(format!("APPINFOSIGN {start}").as_bytes());
    bytes
}

fn native_c8() -> Vec<u8> {
    let mut bytes = vec![0u8; 100];
    bytes[0] = 0xc8;
    put_u32(&mut bytes, 12, 2);
    put_u32(&mut bytes, 8, 1);
    bytes[32..34].copy_from_slice(&100u16.to_le_bytes());
    bytes[34..36].copy_from_slice(&200u16.to_le_bytes());
    put_u32(&mut bytes, 80, 100);
    let words = [
        [0x8001u16, 60],
        [0x8002, 0x1084],
        [30, 0xa0c1],
        [0x8004, 39],
    ];
    put_u32(&mut bytes, 84, 16);
    put_u32(&mut bytes, 96, 116);
    bytes.extend(words.into_iter().flatten().flat_map(u16::to_le_bytes));
    bytes
}

fn native_operation() -> Operation {
    Operation::Convert {
        options: ConversionOptions {
            include_bookmarks: false,
            ..ConversionOptions::default()
        },
    }
}

#[test]
fn hnc8_type0_converts_with_short_io() {
    let input = synthetic_hn();
    for chunk in [1, 3, 7] {
        let mut host = Memory::new(&input).short(1);
        let session = run(&mut host, limits(chunk), convert_op(None));
        assert_eq!(outcome(&session).report.pages_converted, 1);
        assert!(host.output.starts_with(b"%PDF-1.7"));
        assert!(host.output.ends_with(b"%%EOF\n"));
        assert!(host.max_read <= chunk && host.max_write <= chunk);
    }
}

#[test]
fn hnc8_located_failures_are_explicit() {
    let mut invalid = synthetic_hn();
    put_u32(&mut invalid, 0x15c + 20 + 32, 99);
    let session = run(&mut Memory::new(&invalid), limits(8), convert_op(None));
    let error = failure(&session);
    assert_eq!(error_code(error), 16);
    assert!(session.message().contains("page 1"));
    assert!(session.message().contains("image 1"));
    assert!(matches!(error.context, Context::Hnc8 { .. }));
}

#[test]
fn default_tables_do_not_turn_small_allocation_limits_into_invalid_input() {
    let input = synthetic_hn();
    let mut bounded = limits(8);
    bounded.max_allocation_bytes = 8;
    let session = run(&mut Memory::new(&input), bounded, convert_op(None));
    let error = failure(&session);
    assert!(matches!(error.context, Context::Hnc8 { .. }), "{error}");
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::LimitExceeded { .. },
            ..
        }
    ));
}

#[test]
fn hnb_empty_source_rows_are_not_silently_omitted() {
    let mut input = vec![0; 0xd8 + 20];
    input[..8].copy_from_slice(&[72, 78, 0, 0, 0xc8, 0, 0, 0]);
    put_u32(&mut input, 0x90, 1);
    put_u32(&mut input, 0xd8, 0xd8 + 20);
    let session = run(&mut Memory::new(&input), limits(7), native_operation());
    assert!(matches!(failure(&session).context, Context::Hnc8 { .. }));
    assert!(
        session.message().contains("cannot omit source pages"),
        "{}",
        session.message()
    );
    assert!(session.message().contains("page 1"));
}

#[test]
fn hnc8_inspection_streams_outline_validation_without_decoding() {
    let mut c8 = vec![0; 0x50 + 20];
    c8[0] = 0xc8;
    put_u32(&mut c8, 8, 1);
    let mut hnb = vec![0; 0xd8 + 20];
    hnb[..8].copy_from_slice(&[72, 78, 0, 0, 0xc8, 0, 0, 0]);
    put_u32(&mut hnb, 0x90, 1);
    // A level skip is clamped and an out-of-range destination is skipped;
    // each is one warning, and the count is what would be written.
    let mut clamped = hn_metadata();
    put_u32(&mut clamped, 0x15c + 308 + 304, 4);
    let mut skipped = hn_metadata();
    skipped[0x15c + 308 + 280] = b'9';
    for (input, pages, bookmarks, warnings) in [
        (hn_metadata(), 2, Some(2), 0),
        (clamped, 2, Some(2), 1),
        (skipped, 2, Some(1), 1),
        (synthetic_hn(), 1, Some(0), 0),
        (c8, 1, None, 0),
        (hnb, 1, None, 0),
    ] {
        let mut host = Memory::new(&input).short(1);
        let session = run(&mut host, limits(1), Operation::Inspect { format: None });
        let info = outcome(&session).info.as_ref().unwrap();
        assert_eq!(
            (info.page_count, info.bookmark_count),
            (Some(pages), bookmarks)
        );
        assert_eq!(outcome(&session).outline_warnings, warnings);
        assert!(host.output.is_empty());
        assert_eq!(host.flushes, 0);
        assert_eq!(host.max_read, 1);
    }
}

#[test]
fn c8_inspection_reports_the_application_info_package() {
    let inspect = |input: &[u8]| {
        let session = run(
            &mut Memory::new(input),
            limits(1024),
            Operation::Inspect { format: None },
        );
        let outcome = outcome(&session);
        assert_eq!(outcome.info.as_ref().unwrap().page_count, Some(1));
        outcome.application_info.clone()
    };
    let info = inspect(&c8_with_application_info(0)).unwrap();
    assert_eq!(info.doi.as_deref(), Some("INVENTED:1"));
    assert_eq!(info.url.as_deref(), Some("http://example.invalid/x"));
    assert_eq!(info.note_count, 2);
    // A defective package is ignored, as in conversion.
    assert_eq!(inspect(&c8_with_application_info(1)), None);
}

#[test]
fn hna_inspection_rejects_unreadable_outlines_and_resource_limits() {
    for case in 0..4 {
        let mut input = hn_metadata();
        let mut bounded = limits(1);
        match case {
            0 => bounded.max_bookmarks = 1,
            1 => bounded.max_pages = 1,
            2 => bounded.max_allocation_bytes = 8,
            _ => {
                input.pop();
            }
        }
        let session = run(
            &mut Memory::new(&input),
            bounded,
            Operation::Inspect { format: None },
        );
        assert_eq!(error_code(failure(&session)), 16);
        assert!(session.message().contains("byte"));
    }
}

#[test]
fn native_c8_font_resources_are_read_as_host_resources() {
    let bytes = native_c8();
    let font = include_bytes!("../../../../tests/fonts/geometric.ttf");
    let mut session = Session::default();
    assert_eq!(session.add_font_source(font.len() as u64, 0), 1);
    // An absent alternate Latin role (`u32::MAX`) uses the core fallback.
    assert!(session.set_c8_fonts(0, 0, u32::MAX, 0, 'A' as u32, u32::MAX));
    let mut host = Memory {
        fonts: vec![font],
        ..Memory::new(&bytes).short(3)
    };
    let status = session.run(
        &mut host,
        bytes.len() as u64,
        limits(32),
        native_operation(),
    );
    assert_eq!(status, Status::Done, "{}", session.message());
    assert!(host.max_read <= 32 && host.max_write <= 32);
    assert!(host.output.ends_with(b"%%EOF\n"));
    assert_eq!(
        String::from_utf8_lossy(&host.output)
            .matches("/FontFile2 ")
            .count(),
        1
    );
    // Registration closes once the session has run.
    assert!(!session.set_c8_fonts(0, 0, 0, u32::MAX, 0, u32::MAX));
    assert_eq!(session.add_font_source(10, 0), 0);
    assert!(!session.set_c8_latin_state(3, 0));
    assert!(!session.add_hnb_symbol_glyph(0xa1af, 0x41));
}

#[test]
fn symbol_glyphs_need_the_symbols_role_and_distinct_symbol_codes() {
    let mut session = Session::default();
    assert_eq!(session.add_font_source(100, 0), 1);
    assert!(!session.add_hnb_symbol_glyph(0xa1af, 0x41));
    assert!(session.set_c8_fonts(0, 0, u32::MAX, u32::MAX, 0, u32::MAX));
    assert!(!session.add_hnb_symbol_glyph(0xa1af, 0x41));
    let mut session = Session::default();
    assert_eq!(session.add_font_source(100, 0), 1);
    assert!(session.set_c8_fonts(0, 0, u32::MAX, u32::MAX, 0, 0));
    for (code, glyph) in [
        (0xa3c1, 0x41),
        (0x1_a1af, 0x41),
        (0xa1af, 0x10000),
        (0xa1af, 0xd800),
    ] {
        assert!(!session.add_hnb_symbol_glyph(code, glyph));
    }
    assert!(session.add_hnb_symbol_glyph(0xa1af, 0xe000));
    assert!(session.add_hnb_symbol_glyph(0xa3a7, 0xe000));
    assert!(!session.add_hnb_symbol_glyph(0xa1af, 0xe001));
}

#[test]
fn font_configuration_rejects_invalid_resources_and_roles() {
    let mut session = Session::default();
    assert_eq!(session.add_font_source(0, 0), 0);
    assert!(!session.set_c8_fonts(0, 0, 0, u32::MAX, 0, u32::MAX));
    for id in 1..=8 {
        assert_eq!(session.add_font_source(100, 0), id);
    }
    assert_eq!(session.add_font_source(100, 0), 0);
    for (decoration, alias) in [(8, 65), (0, 0xd800), (0, 0x10000)] {
        assert!(!session.set_c8_fonts(0, 0, 0, decoration, alias, u32::MAX));
    }
    assert!(!session.set_c8_fonts(0, 1, 2, u32::MAX, 0, 8));
    assert!(!session.set_c8_fonts(0, 1, 8, u32::MAX, 0, u32::MAX));
    assert!(!session.set_c8_latin_state(3, 5));
    assert!(session.set_c8_fonts(0, 1, 2, 3, 65, 4));
    assert!(!session.set_c8_latin_state(3, 8));
    assert!(session.set_c8_latin_state(3, 5));
    assert!(!session.set_c8_latin_state(3, 5));
    assert!(!session.set_c8_latin_state(99, 6));
    for (state, index) in [(28, 6), (31, 7)] {
        assert!(!session.set_c8_latin_state(state, 8));
        assert!(session.set_c8_latin_state(state, index));
        assert!(!session.set_c8_latin_state(state, index));
    }
    assert!(!session.set_c8_fonts(0, 1, 2, u32::MAX, 0, u32::MAX));
    assert_eq!(session.add_font_source(100, 0), 0);
}

#[test]
fn font_limits_incomplete_roles_and_wrong_operations_are_explicit_errors() {
    let bytes = native_c8();
    // A font larger than the input limit is refused before it is read.
    let mut session = Session::default();
    assert_eq!(session.add_font_source(u64::MAX, 0), 1);
    assert!(session.set_c8_fonts(0, 0, u32::MAX, u32::MAX, 0, u32::MAX));
    let mut host = Memory::new(&bytes);
    session.run(
        &mut host,
        bytes.len() as u64,
        limits(32),
        native_operation(),
    );
    assert!(matches!(
        failure(&session),
        Error {
            kind: ErrorKind::LimitExceeded { .. },
            ..
        }
    ));
    // Fonts without roles, fonts for a document without native text, and
    // fonts for an inspection are configuration errors.
    let pdf = fixture("valid_out_of_order_objects.pdf");
    for (input, operation) in [
        (&bytes, native_operation()),
        (&pdf, native_operation()),
        (&bytes, Operation::Inspect { format: None }),
    ] {
        let mut session = Session::default();
        assert_eq!(session.add_font_source(100, 0), 1);
        if input == &pdf {
            assert!(session.set_c8_fonts(0, 0, u32::MAX, u32::MAX, 0, u32::MAX));
        }
        let mut host = Memory::new(input).short(3);
        session.run(&mut host, input.len() as u64, limits(32), operation);
        assert!(matches!(
            session.result(),
            Some(Err(Error {
                kind: ErrorKind::Malformed,
                ..
            }))
        ));
        assert!(host.output.is_empty());
    }
}

#[test]
fn ttkn_response_registration_is_explicit_consumed_and_conversion_only() {
    let input = include_bytes!("../../../caj2pdf-core/tests/fixtures/ttkn/authored.pdf");
    let response =
        include_bytes!("../../../caj2pdf-core/tests/fixtures/ttkn/response.txt").trim_ascii();
    let mut session = Session::default();
    assert!(session.set_ttkn_response(response));
    assert!(!session.set_ttkn_response(b"bad"));
    assert!(session.response.is_none());
    assert!(session.set_ttkn_response(response));
    let mut host = Memory::new(input).short(7);
    assert_eq!(
        session.run(
            &mut host,
            input.len() as u64,
            Limits::default(),
            convert_op(None)
        ),
        Status::Done
    );
    assert!(session.response.is_none());
    assert!(!session.set_ttkn_response(response));
    assert_eq!(
        session
            .result()
            .unwrap()
            .as_ref()
            .unwrap()
            .report
            .pages_converted,
        1
    );
    let mut session = Session::default();
    assert!(session.set_ttkn_response(response));
    let mut host = Memory::new(input);
    assert_eq!(
        session.run(
            &mut host,
            input.len() as u64,
            Limits::default(),
            Operation::Inspect { format: None }
        ),
        Status::Failed
    );
    assert!(host.output.is_empty());
    assert!(
        !session
            .message()
            .contains(std::str::from_utf8(response).unwrap())
    );
}
