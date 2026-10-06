// SPDX-License-Identifier: MIT

use super::super::text::{TextHeaderAnomaly, read_text_region_header};
use super::super::{
    SegmentSpan,
    dictionary::{DictionaryCatalog, DictionaryDataHeader, DictionaryProgress},
    iaid::IAID_BASE,
    integer::{BITMAP_BASE, BITMAP_CONTEXT_COUNT, INTEGER_CONTEXT_COUNT},
};
use super::*;
use crate::{NeverCancel, RangedSource};
use std::{cell::Cell, error::Error as _};

struct ToggleCancel(Cell<bool>);

impl Cancellation for ToggleCancel {
    fn is_cancelled(&self) -> bool {
        self.0.get()
    }
}

struct Bytes {
    data: Vec<u8>,
}

impl Bytes {
    fn new(data: Vec<u8>) -> Self {
        Self { data }
    }
}

impl RangedSource for Bytes {
    fn size(&self) -> u64 {
        self.data.len() as u64
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> crate::Result<usize> {
        let mut bytes = &self.data[..];
        bytes.read_at(offset, destination)
    }
}

fn segment(
    number: u32,
    segment_type: u8,
    referred_to: Vec<u32>,
    offset: u64,
    length: u64,
) -> SegmentHeader {
    SegmentHeader {
        number,
        segment_type,
        deferred_non_retain: false,
        page_association: 1,
        referred_to,
        data: SegmentSpan { offset, length },
        header_length: 0,
        retention: vec![0xff],
    }
}

fn text_data(flags: u16, instances: u32, body: &[u8]) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&10u32.to_be_bytes());
    data.extend_from_slice(&10u32.to_be_bytes());
    data.extend_from_slice(&0u32.to_be_bytes());
    data.extend_from_slice(&0u32.to_be_bytes());
    data.push(0);
    data.extend_from_slice(&flags.to_be_bytes());
    data.extend_from_slice(&instances.to_be_bytes());
    data.extend_from_slice(body);
    data
}

fn report(symbols: &[SymbolDescriptor]) -> DictionaryReport {
    let exported_symbols = symbols
        .iter()
        .copied()
        .map(|symbol| StoredSymbol {
            store: SymbolStore::Imported,
            store_base: 0,
            symbol,
        })
        .collect();
    DictionaryReport {
        header: DictionaryDataHeader {
            flags: 0x1802,
            mode: DictionaryMode::ArithmeticRefinementAggregate,
            template: 2,
            refinement_template: 1,
            bitmap_context_used: false,
            bitmap_context_retained: false,
            at: [(2, -1); 4],
            at_count: 1,
            refinement_at: [(0, 0); 2],
            refinement_at_count: 0,
            exported_symbols: symbols.len() as u32,
            new_symbols: 0,
            header_bytes: 0,
            body: SegmentSpan {
                offset: 100,
                length: 2,
            },
        },
        catalog: DictionaryCatalog {
            new_symbols: vec![],
            exported_symbols,
        },
        progress: DictionaryProgress {
            mq: Some(ArithmeticSnapshot {
                interval: 0,
                code: 0,
                bit_counter: 0,
                input_offset: 100,
                synthesized_inputs: 0,
                symbols_decoded: 0,
                work_done: 0,
            }),
            ..Default::default()
        },
    }
}

const ONE_PIXEL: SymbolDescriptor = SymbolDescriptor {
    width: 1,
    height: 1,
    row_stride: 1,
    relative_store_offset: 0,
    stored_bytes: 1,
};

const TWO_EXPORTED_REFINED_BODY: [u8; 15] = [
    34, 77, 192, 103, 36, 36, 136, 231, 168, 12, 152, 86, 191, 255, 172,
];

const DIAGONAL_2X2: SymbolDescriptor = SymbolDescriptor {
    width: 2,
    height: 2,
    row_stride: 1,
    relative_store_offset: 0,
    stored_bytes: 2,
};

struct Fixture {
    source: Bytes,
    text_segment: SegmentHeader,
    dictionary_segment: SegmentHeader,
    dictionary: DictionaryReport,
    imported: Bytes,
    fresh: Bytes,
    temporary: Vec<u8>,
    parsed: TextRegionHeader,
    imported_base: u64,
    fresh_base: u64,
    temporary_base: u64,
    code_len: u32,
    budget: TextInstanceBudget,
    header_budget: TextRegionBudget,
    refinement_budget: RefinementBudget,
    mq_budget: MqBudget,
    qe: u16,
}

impl Fixture {
    fn new(flags: u16, instances: u32, body: &[u8], symbols: &[SymbolDescriptor]) -> Self {
        let mut source = Bytes::new(text_data(flags, instances, body));
        let text_segment = segment(3, 6, vec![2], 0, source.size());
        let dictionary_segment = segment(2, 0, vec![1], 100, 2);
        let parsed = read_text_region_header(
            &mut source,
            &text_segment,
            &dictionary_segment,
            &Limits::default(),
            TextRegionBudget::default(),
            &NeverCancel,
        )
        .unwrap();
        let length = symbols
            .iter()
            .map(|symbol| symbol.relative_store_offset + symbol.stored_bytes)
            .max()
            .unwrap_or(0) as usize;
        let count = symbols.len() as u64;
        Self {
            source,
            text_segment,
            dictionary_segment,
            dictionary: report(symbols),
            imported: Bytes::new(vec![0x80; length]),
            fresh: Bytes::new(vec![]),
            temporary: Vec::new(),
            parsed,
            imported_base: 0,
            fresh_base: 0,
            temporary_base: 0,
            code_len: if count <= 1 {
                0
            } else {
                64 - (count - 1).leading_zeros()
            },
            budget: TextInstanceBudget::default(),
            header_budget: TextRegionBudget::default(),
            refinement_budget: RefinementBudget::default(),
            mq_budget: MqBudget::default(),
            qe: 1,
        }
    }

    fn attempt(&mut self) -> TextInstanceResult<TextInstanceProgress> {
        let limits = Limits::default();
        let table = MqTable::standard();
        let mut contexts = coding_unit(self.code_len, &limits, &self.mq_budget);
        let decoder = TextInstanceDecoder::new(
            Payload::from(&self.source.data[..]),
            &self.text_segment,
            self.parsed,
            &self.dictionary_segment,
            &self.dictionary,
            &self.imported.data,
            self.imported_base,
            &self.fresh.data,
            self.fresh_base,
            &mut self.temporary,
            self.temporary_base,
            &table,
            &mut contexts,
            &limits,
            &NeverCancel,
            self.mq_budget,
            self.header_budget,
            self.refinement_budget,
            self.budget,
        )?;
        Ok(decoder.progress())
    }

