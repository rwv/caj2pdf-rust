// SPDX-License-Identifier: MIT

//! Platform-neutral session behind the raw WASM exports.
//!
//! A [`Session`] runs one conversion or inspection to completion on the
//! calling thread. Every read, write, flush, progress report and
//! cancellation check is a synchronous call into a [`Host`]. On `wasm32`,
//! `bridge.rs` implements the host with imported JavaScript functions that
//! run inside a Worker; native tests implement it over memory.

mod hnc8;

use caj2pdf_core::{
    Cancellation, ConversionOptions, ConversionReport, CountingSource, Detection, DocumentInfo,
    Error, InputFormat, Limits, PdfErrorKind, RangedSource, Result,
    caj::{convert_caj, parse_metadata},
    detect_source,
    hnc8::OutlineReport,
    kdh::{KdhPdfSource, convert_kdh},
    pdf::{PdfIndex, PdfRange, copy_pdf_range},
};
use std::{cell::RefCell, io};

/// Largest single allocation a caller may permit inside 32-bit WASM memory.
pub const MAX_ALLOCATION_LIMIT: u64 = 256 * 1024 * 1024;
/// Longest error message kept for the host, in bytes.
pub const MAX_MESSAGE_BYTES: usize = 1024;
/// Progress is reported in thousandths of the document read.
pub const PROGRESS_TOTAL: u32 = 1000;

/// Synchronous host I/O. A failed call returns an error; the host keeps its
/// own description of the failure.
pub trait Host {
    /// Copy at most `destination.len()` bytes of `resource` (0 for the
    /// document, 1..=8 for registered fonts) at `offset` into
    /// `destination`, returning the count. A short read is allowed.
    fn read(&mut self, resource: u32, offset: u64, destination: &mut [u8]) -> io::Result<usize>;
    /// Accept a prefix of `bytes`, returning its length.
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize>;
    /// The output's ordering barrier; called once, after the last write.
    fn flush(&mut self) -> io::Result<()>;
    /// `done` of `total` thousandths of the document have been read.
    fn progress(&mut self, done: u32, total: u32);
    /// Whether the caller has asked to stop.
    fn cancelled(&mut self) -> bool;
}

/// The work a session performs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    /// Convert to PDF. `None` detects the format from the leading signature.
    Convert {
        format: Option<InputFormat>,
        options: ConversionOptions,
    },
    /// Read bounded structure metadata; writes nothing.
    Inspect { format: Option<InputFormat> },
}

/// The result of [`Session::run`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    /// The operation succeeded; see [`Session::result`].
    Done = 0,
    /// The operation failed; see [`Session::result`] and [`Session::message`].
    Failed = 1,
    /// The configuration was refused before any host call.
    Invalid = 2,
    /// The session already ran an operation; reset it first.
    Busy = 3,
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

/// Stable numeric error category shared with JavaScript (1..=16).
pub fn error_code(error: &Error) -> u32 {
    match error {
        Error::UnsupportedFormat => 1,
        Error::InvalidInput { .. } => 2,
        Error::TruncatedInput { .. } => 3,
        Error::LimitExceeded { .. } => 4,
        Error::Io(_) => 5,
        Error::Cancelled => 6,
        // 7 is reserved: JavaScript reports `RANDOM_ACCESS_REQUIRED` itself.
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
        Error::Hnc8(_) | Error::Hnc8Metadata(_) => 16,
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
    /// HN-A outline entries skipped or clamped by a conversion or inspection.
    pub outline_warnings: u32,
    /// Requested C8/HN-B bookmarks were not written because their layout is
    /// unverified.
    pub outline_omitted: bool,
    /// The C8 application-info package read by an inspection; `None` when
    /// absent, defective or not C8.
    pub application_info: Option<caj2pdf_core::hnc8::ApplicationInfo>,
}

/// A ranged source served by the host. The document source (resource 0)
/// reports progress as the furthest byte read.
struct HostSource<'h, H: Host> {
    host: &'h RefCell<&'h mut H>,
    resource: u32,
    size: u64,
    furthest: u64,
    shown: Option<u32>,
}

impl<'h, H: Host> HostSource<'h, H> {
    fn new(host: &'h RefCell<&'h mut H>, resource: u32, size: u64) -> Self {
        Self {
            host,
            resource,
            size,
            furthest: 0,
            shown: None,
        }
    }

    fn report_progress(&mut self, end: u64) {
        if self.resource != 0 || end <= self.furthest {
            return;
        }
        self.furthest = end;
        // `end <= size`, so the quotient is at most `PROGRESS_TOTAL`.
        let done = (u128::from(end) * u128::from(PROGRESS_TOTAL) / u128::from(self.size)) as u32;
        if self.shown != Some(done) {
            self.shown = Some(done);
            self.host.borrow_mut().progress(done, PROGRESS_TOTAL);
        }
    }
}

