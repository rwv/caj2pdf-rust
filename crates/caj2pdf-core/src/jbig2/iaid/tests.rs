// SPDX-License-Identifier: MIT

use super::*;
use crate::jbig2::mq::{MQ_STATE_COUNT, MqSpan, MqState, MqTable};
use crate::{NeverCancel, native::SeekableSource};
use std::{
    future::Future,
    io::Cursor,
    pin::pin,
    task::{Context, Poll, Waker},
};

fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut task = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut task) {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("in-memory test source unexpectedly yielded"),
    }
}

struct Trace {
    bits: Vec<bool>,
    next: usize,
    contexts: Vec<usize>,
}

impl Trace {
    fn new(bits: impl IntoIterator<Item = bool>) -> Self {
        Self {
            bits: bits.into_iter().collect(),
            next: 0,
            contexts: Vec::new(),
        }
    }
}

impl DecisionSource for Trace {
    async fn bit(&mut self, context: usize) -> MqResult<bool> {
        self.contexts.push(context);
        let bit = self.bits.get(self.next).copied().ok_or(MqError {
            offset: Some(73),
            context: Some(context),
            kind: MqErrorKind::MissingTerminator,
        })?;
        self.next += 1;
        Ok(bit)
    }
}

#[test]
fn annex_a3_example_and_every_small_codeword() {
    let limits = Limits::default();
    let budget = MqBudget::default();
    let layout = IaidContextBanks::new(3, &limits, &budget).unwrap().layout();
    let mut example = Trace::new([false, true, false]);
    assert_eq!(ready(decode_decisions(&mut example, layout)).unwrap(), 2);
    assert_eq!(example.contexts, [6657, 6658, 6661]);

    for len in 0..=3u32 {
        let layout = IaidContextBanks::new(len, &limits, &budget)
            .unwrap()
            .layout();
        for value in 0..(1u64 << len) {
            let bits = (0..len).rev().map(|shift| value & (1u64 << shift) != 0);
            let mut trace = Trace::new(bits);
            assert_eq!(ready(decode_decisions(&mut trace, layout)).unwrap(), value);
            assert_eq!(trace.next, len as usize);
            let mut prev = 1usize;
            for (step, &context) in trace.contexts.iter().enumerate() {
                assert_eq!(context, INTEGER_CONTEXT_COUNT + prev);
                let bit = value & (1u64 << (len - 1 - step as u32)) != 0;
                prev = prev * 2 + usize::from(bit);
            }
        }
    }
}

#[test]
fn largest_default_layout_and_short_trace_error() {
    let limits = Limits::default();
    let budget = MqBudget::default();
    let mut owner = IaidContextBanks::with_bitmap_contexts(15, 27, &limits, &budget).unwrap();
    let layout = owner.layout();
    assert_eq!(layout.iaid_context_count(), 32_768);
    assert_eq!(layout.bitmap_base(), 39_424);
    assert_eq!(layout.total_contexts(), 39_451);
    assert_eq!(
        owner.integers.mq_contexts_mut().count(),
        layout.total_contexts()
    );
    let mut trace = Trace::new((0..15).rev().map(|shift| 0x5555u64 & (1 << shift) != 0));
    assert_eq!(ready(decode_decisions(&mut trace, layout)).unwrap(), 0x5555);
    assert_eq!(trace.contexts[0], INTEGER_CONTEXT_COUNT + 1);
    assert_eq!(trace.contexts.len(), 15);
    assert!(
        trace
            .contexts
            .iter()
            .all(|&index| index < layout.bitmap_base())
    );

    let mut short = Trace::new([false]);
    let error = ready(decode_decisions(&mut short, layout)).unwrap_err();
    assert!(matches!(error.kind, MqErrorKind::MissingTerminator));
    assert_eq!(error.offset, Some(73));
    assert_eq!(error.context, Some(INTEGER_CONTEXT_COUNT + 2));
}

#[test]
fn layout_limits_reject_before_allocation_or_input() {
    let limits = Limits::default();
    let budget = MqBudget::default();
    for len in [16, 31, 32, 63, 64, u32::MAX] {
        let error = IaidContextBanks::new(len, &limits, &budget).unwrap_err();
        assert!(matches!(
            error.kind,
            MqErrorKind::InvalidContext | MqErrorKind::LimitExceeded { .. }
        ));
    }
    // This is 32 on wasm32 and 64 on x86_64: the first invalid `usize`
    // shift fails before even considering allocation limits.
    assert!(matches!(
        IaidContextBanks::new(usize::BITS, &limits, &budget)
            .unwrap_err()
            .kind,
        MqErrorKind::InvalidContext
    ));
    assert!(matches!(
        IaidContextBanks::with_bitmap_contexts(3, usize::MAX, &limits, &budget)
            .unwrap_err()
            .kind,
        MqErrorKind::InvalidContext
    ));
    let too_few = MqBudget {
        max_contexts: INTEGER_CONTEXT_COUNT + 7,
        ..budget
    };
    assert!(matches!(
        IaidContextBanks::new(3, &limits, &too_few)
            .unwrap_err()
            .kind,
        MqErrorKind::LimitExceeded {
            resource: "MQ contexts",
            ..
        }
    ));
    let tiny = Limits {
        io_chunk_bytes: 1,
        max_allocation_bytes: 13_000,
        ..limits
    };
    assert!(matches!(
        IaidContextBanks::new(0, &tiny, &budget).unwrap_err().kind,
        MqErrorKind::Source(crate::Error::LimitExceeded {
            resource: "allocation bytes",
            ..
        })
    ));
}