    fn decode_all(&mut self) -> TextInstanceResult<(Vec<TextInstance>, TextInstanceProgress)> {
        self.decode_all_with_policy(TextHeaderPolicy::Strict)
    }

    fn decode_all_with_policy(
        &mut self,
        policy: TextHeaderPolicy,
    ) -> TextInstanceResult<(Vec<TextInstance>, TextInstanceProgress)> {
        let limits = Limits::default();
        let table = MqTable::standard();
        let mut contexts = coding_unit(self.code_len, &limits, &self.mq_budget);
        let mut decoder = TextInstanceDecoder::new_with_header_policy(
            Payload::from(&self.source.data[..]),
            &self.text_segment,
            self.parsed,
            &self.dictionary_segment,
            &self.dictionary,
            &self.imported.data,
            self.imported_base,
            &self.fresh.data,
            self.fresh_base,
            &mut self.temporary,
            self.temporary_base,
            &table,
            &mut contexts,
            &limits,
            &NeverCancel,
            self.mq_budget,
            self.header_budget,
            self.refinement_budget,
            self.budget,
            policy,
        )?;
        let mut instances = Vec::new();
        while let Some(instance) = decoder.next_instance()? {
            instances.push(instance);
            assert!(
                instances.len() <= 8,
                "test fixture unexpectedly emitted many instances"
            );
        }
        Ok((instances, decoder.progress()))
    }
}

/// The contexts of a text region whose IAID width is `code_len`.
fn coding_unit(code_len: u32, limits: &Limits, budget: &MqBudget) -> ContextBank {
    budget
        .context_bank(coding_unit_contexts(code_len).unwrap(), limits)
        .unwrap()
}

fn preflight_reject(mut fixture: Fixture, reason: &str) -> TextInstanceError {
    let error = fixture.attempt().unwrap_err();
    assert!(format!("{:?}", error.kind).contains(reason), "{error:?}");
    error
}

#[test]
fn located_error_variants_and_progress_are_inspectable() {
    let nested_header = TextRegionError {
        segment: 3,
        offset: 23,
        bytes_fetched: 2,
        kind: super::super::text::TextRegionErrorKind::Malformed("test"),
    };
    let nested_mq = ArithmeticError {
        coder: Some(super::super::mq::Coder::T88),
        offset: Some(23),
        context: None,
        kind: super::super::mq::ArithmeticErrorKind::InvalidContext,
    };
    let nested_refinement = RefinementError {
        offset: Some(23),
        bitmap_index: 0,
        row: 0,
        x: 0,
        progress: Box::new(RefinementProgress::default()),
        kind: super::super::refinement::RefinementErrorKind::Cancelled,
    };
    let kinds = [
        TextInstanceErrorKind::InvalidSpan("test"),
        TextInstanceErrorKind::Malformed("test"),
        TextInstanceErrorKind::Unsupported {
            feature: "test",
            value: 1,
        },
        TextInstanceErrorKind::LimitExceeded {
            resource: "test",
            limit: 1,
            attempted: 2,
        },
        TextInstanceErrorKind::Cancelled,
        TextInstanceErrorKind::Header(Box::new(nested_header)),
        TextInstanceErrorKind::Mq(Box::new(nested_mq)),
        TextInstanceErrorKind::Refinement(Box::new(nested_refinement)),
    ];
    for kind in kinds {
        let error = preflight_error(3, 23, 2, kind);
        assert!(error.to_string().contains("segment 3 at source byte 23"));
        assert_eq!(error.progress.header_bytes_fetched, 2);
        let nested = matches!(
            error.kind,
            TextInstanceErrorKind::Header(_)
                | TextInstanceErrorKind::Mq(_)
                | TextInstanceErrorKind::Refinement(_)
        );
        assert_eq!(error.source().is_some(), nested);
        struct RejectFormat;
        impl std::fmt::Write for RejectFormat {
            fn write_str(&mut self, _: &str) -> std::fmt::Result {
                Err(std::fmt::Error)
            }
        }
        assert!(std::fmt::write(&mut RejectFormat, format_args!("{error}")).is_err());
    }
    let site = PreflightSite {
        segment: 3,
        offset: 23,
        header_fetched: 2,
    };
    assert!(preflight_cap(site, "test", 2, 2).is_ok());
    let error = preflight_cap(site, "test", 2, 3).unwrap_err();
    assert!(matches!(
        error.kind,
        TextInstanceErrorKind::LimitExceeded { attempted: 3, .. }
    ));
    assert_eq!(
        catalog_metadata_bytes(2, 3),
        2 * mem::size_of::<SymbolDescriptor>() as u128 + 3 * mem::size_of::<StoredSymbol>() as u128
    );
    assert!(catalog_metadata_bytes(u64::MAX, u64::MAX) > u128::from(u64::MAX));
}

#[test]
fn descriptor_preflight_refuses_identity_geometry_and_store_bounds() {
    let site = PreflightSite {
        segment: 3,
        offset: 23,
        header_fetched: 2,
    };
    let valid = StoredSymbol {
        store: SymbolStore::Imported,
        store_base: 5,
        symbol: ONE_PIXEL,
    };
    assert!(validate_descriptor(site, valid, SymbolStore::Imported, 5, 6, 1).is_ok());
    let cases = [
        (
            StoredSymbol {
                store: SymbolStore::New,
                ..valid
            },
            5,
            6,
            1,
            "identity",
        ),
        (
            StoredSymbol {
                store_base: 6,
                ..valid
            },
            5,
            6,
            1,
            "identity",
        ),
        (
            StoredSymbol {
                symbol: SymbolDescriptor {
                    width: 0,
                    ..ONE_PIXEL
                },
                ..valid
            },
            5,
            6,
            1,
            "zero",
        ),
        (
            StoredSymbol {
                symbol: SymbolDescriptor {
                    row_stride: 2,
                    ..ONE_PIXEL
                },
                ..valid
            },
            5,
            6,
            1,
            "noncanonical",
        ),
        (
            StoredSymbol {
                symbol: SymbolDescriptor {
                    relative_store_offset: u64::MAX,
                    ..ONE_PIXEL
                },
                ..valid
            },
            5,
            u64::MAX,
            u64::MAX,
            "overflow",
        ),
        (valid, 5, 6, 0, "LimitExceeded"),
        (
            StoredSymbol {
                store_base: u64::MAX,
                ..valid
            },
            u64::MAX,
            u64::MAX,
            1,
            "overflow",
        ),
        (valid, 5, 5, 1, "outside"),
    ];
    for (stored, base, size, span, reason) in cases {
        let error =
            validate_descriptor(site, stored, SymbolStore::Imported, base, size, span).unwrap_err();
        assert!(format!("{:?}", error.kind).contains(reason), "{error:?}");
        assert_eq!((error.segment, error.offset), (3, 23));
    }
    assert!(matches!(
        checked_coordinate(None, 10),
        Err(TextInstanceErrorKind::Malformed(_))
    ));
    assert!(matches!(
        cap_coordinate(-11, 10),
        Err(TextInstanceErrorKind::LimitExceeded { attempted: 11, .. })
    ));
    assert_eq!(
        cap_coordinate(i32::MIN as i64, i64::MAX).unwrap(),
        i32::MIN as i64
    );
    assert_eq!(
        cap_coordinate(i32::MAX as i64, i64::MAX).unwrap(),
        i32::MAX as i64
    );
    for value in [i32::MIN as i64 - 1, i32::MAX as i64 + 1] {
        assert!(matches!(
            cap_coordinate(value, i64::MAX),
            Err(TextInstanceErrorKind::Malformed(
                "text coordinate outside T.88 signed 32-bit range"
            ))
        ));
    }
}

