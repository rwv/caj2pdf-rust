// SPDX-License-Identifier: MIT

//! Optional #87 text-region pixel comparison against the SHA-pinned #85 oracle.
//! Pixel references remain in Python; Rust receives only source spans.

#[path = "support/mod.rs"]
#[allow(dead_code)] // The first-dictionary metrics example uses the remaining helpers.
mod support;
#[path = "support/text_case.rs"]
mod text_case;
use text_case::*;

use caj2pdf_core::{
    Error, Limits, MAX_IO_CHUNK, NeverCancel, RangedSource, SequentialSink,
    jbig2::{
        HeaderLimits, SegmentHeader, SegmentSpan,
        dictionary::{DictionaryBudget, DirectDictionaryDecoder},
        iaid::IaidContextBanks,
        mq::{MqBudget, MqErrorKind},
        read_segment_header,
        refinement::{RefinementBudget, RefinementErrorKind},
        refinement_dictionary::{RefinementDictionaryBudget, RefinementDictionaryDecoder},
        text::{
            TextHeaderAnomaly, TextHeaderPolicy, TextRegionBudget, TextRegionErrorKind,
            read_text_region_header_with_policy,
        },
        text_composer::{
            BitmapStore, BitmapView, RandomAccessScratch, TextComposeBudget, TextComposeError,
            TextComposeErrorKind, TextComposer,
        },
        text_instances::{
            TextInstanceBudget, TextInstanceDecoder, TextInstanceError, TextInstanceErrorKind,
        },
    },
    native::{SeekableSource, WriteSink},
};
use sha2::{Digest, Sha256};
use std::{
    cell::Cell,
    env,
    error::Error as StdError,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    rc::Rc,
};
use support::{TempStore, digest_file, hex, peak_rss_kib, ready, table};

const MAX_PLAN_BYTES: u64 = 2 * 1024 * 1024;
const EXPECTED_CASES: usize = 546;
const COMPATIBILITY_ARG: &str = "hn-c8-unused-refinement-template";
const COMPATIBILITY_MARKER: &str = "HN_C8_UNUSED_REFINEMENT_TEMPLATE";

struct CaseOutcome {
    status: &'static str,
    completed: u32,
    rows: u32,
    output_bytes: u64,
    black_pixels: u64,
    pixel_sha: String,
    scratch_bytes: u64,
    max_request: usize,
    peak_resident: u64,
    refusal: String,
    offset: u64,
    stage: String,
    anomaly: &'static str,
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

struct FileScratch(File);

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

/// The source handle is read-only; only the matching producer advances its
/// revision. The producer has exclusive ownership of a private temp file.
struct TrackedSource<S> {
    inner: S,
    revision: Rc<Cell<u64>>,
}

impl<S: RangedSource> RangedSource for TrackedSource<S> {
    fn size(&self) -> u64 {
        self.inner.size()
    }

    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        self.inner.read_at(offset, destination).await
    }
}

impl<S: RangedSource> BitmapView for TrackedSource<S> {
    fn revision(&self) -> caj2pdf_core::Result<u64> {
        Ok(self.revision.get())
    }
}

struct TrackedSink<W> {
    inner: W,
    revision: Rc<Cell<u64>>,
}

impl<W: SequentialSink> SequentialSink for TrackedSink<W> {
    async fn write(&mut self, bytes: &[u8]) -> caj2pdf_core::Result<usize> {
        let next = self
            .revision
            .get()
            .checked_add(1)
            .ok_or(Error::InvalidInput {
                reason: "refined bitmap-store revision exhausted",
            })?;
        let written = self.inner.write(bytes).await?;
        if written > 0 {
            self.revision.set(next);
        }
        Ok(written)
    }

