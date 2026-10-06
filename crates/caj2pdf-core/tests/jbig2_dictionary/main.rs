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
    Limits, RangedSource, SequentialSink,
    jbig2::{
        HeaderLimits, SegmentHeader, SegmentSpan,
        dictionary::{
            DictionaryBudget, DictionaryError, DictionaryErrorKind, DictionaryMode,
            DictionaryReport, DictionaryStores, RefinementDictionaryBudget, SymbolDescriptor,
            SymbolDictionaryDecoder, SymbolStore, read_dictionary_data_header,
        },
        iaid::IAID_BASE,
        integer::{BITMAP_BASE, INTEGER_CONTEXT_COUNT},
        mq::{
            ArithmeticErrorKind, ContextBank, ContextState, MQ_STATE_COUNT, MqBudget, MqState,
            MqTable,
        },
        read_segment_header,
        refinement::RefinementBudget,
    },
};
use common::CancelAfter;
use std::{
    cell::Cell,
    future::{Future, pending},
    io,
    pin::pin,
    rc::Rc,
    task::{Context, Poll, Waker},
};

const ONE_SYMBOL: [u8; 5] = [0x94, 0xa7, 0x7f, 0xff, 0xac];
const TWO_SYMBOLS: [u8; 5] = [0x94, 0x3a, 0x5d, 0xff, 0xac];

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
    pending_at: Option<u64>,
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
            pending_at: None,
            cancel_after_read: None,
            max_request: 0,
            max_offset: 0,
            read_calls: 0,
        }
    }
}

impl RangedSource for Source {
    fn size(&self) -> u64 {
        self.advertised
    }

    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
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
        if self.pending_at.is_some_and(|start| offset >= start) {
            pending::<()>().await;
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

/// A symbol store that accepts at most `max_write` bytes per write and can
/// fail, stay pending, over-report, or raise a cancellation flag.
#[derive(Default)]
struct Store {
    bytes: Vec<u8>,
    max_write: usize,
    fail: bool,
    pending: bool,
    overreport: bool,
    cancel_after_write: Option<Rc<Cell<bool>>>,
    write_fault: Option<Fault>,
    flush_fault: Option<Fault>,
    flushed: bool,
}

impl Store {
    /// A store that accepts whole writes.
    fn unbounded() -> Self {
        Self {
            max_write: usize::MAX,
            ..Self::default()
        }
    }
}

impl SequentialSink for Store {
    async fn write(&mut self, bytes: &[u8]) -> caj2pdf_core::Result<usize> {
        if let Some(fault) = self.write_fault {
            return Err(fault.error());
        }
        if self.fail {
            return Err(caj2pdf_core::Error::Io(io::Error::other(
                "test store failure",
            )));
        }
        if self.pending {
            pending::<()>().await;
        }
        if self.overreport {
            return Ok(bytes.len() + 1);
        }
        let count = bytes.len().min(self.max_write);
        self.bytes.extend_from_slice(&bytes[..count]);
        if let Some(flag) = &self.cancel_after_write {
            flag.set(true);
        }
        Ok(count)
    }

    async fn flush(&mut self) -> caj2pdf_core::Result<()> {
        if let Some(fault) = self.flush_fault {
            return Err(fault.error());
        }
        self.flushed = true;
        Ok(())
    }
}

fn header(source: &mut Source) -> SegmentHeader {
    ready(read_segment_header(
        source,
        SegmentSpan {
            offset: 0,
            length: source.size(),
        },
        &Limits::default(),
        HeaderLimits::default(),
        &CancelAfter::Never,
    ))
    .unwrap()
}

fn table() -> MqTable {
    MqTable::standard()
}

/// The integer and bitmap contexts of a direct dictionary.
fn direct_contexts() -> ContextBank {
    MqBudget::default()
        .context_bank(IAID_BASE, &Limits::default())
        .unwrap()
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

/// The symbol stores a direct dictionary never reads.
struct Unread {
    imported: Source,
    new: Source,
}

impl Default for Unread {
    fn default() -> Self {
        Self {
            imported: Source::new(Vec::new()),
            new: Source::new(Vec::new()),
        }
    }
}

/// The stores of a direct dictionary: only `store` receives symbols.
fn direct_stores<'a>(
    unread: &'a mut Unread,
    store: &'a mut Store,
) -> DictionaryStores<'a, Source, Source, Store> {
    DictionaryStores {
        imported: &mut unread.imported,
        imported_base: 0,
        new_reader: &mut unread.new,
        new_writer: store,
        new_base: 0,
    }
}
