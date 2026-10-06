// SPDX-License-Identifier: MIT

use super::*;
use crate::NeverCancel;

#[test]
fn every_symbol_size_budget_is_checked_before_bitmap_work() {
    let budget = DictionaryBudget::default();
    let wide = i64::from(u32::MAX) + 1;
    let cases = [
        ((-1, 1), budget, (0, 0), "negative symbol dimension"),
        ((1, -1), budget, (0, 0), "negative symbol dimension"),
        ((0, 1), budget, (0, 0), "zero-dimension symbol bitmap"),
        ((1, 0), budget, (0, 0), "zero-dimension symbol bitmap"),
        ((wide, 1), budget, (0, 0), "symbol width exceeds 32 bits"),
        ((1, wide), budget, (0, 0), "symbol height exceeds 32 bits"),
        ((40_000, 1), budget, (0, 0), "symbol width"),
        ((1, 40_000), budget, (0, 0), "symbol height"),
        ((4_000, 4_000), budget, (0, 0), "symbol pixels"),
        ((8, 8), budget, (u64::MAX, 0), "total pixel count overflow"),
        (
            (8, 8),
            DictionaryBudget {
                max_total_pixels: 63,
                ..budget
            },
            (0, 0),
            "dictionary pixels",
        ),
        (
            (8, 8),
            DictionaryBudget {
                max_bytes_per_symbol: 7,
                ..budget
            },
            (0, 0),
            "symbol bytes",
        ),
        ((8, 8), budget, (0, u64::MAX), "stored byte count overflow"),
        (
            (8, 8),
            DictionaryBudget {
                max_stored_bitmap_bytes: 7,
                ..budget
            },
            (0, 0),
            "stored bitmap bytes",
        ),
    ];
    for ((width, height), budget, (pixels, stored), expected) in cases {
        let result = symbol_geometry(width, height, &budget, pixels, stored);
        // Each expected text is the rejection's resource, reason, or feature.
        let text = format!("{result:?}");
        assert!(
            result.is_err() && text.contains(&format!("{expected:?}")),
            "{width}x{height}: {text}"
        );
    }
    assert!(matches!(
        symbol_geometry(9, 2, &budget, 0, 0),
        Ok((9, 2, 18, 4))
    ));
    assert!(matches!(
        symbol_geometry(
            i64::from(u32::MAX),
            1,
            &DictionaryBudget {
                max_width: u32::MAX,
                max_pixels_per_symbol: u64::MAX,
                max_total_pixels: u64::MAX,
                max_bytes_per_symbol: u64::MAX,
                max_stored_bitmap_bytes: u64::MAX,
                ..budget
            },
            0,
            0
        ),
        Ok((u32::MAX, 1, _, _))
    ));
}

#[test]
fn fallible_catalog_reservation_preserves_preflight_location() {
    let site = PreflightSite {
        segment: 7,
        offset: 19,
        header_fetched: 13,
    };
    let entries = reserve_catalog::<u8>(2, site).unwrap();
    assert!(entries.is_empty());
    assert!(entries.capacity() >= 2);

    let error = reserve_catalog::<u8>(usize::MAX, site).unwrap_err();
    assert!(matches!(error.kind, DictionaryErrorKind::AllocationFailed));
    assert_eq!((error.segment, error.offset), (7, 19));
    assert_eq!(error.progress.header_bytes_fetched, 13);
    assert!(error.progress.mq.is_none());
}

struct TinySource;

impl RangedSource for TinySource {
    fn size(&self) -> u64 {
        2
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> crate::Result<usize> {
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
    fn write(&mut self, bytes: &[u8]) -> crate::Result<usize> {
        Ok(bytes.len())
    }
    fn flush(&mut self) -> crate::Result<()> {
        Ok(())
    }
}

#[test]
fn corrupted_internal_counters_refuse_overflow_with_poisoned_progress() {
    for (refine, new_symbols, height_classes, export_runs, field) in [
        (false, 1, u32::MAX, 0, "height class count overflow"),
        (true, 1, u32::MAX, 0, "height class count overflow"),
        (false, 0, 0, u32::MAX, "export run count overflow"),
        (true, 0, 0, u32::MAX, "export run count overflow"),
    ] {
        let mut source = TinySource;
        let mut imported = TinySource;
        let mut new_reader = TinySource;
        let mut new_writer = DiscardSink;
        let limits = Limits::default();
        let table = MqTable::standard();
        let mut contexts = ContextBank::new(IAID_BASE + 1, &limits).unwrap();
        let mq = MqDecoder::new(
            &mut source,
            CodedSpan {
                offset: 0,
                length: 2,
            },
            &table,
            &mut contexts,
            &limits,
            &NeverCancel,
            MqBudget::default(),
        )
        .unwrap();
        let header = DictionaryDataHeader {
            flags: if refine { 0x1802 } else { 0x0800 },
            mode: if refine {
                DictionaryMode::ArithmeticRefinementAggregate
            } else {
                DictionaryMode::ArithmeticDirect
            },
            template: 2,
            refinement_template: u8::from(refine),
            bitmap_context_used: false,
            bitmap_context_retained: false,
            at: [(2, -1); 4],
            at_count: 1,
            refinement_at: [(0, 0); 2],
            refinement_at_count: 0,
            exported_symbols: 0,
            new_symbols,
            header_bytes: 0,
            body: SegmentSpan {
                offset: 0,
                length: 2,
            },
        };
        let mut decoder = SymbolDictionaryDecoder {
            mq,
            stores: DictionaryStores {
                imported: &mut imported,
                imported_base: 0,
                new_reader: &mut new_reader,
                new_writer: &mut new_writer,
                new_base: 0,
            },
            imported: &[],
            plan: Plan {
                refine,
                code_len: 0,
                base_working: 0,
                working_cap: u64::MAX,
                refinement_budget: RefinementBudget::default(),
            },
            header,
            segment: 7,
            limits: &limits,
            cancellation: &NeverCancel,
            budget: DictionaryBudget::default(),
            progress: DictionaryProgress {
                height_classes,
                export_runs,
                ..DictionaryProgress::default()
            },
            refinement_observer: RefinementProgress::default(),
            catalog: DictionaryCatalog {
                new_symbols: Vec::new(),
                exported_symbols: Vec::new(),
            },
            poisoned: false,
            complete: false,
        };
        let error = decoder.decode().unwrap_err();
        assert!(
            matches!(error.kind, DictionaryErrorKind::InvalidSpan(reason) if reason == field),
            "{error}"
        );
        assert_eq!(error.segment, 7);
        assert!(error.progress.poisoned);
        assert_eq!(error.progress.height_classes, height_classes);
        assert_eq!(error.progress.export_runs, export_runs);
    }
}
