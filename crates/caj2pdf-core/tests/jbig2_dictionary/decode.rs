// SPDX-License-Identifier: MIT

//! Invented arithmetic states test the public dictionary model, not T.88
//! Table E.1 or external CAJ/HN symbol-pixel compatibility.

use super::*;

fn segment(
    flags: u16,
    at: &[(i8, i8)],
    refinement_at: &[(i8, i8)],
    exported: u32,
    new: u32,
    body: &[u8],
    references: &[u8],
) -> Source {
    let mut data = Vec::new();
    data.extend_from_slice(&flags.to_be_bytes());
    for &(x, y) in at {
        data.extend_from_slice(&[x as u8, y as u8]);
    }
    for &(x, y) in refinement_at {
        data.extend_from_slice(&[x as u8, y as u8]);
    }
    data.extend_from_slice(&exported.to_be_bytes());
    data.extend_from_slice(&new.to_be_bytes());
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
    bytes.push(1);
    bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&data);
    Source::new(bytes)
}

fn banks() -> IntegerContextBanks {
    IntegerContextBanks::with_extra_contexts(1024, &Limits::default(), &MqBudget::default())
        .unwrap()
}

fn decode_error(
    body: &[u8],
    new_symbols: u32,
    budget: DictionaryBudget,
    limits: Limits,
) -> DictionaryError {
    let mut source = segment(0x0800, &[(2, -1)], &[], 0, new_symbols, body, &[]);
    let hdr = header(&mut source);
    let mut store = Store {
        max_write: 1,
        ..Store::default()
    };
    let mut banks = banks();
    let table = table();
    let mut decoder = match ready(DirectDictionaryDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut banks,
        &mut store,
        &limits,
        &CancelAfter::Never,
        MqBudget::default(),
        budget,
    )) {
        Ok(decoder) => decoder,
        Err(error) => panic!("decoder construction failed: {error}"),
    };
    ready(decoder.decode()).unwrap_err()
}

fn assert_resource(error: DictionaryError, expected: &str) {
    assert!(
        matches!(&error.kind, DictionaryErrorKind::LimitExceeded { resource, .. } if resource == &expected),
        "{error}"
    );
    assert!(error.progress.poisoned);
}

#[allow(clippy::too_many_arguments)]
fn constructor_error(
    flags: u16,
    at: &[(i8, i8)],
    exported_symbols: u32,
    new_symbols: u32,
    body: &[u8],
    budget: DictionaryBudget,
    limits: Limits,
) -> DictionaryError {
    let mut source = segment(flags, at, &[], exported_symbols, new_symbols, body, &[]);
    let hdr = header(&mut source);
    let mut store = Store::default();
    let mut banks = banks();
    let table = table();
    let error = match ready(DirectDictionaryDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut banks,
        &mut store,
        &limits,
        &CancelAfter::Never,
        MqBudget::default(),
        budget,
    )) {
        Ok(_) => panic!("unsupported dictionary accepted"),
        Err(error) => error,
    };
    assert!(store.bytes.is_empty());
    assert!(!store.flushed);
    error
}

#[test]
fn zero_symbol_dictionary_consumes_one_zero_iaex_run_and_finishes() {
    let mut source = segment(0x0800, &[(2, -1)], &[], 0, 0, &[0xff, 0xac], &[]);
    let hdr = header(&mut source);
    source.max_read = 1;
    source.max_request = 0;
    let mut store = Store {
        max_write: 1,
        ..Store::default()
    };
    let mut banks = self::banks();
    let table = self::table();
    let limits = Limits::default();
    let mut decoder = ready(DirectDictionaryDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut banks,
        &mut store,
        &limits,
        &CancelAfter::Never,
        MqBudget::default(),
        DictionaryBudget::default(),
    ))
    .unwrap();
    let report = ready(decoder.decode()).unwrap();
    assert_eq!(report.header.mode, DictionaryMode::ArithmeticDirect);
    assert_eq!(report.header.body.length, 2);
    assert!(report.catalog.new_symbols.is_empty());
    assert!(report.catalog.exported_symbols.is_empty());
    assert_eq!(
        (
            report.progress.height_classes,
            report.progress.export_runs,
            report.progress.mq.unwrap().symbols_decoded
        ),
        (0, 1, 4)
    );
    assert_eq!(report.progress.header_bytes_fetched, hdr.header_length + 12);
    assert!(report.progress.source_bytes_fetched() >= hdr.header_length + 14);
    assert!(!report.progress.poisoned);
    drop(decoder);
    assert!(store.bytes.is_empty());
    assert!(store.flushed);
}

#[test]
fn one_symbol_uses_a_single_mq_unit_and_stores_an_unexported_bitmap() {
    // This byte stream was selected by a bounded search against the
    // independently specified IADH=1, IADW=1, pixel=0, IADW=OOB, IAEX=1
    // control sequence under the invented table above. No standard row or
    // external document byte is embedded here.
    let body = [
        0xee, 0xbf, 0x41, 0xc7, 0x00, 0x54, 0x0f, 0xe4, 0x11, 0x07, 0x7f, 0x2f, 0xff, 0xac,
    ];
    let mut source = segment(0x0800, &[(2, -1)], &[], 0, 1, &body, &[]);
    let hdr = header(&mut source);
    source.max_read = 1;
    source.max_request = 0;
    let mut store = Store {
        max_write: 1,
        ..Store::default()
    };
    let mut banks = banks();
    let table = table();
    let limits = Limits {
        io_chunk_bytes: 2,
        ..Limits::default()
    };
    let mut decoder = ready(DirectDictionaryDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut banks,
        &mut store,
        &limits,
        &CancelAfter::Never,
        MqBudget::default(),
        DictionaryBudget::default(),
    ))
    .unwrap();
    let report = ready(decoder.decode()).unwrap();
    assert_eq!(report.progress.completed_symbols, 1);
    assert_eq!(
        (report.progress.height_classes, report.progress.export_runs),
        (1, 1)
    );
    assert_eq!(report.progress.mq.unwrap().symbols_decoded, 17);
    assert_eq!(report.catalog.new_symbols.len(), 1);
    assert_eq!(
        (
            report.catalog.new_symbols[0].width,
            report.catalog.new_symbols[0].height,
            report.catalog.new_symbols[0].row_stride,
            report.catalog.new_symbols[0].relative_store_offset,
            report.catalog.new_symbols[0].stored_bytes
        ),
        (1, 1, 1, 0, 1)
    );
    assert!(report.catalog.exported_symbols.is_empty());
    drop(decoder);
    assert_eq!(store.bytes, [0]);
    assert!(store.flushed);
    assert!(source.max_request <= 2);
}

