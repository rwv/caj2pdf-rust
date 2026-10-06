// SPDX-License-Identifier: MIT

//! Synthetic HN/C8 documents admitted by the document composition pipeline.
//!
//! Each page carries a compressed text frame (HN-A/C8) that places every
//! image at the page origin, then its chained image descriptors. HN-B pages
//! carry opaque text and admit only one JPEG. All bytes are original test
//! data, not corpus content.

use caj2pdf_core::{
    Cancellation, Error, Limits, RangedSource, SequentialSink,
    hnc8::{
        ComposeError, ComposeOptions, ComposeReport, ComposeType3Workspaces, ComposeVisitor,
        ComposeWorkspaces, Variant, convert_source_pages_pdf,
    },
    jbig2::{
        mq::{MQ_STATE_COUNT, MqState, MqTable},
        text_composer::RandomAccessScratch,
    },
    qm::QmTable,
};
use flate2::{Compression, write::ZlibEncoder};
use std::{
    future::Future,
    io::{self, Write},
    pin::pin,
    task::{Context, Poll, Waker},
};

/// Source units per image pixel when every image fits in a 16-bit extent.
/// At [`RENDER_DPI`] one image pixel is then one device pixel:
/// `1000 × 240 / 2473` points at `0.7419 / 72` pixels per point.
pub const UNITS_PER_PIXEL: u32 = 1000;
pub const RENDER_DPI: &str = "0.7419";

pub fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("test adapters complete immediately"),
    }
}

/// One image record and the pixel extent the text frame declares for it.
pub struct Image {
    pub kind: i32,
    pub payload: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// The document bytes and every descriptor and payload offset, per page.
pub struct Built {
    pub bytes: Vec<u8>,
    pub descriptors: Vec<Vec<u64>>,
    pub payloads: Vec<Vec<u64>>,
}

/// The fixed page-index row of one page.
pub fn page_row(variant: Variant, page: usize) -> usize {
    layout(variant).1 + 20 * page
}

fn layout(variant: Variant) -> (usize, usize) {
    match variant {
        Variant::C8 => (0x08, 0x50),
        Variant::HnA => (0x90, 0x15c),
        Variant::HnB => (0x90, 0xd8),
    }
}

/// A compressed text frame: paired page-size prefix records, then one
/// image-coordinate record per image at the page origin.
fn compressed_text(images: &[Image], units: u32, page: [u16; 2]) -> Vec<u8> {
    let extent = |pixels: u32| u16::try_from(pixels * units).unwrap();
    let mut plain = vec![0x19; 28 + images.len() * 28];
    for (slot, marker) in [(0, 0x8070_u16), (4, 0x8071), (8, 0x8001)] {
        plain[8 + slot..10 + slot].copy_from_slice(&marker.to_le_bytes());
    }
    for (number, image) in images.iter().enumerate() {
        let at = 28 + number * 28;
        plain[at..at + 4].fill(0);
        plain[at + 4..at + 6].copy_from_slice(&extent(image.width).to_le_bytes());
        plain[at + 6..at + 8].copy_from_slice(&extent(image.height).to_le_bytes());
    }
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&plain).unwrap();
    let mut text = Vec::new();
    for value in page {
        text.extend(0x8003_u16.to_le_bytes());
        text.extend(value.to_le_bytes());
    }
    text.extend(b"COMPRESSTEXT");
    text.extend((plain.len() as u32).to_le_bytes());
    text.extend(encoder.finish().unwrap());
    text
}

