// SPDX-License-Identifier: MIT

use caj2pdf_core::{
    Cancellation, Context, ConversionOptions, DEFAULT_IO_CHUNK, Error, ErrorKind, Limits,
    MAX_IO_CHUNK, NeverCancel, RangedSource, native::SeekableSource, read_exact_at,
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
        Error {
            kind: ErrorKind::Truncated {
                expected: 4,
                available: 2,
                ..
            },
            offset: Some(4),
            ..
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
        Error {
            kind: ErrorKind::Truncated {
                expected: 4,
                available: 2,
                ..
            },
            offset: Some(4),
            ..
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
        Err(Error {
            kind: ErrorKind::Malformed,
            ..
        })
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
        Err(Error {
            kind: ErrorKind::Malformed,
            ..
        })
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
        Err(Error {
            kind: ErrorKind::Malformed,
            ..
        })
    ));
}

#[test]
fn configured_limits_reject_oversized_requests_before_io() {
    let mut source = Source::new(b"abc", 3);
    let mut destination = [0; 2];
    let mut limits = Limits::default();
    assert_eq!(limits.io_chunk_bytes, DEFAULT_IO_CHUNK);
    limits.io_chunk_bytes = 0;
    assert!(matches!(
        limits.validate(),
        Err(Error {
            kind: ErrorKind::Malformed,
            ..
        })
    ));
    limits.io_chunk_bytes = MAX_IO_CHUNK + 1;
    assert!(matches!(
        limits.validate(),
        Err(Error {
            kind: ErrorKind::LimitExceeded { .. },
            ..
        })
    ));
    limits.io_chunk_bytes = 2;
    limits.max_allocation_bytes = 1;
    assert!(matches!(
        limits.validate(),
        Err(Error {
            kind: ErrorKind::LimitExceeded { .. },
            ..
        })
    ));
    limits.max_allocation_bytes = 2;
    limits.max_input_bytes = 1;
    assert!(matches!(
        read_exact_at(&mut source, 0, &mut destination, &limits, &NeverCancel),
        Err(Error {
            kind: ErrorKind::LimitExceeded { .. },
            ..
        })
    ));
    limits.max_input_bytes = 3;
    limits.io_chunk_bytes = 1;
    assert!(matches!(
        read_exact_at(&mut source, 0, &mut destination, &limits, &NeverCancel),
        Err(Error {
            kind: ErrorKind::LimitExceeded { .. },
            ..
        })
    ));
    assert!(source.reads.is_empty());

    limits.max_pages = 1;
    limits.max_bookmarks = 1;
    assert!(limits.check_pages(1).is_ok());
    assert!(limits.check_bookmarks(1).is_ok());
    assert!(matches!(
        limits.check_pages(2),
        Err(Error {
            kind: ErrorKind::LimitExceeded { .. },
            ..
        })
    ));
    assert!(matches!(
        limits.check_bookmarks(2),
        Err(Error {
            kind: ErrorKind::LimitExceeded { .. },
            ..
        })
    ));
}

#[test]
fn cancellation_stops_before_and_after_a_read() {
    let state = Rc::new(Cell::new(true));
    let flag = Flag(state.clone());
    let mut source = Source::new(b"abc", 1);
    let mut buffer = [0; 2];
    assert!(matches!(
        read_exact_at(&mut source, 0, &mut buffer, &Limits::default(), &flag),
        Err(Error {
            kind: ErrorKind::Cancelled,
            ..
        })
    ));
    assert!(source.reads.is_empty());

    state.set(false);
    source.cancel_after_read = Some(state.clone());
    assert!(matches!(
        read_exact_at(&mut source, 0, &mut buffer, &Limits::default(), &flag),
        Err(Error {
            kind: ErrorKind::Cancelled,
            ..
        })
    ));
    assert_eq!(source.reads.len(), 1);
}

