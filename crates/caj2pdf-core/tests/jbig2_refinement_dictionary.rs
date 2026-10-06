// SPDX-License-Identifier: MIT

//! Original small dictionaries, MQ-coded for the standard T.88 states. The
//! source bytes here are synthetic test controls, not external document data.

mod common;

use caj2pdf_core::jbig2::{
    HeaderLimits, SegmentHeader, SegmentSpan,
    dictionary::{
        DictionaryBudget, DictionaryCatalog, DictionaryError, DictionaryErrorKind,
        DictionaryProgress, DictionaryReport, DictionaryStores, ImportedDictionary,
        RefinementDictionaryBudget, StoredSymbol, SymbolDescriptor, SymbolDictionaryDecoder,
        SymbolStore, coding_unit_contexts, read_dictionary_data_header,
    },
    iaid::IAID_BASE,
    integer::{BITMAP_BASE, INTEGER_CONTEXT_COUNT, IntegerProcedure, IntegerValue, decode_integer},
    mq::{ArithmeticSnapshot, CodedSpan, ContextBank, ContextState, MqBudget, MqDecoder, MqTable},
    read_segment_header,
    refinement::{RefinementBudget, RefinementErrorKind},
};
use caj2pdf_core::{
    Cancellation, Error, Limits, MAX_BUDGET_COUNT, NeverCancel, RangedSource, SequentialSink,
};
use std::{
    cell::{Cell, RefCell},
    future::{Future, pending},
    io,
    pin::pin,
    rc::Rc,
    task::{Context, Poll, Waker},
};

fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("unexpected pending I/O"),
    }
}

struct Bytes {
    bytes: Vec<u8>,
    advertised_size: Option<u64>,
    calls: usize,
    max_read: usize,
    fail_after_calls: Option<usize>,
}

impl Bytes {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            advertised_size: None,
            calls: 0,
            max_read: usize::MAX,
            fail_after_calls: None,
        }
    }
}

impl RangedSource for Bytes {
    fn size(&self) -> u64 {
        self.advertised_size.unwrap_or(self.bytes.len() as u64)
    }
    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        if self
            .fail_after_calls
            .is_some_and(|limit| self.calls >= limit)
        {
            self.calls += 1;
            return Err(Error::Io(io::Error::other("injected source failure")));
        }
        self.calls += 1;
        let offset = usize::try_from(offset).unwrap_or(usize::MAX);
        let count = self
            .bytes
            .len()
            .saturating_sub(offset)
            .min(destination.len())
            .min(self.max_read);
        if count > 0 {
            destination[..count].copy_from_slice(&self.bytes[offset..offset + count]);
        }
        Ok(count)
    }
}

struct SharedSource {
    bytes: Rc<RefCell<Vec<u8>>>,
    calls: usize,
    advertised_size: Option<u64>,
}
impl SharedSource {
    fn new(bytes: Rc<RefCell<Vec<u8>>>) -> Self {
        Self {
            bytes,
            calls: 0,
            advertised_size: None,
        }
    }
}
impl RangedSource for SharedSource {
    fn size(&self) -> u64 {
        self.advertised_size
            .unwrap_or_else(|| self.bytes.borrow().len() as u64)
    }
    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        self.calls += 1;
        let bytes = self.bytes.borrow();
        let offset = usize::try_from(offset).unwrap_or(usize::MAX);
        let count = bytes.len().saturating_sub(offset).min(destination.len());
        if count > 0 {
            destination[..count].copy_from_slice(&bytes[offset..offset + count]);
        }
        Ok(count)
    }
}

struct SharedSink(Rc<RefCell<Vec<u8>>>);
impl SequentialSink for SharedSink {
    async fn write(&mut self, bytes: &[u8]) -> caj2pdf_core::Result<usize> {
        self.0.borrow_mut().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    async fn flush(&mut self) -> caj2pdf_core::Result<()> {
        Ok(())
    }
}

fn segment(
    number: u8,
    reference: Option<u8>,
    flags: u16,
    exported: u32,
    new: u32,
    body: &[u8],
) -> (Bytes, SegmentHeader) {
    segment_on_page(number, reference, flags, exported, new, body, 1)
}

fn segment_on_page(
    number: u8,
    reference: Option<u8>,
    flags: u16,
    exported: u32,
    new: u32,
    body: &[u8],
    page: u8,
) -> (Bytes, SegmentHeader) {
    let mut data = Vec::new();
    data.extend_from_slice(&flags.to_be_bytes());
    data.extend_from_slice(&[2, 0xff]);
    data.extend_from_slice(&exported.to_be_bytes());
    data.extend_from_slice(&new.to_be_bytes());
    data.extend_from_slice(body);
    let ref_count = u8::from(reference.is_some());
    let mut bytes = vec![0, 0, 0, number, 0, ref_count << 5];
    if let Some(reference) = reference {
        bytes.push(reference);
    }
    bytes.push(page);
    bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&data);
    let mut source = Bytes::new(bytes);
    let span = SegmentSpan {
        offset: 0,
        length: source.size(),
    };
    let header = ready(read_segment_header(
        &mut source,
        span,
        &Limits::default(),
        HeaderLimits::default(),
        &NeverCancel,
    ))
    .unwrap();
    (source, header)
}

fn table() -> MqTable {
    MqTable::standard()
}

/// The contexts of a refinement dictionary whose IAID width is `code_len`.
fn coding_unit(code_len: u32, limits: &Limits, budget: &MqBudget) -> ContextBank {
    budget
        .context_bank(coding_unit_contexts(code_len).unwrap(), limits)
        .unwrap()
}