/// Build a document whose pages hold `pages`, in order. The page size is
/// the largest declared image extent, so a lone image fills its page.
pub fn document(variant: Variant, pages: &[Vec<Image>]) -> Built {
    let images = || pages.iter().flatten();
    let largest = images()
        .map(|image| image.width.max(image.height))
        .max()
        .unwrap_or(1);
    let units = if largest * UNITS_PER_PIXEL <= u32::from(u16::MAX) {
        UNITS_PER_PIXEL
    } else {
        1
    };
    let width = images().map(|image| image.width).max().unwrap_or(1) * units;
    let height = images().map(|image| image.height).max().unwrap_or(1) * units;
    let (count_at, index) = layout(variant);
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
    bytes[count_at..count_at + 4].copy_from_slice(&(pages.len() as i32).to_le_bytes());
    bytes[count_at + 24..count_at + 26].copy_from_slice(&(width as u16).to_le_bytes());
    bytes[count_at + 26..count_at + 28].copy_from_slice(&(height as u16).to_le_bytes());
    let mut built = Built {
        bytes,
        descriptors: Vec::new(),
        payloads: Vec::new(),
    };
    for (number, records) in pages.iter().enumerate() {
        let offset = built.bytes.len();
        let text = if variant == Variant::HnB {
            b"original opaque HN-B text".to_vec()
        } else {
            compressed_text(records, units, [width as u16, height as u16])
        };
        built.bytes.extend(&text);
        let row = index + number * 20;
        built.bytes[row..row + 4].copy_from_slice(&(offset as i32).to_le_bytes());
        built.bytes[row + 4..row + 8].copy_from_slice(&(text.len() as i32).to_le_bytes());
        built.bytes[row + 8..row + 10].copy_from_slice(&(records.len() as i16).to_le_bytes());
        let mut descriptors = Vec::new();
        let mut payloads = Vec::new();
        for record in records {
            let descriptor = built.bytes.len();
            let payload = descriptor + 12;
            built.bytes.extend(record.kind.to_le_bytes());
            built.bytes.extend((payload as i32).to_le_bytes());
            built
                .bytes
                .extend((record.payload.len() as i32).to_le_bytes());
            built.bytes.extend(&record.payload);
            descriptors.push(descriptor as u64);
            payloads.push(payload as u64);
        }
        built.descriptors.push(descriptors);
        built.payloads.push(payloads);
    }
    built
}

/// Invented stationary MQ states; no normative table is embedded.
pub fn invented_mq_table() -> MqTable {
    let states = vec![
        MqState {
            qe: 1,
            next_mps: 0,
            next_lps: 0,
            switch_mps: false,
        };
        MQ_STATE_COUNT
    ];
    MqTable::new(states, &Limits::default()).unwrap()
}

/// An in-memory store; optionally fails reads after a number of calls.
#[derive(Default)]
pub struct Store {
    pub bytes: Vec<u8>,
    pub fail_read_after: Option<usize>,
    pub read_calls: usize,
}

impl RandomAccessScratch for Store {
    fn size(&self) -> caj2pdf_core::Result<u64> {
        Ok(self.bytes.len() as u64)
    }

    async fn set_len(&mut self, length: u64) -> caj2pdf_core::Result<()> {
        self.bytes.resize(length as usize, 0);
        Ok(())
    }

    async fn read_at(&mut self, offset: u64, bytes: &mut [u8]) -> caj2pdf_core::Result<usize> {
        self.read_calls += 1;
        if self
            .fail_read_after
            .is_some_and(|count| self.read_calls > count)
        {
            return Err(Error::Io(io::Error::other("injected scratch read failure")));
        }
        let start = offset as usize;
        let count = bytes.len().min(self.bytes.len().saturating_sub(start));
        bytes[..count].copy_from_slice(&self.bytes[start..start + count]);
        Ok(count)
    }

    async fn write_at(&mut self, offset: u64, bytes: &[u8]) -> caj2pdf_core::Result<usize> {
        let start = offset as usize;
        self.bytes[start..start + bytes.len()].copy_from_slice(bytes);
        Ok(bytes.len())
    }

    async fn flush(&mut self) -> caj2pdf_core::Result<()> {
        Ok(())
    }
}

/// Run the document pipeline with a row store and type-3 symbol stores.
#[allow(clippy::too_many_arguments)]
pub fn convert<S, W, V, C>(
    source: &mut S,
    sink: &mut W,
    table: Option<&QmTable>,
    stores: &mut [Store; 4],
    visitor: &mut V,
    options: ComposeOptions,
    limits: &Limits,
    cancellation: &C,
) -> Result<ComposeReport, ComposeError>
where
    S: RangedSource,
    W: SequentialSink,
    V: ComposeVisitor,
    C: Cancellation,
{
    let mq = invented_mq_table();
    let [rows, first, second, refined] = stores;
    ready(convert_source_pages_pdf(
        source,
        sink,
        table,
        ComposeWorkspaces {
            rows,
            type3: Some(ComposeType3Workspaces {
                table: &mq,
                first,
                second,
                refined,
            }),
        },
        visitor,
        options,
        limits,
        cancellation,
    ))
}
