// SPDX-License-Identifier: MIT

//! Ignored, opt-in CAJ image conformance test.
//!
//! The external T.82 table and document corpus remain outside this repository.
//! A normal test run must report this test as ignored. An explicit run checks
//! every image in the pinned #22 hash catalog and fails on missing inputs.

use caj2pdf_core::jbig1::{Type0Budget, Type0Decoder, Type0Span};
use caj2pdf_core::qm::{ArithmeticBudget, ContextBank, QmState, QmTable};
use caj2pdf_core::{Limits, NeverCancel, SequentialSink, native::SeekableSource};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    env,
    error::Error,
    fs::{File, OpenOptions, remove_file},
    future::Future,
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    pin::pin,
    task::{Context, Poll, Waker},
    time::{SystemTime, UNIX_EPOCH},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const MANIFEST_SHA256: &str = "e88401f0d9cbd08608004c2a9e58577ab85c916b9a9891a4dd5466e346e9203a";
const SAMPLE_IDS_SHA256: &str = "6d9c74816d763bad91f43b0e0ed8b7d6c05261e0997f855408c84ffc62daf8ed";
const STANDARD_SHA256: &str = "11fe241dedbbf4faa542af4a1485566c2794fa69e5c06e2e5c8542adfe9b1ab7";
const EXPECTED_SAMPLES: usize = 27;
const EXPECTED_IMAGES: usize = 1400;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_FIXTURE_BYTES: u64 = 16 * 1024;
const MAX_SOURCE_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_RAW_BYTES: u64 = 128 * 1024 * 1024;
const MAX_ENCODED_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SYMBOLS: u64 = 12_000_000;
const CONTEXT_COUNT: usize = 1024;

struct Image {
    page: u64,
    image: u64,
    offset: u64,
    length: u64,
    encoded_sha256: String,
    width: usize,
    height: usize,
    stride: usize,
    raw_sha256: String,
    visible_sha256: String,
}

struct Sample {
    id: String,
    path: String,
    source_sha256: String,
    images: Vec<Image>,
}

fn run_ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut task = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut task) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("native positioned I/O unexpectedly yielded"),
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").unwrap();
    }
    output
}

fn check_sha(label: &str, actual: &str, expected: &str) -> Result<()> {
    if actual != expected {
        return Err(format!("{label} SHA-256 differs: actual={actual} expected={expected}").into());
    }
    Ok(())
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(format!("{} exceeds its {limit}-byte limit", path.display()).into());
    }
    Ok(bytes)
}

fn hash_span(file: &mut File, offset: u64, len: u64) -> Result<String> {
    let end = offset.checked_add(len).ok_or("hash span overflows")?;
    if end > file.metadata()?.len() {
        return Err("hash span exceeds source size".into());
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut remaining = len;
    let mut buffer = [0_u8; 65_536];
    let mut hash = Sha256::new();
    while remaining != 0 {
        let size = usize::try_from(remaining.min(buffer.len() as u64))?;
        file.read_exact(&mut buffer[..size])?;
        hash.update(&buffer[..size]);
        remaining -= size as u64;
    }
    Ok(hex(&hash.finalize()))
}

fn table_from_external_file(path: &Path) -> Result<QmTable> {
    let bytes = read_bounded(path, MAX_FIXTURE_BYTES)?;
    check_sha(
        "external T.82 fixture",
        &hex(&Sha256::digest(&bytes)),
        STANDARD_SHA256,
    )?;
    let mut lines = std::str::from_utf8(&bytes)?.lines();
    if lines.next() != Some("T82-1993") || lines.next() != Some("113") {
        return Err("external T.82 fixture has an unexpected header".into());
    }
    let mut states = Vec::with_capacity(113);
    for _ in 0..113 {
        let line = lines.next().ok_or("external state table is truncated")?;
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 4 {
            return Err("invalid external state row".into());
        }
        let switch: u8 = fields[3].parse()?;
        if switch > 1 {
            return Err("invalid external switch flag".into());
        }
        states.push(QmState {
            qe: fields[0].parse()?,
            next_lps: fields[1].parse()?,
            next_mps: fields[2].parse()?,
            switch_mps: switch == 1,
        });
    }
    // The fixture hash pins the tail; qm_official_external validates its
    // checkpoints and vector separately.
    if lines.next() != Some("3") || lines.count() != 6 {
        return Err("external T.82 fixture has an unexpected tail".into());
    }
    Ok(QmTable::new(states)?)
}

// This reader supports only the SHA-pinned, one-record-per-line catalog layout.
// It is intentionally not a general JSON parser. Changes require review and a
// new hash, so unrecognized records cannot silently become compatibility PASS.
fn quoted_field(line: &str, key: &str) -> Result<String> {
    let marker = format!("\"{key}\":\"");
    let (_, rest) = line
        .split_once(&marker)
        .ok_or_else(|| format!("missing {key}"))?;
    let (value, _) = rest
        .split_once('"')
        .ok_or_else(|| format!("unterminated {key}"))?;
    if value.contains('\\') {
        return Err(format!("escaped {key} is unsupported").into());
    }
    Ok(value.to_owned())
}

fn number_field(line: &str, key: &str) -> Result<u64> {
    let marker = format!("\"{key}\":");
    let (_, rest) = line
        .split_once(&marker)
        .ok_or_else(|| format!("missing {key}"))?;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return Err(format!("invalid {key}").into());
    }
    Ok(rest[..digits].parse()?)
}