#[test]
fn descriptor_offset_is_relative_to_the_first_append_in_a_prefilled_store() {
    let mut source = segment(0x0800, &[(2, -1)], &[], 0, 1, &ONE_SYMBOL, &[]);
    let hdr = header(&mut source);
    let mut store = Store {
        bytes: vec![0xa5, 0x5a],
        max_write: 1,
        ..Store::default()
    };
    let mut banks = banks();
    let table = table();
    let limits = Limits::default();
    let mut decoder = match ready(DirectDictionaryDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut banks,
        &mut store,
        &limits,
        &CancelAfter::Never,
        MqBudget::default(),
        DictionaryBudget::default(),
    )) {
        Ok(decoder) => decoder,
        Err(error) => panic!("first append was rejected: {error}"),
    };
    let report = ready(decoder.decode()).unwrap();
    assert_eq!(report.catalog.new_symbols[0].relative_store_offset, 0);
    assert_eq!(report.catalog.new_symbols[0].stored_bytes, 1);
    assert_eq!(report.progress.stored_bitmap_bytes, 1);
    drop(decoder);
    assert_eq!(store.bytes, [0xa5, 0x5a, 0x00]);
}

#[test]
fn successive_symbols_share_bitmap_statistics_but_reset_row_history() {
    // Height 1; widths 1 then +0; bitmaps 1 then 0; IADW OOB; IAEX 2
    // leaves both stored symbols unexported. The two bits use GB context 0
    // despite the preceding symbol's set pixel.
    let body = [
        0xee, 0x7d, 0xf6, 0xc9, 0x51, 0xf2, 0x81, 0xb1, 0x95, 0x2a, 0x6d, 0x8d, 0xff, 0xac,
    ];
    let mut source = segment(0x0800, &[(2, -1)], &[], 0, 2, &body, &[]);
    let hdr = header(&mut source);
    let mut store = Store {
        max_write: 1,
        ..Store::default()
    };
    let mut banks = banks();
    let table = table();
    let limits = Limits::default();
    let mut decoder = ready(DirectDictionaryDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut banks,
        &mut store,
        &limits,
        &CancelAfter::Never,
        MqBudget::default(),
        DictionaryBudget::default(),
    ))
    .unwrap();
    let report = ready(decoder.decode()).unwrap();
    assert_eq!(report.progress.completed_symbols, 2);
    assert_eq!(
        (report.progress.height_classes, report.progress.export_runs),
        (1, 1)
    );
    assert_eq!(report.progress.mq.unwrap().symbols_decoded, 22);
    assert_eq!(report.catalog.new_symbols.len(), 2);
    assert_eq!(
        (
            report.catalog.new_symbols[0].relative_store_offset,
            report.catalog.new_symbols[1].relative_store_offset
        ),
        (0, 1)
    );
    assert_eq!(
        (
            report.catalog.new_symbols[0].stored_bytes,
            report.catalog.new_symbols[1].stored_bytes
        ),
        (1, 1)
    );
    assert!(report.catalog.exported_symbols.is_empty());
    drop(decoder);
    assert_eq!(store.bytes, [0x80, 0]);
    assert_eq!(banks.mq_contexts_mut().get(6656).unwrap().state_index, 1);
}

#[test]
fn zero_length_export_run_toggles_flag_and_catalog_keeps_store_offset() {
    // IADH 1, IADW 1, bitmap bit 1, IADW OOB, IAEX runs 0 then 1.
    let body = [
        0xee, 0x3f, 0xf7, 0xe4, 0x4f, 0x7f, 0x34, 0xf2, 0x11, 0xea, 0x2e, 0x75, 0xff, 0xac,
    ];
    let mut source = segment(0x0800, &[(2, -1)], &[], 1, 1, &body, &[]);
    let hdr = header(&mut source);
    let mut store = Store {
        max_write: 1,
        ..Store::default()
    };
    let mut banks = banks();
    let table = table();
    let limits = Limits::default();
    let mut decoder = ready(DirectDictionaryDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut banks,
        &mut store,
        &limits,
        &CancelAfter::Never,
        MqBudget::default(),
        DictionaryBudget::default(),
    ))
    .unwrap();
    let report = ready(decoder.decode()).unwrap();
    assert_eq!(
        (
            report.progress.completed_symbols,
            report.progress.export_runs,
            report.progress.mq.unwrap().symbols_decoded
        ),
        (1, 2, 21)
    );
    assert_eq!(report.catalog.new_symbols, report.catalog.exported_symbols);
    assert_eq!(report.catalog.exported_symbols[0].relative_store_offset, 0);
    drop(decoder);
    assert_eq!(store.bytes, [0x80]);
}

#[test]
fn empty_height_class_then_zero_delta_class_decodes_one_symbol() {
    // IADH 1; immediate IADW OOB (empty class); IADH 0; IADW 1;
    // pixel 0; IADW OOB; IAEX 1 (not exported).
    let body = [
        0xe7, 0xfe, 0xbf, 0x60, 0xed, 0x52, 0xca, 0x40, 0xda, 0xd8, 0xe9, 0xdb, 0xff, 0xac,
    ];
    let mut source = segment(0x0800, &[(2, -1)], &[], 0, 1, &body, &[]);
    let hdr = header(&mut source);
    let mut store = Store {
        max_write: 1,
        ..Store::default()
    };
    let mut banks = banks();
    let table = table();
    let limits = Limits::default();
    let mut decoder = ready(DirectDictionaryDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut banks,
        &mut store,
        &limits,
        &CancelAfter::Never,
        MqBudget::default(),
        DictionaryBudget::default(),
    ))
    .unwrap();
    let report = ready(decoder.decode()).unwrap();
    assert_eq!(
        (
            report.progress.completed_symbols,
            report.progress.height_classes,
            report.progress.export_runs
        ),
        (1, 2, 1)
    );
    assert_eq!(report.progress.mq.unwrap().symbols_decoded, 25);
    assert_eq!(
        (
            report.catalog.new_symbols[0].width,
            report.catalog.new_symbols[0].height
        ),
        (1, 1)
    );
    drop(decoder);
    assert_eq!(store.bytes, [0]);
}

