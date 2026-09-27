// SPDX-License-Identifier: MIT

//! Shared native-only SHA-pinned plan and refusal helpers for text diagnostics.

use crate::support::{digest_file, ready};
use caj2pdf_core::{
    Error, Limits, MAX_IO_CHUNK, NeverCancel, RangedSource,
    jbig2::mq::MqErrorKind,
    jbig2::refinement::RefinementErrorKind,
    jbig2::text::TextRegionErrorKind,
    jbig2::text_instances::TextInstanceErrorKind,
    jbig2::{HeaderLimits, SegmentHeader, SegmentSpan, read_segment_header},
};
use std::{
    error::Error as StdError,
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

pub(crate) const MAX_PLAN_BYTES: u64 = 2 * 1024 * 1024;
pub(crate) const EXPECTED_CASES: usize = 546;

pub(super) struct SpanPin {
    pub(super) offset: u64,
    pub(super) length: u64,
    pub(super) data_sha: String,
    pub(super) new_symbols: u32,
    pub(super) exported_symbols: u32,
}

pub(super) struct Case {
    pub(super) id: String,
    pub(super) page: u32,
    pub(super) image: u32,
    pub(super) source: PathBuf,
    pub(super) first: SpanPin,
    pub(super) second: SpanPin,
    pub(super) text: TextPin,
}

pub(super) struct TextPin {
    pub(super) offset: u64,
    pub(super) length: u64,
    pub(super) encoded_sha: String,
    pub(super) instances: u32,
    pub(super) anomaly: bool,
}

fn parse_pin(fields: &[&str]) -> Result<SpanPin, Box<dyn StdError>> {
    if fields.len() != 5
        || fields[2].len() != 64
        || !fields[2].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("invalid dictionary span pin".into());
    }
    let pin = SpanPin {
        offset: fields[0].parse()?,
        length: fields[1].parse()?,
        data_sha: fields[2].to_ascii_lowercase(),
        new_symbols: fields[3].parse()?,
        exported_symbols: fields[4].parse()?,
    };
    if pin.length < 12 || pin.length > 64 * 1024 * 1024 {
        return Err("dictionary framing span is outside 12..64 MiB".into());
    }
    Ok(pin)
}

pub(super) fn parse_plan(path: &Path) -> Result<Vec<Case>, Box<dyn StdError>> {
    if fs::metadata(path)?.len() > MAX_PLAN_BYTES {
        return Err("diagnostic plan exceeds 2 MiB".into());
    }
    let mut text = String::new();
    File::open(path)?
        .take(MAX_PLAN_BYTES + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > MAX_PLAN_BYTES {
        return Err("diagnostic plan grew beyond 2 MiB".into());
    }
    let mut cases = Vec::new();
    cases.try_reserve_exact(EXPECTED_CASES)?;
    for line in text.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        cases.push(parse_case_fields(&fields)?);
        if cases.len() > EXPECTED_CASES {
            return Err("diagnostic plan has too many cases".into());
        }
    }
    if cases.len() != EXPECTED_CASES {
        return Err("diagnostic plan does not contain 546 cases".into());
    }
    Ok(cases)
}

/// Parse the stable prefix shared by text-only and full-page private plans.
pub(super) fn parse_case_fields(fields: &[&str]) -> Result<Case, Box<dyn StdError>> {
    if fields.len() != 19 || fields[0].is_empty() || fields[3].is_empty() {
        return Err("diagnostic plan line has invalid fields".into());
    }
    if fields[16].len() != 64 || !fields[16].bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("text segment SHA is invalid".into());
    }
    let classification = fields[18];
    if classification != "STANDARD_VALID" && classification != "INTEROPERABILITY_NONCONFORMING" {
        return Err("text header classification is invalid".into());
    }
    let case = Case {
        id: fields[0].to_owned(),
        page: fields[1].parse()?,
        image: fields[2].parse()?,
        source: PathBuf::from(fields[3]),
        first: parse_pin(&fields[4..9])?,
        second: parse_pin(&fields[9..14])?,
        text: TextPin {
            offset: fields[14].parse()?,
            length: fields[15].parse()?,
            encoded_sha: fields[16].to_ascii_lowercase(),
            instances: fields[17].parse()?,
            anomaly: classification == "INTEROPERABILITY_NONCONFORMING",
        },
    };
    if case.page == 0
        || case.image == 0
        || case.first.offset >= case.second.offset
        || case.second.offset >= case.text.offset
        || case.text.length < 23
        || case.text.length > 64 * 1024 * 1024
    {
        return Err("diagnostic plan coordinates or span order are invalid".into());
    }
    Ok(case)
}

