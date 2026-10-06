// SPDX-License-Identifier: MIT

//! Original small dictionaries, MQ-coded for the standard T.88 states. The
//! source bytes here are synthetic test controls, not external document data.

mod common;

use caj2pdf_core::jbig2::{
    SegmentHeader, SegmentSpan,
    dictionary::{
        DictionaryCatalog, DictionaryProgress, DictionaryReport, DictionaryStores,
        ImportedDictionary, StoredSymbol, SymbolDescriptor, SymbolDictionaryDecoder, SymbolStore,
        coding_unit_contexts, read_dictionary_data_header,
    },
    iaid::IAID_BASE,
    integer::{BITMAP_BASE, IntegerProcedure, IntegerValue, decode_integer},
    mq::{ArithmeticSnapshot, CodedSpan, ContextBank, ContextState, MqDecoder, MqTable},
    read_segment_header,
};
use caj2pdf_core::{
    Cancellation, Context, Error, ErrorKind, Limits, NeverCancel, Payload, RangedSource,
};

struct Bytes {
    bytes: Vec<u8>,
}

impl Bytes {
    fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    fn payload(&self) -> Payload<'_> {
        Payload::from(&self.bytes[..])
    }
}

impl RangedSource for Bytes {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> caj2pdf_core::Result<usize> {
        let mut bytes = &self.bytes[..];
        bytes.read_at(offset, destination)
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
    let header = read_segment_header(&mut source, span, &Limits::default(), &NeverCancel).unwrap();
    (source, header)
}

fn table() -> MqTable {
    MqTable::standard()
}

/// The contexts of a refinement dictionary whose IAID width is `code_len`.
fn coding_unit(code_len: u32, limits: &Limits) -> ContextBank {
    ContextBank::new(coding_unit_contexts(code_len).unwrap(), limits).unwrap()
}

fn integer_prefix(body: &[u8], procedures: &[IntegerProcedure]) -> Option<Vec<IntegerValue>> {
    let limits = Limits::default();
    let table = table();
    let mut contexts = coding_unit(1, &limits);
    let mut mq = MqDecoder::new(
        Payload::from(body),
        CodedSpan {
            offset: 0,
            length: body.len() as u64,
        },
        &table,
        &mut contexts,
        &limits,
    )
    .ok()?;
    procedures
        .iter()
        .map(|&procedure| decode_integer(&mut mq, procedure).ok())
        .collect()
}

fn imported(count: u32, width: u32, row: &[u8]) -> (SegmentHeader, DictionaryReport, Bytes) {
    assert_eq!(row.len(), width.div_ceil(8) as usize);
    let (mut source, header) = segment(1, None, 0x0800, count, count, &[0x97, 0xff, 0xac]);
    let data = read_dictionary_data_header(&mut source, &header, &Limits::default(), &NeverCancel)
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
                synthesized_inputs: 0,
                symbols_decoded: 0,
            }),
            ..DictionaryProgress::default()
        },
    };
    (header, report, Bytes::new(row.repeat(count as usize)))
}

struct Observation {
    result: Result<DictionaryReport, Error>,
    output: Vec<u8>,
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
    limits: Limits,
    cancellation: &impl Cancellation,
) -> Observation {
    let (imported_header, imported_report, imported_source) =
        imported(imported_count, imported_width, imported_row);
    let (source, header) = segment(2, Some(1), 0x1802, exported, new, body);
    let mut store = Vec::new();
    let table = table();
    let total = imported_count + new;
    let width = if total <= 1 {
        0
    } else {
        32 - (total - 1).leading_zeros()
    };
    let mut contexts = coding_unit(width, &Limits::default());
    let result = SymbolDictionaryDecoder::new(
        source.payload(),
        &header,
        Some(ImportedDictionary {
            segment: &imported_header,
            report: &imported_report,
        }),
        DictionaryStores {
            imported: &imported_source.bytes,
            imported_base: 0,
            new: &mut store,
            new_base: 0,
        },
        &table,
        &mut contexts,
        &limits,
        cancellation,
    )
    .and_then(|decoder| decoder.decode());
    let output = store;
    let base = BITMAP_BASE;
    let gr_contexts: Vec<_> = (0..1024)
        .filter(|&local| contexts.get(base + local) != Some(ContextState::default()))
        .collect();
    let gr_state_zero = contexts.get(base).unwrap().state_index;
    Observation {
        result,
        output,
        gr_contexts,
        gr_state_zero,
    }
}

