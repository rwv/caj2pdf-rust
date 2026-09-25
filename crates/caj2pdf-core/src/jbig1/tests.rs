// SPDX-License-Identifier: MIT

use super::*;
use crate::{NeverCancel, native::SeekableSource};
use std::{
    cell::Cell,
    future::Future,
    io::Cursor,
    pin::pin,
    rc::Rc,
    task::{Context, Poll, Waker},
};

fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut context) {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("in-memory test unexpectedly yielded"),
    }
}

fn image(width: u32, height: u32, coded: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; DIB_BYTES as usize];
    bytes[0..4].copy_from_slice(&40_u32.to_le_bytes());
    bytes[4..8].copy_from_slice(&width.to_le_bytes());
    bytes[8..12].copy_from_slice(&height.to_le_bytes());
    bytes[12..14].copy_from_slice(&1_u16.to_le_bytes());
    bytes[14..16].copy_from_slice(&1_u16.to_le_bytes());
    bytes[40..43].fill(0xff);
    bytes.extend_from_slice(coded);
    bytes
}

fn table(qe: u16) -> QmTable {
    // Invented, stationary state machine. At qe=1 and a zero code register,
    // the short test trace always takes the zero MPS branch.
    QmTable::new(vec![
        QmState {
            qe,
            next_lps: 0,
            next_mps: 0,
            switch_mps: false
        };
        QM_STATE_COUNT
    ])
    .unwrap()
}

fn arithmetic_budget() -> ArithmeticBudget {
    ArithmeticBudget {
        max_symbols: 1_000,
        max_work: 20_000,
    }
}

#[derive(Default)]
struct BytesSink {
    bytes: Vec<u8>,
    max_write: usize,
    fail: bool,
}

impl SequentialSink for BytesSink {
    async fn write(&mut self, bytes: &[u8]) -> crate::Result<usize> {
        if self.fail {
            return Err(Error::InvalidInput {
                reason: "synthetic sink failure",
            });
        }
        let n = bytes.len().min(self.max_write.max(1));
        self.bytes.extend_from_slice(&bytes[..n]);
        Ok(n)
    }
    async fn flush(&mut self) -> crate::Result<()> {
        Ok(())
    }
}

#[test]
fn ten_context_positions_and_edges_have_fixed_indices() {
    let width = 11;
    let x = 5;
    // The ten Figure-11 neighbors occupy bits 9..0 in documented order.
    let positions = [
        (2, 3),
        (2, 4),
        (1, 3),
        (1, 4),
        (1, 5),
        (1, 6),
        (1, 7),
        (0, 4),
        (0, 5),
        (0, 6),
    ];
    for (bit, (row, column)) in positions.into_iter().enumerate() {
        let mut rows = [vec![0; 2], vec![0; 2], vec![0; 2]];
        rows[row][column / 8] = 0x80 >> (column % 8);
        assert_eq!(
            three_line_context(&rows[0], &rows[1], &rows[2], width, x),
            1 << (9 - bit),
            "neighbor {bit}"
        );
    }
    // At x=0, out-of-bounds left neighbors vanish; previous-row x+2
    // occupies bit 3 in this mapping.
    let mut previous = [0_u8; 2];
    previous[0] = 0x20;
    assert_eq!(
        three_line_context(&[0; 2], &previous, &[0; 2], width, 0),
        1 << 3
    );
}

#[test]
fn zero_rows_at_boundary_widths_are_stride_padded_and_sequential() {
    for width in [7, 8, 9, 31, 32, 33] {
        let bytes = image(width, 3, &[0, 0, 0]);
        let limits = Limits::default();
        let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
        let mut contexts = ContextBank::new(CONTEXT_COUNT, &limits).unwrap();
        let mut sink = BytesSink {
            max_write: 1,
            ..BytesSink::default()
        };
        let table = table(1);
        let mut decoder = ready(Type0Decoder::new(
            &mut source,
            Type0Span {
                record_type: 0,
                offset: 0,
                length: bytes.len() as u64,
            },
            &table,
            &mut contexts,
            &mut sink,
            &limits,
            &NeverCancel,
            arithmetic_budget(),
            Type0Budget::default(),
        ))
        .unwrap();
        let stride = (width.div_ceil(32) * 4) as usize;
        assert_eq!(decoder.progress().info.dib_stride, stride);
        for row in 1..=3 {
            assert!(ready(decoder.decode_next_row()).unwrap());
            assert_eq!(decoder.progress().rows_written, row);
        }
        assert!(!ready(decoder.decode_next_row()).unwrap());
        let report = ready(decoder.finish()).unwrap();
        assert_eq!(
            report.progress.arithmetic.symbols_decoded,
            3 * (u64::from(width) + 1)
        );
        assert_eq!(report.progress.output_bytes_written, (stride * 3) as u64);
        assert_eq!(sink.bytes, vec![0; stride * 3]);
    }
}