/// A second read handle observes the length after the sink flushes each symbol.
/// The ordinary native adapter intentionally snapshots source size at creation.
pub(super) struct GrowingFileSource(pub(super) File);

impl RangedSource for GrowingFileSource {
    fn size(&self) -> u64 {
        self.0.metadata().map_or(0, |metadata| metadata.len())
    }

    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        if destination.len() > MAX_IO_CHUNK {
            return Err(Error::LimitExceeded {
                resource: "I/O request bytes",
                limit: MAX_IO_CHUNK as u64,
                attempted: destination.len() as u64,
            });
        }
        self.0.seek(SeekFrom::Start(offset))?;
        Ok(self.0.read(destination)?)
    }
}

pub(super) fn checked_header<S: RangedSource>(
    source: &mut S,
    source_path: &Path,
    pin: &SpanPin,
    number: u32,
    limits: &Limits,
) -> Result<SegmentHeader, Box<dyn StdError>> {
    let header = ready(read_segment_header(
        source,
        SegmentSpan {
            offset: pin.offset,
            length: pin.length,
        },
        limits,
        HeaderLimits::default(),
        &NeverCancel,
    ))?;
    if header.number != number
        || header.segment_type != 0
        || header.page_association != 1
        || header.data.length < 12
        || (number == 1 && !header.referred_to.is_empty())
        || (number == 2 && header.referred_to.as_slice() != [1])
        || digest_file(source_path, Some((header.data.offset, header.data.length)))? != pin.data_sha
    {
        return Err(format!("dictionary {number} framing or data SHA differs").into());
    }
    Ok(header)
}

pub(super) fn checked_text_segment<S: RangedSource>(
    source: &mut S,
    case: &Case,
    limits: &Limits,
) -> Result<SegmentHeader, Box<dyn StdError>> {
    let header = ready(read_segment_header(
        source,
        SegmentSpan {
            offset: case.text.offset,
            length: case.text.length,
        },
        limits,
        HeaderLimits::default(),
        &NeverCancel,
    ))?;
    if header.number != 3
        || header.segment_type != 6
        || header.page_association != 1
        || header.referred_to.as_slice() != [2]
        || header.data.length < 25
        || digest_file(&case.source, Some((case.text.offset, case.text.length)))?
            != case.text.encoded_sha
    {
        return Err("text region framing or encoded SHA differs".into());
    }
    Ok(header)
}

pub(super) fn mq_refusal_kind(error: &MqErrorKind) -> &'static str {
    match error {
        MqErrorKind::InvalidTable(_) => "mq_invalid_table",
        MqErrorKind::InvalidContext => "mq_invalid_context",
        MqErrorKind::InvalidState => "mq_invalid_state",
        MqErrorKind::InvalidSpan(_) => "mq_invalid_span",
        MqErrorKind::InvalidBudget => "mq_invalid_budget",
        MqErrorKind::MissingTerminator => "mq_missing_terminator",
        MqErrorKind::InvalidMarker(_) => "mq_invalid_marker",
        MqErrorKind::LimitExceeded { .. } => "mq_limit_exceeded",
        MqErrorKind::AllocationFailed => "mq_allocation_failed",
        MqErrorKind::Cancelled => "mq_cancelled",
        MqErrorKind::Source(Error::TruncatedInput { .. }) => "mq_source_truncated_input",
        MqErrorKind::Source(Error::InvalidInput { .. }) => "mq_source_invalid_input",
        MqErrorKind::Source(Error::Io(_)) => "mq_source_io",
        MqErrorKind::Source(_) => "mq_source_other",
        MqErrorKind::Poisoned => "mq_poisoned",
        MqErrorKind::Invariant(_) => "mq_invariant",
        MqErrorKind::WrongSymbolCount { .. } => "mq_wrong_symbol_count",
    }
}

