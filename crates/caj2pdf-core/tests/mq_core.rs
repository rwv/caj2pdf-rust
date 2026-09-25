// SPDX-License-Identifier: MIT

//! Original small examples for the table-supplied T.88 control-flow API.
//! Their invented probabilities do not establish standard compatibility.

use caj2pdf_core::{
    Cancellation, Error, Limits, NeverCancel, RangedSource,
    jbig2::mq::{
        MQ_STATE_COUNT, MqBudget, MqContext, MqContexts, MqDecoder, MqErrorKind, MqSpan, MqState,
        MqTable,
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

fn table(qe: u16) -> MqTable {
    let mut rows = vec![
        MqState {
            qe: 0x4000,
            next_mps: 0,
            next_lps: 0,
            switch_mps: false,
        };
        MQ_STATE_COUNT
    ];
    rows[0] = MqState {
        qe,
        next_mps: 2,
        next_lps: 1,
        switch_mps: true,
    };
    MqTable::new(rows, &Limits::default()).unwrap()
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
        if let Some((reads, flag)) = &self.cancel_after {
            if self.seen.len() == *reads {
                flag.set(true);
            }
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

fn bank(budget: &MqBudget, limits: &Limits) -> MqContexts {
    MqContexts::new(1, limits, budget).unwrap()
}

fn span(source: &Source) -> MqSpan {
    MqSpan {
        offset: 0,
        length: source.advertised,
    }
}

#[test]
fn initialization_mps_and_lps_exchanges_have_independent_register_expectations() {
    let limits = Limits::default();
    let budget = MqBudget::default();
    for (bytes, qe, expected_bit, expected_state, expected_mps, a, c, ct) in [
        (
            &[0x00, 0x00, 0xff, 0xac][..],
            0x4000,
            true,
            1,
            true,
            0x8000,
            0,
            0,
        ),
        (
            &[0x80, 0x00, 0xff, 0xac][..],
            0x4000,
            false,
            2,
            false,
            0x8000,
            0,
            0,
        ),
        (
            &[0x00, 0x00, 0x00, 0xff, 0xac][..],
            0x5000,
            false,
            2,
            false,
            0xa000,
            0,
            0,
        ),
        (
            &[0xc0, 0x00, 0x00, 0xff, 0xac][..],
            0x5000,
            true,
            1,
            true,
            0xc000,
            0x4000_0000,
            7,
        ),
    ] {
        let mut source = Source::new(bytes);
        let state_table = table(qe);
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
        assert_eq!(ready(decoder.decode_bit(0)).unwrap(), expected_bit);
        let progress = decoder.snapshot();
        assert_eq!(
            (progress.interval, progress.code, progress.bit_counter),
            (a, c, ct)
        );
        assert_eq!(progress.symbols_decoded, 1);
        assert_eq!(
            decoder.context(0),
            Some(MqContext {
                state_index: expected_state,
                mps: expected_mps,
            })
        );
        ready(decoder.finish(1)).unwrap();
        assert!(
            source.seen.iter().all(
                |(offset, count)| offset.checked_add(*count as u64).unwrap() <= selected.length
            )
        );
    }
}

#[test]
fn normal_and_post_ff_input_have_distinct_initial_c_and_ct() {
    let limits = Limits::default();
    let budget = MqBudget::default();
    let state_table = table(0x4000);
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
    assert_eq!(initial.current_input_offset, 1);
    assert_eq!(initial.source_bytes_fetched, 5);
    assert!(!ready(decoder.decode_bit(0)).unwrap());
    let after = decoder.snapshot();
    assert_eq!(after.current_input_offset, 2);
    assert_eq!(after.bit_counter, 7);
    ready(decoder.finish(1)).unwrap();
}

#[test]
fn terminal_input_is_counted_and_marker_is_bounded() {
    let limits = Limits::default();
    let budget = MqBudget::default();
    let state_table = table(0x5000);
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
    assert_eq!(progress.current_input_offset, 1);
    assert_eq!(progress.source_bytes_fetched, 3);
    assert_eq!(progress.terminal_inputs, 1);
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
        MqSpan {
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
    assert!(matches!(error.kind, MqErrorKind::InvalidMarker(0x90)));
    assert_eq!(error.offset, Some(1));
}

#[test]
fn short_zero_and_overreported_reads_are_checked() {
    let limits = Limits::default();
    let budget = MqBudget::default();
    let state_table = table(0x4000);
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
    assert!(ready(decoder.decode_bit(0)).unwrap());
    ready(decoder.finish(1)).unwrap();
    assert_eq!(source.seen.len(), 4);

    let mut truncated = Source::new(&[0]);
    truncated.advertised = 4;
    let mut contexts = bank(&budget, &limits);
    let error = ready(MqDecoder::new(
        &mut truncated,
        MqSpan {
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
        MqErrorKind::Source(Error::TruncatedInput { .. })
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
    let late_table = table(0x5000);
    let mut decoder = ready(MqDecoder::new(
        &mut late_truncation,
        MqSpan {
            offset: 0,
            length: 4,
        },
        &late_table,
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
        MqErrorKind::Source(Error::TruncatedInput { .. })
    ));
    assert_eq!(error.offset, Some(2));
    assert_eq!(decoder.snapshot().source_bytes_fetched, 2);
    assert!(decoder.snapshot().poisoned);

    let mut overreport = Source::new(&[0, 0, 0xff, 0xac]);
    overreport.overreport = true;
    let mut contexts = bank(&budget, &limits);
    let error = ready(MqDecoder::new(
        &mut overreport,
        MqSpan {
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
        MqErrorKind::Source(Error::InvalidInput { .. })
    ));
}

#[test]
fn finish_and_context_errors_keep_public_locations() {
    use std::error::Error as _;

    let limits = Limits::default();
    let budget = MqBudget::default();
    let state_table = table(0x4000);
    let mut contexts = bank(&budget, &limits);
    contexts
        .set(
            0,
            MqContext {
                state_index: 5,
                mps: true,
            },
        )
        .unwrap();
    assert_eq!(contexts.get(0).unwrap().state_index, 5);
    contexts.reset();
    assert_eq!(contexts.get(0), Some(MqContext::default()));

    let mut source = Source::new(&[0, 0]);
    let mut decoder = ready(MqDecoder::new(
        &mut source,
        MqSpan {
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
    assert!(matches!(bad_context.kind, MqErrorKind::InvalidContext));
    assert_eq!(bad_context.context, Some(1));
    assert!(bad_context.to_string().contains("context 1"));
    assert!(bad_context.source().is_none());
    ready(decoder.decode_bit(0)).unwrap();
    let at_end = ready(decoder.decode_bit(0)).unwrap_err();
    assert!(matches!(at_end.kind, MqErrorKind::MissingTerminator));
    assert_eq!(at_end.offset, Some(2));
    assert_eq!(at_end.context, Some(0));
    assert!(at_end.to_string().contains("source byte 2"));

    let mut source = Source::new(&[0, 0, 0]);
    let mut contexts = bank(&budget, &limits);
    let decoder = ready(MqDecoder::new(
        &mut source,
        MqSpan {
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
        MqErrorKind::WrongSymbolCount {
            expected: 1,
            decoded: 0
        }
    ));
    assert!(wrong_count.to_string().contains("expected 1 symbols"));

    let decoder = ready(MqDecoder::new(
        &mut source,
        MqSpan {
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
    assert!(matches!(marker.kind, MqErrorKind::MissingTerminator));
    assert_eq!(marker.offset, Some(1));
    assert!(marker.to_string().contains("missing terminal marker"));

    let mut interior_marker = Source::new(&[0xff, 0xac, 0, 0xff, 0xac]);
    let error = ready(MqDecoder::new(
        &mut interior_marker,
        MqSpan {
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
    assert!(matches!(error.kind, MqErrorKind::InvalidMarker(0xac)));
    assert_eq!(error.offset, Some(1));

    // A terminal pair beyond the initial prefetch is checked only by finish.
    let mut bytes = vec![0; 300];
    bytes[298..].copy_from_slice(&[0xff, 0xab]);
    let mut late_marker = Source::new(&bytes);
    let decoder = ready(MqDecoder::new(
        &mut late_marker,
        MqSpan {
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
    assert!(matches!(error.kind, MqErrorKind::InvalidMarker(0xab)));
    assert_eq!(error.offset, Some(299));
}

#[test]
fn invalid_configuration_and_budgets_fail_without_unbounded_work() {
    let limits = Limits::default();
    let budget = MqBudget::default();
    assert!(matches!(
        MqTable::new(vec![], &limits).unwrap_err().kind,
        MqErrorKind::InvalidTable(_)
    ));
    for bad in [
        MqState {
            qe: 0,
            next_mps: 0,
            next_lps: 0,
            switch_mps: false,
        },
        MqState {
            qe: 0x8000,
            next_mps: 0,
            next_lps: 0,
            switch_mps: false,
        },
        MqState {
            qe: 1,
            next_mps: 47,
            next_lps: 0,
            switch_mps: false,
        },
        MqState {
            qe: 1,
            next_mps: 0,
            next_lps: 47,
            switch_mps: false,
        },
    ] {
        let mut rows = vec![bad; MQ_STATE_COUNT];
        rows[1].qe = 0x4000;
        assert!(matches!(
            MqTable::new(rows, &limits).unwrap_err().kind,
            MqErrorKind::InvalidTable(_)
        ));
    }
    assert!(matches!(
        MqContexts::new(0, &limits, &budget).unwrap_err().kind,
        MqErrorKind::InvalidContext
    ));
    assert!(matches!(
        MqContexts::new(usize::MAX, &limits, &budget)
            .unwrap_err()
            .kind,
        MqErrorKind::LimitExceeded { .. }
    ));
    let mut tight = limits;
    tight.max_allocation_bytes = 300;
    assert!(matches!(
        MqContexts::new(1, &tight, &budget).unwrap_err().kind,
        MqErrorKind::Source(Error::LimitExceeded { .. })
    ));
    let mut contexts = bank(&budget, &limits);
    assert!(matches!(
        contexts
            .set(
                0,
                MqContext {
                    state_index: 47,
                    mps: false
                }
            )
            .unwrap_err()
            .kind,
        MqErrorKind::InvalidState
    ));
    assert!(matches!(
        contexts.set(1, MqContext::default()).unwrap_err().kind,
        MqErrorKind::InvalidContext
    ));
    let mut source = Source::new(&[0, 0, 0xff, 0xac]);
    let state_table = table(0x4000);
    for selected in [
        MqSpan {
            offset: 0,
            length: 1,
        },
        MqSpan {
            offset: u64::MAX - 1,
            length: 3,
        },
        MqSpan {
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
        assert!(matches!(error.kind, MqErrorKind::InvalidSpan(_)));
    }
    let span_budget = MqBudget {
        max_span_bytes: 3,
        ..budget
    };
    let error = ready(MqDecoder::new(
        &mut source,
        MqSpan {
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
        MqErrorKind::LimitExceeded {
            resource: "MQ span bytes",
            ..
        }
    ));
    let invalid_budget = MqBudget {
        max_work: 0,
        ..budget
    };
    assert!(matches!(
        MqContexts::new(1, &limits, &invalid_budget)
            .unwrap_err()
            .kind,
        MqErrorKind::InvalidBudget
    ));
    let mut context_pair = MqContexts::new(2, &limits, &budget).unwrap();
    let context_budget = MqBudget {
        max_contexts: 1,
        ..budget
    };
    assert!(matches!(
        ready(MqDecoder::new(
            &mut source,
            MqSpan {
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
        MqErrorKind::InvalidContext
    ));
    let symbol_budget = MqBudget {
        max_symbols: 1,
        ..budget
    };
    let mut decoder = ready(MqDecoder::new(
        &mut source,
        MqSpan {
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
        MqErrorKind::LimitExceeded {
            resource: "MQ symbols",
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
            MqSpan {
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
        MqErrorKind::LimitExceeded {
            resource: "MQ work",
            ..
        }
    ));
    let terminal_budget = MqBudget {
        max_terminal_inputs: 0,
        ..budget
    };
    let mut terminal_source = Source::new(&[0, 0xff, 0xac]);
    let mut contexts = bank(&budget, &limits);
    let terminal_table = table(0x5000);
    let mut decoder = ready(MqDecoder::new(
        &mut terminal_source,
        MqSpan {
            offset: 0,
            length: 3,
        },
        &terminal_table,
        &mut contexts,
        &limits,
        &NeverCancel,
        terminal_budget,
    ))
    .unwrap();
    ready(decoder.decode_bit(0)).unwrap();
    assert!(matches!(
        ready(decoder.decode_bit(0)).unwrap_err().kind,
        MqErrorKind::LimitExceeded {
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
    let state_table = table(0x5000);
    let flag = Rc::new(Cell::new(true));
    let signal = Flag(flag.clone());
    let mut source = Source::new(&[0xc0, 0, 0, 0xff, 0xac]);
    let mut contexts = bank(&budget, &limits);
    let error = ready(MqDecoder::new(
        &mut source,
        MqSpan {
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
    assert!(matches!(error.kind, MqErrorKind::Cancelled));
    flag.set(false);
    source.cancel_after = Some((source.seen.len() + 1, flag.clone()));
    let error = ready(MqDecoder::new(
        &mut source,
        MqSpan {
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
    assert!(matches!(error.kind, MqErrorKind::Cancelled));
    flag.set(false);
    source.cancel_after = None;
    source.pending_at = Some(2);
    let mut decoder = ready(MqDecoder::new(
        &mut source,
        MqSpan {
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
        MqErrorKind::Poisoned
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
    let state_table = table(0x4000);
    for index in 0..4 {
        for replacement in [0x00, 0x7f, 0x90, 0xff] {
            let mut bytes = [0x00, 0x00, 0xff, 0xac];
            bytes[index] = replacement;
            let mut source = Source::new(&bytes);
            let mut contexts = bank(&budget, &limits);
            let result = ready(MqDecoder::new(
                &mut source,
                MqSpan {
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
    // Hand trace with the invented table: C starts at 0x3fbf8000 and A at
    // 0x8000. The first decision (Qe=1) is an MPS exchange that moves the
    // context to state 2 and doubles A to 0xfffe. The second decision
    // (Qe=0x4000) leaves A-Qe=0xbffe >= 0x8000 with C high above Qe, so it
    // returns the MPS unchanged without shifting or reading input.
    let limits = Limits::default();
    let budget = MqBudget::default();
    let state_table = table(1);
    let mut source = Source::new(&[0x7f, 0x7f, 0xff, 0xac]);
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
    assert_eq!(decoder.snapshot().code, 0x3fbf_8000);
    assert!(!ready(decoder.decode_bit(0)).unwrap());
    let first = decoder.snapshot();
    assert_eq!(
        (first.interval, first.code, first.bit_counter),
        (0xfffe, 0x7f7d_0000, 0)
    );
    assert_eq!(
        decoder.context(0),
        Some(MqContext {
            state_index: 2,
            mps: false
        })
    );

    assert!(!ready(decoder.decode_bit(0)).unwrap());
    let second = decoder.snapshot();
    assert_eq!(
        (second.interval, second.code, second.bit_counter),
        (0xbffe, 0x3f7d_0000, 0)
    );
    assert_eq!(second.current_input_offset, first.current_input_offset);
    // Only the symbol decision itself is charged.
    assert_eq!(second.work_done, first.work_done + 1);
    assert_eq!(
        decoder.context(0),
        Some(MqContext {
            state_index: 2,
            mps: false
        })
    );
    ready(decoder.finish(2)).unwrap();
}

#[test]
fn span_beyond_input_limit_is_rejected_before_reading() {
    let limits = Limits {
        max_input_bytes: 3,
        ..Limits::default()
    };
    let budget = MqBudget::default();
    let state_table = table(0x4000);
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
        MqErrorKind::Source(Error::LimitExceeded {
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
    let state_table = table(0x4000);
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
        MqErrorKind::LimitExceeded {
            resource: "MQ work",
            limit: 3,
            attempted: 4
        }
    ));
    assert_eq!(
        error.to_string(),
        "T.88 MQ decoder at source byte 1: MQ work limit 3 exceeded by 4"
    );
    assert_eq!(source.seen, [(0, 1), (1, 1)]);
}

#[test]
fn a_failed_decision_poisons_the_terminal_check() {
    let limits = Limits::default();
    let budget = MqBudget::default();
    let state_table = table(0x4000);
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
        MqErrorKind::MissingTerminator
    ));
    let error = ready(decoder.finish(1)).unwrap_err();
    assert!(matches!(error.kind, MqErrorKind::Poisoned));
    assert_eq!(
        error.to_string(),
        format!(
            "T.88 MQ decoder at source byte {}: decoder state is poisoned",
            error.offset.unwrap()
        )
    );
}

#[test]
fn configuration_errors_render_without_a_source_location() {
    use caj2pdf_core::jbig2::mq::MqError;

    let limits = Limits::default();
    let budget = MqBudget::default();
    let wrong_count = MqTable::new(Vec::new(), &limits).unwrap_err();
    assert_eq!(
        wrong_count.to_string(),
        "T.88 MQ decoder: invalid table: expected exactly 47 states"
    );
    let zero_budget = MqContexts::new(
        1,
        &limits,
        &MqBudget {
            max_symbols: 0,
            ..budget
        },
    )
    .unwrap_err();
    assert_eq!(
        zero_budget.to_string(),
        "T.88 MQ decoder: invalid MQ budget"
    );
    let mut contexts = bank(&budget, &limits);
    let bad_state = contexts
        .set(
            0,
            MqContext {
                state_index: MQ_STATE_COUNT as u8,
                mps: false,
            },
        )
        .unwrap_err();
    assert_eq!(
        bad_state.to_string(),
        "T.88 MQ decoder, context 0: invalid context state index"
    );
    let state_table = table(0x4000);
    let mut source = Source::new(&[0, 0, 0]);
    let short_span = ready(MqDecoder::new(
        &mut source,
        MqSpan {
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
        "T.88 MQ decoder at source byte 1: invalid MQ span: requires at least two terminal bytes"
    );
    let cancelled = ready(MqDecoder::new(
        &mut source,
        MqSpan {
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

    for (kind, detail) in [
        (MqErrorKind::AllocationFailed, "context allocation failed"),
        (
            MqErrorKind::Invariant("interval"),
            "internal invariant: interval",
        ),
    ] {
        let error = MqError {
            offset: None,
            context: None,
            kind,
        };
        assert_eq!(error.to_string(), format!("T.88 MQ decoder: {detail}"));
        assert!(std::error::Error::source(&error).is_none());
    }
}