#[test]
fn hand_derived_first_lps_produces_one_black_pixel() {
    // With A=0x10000, Qe=0x4000, initial C high word 0x9000:
    // control chooses MPS zero since 0x9000 < A-Qe=0xc000.
    // The pixel then narrows A to 0x8000, so C is in the LPS
    // interval and the decoded pixel is one at the row MSB.
    let bytes = image(1, 1, &[0x90, 0x00, 0x00]);
    let limits = Limits::default();
    let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
    let mut contexts = ContextBank::new(CONTEXT_COUNT, &limits).unwrap();
    let mut sink = BytesSink::default();
    let table = table(0x4000);
    let mut decoder = ready(Type0Decoder::new(
        &mut source,
        Type0Span {
            record_type: 0,
            offset: 0,
            length: bytes.len() as u64,
        },
        &table,
        &mut contexts,
        &mut sink,
        &limits,
        &NeverCancel,
        arithmetic_budget(),
        Type0Budget::default(),
    ))
    .unwrap();
    assert!(ready(decoder.decode_next_row()).unwrap());
    let report = ready(decoder.finish()).unwrap();
    assert_eq!(report.progress.arithmetic.symbols_decoded, 2);
    assert_eq!(sink.bytes, [0x80, 0, 0, 0]);
}

#[test]
fn black_row_is_copied_without_decoding_its_pixels() {
    // Qe=0x4000 and C high=0xa000: first control is zero, its
    // pixel is one, then C high is 0x4000 after renormalization.
    // Second control is one, copying the preceding packed row.
    let bytes = image(1, 2, &[0xa0, 0, 0]);
    let limits = Limits::default();
    let table = table(0x4000);
    let mut contexts = ContextBank::new(CONTEXT_COUNT, &limits).unwrap();
    let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
    let mut sink = BytesSink::default();
    let mut decoder = ready(Type0Decoder::new(
        &mut source,
        Type0Span {
            record_type: 0,
            offset: 0,
            length: bytes.len() as u64,
        },
        &table,
        &mut contexts,
        &mut sink,
        &limits,
        &NeverCancel,
        arithmetic_budget(),
        Type0Budget::default(),
    ))
    .unwrap();
    assert!(ready(decoder.decode_next_row()).unwrap());
    assert!(ready(decoder.decode_next_row()).unwrap());
    let report = ready(decoder.finish()).unwrap();
    assert_eq!(report.progress.arithmetic.symbols_decoded, 3);
    assert_eq!(sink.bytes, [0x80, 0, 0, 0, 0x80, 0, 0, 0]);
}

#[test]
fn image_contexts_reset_and_first_row_copy_is_blank() {
    // A=0x10000, Qe=0x7fff, C high word 0xff00 >= 0x8001;
    // first control is LPS one, copying the zero background row.
    let bytes = image(1, 1, &[0xff, 0x00, 0x00]);
    let table = table(0x7fff);
    let limits = Limits::default();
    let mut contexts = ContextBank::new(CONTEXT_COUNT, &limits).unwrap();
    contexts
        .set(
            CONTROL_CONTEXT,
            crate::qm::ContextState {
                state_index: 0,
                mps: true,
            },
        )
        .unwrap();
    for _ in 0..2 {
        let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
        let mut sink = BytesSink::default();
        let mut decoder = ready(Type0Decoder::new(
            &mut source,
            Type0Span {
                record_type: 0,
                offset: 0,
                length: bytes.len() as u64,
            },
            &table,
            &mut contexts,
            &mut sink,
            &limits,
            &NeverCancel,
            arithmetic_budget(),
            Type0Budget::default(),
        ))
        .unwrap();
        assert!(ready(decoder.decode_next_row()).unwrap());
        let report = ready(decoder.finish()).unwrap();
        assert_eq!(report.progress.arithmetic.symbols_decoded, 1);
        assert_eq!(sink.bytes, [0, 0, 0, 0]);
    }
}

