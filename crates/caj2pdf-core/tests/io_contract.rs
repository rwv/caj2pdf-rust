// SPDX-License-Identifier: MIT

use caj2pdf_core::{
    Cancellation, ConversionOptions, DEFAULT_IO_CHUNK, Error, Limits, MAX_IO_CHUNK, NeverCancel,
    PdfErrorKind, RangedSource, SequentialSink, native::SeekableSource, native::WriteSink,
    read_exact_at, write_all,
};
use std::{
    cell::Cell,
    io::{self, Cursor, Seek},
    rc::Rc,
};

struct Flag(Rc<Cell<bool>>);

impl Cancellation for Flag {
    fn is_cancelled(&self) -> bool {
        self.0.get()
    }
}

struct Source {
    data: Vec<u8>,
    advertised_size: u64,
    max_read: usize,
    reads: Vec<(u64, usize)>,
    cancel_after_read: Option<Rc<Cell<bool>>>,
    overreport: bool,
}

impl Source {
    fn new(data: &[u8], max_read: usize) -> Self {
        Self {
            data: data.to_vec(),
            advertised_size: data.len() as u64,
            max_read,
            reads: Vec::new(),
            cancel_after_read: None,
            overreport: false,
        }
    }
}

impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.advertised_size
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> caj2pdf_core::Result<usize> {
        self.reads.push((offset, destination.len()));
        if self.overreport {
            return Ok(destination.len() + 1);
        }
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let available = self.data.len().saturating_sub(start);
        let count = available.min(destination.len()).min(self.max_read);
        if count > 0 {
            destination[..count].copy_from_slice(&self.data[start..start + count]);
        }
        if let Some(flag) = &self.cancel_after_read {
            flag.set(true);
        }
        Ok(count)
    }
}

#[derive(Default)]
struct Sink {
    bytes: Vec<u8>,
    max_write: usize,
    writes: Vec<usize>,
    fail_after: Option<usize>,
    cancel_after_write: Option<Rc<Cell<bool>>>,
    overreport: bool,
}

impl SequentialSink for Sink {
    fn write(&mut self, bytes: &[u8]) -> caj2pdf_core::Result<usize> {
        self.writes.push(bytes.len());
        if self
            .fail_after
            .is_some_and(|limit| self.bytes.len() >= limit)
        {
            return Err(Error::Io(io::Error::other("sink failure")));
        }
        if self.overreport {
            return Ok(bytes.len() + 1);
        }
        let count = bytes.len().min(self.max_write);
        self.bytes.extend_from_slice(&bytes[..count]);
        if let Some(flag) = &self.cancel_after_write {
            flag.set(true);
        }
        Ok(count)
    }

    fn flush(&mut self) -> caj2pdf_core::Result<()> {
        Ok(())
    }
}

#[test]
fn read_exact_distinguishes_short_read_from_truncation() {
    let mut source = Source::new(b"abcdef", 2);
    let mut buffer = [0; 5];
    read_exact_at(
        &mut source,
        1,
        &mut buffer,
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap();
    assert_eq!(&buffer, b"bcdef");
    assert_eq!(source.reads.len(), 3);

    let mut buffer = [0; 4];
    let error = read_exact_at(
        &mut source,
        4,
        &mut buffer,
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        Error::TruncatedInput {
            offset: 4,
            expected: 4,
            available: 2
        }
    ));

    source.advertised_size = 10;
    let error = read_exact_at(
        &mut source,
        4,
        &mut buffer,
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        Error::TruncatedInput {
            offset: 4,
            expected: 4,
            available: 2
        }
    ));
}

#[test]
fn malformed_ranges_and_source_reports_return_errors() {
    let mut source = Source::new(b"abc", 3);
    let mut buffer = [0; 2];
    assert!(matches!(
        read_exact_at(
            &mut source,
            4,
            &mut buffer,
            &Limits::default(),
            &NeverCancel
        ),
        Err(Error::InvalidInput { .. })
    ));
    source.advertised_size = u64::MAX;
    assert!(matches!(
        read_exact_at(
            &mut source,
            u64::MAX,
            &mut buffer,
            &Limits {
                max_input_bytes: u64::MAX,
                ..Limits::default()
            },
            &NeverCancel
        ),
        Err(Error::InvalidInput { .. })
    ));
    source.advertised_size = 3;
    source.overreport = true;
    assert!(matches!(
        read_exact_at(
            &mut source,
            0,
            &mut buffer,
            &Limits::default(),
            &NeverCancel
        ),
        Err(Error::InvalidInput { .. })
    ));
}