#[test]
fn parser_and_report_preflight_refuse_forged_metadata_before_mq() {
    const BODY: &[u8] = &[0xeb, 0x80, 0xa7, 0xff, 0xac];
    let make = || Fixture::new(0x10, 1, BODY, &[ONE_PIXEL]);

    let mut f = make();
    f.dictionary.catalog.new_symbols = vec![
        SymbolDescriptor {
            width: 0,
            ..ONE_PIXEL
        };
        32
    ];
    f.dictionary.header.new_symbols = 32;
    f.dictionary.progress.completed_symbols = 32;
    f.budget.max_metadata_bytes = 1;
    preflight_reject(f, "catalog metadata bytes");

    let mut f = make();
    f.parsed.region.width += 1;
    preflight_reject(f, "differs from source");

    let mut f = make();
    f.source.data[17..19].copy_from_slice(&0xa40cu16.to_be_bytes());
    let error = preflight_reject(f, "MalformedFlags");
    assert!(matches!(error.kind, TextInstanceErrorKind::Header(_)));

    let mut f = make();
    f.text_segment.referred_to.clear();
    preflight_reject(f, "reference count");

    let mut f = make();
    f.dictionary_segment.data.offset = u64::MAX;
    preflight_reject(f, "dictionary segment end overflow");

    let mut f = make();
    f.dictionary.header.body.offset = u64::MAX;
    preflight_reject(f, "dictionary body end overflow");

    let mut f = make();
    f.dictionary_segment.data.offset = u64::MAX - 1;
    f.dictionary_segment.data.length = 1;
    f.dictionary.header.header_bytes = 2;
    preflight_reject(f, "dictionary body start overflow");

    let mut f = make();
    f.dictionary.progress.mq = None;
    preflight_reject(f, "complete ordered report");

    let mut f = make();
    f.budget.max_coordinate_magnitude = -1;
    preflight_reject(f, "negative coordinate cap");

    let mut f = make();
    f.budget.max_total_instance_pixels = MAX_BUDGET_COUNT + 1;
    preflight_reject(f, "total instance pixels budget");

    let mut f = make();
    f.budget.max_temporary_store_bytes = MAX_BUDGET_COUNT + 1;
    preflight_reject(f, "temporary store budget");

    let mut f = make();
    f.budget.max_exported_symbols = 0;
    preflight_reject(f, "exported symbols");

    let mut f = make();
    f.budget.max_instances = 0;
    preflight_reject(f, "instances");

    let f = Fixture::new(0x10, 1, BODY, &[]);
    preflight_reject(f, "no symbols");

    let mut f = make();
    f.temporary_base = u64::MAX;
    preflight_reject(f, "temporary store range overflow");

    let mut f = make();
    f.imported_base = 2;
    preflight_reject(f, "store base outside");

    let mut f = make();
    f.fresh_base = 1;
    preflight_reject(f, "store base outside");

    let mut f = make();
    f.code_len = 1;
    preflight_reject(f, "IAID width");

    let mut f = make();
    f.budget.max_metadata_bytes = 0;
    preflight_reject(f, "catalog metadata");

    let mut f = make();
    f.budget.max_working_bytes = 0;
    preflight_reject(f, "working bytes");
}

#[test]
fn hn_c8_unused_template_policy_decodes_same_instances_as_canonical_header() {
    const BODY: &[u8] = &[0xeb, 0x80, 0xa7, 0xff, 0xac];
    let mut canonical = Fixture::new(0x240c, 1, BODY, &[ONE_PIXEL]);
    let expected = canonical.decode_all().unwrap();
    assert_eq!(expected.0.len(), 1);

    let mut anomaly = Fixture::new(0x240c, 1, BODY, &[ONE_PIXEL]);
    anomaly.source.data[17..19].copy_from_slice(&0xa40cu16.to_be_bytes());
    anomaly.parsed = read_text_region_header_with_policy(
        &mut anomaly.source,
        &anomaly.text_segment,
        &anomaly.dictionary_segment,
        &Limits::default(),
        anomaly.header_budget,
        &NeverCancel,
        TextHeaderPolicy::HnC8UnusedRefinementTemplate,
    )
    .unwrap();
    assert_eq!(anomaly.parsed.flags.raw, 0xa40c);
    assert_eq!(
        anomaly.parsed.anomaly,
        Some(TextHeaderAnomaly::UnusedRefinementTemplate)
    );
    let actual = anomaly
        .decode_all_with_policy(TextHeaderPolicy::HnC8UnusedRefinementTemplate)
        .unwrap();
    assert_eq!(actual.0, expected.0);
    assert_eq!(actual.1, expected.1);

    let mut default_strict = Fixture::new(0x240c, 1, BODY, &[ONE_PIXEL]);
    default_strict.source.data[17..19].copy_from_slice(&0xa40cu16.to_be_bytes());
    default_strict.parsed = anomaly.parsed;
    let error = default_strict.attempt().unwrap_err();
    assert!(matches!(error.kind, TextInstanceErrorKind::Header(_)));

    let mut forged = Fixture::new(0x240c, 1, BODY, &[ONE_PIXEL]);
    forged.parsed.anomaly = Some(TextHeaderAnomaly::UnusedRefinementTemplate);
    let error = forged
        .decode_all_with_policy(TextHeaderPolicy::HnC8UnusedRefinementTemplate)
        .unwrap_err();
    assert!(matches!(
        error.kind,
        TextInstanceErrorKind::Malformed("supplied text header differs from source")
    ));
}