#[test]
fn malformed_wrapper_and_preallocation_limits_fail_before_decoding() {
    let limits = Limits::default();
    let budget = Type0Budget::default();
    let base = image(9, 1, &[0, 0, 0]);
    let info = checked_info(
        (&base[..48]).try_into().unwrap(),
        Type0Span {
            record_type: 0,
            offset: 17,
            length: base.len() as u64,
        },
        &limits,
        arithmetic_budget(),
        budget,
    )
    .unwrap();
    assert_eq!((info.dib_stride, info.visible_bytes), (4, 2));
    let mut cases = Vec::new();
    let mut wrong = base.clone();
    wrong[0] = 39;
    cases.push(wrong);
    let mut wrong = base.clone();
    wrong[4..8].copy_from_slice(&0_u32.to_le_bytes());
    cases.push(wrong);
    let mut wrong = base.clone();
    wrong[12] = 2;
    cases.push(wrong);
    let mut wrong = base.clone();
    wrong[14] = 8;
    cases.push(wrong);
    let mut wrong = base.clone();
    wrong[16] = 1;
    cases.push(wrong);
    let mut wrong = base.clone();
    wrong[32] = 3;
    cases.push(wrong);
    let mut wrong = base.clone();
    wrong[40] = 0;
    cases.push(wrong);
    let mut wrong = base.clone();
    wrong[20] = 5;
    cases.push(wrong);
    for bytes in cases {
        assert!(
            checked_info(
                (&bytes[..48]).try_into().unwrap(),
                Type0Span {
                    record_type: 0,
                    offset: 0,
                    length: bytes.len() as u64
                },
                &limits,
                arithmetic_budget(),
                budget
            )
            .is_err()
        );
    }
    let tiny = Limits {
        max_allocation_bytes: 100,
        ..limits
    };
    assert!(matches!(
        checked_info(
            (&base[..48]).try_into().unwrap(),
            Type0Span {
                record_type: 0,
                offset: 0,
                length: base.len() as u64
            },
            &tiny,
            arithmetic_budget(),
            budget
        )
        .unwrap_err()
        .kind,
        Type0ErrorKind::LimitExceeded {
            resource: "working allocation bytes",
            ..
        }
    ));
    let tiny = Limits {
        max_output_bytes: 3,
        ..limits
    };
    assert!(matches!(
        checked_info(
            (&base[..48]).try_into().unwrap(),
            Type0Span {
                record_type: 0,
                offset: 0,
                length: base.len() as u64
            },
            &tiny,
            arithmetic_budget(),
            budget
        )
        .unwrap_err()
        .kind,
        Type0ErrorKind::LimitExceeded {
            resource: "output bytes",
            ..
        }
    ));
}

#[test]
fn non_type_zero_outer_record_is_rejected_at_image_start() {
    let bytes = image(1, 1, &[0, 0, 0]);
    let limits = Limits::default();
    let table = table(1);
    let mut contexts = ContextBank::new(CONTEXT_COUNT, &limits).unwrap();
    for record_type in [1, 2, 3, u32::MAX] {
        let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
        let mut sink = BytesSink::default();
        let error = ready(Type0Decoder::new(
            &mut source,
            Type0Span {
                record_type,
                offset: 0,
                length: bytes.len() as u64,
            },
            &table,
            &mut contexts,
            &mut sink,
            &limits,
            &NeverCancel,
            arithmetic_budget(),
            Type0Budget::default(),
        ))
        .err()
        .expect("non-type-zero image must be rejected");
        assert_eq!(error.offset, 0);
        assert!(matches!(
            error.kind,
            Type0ErrorKind::Unsupported {
                field: "HN/C8 image record type",
                value
            } if value == u64::from(record_type)
        ));
        assert!(sink.bytes.is_empty());
    }
}

