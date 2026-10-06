// SPDX-License-Identifier: MIT

//! Composer tests over the real instance decoder. Each region's MQ body is
//! coded for the standard T.88 states by the test-only encoder from the
//! placements a test asks for.

use super::*;
use crate::NeverCancel;
use crate::Payload;
use crate::jbig2::{
    SegmentHeader, SegmentSpan,
    dictionary::{
        DictionaryCatalog, DictionaryDataHeader, DictionaryMode, DictionaryProgress,
        DictionaryReport, coding_unit_contexts, symbol_code_length,
    },
    iaid::IAID_BASE,
    integer::{BITMAP_BASE, IntegerProcedure},
    mq::{ArithmeticSnapshot, ContextBank, MqBudget, MqTable},
    refinement::RefinementBudget,
    text::{
        ReferenceCorner, TextHeaderPolicy, TextRegionBudget, TextRegionFlags,
        read_text_region_header_with_policy,
    },
    text_instances::{TextInstanceBudget, TextInstanceDecoder},
};
use std::{cell::Cell, error::Error as StdError, rc::Rc};

/// Raw text-region flags with the top-left reference corner.
fn flags(default_pixel: bool, combination: SymbolCombination) -> u16 {
    let combination = match combination {
        SymbolCombination::Or => 0,
        SymbolCombination::And => 1,
        SymbolCombination::Xor => 2,
        SymbolCombination::Xnor => 3,
    };
    0x10 | combination << 7 | u16::from(default_pixel) << 9
}

/// Flags that also code refined instances with refinement template 1.
const REFINE: u16 = 0x8012;

fn descriptor(width: u32, height: u32, relative_store_offset: u64) -> SymbolDescriptor {
    let row_stride = width.div_ceil(8);
    SymbolDescriptor {
        width,
        height,
        row_stride,
        relative_store_offset,
        stored_bytes: u64::from(row_stride) * u64::from(height),
    }
}

fn stored(store: SymbolStore, symbol: SymbolDescriptor) -> StoredSymbol {
    StoredSymbol {
        store,
        store_base: 0,
        symbol,
    }
}

/// One instance for the test encoder: catalog symbol `id` with its top-left
/// corner at `(x, y)`, refined to `rows` when given.
#[derive(Clone)]
struct Placement {
    id: u32,
    x: i64,
    y: i64,
    refined: Option<Vec<Vec<bool>>>,
}

fn place(id: u32, x: i64, y: i64) -> Placement {
    Placement {
        id,
        x,
        y,
        refined: None,
    }
}

fn refine(id: u32, x: i64, y: i64, rows: Vec<Vec<bool>>) -> Placement {
    Placement {
        refined: Some(rows),
        ..place(id, x, y)
    }
}

/// The rows of `symbol` in `store`.
fn unpack(symbol: StoredSymbol, store: &[u8]) -> Vec<Vec<bool>> {
    let start = (symbol.store_base + symbol.symbol.relative_store_offset) as usize;
    let stride = symbol.symbol.row_stride as usize;
    (0..symbol.symbol.height as usize)
        .map(|row| {
            let bytes = &store[start + row * stride..start + (row + 1) * stride];
            (0..symbol.symbol.width as usize)
                .map(|x| bytes[x / 8] & (0x80 >> (x % 8)) != 0)
                .collect()
        })
        .collect()
}