fn manifest() -> Result<Vec<Sample>> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/conformance/jbig1_oracle.json");
    let bytes = read_bounded(&path, MAX_MANIFEST_BYTES)?;
    check_sha(
        "pinned #22 manifest",
        &hex(&Sha256::digest(&bytes)),
        MANIFEST_SHA256,
    )?;
    let mut samples: Vec<Sample> = Vec::new();
    let mut sample_ids = HashSet::new();
    let mut image_keys = HashSet::new();
    let mut ordered_ids_hash = Sha256::new();
    for line in std::str::from_utf8(&bytes)?.lines() {
        if line.starts_with("    {\"id\":") {
            let id = quoted_field(line, "id")?;
            if !sample_ids.insert(id.clone()) {
                return Err(format!("duplicate sample ID {id}").into());
            }
            ordered_ids_hash.update(id.as_bytes());
            ordered_ids_hash.update([0]);
            samples.push(Sample {
                id,
                path: quoted_field(line, "path")?,
                source_sha256: quoted_field(line, "source_sha256")?,
                images: Vec::new(),
            });
        } else if line.starts_with("      {\"page\":") {
            let sample = samples.last_mut().ok_or("image precedes a sample")?;
            let page = number_field(line, "page")?;
            let image = number_field(line, "image")?;
            if !image_keys.insert((sample.id.clone(), page, image)) {
                return Err(format!("duplicate image key {}:{page}:{image}", sample.id).into());
            }
            if quoted_field(line, "decoder_result")? != "PASS" {
                return Err("pinned oracle record is not PASS".into());
            }
            sample.images.push(Image {
                page,
                image,
                offset: number_field(line, "offset")?,
                length: number_field(line, "length")?,
                encoded_sha256: quoted_field(line, "encoded_sha256")?,
                width: usize::try_from(number_field(line, "width")?)?,
                height: usize::try_from(number_field(line, "height")?)?,
                stride: usize::try_from(number_field(line, "stride")?)?,
                raw_sha256: quoted_field(line, "raw_stride_sha256")?,
                visible_sha256: quoted_field(line, "visible_bits_sha256")?,
            });
        }
    }
    check_sha(
        "ordered sample IDs",
        &hex(&ordered_ids_hash.finalize()),
        SAMPLE_IDS_SHA256,
    )?;
    if samples.len() != EXPECTED_SAMPLES || image_keys.len() != EXPECTED_IMAGES {
        return Err(format!(
            "manifest expected {EXPECTED_SAMPLES} samples and {EXPECTED_IMAGES} unique images; got {} and {}",
            samples.len(), image_keys.len()
        ).into());
    }
    Ok(samples)
}

fn safe_source_path(corpus: &Path, path: &str) -> Result<PathBuf> {
    let relative = Path::new(path);
    if relative.components().next().is_none()
        || !relative
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
        || path.contains('\\')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(format!("unsafe corpus path: {path}").into());
    }
    let source = corpus.join(relative).canonicalize()?;
    if !source.starts_with(corpus) || !source.is_file() {
        return Err(format!("corpus path escapes or is not a file: {path}").into());
    }
    Ok(source)
}

// Private, unique spool permits reverse-row hashing with row-sized memory.
// Drop removes it even when decoding or comparing fails.
struct Spool {
    path: PathBuf,
    file: Option<File>,
}