impl<H: Host> RangedSource for HostSource<'_, H> {
    fn size(&self) -> u64 {
        self.size
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        // A host refuses ranges outside its resource, while the
        // `RangedSource` contract allows a read that runs past the end. Clamp
        // such a read here so it becomes a short read, as on native sources.
        let Some(remaining) = self.size.checked_sub(offset) else {
            return Err(Error::InvalidInput {
                reason: "read starts beyond source size",
            });
        };
        let wanted = usize::try_from(remaining).map_or(destination.len(), |remaining| {
            remaining.min(destination.len())
        });
        if wanted == 0 {
            return Ok(0);
        }
        let count =
            self.host
                .borrow_mut()
                .read(self.resource, offset, &mut destination[..wanted])?;
        if count > wanted {
            return Err(Error::InvalidInput {
                reason: "host read returned more bytes than requested",
            });
        }
        self.report_progress(offset + count as u64);
        Ok(count)
    }
}

/// The host's ordered output.
struct HostSink<'h, H: Host> {
    host: &'h RefCell<&'h mut H>,
}

impl<H: Host> io::Write for HostSink<'_, H> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = self.host.borrow_mut().write(bytes)?;
        if count > bytes.len() {
            return Err(io::Error::other(
                "host write accepted more bytes than offered",
            ));
        }
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.host.borrow_mut().flush()
    }
}

struct HostCancellation<'h, H: Host> {
    host: &'h RefCell<&'h mut H>,
}

impl<H: Host> Cancellation for HostCancellation<'_, H> {
    fn is_cancelled(&self) -> bool {
        self.host.borrow_mut().cancelled()
    }
}

/// Font registration, then one operation and its result. A host keeps one
/// session per WASM instance and resets it between operations.
#[derive(Default)]
pub struct Session {
    fonts: hnc8::Fonts,
    format: Option<InputFormat>,
    result: Option<Result<Outcome>>,
    message: String,
}

impl Session {
    /// Register one ranged font resource and its collection face (0 for a
    /// standalone font) before running. Returns its 1-based host resource
    /// ID, or 0 when registration is rejected.
    pub fn add_font_source(&mut self, size: u64, face: u32) -> u32 {
        if self.result.is_some() || size == 0 {
            return 0;
        }
        self.fonts.add(size, face)
    }

    /// Select zero-based font source indices before running. A missing
    /// alternate, decoration or symbol role uses `u32::MAX`; decoration
    /// aliases are Unicode scalars.
    pub fn set_c8_fonts(
        &mut self,
        cjk: u32,
        latin: u32,
        alternate: u32,
        decoration: u32,
        alias: u32,
        symbols: u32,
    ) -> bool {
        self.result.is_none()
            && self
                .fonts
                .set(cjk, latin, alternate, decoration, alias, symbols)
    }

    /// Assign a verified Latin role (state 3, 28 or 31) after the base roles,
    /// before running.
    pub fn set_c8_latin_state(&mut self, state: u32, index: u32) -> bool {
        self.result.is_none() && self.fonts.set_latin_state(state, index)
    }

    /// Run `operation` over a document of `source_size` bytes to completion.
    pub fn run<H: Host>(
        &mut self,
        host: &mut H,
        source_size: u64,
        limits: Limits,
        operation: Operation,
    ) -> Status {
        if self.result.is_some() {
            return Status::Busy;
        }
        if validate_limits(&limits).is_err() {
            return Status::Invalid;
        }
        let fonts = std::mem::take(&mut self.fonts);
        let host = RefCell::new(host);
        let mut format = None;
        let result = run(&host, source_size, &fonts, limits, operation, &mut format);
        self.format = format;
        let status = match &result {
            Ok(_) => Status::Done,
            Err(error) => {
                self.message = bounded_message(error);
                Status::Failed
            }
        };
        self.result = Some(result);
        status
    }

    /// The selected or detected format, once known.
    pub fn format(&self) -> Option<InputFormat> {
        self.format
    }

    /// The completed result, if any.
    pub fn result(&self) -> Option<&Result<Outcome>> {
        self.result.as_ref()
    }