fn run_with_imported(
    body: &[u8],
    imported_count: u32,
    imported_width: u32,
    imported_row: &[u8],
    new: u32,
    exported: u32,
) -> Observation {
    run_with_imported_io(
        body,
        imported_count,
        imported_width,
        imported_row,
        new,
        exported,
        Limits::default(),
        &NeverCancel,
    )
}

fn run(body: &[u8], imported_count: u32, new: u32, exported: u32) -> Observation {
    run_limited(body, imported_count, new, exported, Limits::default())
}

fn run_limited(
    body: &[u8],
    imported_count: u32,
    new: u32,
    exported: u32,
    limits: Limits,
) -> Observation {
    run_with_imported_io(
        body,
        imported_count,
        1,
        &[0x80],
        new,
        exported,
        limits,
        &NeverCancel,
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
    let observed = run(&[0x97, 0xff, 0xac], 0, 0, 0);
    let report = observed.result.unwrap();
    assert!(report.catalog.new_symbols.is_empty());
    assert!(report.catalog.exported_symbols.is_empty());
    assert_eq!(report.progress.export_runs, 1);
    assert_eq!(report.progress.mq.unwrap().symbols_decoded, 4);
    assert_eq!(observed.output, Vec::<u8>::new());
}

#[test]
fn iaai_zero_and_aggregation_are_located_typed_refusals() {
    let zero = run(&ZERO_PREFIX, 1, 1, 2);
    let error = zero.result.unwrap_err();
    assert!(
        matches!(
            error,
            Error {
                kind: ErrorKind::Malformed,
                reason: "REFAGGNINST zero",
                ..
            }
        ),
        "{error}"
    );
    assert!(zero.output.is_empty());

    let many = run(&MANY_PREFIX, 1, 1, 2);
    let error = many.result.unwrap_err();
    assert!(
        matches!(
            error,
            Error {
                kind: ErrorKind::UnsupportedFormat,
                reason: "REFAGGNINST aggregation",
                ..
            }
        ),
        "{error}"
    );
    assert!(many.output.is_empty());
}

#[test]
fn one_reference_prefix_reaches_bitmap_and_stays_in_one_mq_unit() {
    let observed = run(&ONE_PREFIX, 1, 1, 2);
    match observed.result {
        Ok(report) => {
            assert_eq!(report.progress.iaai.single_reference, 1);
            assert_eq!(report.progress.completed_symbols, 1);
            assert_eq!(report.catalog.new_symbols.len(), 1);
            assert_eq!(observed.output.len(), 1);
        }
        Err(error) => assert_eq!(error.context, Context::Jbig2 { segment: Some(2) }),
    }
}

#[test]
fn one_reference_complete_stream_stores_one_packed_symbol_and_checks_tail() {
    // A bounded test-only search selected these bytes for the independent
    // control sequence IADH=1, IADW=1, IAAI=1, IAID=0, IARDX=17,
    // IARDY=-2596, GR pixel=1, IADW=OOB, IAEX=2.
    let observed = run(&COMPLETE_ONE, 1, 1, 0);
    let report = observed.result.unwrap();
    assert_eq!(report.progress.iaai.single_reference, 1);
    assert_eq!(report.progress.completed_symbols, 1);
    assert_eq!(report.progress.height_classes, 1);
    assert_eq!(report.progress.export_runs, 1);
    assert_eq!(report.progress.mq.unwrap().symbols_decoded, 47);
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
}

#[test]
fn imported_rows_change_gr_context_before_one_refined_bitmap() {
    // IARDX=-16 and IARDY=1 align the imported pixel at (16,0) with the
    // Figure 13 (0,+1) reference tap for target (0,0).
    let set = run_with_imported(&READ_REFERENCE_PIXEL, 1, 18, &[0, 0, 0x80], 1, 0);
    let report = set.result.unwrap();
    assert_eq!(report.progress.completed_symbols, 1);
    assert_eq!(set.gr_contexts, vec![2]);
    assert_eq!(set.output, vec![0x80]);

    let clear = run_with_imported(&READ_REFERENCE_PIXEL, 1, 18, &[0, 0, 0], 1, 0);
    assert_eq!(clear.result.unwrap().progress.completed_symbols, 1);
    assert_eq!(clear.gr_contexts, vec![0]);
}

#[test]
fn a_new_export_retains_the_new_store_identity() {
    let observed = run(&READ_REFERENCE_AND_EXPORT_NEW, 1, 1, 1);
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
    assert!(matches!(error, Error { kind: ErrorKind::LimitExceeded {
            resource: "catalog allocation bytes",
            limit: 20_000,
            attempted,
        }, .. } if attempted > 20_000));
}

#[test]
fn one_iaex_view_exports_imported_then_new_with_distinct_store_owners() {
    let observed = run(&COMPLETE_BOTH_EXPORTS, 1, 1, 2);
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
    let observed = run(&TWO_REFINEMENTS_PREFIX, 1, 2, 0);
    observed.result.unwrap_err();
    // One height class of height one and two decoded pixels means both
    // symbol widths are one; the second IADW is zero.
    assert_eq!(observed.output.len(), 2);
    assert_eq!(observed.gr_contexts, vec![0]);
    assert_eq!(observed.gr_state_zero, 2);
}

#[test]
fn zero_new_symbols_can_reexport_imported_store_in_order() {
    let observed = run(&ZERO_NEW_EXPORT_IMPORTED, 1, 0, 1);
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
fn header_and_contexts_are_read_through_public_decoder() {
    let observed = run(&ONE_PREFIX, 1, 1, 2);
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
    let (imported_header, imported_report, imported_source) = imported(1, 1, &[0x80]);
    let (source, header) = segment(2, Some(1), 0x1802, 1, 0, &[0x7f, 0xff, 0xac]);
    let mut store = Vec::new();
    let limits = Limits::default();
    let table = table();
    let mut contexts = coding_unit(0, &limits);
    // An earlier coding unit adapts one integer, one bitmap, and one IAID
    // context: the first decision in a context always leaves state zero.
    let touched = [0, BITMAP_BASE, IAID_BASE];
    let earlier = [0, 0, 0xff, 0xac];
    let mut mq = MqDecoder::new(
        Payload::from(&earlier[..]),
        CodedSpan {
            offset: 0,
            length: earlier.len() as u64,
        },
        &table,
        &mut contexts,
        &limits,
    )
    .unwrap();
    for index in touched {
        mq.decode_bit(index).unwrap();
    }
    for index in touched {
        assert_ne!(contexts.get(index), Some(ContextState::default()));
    }
    let decoder = SymbolDictionaryDecoder::new(
        source.payload(),
        &header,
        Some(ImportedDictionary {
            segment: &imported_header,
            report: &imported_report,
        }),
        DictionaryStores {
            imported: &imported_source.bytes,
            imported_base: 0,
            new: &mut store,
            new_base: 0,
        },
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
    )
    .unwrap();
    drop(decoder);
    for index in touched {
        assert_eq!(contexts.get(index), Some(ContextState::default()));
    }
}

#[test]
fn the_symbol_limit_counts_imported_and_new_symbols_before_mq() {
    let limits = Limits {
        max_symbols: 1,
        ..Limits::default()
    };
    let observed = run_limited(&COMPLETE_ONE, 1, 1, 0, limits);
    let error = observed.result.unwrap_err();
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::LimitExceeded {
                resource: "total symbols",
                limit: 1,
                attempted: 2,
            },
            ..
        }
    ));
    assert!(observed.output.is_empty());
}