#[test]
fn independent_preflight_limits_identify_the_resource() {
    let bytes = image(9, 1, &[0, 0, 0]);
    let header: &[u8; 48] = (&bytes[..48]).try_into().unwrap();
    let span = Type0Span {
        record_type: 0,
        offset: 19,
        length: bytes.len() as u64,
    };
    let limits = Limits::default();
    let budget = Type0Budget {
        max_width: 8,
        ..Type0Budget::default()
    };
    assert!(matches!(
        checked_info(header, span, &limits, arithmetic_budget(), budget)
            .unwrap_err()
            .kind,
        Type0ErrorKind::LimitExceeded {
            resource: "image width",
            ..
        }
    ));
    let budget = Type0Budget {
        max_height: 0,
        ..Type0Budget::default()
    };
    assert!(matches!(
        checked_info(header, span, &limits, arithmetic_budget(), budget)
            .unwrap_err()
            .kind,
        Type0ErrorKind::LimitExceeded {
            resource: "image height",
            ..
        }
    ));
    let budget = Type0Budget {
        max_pixels: 8,
        ..Type0Budget::default()
    };
    assert!(matches!(
        checked_info(header, span, &limits, arithmetic_budget(), budget)
            .unwrap_err()
            .kind,
        Type0ErrorKind::LimitExceeded {
            resource: "image pixels",
            ..
        }
    ));
    let budget = Type0Budget {
        max_context_work: 90,
        ..Type0Budget::default()
    };
    assert!(matches!(
        checked_info(header, span, &limits, arithmetic_budget(), budget)
            .unwrap_err()
            .kind,
        Type0ErrorKind::LimitExceeded {
            resource: "context work",
            ..
        }
    ));
    let low_symbols = ArithmeticBudget {
        max_symbols: 9,
        ..arithmetic_budget()
    };
    assert!(matches!(
        checked_info(header, span, &limits, low_symbols, Type0Budget::default())
            .unwrap_err()
            .kind,
        Type0ErrorKind::LimitExceeded {
            resource: "arithmetic symbols",
            ..
        }
    ));
}

#[test]
fn public_errors_keep_source_location_and_nested_causes() {
    use std::error::Error as _;
    let offset = 71;
    let values = [
        Type0ErrorKind::InvalidSpan("span"),
        Type0ErrorKind::Truncated("DIB"),
        Type0ErrorKind::Malformed("palette"),
        Type0ErrorKind::Unsupported {
            field: "mode",
            value: 2,
        },
        Type0ErrorKind::LimitExceeded {
            resource: "pixels",
            limit: 4,
            attempted: 5,
        },
        Type0ErrorKind::AllocationFailed,
        Type0ErrorKind::Cancelled,
        Type0ErrorKind::Incomplete,
        Type0ErrorKind::Poisoned,
    ];
    for kind in values {
        let error = Type0Error {
            offset,
            rows_written: 1,
            output_bytes_written: 4,
            kind,
        };
        assert!(error.to_string().contains("source byte 71"));
        assert!(error.source().is_none());
    }
    let nested = [
        Type0ErrorKind::Source(Error::InvalidInput { reason: "read" }),
        Type0ErrorKind::Sink(Error::InvalidInput { reason: "write" }),
        Type0ErrorKind::Arithmetic(ArithmeticError {
            offset: Some(offset),
            context: Some(7),
            kind: ArithmeticErrorKind::InvalidContext,
        }),
    ];
    for kind in nested {
        let error = Type0Error {
            offset,
            rows_written: 0,
            output_bytes_written: 0,
            kind,
        };
        assert!(error.source().is_some());
        assert!(error.to_string().contains("source byte 71"));
    }
}

#[test]
fn sink_failure_poison_and_incomplete_finish_are_explicit() {
    let bytes = image(7, 2, &[0, 0, 0]);
    let limits = Limits::default();
    let table = table(1);
    let mut contexts = ContextBank::new(CONTEXT_COUNT, &limits).unwrap();
    let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
    let mut sink = BytesSink {
        fail: true,
        ..BytesSink::default()
    };
    let mut decoder = ready(Type0Decoder::new(
        &mut source,
        Type0Span {
            record_type: 0,
            offset: 0,
            length: bytes.len() as u64,
        },
        &table,
        &mut contexts,
        &mut sink,
        &limits,
        &NeverCancel,
        arithmetic_budget(),
        Type0Budget::default(),
    ))
    .unwrap();
    assert!(matches!(
        ready(decoder.decode_next_row()).unwrap_err().kind,
        Type0ErrorKind::Sink(_)
    ));
    assert!(matches!(
        ready(decoder.decode_next_row()).unwrap_err().kind,
        Type0ErrorKind::Poisoned
    ));

    let mut contexts = ContextBank::new(CONTEXT_COUNT, &limits).unwrap();
    let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
    let mut sink = BytesSink::default();
    let decoder = ready(Type0Decoder::new(
        &mut source,
        Type0Span {
            record_type: 0,
            offset: 0,
            length: bytes.len() as u64,
        },
        &table,
        &mut contexts,
        &mut sink,
        &limits,
        &NeverCancel,
        arithmetic_budget(),
        Type0Budget::default(),
    ))
    .unwrap();
    assert!(matches!(
        ready(decoder.finish()).unwrap_err().kind,
        Type0ErrorKind::Incomplete
    ));
}

