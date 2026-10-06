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
use crate::{Cancellation, Context, Error, Limits, Result};
use std::io::Write;

/// Output and semantic progress. A failed operation's output is not a
/// completed page, even when `output_bytes_written` equals `packed_bytes`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PageComposeProgress {
    pub generic_bytes_accepted: u64,
    pub output_bytes_written: u64,
    pub rows_written: u32,
    pub max_request_bytes: usize,
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

/// A page-composition error at `offset`, a packed page or output byte
/// rather than an input offset.
fn at(offset: u64, error: Error) -> Error {
    error.or_at(offset, Context::Jbig2 { segment: None })
}

fn cap(resource: &'static str, maximum: u64, attempted: u64) -> Result<()> {
    if attempted > maximum {
        Err(at(0, Error::limit(resource, maximum, attempted)))
    } else {
        Ok(())
    }
}

const INCOMPLETE: &str = "page rows are incomplete";
const NOT_ARMED: &str = "page output is not armed or already flushed";

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
/// failure, discard the final output and call `take_failure` for the located
/// sink error.
pub struct PageOrSink<'a, W: Write, C: Cancellation> {
    profile: PageProfile,
    text: TextComposeReport,
    bitmap: &'a [u8],
    output: &'a mut W,
    cancellation: &'a C,
    chunk: Vec<u8>,
    progress: PageComposeProgress,
    failure: Option<Error>,
    armed: bool,
}

impl<'a, W: Write, C: Cancellation> PageOrSink<'a, W, C> {
    /// `bitmap` is the packed text region that `text` reports.
    pub fn new(
        profile: PageProfile,
        text: TextComposeReport,
        bitmap: &'a [u8],
        output: &'a mut W,
        limits: &Limits,
        cancellation: &'a C,
    ) -> Result<Self> {
        if cancellation.is_cancelled() {
            return Err(at(0, Error::cancelled()));
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
                Error::invalid("text report differs from page profile"),
            ));
        }
        if text.progress.stage != TextComposeStage::Complete {
            return Err(at(0, Error::invalid(INCOMPLETE)));
        }
        // The page-information preflight bounded the page's pixels and
        // packed bytes; the output limit also bounds the packed page.
        cap("page output bytes", limits.max_output_bytes, packed)?;
        if bitmap.len() as u64 != packed {
            return Err(at(0, Error::invalid("text bitmap size differs")));
        }
        // A row is nonempty and the entry point's `Limits::validate` capped
        // the I/O chunk by `max_allocation_bytes`.
        let chunk_size = page.row_stride.min(limits.io_chunk_bytes.max(1));
        let mut chunk = Vec::new();
        chunk.try_reserve_exact(chunk_size).map_err(|_| {
            at(
                0,
                limits.allocation_refused("page OR chunk bytes", chunk_size as u64),
            )
        })?;
        chunk.resize(chunk_size, 0);
        Ok(Self {
            profile,
            text,
            bitmap,
            output,
            cancellation,
            chunk,
            progress: PageComposeProgress::default(),
            failure: None,
            armed: false,
        })
    }

    pub fn progress(&self) -> PageComposeProgress {
        self.progress
    }

    /// Retrieve the first located failure raised through `Write`.
    pub fn take_failure(&mut self) -> Option<Error> {
        self.failure.take()
    }

    fn fail(&mut self, offset: u64, error: Error) -> Error {
        if self.failure.is_none() {
            self.failure = Some(at(offset, error));
        }
        Error::invalid("JBIG2 page OR failed; inspect PageOrSink::take_failure")
    }

    fn check_cancelled(&mut self, offset: u64) -> Result<()> {
        if self.cancellation.is_cancelled() {
            Err(self.fail(offset, Error::cancelled()))
        } else {
            Ok(())
        }
    }

    fn note_request(&mut self, count: usize) {
        self.progress.max_request_bytes = self.progress.max_request_bytes.max(count);
    }

    fn validate_generic_info(&self, info: GenericRegionInfo) -> Result<()> {
        let page = self.profile.page();
        if info.width != page.width
            || info.height != page.height
            || info.x != 0
            || info.y != 0
            || info.row_stride != page.row_stride
            || info.combination_operator != 0
        {
            return Err(at(
                0,
                Error::invalid("generic region differs from preflighted page"),
            ));
        }
        Ok(())
    }

    /// Verify a complete generic-region report, then flush the final output.
    /// The successful report proves neither external corpus parity nor PDF
    /// generation; it proves this bounded bytewise composition only.
    pub fn finish(mut self, generic: &GenericReport) -> Result<PageComposeReport> {
        if let Some(failure) = self.failure.take() {
            return Err(failure);
        }
        self.validate_generic_info(generic.progress.info)?;
        if generic.data != self.profile.generic_header().data
            || generic.mq_span != self.profile.generic_header().mq_span
        {
            return Err(at(
                0,
                Error::invalid("generic report span differs from preflighted segment"),
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
            return Err(at(
                self.progress.output_bytes_written,
                Error::invalid(INCOMPLETE),
            ));
        }
        if self.cancellation.is_cancelled() {
            return Err(at(self.progress.output_bytes_written, Error::cancelled()));
        }
        self.output
            .flush()
            .map_err(|error| at(self.progress.output_bytes_written, error.into()))?;
        // A flush can take long enough for the caller to give up.
        if self.cancellation.is_cancelled() {
            return Err(at(self.progress.output_bytes_written, Error::cancelled()));
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

    fn accept_chunk(&mut self, bytes: &[u8]) -> Result<usize> {
        let page = self.profile.page();
        let offset = self.progress.generic_bytes_accepted;
        self.check_cancelled(offset)?;
        if offset >= page.packed_bytes {
            return Err(self.fail(offset, Error::invalid("extra generic bytes")));
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
                return Err(self.fail(offset + index as u64, Error::invalid("nonzero row padding")));
            }
            self.chunk[index] = generic | value;
        }
        self.progress.generic_bytes_accepted += count as u64;
        self.check_cancelled(offset)?;
        if let Err(error) = self.output.write_all(&self.chunk[..count]) {
            return Err(self.fail(offset, error.into()));
        }
        self.progress.output_bytes_written += count as u64;
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
                    Error::invalid(NOT_ARMED),
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
                    Error::invalid(NOT_ARMED),
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
                    Error::invalid(INCOMPLETE),
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
            return Err(self.fail(0, Error::invalid("page output is already armed")));
        }
        let expected = self.profile.generic_header();
        if header.data != expected.data || header.mq_span != expected.mq_span {
            return Err(self.fail(
                0,
                Error::invalid("generic checked header span differs from page preflight"),
            ));
        }
        if header.info != expected.info || header.pixels != expected.pixels {
            return Err(self.fail(
                0,
                Error::invalid("generic checked header geometry differs from page preflight"),
            ));
        }
        if header.segment != expected.segment
            || header.page_association != expected.page_association
            || header.reference_count != expected.reference_count
        {
            return Err(self.fail(
                0,
                Error::invalid(
                    "generic checked header segment metadata differs from page preflight",
                ),
            ));
        }
        self.armed = true;
        Ok(())
    }
}