/// The MQ body that places each of `placements` in its own strip: T.88
/// §6.4.5 run backwards for the region's corner, transposition, and strip
/// size.
fn encode(
    flags: TextRegionFlags,
    catalog: &[StoredSymbol],
    stores: [&[u8]; 2],
    placements: &[Placement],
) -> Vec<u8> {
    let strips = i64::from(flags.strips());
    let code_len = symbol_code_length(catalog.len() as u64);
    let right = matches!(
        flags.reference_corner,
        ReferenceCorner::TopRight | ReferenceCorner::BottomRight
    );
    let bottom = matches!(
        flags.reference_corner,
        ReferenceCorner::BottomLeft | ReferenceCorner::BottomRight
    );
    let mut encoder = crate::test_support::mq_encoder();
    encoder.integer(IntegerProcedure::Iadt.base(), Some(0));
    let (mut strip_t, mut first_s) = (0, 0);
    for (index, placement) in placements.iter().enumerate() {
        if index > 0 {
            encoder.integer(IntegerProcedure::Iads.base(), None);
        }
        let reference = catalog[placement.id as usize];
        let (width, height) = placement
            .refined
            .as_ref()
            .map_or((reference.symbol.width, reference.symbol.height), |rows| {
                (rows[0].len() as u32, rows.len() as u32)
            });
        let (s, t) = if flags.transposed {
            let t = placement.x + if right { i64::from(width) - 1 } else { 0 };
            (placement.y, t)
        } else {
            let t = placement.y + if bottom { i64::from(height) - 1 } else { 0 };
            (placement.x, t)
        };
        let strip = t.div_euclid(strips) * strips;
        encoder.integer(
            IntegerProcedure::Iadt.base(),
            Some((strip - strip_t) / strips),
        );
        strip_t = strip;
        encoder.integer(IntegerProcedure::Iafs.base(), Some(s - first_s));
        first_s = s;
        if strips > 1 {
            encoder.integer(IntegerProcedure::Iait.base(), Some(t - strip));
        }
        encoder.iaid(IAID_BASE, code_len, u64::from(placement.id));
        if flags.refine {
            encoder.integer(
                IntegerProcedure::Iari.base(),
                Some(i64::from(placement.refined.is_some())),
            );
        }
        if let Some(rows) = &placement.refined {
            let rdw = i64::from(width) - i64::from(reference.symbol.width);
            let rdh = i64::from(height) - i64::from(reference.symbol.height);
            encoder.integer(IntegerProcedure::Iardw.base(), Some(rdw));
            encoder.integer(IntegerProcedure::Iardh.base(), Some(rdh));
            // These cancel the Table 12 offsets, so the reference is aligned.
            encoder.integer(IntegerProcedure::Iardx.base(), Some(-rdw.div_euclid(2)));
            encoder.integer(IntegerProcedure::Iardy.base(), Some(-rdh.div_euclid(2)));
            let store = stores[usize::from(reference.store == SymbolStore::New)];
            encoder.template1(BITMAP_BASE, rows, &unpack(reference, store), (0, 0));
        }
    }
    encoder.finish()
}

/// Text segment data: a `width` by `height` region at the origin.
fn text_data(width: u32, height: u32, flags: u16, instances: u32, body: &[u8]) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&width.to_be_bytes());
    data.extend_from_slice(&height.to_be_bytes());
    data.extend_from_slice(&0u32.to_be_bytes());
    data.extend_from_slice(&0u32.to_be_bytes());
    data.push(0);
    data.extend_from_slice(&flags.to_be_bytes());
    data.extend_from_slice(&instances.to_be_bytes());
    data.extend_from_slice(body);
    data
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

/// A complete refinement-dictionary report exporting `catalog`.
fn report(catalog: &[StoredSymbol]) -> DictionaryReport {
    let new_symbols: Vec<_> = catalog
        .iter()
        .filter(|stored| stored.store == SymbolStore::New)
        .map(|stored| stored.symbol)
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
            exported_symbols: catalog.len() as u32,
            new_symbols: new_symbols.len() as u32,
            header_bytes: 0,
            body: SegmentSpan {
                offset: 100,
                length: 2,
            },
        },
        progress: DictionaryProgress {
            completed_symbols: new_symbols.len() as u32,
            mq: Some(ArithmeticSnapshot {
                interval: 0,
                code: 0,
                bit_counter: 0,
                input_offset: 100,
                synthesized_inputs: 0,
                symbols_decoded: 0,
                work_done: 0,
            }),
            ..DictionaryProgress::default()
        },
        catalog: DictionaryCatalog {
            new_symbols,
            exported_symbols: catalog.to_vec(),
        },
    }
}