#[test]
fn body_mutations_have_bounded_output_and_progress() {
    let limits = Limits {
        max_image_pixels: 8,
        max_symbols: 8,
        ..Limits::default()
    };
    for position in 0..COMPLETE_ONE.len() - 2 {
        for mask in [1u8, 0x80] {
            let mut body = COMPLETE_ONE;
            body[position] ^= mask;
            let observed = run_limited(&body, 1, 1, 0, limits);
            assert!(
                observed.output.len() <= 8,
                "position {position} mask {mask}"
            );
            match observed.result {
                Ok(report) => {
                    assert!(report.progress.header_bytes_fetched <= 12);
                    assert!(report.progress.completed_symbols <= 1);
                    assert!(report.progress.export_runs <= 8);
                }
                Err(error) => assert_eq!(error.context, Context::Jbig2 { segment: Some(2) }),
            }
        }
    }
}

fn forged_error(count: u32, mutate: impl FnOnce(&mut DictionaryReport)) -> Error {
    let (imported_header, mut report, imported_source) = imported(count, 1, &[0x80]);
    mutate(&mut report);
    let (source, header) = segment(2, Some(1), 0x1802, 0, 0, &[0x97, 0xff, 0xac]);
    let mut store = Vec::new();
    let limits = Limits::default();
    let table = table();
    let width = if count <= 1 {
        0
    } else {
        32 - (count - 1).leading_zeros()
    };
    let mut contexts = coding_unit(width, &limits);
    let result = SymbolDictionaryDecoder::new(
        source.payload(),
        &header,
        Some(ImportedDictionary {
            segment: &imported_header,
            report: &report,
        }),
        DictionaryStores {
            imported: &imported_source.bytes,
            imported_base: 0,
            new: &mut store,
            new_base: 0,
        },
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
    );
    match result {
        Ok(_) => panic!("forged report was accepted"),
        Err(error) => error,
    }
}

