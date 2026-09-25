// SPDX-License-Identifier: MIT

//! Adversarial public API tests with invented MQ states and synthetic segments.
//! These bytes are test inputs, not T.88 Table E.1 or external document data.

use caj2pdf_core::jbig2::{
    HeaderErrorKind, HeaderLimits, SegmentHeader, SegmentSpan,
    dictionary::{
        DictionaryBudget, DictionaryError, DictionaryErrorKind, DictionaryReport,
        DirectDictionaryDecoder, read_dictionary_data_header,
    },
    integer::IntegerContextBanks,
    mq::{MQ_STATE_COUNT, MqBudget, MqErrorKind, MqState, MqTable},
    read_segment_header,
};
use caj2pdf_core::{Cancellation, Limits, NeverCancel, RangedSource, SequentialSink};
use std::{
    cell::Cell,
    future::Future,
    io,
    pin::pin,
    rc::Rc,
    task::{Context, Poll, Waker},
};

const ONE_SYMBOL: [u8; 14] = [
    0xee, 0xbf, 0x41, 0xc7, 0x00, 0x54, 0x0f, 0xe4, 0x11, 0x07, 0x7f, 0x2f, 0xff, 0xac,
];
const TWO_SYMBOLS: [u8; 14] = [
    0xee, 0x7d, 0xf6, 0xc9, 0x51, 0xf2, 0x81, 0xb1, 0x95, 0x2a, 0x6d, 0x8d, 0xff, 0xac,
];
const NEGATIVE_WIDTH: [u8; 14] = [
    0xe6, 0xd9, 0x3b, 0xda, 0xe3, 0x82, 0x89, 0xb7, 0xd5, 0x17, 0xef, 0xc7, 0xff, 0xac,
];
const NEGATIVE_HEIGHT: [u8; 14] = [
    0x6d, 0x61, 0x00, 0x6d, 0xc7, 0x0e, 0x4b, 0x54, 0xef, 0x9a, 0x31, 0x9a, 0xff, 0xac,
];
const WIDTH_NINE: [u8; 14] = [
    0xeb, 0x44, 0x77, 0xe9, 0x24, 0x1c, 0x8a, 0x3b, 0xa2, 0xf0, 0xad, 0xf4, 0xff, 0xac,
];

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

#[derive(Clone, Copy)]
enum Fault {
    Io,
    Cancelled,
}

impl Fault {
    fn error(self) -> caj2pdf_core::Error {
        match self {
            Self::Io => caj2pdf_core::Error::Io(io::Error::other("injected I/O failure")),
            Self::Cancelled => caj2pdf_core::Error::Cancelled,
        }
    }
}

struct Source {
    bytes: Vec<u8>,
    size_override: Option<u64>,
    visible_end: usize,
    max_read: usize,
    overreport_from: Option<u64>,
    fault_from: Option<(u64, Fault)>,
    cancel_after_read: Option<(u64, Rc<Cell<bool>>)>,
    max_request: usize,
    max_offset: u64,
    read_calls: usize,
}

impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.size_override.unwrap_or(self.bytes.len() as u64)
    }

    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        self.read_calls += 1;
        self.max_request = self.max_request.max(destination.len());
        self.max_offset = self.max_offset.max(offset);
        if let Some((from, fault)) = self.fault_from {
            if offset >= from {
                return Err(fault.error());
            }
        }
        if self.overreport_from.is_some_and(|from| offset >= from) {
            return Ok(destination.len() + 1);
        }
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let count = self
            .visible_end
            .saturating_sub(start)
            .min(destination.len())
            .min(self.max_read);
        if count != 0 {
            destination[..count].copy_from_slice(&self.bytes[start..start + count]);
            if let Some((from, flag)) = &self.cancel_after_read {
                if offset >= *from {
                    flag.set(true);
                }
            }
        }
        Ok(count)
    }
}

#[derive(Default)]
struct Store {
    bytes: Vec<u8>,
    flushed: bool,
    write_fault: Option<Fault>,
    flush_fault: Option<Fault>,
}