/// A text region coded for the real instance decoder, with the decoder's
/// own copies of the dictionary stores.
struct Region {
    source: Vec<u8>,
    text_segment: SegmentHeader,
    dictionary_segment: SegmentHeader,
    header: TextRegionHeader,
    report: DictionaryReport,
    imported: Vec<u8>,
    imported_base: u64,
    new: Vec<u8>,
    new_base: u64,
    contexts: ContextBank,
    table: MqTable,
    limits: Limits,
    budget: TextInstanceBudget,
}

impl Region {
    /// Place `placements` in a `width` by `height` region over `catalog`,
    /// whose stores hold `imported` and `new`.
    fn new(
        width: u32,
        height: u32,
        flags: u16,
        catalog: &[StoredSymbol],
        [imported, new]: [&[u8]; 2],
        placements: &[Placement],
    ) -> Self {
        let instances = placements.len() as u32;
        let parsed = Self::parse(width, height, flags, instances, &[0xff, 0xac]);
        let body = encode(parsed.0.flags, catalog, [imported, new], placements);
        let (header, source, text_segment, dictionary_segment) =
            Self::parse(width, height, flags, instances, &body);
        let limits = Limits::default();
        let code_len = symbol_code_length(catalog.len() as u64);
        let contexts = MqBudget::default()
            .context_bank(coding_unit_contexts(code_len).unwrap(), &limits)
            .unwrap();
        Self {
            source,
            text_segment,
            dictionary_segment,
            header,
            report: report(catalog),
            imported: imported.to_vec(),
            imported_base: 0,
            new: new.to_vec(),
            new_base: 0,
            contexts,
            table: MqTable::standard(),
            limits,
            budget: TextInstanceBudget::default(),
        }
    }

    fn parse(
        width: u32,
        height: u32,
        flags: u16,
        instances: u32,
        body: &[u8],
    ) -> (TextRegionHeader, Vec<u8>, SegmentHeader, SegmentHeader) {
        let source = text_data(width, height, flags, instances, body);
        let text_segment = segment(3, 6, vec![2], 0, source.len() as u64);
        let dictionary_segment = segment(2, 0, vec![1], 100, 2);
        let header = read_text_region_header_with_policy(
            &mut &source[..],
            &text_segment,
            &dictionary_segment,
            &Limits::default(),
            TextRegionBudget::default(),
            &NeverCancel,
            TextHeaderPolicy::HnC8UnusedRefinementTemplate,
        )
        .unwrap();
        (header, source, text_segment, dictionary_segment)
    }

    /// One 1x1 imported symbol at (1, 1) of a 3x2 region.
    fn one_pixel() -> Self {
        let symbol = stored(SymbolStore::Imported, descriptor(1, 1, 0));
        Self::new(
            3,
            2,
            flags(false, SymbolCombination::Or),
            &[symbol],
            [&[0x80], &[]],
            &[place(0, 1, 1)],
        )
    }

    fn catalog(&self) -> Vec<StoredSymbol> {
        self.report.catalog.exported_symbols.clone()
    }

    /// The real instance decoder, appending refined bitmaps to `refined`.
    fn decoder<'r, C: Cancellation>(
        &'r mut self,
        refined: &'r mut Vec<u8>,
        cancellation: &'r C,
    ) -> TextInstanceDecoder<'r, C> {
        TextInstanceDecoder::new_with_header_policy(
            Payload::from(&self.source[..]),
            &self.text_segment,
            self.header,
            &self.dictionary_segment,
            &self.report,
            &self.imported,
            self.imported_base,
            &self.new,
            self.new_base,
            refined,
            0,
            &self.table,
            &mut self.contexts,
            &self.limits,
            cancellation,
            MqBudget::default(),
            TextRegionBudget::default(),
            RefinementBudget::default(),
            self.budget,
            TextHeaderPolicy::HnC8UnusedRefinementTemplate,
        )
        .unwrap()
    }
}

/// A completed composition: the report, the packed region bitmap, and the
/// refined store the decoder appended to.
struct Composed {
    report: TextComposeReport,
    bitmap: Vec<u8>,
    refined: Vec<u8>,
}