#[test]
fn configured_limits_reject_oversized_requests_before_io() {
    let mut source = Source::new(b"abc", 3);
    let mut destination = [0; 2];
    let mut limits = Limits::default();
    assert_eq!(limits.io_chunk_bytes, DEFAULT_IO_CHUNK);
    limits.io_chunk_bytes = 0;
    assert!(matches!(limits.validate(), Err(Error::InvalidInput { .. })));
    limits.io_chunk_bytes = MAX_IO_CHUNK + 1;
    assert!(matches!(
        limits.validate(),
        Err(Error::LimitExceeded { .. })
    ));
    limits.io_chunk_bytes = 2;
    limits.max_allocation_bytes = 1;
    assert!(matches!(
        limits.validate(),
        Err(Error::LimitExceeded { .. })
    ));
    limits.max_allocation_bytes = 2;
    limits.max_input_bytes = 1;
    assert!(matches!(
        read_exact_at(&mut source, 0, &mut destination, &limits, &NeverCancel),
        Err(Error::LimitExceeded { .. })
    ));
    limits.max_input_bytes = 3;
    limits.io_chunk_bytes = 1;
    assert!(matches!(
        read_exact_at(&mut source, 0, &mut destination, &limits, &NeverCancel),
        Err(Error::LimitExceeded { .. })
    ));
    assert!(source.reads.is_empty());

    limits.max_pages = 1;
    limits.max_bookmarks = 1;
    assert!(limits.check_pages(1).is_ok());
    assert!(limits.check_bookmarks(1).is_ok());
    assert!(matches!(
        limits.check_pages(2),
        Err(Error::LimitExceeded { .. })
    ));
    assert!(matches!(
        limits.check_bookmarks(2),
        Err(Error::LimitExceeded { .. })
    ));
}

#[test]
fn cancellation_stops_before_and_after_awaited_io() {
    let state = Rc::new(Cell::new(true));
    let flag = Flag(state.clone());
    let mut source = Source::new(b"abc", 1);
    let mut buffer = [0; 2];
    assert!(matches!(
        read_exact_at(&mut source, 0, &mut buffer, &Limits::default(), &flag),
        Err(Error::Cancelled)
    ));
    assert!(source.reads.is_empty());

    state.set(false);
    source.cancel_after_read = Some(state.clone());
    assert!(matches!(
        read_exact_at(&mut source, 0, &mut buffer, &Limits::default(), &flag),
        Err(Error::Cancelled)
    ));
    assert_eq!(source.reads.len(), 1);

    state.set(false);
    let mut sink = Sink {
        max_write: 1,
        cancel_after_write: Some(state.clone()),
        ..Sink::default()
    };
    let mut count = 0;
    assert!(matches!(
        write_all(&mut sink, b"abc", &mut count, &Limits::default(), &flag),
        Err(Error::Cancelled)
    ));
    assert_eq!(count, 1);
    assert_eq!(sink.bytes, b"a");
    assert_eq!(sink.writes.len(), 1);
}

