// SPDX-License-Identifier: MIT

//! Public refinement API tests with synthetic bitmaps, MQ-coded for the
//! standard T.88 states by the test-only encoder. No external document
//! pixels are included.

mod common;

use caj2pdf_core::{
    Limits, NeverCancel, RangedSource,
    jbig2::{
        dictionary::SymbolDescriptor,
        integer::BITMAP_BASE,
        mq::{CodedSpan, ContextBank, ContextState, MqBudget, MqDecoder, MqTable},
        refinement::{
            RefinementBudget, RefinementDecoder, RefinementError, RefinementErrorKind,
            RefinementReference, RefinementRequest,
        },
    },
};
use std::io::Write;

struct Source {
    bytes: Vec<u8>,
    max_read: usize,
    calls: u64,
    max_request: usize,
}

impl Source {
    fn new(bytes: &[u8]) -> Self {
        Self {
            bytes: bytes.to_vec(),
            max_read: usize::MAX,
            calls: 0,
            max_request: 0,
        }
    }
}

impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> caj2pdf_core::Result<usize> {
        self.calls += 1;
        self.max_request = self.max_request.max(destination.len());
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let count = self
            .bytes
            .len()
            .saturating_sub(start)
            .min(destination.len())
            .min(self.max_read);
        if count != 0 {
            destination[..count].copy_from_slice(&self.bytes[start..start + count]);
        }
        Ok(count)
    }
}

struct Sink {
    bytes: Vec<u8>,
    max_write: usize,
    calls: u64,
    max_request: usize,
    flushed: bool,
}

impl Sink {
    fn new(prefilled: &[u8]) -> Self {
        Self {
            bytes: prefilled.to_vec(),
            max_write: usize::MAX,
            calls: 0,
            max_request: 0,
            flushed: false,
        }
    }
}

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.calls += 1;
        self.max_request = self.max_request.max(bytes.len());
        let count = bytes.len().min(self.max_write);
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.flushed = true;
        Ok(())
    }
}

fn table() -> MqTable {
    MqTable::standard()
}

/// A coding unit's contexts with the bitmap range at
/// [`BITMAP_BASE`]; refinement needs no IAID contexts.
fn contexts(limits: &Limits, mq_budget: &MqBudget) -> ContextBank {
    mq_budget.context_bank(BITMAP_BASE + 1024, limits).unwrap()
}

/// An MQ stream coding each `(context, pixel)` decision in order.
fn stream(decisions: &[(usize, bool)]) -> Vec<u8> {
    let mut encoder = common::mq_encoder();
    for &(context, bit) in decisions {
        encoder.encode(context, bit);
    }
    encoder.finish()
}

fn whole(bytes: &[u8]) -> CodedSpan {
    CodedSpan {
        offset: 0,
        length: bytes.len() as u64,
    }
}

fn reference(
    width: u32,
    height: u32,
    store_base: u64,
    relative_store_offset: u64,
) -> RefinementReference {
    let stride = width.div_ceil(8);
    RefinementReference {
        store_base,
        symbol: SymbolDescriptor {
            width,
            height,
            row_stride: stride,
            relative_store_offset,
            stored_bytes: u64::from(stride) * u64::from(height),
        },
    }
}

fn request(
    width: u32,
    height: u32,
    reference: RefinementReference,
    dx: i32,
    dy: i32,
) -> RefinementRequest {
    RefinementRequest {
        width,
        height,
        template: 1,
        typical_prediction: false,
        reference_dx: dx,
        reference_dy: dy,
        reference,
    }
}

