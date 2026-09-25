// SPDX-License-Identifier: MIT

//! Platform-neutral poll/resume engine behind the raw WASM exports.
//!
//! An [`Engine`] owns one pinned core future. The future's source and sink
//! record a single outstanding request (read, write, or flush) in shared state
//! and return `Pending`. The host performs that request with its own
//! asynchronous I/O, completes it, and polls again. The staging buffer holds
//! at most one configured chunk; no whole document crosses this boundary.
//!
//! This module has no WASM-specific code, so native unit tests drive the same
//! state machine that `bridge.rs` exposes to JavaScript.

use caj2pdf_core::{
    Cancellation, ConversionOptions, ConversionReport, DocumentInfo, Error, InputFormat, Limits,
    PdfErrorKind, RangedSource, Result, SIGNATURE_BYTES, SequentialSink,
    caj::{convert_caj, parse_metadata},
    copy_range, detect_format,
    kdh::{KdhPdfSource, convert_kdh},
    pdf::{PdfIndex, PdfRange, copy_pdf},
    read_exact_at,
};
use std::{
    cell::RefCell,
    future::{Future, poll_fn},
    pin::Pin,
    rc::Rc,
    task::{Context, Poll, Waker},
};

/// Largest single allocation a caller may permit inside 32-bit WASM memory.
pub const MAX_ALLOCATION_LIMIT: u64 = 256 * 1024 * 1024;
/// Longest error message kept for the host, in bytes.
pub const MAX_MESSAGE_BYTES: usize = 1024;

/// Poll result reported to the host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    /// The future made progress without an I/O request; poll again.
    Idle = 0,
    Read = 1,
    Write = 2,
    Flush = 3,
    Done = 4,
    Failed = 5,
}

/// One outstanding host request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Request {
    /// Copy up to `length` source bytes at `offset` into the staging buffer.
    Read { offset: u64, length: usize },
    /// Deliver the first `length` staging bytes to the sink.
    Write { length: usize },
    /// Await the sink's I/O barrier.
    Flush,
}

#[derive(Clone, Copy)]
enum Response {
    Read(usize),
    Write(usize),
    Flush,
}

/// The work an engine performs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    /// Bounded byte copy used to test the I/O contract; not conversion.
    Copy { offset: u64, length: u64 },
    /// Convert to PDF. `None` detects the format from the leading signature.
    Convert {
        format: Option<InputFormat>,
        options: ConversionOptions,
    },
    /// Read bounded structure metadata; writes nothing.
    Inspect { format: Option<InputFormat> },
}

/// Stable numeric code for an input format (0 means auto/unknown).
pub fn format_code(format: Option<InputFormat>) -> u32 {
    match format {
        None => 0,
        Some(InputFormat::Pdf) => 1,
        Some(InputFormat::Caj) => 2,
        Some(InputFormat::Kdh) => 3,
        Some(InputFormat::Hn) => 4,
        Some(InputFormat::C8) => 5,
        Some(InputFormat::Teb) => 6,
        Some(InputFormat::Nh) => 7,
    }
}

/// Inverse of [`format_code`]. The outer `None` rejects an unknown code.
pub fn format_from_code(code: u32) -> Option<Option<InputFormat>> {
    Some(match code {
        0 => None,
        1 => Some(InputFormat::Pdf),
        2 => Some(InputFormat::Caj),
        3 => Some(InputFormat::Kdh),
        4 => Some(InputFormat::Hn),
        5 => Some(InputFormat::C8),
        6 => Some(InputFormat::Teb),
        7 => Some(InputFormat::Nh),
        _ => return None,
    })
}

/// Stable numeric error category shared with JavaScript (1..=15).
pub fn error_code(error: &Error) -> u32 {
    match error {
        Error::UnsupportedFormat => 1,
        Error::InvalidInput { .. } => 2,
        Error::TruncatedInput { .. } => 3,
        Error::LimitExceeded { .. } => 4,
        Error::Io(_) => 5,
        Error::Cancelled => 6,
        Error::RandomAccessRequired => 7,
        Error::Pdf { kind, .. } => match kind {
            PdfErrorKind::Malformed => 8,
            PdfErrorKind::Encrypted => 9,
            PdfErrorKind::UnsupportedFeature => 10,
            PdfErrorKind::AmbiguousRepair => 11,
        },
        Error::PdfLimitExceeded { .. } => 12,
        Error::Caj { .. } => 13,
        Error::CajLimitExceeded { .. } => 14,
        Error::Kdh { .. } => 15,
    }
}

/// Validate caller limits for use inside 32-bit WASM memory.
pub fn validate_limits(limits: &Limits) -> Result<()> {
    limits.validate()?;
    if limits.max_allocation_bytes > MAX_ALLOCATION_LIMIT {
        return Err(Error::LimitExceeded {
            resource: "WASM allocation limit",
            limit: MAX_ALLOCATION_LIMIT,
            attempted: limits.max_allocation_bytes,
        });
    }
    Ok(())
}

