// SPDX-License-Identifier: MIT

//! Original fixtures exercise the public conversion path with synthetic text
//! and caller-owned QM states; no external document or table bytes are used.

use super::*;
use crate::NeverCancel;
use crate::hnc8::{OutlineDefect, OutlineRepair};
use flate2::{Compression, write::ZlibEncoder};
use std::{
    cell::Cell,
    future::Future,
    io::{self, Write},
    pin::pin,
    rc::Rc,
    task::{Context, Poll, Waker},
};

const PREFIX: [u8; 20] = *b"\x03\x80\x64\x00\x03\x80\xc8\x00COMPRESSTEXT";

fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("synthetic adapters must complete immediately"),
    }
}

struct Flag(Rc<Cell<bool>>);

impl Cancellation for Flag {
    fn is_cancelled(&self) -> bool {
        self.0.get()
    }
}

#[derive(Clone)]
struct Record {
    kind: i32,
    bytes: Vec<u8>,
    coordinate: RawTextCoordinate,
}

impl Record {
    fn jpeg(width: u16, height: u16, sample: u8, x: u16, y: u16) -> Self {
        Self {
            kind: 2,
            bytes: jpeg(width, height, sample),
            coordinate: RawTextCoordinate {
                x,
                y,
                width: 80,
                height: 40,
            },
        }
    }

    fn type0(rows: &[Vec<bool>], x: u16, y: u16) -> Self {
        Self {
            kind: 0,
            bytes: type0(rows),
            coordinate: RawTextCoordinate {
                x,
                y,
                width: 80,
                height: 40,
            },
        }
    }
}

struct Fixture {
    bytes: Vec<u8>,
    index: usize,
    text_offsets: Vec<usize>,
    descriptors: Vec<Vec<u64>>,
    payloads: Vec<Vec<u64>>,
}

fn text(records: &[Record]) -> Vec<u8> {
    let mut plain = vec![0x19; 28 + records.len() * 28];
    for (slot, marker) in [(0, 0x8070_u16), (4, 0x8071), (8, 0x8001)] {
        plain[8 + slot..10 + slot].copy_from_slice(&marker.to_le_bytes());
    }
    for (number, record) in records.iter().enumerate() {
        let at = 28 + number * 28;
        plain[at..at + 2].copy_from_slice(&record.coordinate.x.to_le_bytes());
        plain[at + 2..at + 4].copy_from_slice(&record.coordinate.y.to_le_bytes());
        plain[at + 4..at + 6].copy_from_slice(&record.coordinate.width.to_le_bytes());
        plain[at + 6..at + 8].copy_from_slice(&record.coordinate.height.to_le_bytes());
    }
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&plain).unwrap();
    let mut result = PREFIX.to_vec();
    result.extend((plain.len() as u32).to_le_bytes());
    result.extend(encoder.finish().unwrap());
    result
}

fn image_records(records: &[Record]) -> Vec<u8> {
    let mut plain = Vec::new();
    for record in records {
        let mut image = [0_u8; 28];
        image[..2].copy_from_slice(&0x800a_u16.to_le_bytes());
        image[4..6].copy_from_slice(&record.coordinate.x.to_le_bytes());
        image[6..8].copy_from_slice(&record.coordinate.y.to_le_bytes());
        image[8..10].copy_from_slice(&record.coordinate.width.to_le_bytes());
        image[10..12].copy_from_slice(&record.coordinate.height.to_le_bytes());
        plain.extend(image);
    }
    plain.extend([4, 0x80, 0, 0]);
    plain
}

fn direct_text(records: &[Record]) -> Vec<u8> {
    let plain = image_records(records);
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&plain).unwrap();
    let mut result = b"COMPRESSTEXT".to_vec();
    result.extend((plain.len() as u32).to_le_bytes());
    result.extend(encoder.finish().unwrap());
    result
}

fn fixture(variant: Variant, pages: &[Vec<Record>]) -> Fixture {
    fixture_with_text(variant, pages, text)
}

fn fixture_with_text(
    variant: Variant,
    pages: &[Vec<Record>],
    text: fn(&[Record]) -> Vec<u8>,
) -> Fixture {
    let (count_at, index) = match variant {
        Variant::C8 => (8, 0x50),
        Variant::HnA => (0x90, 0x15c),
        Variant::HnB => (0x90, 0xd8),
    };
    let mut bytes = vec![0; index + pages.len() * 20];
    match variant {
        Variant::C8 => bytes[..4].copy_from_slice(&[0xc8, 0, 0, 0]),
        Variant::HnA | Variant::HnB => {
            bytes[..4].copy_from_slice(b"HN\0\0");
            bytes[4..8].copy_from_slice(match variant {
                Variant::HnA => &[0x90, 1, 0, 0],
                _ => &[0xc8, 0, 0, 0],
            });
        }
    }
    if variant == Variant::HnB {
        bytes[0x88..0x8c].copy_from_slice(&0xc8_u32.to_le_bytes());
    }
    bytes[count_at + 4] = 2; // Authored native records use mode 2.
    bytes[count_at..count_at + 4].copy_from_slice(&(pages.len() as i32).to_le_bytes());
    bytes[count_at + 24..count_at + 26].copy_from_slice(&100_u16.to_le_bytes());
    bytes[count_at + 26..count_at + 28].copy_from_slice(&200_u16.to_le_bytes());
    let mut result = Fixture {
        bytes,
        index,
        text_offsets: Vec::new(),
        descriptors: Vec::new(),
        payloads: Vec::new(),
    };
    for (number, records) in pages.iter().enumerate() {
        let offset = result.bytes.len();
        let page_text = if variant == Variant::HnB {
            b"original opaque HN-B text".to_vec()
        } else {
            text(records)
        };
        result.bytes.extend(&page_text);
        let row = index + number * 20;
        result.bytes[row..row + 4].copy_from_slice(&(offset as i32).to_le_bytes());
        result.bytes[row + 4..row + 8].copy_from_slice(&(page_text.len() as i32).to_le_bytes());
        result.bytes[row + 8..row + 10].copy_from_slice(&(records.len() as i16).to_le_bytes());
        result.text_offsets.push(offset);
        let mut descriptors = Vec::new();
        let mut payloads = Vec::new();
        for record in records {
            let descriptor = result.bytes.len();
            let payload = descriptor + 12;
            result.bytes.extend(record.kind.to_le_bytes());
            result.bytes.extend((payload as i32).to_le_bytes());
            result
                .bytes
                .extend((record.bytes.len() as i32).to_le_bytes());
            result.bytes.extend(&record.bytes);
            descriptors.push(descriptor as u64);
            payloads.push(payload as u64);
        }
        result.descriptors.push(descriptors);
        result.payloads.push(payloads);
    }
    result
}

fn segment(marker: u8, body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0xff, marker];
    bytes.extend(u16::try_from(body.len() + 2).unwrap().to_be_bytes());
    bytes.extend(body);
    bytes
}

#[derive(Default)]
struct Bits {
    bytes: Vec<u8>,
    byte: u8,
    used: u8,
}

