// SPDX-License-Identifier: MIT

//! Composer tests over the real instance decoder. Each region's MQ body is
//! coded for the standard T.88 states by the test-only encoder from the
//! placements a test asks for.

use super::*;
use crate::NeverCancel;
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
use std::io::Write;
use std::{
    cell::{Cell, RefCell},
    error::Error as StdError,
    io,
    rc::Rc,
};

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

fn event(index: u32, symbol_id: u32, x: i64, y: i64, bitmap: TextBitmap) -> TextInstance {
    let symbol = match bitmap {
        TextBitmap::Stored(stored) => stored.symbol,
        TextBitmap::Refined { symbol, .. } => symbol,
    };
    TextInstance {
        index,
        strip: 0,
        symbol_id,
        ri: matches!(bitmap, TextBitmap::Refined { .. }),
        x,
        y,
        width: symbol.width,
        height: symbol.height,
        bitmap,
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
                source_bytes_fetched: 0,
                synthesized_inputs: 0,
                symbols_decoded: 0,
                work_done: 0,
                poisoned: false,
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
/// own views of the dictionary stores.
struct Region {
    source: Bytes,
    text_segment: SegmentHeader,
    dictionary_segment: SegmentHeader,
    header: TextRegionHeader,
    report: DictionaryReport,
    imported: Bytes,
    imported_base: u64,
    new: Bytes,
    new_base: u64,
    contexts: ContextBank,
    table: MqTable,
    limits: Limits,
    budget: TextInstanceBudget,
    refined_base: u64,
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
            imported: Bytes::new(imported),
            imported_base: 0,
            new: Bytes::new(new),
            new_base: 0,
            contexts,
            table: MqTable::standard(),
            limits,
            budget: TextInstanceBudget::default(),
            refined_base: 0,
        }
    }

    fn parse(
        width: u32,
        height: u32,
        flags: u16,
        instances: u32,
        body: &[u8],
    ) -> (TextRegionHeader, Bytes, SegmentHeader, SegmentHeader) {
        let mut source = Bytes::new(&text_data(width, height, flags, instances, body));
        let text_segment = segment(3, 6, vec![2], 0, source.size());
        let dictionary_segment = segment(2, 0, vec![1], 100, 2);
        let header = read_text_region_header_with_policy(
            &mut source,
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

    /// The real instance decoder, appending refined bitmaps to `temporary`.
    fn decoder<'r, C: Cancellation>(
        &'r mut self,
        temporary: &'r mut Appender,
        cancellation: &'r C,
    ) -> TextInstanceDecoder<'r, Bytes, Bytes, Bytes, Appender, C> {
        TextInstanceDecoder::new_with_header_policy(
            &mut self.source,
            &self.text_segment,
            self.header,
            &self.dictionary_segment,
            &self.report,
            &mut self.imported,
            self.imported_base,
            &mut self.new,
            self.new_base,
            temporary,
            self.refined_base,
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

#[derive(Default)]
struct Bytes {
    data: Rc<RefCell<Vec<u8>>>,
    max_part: usize,
    zero_at_call: Option<usize>,
    over_at_call: Option<usize>,
    shrink_at_call: Option<usize>,
    shrink_after_read_at_call: Option<usize>,
    reported_size: Option<u64>,
    calls: usize,
}

impl Bytes {
    fn new(data: &[u8]) -> Self {
        Self {
            data: Rc::new(RefCell::new(data.to_vec())),
            max_part: usize::MAX,
            ..Self::default()
        }
    }

    /// A sink appending to these bytes, as the decoder's refined store.
    fn appender(&self) -> Appender {
        Appender(Rc::clone(&self.data))
    }
}

impl RangedSource for Bytes {
    fn size(&self) -> u64 {
        self.reported_size
            .unwrap_or(self.data.borrow().len() as u64)
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> crate::Result<usize> {
        self.calls += 1;
        let mut data = self.data.borrow_mut();
        if self.shrink_at_call == Some(self.calls) {
            data.clear();
        }
        if self.zero_at_call == Some(self.calls) {
            return Ok(0);
        }
        if self.over_at_call == Some(self.calls) {
            return Ok(destination.len() + 1);
        }
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        if start >= data.len() {
            return Ok(0);
        }
        let n = destination.len().min(self.max_part).min(data.len() - start);
        destination[..n].copy_from_slice(&data[start..start + n]);
        if self.shrink_after_read_at_call == Some(self.calls) {
            data.clear();
        }
        Ok(n)
    }
}

/// Appends the decoder's refined bitmaps to a [`Bytes`] view.
struct Appender(Rc<RefCell<Vec<u8>>>);

impl Write for Appender {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct Scratch {
    data: Vec<u8>,
    size_calls: Cell<usize>,
    fail_size_at_call: Option<usize>,
    max_part: usize,
    read_calls: usize,
    write_calls: usize,
    flushes: usize,
    fail_set_len: bool,
    incorrect_set_len: bool,
    fail_flush_at: Option<usize>,
    zero_read_at: Option<usize>,
    zero_write_at: Option<usize>,
    over_read_at: Option<usize>,
    over_write_at: Option<usize>,
    shrink_after_write: Option<usize>,
    shrink_after_read: Option<usize>,
    invalid_size_when: Option<Rc<Cell<bool>>>,
    cancel_after_write: Option<(usize, Rc<Cell<bool>>)>,
}

impl Scratch {
    fn new() -> Self {
        Self {
            max_part: usize::MAX,
            ..Self::default()
        }
    }
}

impl RandomAccessScratch for Scratch {
    fn size(&self) -> crate::Result<u64> {
        let call = self.size_calls.get() + 1;
        self.size_calls.set(call);
        if self.fail_size_at_call == Some(call) {
            return Err(Error::Io(io::Error::other("size")));
        }
        if self
            .invalid_size_when
            .as_ref()
            .is_some_and(|flag| flag.get())
        {
            Ok(0)
        } else {
            Ok(self.data.len() as u64)
        }
    }

    fn set_len(&mut self, bytes: u64) -> crate::Result<()> {
        if self.fail_set_len {
            return Err(Error::Io(io::Error::other("set_len")));
        }
        self.data
            .resize(bytes as usize - usize::from(self.incorrect_set_len), 0);
        Ok(())
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> crate::Result<usize> {
        self.read_calls += 1;
        if self.zero_read_at == Some(self.read_calls) {
            return Ok(0);
        }
        if self.over_read_at == Some(self.read_calls) {
            return Ok(destination.len() + 1);
        }
        let start = offset as usize;
        if start >= self.data.len() {
            return Ok(0);
        }
        let n = destination
            .len()
            .min(self.max_part)
            .min(self.data.len() - start);
        destination[..n].copy_from_slice(&self.data[start..start + n]);
        if self.shrink_after_read == Some(self.read_calls) {
            self.data.clear();
        }
        Ok(n)
    }

    fn write_at(&mut self, offset: u64, bytes: &[u8]) -> crate::Result<usize> {
        self.write_calls += 1;
        if self.zero_write_at == Some(self.write_calls) {
            return Ok(0);
        }
        if self.over_write_at == Some(self.write_calls) {
            return Ok(bytes.len() + 1);
        }
        let start = offset as usize;
        if start >= self.data.len() {
            return Ok(0);
        }
        let n = bytes.len().min(self.max_part).min(self.data.len() - start);
        self.data[start..start + n].copy_from_slice(&bytes[..n]);
        if self.shrink_after_write == Some(self.write_calls) {
            self.data.clear();
        }
        if let Some((call, flag)) = &self.cancel_after_write
            && *call == self.write_calls
        {
            flag.set(true);
        }
        Ok(n)
    }

    fn flush(&mut self) -> crate::Result<()> {
        self.flushes += 1;
        if self.fail_flush_at == Some(self.flushes) {
            Err(Error::Io(io::Error::other("scratch flush")))
        } else {
            Ok(())
        }
    }
}

#[derive(Default)]
struct Sink {
    data: Vec<u8>,
    max_part: usize,
    calls: usize,
    flushes: usize,
    zero_at_call: Option<usize>,
    over_at_call: Option<usize>,
    fail_flush: bool,
    change_size_on_flush: Option<Rc<Cell<bool>>>,
}

impl Sink {
    fn new() -> Self {
        Self {
            max_part: usize::MAX,
            ..Self::default()
        }
    }
}

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.calls += 1;
        if self.zero_at_call == Some(self.calls) {
            return Ok(0);
        }
        if self.over_at_call == Some(self.calls) {
            return Ok(bytes.len() + 1);
        }
        let n = bytes.len().min(self.max_part);
        self.data.extend_from_slice(&bytes[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.flushes += 1;
        if let Some(flag) = &self.change_size_on_flush {
            flag.set(true);
        }
        if self.fail_flush {
            Err(io::Error::other("output flush"))
        } else {
            Ok(())
        }
    }
}

/// Decode `region` with the real instance decoder and compose it through the
/// given composer views. The refined view is the store the decoder appends
/// refined bitmaps to.
fn compose(
    region: &mut Region,
    imported: &mut Bytes,
    new: &mut Bytes,
    refined: &mut Bytes,
    scratch: &mut Scratch,
    output: &mut Sink,
    budget: TextComposeBudget,
) -> TextComposeResult<TextComposeReport> {
    let catalog = region.catalog();
    let header = region.header;
    let limits = Limits::default();
    let mut temporary = refined.appender();
    let mut decoder = region.decoder(&mut temporary, &NeverCancel);
    let mut composer = TextComposer::new(
        3,
        header,
        &catalog,
        &mut decoder,
        imported,
        0,
        new,
        0,
        refined,
        0,
        scratch,
        output,
        &limits,
        &NeverCancel,
        budget,
    )?;
    composer.compose()
}

fn one_pixel_run(
    imported: &mut Bytes,
    scratch: &mut Scratch,
    output: &mut Sink,
    budget: TextComposeBudget,
) -> TextComposeResult<TextComposeReport> {
    compose(
        &mut Region::one_pixel(),
        imported,
        &mut Bytes::new(&[]),
        &mut Bytes::new(&[]),
        scratch,
        output,
        budget,
    )
}

#[test]
fn completed_report_retains_explicit_header_anomaly() {
    let symbol = stored(SymbolStore::Imported, descriptor(1, 1, 0));
    // SBRTEMPLATE without SBREFINE, eight strips, and SBDSOFFSET 9.
    let mut region = Region::new(1, 1, 0xa40c, &[symbol], [&[0x80], &[]], &[place(0, 0, 0)]);
    assert_eq!(region.header.flags.log_strips, 3);
    assert_eq!(region.header.flags.ds_offset, 9);
    let report = compose(
        &mut region,
        &mut Bytes::new(&[0x80]),
        &mut Bytes::new(&[]),
        &mut Bytes::new(&[]),
        &mut Scratch::new(),
        &mut Sink::new(),
        TextComposeBudget::default(),
    )
    .unwrap();
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
    let mut imported = Bytes::new(&imported_rows);
    let mut new = Bytes::new(&new_rows);
    let mut refined = Bytes::new(&[]);
    let mut scratch = Scratch::new();
    let mut output = Sink::new();
    let report = compose(
        &mut region,
        &mut imported,
        &mut new,
        &mut refined,
        &mut scratch,
        &mut output,
        TextComposeBudget::default(),
    )
    .unwrap();
    // The decoder appended the refined 1x2 bitmap the composer then read.
    assert_eq!(*refined.data.borrow(), [0x80, 0x80]);
    assert_eq!(output.data, [0xd0, 0xb8, 0x18]);
    assert_eq!(scratch.data, output.data);
    assert_eq!(
        (
            report.progress.completed_instances,
            report.progress.output_rows,
            report.progress.touched_pixels
        ),
        (4, 3, 12)
    );
    assert_eq!(
        (report.packed_bytes, report.progress.output_bytes_written),
        (3, 3)
    );
    assert_eq!(report.progress.stage, TextComposeStage::Complete);
    assert!(!report.progress.poisoned);
    assert_eq!(report.progress.source_bytes_read, 6);
    assert_eq!(scratch.flushes, 2);
    assert_eq!(output.flushes, 1);
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
    let mut output = Sink::new();
    let report = compose(
        &mut region,
        &mut Bytes::new(&rows),
        &mut Bytes::new(&[]),
        &mut Bytes::new(&[]),
        &mut Scratch::new(),
        &mut output,
        TextComposeBudget::default(),
    )
    .unwrap();
    assert_eq!(output.data, [0x80, 0x00]);
    assert_eq!(report.progress.touched_pixels, 2);
    assert_eq!(report.progress.source_bytes_read, 1);
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
        let mut output = Sink::new();
        compose(
            &mut region,
            &mut Bytes::new(&rows),
            &mut Bytes::new(&[]),
            &mut Bytes::new(&[]),
            &mut Scratch::new(),
            &mut output,
            TextComposeBudget::default(),
        )
        .unwrap();
        assert_eq!(output.data, [expected], "{operator:?}");
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
        let mut scratch = Scratch::new();
        let mut output = Sink::new();
        let report = compose(
            &mut region,
            &mut Bytes::new(&[]),
            &mut Bytes::new(&[]),
            &mut Bytes::new(&[]),
            &mut scratch,
            &mut output,
            TextComposeBudget::default(),
        )
        .unwrap();
        assert_eq!(output.data, expected);
        assert_eq!(scratch.data, expected);
        assert_eq!(report.progress.completed_instances, 0);
        assert_eq!(report.progress.output_rows, 2);
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
            let mut output = Sink::new();
            compose(
                &mut region,
                &mut Bytes::new(&[0xc0]),
                &mut Bytes::new(&[]),
                &mut Bytes::new(&[]),
                &mut Scratch::new(),
                &mut output,
                TextComposeBudget::default(),
            )
            .unwrap();
            assert_eq!(
                output.data,
                [0, 0, 0x60],
                "{corner:?}, transposed={transposed}"
            );
        }
    }
}

#[test]
fn short_io_is_counted_and_request_sizes_stay_bounded() {
    let symbol = stored(SymbolStore::Imported, descriptor(10, 2, 0));
    let rows = [0xff, 0xc0, 0x80, 0x00];
    let mut region = Region::new(
        10,
        2,
        flags(false, SymbolCombination::Or),
        &[symbol],
        [&rows, &[]],
        &[place(0, 0, 0)],
    );
    let mut imported = Bytes::new(&rows);
    imported.max_part = 1;
    let mut scratch = Scratch::new();
    scratch.max_part = 1;
    let mut output = Sink::new();
    output.max_part = 1;
    let budget = TextComposeBudget {
        max_request_bytes: 1,
        ..TextComposeBudget::default()
    };
    let report = compose(
        &mut region,
        &mut imported,
        &mut Bytes::new(&[]),
        &mut Bytes::new(&[]),
        &mut scratch,
        &mut output,
        budget,
    )
    .unwrap();
    assert_eq!(output.data, [0xff, 0xc0, 0x80, 0x00]);
    assert_eq!(
        (
            report.progress.source_bytes_read,
            report.progress.scratch_bytes_read,
            report.progress.scratch_bytes_written,
            report.progress.output_bytes_written
        ),
        (4, 8, 8, 4)
    );
    assert_eq!(report.progress.max_request_bytes, 1);
    assert!(report.progress.peak_resident_bytes <= budget.max_resident_bytes);
    assert_eq!(report.progress.output_rows, 2);
}

#[test]
fn zero_and_overreported_source_io_are_located_and_poisoned() {
    for overreported in [false, true] {
        let mut imported = Bytes::new(&[0x80]);
        if overreported {
            imported.over_at_call = Some(1);
        } else {
            imported.zero_at_call = Some(1);
        }
        let mut scratch = Scratch::new();
        let mut output = Sink::new();
        let error = one_pixel_run(
            &mut imported,
            &mut scratch,
            &mut output,
            TextComposeBudget::default(),
        )
        .unwrap_err();
        assert_eq!(error.progress.stage, TextComposeStage::Instance);
        assert!(error.progress.poisoned);
        assert_eq!(error.progress.completed_instances, 0);
        assert_eq!(error.offset, 0);
        if overreported {
            assert!(matches!(
                error.kind,
                TextComposeErrorKind::Malformed("symbol source overreported read")
            ));
        } else {
            assert!(matches!(
                error.kind,
                TextComposeErrorKind::Source {
                    store: BitmapStore::Imported,
                    error: Error::TruncatedInput { .. }
                }
            ));
        }
    }
}

#[test]
fn scratch_initialization_rmw_and_readback_failures_preserve_physical_progress() {
    for phase in 0..8 {
        let mut imported = Bytes::new(&[0x80]);
        let mut scratch = Scratch::new();
        let mut output = Sink::new();
        match phase {
            0 => scratch.fail_set_len = true,
            1 => scratch.incorrect_set_len = true,
            2 => scratch.zero_write_at = Some(1),
            3 => scratch.over_write_at = Some(1),
            4 => scratch.zero_read_at = Some(1),
            5 => scratch.zero_write_at = Some(3),
            6 => scratch.zero_read_at = Some(2),
            _ => scratch.over_read_at = Some(2),
        }
        let error = one_pixel_run(
            &mut imported,
            &mut scratch,
            &mut output,
            TextComposeBudget::default(),
        )
        .unwrap_err();
        assert!(error.progress.poisoned, "phase {phase}");
        assert_eq!(error.progress.output_bytes_written, 0, "phase {phase}");
        assert!(
            matches!(
                error.kind,
                TextComposeErrorKind::Scratch(_) | TextComposeErrorKind::Malformed(_)
            ),
            "phase {phase}"
        );
        match phase {
            0..=3 => assert_eq!(error.progress.stage, TextComposeStage::Initialize),
            4..=5 => assert_eq!(error.progress.stage, TextComposeStage::Instance),
            _ => assert_eq!(error.progress.stage, TextComposeStage::Readback),
        }
    }
}

#[test]
fn output_partial_failure_and_flush_failure_are_poisoned() {
    for failure in 0..3 {
        let mut imported = Bytes::new(&[0x80]);
        let mut scratch = Scratch::new();
        let mut output = Sink::new();
        output.max_part = 1;
        match failure {
            0 => output.zero_at_call = Some(2),
            1 => output.over_at_call = Some(2),
            _ => output.fail_flush = true,
        }
        let error = one_pixel_run(
            &mut imported,
            &mut scratch,
            &mut output,
            TextComposeBudget::default(),
        )
        .unwrap_err();
        assert!(error.progress.poisoned);
        assert_eq!(
            error.progress.output_bytes_written,
            if failure == 2 { 2 } else { 1 }
        );
        assert_eq!(output.data.len(), if failure == 2 { 2 } else { 1 });
        if failure == 2 {
            assert_eq!(error.progress.stage, TextComposeStage::OutputFlush);
        } else {
            assert_eq!(error.progress.stage, TextComposeStage::Readback);
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
fn cancellation_after_partial_initialization_poisoned_without_final_output() {
    let mut region = Region::one_pixel();
    let catalog = region.catalog();
    let header = region.header;
    let mut imported = Bytes::new(&[0x80]);
    let mut new = Bytes::new(&[]);
    let mut refined = Bytes::new(&[]);
    let flag = Rc::new(Cell::new(false));
    let cancel = Flag(Rc::clone(&flag));
    let mut scratch = Scratch::new();
    scratch.cancel_after_write = Some((1, Rc::clone(&flag)));
    let mut output = Sink::new();
    let limits = Limits::default();
    let mut temporary = refined.appender();
    let mut decoder = region.decoder(&mut temporary, &cancel);
    let mut composer = TextComposer::new(
        3,
        header,
        &catalog,
        &mut decoder,
        &mut imported,
        0,
        &mut new,
        0,
        &mut refined,
        0,
        &mut scratch,
        &mut output,
        &limits,
        &cancel,
        TextComposeBudget::default(),
    )
    .unwrap();
    let error = composer.compose().unwrap_err();
    assert!(matches!(error.kind, TextComposeErrorKind::Cancelled));
    assert_eq!(error.progress.scratch_bytes_written, 1);
    assert_eq!(error.progress.output_bytes_written, 0);
    assert!(matches!(
        composer.compose().unwrap_err().kind,
        TextComposeErrorKind::Poisoned
    ));
}

/// Compose `region` through views whose catalog differs from the decoder's.
fn compose_with_catalog(
    region: &mut Region,
    catalog: &[StoredSymbol],
    imported: &mut Bytes,
    new: &mut Bytes,
    refined: &mut Bytes,
    output: &mut Sink,
) -> TextComposeResult<TextComposeReport> {
    let header = region.header;
    let limits = Limits::default();
    let mut temporary = refined.appender();
    let mut decoder = region.decoder(&mut temporary, &NeverCancel);
    let mut scratch = Scratch::new();
    let mut composer = TextComposer::new(
        3,
        header,
        catalog,
        &mut decoder,
        imported,
        0,
        new,
        0,
        refined,
        0,
        &mut scratch,
        output,
        &limits,
        &NeverCancel,
        TextComposeBudget::default(),
    )?;
    composer.compose()
}

#[test]
fn handles_unlike_the_callers_catalog_or_views_fail_before_any_symbol_read() {
    let symbol = stored(SymbolStore::Imported, descriptor(1, 1, 0));
    for refined_case in [false, true] {
        let (flags, placement) = if refined_case {
            (REFINE, refine(0, 0, 0, vec![vec![true]]))
        } else {
            (flags(false, SymbolCombination::Or), place(0, 0, 0))
        };
        let mut region = Region::new(3, 2, flags, &[symbol], [&[0x80], &[]], &[placement]);
        let mut catalog = region.catalog();
        if refined_case {
            // The decoder appends refined bitmaps at a base the caller's
            // refined view does not use.
            region.refined_base = 1;
        } else {
            // The caller claims the symbol is in the new store.
            catalog[0].store = SymbolStore::New;
        }
        let mut imported = Bytes::new(&[0x80]);
        let mut new = Bytes::new(&[0x80]);
        let mut refined = Bytes::new(&[0]);
        let mut output = Sink::new();
        let error = compose_with_catalog(
            &mut region,
            &catalog,
            &mut imported,
            &mut new,
            &mut refined,
            &mut output,
        )
        .unwrap_err();
        assert!(
            matches!(
                error.kind,
                TextComposeErrorKind::Malformed(_) | TextComposeErrorKind::InvalidSpan(_)
            ),
            "{error}"
        );
        assert_eq!(imported.calls + new.calls + refined.calls, 0);
        assert_eq!(error.progress.completed_instances, 0);
        assert!(output.data.is_empty());
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
        let mut imported = Bytes::new(&views);
        let mut new = Bytes::new(&views);
        let error = compose_with_catalog(
            &mut region,
            &[symbol],
            &mut imported,
            &mut new,
            &mut Bytes::new(&[]),
            &mut Sink::new(),
        )
        .unwrap_err();
        assert!(
            matches!(
                error.kind,
                TextComposeErrorKind::Malformed("bitmap handle store base differs from view")
            ),
            "{error}"
        );
        assert_eq!(imported.calls + new.calls, 0);
    }
}

#[test]
fn region_and_runtime_budgets_refuse_before_excess_work() {
    let mut checks: Vec<(TextComposeBudget, &str, TextComposeStage)> = Vec::new();
    let default = TextComposeBudget::default();
    checks.push((
        TextComposeBudget {
            max_region_pixels: 5,
            ..default
        },
        "region pixels",
        TextComposeStage::Preflight,
    ));
    checks.push((
        TextComposeBudget {
            max_scratch_bytes: 1,
            ..default
        },
        "scratch bytes",
        TextComposeStage::Preflight,
    ));
    checks.push((
        TextComposeBudget {
            max_output_bytes: 1,
            ..default
        },
        "final output bytes",
        TextComposeStage::Preflight,
    ));
    checks.push((
        TextComposeBudget {
            max_row_bytes: 0,
            ..default
        },
        "row bytes",
        TextComposeStage::Preflight,
    ));
    checks.push((
        TextComposeBudget {
            max_touched_pixels_per_instance: 0,
            ..default
        },
        "per-instance touched pixels",
        TextComposeStage::Instance,
    ));
    checks.push((
        TextComposeBudget {
            max_source_read_bytes: 0,
            ..default
        },
        "source read bytes",
        TextComposeStage::Instance,
    ));
    checks.push((
        TextComposeBudget {
            max_scratch_read_bytes: 0,
            ..default
        },
        "scratch read bytes",
        TextComposeStage::Instance,
    ));
    checks.push((
        TextComposeBudget {
            max_scratch_write_bytes: 1,
            ..default
        },
        "scratch write bytes",
        TextComposeStage::Initialize,
    ));
    checks.push((
        TextComposeBudget {
            max_work_units: 2,
            ..default
        },
        "composition work",
        TextComposeStage::Instance,
    ));
    checks.push((
        TextComposeBudget {
            max_resident_bytes: 1,
            ..default
        },
        "resident row bytes",
        TextComposeStage::Instance,
    ));
    for (budget, resource, stage) in [
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
                max_source_read_calls: 0,
                ..default
            },
            "source read calls",
            TextComposeStage::Instance,
        ),
        (
            TextComposeBudget {
                max_scratch_read_calls: 0,
                ..default
            },
            "scratch read calls",
            TextComposeStage::Instance,
        ),
        (
            TextComposeBudget {
                max_scratch_write_calls: 0,
                ..default
            },
            "scratch write calls",
            TextComposeStage::Initialize,
        ),
        (
            TextComposeBudget {
                max_output_write_calls: 0,
                ..default
            },
            "output write calls",
            TextComposeStage::Readback,
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
                max_work_units: 3,
                ..default
            },
            "composition work",
            TextComposeStage::Readback,
        ),
    ] {
        checks.push((budget, resource, stage));
    }
    for (budget, resource, stage) in checks {
        let mut imported = Bytes::new(&[0x80]);
        let mut scratch = Scratch::new();
        let mut output = Sink::new();
        let error = one_pixel_run(&mut imported, &mut scratch, &mut output, budget).unwrap_err();
        assert_eq!(error.progress.stage, stage, "{resource}");
        assert!(
            matches!(error.kind, TextComposeErrorKind::LimitExceeded { resource: actual, .. } if actual == resource),
            "{resource}"
        );
        assert!(error.progress.scratch_bytes_written <= 3);
        assert!(error.progress.source_bytes_read <= 1);
        assert!(error.progress.output_bytes_written == 0);
    }
}

#[test]
fn changing_source_or_scratch_size_is_refused() {
    let mut imported = Bytes::new(&[0x80]);
    imported.shrink_after_read_at_call = Some(1);
    let mut scratch = Scratch::new();
    let mut output = Sink::new();
    let error = one_pixel_run(
        &mut imported,
        &mut scratch,
        &mut output,
        TextComposeBudget::default(),
    )
    .unwrap_err();
    assert!(matches!(
        error.kind,
        TextComposeErrorKind::StoreMutation {
            store: BitmapStore::Imported,
            reason: "size changed"
        }
    ));
    assert_eq!(error.progress.source_bytes_read, 1);
    assert!(error.progress.poisoned);

    let mut imported = Bytes::new(&[0x80]);
    let mut scratch = Scratch::new();
    scratch.shrink_after_write = Some(1);
    let mut output = Sink::new();
    let error = one_pixel_run(
        &mut imported,
        &mut scratch,
        &mut output,
        TextComposeBudget::default(),
    )
    .unwrap_err();
    assert!(matches!(
        error.kind,
        TextComposeErrorKind::Malformed("scratch size changed")
    ));
    assert_eq!(error.progress.scratch_bytes_written, 1);
    assert!(error.progress.poisoned);
}

#[test]
fn scratch_flush_failures_before_composition_and_readback_are_located() {
    for at in [1, 2] {
        let mut imported = Bytes::new(&[0x80]);
        let mut scratch = Scratch::new();
        scratch.fail_flush_at = Some(at);
        let mut output = Sink::new();
        let error = one_pixel_run(
            &mut imported,
            &mut scratch,
            &mut output,
            TextComposeBudget::default(),
        )
        .unwrap_err();
        assert!(matches!(
            error.kind,
            TextComposeErrorKind::Scratch(Error::Io(_))
        ));
        assert_eq!(
            error.progress.stage,
            if at == 1 {
                TextComposeStage::Initialize
            } else {
                TextComposeStage::Instance
            }
        );
        assert_eq!(
            error.progress.completed_instances,
            if at == 1 { 0 } else { 1 }
        );
        assert_eq!(error.progress.output_bytes_written, 0);
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
            max_source_read_bytes: 1,
            max_scratch_read_bytes: 3,
            max_scratch_write_bytes: 3,
            max_output_bytes: 2,
            max_work_units: 5,
            max_row_bytes: 1,
            max_request_bytes: 1,
            max_resident_bytes: 2,
            ..TextComposeBudget::default()
        };
        match state & 7 {
            0 => budget.max_region_pixels = 5,
            1 => budget.max_touched_pixels_per_instance = 0,
            2 => budget.max_work_units = 2,
            3 => budget.max_scratch_write_bytes = 1,
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
        match compose(
            &mut region,
            &mut Bytes::new(&[0x80]),
            &mut Bytes::new(&[]),
            &mut Bytes::new(&[]),
            &mut Scratch::new(),
            &mut Sink::new(),
            budget,
        ) {
            Ok(report) => {
                completed += 1;
                assert_eq!(report.progress.output_bytes_written, 2);
            }
            Err(error) => {
                refused += 1;
                assert!(matches!(
                    error.kind,
                    TextComposeErrorKind::Malformed(_)
                        | TextComposeErrorKind::InvalidSpan(_)
                        | TextComposeErrorKind::LimitExceeded { .. }
                ));
                assert!(error.progress.source_bytes_read <= 1);
                assert!(error.progress.scratch_bytes_read <= 3);
                assert!(error.progress.scratch_bytes_written <= 3);
                assert!(error.progress.output_bytes_written <= 2);
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
        (
            TextComposeErrorKind::Source {
                store: BitmapStore::New,
                error: Error::Io(io::Error::other("source")),
            },
            "New symbol source",
            true,
        ),
        (
            TextComposeErrorKind::Scratch(Error::Io(io::Error::other("scratch"))),
            "scratch:",
            true,
        ),
        (
            TextComposeErrorKind::Output(Error::Io(io::Error::other("output"))),
            "output:",
            true,
        ),
        (TextComposeErrorKind::Poisoned, "poisoned", false),
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
fn short_reader_mutation_and_final_scratch_mutation_are_rejected() {
    let mut imported = Bytes::new(&[0x80]);
    let mut scratch = Scratch::new();
    scratch.shrink_after_read = Some(1);
    let mut output = Sink::new();
    let error = one_pixel_run(
        &mut imported,
        &mut scratch,
        &mut output,
        TextComposeBudget::default(),
    )
    .unwrap_err();
    assert!(matches!(
        error.kind,
        TextComposeErrorKind::Malformed("scratch size changed")
    ));
    assert_eq!(error.progress.scratch_bytes_read, 1);

    let mut imported = Bytes::new(&[0x80]);
    let mut scratch = Scratch::new();
    let flag = Rc::new(Cell::new(false));
    scratch.invalid_size_when = Some(Rc::clone(&flag));
    let mut output = Sink::new();
    output.change_size_on_flush = Some(flag);
    let error = one_pixel_run(
        &mut imported,
        &mut scratch,
        &mut output,
        TextComposeBudget::default(),
    )
    .unwrap_err();
    assert!(matches!(
        error.kind,
        TextComposeErrorKind::Malformed("scratch size changed")
    ));
    assert_eq!(error.progress.stage, TextComposeStage::OutputFlush);
    assert_eq!(error.progress.output_bytes_written, 2);
}

#[test]
fn scratch_size_errors_are_typed_and_located() {
    let mut imported = Bytes::new(&[0x80]);
    let mut scratch = Scratch::new();
    scratch.fail_size_at_call = Some(1);
    let mut output = Sink::new();
    let error = one_pixel_run(
        &mut imported,
        &mut scratch,
        &mut output,
        TextComposeBudget::default(),
    )
    .unwrap_err();
    assert!(matches!(
        error.kind,
        TextComposeErrorKind::Scratch(Error::Io(_))
    ));
    assert_eq!(error.progress.stage, TextComposeStage::Preflight);

    let mut imported = Bytes::new(&[0x80]);
    let mut scratch = Scratch::new();
    scratch.fail_size_at_call = Some(2);
    let mut output = Sink::new();
    let error = one_pixel_run(
        &mut imported,
        &mut scratch,
        &mut output,
        TextComposeBudget::default(),
    )
    .unwrap_err();
    assert!(matches!(
        error.kind,
        TextComposeErrorKind::Scratch(Error::Io(_))
    ));
    assert_eq!(error.progress.stage, TextComposeStage::Initialize);
    assert_eq!(error.offset, 0);
    assert!(error.progress.poisoned);
}

#[test]
fn constructor_rejects_invalid_header_stream_identity_stores_and_limits() {
    for case in 0..13 {
        let mut region = Region::one_pixel();
        let mut h = region.header;
        let mut imported = Bytes::new(&[0x80]);
        let mut new = Bytes::new(&[]);
        let mut refined = Bytes::new(&[]);
        let mut scratch = Scratch::new();
        let mut output = Sink::new();
        let mut limits = Limits::default();
        let mut budget = TextComposeBudget::default();
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
            6 => budget.max_request_bytes = 0,
            7 => budget.max_request_bytes = limits.io_chunk_bytes + 1,
            8 => imported_base = 2,
            9 => scratch.data.push(0),
            10 => h.flags.refine = true,
            11 => h.region.width = 4,
            _ => h.segment = 4,
        }
        let mut temporary = refined.appender();
        let mut decoder = region.decoder(&mut temporary, &NeverCancel);
        let error = TextComposer::new(
            segment,
            h,
            &catalog,
            &mut decoder,
            &mut imported,
            imported_base,
            &mut new,
            0,
            &mut refined,
            0,
            &mut scratch,
            &mut output,
            &limits,
            &NeverCancel,
            budget,
        )
        .err()
        .expect("expected preflight refusal");
        assert_eq!(
            error.progress.stage,
            TextComposeStage::Preflight,
            "case {case}"
        );
        assert_eq!(error.progress.scratch_bytes_written, 0, "case {case}");
        assert_eq!(error.progress.output_bytes_written, 0, "case {case}");
        match case {
            1 => assert!(matches!(
                error.kind,
                TextComposeErrorKind::LimitExceeded { .. }
            )),
            8 => assert!(matches!(error.kind, TextComposeErrorKind::InvalidSpan(_))),
            _ => assert!(matches!(error.kind, TextComposeErrorKind::Malformed(_))),
        }
    }
}

#[test]
fn malformed_source_descriptors_are_refused_before_io() {
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
        let mut region = Region::one_pixel();
        let mut imported = Bytes::new(&[0x80, 0]);
        let error = compose_with_catalog(
            &mut region,
            &[stored(SymbolStore::Imported, descriptor)],
            &mut imported,
            &mut Bytes::new(&[]),
            &mut Bytes::new(&[]),
            &mut Sink::new(),
        )
        .unwrap_err();
        assert!(matches!(error.kind, TextComposeErrorKind::Malformed(_)));
        assert_eq!(imported.calls, 0);
    }
}

#[test]
fn internal_io_error_mapping_and_pre_io_size_guards_are_located() {
    let mut region = Region::one_pixel();
    let catalog = region.catalog();
    let header = region.header;
    let symbol = catalog[0];
    let mut imported = Bytes::new(&[0x80]);
    let mut new = Bytes::new(&[]);
    let mut refined = Bytes::new(&[]);
    let mut scratch = Scratch::new();
    let mut output = Sink::new();
    let limits = Limits::default();
    let mut temporary = refined.appender();
    let mut decoder = region.decoder(&mut temporary, &NeverCancel);
    let mut composer = TextComposer::new(
        3,
        header,
        &catalog,
        &mut decoder,
        &mut imported,
        0,
        &mut new,
        0,
        &mut refined,
        0,
        &mut scratch,
        &mut output,
        &limits,
        &NeverCancel,
        TextComposeBudget::default(),
    )
    .unwrap();
    assert!(matches!(
        composer.scratch_error(2, Error::Cancelled).kind,
        TextComposeErrorKind::Cancelled
    ));
    assert!(matches!(
        composer.output_error(2, Error::Cancelled).kind,
        TextComposeErrorKind::Cancelled
    ));
    assert!(matches!(
        composer
            .source_error(2, BitmapStore::Imported, Error::Cancelled)
            .kind,
        TextComposeErrorKind::Cancelled
    ));
    assert!(matches!(
        composer
            .source_error(2, BitmapStore::Imported, Error::Io(io::Error::other("I/O")))
            .kind,
        TextComposeErrorKind::Source {
            store: BitmapStore::Imported,
            error: Error::Io(_)
        }
    ));
    assert!(matches!(
        composer.scratch_write(0, &[0]).unwrap_err().kind,
        TextComposeErrorKind::Malformed("scratch size changed")
    ));
    assert!(matches!(
        composer.scratch_read(0, &mut [0]).unwrap_err().kind,
        TextComposeErrorKind::Malformed("scratch size changed")
    ));
    composer.imported.data.borrow_mut().clear();
    assert!(matches!(
        composer
            .checked_event(event(0, 0, 0, 0, TextBitmap::Stored(symbol)))
            .unwrap_err()
            .kind,
        TextComposeErrorKind::StoreMutation {
            store: BitmapStore::Imported,
            reason: "size changed"
        }
    ));
    assert!(matches!(
        composer
            .source_read(BitmapStore::Imported, 1, 0, &mut [0])
            .unwrap_err()
            .kind,
        TextComposeErrorKind::StoreMutation {
            store: BitmapStore::Imported,
            reason: "size changed"
        }
    ));
}

#[test]
fn refined_append_rules_are_checked_between_events() {
    let mut region = Region::one_pixel();
    let catalog = region.catalog();
    let header = region.header;
    let symbol = catalog[0];
    let limits = Limits::default();
    let mut imported = Bytes::new(&[0x80]);
    let mut new = Bytes::new(&[]);
    let mut refined = Bytes::new(&[]);
    let mut scratch = Scratch::new();
    let mut output = Sink::new();
    let mut temporary = refined.appender();
    let mut decoder = region.decoder(&mut temporary, &NeverCancel);
    let mut composer = TextComposer::new(
        3,
        header,
        &catalog,
        &mut decoder,
        &mut imported,
        0,
        &mut new,
        0,
        &mut refined,
        0,
        &mut scratch,
        &mut output,
        &limits,
        &NeverCancel,
        TextComposeBudget::default(),
    )
    .unwrap();
    let unrefined_event = event(0, 0, 0, 0, TextBitmap::Stored(symbol));
    let refined_event = event(
        0,
        0,
        0,
        0,
        TextBitmap::Refined {
            store_base: 0,
            symbol: descriptor(1, 1, 0),
        },
    );
    composer.refined.data.borrow_mut().push(0x80);
    for event in [None, Some(&unrefined_event)] {
        let error = composer.check_views_after_next(event).unwrap_err();
        assert!(matches!(
            error.kind,
            TextComposeErrorKind::StoreMutation {
                store: BitmapStore::Refined,
                reason: "append without refined instance"
            }
        ));
    }
    composer
        .check_views_after_next(Some(&refined_event))
        .unwrap();
    assert_eq!(composer.refined_size, 1);

    composer.refined.data.borrow_mut().clear();
    let error = composer
        .check_views_after_next(Some(&refined_event))
        .unwrap_err();
    assert!(matches!(
        error.kind,
        TextComposeErrorKind::StoreMutation {
            store: BitmapStore::Refined,
            reason: "shrank"
        }
    ));
}

#[test]
fn fixed_bitmap_view_resize_between_events_is_rejected() {
    let limits = Limits::default();
    for store in [BitmapStore::Imported, BitmapStore::New] {
        let mut region = Region::one_pixel();
        let catalog = region.catalog();
        let header = region.header;
        let mut imported = Bytes::new(&[0x80]);
        let mut new = Bytes::new(&[0x80]);
        let mut refined = Bytes::new(&[]);
        let mut scratch = Scratch::new();
        let mut output = Sink::new();
        let mut temporary = refined.appender();
        let mut decoder = region.decoder(&mut temporary, &NeverCancel);
        let mut composer = TextComposer::new(
            3,
            header,
            &catalog,
            &mut decoder,
            &mut imported,
            0,
            &mut new,
            0,
            &mut refined,
            0,
            &mut scratch,
            &mut output,
            &limits,
            &NeverCancel,
            TextComposeBudget::default(),
        )
        .unwrap();
        let view = match store {
            BitmapStore::Imported => &mut composer.imported,
            BitmapStore::New => &mut composer.new,
            BitmapStore::Refined => unreachable!(),
        };
        view.data.borrow_mut().push(0x80);
        let error = composer.check_views_after_next(None).unwrap_err();
        assert!(matches!(
            error.kind,
            TextComposeErrorKind::StoreMutation {
                store: actual,
                reason: "size changed"
            } if actual == store
        ));
        assert_eq!(error.offset, 0);
    }
}

#[test]
fn an_instance_refusal_stops_before_output() {
    let mut region = Region::one_pixel();
    region.budget.max_strips = 0;
    let mut output = Sink::new();
    let error = compose(
        &mut region,
        &mut Bytes::new(&[0x80]),
        &mut Bytes::new(&[]),
        &mut Bytes::new(&[]),
        &mut Scratch::new(),
        &mut output,
        TextComposeBudget::default(),
    )
    .unwrap_err();
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
    assert_eq!(error.progress.output_bytes_written, 0);
    assert!(error.progress.poisoned);
    assert!(output.data.is_empty());
}

#[test]
fn source_row_cap_is_checked_before_reading() {
    let symbol = stored(SymbolStore::Imported, descriptor(9, 1, 0));
    let mut region = Region::new(
        3,
        2,
        flags(false, SymbolCombination::Or),
        &[symbol],
        [&[0x80, 0x00], &[]],
        &[place(0, 0, 0)],
    );
    let mut imported = Bytes::new(&[0x80, 0x00]);
    let error = compose(
        &mut region,
        &mut imported,
        &mut Bytes::new(&[]),
        &mut Bytes::new(&[]),
        &mut Scratch::new(),
        &mut Sink::new(),
        TextComposeBudget {
            max_row_bytes: 1,
            ..TextComposeBudget::default()
        },
    )
    .unwrap_err();
    assert!(matches!(
        error.kind,
        TextComposeErrorKind::LimitExceeded {
            resource: "row bytes",
            ..
        }
    ));
    assert_eq!(error.progress.source_bytes_read, 0);
    assert_eq!(imported.calls, 0);
}

#[test]
fn resident_capacity_and_output_accounting_are_checked_after_preflight() {
    let mut region = Region::one_pixel();
    let catalog = region.catalog();
    let header = region.header;
    let mut imported = Bytes::new(&[0x80]);
    let mut new = Bytes::new(&[]);
    let mut refined = Bytes::new(&[]);
    let mut scratch = Scratch::new();
    let mut output = Sink::new();
    let limits = Limits::default();
    let budget = TextComposeBudget {
        max_resident_bytes: 1,
        max_output_bytes: 2,
        ..TextComposeBudget::default()
    };
    let mut temporary = refined.appender();
    let mut decoder = region.decoder(&mut temporary, &NeverCancel);
    let mut composer = TextComposer::new(
        3,
        header,
        &catalog,
        &mut decoder,
        &mut imported,
        0,
        &mut new,
        0,
        &mut refined,
        0,
        &mut scratch,
        &mut output,
        &limits,
        &NeverCancel,
        budget,
    )
    .unwrap();
    let error = composer
        .note_resident(&vec![0; 2], &Vec::new())
        .unwrap_err();
    assert!(matches!(
        error.kind,
        TextComposeErrorKind::LimitExceeded {
            resource: "resident row bytes",
            ..
        }
    ));
    composer.progress.output_bytes_written = 2;
    let error = composer.output_write(2, &[0]).unwrap_err();
    assert!(matches!(
        error.kind,
        TextComposeErrorKind::LimitExceeded {
            resource: "output bytes",
            ..
        }
    ));
}