fn integer_prefix(body: &[u8], procedures: &[IntegerProcedure]) -> Option<Vec<IntegerValue>> {
    let mut source = Bytes::new(body.to_vec());
    let limits = Limits::default();
    let budget = MqBudget::default();
    let table = table();
    let mut contexts = coding_unit(1, &limits, &budget);
    let mut mq = ready(MqDecoder::new(
        &mut source,
        CodedSpan {
            offset: 0,
            length: body.len() as u64,
        },
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .ok()?;
    procedures
        .iter()
        .map(|&procedure| ready(decode_integer(&mut mq, procedure)).ok())
        .collect()
}

fn imported(count: u32, width: u32, row: &[u8]) -> (SegmentHeader, DictionaryReport, Bytes) {
    assert_eq!(row.len(), width.div_ceil(8) as usize);
    let (mut source, header) = segment(1, None, 0x0800, count, count, &[0x97, 0xff, 0xac]);
    let data = ready(read_dictionary_data_header(
        &mut source,
        &header,
        &Limits::default(),
        DictionaryBudget::default(),
        &NeverCancel,
    ))
    .unwrap();
    let symbols: Vec<_> = (0..count)
        .map(|index| SymbolDescriptor {
            width,
            height: 1,
            row_stride: width.div_ceil(8),
            relative_store_offset: u64::from(index) * row.len() as u64,
            stored_bytes: row.len() as u64,
        })
        .collect();
    let report = DictionaryReport {
        header: data,
        catalog: DictionaryCatalog {
            exported_symbols: symbols
                .iter()
                .map(|&symbol| StoredSymbol {
                    store: SymbolStore::New,
                    store_base: 0,
                    symbol,
                })
                .collect(),
            new_symbols: symbols,
        },
        progress: DictionaryProgress {
            completed_symbols: count,
            mq: Some(ArithmeticSnapshot {
                interval: 0x8000,
                code: 0,
                bit_counter: 0,
                input_offset: data.body.offset,
                source_bytes_fetched: 2,
                synthesized_inputs: 0,
                symbols_decoded: 0,
                work_done: 0,
                poisoned: false,
            }),
            ..DictionaryProgress::default()
        },
    };
    (header, report, Bytes::new(row.repeat(count as usize)))
}

struct Observation {
    result: Result<DictionaryReport, DictionaryError>,
    output: Vec<u8>,
    source_calls: usize,
    imported_calls: usize,
    new_calls: usize,
    gr_contexts: Vec<usize>,
    gr_state_zero: u8,
}

#[allow(clippy::too_many_arguments)]
fn run_with_imported_io(
    body: &[u8],
    imported_count: u32,
    imported_width: u32,
    imported_row: &[u8],
    new: u32,
    exported: u32,
    dict_budget: DictionaryBudget,
    refinement_budget: RefinementBudget,
    second_budget: RefinementDictionaryBudget,
    cancellation: &impl Cancellation,
    mq_budget: MqBudget,
    max_read: usize,
) -> Observation {
    let (imported_header, imported_report, mut imported_source) =
        imported(imported_count, imported_width, imported_row);
    imported_source.max_read = max_read;
    let (mut source, header) = segment(2, Some(1), 0x1802, exported, new, body);
    source.max_read = max_read;
    let store = Rc::new(RefCell::new(Vec::new()));
    let mut new_source = SharedSource::new(store.clone());
    let mut new_sink = SharedSink(store.clone());
    let table = table();
    let limits = Limits::default();
    let total = imported_count + new;
    let width = if total <= 1 {
        0
    } else {
        32 - (total - 1).leading_zeros()
    };
    let mut contexts = coding_unit(width, &limits, &mq_budget);
    let created = ready(SymbolDictionaryDecoder::new(
        &mut source,
        &header,
        Some(ImportedDictionary {
            segment: &imported_header,
            report: &imported_report,
        }),
        DictionaryStores {
            imported: &mut imported_source,
            imported_base: 0,
            new_reader: &mut new_source,
            new_writer: &mut new_sink,
            new_base: 0,
        },
        &table,
        &mut contexts,
        &limits,
        cancellation,
        mq_budget,
        dict_budget,
        refinement_budget,
        second_budget,
    ));
    let result = match created {
        Ok(mut decoder) => ready(decoder.decode()),
        Err(error) => Err(error),
    };
    let output = store.borrow().clone();
    let base = BITMAP_BASE;
    let gr_contexts: Vec<_> = (0..1024)
        .filter(|&local| contexts.get(base + local) != Some(ContextState::default()))
        .collect();
    let gr_state_zero = contexts.get(base).unwrap().state_index;
    Observation {
        result,
        output,
        source_calls: source.calls,
        imported_calls: imported_source.calls,
        new_calls: new_source.calls,
        gr_contexts,
        gr_state_zero,
    }
}

#[allow(clippy::too_many_arguments)]
fn run_with_imported(
    body: &[u8],
    imported_count: u32,
    imported_width: u32,
    imported_row: &[u8],
    new: u32,
    exported: u32,
    dict_budget: DictionaryBudget,
    refinement_budget: RefinementBudget,
    second_budget: RefinementDictionaryBudget,
) -> Observation {
    run_with_imported_io(
        body,
        imported_count,
        imported_width,
        imported_row,
        new,
        exported,
        dict_budget,
        refinement_budget,
        second_budget,
        &NeverCancel,
        MqBudget::default(),
        usize::MAX,
    )
}

#[allow(clippy::too_many_arguments)]
fn run(
    body: &[u8],
    imported_count: u32,
    new: u32,
    exported: u32,
    dict_budget: DictionaryBudget,
    refinement_budget: RefinementBudget,
    second_budget: RefinementDictionaryBudget,
) -> Observation {
    run_with_imported(
        body,
        imported_count,
        1,
        &[0x80],
        new,
        exported,
        dict_budget,
        refinement_budget,
        second_budget,
    )
}

const ONE_PREFIX: [u8; 7] = [0x95, 0x14, 0x32, 0xf5, 0x3f, 0xff, 0xac];
const ZERO_PREFIX: [u8; 4] = [0x95, 0x17, 0xff, 0xac];
const MANY_PREFIX: [u8; 4] = [0x94, 0xef, 0xff, 0xac];
const COMPLETE_ONE: [u8; 8] = [0x95, 0x10, 0x5f, 0x31, 0x1e, 0xb6, 0xff, 0xac];
const READ_REFERENCE_AND_EXPORT_NEW: [u8; 7] = [0x95, 0x11, 0x33, 0x31, 0x2f, 0xff, 0xac];
const READ_REFERENCE_PIXEL: [u8; 6] = [0x95, 0x0b, 0x27, 0x1d, 0xff, 0xac];
const COMPLETE_BOTH_EXPORTS: [u8; 7] = [0x95, 0x0c, 0x06, 0xde, 0x3f, 0xff, 0xac];
const TWO_REFINEMENTS_PREFIX: [u8; 9] = [0x95, 0x13, 0x98, 0x97, 0x8b, 0xe2, 0xbf, 0xff, 0xac];
const ZERO_NEW_EXPORT_IMPORTED: [u8; 3] = [0xa3, 0xff, 0xac];

#[test]
fn zero_new_and_zero_imported_still_consumes_one_iaex() {
    let observed = run(
        &[0x97, 0xff, 0xac],
        0,
        0,
        0,
        DictionaryBudget::default(),
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    );
    let report = observed.result.unwrap();
    assert!(report.catalog.new_symbols.is_empty());
    assert!(report.catalog.exported_symbols.is_empty());
    assert_eq!(report.progress.export_runs, 1);
    assert_eq!(report.progress.mq.unwrap().symbols_decoded, 4);
    assert!(!report.progress.poisoned);
    assert_eq!(observed.output, Vec::<u8>::new());
}

#[test]
fn iaai_zero_and_aggregation_are_located_typed_refusals() {
    let zero = run(
        &ZERO_PREFIX,
        1,
        1,
        2,
        DictionaryBudget::default(),
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    );
    let error = zero.result.unwrap_err();
    assert!(
        matches!(
            error.kind,
            DictionaryErrorKind::Malformed("REFAGGNINST zero")
        ),
        "{error}"
    );
    assert_eq!(error.progress.iaai.zero, 1);
    assert_eq!(error.progress.iaai.single_reference, 0);
    assert!(error.progress.poisoned);
    assert!(zero.output.is_empty());

    let many = run(
        &MANY_PREFIX,
        1,
        1,
        2,
        DictionaryBudget::default(),
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    );
    let error = many.result.unwrap_err();
    assert!(
        matches!(
            error.kind,
            DictionaryErrorKind::Unsupported {
                feature: "REFAGGNINST aggregation",
                value: 2
            }
        ),
        "{error}"
    );
    assert_eq!(error.progress.iaai.aggregation, 1);
    assert!(many.output.is_empty());
}

#[test]
fn one_reference_prefix_reaches_bitmap_and_stays_in_one_mq_unit() {
    let observed = run(
        &ONE_PREFIX,
        1,
        1,
        2,
        DictionaryBudget::default(),
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    );
    match observed.result {
        Ok(report) => {
            assert_eq!(report.progress.iaai.single_reference, 1);
            assert_eq!(report.progress.completed_symbols, 1);
            assert_eq!(report.catalog.new_symbols.len(), 1);
            assert_eq!(observed.output.len(), 1);
        }
        Err(error) => {
            assert_eq!(error.progress.iaai.single_reference, 1);
            assert!(error.progress.mq.unwrap().symbols_decoded > 15);
        }
    }
}

#[test]
fn one_reference_complete_stream_stores_one_packed_symbol_and_checks_tail() {
    // A bounded test-only search selected these bytes for the independent
    // control sequence IADH=1, IADW=1, IAAI=1, IAID=0, IARDX=17,
    // IARDY=-2596, GR pixel=1, IADW=OOB, IAEX=2.
    let observed = run(
        &COMPLETE_ONE,
        1,
        1,
        0,
        DictionaryBudget::default(),
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    );
    let report = observed.result.unwrap();
    assert_eq!(report.progress.iaai.single_reference, 1);
    assert_eq!(report.progress.completed_symbols, 1);
    assert_eq!(report.progress.height_classes, 1);
    assert_eq!(report.progress.export_runs, 1);
    assert_eq!(report.progress.mq.unwrap().symbols_decoded, 47);
    assert_eq!(report.progress.refinement.reference_reads, 0);
    assert_eq!(
        report.catalog.new_symbols,
        vec![SymbolDescriptor {
            width: 1,
            height: 1,
            row_stride: 1,
            relative_store_offset: 0,
            stored_bytes: 1,
        }]
    );
    assert!(report.catalog.exported_symbols.is_empty());
    assert_eq!(observed.output, vec![0x80]);
    assert!(!report.progress.poisoned);
}

#[test]
fn imported_rows_change_gr_context_before_one_refined_bitmap() {
    // IARDX=-16 and IARDY=1 align the imported pixel at (16,0) with the
    // Figure 13 (0,+1) reference tap for target (0,0).
    let set = run_with_imported(
        &READ_REFERENCE_PIXEL,
        1,
        18,
        &[0, 0, 0x80],
        1,
        0,
        DictionaryBudget::default(),
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    );
    let report = set.result.unwrap();
    assert_eq!(report.progress.completed_symbols, 1);
    assert_eq!(report.progress.refinement.reference_reads, 1);
    assert_eq!(report.progress.refinement.reference_bytes_fetched, 3);
    assert_eq!(set.gr_contexts, vec![2]);
    assert_eq!(set.output, vec![0x80]);

    let clear = run_with_imported(
        &READ_REFERENCE_PIXEL,
        1,
        18,
        &[0, 0, 0],
        1,
        0,
        DictionaryBudget::default(),
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    );
    let report = clear.result.unwrap();
    assert_eq!(report.progress.refinement.reference_reads, 1);
    assert_eq!(clear.gr_contexts, vec![0]);
}

#[test]
fn short_framing_body_and_imported_row_reads_preserve_physical_counters() {
    let observed = run_with_imported_io(
        &READ_REFERENCE_PIXEL,
        1,
        18,
        &[0, 0, 0x80],
        1,
        0,
        DictionaryBudget::default(),
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
        &NeverCancel,
        MqBudget::default(),
        1,
    );
    let report = observed.result.unwrap();
    assert_eq!(report.progress.completed_symbols, 1);
    assert_eq!(report.progress.refinement.reference_reads, 3);
    assert_eq!(report.progress.refinement.reference_bytes_fetched, 3);
    assert_eq!(
        report.progress.source_bytes_fetched(),
        12 + READ_REFERENCE_PIXEL.len() as u64
    );
    assert_eq!(observed.imported_calls, 3);
    // Every fetched byte took its own one-byte read.
    assert!(observed.source_calls as u64 >= report.progress.source_bytes_fetched());
}

#[test]
fn a_new_export_retains_the_new_store_identity() {
    let observed = run(
        &READ_REFERENCE_AND_EXPORT_NEW,
        1,
        1,
        1,
        DictionaryBudget::default(),
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    );
    let report = observed.result.unwrap();
    assert_eq!(report.catalog.exported_symbols.len(), 1);
    assert_eq!(report.catalog.exported_symbols[0].store, SymbolStore::New);
    assert_eq!(
        report.catalog.exported_symbols[0]
            .symbol
            .relative_store_offset,
        0
    );
    assert_eq!(observed.output, vec![0x80]);
}

#[test]
fn new_store_absolute_end_overflow_is_rejected_before_writing() {
    let (imported_header, imported_report, mut imported_source) = imported(1, 1, &[0x80]);
    let (mut source, header) = segment(2, Some(1), 0x1802, 1, 1, &READ_REFERENCE_AND_EXPORT_NEW);
    let store = Rc::new(RefCell::new(Vec::new()));
    let mut new_source = SharedSource::new(store.clone());
    new_source.advertised_size = Some(u64::MAX);
    let mut new_sink = SharedSink(store.clone());
    let table = table();
    let limits = Limits::default();
    let mut contexts = coding_unit(1, &limits, &MqBudget::default());
    let mut decoder = ready(SymbolDictionaryDecoder::new(
        &mut source,
        &header,
        Some(ImportedDictionary {
            segment: &imported_header,
            report: &imported_report,
        }),
        DictionaryStores {
            imported: &mut imported_source,
            imported_base: 0,
            new_reader: &mut new_source,
            new_writer: &mut new_sink,
            new_base: u64::MAX,
        },
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        MqBudget::default(),
        DictionaryBudget::default(),
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    ))
    .unwrap();
    let error = ready(decoder.decode()).unwrap_err();
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::InvalidSpan("new store absolute end overflow")
    ));
    assert_eq!(error.progress.iaai.single_reference, 0);
    assert_eq!(error.progress.completed_symbols, 0);
    assert!(error.progress.poisoned);
    assert_eq!(error.segment, 2);
    assert!(error.offset >= header.data.offset);
    assert!(store.borrow().is_empty());
}