/// Decode `region` with the real instance decoder and compose it over the
/// given catalog and views of the dictionary stores.
fn compose_views<C: Cancellation>(
    region: &mut Region,
    catalog: &[StoredSymbol],
    [imported, new]: [&[u8]; 2],
    refined_base: u64,
    budget: TextComposeBudget,
    cancellation: &C,
) -> TextComposeResult<Composed> {
    let header = region.header;
    let limits = Limits::default();
    let mut refined = Vec::new();
    let mut bitmap = Vec::new();
    let mut decoder = region.decoder(&mut refined, cancellation);
    let report = TextComposer::new(
        3,
        header,
        catalog,
        &mut decoder,
        imported,
        0,
        new,
        0,
        refined_base,
        &mut bitmap,
        &limits,
        cancellation,
        budget,
    )?
    .compose()?;
    Ok(Composed {
        report,
        bitmap,
        refined,
    })
}

/// Compose `region` over its own catalog and stores.
fn compose(region: &mut Region, budget: TextComposeBudget) -> TextComposeResult<Composed> {
    let catalog = region.catalog();
    let (imported, new) = (region.imported.clone(), region.new.clone());
    compose_views(region, &catalog, [&imported, &new], 0, budget, &NeverCancel)
}

#[test]
fn completed_report_retains_explicit_header_anomaly() {
    let symbol = stored(SymbolStore::Imported, descriptor(1, 1, 0));
    // SBRTEMPLATE without SBREFINE, eight strips, and SBDSOFFSET 9.
    let mut region = Region::new(1, 1, 0xa40c, &[symbol], [&[0x80], &[]], &[place(0, 0, 0)]);
    assert_eq!(region.header.flags.log_strips, 3);
    assert_eq!(region.header.flags.ds_offset, 9);
    let report = compose(&mut region, TextComposeBudget::default())
        .unwrap()
        .report;
    assert_eq!(report.text_flags_raw, 0xa40c);
    assert_eq!(
        report.header_anomaly,
        Some(TextHeaderAnomaly::UnusedRefinementTemplate)
    );
}

#[test]
fn composes_imported_new_and_refined_handles_in_nonmonotone_order() {
    let imported_symbol = stored(SymbolStore::Imported, descriptor(3, 2, 0));
    let new_symbol = stored(SymbolStore::New, descriptor(2, 2, 0));
    let imported_rows = [0xa0, 0x40];
    let new_rows = [0xc0, 0xc0];
    let mut region = Region::new(
        5,
        3,
        REFINE,
        &[imported_symbol, new_symbol],
        [&imported_rows, &new_rows],
        &[
            place(0, 1, 0),
            place(1, 3, 1),
            refine(0, 0, 0, vec![vec![true], vec![true]]),
            place(1, -4, 1),
        ],
    );
    let composed = compose(&mut region, TextComposeBudget::default()).unwrap();
    // The decoder appended the refined 1x2 bitmap the composer then read.
    assert_eq!(composed.refined, [0x80, 0x80]);
    assert_eq!(composed.bitmap, [0xd0, 0xb8, 0x18]);
    let report = composed.report;
    assert_eq!(
        (
            report.progress.completed_instances,
            report.progress.touched_pixels
        ),
        (4, 12)
    );
    assert_eq!(report.packed_bytes, 3);
    assert_eq!(report.progress.stage, TextComposeStage::Complete);
}

#[test]
fn exact_negative_clipping_and_entirely_off_region_instances() {
    let symbol = stored(SymbolStore::Imported, descriptor(3, 2, 0));
    let rows = [0xa0, 0x40];
    let mut region = Region::new(
        5,
        2,
        flags(false, SymbolCombination::Or),
        &[symbol],
        [&rows, &[]],
        &[place(0, -1, -1), place(0, 5, 4)],
    );
    let composed = compose(&mut region, TextComposeBudget::default()).unwrap();
    assert_eq!(composed.bitmap, [0x80, 0x00]);
    assert_eq!(composed.report.progress.touched_pixels, 2);
    assert_eq!(composed.report.progress.completed_instances, 2);
}