struct PreflightCase {
    flags: u16,
    reference: Option<u8>,
    page: u8,
    body: &'static [u8],
    new: u32,
    exported: u32,
    imported_base: u64,
    new_base: u64,
    bank_width: Option<u32>,
    limits: Limits,
}

impl Default for PreflightCase {
    fn default() -> Self {
        Self {
            flags: 0x1802,
            reference: Some(1),
            page: 1,
            body: &[0x97, 0xff, 0xac],
            new: 0,
            exported: 0,
            imported_base: 0,
            new_base: 0,
            bank_width: None,
            limits: Limits::default(),
        }
    }
}

fn preflight_error(
    setup: PreflightCase,
    mutate_imported: impl FnOnce(&mut SegmentHeader, &mut DictionaryReport),
) -> Error {
    preflight_result(setup, mutate_imported).unwrap_err()
}

fn preflight_result(
    setup: PreflightCase,
    mutate_imported: impl FnOnce(&mut SegmentHeader, &mut DictionaryReport),
) -> Result<(), Error> {
    let (imported_source, mut imported_header, mut report) = {
        let (header, report, source) = imported(1, 1, &[0x80]);
        (source, header, report)
    };
    mutate_imported(&mut imported_header, &mut report);
    let (source, header) = segment_on_page(
        2,
        setup.reference,
        setup.flags,
        setup.exported,
        setup.new,
        setup.body,
        setup.page,
    );
    let mut store = Vec::new();
    let limits = setup.limits;
    let table = table();
    let total = setup.new + 1;
    let width = setup.bank_width.unwrap_or(if total <= 1 {
        0
    } else {
        32 - (total - 1).leading_zeros()
    });
    let mut contexts = coding_unit(width, &limits);
    SymbolDictionaryDecoder::new(
        source.payload(),
        &header,
        Some(ImportedDictionary {
            segment: &imported_header,
            report: &report,
        }),
        DictionaryStores {
            imported: &imported_source.bytes,
            imported_base: setup.imported_base,
            new: &mut store,
            new_base: setup.new_base,
        },
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
    )
    .map(drop)
}