#[test]
fn valid_huffman_and_template_zero_headers_are_typed_refusals() {
    const BODY: &[u8] = &[0xeb, 0x80, 0xa7, 0xff, 0xac];
    for (flags, optional, expected) in [
        (0x0011u16, vec![0, 0], "Huffman text region"),
        (0x0012u16, vec![0, 0, 0, 0], "refinement template 0"),
    ] {
        let mut fixture = Fixture::new(0x10, 1, BODY, &[ONE_PIXEL]);
        let mut data = text_data(flags, 1, BODY);
        data.splice(19..19, optional);
        fixture.source.data = data;
        fixture.text_segment.data.length = fixture.source.size();
        fixture.parsed = read_text_region_header(
            &mut fixture.source,
            &fixture.text_segment,
            &fixture.dictionary_segment,
            &Limits::default(),
            TextRegionBudget::default(),
            &NeverCancel,
        )
        .unwrap();
        let error = preflight_reject(fixture, expected);
        assert!(matches!(
            error.kind,
            TextInstanceErrorKind::Unsupported { .. }
        ));
    }
}

#[test]
fn catalog_preflight_checks_all_new_symbols_and_export_order() {
    const BODY: &[u8] = &[0x7f, 0xff, 0xac];
    let make = || Fixture::new(0x10, 1, BODY, &[ONE_PIXEL]);

    let mut f = make();
    f.dictionary.catalog.new_symbols = vec![ONE_PIXEL];
    f.dictionary.header.new_symbols = 1;
    f.dictionary.progress.completed_symbols = 1;
    f.fresh.data = vec![0x40];
    assert!(
        f.attempt().is_ok(),
        "unexported new symbol has a valid explicit base"
    );

    let mut f = make();
    f.dictionary.catalog.new_symbols = vec![ONE_PIXEL, ONE_PIXEL];
    f.dictionary.header.new_symbols = 2;
    f.dictionary.progress.completed_symbols = 2;
    f.fresh.data = vec![0x40];
    preflight_reject(f, "overlapping new symbols");

    let mut f = make();
    f.dictionary.catalog.new_symbols = vec![SymbolDescriptor {
        row_stride: 2,
        ..ONE_PIXEL
    }];
    f.dictionary.header.new_symbols = 1;
    f.dictionary.progress.completed_symbols = 1;
    f.fresh.data = vec![0x40];
    preflight_reject(f, "noncanonical dictionary symbol descriptor");

    let mut f = make();
    f.dictionary.catalog.exported_symbols[0].symbol.row_stride = 2;
    preflight_reject(f, "noncanonical dictionary symbol descriptor");

    let second = SymbolDescriptor {
        relative_store_offset: 1,
        ..ONE_PIXEL
    };
    let mut f = Fixture::new(0x10, 1, BODY, &[ONE_PIXEL, second]);
    f.dictionary.catalog.exported_symbols.swap(0, 1);
    preflight_reject(f, "overlapping exported symbols");

    let mut f = Fixture::new(0x10, 1, BODY, &[ONE_PIXEL, second]);
    f.dictionary.catalog.new_symbols = vec![ONE_PIXEL];
    f.dictionary.header.new_symbols = 1;
    f.dictionary.progress.completed_symbols = 1;
    f.dictionary.catalog.exported_symbols[0].store = SymbolStore::New;
    f.fresh.data = vec![0x40];
    preflight_reject(f, "imported export follows new");

    let mut f = make();
    f.dictionary.catalog.new_symbols = vec![ONE_PIXEL];
    f.dictionary.header.new_symbols = 1;
    f.dictionary.progress.completed_symbols = 1;
    f.dictionary.catalog.exported_symbols[0].store = SymbolStore::New;
    f.fresh.data = vec![0x40];
    assert!(f.attempt().is_ok(), "new store can be exported");

    let mut f = make();
    f.dictionary.catalog.new_symbols = vec![ONE_PIXEL];
    f.dictionary.header.new_symbols = 1;
    f.dictionary.progress.completed_symbols = 1;
    f.dictionary.catalog.exported_symbols[0] = StoredSymbol {
        store: SymbolStore::New,
        store_base: 0,
        symbol: second,
    };
    f.fresh.data = vec![0x40, 0x40];
    preflight_reject(f, "absent from new catalog");

    let mut f = make();
    f.dictionary.catalog.new_symbols = vec![ONE_PIXEL];
    f.dictionary.header.new_symbols = 1;
    f.dictionary.progress.completed_symbols = 1;
    f.dictionary.catalog.exported_symbols[0] = StoredSymbol {
        store: SymbolStore::New,
        store_base: 0,
        symbol: SymbolDescriptor {
            width: 2,
            ..ONE_PIXEL
        },
    };
    f.fresh.data = vec![0x40];
    preflight_reject(f, "order differs from catalog");
}

#[test]
fn zero_instances_and_zero_symbols_still_validate_initial_iadt_and_terminal() {
    const BODY: &[u8] = &[0xeb, 0x7f, 0xff, 0xac];
    let mut fixture = Fixture::new(0x10, 0, BODY, &[]);
    let (instances, progress) = fixture.decode_all().unwrap();
    assert!(instances.is_empty());
    assert_eq!((progress.completed_instances, progress.strips), (0, 0));
    assert_eq!(progress.decision, TextDecision::Complete);
    assert_eq!(progress.header_bytes_fetched, 23);
    assert!(fixture.temporary.is_empty());
}

#[test]
fn truncated_payload_and_bad_marker_errors_are_located() {
    const BODY: &[u8] = &[0xeb, 0x7f, 0x7f, 0xff, 0xac];
    let mut truncated = Fixture::new(0x10, 1, BODY, &[ONE_PIXEL]);
    truncated.source.data.truncate(23 + 1);
    let error = truncated.attempt().unwrap_err();
    assert!(
        matches!(&error.kind, TextInstanceErrorKind::Header(header)
            if matches!(header.kind, super::super::text::TextRegionErrorKind::InvalidSpan(_))),
        "{error:?}"
    );
    assert_eq!(error.progress.completed_instances, 0);
    let mut bad_marker = Fixture::new(0x10, 1, &[0, 0, 0, 0, 0, 0xff, 0x90], &[ONE_PIXEL]);
    let error = bad_marker.decode_all().unwrap_err();
    assert!(matches!(error.kind, TextInstanceErrorKind::Mq(_)));
}