#[test]
fn all_symbol_operators_preserve_placement_order_and_padding() {
    let first = stored(SymbolStore::Imported, descriptor(2, 1, 0));
    let second = stored(SymbolStore::Imported, descriptor(2, 1, 1));
    let cases = [
        (SymbolCombination::Or, false, 0xc0, 0xe0),
        (SymbolCombination::And, true, 0x80, 0xc0),
        (SymbolCombination::Xor, false, 0xc0, 0xa0),
        (SymbolCombination::Xnor, false, 0x80, 0x20),
    ];
    for (operator, default_pixel, second_byte, expected) in cases {
        let rows = [0xc0, second_byte];
        let mut region = Region::new(
            3,
            1,
            flags(default_pixel, operator),
            &[first, second],
            [&rows, &[]],
            &[place(0, 0, 0), place(1, 1, 0)],
        );
        let composed = compose(&mut region, TextComposeBudget::default()).unwrap();
        assert_eq!(composed.bitmap, [expected], "{operator:?}");
    }
}

#[test]
fn zero_instances_initialize_both_default_pixels_and_clear_padding() {
    for (default_pixel, expected) in [
        (false, [0x00, 0x00, 0x00, 0x00]),
        (true, [0xff, 0xc0, 0xff, 0xc0]),
    ] {
        let mut region = Region::new(
            10,
            2,
            flags(default_pixel, SymbolCombination::And),
            &[],
            [&[], &[]],
            &[],
        );
        let composed = compose(&mut region, TextComposeBudget::default()).unwrap();
        assert_eq!(composed.bitmap, expected);
        assert_eq!(composed.report.progress.completed_instances, 0);
        assert_eq!(composed.report.progress.work_units, 4);
    }
}

#[test]
fn checked_top_left_placement_is_used_for_every_corner_and_transpose_mode() {
    let symbol = stored(SymbolStore::Imported, descriptor(2, 1, 0));
    for (corner, code) in [
        (ReferenceCorner::BottomLeft, 0),
        (ReferenceCorner::TopLeft, 1),
        (ReferenceCorner::BottomRight, 2),
        (ReferenceCorner::TopRight, 3),
    ] {
        for transposed in [false, true] {
            let raw = code << 4 | u16::from(transposed) << 6;
            let mut region = Region::new(4, 3, raw, &[symbol], [&[0xc0], &[]], &[place(0, 1, 2)]);
            assert_eq!(region.header.flags.reference_corner, corner);
            let composed = compose(&mut region, TextComposeBudget::default()).unwrap();
            assert_eq!(
                composed.bitmap,
                [0, 0, 0x60],
                "{corner:?}, transposed={transposed}"
            );
        }
    }
}

#[derive(Clone)]
struct Flag(Rc<Cell<bool>>);

impl Cancellation for Flag {
    fn is_cancelled(&self) -> bool {
        self.0.get()
    }
}

#[test]
fn cancellation_is_checked_before_initialization() {
    let mut region = Region::one_pixel();
    let catalog = region.catalog();
    let flag = Rc::new(Cell::new(false));
    let cancel = Flag(Rc::clone(&flag));
    let header = region.header;
    let limits = Limits::default();
    let mut refined = Vec::new();
    let mut bitmap = Vec::new();
    let mut decoder = region.decoder(&mut refined, &cancel);
    let composer = TextComposer::new(
        3,
        header,
        &catalog,
        &mut decoder,
        &[0x80],
        0,
        &[],
        0,
        0,
        &mut bitmap,
        &limits,
        &cancel,
        TextComposeBudget::default(),
    )
    .unwrap();
    flag.set(true);
    let error = composer.compose().unwrap_err();
    assert!(matches!(error.kind, TextComposeErrorKind::Cancelled));
    assert_eq!(error.progress.stage, TextComposeStage::Initialize);
    assert_eq!(error.progress.work_units, 0);
}