impl SequentialSink for Store {
    async fn write(&mut self, bytes: &[u8]) -> caj2pdf_core::Result<usize> {
        if let Some(fault) = self.write_fault {
            return Err(fault.error());
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    async fn flush(&mut self) -> caj2pdf_core::Result<()> {
        if let Some(fault) = self.flush_fault {
            return Err(fault.error());
        }
        self.flushed = true;
        Ok(())
    }
}

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
    Source {
        visible_end: bytes.len(),
        bytes,
        size_override: None,
        max_read: usize::MAX,
        overreport_from: None,
        fault_from: None,
        cancel_after_read: None,
        max_request: 0,
        max_offset: 0,
        read_calls: 0,
    }
}

fn header(source: &mut Source) -> SegmentHeader {
    ready(read_segment_header(
        source,
        SegmentSpan {
            offset: 0,
            length: source.size(),
        },
        &Limits::default(),
        HeaderLimits::default(),
        &NeverCancel,
    ))
    .unwrap()
}

fn table() -> MqTable {
    let mut states = vec![
        MqState {
            qe: 0x4000,
            next_mps: 0,
            next_lps: 0,
            switch_mps: false
        };
        MQ_STATE_COUNT
    ];
    states[0].next_mps = 1;
    states[0].next_lps = 1;
    states[1].next_mps = 1;
    states[1].next_lps = 1;
    MqTable::new(states, &Limits::default()).unwrap()
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
        Store::default(),
        1024,
    )
}

fn observe_custom(
    mut source: Source,
    header: &SegmentHeader,
    limits: &Limits,
    mq_budget: MqBudget,
    budget: DictionaryBudget,
    mut store: Store,
    extra_contexts: usize,
) -> Observation {
    let table = table();
    let mut banks = IntegerContextBanks::with_extra_contexts(
        extra_contexts,
        &Limits::default(),
        &MqBudget::default(),
    )
    .unwrap();
    let created = ready(DirectDictionaryDecoder::new(
        &mut source,
        header,
        &table,
        &mut banks,
        &mut store,
        limits,
        &NeverCancel,
        mq_budget,
        budget,
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
        DictionaryErrorKind::Mq(ref error) if matches!(error.kind, MqErrorKind::Source(_))));
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
        assert_eq!(
            error.progress.header_bytes_fetched,
            segment.header_length + 12
        );
        assert_eq!(error.progress.mq_initialization_bytes_fetched, 1);
        assert!(error.progress.mq.is_none());
        assert_eq!(
            error.progress.source_bytes_fetched(),
            segment.header_length + 13
        );
        assert!(observation.store.bytes.is_empty());
        assert!(!observation.store.flushed);
    }
}

#[test]
fn framing_reparse_faults_preserve_location_and_fetched_progress() {
    let mut input = source(&ONE_SYMBOL, 1, 1, (2, -1), &[]);
    let segment = header(&mut input);
    input.max_read = 1;
    input.fault_from = Some((0, Fault::Io));
    let observation = defaults(input, &segment);
    let error = observation.result.unwrap_err();
    assert_nested_error(&error, "segment header:");
    assert!(matches!(error.kind,
        DictionaryErrorKind::Header(ref header) if matches!(header.kind, HeaderErrorKind::Source(_))));
    assert_eq!(
        (
            error.segment,
            error.offset,
            error.progress.header_bytes_fetched
        ),
        (1, 0, 0)
    );
    assert!(observation.store.bytes.is_empty());

    let mut input = source(&ONE_SYMBOL, 1, 1, (2, -1), &[]);
    let segment = header(&mut input);
    input.max_read = 1;
    input.visible_end = 3;
    let observation = defaults(input, &segment);
    let error = observation.result.unwrap_err();
    assert!(matches!(error.kind,
        DictionaryErrorKind::Header(ref header) if matches!(header.kind, HeaderErrorKind::Truncated(_))));
    assert_eq!(error.progress.header_bytes_fetched, 3);
    assert!(observation.store.bytes.is_empty());

    let mut input = source(&ONE_SYMBOL, 1, 1, (2, -1), &[]);
    let segment = header(&mut input);
    input.bytes[6] = 0; // still a valid global page, but not the validated header
    let observation = defaults(input, &segment);
    let error = observation.result.unwrap_err();
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Malformed("segment header metadata mismatch")
    ));
    assert_eq!(error.progress.header_bytes_fetched, segment.header_length);
    assert!(observation.source.max_offset < segment.data.offset);
    assert!(observation.store.bytes.is_empty());

    let mut input = source(&ONE_SYMBOL, 1, 1, (2, -1), &[]);
    let segment = header(&mut input);
    input.overreport_from = Some(0);
    let observation = defaults(input, &segment);
    let error = observation.result.unwrap_err();
    assert!(matches!(error.kind,
        DictionaryErrorKind::Header(ref header) if matches!(header.kind, HeaderErrorKind::Malformed("source returned more bytes than requested"))));
    assert_eq!(error.progress.header_bytes_fetched, 0);
    assert!(observation.store.bytes.is_empty());
}

