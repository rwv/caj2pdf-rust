// SPDX-License-Identifier: MIT

//! Public IAID API checks with original bytes for the standard MQ states.
//! These are arithmetic control-flow tests, not T.88 Table E.1 conformance.

use caj2pdf_core::{
    Cancellation, Limits, NeverCancel, RangedSource,
    jbig2::{
        dictionary::coding_unit_contexts,
        iaid::{IAID_BASE, SymbolIdError, checked_symbol_index, decode_iaid},
        integer::{BITMAP_BASE, IntegerProcedure, decode_integer},
        mq::{
            ArithmeticErrorKind, CodedSpan, ContextBank, ContextState, MqBudget, MqDecoder, MqTable,
        },
    },
};
use std::{
    cell::Cell,
    future::Future,
    pin::pin,
    rc::Rc,
    task::{Context, Poll, Waker},
};

fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut task = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut task) {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("unexpected pending source"),
    }
}

fn table() -> MqTable {
    MqTable::standard()
}

struct Source {
    bytes: Vec<u8>,
    advertised: u64,
    pending_at: Option<u64>,
    reads: Vec<(u64, usize)>,
}

impl Source {
    fn new(bytes: &[u8]) -> Self {
        Self {
            bytes: bytes.to_vec(),
            advertised: bytes.len() as u64,
            pending_at: None,
            reads: Vec::new(),
        }
    }

    fn span(&self) -> CodedSpan {
        CodedSpan {
            offset: 0,
            length: self.advertised,
        }
    }
}

impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.advertised
    }

    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        self.reads.push((offset, destination.len()));
        if self.pending_at == Some(offset) {
            std::future::pending::<()>().await;
        }
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let count = self
            .bytes
            .len()
            .saturating_sub(start)
            .min(destination.len());
        if count > 0 {
            destination[..count].copy_from_slice(&self.bytes[start..start + count]);
        }
        Ok(count)
    }
}

/// The contexts of a coding unit whose IAID width is `code_len`.
fn coding_unit(code_len: u32, limits: &Limits, budget: &MqBudget) -> ContextBank {
    budget
        .context_bank(coding_unit_contexts(code_len).unwrap(), limits)
        .unwrap()
}

struct Flag(Rc<Cell<bool>>);

impl Cancellation for Flag {
    fn is_cancelled(&self) -> bool {
        self.0.get()
    }
}

