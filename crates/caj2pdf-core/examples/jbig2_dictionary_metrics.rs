// SPDX-License-Identifier: MIT

//! Optional, external-only metrics for one SHA-pinned large #1 dictionary.
//! No official table states or CAJ bytes are embedded or distributed.

#[path = "support/mod.rs"]
#[allow(dead_code)] // The second diagnostic example uses the remaining helpers.
mod support;

use caj2pdf_core::{
    Limits, NeverCancel,
    jbig2::{
        HeaderLimits, SegmentSpan,
        dictionary::{DictionaryBudget, DirectDictionaryDecoder},
        integer::IntegerContextBanks,
        mq::MqBudget,
        read_segment_header,
    },
    native::{SeekableSource, WriteSink},
};
use std::{
    env,
    error::Error as StdError,
    fs::{self, File},
    path::Path,
};
use support::{TempStore, allocated_disk_bytes, digest_file, peak_rss_kib, ready, table};

const SAMPLE: &str = "pull-72/碳_碳复合材料多重环境下的氧化机理研究_李龙.caj";
const SOURCE_SIZE: u64 = 18_200_390;
const SOURCE_SHA: &str = "01558ff7e30c3131c79fc3267e6eb99c055cc9cb8e1b0e710bc6bd0791c888ab";
const DATA_OFFSET: u64 = 13_229_976;
const DATA_LENGTH: u64 = 9_959;
const DATA_SHA: &str = "48312efc04ca4b43fed872f3e4de9108a08b74ccf339545a40860fa121133baa";
const HEADER_OFFSET: u64 = DATA_OFFSET - 11;
fn run() -> Result<(), Box<dyn StdError>> {
    let mut arguments = env::args_os();
    let _ = arguments.next();
    let corpus = arguments
        .next()
        .ok_or("usage: jbig2_dictionary_metrics CORPUS_DIR PRIVATE_TABLE")?;
    let fixture = arguments
        .next()
        .ok_or("usage: jbig2_dictionary_metrics CORPUS_DIR PRIVATE_TABLE")?;
    if arguments.next().is_some() {
        return Err("usage: jbig2_dictionary_metrics CORPUS_DIR PRIVATE_TABLE".into());
    }
    let source_path = Path::new(&corpus).join(SAMPLE);
    if fs::metadata(&source_path)?.len() != SOURCE_SIZE
        || digest_file(&source_path, None)? != SOURCE_SHA
        || digest_file(&source_path, Some((DATA_OFFSET, DATA_LENGTH)))? != DATA_SHA
    {
        return Err("source identity or selected dictionary span differs".into());
    }
    let limits = Limits::default();
    let mq_budget = MqBudget::default();
    let table = table(Path::new(&fixture), &limits)?;
    let mut source = SeekableSource::new(File::open(&source_path)?)?;
    let header = ready(read_segment_header(
        &mut source,
        SegmentSpan {
            offset: HEADER_OFFSET,
            length: DATA_LENGTH + 11,
        },
        &limits,
        HeaderLimits::default(),
        &NeverCancel,
    ))?;
    if header.number != 1
        || header.segment_type != 0
        || header.data.offset != DATA_OFFSET
        || header.data.length != DATA_LENGTH
    {
        return Err("selected #1 segment header differs".into());
    }
    let (temporary, file) = TempStore::create("caj2pdf-dictionary-metrics")?;
    let mut sink = WriteSink::new(file);
    let mut banks = IntegerContextBanks::with_extra_contexts(1024, &limits, &mq_budget)?;
    let mut decoder = ready(DirectDictionaryDecoder::new(
        &mut source,
        &header,
        &table,
        &mut banks,
        &mut sink,
        &limits,
        &NeverCancel,
        mq_budget,
        DictionaryBudget::default(),
    ))?;
    let report = ready(decoder.decode())?;
    drop(decoder);
    let file = sink.into_inner();
    let metadata = file.metadata()?;
    let stored = metadata.len();
    let allocated = allocated_disk_bytes(&metadata);
    drop(file);
    if report.catalog.new_symbols.len() != 310
        || report.catalog.exported_symbols.len() != 310
        || report.catalog.new_symbols != report.catalog.exported_symbols
        || report.progress.stored_bitmap_bytes != stored
    {
        return Err("dictionary counts or store length differ from pinned metadata".into());
    }
    let mut expected_offset = 0u64;
    for descriptor in &report.catalog.new_symbols {
        if descriptor.relative_store_offset != expected_offset
            || descriptor.stored_bytes
                != u64::from(descriptor.row_stride) * u64::from(descriptor.height)
        {
            return Err("noncontiguous or malformed packed symbol descriptor".into());
        }
        expected_offset = expected_offset
            .checked_add(descriptor.stored_bytes)
            .ok_or("symbol store offset overflows")?;
    }
    if expected_offset != stored {
        return Err("symbol descriptors do not span temporary output".into());
    }
    if digest_file(&source_path, None)? != SOURCE_SHA
        || digest_file(&source_path, Some((DATA_OFFSET, DATA_LENGTH)))? != DATA_SHA
    {
        return Err("source mutated after dictionary decode".into());
    }
    println!(
        "sample=pull-72 page=129 image=1 segment=1 new={} exported={} pixels={} temporary_bytes={} allocated_disk_bytes={} peak_rss_kib={} mq_decisions={} source_fetched={}",
        report.catalog.new_symbols.len(),
        report.catalog.exported_symbols.len(),
        report.progress.decoded_pixels,
        stored,
        allocated,
        peak_rss_kib().unwrap_or(0),
        report.progress.mq.map_or(0, |mq| mq.symbols_decoded),
        report.progress.source_bytes_fetched(),
    );
    drop(temporary);
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("jbig2 dictionary metrics: {error}");
        std::process::exit(1);
    }
}