struct BadSink {
    overreport: bool,
}

impl SequentialSink for BadSink {
    async fn write(&mut self, bytes: &[u8]) -> crate::Result<usize> {
        Ok(if self.overreport { bytes.len() + 1 } else { 0 })
    }
    async fn flush(&mut self) -> crate::Result<()> {
        Ok(())
    }
}

#[test]
fn zero_and_overreported_sink_writes_are_typed_errors() {
    let bytes = image(7, 1, &[0, 0, 0]);
    let limits = Limits::default();
    let table = table(1);
    for overreport in [false, true] {
        let mut contexts = ContextBank::new(CONTEXT_COUNT, &limits).unwrap();
        let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
        let mut sink = BadSink { overreport };
        let mut decoder = ready(Type0Decoder::new(
            &mut source,
            Type0Span {
                record_type: 0,
                offset: 0,
                length: bytes.len() as u64,
            },
            &table,
            &mut contexts,
            &mut sink,
            &limits,
            &NeverCancel,
            arithmetic_budget(),
            Type0Budget::default(),
        ))
        .unwrap();
        let error = ready(decoder.decode_next_row()).unwrap_err();
        assert!(matches!(error.kind, Type0ErrorKind::Sink(_)));
        assert_eq!((error.rows_written, error.output_bytes_written), (0, 0));
    }
}

struct DisruptedSource {
    bytes: Vec<u8>,
    stop_at: usize,
    overreport: bool,
}

impl RangedSource for DisruptedSource {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> crate::Result<usize> {
        if self.overreport {
            return Ok(destination.len() + 1);
        }
        let start = offset as usize;
        if start >= self.stop_at {
            return Ok(0);
        }
        let end = self
            .stop_at
            .min(self.bytes.len())
            .min(start + destination.len());
        let n = end - start;
        destination[..n].copy_from_slice(&self.bytes[start..end]);
        Ok(n)
    }
}

#[test]
fn short_and_overreported_reads_and_span_bounds_are_rejected() {
    let bytes = image(9, 1, &[0, 0, 0]);
    let limits = Limits::default();
    let table = table(1);
    for (stop_at, overreport) in [(24, false), (49, false), (bytes.len(), true)] {
        let mut source = DisruptedSource {
            bytes: bytes.clone(),
            stop_at,
            overreport,
        };
        let mut contexts = ContextBank::new(CONTEXT_COUNT, &limits).unwrap();
        let mut sink = BytesSink::default();
        let error = ready(Type0Decoder::new(
            &mut source,
            Type0Span {
                record_type: 0,
                offset: 0,
                length: bytes.len() as u64,
            },
            &table,
            &mut contexts,
            &mut sink,
            &limits,
            &NeverCancel,
            arithmetic_budget(),
            Type0Budget::default(),
        ))
        .err()
        .unwrap();
        assert!(matches!(
            error.kind,
            Type0ErrorKind::Source(_) | Type0ErrorKind::Arithmetic(_)
        ));
    }
    let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
    let mut contexts = ContextBank::new(CONTEXT_COUNT, &limits).unwrap();
    let mut sink = BytesSink::default();
    let error = ready(Type0Decoder::new(
        &mut source,
        Type0Span {
            record_type: 0,
            offset: 1,
            length: bytes.len() as u64,
        },
        &table,
        &mut contexts,
        &mut sink,
        &limits,
        &NeverCancel,
        arithmetic_budget(),
        Type0Budget::default(),
    ))
    .err()
    .unwrap();
    assert!(matches!(error.kind, Type0ErrorKind::InvalidSpan(_)));

    let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
    let mut contexts = ContextBank::new(CONTEXT_COUNT, &limits).unwrap();
    let mut sink = BytesSink::default();
    let error = ready(Type0Decoder::new(
        &mut source,
        Type0Span {
            record_type: 0,
            offset: u64::MAX,
            length: 1,
        },
        &table,
        &mut contexts,
        &mut sink,
        &limits,
        &NeverCancel,
        arithmetic_budget(),
        Type0Budget::default(),
    ))
    .err()
    .unwrap();
    assert!(matches!(error.kind, Type0ErrorKind::InvalidSpan(_)));

    for count in [CONTEXT_COUNT - 1, CONTEXT_COUNT + 1] {
        let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
        let mut contexts = ContextBank::new(count, &limits).unwrap();
        let mut sink = BytesSink::default();
        let error = ready(Type0Decoder::new(
            &mut source,
            Type0Span {
                record_type: 0,
                offset: 0,
                length: bytes.len() as u64,
            },
            &table,
            &mut contexts,
            &mut sink,
            &limits,
            &NeverCancel,
            arithmetic_budget(),
            Type0Budget::default(),
        ))
        .err()
        .unwrap();
        assert!(matches!(
            error.kind,
            Type0ErrorKind::Malformed("expected exactly 1024 arithmetic contexts")
        ));
    }
}