#[test]
fn runtime_budgets_refuse_before_emitting_an_instance() {
    const BODY: &[u8] = &[0xeb, 0x7f, 0x7f, 0xff, 0xac];
    let mut f = Fixture::new(0x10, 1, BODY, &[ONE_PIXEL]);
    f.budget.max_pixels_per_instance = MAX_BUDGET_COUNT + 1;
    preflight_reject(f, "instance pixels budget");

    let mut f = Fixture::new(0x10, 1, BODY, &[ONE_PIXEL]);
    f.budget.max_strips = 0;
    let error = f.decode_all().unwrap_err();
    assert!(matches!(
        error.kind,
        TextInstanceErrorKind::LimitExceeded {
            resource: "text strips",
            attempted: 1,
            ..
        }
    ));
    assert_eq!(error.progress.completed_instances, 0);

    let mut f = Fixture::new(0x10, 1, BODY, &[ONE_PIXEL]);
    f.budget.max_pixels_per_instance = 0;
    let error = f.decode_all().unwrap_err();
    assert!(matches!(
        error.kind,
        TextInstanceErrorKind::LimitExceeded {
            resource: "instance pixels",
            attempted: 1,
            ..
        }
    ));
    assert_eq!(error.progress.completed_instances, 0);

    let mut f = Fixture::new(0x10, 1, BODY, &[ONE_PIXEL]);
    f.budget.max_total_instance_pixels = 0;
    let error = f.decode_all().unwrap_err();
    assert!(matches!(
        error.kind,
        TextInstanceErrorKind::LimitExceeded {
            resource: "total instance pixels",
            attempted: 1,
            ..
        }
    ));
    assert_eq!(error.progress.completed_instances, 0);

    let mut f = Fixture::new(0x10, 1, BODY, &[ONE_PIXEL]);
    f.mq_budget.max_symbols = 1;
    let error = f.decode_all().unwrap_err();
    assert!(matches!(error.kind, TextInstanceErrorKind::Mq(_)));
    assert_eq!(error.progress.decision, TextDecision::InitialStripT);
}

#[test]
fn refinement_errors_end_the_text_session() {
    const BODY: [u8; 11] = [138, 19, 228, 1, 154, 208, 119, 232, 127, 255, 172];
    let mut f = Fixture::new(0x8012, 1, &BODY, &[ONE_PIXEL]);
    f.qe = 0x4000;
    f.refinement_budget.max_width = 0;
    let error = f.decode_all().unwrap_err();
    assert!(matches!(error.kind, TextInstanceErrorKind::Refinement(_)));
    assert_eq!(error.progress.completed_instances, 0);
    assert!(f.temporary.is_empty());
}

#[test]
fn refined_new_store_handle_remains_distinct_from_dictionary_symbols() {
    const BODY: [u8; 11] = [138, 19, 228, 1, 154, 208, 119, 233, 63, 255, 172];
    let mut f = Fixture::new(0x8012, 1, &BODY, &[ONE_PIXEL]);
    f.qe = 0x4000;
    f.dictionary.catalog.new_symbols.push(ONE_PIXEL);
    f.dictionary.catalog.exported_symbols[0].store = SymbolStore::New;
    f.dictionary.header.new_symbols = 1;
    f.dictionary.progress.completed_symbols = 1;
    f.fresh.data = vec![0x80];
    f.imported.data.clear();
    f.temporary = vec![0; 77];
    f.temporary_base = 77;
    let (events, progress) = f.decode_all().unwrap();
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0].bitmap,
        TextBitmap::Refined { store_base: 77, .. }
    ));
    assert_eq!(progress.refinement.completed_bitmaps, 1);
    assert_eq!(progress.ri_one, 1);
}

#[test]
fn fixed_budget_malformed_body_mutations_end_with_located_results() {
    let mut seed = 0x7f4a7c15u32;
    let mut unexpected_oob = 0;
    let mut outside_strip = 0;
    let mut invalid_ri = 0;
    for candidate in 0..5000u32 {
        let mut body = [0u8; 14];
        for byte in &mut body[..12] {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *byte = seed as u8;
        }
        body[12..].copy_from_slice(&[0xff, 0xac]);
        let flags = [0x10, 0x1c, 0x8012, 0x801e][candidate as usize % 4];
        let mut f = Fixture::new(flags, 2, &body, &[ONE_PIXEL]);
        f.qe = 0x4000;
        f.budget.max_strips = 4;
        f.budget.max_pixels_per_instance = 64;
        f.budget.max_total_instance_pixels = 128;
        f.refinement_budget.max_width = 64;
        f.refinement_budget.max_height = 64;
        match f.decode_all() {
            Ok((events, progress)) => {
                assert_eq!(events.len(), 2);
                assert_eq!(progress.completed_instances, 2);
            }
            Err(error) => {
                assert_eq!(error.segment, 3);
                assert!(error.offset >= 23);
                assert!(error.progress.completed_instances <= 2);
                match error.kind {
                    TextInstanceErrorKind::Malformed("unexpected arithmetic OOB") => {
                        unexpected_oob += 1
                    }
                    TextInstanceErrorKind::Malformed("IAIT outside strip") => outside_strip += 1,
                    TextInstanceErrorKind::Malformed("IARI is not a bit") => invalid_ri += 1,
                    _ => {}
                }
            }
        }
    }
    assert!(unexpected_oob > 0);
    assert!(outside_strip > 0);
    assert!(invalid_ri > 0);
}

#[test]
fn cancellation_is_reported_with_progress() {
    const BODY: &[u8] = &[0x7f, 0xff, 0xac];
    let mut f = Fixture::new(0x10, 1, BODY, &[ONE_PIXEL]);
    let limits = Limits::default();
    let table = MqTable::standard();
    let mut contexts = coding_unit(0, &limits, &f.mq_budget);
    let cancellation = ToggleCancel(Cell::new(false));
    let mut decoder = TextInstanceDecoder::new(
        Payload::from(&f.source.data[..]),
        &f.text_segment,
        f.parsed,
        &f.dictionary_segment,
        &f.dictionary,
        &f.imported.data,
        0,
        &f.fresh.data,
        0,
        &mut f.temporary,
        0,
        &table,
        &mut contexts,
        &limits,
        &cancellation,
        f.mq_budget,
        f.header_budget,
        f.refinement_budget,
        f.budget,
    )
    .unwrap();
    cancellation.0.set(true);
    let cancelled = decoder.next_instance().unwrap_err();
    assert!(matches!(cancelled.kind, TextInstanceErrorKind::Cancelled));
    assert_eq!(cancelled.progress.completed_instances, 0);
}