#[test]
fn signed_negative_height_delta_can_follow_a_taller_empty_class() {
    // IADH 2; immediate width OOB; IADH -1; width 1; pixel 0;
    // width OOB; IAEX 1. The second class height is 1.
    let body = [
        0xd7, 0x6e, 0xbf, 0x46, 0xfa, 0x2a, 0x9b, 0xf0, 0xca, 0x45, 0xa7, 0x7b, 0xff, 0xac,
    ];
    let mut source = segment(0x0800, &[(2, -1)], &[], 0, 1, &body, &[]);
    let hdr = header(&mut source);
    let mut store = Store {
        max_write: 1,
        ..Store::default()
    };
    let mut banks = banks();
    let table = table();
    let limits = Limits::default();
    let mut decoder = ready(DirectDictionaryDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut banks,
        &mut store,
        &limits,
        &CancelAfter::Never,
        MqBudget::default(),
        DictionaryBudget::default(),
    ))
    .unwrap();
    let report = ready(decoder.decode()).unwrap();
    assert_eq!(
        (
            report.progress.height_classes,
            report.progress.completed_symbols,
            report.progress.mq.unwrap().symbols_decoded
        ),
        (2, 1, 25)
    );
    assert_eq!(
        (
            report.catalog.new_symbols[0].width,
            report.catalog.new_symbols[0].height
        ),
        (1, 1)
    );
    drop(decoder);
    assert_eq!(store.bytes, [0]);
}

#[test]
fn packed_width_nine_uses_partial_writes_and_zero_padding() {
    // IADH 1, IADW 9, bitmap 110111000, IADW OOB, IAEX 1.
    // Two output bytes are 11011100 00000000, with seven padding zeros.
    let body = [
        0xeb, 0x44, 0x77, 0xe9, 0x24, 0x1c, 0x8a, 0x3b, 0xa2, 0xf0, 0xad, 0xf4, 0xff, 0xac,
    ];
    let mut source = segment(0x0800, &[(2, -1)], &[], 0, 1, &body, &[]);
    let hdr = header(&mut source);
    let mut store = Store {
        max_write: 1,
        ..Store::default()
    };
    let mut banks = banks();
    let table = table();
    let limits = Limits::default();
    let mut decoder = ready(DirectDictionaryDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut banks,
        &mut store,
        &limits,
        &CancelAfter::Never,
        MqBudget::default(),
        DictionaryBudget::default(),
    ))
    .unwrap();
    let report = ready(decoder.decode()).unwrap();
    assert_eq!(
        (
            report.progress.completed_symbols,
            report.progress.stored_bitmap_bytes,
            report.progress.sink_writes
        ),
        (1, 2, 2)
    );
    assert_eq!(report.progress.mq.unwrap().symbols_decoded, 28);
    assert_eq!(
        (
            report.catalog.new_symbols[0].width,
            report.catalog.new_symbols[0].height,
            report.catalog.new_symbols[0].row_stride,
            report.catalog.new_symbols[0].stored_bytes
        ),
        (9, 1, 2, 2)
    );
    drop(decoder);
    assert_eq!(store.bytes, [0xdc, 0]);
}

#[test]
fn alternating_export_runs_select_second_symbol_in_original_order() {
    // One class, two width-1 bitmaps (0 then 1), width OOB,
    // IAEX runs 1 at flag 0 and 1 at flag 1.
    let body = [
        0xee, 0xf9, 0xfb, 0x9a, 0xba, 0xac, 0x86, 0xce, 0x9d, 0xb5, 0x37, 0x45, 0xff, 0xac,
    ];
    let mut source = segment(0x0800, &[(2, -1)], &[], 1, 2, &body, &[]);
    let hdr = header(&mut source);
    let mut store = Store {
        max_write: 1,
        ..Store::default()
    };
    let mut banks = banks();
    let table = table();
    let limits = Limits::default();
    let mut decoder = ready(DirectDictionaryDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut banks,
        &mut store,
        &limits,
        &CancelAfter::Never,
        MqBudget::default(),
        DictionaryBudget::default(),
    ))
    .unwrap();
    let report = ready(decoder.decode()).unwrap();
    assert_eq!(
        (
            report.progress.completed_symbols,
            report.progress.export_runs,
            report.progress.mq.unwrap().symbols_decoded
        ),
        (2, 2, 26)
    );
    assert_eq!(report.catalog.new_symbols.len(), 2);
    assert_eq!(
        report.catalog.exported_symbols,
        vec![report.catalog.new_symbols[1]]
    );
    assert_eq!(report.catalog.exported_symbols[0].relative_store_offset, 1);
    drop(decoder);
    assert_eq!(store.bytes, [0, 0x80]);
}

#[test]
fn refinement_header_is_classified_and_refused_before_mq_or_output() {
    let mut source = segment(0x1802, &[(2, -1)], &[], 8, 2, &[0, 0, 0xff, 0xac], &[1]);
    let hdr = header(&mut source);
    let parsed = ready(read_dictionary_data_header(
        &mut source,
        &hdr,
        &Limits::default(),
        DictionaryBudget::default(),
        &CancelAfter::Never,
    ))
    .unwrap();
    assert_eq!(parsed.mode, DictionaryMode::ArithmeticRefinementAggregate);
    assert_eq!(parsed.refinement_template, 1);
    assert_eq!(parsed.refinement_at_count, 0);
    assert_eq!(parsed.header_bytes, 12);
    assert_eq!(parsed.body.length, 4);
    let mut store = Store {
        max_write: 1,
        ..Store::default()
    };
    let mut banks = banks();
    let error = match ready(DirectDictionaryDecoder::new(
        &mut source,
        &hdr,
        &table(),
        &mut banks,
        &mut store,
        &Limits::default(),
        &CancelAfter::Never,
        MqBudget::default(),
        DictionaryBudget::default(),
    )) {
        Ok(_) => panic!("refinement/aggregate accepted"),
        Err(error) => error,
    };
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Unsupported {
            feature: "symbol dictionary refinement/aggregation",
            ..
        }
    ));
    assert!(store.bytes.is_empty());
    assert!(!store.flushed);
}

