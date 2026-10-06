// SPDX-License-Identifier: MIT

//! Symbol dictionary tests. Synthetic segments, MQ-coded for the standard
//! T.88 states, test the public dictionary model and its failure handling,
//! not external CAJ/HN symbol-pixel compatibility.
//!
//! Every test shares one source, store, and cancellation type, so the
//! decoder paths they exercise belong to a single generic instantiation.

#[path = "../common/mod.rs"]
mod common;
mod decode;
mod faults;

use caj2pdf_core::{
    Limits, Payload, RangedSource,
    jbig2::{
        SegmentHeader, SegmentSpan,
        dictionary::{
            DictionaryError, DictionaryErrorKind, DictionaryMode, DictionaryReport,
            DictionaryStores, SymbolDescriptor, SymbolDictionaryDecoder, SymbolStore,
            read_dictionary_data_header,
        },
        iaid::IAID_BASE,
        integer::{BITMAP_BASE, INTEGER_CONTEXT_COUNT},
        mq::{ArithmeticErrorKind, ContextBank, ContextState, MqTable},
        read_segment_header,
    },
};
use common::CancelAfter;
use std::{cell::Cell, io, rc::Rc};

const ONE_SYMBOL: [u8; 5] = [0x94, 0xa7, 0x7f, 0xff, 0xac];
const TWO_SYMBOLS: [u8; 5] = [0x94, 0x3a, 0x5d, 0xff, 0xac];

#[derive(Clone, Copy)]
enum Fault {
    Io,
    Cancelled,
}

impl Fault {
    fn error(self) -> caj2pdf_core::Error {
        match self {
            Self::Io => caj2pdf_core::Error::Io(io::Error::other("injected I/O failure")),
            Self::Cancelled => caj2pdf_core::Error::Cancelled,
        }
    }
}

/// An in-memory source with injectable short reads, faults, over-reports,
/// pending reads, and cancellation after a chosen read. Bytes at or beyond
/// `visible_end` or the end of `bytes` read as end of input, so a test may
/// truncate `bytes` without also moving `visible_end`.
struct Source {
    bytes: Vec<u8>,
    advertised: u64,
    visible_end: usize,
    max_read: usize,
    overreport_from: Option<u64>,
    fault_from: Option<(u64, Fault)>,
    cancel_after_read: Option<(u64, Rc<Cell<bool>>)>,
    max_request: usize,
    max_offset: u64,
    read_calls: usize,
}

impl Source {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            advertised: bytes.len() as u64,
            visible_end: bytes.len(),
            bytes,
            max_read: usize::MAX,
            overreport_from: None,
            fault_from: None,
            cancel_after_read: None,
            max_request: 0,
            max_offset: 0,
            read_calls: 0,
        }
    }
}

impl Source {
    /// The visible bytes, as a decoder reads them from memory.
    fn payload(&self) -> Payload<'_> {
        let end = self
            .visible_end
            .min(self.bytes.len())
            .min(usize::try_from(self.advertised).unwrap_or(usize::MAX));
        Payload::from(&self.bytes[..end])
    }
}

impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.advertised
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> caj2pdf_core::Result<usize> {
        self.read_calls += 1;
        self.max_request = self.max_request.max(destination.len());
        self.max_offset = self.max_offset.max(offset);
        if let Some((from, fault)) = self.fault_from
            && offset >= from
        {
            return Err(fault.error());
        }
        if self.overreport_from.is_some_and(|from| offset >= from) {
            return Ok(destination.len() + 1);
        }
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        let count = self
            .visible_end
            .min(self.bytes.len())
            .saturating_sub(start)
            .min(destination.len())
            .min(self.max_read);
        if count != 0 {
            destination[..count].copy_from_slice(&self.bytes[start..start + count]);
            if let Some((from, flag)) = &self.cancel_after_read
                && offset >= *from
            {
                flag.set(true);
            }
        }
        Ok(count)
    }
}

/// A symbol store.
#[derive(Default)]
struct Store {
    bytes: Vec<u8>,
}

fn header(source: &mut Source) -> SegmentHeader {
    read_segment_header(
        source,
        SegmentSpan {
            offset: 0,
            length: source.size(),
        },
        &Limits::default(),
        &CancelAfter::Never,
    )
    .unwrap()
}

fn table() -> MqTable {
    MqTable::standard()
}

/// The integer and bitmap contexts of a direct dictionary.
fn direct_contexts() -> ContextBank {
    caj2pdf_core::jbig2::mq::context_bank(IAID_BASE, &Limits::default()).unwrap()
}

/// The exported descriptors of a direct dictionary, all in its new store.
fn exported(report: &DictionaryReport) -> Vec<SymbolDescriptor> {
    report
        .catalog
        .exported_symbols
        .iter()
        .map(|stored| {
            assert_eq!((stored.store, stored.store_base), (SymbolStore::New, 0));
            stored.symbol
        })
        .collect()
}

/// The imported store a direct dictionary never reads.
#[derive(Default)]
struct Unread {
    imported: Vec<u8>,
}

/// The stores of a direct dictionary: only `store` receives symbols, after
/// any bytes it already holds.
fn direct_stores<'a>(unread: &'a Unread, store: &'a mut Store) -> DictionaryStores<'a> {
    let new_base = store.bytes.len() as u64;
    DictionaryStores {
        imported: &unread.imported,
        imported_base: 0,
        new: &mut store.bytes,
        new_base,
    }
}