#[test]
fn every_corner_and_transpose_uses_correct_pre_and_post_curs() {
    let cases = [
        (ReferenceCorner::TopLeft, false, (13, 20, 15)),
        (ReferenceCorner::TopRight, false, (13, 20, 15)),
        (ReferenceCorner::BottomLeft, false, (13, 16, 15)),
        (ReferenceCorner::BottomRight, false, (13, 16, 15)),
        (ReferenceCorner::TopLeft, true, (20, 13, 17)),
        (ReferenceCorner::TopRight, true, (18, 13, 17)),
        (ReferenceCorner::BottomLeft, true, (20, 13, 17)),
        (ReferenceCorner::BottomRight, true, (18, 13, 17)),
    ];
    for (corner, transposed, expected) in cases {
        assert_eq!(
            geometry(13, 20, 3, 5, corner, transposed, 100).unwrap(),
            expected,
            "{corner:?}, transposed {transposed}"
        );
    }
    assert_eq!(
        geometry(-4, -3, 2, 2, ReferenceCorner::BottomRight, false, 10).unwrap(),
        (-4, -4, -3)
    );
}

#[test]
fn public_real_mq_placement_covers_every_reference_corner_and_transpose() {
    const BODY: &[u8] = &[0xeb, 0x7f, 0x7f, 0xff, 0xac];
    let symbol = SymbolDescriptor {
        width: 3,
        height: 5,
        row_stride: 1,
        relative_store_offset: 0,
        stored_bytes: 5,
    };
    for (flags, expected) in [
        (0x00u16, (0, 0)),
        (0x10, (0, 4)),
        (0x20, (0, 0)),
        (0x30, (0, 4)),
        (0x40, (4, 0)),
        (0x50, (4, 0)),
        (0x60, (2, 0)),
        (0x70, (2, 0)),
    ] {
        let mut f = Fixture::new(flags, 1, BODY, &[symbol]);
        let (events, progress) = f.decode_all().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!((events[0].x, events[0].y), expected, "flags {flags:#06x}");
        assert_eq!((events[0].width, events[0].height), (3, 5));
        assert_eq!(progress.ri_zero, 1);
    }
}

#[test]
fn public_real_mq_accepts_all_four_standard_strip_sizes() {
    const BODY: &[u8] = &[0xeb, 0x80, 0xa7, 0xff, 0xac];
    for flags in [0x10u16, 0x14, 0x18, 0x1c] {
        let mut f = Fixture::new(flags, 1, BODY, &[ONE_PIXEL]);
        let (events, progress) = f.decode_all().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!((events[0].x, events[0].y, events[0].strip), (0, 4, 0));
        assert_eq!(progress.strips, 1);
    }
}

#[test]
fn real_mq_fixed_width_iaid_accepts_last_symbol_and_rejects_unused_codeword() {
    let symbols = [
        ONE_PIXEL,
        SymbolDescriptor {
            relative_store_offset: 1,
            ..ONE_PIXEL
        },
        SymbolDescriptor {
            relative_store_offset: 2,
            ..ONE_PIXEL
        },
    ];
    const VALID: [u8; 4] = [105, 183, 255, 172];
    const INVALID: [u8; 6] = [248, 172, 33, 127, 255, 172];
    let mut fixture = Fixture::new(0x10, 1, &VALID, &symbols);
    fixture.qe = 0x4000;
    let (events, progress) = fixture.decode_all().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!((events[0].symbol_id, events[0].x, events[0].y), (2, -1, 0));
    assert_eq!(
        events[0].bitmap,
        TextBitmap::Stored(fixture.dictionary.catalog.exported_symbols[2])
    );
    assert_eq!(progress.completed_instances, 1);

    let mut fixture = Fixture::new(0x10, 1, &INVALID, &symbols);
    fixture.qe = 0x4000;
    let error = fixture.decode_all().unwrap_err();
    assert!(matches!(
        error.kind,
        TextInstanceErrorKind::Malformed("IAID outside exported catalog")
    ));
    assert_eq!(error.progress.decision, TextDecision::SymbolId);
    assert_eq!(error.progress.completed_instances, 0);
}

#[test]
fn table12_offsets_floor_negative_half_deltas() {
    let reference = SymbolDescriptor {
        width: 4,
        height: 6,
        row_stride: 1,
        relative_store_offset: 0,
        stored_bytes: 6,
    };
    assert_eq!(
        refined_geometry(reference, -1, -3, 0, 1).unwrap(),
        (3, 3, -1, -1)
    );
    assert_eq!(
        refined_geometry(reference, 1, 3, -1, -2).unwrap(),
        (5, 9, -1, -1)
    );
    assert!(matches!(
        refined_geometry(reference, -4, 0, 0, 0),
        Err(TextInstanceErrorKind::Malformed(_))
    ));
    assert!(matches!(
        refined_geometry(reference, 0, 0, i64::MAX, 0),
        Err(TextInstanceErrorKind::Malformed(_))
    ));
    assert!(matches!(
        refined_geometry(reference, 0, 0, 0, i64::MIN),
        Err(TextInstanceErrorKind::Malformed(_))
    ));
}

