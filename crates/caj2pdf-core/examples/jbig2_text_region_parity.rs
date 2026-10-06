// SPDX-License-Identifier: MIT

//! Optional #87 text-region pixel comparison against the SHA-pinned #85 oracle.
//! Pixel references remain in Python; Rust receives only source spans.

#[path = "support/mod.rs"]
#[allow(dead_code)] // The first-dictionary metrics example uses the remaining helpers.
mod support;
#[path = "support/text_case.rs"]
mod text_case;
#[path = "support/text_pipeline.rs"]
#[allow(dead_code)]
mod text_pipeline;
use text_case::*;
use text_pipeline::*;

#[cfg(test)]
use caj2pdf_core::{
    Error, SequentialSink,
    jbig2::{
        mq::MqErrorKind,
        refinement::RefinementErrorKind,
        text::TextRegionErrorKind,
        text_composer::{BitmapStore, RandomAccessScratch, TextComposeErrorKind},
    },
};
use caj2pdf_core::{
    Limits,
    jbig2::text::{TextHeaderAnomaly, TextHeaderPolicy},
    native::{SeekableSource, WriteSink},
};
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::{cell::Cell, fs::OpenOptions, rc::Rc};
use std::{env, error::Error as StdError, fs::File, io::Read, path::Path};
#[cfg(test)]
use support::ready;
use support::{TempStore, hex, table};

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
            if (offset + index as u64 + 1).is_multiple_of(stride) && byte & low_mask != 0 {
                return Err("final row has nonzero low padding bits".into());
            }
            black += u64::from(byte.count_ones());
        }
        digest.update(&bytes[..got]);
        offset += got as u64;
    }
    Ok((hex(&digest.finalize()), black, expected))
}

fn one_case(
    case: &Case,
    table: &caj2pdf_core::jbig2::mq::MqTable,
    policy: TextHeaderPolicy,
) -> Result<CaseOutcome, Box<dyn StdError>> {
    let mut source = MeteredSource::new(SeekableSource::new(File::open(&case.source)?)?);
    let (pixel_store, pixel_file) = TempStore::create("caj2pdf-text-region-pixels")?;
    let mut output = WriteSink::new(pixel_file);
    let outcome = run_text_phase(case, table, policy, &mut source, &mut output)?;
    drop(output);
    let completed = match outcome {
        TextPhaseOutcome::Refused(outcome) => return Ok(outcome),
        TextPhaseOutcome::Complete(completed) => completed,
    };
    let region = completed.report;
    let (pixel_sha, black_pixels, output_bytes) =
        pixel_metrics(pixel_store.path(), region.width, region.height)?;
    if output_bytes != region.packed_bytes || region.progress.output_bytes_written != output_bytes {
        return Err("composed pixel byte count differs from final file".into());
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
}
