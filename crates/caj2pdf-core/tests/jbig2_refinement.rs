// SPDX-License-Identifier: MIT

//! Public refinement API tests with an invented probability table and bitmaps.
//! No normative MQ states or external document pixels are included.

use caj2pdf_core::{
    Limits, NeverCancel, RangedSource, SequentialSink,
    jbig2::{
        dictionary::SymbolDescriptor,
        iaid::IaidContextBanks,
        mq::{MQ_STATE_COUNT, MqBudget, MqContext, MqDecoder, MqSpan, MqState, MqTable},
        refinement::{RefinementBudget, RefinementDecoder, RefinementReference, RefinementRequest},
    },
};
use std::{
    future::Future,
    pin::pin,
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

    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
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

impl SequentialSink for Sink {
    async fn write(&mut self, bytes: &[u8]) -> caj2pdf_core::Result<usize> {
        self.calls += 1;
        self.max_request = self.max_request.max(bytes.len());
        let count = bytes.len().min(self.max_write);
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }

    async fn flush(&mut self) -> caj2pdf_core::Result<()> {
        self.flushed = true;
        Ok(())
    }
}

fn table(limits: &Limits) -> MqTable {
    // The MPS transition records whether a context was used once or twice.
    let mut states = vec![
        MqState {
            qe: 0x4000,
            next_mps: 2,
            next_lps: 2,
            switch_mps: false
        };
        MQ_STATE_COUNT
    ];
    states[0].next_mps = 1;
    states[0].next_lps = 1;
    MqTable::new(states, limits).unwrap()
}

fn banks(limits: &Limits, mq_budget: &MqBudget) -> IaidContextBanks {
    let mut banks = IaidContextBanks::with_bitmap_contexts(1, 1024, limits, mq_budget).unwrap();
    let base = banks.layout().bitmap_base();
    // On the invented FF AC stream, each explicitly decoded bit follows the
    // MPS path. Choosing MPS=1 makes expected packed pixels visible.
    for index in base..base + 1024 {
        banks
            .mq_contexts_mut()
            .set(
                index,
                MqContext {
                    state_index: 0,
                    mps: true,
                },
            )
            .unwrap();
    }
    banks
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
    let table = table(&limits);
    let mut banks = banks(&limits, &mq_budget);
    let layout = banks.layout();
    let base = layout.bitmap_base();
    let mut mq_source = Source::new(&[0xff, 0xac]);
    let mut mq = ready(MqDecoder::new(
        &mut mq_source,
        MqSpan {
            offset: 0,
            length: 2,
        },
        &table,
        banks.mq_contexts_mut(),
        &limits,
        &NeverCancel,
        mq_budget,
    ))
    .unwrap();
    // The valid one-bit reference is at absolute byte 2. Byte 1 is zero, so
    // omitting the adapter's base would select a different GR context.
    let mut reference_source = Source::new(&[0x57, 0x00, 0x80]);
    let reference = reference(1, 1, 1, 1);
    let mut sink = Sink::new(&[0x57]);
    let mut host = RefinementDecoder::new(
        &mut mq,
        layout,
        &mut sink,
        &limits,
        &NeverCancel,
        RefinementBudget::default(),
    )
    .unwrap();
    let first =
        ready(host.decode_bitmap(&mut reference_source, request(1, 1, reference, 0, 0))).unwrap();
    assert_eq!(first.target.relative_store_offset, 0);
    assert_eq!(first.target.stored_bytes, 1);
    assert_eq!(first.progress.output_bytes_written, 1);
    assert_eq!(first.progress.mq.unwrap().symbols_decoded, 1);
    let second =
        ready(host.decode_bitmap(&mut reference_source, request(1, 1, reference, 0, 0))).unwrap();
    assert_eq!(second.target.relative_store_offset, 1);
    assert_eq!(second.progress.completed_bitmaps, 2);
    assert_eq!(second.progress.pixels_decoded, 2);
    assert_eq!(second.progress.mq.unwrap().symbols_decoded, 2);
    drop(host);
    assert_eq!(sink.bytes, [0x57, 0x80, 0x80]);
    assert!(!sink.flushed);
    // Figure 13: the reference centre is context bit 3. Both first pixels
    // use GR context 8; their statistics advance 0 -> 1 -> 2.
    assert_eq!(mq.context(base + 8).unwrap().state_index, 2);
    assert_eq!(mq.context(base).unwrap().state_index, 0);
    ready(mq.finish(2)).unwrap();
}

#[test]
fn an_interleaved_non_gr_mq_decision_keeps_the_sink_and_gr_session() {
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let table = table(&limits);
    let mut banks = banks(&limits, &mq_budget);
    let layout = banks.layout();
    let gr_base = layout.bitmap_base();
    let mut mq_source = Source::new(&[0xff, 0xac]);
    let mut mq = ready(MqDecoder::new(
        &mut mq_source,
        MqSpan {
            offset: 0,
            length: 2,
        },
        &table,
        banks.mq_contexts_mut(),
        &limits,
        &NeverCancel,
        mq_budget,
    ))
    .unwrap();
    let mut reference_source = Source::new(&[0x80]);
    let mut sink = Sink::new(&[0x57]);
    let mut host = RefinementDecoder::new(
        &mut mq,
        layout,
        &mut sink,
        &limits,
        &NeverCancel,
        RefinementBudget::default(),
    )
    .unwrap();
    let first = ready(host.decode_bitmap(
        &mut reference_source,
        request(1, 1, reference(1, 1, 0, 0), 0, 0),
    ))
    .unwrap();
    assert_eq!(first.target.relative_store_offset, 0);
    assert_eq!(first.progress.mq.unwrap().symbols_decoded, 1);

    ready(host.flush_store()).unwrap();
    assert_eq!(host.progress().flushes, 1);

    // Context zero belongs to the Annex A.2 integer domain, disjoint from
    // the GR range. The host exposes the same MQ coding unit between bitmaps.
    let decision = ready(host.mq_mut().unwrap().decode_bit(0)).unwrap();
    assert!(!decision);
    assert_eq!(host.progress().mq.unwrap().symbols_decoded, 2);

    let second = ready(host.decode_bitmap(
        &mut reference_source,
        request(1, 1, reference(1, 1, 0, 0), 0, 0),
    ))
    .unwrap();
    assert_eq!(second.target.relative_store_offset, 1);
    assert_eq!(second.progress.completed_bitmaps, 2);
    assert_eq!(second.progress.output_bytes_written, 2);
    assert_eq!(second.progress.flushes, 1);
    assert_eq!(second.progress.mq.unwrap().symbols_decoded, 3);
    drop(host);
    assert_eq!(sink.bytes, [0x57, 0x80, 0x80]);
    assert!(sink.flushed);
    assert_eq!(mq.context(0).unwrap().state_index, 1);
    assert_eq!(mq.context(gr_base + 8).unwrap().state_index, 2);
    ready(mq.finish(3)).unwrap();
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
        let table = table(&limits);
        let mut banks = banks(&limits, &mq_budget);
        let layout = banks.layout();
        let base = layout.bitmap_base();
        let mut mq_source = Source::new(&[0xff, 0xac]);
        let mut mq = ready(MqDecoder::new(
            &mut mq_source,
            MqSpan {
                offset: 0,
                length: 2,
            },
            &table,
            banks.mq_contexts_mut(),
            &limits,
            &NeverCancel,
            mq_budget,
        ))
        .unwrap();
        let mut reference_source = Source::new(&[0x80]);
        let mut sink = Sink::new(&[]);
        let mut host = RefinementDecoder::new(
            &mut mq,
            layout,
            &mut sink,
            &limits,
            &NeverCancel,
            RefinementBudget::default(),
        )
        .unwrap();
        let report = ready(host.decode_bitmap(
            &mut reference_source,
            request(1, 1, reference(1, 1, 0, 0), dx, dy),
        ))
        .unwrap();
        assert_eq!(report.progress.pixels_decoded, 1);
        drop(host);
        assert_eq!(sink.bytes, [0x80]);
        assert_eq!(
            mq.context(base + expected_context).unwrap().state_index,
            1,
            "offset ({dx}, {dy})"
        );
        ready(mq.finish(1)).unwrap();
    }
}