#[test]
fn real_mq_single_unmodified_instance_and_exact_terminal() {
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let table = MqTable::standard();
    let mut contexts = coding_unit(0, &limits, &mq_budget);
    let mut source = Bytes::new(text_data(0x10, 1, &[0xeb, 0x7f, 0x7f, 0xff, 0xac]));
    let text_segment = segment(3, 6, vec![2], 0, source.size());
    let dictionary_segment = segment(2, 0, vec![1], 100, 2);
    let dictionary = report(&[ONE_PIXEL]);
    let imported = Bytes::new(vec![0x80]);
    let fresh = Bytes::new(vec![]);
    let mut temporary = Vec::new();
    let parsed = read_text_region_header(
        &mut source,
        &text_segment,
        &dictionary_segment,
        &limits,
        TextRegionBudget::default(),
        &NeverCancel,
    )
    .unwrap();
    let mut decoder = TextInstanceDecoder::new(
        Payload::from(&source.data[..]),
        &text_segment,
        parsed,
        &dictionary_segment,
        &dictionary,
        &imported.data,
        0,
        &fresh.data,
        0,
        &mut temporary,
        0,
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        mq_budget,
        TextRegionBudget::default(),
        RefinementBudget::default(),
        TextInstanceBudget::default(),
    )
    .unwrap();
    let first = decoder.next_instance().unwrap().unwrap();
    assert_eq!(
        (first.index, first.strip, first.symbol_id, first.x, first.y),
        (0, 0, 0, 0, 4)
    );
    assert_eq!(
        first.bitmap,
        TextBitmap::Stored(dictionary.catalog.exported_symbols[0])
    );
    assert!(!first.ri);
    assert!(decoder.next_instance().unwrap().is_none());
    assert!(decoder.next_instance().unwrap().is_none());
    let progress = decoder.progress();
    assert_eq!(
        (
            progress.completed_instances,
            progress.ri_zero,
            progress.ri_one,
            progress.strips
        ),
        (1, 1, 0, 1)
    );
}

#[test]
fn real_mq_refinement_reads_reference_and_writes_packed_rows() {
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let table = MqTable::standard();
    let mut contexts = coding_unit(0, &limits, &mq_budget);
    let dictionary_segment = segment(2, 0, vec![1], 100, 2);
    let dictionary = report(&[ONE_PIXEL]);
    const BODY: [u8; 11] = [138, 19, 228, 1, 154, 208, 119, 233, 63, 255, 172];
    let mut source = Bytes::new(text_data(0x8012, 1, &BODY));
    let text_segment = segment(3, 6, vec![2], 0, source.size());
    let imported = Bytes::new(vec![0x80]);
    let fresh = Bytes::new(vec![]);
    let mut temporary = vec![0; 73];
    let parsed = read_text_region_header(
        &mut source,
        &text_segment,
        &dictionary_segment,
        &limits,
        TextRegionBudget::default(),
        &NeverCancel,
    )
    .unwrap();
    let mut decoder = TextInstanceDecoder::new(
        Payload::from(&source.data[..]),
        &text_segment,
        parsed,
        &dictionary_segment,
        &dictionary,
        &imported.data,
        0,
        &fresh.data,
        0,
        &mut temporary,
        73,
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        mq_budget,
        TextRegionBudget::default(),
        RefinementBudget::default(),
        TextInstanceBudget::default(),
    )
    .unwrap();
    let instance = decoder.next_instance().unwrap().unwrap();
    assert_eq!(
        (
            instance.index,
            instance.strip,
            instance.symbol_id,
            instance.x,
            instance.y
        ),
        (0, 0, 0, -3, 3)
    );
    assert_eq!((instance.width, instance.height, instance.ri), (4, 4, true));
    assert_eq!(
        instance.bitmap,
        TextBitmap::Refined {
            store_base: 73,
            symbol: SymbolDescriptor {
                width: 4,
                height: 4,
                row_stride: 1,
                relative_store_offset: 0,
                stored_bytes: 4
            },
        }
    );
    assert!(decoder.next_instance().unwrap().is_none());
    let progress = decoder.progress();
    assert_eq!(
        (
            progress.ri_zero,
            progress.ri_one,
            progress.refinement.completed_bitmaps
        ),
        (0, 1, 1)
    );
    assert_eq!(temporary[..73], [0; 73]);
    assert_eq!(temporary[73..], [0xe0, 0x70, 0x50, 0x10]);
}

#[test]
fn a_text_region_resets_dirty_integer_iaid_and_gr_statistics() {
    const BODY: [u8; 11] = [138, 19, 228, 1, 154, 208, 119, 233, 63, 255, 172];
    let mut f = Fixture::new(0x8012, 1, &BODY, &[ONE_PIXEL]);
    let limits = Limits::default();
    let table = MqTable::standard();
    let mut contexts = coding_unit(0, &limits, &f.mq_budget);
    let indices = [0, BITMAP_BASE, IAID_BASE];
    for index in indices {
        contexts.update(
            index,
            ContextState {
                state_index: 1,
                mps: true,
            },
        );
    }
    let mut decoder = TextInstanceDecoder::new(
        Payload::from(&f.source.data[..]),
        &f.text_segment,
        f.parsed,
        &f.dictionary_segment,
        &f.dictionary,
        &f.imported.data,
        0,
        &f.fresh.data,
        0,
        &mut f.temporary,
        0,
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        f.mq_budget,
        f.header_budget,
        f.refinement_budget,
        f.budget,
    )
    .unwrap();
    for index in indices {
        assert_eq!(decoder.mq.context(index), Some(ContextState::default()));
    }
    assert!(decoder.next_instance().unwrap().unwrap().ri);
    assert!(decoder.next_instance().unwrap().is_none());
}

