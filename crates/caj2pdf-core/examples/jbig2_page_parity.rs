// SPDX-License-Identifier: MIT

//! Private, SHA-pinned full-page JBIG2 pixel diagnostic for issue #95.
//! It emits normalized P4 rows only to a bounded hash sink, never a PDF.

#[path = "support/page_case.rs"]
mod page_case;
#[path = "support/mod.rs"]
#[allow(dead_code)]
mod support;
#[path = "support/text_case.rs"]
#[allow(dead_code)]
mod text_case;
#[path = "support/text_pipeline.rs"]
#[allow(dead_code)]
mod text_pipeline;

use caj2pdf_core::{
    Error, Limits, MAX_IO_CHUNK, NeverCancel, SequentialSink,
    jbig2::{
        DirectoryLimits, HeaderLimits, SegmentDirectory, SegmentHeader, SegmentSpan,
        generic::{
            GenericBudget, GenericError, GenericErrorKind, GenericRegionDecoder,
            read_generic_region_header,
        },
        mq::{MqBudget, MqContexts, MqTable},
        page_compose::{PageComposeBudget, PageComposeError, PageComposeErrorKind, PageOrSink},
        page_info::{PageInfoBudget, PageInfoErrorKind, read_page_info},
        page_profile::{PageProfileErrorKind, validate_observed_page_profile},
        read_embedded_directory,
        text::{
            TextHeaderAnomaly, TextHeaderPolicy, TextRegionBudget, TextRegionErrorKind,
            read_text_region_header_with_policy,
        },
    },
    native::SeekableSource,
};
use page_case::{EncodedPin, PageCase, parse_page_plan};
use sha2::{Digest, Sha256};
use std::{env, error::Error as StdError, fs::File, path::Path};
use support::{digest_file, hex, peak_rss_kib, ready, table};
use text_case::{EXPECTED_CASES, mq_refusal_kind};
use text_pipeline::{
    COMPATIBILITY_ARG, COMPATIBILITY_MARKER, MAX_SOURCE_BYTES, MeteredSource, TextPhaseOutcome,
    run_text_phase,
};

const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
const MAX_RESIDENT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_SCRATCH_BYTES: u64 = 256 * 1024 * 1024;
const MAX_RSS_KIB: u64 = 1024 * 1024;
const HASH_WRITE_CHUNK: usize = 64 * 1024;
const GENERIC_CONTEXT_ALLOWANCE_BYTES: u64 = 16 * 1024;

struct CaseOutcome {
    status: &'static str,
    width: u32,
    height: u32,
    rows: u32,
    output_bytes: u64,
    pixel_sha: String,
    black_pixels: u64,
    source_bytes: u64,
    max_request: usize,
    peak_resident_bytes: u64,
    scratch_bytes: u64,
    rss_kib: u64,
    refusal_kind: String,
    refusal_offset: u64,
    stage: String,
    anomaly_marker: &'static str,
}

impl CaseOutcome {
    fn source_failure(offset: u64) -> Self {
        Self {
            status: "REFUSED",
            width: 0,
            height: 0,
            rows: 0,
            output_bytes: 0,
            pixel_sha: EMPTY_SHA256.to_owned(),
            black_pixels: 0,
            source_bytes: 0,
            max_request: 0,
            peak_resident_bytes: 0,
            scratch_bytes: 0,
            rss_kib: peak_rss_kib().unwrap_or(0),
            refusal_kind: "source_io".to_owned(),
            refusal_offset: offset,
            stage: "Source".to_owned(),
            anomaly_marker: "-",
        }
    }

    fn refused(
        kind: impl Into<String>,
        offset: u64,
        stage: &'static str,
        width: u32,
        height: u32,
        source: &MeteredSource<SeekableSource<File>>,
    ) -> Self {
        Self {
            status: "REFUSED",
            width,
            height,
            rows: 0,
            output_bytes: 0,
            pixel_sha: EMPTY_SHA256.to_owned(),
            black_pixels: 0,
            source_bytes: source.bytes,
            max_request: source.max_request.max(HASH_WRITE_CHUNK),
            peak_resident_bytes: 0,
            scratch_bytes: 0,
            rss_kib: peak_rss_kib().unwrap_or(0),
            refusal_kind: kind.into(),
            refusal_offset: offset,
            stage: stage.to_owned(),
            anomaly_marker: "-",
        }
    }

