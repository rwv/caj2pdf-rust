// SPDX-License-Identifier: MIT

//! Shared bounded dictionary-to-text stage for private JBIG2 diagnostics.
//! No corpus pixels or probability table states are embedded here.

use crate::support::{TempStore, hex, peak_rss_kib, ready};
use crate::text_case::*;
use caj2pdf_core::{
    Error, Limits, MAX_IO_CHUNK, NeverCancel, RangedSource, SequentialSink,
    jbig2::{
        dictionary::{DictionaryBudget, DirectDictionaryDecoder},
        iaid::IaidContextBanks,
        mq::{MqBudget, MqTable},
        refinement::RefinementBudget,
        refinement_dictionary::{RefinementDictionaryBudget, RefinementDictionaryDecoder},
        text::{
            TextHeaderAnomaly, TextHeaderPolicy, TextRegionBudget,
            read_text_region_header_with_policy,
        },
        text_composer::{
            BitmapStore, RandomAccessScratch, TextComposeBudget, TextComposeError,
            TextComposeErrorKind, TextComposeReport, TextComposer,
        },
        text_instances::{TextInstanceBudget, TextInstanceDecoder, TextInstanceError},
    },
    native::{SeekableSource, WriteSink},
};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::fs;
use std::{
    error::Error as StdError,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
};

pub(crate) const COMPATIBILITY_ARG: &str = "hn-c8-unused-refinement-template";
pub(crate) const COMPATIBILITY_MARKER: &str = "HN_C8_UNUSED_REFINEMENT_TEMPLATE";
pub(crate) const MAX_SOURCE_BYTES: u64 = 512 * 1024 * 1024;

pub(crate) struct CaseOutcome {
    pub(crate) status: &'static str,
    pub(crate) completed: u32,
    pub(crate) rows: u32,
    pub(crate) output_bytes: u64,
    pub(crate) black_pixels: u64,
    pub(crate) pixel_sha: String,
    pub(crate) scratch_bytes: u64,
    pub(crate) max_request: usize,
    pub(crate) peak_resident: u64,
    pub(crate) refusal: String,
    pub(crate) offset: u64,
    pub(crate) stage: String,
    pub(crate) anomaly: &'static str,
}

fn header_outcome(status: &'static str, refusal: &'static str, offset: u64) -> CaseOutcome {
    CaseOutcome {
        status,
        completed: 0,
        rows: 0,
        output_bytes: 0,
        black_pixels: 0,
        pixel_sha: hex(&Sha256::digest([])),
        scratch_bytes: 0,
        max_request: 0,
        peak_resident: 0,
        refusal: refusal.to_owned(),
        offset,
        stage: "Header".to_owned(),
        anomaly: "-",
    }
}

pub(crate) struct FileScratch(pub(crate) File);

/// Counts actual ranged requests across the text and generic phases.
pub(crate) struct MeteredSource<S> {
    pub(crate) inner: S,
    pub(crate) calls: u64,
    pub(crate) bytes: u64,
    pub(crate) max_request: usize,
}

impl<S: RangedSource> MeteredSource<S> {
    pub(crate) fn new(inner: S) -> Self {
        Self {
            inner,
            calls: 0,
            bytes: 0,
            max_request: 0,
        }
    }
}

impl<S: RangedSource> RangedSource for MeteredSource<S> {
    fn size(&self) -> u64 {
        self.inner.size()
    }

    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        let attempted =
            self.bytes
                .checked_add(destination.len() as u64)
                .ok_or(Error::InvalidInput {
                    reason: "source byte counter exhausted",
                })?;
        if attempted > MAX_SOURCE_BYTES {
            return Err(Error::LimitExceeded {
                resource: "diagnostic source bytes",
                limit: MAX_SOURCE_BYTES,
                attempted,
            });
        }
        self.calls = self.calls.checked_add(1).ok_or(Error::InvalidInput {
            reason: "source call counter exhausted",
        })?;
        self.max_request = self.max_request.max(destination.len());
        let got = self.inner.read_at(offset, destination).await?;
        if got > destination.len() {
            return Err(Error::InvalidInput {
                reason: "source overreported read",
            });
        }
        self.bytes = self
            .bytes
            .checked_add(got as u64)
            .ok_or(Error::InvalidInput {
                reason: "source byte counter exhausted",
            })?;
        Ok(got)
    }
}

