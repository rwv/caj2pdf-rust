// SPDX-License-Identifier: MIT

//! Original small dictionaries and invented MQ states. The source bytes here
//! are synthetic test controls, not a T.88 table or external document data.

use caj2pdf_core::jbig2::{
    HeaderLimits, SegmentHeader, SegmentSpan,
    dictionary::{
        DictionaryBudget, DictionaryCatalog, DictionaryProgress, DictionaryReport,
        SymbolDescriptor, read_dictionary_data_header,
    },
    iaid::IaidContextBanks,
    mq::{MQ_STATE_COUNT, MqBudget, MqContext, MqSnapshot, MqState, MqTable},
    read_segment_header,
    refinement::{RefinementBudget, RefinementErrorKind},
    refinement_dictionary::{
        RefinementDictionaryBudget, RefinementDictionaryDecoder, RefinementDictionaryError,
        RefinementDictionaryErrorKind, RefinementDictionaryProgress, RefinementDictionaryReport,
        SymbolStore,
    },
};
use caj2pdf_core::{Cancellation, Error, Limits, NeverCancel, RangedSource, SequentialSink};
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
    calls: usize,
    max_read: usize,
    fail_after_calls: Option<usize>,
}

impl Bytes {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            calls: 0,
            max_read: usize::MAX,
            fail_after_calls: None,
        }
    }
}