#[test]
fn invalid_refinement_work_budget_is_located_before_bitmap_output() {
    let budget = RefinementBudget {
        max_reference_reads: MAX_BUDGET_COUNT + 1,
        ..RefinementBudget::default()
    };
    let observed = run(
        &COMPLETE_ONE,
        1,
        1,
        0,
        DictionaryBudget::default(),
        budget,
        RefinementDictionaryBudget::default(),
    );
    let error = observed.result.unwrap_err();
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Refinement(ref nested)
            if matches!(nested.kind, RefinementErrorKind::LimitExceeded {
                resource: "reference reads budget",
                limit: MAX_BUDGET_COUNT,
                attempted,
            } if attempted == MAX_BUDGET_COUNT + 1)
    ));
    assert!(error.progress.poisoned);
    assert!(observed.output.is_empty());
}

#[test]
fn one_mq_symbol_budget_covers_integer_and_iaid_decisions() {
    let mut integer_limit = false;
    let mut iaid_limit = false;
    for max_symbols in 1..64 {
        let observed = run_with_imported_io(
            &COMPLETE_ONE,
            1,
            1,
            &[0x80],
            1,
            0,
            DictionaryBudget::default(),
            RefinementBudget::default(),
            RefinementDictionaryBudget::default(),
            &NeverCancel,
            MqBudget {
                max_symbols,
                ..MqBudget::default()
            },
            usize::MAX,
        );
        let Err(error) = observed.result else {
            continue;
        };
        if let DictionaryErrorKind::Mq(mq) = &error.kind
            && let Some(context) = mq.context
        {
            integer_limit |= context < INTEGER_CONTEXT_COUNT;
            iaid_limit |=
                context >= INTEGER_CONTEXT_COUNT && error.progress.iaai.single_reference == 1;
        }
        if integer_limit && iaid_limit {
            break;
        }
    }
    assert!(integer_limit, "integer decisions must share the MQ cap");
    assert!(iaid_limit, "IAID decisions must share the same MQ cap");
}

#[test]
fn declared_catalog_allocation_is_checked_before_mq_or_store_access() {
    let setup = PreflightCase {
        new: 1_000,
        limits: Limits {
            io_chunk_bytes: 256,
            max_allocation_bytes: 20_000,
            ..Limits::default()
        },
        ..PreflightCase::default()
    };
    let error = preflight_error(setup, |_, _| {});
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::LimitExceeded {
            resource: "catalog allocation bytes",
            limit: 20_000,
            attempted,
        } if attempted > 20_000
    ));
}

#[test]
fn one_iaex_view_exports_imported_then_new_with_distinct_store_owners() {
    let observed = run(
        &COMPLETE_BOTH_EXPORTS,
        1,
        1,
        2,
        DictionaryBudget::default(),
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    );
    let report = observed.result.unwrap();
    assert_eq!(report.progress.iaai.single_reference, 1);
    assert_eq!(report.progress.export_runs, 2);
    assert_eq!(report.catalog.exported_symbols.len(), 2);
    assert_eq!(
        report.catalog.exported_symbols[0].store,
        SymbolStore::Imported
    );
    assert_eq!(
        report.catalog.exported_symbols[0].symbol,
        SymbolDescriptor {
            width: 1,
            height: 1,
            row_stride: 1,
            relative_store_offset: 0,
            stored_bytes: 1,
        }
    );
    assert_eq!(report.catalog.exported_symbols[1].store, SymbolStore::New);
    assert_eq!(
        report.catalog.exported_symbols[1].symbol,
        report.catalog.new_symbols[0]
    );
    assert_eq!(observed.output.len(), 1);
}

#[test]
fn two_refined_symbols_retain_one_gr_context_bank_until_dictionary_failure() {
    // The second symbol starts at the same width. Both signed Y offsets
    // place all reference rows outside a 1x1 import, so each one-pixel
    // bitmap uses GR context zero. Both decisions renormalize, so two uses
    // leave state 2, proving the dictionary retained GR statistics. A later
    // malformed control value is intentionally outside this prefix test.
    let observed = run(
        &TWO_REFINEMENTS_PREFIX,
        1,
        2,
        0,
        DictionaryBudget::default(),
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    );
    let error = observed.result.unwrap_err();
    assert_eq!(error.progress.completed_symbols, 2, "{error}");
    assert_eq!(error.progress.iaai.single_reference, 2);
    assert_eq!(error.progress.refinement.completed_bitmaps, 2);
    // One height class of height one and two decoded pixels means both
    // symbol widths are one; the second IADW is zero.
    assert_eq!(error.progress.height_classes, 1);
    assert_eq!(error.progress.refinement.pixels_decoded, 2);
    assert_eq!(observed.output.len(), 2);
    assert_eq!(observed.gr_contexts, vec![0]);
    assert_eq!(observed.gr_state_zero, 2);
    assert!(error.progress.poisoned);
}

#[test]
fn zero_new_symbols_can_reexport_imported_store_in_order() {
    let observed = run(
        &ZERO_NEW_EXPORT_IMPORTED,
        1,
        0,
        1,
        DictionaryBudget::default(),
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    );
    let report = observed.result.unwrap();
    assert!(report.catalog.new_symbols.is_empty());
    assert_eq!(report.progress.iaai.single_reference, 0);
    assert_eq!(report.progress.export_runs, 2);
    assert_eq!(report.catalog.exported_symbols.len(), 1);
    assert_eq!(
        report.catalog.exported_symbols[0].store,
        SymbolStore::Imported
    );
    assert_eq!(
        report.catalog.exported_symbols[0]
            .symbol
            .relative_store_offset,
        0
    );
    assert_eq!(observed.output.len(), 0);
}

#[test]
fn imported_descriptor_and_count_limits_fail_before_mq_or_reference_reads() {
    let second_budget = RefinementDictionaryBudget {
        max_imported_symbols: 0,
        ..RefinementDictionaryBudget::default()
    };
    let observed = run(
        &ONE_PREFIX,
        1,
        1,
        2,
        DictionaryBudget::default(),
        RefinementBudget::default(),
        second_budget,
    );
    let error = observed.result.unwrap_err();
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::LimitExceeded {
            resource: "imported symbols",
            ..
        }
    ));
    assert!(error.progress.mq.is_none());
    assert_eq!(observed.imported_calls, 0);
    assert!(observed.output.is_empty());
}

#[test]
fn header_and_contexts_are_read_through_public_decoder() {
    let observed = run(
        &ONE_PREFIX,
        1,
        1,
        2,
        DictionaryBudget::default(),
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    );
    assert!(observed.source_calls > 0);
    if let Ok(report) = observed.result {
        assert!(
            report
                .catalog
                .exported_symbols
                .iter()
                .all(|symbol| matches!(symbol.store, SymbolStore::Imported | SymbolStore::New))
        );
    }
}

