// SPDX-License-Identifier: MIT

//! Optional, external-only metrics for one SHA-pinned large #1 dictionary.
//! No official table states or CAJ bytes are embedded or distributed.

use caj2pdf_core::{
    Limits, NeverCancel,
    jbig2::{
        HeaderLimits, SegmentSpan,
        dictionary::{DictionaryBudget, DirectDictionaryDecoder},
        integer::IntegerContextBanks,
        mq::{MQ_STATE_COUNT, MqBudget, MqState, MqTable},
        read_segment_header,
    },
    native::{SeekableSource, WriteSink},
};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::{
    env,
    error::Error as StdError,
    fs::{self, File, OpenOptions},
    future::Future,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    pin::pin,
    task::{Context, Poll, Waker},
    time::{SystemTime, UNIX_EPOCH},
};

const SAMPLE: &str = "pull-72/碳_碳复合材料多重环境下的氧化机理研究_李龙.caj";
const SOURCE_SIZE: u64 = 18_200_390;
const SOURCE_SHA: &str = "01558ff7e30c3131c79fc3267e6eb99c055cc9cb8e1b0e710bc6bd0791c888ab";
const DATA_OFFSET: u64 = 13_229_976;
const DATA_LENGTH: u64 = 9_959;
const DATA_SHA: &str = "48312efc04ca4b43fed872f3e4de9108a08b74ccf339545a40860fa121133baa";
const HEADER_OFFSET: u64 = DATA_OFFSET - 11;
const TABLE_SHA: &str = "bdf6eeeca3bc5d5a8dc1a13acc7698ec356c886b27f6526f3e09fc2c8520ac57";

fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("native file unexpectedly yielded"),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn digest_file(path: &Path, range: Option<(u64, u64)>) -> Result<String, Box<dyn StdError>> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    let (start, count) = range.unwrap_or((0, length));
    if start.checked_add(count).is_none_or(|end| end > length) {
        return Err("digest range escapes source".into());
    }
    file.seek(SeekFrom::Start(start))?;
    let mut remaining = count;
    let mut buffer = [0u8; 64 * 1024];
    let mut hash = Sha256::new();
    while remaining > 0 {
        let requested = remaining.min(buffer.len() as u64) as usize;
        let got = file.read(&mut buffer[..requested])?;
        if got == 0 {
            return Err("source shortened while hashing".into());
        }
        hash.update(&buffer[..got]);
        remaining -= got as u64;
    }
    Ok(hex(&hash.finalize()))
}

fn table(path: &Path, limits: &Limits) -> Result<MqTable, Box<dyn StdError>> {
    let canonical = path.canonicalize()?;
    if !canonical.starts_with("/tmp") {
        return Err("private table fixture must stay in /tmp".into());
    }
    let mut bytes = Vec::new();
    File::open(&canonical)?
        .take(16 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 16 * 1024 || hex(&Sha256::digest(&bytes)) != TABLE_SHA {
        return Err("private table fixture size or SHA differs".into());
    }
    let text = std::str::from_utf8(&bytes)?;
    let mut lines = text.lines();
    if lines.next() != Some("T88-2000-H2") || lines.next() != Some("47") {
        return Err("private table framing differs".into());
    }
    let mut states = Vec::new();
    states.try_reserve_exact(MQ_STATE_COUNT)?;
    for _ in 0..MQ_STATE_COUNT {
        let row = lines.next().ok_or("missing table row")?;
        let parts: Vec<_> = row.split_whitespace().collect();
        if parts.len() != 4 {
            return Err("table row width differs".into());
        }
        let switch: u8 = parts[3].parse()?;
        if switch > 1 {
            return Err("table switch differs".into());
        }
        states.push(MqState {
            qe: parts[0].parse()?,
            next_mps: parts[1].parse()?,
            next_lps: parts[2].parse()?,
            switch_mps: switch == 1,
        });
    }
    MqTable::new(states, limits).map_err(Into::into)
}

struct TempStore(PathBuf);

impl TempStore {
    fn create() -> Result<(Self, File), Box<dyn StdError>> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = env::temp_dir().join(format!(
            "caj2pdf-dictionary-metrics-{}-{nonce}.bin",
            std::process::id()
        ));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok((Self(path), file))
    }
}

impl Drop for TempStore {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn peak_rss_kib() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find(|line| line.starts_with("VmHWM:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

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
    let (temporary, file) = TempStore::create()?;
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
    #[cfg(unix)]
    let allocated = metadata.blocks().saturating_mul(512);
    #[cfg(not(unix))]
    let allocated = 0;
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