impl RangedSource for Bytes {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
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
}
impl SharedSource {
    fn new(bytes: Rc<RefCell<Vec<u8>>>) -> Self {
        Self { bytes, calls: 0 }
    }
}
impl RangedSource for SharedSource {
    fn size(&self) -> u64 {
        self.bytes.borrow().len() as u64
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
    let mut states = vec![
        MqState {
            qe: 0x4000,
            next_mps: 1,
            next_lps: 1,
            switch_mps: false
        };
        MQ_STATE_COUNT
    ];
    states[1].next_mps = 2;
    states[1].next_lps = 2;
    states[2].next_mps = 2;
    states[2].next_lps = 2;
    MqTable::new(states, &Limits::default()).unwrap()
}

fn imported(count: u32, width: u32, row: &[u8]) -> (SegmentHeader, DictionaryReport, Bytes) {
    assert_eq!(row.len(), width.div_ceil(8) as usize);
    let (mut source, header) = segment(1, None, 0x0800, count, count, &[0xff, 0xac]);
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
            new_symbols: symbols.clone(),
            exported_symbols: symbols,
        },
        progress: DictionaryProgress {
            completed_symbols: count,
            mq: Some(MqSnapshot {
                interval: 0x8000,
                code: 0,
                bit_counter: 0,
                current_input_offset: data.body.offset,
                source_bytes_fetched: 2,
                terminal_inputs: 0,
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
    result: Result<RefinementDictionaryReport, RefinementDictionaryError>,
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
    let mut banks =
        IaidContextBanks::with_bitmap_contexts(width, 1024, &limits, &MqBudget::default()).unwrap();
    let created = ready(RefinementDictionaryDecoder::new(
        &mut source,
        &header,
        &imported_header,
        &imported_report,
        &mut imported_source,
        0,
        &mut new_source,
        &mut new_sink,
        0,
        &table,
        &mut banks,
        &limits,
        &NeverCancel,
        MqBudget::default(),
        dict_budget,
        refinement_budget,
        second_budget,
    ));
    let result = match created {
        Ok(mut decoder) => ready(decoder.decode()),
        Err(error) => Err(error),
    };
    let output = store.borrow().clone();
    let base = banks.layout().bitmap_base();
    let gr_contexts: Vec<_> = (0..1024)
        .filter(|&local| banks.mq_contexts_mut().get(base + local) != Some(MqContext::default()))
        .collect();
    let gr_state_zero = banks.mq_contexts_mut().get(base).unwrap().state_index;
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

const ONE_PREFIX: [u8; 16] = [
    0xee, 0xef, 0xfa, 0x7f, 0x07, 0x75, 0xb1, 0x44, 0x32, 0xdc, 0x05, 0x39, 0xc1, 0xba, 0xff, 0xac,
];
const ZERO_PREFIX: [u8; 16] = [
    0xee, 0xf9, 0xbc, 0x99, 0xb6, 0xab, 0x0a, 0xe6, 0xc7, 0x62, 0x40, 0xe4, 0x11, 0xbd, 0xff, 0xac,
];
const MANY_PREFIX: [u8; 16] = [
    0xee, 0xd1, 0x46, 0xf6, 0x6d, 0x21, 0xc1, 0x62, 0xf6, 0x19, 0xe9, 0x42, 0x34, 0x06, 0xff, 0xac,
];
const COMPLETE_ONE: [u8; 32] = [
    0xee, 0xed, 0x20, 0x5c, 0xbc, 0xfb, 0xf8, 0x0d, 0x9d, 0x96, 0x7f, 0x85, 0xf2, 0x37, 0xf7, 0x12,
    0x2a, 0x26, 0x16, 0x28, 0x98, 0x45, 0x3b, 0x19, 0x5b, 0x67, 0x17, 0x6f, 0x8f, 0xb0, 0xff, 0xac,
];
const READ_REFERENCE_AND_EXPORT_NEW: [u8; 32] = [
    0xee, 0xed, 0xbe, 0x3f, 0x77, 0xa9, 0xb8, 0xc1, 0x30, 0x00, 0xbc, 0xc5, 0x12, 0xd0, 0xb7, 0x68,
    0x90, 0xa3, 0x34, 0x8c, 0x6f, 0x2b, 0x53, 0x03, 0x3f, 0x67, 0x0f, 0x5d, 0xfd, 0xb0, 0xff, 0xac,
];
const READ_REFERENCE_PIXEL: [u8; 32] = [
    0xee, 0xe9, 0x3e, 0x3e, 0xb4, 0xfb, 0x74, 0xce, 0x9f, 0xc6, 0x05, 0x31, 0xc1, 0xd2, 0x6d, 0x5d,
    0xcb, 0x76, 0xd8, 0x0d, 0x5d, 0x1f, 0x17, 0xa5, 0x98, 0xe3, 0xfa, 0x21, 0x83, 0xf8, 0xff, 0xac,
];
const COMPLETE_BOTH_EXPORTS: [u8; 32] = [
    0xee, 0xe9, 0xe4, 0xbf, 0xe8, 0xf9, 0xb2, 0x2d, 0x07, 0xb7, 0x7e, 0xd2, 0xd3, 0xd8, 0xc7, 0x4a,
    0xba, 0xf3, 0x00, 0x68, 0xf8, 0x65, 0xdb, 0x4c, 0x12, 0xa0, 0xb9, 0xc0, 0xad, 0xd6, 0xff, 0xac,
];
const TWO_REFINEMENTS_PREFIX: [u8; 64] = [
    0xee, 0xef, 0x86, 0x97, 0xf6, 0x95, 0x1a, 0xda, 0x6b, 0xa7, 0x02, 0x08, 0xa8, 0xa3, 0xe2, 0x5f,
    0xe7, 0x64, 0xf0, 0xc3, 0x4c, 0x1c, 0xf2, 0x8f, 0x1c, 0x7e, 0xd8, 0x55, 0x59, 0x73, 0x35, 0x65,
    0xd1, 0xfa, 0x65, 0x3b, 0x17, 0xaf, 0x44, 0xb8, 0x31, 0xed, 0x6a, 0x30, 0xda, 0x09, 0x19, 0xb5,
    0x08, 0x9a, 0x63, 0x13, 0x5b, 0x91, 0x5a, 0x12, 0x7a, 0xa6, 0x7d, 0x7d, 0x93, 0x94, 0xff, 0xac,
];
const ZERO_NEW_EXPORT_IMPORTED: [u8; 12] = [
    0xfe, 0x4f, 0x11, 0x65, 0x3b, 0xd9, 0x2a, 0xe0, 0x65, 0xbf, 0xff, 0xac,
];

#[test]
fn zero_new_and_zero_imported_still_consumes_one_iaex() {
    let observed = run(
        &[0xff, 0xac],
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
            RefinementDictionaryErrorKind::Malformed("REFAGGNINST zero")
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
            RefinementDictionaryErrorKind::Unsupported {
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
        1,
    );
    let report = observed.result.unwrap();
    assert_eq!(report.progress.completed_symbols, 1);
    assert_eq!(report.progress.refinement.reference_reads, 3);
    assert_eq!(report.progress.refinement.reference_bytes_fetched, 3);
    assert_eq!(
        report.progress.source_bytes_fetched(),
        24 + READ_REFERENCE_PIXEL.len() as u64
    );
    assert_eq!(observed.imported_calls, 3);
    assert!(observed.source_calls > 32);
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
    // bitmap uses GR context zero. The invented table records two uses as
    // state 2, proving the dictionary retained GR statistics. A later
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
        RefinementDictionaryErrorKind::LimitExceeded {
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
        RefinementDictionaryErrorKind::LimitExceeded {
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
        RefinementDictionaryErrorKind::LimitExceeded {
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
    assert!(matches!(
        error.kind,
        RefinementDictionaryErrorKind::Refinement(_)
    ));
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
        RefinementDictionaryErrorKind::LimitExceeded {
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
            assert!(progress.source_bytes_fetched() <= 24 + body.len() as u64);
            assert!(progress.completed_symbols <= 1);
            assert!(progress.export_runs <= 8);
        }
    }
}

fn forged_error(
    count: u32,
    mutate: impl FnOnce(&mut DictionaryReport),
    budget: RefinementDictionaryBudget,
) -> RefinementDictionaryError {
    let (imported_header, mut report, mut imported_source) = imported(count, 1, &[0x80]);
    mutate(&mut report);
    let (mut source, header) = segment(2, Some(1), 0x1802, 0, 0, &[0xff, 0xac]);
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
    let mut banks =
        IaidContextBanks::with_bitmap_contexts(width, 1024, &limits, &MqBudget::default()).unwrap();
    let result = ready(RefinementDictionaryDecoder::new(
        &mut source,
        &header,
        &imported_header,
        &report,
        &mut imported_source,
        0,
        &mut new_source,
        &mut new_sink,
        0,
        &table,
        &mut banks,
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
    new_base: u64,
    bank_width: Option<u32>,
    dictionary_budget: DictionaryBudget,
    refinement_budget: RefinementBudget,
    second_budget: RefinementDictionaryBudget,
}

impl Default for PreflightCase {
    fn default() -> Self {
        Self {
            flags: 0x1802,
            reference: Some(1),
            page: 1,
            body: &[0xff, 0xac],
            fail_after_additional_reads: None,
            new: 0,
            exported: 0,
            imported_base: 0,
            new_base: 0,
            bank_width: None,
            dictionary_budget: DictionaryBudget::default(),
            refinement_budget: RefinementBudget::default(),
            second_budget: RefinementDictionaryBudget::default(),
        }
    }
}

fn preflight_error(
    setup: PreflightCase,
    mutate_imported: impl FnOnce(&mut SegmentHeader, &mut DictionaryReport),
) -> RefinementDictionaryError {
    let error = preflight_result(setup, mutate_imported).unwrap_err();
    assert!(error.progress.mq.is_none());
    error
}

fn preflight_result(
    setup: PreflightCase,
    mutate_imported: impl FnOnce(&mut SegmentHeader, &mut DictionaryReport),
) -> Result<(), RefinementDictionaryError> {
    let (mut imported_source, mut imported_header, mut report) = {
        let (header, report, source) = imported(1, 1, &[0x80]);
        (source, header, report)
    };
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
    let limits = Limits::default();
    let table = table();
    let total = setup.new + 1;
    let width = setup.bank_width.unwrap_or(if total <= 1 {
        0
    } else {
        32 - (total - 1).leading_zeros()
    });
    let mut banks =
        IaidContextBanks::with_bitmap_contexts(width, 1024, &limits, &MqBudget::default()).unwrap();
    let result = ready(RefinementDictionaryDecoder::new(
        &mut source,
        &header,
        &imported_header,
        &report,
        &mut imported_source,
        setup.imported_base,
        &mut new_source,
        &mut new_sink,
        setup.new_base,
        &table,
        &mut banks,
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
            report.catalog.exported_symbols[0].row_stride = 2;
            report.catalog.new_symbols[0].row_stride = 2;
        },
        RefinementDictionaryBudget::default(),
    );
    assert!(matches!(
        error.kind,
        RefinementDictionaryErrorKind::Malformed("noncanonical imported bitmap descriptor")
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
        RefinementDictionaryErrorKind::Malformed("imported exports do not follow new-symbol order")
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
        RefinementDictionaryErrorKind::LimitExceeded {
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
        RefinementDictionaryErrorKind::Malformed(
            "imported dictionary is not a complete direct report"
        )
    ));
}

#[test]
fn imported_bitmap_metadata_and_store_bounds_are_checked_before_mq() {
    for (mutate, reason) in [
        (
            (|report: &mut DictionaryReport| {
                report.catalog.new_symbols[0].width = 0;
                report.catalog.exported_symbols[0].width = 0;
            }) as fn(&mut DictionaryReport),
            "zero imported bitmap dimension",
        ),
        (
            |report: &mut DictionaryReport| {
                report.catalog.new_symbols[0].relative_store_offset = 1;
                report.catalog.exported_symbols[0].relative_store_offset = 1;
            },
            "imported descriptor outside ranged source",
        ),
    ] {
        let error = forged_error(1, mutate, RefinementDictionaryBudget::default());
        assert!(
            matches!(error.kind, RefinementDictionaryErrorKind::Malformed(found) if found == reason)
        );
    }

    let error = forged_error(
        2,
        |report| {
            report.catalog.new_symbols[1].relative_store_offset = 0;
            report.catalog.exported_symbols[1].relative_store_offset = 0;
        },
        RefinementDictionaryBudget::default(),
    );
    assert!(matches!(
        error.kind,
        RefinementDictionaryErrorKind::Malformed("overlapping or unordered imported descriptors")
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
        RefinementDictionaryErrorKind::LimitExceeded {
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
        RefinementDictionaryErrorKind::LimitExceeded {
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
        RefinementDictionaryErrorKind::LimitExceeded {
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
            matches!(error.kind, RefinementDictionaryErrorKind::LimitExceeded { resource: found, .. } if found == resource),
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
    assert!(matches!(
        error.kind,
        RefinementDictionaryErrorKind::Header(_)
    ));
    assert!(error.progress.header_bytes_fetched > 0);

    let error = preflight_error(
        PreflightCase {
            flags: 0x0800,
            ..PreflightCase::default()
        },
        |_, _| {},
    );
    assert!(matches!(
        error.kind,
        RefinementDictionaryErrorKind::Unsupported {
            feature: "dictionary coding mode",
            ..
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
        RefinementDictionaryErrorKind::Unsupported {
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
        RefinementDictionaryErrorKind::Unsupported {
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
        RefinementDictionaryErrorKind::Unsupported {
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
        RefinementDictionaryErrorKind::Malformed(
            "expected exactly the supplied dictionary reference"
        )
    ));

    let error = preflight_error(PreflightCase::default(), |header, _| {
        header.segment_type = 38;
    });
    assert!(matches!(
        error.kind,
        RefinementDictionaryErrorKind::Malformed("imported dictionary segment metadata")
    ));

    let error = preflight_error(PreflightCase::default(), |_, report| {
        report.header.body.offset += 1;
    });
    assert!(matches!(
        error.kind,
        RefinementDictionaryErrorKind::Malformed(
            "imported dictionary is not a complete direct report"
        )
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
        RefinementDictionaryErrorKind::Malformed("imported store base outside source")
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
        RefinementDictionaryErrorKind::InvalidSpan("new store base outside ranged source")
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
        RefinementDictionaryErrorKind::Malformed("exported count exceeds available symbols")
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
        RefinementDictionaryErrorKind::Malformed("IAID width or GR context layout mismatch")
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
        RefinementDictionaryErrorKind::Malformed("zero refinement I/O request bound")
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
        RefinementDictionaryErrorKind::LimitExceeded {
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
        RefinementDictionaryErrorKind::LimitExceeded {
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
        RefinementDictionaryErrorKind::LimitExceeded {
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
        matches!(error.kind, RefinementDictionaryErrorKind::Header(_)),
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
            Err(error) if matches!(error.kind, RefinementDictionaryErrorKind::Mq(_)) => {
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
    assert!(error.progress.header_bytes_fetched >= 24);
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
    let fabricate = |kind| RefinementDictionaryError {
        segment: 2,
        offset: 23,
        progress: Box::new(RefinementDictionaryProgress::default()),
        kind,
    };
    let cases = [
        (header, true),
        (unsupported, false),
        (malformed, false),
        (limit, false),
        (invalid, false),
        (mq, true),
        (refinement, true),
        (sink, true),
        (
            fabricate(RefinementDictionaryErrorKind::AllocationFailed),
            false,
        ),
        (fabricate(RefinementDictionaryErrorKind::Cancelled), false),
        (fabricate(RefinementDictionaryErrorKind::Poisoned), false),
    ];
    for (error, has_source) in cases {
        let display = error.to_string();
        assert!(display.contains("dictionary 2 at source byte"), "{display}");
        assert_eq!(
            std::error::Error::source(&error).is_some(),
            has_source,
            "{display}"
        );
    }
}

#[test]
fn bounded_synthetic_control_witnesses_keep_typed_locations() {
    // These seeds were selected by a bounded test-only search with the
    // invented table above. They are small control-path witnesses, not
    // document samples or exact T.88 probability-state vectors.
    let witnesses: [(u64, &str); 12] = [
        (3, "malformed:REFAGGNINST negative"),
        (14, "malformed:REFAGGNINST OOB"),
        (16, "malformed:future, self, or absent symbol ID"),
        (29, "malformed:symbol-count overrun before width OOB"),
        (85, "malformed:negative export run"),
        (145, "unsupported:zero-dimension symbol bitmap"),
        (200, "malformed:IARDX out of band"),
        (540, "malformed:IARDY outside signed 32-bit range"),
        (1328, "malformed:IAEX out of band"),
        (2351, "malformed:IARDY out of band"),
        (3549, "malformed:export run overshoot"),
        (13188, "malformed:IARDX outside signed 32-bit range"),
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
    for (seed, expected) in witnesses {
        let mut x = seed
            .wrapping_mul(0x9e3779b97f4a7c15)
            .wrapping_add(0x123456789abcdef);
        let mut body = [0u8; 32];
        body[0] = 0xee;
        for byte in &mut body[1..30] {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *byte = x as u8;
        }
        body[30..].copy_from_slice(&[0xff, 0xac]);
        let observed = run(
            &body,
            1,
            1,
            0,
            budget,
            RefinementBudget::default(),
            RefinementDictionaryBudget::default(),
        );
        let error = observed.result.unwrap_err();
        let label = match error.kind {
            RefinementDictionaryErrorKind::Malformed(s) => format!("malformed:{s}"),
            RefinementDictionaryErrorKind::Unsupported { feature, .. } => {
                format!("unsupported:{feature}")
            }
            other => format!("other:{other:?}"),
        };
        assert_eq!(label, expected, "seed {seed}");
        assert_eq!(error.segment, 2);
        assert!(error.offset > 0, "seed {seed}");
        assert!(error.progress.poisoned, "seed {seed}");
        assert!(error.progress.completed_symbols <= 1);
        assert!(observed.output.len() <= 8);
    }
}

#[test]
fn later_symbol_reads_the_new_store_only_after_an_explicit_flush() {
    // A bounded mutation of the two-refinement prefix selects active IAID 1
    // (the first new symbol) for the second target and an in-range reference
    // row. The later malformed width control is outside this prefix check.
    let mut body = TWO_REFINEMENTS_PREFIX;
    body[4] ^= 2;
    body[5] ^= 8;
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
        RefinementDictionaryErrorKind::Refinement(ref nested)
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
) -> (
    Option<RefinementDictionaryError>,
    caj2pdf_core::jbig2::refinement_dictionary::RefinementDictionaryProgress,
    Vec<u8>,
) {
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
    let mut banks =
        IaidContextBanks::with_bitmap_contexts(1, 1024, &limits, &MqBudget::default()).unwrap();
    let mut decoder = ready(RefinementDictionaryDecoder::new(
        &mut source,
        &header,
        &imported_header,
        &imported_report,
        &mut imported_source,
        0,
        &mut new_source,
        &mut sink,
        0,
        &table,
        &mut banks,
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
        RefinementDictionaryErrorKind::Poisoned
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
        assert!(matches!(
            error.kind,
            RefinementDictionaryErrorKind::Refinement(_)
        ));
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
        matches!(error.kind, RefinementDictionaryErrorKind::Refinement(ref nested)
        if matches!(nested.kind, RefinementErrorKind::Cancelled))
    );
    assert!(progress.poisoned);
    assert_eq!(progress.refinement.output_bytes_written, 1);
    assert_eq!(progress.refinement.sink_writes, 1);
    assert_eq!(bytes, vec![0x80]);
    assert!(progress.source_bytes_fetched() >= 24 + COMPLETE_ONE.len() as u64);
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
    assert!(matches!(error.kind, RefinementDictionaryErrorKind::Sink(_)));
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
        RefinementDictionaryErrorKind::Cancelled
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
        RefinementDictionaryErrorKind::Cancelled
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
        RefinementDictionaryErrorKind::Malformed("exported symbol total")
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
                    RefinementDictionaryErrorKind::Malformed("export run overshoot")
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
    assert!(matches!(error.kind, RefinementDictionaryErrorKind::Mq(_)));
    assert_eq!(error.progress.completed_symbols, 1);
    assert_eq!(error.progress.export_runs, 2);
    assert!(error.progress.poisoned);
    assert!(error.offset > 0);
    assert_eq!(observed.output.len(), 1);
}
