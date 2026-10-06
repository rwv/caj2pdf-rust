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
    retry_poisoned: Option<bool>,
    source: Source,
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
        Store::unbounded(),
        IAID_BASE,
    )
}

fn observe_custom(
    mut source: Source,
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
    let mut unread = Unread::default();
    let created = ready(SymbolDictionaryDecoder::new(
        &mut source,
        header,
        None,
        direct_stores(&mut unread, &mut store),
        &table,
        &mut contexts,
        limits,
        &CancelAfter::Never,
        mq_budget,
        budget,
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    ));
    let (result, retry_poisoned) = match created {
        Ok(mut decoder) => {
            let result = ready(decoder.decode());
            let retry_poisoned = result.as_ref().err().map(|_| {
                let retry = ready(decoder.decode()).unwrap_err();
                assert!(retry.to_string().contains("poisoned"));
                matches!(retry.kind, DictionaryErrorKind::Poisoned)
            });
            (result, retry_poisoned)
        }
        Err(error) => (Err(error), None),
    };
    Observation {
        result,
        retry_poisoned,
        source,
        store,
    }
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
fn physical_source_truncation_and_overreport_are_rejected() {
    let mut input = source(&ONE_SYMBOL, 1, 1, (2, -1), &[]);
    let segment = header(&mut input);
    input.max_read = 1;
    input.visible_end = (segment.data.offset + 5) as usize;
    let observation = defaults(input, &segment);
    assert!(matches!(
        observation.result.unwrap_err().kind,
        DictionaryErrorKind::Truncated("exported symbol count")
    ));
    assert!(observation.store.bytes.is_empty());
    assert!(!observation.store.flushed);

    let mut input = source(&ONE_SYMBOL, 1, 1, (2, -1), &[]);
    let segment = header(&mut input);
    input.overreport_from = Some(segment.data.offset);
    let observation = defaults(input, &segment);
    assert!(matches!(
        observation.result.unwrap_err().kind,
        DictionaryErrorKind::Malformed("source read length")
    ));
    assert!(observation.store.bytes.is_empty());

    let mut input = source(&ONE_SYMBOL, 1, 1, (2, -1), &[]);
    let segment = header(&mut input);
    input.max_read = 1;
    input.visible_end = (segment.data.offset + 14) as usize; // only two physical MQ bytes
    let observation = defaults(input, &segment);
    let error = observation.result.unwrap_err();
    assert_nested_error(&error, "MQ:");
    assert!(matches!(error.kind,
        DictionaryErrorKind::Mq(ref error) if matches!(error.kind, ArithmeticErrorKind::Source(_))));
    assert!(observation.store.bytes.is_empty());
}

#[test]
fn failed_mq_initialization_keeps_exact_partial_body_fetch_progress() {
    for fault in [None, Some(Fault::Io), Some(Fault::Cancelled)] {
        let mut input = source(&ONE_SYMBOL, 1, 1, (2, -1), &[]);
        let segment = header(&mut input);
        let body_start = segment.data.offset + 12;
        input.max_read = 1;
        if let Some(fault) = fault {
            input.fault_from = Some((body_start + 1, fault));
        } else {
            input.visible_end = (body_start + 1) as usize;
        }
        let observation = defaults(input, &segment);
        let error = observation.result.unwrap_err();
        assert!(matches!(error.kind, DictionaryErrorKind::Mq(_)), "{error}");
        assert_eq!(error.segment, segment.number);
        assert_eq!(error.progress.header_bytes_fetched, 12);
        assert_eq!(error.progress.mq_initialization_bytes_fetched, 1);
        assert!(error.progress.mq.is_none());
        assert_eq!(error.progress.source_bytes_fetched(), 13);
        assert!(observation.store.bytes.is_empty());
        assert!(!observation.store.flushed);
    }
}

#[test]
fn stale_source_size_and_forged_segment_spans_fail_before_io() {
    let mut input = source(&ONE_SYMBOL, 1, 1, (2, -1), &[]);
    let segment = header(&mut input);
    input.advertised = input.bytes.len() as u64 - 1;
    input.read_calls = 0;
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
    assert_eq!(observation.source.read_calls, 0);

    let mut input = source(&ONE_SYMBOL, 1, 1, (2, -1), &[]);
    let mut segment = header(&mut input);
    segment.data.offset = u64::MAX - 5;
    input.read_calls = 0;
    let observation = defaults(input, &segment);
    let error = observation.result.unwrap_err();
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::InvalidSpan("data end overflow")
    ));
    assert_eq!(error.offset, segment.data.offset);
    assert_eq!(observation.source.read_calls, 0);
}