struct PendingSink;

impl SequentialSink for PendingSink {
    async fn write(&mut self, _bytes: &[u8]) -> crate::Result<usize> {
        std::future::pending().await
    }
    async fn flush(&mut self) -> crate::Result<()> {
        Ok(())
    }
}

#[test]
fn dropped_pending_row_future_poisoned_the_decoder() {
    let bytes = image(7, 1, &[0, 0, 0]);
    let limits = Limits::default();
    let table = table(1);
    let mut contexts = ContextBank::new(CONTEXT_COUNT, &limits).unwrap();
    let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
    let mut sink = PendingSink;
    let mut decoder = ready(Type0Decoder::new(
        &mut source,
        Type0Span {
            record_type: 0,
            offset: 0,
            length: bytes.len() as u64,
        },
        &table,
        &mut contexts,
        &mut sink,
        &limits,
        &NeverCancel,
        arithmetic_budget(),
        Type0Budget::default(),
    ))
    .unwrap();
    {
        let mut future = pin!(decoder.decode_next_row());
        let mut task = Context::from_waker(Waker::noop());
        assert!(future.as_mut().poll(&mut task).is_pending());
    }
    assert!(matches!(
        ready(decoder.decode_next_row()).unwrap_err().kind,
        Type0ErrorKind::Poisoned
    ));
}

struct FlagCancel(Rc<Cell<bool>>);

impl Cancellation for FlagCancel {
    fn is_cancelled(&self) -> bool {
        self.0.get()
    }
}

struct CancellingSink(Rc<Cell<bool>>);

impl SequentialSink for CancellingSink {
    async fn write(&mut self, bytes: &[u8]) -> crate::Result<usize> {
        self.0.set(true);
        Ok(bytes.len())
    }
    async fn flush(&mut self) -> crate::Result<()> {
        Ok(())
    }
}

#[test]
fn cancellation_after_row_write_preserves_byte_progress() {
    let bytes = image(7, 1, &[0, 0, 0]);
    let limits = Limits::default();
    let table = table(1);
    let mut contexts = ContextBank::new(CONTEXT_COUNT, &limits).unwrap();
    let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
    let flag = Rc::new(Cell::new(false));
    let cancel = FlagCancel(flag.clone());
    let mut sink = CancellingSink(flag);
    let mut decoder = ready(Type0Decoder::new(
        &mut source,
        Type0Span {
            record_type: 0,
            offset: 0,
            length: bytes.len() as u64,
        },
        &table,
        &mut contexts,
        &mut sink,
        &limits,
        &cancel,
        arithmetic_budget(),
        Type0Budget::default(),
    ))
    .unwrap();
    let error = ready(decoder.decode_next_row()).unwrap_err();
    assert!(matches!(error.kind, Type0ErrorKind::Cancelled));
    assert_eq!((error.rows_written, error.output_bytes_written), (0, 4));
    assert!(decoder.progress().poisoned);
}