#[test]
fn second_dictionary_clears_integer_iaid_and_gr_statistics_before_mq_work() {
    let (imported_header, imported_report, mut imported_source) = imported(1, 1, &[0x80]);
    let (mut source, header) = segment(2, Some(1), 0x1802, 1, 0, &[0x7f, 0xff, 0xac]);
    let store = Rc::new(RefCell::new(Vec::new()));
    let mut new_source = SharedSource::new(store.clone());
    let mut new_sink = SharedSink(store);
    let limits = Limits::default();
    let table = table();
    let mut contexts = coding_unit(0, &limits, &MqBudget::default());
    // An earlier coding unit adapts one integer, one bitmap, and one IAID
    // context: the first decision in a context always leaves state zero.
    let touched = [0, BITMAP_BASE, IAID_BASE];
    let earlier = [0, 0, 0xff, 0xac];
    let mut earlier_source = Bytes::new(earlier.to_vec());
    let mut mq = ready(MqDecoder::new(
        &mut earlier_source,
        CodedSpan {
            offset: 0,
            length: earlier.len() as u64,
        },
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        MqBudget::default(),
    ))
    .unwrap();
    for index in touched {
        ready(mq.decode_bit(index)).unwrap();
    }
    for index in touched {
        assert_ne!(contexts.get(index), Some(ContextState::default()));
    }
    let decoder = ready(SymbolDictionaryDecoder::new(
        &mut source,
        &header,
        Some(ImportedDictionary {
            segment: &imported_header,
            report: &imported_report,
        }),
        DictionaryStores {
            imported: &mut imported_source,
            imported_base: 0,
            new_reader: &mut new_source,
            new_writer: &mut new_sink,
            new_base: 0,
        },
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        MqBudget::default(),
        DictionaryBudget::default(),
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    ))
    .unwrap();
    drop(decoder);
    for index in touched {
        assert_eq!(contexts.get(index), Some(ContextState::default()));
    }
}

#[test]
fn resource_caps_reject_before_or_during_one_coding_unit() {
    let combined = RefinementDictionaryBudget {
        max_total_symbols: 1,
        ..RefinementDictionaryBudget::default()
    };
    let observed = run(
        &COMPLETE_ONE,
        1,
        1,
        0,
        DictionaryBudget::default(),
        RefinementBudget::default(),
        combined,
    );
    let error = observed.result.unwrap_err();
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::LimitExceeded {
            resource: "total symbols",
            ..
        }
    ));
    assert!(error.progress.mq.is_none());
    assert_eq!(observed.imported_calls, 0);

    let dictionary = DictionaryBudget {
        max_height_classes: 0,
        ..DictionaryBudget::default()
    };
    let observed = run(
        &COMPLETE_ONE,
        1,
        1,
        0,
        dictionary,
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    );
    let error = observed.result.unwrap_err();
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::LimitExceeded {
            resource: "height classes",
            ..
        }
    ));
    assert_eq!(error.progress.completed_symbols, 0);
    assert!(error.progress.poisoned);

    let refinement = RefinementBudget {
        max_reference_reads: 0,
        ..RefinementBudget::default()
    };
    let observed = run_with_imported(
        &READ_REFERENCE_PIXEL,
        1,
        18,
        &[0, 0, 0x80],
        1,
        0,
        DictionaryBudget::default(),
        refinement,
        RefinementDictionaryBudget::default(),
    );
    let error = observed.result.unwrap_err();
    assert!(matches!(error.kind, DictionaryErrorKind::Refinement(_)));
    assert_eq!(error.progress.refinement.reference_reads, 0);
    assert!(observed.output.is_empty());

    let dictionary = DictionaryBudget {
        max_export_runs: 1,
        ..DictionaryBudget::default()
    };
    let observed = run(
        &COMPLETE_BOTH_EXPORTS,
        1,
        1,
        2,
        dictionary,
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    );
    let error = observed.result.unwrap_err();
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::LimitExceeded {
            resource: "export runs",
            ..
        }
    ));
    assert_eq!(error.progress.completed_symbols, 1);
    assert_eq!(error.progress.export_runs, 1);
    assert_eq!(observed.output.len(), 1);
}

#[test]
fn fixed_budget_body_mutations_have_bounded_output_and_progress() {
    let budget = DictionaryBudget {
        max_width: 8,
        max_height: 8,
        max_pixels_per_symbol: 64,
        max_total_pixels: 64,
        max_bytes_per_symbol: 8,
        max_stored_bitmap_bytes: 8,
        max_export_runs: 8,
        ..DictionaryBudget::default()
    };
    for position in 0..COMPLETE_ONE.len() - 2 {
        for mask in [1u8, 0x80] {
            let mut body = COMPLETE_ONE;
            body[position] ^= mask;
            let observed = run(
                &body,
                1,
                1,
                0,
                budget,
                RefinementBudget::default(),
                RefinementDictionaryBudget::default(),
            );
            assert!(
                observed.output.len() <= 8,
                "position {position} mask {mask}"
            );
            let progress = match observed.result {
                Ok(report) => report.progress,
                Err(error) => error.progress.as_ref().to_owned(),
            };
            assert!(progress.source_bytes_fetched() <= 12 + body.len() as u64);
            assert!(progress.completed_symbols <= 1);
            assert!(progress.export_runs <= 8);
        }
    }
}

fn forged_error(
    count: u32,
    mutate: impl FnOnce(&mut DictionaryReport),
    budget: RefinementDictionaryBudget,
) -> DictionaryError {
    let (imported_header, mut report, mut imported_source) = imported(count, 1, &[0x80]);
    mutate(&mut report);
    let (mut source, header) = segment(2, Some(1), 0x1802, 0, 0, &[0x97, 0xff, 0xac]);
    let store = Rc::new(RefCell::new(Vec::new()));
    let mut new_source = SharedSource::new(store.clone());
    let mut new_sink = SharedSink(store);
    let limits = Limits::default();
    let table = table();
    let width = if count <= 1 {
        0
    } else {
        32 - (count - 1).leading_zeros()
    };
    let mut contexts = coding_unit(width, &limits, &MqBudget::default());
    let result = ready(SymbolDictionaryDecoder::new(
        &mut source,
        &header,
        Some(ImportedDictionary {
            segment: &imported_header,
            report: &report,
        }),
        DictionaryStores {
            imported: &mut imported_source,
            imported_base: 0,
            new_reader: &mut new_source,
            new_writer: &mut new_sink,
            new_base: 0,
        },
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        MqBudget::default(),
        DictionaryBudget::default(),
        RefinementBudget::default(),
        budget,
    ));
    let error = match result {
        Ok(_) => panic!("forged report was accepted"),
        Err(error) => error,
    };
    assert_eq!(imported_source.calls, 0);
    assert!(error.progress.mq.is_none());
    error
}

struct PreflightCase {
    flags: u16,
    reference: Option<u8>,
    page: u8,
    body: &'static [u8],
    fail_after_additional_reads: Option<usize>,
    new: u32,
    exported: u32,
    imported_base: u64,
    imported_size: Option<u64>,
    new_base: u64,
    bank_width: Option<u32>,
    dictionary_budget: DictionaryBudget,
    refinement_budget: RefinementBudget,
    second_budget: RefinementDictionaryBudget,
    limits: Limits,
}

impl Default for PreflightCase {
    fn default() -> Self {
        Self {
            flags: 0x1802,
            reference: Some(1),
            page: 1,
            body: &[0x97, 0xff, 0xac],
            fail_after_additional_reads: None,
            new: 0,
            exported: 0,
            imported_base: 0,
            imported_size: None,
            new_base: 0,
            bank_width: None,
            dictionary_budget: DictionaryBudget::default(),
            refinement_budget: RefinementBudget::default(),
            second_budget: RefinementDictionaryBudget::default(),
            limits: Limits::default(),
        }
    }
}

fn preflight_error(
    setup: PreflightCase,
    mutate_imported: impl FnOnce(&mut SegmentHeader, &mut DictionaryReport),
) -> DictionaryError {
    let error = preflight_result(setup, mutate_imported).unwrap_err();
    assert!(error.progress.mq.is_none());
    error
}

fn preflight_result(
    setup: PreflightCase,
    mutate_imported: impl FnOnce(&mut SegmentHeader, &mut DictionaryReport),
) -> Result<(), DictionaryError> {
    let (mut imported_source, mut imported_header, mut report) = {
        let (header, report, source) = imported(1, 1, &[0x80]);
        (source, header, report)
    };
    imported_source.advertised_size = setup.imported_size;
    mutate_imported(&mut imported_header, &mut report);
    let (mut source, header) = segment_on_page(
        2,
        setup.reference,
        setup.flags,
        setup.exported,
        setup.new,
        setup.body,
        setup.page,
    );
    source.fail_after_calls = setup
        .fail_after_additional_reads
        .map(|additional| source.calls + additional);
    let store = Rc::new(RefCell::new(Vec::new()));
    let mut new_source = SharedSource::new(store.clone());
    let mut new_sink = SharedSink(store);
    let limits = setup.limits;
    let table = table();
    let total = setup.new + 1;
    let width = setup.bank_width.unwrap_or(if total <= 1 {
        0
    } else {
        32 - (total - 1).leading_zeros()
    });
    let mut contexts = coding_unit(width, &limits, &MqBudget::default());
    let result = ready(SymbolDictionaryDecoder::new(
        &mut source,
        &header,
        Some(ImportedDictionary {
            segment: &imported_header,
            report: &report,
        }),
        DictionaryStores {
            imported: &mut imported_source,
            imported_base: setup.imported_base,
            new_reader: &mut new_source,
            new_writer: &mut new_sink,
            new_base: setup.new_base,
        },
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        MqBudget::default(),
        setup.dictionary_budget,
        setup.refinement_budget,
        setup.second_budget,
    ))
    .map(drop);
    assert_eq!(imported_source.calls, 0);
    result
}

