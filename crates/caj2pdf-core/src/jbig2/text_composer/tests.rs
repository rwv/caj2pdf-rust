// SPDX-License-Identifier: MIT

use super::*;
use crate::NeverCancel;
use crate::jbig2::{
    SegmentSpan,
    text::{ReferenceCorner, RegionCombination, RegionInfo, TextRegionFlags},
};
use std::{
    cell::Cell,
    error::Error as StdError,
    future::Future,
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
        Poll::Pending => panic!("unexpected pending test I/O"),
    }
}

fn header(
    width: u32,
    height: u32,
    count: u32,
    default_pixel: bool,
    combination: SymbolCombination,
) -> TextRegionHeader {
    TextRegionHeader {
        segment: 3,
        page_association: 1,
        dictionary_segment: 2,
        anomaly: None,
        region: RegionInfo {
            width,
            height,
            x: 0,
            y: 0,
            combination: RegionCombination::Or,
        },
        flags: TextRegionFlags {
            raw: 0x10,
            huffman: false,
            refine: false,
            log_strips: 0,
            reference_corner: ReferenceCorner::TopLeft,
            transposed: false,
            combination,
            default_pixel,
            ds_offset: 0,
            refinement_template: 0,
        },
        huffman_flags: None,
        refinement_at: None,
        instances: count,
        header_bytes: 23,
        body: SegmentSpan {
            offset: 23,
            length: 2,
        },
    }
}

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

#[derive(Default)]
struct Manual {
    header: Option<TextRegionHeader>,
    events: Vec<TextInstance>,
    next: usize,
    fail_at: Option<usize>,
    early_end: bool,
}

impl Manual {
    fn new(header: TextRegionHeader, events: Vec<TextInstance>) -> Self {
        Self {
            header: Some(header),
            events,
            next: 0,
            fail_at: None,
            early_end: false,
        }
    }
}

impl TextInstanceSource for Manual {
    fn segment(&self) -> u32 {
        3
    }

    fn header(&self) -> TextRegionHeader {
        self.header.unwrap()
    }

    async fn next(&mut self) -> TextInstanceResult<Option<TextInstance>> {
        if self.fail_at == Some(self.next) {
            return Err(TextInstanceError {
                segment: 3,
                offset: 27,
                progress: Box::new(crate::jbig2::text_instances::TextInstanceProgress::default()),
                kind: crate::jbig2::text_instances::TextInstanceErrorKind::Malformed("test stream"),
            });
        }
        if self.early_end {
            return Ok(None);
        }
        let event = self.events.get(self.next).copied();
        self.next += 1;
        Ok(event)
    }
}

#[derive(Default)]
struct Bytes {
    data: Vec<u8>,
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
            data: data.to_vec(),
            max_part: usize::MAX,
            ..Self::default()
        }
    }
}

