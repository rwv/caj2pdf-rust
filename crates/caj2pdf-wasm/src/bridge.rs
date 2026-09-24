// SPDX-License-Identifier: MIT

//! Single-operation poll/resume bridge for JavaScript Promise based I/O.
//!
//! The caller polls a core future until it requests a range read, write, or
//! flush. JavaScript awaits that request, supplies its bounded result, then
//! polls again. The staging pointer is valid only until reset or another poll.

use caj2pdf_core::{
    Cancellation, ConversionReport, Error, Limits, MAX_IO_CHUNK, RangedSource, Result,
    SequentialSink, copy_range,
};
use std::{
    cell::RefCell,
    future::{Future, poll_fn},
    pin::Pin,
    rc::Rc,
    task::{Context, Poll, Waker},
};

const IDLE: u32 = 0;
const READ: u32 = 1;
const WRITE: u32 = 2;
const FLUSH: u32 = 3;
const DONE: u32 = 4;
const FAILED: u32 = 5;

const START_OK: u32 = 0;
const START_BUSY: u32 = 1;
const START_INVALID: u32 = 2;

#[derive(Clone, Copy)]
enum Request {
    Read { offset: u64, length: usize },
    Write { length: usize },
    Flush,
}

#[derive(Clone, Copy)]
enum Response {
    Read(usize),
    Write(usize),
    Flush,
}

struct Shared {
    staging: Vec<u8>,
    request: Option<Request>,
    response: Option<Response>,
    cancelled: bool,
}

impl Shared {
    fn new(chunk_size: usize) -> Self {
        Self {
            staging: vec![0; chunk_size],
            request: None,
            response: None,
            cancelled: false,
        }
    }
}

struct BridgeSource {
    shared: Rc<RefCell<Shared>>,
    size: u64,
}

impl RangedSource for BridgeSource {
    fn size(&self) -> u64 {
        self.size
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        poll_fn(|_| {
            let mut shared = self.shared.borrow_mut();
            if shared.cancelled {
                return Poll::Ready(Err(Error::Cancelled));
            }
            if let Some(Response::Read(length)) = shared.response.take() {
                if length > destination.len() || length > shared.staging.len() {
                    return Poll::Ready(Err(Error::InvalidInput {
                        reason: "WASM read response exceeded its buffer",
                    }));
                }
                destination[..length].copy_from_slice(&shared.staging[..length]);
                return Poll::Ready(Ok(length));
            }
            if destination.len() > shared.staging.len() {
                return Poll::Ready(Err(Error::LimitExceeded {
                    resource: "WASM staging bytes",
                    limit: shared.staging.len() as u64,
                    attempted: destination.len() as u64,
                }));
            }
            if shared.request.is_none() {
                shared.request = Some(Request::Read {
                    offset,
                    length: destination.len(),
                });
            }
            Poll::Pending
        })
        .await
    }
}

struct BridgeSink {
    shared: Rc<RefCell<Shared>>,
}

impl SequentialSink for BridgeSink {
    async fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        poll_fn(|_| {
            let mut shared = self.shared.borrow_mut();
            if shared.cancelled {
                return Poll::Ready(Err(Error::Cancelled));
            }
            if let Some(Response::Write(length)) = shared.response.take() {
                if length > bytes.len() {
                    return Poll::Ready(Err(Error::InvalidInput {
                        reason: "WASM write response exceeded its chunk",
                    }));
                }
                return Poll::Ready(Ok(length));
            }
            if shared.request.is_none() {
                if bytes.len() > shared.staging.len() {
                    return Poll::Ready(Err(Error::LimitExceeded {
                        resource: "WASM staging bytes",
                        limit: shared.staging.len() as u64,
                        attempted: bytes.len() as u64,
                    }));
                }
                shared.staging[..bytes.len()].copy_from_slice(bytes);
                shared.request = Some(Request::Write {
                    length: bytes.len(),
                });
            }
            Poll::Pending
        })
        .await
    }

    async fn flush(&mut self) -> Result<()> {
        poll_fn(|_| {
            let mut shared = self.shared.borrow_mut();
            if shared.cancelled {
                return Poll::Ready(Err(Error::Cancelled));
            }
            if let Some(Response::Flush) = shared.response.take() {
                return Poll::Ready(Ok(()));
            }
            if shared.request.is_none() {
                shared.request = Some(Request::Flush);
            }
            Poll::Pending
        })
        .await
    }
}

struct BridgeCancellation(Rc<RefCell<Shared>>);

impl Cancellation for BridgeCancellation {
    fn is_cancelled(&self) -> bool {
        self.0.borrow().cancelled
    }
}

type Operation = Pin<Box<dyn Future<Output = Result<ConversionReport>>>>;

struct Engine {
    shared: Rc<RefCell<Shared>>,
    operation: Operation,
    result: Option<Result<ConversionReport>>,
}

thread_local! {
    static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
}

/// Start one bounded copy proof. A WASM instance permits one active operation.
/// Returns 0 on success, 1 when busy, or 2 for an invalid chunk size.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_start(
    source_size: u64,
    offset: u64,
    length: u64,
    chunk_size: u32,
) -> u32 {
    let chunk_size = chunk_size as usize;
    if chunk_size == 0 || chunk_size > MAX_IO_CHUNK {
        return START_INVALID;
    }
    ENGINE.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_some() {
            return START_BUSY;
        }
        let shared = Rc::new(RefCell::new(Shared::new(chunk_size)));
        let task_shared = Rc::clone(&shared);
        let operation = Box::pin(async move {
            let mut source = BridgeSource {
                shared: Rc::clone(&task_shared),
                size: source_size,
            };
            let mut sink = BridgeSink {
                shared: Rc::clone(&task_shared),
            };
            let cancellation = BridgeCancellation(task_shared);
            let limits = Limits {
                io_chunk_bytes: chunk_size,
                ..Limits::default()
            };
            copy_range(
                &mut source,
                &mut sink,
                offset,
                length,
                &limits,
                &cancellation,
            )
            .await
        });
        *slot = Some(Engine {
            shared,
            operation,
            result: None,
        });
        START_OK
    })
}