    async fn flush(&mut self) -> caj2pdf_core::Result<()> {
        self.inner.flush().await
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

fn compose_refusal_kind(error: &TextComposeErrorKind) -> String {
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

fn pixel_metrics(
    path: &Path,
    width: u32,
    height: u32,
) -> Result<(String, u64, u64), Box<dyn StdError>> {
    let mut file = File::open(path)?;
    let stride = u64::from(width).div_ceil(8);
    let expected = stride * u64::from(height);
    if stride == 0 || file.metadata()?.len() != expected {
        return Err("final pixel file length differs from region geometry".into());
    }
    let low_unused = (8 - width % 8) % 8;
    let low_mask = ((1u16 << low_unused) - 1) as u8;
    let mut bytes = [0u8; 64 * 1024];
    let mut digest = Sha256::new();
    let mut black = 0u64;
    let mut offset = 0u64;
    while offset < expected {
        let requested = (expected - offset).min(bytes.len() as u64) as usize;
        let got = file.read(&mut bytes[..requested])?;
        if got == 0 {
            return Err("final pixel file shortened while hashing".into());
        }
        for (index, byte) in bytes[..got].iter().enumerate() {
            if (offset + index as u64 + 1) % stride == 0 && byte & low_mask != 0 {
                return Err("final row has nonzero low padding bits".into());
            }
            black += u64::from(byte.count_ones());
        }
        digest.update(&bytes[..got]);
        offset += got as u64;
    }
    Ok((hex(&digest.finalize()), black, expected))
}

fn peak_resident_bytes() -> u64 {
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
        peak_resident: peak_resident_bytes(),
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
        peak_resident: peak_resident_bytes(),
        refusal: compose_refusal_kind(&error.kind),
        offset: error.offset,
        stage: format!("{:?}", progress.stage),
        anomaly: "-",
    }
}

fn one_case(
    case: &Case,
    table: &caj2pdf_core::jbig2::mq::MqTable,
    policy: TextHeaderPolicy,
) -> Result<CaseOutcome, Box<dyn StdError>> {
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let dictionary_budget = DictionaryBudget::default();
    let mut source = SeekableSource::new(File::open(&case.source)?)?;
    let first = checked_header(&mut source, &case.source, &case.first, 1, &limits)?;
    let second = checked_header(&mut source, &case.source, &case.second, 2, &limits)?;
    let third = checked_text_segment(&mut source, case, &limits)?;
    let text_header = ready(read_text_region_header_with_policy(
        &mut source,
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
            return Ok(header_outcome(
                "HEADER_REFUSED",
                "malformed_text_header",
                error.offset,
            ));
        }
        Ok(_) => {
            return Ok(header_outcome(
                "REFUSED",
                "header_classification_or_count",
                third.data.offset + 19,
            ));
        }
        Err(error) => {
            return Ok(header_outcome(
                "REFUSED",
                header_refusal_kind(&error.kind),
                error.offset,
            ));
        }
    };
    let (first_store, first_file) = TempStore::create("caj2pdf-dictionary-first")?;
    let mut first_sink = WriteSink::new(first_file);
    let mut first_banks = caj2pdf_core::jbig2::integer::IntegerContextBanks::with_extra_contexts(
        1024, &limits, &mq_budget,
    )?;
    let mut first_decoder = ready(DirectDictionaryDecoder::new(
        &mut source,
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
    drop(first_sink);
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
        &mut source,
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
    drop(second_sink);
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
    let refined_revision = Rc::new(Cell::new(0));
    let mut temporary_sink = TrackedSink {
        inner: WriteSink::new(temporary_file),
        revision: Rc::clone(&refined_revision),
    };
    let mut text_decoder = match ready(TextInstanceDecoder::new_with_header_policy(
        &mut source,
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
        Err(error) => return Ok(instance_refusal(error)),
    };
    let mut imported_for_compose = TrackedSource {
        inner: SeekableSource::new(File::open(first_store.path())?)?,
        revision: Rc::new(Cell::new(0)),
    };
    let mut new_for_compose = TrackedSource {
        inner: SeekableSource::new(File::open(second_store.path())?)?,
        revision: Rc::new(Cell::new(0)),
    };
    let mut refined_for_compose = TrackedSource {
        inner: GrowingFileSource(File::open(temporary_store.path())?),
        revision: Rc::clone(&refined_revision),
    };
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
    let (pixel_store, pixel_file) = TempStore::create("caj2pdf-text-region-pixels")?;
    let mut output = WriteSink::new(pixel_file);
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
            &mut output,
            &limits,
            &NeverCancel,
            TextComposeBudget::default(),
        ) {
            Ok(composer) => composer,
            Err(error) => return Ok(compose_refusal(error, scratch.size()?)),
        };
        ready(composer.compose())
    };
    let region = match composed {
        Ok(region) => region,
        Err(error) => return Ok(compose_refusal(error, scratch.size()?)),
    };
    drop(output);
    let (pixel_sha, black_pixels, output_bytes) =
        pixel_metrics(pixel_store.path(), region.width, region.height)?;
    if output_bytes != region.packed_bytes || region.progress.output_bytes_written != output_bytes {
        return Err("composed pixel byte count differs from final file".into());
    }
    if region.text_flags_raw != text_header.flags.raw
        || region.header_anomaly != text_header.anomaly
    {
        return Err("composed report differs from validated text-header policy".into());
    }
    Ok(CaseOutcome {
        status: "COMPLETE",
        completed: region.progress.completed_instances,
        rows: region.progress.output_rows,
        output_bytes,
        black_pixels,
        pixel_sha,
        scratch_bytes: region.packed_bytes,
        max_request: region.progress.max_request_bytes,
        peak_resident: peak_resident_bytes(),
        refusal: "-".to_owned(),
        offset: 0,
        stage: format!("{:?}", region.progress.stage),
        anomaly: if region.header_anomaly == Some(TextHeaderAnomaly::UnusedRefinementTemplate) {
            COMPATIBILITY_MARKER
        } else {
            "-"
        },
    })
}

fn run() -> Result<(), Box<dyn StdError>> {
    let mut args = env::args_os();
    let _ = args.next();
    let fixture = args.next().ok_or("usage: EXAMPLE PRIVATE_TABLE PLAN.tsv")?;
    let plan = args.next().ok_or("usage: EXAMPLE PRIVATE_TABLE PLAN.tsv")?;
    let policy = match args.next() {
        None => TextHeaderPolicy::Strict,
        Some(value) if value == COMPATIBILITY_ARG => TextHeaderPolicy::HnC8UnusedRefinementTemplate,
        _ => {
            return Err(
                "usage: EXAMPLE PRIVATE_TABLE PLAN.tsv [hn-c8-unused-refinement-template]".into(),
            );
        }
    };
    if args.next().is_some() {
        return Err(
            "usage: EXAMPLE PRIVATE_TABLE PLAN.tsv [hn-c8-unused-refinement-template]".into(),
        );
    }
    let cases = parse_plan(Path::new(&plan))?;
    let limits = Limits::default();
    let table = table(Path::new(&fixture), &limits)?;
    for case in cases {
        let outcome = one_case(&case, &table, policy).map_err(|error| {
            format!(
                "{} page {} image {}: {error}",
                case.id, case.page, case.image
            )
        })?;
        let mut row = format!(
            "CASE\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            case.id,
            case.page,
            case.image,
            outcome.status,
            outcome.completed,
            outcome.rows,
            outcome.output_bytes,
            outcome.black_pixels,
            outcome.pixel_sha,
            outcome.scratch_bytes,
            outcome.max_request,
            outcome.peak_resident,
            outcome.refusal,
            outcome.offset,
            outcome.stage,
        );
        if policy != TextHeaderPolicy::Strict {
            row.push('\t');
            row.push_str(outcome.anomaly);
        }
        println!("{row}");
    }
    println!("TOTAL\t{EXPECTED_CASES}");
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("jbig2 text region parity: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusal_tags_keep_nested_failure_categories() {
        assert_eq!(
            mq_refusal_kind(&MqErrorKind::MissingTerminator),
            "mq_missing_terminator"
        );
        assert_eq!(
            mq_refusal_kind(&MqErrorKind::InvalidMarker(0x00)),
            "mq_invalid_marker"
        );
        assert_eq!(
            mq_refusal_kind(&MqErrorKind::Source(Error::TruncatedInput {
                offset: 7,
                expected: 2,
                available: 1,
            })),
            "mq_source_truncated_input"
        );
        assert_eq!(
            mq_refusal_kind(&MqErrorKind::Source(Error::InvalidInput {
                reason: "overreported read",
            })),
            "mq_source_invalid_input"
        );
        assert_eq!(
            refinement_refusal_kind(&RefinementErrorKind::TruncatedReference),
            "refinement_truncated_reference"
        );
        assert_eq!(
            refinement_refusal_kind(&RefinementErrorKind::Sink(Error::Io(
                std::io::Error::other("test"),
            ))),
            "refinement_sink_io"
        );
        assert_eq!(
            header_refusal_kind(&TextRegionErrorKind::MalformedFlags {
                field: "test",
                raw: 0xa40c,
            }),
            "header_malformed_flags"
        );
        assert_eq!(
            compose_refusal_kind(&TextComposeErrorKind::Source {
                store: BitmapStore::Refined,
                error: Error::TruncatedInput {
                    offset: 7,
                    expected: 2,
                    available: 0,
                },
            }),
            "refined_source_truncated_input"
        );
        assert_eq!(
            compose_refusal_kind(&TextComposeErrorKind::StoreMutation {
                store: BitmapStore::Imported,
                reason: "test",
            }),
            "imported_store_mutation"
        );
        assert_eq!(
            compose_refusal_kind(&TextComposeErrorKind::Output(Error::Cancelled)),
            "output_cancelled"
        );
    }

    #[test]
    fn native_scratch_round_trip_and_pixel_metrics() {
        let (store, file) = TempStore::create("caj2pdf-text-composer-test").unwrap();
        drop(file);
        let mut scratch = FileScratch(
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(store.path())
                .unwrap(),
        );
        ready(scratch.set_len(2)).unwrap();
        assert_eq!(scratch.size().unwrap(), 2);
        assert_eq!(
            ready(scratch.write_at(0, &[0b1010_0000, 0b0100_0000])).unwrap(),
            2
        );
        ready(scratch.flush()).unwrap();
        let mut read = [0u8; 2];
        assert_eq!(ready(scratch.read_at(0, &mut read)).unwrap(), 2);
        assert_eq!(read, [0b1010_0000, 0b0100_0000]);
        let (hash, black, bytes) = pixel_metrics(store.path(), 3, 2).unwrap();
        assert_eq!(hash, hex(&Sha256::digest(read)));
        assert_eq!((black, bytes), (3, 2));
        ready(scratch.write_at(1, &[0b0100_0001])).unwrap();
        assert!(pixel_metrics(store.path(), 3, 2).is_err());
    }

    #[test]
    fn refined_writer_advances_revision_only_after_written_bytes() {
        let revision = Rc::new(Cell::new(0));
        let mut sink = TrackedSink {
            inner: WriteSink::new(Vec::new()),
            revision: Rc::clone(&revision),
        };
        assert_eq!(ready(sink.write(&[1, 2, 3])).unwrap(), 3);
        assert_eq!(revision.get(), 1);
        ready(sink.flush()).unwrap();
        assert_eq!(revision.get(), 1);
        assert_eq!(ready(sink.write(&[])).unwrap(), 0);
        assert_eq!(revision.get(), 1);
    }
}