#[test]
fn zero_length_id_and_symbol_array_boundary() {
    let limits = Limits::default();
    let budget = MqBudget::default();
    let table = table();
    let mut contexts = coding_unit(0, &limits, &budget);
    assert_eq!(contexts.len(), IAID_BASE + 1);
    let mut source = Source::new(&[0x7f, 0xff, 0xac]);
    let span = source.span();
    let mut decoder = ready(MqDecoder::new(
        &mut source,
        span,
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .unwrap();
    let before = decoder.snapshot();
    assert_eq!(ready(decode_iaid(&mut decoder, 0)).unwrap(), 0);
    assert_eq!(decoder.snapshot(), before);
    ready(decoder.finish(0)).unwrap();

    assert_eq!(checked_symbol_index(0, 1, 1), Ok(0));
    assert_eq!(checked_symbol_index(2, 3, 3), Ok(2));
    assert_eq!(
        checked_symbol_index(0, 0, 0),
        Err(SymbolIdError::EmptySymbolSet)
    );
    assert_eq!(
        checked_symbol_index(3, 3, 3),
        Err(SymbolIdError::OutOfRange { id: 3, count: 3 })
    );
    assert_eq!(
        checked_symbol_index(2, 3, 2),
        Err(SymbolIdError::SymbolArrayLength {
            declared: 3,
            actual: 2
        })
    );
    assert_eq!(
        checked_symbol_index(0, 1, 2),
        Err(SymbolIdError::SymbolArrayLength {
            declared: 1,
            actual: 2
        })
    );
    if usize::BITS < 64 {
        assert_eq!(
            checked_symbol_index(0, u64::MAX, 0),
            Err(SymbolIdError::TooManySymbols { count: u64::MAX })
        );
    }
}

#[test]
fn symbol_index_errors_name_the_rejected_values() {
    let messages = [
        (
            checked_symbol_index(0, 0, 0).unwrap_err(),
            "symbol count must be nonzero",
        ),
        (
            checked_symbol_index(2, 3, 2).unwrap_err(),
            "declared 3 symbols, but the array has 2",
        ),
        (
            checked_symbol_index(3, 3, 3).unwrap_err(),
            "symbol ID 3 is outside 0..3",
        ),
        // Only a narrower address space can produce this from real input.
        (
            SymbolIdError::TooManySymbols { count: u64::MAX },
            "symbol count 18446744073709551615 exceeds the address space",
        ),
    ];
    for (error, message) in messages {
        assert_eq!(error.to_string(), message);
        assert!(std::error::Error::source(&error).is_none());
    }
}

#[test]
fn iaid_and_a2_use_distinct_adaptive_banks_on_one_stream() {
    let limits = Limits::default();
    let budget = MqBudget::default();
    let table = table();
    let bytes = [0x00, 0x00, 0x0a, 0xc0, 0x76, 0x01, 0xff, 0xac];
    let mut source = Source::new(&bytes);
    let span = source.span();
    let mut contexts = coding_unit(1, &limits, &budget);
    let mut decoder = ready(MqDecoder::new(
        &mut source,
        span,
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .unwrap();
    let first = ready(decode_iaid(&mut decoder, 1)).unwrap();
    let iaid_after_first = decoder.context(IAID_BASE + 1).unwrap();
    assert_ne!(iaid_after_first, ContextState::default());
    let _integer = ready(decode_integer(&mut decoder, IntegerProcedure::Iaai)).unwrap();
    assert_eq!(decoder.context(IAID_BASE + 1), Some(iaid_after_first));
    assert_ne!(decoder.context(1), Some(ContextState::default()));
    assert_eq!(decoder.context(BITMAP_BASE), Some(ContextState::default()));
    let second = ready(decode_iaid(&mut decoder, 1)).unwrap();
    assert_eq!((first, second), (0, 1));
    assert_ne!(decoder.context(IAID_BASE + 1), Some(iaid_after_first));
    let symbols = decoder.snapshot().symbols_decoded;
    assert!(symbols >= 6);
    ready(decoder.finish(symbols)).unwrap();
    assert!(
        source
            .reads
            .iter()
            .all(|&(offset, len)| offset + len as u64 <= span.length)
    );
}

#[test]
fn the_full_iaid_range_is_checked_before_a_decision() {
    let limits = Limits::default();
    let budget = MqBudget::default();
    let table = table();
    let bytes = [0x7f, 0xff, 0xac];
    let mut source = Source::new(&bytes);
    let span = source.span();
    let mut contexts = coding_unit(1, &limits, &budget);
    let mut decoder = ready(MqDecoder::new(
        &mut source,
        span,
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .unwrap();
    for code_len in [2, 63, 64, u32::MAX] {
        let before = decoder.snapshot();
        let error = ready(decode_iaid(&mut decoder, code_len)).unwrap_err();
        assert!(matches!(error.kind, ArithmeticErrorKind::InvalidContext));
        assert_eq!(decoder.snapshot(), before);
    }
    ready(decoder.finish(0)).unwrap();
}

#[test]
fn symbol_and_work_limits_and_cancellation_propagate() {
    let limits = Limits {
        io_chunk_bytes: 1,
        ..Limits::default()
    };
    let table = table();
    let bytes = [0x80, 0, 0, 0xff, 0xac];
    let budget = MqBudget {
        max_symbols: 1,
        ..MqBudget::default()
    };
    let mut source = Source::new(&bytes);
    let span = source.span();
    let mut contexts = coding_unit(2, &limits, &budget);
    let mut decoder = ready(MqDecoder::new(
        &mut source,
        span,
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .unwrap();
    let error = ready(decode_iaid(&mut decoder, 2)).unwrap_err();
    assert!(matches!(
        error.kind,
        ArithmeticErrorKind::LimitExceeded {
            resource: "symbols",
            ..
        }
    ));
    assert_eq!(decoder.snapshot().symbols_decoded, 1);

    let budget = MqBudget {
        max_work: 3,
        ..MqBudget::default()
    };
    let mut source = Source::new(&bytes);
    let span = source.span();
    let mut contexts = coding_unit(1, &limits, &budget);
    let mut decoder = ready(MqDecoder::new(
        &mut source,
        span,
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .unwrap();
    let error = ready(decode_iaid(&mut decoder, 1)).unwrap_err();
    assert!(matches!(
        error.kind,
        ArithmeticErrorKind::LimitExceeded {
            resource: "arithmetic work",
            ..
        }
    ));

    let budget = MqBudget::default();
    let cancelled = Rc::new(Cell::new(false));
    let flag = Flag(cancelled.clone());
    let mut source = Source::new(&bytes);
    let span = source.span();
    let mut contexts = coding_unit(0, &limits, &budget);
    let mut decoder = ready(MqDecoder::new(
        &mut source,
        span,
        &table,
        &mut contexts,
        &limits,
        &flag,
        budget,
    ))
    .unwrap();
    cancelled.set(true);
    let error = ready(decode_iaid(&mut decoder, 0)).unwrap_err();
    assert!(matches!(error.kind, ArithmeticErrorKind::Cancelled));
    assert_eq!(decoder.snapshot().symbols_decoded, 0);
}

#[test]
fn truncated_source_marker_and_dropped_future_are_located() {
    let limits = Limits {
        io_chunk_bytes: 1,
        ..Limits::default()
    };
    let budget = MqBudget::default();
    let table = table();
    let mut source = Source::new(&[0x80, 0]);
    source.advertised = 4;
    let span = source.span();
    let mut contexts = coding_unit(2, &limits, &budget);
    let mut decoder = ready(MqDecoder::new(
        &mut source,
        span,
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .unwrap();
    let error = ready(decode_iaid(&mut decoder, 2)).unwrap_err();
    assert!(matches!(
        error.kind,
        ArithmeticErrorKind::Source(caj2pdf_core::Error::TruncatedInput { .. })
    ));
    assert_eq!(error.offset, Some(2));

    for (bytes, invalid_marker) in [
        (&[0x80, 0][..], false),
        (&[0x80, 0xff, 0x90, 0xff, 0xac][..], true),
    ] {
        let mut source = Source::new(bytes);
        let span = source.span();
        let mut contexts = coding_unit(2, &limits, &budget);
        let mut decoder = ready(MqDecoder::new(
            &mut source,
            span,
            &table,
            &mut contexts,
            &limits,
            &NeverCancel,
            budget,
        ))
        .unwrap();
        let error = ready(decode_iaid(&mut decoder, 2)).unwrap_err();
        if invalid_marker {
            assert!(matches!(
                error.kind,
                ArithmeticErrorKind::InvalidMarker(0x90)
            ));
        } else {
            assert!(matches!(error.kind, ArithmeticErrorKind::MissingTerminator));
        }
        assert_eq!(error.offset, Some(2));
    }

    let mut source = Source::new(&[0x80, 0, 0xff, 0x90]);
    let span = source.span();
    let mut contexts = coding_unit(0, &limits, &budget);
    let mut decoder = ready(MqDecoder::new(
        &mut source,
        span,
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .unwrap();
    assert_eq!(ready(decode_iaid(&mut decoder, 0)).unwrap(), 0);
    let error = ready(decoder.finish(0)).unwrap_err();
    assert!(matches!(
        error.kind,
        ArithmeticErrorKind::InvalidMarker(0x90)
    ));

    let mut source = Source::new(&[0x80, 0, 0, 0xff, 0xac]);
    source.pending_at = Some(2);
    let span = source.span();
    let mut contexts = coding_unit(2, &limits, &budget);
    let mut decoder = ready(MqDecoder::new(
        &mut source,
        span,
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .unwrap();
    {
        let mut future = pin!(decode_iaid(&mut decoder, 2));
        let mut task = Context::from_waker(Waker::noop());
        assert!(matches!(future.as_mut().poll(&mut task), Poll::Pending));
    }
    let error = ready(decode_iaid(&mut decoder, 2)).unwrap_err();
    assert!(matches!(error.kind, ArithmeticErrorKind::Poisoned));
    assert!(decoder.snapshot().poisoned);
}

#[test]
fn zero_bit_call_rejects_a_poisoned_shared_decoder() {
    let limits = Limits {
        io_chunk_bytes: 1,
        ..Limits::default()
    };
    let budget = MqBudget::default();
    let table = table();
    let mut source = Source::new(&[0x80, 0, 0, 0xff, 0xac]);
    source.pending_at = Some(2);
    let span = source.span();
    let mut contexts = coding_unit(0, &limits, &budget);
    let mut decoder = ready(MqDecoder::new(
        &mut source,
        span,
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .unwrap();
    assert!(!ready(decoder.decode_bit(0)).unwrap());
    {
        let mut future = pin!(decoder.decode_bit(0));
        let mut task = Context::from_waker(Waker::noop());
        assert!(matches!(future.as_mut().poll(&mut task), Poll::Pending));
    }
    let error = ready(decode_iaid(&mut decoder, 0)).unwrap_err();
    assert!(matches!(error.kind, ArithmeticErrorKind::Poisoned));
    assert_eq!(decoder.snapshot().symbols_decoded, 1);
}

#[test]
fn bounded_mutation_smoke_keeps_work_and_reads_within_limits() {
    let limits = Limits {
        io_chunk_bytes: 1,
        max_input_bytes: 5,
        max_allocation_bytes: 20_000,
        ..Limits::default()
    };
    let budget = MqBudget {
        max_span_bytes: 5,
        max_contexts: 7688,
        max_symbols: 3,
        max_work: 24,
        max_terminal_inputs: 4,
    };
    let table = table();
    for seed in 0..128u8 {
        let mut source = Source::new(&[seed, seed ^ 0x55, 0, 0xff, 0xac]);
        let span = source.span();
        let mut contexts = coding_unit(3, &limits, &budget);
        if let Ok(mut decoder) = ready(MqDecoder::new(
            &mut source,
            span,
            &table,
            &mut contexts,
            &limits,
            &NeverCancel,
            budget,
        )) {
            let _ = ready(decode_iaid(&mut decoder, 3));
            let snapshot = decoder.snapshot();
            assert!(snapshot.work_done <= budget.max_work);
            assert!(snapshot.symbols_decoded <= budget.max_symbols);
        }
        assert!(
            source
                .reads
                .iter()
                .all(|&(offset, len)| len == 1 && offset < span.length)
        );
    }
}
