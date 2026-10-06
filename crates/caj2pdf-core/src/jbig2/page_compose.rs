// SPDX-License-Identifier: MIT

//! Bounded bytewise OR of the observed full-page text and generic regions.
//!
//! The composed text region is a packed bitmap in memory. A validated generic
//! decoder sends its packed rows through this sequential sink, which ORs each
//! accepted chunk with the same text bytes and forwards it at once; this
//! module decodes no arithmetic data.

use super::{
    generic::{GenericRegionHeader, GenericRegionInfo, GenericReport},
    page_info::PageInfo,
    page_profile::PageProfile,
    text::TextHeaderAnomaly,
    text_composer::{TextComposeReport, TextComposeStage},
};
use crate::{Cancellation, Error, Limits, MAX_BUDGET_COUNT};
use std::io::Write;
use std::{error, fmt};

/// Independent bounds for a single page OR operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageComposeBudget {
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels: u64,
    pub max_packed_bytes: u64,
    pub max_generic_bytes: u64,
    pub max_output_bytes: u64,
    pub max_output_write_calls: u64,
    pub max_work_units: u64,
    pub max_generic_request_bytes: usize,
    pub max_output_request_bytes: usize,
    pub max_resident_bytes: u64,
}

impl Default for PageComposeBudget {
    fn default() -> Self {
        Self {
            max_width: 32_768,
            max_height: 32_768,
            max_pixels: 12_000_000,
            max_packed_bytes: 128 * 1024 * 1024,
            max_generic_bytes: 128 * 1024 * 1024,
            max_output_bytes: 128 * 1024 * 1024,
            max_output_write_calls: 10_000_000,
            max_work_units: 400_000_000,
            max_generic_request_bytes: 256 * 1024,
            max_output_request_bytes: 256 * 1024,
            max_resident_bytes: 256 * 1024,
        }
    }
}

/// Output and semantic progress. A failed operation's output is not a
/// completed page, even when `output_bytes_written` equals `packed_bytes`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PageComposeProgress {
    pub generic_bytes_accepted: u64,
    pub output_bytes_written: u64,
    pub rows_written: u32,
    pub output_write_calls: u64,
    /// One unit per ORed byte and per byte written.
    pub work_units: u64,
    pub max_request_bytes: usize,
    pub peak_resident_bytes: u64,
    pub producer_flushed: bool,
}

/// One validated, fully emitted packed page. No PDF has been created here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageComposeReport {
    pub page: PageInfo,
    pub text_flags_raw: u16,
    pub text_header_anomaly: Option<TextHeaderAnomaly>,
    pub text_segment: u32,
    pub generic_segment: u32,
    pub progress: PageComposeProgress,
}

#[derive(Debug)]
pub struct PageComposeError {
    /// Page or output byte coordinate, selected by the error kind.
    pub offset: u64,
    pub progress: Box<PageComposeProgress>,
    pub kind: PageComposeErrorKind,
}

#[derive(Debug)]
pub enum PageComposeErrorKind {
    Malformed(&'static str),
    InvalidSpan(&'static str),
    LimitExceeded {
        resource: &'static str,
        limit: u64,
        attempted: u64,
    },
    AllocationFailed,
    Cancelled,
    Limits(Error),
    Output(Error),
    Incomplete,
}

pub type PageComposeResult<T> = Result<T, PageComposeError>;

impl fmt::Display for PageComposeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "JBIG2 page OR at byte {}: ", self.offset)?;
        match &self.kind {
            PageComposeErrorKind::Malformed(reason) => write!(f, "malformed {reason}"),
            PageComposeErrorKind::InvalidSpan(reason) => write!(f, "invalid span: {reason}"),
            PageComposeErrorKind::LimitExceeded {
                resource,
                limit,
                attempted,
            } => write!(f, "{resource} limit {limit} exceeded by {attempted}"),
            PageComposeErrorKind::AllocationFailed => f.write_str("chunk allocation failed"),
            PageComposeErrorKind::Cancelled => f.write_str("cancelled"),
            PageComposeErrorKind::Limits(source) => write!(f, "limits: {source}"),
            PageComposeErrorKind::Output(source) => write!(f, "output: {source}"),
            PageComposeErrorKind::Incomplete => f.write_str("page rows are incomplete"),
        }
    }
}