pub(crate) struct CompletedText {
    pub(crate) scratch: FileScratch,
    /// Keeps the private scratch file alive on platforms without unlink-on-open.
    pub(crate) _scratch_store: TempStore,
    pub(crate) report: TextComposeReport,
    pub(crate) scratch_bytes: u64,
}

pub(crate) enum TextPhaseOutcome {
    Complete(CompletedText),
    Refused(CaseOutcome),
}

impl RandomAccessScratch for FileScratch {
    fn size(&self) -> caj2pdf_core::Result<u64> {
        Ok(self.0.metadata()?.len())
    }

    async fn set_len(&mut self, bytes: u64) -> caj2pdf_core::Result<()> {
        self.0.set_len(bytes)?;
        Ok(())
    }

    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        if destination.len() > MAX_IO_CHUNK {
            return Err(Error::LimitExceeded {
                resource: "scratch read request bytes",
                limit: MAX_IO_CHUNK as u64,
                attempted: destination.len() as u64,
            });
        }
        self.0.seek(SeekFrom::Start(offset))?;
        Ok(self.0.read(destination)?)
    }

    async fn write_at(&mut self, offset: u64, bytes: &[u8]) -> caj2pdf_core::Result<usize> {
        if bytes.len() > MAX_IO_CHUNK {
            return Err(Error::LimitExceeded {
                resource: "scratch write request bytes",
                limit: MAX_IO_CHUNK as u64,
                attempted: bytes.len() as u64,
            });
        }
        self.0.seek(SeekFrom::Start(offset))?;
        Ok(self.0.write(bytes)?)
    }

    async fn flush(&mut self) -> caj2pdf_core::Result<()> {
        self.0.flush()?;
        self.0.sync_data()?;
        Ok(())
    }
}

fn host_error_kind(error: &Error) -> &'static str {
    match error {
        Error::TruncatedInput { .. } => "truncated_input",
        Error::InvalidInput { .. } => "invalid_input",
        Error::LimitExceeded { .. } => "limit_exceeded",
        Error::Io(_) => "io",
        Error::Cancelled => "cancelled",
        _ => "other",
    }
}

fn bitmap_store_tag(store: BitmapStore) -> &'static str {
    match store {
        BitmapStore::Imported => "imported",
        BitmapStore::New => "new",
        BitmapStore::Refined => "refined",
    }
}

pub(crate) fn compose_refusal_kind(error: &TextComposeErrorKind) -> String {
    match error {
        TextComposeErrorKind::InvalidSpan(_) => "compose_invalid_span".to_owned(),
        TextComposeErrorKind::Malformed(_) => "compose_malformed".to_owned(),
        TextComposeErrorKind::LimitExceeded { .. } => "compose_limit_exceeded".to_owned(),
        TextComposeErrorKind::AllocationFailed => "compose_allocation_failed".to_owned(),
        TextComposeErrorKind::Cancelled => "compose_cancelled".to_owned(),
        TextComposeErrorKind::Instance(error) => {
            format!("instance_{}", refusal_kind(&error.kind))
        }
        TextComposeErrorKind::Source { store, error } => {
            format!(
                "{}_source_{}",
                bitmap_store_tag(*store),
                host_error_kind(error)
            )
        }
        TextComposeErrorKind::StoreMutation { store, .. } => {
            format!("{}_store_mutation", bitmap_store_tag(*store))
        }
        TextComposeErrorKind::Scratch(error) => {
            format!("scratch_{}", host_error_kind(error))
        }
        TextComposeErrorKind::Output(error) => format!("output_{}", host_error_kind(error)),
        TextComposeErrorKind::Poisoned => "compose_poisoned".to_owned(),
    }
}

pub(crate) fn peak_resident_bytes() -> u64 {
    peak_rss_kib().unwrap_or(0).saturating_mul(1024)
}

fn instance_refusal(error: TextInstanceError) -> CaseOutcome {
    let progress = *error.progress;
    CaseOutcome {
        status: "REFUSED",
        completed: progress.completed_instances,
        rows: 0,
        output_bytes: 0,
        black_pixels: 0,
        pixel_sha: hex(&Sha256::digest([])),
        scratch_bytes: 0,
        max_request: 0,
        peak_resident: 0,
        refusal: format!("instance_{}", refusal_kind(&error.kind)),
        offset: error.offset,
        stage: format!("{:?}", progress.decision),
        anomaly: "-",
    }
}