#[test]
fn stale_source_size_and_forged_segment_spans_fail_before_io() {
    let mut input = source(&ONE_SYMBOL, 1, 1, (2, -1), &[]);
    let segment = header(&mut input);
    input.size_override = Some(input.bytes.len() as u64 - 1);
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

    let mut input = source(&ONE_SYMBOL, 1, 1, (2, -1), &[]);
    let mut segment = header(&mut input);
    segment.data.offset = 0;
    input.read_calls = 0;
    let observation = defaults(input, &segment);
    let error = observation.result.unwrap_err();
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::InvalidSpan("segment header start underflow")
    ));
    assert_eq!(error.offset, 0);
    assert_eq!(observation.source.read_calls, 0);
}

struct CancelNow;

impl Cancellation for CancelNow {
    fn is_cancelled(&self) -> bool {
        true
    }
}

struct Flag(Rc<Cell<bool>>);

impl Cancellation for Flag {
    fn is_cancelled(&self) -> bool {
        self.0.get()
    }
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
        &CancelNow,
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
        &Flag(cancelled),
    ))
    .unwrap_err();
    assert!(matches!(error.kind, DictionaryErrorKind::Cancelled));
    assert_eq!(error.offset, segment.data.offset + 1);
    assert_eq!(
        error.progress.header_bytes_fetched,
        segment.header_length + 1
    );
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
        assert_eq!(
            error.progress.header_bytes_fetched,
            segment.header_length + 4
        );
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
        Store::default(),
        0,
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
                ..Store::default()
            },
            1024,
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
            ..Store::default()
        },
        1024,
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
        let index = (state as usize) % 12;
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
                    error.progress.source_bytes_fetched() <= segment.header_length * 2 + 12 + 14,
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
    // Each input changes one byte of an invented one- or two-symbol stream.
    // The failures exercise the model's signed OOB and export-run boundaries.
    let cases = [
        (
            [
                0x71, 0x7d, 0xf6, 0xc9, 0x51, 0xf2, 0x81, 0xb1, 0x95, 0x2a, 0x6d, 0x8d, 0xff, 0xac,
            ],
            "IADH out of band",
            0,
            0,
        ),
        (
            [
                0xee, 0xbb, 0xf6, 0xc9, 0x51, 0xf2, 0x81, 0xb1, 0x95, 0x2a, 0x6d, 0x8d, 0xff, 0xac,
            ],
            "IAEX out of band",
            1,
            1,
        ),
        (
            [
                0xee, 0xbf, 0xfc, 0xc7, 0x00, 0x54, 0x0f, 0xe4, 0x11, 0x07, 0x7f, 0x2f, 0xff, 0xac,
            ],
            "export run overshoot",
            1,
            3,
        ),
    ];
    for (body, expected, stored, export_runs) in cases {
        let mut input = source(&body, 1, 1, (2, -1), &[]);
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
    // The first two bodies each change one byte in a synthetic export-run
    // fixture. The unmodified fixture exports one symbol, while the segment
    // header deliberately advertises zero exports.
    let cases = [
        (
            [
                0xdf, 0x3f, 0xf7, 0xe4, 0x4f, 0x7f, 0x34, 0xf2, 0x11, 0xea, 0x2e, 0x75, 0xff, 0xac,
            ],
            "zero-dimension symbol bitmap",
            &[][..],
            0,
        ),
        (
            [
                0xee, 0x3f, 0xa1, 0xe4, 0x4f, 0x7f, 0x34, 0xf2, 0x11, 0xea, 0x2e, 0x75, 0xff, 0xac,
            ],
            "negative export run",
            &[0x80][..],
            2,
        ),
        (
            [
                0xee, 0x3f, 0xf7, 0xe4, 0x4f, 0x7f, 0x34, 0xf2, 0x11, 0xea, 0x2e, 0x75, 0xff, 0xac,
            ],
            "exported symbol total",
            &[0x80][..],
            2,
        ),
    ];
    for (body, expected, stored, export_runs) in cases {
        let mut input = source(&body, 1, 1, (2, -1), &[]);
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