pub(super) fn refinement_refusal_kind(error: &RefinementErrorKind) -> &'static str {
    match error {
        RefinementErrorKind::Malformed(_) => "refinement_malformed",
        RefinementErrorKind::InvalidSpan(_) => "refinement_invalid_span",
        RefinementErrorKind::TruncatedReference => "refinement_truncated_reference",
        RefinementErrorKind::Unsupported { .. } => "refinement_unsupported",
        RefinementErrorKind::LimitExceeded { .. } => "refinement_limit_exceeded",
        RefinementErrorKind::AllocationFailed => "refinement_allocation_failed",
        RefinementErrorKind::Cancelled => "refinement_cancelled",
        RefinementErrorKind::ReferenceSource(Error::TruncatedInput { .. }) => {
            "refinement_source_truncated_input"
        }
        RefinementErrorKind::ReferenceSource(Error::InvalidInput { .. }) => {
            "refinement_source_invalid_input"
        }
        RefinementErrorKind::ReferenceSource(Error::Io(_)) => "refinement_source_io",
        RefinementErrorKind::ReferenceSource(_) => "refinement_source_other",
        RefinementErrorKind::Sink(Error::TruncatedInput { .. }) => {
            "refinement_sink_truncated_input"
        }
        RefinementErrorKind::Sink(Error::InvalidInput { .. }) => "refinement_sink_invalid_input",
        RefinementErrorKind::Sink(Error::Io(_)) => "refinement_sink_io",
        RefinementErrorKind::Sink(_) => "refinement_sink_other",
        RefinementErrorKind::Mq(error) => mq_refusal_kind(&error.kind),
        RefinementErrorKind::Poisoned => "refinement_poisoned",
    }
}

pub(super) fn header_refusal_kind(error: &TextRegionErrorKind) -> &'static str {
    match error {
        TextRegionErrorKind::InvalidSpan(_) => "header_invalid_span",
        TextRegionErrorKind::Truncated(_) => "header_truncated",
        TextRegionErrorKind::Malformed(_) => "header_malformed",
        TextRegionErrorKind::MalformedFlags { .. } => "header_malformed_flags",
        TextRegionErrorKind::Unsupported { .. } => "header_unsupported",
        TextRegionErrorKind::LimitExceeded { .. } => "header_limit_exceeded",
        TextRegionErrorKind::Cancelled => "header_cancelled",
        TextRegionErrorKind::Source(Error::TruncatedInput { .. }) => {
            "header_source_truncated_input"
        }
        TextRegionErrorKind::Source(Error::InvalidInput { .. }) => "header_source_invalid_input",
        TextRegionErrorKind::Source(Error::Io(_)) => "header_source_io",
        TextRegionErrorKind::Source(_) => "header_source_other",
    }
}

pub(super) fn refusal_kind(error: &TextInstanceErrorKind) -> &'static str {
    match error {
        TextInstanceErrorKind::InvalidSpan(_) => "invalid_span",
        TextInstanceErrorKind::Malformed(_) => "malformed",
        TextInstanceErrorKind::Unsupported { .. } => "unsupported",
        TextInstanceErrorKind::LimitExceeded { .. } => "limit_exceeded",
        TextInstanceErrorKind::Cancelled => "cancelled",
        TextInstanceErrorKind::Header(error) => header_refusal_kind(&error.kind),
        TextInstanceErrorKind::Mq(error) => mq_refusal_kind(&error.kind),
        TextInstanceErrorKind::Refinement(error) => refinement_refusal_kind(&error.kind),
        TextInstanceErrorKind::Poisoned => "poisoned",
    }
}