#[test]
fn handles_unlike_the_callers_catalog_or_views_fail_before_composition() {
    let symbol = stored(SymbolStore::Imported, descriptor(1, 1, 0));
    for refined_case in [false, true] {
        let (flags, placement) = if refined_case {
            (REFINE, refine(0, 0, 0, vec![vec![true]]))
        } else {
            (flags(false, SymbolCombination::Or), place(0, 0, 0))
        };
        let mut region = Region::new(3, 2, flags, &[symbol], [&[0x80], &[]], &[placement]);
        let mut catalog = region.catalog();
        // The decoder appends refined bitmaps at a base the caller's refined
        // view does not use, or the caller claims the symbol is in the new
        // store.
        let refined_base = u64::from(refined_case);
        if !refined_case {
            catalog[0].store = SymbolStore::New;
        }
        let error = compose_views(
            &mut region,
            &catalog,
            [&[0x80], &[0x80]],
            refined_base,
            TextComposeBudget::default(),
            &NeverCancel,
        )
        .err()
        .expect("expected a refusal");
        assert!(
            matches!(
                error.kind,
                TextComposeErrorKind::Malformed(_) | TextComposeErrorKind::InvalidSpan(_)
            ),
            "{error}"
        );
        assert_eq!(error.progress.completed_instances, 0);
        assert_eq!(error.progress.touched_pixels, 0);
    }
}

#[test]
fn catalog_store_base_must_match_the_caller_view() {
    for store in [SymbolStore::Imported, SymbolStore::New] {
        let symbol = StoredSymbol {
            store,
            store_base: 1,
            symbol: descriptor(1, 1, 0),
        };
        let views = [0x80, 0x80];
        let (imported, new): (&[u8], &[u8]) = match store {
            SymbolStore::Imported => (&views, &[]),
            SymbolStore::New => (&[], &views),
        };
        // The decoder sees the store at base 1, as the catalog says; the
        // composer's views start at zero.
        let mut region = Region::new(
            3,
            2,
            flags(false, SymbolCombination::Or),
            &[symbol],
            [imported, new],
            &[place(0, 0, 0)],
        );
        match store {
            SymbolStore::Imported => region.imported_base = 1,
            SymbolStore::New => region.new_base = 1,
        }
        let error = compose_views(
            &mut region,
            &[symbol],
            [&views, &views],
            0,
            TextComposeBudget::default(),
            &NeverCancel,
        )
        .err()
        .expect("expected a refusal");
        assert!(
            matches!(
                error.kind,
                TextComposeErrorKind::Malformed("bitmap handle store base differs from view")
            ),
            "{error}"
        );
    }
}

#[test]
fn region_and_runtime_budgets_refuse_before_excess_work() {
    let default = TextComposeBudget::default();
    let checks = [
        (
            TextComposeBudget {
                max_region_pixels: 5,
                ..default
            },
            "region pixels",
            TextComposeStage::Preflight,
        ),
        (
            TextComposeBudget {
                max_scratch_bytes: 1,
                ..default
            },
            "scratch bytes",
            TextComposeStage::Preflight,
        ),
        (
            TextComposeBudget {
                max_row_bytes: 0,
                ..default
            },
            "row bytes",
            TextComposeStage::Preflight,
        ),
        (
            TextComposeBudget {
                max_touched_pixels_per_instance: 0,
                ..default
            },
            "per-instance touched pixels",
            TextComposeStage::Instance,
        ),
        (
            TextComposeBudget {
                max_total_touched_pixels: 0,
                ..default
            },
            "total touched pixels",
            TextComposeStage::Instance,
        ),
        (
            TextComposeBudget {
                max_symbol_bytes: 0,
                ..default
            },
            "symbol bytes",
            TextComposeStage::Instance,
        ),
        (
            TextComposeBudget {
                max_work_units: 0,
                ..default
            },
            "composition work",
            TextComposeStage::Initialize,
        ),
        (
            TextComposeBudget {
                max_work_units: 2,
                ..default
            },
            "composition work",
            TextComposeStage::Instance,
        ),
    ];
    for (budget, resource, stage) in checks {
        let error = compose(&mut Region::one_pixel(), budget)
            .err()
            .expect("expected a refusal");
        assert_eq!(error.progress.stage, stage, "{resource}");
        assert!(
            matches!(error.kind, TextComposeErrorKind::LimitExceeded { resource: actual, .. } if actual == resource),
            "{resource}"
        );
        assert_eq!(error.progress.touched_pixels, 0, "{resource}");
    }
}