#[test]
fn malformed_and_bounded_headers_never_enter_mq() {
    let cases = [
        (
            0x8800,
            vec![(2, -1)],
            vec![],
            1,
            1,
            vec![0xff, 0xac],
            "reserved",
        ),
        (
            0x0804,
            vec![(2, -1)],
            vec![],
            1,
            1,
            vec![0xff, 0xac],
            "Huffman selection",
        ),
        (
            0x1800,
            vec![(2, -1)],
            vec![],
            1,
            1,
            vec![0xff, 0xac],
            "unused refinement",
        ),
        (
            0x0800,
            vec![(0, 0)],
            vec![],
            1,
            1,
            vec![0xff, 0xac],
            "undecoded pixel",
        ),
        (0x0800, vec![(2, -1)], vec![], 1, 1, vec![0], "terminal"),
    ];
    for (flags, at, rat, exported, new, body, expected) in cases {
        let mut source = segment(flags, &at, &rat, exported, new, &body, &[]);
        let hdr = header(&mut source);
        let error = ready(read_dictionary_data_header(
            &mut source,
            &hdr,
            &Limits::default(),
            DictionaryBudget::default(),
            &CancelAfter::Never,
        ))
        .unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
        if flags == 0x8800 || flags == 0x0804 || flags == 0x1800 {
            assert_eq!(error.offset, hdr.data.offset);
        } else if at[0] == (0, 0) {
            assert_eq!(error.offset, hdr.data.offset + 2);
        }
    }
    let mut source = segment(0x0800, &[(2, -1)], &[], 1, 1, &[0xff, 0xac], &[]);
    let hdr = header(&mut source);
    let budget = DictionaryBudget {
        max_data_header_bytes: 3,
        ..DictionaryBudget::default()
    };
    let before = source.read_calls;
    let error = ready(read_dictionary_data_header(
        &mut source,
        &hdr,
        &Limits::default(),
        budget,
        &CancelAfter::Never,
    ))
    .unwrap_err();
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::LimitExceeded {
            resource: "dictionary header bytes",
            ..
        }
    ));
    assert_eq!(source.read_calls - before, 6); // framing, then flags; AT refused before I/O.
    assert_eq!(error.progress.header_bytes_fetched, hdr.header_length + 2);

    let mut source = segment(0x0800, &[(2, -1)], &[], 3, 3, &[0xff, 0xac], &[]);
    let hdr = header(&mut source);
    let budget = DictionaryBudget {
        max_exported_symbols: 2,
        ..DictionaryBudget::default()
    };
    let error = ready(read_dictionary_data_header(
        &mut source,
        &hdr,
        &Limits::default(),
        budget,
        &CancelAfter::Never,
    ))
    .unwrap_err();
    assert_eq!(error.offset, hdr.data.offset + 4);
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::LimitExceeded {
            resource: "exported symbols",
            ..
        }
    ));
    let budget = DictionaryBudget {
        max_new_symbols: 2,
        ..DictionaryBudget::default()
    };
    let error = ready(read_dictionary_data_header(
        &mut source,
        &hdr,
        &Limits::default(),
        budget,
        &CancelAfter::Never,
    ))
    .unwrap_err();
    assert_eq!(error.offset, hdr.data.offset + 8);
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::LimitExceeded {
            resource: "new symbols",
            ..
        }
    ));
}

#[test]
fn terminal_failure_reports_extra_physical_fetch_and_poison() {
    let mut body = vec![0; 300];
    body[..2].copy_from_slice(&[0xf0, 0x00]);
    body[298..].copy_from_slice(&[0xff, 0xab]);
    let mut source = segment(0x0800, &[(2, -1)], &[], 0, 0, &body, &[]);
    let hdr = header(&mut source);
    let mut store = Store {
        max_write: 1,
        ..Store::default()
    };
    let mut banks = banks();
    let table = table();
    let limits = Limits::default();
    let mut decoder = ready(DirectDictionaryDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut banks,
        &mut store,
        &limits,
        &CancelAfter::Never,
        MqBudget::default(),
        DictionaryBudget::default(),
    ))
    .unwrap();
    assert_eq!(decoder.progress().mq.unwrap().source_bytes_fetched, 256);
    let error = ready(decoder.decode()).unwrap_err();
    assert!(
        matches!(error.kind, DictionaryErrorKind::Mq(ref mq) if matches!(mq.kind, MqErrorKind::InvalidMarker(0xab)))
    );
    assert_eq!(error.progress.mq.unwrap().source_bytes_fetched, 258);
    assert!(error.progress.poisoned);
    assert_eq!(decoder.progress().mq.unwrap().source_bytes_fetched, 258);
    assert!(matches!(
        ready(decoder.decode()).unwrap_err().kind,
        DictionaryErrorKind::Poisoned
    ));
    drop(decoder);
    assert!(store.bytes.is_empty());
    assert!(!store.flushed);
}

#[test]
fn dropped_pending_finish_keeps_live_mq_progress_and_poison() {
    let mut body = vec![0; 300];
    body[..2].copy_from_slice(&[0xf0, 0x00]);
    body[298..].copy_from_slice(&[0xff, 0xac]);
    let mut source = segment(0x0800, &[(2, -1)], &[], 0, 0, &body, &[]);
    let hdr = header(&mut source);
    source.max_read = 1;
    source.pending_at = Some(hdr.data.offset + 12 + 299);
    let mut store = Store {
        max_write: 1,
        ..Store::default()
    };
    let mut banks = banks();
    let table = table();
    let limits = Limits::default();
    let mut decoder = ready(DirectDictionaryDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut banks,
        &mut store,
        &limits,
        &CancelAfter::Never,
        MqBudget::default(),
        DictionaryBudget::default(),
    ))
    .unwrap();
    let mut future = Box::pin(decoder.decode());
    assert!(matches!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
    drop(future);
    let progress = decoder.progress();
    assert!(progress.poisoned);
    assert!(progress.mq.unwrap().poisoned);
    assert_eq!(progress.mq.unwrap().source_bytes_fetched, 257);
    assert!(matches!(
        ready(decoder.decode()).unwrap_err().kind,
        DictionaryErrorKind::Poisoned
    ));
    drop(decoder);
    assert!(store.bytes.is_empty());
    assert!(!store.flushed);
}

#[test]
fn failed_zero_and_overreported_store_writes_poison_partial_catalog() {
    let body = [
        0xee, 0xbf, 0x41, 0xc7, 0x00, 0x54, 0x0f, 0xe4, 0x11, 0x07, 0x7f, 0x2f, 0xff, 0xac,
    ];
    for mode in 0..3 {
        let mut source = segment(0x0800, &[(2, -1)], &[], 0, 1, &body, &[]);
        let hdr = header(&mut source);
        let mut store = Store {
            max_write: if mode == 1 { 0 } else { 1 },
            fail: mode == 0,
            overreport: mode == 2,
            ..Store::default()
        };
        let mut banks = banks();
        let table = table();
        let limits = Limits::default();
        let mut decoder = ready(DirectDictionaryDecoder::new(
            &mut source,
            &hdr,
            &table,
            &mut banks,
            &mut store,
            &limits,
            &CancelAfter::Never,
            MqBudget::default(),
            DictionaryBudget::default(),
        ))
        .unwrap();
        let error = ready(decoder.decode()).unwrap_err();
        assert!(matches!(
            error.kind,
            DictionaryErrorKind::Sink(_) | DictionaryErrorKind::Malformed("sink write length")
        ));
        assert_eq!(error.progress.completed_symbols, 0);
        assert!(error.progress.poisoned);
        assert!(matches!(
            ready(decoder.decode()).unwrap_err().kind,
            DictionaryErrorKind::Poisoned
        ));
        drop(decoder);
        assert!(store.bytes.is_empty());
        assert!(!store.flushed);
    }
}

