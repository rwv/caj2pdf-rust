// SPDX-License-Identifier: MIT

//! Native, external-only batch probe for the type-38 region slice. The Python
//! driver supplies a SHA-checked metadata plan; no corpus or T.88 table is
//! built into this program. Output is a small, line-oriented metrics stream.

use caj2pdf_core::{
    Error, Limits, NeverCancel, RangedSource, SequentialSink,
    jbig2::{
        HeaderLimits, SegmentSpan,
        generic::{GenericBudget, GenericRegionDecoder},
        mq::{MQ_STATE_COUNT, MqBudget, MqContexts, MqState, MqTable},
        read_segment_header,
    },
    native::SeekableSource,
};
use sha2::{Digest, Sha256};
use std::{
    env,
    error::Error as StdError,
    fs::{self, File},
    future::Future,
    io::{BufRead, BufReader, Read},
    path::Path,
    pin::pin,
    task::{Context, Poll, Waker},
    time::Instant,
};

const FIXTURE_SHA: &str = "bdf6eeeca3bc5d5a8dc1a13acc7698ec356c886b27f6526f3e09fc2c8520ac57";
const MAX_PLAN_BYTES: u64 = 2 * 1024 * 1024;
const MAX_FIXTURE_BYTES: u64 = 16 * 1024;
const MAX_SEGMENT_BYTES: u64 = 64 * 1024 * 1024;
const EXPECTED_CASES: usize = 546;

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
fn digest_file(path: &Path, max: u64) -> Result<(String, u64), Box<dyn StdError>> {
    let size = fs::metadata(path)?.len();
    if size > max {
        return Err(format!("file exceeds {max}-byte cap").into());
    }
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut read_total = 0u64;
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        read_total = read_total
            .checked_add(count as u64)
            .ok_or("hash input counter overflows")?;
    }
    if read_total != size {
        return Err("file changed while hashing".into());
    }
    Ok((hex(&digest.finalize()), size))
}
fn digest_span(path: &Path, span: SegmentSpan) -> Result<String, Box<dyn StdError>> {
    if span.length == 0 || span.length > MAX_SEGMENT_BYTES {
        return Err("selected segment length exceeds cap".into());
    }
    let mut source = SeekableSource::new(File::open(path)?)?;
    let end = span
        .offset
        .checked_add(span.length)
        .ok_or("selected segment end overflows")?;
    if end > source.size() {
        return Err("selected segment escapes source".into());
    }
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut done = 0u64;
    while done < span.length {
        let count = (span.length - done).min(buffer.len() as u64) as usize;
        let got = ready(source.read_at(span.offset + done, &mut buffer[..count]))?;
        if got == 0 || got > count {
            return Err("selected source span changed or stalled".into());
        }
        digest.update(&buffer[..got]);
        done += got as u64;
    }
    Ok(hex(&digest.finalize()))
}
fn table(path: &Path, limits: &Limits) -> Result<MqTable, Box<dyn StdError>> {
    let canonical = path.canonicalize()?;
    if !canonical.starts_with("/tmp") {
        return Err("private official table fixture must stay in /tmp".into());
    }
    let (sha, size) = digest_file(&canonical, MAX_FIXTURE_BYTES)?;
    if sha != FIXTURE_SHA {
        return Err("private official table fixture SHA differs".into());
    }
    let text = fs::read_to_string(&canonical)?;
    if text.len() as u64 != size {
        return Err("private fixture changed while parsing".into());
    }
    let mut lines = text.lines();
    if lines.next() != Some("T88-2000-H2") || lines.next() != Some("47") {
        return Err("private fixture framing differs".into());
    }
    let mut states = Vec::new();
    states.try_reserve_exact(MQ_STATE_COUNT)?;
    for _ in 0..MQ_STATE_COUNT {
        let row = lines.next().ok_or("missing private table row")?;
        let fields: Vec<_> = row.split_whitespace().collect();
        if fields.len() != 4 {
            return Err("private table row has wrong width".into());
        }
        let switch: u8 = fields[3].parse()?;
        if switch > 1 {
            return Err("private table switch is not boolean".into());
        }
        states.push(MqState {
            qe: fields[0].parse()?,
            next_mps: fields[1].parse()?,
            next_lps: fields[2].parse()?,
            switch_mps: switch == 1,
        });
    }
    MqTable::new(states, limits).map_err(Into::into)
}