impl error::Error for PageComposeError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match &self.kind {
            PageComposeErrorKind::Limits(source) | PageComposeErrorKind::Output(source) => {
                Some(source)
            }
            _ => None,
        }
    }
}

fn at(offset: u64, kind: PageComposeErrorKind) -> PageComposeError {
    PageComposeError {
        offset,
        progress: Box::new(PageComposeProgress::default()),
        kind,
    }
}

fn cap(resource: &'static str, maximum: u64, attempted: u64) -> PageComposeResult<()> {
    if attempted > maximum {
        Err(at(
            0,
            PageComposeErrorKind::LimitExceeded {
                resource,
                limit: maximum,
                attempted,
            },
        ))
    } else {
        Ok(())
    }
}

fn checked_resident_capacity(
    actual: usize,
    budget: &PageComposeBudget,
    limits: &Limits,
) -> PageComposeResult<u64> {
    let actual = actual as u64;
    cap(
        "resident chunk bytes",
        budget.max_resident_bytes.min(limits.max_allocation_bytes),
        actual,
    )?;
    Ok(actual)
}

fn output_error(error: Error) -> PageComposeErrorKind {
    if matches!(error, Error::Cancelled) {
        PageComposeErrorKind::Cancelled
    } else {
        PageComposeErrorKind::Output(error)
    }
}

/// The only page output path for an already preflighted observed profile.
///
/// Call `GenericRegionDecoder::arm_page_output(profile.generic_header())`
/// before decoding a row. That method checks the decoder's actual header and
/// arms this sink only when it matches preflight; writes and flushes before
/// arming fail.
/// `GenericRegionDecoder` may call `write` several times per row. This adapter
/// accepts at most one row remainder and one bounded chunk per call, so its
/// caller retries the unaccepted suffix as `Write::write_all` does. Every
/// accepted chunk is ORed with the same-position text bytes and immediately
/// forwarded. `flush` records successful generic-stream completion; `finish`
/// separately checks `GenericReport` and flushes the final output. After any
/// failure, discard the final output and call `take_failure` for a typed sink
/// error.
pub struct PageOrSink<'a, W: Write, C: Cancellation> {
    profile: PageProfile,
    text: TextComposeReport,
    bitmap: &'a [u8],
    output: &'a mut W,
    cancellation: &'a C,
    budget: PageComposeBudget,
    chunk: Vec<u8>,
    progress: PageComposeProgress,
    failure: Option<PageComposeError>,
    armed: bool,
}