#[test]
fn dropped_pending_store_and_cancellation_after_partial_row_are_terminal() {
    let body = [
        0xee, 0xbf, 0x41, 0xc7, 0x00, 0x54, 0x0f, 0xe4, 0x11, 0x07, 0x7f, 0x2f, 0xff, 0xac,
    ];
    let mut source = segment(0x0800, &[(2, -1)], &[], 0, 1, &body, &[]);
    let hdr = header(&mut source);
    let mut store = Store {
        max_write: 1,
        pending: true,
        ..Store::default()
    };
    let mut banks = banks();
    let table = table();
    let limits = Limits::default();
    let mut decoder = ready(DirectDictionaryDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut banks,
        &mut store,
        &limits,
        &CancelAfter::Never,
        MqBudget::default(),
        DictionaryBudget::default(),
    ))
    .unwrap();
    let mut future = Box::pin(decoder.decode());
    assert!(matches!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
    drop(future);
    assert!(decoder.progress().poisoned);
    assert!(matches!(
        ready(decoder.decode()).unwrap_err().kind,
        DictionaryErrorKind::Poisoned
    ));
    drop(decoder);
    assert!(store.bytes.is_empty());

    let body = [
        0xeb, 0x44, 0x77, 0xe9, 0x24, 0x1c, 0x8a, 0x3b, 0xa2, 0xf0, 0xad, 0xf4, 0xff, 0xac,
    ];
    let mut source = segment(0x0800, &[(2, -1)], &[], 0, 1, &body, &[]);
    let hdr = header(&mut source);
    let signal = Rc::new(Cell::new(false));
    let cancellation = CancelAfter::while_set(signal.clone());
    let mut store = Store {
        max_write: 1,
        cancel_after_write: Some(signal),
        ..Store::default()
    };
    let mut banks = self::banks();
    let table = self::table();
    let mut decoder = ready(DirectDictionaryDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut banks,
        &mut store,
        &limits,
        &cancellation,
        MqBudget::default(),
        DictionaryBudget::default(),
    ))
    .unwrap();
    let error = ready(decoder.decode()).unwrap_err();
    assert!(matches!(error.kind, DictionaryErrorKind::Cancelled));
    assert_eq!(
        (
            error.progress.completed_symbols,
            error.progress.stored_bitmap_bytes
        ),
        (0, 1)
    );
    assert!(error.progress.poisoned);
    drop(decoder);
    assert_eq!(store.bytes, [0xdc]);
    assert!(!store.flushed);
}

#[test]
fn conditional_dictionary_headers_classify_each_mode_and_field_layout() {
    let cases = [
        (0x0000, 4, 0, DictionaryMode::ArithmeticDirect),
        (0x0400, 1, 0, DictionaryMode::ArithmeticDirect),
        (0x0c00, 1, 0, DictionaryMode::ArithmeticDirect),
        (0x0002, 4, 2, DictionaryMode::ArithmeticRefinementAggregate),
        (0x1002, 4, 0, DictionaryMode::ArithmeticRefinementAggregate),
        (0x0001, 0, 0, DictionaryMode::HuffmanDirect),
        (0x0003, 0, 2, DictionaryMode::HuffmanRefinementAggregate),
    ];
    for (flags, at_count, refinement_count, expected_mode) in cases {
        let at = vec![(2, -1); at_count];
        let refinement_at = vec![(-1, -2); refinement_count];
        let mut source = segment(flags, &at, &refinement_at, 2, 3, &[0xff, 0xac], &[]);
        let hdr = header(&mut source);
        let parsed = ready(read_dictionary_data_header(
            &mut source,
            &hdr,
            &Limits::default(),
            DictionaryBudget::default(),
            &CancelAfter::Never,
        ))
        .unwrap();
        assert_eq!(parsed.mode, expected_mode, "flags {flags:#06x}");
        assert_eq!(parsed.at_count, at_count as u8);
        assert_eq!(parsed.refinement_at_count, refinement_count as u8);
        assert_eq!(
            parsed.header_bytes,
            10 + 2 * (at_count + refinement_count) as u64
        );
        assert_eq!(parsed.body.length, 2);
        assert_eq!(parsed.exported_symbols, 2);
        assert_eq!(parsed.new_symbols, 3);
        if refinement_count != 0 {
            assert_eq!(parsed.refinement_at[0], (-1, -2));
        }
    }
}

#[test]
fn invalid_flag_combinations_are_located_at_flags_before_mq() {
    let cases = [
        (0x0009, "reserved Huffman selector"),
        (0x0801, "Huffman dictionary template"),
        (0x0101, "Huffman direct bitmap flags"),
        (0x0080, "arithmetic dictionary Huffman selection flags"),
    ];
    for (flags, expected) in cases {
        let mut source = segment(flags, &[], &[], 0, 0, &[0xff, 0xac], &[]);
        let hdr = header(&mut source);
        let error = ready(read_dictionary_data_header(
            &mut source,
            &hdr,
            &Limits::default(),
            DictionaryBudget::default(),
            &CancelAfter::Never,
        ))
        .unwrap_err();
        assert_eq!(error.offset, hdr.data.offset);
        assert!(error.to_string().contains(expected), "{error}");
        assert_eq!(error.progress.header_bytes_fetched, hdr.header_length + 2);
    }
}