struct CountingSource<S> {
    inner: S,
    calls: u64,
    bytes: u64,
    max_request: usize,
}
impl<S: RangedSource> RangedSource for CountingSource<S> {
    fn size(&self) -> u64 {
        self.inner.size()
    }
    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        self.calls = self.calls.checked_add(1).ok_or(Error::InvalidInput {
            reason: "source read count overflows",
        })?;
        self.max_request = self.max_request.max(destination.len());
        let got = self.inner.read_at(offset, destination).await?;
        if got <= destination.len() {
            self.bytes = self
                .bytes
                .checked_add(got as u64)
                .ok_or(Error::InvalidInput {
                    reason: "source byte count overflows",
                })?;
        }
        Ok(got)
    }
}
struct RowHashSink {
    hash: Sha256,
    black: u64,
    bytes: u64,
    rows: u32,
    stride: usize,
    height: u32,
    padding_mask: u8,
    max_write: usize,
    flushed: bool,
}
impl RowHashSink {
    fn new(width: u32, height: u32) -> Self {
        let stride = width.div_ceil(8) as usize;
        let padding_mask = if width % 8 == 0 {
            0
        } else {
            (1u8 << (8 - width % 8)) - 1
        };
        Self {
            hash: Sha256::new(),
            black: 0,
            bytes: 0,
            rows: 0,
            stride,
            height,
            padding_mask,
            max_write: 0,
            flushed: false,
        }
    }
}
impl SequentialSink for RowHashSink {
    async fn write(&mut self, row: &[u8]) -> caj2pdf_core::Result<usize> {
        if row.len() != self.stride
            || self.rows >= self.height
            || row.last().is_some_and(|last| last & self.padding_mask != 0)
        {
            return Err(Error::InvalidInput {
                reason: "generic output row shape or padding differs",
            });
        }
        self.hash.update(row);
        self.black = self
            .black
            .checked_add(
                row.iter()
                    .map(|byte| u64::from(byte.count_ones()))
                    .sum::<u64>(),
            )
            .ok_or(Error::InvalidInput {
                reason: "black pixel count overflows",
            })?;
        self.bytes = self
            .bytes
            .checked_add(row.len() as u64)
            .ok_or(Error::InvalidInput {
                reason: "output byte count overflows",
            })?;
        self.rows += 1;
        self.max_write = self.max_write.max(row.len());
        Ok(row.len())
    }
    async fn flush(&mut self) -> caj2pdf_core::Result<()> {
        self.flushed = true;
        Ok(())
    }
}

