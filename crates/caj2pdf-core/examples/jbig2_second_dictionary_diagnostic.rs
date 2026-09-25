// SPDX-License-Identifier: MIT

//! Optional #2 IAAI prefix diagnostic for an external SHA-pinned corpus.
//! It reports decoder control-flow observations, never pixel compatibility.

#[path = "support/mod.rs"]
#[allow(dead_code)] // The first-dictionary metrics example uses the remaining helpers.
mod support;

use caj2pdf_core::{
    Error, Limits, MAX_IO_CHUNK, NeverCancel, RangedSource,
    jbig2::{
        HeaderLimits, SegmentHeader, SegmentSpan,
        dictionary::{DictionaryBudget, DirectDictionaryDecoder},
        iaid::IaidContextBanks,
        mq::MqBudget,
        read_segment_header,
        refinement::RefinementBudget,
        refinement_dictionary::{
            RefinementDictionaryBudget, RefinementDictionaryDecoder, RefinementDictionaryErrorKind,
        },
    },
    native::{SeekableSource, WriteSink},
};
use std::{
    env,
    error::Error as StdError,
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
use support::{TempStore, digest_file, ready, table};

const MAX_PLAN_BYTES: u64 = 2 * 1024 * 1024;
const EXPECTED_CASES: usize = 546;

struct SpanPin {
    offset: u64,
    length: u64,
    data_sha: String,
    new_symbols: u32,
    exported_symbols: u32,
}

struct Case {
    id: String,
    page: u32,
    image: u32,
    source: PathBuf,
    first: SpanPin,
    second: SpanPin,
}

struct CaseOutcome {
    status: &'static str,
    single_reference: u32,
    zero: u32,
    aggregation: u32,
    refusal: &'static str,
    offset: u64,
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

fn parse_plan(path: &Path) -> Result<Vec<Case>, Box<dyn StdError>> {
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
        if fields.len() != 14 || fields[0].is_empty() || fields[3].is_empty() {
            return Err("diagnostic plan line has invalid fields".into());
        }
        let case = Case {
            id: fields[0].to_owned(),
            page: fields[1].parse()?,
            image: fields[2].parse()?,
            source: PathBuf::from(fields[3]),
            first: parse_pin(&fields[4..9])?,
            second: parse_pin(&fields[9..14])?,
        };
        if case.page == 0 || case.image == 0 || case.first.offset >= case.second.offset {
            return Err("diagnostic plan coordinates or span order are invalid".into());
        }
        cases.push(case);
        if cases.len() > EXPECTED_CASES {
            return Err("diagnostic plan has too many cases".into());
        }
    }
    if cases.len() != EXPECTED_CASES {
        return Err("diagnostic plan does not contain 546 cases".into());
    }
    Ok(cases)
}

/// A second read handle observes the length after the sink flushes each symbol.
/// The ordinary native adapter intentionally snapshots source size at creation.
struct GrowingFileSource(File);

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

fn checked_header(
    source: &mut SeekableSource<File>,
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
    let result = ready(decoder.decode());
    drop(decoder);
    drop(second_sink);
    drop(new_source);
    drop(second_store);
    drop(imported);
    drop(first_store);
    match result {
        Ok(report) => {
            if report.header.new_symbols != case.second.new_symbols
                || report.header.exported_symbols != case.second.exported_symbols
                || report.catalog.new_symbols.len() != case.second.new_symbols as usize
                || report.catalog.exported_symbols.len() != case.second.exported_symbols as usize
                || report.progress.completed_symbols != case.second.new_symbols
            {
                return Err("second dictionary symbol counts differ from pin".into());
            }
            let counts = report.progress.iaai;
            if counts
                .single_reference
                .checked_add(counts.zero)
                .and_then(|value| value.checked_add(counts.aggregation))
                != Some(case.second.new_symbols)
            {
                return Err("second dictionary IAAI counts differ from decoded symbols".into());
            }
            let offset = report.progress.mq.map_or(0, |mq| mq.current_input_offset);
            Ok(CaseOutcome {
                status: "COMPLETE",
                single_reference: counts.single_reference,
                zero: counts.zero,
                aggregation: counts.aggregation,
                refusal: "-",
                offset,
            })
        }
        Err(error) => {
            let kind = match &error.kind {
                RefinementDictionaryErrorKind::Malformed("REFAGGNINST zero")
                    if error.progress.iaai.zero > 0 =>
                {
                    "iaai_zero"
                }
                RefinementDictionaryErrorKind::Unsupported {
                    feature: "REFAGGNINST aggregation",
                    value,
                } if *value > 1 && error.progress.iaai.aggregation > 0 => "iaai_multiple",
                _ => return Err(error.into()),
            };
            let counts = error.progress.iaai;
            Ok(CaseOutcome {
                status: "REFUSED",
                single_reference: counts.single_reference,
                zero: counts.zero,
                aggregation: counts.aggregation,
                refusal: kind,
                offset: error.offset,
            })
        }
    }
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
            "CASE\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            case.id,
            case.page,
            case.image,
            outcome.status,
            outcome.single_reference,
            outcome.zero,
            outcome.aggregation,
            outcome.refusal,
            outcome.offset
        );
    }
    println!("TOTAL\t{EXPECTED_CASES}");
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("jbig2 second dictionary diagnostic: {error}");
        std::process::exit(1);
    }
}