impl Spool {
    fn new(index: usize) -> Result<Self> {
        let dir = env::var_os("CAJ2PDF_T82_SPOOL_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(env::temp_dir);
        if !dir.is_dir() {
            return Err(format!("spool directory is missing: {}", dir.display()).into());
        }
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        for attempt in 0..100_u32 {
            let path = dir.join(format!(
                "caj2pdf-t82-{}-{stamp}-{index}-{attempt}.raw",
                std::process::id()
            ));
            let mut options = OpenOptions::new();
            options.create_new(true).read(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                options.mode(0o600);
            }
            match options.open(&path) {
                Ok(file) => {
                    return Ok(Self {
                        path,
                        file: Some(file),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err("could not create a unique spool file".into())
    }

    fn file(&mut self) -> &mut File {
        self.file.as_mut().expect("spool file is open")
    }
}

impl Drop for Spool {
    fn drop(&mut self) {
        // Close before unlinking so Windows can remove the file as well.
        drop(self.file.take());
        let _ = remove_file(&self.path);
    }
}

impl SequentialSink for Spool {
    async fn write(&mut self, bytes: &[u8]) -> caj2pdf_core::Result<usize> {
        Ok(self.file().write(bytes)?)
    }

    async fn flush(&mut self) -> caj2pdf_core::Result<()> {
        Ok(self.file().flush()?)
    }
}

fn run_image(
    source_path: &Path,
    source_size: u64,
    image: &Image,
    table: &QmTable,
    index: usize,
) -> Result<(String, String, String)> {
    let end = image
        .offset
        .checked_add(image.length)
        .ok_or("encoded span overflows")?;
    if image.length < 48 || image.length > MAX_ENCODED_BYTES || end > source_size {
        return Err("encoded span is outside source or exceeds local bound".into());
    }
    let mut source = File::open(source_path)?;
    let encoded_sha = hash_span(&mut source, image.offset, image.length)?;
    check_sha("encoded image span", &encoded_sha, &image.encoded_sha256)?;

    let (width, height, stride) = (image.width, image.height, image.stride);
    if width == 0 || width > 10_000 || height == 0 || height > 20_000 {
        return Err("image geometry outside local bounds".into());
    }
    let expected_stride = width.checked_add(31).ok_or("stride overflows")? / 32 * 4;
    if stride != expected_stride {
        return Err("manifest stride differs from checked DIB stride".into());
    }
    let raw_bytes = stride.checked_mul(height).ok_or("raw length overflows")?;
    if raw_bytes as u64 > MAX_RAW_BYTES {
        return Err("raw image exceeds local bound".into());
    }
    let max_symbols = u64::try_from(
        width
            .checked_mul(height)
            .and_then(|n| n.checked_add(height))
            .ok_or("symbol count overflows")?,
    )?;
    if max_symbols > MAX_SYMBOLS {
        return Err("symbol budget exceeds local hard cap".into());
    }
    let max_work = max_symbols
        .checked_mul(32)
        .and_then(|n| n.checked_add(1024))
        .ok_or("work budget overflows")?;

    let mut output = Spool::new(index)?;
    let limits = Limits::default();
    let mut contexts = ContextBank::new(CONTEXT_COUNT, &limits)?;
    // Decode from the same open file whose encoded span was just hashed.
    let mut ranged = SeekableSource::new(source)?;
    let mut decoder = run_ready(Type0Decoder::new(
        &mut ranged,
        Type0Span {
            // The pinned #22 manifest contains only catalogued type-0 rows;
            // #28's container parser must supply the actual outer type.
            record_type: 0,
            offset: image.offset,
            length: image.length,
        },
        table,
        &mut contexts,
        &mut output,
        &limits,
        &NeverCancel,
        ArithmeticBudget {
            max_symbols,
            max_work,
        },
        Type0Budget {
            max_width: 10_000,
            max_height: 20_000,
            max_pixels: MAX_SYMBOLS,
            max_context_work: MAX_SYMBOLS * 10 + 20_000,
        },
    ))?;
    let info = decoder.progress().info;
    if (info.width as usize, info.height as usize, info.dib_stride) != (width, height, stride) {
        return Err("decoded DIB dimensions differ from the pinned manifest".into());
    }
    for _ in 0..height {
        if !run_ready(decoder.decode_next_row())? {
            return Err("row decoder ended before the declared DIB height".into());
        }
    }
    let report = run_ready(decoder.finish())?;
    if report.progress.rows_written as usize != height
        || report.progress.output_bytes_written != raw_bytes as u64
    {
        return Err("row decoder output progress differs from DIB geometry".into());
    }

    let mut row = vec![0_u8; stride];
    let visible_bytes = width.checked_add(7).ok_or("visible length overflows")? / 8;
    let last_mask = if width % 8 == 0 {
        0xff
    } else {
        0xff << (8 - width % 8)
    };
    let mut raw_hash = Sha256::new();
    let mut visible_hash = Sha256::new();
    for y in (0..height).rev() {
        let pos = y
            .checked_mul(stride)
            .ok_or("reverse row offset overflows")?;
        output.file().seek(SeekFrom::Start(pos as u64))?;
        output.file().read_exact(&mut row)?;
        raw_hash.update(&row);
        visible_hash.update(&row[..visible_bytes - 1]);
        visible_hash.update([row[visible_bytes - 1] & last_mask]);
    }
    let actual_raw = hex(&raw_hash.finalize());
    let actual_visible = hex(&visible_hash.finalize());
    check_sha("raw stride", &actual_raw, &image.raw_sha256)?;
    check_sha("visible bits", &actual_visible, &image.visible_sha256)?;
    Ok((encoded_sha, actual_raw, actual_visible))
}

#[test]
#[ignore = "requires CAJ2PDF_CORPUS_DIR and CAJ2PDF_T82_VECTOR_FILE external inputs"]
fn all_pinned_caj_images() {
    run_all().unwrap_or_else(|error| panic!("CAJ oracle conformance failed: {error}"));
}

fn run_all() -> Result<()> {
    let corpus = PathBuf::from(
        env::var_os("CAJ2PDF_CORPUS_DIR").ok_or("explicit run requires CAJ2PDF_CORPUS_DIR")?,
    )
    .canonicalize()?;
    if !corpus.is_dir() {
        return Err("CAJ2PDF_CORPUS_DIR is not a directory".into());
    }
    let fixture = PathBuf::from(
        env::var_os("CAJ2PDF_T82_VECTOR_FILE")
            .ok_or("explicit run requires CAJ2PDF_T82_VECTOR_FILE")?,
    );
    let table = table_from_external_file(&fixture)?;
    let samples = manifest()?;
    println!(
        "BATCH_START\texpected={EXPECTED_IMAGES}\tmanifest_sha256={MANIFEST_SHA256}\ttable_sha256={STANDARD_SHA256}"
    );

    let mut index = 0usize;
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut sources_passed = 0usize;
    let mut sources_failed = 0usize;
    for sample in &samples {
        // Hash each whole source once, then verify every image span separately.
        let verified_source = (|| -> Result<(PathBuf, u64)> {
            let path = safe_source_path(&corpus, &sample.path)?;
            let mut file = File::open(&path)?;
            let size = file.metadata()?.len();
            if size > MAX_SOURCE_BYTES {
                return Err("source exceeds 8 GiB local bound".into());
            }
            let actual = hash_span(&mut file, 0, size)?;
            check_sha("source", &actual, &sample.source_sha256)?;
            Ok((path, size))
        })();
        match &verified_source {
            Ok(_) => {
                sources_passed += 1;
                println!(
                    "SOURCE\t{}\tPASS\tsha256={}",
                    sample.id, sample.source_sha256
                );
            }
            Err(error) => {
                sources_failed += 1;
                println!("SOURCE\t{}\tFAIL\t{error}", sample.id);
            }
        }
        for image in &sample.images {
            let result = match &verified_source {
                Ok((path, size)) => run_image(path, *size, image, &table, index),
                Err(error) => Err(format!("source verification failed: {error}").into()),
            };
            match result {
                Ok((encoded, raw, visible)) => {
                    passed += 1;
                    println!(
                        "IMAGE\t{index}\t{}\t{}\t{}\tPASS\tencoded={encoded}\traw={raw}\tvisible={visible}",
                        sample.id, image.page, image.image
                    );
                }
                Err(error) => {
                    failed += 1;
                    println!(
                        "IMAGE\t{index}\t{}\t{}\t{}\tFAIL\t{error}",
                        sample.id, image.page, image.image
                    );
                }
            }
            index += 1;
        }
    }
    println!(
        "BATCH_SUMMARY\texpected={EXPECTED_IMAGES}\tprocessed={index}\tpass={passed}\tfail={failed}\tsources_pass={sources_passed}\tsources_fail={sources_failed}"
    );
    if index != EXPECTED_IMAGES
        || failed != 0
        || sources_passed != EXPECTED_SAMPLES
        || sources_failed != 0
    {
        return Err("batch did not pass all 1,400 pinned images and 27 sources".into());
    }
    Ok(())
}
