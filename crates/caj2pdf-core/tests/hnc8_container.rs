// SPDX-License-Identifier: MIT

use caj2pdf_core::{
    Cancellation, Limits, NeverCancel, RangedSource, SequentialSink,
    hnc8::{Budget, ErrorKind, Hnc8Reader, Variant},
    jbig1::{Type0Budget, Type0Decoder},
    qm::{ArithmeticBudget, ContextBank, QM_STATE_COUNT, QmState, QmTable},
};
use std::{
    cell::Cell,
    error::Error as StdError,
    future::{Future, pending},
    pin::pin,
    task::{Context, Poll, Waker},
};

fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("unexpected pending source"),
    }
}

struct Source {
    bytes: Vec<u8>,
    size: u64,
    max_read: usize,
    overreport: bool,
    zero: bool,
    pending: bool,
    reads: usize,
    largest_request: usize,
}

impl Source {
    fn new(bytes: Vec<u8>) -> Self {
        let size = bytes.len() as u64;
        Self {
            bytes,
            size,
            max_read: usize::MAX,
            overreport: false,
            zero: false,
            pending: false,
            reads: 0,
            largest_request: 0,
        }
    }
}

impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.size
    }

    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        self.reads += 1;
        self.largest_request = self.largest_request.max(destination.len());
        if self.pending {
            pending::<()>().await;
        }
        if self.overreport {
            return Ok(destination.len() + 1);
        }
        if self.zero {
            return Ok(0);
        }
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let count = self
            .bytes
            .len()
            .saturating_sub(start)
            .min(destination.len())
            .min(self.max_read);
        if count > 0 {
            destination[..count].copy_from_slice(&self.bytes[start..start + count]);
        }
        Ok(count)
    }
}