#[test]
fn two_bitmaps_share_gr_statistics_but_restart_target_history_and_store_offsets() {
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let table = table();
    let mut contexts = contexts(&limits, &mq_budget);
    let base = BITMAP_BASE;
    // Figure 13: the reference centre is context bit 3. Both first pixels
    // use GR context 8.
    let bytes = stream(&[(base + 8, true), (base + 8, true)]);
    let mut mq_source = Source::new(&bytes);
    let mut mq = MqDecoder::new(
        &mut mq_source,
        whole(&bytes),
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        mq_budget,
    )
    .unwrap();
    // The valid one-bit reference is at absolute byte 2. Byte 1 is zero, so
    // omitting the adapter's base would select a different GR context.
    let mut reference_source = Source::new(&[0x57, 0x00, 0x80]);
    let reference = reference(1, 1, 1, 1);
    let mut sink = Sink::new(&[0x57]);
    let mut host = RefinementDecoder::new(
        &mut mq,
        &mut sink,
        &limits,
        &NeverCancel,
        RefinementBudget::default(),
    )
    .unwrap();
    let first = host
        .decode_bitmap(&mut reference_source, request(1, 1, reference, 0, 0))
        .unwrap();
    assert_eq!(first.target.relative_store_offset, 0);
    assert_eq!(first.target.stored_bytes, 1);
    assert_eq!(first.progress.output_bytes_written, 1);
    assert_eq!(first.progress.mq.unwrap().symbols_decoded, 1);
    let second = host
        .decode_bitmap(&mut reference_source, request(1, 1, reference, 0, 0))
        .unwrap();
    assert_eq!(second.target.relative_store_offset, 1);
    assert_eq!(second.progress.completed_bitmaps, 2);
    assert_eq!(second.progress.pixels_decoded, 2);
    assert_eq!(second.progress.mq.unwrap().symbols_decoded, 2);
    drop(host);
    assert_eq!(sink.bytes, [0x57, 0x80, 0x80]);
    assert!(!sink.flushed);
    // The first decision, at A = 0x8000 in state 0, always renormalizes.
    assert_ne!(mq.context(base + 8), Some(ContextState::default()));
    assert_eq!(mq.context(base), Some(ContextState::default()));
    mq.finish(2).unwrap();
}

#[test]
fn an_interleaved_non_gr_mq_decision_keeps_the_sink_and_gr_session() {
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let table = table();
    let mut contexts = contexts(&limits, &mq_budget);
    let gr_base = BITMAP_BASE;
    let bytes = stream(&[(gr_base + 8, true), (0, false), (gr_base + 8, true)]);
    let mut mq_source = Source::new(&bytes);
    let mut mq = MqDecoder::new(
        &mut mq_source,
        whole(&bytes),
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        mq_budget,
    )
    .unwrap();
    let mut reference_source = Source::new(&[0x80]);
    let mut sink = Sink::new(&[0x57]);
    let mut host = RefinementDecoder::new(
        &mut mq,
        &mut sink,
        &limits,
        &NeverCancel,
        RefinementBudget::default(),
    )
    .unwrap();
    let first = host
        .decode_bitmap(
            &mut reference_source,
            request(1, 1, reference(1, 1, 0, 0), 0, 0),
        )
        .unwrap();
    assert_eq!(first.target.relative_store_offset, 0);
    assert_eq!(first.progress.mq.unwrap().symbols_decoded, 1);

    host.flush_store().unwrap();
    assert_eq!(host.progress().flushes, 1);

    // Context zero belongs to the Annex A.2 integer domain, disjoint from
    // the GR range. The host exposes the same MQ coding unit between bitmaps.
    let decision = host.mq_mut().unwrap().decode_bit(0).unwrap();
    assert!(!decision);
    assert_eq!(host.progress().mq.unwrap().symbols_decoded, 2);

    let second = host
        .decode_bitmap(
            &mut reference_source,
            request(1, 1, reference(1, 1, 0, 0), 0, 0),
        )
        .unwrap();
    assert_eq!(second.target.relative_store_offset, 1);
    assert_eq!(second.progress.completed_bitmaps, 2);
    assert_eq!(second.progress.output_bytes_written, 2);
    assert_eq!(second.progress.flushes, 1);
    assert_eq!(second.progress.mq.unwrap().symbols_decoded, 3);
    drop(host);
    assert_eq!(sink.bytes, [0x57, 0x80, 0x80]);
    assert!(sink.flushed);
    assert_ne!(mq.context(gr_base + 8), Some(ContextState::default()));
    mq.finish(3).unwrap();
}