#[test]
fn arithmetic_virtual_padding_and_work_limit_remain_bounded() {
    let bytes = image(33, 3, &[0xa0]);
    let limits = Limits::default();
    let table = table(0x4000);
    let mut contexts = ContextBank::new(CONTEXT_COUNT, &limits).unwrap();
    let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
    let mut sink = BytesSink::default();
    let mut decoder = ready(Type0Decoder::new(
        &mut source,
        Type0Span {
            record_type: 0,
            offset: 0,
            length: bytes.len() as u64,
        },
        &table,
        &mut contexts,
        &mut sink,
        &limits,
        &NeverCancel,
        arithmetic_budget(),
        Type0Budget::default(),
    ))
    .unwrap();
    for _ in 0..3 {
        assert!(ready(decoder.decode_next_row()).unwrap());
    }
    let report = ready(decoder.finish()).unwrap();
    assert!(report.progress.arithmetic.virtual_zero_bytes > 0);
    assert_eq!(report.progress.arithmetic.physical_bytes_consumed, 1);

    let mut contexts = ContextBank::new(CONTEXT_COUNT, &limits).unwrap();
    let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
    let mut sink = BytesSink::default();
    let low = ArithmeticBudget {
        max_symbols: 102,
        max_work: 3,
    };
    let error = match ready(Type0Decoder::new(
        &mut source,
        Type0Span {
            record_type: 0,
            offset: 0,
            length: bytes.len() as u64,
        },
        &table,
        &mut contexts,
        &mut sink,
        &limits,
        &NeverCancel,
        low,
        Type0Budget::default(),
    )) {
        Ok(mut decoder) => ready(decoder.decode_next_row()).unwrap_err(),
        Err(error) => error,
    };
    assert!(matches!(error.kind, Type0ErrorKind::Arithmetic(_)));
}

#[test]
fn report_separates_source_prefetch_from_consumed_and_virtual_bytes() {
    let bytes = image(1, 1, &[0; 10]);
    let limits = Limits::default();
    let table = table(1);
    let mut contexts = ContextBank::new(CONTEXT_COUNT, &limits).unwrap();
    let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
    let mut sink = BytesSink::default();
    let mut decoder = ready(Type0Decoder::new(
        &mut source,
        Type0Span {
            record_type: 0,
            offset: 0,
            length: bytes.len() as u64,
        },
        &table,
        &mut contexts,
        &mut sink,
        &limits,
        &NeverCancel,
        arithmetic_budget(),
        Type0Budget::default(),
    ))
    .unwrap();
    assert!(ready(decoder.decode_next_row()).unwrap());
    let snapshot = ready(decoder.finish()).unwrap().progress.arithmetic;
    assert_eq!(snapshot.physical_bytes_consumed, 3);
    assert_eq!(snapshot.source_bytes_fetched, 10);
    assert_eq!(snapshot.virtual_zero_bytes, 0);
}

#[test]
fn fixed_budget_mutations_return_without_panic_or_unbounded_work() {
    let original = image(9, 2, &[0x90, 0, 0]);
    let limits = Limits::default();
    let table = table(0x4000);
    let budget = ArithmeticBudget {
        max_symbols: 20,
        max_work: 600,
    };
    for seed in 0..128_usize {
        let mut bytes = original.clone();
        let index = (seed * 17) % bytes.len();
        bytes[index] ^= 1 << (seed % 8);
        let mut contexts = ContextBank::new(CONTEXT_COUNT, &limits).unwrap();
        let mut source = SeekableSource::new(Cursor::new(bytes.clone())).unwrap();
        let mut sink = BytesSink::default();
        if let Ok(mut decoder) = ready(Type0Decoder::new(
            &mut source,
            Type0Span {
                record_type: 0,
                offset: 0,
                length: bytes.len() as u64,
            },
            &table,
            &mut contexts,
            &mut sink,
            &limits,
            &NeverCancel,
            budget,
            Type0Budget::default(),
        )) {
            for _ in 0..2 {
                if ready(decoder.decode_next_row()).is_err() {
                    break;
                }
            }
            // A malformed coding stream may still produce plausible
            // pixels through T.82 virtual zeros; no validity claim here.
        }
    }
}
