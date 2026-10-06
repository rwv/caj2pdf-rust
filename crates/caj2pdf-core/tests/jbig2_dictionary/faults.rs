// SPDX-License-Identifier: MIT

//! Adversarial public API tests with synthetic segments. These bytes are
//! test inputs, not external document data.

use super::*;

const NEGATIVE_WIDTH: [u8; 4] = [0x89, 0x7f, 0xff, 0xac];
const NEGATIVE_HEIGHT: [u8; 3] = [0xc3, 0xff, 0xac];
const WIDTH_NINE: [u8; 6] = [0x90, 0x26, 0x08, 0xbb, 0xff, 0xac];

fn source(body: &[u8], new_symbols: u32, page: u8, at: (i8, i8), references: &[u8]) -> Source {
    let mut data = vec![0x08, 0x00, at.0 as u8, at.1 as u8];
    data.extend_from_slice(&0u32.to_be_bytes()); // exported count
    data.extend_from_slice(&new_symbols.to_be_bytes());
    data.extend_from_slice(body);
    let mut bytes = vec![
        0,
        0,
        0,
        1 + references.len() as u8,
        0,
        (references.len() as u8) << 5,
    ];
    bytes.extend_from_slice(references);
    bytes.push(page);
    bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&data);
    Source::new(bytes)
}

struct Observation {
    result: Result<DictionaryReport, DictionaryError>,
    store: Store,
}

fn assert_nested_error(error: &DictionaryError, label: &str) {
    assert!(error.to_string().contains(label), "{error}");
    assert!(std::error::Error::source(error).is_some());
}

fn observe(
    source: Source,
    header: &SegmentHeader,
    limits: &Limits,
    mq_budget: MqBudget,
    budget: DictionaryBudget,
) -> Observation {
    observe_custom(
        source,
        header,
        limits,
        mq_budget,
        budget,
        Store::default(),
        IAID_BASE,
    )
}

fn observe_custom(
    source: Source,
    header: &SegmentHeader,
    limits: &Limits,
    mq_budget: MqBudget,
    budget: DictionaryBudget,
    mut store: Store,
    context_count: usize,
) -> Observation {
    let table = table();
    let mut contexts = MqBudget::default()
        .context_bank(context_count, &Limits::default())
        .unwrap();
    let unread = Unread::default();
    let result = SymbolDictionaryDecoder::new(
        source.payload(),
        header,
        None,
        direct_stores(&unread, &mut store),
        &table,
        &mut contexts,
        limits,
        &CancelAfter::Never,
        mq_budget,
        budget,
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    )
    .and_then(|decoder| decoder.decode());
    Observation { result, store }
}

fn defaults(source: Source, header: &SegmentHeader) -> Observation {
    observe(
        source,
        header,
        &Limits::default(),
        MqBudget::default(),
        DictionaryBudget::default(),
    )
}

#[test]
fn truncated_payloads_are_refused_before_mq() {
    for visible in [5, 14] {
        let mut input = source(&ONE_SYMBOL, 1, 1, (2, -1), &[]);
        let segment = header(&mut input);
        input.visible_end = (segment.data.offset + visible) as usize;
        let observation = defaults(input, &segment);
        let error = observation.result.unwrap_err();
        assert!(
            matches!(
                error.kind,
                DictionaryErrorKind::InvalidSpan("data outside source")
            ),
            "{error}"
        );
        assert!(error.progress.mq.is_none());
        assert!(observation.store.bytes.is_empty());
    }
}

#[test]
fn stale_source_size_and_forged_segment_spans_fail_before_io() {
    let mut input = source(&ONE_SYMBOL, 1, 1, (2, -1), &[]);
    let segment = header(&mut input);
    input.advertised = input.bytes.len() as u64 - 1;
    let observation = defaults(input, &segment);
    let error = observation.result.unwrap_err();
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::InvalidSpan("data outside source")
    ));
    assert!(
        error
            .to_string()
            .contains("invalid span: data outside source")
    );
    assert!(std::error::Error::source(&error).is_none());
    assert_eq!(error.offset, segment.data.offset);

    let mut input = source(&ONE_SYMBOL, 1, 1, (2, -1), &[]);
    let mut segment = header(&mut input);
    segment.data.offset = u64::MAX - 5;
    let observation = defaults(input, &segment);
    let error = observation.result.unwrap_err();
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::InvalidSpan("data end overflow")
    ));
    assert_eq!(error.offset, segment.data.offset);
}

#[test]
fn cancellation_at_dictionary_entry_reads_no_header_or_body() {
    let mut input = source(&ONE_SYMBOL, 1, 1, (2, -1), &[]);
    let segment = header(&mut input);
    input.read_calls = 0;
    let error = read_dictionary_data_header(
        &mut input,
        &segment,
        &Limits::default(),
        DictionaryBudget::default(),
        &CancelAfter::new(0),
    )
    .unwrap_err();
    assert!(matches!(error.kind, DictionaryErrorKind::Cancelled));
    assert!(error.to_string().contains("cancelled"));
    assert!(std::error::Error::source(&error).is_none());
    assert_eq!((error.segment, error.offset), (1, segment.data.offset));
    assert_eq!(error.progress.header_bytes_fetched, 0);
    assert_eq!(input.read_calls, 0);
}