#[test]
fn zero_overreported_and_failing_writes_are_not_success() {
    let mut sink = Sink::default();
    let mut count = 0;
    let error = write_all(
        &mut sink,
        b"x",
        &mut count,
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap_err();
    assert!(matches!(error, Error::Io(ref e) if e.kind() == io::ErrorKind::WriteZero));
    assert_eq!(count, 0);

    sink.max_write = 1;
    sink.overreport = true;
    assert!(matches!(
        write_all(
            &mut sink,
            b"x",
            &mut count,
            &Limits::default(),
            &NeverCancel
        ),
        Err(Error::InvalidInput { .. })
    ));

    sink.overreport = false;
    sink.fail_after = Some(1);
    assert!(matches!(
        write_all(
            &mut sink,
            b"xyz",
            &mut count,
            &Limits::default(),
            &NeverCancel
        ),
        Err(Error::Io(_))
    ));
    assert_eq!(sink.bytes, b"x");
    assert_eq!(count, 1);
}

#[test]
fn output_budget_is_cumulative_and_counts_use_checked_arithmetic() {
    let mut sink = Sink {
        max_write: 4,
        ..Sink::default()
    };
    let limits = Limits {
        max_output_bytes: 5,
        ..Limits::default()
    };
    let mut count = 4;
    assert!(matches!(
        write_all(&mut sink, b"xy", &mut count, &limits, &NeverCancel),
        Err(Error::LimitExceeded { .. })
    ));
    assert_eq!(count, 4);
    assert!(sink.writes.is_empty());

    count = u64::MAX;
    assert!(matches!(
        write_all(
            &mut sink,
            b"x",
            &mut count,
            &Limits {
                max_output_bytes: u64::MAX,
                ..Limits::default()
            },
            &NeverCancel
        ),
        Err(Error::InvalidInput { .. })
    ));
}

#[test]
fn native_adapters_borrow_handles_and_restore_input_position() {
    let mut input = Cursor::new(b"abcdef".to_vec());
    input.set_position(4);
    let mut output = Vec::new();
    {
        let mut source = SeekableSource::new(&mut input).unwrap();
        assert_eq!(source.size(), 6);
        let mut sink = WriteSink::new(&mut output);
        let mut buffer = [0; 4];
        read_exact_at(
            &mut source,
            1,
            &mut buffer,
            &Limits::default(),
            &NeverCancel,
        )
        .unwrap();
        let mut written = 0;
        write_all(
            &mut sink,
            &buffer,
            &mut written,
            &Limits::default(),
            &NeverCancel,
        )
        .unwrap();
        assert_eq!(written, 4);
        assert_eq!(source.into_inner().stream_position().unwrap(), 5);
        assert_eq!(sink.into_inner().as_slice(), b"bcde");
    }
    assert_eq!(output, b"bcde");

    input.set_position(2);
    let source = SeekableSource::new(&mut input).unwrap();
    assert_eq!(source.size(), 6);
    assert_eq!(input.stream_position().unwrap(), 2);
}

#[test]
fn native_adapters_reject_oversized_direct_calls() {
    let mut input = Cursor::new(vec![0; MAX_IO_CHUNK + 1]);
    let mut source = SeekableSource::new(&mut input).unwrap();
    let mut destination = vec![0; MAX_IO_CHUNK + 1];
    assert!(matches!(
        source.read_at(0, &mut destination),
        Err(Error::LimitExceeded { .. })
    ));
    assert!(matches!(
        source.read_at(u64::MAX, &mut []),
        Err(Error::InvalidInput { .. })
    ));
    let mut output = Vec::new();
    let mut sink = WriteSink::new(&mut output);
    assert!(matches!(
        sink.write(&destination),
        Err(Error::LimitExceeded { .. })
    ));
    assert!(output.is_empty());
}

#[test]
fn error_types_preserve_context_and_sources() {
    let error = Error::from(io::Error::other("disk failed"));
    assert!(error.to_string().contains("disk failed"));
    assert!(std::error::Error::source(&error).is_some());
    assert!(std::error::Error::source(&Error::Cancelled).is_none());
    assert!(Error::UnsupportedFormat.to_string().contains("unsupported"));
    assert_eq!(Error::Cancelled.to_string(), "operation cancelled");
    assert_eq!(
        Error::TruncatedInput {
            offset: 4,
            expected: 3,
            available: 2
        }
        .to_string(),
        "truncated input at offset 4: needed 3 bytes, got 2"
    );
    assert!(
        Error::InvalidInput { reason: "x" }
            .to_string()
            .contains('x')
    );
    assert!(
        Error::LimitExceeded {
            resource: "bytes",
            limit: 1,
            attempted: 2
        }
        .to_string()
        .contains("attempted 2")
    );
    assert!(ConversionOptions::default().include_bookmarks);
}

#[test]
fn located_format_errors_name_their_offset_record_and_object() {
    let cases = [
        (
            Error::Caj {
                offset: 0x14,
                record: None,
                reason: "CAJ page table extends beyond source",
            },
            "malformed CAJ at byte 20: CAJ page table extends beyond source",
        ),
        (
            Error::Caj {
                offset: 0x114,
                record: Some(3),
                reason: "empty CAJ TOC title",
            },
            "malformed CAJ at byte 276, record 3: empty CAJ TOC title",
        ),
        (
            Error::CajLimitExceeded {
                offset: 16,
                record: None,
                resource: "pages",
                limit: 2,
                attempted: 5,
            },
            "CAJ pages limit exceeded at byte 16: maximum 2, attempted 5",
        ),
        (
            Error::CajLimitExceeded {
                offset: 584,
                record: Some(2),
                resource: "CAJ title bytes",
                limit: 10,
                attempted: 12,
            },
            "CAJ CAJ title bytes limit exceeded at byte 584, record 2: maximum 10, attempted 12",
        ),
        (
            Error::Kdh {
                offset: 0x28,
                reason: "KDH version field is invalid",
            },
            "malformed KDH at byte 40: KDH version field is invalid",
        ),
        (
            Error::Pdf {
                offset: 9,
                object: None,
                kind: PdfErrorKind::Encrypted,
                reason: "encrypted input",
            },
            "encrypted PDF at byte 9: encrypted input",
        ),
        (
            Error::Pdf {
                offset: 70,
                object: Some((12, 1)),
                kind: PdfErrorKind::AmbiguousRepair,
                reason: "two candidates",
            },
            "ambiguous repair PDF at byte 70, object 12 1: two candidates",
        ),
        (
            Error::PdfLimitExceeded {
                offset: 3,
                object: None,
                resource: "output bytes",
                limit: 128,
                attempted: 129,
            },
            "PDF output bytes limit exceeded at byte 3: maximum 128, attempted 129",
        ),
        (
            Error::PdfLimitExceeded {
                offset: 44,
                object: Some((7, 0)),
                resource: "stream bytes",
                limit: 4,
                attempted: 8,
            },
            "PDF stream bytes limit exceeded at byte 44, object 7 0: maximum 4, attempted 8",
        ),
    ];
    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
        assert!(std::error::Error::source(&error).is_none());
    }
    assert_eq!(PdfErrorKind::Malformed.to_string(), "malformed");
    assert_eq!(
        PdfErrorKind::UnsupportedFeature.to_string(),
        "unsupported feature"
    );
}