impl<'a, W: Write, C: Cancellation> PageOrSink<'a, W, C> {
    /// `bitmap` is the packed text region that `text` reports.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        profile: PageProfile,
        text: TextComposeReport,
        bitmap: &'a [u8],
        output: &'a mut W,
        limits: &Limits,
        cancellation: &'a C,
        budget: PageComposeBudget,
    ) -> PageComposeResult<Self> {
        limits
            .validate()
            .map_err(|error| at(0, PageComposeErrorKind::Limits(error)))?;
        if cancellation.is_cancelled() {
            return Err(at(0, PageComposeErrorKind::Cancelled));
        }
        if budget.max_generic_request_bytes == 0
            || budget.max_output_request_bytes == 0
            || budget.max_resident_bytes == 0
        {
            return Err(at(
                0,
                PageComposeErrorKind::Malformed("zero request or resident cap"),
            ));
        }
        let counters = [
            budget.max_packed_bytes,
            budget.max_generic_bytes,
            budget.max_output_bytes,
            budget.max_output_write_calls,
            budget.max_work_units,
        ];
        if counters.into_iter().any(|count| count > MAX_BUDGET_COUNT) {
            return Err(at(
                0,
                PageComposeErrorKind::Malformed("budget count exceeds hard ceiling"),
            ));
        }
        // PageProfile can only be made by the checked observed-profile
        // preflight. Its dimensions, packed geometry, flags, and operators
        // are therefore already validated before this sink is constructed.
        let page = profile.page();
        let stride = page.row_stride as u64;
        let packed = page.packed_bytes;
        if text.width != page.width
            || text.height != page.height
            || u64::from(text.row_stride) != stride
            || text.packed_bytes != packed
            || text.header != profile.text_header()
            || text.text_flags_raw != profile.text_flags_raw()
            || text.header_anomaly != profile.text_header_anomaly()
        {
            return Err(at(
                0,
                PageComposeErrorKind::Malformed("text report differs from page profile"),
            ));
        }
        if text.progress.stage != TextComposeStage::Complete {
            return Err(at(0, PageComposeErrorKind::Incomplete));
        }
        // u32 dimensions make both products fit u64. With packed rows,
        // packed * 2 stays below 2^63 even at the u32 dimension ceiling.
        let pixels = u64::from(page.width) * u64::from(page.height);
        let work = packed * 2;
        cap(
            "page width",
            u64::from(budget.max_width),
            u64::from(page.width),
        )?;
        cap(
            "page height",
            u64::from(budget.max_height),
            u64::from(page.height),
        )?;
        cap("page pixels", budget.max_pixels, pixels)?;
        cap("packed page bytes", budget.max_packed_bytes, packed)?;
        cap("generic input bytes", budget.max_generic_bytes, packed)?;
        cap(
            "page output bytes",
            budget.max_output_bytes.min(limits.max_output_bytes),
            packed,
        )?;
        cap("page work units", budget.max_work_units, work)?;
        if bitmap.len() as u64 != packed {
            return Err(at(
                0,
                PageComposeErrorKind::InvalidSpan("text bitmap size differs"),
            ));
        }
        let chunk_size = page
            .row_stride
            .min(budget.max_generic_request_bytes)
            .min(budget.max_output_request_bytes)
            .min(limits.io_chunk_bytes)
            .min(usize::try_from(budget.max_resident_bytes).unwrap_or(usize::MAX));
        // Every factor is nonzero and chunk_size <= limits.io_chunk_bytes;
        // Limits::validate already capped that chunk by max_allocation_bytes.
        let mut chunk = Vec::new();
        chunk
            .try_reserve_exact(chunk_size)
            .map_err(|_| at(0, PageComposeErrorKind::AllocationFailed))?;
        let resident = checked_resident_capacity(chunk.capacity(), &budget, limits)?;
        chunk.resize(chunk_size, 0);
        Ok(Self {
            profile,
            text,
            bitmap,
            output,
            cancellation,
            budget,
            chunk,
            progress: PageComposeProgress {
                peak_resident_bytes: resident,
                ..PageComposeProgress::default()
            },
            failure: None,
            armed: false,
        })
    }

    pub fn progress(&self) -> PageComposeProgress {
        self.progress
    }

    /// Retrieve the first typed failure raised through `Write`.
    pub fn take_failure(&mut self) -> Option<PageComposeError> {
        self.failure.take()
    }

    fn error(&self, offset: u64, kind: PageComposeErrorKind) -> PageComposeError {
        PageComposeError {
            offset,
            progress: Box::new(self.progress),
            kind,
        }
    }

    fn fail(&mut self, offset: u64, kind: PageComposeErrorKind) -> Error {
        if self.failure.is_none() {
            self.failure = Some(self.error(offset, kind));
        }
        Error::InvalidInput {
            reason: "JBIG2 page OR failed; inspect PageOrSink::take_failure",
        }
    }

    fn check_cancelled(&mut self, offset: u64) -> Result<(), Error> {
        if self.cancellation.is_cancelled() {
            Err(self.fail(offset, PageComposeErrorKind::Cancelled))
        } else {
            Ok(())
        }
    }

    fn note_request(&mut self, count: usize) {
        self.progress.max_request_bytes = self.progress.max_request_bytes.max(count);
    }

    fn validate_generic_info(&self, info: GenericRegionInfo) -> PageComposeResult<()> {
        let page = self.profile.page();
        if info.width != page.width
            || info.height != page.height
            || info.x != 0
            || info.y != 0
            || info.row_stride != page.row_stride
            || info.combination_operator != 0
        {
            return Err(self.error(
                0,
                PageComposeErrorKind::Malformed("generic region differs from preflighted page"),
            ));
        }
        Ok(())
    }

    /// Verify a complete generic-region report, then flush the final output.
    /// The successful report proves neither external corpus parity nor PDF
    /// generation; it proves this bounded bytewise composition only.
    pub fn finish(mut self, generic: &GenericReport) -> PageComposeResult<PageComposeReport> {
        if let Some(failure) = self.failure.take() {
            return Err(failure);
        }
        self.validate_generic_info(generic.progress.info)?;
        if generic.data != self.profile.generic_header().data
            || generic.mq_span != self.profile.generic_header().mq_span
        {
            return Err(self.error(
                0,
                PageComposeErrorKind::InvalidSpan(
                    "generic report span differs from preflighted segment",
                ),
            ));
        }
        let page = self.profile.page();
        let pixels = u64::from(page.width) * u64::from(page.height);
        if !self.armed
            || !self.progress.producer_flushed
            || self.progress.generic_bytes_accepted != page.packed_bytes
            || self.progress.output_bytes_written != page.packed_bytes
            || self.progress.rows_written != page.height
            || generic.progress.rows_written != page.height
            || generic.progress.pixels_decoded != pixels
            || generic.progress.output_bytes_written != page.packed_bytes
            || generic.progress.mq.symbols_decoded != pixels
        {
            return Err(self.error(
                self.progress.output_bytes_written,
                PageComposeErrorKind::Incomplete,
            ));
        }
        if self.cancellation.is_cancelled() {
            return Err(self.error(
                self.progress.output_bytes_written,
                PageComposeErrorKind::Cancelled,
            ));
        }
        self.output.flush().map_err(|error| {
            self.error(
                self.progress.output_bytes_written,
                output_error(error.into()),
            )
        })?;
        // A flush can take long enough for the caller to give up.
        if self.cancellation.is_cancelled() {
            return Err(self.error(
                self.progress.output_bytes_written,
                PageComposeErrorKind::Cancelled,
            ));
        }
        Ok(PageComposeReport {
            page,
            text_flags_raw: self.text.text_flags_raw,
            text_header_anomaly: self.text.header_anomaly,
            text_segment: self.profile.text_segment(),
            generic_segment: self.profile.generic_segment(),
            progress: self.progress,
        })
    }

    fn accept_chunk(&mut self, bytes: &[u8]) -> Result<usize, Error> {
        let page = self.profile.page();
        let offset = self.progress.generic_bytes_accepted;
        self.check_cancelled(offset)?;
        if offset >= page.packed_bytes {
            return Err(self.fail(
                offset,
                PageComposeErrorKind::Malformed("extra generic bytes"),
            ));
        }
        let column = (offset % page.row_stride as u64) as usize;
        let row_remaining = page.row_stride - column;
        let count = bytes.len().min(row_remaining).min(self.chunk.len());
        self.note_request(count);
        let mask = if page.width.is_multiple_of(8) {
            0xff
        } else {
            0xff << (8 - page.width % 8)
        };
        let last_column = page.row_stride - 1;
        // `offset < packed_bytes`, the bitmap length, which fits a `usize`.
        let text = &self.bitmap[offset as usize..offset as usize + count];
        for (index, (&generic, &value)) in bytes[..count].iter().zip(text).enumerate() {
            if column + index == last_column && ((generic | value) & !mask) != 0 {
                return Err(self.fail(
                    offset + index as u64,
                    PageComposeErrorKind::Malformed("nonzero row padding"),
                ));
            }
            self.chunk[index] = generic | value;
        }
        self.progress.generic_bytes_accepted += count as u64;
        self.progress.work_units += count as u64;
        let mut sent = 0usize;
        while sent < count {
            self.check_cancelled(offset + sent as u64)?;
            let requested = (count - sent).min(self.budget.max_output_request_bytes);
            let attempted = self.progress.output_write_calls + 1;
            if attempted > self.budget.max_output_write_calls {
                return Err(self.fail(
                    offset + sent as u64,
                    PageComposeErrorKind::LimitExceeded {
                        resource: "output write calls",
                        limit: self.budget.max_output_write_calls,
                        attempted,
                    },
                ));
            }
            self.progress.output_write_calls = attempted;
            self.note_request(requested);
            if let Err(error) = self.output.write_all(&self.chunk[sent..sent + requested]) {
                return Err(self.fail(offset + sent as u64, output_error(error.into())));
            }
            self.progress.output_bytes_written += requested as u64;
            self.progress.work_units += requested as u64;
            sent += requested;
        }
        if column + count == page.row_stride {
            self.progress.rows_written += 1;
        }
        Ok(count)
    }
}