#[test]
fn scoped_resets_do_not_erase_adjacent_models() {
    let limits = Limits::default();
    let budget = MqBudget::default();
    let mut owner = IaidContextBanks::with_bitmap_contexts(3, 1, &limits, &budget).unwrap();
    let layout = owner.layout();
    let active = MqContext {
        state_index: 2,
        mps: true,
    };
    for index in [1, layout.iaid_base() + 1, layout.bitmap_base()] {
        owner.mq_contexts_mut().set(index, active).unwrap();
    }
    owner.reset_non_iaid_integer_contexts().unwrap();
    assert_eq!(owner.mq_contexts_mut().get(1), Some(MqContext::default()));
    assert_eq!(
        owner.mq_contexts_mut().get(layout.iaid_base() + 1),
        Some(active)
    );
    assert_eq!(
        owner.mq_contexts_mut().get(layout.bitmap_base()),
        Some(active)
    );
    owner.mq_contexts_mut().set(1, active).unwrap();
    owner.reset_iaid_contexts().unwrap();
    assert_eq!(owner.mq_contexts_mut().get(1), Some(active));
    assert_eq!(
        owner.mq_contexts_mut().get(layout.iaid_base() + 1),
        Some(MqContext::default())
    );
    assert_eq!(
        owner.mq_contexts_mut().get(layout.bitmap_base()),
        Some(active)
    );
    owner
        .mq_contexts_mut()
        .set(layout.iaid_base() + 1, active)
        .unwrap();
    owner.reset_for_symbol_dictionary().unwrap();
    assert_eq!(owner.mq_contexts_mut().get(1), Some(MqContext::default()));
    assert_eq!(
        owner.mq_contexts_mut().get(layout.iaid_base() + 1),
        Some(MqContext::default())
    );
    assert_eq!(
        owner.mq_contexts_mut().get(layout.bitmap_base()),
        Some(active)
    );
    owner.mq_contexts_mut().set(1, active).unwrap();
    owner
        .mq_contexts_mut()
        .set(layout.iaid_base() + 1, active)
        .unwrap();
    owner.reset_for_text_region();
    for index in [1, layout.iaid_base() + 1, layout.bitmap_base()] {
        assert_eq!(
            owner.mq_contexts_mut().get(index),
            Some(MqContext::default())
        );
    }
    assert_eq!(owner.layout().code_len(), 3);
}

#[test]
fn public_decoder_rejects_wrong_width_and_preserves_stream_across_ids() {
    let limits = Limits::default();
    let budget = MqBudget::default();
    let mut rows = vec![
        MqState {
            qe: 0x4000,
            next_mps: 1,
            next_lps: 1,
            switch_mps: true,
        };
        MQ_STATE_COUNT
    ];
    rows[1].next_mps = 2;
    rows[1].next_lps = 2;
    let table = MqTable::new(rows, &limits).unwrap();
    let bytes = [0x80, 0, 0, 0, 0xff, 0xac];
    let mut source = SeekableSource::new(Cursor::new(bytes)).unwrap();
    let mut owner = IaidContextBanks::new(3, &limits, &budget).unwrap();
    let layout = owner.layout();
    let changed = IaidContextBanks::new(1, &limits, &budget).unwrap().layout();
    let mut decoder = ready(MqDecoder::new(
        &mut source,
        MqSpan {
            offset: 0,
            length: bytes.len() as u64,
        },
        &table,
        owner.mq_contexts_mut(),
        &limits,
        &NeverCancel,
        budget,
    ))
    .unwrap();
    let before = decoder.snapshot();
    let error = ready(decode_iaid(&mut decoder, changed)).unwrap_err();
    assert!(matches!(error.kind, MqErrorKind::InvalidContext));
    assert_eq!(decoder.snapshot(), before);
    let first = ready(decode_iaid(&mut decoder, layout)).unwrap();
    let after_first = decoder.context(layout.iaid_base() + 1).unwrap();
    let second = ready(decode_iaid(&mut decoder, layout)).unwrap();
    let after_second = decoder.context(layout.iaid_base() + 1).unwrap();
    assert!(first < 8 && second < 8);
    assert_ne!(after_first, MqContext::default());
    assert_ne!(after_first, after_second);
    assert_eq!(decoder.snapshot().symbols_decoded, 6);
    ready(decoder.finish(6)).unwrap();
}