#[test]
fn cancellation_at_dictionary_entry_reads_no_header_or_body() {
    let mut input = source(&ONE_SYMBOL, 1, 1, (2, -1), &[]);
    let segment = header(&mut input);
    input.read_calls = 0;
    let error = ready(read_dictionary_data_header(
        &mut input,
        &segment,
        &Limits::default(),
        DictionaryBudget::default(),
        &CancelAfter::new(0),
    ))
    .unwrap_err();
    assert!(matches!(error.kind, DictionaryErrorKind::Cancelled));
    assert!(error.to_string().contains("cancelled"));
    assert!(std::error::Error::source(&error).is_none());
    assert_eq!((error.segment, error.offset), (1, segment.data.offset));
    assert_eq!(error.progress.source_bytes_fetched(), 0);
    assert_eq!(input.read_calls, 0);
}

#[test]
fn cancellation_after_one_header_byte_preserves_partial_progress() {
    let mut input = source(&ONE_SYMBOL, 1, 1, (2, -1), &[]);
    let segment = header(&mut input);
    let cancelled = Rc::new(Cell::new(false));
    input.max_read = 1;
    input.cancel_after_read = Some((segment.data.offset, Rc::clone(&cancelled)));
    let error = ready(read_dictionary_data_header(
        &mut input,
        &segment,
        &Limits::default(),
        DictionaryBudget::default(),
        &CancelAfter::While(cancelled),
    ))
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
        let observation = defaults(input, &segment);
        let error = observation.result.unwrap_err();
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
        assert!(observation.store.bytes.is_empty());
        assert!(!observation.store.flushed);
    }
}

#[test]
fn context_bank_count_is_checked_before_arithmetic_or_output() {
    let mut input = source(&ONE_SYMBOL, 1, 1, (2, -1), &[]);
    let segment = header(&mut input);
    input.max_offset = 0;
    let observation = observe_custom(
        input,
        &segment,
        &Limits::default(),
        MqBudget::default(),
        DictionaryBudget::default(),
        Store::unbounded(),
        IAID_BASE + 1,
    );
    let error = observation.result.unwrap_err();
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Malformed("expected exactly 7680 integer and bitmap MQ contexts")
    ));
    assert!(observation.source.max_offset < segment.data.offset + 12);
    assert!(observation.store.bytes.is_empty());
    assert!(!observation.store.flushed);
}

#[test]
fn sink_flush_failure_and_cancellation_poison_completed_bitmap() {
    for fault in [Fault::Io, Fault::Cancelled] {
        let mut input = source(&ONE_SYMBOL, 1, 1, (2, -1), &[]);
        let segment = header(&mut input);
        input.max_read = 1;
        let observation = observe_custom(
            input,
            &segment,
            &Limits::default(),
            MqBudget::default(),
            DictionaryBudget::default(),
            Store {
                flush_fault: Some(fault),
                ..Store::unbounded()
            },
            IAID_BASE,
        );
        let error = observation.result.unwrap_err();
        match fault {
            Fault::Io => {
                assert!(matches!(error.kind, DictionaryErrorKind::Sink(_)));
                assert_nested_error(&error, "sink:");
            }
            Fault::Cancelled => assert!(matches!(error.kind, DictionaryErrorKind::Cancelled)),
        }
        assert_eq!(error.progress.completed_symbols, 1);
        assert_eq!(error.progress.stored_bitmap_bytes, 1);
        assert!(error.progress.poisoned);
        assert_eq!(observation.retry_poisoned, Some(true));
        assert_eq!(observation.store.bytes, [0]);
        assert!(!observation.store.flushed);
    }
}