#[test]
fn one_byte_io_keeps_rows_packed_and_reuses_three_reference_rows() {
    let limits = Limits {
        io_chunk_bytes: 2,
        ..Limits::default()
    };
    let mq_budget = MqBudget::default();
    let table = table(&limits);
    let mut banks = banks(&limits, &mq_budget);
    let layout = banks.layout();
    let mut mq_source = Source::new(&[0xff, 0xac]);
    let mut mq = ready(MqDecoder::new(
        &mut mq_source,
        MqSpan {
            offset: 0,
            length: 2,
        },
        &table,
        banks.mq_contexts_mut(),
        &limits,
        &NeverCancel,
        mq_budget,
    ))
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
        RefinementDecoder::new(&mut mq, layout, &mut sink, &limits, &NeverCancel, budget).unwrap();
    let report = ready(host.decode_bitmap(
        &mut reference_source,
        request(9, 2, reference(9, 3, 0, 0), 0, 0),
    ))
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
    ready(mq.finish(18)).unwrap();
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
        let table = table(&limits);
        let mut banks = banks(&limits, &mq_budget);
        let layout = banks.layout();
        if clear_pixels {
            // With an all-zero reference and previously decoded zero pixels,
            // every decision uses GR context zero. The invented stream takes
            // its MPS path; switching that context's MPS to zero must leave
            // both packed bytes clear, including the seven padding bits.
            banks
                .mq_contexts_mut()
                .set(
                    layout.bitmap_base(),
                    MqContext {
                        state_index: 0,
                        mps: false,
                    },
                )
                .unwrap();
        }
        let mut mq_source = Source::new(&[0xff, 0xac]);
        let mut mq = ready(MqDecoder::new(
            &mut mq_source,
            MqSpan {
                offset: 0,
                length: 2,
            },
            &table,
            banks.mq_contexts_mut(),
            &limits,
            &NeverCancel,
            mq_budget,
        ))
        .unwrap();
        let mut reference_source = Source::new(&[0x00]);
        let mut sink = Sink::new(&[]);
        let mut host = RefinementDecoder::new(
            &mut mq,
            layout,
            &mut sink,
            &limits,
            &NeverCancel,
            RefinementBudget::default(),
        )
        .unwrap();
        let report = ready(host.decode_bitmap(
            &mut reference_source,
            request(width, 1, reference(1, 1, 0, 0), 0, 0),
        ))
        .unwrap();
        assert_eq!(report.target.row_stride, expected.len() as u32);
        assert_eq!(
            report.progress.mq.unwrap().symbols_decoded,
            u64::from(width)
        );
        drop(host);
        assert_eq!(sink.bytes, expected);
        ready(mq.finish(u64::from(width))).unwrap();
    }
}
