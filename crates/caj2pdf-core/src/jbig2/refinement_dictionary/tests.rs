// SPDX-License-Identifier: MIT

use super::*;
use crate::NeverCancel;
use std::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};

fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("unexpected pending test I/O"),
    }
}

struct TinySource;

impl RangedSource for TinySource {
    fn size(&self) -> u64 {
        2
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> crate::Result<usize> {
        let source = [0xff, 0xac];
        let offset = usize::try_from(offset).unwrap_or(usize::MAX);
        let count = source.len().saturating_sub(offset).min(destination.len());
        if count != 0 {
            destination[..count].copy_from_slice(&source[offset..offset + count]);
        }
        Ok(count)
    }
}

struct DiscardSink;

impl SequentialSink for DiscardSink {
    async fn write(&mut self, bytes: &[u8]) -> crate::Result<usize> {
        Ok(bytes.len())
    }
    async fn flush(&mut self) -> crate::Result<()> {
        Ok(())
    }
}

#[test]
fn signed_dimensions_keep_distinct_malformed_and_unsupported_boundaries() {
    assert!(matches!(
        checked_dimensions(-1, 1),
        Err(RefinementDictionaryErrorKind::Malformed(
            "negative symbol dimension"
        ))
    ));
    assert!(matches!(
        checked_dimensions(1, -1),
        Err(RefinementDictionaryErrorKind::Malformed(
            "negative symbol dimension"
        ))
    ));
    for (width, height) in [(0, 1), (1, 0)] {
        assert!(matches!(
            checked_dimensions(width, height),
            Err(RefinementDictionaryErrorKind::Unsupported {
                feature: "zero-dimension symbol bitmap",
                value: 0,
            })
        ));
    }
    assert!(matches!(
        checked_dimensions(i64::from(u32::MAX) + 1, 1),
        Err(RefinementDictionaryErrorKind::Malformed(
            "symbol width exceeds 32 bits"
        ))
    ));
    assert!(matches!(
        checked_dimensions(1, i64::from(u32::MAX) + 1),
        Err(RefinementDictionaryErrorKind::Malformed(
            "symbol height exceeds 32 bits"
        ))
    ));
    assert_eq!(checked_dimensions(1, 1).unwrap(), (1, 1));
    assert_eq!(
        checked_dimensions(i64::from(u32::MAX), i64::from(u32::MAX)).unwrap(),
        (u32::MAX, u32::MAX)
    );
}

#[test]
fn class_and_export_run_counts_report_their_own_overflow() {
    let progress = RefinementDictionaryProgress::default();
    assert_eq!(
        next_count(0, "height class count overflow", 7, 19, progress).unwrap(),
        1
    );
    assert_eq!(
        next_count(u32::MAX - 1, "export run count overflow", 7, 19, progress).unwrap(),
        u32::MAX
    );
    for field in ["height class count overflow", "export run count overflow"] {
        let error = next_count(u32::MAX, field, 7, 19, progress).unwrap_err();
        assert!(
            matches!(error.kind, RefinementDictionaryErrorKind::InvalidSpan(reason) if reason == field)
        );
        assert_eq!(error.segment, 7);
        assert_eq!(error.offset, 19);
        assert_eq!(*error.progress, progress);
    }
}

#[test]
fn fallible_catalog_reservation_preserves_preflight_location() {
    let site = PreflightSite {
        segment: 7,
        offset: 19,
        header_fetched: 13,
        max_allocation_bytes: u64::MAX,
    };
    let entries = reserve_catalog::<u8>(2, site).unwrap();
    assert!(entries.is_empty());
    assert!(entries.capacity() >= 2);

    let error = reserve_catalog::<u8>(usize::MAX, site).unwrap_err();
    assert!(matches!(
        error.kind,
        RefinementDictionaryErrorKind::AllocationFailed
    ));
    assert_eq!(error.segment, 7);
    assert_eq!(error.offset, 19);
    assert_eq!(error.progress.header_bytes_fetched, 13);
    assert!(error.progress.mq.is_none());

    let (new, exported) = reserve_catalogs((1, 2), site, 3).unwrap();
    assert!(new.is_empty() && exported.is_empty());
    for (new_count, exported_count) in [(usize::MAX, 0), (0, usize::MAX)] {
        let error = reserve_catalogs((new_count, exported_count), site, 0).unwrap_err();
        assert!(matches!(
            error.kind,
            RefinementDictionaryErrorKind::AllocationFailed
        ));
        assert_eq!(error.progress.header_bytes_fetched, 13);
    }
    let capped = PreflightSite {
        max_allocation_bytes: 2,
        ..site
    };
    let error = reserve_catalogs((1, 2), capped, 3).unwrap_err();
    assert!(matches!(
        error.kind,
        RefinementDictionaryErrorKind::LimitExceeded {
            resource: "catalog allocation bytes",
            limit: 2,
            attempted: 3,
        }
    ));
}

#[test]
fn corrupted_internal_counters_refuse_overflow_with_poisoned_progress() {
    for (new_symbols, height_classes, export_runs, field) in [
        (1, u32::MAX, 0, "height class count overflow"),
        (0, 0, u32::MAX, "export run count overflow"),
    ] {
        let mut source = TinySource;
        let mut imported_source = TinySource;
        let mut new_source = TinySource;
        let mut sink = DiscardSink;
        let limits = Limits::default();
        let budget = MqBudget::default();
        let states = vec![
            MqState {
                qe: 0x4000,
                next_mps: 1,
                next_lps: 1,
                switch_mps: false,
            };
            MQ_STATE_COUNT
        ];
        let table = MqTable::new(states, &limits).unwrap();
        let mut banks =
            IaidContextBanks::with_bitmap_contexts(0, GR_CONTEXTS, &limits, &budget).unwrap();
        let layout = banks.layout();
        let mq = ready(MqDecoder::new(
            &mut source,
            MqSpan {
                offset: 0,
                length: 2,
            },
            &table,
            banks.mq_contexts_mut(),
            &limits,
            &NeverCancel,
            budget,
        ))
        .unwrap();
        let header = DictionaryDataHeader {
            flags: 0x1802,
            mode: DictionaryMode::ArithmeticRefinementAggregate,
            template: 2,
            refinement_template: 1,
            bitmap_context_used: false,
            bitmap_context_retained: false,
            at: [(2, -1); 4],
            at_count: 1,
            refinement_at: [(0, 0); 2],
            refinement_at_count: 0,
            exported_symbols: 0,
            new_symbols,
            header_bytes: 0,
            body: super::super::SegmentSpan {
                offset: 0,
                length: 2,
            },
        };
        let mut decoder = RefinementDictionaryDecoder {
            mq,
            imported_source: &mut imported_source,
            new_source: &mut new_source,
            new_sink: &mut sink,
            imported: &[],
            imported_base: 0,
            new_base: 0,
            layout,
            header,
            segment: 7,
            limits: &limits,
            cancellation: &NeverCancel,
            dictionary_budget: DictionaryBudget::default(),
            refinement_budget: RefinementBudget::default(),
            budget: RefinementDictionaryBudget::default(),
            base_working: 0,
            progress: RefinementDictionaryProgress {
                height_classes,
                export_runs,
                ..RefinementDictionaryProgress::default()
            },
            refinement_observer: RefinementProgress::default(),
            catalog: RefinementDictionaryCatalog {
                new_symbols: Vec::new(),
                exported_symbols: Vec::new(),
            },
            poisoned: false,
            complete: false,
        };
        let error = ready(decoder.decode()).unwrap_err();
        assert!(
            matches!(error.kind, RefinementDictionaryErrorKind::InvalidSpan(reason) if reason == field)
        );
        assert_eq!(error.segment, 7);
        assert!(error.progress.poisoned);
        assert_eq!(error.progress.height_classes, height_classes);
        assert_eq!(error.progress.export_runs, export_runs);
    }
}