#[test]
fn forged_imported_reports_fail_before_mq_and_store_access() {
    let error = forged_error(1, |report| {
        report.catalog.exported_symbols[0].symbol.row_stride = 2;
        report.catalog.new_symbols[0].row_stride = 2;
    });
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Malformed,
            reason: "noncanonical imported bitmap descriptor",
            ..
        }
    ));

    let error = forged_error(2, |report| {
        report.catalog.exported_symbols.swap(0, 1);
    });
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Malformed,
            reason: "imported exports do not follow new-symbol order",
            ..
        }
    ));

    let error = forged_error(1, |report| {
        report.progress.mq = None;
    });
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Malformed,
            reason: "imported dictionary is not a complete direct report",
            ..
        }
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
            matches!(error, Error { kind: ErrorKind::Malformed, reason: found, .. } if found == reason),
            "{error}"
        );
    }

    let error = forged_error(1, |report| {
        report.catalog.exported_symbols[0]
            .symbol
            .relative_store_offset = u64::MAX;
        report.catalog.new_symbols[0].relative_store_offset = u64::MAX;
    });
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Malformed,
            reason: "imported descriptor end overflow",
            ..
        }
    ));

    let error = preflight_error(
        PreflightCase {
            imported_base: u64::MAX,
            ..PreflightCase::default()
        },
        |_, _| {},
    );
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Malformed,
            reason: "imported store base outside the store",
            ..
        }
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
            "imported descriptor outside the store",
        ),
    ] {
        let error = forged_error(1, mutate);
        assert!(
            matches!(error, Error { kind: ErrorKind::Malformed, reason: found, .. } if found == reason)
        );
    }

    let error = forged_error(2, |report| {
        report.catalog.new_symbols[1].relative_store_offset = 0;
        report.catalog.exported_symbols[1]
            .symbol
            .relative_store_offset = 0;
    });
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Malformed,
            reason: "overlapping or unordered imported descriptors",
            ..
        }
    ));
}