    fn line(&self, case: &PageCase) -> String {
        [
            "CASE".to_owned(),
            case.text.id.clone(),
            case.text.page.to_string(),
            case.text.image.to_string(),
            self.status.to_owned(),
            self.width.to_string(),
            self.height.to_string(),
            self.rows.to_string(),
            self.output_bytes.to_string(),
            self.pixel_sha.clone(),
            self.black_pixels.to_string(),
            self.source_bytes.to_string(),
            self.max_request.to_string(),
            self.peak_resident_bytes.to_string(),
            self.scratch_bytes.to_string(),
            self.rss_kib.to_string(),
            self.refusal_kind.clone(),
            self.refusal_offset.to_string(),
            self.stage.clone(),
            self.anomaly_marker.to_owned(),
        ]
        .join("\t")
    }
}

fn framed_span(header: &SegmentHeader) -> Option<SegmentSpan> {
    Some(SegmentSpan {
        offset: header.data.offset.checked_sub(header.header_length)?,
        length: header.header_length.checked_add(header.data.length)?,
    })
}

fn pins_match(case: &PageCase, directory: &SegmentDirectory) -> bool {
    let pins = [
        case.page.span,
        SegmentSpan {
            offset: case.text.first.offset,
            length: case.text.first.length,
        },
        SegmentSpan {
            offset: case.text.second.offset,
            length: case.text.second.length,
        },
        SegmentSpan {
            offset: case.text.text.offset,
            length: case.text.text.length,
        },
        case.generic.span,
    ];
    directory.segments.len() == pins.len()
        && directory
            .segments
            .iter()
            .zip(pins)
            .all(|(segment, pin)| framed_span(segment) == Some(pin))
}

fn digest_matches(path: &Path, pin: &EncodedPin) -> bool {
    digest_file(path, Some((pin.span.offset, pin.span.length))).is_ok_and(|sha| sha == pin.sha256)
}

struct DiscardSink {
    bytes: u64,
    max_request: usize,
    flushed: bool,
}

impl DiscardSink {
    fn new() -> Self {
        Self {
            bytes: 0,
            max_request: 0,
            flushed: false,
        }
    }
}

impl SequentialSink for DiscardSink {
    async fn write(&mut self, bytes: &[u8]) -> caj2pdf_core::Result<usize> {
        if bytes.len() > MAX_IO_CHUNK {
            return Err(Error::LimitExceeded {
                resource: "discard request bytes",
                limit: MAX_IO_CHUNK as u64,
                attempted: bytes.len() as u64,
            });
        }
        self.max_request = self.max_request.max(bytes.len());
        self.bytes = self
            .bytes
            .checked_add(bytes.len() as u64)
            .ok_or(Error::InvalidInput {
                reason: "discard byte count overflowed",
            })?;
        Ok(bytes.len())
    }
    async fn flush(&mut self) -> caj2pdf_core::Result<()> {
        self.flushed = true;
        Ok(())
    }
}

/// Hash only normalized top-down, MSB-first packed page rows (1 = black).
struct PackedHashSink {
    hash: Sha256,
    stride: u64,
    expected: u64,
    low_mask: u8,
    bytes: u64,
    black: u64,
    max_request: usize,
    flushed: bool,
}

impl PackedHashSink {
    fn new(width: u32, height: u32) -> Self {
        let stride = u64::from(width.div_ceil(8));
        let low_unused = (8 - width % 8) % 8;
        Self {
            hash: Sha256::new(),
            stride,
            expected: stride * u64::from(height),
            low_mask: ((1u16 << low_unused) - 1) as u8,
            bytes: 0,
            black: 0,
            max_request: 0,
            flushed: false,
        }
    }

    fn finish(self) -> Result<(String, u64, u64), &'static str> {
        if !self.flushed || self.bytes != self.expected {
            return Err("final hash sink did not receive one complete page");
        }
        Ok((hex(&self.hash.finalize()), self.black, self.bytes))
    }
}

