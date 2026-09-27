// SPDX-License-Identifier: MIT

//! Optional #86 text-instance diagnostic for an external SHA-pinned corpus.
//! It reports decoder control flow, never placement or pixel compatibility.

#[path = "support/mod.rs"]
#[allow(dead_code)] // The first-dictionary metrics example uses the remaining helpers.
mod support;
#[path = "support/text_case.rs"]
mod text_case;
use text_case::*;

use caj2pdf_core::{
    Error, Limits, MAX_IO_CHUNK, NeverCancel, RangedSource,
    jbig2::{
        HeaderLimits, SegmentHeader, SegmentSpan,
        dictionary::{DictionaryBudget, DirectDictionaryDecoder},
        iaid::IaidContextBanks,
        mq::{MqBudget, MqErrorKind},
        read_segment_header,
        refinement::{RefinementBudget, RefinementErrorKind},
        refinement_dictionary::{
            RefinementDictionaryBudget, RefinementDictionaryDecoder, SymbolStore,
        },
        text::{TextRegionBudget, TextRegionErrorKind, read_text_region_header},
        text_instances::{
            TextBitmap, TextInstance, TextInstanceBudget, TextInstanceDecoder,
            TextInstanceErrorKind,
        },
    },
    native::{SeekableSource, WriteSink},
};
use sha2::{Digest, Sha256};
use std::{
    env,
    error::Error as StdError,
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
use support::{TempStore, digest_file, hex, ready, table};

const MAX_PLAN_BYTES: u64 = 2 * 1024 * 1024;
const EXPECTED_CASES: usize = 546;

struct CaseOutcome {
    status: &'static str,
    completed: u32,
    ri_zero: u32,
    ri_one: u32,
    strips: u32,
    refusal: &'static str,
    offset: u64,
    decision: String,
    trace_sha: String,
}

fn header_outcome(status: &'static str, refusal: &'static str, offset: u64) -> CaseOutcome {
    CaseOutcome {
        status,
        completed: 0,
        ri_zero: 0,
        ri_one: 0,
        strips: 0,
        refusal,
        offset,
        decision: "Header".to_owned(),
        trace_sha: hex(&Sha256::digest([])),
    }
}

/// Hash each completed placement and bitmap handle in an explicit wire order.
/// This is a local trace fingerprint, not an independent placement oracle.
fn trace_event(hash: &mut Sha256, instance: TextInstance) {
    hash.update(instance.index.to_be_bytes());
    hash.update(instance.strip.to_be_bytes());
    hash.update(instance.symbol_id.to_be_bytes());
    hash.update([u8::from(instance.ri)]);
    hash.update(instance.x.to_be_bytes());
    hash.update(instance.y.to_be_bytes());
    hash.update(instance.width.to_be_bytes());
    hash.update(instance.height.to_be_bytes());
    let (store, base, descriptor) = match instance.bitmap {
        TextBitmap::Stored(stored) => (
            match stored.store {
                SymbolStore::Imported => 0,
                SymbolStore::New => 1,
            },
            stored.store_base,
            stored.symbol,
        ),
        TextBitmap::Refined { store_base, symbol } => (2, store_base, symbol),
    };
    hash.update([store]);
    hash.update(base.to_be_bytes());
    hash.update(descriptor.width.to_be_bytes());
    hash.update(descriptor.height.to_be_bytes());
    hash.update(descriptor.row_stride.to_be_bytes());
    hash.update(descriptor.relative_store_offset.to_be_bytes());
    hash.update(descriptor.stored_bytes.to_be_bytes());
}

fn one_case(
    case: &Case,
    table: &caj2pdf_core::jbig2::mq::MqTable,
) -> Result<CaseOutcome, Box<dyn StdError>> {
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let dictionary_budget = DictionaryBudget::default();
    let mut source = SeekableSource::new(File::open(&case.source)?)?;
    let first = checked_header(&mut source, &case.source, &case.first, 1, &limits)?;
    let second = checked_header(&mut source, &case.source, &case.second, 2, &limits)?;
    let third = checked_text_segment(&mut source, case, &limits)?;
    let text_header = ready(read_text_region_header(
        &mut source,
        &third,
        &second,
        &limits,
        TextRegionBudget::default(),
        &NeverCancel,
    ));
    let text_header = match text_header {
        Ok(header) if !case.text.anomaly && header.instances == case.text.instances => header,
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
    let mut temporary_sink = WriteSink::new(temporary_file);
    let mut text_decoder = match ready(TextInstanceDecoder::new(
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
    )) {
        Ok(decoder) => decoder,
        Err(error) => {
            let progress = *error.progress;
            return Ok(CaseOutcome {
                status: "REFUSED",
                completed: progress.completed_instances,
                ri_zero: progress.ri_zero,
                ri_one: progress.ri_one,
                strips: progress.strips,
                refusal: refusal_kind(&error.kind),
                offset: error.offset,
                decision: format!("{:?}", progress.decision),
                trace_sha: hex(&Sha256::digest([])),
            });
        }
    };
    let mut trace = Sha256::new();
    let outcome = loop {
        match ready(text_decoder.next()) {
            Ok(Some(instance)) => trace_event(&mut trace, instance),
            Ok(None) => {
                let progress = text_decoder.progress();
                break CaseOutcome {
                    status: "COMPLETE",
                    completed: progress.completed_instances,
                    ri_zero: progress.ri_zero,
                    ri_one: progress.ri_one,
                    strips: progress.strips,
                    refusal: "-",
                    offset: progress.mq.map_or(0, |mq| mq.current_input_offset),
                    decision: format!("{:?}", progress.decision),
                    trace_sha: hex(&trace.finalize()),
                };
            }
            Err(error) => {
                let progress = *error.progress;
                break CaseOutcome {
                    status: "REFUSED",
                    completed: progress.completed_instances,
                    ri_zero: progress.ri_zero,
                    ri_one: progress.ri_one,
                    strips: progress.strips,
                    refusal: refusal_kind(&error.kind),
                    offset: error.offset,
                    decision: format!("{:?}", progress.decision),
                    trace_sha: hex(&trace.finalize()),
                };
            }
        }
    };
    drop(text_decoder);
    drop(temporary_sink);
    drop(temporary_store);
    drop(new_store);
    drop(second_store);
    drop(imported);
    drop(first_store);
    Ok(outcome)
}

fn run() -> Result<(), Box<dyn StdError>> {
    let mut args = env::args_os();
    let _ = args.next();
    let fixture = args.next().ok_or("usage: EXAMPLE PRIVATE_TABLE PLAN.tsv")?;
    let plan = args.next().ok_or("usage: EXAMPLE PRIVATE_TABLE PLAN.tsv")?;
    if args.next().is_some() {
        return Err("usage: EXAMPLE PRIVATE_TABLE PLAN.tsv".into());
    }
    let cases = parse_plan(Path::new(&plan))?;
    let limits = Limits::default();
    let table = table(Path::new(&fixture), &limits)?;
    for case in cases {
        let outcome = one_case(&case, &table).map_err(|error| {
            format!(
                "{} page {} image {}: {error}",
                case.id, case.page, case.image
            )
        })?;
        println!(
            "CASE\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            case.id,
            case.page,
            case.image,
            outcome.status,
            outcome.completed,
            outcome.ri_zero,
            outcome.ri_one,
            outcome.strips,
            outcome.refusal,
            outcome.offset,
            outcome.decision,
            outcome.trace_sha
        );
    }
    println!("TOTAL\t{EXPECTED_CASES}");
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("jbig2 text instance diagnostic: {error}");
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
    }
}