#[test]
fn the_symbol_pixel_limit_blocks_output_before_refinement() {
    let limits = Limits {
        max_image_pixels: 0,
        ..Limits::default()
    };
    let observed = run_limited(&COMPLETE_ONE, 1, 1, 0, limits);
    let error = observed.result.unwrap_err();
    assert!(
        matches!(
            error,
            Error {
                kind: ErrorKind::LimitExceeded {
                    resource: "symbol pixels",
                    limit: 0,
                    ..
                },
                ..
            }
        ),
        "{error}"
    );
    assert!(observed.output.is_empty());
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
        error,
        Error {
            kind: ErrorKind::Malformed,
            ..
        }
    ));

    // A direct dictionary imports nothing.
    let error = preflight_error(
        PreflightCase {
            flags: 0x0800,
            ..PreflightCase::default()
        },
        |_, _| {},
    );
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::UnsupportedFormat,
            reason: "imported dictionary references",
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
        error,
        Error {
            kind: ErrorKind::UnsupportedFormat,
            reason: "bitmap context carry",
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
        error,
        Error {
            kind: ErrorKind::UnsupportedFormat,
            reason: "second dictionary flags or adaptive template",
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
        error,
        Error {
            kind: ErrorKind::UnsupportedFormat,
            reason: "dictionary page association",
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
        error,
        Error {
            kind: ErrorKind::Malformed,
            reason: "expected exactly the supplied dictionary reference",
            ..
        }
    ));

    let error = preflight_error(PreflightCase::default(), |header, _| {
        header.segment_type = 38;
    });
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Malformed,
            reason: "imported dictionary segment metadata",
            ..
        }
    ));

    let error = preflight_error(PreflightCase::default(), |_, report| {
        report.header.body.offset += 1;
    });
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Malformed,
            reason: "imported dictionary is not a complete direct report",
            ..
        }
    ));

    let error = preflight_error(
        PreflightCase {
            imported_base: 2,
            ..PreflightCase::default()
        },
        |_, _| {},
    );
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Malformed,
            reason: "imported store base outside the store",
            ..
        }
    ));

    let error = preflight_error(
        PreflightCase {
            new_base: 1,
            ..PreflightCase::default()
        },
        |_, _| {},
    );
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Malformed,
            ..
        }
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
        error,
        Error {
            kind: ErrorKind::Malformed,
            reason: "exported count exceeds available symbols",
            ..
        }
    ));

    let error = preflight_error(
        PreflightCase {
            bank_width: Some(1),
            ..PreflightCase::default()
        },
        |_, _| {},
    );
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Malformed,
            reason: "IAID width or GR context layout mismatch",
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
            error,
            Error {
                kind: ErrorKind::Truncated { .. },
                reason: "MQ body terminal pair",
                ..
            }
        ),
        "{error}"
    );
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
    let malformed = run(&ZERO_PREFIX, 1, 1, 2).result.unwrap_err();
    let limit = preflight_error(
        PreflightCase {
            limits: Limits {
                max_symbols: 0,
                ..Limits::default()
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
    )
    .result
    .unwrap_err();
    for error in [header, unsupported, malformed, limit, invalid, mq] {
        let display = error.to_string();
        assert!(display.contains(", segment 2: "), "{display}");
        assert!(error.offset.is_some(), "{display}");
        common::assert_display_propagates_fmt_error(&error);
        assert!(std::error::Error::source(&error).is_none(), "{display}");
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
    let limits = Limits {
        max_image_pixels: 64,
        max_symbols: 4,
        ..Limits::default()
    };
    for (body, expected) in witnesses {
        let observed = run_limited(body, 1, 1, 0, limits);
        let error = observed.result.unwrap_err();
        let label = match error.kind {
            ErrorKind::Malformed => format!("malformed:{}", error.reason),
            ErrorKind::UnsupportedFormat => format!("unsupported:{}", error.reason),
            ref other => format!("other:{other:?}"),
        };
        assert_eq!(label, expected);
        assert_eq!(error.context, Context::Jbig2 { segment: Some(2) });
        assert!(error.offset > Some(0), "{expected}");
        assert!(observed.output.len() <= 8);
    }
}

#[test]
fn a_later_symbol_refines_an_earlier_new_symbol() {
    // A two-refinement prefix that selects active IAID 1 (the first new
    // symbol) for the second target and an in-range reference row. The later
    // malformed width control is outside this prefix check.
    let body = [0x95, 0x13, 0x98, 0x97, 0x6f, 0x41, 0x9a, 0x7f, 0xff, 0xac];
    let observed = run(&body, 1, 2, 0);
    observed.result.unwrap_err();
    assert_eq!(observed.output.len(), 2);
}

#[test]
fn export_count_mismatch_refuses_partial_cross_store_catalog() {
    // The same independently chosen stream exports both the import and its
    // refined successor. Claiming only one export must fail during IAEX.
    let observed = run(&COMPLETE_BOTH_EXPORTS, 1, 1, 1);
    let error = observed.result.unwrap_err();
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Malformed,
            reason: "exported symbol total",
            ..
        }
    ));
    assert_eq!(observed.output.len(), 1);
}

