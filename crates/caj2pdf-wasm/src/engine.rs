// SPDX-License-Identifier: MIT

//! Platform-neutral session behind the raw WASM exports.
//!
//! A [`Session`] runs one conversion or inspection to completion on the
//! calling thread. Every read, write, flush, progress report and
//! cancellation check is a synchronous call into a [`Host`]. On `wasm32`,
//! `bridge.rs` implements the host with imported JavaScript functions that
//! run inside a Worker; native tests implement it over memory.

mod fonts;

use caj2pdf_core::{
    Context, ConversionOptions, ConversionReport, DocumentInfo, Error, ErrorKind,
    FONTS_REQUIRE_HNC8, InputFormat, InspectOptions, Limits, Progress, RangedSource, Result,
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

/// The work a session performs. A `None` format is detected from the
/// leading signature.
pub enum Operation {
    /// Convert to PDF with the registered fonts, if any.
    Convert { options: ConversionOptions<'static> },
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
        Some(InputFormat::Caa) => 8,
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
        8 => Some(InputFormat::Caa),
        _ => return None,
    })
}

/// Stable numeric error category shared with JavaScript (1..=16), from the
/// error's kind and the structure it was located in. An I/O failure or a
/// cancellation has its own code wherever it happened; any other HN/C8 or
/// JBIG2 failure is `16`; a truncation is `3` and a resource limit outside
/// a CAJ or PDF structure is `4`.
pub fn error_code(error: &Error) -> u32 {
    match (&error.kind, error.context) {
        (ErrorKind::Io(_), _) => 5,
        (ErrorKind::Cancelled, _) => 6,
        // 7 is reserved: JavaScript reports `RANDOM_ACCESS_REQUIRED` itself.
        (_, Context::Hnc8 { .. } | Context::Jbig2 { .. }) => 16,
        (ErrorKind::Truncated { .. }, _) => 3,
        (ErrorKind::LimitExceeded { .. }, Context::Pdf { .. }) => 12,
        (ErrorKind::LimitExceeded { .. }, Context::Caj { .. }) => 14,
        (ErrorKind::LimitExceeded { .. }, _) => 4,
        (ErrorKind::Malformed, Context::Pdf { repair: true, .. }) => 11,
        (ErrorKind::Malformed, Context::Pdf { .. }) => 8,
        (ErrorKind::Encrypted, Context::Pdf { .. }) => 9,
        (ErrorKind::UnsupportedFormat, Context::Pdf { .. }) => 10,
        (_, Context::Caj { .. }) => 13,
        (_, Context::Kdh) => 15,
        (ErrorKind::UnsupportedFormat, _) => 1,
        (ErrorKind::Malformed | ErrorKind::Encrypted, Context::None) => 2,
    }
}