#[test]
fn cancellation_after_one_header_byte_preserves_partial_progress() {
    let mut input = source(&ONE_SYMBOL, 1, 1, (2, -1), &[]);
    let segment = header(&mut input);
    let cancelled = Rc::new(Cell::new(false));
    input.max_read = 1;
    input.cancel_after_read = Some((segment.data.offset, Rc::clone(&cancelled)));
    let error = read_dictionary_data_header(
        &mut input,
        &segment,
        &Limits::default(),
        DictionaryBudget::default(),
        &CancelAfter::While(cancelled),
    )
    .unwrap_err();
    assert!(matches!(error.kind, DictionaryErrorKind::Cancelled));
    assert_eq!(error.offset, segment.data.offset + 1);
    assert_eq!(error.progress.header_bytes_fetched, 1);
    assert_eq!(input.max_offset, segment.data.offset);
}

#[test]
fn dictionary_header_source_error_and_cancellation_keep_partial_fetch_count() {
    for fault in [Fault::Io, Fault::Cancelled] {
        let mut input = source(&ONE_SYMBOL, 1, 1, (2, -1), &[]);
        let segment = header(&mut input);
        input.max_read = 1;
        input.fault_from = Some((segment.data.offset + 4, fault));
        let error = read_dictionary_data_header(
            &mut input,
            &segment,
            &Limits::default(),
            DictionaryBudget::default(),
            &CancelAfter::Never,
        )
        .unwrap_err();
        match fault {
            Fault::Io => {
                assert!(matches!(error.kind, DictionaryErrorKind::Source(_)));
                assert_nested_error(&error, "source:");
            }
            Fault::Cancelled => assert!(matches!(error.kind, DictionaryErrorKind::Cancelled)),
        }
        assert_eq!(error.segment, 1);
        assert_eq!(error.offset, segment.data.offset + 4);
        assert_eq!(error.progress.header_bytes_fetched, 4);
    }
}

#[test]
fn context_bank_count_is_checked_before_arithmetic_or_output() {
    let mut input = source(&ONE_SYMBOL, 1, 1, (2, -1), &[]);
    let segment = header(&mut input);
    let observation = observe_custom(
        input,
        &segment,
        &Limits::default(),
        MqBudget::default(),
        DictionaryBudget::default(),
        Store::default(),
        IAID_BASE + 1,
    );
    let error = observation.result.unwrap_err();
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Malformed("expected exactly 7680 integer and bitmap MQ contexts")
    ));
    assert!(observation.store.bytes.is_empty());
}

#[test]
fn page_reference_and_at_constraints_fail_before_mq() {
    struct Case {
        page: u8,
        at: (i8, i8),
        references: &'static [u8],
        expected: &'static str,
    }
    let cases = [
        Case {
            page: 0,
            at: (2, -1),
            references: &[],
            expected: "dictionary page association",
        },
        Case {
            page: 1,
            at: (2, -1),
            references: &[0],
            expected: "imported dictionary references",
        },
        Case {
            page: 1,
            at: (3, -1),
            references: &[],
            expected: "adaptive pixel",
        },
    ];
    for case in cases {
        let mut input = source(&ONE_SYMBOL, 1, case.page, case.at, case.references);
        let segment = header(&mut input);
        let observation = defaults(input, &segment);
        let error = observation.result.unwrap_err();
        assert!(error.to_string().contains(case.expected), "{error}");
        assert!(observation.store.bytes.is_empty());
    }

    let mut input = source(&ONE_SYMBOL, 1, 1, (2, -1), &[]);
    input.bytes[4] = 4; // a region segment cannot be decoded as a dictionary
    let segment = header(&mut input);
    let observation = defaults(input, &segment);
    assert!(matches!(
        observation.result.unwrap_err().kind,
        DictionaryErrorKind::Unsupported {
            feature: "segment type",
            value: 4
        }
    ));
    assert!(observation.store.bytes.is_empty());
}

#[test]
fn malformed_geometry_and_limits_fail_before_output() {
    let cases = [
        (
            &NEGATIVE_HEIGHT[..],
            DictionaryBudget::default(),
            "height class dimension",
        ),
        (
            &NEGATIVE_WIDTH[..],
            DictionaryBudget::default(),
            "negative symbol dimension",
        ),
        (
            &WIDTH_NINE[..],
            DictionaryBudget {
                max_width: 8,
                ..DictionaryBudget::default()
            },
            "symbol width limit",
        ),
        (
            &ONE_SYMBOL[..],
            DictionaryBudget {
                max_height: 0,
                ..DictionaryBudget::default()
            },
            "height class limit",
        ),
    ];
    for (body, budget, expected) in cases {
        let mut input = source(body, 1, 1, (2, -1), &[]);
        let segment = header(&mut input);
        let observation = observe(
            input,
            &segment,
            &Limits::default(),
            MqBudget::default(),
            budget,
        );
        let error = observation.result.unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
        assert!(observation.store.bytes.is_empty());
    }
}