#[test]
fn forged_imported_reports_fail_before_mq_and_store_access() {
    let error = forged_error(
        1,
        |report| {
            report.catalog.exported_symbols[0].symbol.row_stride = 2;
            report.catalog.new_symbols[0].row_stride = 2;
        },
        RefinementDictionaryBudget::default(),
    );
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Malformed("noncanonical imported bitmap descriptor")
    ));

    let error = forged_error(
        2,
        |report| {
            report.catalog.exported_symbols.swap(0, 1);
        },
        RefinementDictionaryBudget::default(),
    );
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Malformed("imported exports do not follow new-symbol order")
    ));

    let budget = RefinementDictionaryBudget {
        max_imported_symbols: 1,
        ..RefinementDictionaryBudget::default()
    };
    let error = forged_error(
        2,
        |report| {
            report.catalog.exported_symbols.truncate(1);
            report.header.exported_symbols = 1;
        },
        budget,
    );
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::LimitExceeded {
            resource: "imported catalog new symbols",
            ..
        }
    ));

    let error = forged_error(
        1,
        |report| {
            report.progress.poisoned = true;
        },
        RefinementDictionaryBudget::default(),
    );
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Malformed("imported dictionary is not a complete direct report")
    ));
}

#[test]
fn forged_imported_span_overflows_fail_before_arithmetic_or_store_io() {
    for (mutate, reason) in [
        (
            (|header: &mut SegmentHeader, _: &mut DictionaryReport| {
                header.data.offset = u64::MAX;
            }) as fn(&mut SegmentHeader, &mut DictionaryReport),
            "imported dictionary body offset overflow",
        ),
        (
            |_: &mut SegmentHeader, report: &mut DictionaryReport| {
                report.header.body.offset = u64::MAX;
            },
            "imported dictionary body end overflow",
        ),
        (
            |header: &mut SegmentHeader, _: &mut DictionaryReport| {
                header.data.length = u64::MAX;
            },
            "imported segment data end overflow",
        ),
    ] {
        let error = preflight_error(PreflightCase::default(), mutate);
        assert!(
            matches!(error.kind, DictionaryErrorKind::Malformed(found) if found == reason),
            "{error}"
        );
    }

    let error = forged_error(
        1,
        |report| {
            report.catalog.exported_symbols[0]
                .symbol
                .relative_store_offset = u64::MAX;
            report.catalog.new_symbols[0].relative_store_offset = u64::MAX;
        },
        RefinementDictionaryBudget::default(),
    );
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Malformed("imported descriptor end overflow")
    ));

    let error = preflight_error(
        PreflightCase {
            imported_base: u64::MAX,
            imported_size: Some(u64::MAX),
            ..PreflightCase::default()
        },
        |_, _| {},
    );
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Malformed("imported store absolute end overflow")
    ));
}

#[test]
fn imported_bitmap_metadata_and_store_bounds_are_checked_before_mq() {
    for (mutate, reason) in [
        (
            (|report: &mut DictionaryReport| {
                report.catalog.new_symbols[0].width = 0;
                report.catalog.exported_symbols[0].symbol.width = 0;
            }) as fn(&mut DictionaryReport),
            "zero imported bitmap dimension",
        ),
        (
            |report: &mut DictionaryReport| {
                report.catalog.new_symbols[0].relative_store_offset = 1;
                report.catalog.exported_symbols[0]
                    .symbol
                    .relative_store_offset = 1;
            },
            "imported descriptor outside ranged source",
        ),
    ] {
        let error = forged_error(1, mutate, RefinementDictionaryBudget::default());
        assert!(matches!(error.kind, DictionaryErrorKind::Malformed(found) if found == reason));
    }

    let error = forged_error(
        2,
        |report| {
            report.catalog.new_symbols[1].relative_store_offset = 0;
            report.catalog.exported_symbols[1]
                .symbol
                .relative_store_offset = 0;
        },
        RefinementDictionaryBudget::default(),
    );
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Malformed("overlapping or unordered imported descriptors")
    ));

    let error = forged_error(
        1,
        |_| {},
        RefinementDictionaryBudget {
            max_imported_bitmap_bytes: 0,
            ..RefinementDictionaryBudget::default()
        },
    );
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::LimitExceeded {
            resource: "imported bitmap bytes",
            ..
        }
    ));

    let error = forged_error(
        1,
        |_| {},
        RefinementDictionaryBudget {
            max_imported_store_span: 0,
            ..RefinementDictionaryBudget::default()
        },
    );
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::LimitExceeded {
            resource: "imported store span",
            ..
        }
    ));

    let error = forged_error(
        1,
        |_| {},
        RefinementDictionaryBudget {
            max_catalog_bytes: 0,
            ..RefinementDictionaryBudget::default()
        },
    );
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::LimitExceeded {
            resource: "imported catalog metadata bytes",
            ..
        }
    ));
}

#[test]
fn each_symbol_geometry_bound_blocks_output_before_refinement() {
    for (budget, resource) in [
        (
            DictionaryBudget {
                max_width: 0,
                ..DictionaryBudget::default()
            },
            "symbol width",
        ),
        (
            DictionaryBudget {
                max_height: 0,
                ..DictionaryBudget::default()
            },
            "height class",
        ),
        (
            DictionaryBudget {
                max_pixels_per_symbol: 0,
                ..DictionaryBudget::default()
            },
            "symbol pixels",
        ),
        (
            DictionaryBudget {
                max_bytes_per_symbol: 0,
                ..DictionaryBudget::default()
            },
            "symbol bytes",
        ),
        (
            DictionaryBudget {
                max_total_pixels: 0,
                ..DictionaryBudget::default()
            },
            "dictionary pixels",
        ),
        (
            DictionaryBudget {
                max_stored_bitmap_bytes: 0,
                ..DictionaryBudget::default()
            },
            "stored bitmap bytes",
        ),
    ] {
        let observed = run(
            &COMPLETE_ONE,
            1,
            1,
            0,
            budget,
            RefinementBudget::default(),
            RefinementDictionaryBudget::default(),
        );
        let error = observed.result.unwrap_err();
        assert!(
            matches!(error.kind, DictionaryErrorKind::LimitExceeded { resource: found, .. } if found == resource),
            "{resource}: {error}"
        );
        assert_eq!(error.progress.completed_symbols, 0);
        assert!(observed.output.is_empty());
    }
}

#[test]
fn second_dictionary_preflight_rejects_wrong_mode_reference_and_store_views() {
    let error = preflight_error(
        PreflightCase {
            flags: 0xe802,
            ..PreflightCase::default()
        },
        |_, _| {},
    );
    assert!(matches!(error.kind, DictionaryErrorKind::Malformed(_)));
    assert!(error.progress.header_bytes_fetched > 0);

    // A direct dictionary imports nothing.
    let error = preflight_error(
        PreflightCase {
            flags: 0x0800,
            ..PreflightCase::default()
        },
        |_, _| {},
    );
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Unsupported {
            feature: "imported dictionary references",
            value: 1,
        }
    ));

    let error = preflight_error(
        PreflightCase {
            flags: 0x1902,
            ..PreflightCase::default()
        },
        |_, _| {},
    );
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Unsupported {
            feature: "bitmap context carry",
            ..
        }
    ));

    let error = preflight_error(
        PreflightCase {
            flags: 0x1c02,
            ..PreflightCase::default()
        },
        |_, _| {},
    );
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Unsupported {
            feature: "second dictionary flags or adaptive template",
            ..
        }
    ));

    let error = preflight_error(
        PreflightCase {
            page: 2,
            ..PreflightCase::default()
        },
        |_, _| {},
    );
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Unsupported {
            feature: "dictionary page association",
            ..
        }
    ));

    let error = preflight_error(
        PreflightCase {
            reference: None,
            ..PreflightCase::default()
        },
        |_, _| {},
    );
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Malformed("expected exactly the supplied dictionary reference")
    ));

    let error = preflight_error(PreflightCase::default(), |header, _| {
        header.segment_type = 38;
    });
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Malformed("imported dictionary segment metadata")
    ));

    let error = preflight_error(PreflightCase::default(), |_, report| {
        report.header.body.offset += 1;
    });
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Malformed("imported dictionary is not a complete direct report")
    ));

    let error = preflight_error(
        PreflightCase {
            imported_base: 2,
            ..PreflightCase::default()
        },
        |_, _| {},
    );
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Malformed("imported store base outside source")
    ));

    let error = preflight_error(
        PreflightCase {
            new_base: 1,
            ..PreflightCase::default()
        },
        |_, _| {},
    );
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::InvalidSpan("new store base outside ranged source")
    ));
}