impl<W: Write, C: Cancellation> Write for PageOrSink<'_, W, C> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if !self.armed || self.progress.producer_flushed {
            return Err(self
                .fail(
                    self.progress.generic_bytes_accepted,
                    PageComposeErrorKind::Malformed("page output is not armed or already flushed"),
                )
                .into());
        }
        if bytes.is_empty() {
            return Ok(0);
        }
        Ok(self.accept_chunk(bytes)?)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if !self.armed {
            return Err(self
                .fail(
                    self.progress.generic_bytes_accepted,
                    PageComposeErrorKind::Malformed("page output is not armed or already flushed"),
                )
                .into());
        }
        let page = self.profile.page();
        if self.progress.generic_bytes_accepted != page.packed_bytes
            || self.progress.output_bytes_written != page.packed_bytes
            || self.progress.rows_written != page.height
        {
            return Err(self
                .fail(
                    self.progress.generic_bytes_accepted,
                    PageComposeErrorKind::Incomplete,
                )
                .into());
        }
        self.check_cancelled(self.progress.generic_bytes_accepted)?;
        self.progress.producer_flushed = true;
        Ok(())
    }
}

impl<W: Write, C: Cancellation> PageOrSink<'_, W, C> {
    /// Accept rows only after the generic decoder compared the header it
    /// parsed with its caller's preflight header; see
    /// [`GenericRegionDecoder::arm_page_output`](super::generic::GenericRegionDecoder::arm_page_output).
    pub(super) fn arm_checked_header(&mut self, header: GenericRegionHeader) -> crate::Result<()> {
        if self.armed {
            return Err(self.fail(
                0,
                PageComposeErrorKind::Malformed("page output is already armed"),
            ));
        }
        let expected = self.profile.generic_header();
        if header.data != expected.data || header.mq_span != expected.mq_span {
            return Err(self.fail(
                0,
                PageComposeErrorKind::InvalidSpan(
                    "generic checked header span differs from page preflight",
                ),
            ));
        }
        if header.info != expected.info || header.pixels != expected.pixels {
            return Err(self.fail(
                0,
                PageComposeErrorKind::Malformed(
                    "generic checked header geometry differs from page preflight",
                ),
            ));
        }
        if header.segment != expected.segment
            || header.page_association != expected.page_association
            || header.reference_count != expected.reference_count
        {
            return Err(self.fail(
                0,
                PageComposeErrorKind::Malformed(
                    "generic checked header segment metadata differs from page preflight",
                ),
            ));
        }
        self.armed = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actual_allocator_capacity_must_fit_both_resident_limits() {
        let budget = PageComposeBudget {
            max_resident_bytes: 3,
            ..PageComposeBudget::default()
        };
        let limits = Limits {
            max_allocation_bytes: 4,
            ..Limits::default()
        };
        assert_eq!(checked_resident_capacity(3, &budget, &limits).unwrap(), 3);
        let error = checked_resident_capacity(4, &budget, &limits).unwrap_err();
        assert!(matches!(
            error.kind,
            PageComposeErrorKind::LimitExceeded {
                resource: "resident chunk bytes",
                limit: 3,
                attempted: 4,
            }
        ));
        let larger_budget = PageComposeBudget {
            max_resident_bytes: 5,
            ..budget
        };
        assert!(matches!(
            checked_resident_capacity(5, &larger_budget, &limits)
                .unwrap_err()
                .kind,
            PageComposeErrorKind::LimitExceeded {
                resource: "resident chunk bytes",
                limit: 4,
                attempted: 5,
            }
        ));
    }
}