#[test]
fn extra_width_before_oob_fails_after_one_committed_symbol() {
    let mut input = source(&TWO_SYMBOLS, 1, 1, (2, -1), &[]);
    let segment = header(&mut input);
    let observation = defaults(input, &segment);
    let error = observation.result.unwrap_err();
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Malformed("symbol-count overrun before width OOB")
    ));
    assert_eq!(error.progress.completed_symbols, 1);
    assert_eq!(error.progress.stored_bitmap_bytes, 1);
    assert_eq!(observation.store.bytes.len(), 1);
}

#[test]
fn fixed_budget_mutations_terminate_with_bounded_io_and_state() {
    // Fixed seed and 96 cases keep this useful as a regression test rather than
    // an unbounded fuzzer. Preserve the synthetic FFAC marker to reach the model.
    let mut state = 0x6a09_e667_f3bc_c909u64;
    let limits = Limits {
        io_chunk_bytes: 2,
        ..Limits::default()
    };
    let mq_budget = MqBudget {
        max_symbols: 256,
        max_work: 1024,
        max_terminal_inputs: 256,
        ..MqBudget::default()
    };
    let budget = DictionaryBudget {
        max_height_classes: 4,
        max_export_runs: 4,
        max_width: 16,
        max_height: 16,
        max_total_pixels: 256,
        max_stored_bitmap_bytes: 32,
        max_source_request_bytes: 2,
        ..DictionaryBudget::default()
    };
    for case in 0..96 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let mut body = ONE_SYMBOL;
        let index = (state as usize) % (ONE_SYMBOL.len() - 2);
        body[index] ^= (state >> 32) as u8 | 1;
        let mut input = source(&body, 1, 1, (2, -1), &[]);
        let segment = header(&mut input);
        let observation = observe(input, &segment, &limits, mq_budget, budget);
        assert!(observation.store.bytes.len() <= 32, "case {case}");
        match observation.result {
            Ok(report) => {
                assert_eq!(report.progress.completed_symbols, 1, "case {case}");
                assert!(
                    report.progress.mq.unwrap().symbols_decoded <= 256,
                    "case {case}"
                );
            }
            Err(error) => {
                assert!(
                    error.progress.header_bytes_fetched <= 12,
                    "case {case}: {error}"
                );
                assert!(
                    error.progress.mq.is_none_or(|mq| mq.symbols_decoded <= 256),
                    "case {case}"
                );
            }
        }
    }
}

#[test]
fn integer_oob_and_export_overshoot_keep_the_partial_store() {
    // Each body codes a one-symbol dictionary that ends in a refusal at the
    // model's signed OOB and export-run boundaries.
    let cases = [
        (&[0xcf, 0xff, 0xac][..], "IADH out of band", 0, 0),
        (
            &[0x94, 0xa2, 0xbf, 0xff, 0xac][..],
            "IAEX out of band",
            1,
            1,
        ),
        (
            &[0x94, 0xa8, 0x9a, 0xab, 0xff, 0xac][..],
            "export run overshoot",
            1,
            3,
        ),
    ];
    for (body, expected, stored, export_runs) in cases {
        let mut input = source(body, 1, 1, (2, -1), &[]);
        let segment = header(&mut input);
        let observation = defaults(input, &segment);
        let error = observation.result.unwrap_err();
        assert!(matches!(error.kind, DictionaryErrorKind::Malformed(reason) if reason == expected));
        assert_eq!(error.progress.stored_bitmap_bytes, stored);
        assert_eq!(error.progress.export_runs, export_runs);
        assert_eq!(observation.store.bytes.len(), stored as usize);
    }
}

#[test]
fn zero_dimension_negative_export_and_exported_total_are_typed() {
    // The bodies code a zero-width symbol, a negative export run, and a
    // fixture that exports one symbol while the segment header deliberately
    // advertises zero exports.
    let cases = [
        (
            &[0x7f, 0xff, 0xac][..],
            "zero-dimension symbol bitmap",
            &[][..],
            0,
        ),
        (
            &[0x93, 0xfc, 0x21, 0xff, 0xac][..],
            "negative export run",
            &[0x80][..],
            2,
        ),
        (
            &[0x93, 0xfc, 0x77, 0xff, 0xac][..],
            "exported symbol total",
            &[0x80][..],
            2,
        ),
    ];
    for (body, expected, stored, export_runs) in cases {
        let mut input = source(body, 1, 1, (2, -1), &[]);
        let segment = header(&mut input);
        let observation = defaults(input, &segment);
        let error = observation.result.unwrap_err();
        if expected == "zero-dimension symbol bitmap" {
            assert!(
                matches!(error.kind, DictionaryErrorKind::Unsupported { feature, value: 0 } if feature == expected)
            );
        } else {
            assert!(
                matches!(error.kind, DictionaryErrorKind::Malformed(reason) if reason == expected)
            );
        }
        assert_eq!(error.progress.export_runs, export_runs);
        assert_eq!(observation.store.bytes, stored);
    }
}