#[test]
fn decoded_geometry_and_output_limits_stop_before_excess_work() {
    let defaults = DictionaryBudget::default();
    let cases = [
        (
            DictionaryBudget {
                max_height_classes: 0,
                ..defaults
            },
            "height classes",
        ),
        (
            DictionaryBudget {
                max_height: 0,
                ..defaults
            },
            "height class",
        ),
        (
            DictionaryBudget {
                max_width: 0,
                ..defaults
            },
            "symbol width",
        ),
        (
            DictionaryBudget {
                max_pixels_per_symbol: 0,
                ..defaults
            },
            "symbol pixels",
        ),
        (
            DictionaryBudget {
                max_total_pixels: 0,
                ..defaults
            },
            "dictionary pixels",
        ),
        (
            DictionaryBudget {
                max_bytes_per_symbol: 0,
                ..defaults
            },
            "symbol bytes",
        ),
        (
            DictionaryBudget {
                max_stored_bitmap_bytes: 0,
                ..defaults
            },
            "stored bitmap bytes",
        ),
        (
            DictionaryBudget {
                max_sink_writes: 0,
                ..defaults
            },
            "sink writes",
        ),
        (
            DictionaryBudget {
                max_export_runs: 0,
                ..defaults
            },
            "export runs",
        ),
    ];
    for (budget, resource) in cases {
        assert_resource(
            decode_error(&ONE_SYMBOL, 1, budget, Limits::default()),
            resource,
        );
    }
    assert_resource(
        decode_error(
            &ONE_SYMBOL,
            1,
            defaults,
            Limits {
                max_output_bytes: 0,
                ..Limits::default()
            },
        ),
        "output bytes",
    );
    assert_resource(
        decode_error(
            &TWO_SYMBOLS,
            2,
            DictionaryBudget {
                max_total_pixels: 1,
                ..defaults
            },
            Limits::default(),
        ),
        "dictionary pixels",
    );
    assert_resource(
        decode_error(
            &TWO_SYMBOLS,
            2,
            DictionaryBudget {
                max_stored_bitmap_bytes: 1,
                ..defaults
            },
            Limits::default(),
        ),
        "stored bitmap bytes",
    );
    assert_resource(
        decode_error(
            &TWO_SYMBOLS,
            2,
            DictionaryBudget {
                max_sink_writes: 1,
                ..defaults
            },
            Limits::default(),
        ),
        "sink writes",
    );
}

#[test]
fn working_budget_covers_row_scratch_in_addition_to_preflight_catalog() {
    let preflight_bytes = 7680 * std::mem::size_of::<MqContext>()
        + MQ_STATE_COUNT * std::mem::size_of::<MqState>()
        + 256
        + std::mem::size_of::<SymbolDescriptor>();
    assert_resource(
        decode_error(
            &ONE_SYMBOL,
            1,
            DictionaryBudget {
                max_working_bytes: preflight_bytes as u64,
                ..DictionaryBudget::default()
            },
            Limits::default(),
        ),
        "dictionary working bytes",
    );
}

#[test]
fn unsupported_modes_and_direct_features_refuse_before_mq_or_store() {
    let defaults = DictionaryBudget::default();
    type ModeCase<'a> = (u16, &'a [(i8, i8)], u32, u32, &'a str);
    let cases: [ModeCase<'_>; 6] = [
        (0x0001, &[], 0, 0, "Huffman symbol dictionary"),
        (0x0000, &[(2, -1); 4], 0, 0, "dictionary generic template"),
        (0x0900, &[(2, -1)], 0, 0, "bitmap context carry"),
        (0x0a00, &[(2, -1)], 0, 0, "bitmap context carry"),
        (0x0800, &[(1, -1)], 0, 0, "adaptive pixel"),
        (0x0800, &[(2, -1)], 2, 1, "exported count exceeds"),
    ];
    for (flags, at, exported, new, expected) in cases {
        let error = constructor_error(
            flags,
            at,
            exported,
            new,
            &[0xff, 0xac],
            defaults,
            Limits::default(),
        );
        assert!(error.to_string().contains(expected), "{error}");
        assert!(error.progress.header_bytes_fetched >= 10);
        assert!(error.progress.mq.is_none());
    }
}

#[test]
fn preflight_resource_limits_are_typed_and_count_header_io() {
    let defaults = DictionaryBudget::default();
    let cases = [
        (
            DictionaryBudget {
                max_catalog_bytes: 0,
                ..defaults
            },
            Limits::default(),
            "catalog metadata bytes",
        ),
        (
            DictionaryBudget {
                max_working_bytes: 0,
                ..defaults
            },
            Limits::default(),
            "dictionary working bytes",
        ),
        (
            DictionaryBudget {
                max_source_request_bytes: 1,
                ..defaults
            },
            Limits::default(),
            "MQ source request bytes",
        ),
        (
            defaults,
            Limits {
                io_chunk_bytes: 1,
                max_allocation_bytes: 1,
                ..Limits::default()
            },
            "catalog allocation bytes",
        ),
    ];
    for (budget, limits, expected) in cases {
        let error = constructor_error(0x0800, &[(2, -1)], 0, 1, &ONE_SYMBOL, budget, limits);
        assert!(
            matches!(&error.kind, DictionaryErrorKind::LimitExceeded { resource, .. } if resource == &expected),
            "{error}"
        );
        assert_eq!(error.progress.header_bytes_fetched, 23);
        assert!(error.progress.mq.is_none());
    }
    let error = constructor_error(
        0x0800,
        &[(2, -1)],
        0,
        1,
        &ONE_SYMBOL,
        defaults,
        Limits {
            max_input_bytes: 1,
            ..Limits::default()
        },
    );
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::LimitExceeded {
            resource: "dictionary data bytes",
            ..
        }
    ));
    assert_eq!(error.progress.header_bytes_fetched, 0);
}

#[test]
fn dictionary_body_limit_uses_the_parsed_body_length() {
    let mut source = segment(0x0800, &[(2, -1)], &[], 0, 1, &ONE_SYMBOL, &[]);
    let hdr = header(&mut source);
    let error = ready(read_dictionary_data_header(
        &mut source,
        &hdr,
        &Limits::default(),
        DictionaryBudget {
            max_body_bytes: 13,
            ..DictionaryBudget::default()
        },
        &CancelAfter::Never,
    ))
    .unwrap_err();
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::LimitExceeded {
            resource: "dictionary body bytes",
            limit: 13,
            attempted: 14,
        }
    ));
    assert_eq!(error.offset, hdr.data.offset + 12);
    assert_eq!(error.progress.header_bytes_fetched, hdr.header_length + 12);
}

#[test]
fn invalid_limits_and_zero_io_request_bound_are_rejected_before_framing_io() {
    let mut source = segment(0x0800, &[(2, -1)], &[], 0, 1, &ONE_SYMBOL, &[]);
    let hdr = header(&mut source);
    let before = source.read_calls;
    let error = ready(read_dictionary_data_header(
        &mut source,
        &hdr,
        &Limits::default(),
        DictionaryBudget {
            max_source_request_bytes: 0,
            ..DictionaryBudget::default()
        },
        &CancelAfter::Never,
    ))
    .unwrap_err();
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Malformed("zero I/O request bound")
    ));
    assert_eq!(source.read_calls, before);
    assert_eq!(error.progress.header_bytes_fetched, 0);

    let error = ready(read_dictionary_data_header(
        &mut source,
        &hdr,
        &Limits {
            io_chunk_bytes: 0,
            ..Limits::default()
        },
        DictionaryBudget::default(),
        &CancelAfter::Never,
    ))
    .unwrap_err();
    assert_eq!(error.offset, hdr.data.offset);
    assert!(matches!(
        error.kind,
        DictionaryErrorKind::Source(caj2pdf_core::Error::InvalidInput {
            reason: "I/O chunk size must be nonzero"
        })
    ));
    assert_eq!(source.read_calls, before);
}

