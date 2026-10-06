// SPDX-License-Identifier: MIT

use super::*;
use crate::{NeverCancel, Payload};

#[test]
fn every_symbol_size_is_checked_before_bitmap_work() {
    let limits = Limits::default();
    let wide = i64::from(u32::MAX) + 1;
    let cases = [
        ((-1, 1), "negative symbol dimension"),
        ((1, -1), "negative symbol dimension"),
        ((0, 1), "zero-dimension symbol bitmap"),
        ((1, 0), "zero-dimension symbol bitmap"),
        ((wide, 1), "symbol width exceeds 32 bits"),
        ((1, wide), "symbol height exceeds 32 bits"),
        ((4_000, 4_000), "symbol pixels"),
    ];
    for ((width, height), expected) in cases {
        let result = symbol_geometry(width, height, &limits);
        // Each expected text is the rejection's resource, reason, or feature.
        let text = format!("{result:?}");
        assert!(
            result.is_err() && text.contains(&format!("{expected:?}")),
            "{width}x{height}: {text}"
        );
    }
    assert!(matches!(symbol_geometry(9, 2, &limits), Ok((9, 2, 18, 4))));
    assert!(matches!(
        symbol_geometry(
            i64::from(u32::MAX),
            1,
            &Limits {
                max_image_pixels: u64::MAX,
                ..limits
            },
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

#[test]
fn corrupted_internal_counters_refuse_overflow_with_located_progress() {
    for (refine, new_symbols, height_classes, export_runs, field) in [
        (false, 1, u32::MAX, 0, "height class count overflow"),
        (true, 1, u32::MAX, 0, "height class count overflow"),
        (false, 0, 0, u32::MAX, "export run count overflow"),
        (true, 0, 0, u32::MAX, "export run count overflow"),
    ] {
        let source = [0xff, 0xac];
        let mut new = Vec::new();
        let limits = Limits::default();
        let table = MqTable::standard();
        let mut contexts = ContextBank::new(IAID_BASE + 1, &limits).unwrap();
        let mq = MqDecoder::new(
            Payload::from(&source[..]),
            CodedSpan {
                offset: 0,
                length: 2,
            },
            &table,
            &mut contexts,
            &limits,
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
        let decoder = SymbolDictionaryDecoder {
            mq,
            stores: DictionaryStores {
                imported: &source,
                imported_base: 0,
                new: &mut new,
                new_base: 0,
            },
            imported: &[],
            plan: Plan {
                refine,
                code_len: 0,
            },
            header,
            segment: 7,
            limits: &limits,
            cancellation: &NeverCancel,
            progress: DictionaryProgress {
                height_classes,
                export_runs,
                ..DictionaryProgress::default()
            },
            catalog: DictionaryCatalog {
                new_symbols: Vec::new(),
                exported_symbols: Vec::new(),
            },
        };
        let error = decoder.decode().unwrap_err();
        assert!(
            matches!(error.kind, DictionaryErrorKind::InvalidSpan(reason) if reason == field),
            "{error}"
        );
        assert_eq!(error.segment, 7);
        assert_eq!(error.progress.height_classes, height_classes);
        assert_eq!(error.progress.export_runs, export_runs);
    }
}