#[test]
fn signed_offsets_select_the_specified_reference_taps_without_overflow() {
    // For a 1x1 reference containing one set pixel, these are Figure 13's
    // context-bit weights after alignment at (x-DX, y-DY).
    let cases = [
        (0, 0, 8),
        (1, 0, 4),
        (-1, 0, 16),
        (0, 1, 2),
        (0, -1, 32),
        (i32::MIN, i32::MAX, 0),
        (i32::MAX, i32::MIN, 0),
    ];
    for (dx, dy, expected_context) in cases {
        let limits = Limits::default();
        let mq_budget = MqBudget::default();
        let table = table();
        let mut contexts = contexts(&limits, &mq_budget);
        let base = BITMAP_BASE;
        let bytes = stream(&[(base + expected_context, true)]);
        let mut mq_source = Source::new(&bytes);
        let mut mq = MqDecoder::new(
            &mut mq_source,
            whole(&bytes),
            &table,
            &mut contexts,
            &limits,
            &NeverCancel,
            mq_budget,
        )
        .unwrap();
        let mut reference_source = Source::new(&[0x80]);
        let mut sink = Sink::new(&[]);
        let mut host = RefinementDecoder::new(
            &mut mq,
            &mut sink,
            &limits,
            &NeverCancel,
            RefinementBudget::default(),
        )
        .unwrap();
        let report = host
            .decode_bitmap(
                &mut reference_source,
                request(1, 1, reference(1, 1, 0, 0), dx, dy),
            )
            .unwrap();
        assert_eq!(report.progress.pixels_decoded, 1);
        drop(host);
        assert_eq!(sink.bytes, [0x80], "offset ({dx}, {dy})");
        // The only decision starts in state 0 at A = 0x8000, so it always
        // renormalizes and moves its context to state 1.
        assert_eq!(
            mq.context(base + expected_context).unwrap().state_index,
            1,
            "offset ({dx}, {dy})"
        );
        mq.finish(1).unwrap();
    }
}