#[test]
fn integer_decision_budget_failure_poisoned_before_any_bitmap() {
    let mut source = segment(0x0800, &[(2, -1)], &[], 0, 1, &ONE_SYMBOL, &[]);
    let hdr = header(&mut source);
    let mut store = Store {
        max_write: 1,
        ..Store::default()
    };
    let mut banks = banks();
    let table = table();
    let limits = Limits::default();
    let mut decoder = match ready(DirectDictionaryDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut banks,
        &mut store,
        &limits,
        &CancelAfter::Never,
        MqBudget {
            max_symbols: 1,
            ..MqBudget::default()
        },
        DictionaryBudget::default(),
    )) {
        Ok(decoder) => decoder,
        Err(error) => panic!("small decision budget prevented construction: {error}"),
    };
    let error = ready(decoder.decode()).unwrap_err();
    assert!(matches!(error.kind, DictionaryErrorKind::Mq(_)));
    assert_eq!(error.progress.completed_symbols, 0);
    assert_eq!(error.progress.mq.unwrap().symbols_decoded, 1);
    assert!(error.progress.poisoned);
    drop(decoder);
    assert!(store.bytes.is_empty());
}

#[test]
fn bitmap_decision_budget_failure_reports_bitmap_context_and_no_store_bytes() {
    let mut source = segment(0x0800, &[(2, -1)], &[], 0, 1, &ONE_SYMBOL, &[]);
    let hdr = header(&mut source);
    let mut store = Store {
        max_write: 1,
        ..Store::default()
    };
    let mut banks = banks();
    let table = table();
    let limits = Limits::default();
    let mut decoder = match ready(DirectDictionaryDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut banks,
        &mut store,
        &limits,
        &CancelAfter::Never,
        MqBudget {
            max_symbols: 8,
            ..MqBudget::default()
        },
        DictionaryBudget::default(),
    )) {
        Ok(decoder) => decoder,
        Err(error) => panic!("bitmap decision budget prevented construction: {error}"),
    };
    let error = ready(decoder.decode()).unwrap_err();
    assert!(
        matches!(&error.kind, DictionaryErrorKind::Mq(mq) if mq.context.is_some_and(|context| context >= INTEGER_CONTEXT_COUNT)),
        "{error}"
    );
    assert_eq!(error.progress.completed_symbols, 0);
    assert!(error.progress.poisoned);
    drop(decoder);
    assert!(store.bytes.is_empty());
}

// The following bodies were produced offline by an independently written
// encoder for the invented constant-probability model above (every decision
// uses Qe=0x4000 and MPS=0); only the resulting bytes are kept. Each encodes
// IADH=1 followed by the stated IADW, which is all these tests consume.
/// IADW = 2^32, the smallest width that no longer fits `u32`.
const WIDTH_2_POW_32: [u8; 7] = [0xe8, 0x00, 0x00, 0x04, 0x54, 0xff, 0xac];
/// IADW = 2^32 - 1.
const WIDTH_U32_MAX: [u8; 8] = [0xe8, 0x00, 0x00, 0x04, 0x55, 0x3f, 0xff, 0xac];
/// IADW = 48000: a 6000-byte packed row.
const WIDTH_48000: [u8; 8] = [0xe8, 0x3f, 0xff, 0x6a, 0xba, 0x7f, 0xff, 0xac];

#[test]
fn decoded_width_beyond_u32_is_malformed_while_u32_max_meets_the_width_budget() {
    let unbounded = DictionaryBudget {
        max_width: u32::MAX,
        ..DictionaryBudget::default()
    };
    let error = decode_error(&WIDTH_2_POW_32, 1, unbounded, Limits::default());
    assert!(
        matches!(
            error.kind,
            DictionaryErrorKind::Malformed("symbol width exceeds 32 bits")
        ),
        "{error}"
    );
    assert!(error.progress.poisoned);
    assert_eq!(error.progress.completed_symbols, 0);
    assert_eq!(error.progress.stored_bitmap_bytes, 0);

    // One less is a representable width, so the configured budget decides.
    let error = decode_error(
        &WIDTH_U32_MAX,
        1,
        DictionaryBudget::default(),
        Limits::default(),
    );
    assert!(
        matches!(
            error.kind,
            DictionaryErrorKind::LimitExceeded {
                resource: "symbol width",
                limit: 32_768,
                attempted: 4_294_967_295,
            }
        ),
        "{error}"
    );
    let error = decode_error(&WIDTH_U32_MAX, 1, unbounded, Limits::default());
    assert_resource(error, "symbol pixels");
}

#[test]
fn row_scratch_allocation_limit_is_exact_and_precedes_bitmap_decisions() {
    // Three 6000-byte rows need 18000 bytes of scratch, more than the MQ
    // decoder's own fixed working allocation.
    let wide = DictionaryBudget {
        max_width: 48_000,
        ..DictionaryBudget::default()
    };
    let limits = |max_allocation_bytes| Limits {
        io_chunk_bytes: 256,
        max_allocation_bytes,
        ..Limits::default()
    };
    let error = decode_error(&WIDTH_48000, 1, wide, limits(17_999));
    assert!(
        matches!(
            error.kind,
            DictionaryErrorKind::LimitExceeded {
                resource: "row scratch bytes",
                limit: 17_999,
                attempted: 18_000,
            }
        ),
        "{error}"
    );
    assert!(error.progress.poisoned);
    assert_eq!(error.progress.mq.unwrap().symbols_decoded, 42);
    assert_eq!(error.progress.sink_writes, 0);
    // With exactly enough scratch the decoder goes on to pixel decisions.
    let error = decode_error(&WIDTH_48000, 1, wide, limits(18_000));
    assert!(
        !matches!(
            error.kind,
            DictionaryErrorKind::LimitExceeded {
                resource: "row scratch bytes",
                ..
            }
        ),
        "{error}"
    );
    assert!(
        error.progress.mq.unwrap().symbols_decoded > 42,
        "{error}: {:?}",
        error.progress
    );
}

