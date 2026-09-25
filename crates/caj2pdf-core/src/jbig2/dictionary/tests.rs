// SPDX-License-Identifier: MIT

use super::*;

const HEADER: DictionaryDataHeader = DictionaryDataHeader {
    flags: 0,
    mode: DictionaryMode::ArithmeticDirect,
    template: 0,
    refinement_template: 0,
    bitmap_context_used: false,
    bitmap_context_retained: false,
    at: [(0, 0); 4],
    at_count: 0,
    refinement_at: [(0, 0); 2],
    refinement_at_count: 0,
    exported_symbols: 0,
    new_symbols: 1,
    header_bytes: 0,
    body: SegmentSpan {
        offset: 0,
        length: 0,
    },
};

#[test]
fn every_symbol_size_budget_is_checked_before_bitmap_work() {
    let budget = DictionaryBudget::default();
    let limits = Limits::default();
    let progress = DictionaryProgress::default();
    let wide = i64::from(u32::MAX) + 1;
    let cases = [
        (
            (-1, 1),
            budget,
            limits,
            progress,
            "negative symbol dimension",
        ),
        (
            (0, 1),
            budget,
            limits,
            progress,
            "zero-dimension symbol bitmap",
        ),
        (
            (wide, 1),
            budget,
            limits,
            progress,
            "symbol width exceeds 32 bits",
        ),
        (
            (1, wide),
            budget,
            limits,
            progress,
            "symbol height exceeds 32 bits",
        ),
        ((40_000, 1), budget, limits, progress, "symbol width"),
        ((1, 40_000), budget, limits, progress, "symbol height"),
        ((4_000, 4_000), budget, limits, progress, "symbol pixels"),
        (
            (8, 8),
            budget,
            limits,
            DictionaryProgress {
                decoded_pixels: u64::MAX,
                ..progress
            },
            "total pixel count overflow",
        ),
        (
            (8, 8),
            DictionaryBudget {
                max_total_pixels: 63,
                ..budget
            },
            limits,
            progress,
            "dictionary pixels",
        ),
        (
            (8, 8),
            DictionaryBudget {
                max_bytes_per_symbol: 7,
                ..budget
            },
            limits,
            progress,
            "symbol bytes",
        ),
        (
            (8, 8),
            budget,
            limits,
            DictionaryProgress {
                stored_bitmap_bytes: u64::MAX,
                ..progress
            },
            "stored byte count overflow",
        ),
        (
            (8, 8),
            DictionaryBudget {
                max_stored_bitmap_bytes: 7,
                ..budget
            },
            limits,
            progress,
            "stored bitmap bytes",
        ),
        (
            (8, 8),
            budget,
            Limits {
                max_output_bytes: 7,
                ..limits
            },
            progress,
            "output bytes",
        ),
        (
            (8, 8),
            budget,
            Limits {
                max_allocation_bytes: 2,
                ..limits
            },
            progress,
            "row scratch bytes",
        ),
        (
            (8, 8),
            DictionaryBudget {
                max_working_bytes: 0,
                ..budget
            },
            limits,
            progress,
            "dictionary working bytes",
        ),
    ];
    for ((width, height), budget, limits, progress, expected) in cases {
        let result = symbol_geometry(width, height, &budget, &limits, &HEADER, &progress);
        // Each expected text is the rejection's resource, reason, or feature.
        let text = format!("{result:?}");
        assert!(
            result.is_err() && text.contains(&format!("{expected:?}")),
            "{width}x{height}: {text}"
        );
    }
    assert!(matches!(
        symbol_geometry(9, 2, &budget, &limits, &HEADER, &progress),
        Ok((9, 2, 2, 18, 4))
    ));
}

#[test]
fn a_refused_catalog_reservation_reports_the_fetched_header() {
    let segment = SegmentHeader {
        number: 7,
        segment_type: 0,
        deferred_non_retain: false,
        page_association: 1,
        referred_to: Vec::new(),
        data: SegmentSpan {
            offset: 11,
            length: 0,
        },
        header_length: 11,
        retention: Vec::new(),
    };
    let error = allocation_failed(&segment, 42, 29);
    assert_eq!((error.segment, error.offset), (7, 42));
    assert!(matches!(error.kind, DictionaryErrorKind::AllocationFailed));
    assert_eq!(error.progress.header_bytes_fetched, 29);
}