#[test]
fn one_byte_io_keeps_rows_packed_and_reuses_three_reference_rows() {
    let limits = Limits {
        io_chunk_bytes: 2,
        ..Limits::default()
    };
    let mq_budget = MqBudget::default();
    let table = table();
    let mut contexts = contexts(&limits, &mq_budget);
    // Every target pixel is set. The template-1 contexts of the 18 pixels,
    // over the all-set reference: the left edge, the row interior, and the
    // right edge, with the previous target row set on the second row.
    let base = BITMAP_BASE;
    let mut decisions = vec![(base + 15, true)];
    decisions.extend([(base + 95, true); 7]);
    decisions.extend([(base + 90, true), (base + 431, true)]);
    decisions.extend([(base + 1023, true); 7]);
    decisions.push((base + 890, true));
    let bytes = stream(&decisions);
    let mut mq_source = Source::new(&bytes);
    let mut mq = MqDecoder::new(
        &mut mq_source,
        whole(&bytes),
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        mq_budget,
    )
    .unwrap();
    // The low seven bits in each second reference byte are outside width 9.
    let mut reference_source = Source::new(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
    reference_source.max_read = 1;
    let mut sink = Sink::new(&[]);
    sink.max_write = 1;
    let budget = RefinementBudget {
        max_reference_reads: 6,
        max_reference_bytes_fetched: 6,
        max_sink_writes: 4,
        max_source_request_bytes: 1,
        max_sink_request_bytes: 1,
        ..RefinementBudget::default()
    };
    let mut host =
        RefinementDecoder::new(&mut mq, &mut sink, &limits, &NeverCancel, budget).unwrap();
    let report = host
        .decode_bitmap(
            &mut reference_source,
            request(9, 2, reference(9, 3, 0, 0), 0, 0),
        )
        .unwrap();
    assert_eq!(report.progress.reference_reads, 6);
    assert_eq!(report.progress.reference_bytes_fetched, 6);
    assert_eq!(report.progress.sink_writes, 4);
    assert_eq!(report.progress.output_bytes_written, 4);
    assert_eq!(report.progress.rows_written, 2);
    assert_eq!(report.progress.context_work, 180);
    assert_eq!(report.progress.mq.unwrap().symbols_decoded, 18);
    assert_eq!(reference_source.max_request, 1);
    drop(host);
    assert_eq!(sink.bytes, [0xff, 0x80, 0xff, 0x80]);
    assert_eq!(sink.max_request, 1);
    mq.finish(18).unwrap();
}

#[test]
fn exact_packed_set_and_clear_pixels_at_byte_boundaries() {
    let cases: [(u32, &[u8], bool); 3] = [
        (7, &[0xfe], false),
        (8, &[0xff], false),
        (9, &[0x00, 0x00], true),
    ];
    for (width, expected, clear_pixels) in cases {
        let limits = Limits::default();
        let mq_budget = MqBudget::default();
        let table = table();
        let mut contexts = contexts(&limits, &mq_budget);
        // Over an all-zero reference, the only nonzero template-1 neighbour
        // in one row is the target pixel to the left (context bit 6). Clear
        // pixels must leave both packed bytes clear, including the seven
        // padding bits.
        let base = BITMAP_BASE;
        let decisions: Vec<_> = (0..width)
            .map(|x| {
                let left = !clear_pixels && x > 0;
                (base + if left { 64 } else { 0 }, !clear_pixels)
            })
            .collect();
        let bytes = stream(&decisions);
        let mut mq_source = Source::new(&bytes);
        let mut mq = MqDecoder::new(
            &mut mq_source,
            whole(&bytes),
            &table,
            &mut contexts,
            &limits,
            &NeverCancel,
            mq_budget,
        )
        .unwrap();
        let mut reference_source = Source::new(&[0x00]);
        let mut sink = Sink::new(&[]);
        let mut host = RefinementDecoder::new(
            &mut mq,
            &mut sink,
            &limits,
            &NeverCancel,
            RefinementBudget::default(),
        )
        .unwrap();
        let report = host
            .decode_bitmap(
                &mut reference_source,
                request(width, 1, reference(1, 1, 0, 0), 0, 0),
            )
            .unwrap();
        assert_eq!(report.target.row_stride, expected.len() as u32);
        assert_eq!(
            report.progress.mq.unwrap().symbols_decoded,
            u64::from(width)
        );
        drop(host);
        assert_eq!(sink.bytes, expected);
        mq.finish(u64::from(width)).unwrap();
    }
}

#[test]
fn a_failed_bitmap_poisons_interleaved_mq_access() {
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let table = table();
    let mut contexts = contexts(&limits, &mq_budget);
    let bytes = stream(&[(BITMAP_BASE + 8, true)]);
    let mut mq_source = Source::new(&bytes);
    let mut mq = MqDecoder::new(
        &mut mq_source,
        whole(&bytes),
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        mq_budget,
    )
    .unwrap();
    let mut reference_source = Source::new(&[0x80]);
    let mut sink = Sink::new(&[]);
    sink.max_write = 0;
    let mut host = RefinementDecoder::new(
        &mut mq,
        &mut sink,
        &limits,
        &NeverCancel,
        RefinementBudget::default(),
    )
    .unwrap();
    assert!(host.mq_mut().is_ok());
    let error = host
        .decode_bitmap(
            &mut reference_source,
            request(1, 1, reference(1, 1, 0, 0), 0, 0),
        )
        .unwrap_err();
    assert!(
        matches!(error.kind, RefinementErrorKind::Sink(_)),
        "{error:?}"
    );
    assert!(error.progress.poisoned);
    assert!(matches!(
        host.mq_mut(),
        Err(RefinementError {
            kind: RefinementErrorKind::Poisoned,
            ..
        })
    ));
}