#[derive(Default)]
struct Sink(Vec<u8>);
impl SequentialSink for Sink {
    async fn write(&mut self, bytes: &[u8]) -> caj2pdf_core::Result<usize> {
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    async fn flush(&mut self) -> caj2pdf_core::Result<()> {
        Ok(())
    }
}

struct Flag(Cell<bool>);
impl Cancellation for Flag {
    fn is_cancelled(&self) -> bool {
        self.0.get()
    }
}

fn put_i32(bytes: &mut [u8], offset: usize, value: i32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn put_i16(bytes: &mut [u8], offset: usize, value: i16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}
fn c8(pages: i32) -> Vec<u8> {
    let mut bytes = vec![0; 1024];
    bytes[..4].copy_from_slice(&[0xc8, 0, 0, 0]);
    put_i32(&mut bytes, 8, pages);
    bytes
}
fn hn(variant: Variant, pages: i32, outline: i32) -> (Vec<u8>, usize) {
    let mut bytes = vec![0; 1200];
    bytes[..4].copy_from_slice(b"HN\0\0");
    bytes[4..8].copy_from_slice(match variant {
        Variant::HnA => &[0x90, 1, 0, 0],
        Variant::HnB => &[0xc8, 0, 0, 0],
        Variant::C8 => panic!("test helper requires HN"),
    });
    put_i32(&mut bytes, 0x90, pages);
    if variant == Variant::HnA {
        put_i32(&mut bytes, 0x158, outline);
    }
    (
        bytes,
        if variant == Variant::HnA && outline >= 0 {
            0x15c + 308 * outline as usize
        } else {
            0xd8
        },
    )
}
fn page(bytes: &mut [u8], index: usize, number: usize, text: i32, text_len: i32, images: i16) {
    let row = index + 20 * (number - 1);
    put_i32(bytes, row, text);
    put_i32(bytes, row + 4, text_len);
    put_i16(bytes, row + 8, images);
    // Unknown fields deliberately contain nonzero, even negative-looking bits.
    bytes[row + 10..row + 20].copy_from_slice(&[0xff, 0x80, 1, 2, 3, 4, 5, 6, 7, 8]);
}
fn image(bytes: &mut [u8], descriptor: usize, kind: i32, offset: i32, length: i32) {
    put_i32(bytes, descriptor, kind);
    put_i32(bytes, descriptor + 4, offset);
    put_i32(bytes, descriptor + 8, length);
}
fn error_after_page(bytes: Vec<u8>) -> caj2pdf_core::hnc8::Hnc8Error {
    let mut source = Source::new(bytes);
    let limits = Limits::default();
    let mut reader = ready(Hnc8Reader::open(
        &mut source,
        &limits,
        &NeverCancel,
        Budget::default(),
    ))
    .unwrap();
    ready(reader.next_page()).unwrap();
    ready(reader.next_image()).unwrap_err()
}
fn page_error(bytes: Vec<u8>) -> caj2pdf_core::hnc8::Hnc8Error {
    let mut source = Source::new(bytes);
    let limits = Limits::default();
    let mut reader = ready(Hnc8Reader::open(
        &mut source,
        &limits,
        &NeverCancel,
        Budget::default(),
    ))
    .unwrap();
    ready(reader.next_page()).unwrap_err()
}

#[test]
fn c8_zero_image_page_and_four_types_follow_payload_ends_with_gaps() {
    let mut bytes = c8(2);
    page(&mut bytes, 0x50, 1, 160, 3, 0);
    page(&mut bytes, 0x50, 2, 200, 4, 4);
    image(&mut bytes, 204, 0, 240, 2);
    image(&mut bytes, 242, 1, 270, 3);
    image(&mut bytes, 273, 2, 300, 4);
    image(&mut bytes, 304, 3, 340, 5);
    let mut source = Source::new(bytes);
    source.max_read = 1;
    let limits = Limits {
        io_chunk_bytes: 1,
        ..Limits::default()
    };
    let mut reader = ready(Hnc8Reader::open(
        &mut source,
        &limits,
        &NeverCancel,
        Budget::default(),
    ))
    .unwrap();
    assert_eq!(reader.header().variant, Variant::C8);
    assert_eq!(reader.header().page_index.offset, 0x50);
    assert_eq!(reader.header().page_index.length, 40);
    let first = ready(reader.next_page()).unwrap().unwrap();
    assert_eq!(
        (
            first.page_number,
            first.row_offset,
            first.text.offset,
            first.text.length,
            first.image_count
        ),
        (1, 0x50, 160, 3, 0)
    );
    assert_eq!(first.unknown, [0xff, 0x80, 1, 2, 3, 4, 5, 6, 7, 8]);
    assert!(ready(reader.next_image()).unwrap().is_none());
    let second = ready(reader.next_page()).unwrap().unwrap();
    assert_eq!(
        (
            second.page_number,
            second.row_offset,
            second.text.offset,
            second.text.length,
            second.image_count
        ),
        (2, 0x64, 200, 4, 4)
    );
    for (number, descriptor, kind, offset, length) in [
        (1, 204, 0, 240, 2),
        (2, 242, 1, 270, 3),
        (3, 273, 2, 300, 4),
        (4, 304, 3, 340, 5),
    ] {
        let actual = ready(reader.next_image()).unwrap().unwrap();
        assert_eq!(
            (
                actual.page_number,
                actual.image_number,
                actual.descriptor_offset,
                actual.record_type,
                actual.payload.offset,
                actual.payload.length
            ),
            (2, number, descriptor, kind, offset, length)
        );
        assert_eq!(actual.type0_span().is_some(), kind == 0);
    }
    assert!(ready(reader.next_image()).unwrap().is_none());
    assert!(ready(reader.next_page()).unwrap().is_none());
    assert_eq!(reader.source_mut().largest_request, 1);
}

#[test]
fn hn_a_and_b_have_distinct_checked_index_offsets() {
    for (variant, outline, expected) in [(Variant::HnA, 1, 0x290), (Variant::HnB, 0, 0xd8)] {
        let (mut bytes, index) = hn(variant, 1, outline);
        assert_eq!(index, expected);
        page(&mut bytes, index, 1, 900, 0, 0);
        let mut source = Source::new(bytes);
        let limits = Limits::default();
        let mut reader = ready(Hnc8Reader::open(
            &mut source,
            &limits,
            &NeverCancel,
            Budget::default(),
        ))
        .unwrap();
        assert_eq!(reader.header().variant, variant);
        assert_eq!(reader.header().page_index.offset, expected as u64);
        assert_eq!(
            ready(reader.next_page()).unwrap().unwrap().row_offset,
            expected as u64
        );
    }
}

#[test]
fn text_and_records_may_alias_across_pages_but_each_identity_is_checked() {
    let mut bytes = c8(2);
    // The same descriptor/payload is named independently by two pages.
    page(&mut bytes, 0x50, 1, 200, 0, 1);
    page(&mut bytes, 0x50, 2, 200, 0, 1);
    image(&mut bytes, 200, 0, 240, 2);
    let mut source = Source::new(bytes);
    let limits = Limits::default();
    let mut reader = ready(Hnc8Reader::open(
        &mut source,
        &limits,
        &NeverCancel,
        Budget::default(),
    ))
    .unwrap();
    assert_eq!(ready(reader.next_page()).unwrap().unwrap().page_number, 1);
    assert_eq!(ready(reader.next_image()).unwrap().unwrap().image_number, 1);
    assert_eq!(ready(reader.next_page()).unwrap().unwrap().page_number, 2);
    let second = ready(reader.next_image()).unwrap().unwrap();
    assert_eq!(
        (
            second.page_number,
            second.descriptor_offset,
            second.payload.offset
        ),
        (2, 200, 240)
    );
}

#[test]
fn disjoint_out_of_order_page_intervals_are_accepted() {
    let mut bytes = c8(2);
    page(&mut bytes, 0x50, 1, 400, 0, 1);
    page(&mut bytes, 0x50, 2, 200, 0, 1);
    image(&mut bytes, 400, 0, 440, 2);
    image(&mut bytes, 200, 1, 240, 2);
    let mut source = Source::new(bytes);
    let limits = Limits::default();
    let mut reader = ready(Hnc8Reader::open(
        &mut source,
        &limits,
        &NeverCancel,
        Budget::default(),
    ))
    .unwrap();
    assert_eq!(ready(reader.next_page()).unwrap().unwrap().page_number, 1);
    assert_eq!(
        ready(reader.next_image()).unwrap().unwrap().payload.offset,
        440
    );
    assert_eq!(ready(reader.next_page()).unwrap().unwrap().page_number, 2);
    let later_page = ready(reader.next_image()).unwrap().unwrap();
    assert_eq!(
        (
            later_page.page_number,
            later_page.record_type,
            later_page.payload.offset
        ),
        (2, 1, 240)
    );
}

#[test]
fn type0_handoff_keeps_outer_type_and_full_dib_span() {
    let mut bytes = c8(1);
    page(&mut bytes, 0x50, 1, 180, 0, 1);
    image(&mut bytes, 180, 0, 256, 51);
    let dib = &mut bytes[256..304];
    dib[0..4].copy_from_slice(&40_u32.to_le_bytes());
    dib[4..8].copy_from_slice(&1_u32.to_le_bytes());
    dib[8..12].copy_from_slice(&1_u32.to_le_bytes());
    dib[12..14].copy_from_slice(&1_u16.to_le_bytes());
    dib[14..16].copy_from_slice(&1_u16.to_le_bytes());
    dib[40..43].fill(0xff);
    bytes[304..307].copy_from_slice(&[0x90, 0, 0]);
    let mut source = Source::new(bytes);
    let limits = Limits::default();
    let table = QmTable::new(vec![
        QmState {
            qe: 0x4000,
            next_lps: 0,
            next_mps: 0,
            switch_mps: false
        };
        QM_STATE_COUNT
    ])
    .unwrap();
    let mut contexts = ContextBank::new(1024, &limits).unwrap();
    let mut sink = Sink::default();
    let mut reader = ready(Hnc8Reader::open(
        &mut source,
        &limits,
        &NeverCancel,
        Budget::default(),
    ))
    .unwrap();
    ready(reader.next_page()).unwrap();
    let span = ready(reader.next_image())
        .unwrap()
        .unwrap()
        .type0_span()
        .unwrap();
    assert_eq!((span.record_type, span.offset, span.length), (0, 256, 51));
    let mut decoder = ready(Type0Decoder::new(
        reader.source_mut(),
        span,
        &table,
        &mut contexts,
        &mut sink,
        &limits,
        &NeverCancel,
        ArithmeticBudget {
            max_symbols: 100,
            max_work: 1_000,
        },
        Type0Budget::default(),
    ))
    .unwrap();
    assert!(ready(decoder.decode_next_row()).unwrap());
    ready(decoder.finish()).unwrap();
    // Hand-derived invented MQ state: 0x9000 selects zero control and one pixel.
    assert_eq!(sink.0, [0x80, 0, 0, 0]);
}

#[test]
fn signatures_markers_and_header_counts_are_located() {
    let limits = Limits::default();
    for (bytes, field, offset) in [
        (vec![1, 2, 3, 4], "signature", 0),
        (b"HN\0\0bad!".to_vec(), "HN marker", 4),
    ] {
        let mut source = Source::new(bytes);
        let error = ready(Hnc8Reader::open(
            &mut source,
            &limits,
            &NeverCancel,
            Budget::default(),
        ))
        .err()
        .unwrap();
        assert_eq!((error.kind.field(), error.offset), (field, offset));
        assert!(matches!(error.kind, ErrorKind::Unsupported { .. }));
    }
    for count in [0, -1] {
        let mut source = Source::new(c8(count));
        let error = ready(Hnc8Reader::open(
            &mut source,
            &limits,
            &NeverCancel,
            Budget::default(),
        ))
        .err()
        .unwrap();
        assert_eq!((error.kind.field(), error.offset), ("page count", 8));
    }
    let mut source = Source::new(c8(2));
    let small = Limits {
        max_pages: 1,
        ..limits
    };
    let error = ready(Hnc8Reader::open(
        &mut source,
        &small,
        &NeverCancel,
        Budget::default(),
    ))
    .err()
    .unwrap();
    assert!(matches!(
        error.kind,
        ErrorKind::LimitExceeded {
            resource: "pages",
            limit: 1,
            attempted: 2
        }
    ));
}

#[test]
fn outline_count_negative_limit_and_checked_index_arithmetic() {
    let limits = Limits::default();
    let (mut bytes, _) = hn(Variant::HnA, 1, -1);
    let mut source = Source::new(bytes.clone());
    let error = ready(Hnc8Reader::open(
        &mut source,
        &limits,
        &NeverCancel,
        Budget::default(),
    ))
    .err()
    .unwrap();
    assert_eq!((error.kind.field(), error.offset), ("outline count", 0x158));
    put_i32(&mut bytes, 0x158, 2);
    let mut source = Source::new(bytes.clone());
    let budget = Budget {
        max_outline_records: 1,
        ..Budget::default()
    };
    let error = ready(Hnc8Reader::open(&mut source, &limits, &NeverCancel, budget))
        .err()
        .unwrap();
    assert!(matches!(
        error.kind,
        ErrorKind::LimitExceeded {
            resource: "outline records",
            limit: 1,
            attempted: 2
        }
    ));
    // i32::MAX * 308 is representable in u64, but the checked index start
    // lies far beyond this source and must fail before any seek/allocation.
    put_i32(&mut bytes, 0x158, i32::MAX);
    let mut source = Source::new(bytes);
    let budget = Budget {
        max_outline_records: u32::MAX,
        ..Budget::default()
    };
    let error = ready(Hnc8Reader::open(&mut source, &limits, &NeverCancel, budget))
        .err()
        .unwrap();
    assert_eq!(error.kind.field(), "page index");
    assert_eq!(error.offset, 0x15c_u64 + i32::MAX as u64 * 308);
}

#[test]
fn negative_and_out_of_source_page_fields_fail_even_without_images() {
    for (text, length, count, field, offset) in [
        (-1, 0, 0, "text offset", 0x50),
        (200, -1, 0, "text length", 0x54),
        (200, 0, -1, "image count", 0x58),
        (2000, 0, 0, "text span", 0x50),
        (1000, 100, 0, "text span", 0x50),
    ] {
        let mut bytes = c8(1);
        page(&mut bytes, 0x50, 1, text, length, count);
        let error = page_error(bytes);
        assert_eq!(
            (error.kind.field(), error.offset, error.page),
            (field, offset, Some(1))
        );
    }
}

#[test]
fn page_and_image_budgets_are_distinct_and_located() {
    let mut bytes = c8(2);
    page(&mut bytes, 0x50, 1, 200, 3, 1);
    page(&mut bytes, 0x50, 2, 300, 0, 1);
    image(&mut bytes, 203, 0, 240, 4);
    image(&mut bytes, 300, 0, 340, 4);
    let limits = Limits::default();
    let cases = [
        (
            Budget {
                max_text_span_bytes: 2,
                ..Budget::default()
            },
            "text span bytes",
        ),
        (
            Budget {
                max_images_per_page: 0,
                ..Budget::default()
            },
            "images per page",
        ),
        (
            Budget {
                max_images_total: 0,
                ..Budget::default()
            },
            "images total",
        ),
    ];
    for (budget, expected) in cases {
        let mut source = Source::new(bytes.clone());
        let mut reader =
            ready(Hnc8Reader::open(&mut source, &limits, &NeverCancel, budget)).unwrap();
        assert_eq!(
            ready(reader.next_page()).unwrap_err().kind.field(),
            expected
        );
    }
    let mut source = Source::new(bytes.clone());
    let budget = Budget {
        max_images_total: 1,
        ..Budget::default()
    };
    let mut reader = ready(Hnc8Reader::open(&mut source, &limits, &NeverCancel, budget)).unwrap();
    ready(reader.next_page()).unwrap();
    ready(reader.next_image()).unwrap();
    assert_eq!(
        ready(reader.next_page()).unwrap_err().kind.field(),
        "images total"
    );
    let mut source = Source::new(bytes);
    let budget = Budget {
        max_image_span_bytes: 3,
        ..Budget::default()
    };
    let mut reader = ready(Hnc8Reader::open(&mut source, &limits, &NeverCancel, budget)).unwrap();
    ready(reader.next_page()).unwrap();
    assert_eq!(
        ready(reader.next_image()).unwrap_err().kind.field(),
        "image span bytes"
    );
}

#[test]
fn malformed_descriptors_are_located_and_poison_normal_cursor() {
    for (kind, offset, length, field, error_offset) in [
        (-1, 240, 2, "image type", 200),
        (4, 240, 2, "image type", 200),
        (0, -1, 2, "image offset", 204),
        (0, 240, -1, "image length", 208),
        (0, 240, 0, "image length", 208),
        (0, 205, 2, "image offset", 204),
        (0, 1010, 30, "image payload", 204),
        (0, 40, 2, "image offset", 204),
    ] {
        let mut bytes = c8(1);
        page(&mut bytes, 0x50, 1, 200, 0, 1);
        image(&mut bytes, 200, kind, offset, length);
        let mut source = Source::new(bytes);
        let limits = Limits::default();
        let mut reader = ready(Hnc8Reader::open(
            &mut source,
            &limits,
            &NeverCancel,
            Budget::default(),
        ))
        .unwrap();
        ready(reader.next_page()).unwrap();
        let error = ready(reader.next_image()).unwrap_err();
        assert_eq!(
            (error.kind.field(), error.offset, error.image),
            (field, error_offset, Some(1))
        );
        assert!(matches!(
            ready(reader.next_page()).unwrap_err().kind,
            ErrorKind::Poisoned
        ));
    }
}

#[test]
fn unknown_positive_type_is_refused_before_unmeasured_trailing_fields() {
    let mut bytes = c8(1);
    page(&mut bytes, 0x50, 1, 200, 0, 1);
    image(&mut bytes, 200, 4, -1, -1);
    let error = error_after_page(bytes);
    assert_eq!(
        (error.kind.field(), error.kind.as_str(), error.offset),
        ("image type", "unsupported", 200)
    );
    assert!(matches!(
        error.kind,
        ErrorKind::Unsupported {
            field: "image type",
            value: 4
        }
    ));
}

#[test]
fn descriptor_payload_chain_cannot_regress_on_later_image() {
    let mut bytes = c8(1);
    page(&mut bytes, 0x50, 1, 200, 0, 2);
    image(&mut bytes, 200, 0, 240, 2);
    // Descriptor 2 starts at previous payload end, 242. Its payload would
    // regress into descriptor 1's payload if accepted.
    image(&mut bytes, 242, 1, 241, 3);
    let mut source = Source::new(bytes);
    let limits = Limits::default();
    let mut reader = ready(Hnc8Reader::open(
        &mut source,
        &limits,
        &NeverCancel,
        Budget::default(),
    ))
    .unwrap();
    ready(reader.next_page()).unwrap();
    assert_eq!(
        ready(reader.next_image())
            .unwrap()
            .unwrap()
            .descriptor_offset,
        200
    );
    let error = ready(reader.next_image()).unwrap_err();
    assert_eq!(
        (error.kind.field(), error.offset, error.image),
        ("image offset", 246, Some(2))
    );
}

#[test]
fn protected_descriptor_and_payload_ranges_are_rejected() {
    // Text may reference metadata pending semantic study, but a descriptor
    // may never reinterpret page-index bytes as an image record.
    let mut bytes = c8(1);
    page(&mut bytes, 0x50, 1, 0, 90, 1);
    let error = error_after_page(bytes);
    assert_eq!(error.kind.field(), "image descriptor");
    let mut bytes = c8(1);
    page(&mut bytes, 0x50, 1, 200, 0, 1);
    image(&mut bytes, 200, 0, 10, 2);
    let error = error_after_page(bytes);
    assert_eq!(error.kind.field(), "image offset");
}

#[test]
fn incomplete_page_cannot_silently_drop_images() {
    let mut bytes = c8(1);
    page(&mut bytes, 0x50, 1, 200, 0, 1);
    image(&mut bytes, 200, 0, 240, 2);
    let mut source = Source::new(bytes);
    let limits = Limits::default();
    let mut reader = ready(Hnc8Reader::open(
        &mut source,
        &limits,
        &NeverCancel,
        Budget::default(),
    ))
    .unwrap();
    ready(reader.next_page()).unwrap();
    assert!(matches!(
        ready(reader.next_page()).unwrap_err().kind,
        ErrorKind::IncompletePage
    ));
    assert_eq!(ready(reader.next_image()).unwrap().unwrap().page_number, 1);
    assert!(ready(reader.next_page()).unwrap().is_none());
}

#[test]
fn truncation_and_disrupted_reads_are_located() {
    let limits = Limits::default();
    for (size, field) in [(0, "signature"), (4, "page count"), (0x50, "page index")] {
        let mut bytes = c8(1);
        bytes.truncate(size);
        let mut source = Source::new(bytes);
        let error = ready(Hnc8Reader::open(
            &mut source,
            &limits,
            &NeverCancel,
            Budget::default(),
        ))
        .err()
        .unwrap();
        assert_eq!(error.kind.field(), field);
    }
    let (mut bytes, _) = hn(Variant::HnB, 1, 0);
    bytes.truncate(6);
    let mut source = Source::new(bytes);
    let error = ready(Hnc8Reader::open(
        &mut source,
        &limits,
        &NeverCancel,
        Budget::default(),
    ))
    .err()
    .unwrap();
    assert_eq!((error.kind.field(), error.offset), ("HN marker", 4));
    let (mut bytes, _) = hn(Variant::HnA, 1, 0);
    bytes.truncate(0x15a);
    let mut source = Source::new(bytes);
    let error = ready(Hnc8Reader::open(
        &mut source,
        &limits,
        &NeverCancel,
        Budget::default(),
    ))
    .err()
    .unwrap();
    assert_eq!((error.kind.field(), error.offset), ("outline count", 0x158));
    let mut bytes = c8(1);
    page(&mut bytes, 0x50, 1, 200, 0, 1);
    image(&mut bytes, 200, 0, 240, 2);
    for (size, field) in [
        (200, "image descriptor"),
        (207, "image descriptor"),
        (241, "image payload"),
    ] {
        let mut truncated = bytes.clone();
        truncated.truncate(size);
        let mut source = Source::new(truncated);
        let mut reader = ready(Hnc8Reader::open(
            &mut source,
            &limits,
            &NeverCancel,
            Budget::default(),
        ))
        .unwrap();
        // Zero-length text may end at EOF; each case fails at a later boundary.
        ready(reader.next_page()).unwrap();
        let error = ready(reader.next_image()).unwrap_err();
        assert_eq!(error.kind.field(), field);
    }
    let mut source = Source::new(c8(1));
    source.overreport = true;
    let error = ready(Hnc8Reader::open(
        &mut source,
        &limits,
        &NeverCancel,
        Budget::default(),
    ))
    .err()
    .unwrap();
    assert_eq!(error.kind.field(), "signature");
    assert!(matches!(error.kind, ErrorKind::Source { .. }));
    let mut source = Source::new(c8(1));
    source.zero = true;
    let error = ready(Hnc8Reader::open(
        &mut source,
        &limits,
        &NeverCancel,
        Budget::default(),
    ))
    .err()
    .unwrap();
    assert!(matches!(error.kind, ErrorKind::Truncated { .. }));

    let mut bytes = c8(1);
    page(&mut bytes, 0x50, 1, 200, 0, 1);
    image(&mut bytes, 200, 0, 240, 2);
    let mut source = Source::new(bytes);
    let mut reader = ready(Hnc8Reader::open(
        &mut source,
        &limits,
        &NeverCancel,
        Budget::default(),
    ))
    .unwrap();
    reader.source_mut().overreport = true;
    let error = ready(reader.next_page()).unwrap_err();
    assert_eq!((error.kind.field(), error.page), ("page row", Some(1)));
}

#[test]
fn configured_source_size_and_cancellation_are_checked() {
    let mut bytes = c8(1);
    page(&mut bytes, 0x50, 1, 200, 0, 1);
    image(&mut bytes, 200, 0, 240, 2);
    let mut source = Source::new(bytes.clone());
    let limits = Limits {
        max_input_bytes: 100,
        ..Limits::default()
    };
    let error = ready(Hnc8Reader::open(
        &mut source,
        &limits,
        &NeverCancel,
        Budget::default(),
    ))
    .err()
    .unwrap();
    assert!(matches!(
        error.kind,
        ErrorKind::LimitExceeded {
            resource: "source bytes",
            ..
        }
    ));
    assert_eq!(source.reads, 0);
    let mut source = Source::new(bytes);
    let flag = Flag(Cell::new(false));
    let limits = Limits::default();
    let mut reader = ready(Hnc8Reader::open(
        &mut source,
        &limits,
        &flag,
        Budget::default(),
    ))
    .unwrap();
    ready(reader.next_page()).unwrap();
    flag.0.set(true);
    assert!(matches!(
        ready(reader.next_image()).unwrap_err().kind,
        ErrorKind::Cancelled
    ));
    flag.0.set(false);
    assert_eq!(ready(reader.next_image()).unwrap().unwrap().record_type, 0);
}

#[test]
fn dropped_page_future_poisoning_prevents_cross_page_recovery() {
    let mut bytes = c8(1);
    page(&mut bytes, 0x50, 1, 200, 0, 0);
    let mut source = Source::new(bytes);
    let limits = Limits::default();
    let mut reader = ready(Hnc8Reader::open(
        &mut source,
        &limits,
        &NeverCancel,
        Budget::default(),
    ))
    .unwrap();
    reader.source_mut().pending = true;
    {
        let mut future = pin!(reader.next_page());
        let mut context = Context::from_waker(Waker::noop());
        assert!(matches!(future.as_mut().poll(&mut context), Poll::Pending));
    }
    assert!(matches!(
        ready(reader.next_page()).unwrap_err().kind,
        ErrorKind::Poisoned
    ));
}

#[test]
fn dropped_image_future_poisoning_prevents_later_record_attachment() {
    let mut bytes = c8(1);
    page(&mut bytes, 0x50, 1, 200, 0, 1);
    image(&mut bytes, 200, 0, 240, 2);
    let mut source = Source::new(bytes);
    let limits = Limits::default();
    let mut reader = ready(Hnc8Reader::open(
        &mut source,
        &limits,
        &NeverCancel,
        Budget::default(),
    ))
    .unwrap();
    ready(reader.next_page()).unwrap();
    reader.source_mut().pending = true;
    {
        let mut future = pin!(reader.next_image());
        let mut context = Context::from_waker(Waker::noop());
        assert!(matches!(future.as_mut().poll(&mut context), Poll::Pending));
    }
    assert!(matches!(
        ready(reader.next_image()).unwrap_err().kind,
        ErrorKind::Poisoned
    ));
    assert!(matches!(
        ready(reader.next_page()).unwrap_err().kind,
        ErrorKind::Poisoned
    ));
}

#[test]
fn fixed_seed_mutations_terminate_under_fixed_record_budget() {
    let mut template = c8(2);
    page(&mut template, 0x50, 1, 200, 0, 1);
    page(&mut template, 0x50, 2, 300, 0, 1);
    image(&mut template, 200, 0, 240, 2);
    image(&mut template, 300, 1, 340, 2);
    let mut state = 0x5eed_1234_u32;
    for _ in 0..128 {
        let mut bytes = template.clone();
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let positions = [
            8, 0x50, 0x54, 0x58, 0x64, 0x68, 0x6c, 200, 204, 208, 300, 304, 308,
        ];
        let at = positions[state as usize % positions.len()];
        bytes[at] ^= (state >> 8) as u8;
        let mut source = Source::new(bytes);
        let limits = Limits {
            max_pages: 4,
            io_chunk_bytes: 4,
            ..Limits::default()
        };
        let budget = Budget {
            max_images_per_page: 4,
            max_images_total: 8,
            ..Budget::default()
        };
        if let Ok(mut reader) = ready(Hnc8Reader::open(&mut source, &limits, &NeverCancel, budget))
        {
            for _ in 0..4 {
                match ready(reader.next_page()) {
                    Ok(Some(page)) => {
                        for _ in 0..page.image_count.min(4) {
                            if ready(reader.next_image()).is_err() {
                                break;
                            }
                        }
                    }
                    _ => break,
                }
            }
        }
        assert!(source.reads <= 200, "mutation exceeded fixed work budget");
    }
}

#[test]
fn public_error_contract_preserves_variant_location_field_and_cause() {
    assert_eq!(Variant::C8.as_str(), "C8");
    assert_eq!(Variant::HnA.as_str(), "HN-A");
    assert_eq!(Variant::HnB.as_str(), "HN-B");
    let limits = Limits::default();
    let mut source = Source::new(c8(0));
    let malformed = ready(Hnc8Reader::open(
        &mut source,
        &limits,
        &NeverCancel,
        Budget::default(),
    ))
    .err()
    .unwrap();
    assert_eq!(
        (malformed.kind.field(), malformed.kind.as_str()),
        ("page count", "malformed")
    );
    assert!(malformed.to_string().contains("HN/C8 C8 at byte 8"));
    assert!(StdError::source(&malformed).is_none());

    let mut source = Source::new(vec![0xff; 4]);
    let unsupported = ready(Hnc8Reader::open(
        &mut source,
        &limits,
        &NeverCancel,
        Budget::default(),
    ))
    .err()
    .unwrap();
    assert_eq!(
        (unsupported.kind.field(), unsupported.kind.as_str()),
        ("signature", "unsupported")
    );
    assert!(unsupported.to_string().contains("HN/C8 at byte 0"));

    let mut source = Source::new(c8(1));
    source.overreport = true;
    let source_error = ready(Hnc8Reader::open(
        &mut source,
        &limits,
        &NeverCancel,
        Budget::default(),
    ))
    .err()
    .unwrap();
    assert_eq!(
        (source_error.kind.field(), source_error.kind.as_str()),
        ("signature", "source")
    );
    assert!(source_error.to_string().contains("source error"));
    assert!(StdError::source(&source_error).is_some());

    let mut bytes = c8(1);
    page(&mut bytes, 0x50, 1, 1000, 100, 0);
    let truncated = page_error(bytes);
    assert_eq!(
        (truncated.kind.field(), truncated.kind.as_str()),
        ("text span", "truncated")
    );
    assert!(truncated.to_string().contains("page 1"));

    let mut source = Source::new(c8(2));
    let small = Limits {
        max_pages: 1,
        ..limits
    };
    let limited = ready(Hnc8Reader::open(
        &mut source,
        &small,
        &NeverCancel,
        Budget::default(),
    ))
    .err()
    .unwrap();
    assert_eq!(
        (limited.kind.field(), limited.kind.as_str()),
        ("pages", "limit")
    );
    assert!(limited.to_string().contains("limit 1 exceeded by 2"));

    let flag = Flag(Cell::new(true));
    let mut source = Source::new(c8(1));
    let cancelled = ready(Hnc8Reader::open(
        &mut source,
        &limits,
        &flag,
        Budget::default(),
    ))
    .err()
    .unwrap();
    assert_eq!(
        (cancelled.kind.field(), cancelled.kind.as_str()),
        ("cancellation", "cancelled")
    );
    assert!(cancelled.to_string().contains("cancelled"));

    let mut bytes = c8(1);
    page(&mut bytes, 0x50, 1, 200, 0, 1);
    image(&mut bytes, 200, 0, 240, 2);
    let mut source = Source::new(bytes);
    let mut reader = ready(Hnc8Reader::open(
        &mut source,
        &limits,
        &NeverCancel,
        Budget::default(),
    ))
    .unwrap();
    let no_page = ready(reader.next_image()).unwrap_err();
    assert_eq!(
        (no_page.kind.field(), no_page.kind.as_str()),
        ("page cursor", "no_current_page")
    );
    assert!(no_page.to_string().contains("no current page"));
    ready(reader.next_page()).unwrap();
    let incomplete = ready(reader.next_page()).unwrap_err();
    assert_eq!(
        (incomplete.kind.field(), incomplete.kind.as_str()),
        ("image count", "incomplete_page")
    );
    assert_eq!(
        (incomplete.page, incomplete.image, incomplete.offset),
        (Some(1), Some(1), 200)
    );
    assert!(incomplete.to_string().contains("page 1, image 1"));
    reader.source_mut().overreport = true;
    assert!(ready(reader.next_image()).is_err());
    let poisoned = ready(reader.next_image()).unwrap_err();
    assert_eq!(
        (poisoned.kind.field(), poisoned.kind.as_str()),
        ("reader state", "poisoned")
    );
    assert!(poisoned.to_string().contains("reader is poisoned"));
}

#[test]
fn independent_page_probe_can_report_later_errors_without_recovery() {
    let mut bytes = c8(2);
    page(&mut bytes, 0x50, 1, 200, 0, -1);
    page(&mut bytes, 0x50, 2, 300, 0, 1);
    image(&mut bytes, 300, 0, 340, 2);
    let mut source = Source::new(bytes);
    let limits = Limits::default();
    {
        let mut bad_page = ready(Hnc8Reader::open(
            &mut source,
            &limits,
            &NeverCancel,
            Budget::default(),
        ))
        .unwrap();
        assert_eq!(ready(bad_page.next_page()).unwrap_err().page, Some(1));
    }
    {
        let mut probe = ready(Hnc8Reader::probe_at_page(
            &mut source,
            &limits,
            &NeverCancel,
            Budget::default(),
            2,
        ))
        .unwrap();
        assert_eq!(ready(probe.next_page()).unwrap().unwrap().page_number, 2);
        assert_eq!(ready(probe.next_image()).unwrap().unwrap().page_number, 2);
        assert!(ready(probe.next_image()).unwrap().is_none());
    }
    for page_number in [0, 3] {
        let error = ready(Hnc8Reader::probe_at_page(
            &mut source,
            &limits,
            &NeverCancel,
            Budget::default(),
            page_number,
        ))
        .err()
        .unwrap();
        assert_eq!(
            (error.kind.field(), error.kind.as_str()),
            ("page number", "malformed")
        );
    }
}
