// SPDX-License-Identifier: MIT

use super::super::{
    SegmentSpan, dictionary::DictionaryDataHeader,
    refinement_dictionary::RefinementDictionaryCatalog,
};
use super::*;
use crate::NeverCancel;
use std::{
    cell::{Cell, RefCell},
    error::Error as _,
    future::Future,
    pin::pin,
    rc::Rc,
    task::{Context, Poll, Waker},
};

struct ToggleCancel(Cell<bool>);

impl Cancellation for ToggleCancel {
    fn is_cancelled(&self) -> bool {
        self.0.get()
    }
}

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

#[derive(Default)]
struct Bytes {
    data: Vec<u8>,
    max_read: usize,
    fault: Option<(u64, ReadFault)>,
    body_reads: u32,
    body_start: u64,
}

#[derive(Clone, Copy)]
enum ReadFault {
    Zero,
    Overreport,
}

impl Bytes {
    fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            max_read: usize::MAX,
            fault: None,
            body_reads: 0,
            body_start: 23,
        }
    }
}

impl RangedSource for Bytes {
    fn size(&self) -> u64 {
        self.data.len() as u64
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> crate::Result<usize> {
        if offset >= self.body_start {
            self.body_reads += 1;
        }
        if let Some((at, fault)) = self.fault {
            if offset >= at {
                return match fault {
                    ReadFault::Zero => Ok(0),
                    ReadFault::Overreport => Ok(destination.len() + 1),
                };
            }
        }
        let offset = usize::try_from(offset).unwrap_or(usize::MAX);
        let count = self
            .data
            .len()
            .saturating_sub(offset)
            .min(destination.len())
            .min(self.max_read);
        if count != 0 {
            destination[..count].copy_from_slice(&self.data[offset..offset + count]);
        }
        Ok(count)
    }
}

#[derive(Default)]
struct Sink(Vec<u8>);

impl SequentialSink for Sink {
    async fn write(&mut self, bytes: &[u8]) -> crate::Result<usize> {
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    async fn flush(&mut self) -> crate::Result<()> {
        Ok(())
    }
}

struct BufferingSink {
    pending: Vec<u8>,
    visible: Rc<RefCell<Vec<u8>>>,
}

struct PendingFlushSink {
    bytes: Vec<u8>,
    pending: Rc<Cell<bool>>,
}

impl SequentialSink for PendingFlushSink {
    async fn write(&mut self, bytes: &[u8]) -> crate::Result<usize> {
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    async fn flush(&mut self) -> crate::Result<()> {
        if self.pending.get() {
            std::future::pending::<()>().await;
        }
        Ok(())
    }
}

impl SequentialSink for BufferingSink {
    async fn write(&mut self, bytes: &[u8]) -> crate::Result<usize> {
        self.pending.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    async fn flush(&mut self) -> crate::Result<()> {
        self.visible.borrow_mut().extend(self.pending.drain(..));
        Ok(())
    }
}

fn table_with_qe(limits: &Limits, qe: u16) -> MqTable {
    MqTable::new(
        vec![
            MqState {
                qe,
                next_mps: 0,
                next_lps: 0,
                switch_mps: false
            };
            MQ_STATE_COUNT
        ],
        limits,
    )
    .unwrap()
}

fn table(limits: &Limits) -> MqTable {
    table_with_qe(limits, 1)
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

fn report(symbols: &[SymbolDescriptor]) -> RefinementDictionaryReport {
    let exported_symbols = symbols
        .iter()
        .copied()
        .map(|symbol| StoredSymbol {
            store: SymbolStore::Imported,
            store_base: 0,
            symbol,
        })
        .collect();
    RefinementDictionaryReport {
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
        catalog: RefinementDictionaryCatalog {
            new_symbols: vec![],
            exported_symbols,
        },
        progress: super::super::refinement_dictionary::RefinementDictionaryProgress {
            mq: Some(MqSnapshot {
                interval: 0,
                code: 0,
                bit_counter: 0,
                current_input_offset: 100,
                source_bytes_fetched: 0,
                terminal_inputs: 0,
                symbols_decoded: 0,
                work_done: 0,
                poisoned: false,
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

const TWO_EXPORTED_REFINED_BODY: [u8; 14] = [
    153, 141, 235, 153, 191, 232, 238, 189, 166, 16, 4, 200, 255, 172,
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
    dictionary: RefinementDictionaryReport,
    imported: Bytes,
    fresh: Bytes,
    temporary: Sink,
    parsed: TextRegionHeader,
    imported_base: u64,
    fresh_base: u64,
    temporary_base: u64,
    banks_code_len: u32,
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
        let parsed = ready(read_text_region_header(
            &mut source,
            &text_segment,
            &dictionary_segment,
            &Limits::default(),
            TextRegionBudget::default(),
            &NeverCancel,
        ))
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
            temporary: Sink::default(),
            parsed,
            imported_base: 0,
            fresh_base: 0,
            temporary_base: 0,
            banks_code_len: if count <= 1 {
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
        let table = table_with_qe(&limits, self.qe);
        let mut banks = IaidContextBanks::with_bitmap_contexts(
            self.banks_code_len,
            GR_CONTEXTS,
            &limits,
            &self.mq_budget,
        )
        .unwrap();
        let decoder = ready(TextInstanceDecoder::new(
            &mut self.source,
            &self.text_segment,
            self.parsed,
            &self.dictionary_segment,
            &self.dictionary,
            &mut self.imported,
            self.imported_base,
            &mut self.fresh,
            self.fresh_base,
            &mut self.temporary,
            self.temporary_base,
            &table,
            &mut banks,
            &limits,
            &NeverCancel,
            self.mq_budget,
            self.header_budget,
            self.refinement_budget,
            self.budget,
        ))?;
        Ok(decoder.progress())
    }

    fn decode_all(&mut self) -> TextInstanceResult<(Vec<TextInstance>, TextInstanceProgress)> {
        let limits = Limits::default();
        let table = table_with_qe(&limits, self.qe);
        let mut banks = IaidContextBanks::with_bitmap_contexts(
            self.banks_code_len,
            GR_CONTEXTS,
            &limits,
            &self.mq_budget,
        )
        .unwrap();
        let mut decoder = ready(TextInstanceDecoder::new(
            &mut self.source,
            &self.text_segment,
            self.parsed,
            &self.dictionary_segment,
            &self.dictionary,
            &mut self.imported,
            self.imported_base,
            &mut self.fresh,
            self.fresh_base,
            &mut self.temporary,
            self.temporary_base,
            &table,
            &mut banks,
            &limits,
            &NeverCancel,
            self.mq_budget,
            self.header_budget,
            self.refinement_budget,
            self.budget,
        ))?;
        let mut instances = Vec::new();
        while let Some(instance) = ready(decoder.next())? {
            instances.push(instance);
            assert!(
                instances.len() <= 8,
                "test fixture unexpectedly emitted many instances"
            );
        }
        Ok((instances, decoder.progress()))
    }
}

fn preflight_reject(mut fixture: Fixture, reason: &str) -> TextInstanceError {
    let error = fixture.attempt().unwrap_err();
    assert!(format!("{:?}", error.kind).contains(reason), "{error:?}");
    assert_eq!(fixture.source.body_reads, 0, "preflight read MQ body");
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
    let nested_mq = MqError {
        offset: Some(23),
        context: None,
        kind: super::super::mq::MqErrorKind::InvalidContext,
    };
    let nested_refinement = RefinementError {
        offset: Some(23),
        bitmap_index: 0,
        row: 0,
        x: 0,
        progress: Box::new(RefinementProgress::default()),
        kind: super::super::refinement::RefinementErrorKind::Poisoned,
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
        TextInstanceErrorKind::Poisoned,
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
    let progress = TextInstanceProgress {
        header_bytes_fetched: 2,
        mq_initialization_bytes_fetched: 3,
        mq: Some(MqSnapshot {
            interval: 0,
            code: 0,
            bit_counter: 0,
            current_input_offset: 23,
            source_bytes_fetched: 5,
            terminal_inputs: 0,
            symbols_decoded: 0,
            work_done: 0,
            poisoned: false,
        }),
        ..Default::default()
    };
    assert_eq!(progress.source_bytes_fetched(), 10);
    assert_eq!(TextInstanceProgress::default().source_bytes_fetched(), 0);
    let site = PreflightSite {
        segment: 3,
        offset: 23,
        fetched: 2,
    };
    assert!(site.cap("test", 2, 2).is_ok());
    let error = site.cap("test", 2, 3).unwrap_err();
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
        fetched: 2,
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
    const BODY: &[u8] = &[0, 0, 0, 0, 0, 0xff, 0xac];
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
    f.dictionary.progress.poisoned = true;
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
    f.banks_code_len = 1;
    preflight_reject(f, "IAID width");

    let mut f = make();
    f.budget.max_metadata_bytes = 0;
    preflight_reject(f, "catalog metadata");

    let mut f = make();
    f.budget.max_working_bytes = 0;
    preflight_reject(f, "working bytes");
}

#[test]
fn valid_huffman_and_template_zero_headers_are_typed_refusals() {
    const BODY: &[u8] = &[0, 0, 0, 0, 0, 0xff, 0xac];
    for (flags, optional, expected) in [
        (0x0011u16, vec![0, 0], "Huffman text region"),
        (0x0012u16, vec![0, 0, 0, 0], "refinement template 0"),
    ] {
        let mut fixture = Fixture::new(0x10, 1, BODY, &[ONE_PIXEL]);
        let mut data = text_data(flags, 1, BODY);
        data.splice(19..19, optional);
        fixture.source.data = data;
        fixture.text_segment.data.length = fixture.source.size();
        fixture.parsed = ready(read_text_region_header(
            &mut fixture.source,
            &fixture.text_segment,
            &fixture.dictionary_segment,
            &Limits::default(),
            TextRegionBudget::default(),
            &NeverCancel,
        ))
        .unwrap();
        fixture.source.body_start = fixture.parsed.body.offset;
        fixture.source.body_reads = 0;
        let error = preflight_reject(fixture, expected);
        assert!(matches!(
            error.kind,
            TextInstanceErrorKind::Unsupported { .. }
        ));
    }
}

#[test]
fn catalog_preflight_checks_all_new_symbols_and_export_order() {
    const BODY: &[u8] = &[0, 0, 0, 0, 0, 0xff, 0xac];
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
    let mut fixture = Fixture::new(0x10, 0, &[0, 0, 0, 0, 0, 0xff, 0xac], &[]);
    let (instances, progress) = fixture.decode_all().unwrap();
    assert!(instances.is_empty());
    assert_eq!((progress.completed_instances, progress.strips), (0, 0));
    assert_eq!(progress.decision, TextDecision::Complete);
    assert_eq!(progress.source_bytes_fetched(), 30);
    assert!(!progress.poisoned);
    assert!(fixture.temporary.0.is_empty());
}

#[test]
fn mq_initialization_bytes_are_counted_once_with_or_without_a_snapshot() {
    const BODY: &[u8] = &[0, 0, 0, 0, 0, 0xff, 0xac];
    let mut successful = Fixture::new(0x10, 0, BODY, &[]);
    let progress = successful.attempt().unwrap();
    assert_eq!(progress.header_bytes_fetched, 23);
    assert_eq!(progress.mq_initialization_bytes_fetched, 0);
    assert_eq!(progress.mq.unwrap().source_bytes_fetched, BODY.len() as u64);
    assert_eq!(progress.source_bytes_fetched(), 23 + BODY.len() as u64);

    let mut failed = Fixture::new(0x10, 0, BODY, &[]);
    failed.source.max_read = 1;
    failed.source.fault = Some((failed.parsed.body.offset + 1, ReadFault::Zero));
    let error = failed.attempt().unwrap_err();
    assert!(matches!(error.kind, TextInstanceErrorKind::Mq(_)));
    assert_eq!(error.progress.header_bytes_fetched, 23);
    assert_eq!(error.progress.mq_initialization_bytes_fetched, 1);
    assert!(error.progress.mq.is_none());
    assert_eq!(error.progress.source_bytes_fetched(), 24);
}

#[test]
fn short_reads_succeed_but_zero_and_overreported_mq_reads_are_located() {
    const BODY: &[u8] = &[0, 0, 0, 0, 0, 0xff, 0xac];
    let mut short = Fixture::new(0x10, 1, BODY, &[ONE_PIXEL]);
    short.source.max_read = 1;
    let (events, progress) = short.decode_all().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(progress.source_bytes_fetched(), 30);
    for fault in [ReadFault::Zero, ReadFault::Overreport] {
        let mut fixture = Fixture::new(0x10, 1, BODY, &[ONE_PIXEL]);
        fixture.source.fault = Some((fixture.parsed.body.offset, fault));
        let error = fixture.attempt().unwrap_err();
        assert!(matches!(error.kind, TextInstanceErrorKind::Mq(_)));
        assert_eq!(error.progress.completed_instances, 0);
        assert!(fixture.source.body_reads > 0);
        assert_eq!(error.progress.header_bytes_fetched, 23);
        assert!(error.offset >= 23);
    }
    let mut bad_marker = Fixture::new(0x10, 1, &[0, 0, 0, 0, 0, 0xff, 0x90], &[ONE_PIXEL]);
    let error = bad_marker.decode_all().unwrap_err();
    assert!(matches!(error.kind, TextInstanceErrorKind::Mq(_)));
}

#[test]
fn runtime_budgets_refuse_before_emitting_an_instance() {
    const BODY: &[u8] = &[0, 0, 0, 0, 0, 0xff, 0xac];
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
fn refinement_host_and_reference_errors_poison_the_text_session() {
    const BODY: [u8; 14] = [
        235, 233, 217, 144, 134, 94, 12, 87, 10, 58, 12, 111, 255, 172,
    ];
    let mut f = Fixture::new(0x8012, 1, &BODY, &[ONE_PIXEL]);
    f.qe = 0x4000;
    f.refinement_budget.max_source_request_bytes = 0;
    let error = f.decode_all().unwrap_err();
    assert!(matches!(error.kind, TextInstanceErrorKind::Refinement(_)));
    assert!(error.progress.poisoned);
    assert!(f.temporary.0.is_empty());

    let mut f = Fixture::new(0x8012, 1, &BODY, &[ONE_PIXEL]);
    f.qe = 0x4000;
    f.imported.fault = Some((0, ReadFault::Zero));
    let error = f.decode_all().unwrap_err();
    assert!(matches!(error.kind, TextInstanceErrorKind::Refinement(_)));
    assert_eq!(error.progress.refinement.reference_reads, 1);
    assert!(error.progress.poisoned);
}

#[test]
fn refined_new_store_handle_remains_distinct_from_dictionary_symbols() {
    const BODY: [u8; 14] = [
        235, 233, 217, 144, 134, 94, 12, 87, 10, 58, 12, 111, 255, 172,
    ];
    let mut f = Fixture::new(0x8012, 1, &BODY, &[ONE_PIXEL]);
    f.qe = 0x4000;
    f.dictionary.catalog.new_symbols.push(ONE_PIXEL);
    f.dictionary.catalog.exported_symbols[0].store = SymbolStore::New;
    f.dictionary.header.new_symbols = 1;
    f.dictionary.progress.completed_symbols = 1;
    f.fresh.data = vec![0x80];
    f.imported.data.clear();
    f.temporary_base = 77;
    let (events, progress) = f.decode_all().unwrap();
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0].bitmap,
        TextBitmap::Refined { store_base: 77, .. }
    ));
    assert_eq!(progress.refinement.reference_reads, 1);
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
fn cancellation_and_repeated_pull_report_poisoned_progress() {
    const BODY: &[u8] = &[0, 0, 0, 0, 0, 0xff, 0xac];
    let mut f = Fixture::new(0x10, 1, BODY, &[ONE_PIXEL]);
    let limits = Limits::default();
    let table = table(&limits);
    let mut banks =
        IaidContextBanks::with_bitmap_contexts(0, GR_CONTEXTS, &limits, &f.mq_budget).unwrap();
    let cancellation = ToggleCancel(Cell::new(false));
    let mut decoder = ready(TextInstanceDecoder::new(
        &mut f.source,
        &f.text_segment,
        f.parsed,
        &f.dictionary_segment,
        &f.dictionary,
        &mut f.imported,
        0,
        &mut f.fresh,
        0,
        &mut f.temporary,
        0,
        &table,
        &mut banks,
        &limits,
        &cancellation,
        f.mq_budget,
        f.header_budget,
        f.refinement_budget,
        f.budget,
    ))
    .unwrap();
    cancellation.0.set(true);
    let cancelled = ready(decoder.next()).unwrap_err();
    assert!(matches!(cancelled.kind, TextInstanceErrorKind::Cancelled));
    assert!(cancelled.progress.poisoned);
    cancellation.0.set(false);
    let repeated = ready(decoder.next()).unwrap_err();
    assert!(matches!(repeated.kind, TextInstanceErrorKind::Poisoned));
    assert_eq!(repeated.progress.completed_instances, 0);
}

#[test]
fn dropped_pending_refinement_flush_poisoned_and_requires_discard() {
    const BODY: [u8; 14] = [
        235, 233, 217, 144, 134, 94, 12, 87, 10, 58, 12, 111, 255, 172,
    ];
    let mut f = Fixture::new(0x8012, 1, &BODY, &[ONE_PIXEL]);
    let limits = Limits::default();
    let table = table_with_qe(&limits, 0x4000);
    let mut banks =
        IaidContextBanks::with_bitmap_contexts(0, GR_CONTEXTS, &limits, &f.mq_budget).unwrap();
    let pending = Rc::new(Cell::new(false));
    let mut sink = PendingFlushSink {
        bytes: Vec::new(),
        pending: Rc::clone(&pending),
    };
    let mut decoder = ready(TextInstanceDecoder::new(
        &mut f.source,
        &f.text_segment,
        f.parsed,
        &f.dictionary_segment,
        &f.dictionary,
        &mut f.imported,
        0,
        &mut f.fresh,
        0,
        &mut sink,
        0,
        &table,
        &mut banks,
        &limits,
        &NeverCancel,
        f.mq_budget,
        f.header_budget,
        f.refinement_budget,
        f.budget,
    ))
    .unwrap();
    pending.set(true);
    let mut future = Box::pin(decoder.next());
    assert!(matches!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
    drop(future);
    let progress = decoder.progress();
    assert!(progress.poisoned);
    assert_eq!(progress.refinement.output_bytes_written, 4);
    assert_eq!(progress.refinement.flushes, 1);
    let error = ready(decoder.next()).unwrap_err();
    assert!(matches!(error.kind, TextInstanceErrorKind::Poisoned));
    drop(decoder);
    assert_eq!(sink.bytes, [0xe0, 0x70, 0x50, 0x10]);
}

#[test]
fn poisoned_prior_refinement_progress_cannot_resume_a_temporary_store() {
    const BODY: [u8; 14] = [
        235, 233, 217, 144, 134, 94, 12, 87, 10, 58, 12, 111, 255, 172,
    ];
    let mut f = Fixture::new(0x8012, 1, &BODY, &[ONE_PIXEL]);
    let limits = Limits::default();
    let table = table_with_qe(&limits, 0x4000);
    let mut banks =
        IaidContextBanks::with_bitmap_contexts(0, GR_CONTEXTS, &limits, &f.mq_budget).unwrap();
    let mut decoder = ready(TextInstanceDecoder::new(
        &mut f.source,
        &f.text_segment,
        f.parsed,
        &f.dictionary_segment,
        &f.dictionary,
        &mut f.imported,
        0,
        &mut f.fresh,
        0,
        &mut f.temporary,
        0,
        &table,
        &mut banks,
        &limits,
        &NeverCancel,
        f.mq_budget,
        f.header_budget,
        f.refinement_budget,
        f.budget,
    ))
    .unwrap();
    decoder.progress.refinement.poisoned = true;
    let error = ready(decoder.next()).unwrap_err();
    assert!(matches!(error.kind, TextInstanceErrorKind::Refinement(_)));
    assert!(error.progress.poisoned);
    drop(decoder);
    assert!(f.temporary.0.is_empty());
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
    const BODY: &[u8] = &[0, 0, 0, 0, 0, 0xff, 0xac];
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
    const BODY: &[u8] = &[0, 0, 0, 0, 0, 0xff, 0xac];
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
    const VALID: [u8; 14] = [
        204, 103, 196, 91, 95, 211, 152, 13, 121, 0, 247, 238, 255, 172,
    ];
    const INVALID: [u8; 14] = [
        10, 31, 169, 106, 252, 198, 7, 252, 187, 114, 178, 79, 255, 172,
    ];
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
    let table = table(&limits);
    let mut banks =
        IaidContextBanks::with_bitmap_contexts(0, GR_CONTEXTS, &limits, &mq_budget).unwrap();
    let mut source = Bytes::new(text_data(0x10, 1, &[0, 0, 0, 0, 0, 0xff, 0xac]));
    let text_segment = segment(3, 6, vec![2], 0, source.size());
    let dictionary_segment = segment(2, 0, vec![1], 100, 2);
    let dictionary = report(&[ONE_PIXEL]);
    let mut imported = Bytes::new(vec![0x80]);
    let mut fresh = Bytes::new(vec![]);
    let mut temporary = Sink::default();
    let parsed = ready(read_text_region_header(
        &mut source,
        &text_segment,
        &dictionary_segment,
        &limits,
        TextRegionBudget::default(),
        &NeverCancel,
    ))
    .unwrap();
    let mut decoder = ready(TextInstanceDecoder::new(
        &mut source,
        &text_segment,
        parsed,
        &dictionary_segment,
        &dictionary,
        &mut imported,
        0,
        &mut fresh,
        0,
        &mut temporary,
        0,
        &table,
        &mut banks,
        &limits,
        &NeverCancel,
        mq_budget,
        TextRegionBudget::default(),
        RefinementBudget::default(),
        TextInstanceBudget::default(),
    ))
    .unwrap();
    let first = ready(decoder.next()).unwrap().unwrap();
    assert_eq!(
        (first.index, first.strip, first.symbol_id, first.x, first.y),
        (0, 0, 0, 0, 4)
    );
    assert_eq!(
        first.bitmap,
        TextBitmap::Stored(dictionary.catalog.exported_symbols[0])
    );
    assert!(!first.ri);
    assert!(ready(decoder.next()).unwrap().is_none());
    assert!(ready(decoder.next()).unwrap().is_none());
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
    assert!(!progress.poisoned);
}

#[test]
fn real_mq_refinement_reads_reference_and_writes_packed_rows() {
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    // An invented flat 47-state table; this is not T.88 Table E.1.
    let table = MqTable::new(
        vec![
            MqState {
                qe: 0x4000,
                next_mps: 0,
                next_lps: 0,
                switch_mps: false
            };
            MQ_STATE_COUNT
        ],
        &limits,
    )
    .unwrap();
    let mut banks =
        IaidContextBanks::with_bitmap_contexts(0, GR_CONTEXTS, &limits, &mq_budget).unwrap();
    let dictionary_segment = segment(2, 0, vec![1], 100, 2);
    let dictionary = report(&[ONE_PIXEL]);
    const BODY: [u8; 14] = [
        235, 233, 217, 144, 134, 94, 12, 87, 10, 58, 12, 111, 255, 172,
    ];
    let mut source = Bytes::new(text_data(0x8012, 1, &BODY));
    let text_segment = segment(3, 6, vec![2], 0, source.size());
    let mut imported = Bytes::new(vec![0x80]);
    let mut fresh = Bytes::new(vec![]);
    let mut temporary = Sink::default();
    let parsed = ready(read_text_region_header(
        &mut source,
        &text_segment,
        &dictionary_segment,
        &limits,
        TextRegionBudget::default(),
        &NeverCancel,
    ))
    .unwrap();
    let mut decoder = ready(TextInstanceDecoder::new(
        &mut source,
        &text_segment,
        parsed,
        &dictionary_segment,
        &dictionary,
        &mut imported,
        0,
        &mut fresh,
        0,
        &mut temporary,
        73,
        &table,
        &mut banks,
        &limits,
        &NeverCancel,
        mq_budget,
        TextRegionBudget::default(),
        RefinementBudget::default(),
        TextInstanceBudget::default(),
    ))
    .unwrap();
    let instance = ready(decoder.next()).unwrap().unwrap();
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
    assert!(ready(decoder.next()).unwrap().is_none());
    let progress = decoder.progress();
    assert_eq!(
        (
            progress.ri_zero,
            progress.ri_one,
            progress.refinement.reference_reads
        ),
        (0, 1, 1)
    );
    drop(decoder);
    assert_eq!(temporary.0, [0xe0, 0x70, 0x50, 0x10]);
}

#[test]
fn a_text_region_resets_dirty_integer_iaid_and_gr_statistics() {
    const BODY: [u8; 14] = [
        235, 233, 217, 144, 134, 94, 12, 87, 10, 58, 12, 111, 255, 172,
    ];
    let mut f = Fixture::new(0x8012, 1, &BODY, &[ONE_PIXEL]);
    let limits = Limits::default();
    let table = table_with_qe(&limits, 0x4000);
    let mut banks =
        IaidContextBanks::with_bitmap_contexts(0, GR_CONTEXTS, &limits, &f.mq_budget).unwrap();
    let layout = banks.layout();
    let indices = [0, layout.iaid_base(), layout.bitmap_base()];
    for index in indices {
        banks
            .mq_contexts_mut()
            .set(
                index,
                MqContext {
                    state_index: 1,
                    mps: true,
                },
            )
            .unwrap();
    }
    let mut decoder = ready(TextInstanceDecoder::new(
        &mut f.source,
        &f.text_segment,
        f.parsed,
        &f.dictionary_segment,
        &f.dictionary,
        &mut f.imported,
        0,
        &mut f.fresh,
        0,
        &mut f.temporary,
        0,
        &table,
        &mut banks,
        &limits,
        &NeverCancel,
        f.mq_budget,
        f.header_budget,
        f.refinement_budget,
        f.budget,
    ))
    .unwrap();
    for index in indices {
        assert_eq!(decoder.mq.context(index), Some(MqContext::default()));
    }
    assert!(ready(decoder.next()).unwrap().unwrap().ri);
    assert!(ready(decoder.next()).unwrap().is_none());
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
    // Every invented state has the same Qe and MPS, so decisions stay the
    // same as the flat fixture while state indices record repeated use.
    let states = (0..MQ_STATE_COUNT)
        .map(|index| MqState {
            qe: 0x4000,
            next_mps: (index + 1).min(MQ_STATE_COUNT - 1) as u8,
            next_lps: (index + 1).min(MQ_STATE_COUNT - 1) as u8,
            switch_mps: false,
        })
        .collect();
    let table = MqTable::new(states, &limits).unwrap();
    let mut banks =
        IaidContextBanks::with_bitmap_contexts(1, GR_CONTEXTS, &limits, &f.mq_budget).unwrap();
    let iaid_base = banks.layout().iaid_base();
    let gr_base = banks.layout().bitmap_base();
    let mut decoder = ready(TextInstanceDecoder::new(
        &mut f.source,
        &f.text_segment,
        f.parsed,
        &f.dictionary_segment,
        &f.dictionary,
        &mut f.imported,
        0,
        &mut f.fresh,
        0,
        &mut f.temporary,
        0,
        &table,
        &mut banks,
        &limits,
        &NeverCancel,
        f.mq_budget,
        f.header_budget,
        f.refinement_budget,
        f.budget,
    ))
    .unwrap();
    let first = ready(decoder.next()).unwrap().unwrap();
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
    // floor(-1/2)=-1 on both axes. GR context 6706 is reached with that
    // offset; truncation toward zero would use context 6664 instead.
    assert_eq!(decoder.mq.context(6706).unwrap().state_index, 1);
    assert_eq!(decoder.mq.context(6664).unwrap().state_index, 0);
    let integer_after_first: Vec<_> = (0..iaid_base)
        .map(|index| decoder.mq.context(index).unwrap())
        .collect();
    let iaid_after_first = decoder.mq.context(iaid_base + 1).unwrap();
    let gr_after_first: Vec<_> = (gr_base..gr_base + GR_CONTEXTS)
        .map(|index| decoder.mq.context(index).unwrap())
        .collect();
    assert!(
        integer_after_first
            .iter()
            .any(|state| state.state_index > 0)
    );
    assert!(iaid_after_first.state_index > 0);
    assert!(gr_after_first.iter().any(|state| state.state_index > 0));

    let second = ready(decoder.next()).unwrap().unwrap();
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
    assert!((0..iaid_base).any(|index| {
        decoder.mq.context(index).unwrap().state_index > integer_after_first[index].state_index
    }));
    assert!(decoder.mq.context(iaid_base + 1).unwrap().state_index > iaid_after_first.state_index);
    assert!((0..GR_CONTEXTS).any(|index| {
        decoder.mq.context(gr_base + index).unwrap().state_index > gr_after_first[index].state_index
    }));
    assert!(ready(decoder.next()).unwrap().is_none());
    assert_eq!(decoder.progress().ri_one, 2);
    drop(decoder);
    assert_eq!(f.temporary.0, [0, 160, 224, 224, 192, 192, 96, 96]);
}

#[test]
fn refined_handle_is_visible_to_a_reopened_view_before_next_pull() {
    const BODY: [u8; 14] = [
        235, 233, 217, 144, 134, 94, 12, 87, 10, 58, 12, 111, 255, 172,
    ];
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let table = table_with_qe(&limits, 0x4000);
    let mut banks =
        IaidContextBanks::with_bitmap_contexts(0, GR_CONTEXTS, &limits, &mq_budget).unwrap();
    let mut source = Bytes::new(text_data(0x8012, 1, &BODY));
    let text_segment = segment(3, 6, vec![2], 0, source.size());
    let dictionary_segment = segment(2, 0, vec![1], 100, 2);
    let dictionary = report(&[ONE_PIXEL]);
    let mut imported = Bytes::new(vec![0x80]);
    let mut fresh = Bytes::new(vec![]);
    let visible = Rc::new(RefCell::new(Vec::new()));
    let mut temporary = BufferingSink {
        pending: Vec::new(),
        visible: Rc::clone(&visible),
    };
    let parsed = ready(read_text_region_header(
        &mut source,
        &text_segment,
        &dictionary_segment,
        &limits,
        TextRegionBudget::default(),
        &NeverCancel,
    ))
    .unwrap();
    let mut decoder = ready(TextInstanceDecoder::new(
        &mut source,
        &text_segment,
        parsed,
        &dictionary_segment,
        &dictionary,
        &mut imported,
        0,
        &mut fresh,
        0,
        &mut temporary,
        0,
        &table,
        &mut banks,
        &limits,
        &NeverCancel,
        mq_budget,
        TextRegionBudget::default(),
        RefinementBudget::default(),
        TextInstanceBudget::default(),
    ))
    .unwrap();
    let instance = ready(decoder.next()).unwrap().unwrap();
    assert!(matches!(instance.bitmap, TextBitmap::Refined { .. }));
    assert_eq!(*visible.borrow(), [0xe0, 0x70, 0x50, 0x10]);
    assert_eq!(decoder.progress().refinement.flushes, 1);
    assert!(ready(decoder.next()).unwrap().is_none());
}

#[test]
fn real_mq_multistrip_oob_and_subsequent_s_with_ds_offset() {
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let table = MqTable::new(
        vec![
            MqState {
                qe: 0x4000,
                next_mps: 0,
                next_lps: 0,
                switch_mps: false
            };
            MQ_STATE_COUNT
        ],
        &limits,
    )
    .unwrap();
    let mut banks =
        IaidContextBanks::with_bitmap_contexts(0, GR_CONTEXTS, &limits, &mq_budget).unwrap();
    let dictionary_segment = segment(2, 0, vec![1], 100, 2);
    let dictionary = report(&[ONE_PIXEL]);
    const DIFFERENT: [u8; 22] = [
        8, 228, 89, 64, 225, 208, 5, 116, 231, 189, 187, 198, 231, 62, 32, 43, 175, 165, 243, 232,
        255, 172,
    ];
    const SAME: [u8; 22] = [
        233, 120, 85, 216, 174, 231, 90, 233, 97, 208, 37, 183, 16, 231, 246, 124, 120, 201, 31,
        170, 255, 172,
    ];
    for (flags, body, expected) in [
        (
            0x001c,
            DIFFERENT,
            [(-506_465_247, 2216, 0), (-506_465_245, 2257, 1)],
        ),
        (0x781c, SAME, [(-62, 425, 0), (182, 426, 0)]),
    ] {
        let mut source = Bytes::new(text_data(flags, 2, &body));
        let text_segment = segment(3, 6, vec![2], 0, source.size());
        let mut imported = Bytes::new(vec![0x80]);
        let mut fresh = Bytes::new(vec![]);
        let mut temporary = Sink::default();
        let parsed = ready(read_text_region_header(
            &mut source,
            &text_segment,
            &dictionary_segment,
            &limits,
            TextRegionBudget::default(),
            &NeverCancel,
        ))
        .unwrap();
        let mut decoder = ready(TextInstanceDecoder::new(
            &mut source,
            &text_segment,
            parsed,
            &dictionary_segment,
            &dictionary,
            &mut imported,
            0,
            &mut fresh,
            0,
            &mut temporary,
            0,
            &table,
            &mut banks,
            &limits,
            &NeverCancel,
            mq_budget,
            TextRegionBudget::default(),
            RefinementBudget::default(),
            TextInstanceBudget {
                max_strips: 4,
                ..TextInstanceBudget::default()
            },
        ))
        .unwrap();
        for (index, expected) in expected.into_iter().enumerate() {
            let instance = ready(decoder.next()).unwrap().unwrap();
            assert_eq!((instance.x, instance.y, instance.strip), expected);
            assert_eq!(instance.index, index as u32);
            assert_eq!(
                instance.bitmap,
                TextBitmap::Stored(dictionary.catalog.exported_symbols[0])
            );
        }
        assert!(ready(decoder.next()).unwrap().is_none());
        let progress = decoder.progress();
        assert_eq!(progress.completed_instances, 2);
        assert_eq!(progress.ri_zero, 2);
        assert!(!progress.poisoned);
    }
}