#[test]
fn public_mq_negative_half_deltas_and_contexts_continue_across_refined_instances() {
    let second = SymbolDescriptor {
        relative_store_offset: 2,
        ..DIAGONAL_2X2
    };
    let mut f = Fixture::new(
        0x8012,
        2,
        &TWO_EXPORTED_REFINED_BODY,
        &[DIAGONAL_2X2, second],
    );
    f.imported.data = vec![0x80, 0x40, 0x40, 0x80];
    let limits = Limits::default();
    let table = MqTable::standard();
    let mut contexts = coding_unit(1, &limits, &f.mq_budget);
    let mut decoder = TextInstanceDecoder::new(
        Payload::from(&f.source.data[..]),
        &f.text_segment,
        f.parsed,
        &f.dictionary_segment,
        &f.dictionary,
        &f.imported.data,
        0,
        &f.fresh.data,
        0,
        &mut f.temporary,
        0,
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        f.mq_budget,
        f.header_budget,
        f.refinement_budget,
        f.budget,
    )
    .unwrap();
    let first = decoder.next_instance().unwrap().unwrap();
    assert!(first.ri);
    assert_eq!(first.symbol_id, 1);
    assert_eq!(
        (first.x, first.y, first.width, first.height),
        (2, -53, 1, 1)
    );
    assert_eq!(
        first.bitmap,
        TextBitmap::Refined {
            store_base: 0,
            symbol: SymbolDescriptor {
                width: 1,
                height: 1,
                row_stride: 1,
                relative_store_offset: 0,
                stored_bytes: 1,
            },
        }
    );
    // Independently traced IARDW/H=-1, IARDX/Y=0. Table 12 requires
    // floor(-1/2)=-1 on both axes. GR context 48 is reached with that
    // offset; truncation toward zero would use GR context 6 instead.
    assert_eq!(decoder.mq.context(BITMAP_BASE + 48).unwrap().state_index, 1);
    assert_eq!(decoder.mq.context(BITMAP_BASE + 6).unwrap().state_index, 0);
    let integer_after_first: Vec<_> = (0..INTEGER_CONTEXT_COUNT)
        .map(|index| decoder.mq.context(index).unwrap())
        .collect();
    let iaid_after_first = decoder.mq.context(IAID_BASE + 1).unwrap();
    let gr_after_first: Vec<_> = (BITMAP_BASE..BITMAP_BASE + BITMAP_CONTEXT_COUNT)
        .map(|index| decoder.mq.context(index).unwrap())
        .collect();
    assert!(
        integer_after_first
            .iter()
            .any(|state| state.state_index > 0)
    );
    assert!(iaid_after_first.state_index > 0);
    assert!(gr_after_first.iter().any(|state| state.state_index > 0));

    let second = decoder.next_instance().unwrap().unwrap();
    assert!(second.ri);
    assert_eq!(second.symbol_id, 1);
    assert_eq!(
        (second.x, second.y, second.width, second.height),
        (-1, -53, 3, 7)
    );
    assert_eq!(
        second.bitmap,
        TextBitmap::Refined {
            store_base: 0,
            symbol: SymbolDescriptor {
                width: 3,
                height: 7,
                row_stride: 1,
                relative_store_offset: 1,
                stored_bytes: 7,
            },
        }
    );
    assert!((0..INTEGER_CONTEXT_COUNT).any(|index| {
        decoder.mq.context(index).unwrap().state_index > integer_after_first[index].state_index
    }));
    assert!(decoder.mq.context(IAID_BASE + 1).unwrap().state_index > iaid_after_first.state_index);
    assert!((0..BITMAP_CONTEXT_COUNT).any(|index| {
        decoder.mq.context(BITMAP_BASE + index).unwrap().state_index
            > gr_after_first[index].state_index
    }));
    assert!(decoder.next_instance().unwrap().is_none());
    assert_eq!(decoder.progress().ri_one, 2);
    assert_eq!(f.temporary, [0, 160, 224, 224, 192, 192, 96, 96]);
}

#[test]
fn refined_bitmap_is_in_the_store_before_the_next_pull() {
    const BODY: [u8; 11] = [138, 19, 228, 1, 154, 208, 119, 233, 63, 255, 172];
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let table = MqTable::standard();
    let mut contexts = coding_unit(0, &limits, &mq_budget);
    let mut source = Bytes::new(text_data(0x8012, 1, &BODY));
    let text_segment = segment(3, 6, vec![2], 0, source.size());
    let dictionary_segment = segment(2, 0, vec![1], 100, 2);
    let dictionary = report(&[ONE_PIXEL]);
    let imported = Bytes::new(vec![0x80]);
    let fresh = Bytes::new(vec![]);
    let mut temporary = Vec::new();
    let parsed = read_text_region_header(
        &mut source,
        &text_segment,
        &dictionary_segment,
        &limits,
        TextRegionBudget::default(),
        &NeverCancel,
    )
    .unwrap();
    let mut decoder = TextInstanceDecoder::new(
        Payload::from(&source.data[..]),
        &text_segment,
        parsed,
        &dictionary_segment,
        &dictionary,
        &imported.data,
        0,
        &fresh.data,
        0,
        &mut temporary,
        0,
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        mq_budget,
        TextRegionBudget::default(),
        RefinementBudget::default(),
        TextInstanceBudget::default(),
    )
    .unwrap();
    let instance = decoder.next_instance().unwrap().unwrap();
    assert!(matches!(instance.bitmap, TextBitmap::Refined { .. }));
    assert_eq!(decoder.refined_store(), [0xe0, 0x70, 0x50, 0x10]);
    assert!(decoder.next_instance().unwrap().is_none());
}

#[test]
fn real_mq_multistrip_oob_and_subsequent_s_with_ds_offset() {
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let table = MqTable::standard();
    let mut contexts = coding_unit(0, &limits, &mq_budget);
    let dictionary_segment = segment(2, 0, vec![1], 100, 2);
    let dictionary = report(&[ONE_PIXEL]);
    const DIFFERENT: &[u8] = &[
        247, 217, 127, 188, 37, 23, 97, 223, 81, 200, 3, 63, 255, 172,
    ];
    const SAME: &[u8] = &[136, 126, 124, 158, 65, 35, 255, 172];
    for (flags, body, expected) in [
        (
            0x001c,
            DIFFERENT,
            [(-506_465_247, 2216, 0), (-506_465_245, 2257, 1)],
        ),
        (0x781c, SAME, [(-62, 425, 0), (182, 426, 0)]),
    ] {
        let mut source = Bytes::new(text_data(flags, 2, body));
        let text_segment = segment(3, 6, vec![2], 0, source.size());
        let imported = Bytes::new(vec![0x80]);
        let fresh = Bytes::new(vec![]);
        let mut temporary = Vec::new();
        let parsed = read_text_region_header(
            &mut source,
            &text_segment,
            &dictionary_segment,
            &limits,
            TextRegionBudget::default(),
            &NeverCancel,
        )
        .unwrap();
        let mut decoder = TextInstanceDecoder::new(
            Payload::from(&source.data[..]),
            &text_segment,
            parsed,
            &dictionary_segment,
            &dictionary,
            &imported.data,
            0,
            &fresh.data,
            0,
            &mut temporary,
            0,
            &table,
            &mut contexts,
            &limits,
            &NeverCancel,
            mq_budget,
            TextRegionBudget::default(),
            RefinementBudget::default(),
            TextInstanceBudget {
                max_strips: 4,
                ..TextInstanceBudget::default()
            },
        )
        .unwrap();
        for (index, expected) in expected.into_iter().enumerate() {
            let instance = decoder.next_instance().unwrap().unwrap();
            assert_eq!((instance.x, instance.y, instance.strip), expected);
            assert_eq!(instance.index, index as u32);
            assert_eq!(
                instance.bitmap,
                TextBitmap::Stored(dictionary.catalog.exported_symbols[0])
            );
        }
        assert!(decoder.next_instance().unwrap().is_none());
        let progress = decoder.progress();
        assert_eq!(progress.completed_instances, 2);
        assert_eq!(progress.ri_zero, 2);
    }
}