impl Bits {
    fn put(&mut self, value: u16, bits: u8) {
        for shift in (0..bits).rev() {
            self.byte = (self.byte << 1) | ((value >> shift) as u8 & 1);
            self.used += 1;
            if self.used == 8 {
                self.bytes.push(self.byte);
                if self.byte == 0xff {
                    self.bytes.push(0);
                }
                self.byte = 0;
                self.used = 0;
            }
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.used > 0 {
            self.put((1 << (8 - self.used)) - 1, 8 - self.used);
        }
        self.bytes
    }
}

/// Original constant-gray baseline JPEG. Custom DC codes are four bits,
/// AC uses one EOB bit, and each constant block has only a DC coefficient.
fn jpeg(width: u16, height: u16, sample: u8) -> Vec<u8> {
    let mut bytes = vec![0xff, 0xd8];
    bytes.extend(segment(0xe0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0"));
    bytes.extend(segment(0xfe, b"original composition fixture"));
    bytes.extend(segment(0xdb, &[&[0_u8][..], &[1_u8; 64][..]].concat()));
    let [h0, h1] = height.to_be_bytes();
    let [w0, w1] = width.to_be_bytes();
    bytes.extend(segment(0xc0, &[8, h0, h1, w0, w1, 1, 1, 0x11, 0]));
    let mut dc = vec![0, 0, 0, 0, 12];
    dc.extend([0; 12]);
    dc.extend(0..12);
    bytes.extend(segment(0xc4, &dc));
    let mut ac = vec![0x10, 1];
    ac.extend([0; 15]);
    ac.push(0);
    bytes.extend(segment(0xc4, &ac));
    bytes.extend(segment(0xda, &[1, 1, 0, 0, 63, 0]));
    let mut bits = Bits::default();
    let mut previous = 0;
    for _ in 0..u32::from(width.div_ceil(8)) * u32::from(height.div_ceil(8)) {
        let value = 8 * (i32::from(sample) - 128);
        let difference = value - previous;
        previous = value;
        let category = (32 - difference.unsigned_abs().leading_zeros()) as u8;
        bits.put(u16::from(category), 4);
        if category > 0 {
            let amplitude = if difference < 0 {
                difference + (1 << category) - 1
            } else {
                difference
            };
            bits.put(amplitude as u16, category);
        }
        bits.put(0, 1);
    }
    bytes.extend(bits.finish());
    bytes.extend([0xff, 0xd9]);
    bytes
}

fn table() -> QmTable {
    use crate::qm::{QM_STATE_COUNT, QmState};
    QmTable::new(vec![
        QmState {
            qe: 0x4000,
            next_mps: 0,
            next_lps: 0,
            switch_mps: false,
        };
        QM_STATE_COUNT
    ])
    .unwrap()
}

/// Original interval encoder for the invented constant table above. Every
/// context stays in state zero with MPS zero, so only decisions are needed.
fn arithmetic(decisions: impl IntoIterator<Item = bool>) -> Vec<u8> {
    let mut low = Vec::<u8>::new();
    let mut interval = 0x1_0000_u32;
    let mut shift = 0;
    for bit in decisions {
        let narrowed = interval - 0x4000;
        if bit != (narrowed >= 0x4000) {
            interval = narrowed;
        } else {
            low.resize(low.len().max(16 + shift), 0);
            for j in 0..16 {
                if narrowed & (1 << j) != 0 {
                    let mut index = 15 + shift - j;
                    loop {
                        low[index] += 1;
                        if low[index] < 2 {
                            break;
                        }
                        low[index] = 0;
                        index -= 1;
                    }
                }
            }
            interval = 0x4000;
        }
        while interval < 0x8000 {
            interval <<= 1;
            shift += 1;
        }
    }
    let mut bytes = low
        .chunks(8)
        .map(|chunk| {
            chunk
                .iter()
                .enumerate()
                .fold(0, |byte, (index, bit)| byte | (bit << (7 - index)))
        })
        .collect::<Vec<_>>();
    while bytes.last() == Some(&0) {
        bytes.pop();
    }
    if bytes.is_empty() {
        bytes.push(0);
    }
    bytes
}

fn type0(rows: &[Vec<bool>]) -> Vec<u8> {
    let mut bytes = vec![0; 48];
    bytes[..4].copy_from_slice(&40_u32.to_le_bytes());
    bytes[4..8].copy_from_slice(&(rows[0].len() as i32).to_le_bytes());
    bytes[8..12].copy_from_slice(&(rows.len() as i32).to_le_bytes());
    bytes[12..14].copy_from_slice(&1_u16.to_le_bytes());
    bytes[14..16].copy_from_slice(&1_u16.to_le_bytes());
    bytes[32..36].copy_from_slice(&2_u32.to_le_bytes());
    bytes[40..43].fill(0xff);
    // Always choose the per-pixel row path. Constant invented states make
    // the context values immaterial without bypassing production contexts.
    bytes
        .extend(arithmetic(rows.iter().flat_map(|row| {
            std::iter::once(false).chain(row.iter().copied())
        })));
    bytes
}

fn rows(width: usize) -> Vec<Vec<bool>> {
    (0..3)
        .map(|y| (0..width).map(|x| x == y || x + y + 1 == width).collect())
        .collect()
}

fn reversed_packed(rows: &[Vec<bool>]) -> Vec<u8> {
    let stride = rows[0].len().div_ceil(8);
    let mut packed = vec![0; stride * rows.len()];
    for (number, row) in rows.iter().rev().enumerate() {
        for (x, bit) in row.iter().enumerate() {
            if *bit {
                packed[number * stride + x / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    packed
}

#[derive(Clone, Copy, Debug)]
enum Fault {
    Zero,
    Overreport,
    Io,
}

fn fault(fault: Fault, requested: usize) -> crate::Result<usize> {
    match fault {
        Fault::Zero => Ok(0),
        Fault::Overreport => Ok(requested + 1),
        Fault::Io => Err(Error::Io(io::Error::other("original synthetic fault"))),
    }
}

struct Source {
    bytes: Vec<u8>,
    short: usize,
    max_request: usize,
    fault_at: Option<(u64, Fault)>,
    payload_start: Option<u64>,
    payload_passes: usize,
    mutate_at_pass: Option<(usize, usize)>,
}

impl Source {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            short: usize::MAX,
            max_request: 0,
            fault_at: None,
            payload_start: None,
            payload_passes: 0,
            mutate_at_pass: None,
        }
    }
}

impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> crate::Result<usize> {
        self.max_request = self.max_request.max(destination.len());
        if self.fault_at.is_some_and(|(at, _)| offset >= at) {
            return fault(self.fault_at.unwrap().1, destination.len());
        }
        if self.payload_start == Some(offset) {
            self.payload_passes += 1;
            if let Some((pass, index)) = self.mutate_at_pass
                && pass == self.payload_passes
            {
                self.bytes[index] ^= 1;
            }
        }
        let start = usize::try_from(offset).unwrap();
        let count = destination
            .len()
            .min(self.short)
            .min(self.bytes.len().saturating_sub(start));
        if count > 0 {
            destination[..count].copy_from_slice(&self.bytes[start..start + count]);
        }
        Ok(count)
    }
}

#[derive(Default)]
struct Sink {
    bytes: Vec<u8>,
    short: Option<usize>,
    max_request: usize,
    fail_after: Option<usize>,
    cancel: Option<Rc<Cell<bool>>>,
    fail_flush: bool,
}

impl SequentialSink for Sink {
    async fn write(&mut self, bytes: &[u8]) -> crate::Result<usize> {
        self.max_request = self.max_request.max(bytes.len());
        if self.fail_after.is_some_and(|at| self.bytes.len() >= at) {
            return Err(Error::Io(io::Error::other("synthetic PDF sink failure")));
        }
        let count = bytes.len().min(self.short.unwrap_or(usize::MAX));
        self.bytes.extend(&bytes[..count]);
        if let Some(flag) = &self.cancel {
            flag.set(true);
        }
        Ok(count)
    }

    async fn flush(&mut self) -> crate::Result<()> {
        if self.fail_flush {
            Err(Error::Io(io::Error::other("synthetic PDF flush failure")))
        } else {
            Ok(())
        }
    }
}

#[derive(Default)]
struct Scratch {
    bytes: Vec<u8>,
    peak: usize,
    max_request: usize,
    short: Option<usize>,
    read_fault: Option<Fault>,
    write_fault: Option<Fault>,
    fail_initialize: bool,
    fail_flush: bool,
    fail_cleanup: bool,
    cancel_write: Option<Rc<Cell<bool>>>,
    cancel_read: Option<Rc<Cell<bool>>>,
    written: u64,
    read: u64,
}

impl RandomAccessScratch for Scratch {
    fn size(&self) -> crate::Result<u64> {
        Ok(self.bytes.len() as u64)
    }

    async fn set_len(&mut self, bytes: u64) -> crate::Result<()> {
        if (bytes > 0 && self.fail_initialize) || (bytes == 0 && self.fail_cleanup && self.peak > 0)
        {
            return Err(Error::Io(io::Error::other(
                "synthetic scratch resize failure",
            )));
        }
        self.bytes.resize(usize::try_from(bytes).unwrap(), 0);
        self.peak = self.peak.max(self.bytes.len());
        Ok(())
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> crate::Result<usize> {
        self.max_request = self.max_request.max(destination.len());
        if let Some(mode) = self.read_fault {
            return fault(mode, destination.len());
        }
        let start = usize::try_from(offset).unwrap();
        assert!(start <= self.bytes.len());
        let count = destination
            .len()
            .min(self.short.unwrap_or(usize::MAX))
            .min(self.bytes.len() - start);
        destination[..count].copy_from_slice(&self.bytes[start..start + count]);
        self.read += count as u64;
        if let Some(flag) = &self.cancel_read {
            flag.set(true);
        }
        Ok(count)
    }

    async fn write_at(&mut self, offset: u64, bytes: &[u8]) -> crate::Result<usize> {
        self.max_request = self.max_request.max(bytes.len());
        if let Some(mode) = self.write_fault {
            return fault(mode, bytes.len());
        }
        let start = usize::try_from(offset).unwrap();
        let count = bytes.len().min(self.short.unwrap_or(usize::MAX));
        assert!(start + count <= self.bytes.len());
        self.bytes[start..start + count].copy_from_slice(&bytes[..count]);
        self.written += count as u64;
        if let Some(flag) = &self.cancel_write {
            flag.set(true);
        }
        Ok(count)
    }

    async fn flush(&mut self) -> crate::Result<()> {
        if self.fail_flush {
            Err(Error::Io(io::Error::other(
                "synthetic scratch flush failure",
            )))
        } else {
            Ok(())
        }
    }
}

#[derive(Default)]
struct Visitor {
    mappings: Vec<(u32, Option<u32>)>,
    images: Vec<(ImageRecord, u32, u32, u32, [f64; 6])>,
    sizes: Vec<Option<PageSpec>>,
    fail: bool,
    cancel: Option<Rc<Cell<bool>>>,
}

impl ComposeVisitor for Visitor {
    async fn page(&mut self, page: ComposePage<'_>) -> crate::Result<()> {
        self.mappings
            .push((page.source.page_number, page.output_page));
        self.sizes.push(page.size);
        for image in page.images {
            self.images.push((
                image.record,
                image.visible_width,
                image.display_width,
                image.height,
                image.transform,
            ));
        }
        if let Some(flag) = &self.cancel {
            flag.set(true);
        }
        if self.fail {
            Err(Error::Io(io::Error::other("synthetic visitor failure")))
        } else {
            Ok(())
        }
    }
}

fn convert(
    source: &mut Source,
    sink: &mut Sink,
    table: Option<&QmTable>,
    scratch: &mut Scratch,
    visitor: &mut Visitor,
    options: ComposeOptions,
    limits: &Limits,
) -> Result<ComposeReport, ComposeError> {
    ready(convert_source_pages_pdf(
        source,
        sink,
        table,
        scratch,
        visitor,
        options,
        limits,
        &NeverCancel,
    ))
}

fn contains(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|part| part == needle)
}

struct Harness {
    fixture: Fixture,
    source: Source,
    sink: Sink,
    scratch: Scratch,
    visitor: Visitor,
}

impl Harness {
    fn new(variant: Variant, pages: &[Vec<Record>]) -> Self {
        let mut fixture = fixture(variant, pages);
        let source = Source::new(std::mem::take(&mut fixture.bytes));
        Self {
            fixture,
            source,
            sink: Sink::default(),
            scratch: Scratch::default(),
            visitor: Visitor::default(),
        }
    }

    fn type0() -> Self {
        Self::new(Variant::C8, &[vec![Record::type0(&rows(9), 0, 0)]])
    }

    fn run(
        &mut self,
        table: Option<&QmTable>,
        options: ComposeOptions,
        limits: &Limits,
    ) -> Result<ComposeReport, ComposeError> {
        convert(
            &mut self.source,
            &mut self.sink,
            table,
            &mut self.scratch,
            &mut self.visitor,
            options,
            limits,
        )
    }

    fn cancelled(&mut self, flag: Rc<Cell<bool>>) -> Result<ComposeReport, ComposeError> {
        ready(convert_source_pages_pdf(
            &mut self.source,
            &mut self.sink,
            Some(&table()),
            &mut self.scratch,
            &mut self.visitor,
            ComposeOptions::default(),
            &Limits::default(),
            &Flag(flag),
        ))
    }
}

fn located(error: &ComposeError, variant: Variant, page: Option<u32>, image: Option<u32>) {
    assert_eq!(error.variant, Some(variant));
    assert_eq!(error.page, page);
    assert_eq!(error.image, image);
    assert!(error.offset.is_some());
}

#[test]
fn hnb_maps_all_six_rows_without_a_table_or_scratch_activity() {
    let first = Record::jpeg(16, 8, 55, 99, 99);
    let last = Record::jpeg(8, 16, 205, 99, 99);
    let built = fixture(
        Variant::HnB,
        &[
            vec![first.clone()],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![last.clone()],
        ],
    );
    let mut source = Source::new(built.bytes);
    let mut sink = Sink::default();
    let mut scratch = Scratch::default();
    let mut visitor = Visitor::default();
    let report = ready(convert_source_pages_pdf(
        &mut source,
        &mut sink,
        None,
        &mut scratch,
        &mut visitor,
        ComposeOptions::default(),
        &Limits::default(),
        &NeverCancel,
    ))
    .unwrap();
    assert_eq!(
        visitor.mappings,
        [
            (1, Some(1)),
            (2, None),
            (3, None),
            (4, None),
            (5, None),
            (6, Some(2))
        ]
    );
    assert_eq!(report.source_variant, Variant::HnB);
    assert_eq!(report.source_pages, 6);
    assert_eq!(report.output_pages, 2);
    assert_eq!(report.no_image_pages, 4);
    assert_eq!(report.type0_images, 0);
    assert_eq!(report.jpeg_images, 2);
    assert_eq!(report.conversion.pages_converted, 2);
    assert_eq!(
        report.conversion.output_bytes_written,
        sink.bytes.len() as u64
    );
    assert_eq!(visitor.images[0].4, [3.84, 0.0, 0.0, -1.92, 0.0, 1.92]);
    assert_eq!(visitor.images[1].4, [1.92, 0.0, 0.0, -3.84, 0.0, 3.84]);
    assert_eq!(visitor.sizes[1..5], [None; 4]);
    assert!(contains(&sink.bytes, &first.bytes));
    assert!(contains(&sink.bytes, &last.bytes));
    assert_eq!(scratch.peak, 0);
    assert_eq!(scratch.max_request, 0);
    assert_eq!(report.peak_row_store_bytes, 0);
}

#[test]
fn invented_hna_c8_text_preserves_order_overlap_repeats_and_off_page_positions() {
    for variant in [Variant::HnA, Variant::C8] {
        let first = Record::jpeg(16, 16, 50, 0, 0);
        let mut second = Record::jpeg(8, 8, 210, 17, 3);
        second.kind = 1;
        let repeated = second.clone();
        let outside = Record::jpeg(8, 8, 125, u16::MAX, u16::MAX);
        let built = fixture(variant, &[vec![first, second, repeated, outside]]);
        let expected_descriptors = built.descriptors[0].clone();
        let mut source = Source::new(built.bytes);
        let mut sink = Sink::default();
        let mut scratch = Scratch::default();
        let mut visitor = Visitor::default();
        let report = convert(
            &mut source,
            &mut sink,
            None,
            &mut scratch,
            &mut visitor,
            ComposeOptions::default(),
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(report.output_pages, 1);
        assert_eq!(report.jpeg_images, 4);
        assert_eq!(
            visitor
                .images
                .iter()
                .map(|image| image.0.descriptor_offset)
                .collect::<Vec<_>>(),
            expected_descriptors
        );
        assert_eq!(visitor.images[1].4, visitor.images[2].4);
        let scale = 240.0 / 2473.0;
        assert_eq!(
            visitor.images[1].4,
            [
                80.0 * scale,
                0.0,
                0.0,
                -40.0 * scale,
                17.0 * scale,
                200.0 * scale - 3.0 * scale
            ]
        );
        assert!(visitor.images[3].4[4] > 3.84);
        assert!(visitor.images[3].4[5] < 0.0);
        let content = crate::test_support::pdf_text(&sink.bytes);
        let locations = (0..4)
            .map(|number| content.find(&format!("/Im{number} Do")).unwrap())
            .collect::<Vec<_>>();
        assert!(locations.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(scratch.peak, 0);
    }
}

#[test]
fn mixed_images_preserve_exact_padding_and_asymmetric_reversed_rows() {
    for width in [1, 7, 8, 9, 24, 25, 28, 31, 32, 33] {
        let pixels = rows(width);
        let bilevel = Record::type0(&pixels, 0, 0);
        let jpeg = Record::jpeg(8, 8, 155, 9, 0);
        let built = fixture(
            Variant::C8,
            &[vec![bilevel, jpeg, Record::type0(&pixels, 13, 1)]],
        );
        let mut source = Source::new(built.bytes);
        let mut sink = Sink::default();
        let mut scratch = Scratch::default();
        let mut visitor = Visitor::default();
        let report = convert(
            &mut source,
            &mut sink,
            Some(&table()),
            &mut scratch,
            &mut visitor,
            ComposeOptions::default(),
            &Limits::default(),
        )
        .unwrap();
        let display_width = width;
        assert_eq!(visitor.images[0].1, width as u32);
        assert_eq!(visitor.images[0].2, display_width as u32);
        assert_eq!(
            visitor.sizes[0].unwrap().width_points,
            100.0 * (240.0 / 2473.0)
        );
        assert_eq!(visitor.images[0].4[3], -40.0 * (240.0 / 2473.0));
        let expected = reversed_packed(&pixels);
        assert!(
            crate::test_support::bilevel_pixels(&sink.bytes).contains(&expected),
            "width {width}"
        );
        assert!(contains(
            &sink.bytes,
            format!("/Width {display_width}\n/Height 3\n").as_bytes()
        ));
        assert_eq!(report.type0_images, 2);
        assert_eq!(report.jpeg_images, 1);
        let scratch_bytes = width.div_ceil(32) * 4 * pixels.len();
        assert_eq!(report.peak_row_store_bytes, scratch_bytes as u64);
        assert_eq!(report.row_store_read_bytes, 2 * scratch_bytes as u64);
        assert_eq!(report.row_store_written_bytes, 2 * scratch_bytes as u64);
        assert_eq!(scratch.peak, scratch_bytes);
        assert!(scratch.bytes.is_empty());
    }
}

#[test]
fn short_source_scratch_and_sink_calls_stay_bounded() {
    let built = fixture(
        Variant::C8,
        &[vec![
            Record::type0(&rows(33), 0, 0),
            Record::jpeg(8, 8, 128, 1, 1),
        ]],
    );
    let mut source = Source::new(built.bytes);
    source.short = 1;
    let mut sink = Sink {
        short: Some(2),
        ..Sink::default()
    };
    let mut scratch = Scratch {
        short: Some(1),
        ..Scratch::default()
    };
    let mut visitor = Visitor::default();
    let limits = Limits {
        io_chunk_bytes: 7,
        ..Limits::default()
    };
    let report = convert(
        &mut source,
        &mut sink,
        Some(&table()),
        &mut scratch,
        &mut visitor,
        ComposeOptions::default(),
        &limits,
    )
    .unwrap();
    assert_eq!(report.output_pages, 1);
    assert!(source.max_request <= 7);
    assert!(sink.max_request <= 7);
    assert!(scratch.max_request <= 7);
    assert_eq!(scratch.read, 24);
    assert_eq!(scratch.written, 24);
    assert!(scratch.bytes.is_empty());
}

#[test]
fn current_page_metadata_and_row_storage_peaks_do_not_accumulate_across_pages() {
    let one = vec![Record::type0(&rows(9), 0, 0), Record::jpeg(8, 8, 90, 2, 2)];
    let mut peaks = Vec::new();
    for count in [1, 20] {
        let built = fixture(Variant::C8, &vec![one.clone(); count]);
        let mut source = Source::new(built.bytes);
        let mut sink = Sink::default();
        let mut scratch = Scratch::default();
        let mut visitor = Visitor::default();
        let report = convert(
            &mut source,
            &mut sink,
            Some(&table()),
            &mut scratch,
            &mut visitor,
            ComposeOptions::default(),
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(report.output_pages, count as u32);
        assert_eq!(report.row_store_written_bytes, count as u64 * 12);
        assert_eq!(report.row_store_read_bytes, count as u64 * 12);
        assert!(scratch.bytes.is_empty());
        peaks.push((
            report.peak_page_metadata_bytes,
            report.peak_text_working_bytes,
            report.peak_row_store_bytes,
            scratch.peak,
        ));
    }
    assert_eq!(peaks[0], peaks[1]);
}

#[test]
fn invalid_text_marker_is_rejected_without_emitting_images() {
    for variant in [Variant::C8, Variant::HnA] {
        let mut case = Harness::new(variant, &[vec![Record::jpeg(8, 8, 128, 0, 0)]]);
        case.source.bytes[case.fixture.text_offsets[0] + 8] ^= 1;
        let error = ready(convert_source_pages_pdf(
            &mut case.source,
            &mut case.sink,
            None,
            &mut case.scratch,
            &mut case.visitor,
            ComposeOptions::default(),
            &Limits::default(),
            &NeverCancel,
        ))
        .unwrap_err();
        located(&error, variant, Some(1), None);
        assert_eq!(error.stage, ComposeStage::Text);
        assert!(matches!(error.kind, ComposeErrorKind::Container(_)));
        assert!(case.visitor.mappings.is_empty());
        assert!(!contains(&case.sink.bytes, b"/Subtype /Image"));
    }
}

#[test]
fn pure_text_refusals_are_explicit_and_hnb_no_image_rows_are_observed() {
    for variant in [Variant::C8, Variant::HnA, Variant::HnB] {
        let mut case = Harness::new(variant, &[vec![], vec![]]);
        let error = case
            .run(None, ComposeOptions::default(), &Limits::default())
            .unwrap_err();
        assert_eq!(error.stage, ComposeStage::Preflight);
        assert!(matches!(error.kind, ComposeErrorKind::NoImages));
        if variant == Variant::HnB {
            located(&error, variant, None, None);
            assert_eq!(case.visitor.mappings, [(1, None), (2, None)]);
        } else {
            located(&error, variant, Some(1), None);
            assert!(case.visitor.mappings.is_empty());
        }
        assert!(!contains(&case.sink.bytes, b"/Subtype /Image"));
    }
}

#[test]
fn hnb_multi_image_and_non_jpeg_rows_are_never_silently_selected() {
    let mut many = Harness::new(Variant::HnB, &[vec![Record::jpeg(8, 8, 100, 0, 0); 2]]);
    let error = many
        .run(None, ComposeOptions::default(), &Limits::default())
        .unwrap_err();
    located(&error, Variant::HnB, Some(1), None);
    assert!(matches!(error.kind, ComposeErrorKind::Unsupported(_)));
    assert!(!contains(&many.sink.bytes, b"/Subtype /Image"));
    for kind in [0, 1, 3] {
        let mut record = Record::jpeg(8, 8, 100, 0, 0);
        record.kind = kind;
        let mut case = Harness::new(Variant::HnB, &[vec![record]]);
        let error = case
            .run(
                Some(&table()),
                ComposeOptions::default(),
                &Limits::default(),
            )
            .unwrap_err();
        located(&error, Variant::HnB, Some(1), Some(1));
        assert_eq!(error.stage, ComposeStage::Headers);
        assert!(
            matches!(error.kind, ComposeErrorKind::UnsupportedImageType(actual) if actual == kind as u32)
        );
        assert_eq!(error.offset, Some(case.fixture.descriptors[0][0]));
        assert!(!contains(&case.sink.bytes, b"/Subtype /Image"));
    }
}

#[test]
fn late_invalid_type1_or_missing_type3_workspace_refuses_page_before_output() {
    for kind in [1, 3] {
        let first = Record::jpeg(8, 8, 70, 0, 0);
        let mut second = Record::jpeg(8, 8, 190, 0, 0);
        second.kind = kind;
        if kind == 1 {
            second.bytes[0] = 0; // Invalid SOI must not be accepted by its record tag.
        }
        let mut case = Harness::new(Variant::C8, &[vec![first, second]]);
        let error = case
            .run(None, ComposeOptions::default(), &Limits::default())
            .unwrap_err();
        located(&error, Variant::C8, Some(1), Some(2));
        assert_eq!(error.stage, ComposeStage::Headers);
        if kind == 3 {
            assert_eq!(error.offset, Some(case.fixture.descriptors[0][1]));
            assert!(matches!(
                error.kind,
                ComposeErrorKind::MissingType3Workspaces
            ));
            assert!(error.to_string().contains("symbol stores"));
        } else {
            assert!(matches!(error.kind, ComposeErrorKind::Jpeg(_)));
            assert_eq!(error.offset, Some(case.fixture.payloads[0][1]));
        }
        assert!(!contains(&case.sink.bytes, b"/Subtype /Image"));
    }
}

#[test]
fn missing_table_and_text_working_allocation_failures_remain_located() {
    let mut missing = Harness::type0();
    let error = missing
        .run(None, ComposeOptions::default(), &Limits::default())
        .unwrap_err();
    located(&error, Variant::C8, Some(1), Some(1));
    assert_eq!(error.stage, ComposeStage::Headers);
    assert_eq!(error.offset, Some(missing.fixture.descriptors[0][0]));
    assert!(matches!(error.kind, ComposeErrorKind::MissingTable));
    assert_eq!(missing.scratch.peak, 0);
    let mut bounded = Harness::type0();
    let limits = Limits {
        io_chunk_bytes: 7,
        max_allocation_bytes: 512,
        ..Limits::default()
    };
    let error = bounded
        .run(Some(&table()), ComposeOptions::default(), &limits)
        .unwrap_err();
    located(&error, Variant::C8, Some(1), None);
    assert_eq!(error.stage, ComposeStage::Text);
    assert!(matches!(error.kind, ComposeErrorKind::Container(ref inner)
    if matches!(inner.kind, super::super::ErrorKind::LimitExceeded {
        resource: "text decoder allocation reservation", limit: 512,
        attempted: crate::hnc8::TEXT_DECODER_RESERVATION_BYTES,
    })));
    assert_eq!(bounded.scratch.peak, 0);
}

#[test]
fn metadata_allocation_and_count_ceilings_precede_text_or_payload_output() {
    let images = vec![Record::jpeg(8, 8, 128, 0, 0); 2];
    let mut case = Harness::new(Variant::C8, std::slice::from_ref(&images));
    let mut options = ComposeOptions::default();
    options.budget.max_page_metadata_bytes = 1;
    let error = case.run(None, options, &Limits::default()).unwrap_err();
    assert_eq!(error.stage, ComposeStage::Preflight);
    assert!(matches!(
        error.kind,
        ComposeErrorKind::Io(Error::LimitExceeded {
            resource: "current-page metadata bytes",
            ..
        })
    ));
    assert!(!contains(&case.sink.bytes, b"/Subtype /Image"));
    let mut case = Harness::new(Variant::C8, &[images]);
    let limits = Limits {
        io_chunk_bytes: 7,
        max_allocation_bytes: 64,
        ..Limits::default()
    };
    let error = case
        .run(None, ComposeOptions::default(), &limits)
        .unwrap_err();
    assert_eq!(error.stage, ComposeStage::Preflight);
    assert!(matches!(
        error.kind,
        ComposeErrorKind::Io(Error::LimitExceeded {
            resource: "allocation bytes",
            ..
        })
    ));
    let mut case = Harness::new(Variant::C8, &[vec![Record::jpeg(8, 8, 128, 0, 0)]]);
    let row = case.fixture.index;
    case.source.bytes[row + 8..row + 10].copy_from_slice(&8193_i16.to_le_bytes());
    let mut options = ComposeOptions::default();
    options.container.max_images_per_page = 10_000;
    let error = case.run(None, options, &Limits::default()).unwrap_err();
    assert_eq!(error.stage, ComposeStage::Preflight);
    assert!(matches!(
        error.kind,
        ComposeErrorKind::Io(Error::LimitExceeded {
            resource: "PDF image placements per page",
            limit: 8192,
            attempted: 8193
        })
    ));
}

#[test]
fn malformed_counts_spans_and_text_declared_length_are_not_partial_passes() {
    for corruption in 0..4 {
        let mut case = Harness::new(Variant::C8, &[vec![Record::jpeg(8, 8, 128, 0, 0)]]);
        let row = case.fixture.index;
        let descriptor = case.fixture.descriptors[0][0] as usize;
        match corruption {
            0 => case.source.bytes[row + 8..row + 10].copy_from_slice(&(-1_i16).to_le_bytes()),
            1 => case.source.bytes[descriptor + 4..descriptor + 8]
                .copy_from_slice(&i32::MAX.to_le_bytes()),
            2 => case.source.bytes[case.fixture.text_offsets[0] + 20] ^= 1,
            _ => case.source.bytes[row + 8..row + 10].copy_from_slice(&2_i16.to_le_bytes()),
        }
        let error = case
            .run(None, ComposeOptions::default(), &Limits::default())
            .unwrap_err();
        located(
            &error,
            Variant::C8,
            Some(1),
            if corruption == 1 { Some(1) } else { None },
        );
        assert!(matches!(error.kind, ComposeErrorKind::Container(_)));
        assert_eq!(
            error.stage,
            if corruption >= 2 {
                ComposeStage::Text
            } else {
                ComposeStage::Container
            }
        );
        assert!(!contains(&case.sink.bytes, b"/Subtype /Image"));
        assert!(case.visitor.mappings.is_empty());
    }
}

#[test]
fn size_stable_jpeg_mutation_between_preflight_and_copy_is_detected() {
    let mut case = Harness::new(Variant::HnB, &[vec![Record::jpeg(8, 8, 128, 0, 0)]]);
    let start = case.fixture.payloads[0][0];
    case.source.payload_start = Some(start);
    // Change only an invented comment byte, preserving all marker lengths.
    case.source.mutate_at_pass = Some((2, start as usize + 24));
    let error = case
        .run(None, ComposeOptions::default(), &Limits::default())
        .unwrap_err();
    located(&error, Variant::HnB, Some(1), Some(1));
    assert_eq!(error.stage, ComposeStage::Pdf);
    assert!(
        matches!(error.kind, ComposeErrorKind::Jpeg(ref inner) if matches!(inner.kind, super::super::Type2PdfErrorKind::SourceChanged))
    );
    assert!(case.visitor.mappings.is_empty());
}

#[test]
fn bounded_row_store_capacity_and_work_failures_reset_the_store() {
    for capacity in [true, false] {
        let mut case = Harness::type0();
        let mut options = ComposeOptions::default();
        if capacity {
            options.budget.max_row_store_bytes = 11;
        } else {
            options.budget.max_row_store_io_bytes = 11;
        }
        let error = case
            .run(Some(&table()), options, &Limits::default())
            .unwrap_err();
        located(&error, Variant::C8, Some(1), Some(1));
        assert!(case.scratch.bytes.is_empty());
        assert!(case.visitor.mappings.is_empty());
        assert!(matches!(
            error.kind,
            ComposeErrorKind::Io(Error::LimitExceeded { .. }) | ComposeErrorKind::Image(_)
        ));
    }
}

#[test]
fn exclusive_dirty_row_store_is_overwritten_and_reset() {
    let mut case = Harness::type0();
    case.scratch.bytes = vec![0xee; 100];
    case.run(
        Some(&table()),
        ComposeOptions::default(),
        &Limits::default(),
    )
    .unwrap();
    assert!(case.scratch.bytes.is_empty());
    assert!(
        crate::test_support::bilevel_pixels(&case.sink.bytes).contains(&reversed_packed(&rows(9)))
    );
}

#[test]
fn scratch_short_zero_overreported_and_error_paths_preserve_identity_and_cleanup() {
    for read in [true, false] {
        for mode in [Fault::Zero, Fault::Overreport, Fault::Io] {
            let mut case = Harness::type0();
            if read {
                case.scratch.read_fault = Some(mode);
            } else {
                case.scratch.write_fault = Some(mode);
            }
            let error = case
                .run(
                    Some(&table()),
                    ComposeOptions::default(),
                    &Limits::default(),
                )
                .unwrap_err();
            located(&error, Variant::C8, Some(1), Some(1));
            assert_eq!(error.stage, ComposeStage::Scratch);
            assert!(case.scratch.bytes.is_empty());
            assert!(case.visitor.mappings.is_empty());
        }
    }
}

#[test]
fn scratch_prepare_flush_and_cleanup_failures_cannot_be_successful() {
    for fault in 0..3 {
        let mut case = Harness::type0();
        match fault {
            0 => case.scratch.fail_initialize = true,
            1 => case.scratch.fail_flush = true,
            _ => case.scratch.fail_cleanup = true,
        }
        let error = case
            .run(
                Some(&table()),
                ComposeOptions::default(),
                &Limits::default(),
            )
            .unwrap_err();
        located(&error, Variant::C8, Some(1), Some(1));
        let stage = if fault == 2 {
            ComposeStage::Cleanup
        } else {
            ComposeStage::Scratch
        };
        assert_eq!(error.stage, stage);
        assert!(case.visitor.mappings.is_empty());
        if stage != ComposeStage::Cleanup {
            assert!(case.scratch.bytes.is_empty());
        }
    }
    let mut case = Harness::type0();
    case.scratch.write_fault = Some(Fault::Io);
    case.scratch.fail_cleanup = true;
    let error = case
        .run(
            Some(&table()),
            ComposeOptions::default(),
            &Limits::default(),
        )
        .unwrap_err();
    assert_eq!(error.stage, ComposeStage::Cleanup);
    assert!(
        matches!(error.kind, ComposeErrorKind::Cleanup { ref primary, cleanup: Error::Io(_) } if primary.stage == ComposeStage::Scratch)
    );
    assert!(error.to_string().contains("cleanup also failed"));
}

#[test]
fn cancellation_during_decode_or_readback_still_resets_storage() {
    for read in [true, false] {
        let mut case = Harness::type0();
        let flag = Rc::new(Cell::new(false));
        if read {
            case.scratch.cancel_read = Some(flag.clone());
        } else {
            case.scratch.cancel_write = Some(flag.clone());
        }
        let error = case.cancelled(flag).unwrap_err();
        located(&error, Variant::C8, Some(1), Some(1));
        assert_eq!(
            error.stage,
            if read {
                ComposeStage::Pdf
            } else {
                ComposeStage::Decode
            }
        );
        assert!(case.scratch.bytes.is_empty());
        assert!(case.visitor.mappings.is_empty());
    }
}

#[test]
fn source_and_pdf_sink_failures_are_located_and_never_emit_success_events() {
    for mode in [Fault::Zero, Fault::Overreport, Fault::Io] {
        let mut case = Harness::type0();
        case.source.fault_at = Some((case.fixture.payloads[0][0], mode));
        let error = case
            .run(
                Some(&table()),
                ComposeOptions::default(),
                &Limits::default(),
            )
            .unwrap_err();
        located(&error, Variant::C8, Some(1), Some(1));
        assert_eq!(error.stage, ComposeStage::Headers);
        assert!(case.scratch.bytes.is_empty());
        assert!(case.visitor.mappings.is_empty());
    }
    let mut case = Harness::type0();
    case.sink.fail_after = Some(60);
    let error = case
        .run(
            Some(&table()),
            ComposeOptions::default(),
            &Limits::default(),
        )
        .unwrap_err();
    located(&error, Variant::C8, Some(1), Some(1));
    assert_eq!(error.stage, ComposeStage::Pdf);
    assert!(case.scratch.bytes.is_empty());
    assert!(case.visitor.mappings.is_empty());
}

#[test]
fn visitor_failure_and_inter_page_cancellation_invalidate_the_whole_conversion() {
    let mut case = Harness::type0();
    case.visitor.fail = true;
    let error = case
        .run(
            Some(&table()),
            ComposeOptions::default(),
            &Limits::default(),
        )
        .unwrap_err();
    located(&error, Variant::C8, Some(1), None);
    assert_eq!(error.stage, ComposeStage::Visitor);
    assert_eq!(case.visitor.mappings, [(1, Some(1))]);
    assert!(case.scratch.bytes.is_empty());
    let mut case = Harness::new(Variant::C8, &vec![vec![Record::type0(&rows(9), 0, 0)]; 2]);
    let flag = Rc::new(Cell::new(false));
    case.visitor.cancel = Some(flag.clone());
    let error = case.cancelled(flag).unwrap_err();
    located(&error, Variant::C8, Some(2), None);
    assert_eq!(error.stage, ComposeStage::Container);
    assert_eq!(case.visitor.mappings, [(1, Some(1))]);
    assert!(case.scratch.bytes.is_empty());
}

#[test]
fn invalid_configuration_is_refused_before_source_sink_or_store_access() {
    for field in 0..5 {
        for value in [0, MAX_BUDGET_COUNT + 1] {
            let mut case = Harness::type0();
            let mut options = ComposeOptions::default();
            match field {
                0 => options.arithmetic.max_symbols = value,
                1 => options.arithmetic.max_work = value,
                2 => options.budget.max_page_metadata_bytes = value,
                3 => options.budget.max_row_store_bytes = value,
                _ => options.budget.max_row_store_io_bytes = value,
            }
            let error = case
                .run(Some(&table()), options, &Limits::default())
                .unwrap_err();
            assert_eq!(error.stage, ComposeStage::Preflight);
            assert!(matches!(error.kind, ComposeErrorKind::InvalidOptions(_)));
            assert_eq!(error.variant, None);
            assert_eq!(case.source.max_request, 0);
            assert!(case.sink.bytes.is_empty());
            assert_eq!(case.scratch.max_request, 0);
        }
    }
    let mut case = Harness::type0();
    let limits = Limits {
        io_chunk_bytes: 0,
        ..Limits::default()
    };
    let error = case
        .run(None, ComposeOptions::default(), &limits)
        .unwrap_err();
    assert!(matches!(
        error.kind,
        ComposeErrorKind::Io(Error::InvalidInput { .. })
    ));
    assert!(case.sink.bytes.is_empty());
}

#[test]
fn resource_arithmetic_and_noop_visitor_are_checked_at_boundaries() {
    assert_eq!(metadata_bytes(3, 8).unwrap(), 24);
    assert!(metadata_bytes(u64::MAX, 2).is_err());
    assert_eq!(add_store_bytes(u64::MAX - 2, 2).unwrap(), u64::MAX);
    assert!(add_store_bytes(u64::MAX, 1).is_err());
    let unlimited = Limits {
        max_allocation_bytes: u64::MAX,
        ..Limits::default()
    };
    assert!(
        page_vector::<u8>(0, &unlimited, "test empty metadata")
            .unwrap()
            .is_empty()
    );
    let small = page_vector::<u8>(3, &unlimited, "test small metadata").unwrap();
    assert!(small.is_empty());
    assert!(small.capacity() >= 3);
    assert!(matches!(
        page_vector::<u8>(usize::MAX, &unlimited, "test current-page reserve"),
        Err(Error::LimitExceeded {
            resource: "test current-page reserve",
            attempted,
            ..
        }) if attempted == usize::MAX as u64
    ));
    let restricted = Limits {
        max_allocation_bytes: 2,
        ..Limits::default()
    };
    assert!(matches!(
        page_vector::<u8>(3, &restricted, "test metadata allocation"),
        Err(Error::LimitExceeded {
            resource: "allocation bytes",
            limit: 2,
            attempted: 3,
        })
    ));
    let page = PageRecord {
        page_number: 1,
        row_offset: 80,
        text: super::super::Span {
            offset: 100,
            length: 0,
        },
        image_count: 0,
        unknown: [0; 10],
    };
    ready(().page(ComposePage {
        source: page,
        output_page: None,
        size: None,
        images: &[],
    }))
    .unwrap();
}

#[test]
fn larger_padded_rows_are_streamed_in_chunks_without_growing_page_metadata() {
    let mut case = Harness::new(Variant::C8, &[vec![Record::type0(&rows(4097), 0, 0)]]);
    let limits = Limits {
        io_chunk_bytes: 17,
        ..Limits::default()
    };
    let report = case
        .run(Some(&table()), ComposeOptions::default(), &limits)
        .unwrap();
    assert_eq!(report.peak_row_store_bytes, 516 * 3);
    assert_eq!(report.row_store_read_bytes, 516 * 3);
    assert_eq!(report.row_store_written_bytes, 516 * 3);
    assert!(case.source.max_request <= 17);
    assert!(case.scratch.max_request <= 17);
    assert!(case.sink.max_request <= 17);
    assert!(case.scratch.bytes.is_empty());
    assert!(
        crate::test_support::bilevel_pixels(&case.sink.bytes)
            .contains(&reversed_packed(&rows(4097)))
    );
}

#[test]
fn changed_type0_wrapper_and_malformed_jpeg_headers_are_located() {
    let mut case = Harness::type0();
    let payload = case.fixture.payloads[0][0];
    case.source.payload_start = Some(payload);
    case.source.mutate_at_pass = Some((2, payload as usize + 4));
    let error = case
        .run(
            Some(&table()),
            ComposeOptions::default(),
            &Limits::default(),
        )
        .unwrap_err();
    located(&error, Variant::C8, Some(1), Some(1));
    assert_eq!(error.stage, ComposeStage::Decode);
    assert_eq!(error.offset, Some(payload));
    assert!(matches!(error.kind, ComposeErrorKind::Image(_)));
    assert!(case.scratch.bytes.is_empty());
    let mut case = Harness::new(Variant::HnB, &[vec![Record::jpeg(8, 8, 128, 0, 0)]]);
    case.source.bytes[case.fixture.payloads[0][0] as usize + 1] = 0;
    let error = case
        .run(None, ComposeOptions::default(), &Limits::default())
        .unwrap_err();
    located(&error, Variant::HnB, Some(1), Some(1));
    assert_eq!(error.stage, ComposeStage::Headers);
    assert!(matches!(error.kind, ComposeErrorKind::Jpeg(_)));
}

#[test]
fn initial_pdf_and_final_flush_failures_keep_known_variant_and_source_anchor() {
    for initial in [true, false] {
        let mut case = Harness::new(Variant::HnB, &[vec![Record::jpeg(8, 8, 128, 0, 0)]]);
        if initial {
            case.sink.fail_after = Some(0);
        } else {
            case.sink.fail_flush = true;
        }
        let error = case
            .run(None, ComposeOptions::default(), &Limits::default())
            .unwrap_err();
        located(&error, Variant::HnB, None, None);
        assert_eq!(error.stage, ComposeStage::Pdf);
        assert_eq!(error.offset, Some(0));
        assert!(matches!(error.kind, ComposeErrorKind::Io(Error::Io(_))));
        assert_eq!(case.visitor.mappings.len(), usize::from(!initial));
    }
}

fn example_container_error() -> Hnc8Error {
    Hnc8Error {
        variant: Some(Variant::C8),
        offset: 99,
        page: Some(2),
        image: Some(3),
        kind: super::super::ErrorKind::Malformed {
            field: "invented field",
            reason: "original error example",
        },
    }
}

fn example_arithmetic_error() -> ArithmeticError {
    ArithmeticError {
        offset: Some(99),
        context: Some(7),
        kind: crate::qm::ArithmeticErrorKind::InvalidContext,
    }
}

fn example_image_error(kind: crate::jbig1::Type0ErrorKind) -> Type0Error {
    Type0Error {
        offset: 99,
        rows_written: 1,
        output_bytes_written: 4,
        kind,
    }
}

fn assert_error_description(error: &ComposeError, nested: bool) {
    use std::{error::Error as _, fmt};
    struct Refuse;
    impl fmt::Write for Refuse {
        fn write_str(&mut self, _: &str) -> fmt::Result {
            Err(fmt::Error)
        }
    }
    assert!(error.to_string().contains("HN/C8 page composition"));
    assert_eq!(error.source().is_some(), nested);
    assert!(fmt::write(&mut Refuse, format_args!("{error}")).is_err());
}

#[test]
fn nested_type0_error_mapping_preserves_identity_stage_and_error_sources() {
    use crate::jbig1::Type0ErrorKind;
    let at = At {
        variant: Some(Variant::C8),
        page: Some(2),
        image: Some(3),
        offset: Some(41),
    };
    let cases = [
        (
            Type0PdfErrorKind::Image(Box::new(example_image_error(Type0ErrorKind::Malformed(
                "original error",
            )))),
            ComposeStage::Decode,
            true,
        ),
        (
            Type0PdfErrorKind::Image(Box::new(example_image_error(Type0ErrorKind::Sink(
                Error::Cancelled,
            )))),
            ComposeStage::Scratch,
            true,
        ),
        (
            Type0PdfErrorKind::Contexts(Box::new(example_arithmetic_error())),
            ComposeStage::Decode,
            true,
        ),
        (
            Type0PdfErrorKind::Container(Box::new(example_container_error())),
            ComposeStage::Container,
            true,
        ),
        (
            Type0PdfErrorKind::Pdf(Error::Cancelled),
            ComposeStage::Pdf,
            true,
        ),
        (
            Type0PdfErrorKind::InvalidOptions("original options"),
            ComposeStage::Decode,
            false,
        ),
        (
            Type0PdfErrorKind::InvalidSelection("original selection"),
            ComposeStage::Decode,
            false,
        ),
        (
            Type0PdfErrorKind::UnsupportedImageType(3),
            ComposeStage::Decode,
            false,
        ),
        (
            Type0PdfErrorKind::MultipleImages(2),
            ComposeStage::Decode,
            false,
        ),
        (Type0PdfErrorKind::NoImages, ComposeStage::Decode, false),
    ];
    for (kind, stage, nested) in cases {
        let error = type0_decode(
            at,
            Type0PdfError {
                page: Some(2),
                image: Some(3),
                offset: Some(99),
                kind,
            },
        );
        located(&error, Variant::C8, Some(2), Some(3));
        assert_eq!(error.offset, Some(99));
        assert_eq!(error.stage, stage);
        assert_error_description(&error, nested);
    }
    let jpeg = Type2PdfError {
        page: Some(2),
        image: Some(3),
        offset: None,
        kind: super::super::Type2PdfErrorKind::InvalidOptions("original JPEG choice"),
    };
    let error = at.jpeg(ComposeStage::Headers)(jpeg);
    assert_eq!(error.offset, Some(41));
    assert_error_description(&error, true);
    let error = at.contexts()(example_arithmetic_error());
    located(&error, Variant::C8, Some(2), Some(3));
    assert_eq!(error.offset, Some(41));
    assert_eq!(error.stage, ComposeStage::Decode);
    assert!(matches!(error.kind, ComposeErrorKind::Contexts(ref inner)
        if inner.context == Some(7)
            && matches!(inner.kind, crate::qm::ArithmeticErrorKind::InvalidContext)));
    assert_error_description(&error, true);
}

#[test]
fn scratch_error_mapping_preserves_primary_and_cleanup_failures() {
    let at = At {
        variant: Some(Variant::HnA),
        page: Some(2),
        image: Some(3),
        offset: Some(41),
    };
    for stage in [
        Type0ScratchStage::Prepare,
        Type0ScratchStage::Decode,
        Type0ScratchStage::Emit,
        Type0ScratchStage::Cleanup,
    ] {
        let expected = if stage == Type0ScratchStage::Cleanup {
            ComposeStage::Cleanup
        } else {
            ComposeStage::Scratch
        };
        let error = scratch_error(
            at,
            Type0ScratchError {
                stage,
                kind: Type0ScratchErrorKind::Store(Error::Cancelled),
                cleanup_error: None,
                report: Default::default(),
            },
        );
        located(&error, Variant::HnA, Some(2), Some(3));
        assert_eq!(error.stage, expected);
        assert_error_description(&error, true);
    }
    let error = scratch_error(
        at,
        Type0ScratchError {
            stage: Type0ScratchStage::Emit,
            kind: Type0ScratchErrorKind::Pdf(Error::Cancelled),
            cleanup_error: Some(Error::Io(io::Error::other("original cleanup error"))),
            report: Default::default(),
        },
    );
    assert_eq!(error.stage, ComposeStage::Cleanup);
    assert!(
        matches!(error.kind, ComposeErrorKind::Cleanup { ref primary, .. } if primary.stage == ComposeStage::Pdf)
    );
    assert_error_description(&error, true);
    for kind in [
        ComposeErrorKind::MissingTable,
        ComposeErrorKind::UnsupportedImageType(1),
        ComposeErrorKind::InvalidOptions("example"),
        ComposeErrorKind::Unsupported("example"),
        ComposeErrorKind::NoImages,
    ] {
        assert_error_description(&At::NONE.error(ComposeStage::Preflight, kind), false);
    }
}

#[test]
fn dropped_pending_conversion_requires_caller_owned_store_disposal() {
    struct OwnedScratch {
        inner: Scratch,
        live_bytes: Rc<Cell<usize>>,
        disposed: Rc<Cell<bool>>,
        pending_read: bool,
    }

    impl Drop for OwnedScratch {
        fn drop(&mut self) {
            self.inner.bytes.clear();
            self.live_bytes.set(0);
            self.disposed.set(true);
        }
    }

    impl RandomAccessScratch for OwnedScratch {
        fn size(&self) -> crate::Result<u64> {
            self.inner.size()
        }
        async fn set_len(&mut self, bytes: u64) -> crate::Result<()> {
            self.inner.set_len(bytes).await?;
            self.live_bytes.set(self.inner.bytes.len());
            Ok(())
        }
        async fn read_at(&mut self, offset: u64, bytes: &mut [u8]) -> crate::Result<usize> {
            if self.pending_read {
                std::future::pending().await
            } else {
                self.inner.read_at(offset, bytes).await
            }
        }
        async fn write_at(&mut self, offset: u64, bytes: &[u8]) -> crate::Result<usize> {
            if !self.pending_read {
                std::future::pending().await
            } else {
                self.inner.write_at(offset, bytes).await
            }
        }
        async fn flush(&mut self) -> crate::Result<()> {
            self.inner.flush().await
        }
    }

    for pending_read in [true, false] {
        let mut case = Harness::type0();
        let live = Rc::new(Cell::new(0));
        let disposed = Rc::new(Cell::new(false));
        let mut scratch = OwnedScratch {
            inner: Scratch::default(),
            live_bytes: live.clone(),
            disposed: disposed.clone(),
            pending_read,
        };
        let table = table();
        let options = ComposeOptions::default();
        let limits = Limits::default();
        {
            let mut future = pin!(convert_source_pages_pdf(
                &mut case.source,
                &mut case.sink,
                Some(&table),
                &mut scratch,
                &mut case.visitor,
                options,
                &limits,
                &NeverCancel,
            ));
            assert!(matches!(
                future
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop())),
                Poll::Pending
            ));
            assert_eq!(live.get(), 12);
        }
        // Dropping the borrowed future cannot await set_len(0). The adapter
        // owner releases its backing storage; no successful report exists.
        assert_eq!(live.get(), 12);
        assert!(!disposed.get());
        assert!(case.visitor.mappings.is_empty());
        assert!(!contains(&case.sink.bytes, b"%%EOF"));
        drop(scratch);
        assert!(disposed.get());
        assert_eq!(live.get(), 0);
    }
}

fn render_original_pdf(bytes: &[u8]) -> Vec<u8> {
    render_original_pdf_at(bytes, "0.7419")
}

fn render_original_pdf_at(bytes: &[u8], dpi: &str) -> Vec<u8> {
    use std::{
        fs,
        path::PathBuf,
        process::Command,
        sync::atomic::{AtomicUsize, Ordering},
    };
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Artifacts(PathBuf);
    impl Drop for Artifacts {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let directory = std::env::temp_dir().join(format!(
        "caj2pdf-compose-original-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    ));
    fs::create_dir(&directory).unwrap();
    let artifacts = Artifacts(directory);
    let pdf = artifacts.0.join("original.pdf");
    let raster = artifacts.0.join("original.pgm");
    fs::write(&pdf, bytes).unwrap();
    let checked = Command::new("qpdf")
        .arg("--check")
        .arg(&pdf)
        .output()
        .expect("qpdf is a required PDF test tool");
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let rendered = Command::new("mutool")
        .args([
            "draw", "-q", "-A", "0", "-r", dpi, "-c", "gray", "-F", "pgm", "-o",
        ])
        .arg(&raster)
        .arg(&pdf)
        .arg("1")
        .output()
        .expect("mutool is a required PDF test tool");
    assert!(
        rendered.status.success(),
        "{}",
        String::from_utf8_lossy(&rendered.stderr)
    );
    fs::read(raster).unwrap()
}

#[test]
fn independent_render_proves_padded_orientation_and_later_overlap_pixels() {
    let pixels = rows(9);
    // 1000 source units per pixel at the renderer's 0.7419 DPI. This
    // deliberately separates display units from encoded pixel dimensions.
    let mut records = vec![
        Record::jpeg(32, 16, 50, 0, 0),
        Record::type0(&pixels, 0, 0),
        Record::jpeg(8, 8, 90, 16000, 4000),
        Record::jpeg(8, 8, 210, 16000, 4000),
    ];
    for (record, [width, height]) in
        records
            .iter_mut()
            .zip([[32000, 16000], [9000, 3000], [8000, 8000], [8000, 8000]])
    {
        record.coordinate.width = width;
        record.coordinate.height = height;
    }
    let mut case = Harness::new(Variant::C8, &[records]);
    case.source.bytes[32..34].copy_from_slice(&32000_u16.to_le_bytes());
    case.source.bytes[34..36].copy_from_slice(&16000_u16.to_le_bytes());
    case.run(
        Some(&table()),
        ComposeOptions::default(),
        &Limits::default(),
    )
    .unwrap();
    let raster = render_original_pdf(&case.sink.bytes);
    let header = b"P5\n32 16\n255\n";
    assert!(
        raster.starts_with(header),
        "unexpected original raster geometry"
    );
    let samples = &raster[header.len()..];
    assert_eq!(samples.len(), 32 * 16);
    // Check every visible pixel after the negative CTM and row reversal;
    // dropped storage padding must leave the background image visible.
    for (y, row) in pixels.iter().enumerate() {
        for x in 0..9 {
            assert_eq!(
                samples[y * 32 + x],
                if row.get(x) == Some(&true) { 0 } else { 255 },
                "x={x}, y={y}"
            );
        }
    }
    assert!(
        samples[30].abs_diff(50) <= 1,
        "storage padding must not paint"
    );
    assert!(
        samples[15 * 32 + 30].abs_diff(50) <= 1,
        "background draw is present"
    );
    assert!(
        samples[6 * 32 + 18].abs_diff(210) <= 1,
        "the later overlapping JPEG paints over the earlier 90-gray draw"
    );
    assert_eq!(case.visitor.images.len(), 4);
    assert!(case.scratch.bytes.is_empty());
}

#[test]
fn optional_hna_outlines_preserve_image_bytes_and_use_nullable_xyz() {
    let image = Record::jpeg(8, 8, 90, 0, 0);
    let mut fixture = fixture(Variant::HnA, &[vec![image.clone()]]);
    let at = fixture.index;
    fixture.bytes.splice(at..at, [0; 308]);
    fixture.bytes[344..348].copy_from_slice(&1_i32.to_le_bytes());
    fixture.bytes[at..at + 8].copy_from_slice(b"Original");
    fixture.bytes[at + 280] = b'1';
    fixture.bytes[at + 304..at + 308].copy_from_slice(&1_u32.to_le_bytes());
    let row = at + 308;
    fixture.bytes[row..row + 4]
        .copy_from_slice(&((fixture.text_offsets[0] + 308) as i32).to_le_bytes());
    let descriptor = fixture.descriptors[0][0] as usize + 308;
    fixture.bytes[descriptor + 4..descriptor + 8]
        .copy_from_slice(&((fixture.payloads[0][0] + 308) as i32).to_le_bytes());
    let limits = Limits::default();
    let mut sink = Sink::default();
    let report = convert(
        &mut Source::new(fixture.bytes.clone()),
        &mut sink,
        None,
        &mut Scratch::default(),
        &mut Visitor::default(),
        ComposeOptions {
            include_bookmarks: true,
            ..ComposeOptions::default()
        },
        &limits,
    )
    .unwrap();
    assert_eq!(report.conversion.bookmarks_written, 1);
    assert_eq!(report.output_pages, 1);
    assert!(contains(&sink.bytes, b"/XYZ null null null"));
    assert!(contains(&sink.bytes, &image.bytes));
    assert_eq!((report.outline.declared, report.outline.written), (1, 1));
    assert_eq!(report.outline.defects, 0);
    let pdf = |bytes: &[u8], include_bookmarks: bool, limits: &Limits| {
        let mut sink = Sink::default();
        convert(
            &mut Source::new(bytes.to_vec()),
            &mut sink,
            None,
            &mut Scratch::default(),
            &mut Visitor::default(),
            ComposeOptions {
                include_bookmarks,
                ..ComposeOptions::default()
            },
            limits,
        )
        .map(|report| (report, sink.bytes))
    };
    // An unreadable outline table still fails the document.
    let error = pdf(
        &fixture.bytes,
        true,
        &Limits {
            max_bookmarks: 0,
            ..Limits::default()
        },
    )
    .unwrap_err();
    assert!(matches!(error.kind, ComposeErrorKind::Container(_)));
    // A destination outside the one-page source skips only that bookmark:
    // the PDF is byte-identical to one written without bookmarks.
    fixture.bytes[at + 280] = b'2';
    let (report, skipped) = pdf(&fixture.bytes, true, &limits).unwrap();
    assert_eq!(report.conversion.bookmarks_written, 0);
    assert_eq!(report.output_pages, 1);
    assert_eq!(
        report.outline.recorded_defects(),
        [OutlineDefect {
            offset: at as u64 + 280,
            reason: "destination is outside source pages",
            repair: OutlineRepair::Skipped,
        }]
    );
    let (_, without) = pdf(&fixture.bytes, false, &limits).unwrap();
    assert_eq!(skipped, without);
}

#[test]
fn unproven_outline_variants_are_reported_not_written_or_failed() {
    let mut outputs = Vec::new();
    for include_bookmarks in [false, true] {
        let fixture = fixture(Variant::C8, &[vec![Record::jpeg(8, 8, 90, 0, 0)]]);
        let mut sink = Sink::default();
        let report = convert(
            &mut Source::new(fixture.bytes),
            &mut sink,
            None,
            &mut Scratch::default(),
            &mut Visitor::default(),
            ComposeOptions {
                include_bookmarks,
                ..ComposeOptions::default()
            },
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(report.outline.unverified, include_bookmarks);
        assert_eq!(report.conversion.bookmarks_written, 0);
        outputs.push(sink.bytes);
    }
    assert_eq!(outputs[0], outputs[1]);
}

#[test]
fn uncompressed_text_composes_the_same_ordered_jpeg_page_as_compressed_text() {
    fn raw(records: &[Record]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for record in records {
            bytes.extend(0x800a_u16.to_le_bytes());
            bytes.extend([0; 2]);
            bytes.extend(record.coordinate.x.to_le_bytes());
            bytes.extend(record.coordinate.y.to_le_bytes());
            bytes.extend(record.coordinate.width.to_le_bytes());
            bytes.extend(record.coordinate.height.to_le_bytes());
            bytes.extend([0; 16]);
        }
        bytes.extend(0x8004_u16.to_le_bytes());
        bytes.extend([0; 2]);
        bytes
    }
    let pages = [vec![
        Record::jpeg(16, 16, 50, 0, 0),
        Record::jpeg(8, 8, 210, 17, 3),
    ]];
    let mut compressed = Harness::new(Variant::HnA, &pages);
    compressed
        .run(None, ComposeOptions::default(), &Limits::default())
        .unwrap();
    let fixture = fixture_with_text(Variant::HnA, &pages, raw);
    let mut source = Source::new(fixture.bytes);
    let mut sink = Sink::default();
    let report = convert(
        &mut source,
        &mut sink,
        None,
        &mut Scratch::default(),
        &mut Visitor::default(),
        ComposeOptions::default(),
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(report.output_pages, 1);
    assert_eq!(report.jpeg_images, 2);
    assert_eq!(sink.bytes, compressed.sink.bytes);
}

mod type3_fixture {
    include!("../../../tests/common/type3_fixture.rs");
}

fn mq_table(limits: &Limits) -> MqTable {
    use crate::jbig2::mq::{MQ_STATE_COUNT, MqState};
    MqTable::new(
        vec![
            MqState {
                qe: 1,
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

fn type3_record(width: u32, height: u32, x: u16, y: u16) -> Record {
    Record {
        kind: 3,
        bytes: type3_fixture::payload(width, height, 0x10),
        coordinate: RawTextCoordinate {
            x,
            y,
            width: 80,
            height: 40,
        },
    }
}

#[test]
fn type3_complete_mixed_pages_reuse_stores_and_keep_top_first_pixels() {
    let limits = Limits::default();
    let mq = mq_table(&limits);
    let mut first_image = type3_record(3, 5, 0, 0);
    first_image.coordinate.width = 3000;
    first_image.coordinate.height = 5000;
    let mut f = fixture_with_text(
        Variant::C8,
        &[
            vec![first_image],
            vec![
                Record::jpeg(40, 30, 128, 0, 0),
                type3_record(9, 3, 12, 7),
                type3_record(31, 2, 24, 14),
                Record::type0(&rows(9), 0, 20),
            ],
        ],
        direct_text,
    );
    f.bytes[32..34].copy_from_slice(&32000_u16.to_le_bytes());
    f.bytes[34..36].copy_from_slice(&5000_u16.to_le_bytes());
    let mut source = Source::new(f.bytes);
    let mut sink = Sink::default();
    let mut rows = Scratch::default();
    let mut first = Scratch {
        short: Some(1),
        ..Default::default()
    };
    let mut second = Scratch::default();
    let mut refined = Scratch::default();
    let mut visitor = Visitor::default();
    let report = ready(convert_source_pages_pdf(
        &mut source,
        &mut sink,
        Some(&table()),
        ComposeWorkspaces {
            rows: &mut rows,
            type3: Some(ComposeType3Workspaces {
                table: &mq,
                first: &mut first,
                second: &mut second,
                refined: &mut refined,
            }),
        },
        &mut visitor,
        ComposeOptions {
            type3: Type3PdfOptions {
                page_compose: crate::jbig2::page_compose::PageComposeBudget {
                    max_output_request_bytes: 1,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        },
        &limits,
        &NeverCancel,
    ))
    .unwrap();
    assert_eq!(
        (report.output_pages, report.type3_images, report.jpeg_images),
        (2, 3, 1)
    );
    assert!(report.peak_row_store_bytes > 0);
    assert!(report.row_store_written_bytes > 0);
    assert!(report.row_store_read_bytes > 0);
    for store in [&rows, &first, &second, &refined] {
        assert!(store.bytes.is_empty());
    }
    assert_eq!(report.type0_images, 1);
    let raster = render_original_pdf(&sink.bytes);
    let header = b"P5\n32 5\n255\n";
    assert!(
        raster.starts_with(header),
        "{:?}",
        &raster[..raster.len().min(80)]
    );
    let pixels = &raster[header.len()..];
    assert_eq!(pixels.len(), 160);
    assert_eq!(pixels[0], 0);
    assert!(pixels[1..].iter().all(|&pixel| pixel == 255));
    let pdf = String::from_utf8_lossy(&sink.bytes);
    assert_eq!(
        visitor.sizes[0].unwrap().width_points,
        32000.0 * (240.0 / 2473.0)
    );
    assert_eq!(visitor.images[0].4[5], 0.0);
    // Image widths exclude storage padding.
    assert_eq!(pdf.matches("/Width 3\n").count(), 1);
    assert_eq!(pdf.matches("/Width 9\n").count(), 2);
    assert_eq!(pdf.matches("/Width 31\n").count(), 1);
    assert!(crate::test_support::bilevel_pixels(&sink.bytes).contains(&vec![0x80, 0, 0, 0, 0]));
}

#[test]
fn type3_failures_keep_location_and_cleanup_all_stores() {
    for mode in 0..12 {
        let limits = Limits::default();
        let mq = mq_table(&limits);
        let mut record = type3_record(9, 3, 0, 0);
        if mode == 0 {
            record.bytes[4..8].copy_from_slice(&0_i32.to_le_bytes());
        }
        let f = fixture(Variant::C8, &[vec![record]]);
        let mut source = Source::new(f.bytes);
        if mode == 1 {
            source.payload_start = Some(f.payloads[0][0]);
            source.mutate_at_pass = Some((3, f.payloads[0][0] as usize + 36));
        }
        let mut sink = Sink::default();
        let mut rows = Scratch::default();
        let mut first = Scratch::default();
        let mut second = Scratch::default();
        let mut refined = Scratch::default();
        let flag = Rc::new(Cell::new(false));
        let mut options = ComposeOptions::default();
        match mode {
            2 => rows.fail_initialize = true,
            3 => rows.write_fault = Some(Fault::Overreport),
            4 => rows.read_fault = Some(Fault::Overreport),
            5 => rows.cancel_write = Some(flag.clone()),
            6 => options.budget.max_row_store_bytes = 1,
            7 => options.budget.max_row_store_io_bytes = 1,
            8 => rows.fail_cleanup = true,
            9 => {
                rows.fail_cleanup = true;
                rows.read_fault = Some(Fault::Io);
            }
            10 => rows.fail_flush = true,
            11 => sink.fail_after = Some(20),
            _ => (),
        }
        let error = ready(convert_source_pages_pdf(
            &mut source,
            &mut sink,
            Some(&table()),
            ComposeWorkspaces {
                rows: &mut rows,
                type3: Some(ComposeType3Workspaces {
                    table: &mq,
                    first: &mut first,
                    second: &mut second,
                    refined: &mut refined,
                }),
            },
            &mut (),
            options,
            &limits,
            &Flag(flag),
        ))
        .unwrap_err();
        located(&error, Variant::C8, Some(1), Some(1));
        assert!(
            std::error::Error::source(&error).is_some(),
            "mode {mode}: {error}"
        );
        assert!(!error.to_string().is_empty());
        if mode == 1 {
            assert!(
                matches!(error.kind, ComposeErrorKind::Type3(ref inner) if matches!(inner.kind, super::super::convert_jbig2::Type3PdfErrorKind::SourceChanged)),
                "{error}"
            );
        }
        if mode == 9 {
            assert!(matches!(error.kind, ComposeErrorKind::Cleanup { .. }));
        }
        for store in [&first, &second, &refined] {
            assert!(store.bytes.is_empty());
        }
        if !rows.fail_cleanup {
            assert!(rows.bytes.is_empty());
        }
        assert!(!contains(&sink.bytes, b"%%EOF"));
    }
}

#[test]
fn type3_anomaly_is_explicitly_opted_in_and_reported_per_image() {
    use crate::jbig2::text::TextHeaderPolicy;
    struct Anomalies(Vec<Option<TextHeaderAnomaly>>);
    impl ComposeVisitor for Anomalies {
        async fn page(&mut self, page: ComposePage<'_>) -> crate::Result<()> {
            self.0
                .extend(page.images.iter().map(|i| i.type3_text_header_anomaly));
            Ok(())
        }
    }
    for policy in [
        TextHeaderPolicy::Strict,
        TextHeaderPolicy::HnC8UnusedRefinementTemplate,
    ] {
        let limits = Limits::default();
        let table = mq_table(&limits);
        let mut record = type3_record(3, 2, 0, 0);
        record.bytes = type3_fixture::payload(3, 2, 0xa40c);
        let f = fixture(Variant::C8, &[vec![record]]);
        let mut source = Source::new(f.bytes);
        let mut sink = Sink::default();
        let mut stores: [Scratch; 4] = Default::default();
        let [rows, first, second, refined] = &mut stores;
        let mut visitor = Anomalies(Vec::new());
        let result = ready(convert_source_pages_pdf(
            &mut source,
            &mut sink,
            None,
            ComposeWorkspaces {
                rows,
                type3: Some(ComposeType3Workspaces {
                    table: &table,
                    first,
                    second,
                    refined,
                }),
            },
            &mut visitor,
            ComposeOptions {
                type3: Type3PdfOptions {
                    text_header_policy: policy,
                    ..Default::default()
                },
                ..Default::default()
            },
            &limits,
            &NeverCancel,
        ));
        if policy == TextHeaderPolicy::Strict {
            assert!(result.is_err());
            assert!(visitor.0.is_empty());
        } else {
            assert_eq!(result.unwrap().type3_images, 1);
            assert_eq!(
                visitor.0,
                [Some(TextHeaderAnomaly::UnusedRefinementTemplate)]
            );
        }
        assert!(stores.iter().all(|s| s.bytes.is_empty()));
    }
}

fn repeated_raw(records: &[Record]) -> Vec<u8> {
    image_records(&records[..2])
}
fn repeated_direct(records: &[Record]) -> Vec<u8> {
    direct_text(&records[..2])
}

#[test]
fn repeated_payload_groups_collapse_to_identical_pdf_with_explicit_aliases() {
    struct Aliases(Vec<Option<u32>>);
    impl ComposeVisitor for Aliases {
        async fn page(&mut self, page: ComposePage<'_>) -> crate::Result<()> {
            assert_eq!(page.output_page, Some(1));
            for image in page.images {
                self.0.push(image.duplicate_of);
                if let Some(original) = image.duplicate_of {
                    assert_eq!(
                        image.transform,
                        page.images[original as usize - 1].transform
                    );
                }
            }
            Ok(())
        }
    }
    for (variant, framing) in [
        (Variant::HnA, repeated_raw as fn(&[Record]) -> Vec<u8>),
        (Variant::C8, repeated_direct),
    ] {
        let limits = Limits::default();
        let mq = mq_table(&limits);
        let pair = [type3_record(3, 5, 0, 0), Record::jpeg(7, 4, 130, 2, 3)];
        let mut reference = None;
        for repetitions in [1, 3] {
            let records = pair
                .iter()
                .cloned()
                .cycle()
                .take(pair.len() * repetitions)
                .collect::<Vec<_>>();
            let f = fixture_with_text(variant, &[records], framing);
            let mut source = Source::new(f.bytes);
            source.short = 11;
            let mut sink = Sink::default();
            let mut stores: [Scratch; 4] = Default::default();
            let [rows, first, second, refined] = &mut stores;
            let mut aliases = Aliases(Vec::new());
            let report = ready(convert_source_pages_pdf(
                &mut source,
                &mut sink,
                None,
                ComposeWorkspaces {
                    rows,
                    type3: Some(ComposeType3Workspaces {
                        table: &mq,
                        first,
                        second,
                        refined,
                    }),
                },
                &mut aliases,
                ComposeOptions::default(),
                &limits,
                &NeverCancel,
            ))
            .unwrap();
            assert_eq!(
                (
                    report.source_pages,
                    report.output_pages,
                    report.type3_images,
                    report.jpeg_images
                ),
                (1, 1, 1, 1)
            );
            assert_eq!(
                report.duplicate_image_records,
                ((repetitions - 1) * 2) as u64
            );
            assert!(stores.iter().all(|s| s.bytes.is_empty()));
            if repetitions == 1 {
                reference = Some(sink.bytes);
                assert_eq!(aliases.0, [None, None]);
            } else {
                assert_eq!(
                    sink.bytes,
                    reference.take().unwrap(),
                    "duplicates must not paint a second time"
                );
                assert_eq!(aliases.0, [None, None, Some(1), Some(2), Some(1), Some(2)]);
            }
        }
    }
}

#[test]
fn repeated_groups_reject_conflicts_partial_groups_and_read_failures_before_draws() {
    for mode in 0..8 {
        let pair = vec![
            Record::jpeg(512, 512, 90, 0, 0),
            Record::jpeg(7, 4, 190, 2, 3),
        ];
        let mut records = pair
            .iter()
            .cloned()
            .cycle()
            .take(pair.len() * 2)
            .collect::<Vec<_>>();
        match mode {
            0 => records[2].kind = 0,
            1 => records[2].bytes.push(0),
            2 => {
                let last = records[2].bytes.len() - 1;
                records[2].bytes[last] ^= 1;
            }
            3 => {
                records.pop();
            }
            _ => (),
        }
        let f = fixture_with_text(Variant::C8, &[records], repeated_direct);
        let mut source = Source::new(f.bytes);
        if (4..=6).contains(&mode) {
            source.fault_at = Some((
                f.payloads[0][2],
                [Fault::Zero, Fault::Overreport, Fault::Io][mode - 4],
            ));
        }
        // A zero-coordinate page cannot be mapped by modulo or guessed origins.
        if mode == 7 {
            let f = fixture_with_text(Variant::C8, &[pair], |_| direct_text(&[]));
            source = Source::new(f.bytes);
        }
        let mut sink = Sink::default();
        let error = convert(
            &mut source,
            &mut sink,
            None,
            &mut Scratch::default(),
            &mut Visitor::default(),
            ComposeOptions::default(),
            &Limits::default(),
        )
        .unwrap_err();
        assert_eq!(error.page, Some(1));
        if mode != 3 && mode != 7 {
            assert_eq!(error.image, Some(3));
        }
        if mode == 2 {
            assert!(error.to_string().contains("payload differs"));
        }
        if mode < 2 {
            assert!(error.to_string().contains("type or length differs"));
        }
        assert!(!contains(&sink.bytes, b"/Subtype /Image"));
        assert!(!contains(&sink.bytes, b"%%EOF"));
    }
}

#[test]
fn repeated_payload_comparison_cancels_and_propagates_first_read_failures() {
    struct Stop(Cell<usize>);
    impl Cancellation for Stop {
        fn is_cancelled(&self) -> bool {
            let remaining = self.0.get();
            self.0.set(remaining.saturating_sub(1));
            remaining == 0
        }
    }
    let payload = vec![0x55; 3100];
    let make_record = |number, offset| ImageRecord {
        page_number: 1,
        image_number: number,
        descriptor_offset: offset - 12,
        record_type: 2,
        payload: super::super::Span {
            offset,
            length: payload.len() as u64,
        },
    };
    let first = make_record(1, 12);
    let second = make_record(2, 3124);
    let at = At {
        variant: Some(Variant::C8),
        page: Some(1),
        image: Some(2),
        offset: Some(3112),
    };
    let mut bytes = vec![0; 12];
    bytes.extend(&payload);
    bytes.extend([0; 12]);
    bytes.extend(&payload);
    let limits = Limits::default();
    let mut completed = false;
    for checkpoints in 0..50 {
        let mut source = Source::new(bytes.clone());
        let result = ready(verify_repeated_image(
            &mut source,
            first,
            second,
            at,
            &limits,
            &Stop(Cell::new(checkpoints)),
        ));
        match result {
            Ok(()) => {
                completed = true;
                break;
            }
            Err(error) => assert!(matches!(error.kind, ComposeErrorKind::Io(Error::Cancelled))),
        }
    }
    assert!(completed);
    let mut source = Source::new(bytes);
    source.fault_at = Some((0, Fault::Io));
    let error = ready(verify_repeated_image(
        &mut source,
        first,
        second,
        at,
        &limits,
        &NeverCancel,
    ))
    .unwrap_err();
    assert!(matches!(error.kind, ComposeErrorKind::Io(Error::Io(_))));
}

#[test]
fn source_page_and_image_extents_are_independent_of_decoded_pixels() {
    for variant in [Variant::HnA, Variant::C8] {
        let encoders = [text as fn(&[Record]) -> Vec<u8>, direct_text, image_records];
        let count = if variant == Variant::HnA { 3 } else { 2 };
        for &encode in &encoders[..count] {
            for pixels in [(8, 8), (32, 16)] {
                let mut record = Record::jpeg(pixels.0, pixels.1, 90, 7, 11);
                record.coordinate.width = 12365;
                record.coordinate.height = 2473;
                let mut built = fixture_with_text(variant, &[vec![record]], encode);
                let offset = if variant == Variant::C8 { 32 } else { 168 };
                built.bytes[offset..offset + 2].copy_from_slice(&2473_u16.to_le_bytes());
                built.bytes[offset + 2..offset + 4].copy_from_slice(&4946_u16.to_le_bytes());
                let text_offset = built.text_offsets[0];
                if variant == Variant::HnA && built.bytes[text_offset..text_offset + 2] == [3, 0x80]
                {
                    built.bytes[text_offset + 2..text_offset + 4]
                        .copy_from_slice(&2473_u16.to_le_bytes());
                    built.bytes[text_offset + 6..text_offset + 8]
                        .copy_from_slice(&4946_u16.to_le_bytes());
                }
                let mut visitor = Visitor::default();
                convert(
                    &mut Source::new(built.bytes),
                    &mut Sink::default(),
                    None,
                    &mut Scratch::default(),
                    &mut visitor,
                    ComposeOptions::default(),
                    &Limits::default(),
                )
                .unwrap();
                let size = visitor.sizes[0].unwrap();
                assert_eq!((size.width_points, size.height_points), (240.0, 480.0));
                let transform = visitor.images[0].4;
                assert_eq!(
                    transform,
                    [
                        1200.0,
                        0.0,
                        0.0,
                        -240.0,
                        7.0 * (240.0 / 2473.0),
                        480.0 - 11.0 * (240.0 / 2473.0)
                    ]
                );
                assert_eq!(
                    (visitor.images[0].1, visitor.images[0].3),
                    (u32::from(pixels.0), u32::from(pixels.1))
                );
            }
        }
    }
}

#[test]
fn missing_page_or_image_extent_fails_before_emitting_image_data() {
    for field in 0..4 {
        let mut record = Record::jpeg(8, 8, 90, 0, 0);
        if field == 2 {
            record.coordinate.width = 0;
        }
        if field == 3 {
            record.coordinate.height = 0;
        }
        let mut built = fixture(Variant::C8, &[vec![record]]);
        if field < 2 {
            built.bytes[32 + field * 2..34 + field * 2].fill(0);
        }
        let mut sink = Sink::default();
        let mut visitor = Visitor::default();
        let error = convert(
            &mut Source::new(built.bytes),
            &mut sink,
            None,
            &mut Scratch::default(),
            &mut visitor,
            ComposeOptions::default(),
            &Limits::default(),
        )
        .unwrap_err();
        assert_eq!(error.stage, ComposeStage::Geometry);
        assert!(visitor.images.is_empty());
        assert!(!contains(&sink.bytes, b"/Subtype /Image"));
    }
}

fn mixed_codec_content_page() -> Vec<u8> {
    // Capture the ordinary composer's checked source descriptors, not its PDF
    // images. The second document decodes directly from the original source.
    #[derive(Default)]
    struct Plan(Vec<ComposedImage>);
    impl ComposeVisitor for Plan {
        async fn page(&mut self, page: ComposePage<'_>) -> crate::Result<()> {
            self.0.extend_from_slice(page.images);
            Ok(())
        }
    }
    let pixels = rows(9);
    let fixture = fixture(
        Variant::C8,
        &[vec![
            Record::type0(&pixels, 0, 0),
            Record::jpeg(8, 8, 155, 9, 0),
            type3_record(3, 5, 18, 0),
        ]],
    );
    let mut source = Source::new(fixture.bytes);
    source.short = 3;
    let limits = Limits {
        io_chunk_bytes: 64,
        ..Default::default()
    };
    let options = ComposeOptions::default();
    let qm = table();
    let mq = mq_table(&limits);
    let (mut rows, mut first, mut second, mut refined) = (
        Scratch::default(),
        Scratch::default(),
        Scratch::default(),
        Scratch::default(),
    );
    let mut workspaces = ComposeWorkspaces {
        rows: &mut rows,
        type3: Some(ComposeType3Workspaces {
            table: &mq,
            first: &mut first,
            second: &mut second,
            refined: &mut refined,
        }),
    };
    let mut plan = Plan::default();
    let mut baseline = Sink::default();
    let stores = workspaces.type3.as_mut().unwrap();
    let mut report = ready(convert_source_pages_pdf(
        &mut source,
        &mut baseline,
        Some(&qm),
        ComposeWorkspaces {
            rows: &mut *workspaces.rows,
            type3: Some(ComposeType3Workspaces {
                table: &mq,
                first: &mut *stores.first,
                second: &mut *stores.second,
                refined: &mut *stores.refined,
            }),
        },
        &mut plan,
        options,
        &limits,
        &NeverCancel,
    ))
    .unwrap();
    assert_eq!(plan.0.len(), 3);
    let mut sink = Sink {
        short: Some(7),
        ..Default::default()
    };
    ready(async {
        let mut font_source = Source::new(crate::pdf::drawing_font());
        let mut font = crate::pdf::OpenTypeFont::read(&mut font_source, 0, &limits, &NeverCancel)
            .await
            .unwrap();
        let mut document = PdfDocument::new(&mut sink, &limits, &NeverCancel)
            .await
            .unwrap();
        let handle = document.add_font(&font).unwrap();
        let mut handles = Vec::new();
        let mut contexts = None;
        for image in &mut plan.0 {
            handles.push(
                emit_image(
                    &mut source,
                    &mut document,
                    image,
                    At::NONE.image(image.record),
                    &mut contexts,
                    &mut workspaces,
                    Some(&qm),
                    options,
                    &limits,
                    &NeverCancel,
                    &mut report,
                )
                .await
                .unwrap(),
            );
        }
        let fonts = [&handle];
        let mut page = document
            .begin_content_page(
                PageSpec {
                    width_points: 120.0,
                    height_points: 100.0,
                },
                &fonts,
                &handles,
            )
            .await
            .unwrap();
        page.glyph(0, 'A', [20.0, 0.0, 0.0, 20.0, 10.0, 50.0])
            .await
            .unwrap();
        page.image(0, [18.0, 0.0, 0.0, -6.0, 12.0, 60.0])
            .await
            .unwrap();
        page.segment([10.0, 55.0], [90.0, 55.0], 2.0).await.unwrap();
        page.image(1, [16.0, 0.0, 0.0, 16.0, 20.0, 48.0])
            .await
            .unwrap();
        page.glyph(0, '中', [20.0, 0.0, 0.0, 20.0, 30.0, 50.0])
            .await
            .unwrap();
        page.image(2, [6.0, 0.0, 0.0, 10.0, 34.0, 54.0])
            .await
            .unwrap();
        page.fill_polygon(&[[32.0, 52.0], [42.0, 52.0], [37.0, 62.0]])
            .await
            .unwrap();
        page.finish().await.unwrap();
        document.embed_font(&handle, &mut font).await.unwrap();
        assert_eq!(document.finish().await.unwrap().pages_converted, 1);
    });
    assert_eq!(
        (report.type0_images, report.jpeg_images, report.type3_images),
        (2, 2, 2)
    );
    let stores = workspaces.type3.as_mut().unwrap();
    for store in [workspaces.rows, stores.first, stores.second, stores.refined] {
        assert!(store.bytes.is_empty());
        assert!(store.max_request <= 64);
    }
    assert_eq!(
        crate::test_support::bilevel_pixels(&baseline.bytes),
        crate::test_support::bilevel_pixels(&sink.bytes)
    );
    let content = crate::test_support::pdf_text(&sink.bytes);
    let operators = [
        "<0041> Tj",
        "/Im0 Do",
        " l S Q",
        "/Im1 Do",
        "<4E2D> Tj",
        "/Im2 Do",
        "h f Q",
    ];
    let positions: Vec<_> = operators
        .iter()
        .map(|operator| content.find(operator).unwrap())
        .collect();
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    let decoded = crate::test_support::bilevel_pixels(&sink.bytes);
    assert_eq!(decoded[0], reversed_packed(&pixels));
    assert_eq!(decoded[1], vec![0x80, 0, 0, 0, 0]);
    sink.bytes
}

#[test]
fn decoded_images_share_a_content_page_with_glyphs_and_vectors() {
    mixed_codec_content_page();
}

#[test]
fn independent_render_checks_decoded_images_in_native_content_order() {
    let raster = render_original_pdf_at(&mixed_codec_content_page(), "72");
    let header = b"P5\n120 100\n255\n";
    assert!(raster.starts_with(header));
    let pixels = &raster[header.len()..];
    assert_eq!(pixels.len(), 120 * 100);
    let pixel = |x: usize, y: usize| pixels[(99 - y) * 120 + x];
    assert_eq!(pixel(11, 55), 0); // Original A outline.
    assert_eq!(pixel(80, 55), 0); // Uncovered segment.
    assert_eq!(pixel(25, 55), 155); // JPEG overwrites the segment.
    assert_eq!(pixel(37, 55), 0); // Polygon overwrites the white type-3 image.
    assert_eq!(pixel(110, 90), 255);

    // The real native-record translator uses the same PDF writer. This scale
    // makes one raw source unit one pixel, independently fixing image bounds
    // at x=30..110/y=40..90 in the 600-unit original fixture. The second,
    // top-first image must cover the first image's reversed row order.
    let raster = render_original_pdf_at(&super::super::native_page::mixed_page(), "741.9");
    let header = b"P5\n600 600\n255\n";
    assert!(raster.starts_with(header));
    let pixels = &raster[header.len()..];
    assert_eq!(pixels.len(), 600 * 600);
    let pixel = |x: usize, y: usize| pixels[y * 600 + x];
    assert_eq!(pixel(40, 50), 0);
    assert_eq!(pixel(100, 50), 255);
    assert_eq!(pixel(40, 80), 255);
    assert_eq!(pixel(100, 80), 0);
    assert_eq!(pixel(85, 120), 68); // Original CJK-labelled triangle.
    assert_eq!(pixel(160, 120), 68); // Original Latin-labelled rectangle.
    assert_eq!(pixel(550, 550), 255);
}

#[test]
fn raw_hna_marked_images_keep_full_page_and_offset_geometry() {
    fn marked(records: &[Record], prefix: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        if prefix {
            for word in [0x8003_u16, 100, 0x8003, 200] {
                bytes.extend(word.to_le_bytes());
            }
        }
        for record in records {
            for word in [
                0x800a,
                0xd300,
                0xc000 | record.coordinate.x,
                record.coordinate.y,
                0xc000 | record.coordinate.width,
                record.coordinate.height,
            ] {
                bytes.extend(word.to_le_bytes());
            }
            bytes.extend([0; 16]);
        }
        bytes.extend(0x8004_u16.to_le_bytes());
        bytes.extend([0; 2]);
        bytes
    }
    let pages = [vec![
        Record::jpeg(32, 24, 50, 0, 0),
        Record::jpeg(28, 18, 210, 20, 30),
    ]];
    let mut reference = Harness::new(Variant::HnA, &pages);
    reference
        .run(None, ComposeOptions::default(), &Limits::default())
        .unwrap();
    for make_text in [
        (|records: &[Record]| marked(records, true)) as fn(&[Record]) -> Vec<u8>,
        |records: &[Record]| marked(records, false),
    ] {
        let fixture = fixture_with_text(Variant::HnA, &pages, make_text);
        let mut source = Source::new(fixture.bytes);
        let mut sink = Sink::default();
        let report = convert(
            &mut source,
            &mut sink,
            None,
            &mut Scratch::default(),
            &mut Visitor::default(),
            ComposeOptions::default(),
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(report.output_pages, 1);
        assert_eq!(report.jpeg_images, 2);
        assert_eq!(sink.bytes, reference.sink.bytes);
    }
}

#[test]
fn hna_paired_page_sizes_override_header_per_page_in_raw_and_compressed_text() {
    fn raw(records: &[Record]) -> Vec<u8> {
        let mut bytes = vec![3, 0x80, 100, 0, 3, 0x80, 200, 0];
        bytes.extend(image_records(records));
        bytes
    }
    for make_text in [raw as fn(&[Record]) -> Vec<u8>, text] {
        let pages = vec![vec![Record::jpeg(8, 4, 70, 10, 30)]; 2];
        let mut fixture = fixture_with_text(Variant::HnA, &pages, make_text);
        for (offset, [width, height]) in fixture
            .text_offsets
            .iter()
            .zip([[400u16, 240u16], [320, 200]])
        {
            fixture.bytes[offset + 2..offset + 4].copy_from_slice(&width.to_le_bytes());
            fixture.bytes[offset + 6..offset + 8].copy_from_slice(&height.to_le_bytes());
        }
        let mut source = Source::new(fixture.bytes);
        source.short = 3;
        let mut sink = Sink::default();
        let mut visitor = Visitor::default();
        convert(
            &mut source,
            &mut sink,
            None,
            &mut Scratch::default(),
            &mut visitor,
            ComposeOptions::default(),
            &Limits::default(),
        )
        .unwrap();
        let unit = 240.0 / 2473.0;
        for (index, [width, height]) in [[400.0, 240.0], [320.0, 200.0]].into_iter().enumerate() {
            let size = visitor.sizes[index].unwrap();
            assert_eq!(size.width_points, width * unit);
            assert_eq!(size.height_points, height * unit);
            assert_eq!(
                visitor.images[index].4,
                [
                    80.0 * unit,
                    0.0,
                    0.0,
                    -40.0 * unit,
                    10.0 * unit,
                    (height - 30.0) * unit
                ]
            );
        }
    }
}

#[test]
fn hna_zero_page_prefix_dimensions_fail_before_emitting_images() {
    for field in [2, 6] {
        let mut fixture = fixture(Variant::HnA, &[vec![Record::jpeg(8, 4, 70, 10, 30)]]);
        let offset = fixture.text_offsets[0] + field;
        fixture.bytes[offset..offset + 2].fill(0);
        let mut sink = Sink::default();
        let error = convert(
            &mut Source::new(fixture.bytes),
            &mut sink,
            None,
            &mut Scratch::default(),
            &mut Visitor::default(),
            ComposeOptions::default(),
            &Limits::default(),
        )
        .unwrap_err();
        assert_eq!(error.page, Some(1));
        assert_eq!(error.stage, ComposeStage::Geometry);
        assert!(!contains(&sink.bytes, b"/Subtype /Image"));
    }
}
mod native_document;

mod malformed;

mod application_info;

mod route;
