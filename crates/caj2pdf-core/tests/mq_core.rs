// SPDX-License-Identifier: MIT

//! Original small examples for the T.88 MQ control-flow API. The register
//! traces are hand-derived from the standard T.88 Table E.1 states.

use caj2pdf_core::{
    Cancellation, Error, Limits, MAX_BUDGET_COUNT, NeverCancel, RangedSource,
    jbig2::mq::{
        ArithmeticErrorKind, CodedSpan, ContextBank, ContextState, MqBudget, MqDecoder, MqTable,
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
    let mut context = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut context) {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("unexpected pending test source"),
    }
}

fn table() -> MqTable {
    MqTable::standard()
}

struct Source {
    bytes: Vec<u8>,
    advertised: u64,
    max_read: usize,
    overreport: bool,
    pending_at: Option<u64>,
    cancel_after: Option<(usize, Rc<Cell<bool>>)>,
    seen: Vec<(u64, usize)>,
}

impl Source {
    fn new(bytes: &[u8]) -> Self {
        Self {
            bytes: bytes.to_vec(),
            advertised: bytes.len() as u64,
            max_read: usize::MAX,
            overreport: false,
            pending_at: None,
            cancel_after: None,
            seen: Vec::new(),
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
        self.seen.push((offset, destination.len()));
        if self.pending_at == Some(offset) {
            std::future::pending::<()>().await;
        }
        if self.overreport {
            return Ok(destination.len() + 1);
        }
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let count = self
            .bytes
            .len()
            .saturating_sub(start)
            .min(destination.len())
            .min(self.max_read);
        if count > 0 {
            destination[..count].copy_from_slice(&self.bytes[start..start + count]);
        }
        if let Some((reads, flag)) = &self.cancel_after
            && self.seen.len() == *reads
        {
            flag.set(true);
        }
        Ok(count)
    }
}

struct Flag(Rc<Cell<bool>>);

impl Cancellation for Flag {
    fn is_cancelled(&self) -> bool {
        self.0.get()
    }
}

fn bank(budget: &MqBudget, limits: &Limits) -> ContextBank {
    budget.context_bank(1, limits).unwrap()
}

#[test]
fn counter_budgets_above_the_ceiling_are_rejected_before_allocation() {
    let limits = Limits::default();
    type Field = fn(&mut MqBudget) -> &mut u64;
    let fields: [Field; 3] = [
        |budget| &mut budget.max_symbols,
        |budget| &mut budget.max_work,
        |budget| &mut budget.max_terminal_inputs,
    ];
    for field in fields {
        let mut budget = MqBudget::default();
        *field(&mut budget) = MAX_BUDGET_COUNT;
        budget.context_bank(1, &limits).unwrap();
        *field(&mut budget) = MAX_BUDGET_COUNT + 1;
        assert!(matches!(
            budget.context_bank(1, &limits).unwrap_err().kind,
            ArithmeticErrorKind::InvalidBudget
        ));
        let mut contexts = bank(&MqBudget::default(), &limits);
        let mut source = Source::new(&[0, 0, 0xff, 0xac]);
        let error = ready(MqDecoder::new(
            &mut source,
            CodedSpan {
                offset: 0,
                length: 4,
            },
            &table(),
            &mut contexts,
            &limits,
            &NeverCancel,
            budget,
        ))
        .err()
        .unwrap();
        assert!(matches!(error.kind, ArithmeticErrorKind::InvalidBudget));
        assert!(source.seen.is_empty());
    }
}

#[test]
fn context_banks_beyond_the_address_space_are_refused_without_allocating() {
    let limits = Limits {
        max_allocation_bytes: u64::MAX,
        ..Limits::default()
    };
    let budget = MqBudget {
        max_contexts: usize::MAX,
        ..MqBudget::default()
    };
    // The byte size of this bank overflows `usize`.
    assert!(matches!(
        budget.context_bank(usize::MAX, &limits).unwrap_err().kind,
        ArithmeticErrorKind::InvalidContext
    ));
    // This byte size fits `usize` but exceeds `isize::MAX`, so the fallible
    // reservation fails before the allocator is called.
    #[cfg(target_pointer_width = "64")]
    assert!(matches!(
        budget.context_bank(1 << 62, &limits).unwrap_err().kind,
        ArithmeticErrorKind::AllocationFailed
    ));
}

fn span(source: &Source) -> CodedSpan {
    CodedSpan {
        offset: 0,
        length: source.advertised,
    }
}

/// Decode `count` decisions in context zero, recording each bit with the
/// registers and the context state after it.
fn register_trace(bytes: &[u8], count: usize) -> Vec<(bool, u32, u32, u8, ContextState)> {
    let limits = Limits::default();
    let budget = MqBudget::default();
    let mut source = Source::new(bytes);
    let state_table = table();
    let mut contexts = bank(&budget, &limits);
    let selected = span(&source);
    let mut decoder = ready(MqDecoder::new(
        &mut source,
        selected,
        &state_table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .unwrap();
    assert_eq!(decoder.snapshot().interval, 0x8000);
    let mut trace = Vec::new();
    for _ in 0..count {
        let bit = ready(decoder.decode_bit(0)).unwrap();
        let progress = decoder.snapshot();
        trace.push((
            bit,
            progress.interval,
            progress.code,
            progress.bit_counter,
            decoder.context(0).unwrap(),
        ));
    }
    assert_eq!(decoder.snapshot().symbols_decoded, count as u64);
    ready(decoder.finish(count as u64)).unwrap();
    assert!(
        source
            .seen
            .iter()
            .all(|(offset, count)| offset.checked_add(*count as u64).unwrap() <= selected.length)
    );
    trace
}

fn state(state_index: u8, mps: bool) -> ContextState {
    ContextState { state_index, mps }
}

#[test]
fn initialization_mps_and_lps_exchanges_have_independent_register_expectations() {
    // Table E.1: state 0 has Qe 0x5601, NMPS = NLPS = 1 and SWITCH; state 1
    // has Qe 0x3401, NMPS 2, NLPS 6; state 2 has Qe 0x1801.
    // C = 0: A - Qe = 0x29FF < Qe exchanges the lower subinterval [0, Qe)
    // to the MPS. The second decision splits A = 0xAC02 normally, so C in
    // [0, Qe) is an LPS; its renormalization reads the terminal FF.
    assert_eq!(
        register_trace(&[0x00, 0x00, 0xff, 0xac], 2),
        [
            (false, 0xac02, 0, 0, state(1, false)),
            (true, 0xd004, 0x0003_fc00, 6, state(6, false)),
        ]
    );
    // C high 0x6000 lies in the upper subinterval, exchanged to the LPS,
    // which also switches the MPS.
    assert_eq!(
        register_trace(&[0xc0, 0x00, 0xff, 0xac], 1),
        [(true, 0xa7fc, 0x27fd_fe00, 7, state(1, true))]
    );
    // C high 0x2080: an exchanged MPS; then an ordinary MPS in the upper
    // subinterval with one renormalization shift; then, with A - Qe =
    // 0xD801 >= 0x8000, an MPS that neither renormalizes nor changes state.
    assert_eq!(
        register_trace(&[0x41, 0x00, 0xff, 0xac], 3),
        [
            (false, 0xac02, 0x4100_0000, 0, state(1, false)),
            (false, 0xf002, 0x19ff_fe00, 7, state(2, false)),
            (false, 0xd801, 0x01fe_fe00, 7, state(2, false)),
        ]
    );
}

#[test]
fn normal_and_post_ff_input_have_distinct_initial_c_and_ct() {
    let limits = Limits::default();
    let budget = MqBudget::default();
    let state_table = table();
    let mut source = Source::new(&[0xff, 0x7f, 0x00, 0xff, 0xac]);
    let mut contexts = bank(&budget, &limits);
    let selected = span(&source);
    let mut decoder = ready(MqDecoder::new(
        &mut source,
        selected,
        &state_table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .unwrap();
    let initial = decoder.snapshot();
    assert_eq!(
        (initial.interval, initial.code, initial.bit_counter),
        (0x8000, 0x7fff_0000, 0)
    );
    assert_eq!(initial.input_offset, 1);
    assert_eq!(initial.source_bytes_fetched, 5);
    // C high 0x7FFF is an exchanged LPS; two renormalization shifts read
    // the byte after the stuffed one as eight fresh bits.
    assert!(ready(decoder.decode_bit(0)).unwrap());
    let after = decoder.snapshot();
    assert_eq!(after.input_offset, 2);
    assert_eq!(after.bit_counter, 6);
    ready(decoder.finish(1)).unwrap();
}

#[test]
fn terminal_input_is_counted_and_marker_is_bounded() {
    let limits = Limits::default();
    let budget = MqBudget::default();
    let state_table = table();
    let mut source = Source::new(&[0x00, 0xff, 0xac]);
    let mut contexts = bank(&budget, &limits);
    let selected = span(&source);
    let mut decoder = ready(MqDecoder::new(
        &mut source,
        selected,
        &state_table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .unwrap();
    assert!(!ready(decoder.decode_bit(0)).unwrap());
    assert!(ready(decoder.decode_bit(0)).unwrap());
    let progress = decoder.snapshot();
    assert_eq!(progress.input_offset, 1);
    assert_eq!(progress.source_bytes_fetched, 3);
    assert_eq!(progress.synthesized_inputs, 1);
    ready(decoder.finish(2)).unwrap();
    assert!(
        source
            .seen
            .iter()
            .all(|(offset, count)| offset + *count as u64 <= selected.length)
    );

    let mut malformed = Source::new(&[0xff, 0x90]);
    let mut contexts = bank(&budget, &limits);
    let error = ready(MqDecoder::new(
        &mut malformed,
        CodedSpan {
            offset: 0,
            length: 2,
        },
        &state_table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .err()
    .expect("malformed marker must fail");
    assert!(matches!(
        error.kind,
        ArithmeticErrorKind::InvalidMarker(0x90)
    ));
    assert_eq!(error.offset, Some(1));
}

#[test]
fn short_zero_and_overreported_reads_are_checked() {
    let limits = Limits::default();
    let budget = MqBudget::default();
    let state_table = table();
    let mut source = Source::new(&[0, 0, 0xff, 0xac]);
    source.max_read = 1;
    let mut contexts = bank(&budget, &limits);
    let selected = span(&source);
    let mut decoder = ready(MqDecoder::new(
        &mut source,
        selected,
        &state_table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .unwrap();
    assert!(!ready(decoder.decode_bit(0)).unwrap());
    ready(decoder.finish(1)).unwrap();
    assert_eq!(source.seen.len(), 4);

    let mut truncated = Source::new(&[0]);
    truncated.advertised = 4;
    let mut contexts = bank(&budget, &limits);
    let error = ready(MqDecoder::new(
        &mut truncated,
        CodedSpan {
            offset: 0,
            length: 4,
        },
        &state_table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .err()
    .unwrap();
    assert!(matches!(
        error.kind,
        ArithmeticErrorKind::Source(Error::TruncatedInput { .. })
    ));
    assert_eq!(error.offset, Some(1));
    assert!(std::error::Error::source(&error).is_some());

    let one_byte_limits = Limits {
        io_chunk_bytes: 1,
        ..limits
    };
    let mut late_truncation = Source::new(&[0, 0]);
    late_truncation.advertised = 4;
    let mut contexts = bank(&budget, &one_byte_limits);
    let mut decoder = ready(MqDecoder::new(
        &mut late_truncation,
        CodedSpan {
            offset: 0,
            length: 4,
        },
        &state_table,
        &mut contexts,
        &one_byte_limits,
        &NeverCancel,
        budget,
    ))
    .unwrap();
    ready(decoder.decode_bit(0)).unwrap();
    let error = ready(decoder.decode_bit(0)).unwrap_err();
    assert!(matches!(
        error.kind,
        ArithmeticErrorKind::Source(Error::TruncatedInput { .. })
    ));
    assert_eq!(error.offset, Some(2));
    assert_eq!(decoder.snapshot().source_bytes_fetched, 2);
    assert!(decoder.snapshot().poisoned);

    let mut overreport = Source::new(&[0, 0, 0xff, 0xac]);
    overreport.overreport = true;
    let mut contexts = bank(&budget, &limits);
    let error = ready(MqDecoder::new(
        &mut overreport,
        CodedSpan {
            offset: 0,
            length: 4,
        },
        &state_table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .err()
    .unwrap();
    assert!(matches!(
        error.kind,
        ArithmeticErrorKind::Source(Error::InvalidInput { .. })
    ));
}

#[test]
fn finish_and_context_errors_keep_public_locations() {
    use std::error::Error as _;

    let limits = Limits::default();
    let budget = MqBudget::default();
    let state_table = table();
    let mut contexts = bank(&budget, &limits);

    let mut source = Source::new(&[0, 0]);
    let mut decoder = ready(MqDecoder::new(
        &mut source,
        CodedSpan {
            offset: 0,
            length: 2,
        },
        &state_table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .unwrap();
    let bad_context = ready(decoder.decode_bit(1)).unwrap_err();
    assert!(matches!(
        bad_context.kind,
        ArithmeticErrorKind::InvalidContext
    ));
    assert_eq!(bad_context.context, Some(1));
    assert!(bad_context.to_string().contains("context 1"));
    assert!(bad_context.source().is_none());
    ready(decoder.decode_bit(0)).unwrap();
    let at_end = ready(decoder.decode_bit(0)).unwrap_err();
    assert!(matches!(
        at_end.kind,
        ArithmeticErrorKind::MissingTerminator
    ));
    assert_eq!(at_end.offset, Some(2));
    assert_eq!(at_end.context, Some(0));
    assert!(at_end.to_string().contains("source byte 2"));

    let mut source = Source::new(&[0, 0, 0]);
    let mut contexts = bank(&budget, &limits);
    let decoder = ready(MqDecoder::new(
        &mut source,
        CodedSpan {
            offset: 0,
            length: 3,
        },
        &state_table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .unwrap();
    let wrong_count = ready(decoder.finish(1)).unwrap_err();
    assert!(matches!(
        wrong_count.kind,
        ArithmeticErrorKind::SymbolCount {
            expected: 1,
            decoded: 0
        }
    ));
    assert!(wrong_count.to_string().contains("expected 1 symbols"));

    let decoder = ready(MqDecoder::new(
        &mut source,
        CodedSpan {
            offset: 0,
            length: 3,
        },
        &state_table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .unwrap();
    let marker = ready(decoder.finish(0)).unwrap_err();
    assert!(matches!(
        marker.kind,
        ArithmeticErrorKind::MissingTerminator
    ));
    assert_eq!(marker.offset, Some(1));
    assert!(marker.to_string().contains("missing terminal marker"));

    let mut interior_marker = Source::new(&[0xff, 0xac, 0, 0xff, 0xac]);
    let error = ready(MqDecoder::new(
        &mut interior_marker,
        CodedSpan {
            offset: 0,
            length: 5,
        },
        &state_table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .err()
    .unwrap();
    assert!(matches!(
        error.kind,
        ArithmeticErrorKind::InvalidMarker(0xac)
    ));
    assert_eq!(error.offset, Some(1));

    // A terminal pair beyond the initial prefetch is checked only by finish.
    let mut bytes = vec![0; 300];
    bytes[298..].copy_from_slice(&[0xff, 0xab]);
    let mut late_marker = Source::new(&bytes);
    let decoder = ready(MqDecoder::new(
        &mut late_marker,
        CodedSpan {
            offset: 0,
            length: 300,
        },
        &state_table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .unwrap();
    let error = ready(decoder.finish(0)).unwrap_err();
    assert!(matches!(
        error.kind,
        ArithmeticErrorKind::InvalidMarker(0xab)
    ));
    assert_eq!(error.offset, Some(299));
}

#[test]
fn invalid_configuration_and_budgets_fail_without_unbounded_work() {
    let limits = Limits::default();
    let budget = MqBudget::default();
    assert!(matches!(
        budget.context_bank(0, &limits).unwrap_err().kind,
        ArithmeticErrorKind::InvalidContext
    ));
    assert!(matches!(
        budget.context_bank(usize::MAX, &limits).unwrap_err().kind,
        ArithmeticErrorKind::LimitExceeded {
            resource: "MQ contexts",
            ..
        }
    ));
    let mut source = Source::new(&[0, 0, 0xff, 0xac]);
    let state_table = table();
    // The bank fits, but not with the state table and the input buffer.
    let tight = Limits {
        io_chunk_bytes: 64,
        max_allocation_bytes: 300,
        ..limits
    };
    let mut contexts = bank(&budget, &tight);
    let error = ready(MqDecoder::new(
        &mut source,
        CodedSpan {
            offset: 0,
            length: 4,
        },
        &state_table,
        &mut contexts,
        &tight,
        &NeverCancel,
        budget,
    ))
    .err()
    .unwrap();
    assert!(matches!(
        error.kind,
        ArithmeticErrorKind::Source(Error::LimitExceeded { .. })
    ));
    assert!(source.seen.is_empty());
    for selected in [
        CodedSpan {
            offset: 0,
            length: 1,
        },
        CodedSpan {
            offset: u64::MAX - 1,
            length: 3,
        },
        CodedSpan {
            offset: 3,
            length: 3,
        },
    ] {
        let error = ready(MqDecoder::new(
            &mut source,
            selected,
            &state_table,
            &mut contexts,
            &limits,
            &NeverCancel,
            budget,
        ))
        .err()
        .unwrap();
        assert!(matches!(error.kind, ArithmeticErrorKind::InvalidSpan(_)));
    }
    let span_budget = MqBudget {
        max_span_bytes: 3,
        ..budget
    };
    let error = ready(MqDecoder::new(
        &mut source,
        CodedSpan {
            offset: 0,
            length: 4,
        },
        &state_table,
        &mut contexts,
        &limits,
        &NeverCancel,
        span_budget,
    ))
    .err()
    .unwrap();
    assert!(matches!(
        error.kind,
        ArithmeticErrorKind::LimitExceeded {
            resource: "MQ span bytes",
            ..
        }
    ));
    let invalid_budget = MqBudget {
        max_work: 0,
        ..budget
    };
    assert!(matches!(
        invalid_budget.context_bank(1, &limits).unwrap_err().kind,
        ArithmeticErrorKind::InvalidBudget
    ));
    let mut context_pair = budget.context_bank(2, &limits).unwrap();
    let context_budget = MqBudget {
        max_contexts: 1,
        ..budget
    };
    assert!(matches!(
        ready(MqDecoder::new(
            &mut source,
            CodedSpan {
                offset: 0,
                length: 4
            },
            &state_table,
            &mut context_pair,
            &limits,
            &NeverCancel,
            context_budget,
        ))
        .err()
        .unwrap()
        .kind,
        ArithmeticErrorKind::InvalidContext
    ));
    let symbol_budget = MqBudget {
        max_symbols: 1,
        ..budget
    };
    let mut decoder = ready(MqDecoder::new(
        &mut source,
        CodedSpan {
            offset: 0,
            length: 4,
        },
        &state_table,
        &mut contexts,
        &limits,
        &NeverCancel,
        symbol_budget,
    ))
    .unwrap();
    ready(decoder.decode_bit(0)).unwrap();
    assert!(matches!(
        ready(decoder.decode_bit(0)).unwrap_err().kind,
        ArithmeticErrorKind::LimitExceeded {
            resource: "symbols",
            ..
        }
    ));

    let work_budget = MqBudget {
        max_work: 1,
        ..budget
    };
    let mut contexts = bank(&budget, &limits);
    assert!(matches!(
        ready(MqDecoder::new(
            &mut source,
            CodedSpan {
                offset: 0,
                length: 4
            },
            &state_table,
            &mut contexts,
            &limits,
            &NeverCancel,
            work_budget
        ))
        .err()
        .unwrap()
        .kind,
        ArithmeticErrorKind::LimitExceeded {
            resource: "arithmetic work",
            ..
        }
    ));
    let terminal_budget = MqBudget {
        max_terminal_inputs: 0,
        ..budget
    };
    let mut terminal_source = Source::new(&[0, 0xff, 0xac]);
    let mut contexts = bank(&budget, &limits);
    let mut decoder = ready(MqDecoder::new(
        &mut terminal_source,
        CodedSpan {
            offset: 0,
            length: 3,
        },
        &state_table,
        &mut contexts,
        &limits,
        &NeverCancel,
        terminal_budget,
    ))
    .unwrap();
    ready(decoder.decode_bit(0)).unwrap();
    assert!(matches!(
        ready(decoder.decode_bit(0)).unwrap_err().kind,
        ArithmeticErrorKind::LimitExceeded {
            resource: "MQ terminal inputs",
            ..
        }
    ));
}

#[test]
fn cancellation_and_dropped_pending_future_poison_decoder() {
    let limits = Limits {
        io_chunk_bytes: 1,
        ..Limits::default()
    };
    let budget = MqBudget::default();
    let state_table = table();
    let flag = Rc::new(Cell::new(true));
    let signal = Flag(flag.clone());
    let mut source = Source::new(&[0xc0, 0, 0, 0xff, 0xac]);
    let mut contexts = bank(&budget, &limits);
    let error = ready(MqDecoder::new(
        &mut source,
        CodedSpan {
            offset: 0,
            length: 5,
        },
        &state_table,
        &mut contexts,
        &limits,
        &signal,
        budget,
    ))
    .err()
    .unwrap();
    assert!(matches!(error.kind, ArithmeticErrorKind::Cancelled));
    flag.set(false);
    source.cancel_after = Some((source.seen.len() + 1, flag.clone()));
    let error = ready(MqDecoder::new(
        &mut source,
        CodedSpan {
            offset: 0,
            length: 5,
        },
        &state_table,
        &mut contexts,
        &limits,
        &signal,
        budget,
    ))
    .err()
    .unwrap();
    assert!(matches!(error.kind, ArithmeticErrorKind::Cancelled));
    flag.set(false);
    source.cancel_after = None;
    source.pending_at = Some(2);
    let mut decoder = ready(MqDecoder::new(
        &mut source,
        CodedSpan {
            offset: 0,
            length: 5,
        },
        &state_table,
        &mut contexts,
        &limits,
        &signal,
        budget,
    ))
    .unwrap();
    {
        let mut future = Box::pin(decoder.decode_bit(0));
        let mut task = Context::from_waker(Waker::noop());
        assert!(matches!(future.as_mut().poll(&mut task), Poll::Pending));
    }
    assert!(decoder.snapshot().poisoned);
    assert!(matches!(
        ready(decoder.decode_bit(0)).unwrap_err().kind,
        ArithmeticErrorKind::Poisoned
    ));
}

#[test]
fn fixed_budget_mutations_terminate_and_stay_within_span() {
    let limits = Limits {
        io_chunk_bytes: 3,
        ..Limits::default()
    };
    let budget = MqBudget {
        max_span_bytes: 8,
        max_contexts: 1,
        max_symbols: 8,
        max_work: 80,
        max_terminal_inputs: 3,
    };
    let state_table = table();
    for index in 0..4 {
        for replacement in [0x00, 0x7f, 0x90, 0xff] {
            let mut bytes = [0x00, 0x00, 0xff, 0xac];
            bytes[index] = replacement;
            let mut source = Source::new(&bytes);
            let mut contexts = bank(&budget, &limits);
            let result = ready(MqDecoder::new(
                &mut source,
                CodedSpan {
                    offset: 0,
                    length: 4,
                },
                &state_table,
                &mut contexts,
                &limits,
                &NeverCancel,
                budget,
            ));
            if let Ok(mut decoder) = result {
                for _ in 0..budget.max_symbols {
                    if ready(decoder.decode_bit(0)).is_err() {
                        break;
                    }
                    assert!(decoder.snapshot().work_done <= budget.max_work);
                }
            }
            assert!(
                source
                    .seen
                    .iter()
                    .all(|(offset, count)| offset + *count as u64 <= 4)
            );
        }
    }
}

#[test]
fn wide_interval_mps_in_upper_subinterval_skips_renormalization() {
    // The third decision of the C-high-0x2080 trace above: A - Qe = 0xD801
    // keeps the MPS without shifting, reading input, or changing state.
    let limits = Limits::default();
    let budget = MqBudget::default();
    let state_table = table();
    let mut source = Source::new(&[0x41, 0x00, 0xff, 0xac]);
    let mut contexts = bank(&budget, &limits);
    let whole = span(&source);
    let mut decoder = ready(MqDecoder::new(
        &mut source,
        whole,
        &state_table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .unwrap();
    ready(decoder.decode_bit(0)).unwrap();
    ready(decoder.decode_bit(0)).unwrap();
    let first = decoder.snapshot();
    assert!(!ready(decoder.decode_bit(0)).unwrap());
    let second = decoder.snapshot();
    assert_eq!(second.interval, 0xd801);
    assert_eq!(second.bit_counter, first.bit_counter);
    assert_eq!(second.input_offset, first.input_offset);
    // Only the symbol decision itself is charged.
    assert_eq!(second.work_done, first.work_done + 1);
    assert_eq!(decoder.context(0), Some(state(2, false)));
    ready(decoder.finish(3)).unwrap();
}

#[test]
fn span_beyond_input_limit_is_rejected_before_reading() {
    let limits = Limits {
        max_input_bytes: 3,
        ..Limits::default()
    };
    let budget = MqBudget::default();
    let state_table = table();
    let mut source = Source::new(&[0, 0, 0xff, 0xac]);
    let mut contexts = bank(&budget, &limits);
    let whole = span(&source);
    let error = ready(MqDecoder::new(
        &mut source,
        whole,
        &state_table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .err()
    .unwrap();
    assert_eq!(error.offset, Some(0));
    assert!(matches!(
        error.kind,
        ArithmeticErrorKind::Source(Error::LimitExceeded {
            limit: 3,
            attempted: 4,
            ..
        })
    ));
    assert!(source.seen.is_empty());
}

#[test]
fn exhausted_work_stops_the_terminal_refill_before_reading() {
    // One-byte refills make initialization cost exactly three work units:
    // two fetched bytes and one byte-input event. The terminal check then
    // needs an uncached byte with no budget left.
    let limits = Limits {
        io_chunk_bytes: 1,
        ..Limits::default()
    };
    let budget = MqBudget {
        max_work: 3,
        ..MqBudget::default()
    };
    let state_table = table();
    let mut source = Source::new(&[0, 0, 0xff, 0xac]);
    let mut contexts = bank(&budget, &limits);
    let whole = span(&source);
    let decoder = ready(MqDecoder::new(
        &mut source,
        whole,
        &state_table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .unwrap();
    assert_eq!(decoder.snapshot().work_done, 3);
    let error = ready(decoder.finish(0)).unwrap_err();
    assert_eq!((error.offset, error.context), (Some(1), None));
    assert!(matches!(
        error.kind,
        ArithmeticErrorKind::LimitExceeded {
            resource: "arithmetic work",
            limit: 3,
            attempted: 4
        }
    ));
    assert_eq!(
        error.to_string(),
        "T.88 MQ decoder at source byte 1: arithmetic work limit 3 exceeded by 4"
    );
    assert_eq!(source.seen, [(0, 1), (1, 1)]);
}

#[test]
fn a_failed_decision_poisons_later_decisions_and_the_terminal_check() {
    let limits = Limits::default();
    let budget = MqBudget::default();
    let state_table = table();
    let mut source = Source::new(&[0, 0]);
    let mut contexts = bank(&budget, &limits);
    let whole = span(&source);
    let mut decoder = ready(MqDecoder::new(
        &mut source,
        whole,
        &state_table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .unwrap();
    ready(decoder.decode_bit(0)).unwrap();
    assert!(matches!(
        ready(decoder.decode_bit(0)).unwrap_err().kind,
        ArithmeticErrorKind::MissingTerminator
    ));
    let next = ready(decoder.decode_bit(0)).unwrap_err();
    assert!(matches!(next.kind, ArithmeticErrorKind::Poisoned));
    assert_eq!(next.context, Some(0));
    let error = ready(decoder.finish(1)).unwrap_err();
    assert!(matches!(error.kind, ArithmeticErrorKind::Poisoned));
    assert_eq!(
        error.to_string(),
        format!(
            "T.88 MQ decoder at source byte {}: decoder is poisoned after an error",
            error.offset.unwrap()
        )
    );
}

#[test]
fn configuration_errors_render_without_a_source_location() {
    use caj2pdf_core::jbig2::mq::{ArithmeticError, Coder};

    let limits = Limits::default();
    let budget = MqBudget::default();
    let zero_budget = MqBudget {
        max_symbols: 0,
        ..budget
    }
    .context_bank(1, &limits)
    .unwrap_err();
    assert_eq!(zero_budget.to_string(), "T.88 MQ decoder: invalid budget");
    let mut contexts = bank(&budget, &limits);
    let state_table = table();
    let mut source = Source::new(&[0, 0, 0]);
    let short_span = ready(MqDecoder::new(
        &mut source,
        CodedSpan {
            offset: 1,
            length: 1,
        },
        &state_table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .err()
    .unwrap();
    assert_eq!(
        short_span.to_string(),
        "T.88 MQ decoder at source byte 1: invalid span: requires at least two terminal bytes"
    );
    let cancelled = ready(MqDecoder::new(
        &mut source,
        CodedSpan {
            offset: 0,
            length: 3,
        },
        &state_table,
        &mut contexts,
        &limits,
        &Flag(Rc::new(Cell::new(true))),
        budget,
    ))
    .err()
    .unwrap();
    assert_eq!(
        cancelled.to_string(),
        "T.88 MQ decoder at source byte 0: cancelled"
    );
    assert!(source.seen.is_empty());

    let error = ArithmeticError {
        coder: Some(Coder::T88),
        offset: None,
        context: None,
        kind: ArithmeticErrorKind::AllocationFailed,
    };
    assert_eq!(
        error.to_string(),
        "T.88 MQ decoder: context allocation failed"
    );
    assert!(std::error::Error::source(&error).is_none());
}