impl SequentialSink for PackedHashSink {
    async fn write(&mut self, bytes: &[u8]) -> caj2pdf_core::Result<usize> {
        if bytes.len() > MAX_IO_CHUNK {
            return Err(Error::LimitExceeded {
                resource: "hash request bytes",
                limit: MAX_IO_CHUNK as u64,
                attempted: bytes.len() as u64,
            });
        }
        self.max_request = self.max_request.max(bytes.len());
        let accepted = bytes.len().min(HASH_WRITE_CHUNK);
        let end = self
            .bytes
            .checked_add(accepted as u64)
            .ok_or(Error::InvalidInput {
                reason: "hash byte count overflowed",
            })?;
        if end > self.expected {
            return Err(Error::InvalidInput {
                reason: "hash sink received bytes past page",
            });
        }
        for (index, byte) in bytes[..accepted].iter().enumerate() {
            if (self.bytes + index as u64 + 1).is_multiple_of(self.stride)
                && byte & self.low_mask != 0
            {
                return Err(Error::InvalidInput {
                    reason: "nonzero low row padding",
                });
            }
            self.black += u64::from(byte.count_ones());
        }
        self.hash.update(&bytes[..accepted]);
        self.bytes = end;
        Ok(accepted)
    }
    async fn flush(&mut self) -> caj2pdf_core::Result<()> {
        if self.bytes != self.expected {
            return Err(Error::InvalidInput {
                reason: "hash sink flushed a partial page",
            });
        }
        self.flushed = true;
        Ok(())
    }
}

fn page_info_tag(kind: &PageInfoErrorKind) -> &'static str {
    match kind {
        PageInfoErrorKind::InvalidSpan(_) => "page_invalid_span",
        PageInfoErrorKind::Truncated(_) => "page_truncated",
        PageInfoErrorKind::Malformed(_) => "page_malformed",
        PageInfoErrorKind::Unsupported { .. } => "page_unsupported",
        PageInfoErrorKind::LimitExceeded { .. } => "page_limit_exceeded",
        PageInfoErrorKind::Cancelled => "page_cancelled",
        PageInfoErrorKind::Source(_) => "page_source",
    }
}

fn generic_tag(error: &GenericError) -> &'static str {
    match &error.kind {
        GenericErrorKind::InvalidSpan(_) => "generic_invalid_span",
        GenericErrorKind::Truncated(_) => "generic_truncated",
        GenericErrorKind::Malformed(_) => "generic_malformed",
        GenericErrorKind::Unsupported { .. } | GenericErrorKind::UnsupportedAt { .. } => {
            "generic_unsupported"
        }
        GenericErrorKind::LimitExceeded { .. } => "generic_limit_exceeded",
        GenericErrorKind::AllocationFailed => "generic_allocation_failed",
        GenericErrorKind::Cancelled => "generic_cancelled",
        GenericErrorKind::Source(_) => "generic_source",
        GenericErrorKind::Sink(_) => "generic_sink",
        GenericErrorKind::Mq(_) => "generic_mq",
        GenericErrorKind::Incomplete => "generic_incomplete",
        GenericErrorKind::Poisoned => "generic_poisoned",
    }
}

fn page_compose_tag(error: &PageComposeError) -> &'static str {
    match &error.kind {
        PageComposeErrorKind::Malformed(_) => "page_compose_malformed",
        PageComposeErrorKind::InvalidSpan(_) => "page_compose_invalid_span",
        PageComposeErrorKind::LimitExceeded { .. } => "page_compose_limit_exceeded",
        PageComposeErrorKind::AllocationFailed => "page_compose_allocation_failed",
        PageComposeErrorKind::Cancelled => "page_compose_cancelled",
        PageComposeErrorKind::Limits(_) => "page_compose_limits",
        PageComposeErrorKind::Scratch(_) => "page_compose_scratch",
        PageComposeErrorKind::Output(_) => "page_compose_output",
        PageComposeErrorKind::Incomplete => "page_compose_incomplete",
        PageComposeErrorKind::Poisoned => "page_compose_poisoned",
    }
}