fn compose_refusal(error: TextComposeError, scratch_bytes: u64) -> CaseOutcome {
    let progress = *error.progress;
    CaseOutcome {
        status: "REFUSED",
        completed: progress.completed_instances,
        rows: progress.output_rows,
        output_bytes: progress.output_bytes_written,
        black_pixels: 0,
        pixel_sha: hex(&Sha256::digest([])),
        scratch_bytes,
        max_request: progress.max_request_bytes,
        peak_resident: progress.peak_resident_bytes,
        refusal: compose_refusal_kind(&error.kind),
        offset: error.offset,
        stage: format!("{:?}", progress.stage),
        anomaly: "-",
    }
}

pub(crate) fn run_text_phase<W: SequentialSink>(
    case: &Case,
    table: &MqTable,
    policy: TextHeaderPolicy,
    source: &mut MeteredSource<SeekableSource<File>>,
    output: &mut W,
) -> Result<TextPhaseOutcome, Box<dyn StdError>> {
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let dictionary_budget = DictionaryBudget::default();
    let first = checked_header(&mut *source, &case.source, &case.first, 1, &limits)?;
    let second = checked_header(&mut *source, &case.source, &case.second, 2, &limits)?;
    let third = checked_text_segment(&mut *source, case, &limits)?;
    let text_header = ready(read_text_region_header_with_policy(
        &mut *source,
        &third,
        &second,
        &limits,
        TextRegionBudget::default(),
        &NeverCancel,
        policy,
    ));
    let text_header = match text_header {
        Ok(header)
            if header.instances == case.text.instances
                && header.anomaly
                    == if case.text.anomaly
                        && policy == TextHeaderPolicy::HnC8UnusedRefinementTemplate
                    {
                        Some(TextHeaderAnomaly::UnusedRefinementTemplate)
                    } else {
                        None
                    }
                && (!case.text.anomaly || policy != TextHeaderPolicy::Strict) =>
        {
            header
        }
        Err(error)
            if case.text.anomaly
                && matches!(
                    &error.kind,
                    caj2pdf_core::jbig2::text::TextRegionErrorKind::MalformedFlags {
                        field: "SBRTEMPLATE without SBREFINE",
                        raw: 0xa40c
                    }
                ) =>
        {
            if error.offset != third.data.offset + 17 {
                return Err("strict text-header refusal has the wrong flags offset".into());
            }
            return Ok(TextPhaseOutcome::Refused(header_outcome(
                "HEADER_REFUSED",
                "malformed_text_header",
                error.offset,
            )));
        }
        Ok(_) => {
            return Ok(TextPhaseOutcome::Refused(header_outcome(
                "REFUSED",
                "header_classification_or_count",
                third.data.offset + 19,
            )));
        }
        Err(error) => {
            return Ok(TextPhaseOutcome::Refused(header_outcome(
                "REFUSED",
                header_refusal_kind(&error.kind),
                error.offset,
            )));
        }
    };
    let (first_store, first_file) = TempStore::create("caj2pdf-dictionary-first")?;
    let mut first_sink = WriteSink::new(first_file);
    let mut first_banks = caj2pdf_core::jbig2::integer::IntegerContextBanks::with_extra_contexts(
        1024, &limits, &mq_budget,
    )?;
    let mut first_decoder = ready(DirectDictionaryDecoder::new(
        &mut *source,
        &first,
        table,
        &mut first_banks,
        &mut first_sink,
        &limits,
        &NeverCancel,
        mq_budget,
        dictionary_budget,
    ))?;
    let first_report = ready(first_decoder.decode())?;
    drop(first_decoder);
    let first_bytes = first_sink.into_inner().metadata()?.len();
    if first_report.header.new_symbols != case.first.new_symbols
        || first_report.header.exported_symbols != case.first.exported_symbols
    {
        return Err("first dictionary symbol counts differ from pin".into());
    }
    let mut imported = SeekableSource::new(File::open(first_store.path())?)?;
    let (second_store, second_file) = TempStore::create("caj2pdf-dictionary-second")?;
    let mut second_sink = WriteSink::new(second_file);
    let mut new_source = GrowingFileSource(File::open(second_store.path())?);
    let total = u64::from(case.first.exported_symbols) + u64::from(case.second.new_symbols);
    let code_len = if total <= 1 {
        0
    } else {
        64 - (total - 1).leading_zeros()
    };
    let mut banks = IaidContextBanks::with_bitmap_contexts(code_len, 1024, &limits, &mq_budget)?;
    let mut decoder = ready(RefinementDictionaryDecoder::new(
        &mut *source,
        &second,
        &first,
        &first_report,
        &mut imported,
        0,
        &mut new_source,
        &mut second_sink,
        0,
        table,
        &mut banks,
        &limits,
        &NeverCancel,
        mq_budget,
        dictionary_budget,
        RefinementBudget::default(),
        RefinementDictionaryBudget::default(),
    ))?;
    let report = ready(decoder.decode())?;
    drop(decoder);
    let second_bytes = second_sink.into_inner().metadata()?.len();
    drop(new_source);
    if report.header.new_symbols != case.second.new_symbols
        || report.header.exported_symbols != case.second.exported_symbols
        || report.catalog.new_symbols.len() != case.second.new_symbols as usize
        || report.catalog.exported_symbols.len() != case.second.exported_symbols as usize
        || report.progress.completed_symbols != case.second.new_symbols
    {
        return Err("second dictionary symbol counts differ from pin".into());
    }
    let total = report.catalog.exported_symbols.len() as u64;
    let code_len = if total <= 1 {
        0
    } else {
        64 - (total - 1).leading_zeros()
    };
    let mut text_banks =
        IaidContextBanks::with_bitmap_contexts(code_len, 1024, &limits, &mq_budget)?;
    let mut new_store = SeekableSource::new(File::open(second_store.path())?)?;
    let (temporary_store, temporary_file) = TempStore::create("caj2pdf-text-refined")?;
    let mut temporary_sink = WriteSink::new(temporary_file);
    let mut text_decoder = match ready(TextInstanceDecoder::new_with_header_policy(
        &mut *source,
        &third,
        text_header,
        &second,
        &report,
        &mut imported,
        0,
        &mut new_store,
        0,
        &mut temporary_sink,
        0,
        table,
        &mut text_banks,
        &limits,
        &NeverCancel,
        mq_budget,
        TextRegionBudget::default(),
        RefinementBudget::default(),
        TextInstanceBudget::default(),
        policy,
    )) {
        Ok(decoder) => decoder,
        Err(error) => return Ok(TextPhaseOutcome::Refused(instance_refusal(error))),
    };
    let mut imported_for_compose = SeekableSource::new(File::open(first_store.path())?)?;
    let mut new_for_compose = SeekableSource::new(File::open(second_store.path())?)?;
    let mut refined_for_compose = GrowingFileSource(File::open(temporary_store.path())?);
    #[cfg(unix)]
    for path in [
        first_store.path(),
        second_store.path(),
        temporary_store.path(),
    ] {
        fs::remove_file(path)?;
    }
    let (scratch_store, scratch_file) = TempStore::create("caj2pdf-text-region-scratch")?;
    drop(scratch_file);
    let mut scratch = FileScratch(
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(scratch_store.path())?,
    );
    #[cfg(unix)]
    fs::remove_file(scratch_store.path())?;
    let composed = {
        let mut composer = match TextComposer::new(
            third.number,
            text_header,
            &report.catalog.exported_symbols,
            &mut text_decoder,
            &mut imported_for_compose,
            0,
            &mut new_for_compose,
            0,
            &mut refined_for_compose,
            0,
            &mut scratch,
            output,
            &limits,
            &NeverCancel,
            TextComposeBudget::default(),
        ) {
            Ok(composer) => composer,
            Err(error) => {
                return Ok(TextPhaseOutcome::Refused(compose_refusal(
                    error,
                    scratch.size()?,
                )));
            }
        };
        ready(composer.compose())
    };
    let region = match composed {
        Ok(region) => region,
        Err(error) => {
            return Ok(TextPhaseOutcome::Refused(compose_refusal(
                error,
                scratch.size()?,
            )));
        }
    };
    if region.progress.output_bytes_written != region.packed_bytes {
        return Err("composed pixel byte count differs from region geometry".into());
    }
    if region.text_flags_raw != text_header.flags.raw
        || region.header_anomaly != text_header.anomaly
    {
        return Err("composed report differs from validated text-header policy".into());
    }
    drop(text_decoder);
    let refined_bytes = temporary_sink.into_inner().metadata()?.len();
    let scratch_bytes = first_bytes
        .checked_add(second_bytes)
        .and_then(|n| n.checked_add(refined_bytes))
        .and_then(|n| n.checked_add(region.packed_bytes))
        .ok_or("diagnostic scratch byte count overflowed")?;
    Ok(TextPhaseOutcome::Complete(CompletedText {
        scratch,
        _scratch_store: scratch_store,
        report: region,
        scratch_bytes,
    }))
}