    /// The completed outcome, if the operation succeeded.
    pub fn outcome(&self) -> Option<&Outcome> {
        self.result.as_ref()?.as_ref().ok()
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

fn run<'h, H: Host>(
    host: &'h RefCell<&'h mut H>,
    source_size: u64,
    fonts: &hnc8::Fonts,
    limits: Limits,
    operation: Operation,
    detected: &mut Option<InputFormat>,
) -> Result<Outcome> {
    let mut source = HostSource::new(host, 0, source_size);
    let mut sink = HostSink { host };
    let cancellation = HostCancellation { host };
    let explicit = match operation {
        Operation::Convert { format, .. } | Operation::Inspect { format } => format,
    };
    limits.check_input_size(source_size)?;
    let Detection {
        format,
        header_offset,
        bytes_read: detected_bytes,
    } = resolve_format(&mut source, explicit, &limits, &cancellation)?;
    *detected = Some(format);
    if fonts.count() != 0
        && !(matches!(operation, Operation::Convert { .. })
            && matches!(format, InputFormat::C8 | InputFormat::Hn))
    {
        return Err(Error::InvalidInput {
            reason: "explicit native font resources require converting a C8 or HN-B document",
        });
    }
    let mut outcome = match operation {
        Operation::Convert { options, .. } => {
            let (report, outline) = match format {
                InputFormat::Pdf => {
                    let range = pdf_range(source_size, header_offset);
                    (
                        copy_pdf_range(&mut source, &mut sink, range, &limits, &cancellation)?,
                        OutlineReport::default(),
                    )
                }
                InputFormat::Caj => (
                    convert_caj(&mut source, &mut sink, options, &limits, &cancellation)?,
                    OutlineReport::default(),
                ),
                InputFormat::Kdh => (
                    convert_kdh(&mut source, &mut sink, &limits, &cancellation)?,
                    OutlineReport::default(),
                ),
                InputFormat::Hn | InputFormat::C8 => hnc8::convert(
                    host,
                    &mut source,
                    &mut sink,
                    fonts,
                    options,
                    &limits,
                    &cancellation,
                )?,
                _ => return Err(Error::UnsupportedFormat),
            };
            Outcome {
                report,
                info: None,
                outline_warnings: outline.defects,
                outline_omitted: outline.unverified,
                application_info: None,
            }
        }
        Operation::Inspect { .. } => {
            inspect(&mut source, format, header_offset, &limits, &cancellation)?
        }
    };
    outcome.report.input_bytes_read = outcome
        .report
        .input_bytes_read
        .saturating_add(detected_bytes);
    Ok(outcome)
}

/// An explicit format skips detection, so an explicit PDF must start with
/// its `%PDF-` header.
fn resolve_format<S: RangedSource, C: Cancellation>(
    source: &mut S,
    format: Option<InputFormat>,
    limits: &Limits,
    cancellation: &C,
) -> Result<Detection> {
    if let Some(format) = format {
        return Ok(Detection {
            format,
            header_offset: 0,
            bytes_read: 0,
        });
    }
    detect_source(source, limits, cancellation)?.ok_or(Error::UnsupportedFormat)
}

/// The PDF viewed from its `%PDF-` header, which may follow leading bytes.
fn pdf_range(size: u64, header_offset: u64) -> PdfRange {
    PdfRange {
        offset: header_offset,
        length: size - header_offset,
    }
}

fn inspect<S: RangedSource, C: Cancellation>(
    source: &mut S,
    format: InputFormat,
    header_offset: u64,
    limits: &Limits,
    cancellation: &C,
) -> Result<Outcome> {
    let mut input_bytes_read = 0;
    let mut counted = CountingSource::new(source, &mut input_bytes_read);
    let mut application_info = None;
    let (page_count, bookmark_count, outline_warnings) = match format {
        InputFormat::Pdf => {
            let range = pdf_range(counted.size(), header_offset);
            (
                pdf_pages(&mut counted, range, limits, cancellation)?,
                None,
                0,
            )
        }
        InputFormat::Caj => {
            let metadata = parse_metadata(&mut counted, limits, cancellation)?;
            (
                metadata.page_count,
                Some(metadata.bookmarks.len() as u32),
                0,
            )
        }
        InputFormat::Kdh => {
            let mut decoded = KdhPdfSource::open(&mut counted, limits, cancellation)?;
            let range = pdf_range(decoded.size(), 0);
            (
                pdf_pages(&mut decoded, range, limits, cancellation)?,
                None,
                0,
            )
        }
        InputFormat::Hn | InputFormat::C8 => {
            let inspected = hnc8::inspect(&mut counted, limits, cancellation)?;
            application_info = inspected.application_info;
            (
                inspected.pages,
                inspected.bookmarks,
                inspected.outline_warnings,
            )
        }
        _ => return Err(Error::UnsupportedFormat),
    };
    Ok(Outcome {
        report: ConversionReport {
            input_bytes_read,
            ..ConversionReport::default()
        },
        info: Some(DocumentInfo {
            format,
            page_count,
            bookmark_count,
        }),
        outline_warnings,
        outline_omitted: false,
        application_info,
    })
}

fn pdf_pages<S: RangedSource, C: Cancellation>(
    source: &mut S,
    range: PdfRange,
    limits: &Limits,
    cancellation: &C,
) -> Result<u32> {
    let index = PdfIndex::open(source, range, limits, cancellation)?;
    // The PDF index enforces `Limits::max_pages`, a u32.
    Ok(index.pages().len() as u32)
}

#[cfg(test)]
mod tests;