#[test]
fn fewer_exported_symbols_than_declared_are_rejected_after_the_final_run() {
    // ONE_SYMBOL ends with IAEX=1 while the export flag is off, so the only
    // symbol is not exported even though the header declares one export.
    let mut source = segment(0x0800, &[(2, -1)], &[], 1, 1, &ONE_SYMBOL, &[]);
    let hdr = header(&mut source);
    let mut store = Store {
        max_write: usize::MAX,
        ..Store::default()
    };
    let mut banks = banks();
    let table = table();
    let limits = Limits::default();
    let mut decoder = ready(DirectDictionaryDecoder::new(
        &mut source,
        &hdr,
        &table,
        &mut banks,
        &mut store,
        &limits,
        &CancelAfter::Never,
        MqBudget::default(),
        DictionaryBudget::default(),
    ))
    .unwrap();
    let error = ready(decoder.decode()).unwrap_err();
    assert!(
        matches!(
            error.kind,
            DictionaryErrorKind::Malformed("exported symbol total")
        ),
        "{error}"
    );
    assert_eq!(error.progress.completed_symbols, 1);
    assert_eq!(error.progress.export_runs, 1);
    assert!(error.progress.poisoned);
    assert!(matches!(
        ready(decoder.decode()).unwrap_err().kind,
        DictionaryErrorKind::Poisoned
    ));
    drop(decoder);
    assert_eq!(store.bytes, [0]);
    assert!(!store.flushed);
}

#[test]
fn count_field_cut_by_the_segment_length_is_truncated_before_reading_it() {
    let mut source = segment(0x0800, &[(2, -1)], &[], 0, 0, &[], &[]);
    // Keep flags, AT, the exported count, and two bytes of the new count.
    source.bytes.truncate(source.bytes.len() - 2);
    source.bytes[7..11].copy_from_slice(&10u32.to_be_bytes());
    source.advertised = source.bytes.len() as u64;
    let hdr = header(&mut source);
    let reads = source.read_calls;
    let error = ready(read_dictionary_data_header(
        &mut source,
        &hdr,
        &Limits::default(),
        DictionaryBudget::default(),
        &CancelAfter::Never,
    ))
    .unwrap_err();
    assert!(
        matches!(
            error.kind,
            DictionaryErrorKind::Truncated("new symbol count")
        ),
        "{error}"
    );
    // Data starts at byte 11; flags, AT, and the exported count were read.
    assert_eq!(error.offset, 19);
    assert_eq!(error.progress.header_bytes_fetched, 11 + 8);
    assert!(source.read_calls > reads);
}

/// Places a complete segment so its data ends at the last addressable byte.
struct AddressSpaceEnd {
    base: u64,
    bytes: Vec<u8>,
}

impl RangedSource for AddressSpaceEnd {
    fn size(&self) -> u64 {
        u64::MAX
    }
    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        let start = usize::try_from(offset - self.base).unwrap();
        let count = self
            .bytes
            .len()
            .saturating_sub(start)
            .min(destination.len());
        destination[..count].copy_from_slice(&self.bytes[start..start + count]);
        Ok(count)
    }
}

#[test]
fn header_field_at_the_end_of_the_address_space_is_an_invalid_span() {
    let mut bytes = segment(0x0800, &[(2, -1)], &[], 0, 0, &[], &[]).bytes;
    // One data byte: the two-byte flags field would end past u64::MAX.
    bytes.truncate(12);
    bytes[7..11].copy_from_slice(&1u32.to_be_bytes());
    let base = u64::MAX - bytes.len() as u64;
    let mut source = AddressSpaceEnd { base, bytes };
    let hdr = ready(read_segment_header(
        &mut source,
        SegmentSpan {
            offset: base,
            length: 12,
        },
        &Limits::default(),
        HeaderLimits::default(),
        &CancelAfter::Never,
    ))
    .unwrap();
    assert_eq!(hdr.data.offset + hdr.data.length, u64::MAX);
    let error = ready(read_dictionary_data_header(
        &mut source,
        &hdr,
        &Limits::default(),
        DictionaryBudget::default(),
        &CancelAfter::Never,
    ))
    .unwrap_err();
    assert!(
        matches!(
            error.kind,
            DictionaryErrorKind::InvalidSpan("header offset overflow")
        ),
        "{error}"
    );
    assert_eq!(error.offset, u64::MAX - 1);
    assert_eq!(error.progress.header_bytes_fetched, 11);
}

#[test]
fn cancellation_at_every_checkpoint_never_reports_a_catalog() {
    let table = table();
    let mut cancelled_runs = 0;
    for polls in 0..10_000 {
        let cancellation = CancelAfter::new(polls);
        let mut source = segment(0x0800, &[(2, -1)], &[], 0, 1, &ONE_SYMBOL, &[]);
        let hdr = header(&mut source);
        let mut store = Store {
            max_write: usize::MAX,
            ..Store::default()
        };
        let mut banks = banks();
        let limits = Limits::default();
        let result = ready(async {
            let mut decoder = DirectDictionaryDecoder::new(
                &mut source,
                &hdr,
                &table,
                &mut banks,
                &mut store,
                &limits,
                &cancellation,
                MqBudget::default(),
                DictionaryBudget::default(),
            )
            .await?;
            decoder.decode().await
        });
        match result {
            Ok(report) => {
                assert_eq!(report.progress.completed_symbols, 1);
                assert_eq!(store.bytes, [0]);
                assert!(store.flushed);
                // Cancellation was observed at a checkpoint in every earlier run.
                assert!(cancelled_runs > 20, "{cancelled_runs}");
                return;
            }
            Err(error) => {
                let cancelled = match &error.kind {
                    DictionaryErrorKind::Cancelled => true,
                    DictionaryErrorKind::Mq(inner) => matches!(inner.kind, MqErrorKind::Cancelled),
                    DictionaryErrorKind::Header(inner) => {
                        matches!(inner.kind, HeaderErrorKind::Cancelled)
                    }
                    _ => false,
                };
                assert!(cancelled, "poll {polls}: {error}");
                assert!(store.bytes.len() <= 1);
                cancelled_runs += 1;
            }
        }
    }
    panic!("decode never completed without cancellation");
}

#[test]
fn allocation_failure_message_and_formatter_errors_are_reported() {
    // Catalog and row reservation failures need a real allocator failure;
    // the message is still part of the public error contract.
    let error = DictionaryError {
        segment: 4,
        offset: 9,
        progress: Box::default(),
        kind: DictionaryErrorKind::AllocationFailed,
    };
    assert_eq!(
        error.to_string(),
        "JBIG2 symbol dictionary segment 4 at source byte 9: allocation failed"
    );
    assert!(std::error::Error::source(&error).is_none());
    common::assert_display_propagates_fmt_error(&error);
}