#[test]
fn fixed_budget_mutations_produce_bounded_success_or_typed_refusal() {
    let symbol = stored(SymbolStore::Imported, descriptor(1, 1, 0));
    let mut state = 0x4b1d_57a9_u32;
    let mut refused = 0;
    let mut completed = 0;
    for _ in 0..96 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        // Mostly off-region placements, and now and then one inside.
        let (x, y) = if state & 0x30 == 0 {
            (i64::from(state >> 8 & 3), i64::from(state >> 10 & 1))
        } else {
            (
                i64::from((state >> 16) as i16),
                i64::from((state >> 1) as i16),
            )
        };
        let mut budget = TextComposeBudget {
            max_region_pixels: 6,
            max_scratch_bytes: 2,
            max_touched_pixels_per_instance: 1,
            max_total_touched_pixels: 1,
            max_work_units: 3,
            max_row_bytes: 1,
            ..TextComposeBudget::default()
        };
        match state & 7 {
            0 => budget.max_region_pixels = 5,
            1 => budget.max_touched_pixels_per_instance = 0,
            2 => budget.max_work_units = 2,
            3 => budget.max_symbol_bytes = 0,
            _ => {}
        }
        let mut region = Region::new(
            3,
            2,
            flags(false, SymbolCombination::Or),
            &[symbol],
            [&[0x80], &[]],
            &[place(0, x, y)],
        );
        match compose(&mut region, budget) {
            Ok(composed) => {
                completed += 1;
                assert_eq!(composed.bitmap.len(), 2);
                assert!(composed.report.progress.work_units <= 3);
            }
            Err(error) => {
                refused += 1;
                assert!(matches!(
                    error.kind,
                    TextComposeErrorKind::Malformed(_)
                        | TextComposeErrorKind::InvalidSpan(_)
                        | TextComposeErrorKind::LimitExceeded { .. }
                ));
                assert!(error.progress.touched_pixels <= 1);
            }
        }
    }
    assert!(completed > 0);
    assert!(refused > 0);
}

#[test]
fn composition_errors_have_stable_messages_and_nested_causes() {
    use crate::jbig2::text_instances::{TextInstanceErrorKind, TextInstanceProgress};
    let instance = TextInstanceError {
        segment: 3,
        offset: 7,
        progress: Box::new(TextInstanceProgress::default()),
        kind: TextInstanceErrorKind::Malformed("fixture"),
    };
    let cases = vec![
        (
            TextComposeErrorKind::InvalidSpan("fixture"),
            "invalid span",
            false,
        ),
        (
            TextComposeErrorKind::Malformed("fixture"),
            "malformed",
            false,
        ),
        (limited("fixture", 1, 2), "limit 1", false),
        (
            TextComposeErrorKind::AllocationFailed,
            "allocation failed",
            false,
        ),
        (TextComposeErrorKind::Cancelled, "cancelled", false),
        (
            TextComposeErrorKind::Instance(Box::new(instance)),
            "instance:",
            true,
        ),
    ];
    for (kind, phrase, has_source) in cases {
        let error = TextComposeError {
            segment: 3,
            offset: 12,
            progress: Box::new(TextComposeProgress::default()),
            kind,
        };
        assert!(error.to_string().contains(phrase));
        assert_eq!(StdError::source(&error).is_some(), has_source);
    }
}

#[test]
fn composition_error_display_propagates_partial_writer_failure() {
    #[derive(Default)]
    struct FailOnSecondWrite(usize);

    impl std::fmt::Write for FailOnSecondWrite {
        fn write_str(&mut self, chunk: &str) -> std::fmt::Result {
            if chunk.is_empty() {
                return Ok(());
            }
            self.0 += 1;
            if self.0 == 2 {
                Err(std::fmt::Error)
            } else {
                Ok(())
            }
        }
    }

    let error = TextComposeError {
        segment: 3,
        offset: 12,
        progress: Box::new(TextComposeProgress::default()),
        kind: TextComposeErrorKind::Cancelled,
    };
    let mut writer = FailOnSecondWrite::default();
    assert!(std::fmt::write(&mut writer, format_args!("{error}")).is_err());
    assert_eq!(writer.0, 2);
}