#[test]
fn native_adapters_borrow_handles_and_restore_input_position() {
    let mut input = Cursor::new(b"abcdef".to_vec());
    input.set_position(4);
    let mut source = SeekableSource::new(&mut input).unwrap();
    assert_eq!(source.size(), 6);
    let mut buffer = [0; 4];
    read_exact_at(
        &mut source,
        1,
        &mut buffer,
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap();
    assert_eq!(&buffer, b"bcde");
    assert_eq!(source.into_inner().stream_position().unwrap(), 5);

    input.set_position(2);
    let source = SeekableSource::new(&mut input).unwrap();
    assert_eq!(source.size(), 6);
    assert_eq!(input.stream_position().unwrap(), 2);
}

#[test]
fn byte_slices_are_sources() {
    let mut source: &[u8] = b"abcdef";
    assert_eq!(source.size(), 6);
    let mut buffer = [0; 3];
    read_exact_at(
        &mut source,
        2,
        &mut buffer,
        &Limits::default(),
        &NeverCancel,
    )
    .unwrap();
    assert_eq!(&buffer, b"cde");
    assert_eq!(source.read_at(6, &mut buffer).unwrap(), 0);
    assert_eq!(source.read_at(4, &mut buffer).unwrap(), 2);
    assert!(matches!(
        source.read_at(7, &mut buffer),
        Err(Error {
            kind: ErrorKind::Malformed,
            ..
        })
    ));
}

#[test]
fn native_adapters_reject_oversized_direct_calls() {
    let mut input = Cursor::new(vec![0; MAX_IO_CHUNK + 1]);
    let mut source = SeekableSource::new(&mut input).unwrap();
    let mut destination = vec![0; MAX_IO_CHUNK + 1];
    assert!(matches!(
        source.read_at(0, &mut destination),
        Err(Error {
            kind: ErrorKind::LimitExceeded { .. },
            ..
        })
    ));
    assert!(matches!(
        source.read_at(u64::MAX, &mut []),
        Err(Error {
            kind: ErrorKind::Malformed,
            ..
        })
    ));
}

#[test]
fn error_types_preserve_context_and_sources() {
    let error = Error::from(io::Error::other("disk failed"));
    assert!(error.to_string().contains("disk failed"));
    assert!(std::error::Error::source(&error).is_some());
    assert!(std::error::Error::source(&Error::cancelled()).is_none());
    assert!(
        Error::from(ErrorKind::UnsupportedFormat)
            .to_string()
            .contains("unsupported")
    );
    assert_eq!(Error::cancelled().to_string(), "operation cancelled");
    assert_eq!(
        Error::truncated(4, 3, 2).to_string(),
        "truncated input at byte 4: expected 3 bytes, available 2"
    );
    assert!(Error::invalid("x").to_string().contains('x'));
    assert!(
        Error::limit("bytes", 1, 2)
            .to_string()
            .contains("attempted 2")
    );
    assert!(ConversionOptions::default().include_bookmarks);
}

#[test]
fn located_format_errors_name_their_offset_record_and_object() {
    let cases = [
        (
            Error::malformed(0x14, "CAJ page table extends beyond source").in_caj(None),
            "malformed CAJ at byte 20: CAJ page table extends beyond source",
        ),
        (
            Error::malformed(0x114, "empty CAJ TOC title").in_caj(Some(3)),
            "malformed CAJ at byte 276, record 3: empty CAJ TOC title",
        ),
        (
            Error::limit("pages", 2, 5).at(16).in_caj(None),
            "CAJ pages limit exceeded at byte 16: maximum 2, attempted 5",
        ),
        (
            Error::limit("CAJ title bytes", 10, 12)
                .at(584)
                .in_caj(Some(2)),
            "CAJ CAJ title bytes limit exceeded at byte 584, record 2: maximum 10, attempted 12",
        ),
        (
            Error::malformed(0x28, "KDH version field is invalid").within(Context::Kdh),
            "malformed KDH at byte 40: KDH version field is invalid",
        ),
        (
            Error::from(ErrorKind::Encrypted)
                .at(9)
                .because("encrypted input")
                .within(Context::Pdf {
                    object: None,
                    repair: false,
                }),
            "encrypted PDF at byte 9: encrypted input",
        ),
        (
            Error::from(ErrorKind::Malformed)
                .at(70)
                .because("two candidates")
                .within(Context::Pdf {
                    object: Some((12, 1)),
                    repair: true,
                }),
            "ambiguous repair PDF at byte 70, object 12 1: two candidates",
        ),
        (
            Error::limit("output bytes", 128, 129).at(3).in_pdf(None),
            "PDF output bytes limit exceeded at byte 3: maximum 128, attempted 129",
        ),
        (
            Error::limit("stream bytes", 4, 8)
                .at(44)
                .in_pdf(Some((7, 0))),
            "PDF stream bytes limit exceeded at byte 44, object 7 0: maximum 4, attempted 8",
        ),
    ];
    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
        assert!(std::error::Error::source(&error).is_none());
    }
}