impl RangedSource for Bytes {
    fn size(&self) -> u64 {
        self.reported_size.unwrap_or(self.data.len() as u64)
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> crate::Result<usize> {
        self.calls += 1;
        if self.shrink_at_call == Some(self.calls) {
            self.data.clear();
        }
        if self.zero_at_call == Some(self.calls) {
            return Ok(0);
        }
        if self.over_at_call == Some(self.calls) {
            return Ok(destination.len() + 1);
        }
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        if start >= self.data.len() {
            return Ok(0);
        }
        let n = destination
            .len()
            .min(self.max_part)
            .min(self.data.len() - start);
        destination[..n].copy_from_slice(&self.data[start..start + n]);
        if self.shrink_after_read_at_call == Some(self.calls) {
            self.data.clear();
        }
        Ok(n)
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

    async fn set_len(&mut self, bytes: u64) -> crate::Result<()> {
        if self.fail_set_len {
            return Err(Error::Io(io::Error::other("set_len")));
        }
        self.data
            .resize(bytes as usize - usize::from(self.incorrect_set_len), 0);
        Ok(())
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> crate::Result<usize> {
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

    async fn write_at(&mut self, offset: u64, bytes: &[u8]) -> crate::Result<usize> {
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

    async fn flush(&mut self) -> crate::Result<()> {
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
    pending_flush: Option<Rc<Cell<bool>>>,
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

impl SequentialSink for Sink {
    async fn write(&mut self, bytes: &[u8]) -> crate::Result<usize> {
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

    async fn flush(&mut self) -> crate::Result<()> {
        self.flushes += 1;
        if let Some(flag) = &self.change_size_on_flush {
            flag.set(true);
        }
        if self.pending_flush.as_ref().is_some_and(|flag| flag.get()) {
            std::future::pending::<()>().await;
        }
        if self.fail_flush {
            Err(Error::Io(io::Error::other("output flush")))
        } else {
            Ok(())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn compose_manual(
    header: TextRegionHeader,
    catalog: &[StoredSymbol],
    events: Vec<TextInstance>,
    imported: &mut Bytes,
    new: &mut Bytes,
    refined: &mut Bytes,
    scratch: &mut Scratch,
    output: &mut Sink,
    budget: TextComposeBudget,
) -> TextComposeResult<TextComposeReport> {
    let mut stream = Manual::new(header, events);
    let limits = Limits::default();
    let mut composer = TextComposer::new(
        3,
        header,
        catalog,
        &mut stream,
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
    ready(composer.compose())
}

#[test]
fn completed_report_retains_explicit_header_anomaly() {
    let symbol = stored(SymbolStore::Imported, descriptor(1, 1, 0));
    let mut parsed = header(1, 1, 1, false, SymbolCombination::Or);
    parsed.flags.raw = 0xa40c;
    parsed.flags.refinement_template = 1;
    parsed.flags.log_strips = 3;
    parsed.flags.ds_offset = 9;
    parsed.anomaly = Some(TextHeaderAnomaly::UnusedRefinementTemplate);
    let report = compose_manual(
        parsed,
        &[symbol],
        vec![event(0, 0, 0, 0, TextBitmap::Stored(symbol))],
        &mut Bytes::new(&[0x80]),
        &mut Bytes::new(&[]),
        &mut Bytes::new(&[]),
        &mut Scratch::new(),
        &mut Sink::new(),
        TextComposeBudget::default(),
    )
    .unwrap();
    assert_eq!(report.text_flags_raw, 0xa40c);
    assert_eq!(report.header_anomaly, parsed.anomaly);
}

#[test]
fn composes_imported_new_and_refined_handles_in_nonmonotone_order() {
    let imported_symbol = stored(SymbolStore::Imported, descriptor(3, 2, 0));
    let new_symbol = stored(SymbolStore::New, descriptor(2, 2, 0));
    let refined_symbol = descriptor(1, 2, 0);
    let events = vec![
        event(0, 0, 1, 0, TextBitmap::Stored(imported_symbol)),
        event(1, 1, 3, 1, TextBitmap::Stored(new_symbol)),
        event(
            2,
            0,
            0,
            0,
            TextBitmap::Refined {
                store_base: 0,
                symbol: refined_symbol,
            },
        ),
        event(3, 1, -4, 1, TextBitmap::Stored(new_symbol)),
    ];
    let mut imported = Bytes::new(&[0xa0, 0x40]);
    let mut new = Bytes::new(&[0xc0, 0xc0]);
    let mut refined = Bytes::new(&[0x80, 0x80]);
    let mut scratch = Scratch::new();
    let mut output = Sink::new();
    let report = compose_manual(
        header(5, 3, 4, false, SymbolCombination::Or),
        &[imported_symbol, new_symbol],
        events,
        &mut imported,
        &mut new,
        &mut refined,
        &mut scratch,
        &mut output,
        TextComposeBudget::default(),
    )
    .unwrap();
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
    let events = vec![
        event(0, 0, -1, -1, TextBitmap::Stored(symbol)),
        event(1, 0, 5, 4, TextBitmap::Stored(symbol)),
    ];
    let mut imported = Bytes::new(&[0xa0, 0x40]);
    let mut new = Bytes::new(&[]);
    let mut refined = Bytes::new(&[]);
    let mut scratch = Scratch::new();
    let mut output = Sink::new();
    let report = compose_manual(
        header(5, 2, 2, false, SymbolCombination::Or),
        &[symbol],
        events,
        &mut imported,
        &mut new,
        &mut refined,
        &mut scratch,
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
        let mut imported = Bytes::new(&[0xc0, second_byte]);
        let mut new = Bytes::new(&[]);
        let mut refined = Bytes::new(&[]);
        let mut scratch = Scratch::new();
        let mut output = Sink::new();
        let events = vec![
            event(0, 0, 0, 0, TextBitmap::Stored(first)),
            event(1, 1, 1, 0, TextBitmap::Stored(second)),
        ];
        compose_manual(
            header(3, 1, 2, default_pixel, operator),
            &[first, second],
            events,
            &mut imported,
            &mut new,
            &mut refined,
            &mut scratch,
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
        let mut imported = Bytes::new(&[]);
        let mut new = Bytes::new(&[]);
        let mut refined = Bytes::new(&[]);
        let mut scratch = Scratch::new();
        let mut output = Sink::new();
        let report = compose_manual(
            header(10, 2, 0, default_pixel, SymbolCombination::And),
            &[],
            vec![],
            &mut imported,
            &mut new,
            &mut refined,
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
    for corner in [
        ReferenceCorner::TopLeft,
        ReferenceCorner::TopRight,
        ReferenceCorner::BottomLeft,
        ReferenceCorner::BottomRight,
    ] {
        for transposed in [false, true] {
            let mut h = header(4, 3, 1, false, SymbolCombination::Or);
            h.flags.reference_corner = corner;
            h.flags.transposed = transposed;
            let mut imported = Bytes::new(&[0xc0]);
            let mut new = Bytes::new(&[]);
            let mut refined = Bytes::new(&[]);
            let mut scratch = Scratch::new();
            let mut output = Sink::new();
            compose_manual(
                h,
                &[symbol],
                vec![event(0, 0, 1, 2, TextBitmap::Stored(symbol))],
                &mut imported,
                &mut new,
                &mut refined,
                &mut scratch,
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

fn one_pixel_run(
    imported: &mut Bytes,
    scratch: &mut Scratch,
    output: &mut Sink,
    budget: TextComposeBudget,
) -> TextComposeResult<TextComposeReport> {
    let symbol = stored(SymbolStore::Imported, descriptor(1, 1, 0));
    compose_manual(
        header(3, 2, 1, false, SymbolCombination::Or),
        &[symbol],
        vec![event(0, 0, 1, 1, TextBitmap::Stored(symbol))],
        imported,
        &mut Bytes::new(&[]),
        &mut Bytes::new(&[]),
        scratch,
        output,
        budget,
    )
}

#[test]
fn short_io_is_counted_and_request_sizes_stay_bounded() {
    let symbol = stored(SymbolStore::Imported, descriptor(10, 2, 0));
    let mut imported = Bytes::new(&[0xff, 0xc0, 0x80, 0x00]);
    imported.max_part = 1;
    let mut scratch = Scratch::new();
    scratch.max_part = 1;
    let mut output = Sink::new();
    output.max_part = 1;
    let budget = TextComposeBudget {
        max_request_bytes: 1,
        ..TextComposeBudget::default()
    };
    let report = compose_manual(
        header(10, 2, 1, false, SymbolCombination::Or),
        &[symbol],
        vec![event(0, 0, 0, 0, TextBitmap::Stored(symbol))],
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
    let symbol = stored(SymbolStore::Imported, descriptor(1, 1, 0));
    let mut stream = Manual::new(
        header(3, 2, 1, false, SymbolCombination::Or),
        vec![event(0, 0, 1, 1, TextBitmap::Stored(symbol))],
    );
    let mut imported = Bytes::new(&[0x80]);
    let mut new = Bytes::new(&[]);
    let mut refined = Bytes::new(&[]);
    let flag = Rc::new(Cell::new(false));
    let cancel = Flag(Rc::clone(&flag));
    let mut scratch = Scratch::new();
    scratch.cancel_after_write = Some((1, Rc::clone(&flag)));
    let mut output = Sink::new();
    let limits = Limits::default();
    let catalog = [symbol];
    let mut composer = TextComposer::new(
        3,
        stream.header(),
        &catalog,
        &mut stream,
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
    let error = ready(composer.compose()).unwrap_err();
    assert!(matches!(error.kind, TextComposeErrorKind::Cancelled));
    assert_eq!(error.progress.scratch_bytes_written, 1);
    assert_eq!(error.progress.output_bytes_written, 0);
    assert!(matches!(
        ready(composer.compose()).unwrap_err().kind,
        TextComposeErrorKind::Poisoned
    ));
}

#[test]
fn dropped_pending_final_flush_poisoned_and_never_reports_completion() {
    let symbol = stored(SymbolStore::Imported, descriptor(1, 1, 0));
    let mut stream = Manual::new(
        header(3, 2, 1, false, SymbolCombination::Or),
        vec![event(0, 0, 1, 1, TextBitmap::Stored(symbol))],
    );
    let mut imported = Bytes::new(&[0x80]);
    let mut new = Bytes::new(&[]);
    let mut refined = Bytes::new(&[]);
    let mut scratch = Scratch::new();
    let flag = Rc::new(Cell::new(true));
    let mut output = Sink::new();
    output.pending_flush = Some(Rc::clone(&flag));
    let limits = Limits::default();
    let catalog = [symbol];
    let mut composer = TextComposer::new(
        3,
        stream.header(),
        &catalog,
        &mut stream,
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
    let mut future = Box::pin(composer.compose());
    assert!(matches!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop())),
        Poll::Pending
    ));
    drop(future);
    assert_eq!(composer.progress().output_bytes_written, 2);
    assert!(composer.progress().poisoned);
    flag.set(false);
    assert!(matches!(
        ready(composer.compose()).unwrap_err().kind,
        TextComposeErrorKind::Poisoned
    ));
}

#[test]
fn forged_handles_and_geometry_fail_before_any_symbol_read() {
    let symbol = stored(SymbolStore::Imported, descriptor(1, 1, 0));
    let bad = [
        event(
            0,
            0,
            0,
            0,
            TextBitmap::Stored(stored(SymbolStore::New, descriptor(1, 1, 0))),
        ),
        event(
            0,
            0,
            0,
            0,
            TextBitmap::Refined {
                store_base: 1,
                symbol: descriptor(1, 1, 0),
            },
        ),
        TextInstance {
            width: 2,
            ..event(0, 0, 0, 0, TextBitmap::Stored(symbol))
        },
        event(
            0,
            0,
            0,
            0,
            TextBitmap::Stored(stored(SymbolStore::Imported, descriptor(1, 1, 9))),
        ),
        event(0, 0, i32::MAX as i64 + 1, 0, TextBitmap::Stored(symbol)),
        event(1, 0, 0, 0, TextBitmap::Stored(symbol)),
    ];
    for instance in bad {
        let mut imported = Bytes::new(&[0x80]);
        let mut new = Bytes::new(&[]);
        let mut refined = Bytes::new(&[]);
        let mut scratch = Scratch::new();
        let mut output = Sink::new();
        let error = compose_manual(
            header(3, 2, 1, false, SymbolCombination::Or),
            &[symbol],
            vec![instance],
            &mut imported,
            &mut new,
            &mut refined,
            &mut scratch,
            &mut output,
            TextComposeBudget::default(),
        )
        .unwrap_err();
        assert!(matches!(
            error.kind,
            TextComposeErrorKind::Malformed(_) | TextComposeErrorKind::InvalidSpan(_)
        ));
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
        let mut imported = Bytes::new(&[0x80, 0x80]);
        let mut new = Bytes::new(&[0x80, 0x80]);
        let mut refined = Bytes::new(&[]);
        let mut scratch = Scratch::new();
        let mut output = Sink::new();
        let error = compose_manual(
            header(3, 2, 1, false, SymbolCombination::Or),
            &[symbol],
            vec![event(0, 0, 0, 0, TextBitmap::Stored(symbol))],
            &mut imported,
            &mut new,
            &mut refined,
            &mut scratch,
            &mut output,
            TextComposeBudget::default(),
        )
        .unwrap_err();
        assert!(matches!(
            error.kind,
            TextComposeErrorKind::Malformed("bitmap handle store base differs from view")
        ));
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
        let mut instance = event(
            0,
            0,
            ((state >> 16) as i16) as i64,
            ((state >> 1) as i16) as i64,
            TextBitmap::Stored(symbol),
        );
        match state & 7 {
            0 => instance.index = 1,
            1 => instance.symbol_id = 9,
            2 => instance.width = 0,
            3 => {
                instance.bitmap = TextBitmap::Refined {
                    store_base: 1,
                    symbol: descriptor(1, 1, 0),
                }
            }
            _ => {}
        }
        let mut imported = Bytes::new(&[0x80]);
        let mut new = Bytes::new(&[]);
        let mut refined = Bytes::new(&[]);
        let mut scratch = Scratch::new();
        let mut output = Sink::new();
        let budget = TextComposeBudget {
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
        match compose_manual(
            header(3, 2, 1, false, SymbolCombination::Or),
            &[symbol],
            vec![instance],
            &mut imported,
            &mut new,
            &mut refined,
            &mut scratch,
            &mut output,
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
fn constructor_rejects_invalid_header_stream_identity_stores_and_limits() {
    let symbol = stored(SymbolStore::Imported, descriptor(1, 1, 0));
    for case in 0..13 {
        let mut h = header(3, 2, 1, false, SymbolCombination::Or);
        let mut stream = Manual::new(h, vec![event(0, 0, 1, 1, TextBitmap::Stored(symbol))]);
        let mut imported = Bytes::new(&[0x80]);
        let mut new = Bytes::new(&[]);
        let mut refined = Bytes::new(&[]);
        let mut scratch = Scratch::new();
        let mut output = Sink::new();
        let mut limits = Limits::default();
        let mut budget = TextComposeBudget::default();
        let mut segment = 3;
        let mut catalog = vec![symbol];
        let mut imported_base = 0;
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
            11 => stream.header = Some(header(4, 2, 1, false, SymbolCombination::Or)),
            _ => h.segment = 4,
        }
        if case != 11 {
            stream.header = Some(h);
        }
        let error = TextComposer::new(
            segment,
            h,
            &catalog,
            &mut stream,
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
        let symbol = stored(SymbolStore::Imported, descriptor);
        let mut imported = Bytes::new(&[0x80, 0]);
        let mut new = Bytes::new(&[]);
        let mut refined = Bytes::new(&[]);
        let mut scratch = Scratch::new();
        let mut output = Sink::new();
        let error = compose_manual(
            header(3, 2, 1, false, SymbolCombination::Or),
            &[symbol],
            vec![event(0, 0, 1, 1, TextBitmap::Stored(symbol))],
            &mut imported,
            &mut new,
            &mut refined,
            &mut scratch,
            &mut output,
            TextComposeBudget::default(),
        )
        .unwrap_err();
        assert!(matches!(error.kind, TextComposeErrorKind::Malformed(_)));
        assert_eq!(imported.calls, 0);
    }
}

#[test]
fn internal_io_error_mapping_and_pre_io_size_guards_are_located() {
    let symbol = stored(SymbolStore::Imported, descriptor(1, 1, 0));
    let mut stream = Manual::new(header(3, 2, 1, false, SymbolCombination::Or), vec![]);
    let mut imported = Bytes::new(&[0x80]);
    let mut new = Bytes::new(&[]);
    let mut refined = Bytes::new(&[]);
    let mut scratch = Scratch::new();
    let mut output = Sink::new();
    let limits = Limits::default();
    let catalog = [symbol];
    let mut composer = TextComposer::new(
        3,
        stream.header(),
        &catalog,
        &mut stream,
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
        ready(composer.scratch_write(0, &[0])).unwrap_err().kind,
        TextComposeErrorKind::Malformed("scratch size changed")
    ));
    assert!(matches!(
        ready(composer.scratch_read(0, &mut [0])).unwrap_err().kind,
        TextComposeErrorKind::Malformed("scratch size changed")
    ));
    composer.imported.data.clear();
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
        ready(composer.source_read(BitmapStore::Imported, 1, 0, &mut [0]))
            .unwrap_err()
            .kind,
        TextComposeErrorKind::StoreMutation {
            store: BitmapStore::Imported,
            reason: "size changed"
        }
    ));
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
fn refined_append_rules_are_checked_between_events() {
    let symbol = stored(SymbolStore::Imported, descriptor(1, 1, 0));
    let limits = Limits::default();
    let mut stream = Manual::new(header(1, 1, 0, false, SymbolCombination::Or), vec![]);
    let mut imported = Bytes::new(&[0x80]);
    let mut new = Bytes::new(&[]);
    let mut refined = Bytes::new(&[]);
    let mut scratch = Scratch::new();
    let mut output = Sink::new();
    let catalog = [symbol];
    let mut composer = TextComposer::new(
        3,
        stream.header(),
        &catalog,
        &mut stream,
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
    composer.refined.data.push(0x80);
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

    composer.refined.data.clear();
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
    let symbol = stored(SymbolStore::Imported, descriptor(1, 1, 0));
    let limits = Limits::default();
    let catalog = [symbol];
    for store in [BitmapStore::Imported, BitmapStore::New] {
        let mut stream = Manual::new(header(1, 1, 0, false, SymbolCombination::Or), vec![]);
        let mut imported = Bytes::new(&[0x80]);
        let mut new = Bytes::new(&[0x80]);
        let mut refined = Bytes::new(&[]);
        let mut scratch = Scratch::new();
        let mut output = Sink::new();
        let mut composer = TextComposer::new(
            3,
            stream.header(),
            &catalog,
            &mut stream,
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
        view.data.push(0x80);
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
fn stream_refusal_and_early_terminal_stop_before_output() {
    let symbol = stored(SymbolStore::Imported, descriptor(1, 1, 0));
    for fail in [true, false] {
        let mut stream = Manual::new(header(3, 2, 1, false, SymbolCombination::Or), vec![]);
        if fail {
            stream.fail_at = Some(0);
        } else {
            stream.early_end = true;
        }
        let mut imported = Bytes::new(&[0x80]);
        let mut new = Bytes::new(&[]);
        let mut refined = Bytes::new(&[]);
        let mut scratch = Scratch::new();
        let mut output = Sink::new();
        let limits = Limits::default();
        let catalog = [symbol];
        let mut composer = TextComposer::new(
            3,
            stream.header(),
            &catalog,
            &mut stream,
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
        let error = ready(composer.compose()).unwrap_err();
        if fail {
            assert_eq!(error.offset, 27);
            assert!(matches!(error.kind, TextComposeErrorKind::Instance(_)));
        } else {
            assert!(matches!(
                error.kind,
                TextComposeErrorKind::Malformed("instance stream ended before declared count")
            ));
        }
        assert_eq!(error.progress.completed_instances, 0);
        assert_eq!(error.progress.output_bytes_written, 0);
        assert!(error.progress.poisoned);
    }
}

#[test]
fn descriptor_spans_and_source_row_cap_are_checked_before_reading() {
    for case in 0..4 {
        let (descriptor, base, reported_size) = match case {
            0 => (descriptor(1, 1, 1), u64::MAX, Some(u64::MAX)),
            1 => (descriptor(1, 1, 1), u64::MAX - 1, Some(u64::MAX)),
            2 => (descriptor(1, 1, 2), 0, None),
            _ => (descriptor(9, 1, 0), 0, None),
        };
        let symbol = StoredSymbol {
            store: SymbolStore::Imported,
            store_base: base,
            symbol: descriptor,
        };
        let mut stream = Manual::new(
            header(3, 2, 1, false, SymbolCombination::Or),
            vec![event(0, 0, 0, 0, TextBitmap::Stored(symbol))],
        );
        let mut imported = Bytes::new(if case == 3 { &[0x80, 0x00] } else { &[0x80] });
        imported.reported_size = reported_size;
        let mut new = Bytes::new(&[]);
        let mut refined = Bytes::new(&[]);
        let mut scratch = Scratch::new();
        let mut output = Sink::new();
        let limits = Limits::default();
        let budget = if case == 3 {
            TextComposeBudget {
                max_row_bytes: 1,
                ..TextComposeBudget::default()
            }
        } else {
            TextComposeBudget::default()
        };
        let catalog = [symbol];
        let mut composer = TextComposer::new(
            3,
            stream.header(),
            &catalog,
            &mut stream,
            &mut imported,
            base,
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
        let error = ready(composer.compose()).unwrap_err();
        if case == 3 {
            assert!(matches!(
                error.kind,
                TextComposeErrorKind::LimitExceeded {
                    resource: "row bytes",
                    ..
                }
            ));
        } else {
            assert!(matches!(error.kind, TextComposeErrorKind::InvalidSpan(_)));
        }
        assert_eq!(error.progress.source_bytes_read, 0);
        assert_eq!(imported.calls, 0);
    }
}

#[test]
fn resident_capacity_and_output_accounting_are_checked_after_preflight() {
    let symbol = stored(SymbolStore::Imported, descriptor(1, 1, 0));
    let mut stream = Manual::new(header(3, 2, 1, false, SymbolCombination::Or), vec![]);
    let mut imported = Bytes::new(&[0x80]);
    let mut new = Bytes::new(&[]);
    let mut refined = Bytes::new(&[]);
    let mut scratch = Scratch::new();
    let mut output = Sink::new();
    let limits = Limits::default();
    let catalog = [symbol];
    let budget = TextComposeBudget {
        max_resident_bytes: 1,
        max_output_bytes: 2,
        ..TextComposeBudget::default()
    };
    let mut composer = TextComposer::new(
        3,
        stream.header(),
        &catalog,
        &mut stream,
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
    let error = ready(composer.output_write(2, &[0])).unwrap_err();
    assert!(matches!(
        error.kind,
        TextComposeErrorKind::LimitExceeded {
            resource: "output bytes",
            ..
        }
    ));
}

#[test]
fn production_instance_stream_composes_one_real_mq_symbol_and_terminal() {
    use crate::jbig2::{
        SegmentHeader,
        dictionary::{DictionaryDataHeader, DictionaryMode},
        iaid::IaidContextBanks,
        mq::{MQ_STATE_COUNT, MqBudget, MqSnapshot, MqState, MqTable},
        refinement::RefinementBudget,
        refinement_dictionary::{
            RefinementDictionaryCatalog, RefinementDictionaryProgress, RefinementDictionaryReport,
        },
        text::{TextRegionBudget, read_text_region_header},
        text_instances::{TextInstanceBudget, TextInstanceDecoder},
    };
    let mut data = Vec::new();
    data.extend_from_slice(&10u32.to_be_bytes());
    data.extend_from_slice(&10u32.to_be_bytes());
    data.extend_from_slice(&0u32.to_be_bytes());
    data.extend_from_slice(&0u32.to_be_bytes());
    data.push(0);
    data.extend_from_slice(&0x10u16.to_be_bytes());
    data.extend_from_slice(&1u32.to_be_bytes());
    data.extend_from_slice(&[0, 0, 0, 0, 0, 0xff, 0xac]);
    let mut source = Bytes::new(&data);
    let segment = |number, segment_type, referred_to: Vec<u32>, offset, length| SegmentHeader {
        number,
        segment_type,
        deferred_non_retain: false,
        page_association: 1,
        referred_to,
        data: SegmentSpan { offset, length },
        header_length: 0,
        retention: vec![0xff],
    };
    let text_segment = segment(3, 6, vec![2], 0, data.len() as u64);
    let dictionary_segment = segment(2, 0, vec![1], 100, 2);
    let symbol = stored(SymbolStore::Imported, descriptor(1, 1, 0));
    let report = RefinementDictionaryReport {
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
            exported_symbols: 1,
            new_symbols: 0,
            header_bytes: 0,
            body: SegmentSpan {
                offset: 100,
                length: 2,
            },
        },
        catalog: RefinementDictionaryCatalog {
            new_symbols: vec![],
            exported_symbols: vec![symbol],
        },
        progress: RefinementDictionaryProgress {
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
    };
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let table = MqTable::new(
        vec![
            MqState {
                qe: 1,
                next_mps: 0,
                next_lps: 0,
                switch_mps: false,
            };
            MQ_STATE_COUNT
        ],
        &limits,
    )
    .unwrap();
    let mut banks = IaidContextBanks::with_bitmap_contexts(0, 1024, &limits, &mq_budget).unwrap();
    let parsed = ready(read_text_region_header(
        &mut source,
        &text_segment,
        &dictionary_segment,
        &limits,
        TextRegionBudget::default(),
        &NeverCancel,
    ))
    .unwrap();
    let mut imported_for_decode = Bytes::new(&[0x80]);
    let mut new_for_decode = Bytes::new(&[]);
    let mut refined_sink = Sink::new();
    let mut decoder = ready(TextInstanceDecoder::new(
        &mut source,
        &text_segment,
        parsed,
        &dictionary_segment,
        &report,
        &mut imported_for_decode,
        0,
        &mut new_for_decode,
        0,
        &mut refined_sink,
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
    let mut imported_for_compose = Bytes::new(&[0x80]);
    let mut new_for_compose = Bytes::new(&[]);
    let mut refined_for_compose = Bytes::new(&[]);
    let mut scratch = Scratch::new();
    let mut output = Sink::new();
    let mut composer = TextComposer::new(
        3,
        parsed,
        &report.catalog.exported_symbols,
        &mut decoder,
        &mut imported_for_compose,
        0,
        &mut new_for_compose,
        0,
        &mut refined_for_compose,
        0,
        &mut scratch,
        &mut output,
        &limits,
        &NeverCancel,
        TextComposeBudget::default(),
    )
    .unwrap();
    let composed = ready(composer.compose()).unwrap();
    assert_eq!(
        (
            composed.progress.completed_instances,
            composed.progress.output_rows
        ),
        (1, 10)
    );
    assert_eq!(output.data.len(), 20);
    assert_eq!(&output.data[8..10], &[0x80, 0]);
    assert!(output.data[..8].iter().all(|byte| *byte == 0));
    assert!(output.data[10..].iter().all(|byte| *byte == 0));
}