/// Validate caller limits for use inside 32-bit WASM memory.
pub fn validate_limits(limits: &Limits) -> Result<()> {
    limits.validate()?;
    if limits.max_allocation_bytes > MAX_ALLOCATION_LIMIT {
        return Err(Error::limit(
            "WASM allocation limit",
            MAX_ALLOCATION_LIMIT,
            limits.max_allocation_bytes,
        ));
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

/// A ranged source served by the host: the document (resource 0) or a font.
struct HostSource<'h, H: Host> {
    host: &'h RefCell<&'h mut H>,
    resource: u32,
    size: u64,
}

impl<'h, H: Host> HostSource<'h, H> {
    fn new(host: &'h RefCell<&'h mut H>, resource: u32, size: u64) -> Self {
        Self {
            host,
            resource,
            size,
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
            return Err(Error::invalid("read starts beyond source size"));
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
            return Err(Error::invalid(
                "host read returned more bytes than requested",
            ));
        }
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

/// The reported format, progress in thousandths of the document read, and
/// the host's cancellation.
struct HostProgress<'h, H: Host> {
    host: &'h RefCell<&'h mut H>,
    format: Option<InputFormat>,
    shown: Option<u32>,
}

impl<H: Host> Progress for HostProgress<'_, H> {
    fn format(&mut self, format: Option<InputFormat>) {
        self.format = format;
    }

    fn input_read(&mut self, done: u64, total: u64) {
        // A host source never reads past its size, so the quotient is at
        // most `PROGRESS_TOTAL`.
        let done = (u128::from(done.min(total)) * u128::from(PROGRESS_TOTAL)
            / u128::from(total.max(1))) as u32;
        if self.shown != Some(done) {
            self.shown = Some(done);
            self.host.borrow_mut().progress(done, PROGRESS_TOTAL);
        }
    }

    fn is_cancelled(&self) -> bool {
        self.host.borrow_mut().cancelled()
    }
}

/// Font registration, then one operation and its result. A host keeps one
/// session per WASM instance and resets it between operations.
#[derive(Default)]
pub struct Session {
    response: Option<caj2pdf_core::pdf::TtknResponse>,
    fonts: fonts::Fonts,
    format: Option<InputFormat>,
    result: Option<Result<Outcome>>,
    message: String,
}

impl Session {
    /// Set an explicit, case-sensitive TTKN response before conversion.
    pub fn set_ttkn_response(&mut self, ascii: &[u8]) -> bool {
        if self.result.is_some() {
            return false;
        }
        match caj2pdf_core::pdf::TtknResponse::new(ascii) {
            Ok(response) => {
                self.response = Some(response);
                true
            }
            Err(_) => {
                self.response = None;
                false
            }
        }
    }

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

    /// Map an HN-B mode-0 symbol code to a glyph of the symbols role, after
    /// the roles and before running.
    pub fn add_hnb_symbol_glyph(&mut self, code: u32, glyph: u32) -> bool {
        self.result.is_none() && self.fonts.add_symbol_glyph(code, glyph)
    }

    /// Bind the symbol glyph map to the symbols font's identity, after the
    /// roles and before running.
    pub fn set_hnb_symbol_font(&mut self, checksum: u32, length: u32, words: [u64; 8]) -> bool {
        self.result.is_none() && self.fonts.set_symbol_font(checksum, length, words)
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
        let response = self.response.take();
        let result = run(
            &host,
            source_size,
            &fonts,
            response.as_ref(),
            limits,
            operation,
            &mut format,
        );
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

/// Locate an unlocated failure of an HN/C8 operation in HN/C8, so that it
/// keeps the `HNC8` code.
fn in_hnc8(error: Error, format: Option<InputFormat>) -> Error {
    match format {
        Some(InputFormat::Hn | InputFormat::C8) if error.context == Context::None => {
            error.within(Context::Hnc8 {
                variant: None,
                page: None,
                image: None,
                segment: None,
                stage: None,
            })
        }
        _ => error,
    }
}

fn run<'h, H: Host>(
    host: &'h RefCell<&'h mut H>,
    source_size: u64,
    fonts: &fonts::Fonts,
    response: Option<&caj2pdf_core::pdf::TtknResponse>,
    limits: Limits,
    operation: Operation,
    detected: &mut Option<InputFormat>,
) -> Result<Outcome> {
    let mut source = HostSource::new(host, 0, source_size);
    let mut progress = HostProgress {
        host,
        format: None,
        shown: None,
    };
    limits.check_input_size(source_size)?;
    let result = match operation {
        Operation::Convert { options } => {
            let options = ConversionOptions {
                fonts: fonts.resources(host),
                ..options
            };
            let mut sink = HostSink { host };
            let result = match response {
                Some(response) => caj2pdf_core::convert_with_ttkn_response(
                    &mut source,
                    &mut sink,
                    options,
                    response,
                    &limits,
                    &mut progress,
                ),
                None => {
                    caj2pdf_core::convert(&mut source, &mut sink, options, &limits, &mut progress)
                }
            };
            result.map(|report| Outcome {
                outline_warnings: report.outline.defects,
                outline_omitted: report.outline.unverified,
                report,
                info: None,
                application_info: None,
            })
        }
        Operation::Inspect { .. } if response.is_some() => {
            return Err(Error::invalid(
                "TTKN response is only supported for conversion",
            ));
        }
        Operation::Inspect { .. } if fonts.count() != 0 => {
            return Err(Error::invalid(FONTS_REQUIRE_HNC8));
        }
        Operation::Inspect { format } => {
            let options = InspectOptions {
                format,
                ..InspectOptions::default()
            };
            caj2pdf_core::inspect(&mut source, &options, &limits, &mut progress).and_then(inspected)
        }
    };
    *detected = progress.format;
    result.map_err(|error| in_hnc8(error, progress.format))
}

/// An inspection's outcome. CAA descriptors expose their family without
/// document counts; other count-less inputs keep the existing refusal.
fn inspected(info: DocumentInfo) -> Result<Outcome> {
    if info.page_count.is_none() && info.format != InputFormat::Caa {
        return Err(ErrorKind::UnsupportedFormat.into());
    }
    Ok(Outcome {
        report: ConversionReport {
            input_bytes_read: info.input_bytes_read,
            ..ConversionReport::default()
        },
        outline_warnings: info.outline.defects,
        outline_omitted: false,
        application_info: info.application_info.info.clone(),
        info: Some(info),
    })
}

#[cfg(test)]
mod tests;