#[test]
fn combined_preflight_limits_and_iaid_layout_are_independent() {
    let error = preflight_error(
        PreflightCase {
            exported: 2,
            ..PreflightCase::default()
        },
        |_, _| {},
    );
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Malformed("exported count exceeds available symbols")
    ));

    let error = preflight_error(
        PreflightCase {
            bank_width: Some(1),
            ..PreflightCase::default()
        },
        |_, _| {},
    );
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Malformed("IAID width or GR context layout mismatch")
    ));

    let error = preflight_error(
        PreflightCase {
            refinement_budget: RefinementBudget {
                max_source_request_bytes: 0,
                ..RefinementBudget::default()
            },
            ..PreflightCase::default()
        },
        |_, _| {},
    );
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Malformed("zero refinement I/O request bound")
    ));

    let error = preflight_error(
        PreflightCase {
            new: 1,
            exported: 1,
            second_budget: RefinementDictionaryBudget {
                max_catalog_bytes: 64,
                ..RefinementDictionaryBudget::default()
            },
            ..PreflightCase::default()
        },
        |_, _| {},
    );
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::LimitExceeded {
            resource: "catalog metadata bytes",
            ..
        }
    ));

    let error = preflight_error(
        PreflightCase {
            second_budget: RefinementDictionaryBudget {
                max_working_bytes: 0,
                ..RefinementDictionaryBudget::default()
            },
            ..PreflightCase::default()
        },
        |_, _| {},
    );
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::LimitExceeded {
            resource: "dictionary working bytes",
            ..
        }
    ));

    let error = preflight_error(
        PreflightCase {
            dictionary_budget: DictionaryBudget {
                max_source_request_bytes: 1,
                ..DictionaryBudget::default()
            },
            ..PreflightCase::default()
        },
        |_, _| {},
    );
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::LimitExceeded {
            resource: "MQ source request bytes",
            ..
        }
    ));

    let error = preflight_error(
        PreflightCase {
            body: &[],
            ..PreflightCase::default()
        },
        |_, _| {},
    );
    assert!(
        matches!(
            error.kind,
            DictionaryErrorKind::Truncated("MQ body terminal pair")
        ),
        "{error}"
    );
    assert_eq!(error.progress.mq_initialization_bytes_fetched, 0);
}

#[test]
fn source_failure_during_mq_initialization_has_a_located_cause() {
    // Try adjacent read boundaries rather than depending on the parser's
    // batching. A failure after header validation must remain an MQ error.
    let mut mq_error = None;
    for additional in 0..16 {
        let outcome = preflight_result(
            PreflightCase {
                fail_after_additional_reads: Some(additional),
                ..PreflightCase::default()
            },
            |_, _| {},
        );
        match outcome {
            Err(error) if matches!(error.kind, DictionaryErrorKind::Mq(_)) => {
                mq_error = Some(error);
                break;
            }
            Ok(()) => break,
            _ => {}
        }
    }
    let error = mq_error.expect("one boundary must fail MQ initialization");
    assert_eq!(error.segment, 2);
    assert!(error.offset >= 24);
    assert!(error.progress.header_bytes_fetched >= 12);
    assert!(error.progress.mq.is_none());
    assert!(std::error::Error::source(&error).is_some());
}

#[test]
fn public_error_messages_keep_locations_and_nested_sources() {
    let header = preflight_error(
        PreflightCase {
            flags: 0xe802,
            ..PreflightCase::default()
        },
        |_, _| {},
    );
    let unsupported = preflight_error(
        PreflightCase {
            flags: 0x0800,
            ..PreflightCase::default()
        },
        |_, _| {},
    );
    let malformed = run(
        &ZERO_PREFIX,
        1,
        1,
        2,
        DictionaryBudget::default(),
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    )
    .result
    .unwrap_err();
    let limit = preflight_error(
        PreflightCase {
            second_budget: RefinementDictionaryBudget {
                max_working_bytes: 0,
                ..RefinementDictionaryBudget::default()
            },
            ..PreflightCase::default()
        },
        |_, _| {},
    );
    let invalid = preflight_error(
        PreflightCase {
            new_base: 1,
            ..PreflightCase::default()
        },
        |_, _| {},
    );
    let mq = run(
        &COMPLETE_BOTH_EXPORTS[..COMPLETE_BOTH_EXPORTS.len() - 1],
        1,
        1,
        2,
        DictionaryBudget::default(),
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    )
    .result
    .unwrap_err();
    let refinement = sink_fault(SinkFault::Io, &NeverCancel, false).0.unwrap();
    let sink = sink_fault(SinkFault::FlushIo, &NeverCancel, false)
        .0
        .unwrap();
    let fabricate = |kind| DictionaryError {
        segment: 2,
        offset: 23,
        progress: Box::new(DictionaryProgress::default()),
        kind,
    };
    let cases = [
        (header, false),
        (unsupported, false),
        (malformed, false),
        (limit, false),
        (invalid, false),
        (mq, true),
        (refinement, true),
        (sink, true),
        (fabricate(DictionaryErrorKind::AllocationFailed), false),
        (fabricate(DictionaryErrorKind::Cancelled), false),
        (fabricate(DictionaryErrorKind::Poisoned), false),
    ];
    for (error, has_source) in cases {
        let display = error.to_string();
        assert!(
            display.contains("dictionary segment 2 at source byte"),
            "{display}"
        );
        common::assert_display_propagates_fmt_error(&error);
        assert_eq!(
            std::error::Error::source(&error).is_some(),
            has_source,
            "{display}"
        );
    }
}

#[test]
fn bounded_synthetic_control_witnesses_keep_typed_locations() {
    // Each body codes the decisions of one small control-path witness up to
    // its refusal. They are not document samples.
    let witnesses: [(&[u8], &str); 12] = [
        (&[0x94, 0x0a, 0xff, 0xac], "malformed:REFAGGNINST negative"),
        (&[0x94, 0x6f, 0xff, 0xac], "malformed:REFAGGNINST OOB"),
        (
            &[0x94, 0xff, 0xac],
            "malformed:future, self, or absent symbol ID",
        ),
        (
            &[0x95, 0x09, 0x9b, 0x73, 0x90, 0xae, 0x8d, 0x46, 0xff, 0xac],
            "malformed:symbol-count overrun before width OOB",
        ),
        (
            &[0x95, 0x0b, 0x2a, 0x18, 0xcd, 0x22, 0x57, 0xff, 0xac],
            "malformed:negative export run",
        ),
        (
            &[0x95, 0x7f, 0xff, 0xac],
            "unsupported:zero-dimension symbol bitmap",
        ),
        (
            &[0x95, 0x0e, 0x99, 0x0f, 0xff, 0xac],
            "malformed:IARDX out of band",
        ),
        (
            &[0x95, 0x13, 0xe6, 0x74, 0xd3, 0xa7, 0x2c, 0xbf, 0xff, 0xac],
            "malformed:IARDY outside signed 32-bit range",
        ),
        (
            &[0x95, 0x0c, 0x3e, 0x26, 0xd7, 0xff, 0xac],
            "malformed:IAEX out of band",
        ),
        (
            &[0x95, 0x13, 0xdb, 0xff, 0xac],
            "malformed:IARDY out of band",
        ),
        (
            &[0x95, 0x0c, 0x40, 0xd7, 0x06, 0xeb, 0xff, 0xac],
            "malformed:export run overshoot",
        ),
        (
            &[0x95, 0x0e, 0xed, 0xf0, 0x71, 0x52, 0x40, 0x7f, 0xff, 0xac],
            "malformed:IARDX outside signed 32-bit range",
        ),
    ];
    let budget = DictionaryBudget {
        max_width: 8,
        max_height: 8,
        max_pixels_per_symbol: 64,
        max_total_pixels: 64,
        max_bytes_per_symbol: 8,
        max_stored_bitmap_bytes: 8,
        max_export_runs: 4,
        max_height_classes: 4,
        ..DictionaryBudget::default()
    };
    for (body, expected) in witnesses {
        let observed = run(
            body,
            1,
            1,
            0,
            budget,
            RefinementBudget::default(),
            RefinementDictionaryBudget::default(),
        );
        let error = observed.result.unwrap_err();
        let label = match error.kind {
            DictionaryErrorKind::Malformed(s) => format!("malformed:{s}"),
            DictionaryErrorKind::Unsupported { feature, .. } => {
                format!("unsupported:{feature}")
            }
            other => format!("other:{other:?}"),
        };
        assert_eq!(label, expected);
        assert_eq!(error.segment, 2);
        assert!(error.offset > 0, "{expected}");
        assert!(error.progress.poisoned, "{expected}");
        assert!(error.progress.completed_symbols <= 1);
        assert!(observed.output.len() <= 8);
    }
}