#[test]
fn constructor_rejects_invalid_header_stream_identity_stores_and_limits() {
    for case in 0..10 {
        let mut region = Region::one_pixel();
        let mut h = region.header;
        let mut limits = Limits::default();
        let mut segment = 3;
        let mut catalog = region.catalog();
        let mut imported_base = 0;
        // Every header change also differs from the decoder's own header.
        match case {
            0 => limits.io_chunk_bytes = 0,
            1 => limits.io_chunk_bytes = crate::MAX_IO_CHUNK + 1,
            2 => segment = 4,
            3 => h.region.width = 0,
            4 => h.flags.huffman = true,
            5 => catalog.clear(),
            6 => imported_base = 2,
            7 => h.flags.refine = true,
            8 => h.region.width = 4,
            _ => h.segment = 4,
        }
        let mut refined = Vec::new();
        let mut bitmap = Vec::new();
        let mut decoder = region.decoder(&mut refined, &NeverCancel);
        let error = TextComposer::new(
            segment,
            h,
            &catalog,
            &mut decoder,
            &[0x80],
            imported_base,
            &[],
            0,
            0,
            &mut bitmap,
            &limits,
            &NeverCancel,
            TextComposeBudget::default(),
        )
        .err()
        .expect("expected preflight refusal");
        assert_eq!(
            error.progress.stage,
            TextComposeStage::Preflight,
            "case {case}"
        );
        assert!(bitmap.is_empty(), "case {case}");
        match case {
            1 => assert!(matches!(
                error.kind,
                TextComposeErrorKind::LimitExceeded { .. }
            )),
            6 => assert!(matches!(error.kind, TextComposeErrorKind::InvalidSpan(_))),
            _ => assert!(matches!(error.kind, TextComposeErrorKind::Malformed(_))),
        }
    }
}

#[test]
fn malformed_source_descriptors_are_refused() {
    let cases = [
        descriptor(0, 1, 0),
        descriptor(1, 0, 0),
        SymbolDescriptor {
            row_stride: 2,
            ..descriptor(1, 1, 0)
        },
        SymbolDescriptor {
            stored_bytes: 2,
            ..descriptor(1, 1, 0)
        },
    ];
    for descriptor in cases {
        let error = compose_views(
            &mut Region::one_pixel(),
            &[stored(SymbolStore::Imported, descriptor)],
            [&[0x80, 0], &[]],
            0,
            TextComposeBudget::default(),
            &NeverCancel,
        )
        .err()
        .expect("expected a refusal");
        assert!(matches!(error.kind, TextComposeErrorKind::Malformed(_)));
        assert_eq!(error.progress.touched_pixels, 0);
    }
}

#[test]
fn an_instance_refusal_stops_composition() {
    let mut region = Region::one_pixel();
    region.budget.max_strips = 0;
    let error = compose(&mut region, TextComposeBudget::default())
        .err()
        .expect("expected a refusal");
    assert!(
        matches!(&error.kind, TextComposeErrorKind::Instance(instance)
        if matches!(instance.kind, crate::jbig2::text_instances::TextInstanceErrorKind::LimitExceeded {
            resource: "text strips",
            ..
        })),
        "{error}"
    );
    assert_eq!(error.offset, region.header.body.offset + 2);
    assert_eq!(error.progress.completed_instances, 0);
    assert_eq!(error.progress.touched_pixels, 0);
}

#[test]
fn symbol_row_cap_is_checked_before_composition() {
    let symbol = stored(SymbolStore::Imported, descriptor(9, 1, 0));
    let mut region = Region::new(
        3,
        2,
        flags(false, SymbolCombination::Or),
        &[symbol],
        [&[0x80, 0x00], &[]],
        &[place(0, 0, 0)],
    );
    let error = compose(
        &mut region,
        TextComposeBudget {
            max_row_bytes: 1,
            ..TextComposeBudget::default()
        },
    )
    .err()
    .expect("expected a refusal");
    assert!(matches!(
        error.kind,
        TextComposeErrorKind::LimitExceeded {
            resource: "row bytes",
            ..
        }
    ));
    assert_eq!(error.progress.touched_pixels, 0);
}