/// Result of a completed operation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Outcome {
    pub report: ConversionReport,
    /// Present for [`Operation::Inspect`].
    pub info: Option<DocumentInfo>,
}

struct Shared {
    staging: Vec<u8>,
    request: Option<Request>,
    response: Option<Response>,
    cancelled: bool,
    format: Option<InputFormat>,
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
            if let Some(Response::Read(length)) = shared.response {
                shared.response = None;
                // `complete_read` bounded `length` by the requested length.
                destination[..length].copy_from_slice(&shared.staging[..length]);
                return Poll::Ready(Ok(length));
            }
            let length = destination.len().min(shared.staging.len());
            shared.request = Some(Request::Read { offset, length });
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
            if let Some(Response::Write(length)) = shared.response {
                shared.response = None;
                return Poll::Ready(Ok(length));
            }
            if shared.request.is_none() {
                // A sink may accept a prefix; the core retries the rest.
                let length = bytes.len().min(shared.staging.len());
                shared.staging[..length].copy_from_slice(&bytes[..length]);
                shared.request = Some(Request::Write { length });
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
            if let Some(Response::Flush) = shared.response {
                shared.response = None;
                return Poll::Ready(Ok(()));
            }
            shared.request = Some(Request::Flush);
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

type Task = Pin<Box<dyn Future<Output = Result<Outcome>>>>;

/// One active operation. A host keeps at most one engine per WASM instance.
pub struct Engine {
    shared: Rc<RefCell<Shared>>,
    task: Task,
    result: Option<Result<Outcome>>,
    message: String,
}

impl Engine {
    /// Validate the configuration and create a pinned operation.
    ///
    /// Nothing is read until the first [`Engine::poll`].
    pub fn start(source_size: u64, limits: Limits, operation: Operation) -> Result<Self> {
        validate_limits(&limits)?;
        let shared = Rc::new(RefCell::new(Shared {
            staging: vec![0; limits.io_chunk_bytes],
            request: None,
            response: None,
            cancelled: false,
            format: None,
        }));
        let source = BridgeSource {
            shared: Rc::clone(&shared),
            size: source_size,
        };
        let sink = BridgeSink {
            shared: Rc::clone(&shared),
        };
        let cancellation = BridgeCancellation(Rc::clone(&shared));
        let task = Box::pin(run(source, sink, cancellation, limits, operation));
        Ok(Self {
            shared,
            task,
            result: None,
            message: String::new(),
        })
    }

    /// Advance the operation until it needs host I/O or completes.
    pub fn poll(&mut self) -> Status {
        if self.result.is_none() {
            let mut context = Context::from_waker(Waker::noop());
            if let Poll::Ready(result) = self.task.as_mut().poll(&mut context) {
                if let Err(error) = &result {
                    self.message = bounded_message(error);
                }
                self.shared.borrow_mut().request = None;
                self.result = Some(result);
            }
        }
        match (&self.result, self.request()) {
            (Some(Ok(_)), _) => Status::Done,
            (Some(Err(_)), _) => Status::Failed,
            (None, Some(Request::Read { .. })) => Status::Read,
            (None, Some(Request::Write { .. })) => Status::Write,
            (None, Some(Request::Flush)) => Status::Flush,
            (None, None) => Status::Idle,
        }
    }

    /// The outstanding request, if the future is waiting for the host.
    pub fn request(&self) -> Option<Request> {
        self.shared.borrow().request
    }

    /// Run `access` with the staging chunk. The host copies read bytes into it
    /// or delivers written bytes from it.
    pub fn with_staging<T>(&self, access: impl FnOnce(&mut [u8]) -> T) -> T {
        access(&mut self.shared.borrow_mut().staging)
    }

    /// Complete a pending read with `length` bytes now in staging.
    /// Returns false, leaving the request pending, for an invalid response.
    pub fn complete_read(&self, length: usize) -> bool {
        self.complete(|request| match request {
            Request::Read {
                length: maximum, ..
            } if length <= maximum => Some(Response::Read(length)),
            _ => None,
        })
    }

    /// Complete a pending write with the number of bytes the sink accepted.
    pub fn complete_write(&self, length: usize) -> bool {
        self.complete(|request| match request {
            Request::Write { length: maximum } if length <= maximum => {
                Some(Response::Write(length))
            }
            _ => None,
        })
    }

    /// Complete a pending flush.
    pub fn complete_flush(&self) -> bool {
        self.complete(|request| (request == Request::Flush).then_some(Response::Flush))
    }

    fn complete(&self, accept: impl FnOnce(Request) -> Option<Response>) -> bool {
        let mut shared = self.shared.borrow_mut();
        let Some(response) = shared.request.and_then(accept) else {
            return false;
        };
        shared.request = None;
        shared.response = Some(response);
        true
    }

    /// Request cancellation; the next poll resolves with `Error::Cancelled`.
    pub fn cancel(&self) {
        self.shared.borrow_mut().cancelled = true;
    }

    /// The selected or detected format, once known.
    pub fn format(&self) -> Option<InputFormat> {
        self.shared.borrow().format
    }

    /// The completed result, if any.
    pub fn result(&self) -> Option<&Result<Outcome>> {
        self.result.as_ref()
    }

    /// A human-readable error message bounded to [`MAX_MESSAGE_BYTES`].
    pub fn message(&self) -> &str {
        &self.message
    }
}

fn bounded_message(error: &Error) -> String {
    let mut message = error.to_string();
    if message.len() > MAX_MESSAGE_BYTES {
        let mut end = MAX_MESSAGE_BYTES;
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
    }
    message
}

async fn run(
    mut source: BridgeSource,
    mut sink: BridgeSink,
    cancellation: BridgeCancellation,
    limits: Limits,
    operation: Operation,
) -> Result<Outcome> {
    let (format, detected_bytes) = match operation {
        Operation::Copy { offset, length } => {
            let report = copy_range(
                &mut source,
                &mut sink,
                offset,
                length,
                &limits,
                &cancellation,
            )
            .await?;
            return Ok(Outcome { report, info: None });
        }
        Operation::Convert { format, .. } | Operation::Inspect { format } => {
            limits.check_input_size(source.size)?;
            resolve_format(&mut source, format, &limits, &cancellation).await?
        }
    };
    source.shared.borrow_mut().format = Some(format);
    let mut outcome = match operation {
        Operation::Convert { options, .. } => Outcome {
            report: match format {
                InputFormat::Pdf => copy_pdf(&mut source, &mut sink, &limits, &cancellation).await,
                InputFormat::Caj => {
                    convert_caj(&mut source, &mut sink, options, &limits, &cancellation).await
                }
                InputFormat::Kdh => {
                    convert_kdh(&mut source, &mut sink, &limits, &cancellation).await
                }
                _ => Err(Error::UnsupportedFormat),
            }?,
            info: None,
        },
        _ => inspect(&mut source, format, &limits, &cancellation).await?,
    };
    outcome.report.input_bytes_read = outcome
        .report
        .input_bytes_read
        .saturating_add(detected_bytes);
    Ok(outcome)
}

async fn resolve_format(
    source: &mut BridgeSource,
    format: Option<InputFormat>,
    limits: &Limits,
    cancellation: &BridgeCancellation,
) -> Result<(InputFormat, u64)> {
    if let Some(format) = format {
        return Ok((format, 0));
    }
    let length = source.size.min(SIGNATURE_BYTES as u64) as usize;
    let mut prefix = [0; SIGNATURE_BYTES];
    read_exact_at(source, 0, &mut prefix[..length], limits, cancellation).await?;
    let format = detect_format(&prefix[..length]).ok_or(Error::UnsupportedFormat)?;
    Ok((format, length as u64))
}

async fn inspect(
    source: &mut BridgeSource,
    format: InputFormat,
    limits: &Limits,
    cancellation: &BridgeCancellation,
) -> Result<Outcome> {
    let mut counted = CountingSource {
        inner: source,
        bytes_read: 0,
    };
    let (page_count, bookmark_count) = match format {
        InputFormat::Pdf => (pdf_pages(&mut counted, limits, cancellation).await?, None),
        InputFormat::Caj => {
            let metadata = parse_metadata(&mut counted, limits, cancellation).await?;
            (metadata.page_count, Some(metadata.bookmarks.len() as u32))
        }
        InputFormat::Kdh => {
            let mut decoded = KdhPdfSource::open(&mut counted, limits, cancellation).await?;
            (pdf_pages(&mut decoded, limits, cancellation).await?, None)
        }
        _ => return Err(Error::UnsupportedFormat),
    };
    Ok(Outcome {
        report: ConversionReport {
            input_bytes_read: counted.bytes_read,
            ..ConversionReport::default()
        },
        info: Some(DocumentInfo {
            format,
            page_count,
            bookmark_count,
        }),
    })
}

async fn pdf_pages<S: RangedSource>(
    source: &mut S,
    limits: &Limits,
    cancellation: &BridgeCancellation,
) -> Result<u32> {
    let range = PdfRange {
        offset: 0,
        length: source.size(),
    };
    let index = PdfIndex::open(source, range, limits, cancellation).await?;
    // The PDF index enforces `Limits::max_pages`, a u32.
    Ok(index.pages().len() as u32)
}

struct CountingSource<'a> {
    inner: &'a mut BridgeSource,
    bytes_read: u64,
}

impl RangedSource for CountingSource<'_> {
    fn size(&self) -> u64 {
        self.inner.size
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        let read = self.inner.read_at(offset, destination).await?;
        self.bytes_read = self.bytes_read.saturating_add(read as u64);
        Ok(read)
    }
}

#[cfg(test)]
mod tests;