#[test]
fn later_symbol_reads_the_new_store_only_after_an_explicit_flush() {
    // A two-refinement prefix that selects active IAID 1 (the first new
    // symbol) for the second target and an in-range reference row. The later
    // malformed width control is outside this prefix check.
    let body = [0x95, 0x13, 0x98, 0x97, 0x6f, 0x41, 0x9a, 0x7f, 0xff, 0xac];
    let observed = run(
        &body,
        1,
        2,
        0,
        DictionaryBudget::default(),
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    );
    let error = observed.result.unwrap_err();
    assert_eq!(error.progress.completed_symbols, 2, "{error}");
    assert_eq!(error.progress.iaai.single_reference, 2);
    assert_eq!(observed.new_calls, 1);
    assert_eq!(observed.output.len(), 2);

    let observed = run(
        &body,
        1,
        2,
        0,
        DictionaryBudget::default(),
        RefinementBudget {
            max_flushes: 0,
            ..RefinementBudget::default()
        },
        RefinementDictionaryBudget::default(),
    );
    let error = observed.result.unwrap_err();
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Refinement(ref nested)
            if matches!(nested.kind, RefinementErrorKind::LimitExceeded { resource: "store flushes", .. })
    ));
    assert_eq!(error.progress.completed_symbols, 1);
    assert_eq!(observed.new_calls, 0);
    assert_eq!(observed.output.len(), 1);
}

struct Flag(Rc<Cell<bool>>);

impl Cancellation for Flag {
    fn is_cancelled(&self) -> bool {
        self.0.get()
    }
}

enum SinkFault {
    Zero,
    Overreport,
    Io,
    Pending,
    CancelAfterWrite(Rc<Cell<bool>>),
    CancelAfterFlush(Rc<Cell<bool>>),
    FlushCancelled,
    FlushIo,
}

struct FaultSink {
    bytes: Rc<RefCell<Vec<u8>>>,
    fault: SinkFault,
}

impl SequentialSink for FaultSink {
    async fn write(&mut self, bytes: &[u8]) -> caj2pdf_core::Result<usize> {
        match &self.fault {
            SinkFault::Zero => Ok(0),
            SinkFault::Overreport => Ok(bytes.len() + 1),
            SinkFault::Io => Err(Error::Io(io::Error::other("injected sink failure"))),
            SinkFault::Pending => {
                pending::<()>().await;
                unreachable!()
            }
            SinkFault::CancelAfterWrite(flag) => {
                self.bytes.borrow_mut().extend_from_slice(bytes);
                flag.set(true);
                Ok(bytes.len())
            }
            SinkFault::CancelAfterFlush(_) => {
                self.bytes.borrow_mut().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            SinkFault::FlushCancelled | SinkFault::FlushIo => {
                self.bytes.borrow_mut().extend_from_slice(bytes);
                Ok(bytes.len())
            }
        }
    }

    async fn flush(&mut self) -> caj2pdf_core::Result<()> {
        match &self.fault {
            SinkFault::FlushIo => Err(Error::Io(io::Error::other("injected flush failure"))),
            SinkFault::FlushCancelled => Err(Error::Cancelled),
            SinkFault::CancelAfterFlush(flag) => {
                flag.set(true);
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

fn sink_fault<C: Cancellation>(
    fault: SinkFault,
    cancellation: &C,
    pending_expected: bool,
) -> (Option<DictionaryError>, DictionaryProgress, Vec<u8>) {
    let (imported_header, imported_report, mut imported_source) = imported(1, 1, &[0x80]);
    let (mut source, header) = segment(2, Some(1), 0x1802, 0, 1, &COMPLETE_ONE);
    let store = Rc::new(RefCell::new(Vec::new()));
    let mut new_source = SharedSource::new(store.clone());
    let mut sink = FaultSink {
        bytes: store.clone(),
        fault,
    };
    let limits = Limits::default();
    let table = table();
    let mut contexts = coding_unit(1, &limits, &MqBudget::default());
    let mut decoder = ready(SymbolDictionaryDecoder::new(
        &mut source,
        &header,
        Some(ImportedDictionary {
            segment: &imported_header,
            report: &imported_report,
        }),
        DictionaryStores {
            imported: &mut imported_source,
            imported_base: 0,
            new_reader: &mut new_source,
            new_writer: &mut sink,
            new_base: 0,
        },
        &table,
        &mut contexts,
        &limits,
        cancellation,
        MqBudget::default(),
        DictionaryBudget::default(),
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    ))
    .unwrap();
    let error = if pending_expected {
        {
            let future = decoder.decode();
            let mut future = pin!(future);
            assert!(matches!(
                future
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop())),
                Poll::Pending
            ));
        }
        None
    } else {
        Some(ready(decoder.decode()).unwrap_err())
    };
    let progress = decoder.progress();
    assert!(matches!(
        ready(decoder.decode()).unwrap_err().kind,
        DictionaryErrorKind::Poisoned
    ));
    drop(decoder);
    let output = store.borrow().clone();
    (error, progress, output)
}

#[test]
fn zero_overreported_and_failed_sink_writes_poison_partial_dictionary() {
    for fault in [SinkFault::Zero, SinkFault::Overreport, SinkFault::Io] {
        let (error, progress, bytes) = sink_fault(fault, &NeverCancel, false);
        let error = error.unwrap();
        assert!(matches!(error.kind, DictionaryErrorKind::Refinement(_)));
        assert!(progress.poisoned);
        assert_eq!(progress.refinement.sink_writes, 1);
        assert_eq!(progress.completed_symbols, 0);
        assert!(bytes.is_empty());
    }
}

#[test]
fn cancellation_after_partial_output_reports_fetched_and_semantic_positions() {
    let flag = Rc::new(Cell::new(false));
    let token = Flag(flag.clone());
    let (error, progress, bytes) = sink_fault(SinkFault::CancelAfterWrite(flag), &token, false);
    let error = error.unwrap();
    assert!(
        matches!(error.kind, DictionaryErrorKind::Refinement(ref nested)
        if matches!(nested.kind, RefinementErrorKind::Cancelled))
    );
    assert!(progress.poisoned);
    assert_eq!(progress.refinement.output_bytes_written, 1);
    assert_eq!(progress.refinement.sink_writes, 1);
    assert_eq!(bytes, vec![0x80]);
    assert!(progress.source_bytes_fetched() >= 12 + COMPLETE_ONE.len() as u64);
    assert!(progress.mq.unwrap().symbols_decoded > 0);
}

#[test]
fn dropping_a_pending_bitmap_write_preserves_observed_progress_and_poison() {
    let (error, progress, bytes) = sink_fault(SinkFault::Pending, &NeverCancel, true);
    assert!(error.is_none());
    assert!(progress.poisoned);
    assert_eq!(progress.refinement.sink_writes, 1);
    assert_eq!(progress.refinement.output_bytes_written, 0);
    assert!(progress.mq.unwrap().symbols_decoded > 0);
    assert!(bytes.is_empty());
}

#[test]
fn final_flush_failure_poison_after_terminal_validation() {
    let (error, progress, bytes) = sink_fault(SinkFault::FlushIo, &NeverCancel, false);
    let error = error.unwrap();
    assert!(matches!(error.kind, DictionaryErrorKind::Sink(_)));
    assert_eq!(progress.completed_symbols, 1);
    assert_eq!(progress.export_runs, 1);
    assert!(progress.poisoned);
    assert_eq!(bytes, vec![0x80]);
}

#[test]
fn cancellation_after_final_flush_preserves_completed_bitmap_progress() {
    let flag = Rc::new(Cell::new(false));
    let token = Flag(flag.clone());
    let (error, progress, bytes) = sink_fault(SinkFault::CancelAfterFlush(flag), &token, false);
    assert!(matches!(
        error.unwrap().kind,
        DictionaryErrorKind::Cancelled
    ));
    assert_eq!(progress.completed_symbols, 1);
    assert_eq!(progress.export_runs, 1);
    assert!(progress.poisoned);
    assert_eq!(bytes, vec![0x80]);
}

#[test]
fn sink_cancelled_flush_maps_to_dictionary_cancellation() {
    let (error, progress, bytes) = sink_fault(SinkFault::FlushCancelled, &NeverCancel, false);
    assert!(matches!(
        error.unwrap().kind,
        DictionaryErrorKind::Cancelled
    ));
    assert_eq!(progress.completed_symbols, 1);
    assert_eq!(progress.export_runs, 1);
    assert!(progress.poisoned);
    assert_eq!(bytes, vec![0x80]);
}

#[test]
fn export_count_mismatch_refuses_partial_cross_store_catalog() {
    // The same independently chosen stream exports both the import and its
    // refined successor. Claiming only one export must fail during IAEX.
    let observed = run(
        &COMPLETE_BOTH_EXPORTS,
        1,
        1,
        1,
        DictionaryBudget::default(),
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    );
    let error = observed.result.unwrap_err();
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Malformed("exported symbol total")
    ));
    assert_eq!(error.progress.completed_symbols, 1);
    assert_eq!(error.progress.export_runs, 2);
    assert!(error.progress.poisoned);
    assert_eq!(observed.output.len(), 1);
}

#[test]
fn malformed_iaex_mutations_remain_bounded_and_expose_overshoot() {
    let mut saw_overshoot = false;
    let budget = DictionaryBudget {
        max_export_runs: 2,
        ..DictionaryBudget::default()
    };
    for byte in 0u8..=255 {
        let observed = run(
            &[byte, 0xff, 0xac],
            0,
            0,
            0,
            budget,
            RefinementBudget::default(),
            RefinementDictionaryBudget::default(),
        );
        match observed.result {
            Ok(report) => assert_eq!(report.progress.export_runs, 1),
            Err(error) => {
                assert!(error.progress.export_runs <= 2);
                saw_overshoot |= matches!(
                    error.kind,
                    DictionaryErrorKind::Malformed("export run overshoot")
                );
            }
        }
        assert!(observed.output.is_empty());
    }
    assert!(
        saw_overshoot,
        "bounded mutation must exercise IAEX overshoot"
    );
}

#[test]
fn truncated_terminal_after_complete_exports_is_located_and_poisoned() {
    let observed = run(
        &COMPLETE_BOTH_EXPORTS[..COMPLETE_BOTH_EXPORTS.len() - 1],
        1,
        1,
        2,
        DictionaryBudget::default(),
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    );
    let error = observed.result.unwrap_err();
    assert!(matches!(error.kind, DictionaryErrorKind::Mq(_)));
    assert_eq!(error.progress.completed_symbols, 1);
    assert_eq!(error.progress.export_runs, 2);
    assert!(error.progress.poisoned);
    assert!(error.offset > 0);
    assert_eq!(observed.output.len(), 1);
}

#[test]
fn signed_height_class_deltas_reach_the_next_symbol_control() {
    // The real MQ and Annex A.2 decoders produce +2, width OOB, then -1 or
    // zero. The public dictionary decoder accepts each second class before
    // encountering an independently checked later control value.
    let negative = [0x75, 0x45, 0x10, 0xff, 0xac];
    let zero = [0x75, 0xa2, 0x60, 0x3f, 0xff, 0xac];
    let procedures = [
        IntegerProcedure::Iadh,
        IntegerProcedure::Iadw,
        IntegerProcedure::Iadh,
    ];
    for (body, expected_delta) in [
        (negative.as_slice(), IntegerValue::Signed(-1)),
        (zero.as_slice(), IntegerValue::Signed(0)),
    ] {
        assert_eq!(
            integer_prefix(body, &procedures).unwrap(),
            [
                IntegerValue::Signed(2),
                IntegerValue::OutOfBand,
                expected_delta
            ]
        );
        let observed = run(
            body,
            1,
            1,
            0,
            DictionaryBudget::default(),
            RefinementBudget::default(),
            RefinementDictionaryBudget::default(),
        );
        let error = observed.result.unwrap_err();
        assert_eq!(error.progress.height_classes, 2);
        assert_eq!(error.progress.completed_symbols, 0);
        assert!(error.progress.poisoned);
        assert!(observed.output.is_empty());
        match expected_delta {
            IntegerValue::Signed(-1) => assert!(matches!(
                error.kind,
                DictionaryErrorKind::Unsupported {
                    feature: "REFAGGNINST aggregation",
                    value: 4,
                }
            )),
            IntegerValue::Signed(0) => assert!(matches!(
                error.kind,
                DictionaryErrorKind::Malformed("negative symbol dimension")
            )),
            _ => unreachable!(),
        }
    }
}

#[test]
fn cancellation_checkpoints_preserve_typed_progress_through_exports() {
    let mut saw_host = false;
    let mut saw_mq = false;
    let mut saw_completed_prefix = false;
    for allowed in 0..180 {
        let cancellation = common::CancelAfter::new(allowed);
        let observed = run_with_imported_io(
            &COMPLETE_ONE,
            1,
            1,
            &[0x80],
            1,
            0,
            DictionaryBudget::default(),
            RefinementBudget::default(),
            RefinementDictionaryBudget::default(),
            &cancellation,
            MqBudget::default(),
            usize::MAX,
        );
        if let Err(error) = observed.result {
            assert!(observed.output.len() <= 1);
            match &error.kind {
                DictionaryErrorKind::Cancelled => {
                    // Only a cancellation before the coding unit starts
                    // leaves the decoder unpoisoned.
                    assert!(
                        error.progress.poisoned || error.progress.mq.is_none(),
                        "allowed {allowed}"
                    );
                    saw_completed_prefix |=
                        error.progress.completed_symbols == 1 && error.progress.export_runs == 1;
                }
                DictionaryErrorKind::Refinement(nested)
                    if matches!(nested.kind, RefinementErrorKind::Cancelled) =>
                {
                    saw_host = true
                }
                DictionaryErrorKind::Mq(mq)
                    if matches!(
                        mq.kind,
                        caj2pdf_core::jbig2::mq::ArithmeticErrorKind::Cancelled
                    ) =>
                {
                    saw_mq = true;
                }
                _ => {}
            }
        }
    }
    assert!(saw_host);
    assert!(saw_mq);
    assert!(saw_completed_prefix);
}

#[test]
fn negative_width_delta_reaches_a_positive_second_symbol_geometry() {
    let body = [
        0x93, 0xba, 0xe0, 0x88, 0x35, 0xb8, 0x36, 0x3b, 0xd3, 0x3f, 0xff, 0xac,
    ];
    assert_eq!(
        integer_prefix(&body, &[IntegerProcedure::Iadh, IntegerProcedure::Iadw]).unwrap(),
        [IntegerValue::Signed(1), IntegerValue::Signed(2)]
    );
    let budget = DictionaryBudget {
        max_width: 2,
        max_height: 1,
        max_total_pixels: 3,
        ..DictionaryBudget::default()
    };
    let observed = run(
        &body,
        1,
        2,
        0,
        budget,
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    );
    let error = observed.result.unwrap_err();
    assert_eq!(error.progress.height_classes, 1);
    assert_eq!(error.progress.completed_symbols, 1);
    assert_eq!(error.progress.refinement.pixels_decoded, 2);
    assert_eq!(observed.output.len(), 1);
    assert!(
        matches!(
            error.kind,
            DictionaryErrorKind::Unsupported {
                feature: "REFAGGNINST aggregation",
                value: 40,
            }
        ),
        "{error:?}"
    );

    // A two-pixel cap stops at the second geometry; a three-pixel cap reaches
    // IAAI. The first symbol has width two and both heights are one, so the
    // accepted second width is one and its signed IADW is -1.
    let observed = run(
        &body,
        1,
        2,
        0,
        DictionaryBudget {
            max_total_pixels: 2,
            ..budget
        },
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    );
    let error = observed.result.unwrap_err();
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::LimitExceeded {
            resource: "dictionary pixels",
            attempted: 3,
            ..
        }
    ));
}