struct Case<'a> {
    id: &'a str,
    page: u32,
    image: u32,
    path: &'a Path,
    page_span: SegmentSpan,
    page_sha: &'a str,
    generic_span: SegmentSpan,
    generic_sha: &'a str,
    width: u32,
    height: u32,
    pixel_sha: &'a str,
    black: u64,
}
fn parse_case(line: &str) -> Result<Case<'_>, Box<dyn StdError>> {
    let fields: Vec<_> = line.split('\t').collect();
    if fields.len() != 14 {
        return Err("plan row must have 14 tab-separated fields".into());
    }
    if fields
        .iter()
        .any(|field| field.is_empty() || field.contains('\r') || field.contains('\n'))
    {
        return Err("plan row has empty or control field".into());
    }
    for index in [6, 9, 12] {
        if fields[index].len() != 64
            || !fields[index]
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err("plan digest must be lowercase SHA-256".into());
        }
    }
    Ok(Case {
        id: fields[0],
        page: fields[1].parse()?,
        image: fields[2].parse()?,
        path: Path::new(fields[3]),
        page_span: SegmentSpan {
            offset: fields[4].parse()?,
            length: fields[5].parse()?,
        },
        page_sha: fields[6],
        generic_span: SegmentSpan {
            offset: fields[7].parse()?,
            length: fields[8].parse()?,
        },
        generic_sha: fields[9],
        width: fields[10].parse()?,
        height: fields[11].parse()?,
        pixel_sha: fields[12],
        black: fields[13].parse()?,
    })
}
fn peak_rss_kib() -> Option<u64> {
    let text = fs::read_to_string("/proc/self/status").ok()?;
    let row = text.lines().find(|line| line.starts_with("VmHWM:"))?;
    row.split_whitespace().nth(1)?.parse().ok()
}
fn decode(
    case: Case<'_>,
    table: &MqTable,
    limits: &Limits,
    mq_budget: MqBudget,
) -> Result<String, Box<dyn StdError>> {
    let label = format!(
        "{} page {} image {} segment #0 {}+{} segment #4 {}+{}",
        case.id,
        case.page,
        case.image,
        case.page_span.offset,
        case.page_span.length,
        case.generic_span.offset,
        case.generic_span.length
    );
    let started = Instant::now();
    if case.width == 0 || case.height == 0 {
        return Err(format!("{label}: zero dimensions").into());
    }
    if digest_span(case.path, case.page_span)? != case.page_sha
        || digest_span(case.path, case.generic_span)? != case.generic_sha
    {
        return Err(
            format!("{label}: selected #0/#4 encoded SHA differs before Rust decode").into(),
        );
    }
    let mut source = CountingSource {
        inner: SeekableSource::new(File::open(case.path)?)?,
        calls: 0,
        bytes: 0,
        max_request: 0,
    };
    let page_header = ready(read_segment_header(
        &mut source,
        case.page_span,
        limits,
        HeaderLimits::default(),
        &NeverCancel,
    ))
    .map_err(|e| format!("{label}: page-info header: {e}"))?;
    if page_header.number != 0
        || page_header.segment_type != 48
        || !page_header.referred_to.is_empty()
    {
        return Err(format!("{label}: page-info header profile differs").into());
    }
    let header = ready(read_segment_header(
        &mut source,
        case.generic_span,
        limits,
        HeaderLimits::default(),
        &NeverCancel,
    ))
    .map_err(|e| format!("{label}: generic header: {e}"))?;
    if header.number != 4 || header.segment_type != 38 || !header.referred_to.is_empty() {
        return Err(format!("{label}: generic header profile differs").into());
    }
    let mut contexts = MqContexts::new(1024, limits, &mq_budget)?;
    let mut sink = RowHashSink::new(case.width, case.height);
    let mut decoder = ready(GenericRegionDecoder::new(
        &mut source,
        &header,
        table,
        &mut contexts,
        &mut sink,
        limits,
        &NeverCancel,
        mq_budget,
        GenericBudget::default(),
    ))
    .map_err(|e| format!("{label}: {e}"))?;
    let info = decoder.progress().info;
    if (info.width, info.height) != (case.width, case.height) {
        return Err(format!("{label}: decoded dimensions differ").into());
    }
    for _ in 0..case.height {
        ready(decoder.decode_next_row()).map_err(|e| format!("{label}: {e}"))?;
    }
    if ready(decoder.decode_next_row()).map_err(|e| format!("{label}: {e}"))? {
        return Err(format!("{label}: extra generic row").into());
    }
    let report = ready(decoder.finish()).map_err(|e| format!("{label}: {e}"))?;
    let digest = hex(&sink.hash.finalize());
    let expected_bytes = u64::from(case.width.div_ceil(8)) * u64::from(case.height);
    if report.progress.rows_written != case.height
        || report.progress.pixels_decoded != u64::from(case.width) * u64::from(case.height)
        || report.progress.output_bytes_written != expected_bytes
        || sink.rows != case.height
        || sink.bytes != expected_bytes
        || !sink.flushed
    {
        return Err(format!("{label}: row/symbol/output progress differs").into());
    }
    if digest != case.pixel_sha || sink.black != case.black {
        return Err(format!(
            "{label}: Rust generic pixel mismatch: SHA {digest} expected {}, black {} expected {}",
            case.pixel_sha, sink.black, case.black
        )
        .into());
    }
    if digest_span(case.path, case.page_span)? != case.page_sha
        || digest_span(case.path, case.generic_span)? != case.generic_sha
    {
        return Err(
            format!("{label}: selected #0/#4 encoded SHA differs after Rust decode").into(),
        );
    }
    Ok([
        "CASE".to_owned(),
        case.id.to_owned(),
        case.page.to_string(),
        case.image.to_string(),
        case.width.to_string(),
        case.height.to_string(),
        digest,
        sink.black.to_string(),
        sink.rows.to_string(),
        sink.stride.to_string(),
        report.progress.pixels_decoded.to_string(),
        source.calls.to_string(),
        source.bytes.to_string(),
        source.max_request.to_string(),
        sink.max_write.to_string(),
        report.progress.mq.source_bytes_fetched.to_string(),
        started.elapsed().as_millis().to_string(),
    ]
    .join("\t"))
}
fn run() -> Result<(), Box<dyn StdError>> {
    let mut args = env::args_os();
    let _program = args.next();
    let fixture = args
        .next()
        .ok_or("usage: jbig2_generic_parity TABLE_FIXTURE PLAN")?;
    let plan = args
        .next()
        .ok_or("usage: jbig2_generic_parity TABLE_FIXTURE PLAN")?;
    if args.next().is_some() {
        return Err("usage: jbig2_generic_parity TABLE_FIXTURE PLAN".into());
    }
    let limits = Limits {
        io_chunk_bytes: 64 * 1024,
        ..Limits::default()
    };
    let budget = MqBudget::default();
    let table = table(Path::new(&fixture), &limits)?;
    let plan_path = Path::new(&plan);
    if fs::metadata(plan_path)?.len() > MAX_PLAN_BYTES {
        return Err("parity plan exceeds 2 MiB".into());
    }
    let file = BufReader::new(File::open(plan_path)?);
    let mut count = 0usize;
    let began = Instant::now();
    for line in file.lines() {
        let line = line?;
        let case = parse_case(&line)?;
        let result = decode(case, &table, &limits, budget)?;
        println!("{result}");
        count += 1;
        if count > EXPECTED_CASES {
            return Err("parity plan has extra images".into());
        }
    }
    if count != EXPECTED_CASES {
        return Err(format!("parity plan has {count} images, expected {EXPECTED_CASES}").into());
    }
    println!(
        "TOTAL\t{count}\t{}\t{}",
        began.elapsed().as_millis(),
        peak_rss_kib().unwrap_or(0)
    );
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("JBIG2 generic Rust parity FAIL: {error}");
        std::process::exit(1);
    }
}