#[test]
fn sink_write_cancellation_does_not_claim_a_completed_bitmap() {
    let mut input = source(&ONE_SYMBOL, 1, 1, (2, -1), &[]);
    let segment = header(&mut input);
    let observation = observe_custom(
        input,
        &segment,
        &Limits::default(),
        MqBudget::default(),
        DictionaryBudget::default(),
        Store {
            write_fault: Some(Fault::Cancelled),
            ..Store::unbounded()
        },
        IAID_BASE,
    );
    let error = observation.result.unwrap_err();
    assert!(matches!(error.kind, DictionaryErrorKind::Cancelled));
    assert_eq!(error.progress.sink_writes, 1);
    assert_eq!(error.progress.completed_symbols, 0);
    assert_eq!(error.progress.stored_bitmap_bytes, 0);
    assert!(error.progress.poisoned);
    assert_eq!(observation.retry_poisoned, Some(true));
    assert!(observation.store.bytes.is_empty());
    assert!(!observation.store.flushed);
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
        input.max_offset = 0;
        let observation = defaults(input, &segment);
        let error = observation.result.unwrap_err();
        assert!(error.to_string().contains(case.expected), "{error}");
        assert!(observation.source.max_offset < segment.data.offset + 12);
        assert!(observation.store.bytes.is_empty());
        assert!(!observation.store.flushed);
    }

    let mut input = source(&ONE_SYMBOL, 1, 1, (2, -1), &[]);
    input.bytes[4] = 4; // a region segment cannot be decoded as a dictionary
    let segment = header(&mut input);
    input.max_offset = 0;
    let observation = defaults(input, &segment);
    assert!(matches!(
        observation.result.unwrap_err().kind,
        DictionaryErrorKind::Unsupported {
            feature: "segment type",
            value: 4
        }
    ));
    assert!(observation.source.max_offset < segment.data.offset);
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
        assert_eq!(observation.retry_poisoned, Some(true));
        assert!(observation.store.bytes.is_empty());
        assert!(!observation.store.flushed);
    }
}

#[test]
fn extra_width_before_oob_poisons_after_one_committed_symbol() {
    let mut input = source(&TWO_SYMBOLS, 1, 1, (2, -1), &[]);
    let segment = header(&mut input);
    input.max_read = 1;
    let observation = defaults(input, &segment);
    let error = observation.result.unwrap_err();
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Malformed("symbol-count overrun before width OOB")
    ));
    assert_eq!(error.progress.completed_symbols, 1);
    assert_eq!(error.progress.stored_bitmap_bytes, 1);
    assert_eq!(observation.retry_poisoned, Some(true));
    assert_eq!(observation.store.bytes.len(), 1);
    assert!(!observation.store.flushed);
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
        max_sink_writes: 64,
        max_source_request_bytes: 2,
        max_sink_request_bytes: 2,
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
        input.max_read = 1;
        input.max_request = 0;
        let observation = observe(input, &segment, &limits, mq_budget, budget);
        assert!(observation.source.max_request <= 2, "case {case}");
        assert!(observation.store.bytes.len() <= 32, "case {case}");
        match observation.result {
            Ok(report) => {
                assert_eq!(report.progress.completed_symbols, 1, "case {case}");
                assert!(
                    report.progress.mq.unwrap().symbols_decoded <= 256,
                    "case {case}"
                );
                assert!(observation.store.flushed, "case {case}");
            }
            Err(error) => {
                assert!(
                    error.progress.source_bytes_fetched() <= 12 + ONE_SYMBOL.len() as u64,
                    "case {case}: {error}"
                );
                assert!(
                    error.progress.mq.is_none_or(|mq| mq.symbols_decoded <= 256),
                    "case {case}"
                );
                assert!(!observation.store.flushed, "case {case}");
                if observation.retry_poisoned.is_some() {
                    assert_eq!(observation.retry_poisoned, Some(true), "case {case}");
                }
            }
        }
    }
}

#[test]
fn integer_oob_and_export_overshoot_poison_partial_store() {
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
        assert!(!observation.store.flushed);
        assert_eq!(observation.retry_poisoned, Some(true));
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
        assert!(error.progress.poisoned);
        assert_eq!(observation.retry_poisoned, Some(true));
        assert_eq!(observation.store.bytes, stored);
        assert!(!observation.store.flushed);
    }
}