fn text_pipeline_failure(
    error: &(dyn StdError + 'static),
    fallback: u64,
) -> (String, u64, &'static str) {
    use caj2pdf_core::jbig2::{
        HeaderError, HeaderErrorKind,
        dictionary::{DictionaryError, DictionaryErrorKind},
        refinement_dictionary::{RefinementDictionaryError, RefinementDictionaryErrorKind},
    };
    if let Some(error) = error.downcast_ref::<DictionaryError>() {
        let tag = match &error.kind {
            DictionaryErrorKind::InvalidSpan(_) => "dictionary_invalid_span".to_owned(),
            DictionaryErrorKind::Truncated(_) => "dictionary_truncated".to_owned(),
            DictionaryErrorKind::Malformed(_) => "dictionary_malformed".to_owned(),
            DictionaryErrorKind::Unsupported { .. } | DictionaryErrorKind::UnsupportedAt { .. } => {
                "dictionary_unsupported".to_owned()
            }
            DictionaryErrorKind::LimitExceeded { .. } => "dictionary_limit_exceeded".to_owned(),
            DictionaryErrorKind::AllocationFailed => "dictionary_allocation_failed".to_owned(),
            DictionaryErrorKind::Cancelled => "dictionary_cancelled".to_owned(),
            DictionaryErrorKind::Source(_) => "dictionary_source".to_owned(),
            DictionaryErrorKind::Header(_) => "dictionary_header".to_owned(),
            DictionaryErrorKind::Sink(_) => "dictionary_sink".to_owned(),
            DictionaryErrorKind::Mq(inner) => {
                format!("dictionary_{}", mq_refusal_kind(&inner.kind))
            }
            DictionaryErrorKind::Poisoned => "dictionary_poisoned".to_owned(),
        };
        return (tag, error.offset, "Dictionary");
    }
    if let Some(error) = error.downcast_ref::<RefinementDictionaryError>() {
        let tag = match &error.kind {
            RefinementDictionaryErrorKind::InvalidSpan(_) => "refinement_dictionary_invalid_span",
            RefinementDictionaryErrorKind::Malformed(_) => "refinement_dictionary_malformed",
            RefinementDictionaryErrorKind::Unsupported { .. } => {
                "refinement_dictionary_unsupported"
            }
            RefinementDictionaryErrorKind::LimitExceeded { .. } => {
                "refinement_dictionary_limit_exceeded"
            }
            RefinementDictionaryErrorKind::AllocationFailed => {
                "refinement_dictionary_allocation_failed"
            }
            RefinementDictionaryErrorKind::Cancelled => "refinement_dictionary_cancelled",
            RefinementDictionaryErrorKind::Header(_) => "refinement_dictionary_header",
            RefinementDictionaryErrorKind::Mq(_) => "refinement_dictionary_mq",
            RefinementDictionaryErrorKind::Refinement(_) => "refinement_dictionary_refinement",
            RefinementDictionaryErrorKind::Sink(_) => "refinement_dictionary_sink",
            RefinementDictionaryErrorKind::Poisoned => "refinement_dictionary_poisoned",
        };
        return (tag.to_owned(), error.offset, "RefinementDictionary");
    }
    if let Some(error) = error.downcast_ref::<HeaderError>() {
        let tag = match &error.kind {
            HeaderErrorKind::InvalidSpan(_) => "segment_header_invalid_span",
            HeaderErrorKind::Truncated(_) => "segment_header_truncated",
            HeaderErrorKind::Malformed(_) => "segment_header_malformed",
            HeaderErrorKind::Unsupported { .. } => "segment_header_unsupported",
            HeaderErrorKind::LimitExceeded { .. } => "segment_header_limit_exceeded",
            HeaderErrorKind::AllocationFailed => "segment_header_allocation_failed",
            HeaderErrorKind::Cancelled => "segment_header_cancelled",
            HeaderErrorKind::Source(_) => "segment_header_source",
        };
        return (tag.to_owned(), error.offset, "SegmentHeader");
    }
    if error.downcast_ref::<std::io::Error>().is_some() {
        return ("text_source_io".to_owned(), fallback, "Text");
    }
    ("text_pipeline_invariant".to_owned(), fallback, "Text")
}

fn run_case(case: &PageCase, table: &MqTable, policy: TextHeaderPolicy) -> CaseOutcome {
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let file = match File::open(&case.text.source) {
        Ok(file) => file,
        Err(_) => return CaseOutcome::source_failure(case.record.span.offset),
    };
    let native = match SeekableSource::new(file) {
        Ok(source) => source,
        Err(_) => return CaseOutcome::source_failure(case.record.span.offset),
    };
    let mut source = MeteredSource::new(native);
    if !digest_matches(&case.text.source, &case.record) {
        return CaseOutcome::refused(
            "record_sha_changed",
            case.record.span.offset,
            "Source",
            0,
            0,
            &source,
        );
    }
    let embedded = SegmentSpan {
        offset: case.record.span.offset + 48,
        length: case.record.span.length - 48,
    };
    let directory = match ready(read_embedded_directory(
        &mut source,
        embedded,
        &limits,
        HeaderLimits::default(),
        DirectoryLimits::default(),
        &NeverCancel,
    )) {
        Ok(directory) => directory,
        Err(error) => {
            return CaseOutcome::refused(
                "directory_refused",
                error.offset,
                "Directory",
                0,
                0,
                &source,
            );
        }
    };
    if !pins_match(case, &directory) {
        return CaseOutcome::refused(
            "directory_span_changed",
            embedded.offset,
            "Directory",
            0,
            0,
            &source,
        );
    }
    if !digest_matches(&case.text.source, &case.page)
        || !digest_matches(&case.text.source, &case.generic)
    {
        return CaseOutcome::refused(
            "segment_sha_changed",
            embedded.offset,
            "Source",
            0,
            0,
            &source,
        );
    }
    let page = match ready(read_page_info(
        &mut source,
        &directory.segments[0],
        &limits,
        PageInfoBudget::default(),
        &NeverCancel,
    )) {
        Ok(page) => page,
        Err(error) => {
            return CaseOutcome::refused(
                page_info_tag(&error.kind),
                error.offset,
                "PageInfo",
                0,
                0,
                &source,
            );
        }
    };
    let generic = match ready(read_generic_region_header(
        &mut source,
        &directory.segments[4],
        &limits,
        &NeverCancel,
        mq_budget,
        GenericBudget::default(),
    )) {
        Ok(header) => header,
        Err(error) => {
            return CaseOutcome::refused(
                generic_tag(&error),
                error.offset,
                "GenericHeader",
                page.width,
                page.height,
                &source,
            );
        }
    };
    let text = match ready(read_text_region_header_with_policy(
        &mut source,
        &directory.segments[3],
        &directory.segments[2],
        &limits,
        TextRegionBudget::default(),
        &NeverCancel,
        policy,
    )) {
        Ok(header) => header,
        Err(error)
            if case.text.text.anomaly
                && policy == TextHeaderPolicy::Strict
                && matches!(
                    error.kind,
                    TextRegionErrorKind::MalformedFlags {
                        field: "SBRTEMPLATE without SBREFINE",
                        raw: 0xa40c
                    }
                )
                && error.offset == directory.segments[3].data.offset + 17 =>
        {
            let mut outcome = CaseOutcome::refused(
                "malformed_text_header",
                error.offset,
                "Header",
                page.width,
                page.height,
                &source,
            );
            outcome.status = "HEADER_REFUSED";
            return outcome;
        }
        Err(error) => {
            return CaseOutcome::refused(
                text_case::header_refusal_kind(&error.kind),
                error.offset,
                "TextHeader",
                page.width,
                page.height,
                &source,
            );
        }
    };
    if text.instances != case.text.text.instances
        || text.anomaly
            != if case.text.text.anomaly && policy != TextHeaderPolicy::Strict {
                Some(TextHeaderAnomaly::UnusedRefinementTemplate)
            } else {
                None
            }
    {
        return CaseOutcome::refused(
            "text_header_pin_changed",
            text.body.offset,
            "TextHeader",
            page.width,
            page.height,
            &source,
        );
    }
    let profile = match validate_observed_page_profile(&directory, page, &text, generic) {
        Ok(profile) => profile,
        Err(error) => {
            let tag = match error.kind {
                PageProfileErrorKind::Malformed(_) => "page_profile_malformed",
                PageProfileErrorKind::Unsupported { .. } => "page_profile_unsupported",
            };
            return CaseOutcome::refused(
                tag,
                embedded.offset,
                "Profile",
                page.width,
                page.height,
                &source,
            );
        }
    };
    let mut text_output = DiscardSink::new();
    let text_phase = run_text_phase(&case.text, table, policy, &mut source, &mut text_output);
    let mut completed = match text_phase {
        Ok(TextPhaseOutcome::Complete(complete)) => complete,
        Ok(TextPhaseOutcome::Refused(refusal)) => {
            return CaseOutcome {
                status: refusal.status,
                width: page.width,
                height: page.height,
                rows: 0,
                output_bytes: 0,
                pixel_sha: EMPTY_SHA256.to_owned(),
                black_pixels: 0,
                source_bytes: source.bytes,
                max_request: source
                    .max_request
                    .max(refusal.max_request)
                    .max(HASH_WRITE_CHUNK),
                peak_resident_bytes: refusal.peak_resident,
                scratch_bytes: refusal.scratch_bytes,
                rss_kib: peak_rss_kib().unwrap_or(0),
                refusal_kind: refusal.refusal,
                refusal_offset: refusal.offset,
                stage: refusal.stage,
                anomaly_marker: "-",
            };
        }
        Err(error) => {
            let (kind, offset, stage) =
                text_pipeline_failure(error.as_ref(), case.text.first.offset);
            return CaseOutcome {
                status: "REFUSED",
                width: page.width,
                height: page.height,
                rows: 0,
                output_bytes: 0,
                pixel_sha: EMPTY_SHA256.to_owned(),
                black_pixels: 0,
                source_bytes: source.bytes,
                max_request: source.max_request.max(HASH_WRITE_CHUNK),
                peak_resident_bytes: 0,
                scratch_bytes: 0,
                rss_kib: peak_rss_kib().unwrap_or(0),
                refusal_kind: kind,
                refusal_offset: offset,
                stage: stage.to_owned(),
                anomaly_marker: "-",
            };
        }
    };
    if text_output.bytes != page.packed_bytes || !text_output.flushed {
        return CaseOutcome::refused(
            "text_discard_incomplete",
            case.text.text.offset,
            "Text",
            page.width,
            page.height,
            &source,
        );
    }
    let mut final_output = PackedHashSink::new(page.width, page.height);
    let mut page_sink = match PageOrSink::new(
        profile,
        completed.report,
        &mut completed.scratch,
        &mut final_output,
        &limits,
        &NeverCancel,
        PageComposeBudget::default(),
    ) {
        Ok(sink) => sink,
        Err(error) => {
            return CaseOutcome::refused(
                page_compose_tag(&error),
                error.offset,
                "PageCompose",
                page.width,
                page.height,
                &source,
            );
        }
    };
    let mut contexts = match MqContexts::new(1024, &limits, &mq_budget) {
        Ok(contexts) => contexts,
        Err(_) => {
            return CaseOutcome::refused(
                "generic_contexts",
                case.generic.span.offset,
                "Generic",
                page.width,
                page.height,
                &source,
            );
        }
    };
    let generic_result = (|| {
        let mut decoder = ready(GenericRegionDecoder::new(
            &mut source,
            &directory.segments[4],
            table,
            &mut contexts,
            &mut page_sink,
            &limits,
            &NeverCancel,
            mq_budget,
            GenericBudget::default(),
        ))?;
        decoder.arm_page_output(profile.generic_header())?;
        while ready(decoder.decode_next_row())? {}
        ready(decoder.finish())
    })();
    let generic_report = match generic_result {
        Ok(report) => report,
        Err(error) => {
            let page_failure = page_sink.take_failure();
            let (kind, offset) = page_failure
                .as_ref()
                .map_or((generic_tag(&error), error.offset), |failure| {
                    (page_compose_tag(failure), failure.offset)
                });
            return CaseOutcome::refused(kind, offset, "Generic", page.width, page.height, &source);
        }
    };
    let page_report = match ready(page_sink.finish(&generic_report)) {
        Ok(report) => report,
        Err(error) => {
            return CaseOutcome::refused(
                page_compose_tag(&error),
                error.offset,
                "PageCompose",
                page.width,
                page.height,
                &source,
            );
        }
    };
    drop(page_sink);
    let final_max_request = final_output.max_request;
    let (pixel_sha, black_pixels, output_bytes) = match final_output.finish() {
        Ok(metrics) => metrics,
        Err(_) => {
            return CaseOutcome::refused(
                "hash_sink_incomplete",
                0,
                "Hash",
                page.width,
                page.height,
                &source,
            );
        }
    };
    let max_request = source
        .max_request
        .max(completed.report.progress.max_request_bytes)
        .max(page_report.progress.max_request_bytes)
        .max(text_output.max_request)
        .max(final_max_request)
        .max(page.row_stride)
        .max(HASH_WRITE_CHUNK);
    // This is a bounded row/composition working-set estimate: the text
    // composer's reported resident buffers, or the concurrent page-sink
    // buffer plus three generic rows and an allowance for generic MQ contexts.
    // It excludes dictionary/refinement context banks and symbol catalogs;
    // VmHWM (`rss_kib`) measures the whole process, including those objects.
    let generic_resident = (page.row_stride as u64)
        .saturating_mul(3)
        .saturating_add(GENERIC_CONTEXT_ALLOWANCE_BYTES);
    let peak_resident_bytes = completed.report.progress.peak_resident_bytes.max(
        page_report
            .progress
            .peak_resident_bytes
            .saturating_add(generic_resident),
    );
    let rss_kib = peak_rss_kib().unwrap_or(0);
    if source.bytes > MAX_SOURCE_BYTES
        || max_request > MAX_IO_CHUNK
        || peak_resident_bytes > MAX_RESIDENT_BYTES
        || completed.scratch_bytes > MAX_SCRATCH_BYTES
        || rss_kib > MAX_RSS_KIB
    {
        return CaseOutcome::refused(
            "diagnostic_budget_exceeded",
            0,
            "Metrics",
            page.width,
            page.height,
            &source,
        );
    }
    CaseOutcome {
        status: "COMPLETE",
        width: page.width,
        height: page.height,
        rows: page_report.progress.rows_written,
        output_bytes,
        pixel_sha,
        black_pixels,
        source_bytes: source.bytes,
        max_request,
        peak_resident_bytes,
        scratch_bytes: completed.scratch_bytes,
        rss_kib,
        refusal_kind: "-".to_owned(),
        refusal_offset: 0,
        stage: "Complete".to_owned(),
        anomaly_marker: if page_report.text_header_anomaly
            == Some(TextHeaderAnomaly::UnusedRefinementTemplate)
        {
            COMPATIBILITY_MARKER
        } else {
            "-"
        },
    }
}

fn run() -> Result<(), Box<dyn StdError>> {
    let mut args = env::args_os();
    let _ = args.next();
    let fixture = args
        .next()
        .ok_or("usage: EXAMPLE PRIVATE_TABLE PLAN.tsv [hn-c8-unused-refinement-template]")?;
    let plan = args
        .next()
        .ok_or("usage: EXAMPLE PRIVATE_TABLE PLAN.tsv [hn-c8-unused-refinement-template]")?;
    let policy = match args.next() {
        None => TextHeaderPolicy::Strict,
        Some(value) if value == COMPATIBILITY_ARG => TextHeaderPolicy::HnC8UnusedRefinementTemplate,
        _ => return Err("invalid compatibility policy".into()),
    };
    if args.next().is_some() {
        return Err("too many arguments".into());
    }
    let cases = parse_page_plan(Path::new(&plan))?;
    let table = table(Path::new(&fixture), &Limits::default())?;
    for case in &cases {
        println!("{}", run_case(case, &table, policy).line(case));
    }
    println!("TOTAL\t{EXPECTED_CASES}");
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("jbig2 page parity: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use caj2pdf_core::{RangedSource, read_exact_at, write_all};

    #[test]
    fn packed_hash_uses_p4_rows_and_rejects_padding() {
        let mut sink = PackedHashSink::new(3, 2);
        assert_eq!(ready(sink.write(&[0b1010_0000])).unwrap(), 1);
        assert_eq!(ready(sink.write(&[0b0100_0000])).unwrap(), 1);
        ready(sink.flush()).unwrap();
        let (hash, black, bytes) = sink.finish().unwrap();
        assert_eq!(hash, hex(&Sha256::digest([0b1010_0000, 0b0100_0000])));
        assert_eq!((black, bytes), (3, 2));

        let mut malformed = PackedHashSink::new(3, 1);
        assert!(ready(malformed.write(&[0b1010_0001])).is_err());
        assert!(ready(malformed.flush()).is_err());
    }

    #[test]
    fn bounded_sink_uses_short_writes_and_rejects_excess() {
        let mut sink = PackedHashSink::new(8, (HASH_WRITE_CHUNK + 1) as u32);
        assert_eq!(
            ready(sink.write(&vec![0; HASH_WRITE_CHUNK + 1])).unwrap(),
            HASH_WRITE_CHUNK
        );
        assert_eq!(ready(sink.write(&[0])).unwrap(), 1);
        ready(sink.flush()).unwrap();
        assert_eq!(sink.finish().unwrap().2, (HASH_WRITE_CHUNK + 1) as u64);
        let mut small = PackedHashSink::new(8, 1);
        assert!(ready(small.write(&[0, 0])).is_err());
    }

    struct FaultSource {
        mode: u8,
    }

    impl RangedSource for FaultSource {
        fn size(&self) -> u64 {
            3
        }
        async fn read_at(
            &mut self,
            offset: u64,
            destination: &mut [u8],
        ) -> caj2pdf_core::Result<usize> {
            if self.mode == 2 {
                return Ok(0);
            }
            if self.mode == 3 {
                return Ok(destination.len() + 1);
            }
            let count = destination.len().min(1);
            destination[..count]
                .copy_from_slice(&[b'a', b'b', b'c'][offset as usize..offset as usize + count]);
            Ok(count)
        }
    }

    struct FaultSink {
        mode: u8,
        bytes: Vec<u8>,
    }

    impl SequentialSink for FaultSink {
        async fn write(&mut self, bytes: &[u8]) -> caj2pdf_core::Result<usize> {
            if self.mode == 2 {
                return Ok(0);
            }
            if self.mode == 3 {
                return Ok(bytes.len() + 1);
            }
            self.bytes.extend_from_slice(&bytes[..1]);
            Ok(1)
        }
        async fn flush(&mut self) -> caj2pdf_core::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn native_stream_contract_handles_short_zero_and_overreported_io() {
        let mut source = MeteredSource::new(FaultSource { mode: 1 });
        let mut bytes = [0u8; 3];
        ready(read_exact_at(
            &mut source,
            0,
            &mut bytes,
            &Limits::default(),
            &NeverCancel,
        ))
        .unwrap();
        assert_eq!(bytes, *b"abc");
        assert_eq!((source.calls, source.bytes, source.max_request), (3, 3, 3));
        for (mode, expected) in [(2, "truncated"), (3, "invalid")] {
            let mut source = MeteredSource::new(FaultSource { mode });
            let error = ready(read_exact_at(
                &mut source,
                0,
                &mut bytes,
                &Limits::default(),
                &NeverCancel,
            ))
            .unwrap_err();
            assert!(match (expected, error) {
                ("truncated", Error::TruncatedInput { .. })
                | ("invalid", Error::InvalidInput { .. }) => true,
                _ => false,
            });
        }

        let mut sink = FaultSink {
            mode: 1,
            bytes: Vec::new(),
        };
        let mut count = 0;
        ready(write_all(
            &mut sink,
            b"abc",
            &mut count,
            &Limits::default(),
            &NeverCancel,
        ))
        .unwrap();
        assert_eq!((sink.bytes, count), (b"abc".to_vec(), 3));
        for mode in [2, 3] {
            let mut sink = FaultSink {
                mode,
                bytes: Vec::new(),
            };
            let mut count = 0;
            let error = ready(write_all(
                &mut sink,
                b"a",
                &mut count,
                &Limits::default(),
                &NeverCancel,
            ))
            .unwrap_err();
            assert!(match (mode, error) {
                (2, Error::Io(inner)) => inner.kind() == std::io::ErrorKind::WriteZero,
                (3, Error::InvalidInput { .. }) => true,
                _ => false,
            });
            assert_eq!(count, 0);
        }
    }

    #[test]
    fn text_pipeline_errors_keep_the_actual_segment_offset() {
        use caj2pdf_core::jbig2::{
            dictionary::{DictionaryError, DictionaryErrorKind, DictionaryProgress},
            refinement_dictionary::{
                RefinementDictionaryError, RefinementDictionaryErrorKind,
                RefinementDictionaryProgress,
            },
        };
        let first = DictionaryError {
            segment: 1,
            offset: 123,
            progress: Box::new(DictionaryProgress::default()),
            kind: DictionaryErrorKind::Malformed("invented fault"),
        };
        assert_eq!(
            text_pipeline_failure(&first, 0),
            ("dictionary_malformed".to_owned(), 123, "Dictionary")
        );
        let second = RefinementDictionaryError {
            segment: 2,
            offset: 456,
            progress: Box::new(RefinementDictionaryProgress::default()),
            kind: RefinementDictionaryErrorKind::Malformed("invented fault"),
        };
        assert_eq!(
            text_pipeline_failure(&second, 0),
            (
                "refinement_dictionary_malformed".to_owned(),
                456,
                "RefinementDictionary"
            )
        );
    }
}