/// Poll the core future. 1=read, 2=write, 3=flush, 4=done, 5=error.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_poll() -> u32 {
    ENGINE.with(|cell| {
        let mut slot = cell.borrow_mut();
        let Some(engine) = slot.as_mut() else {
            return IDLE;
        };
        if let Some(result) = &engine.result {
            return if result.is_ok() { DONE } else { FAILED };
        }
        let mut context = Context::from_waker(Waker::noop());
        if let Poll::Ready(result) = engine.operation.as_mut().poll(&mut context) {
            engine.result = Some(result);
        }
        if let Some(result) = &engine.result {
            return if result.is_ok() { DONE } else { FAILED };
        }
        match engine.shared.borrow().request {
            Some(Request::Read { .. }) => READ,
            Some(Request::Write { .. }) => WRITE,
            Some(Request::Flush) => FLUSH,
            None => IDLE,
        }
    })
}

/// Byte offset for the outstanding read request (JavaScript BigInt).
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_request_offset() -> u64 {
    ENGINE.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|engine| match engine.shared.borrow().request {
                Some(Request::Read { offset, .. }) => Some(offset),
                _ => None,
            })
            .unwrap_or(0)
    })
}

/// Byte count for the outstanding read or write request.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_request_length() -> u32 {
    ENGINE.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|engine| match engine.shared.borrow().request {
                Some(Request::Read { length, .. } | Request::Write { length }) => {
                    Some(length as u32)
                }
                _ => None,
            })
            .unwrap_or(0)
    })
}

/// Pointer into exported WASM memory for one bounded staging chunk.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_buffer_ptr() -> u32 {
    ENGINE.with(|cell| {
        cell.borrow_mut()
            .as_mut()
            .map(|engine| engine.shared.borrow_mut().staging.as_mut_ptr() as u32)
            .unwrap_or(0)
    })
}

/// Complete the pending read after JavaScript copies bytes into staging memory.
/// Returns 1 only for a valid response; a zero-length read is allowed.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_complete_read(length: u32) -> u32 {
    complete_response(|request| match request {
        Request::Read {
            length: maximum, ..
        } if length as usize <= maximum => Some(Response::Read(length as usize)),
        _ => None,
    })
}

/// Complete a pending write after the sink accepts a prefix of its chunk.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_complete_write(length: u32) -> u32 {
    complete_response(|request| match request {
        Request::Write { length: maximum } if length as usize <= maximum => {
            Some(Response::Write(length as usize))
        }
        _ => None,
    })
}

/// Complete an awaited sink flush.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_complete_flush() -> u32 {
    complete_response(|request| match request {
        Request::Flush => Some(Response::Flush),
        _ => None,
    })
}

fn complete_response(accept: impl FnOnce(Request) -> Option<Response>) -> u32 {
    ENGINE.with(|cell| {
        let slot = cell.borrow();
        let Some(engine) = slot.as_ref() else {
            return 0;
        };
        let mut shared = engine.shared.borrow_mut();
        if shared.response.is_some() {
            return 0;
        }
        let Some(request) = shared.request else {
            return 0;
        };
        let Some(response) = accept(request) else {
            return 0;
        };
        shared.request = None;
        shared.response = Some(response);
        1
    })
}

/// Cancel the active operation; the next poll resolves with a typed error.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_cancel() {
    ENGINE.with(|cell| {
        if let Some(engine) = cell.borrow().as_ref() {
            engine.shared.borrow_mut().cancelled = true;
        }
    });
}

/// Numeric error category for a completed operation (1..=12).
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_error_kind() -> u32 {
    ENGINE.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|engine| engine.result.as_ref())
            .and_then(|result| result.as_ref().err())
            .map(|error| match error {
                Error::UnsupportedFormat => 1,
                Error::InvalidInput { .. } => 2,
                Error::TruncatedInput { .. } => 3,
                Error::LimitExceeded { .. } => 4,
                Error::Io(_) => 5,
                Error::Cancelled => 6,
                Error::RandomAccessRequired => 7,
                Error::Pdf { kind, .. } => match kind {
                    caj2pdf_core::PdfErrorKind::Malformed => 8,
                    caj2pdf_core::PdfErrorKind::Encrypted => 9,
                    caj2pdf_core::PdfErrorKind::UnsupportedFeature => 10,
                    caj2pdf_core::PdfErrorKind::AmbiguousRepair => 11,
                },
                Error::PdfLimitExceeded { .. } => 12,
            })
            .unwrap_or(0)
    })
}

/// Bytes read by a successful proof; zero when unavailable.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_input_bytes_read() -> u64 {
    ENGINE.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|engine| engine.result.as_ref())
            .and_then(|result| result.as_ref().ok())
            .map(|report| report.input_bytes_read)
            .unwrap_or(0)
    })
}

/// Bytes written by a successful proof; zero when unavailable.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_output_bytes_written() -> u64 {
    ENGINE.with(|cell| {
        cell.borrow()
            .as_ref()
            .and_then(|engine| engine.result.as_ref())
            .and_then(|result| result.as_ref().ok())
            .map(|report| report.output_bytes_written)
            .unwrap_or(0)
    })
}

/// Release the operation and its bounded staging allocation.
#[unsafe(no_mangle)]
pub extern "C" fn caj2pdf_io_reset() {
    ENGINE.with(|cell| *cell.borrow_mut() = None);
}