#[test]
fn malformed_iaex_mutations_remain_bounded_and_expose_overshoot() {
    let mut saw_overshoot = false;
    let limits = Limits {
        max_symbols: 2,
        ..Limits::default()
    };
    for byte in 0u8..=255 {
        let observed = run_limited(&[byte, 0xff, 0xac], 0, 0, 0, limits);
        match observed.result {
            Ok(report) => assert_eq!(report.progress.export_runs, 1),
            Err(error) => {
                saw_overshoot |= matches!(
                    error,
                    Error {
                        kind: ErrorKind::Malformed,
                        reason: "export run overshoot",
                        ..
                    }
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
fn truncated_terminal_after_complete_exports_is_located() {
    let observed = run(
        &COMPLETE_BOTH_EXPORTS[..COMPLETE_BOTH_EXPORTS.len() - 1],
        1,
        1,
        2,
    );
    let error = observed.result.unwrap_err();
    assert!(error.reason.contains("MQ"));
    assert!(error.offset > Some(0));
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
        let observed = run(body, 1, 1, 0);
        let error = observed.result.unwrap_err();
        assert!(observed.output.is_empty());
        match expected_delta {
            IntegerValue::Signed(-1) => assert!(matches!(
                error,
                Error {
                    kind: ErrorKind::UnsupportedFormat,
                    reason: "REFAGGNINST aggregation",
                    ..
                }
            )),
            IntegerValue::Signed(0) => assert!(matches!(
                error,
                Error {
                    kind: ErrorKind::Malformed,
                    reason: "negative symbol dimension",
                    ..
                }
            )),
            _ => unreachable!(),
        }
    }
}

#[test]
fn cancellation_checkpoints_cover_store_and_exports() {
    let mut saw_before_store = false;
    let mut saw_after_store = false;
    for allowed in 0..180 {
        let cancellation = common::CancelAfter::new(allowed);
        let observed = run_with_imported_io(
            &COMPLETE_ONE,
            1,
            1,
            &[0x80],
            1,
            0,
            Limits::default(),
            &cancellation,
        );
        if let Err(error) = observed.result {
            assert!(observed.output.len() <= 1);
            if matches!(error.kind, ErrorKind::Cancelled) {
                saw_before_store |= observed.output.is_empty();
                saw_after_store |= observed.output.len() == 1;
            }
        }
    }
    assert!(saw_before_store);
    assert!(saw_after_store);
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
    let observed = run(&body, 1, 2, 0);
    let error = observed.result.unwrap_err();
    assert_eq!(observed.output.len(), 1);
    assert!(
        matches!(
            error,
            Error {
                kind: ErrorKind::UnsupportedFormat,
                reason: "REFAGGNINST aggregation",
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn negative_initial_width_delta_is_rejected_before_bitmap_io() {
    let body = [0x5e, 0x7f, 0xff, 0xac];
    assert_eq!(
        integer_prefix(&body, &[IntegerProcedure::Iadh, IntegerProcedure::Iadw]).unwrap(),
        [IntegerValue::Signed(3), IntegerValue::Signed(-1)]
    );
    let observed = run(&body, 1, 1, 0);
    let error = observed.result.unwrap_err();
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Malformed,
            reason: "negative symbol dimension",
            ..
        }
    ));
    assert!(observed.output.is_empty());
}

#[test]
fn height_class_oob_is_a_located_refusal_before_bitmap_output() {
    let body = [0xcf, 0xff, 0xac];
    assert_eq!(
        integer_prefix(&body, &[IntegerProcedure::Iadh]).unwrap(),
        [IntegerValue::OutOfBand]
    );
    let observed = run(&body, 1, 1, 0);
    let error = observed.result.unwrap_err();
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Malformed,
            reason: "IADH out of band",
            ..
        }
    ));
    assert!(observed.output.is_empty());
}

#[test]
fn final_iaex_count_mismatch_rejects_an_underexported_import() {
    // IAEX skips the sole imported symbol, while the header requires one
    // export. The mismatch is detected after consuming the complete run.
    let observed = run(&[0x87, 0xff, 0xac], 1, 0, 1);
    let error = observed.result.unwrap_err();
    assert!(matches!(
        error,
        Error {
            kind: ErrorKind::Malformed,
            reason: "exported symbol total",
            ..
        }
    ));
    assert!(observed.output.is_empty());
}