#[test]
fn negative_initial_width_delta_is_rejected_before_bitmap_io() {
    let body = [0x5e, 0x7f, 0xff, 0xac];
    assert_eq!(
        integer_prefix(&body, &[IntegerProcedure::Iadh, IntegerProcedure::Iadw]).unwrap(),
        [IntegerValue::Signed(3), IntegerValue::Signed(-1)]
    );
    let observed = run(
        &body,
        1,
        1,
        0,
        DictionaryBudget::default(),
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    );
    let error = observed.result.unwrap_err();
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Malformed("negative symbol dimension")
    ));
    assert!(observed.output.is_empty());
}

#[test]
fn reference_row_caches_are_charged_after_preflight_working_memory() {
    let error = preflight_error(
        PreflightCase {
            new: 1,
            second_budget: RefinementDictionaryBudget {
                max_working_bytes: 0,
                ..RefinementDictionaryBudget::default()
            },
            ..PreflightCase::default()
        },
        |_, _| {},
    );
    let base_working = match error.kind {
        DictionaryErrorKind::LimitExceeded {
            resource: "dictionary working bytes",
            attempted,
            ..
        } => attempted,
        other => panic!("unexpected preflight result: {other:?}"),
    };
    let observed = run(
        &COMPLETE_ONE,
        1,
        1,
        0,
        DictionaryBudget::default(),
        RefinementBudget::default(),
        RefinementDictionaryBudget {
            max_working_bytes: base_working,
            ..RefinementDictionaryBudget::default()
        },
    );
    let error = observed.result.unwrap_err();
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::LimitExceeded {
            resource: "dictionary working bytes",
            attempted,
            ..
        } if attempted > base_working
    ));
    assert!(error.progress.mq.is_some());
    assert_eq!(error.progress.completed_symbols, 0);
    assert!(observed.output.is_empty());
}

#[test]
fn height_class_oob_is_a_located_refusal_before_bitmap_output() {
    let body = [0xcf, 0xff, 0xac];
    assert_eq!(
        integer_prefix(&body, &[IntegerProcedure::Iadh]).unwrap(),
        [IntegerValue::OutOfBand]
    );
    let observed = run(
        &body,
        1,
        1,
        0,
        DictionaryBudget::default(),
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    );
    let error = observed.result.unwrap_err();
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Malformed("IADH out of band")
    ));
    assert_eq!(error.progress.height_classes, 1);
    assert_eq!(error.progress.completed_symbols, 0);
    assert!(observed.output.is_empty());
}

#[test]
fn final_iaex_count_mismatch_rejects_an_underexported_import() {
    // IAEX skips the sole imported symbol, while the header requires one
    // export. The mismatch is detected after consuming the complete run.
    let observed = run(
        &[0x87, 0xff, 0xac],
        1,
        0,
        1,
        DictionaryBudget::default(),
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    );
    let error = observed.result.unwrap_err();
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Malformed("exported symbol total")
    ));
    assert_eq!(error.progress.export_runs, 1);
    assert!(error.progress.poisoned);
    assert!(observed.output.is_empty());
}
