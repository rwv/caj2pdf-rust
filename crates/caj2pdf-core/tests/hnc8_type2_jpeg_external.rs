// SPDX-License-Identifier: MIT

//! Opt-in, hash-pinned inventory of the external HN/C8 type-2 JPEG corpus.
//! Source paths come from the pre-existing #22 catalog; this inventory adds
//! only source indices, coordinates, SHA-256 values, and header metadata.
//! A normal test run reads no private document and reports zero matches.

use caj2pdf_core::{
    Limits, NeverCancel, RangedSource,
    hnc8::{
        Budget, ErrorKind, Hnc8Error, Hnc8Reader, JpegBudget, JpegColor, Variant,
        read_type2_jpeg_info,
    },
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    env,
    error::Error,
    fs::{self, File, OpenOptions},
    future::Future,
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    pin::pin,
    sync::atomic::{AtomicU64, Ordering},
    task::{Context, Poll, Waker},
};

type TestResult<T> = Result<T, Box<dyn Error>>;
const CATALOG_SHA256: &str = "e88401f0d9cbd08608004c2a9e58577ab85c916b9a9891a4dd5466e346e9203a";
const MATRIX_SHA256: &str = "af42132133911f3597eed9318f613494c4047353cb7e15fc4d1008cf59ef44a9";
const SOURCE_IDS_SHA256: &str = "6d9c74816d763bad91f43b0e0ed8b7d6c05261e0997f855408c84ffc62daf8ed";
const INVENTORY_SHA256: &str = "f582ffeb068eb32f7c6bcb0619a3bbd567008739a7f6b719b7c6e9eee97db0ac";
const SOURCES: usize = 27;
const TYPE2_IMAGES: usize = 1_085;
const OTHER_TYPES: [usize; 3] = [1_400, 6, 546];
const EXPECTED_ANOMALIES: usize = 3;
const IO_CHUNK: usize = 65_536;
const MAX_CATALOG: u64 = 1_048_576;
const MAX_MATRIX: u64 = 1_048_576;
const MAX_INVENTORY: u64 = 262_144;
const MAX_SOURCE: u64 = 8 * 1024 * 1024 * 1024;
const MAX_JPEG: u64 = 64 * 1024 * 1024;
static NEXT_TEST_SOURCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
struct Image {
    page: u32,
    number: u32,
    descriptor: u64,
    offset: u64,
    length: u64,
    sha: String,
    width: u16,
    height: u16,
    precision: u8,
    components: u8,
    jfif: bool,
    scans: u32,
}

#[derive(Debug)]
struct Sample {
    path: String,
    size: u64,
    sha: String,
    images: Vec<Image>,
}

fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("native file adapter unexpectedly yielded"),
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

fn check_hash(label: &str, actual: &str, expected: &str) -> TestResult<()> {
    if actual != expected {
        return Err(format!("{label} SHA-256 mismatch").into());
    }
    Ok(())
}

fn bounded_read(path: &Path, limit: u64) -> TestResult<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(format!("pinned metadata exceeds {limit}-byte bound").into());
    }
    Ok(bytes)
}

fn hash_span(file: &mut File, offset: u64, length: u64) -> TestResult<String> {
    let end = offset.checked_add(length).ok_or("source span overflows")?;
    if end > file.metadata()?.len() {
        return Err("source span exceeds current file size".into());
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut remaining = length;
    let mut buffer = [0_u8; IO_CHUNK];
    let mut hash = Sha256::new();
    while remaining != 0 {
        let n = usize::try_from(remaining.min(IO_CHUNK as u64))?;
        file.read_exact(&mut buffer[..n])?;
        hash.update(&buffer[..n]);
        remaining -= n as u64;
    }
    Ok(hex(&hash.finalize()))
}

fn verify_source(path: &Path, size: u64, sha: &str) -> TestResult<()> {
    let mut file = File::open(path)?;
    if file.metadata()?.len() != size {
        return Err("source size differs from the pinned identity".into());
    }
    check_hash("source", &hash_span(&mut file, 0, size)?, sha)
}

fn quoted(line: &str, key: &str) -> TestResult<String> {
    let marker = format!("\"{key}\":\"");
    let (_, rest) = line
        .split_once(&marker)
        .ok_or_else(|| format!("#22 catalog lacks {key}"))?;
    let (value, _) = rest
        .split_once('"')
        .ok_or_else(|| format!("#22 catalog has unterminated {key}"))?;
    if value.contains('\\') {
        return Err("escaped catalog identity is unsupported".into());
    }
    Ok(value.to_owned())
}

fn pinned_metadata(name: &str, limit: u64, sha: &str) -> TestResult<Vec<u8>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/conformance")
        .join(name);
    let bytes = bounded_read(&path, limit)?;
    check_hash(name, &hex(&Sha256::digest(&bytes)), sha)?;
    Ok(bytes)
}

fn parse_u64(text: &str) -> TestResult<u64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("invalid inventory integer".into());
    }
    Ok(text.parse()?)
}

fn parse_sha(text: &str) -> TestResult<String> {
    if text.len() != 64
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("invalid inventory SHA-256".into());
    }
    Ok(text.to_owned())
}

fn catalog() -> TestResult<Vec<Sample>> {
    pinned_metadata("matrix.json", MAX_MATRIX, MATRIX_SHA256)?;
    let bytes = pinned_metadata("jbig1_oracle.json", MAX_CATALOG, CATALOG_SHA256)?;
    let mut samples = Vec::new();
    let mut ids = HashSet::new();
    let mut id_hash = Sha256::new();
    for line in std::str::from_utf8(&bytes)?.lines() {
        if !line.starts_with("    {\"id\":") {
            continue;
        }
        let id = quoted(line, "id")?;
        if !ids.insert(id.clone()) {
            return Err("duplicate #22 source identity".into());
        }
        id_hash.update(id.as_bytes());
        id_hash.update([0]);
        samples.push(Sample {
            path: quoted(line, "path")?,
            size: 0,
            sha: quoted(line, "source_sha256")?,
            images: Vec::new(),
        });
    }
    if samples.len() != SOURCES {
        return Err("#22 catalog source count differs from 27".into());
    }
    check_hash(
        "ordered #22 source IDs",
        &hex(&id_hash.finalize()),
        SOURCE_IDS_SHA256,
    )?;

    let bytes = pinned_metadata(
        "hnc8_type2_jpeg_inventory.tsv",
        MAX_INVENTORY,
        INVENTORY_SHA256,
    )?;
    let mut source_rows = 0;
    let mut image_rows = 0;
    let mut image_keys = HashSet::new();
    for line in std::str::from_utf8(&bytes)?.lines() {
        if line.starts_with('#') {
            continue;
        }
        let fields: Vec<_> = line.split('\t').collect();
        match fields.as_slice() {
            ["S", index, size, sha] => {
                let index = usize::try_from(parse_u64(index)?)?;
                if index != source_rows || index >= samples.len() {
                    return Err("inventory source order differs from #22".into());
                }
                let sha = parse_sha(sha)?;
                check_hash("inventory source identity", &sha, &samples[index].sha)?;
                samples[index].size = parse_u64(size)?;
                if samples[index].size > MAX_SOURCE {
                    return Err("pinned source exceeds local bound".into());
                }
                source_rows += 1;
            }
            [
                "I",
                index,
                page,
                number,
                descriptor,
                offset,
                length,
                sha,
                width,
                height,
                precision,
                components,
                jfif,
                app14_count,
                scans,
            ] => {
                let index = usize::try_from(parse_u64(index)?)?;
                if source_rows == 0 || index != source_rows - 1 {
                    return Err("inventory image is outside its source group".into());
                }
                let page = u32::try_from(parse_u64(page)?)?;
                let number = u32::try_from(parse_u64(number)?)?;
                if page == 0 || number == 0 || !image_keys.insert((index, page, number)) {
                    return Err("invalid or duplicate inventory image identity".into());
                }
                let descriptor = parse_u64(descriptor)?;
                let offset = parse_u64(offset)?;
                let length = parse_u64(length)?;
                if descriptor.checked_add(12).is_none_or(|end| end > offset)
                    || length == 0
                    || length > MAX_JPEG
                    || offset
                        .checked_add(length)
                        .is_none_or(|end| end > samples[index].size)
                {
                    return Err("inventory image span is invalid".into());
                }
                if *app14_count != "0" {
                    return Err("pinned corpus unexpectedly declares JPEG APP14".into());
                }
                let jfif = match *jfif {
                    "0" => false,
                    "1" => true,
                    _ => return Err("invalid inventory JFIF flag".into()),
                };
                samples[index].images.push(Image {
                    page,
                    number,
                    descriptor,
                    offset,
                    length,
                    sha: parse_sha(sha)?,
                    width: u16::try_from(parse_u64(width)?)?,
                    height: u16::try_from(parse_u64(height)?)?,
                    precision: u8::try_from(parse_u64(precision)?)?,
                    components: u8::try_from(parse_u64(components)?)?,
                    jfif,
                    scans: u32::try_from(parse_u64(scans)?)?,
                });
                image_rows += 1;
            }
            _ => return Err("inventory row layout is invalid".into()),
        }
    }
    if source_rows != SOURCES || image_rows != TYPE2_IMAGES {
        return Err("inventory source or type-2 count differs from pinned totals".into());
    }
    Ok(samples)
}

fn source_path(root: &Path, relative: &str) -> TestResult<PathBuf> {
    let path = Path::new(relative);
    if path.components().next().is_none()
        || !path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
        || relative.contains('\\')
        || relative
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err("unsafe #22 catalog source path".into());
    }
    let source = root.join(path).canonicalize()?;
    if !source.starts_with(root) || !source.is_file() {
        return Err("source path escapes corpus or is not a file".into());
    }
    Ok(source)
}

struct CountedSource {
    file: File,
    size: u64,
    largest_request: usize,
    bytes_read: u64,
}

impl CountedSource {
    fn open(path: &Path, size: u64) -> TestResult<Self> {
        Ok(Self {
            file: File::open(path)?,
            size,
            largest_request: 0,
            bytes_read: 0,
        })
    }
}

impl RangedSource for CountedSource {
    fn size(&self) -> u64 {
        self.size
    }

    async fn read_at(
        &mut self,
        offset: u64,
        destination: &mut [u8],
    ) -> caj2pdf_core::Result<usize> {
        if destination.len() > IO_CHUNK {
            return Err(caj2pdf_core::Error::LimitExceeded {
                resource: "corpus source request bytes",
                limit: IO_CHUNK as u64,
                attempted: destination.len() as u64,
            });
        }
        self.largest_request = self.largest_request.max(destination.len());
        self.file.seek(SeekFrom::Start(offset))?;
        let n = self.file.read(destination)?;
        self.bytes_read += n as u64;
        Ok(n)
    }
}

fn expected_anomaly(index: usize, error: &Hnc8Error) -> bool {
    if index != 1 {
        return false;
    }
    let (page, image, offset, field, kind) = match (error.page, error.image) {
        (Some(2), Some(1)) => (2, Some(1), 12_886, "image type", "unsupported"),
        (Some(3), None) => (3, None, 264, "image count", "malformed"),
        (Some(4), None) => (4, None, 276, "text span", "truncated"),
        _ => return false,
    };
    error.page == Some(page)
        && error.image == image
        && error.offset == offset
        && error.kind.field() == field
        && error.kind.as_str() == kind
}

fn peak_rss_kib() -> TestResult<u64> {
    #[cfg(target_os = "linux")]
    {
        let status = fs::read_to_string("/proc/self/status")?;
        let value = status
            .lines()
            .find(|line| line.starts_with("VmHWM:"))
            .and_then(|line| line.split_whitespace().nth(1))
            .ok_or("Linux VmHWM is absent")?;
        Ok(value.parse()?)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(0)
    }
}

#[test]
fn optional_type2_corpus_is_not_run_by_default() {
    println!("NOT_RUN\tchecked=0\tmatched=0\tunsupported=0\tskipped=0\tfailed=0");
}

#[test]
fn pinned_inventory_is_self_consistent_without_private_sources() {
    let samples = catalog().unwrap();
    assert_eq!(samples.len(), SOURCES);
    assert_eq!(
        samples
            .iter()
            .map(|sample| sample.images.len())
            .sum::<usize>(),
        TYPE2_IMAGES
    );
}

#[test]
fn source_identity_rejects_append_and_same_size_rewrite() {
    let path = env::temp_dir().join(format!(
        "caj2pdf-type2-identity-{}-{}",
        std::process::id(),
        NEXT_TEST_SOURCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    file.write_all(b"original").unwrap();
    file.flush().unwrap();
    let sha = hex(&Sha256::digest(b"original"));
    verify_source(&path, 8, &sha).unwrap();
    file.write_all(b"!").unwrap();
    file.flush().unwrap();
    assert!(verify_source(&path, 8, &sha).is_err());
    file.set_len(8).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(b"changed!").unwrap();
    file.flush().unwrap();
    assert!(verify_source(&path, 8, &sha).is_err());
    drop(file);
    fs::remove_file(path).unwrap();
}

#[test]
#[ignore = "requires explicit private CAJ2PDF_CORPUS_DIR"]
fn all_pinned_type2_jpeg_headers() {
    run_all().unwrap_or_else(|error| panic!("HN/C8 type-2 inventory failed: {error}"));
}

fn run_all() -> TestResult<()> {
    let corpus = PathBuf::from(
        env::var_os("CAJ2PDF_CORPUS_DIR").ok_or("explicit run requires CAJ2PDF_CORPUS_DIR")?,
    )
    .canonicalize()?;
    if !corpus.is_dir() {
        return Err("CAJ2PDF_CORPUS_DIR is not a directory".into());
    }
    let samples = catalog()?;
    let mut paths = Vec::with_capacity(SOURCES);
    for sample in &samples {
        let path = source_path(&corpus, &sample.path)?;
        verify_source(&path, sample.size, &sample.sha)?;
        paths.push(path);
    }
    println!(
        "START\texpected_sources={SOURCES}\texpected_type2={TYPE2_IMAGES}\tcatalog_sha256={CATALOG_SHA256}\tinventory_sha256={INVENTORY_SHA256}"
    );

    let limits = Limits {
        io_chunk_bytes: IO_CHUNK,
        max_input_bytes: MAX_SOURCE,
        max_output_bytes: MAX_JPEG,
        max_allocation_bytes: MAX_JPEG,
        max_pages: 100_000,
        max_bookmarks: 100_000,
    };
    let jpeg_budget = JpegBudget {
        max_payload_bytes: MAX_JPEG,
        max_markers: 65_536,
        max_work_bytes: 128 * 1024 * 1024,
    };
    let mut checked = 0;
    let mut matched = 0;
    let mut failed = 0;
    let mut unsupported = 0;
    let skipped = 0;
    let mut other_types = [0; 3];
    let mut anomalies = 0;
    let mut gray = 0;
    let mut ycbcr = 0;
    let mut variant_profiles = [[0_usize; 2]; 3];
    let mut max_request = 0;
    let mut max_payload = 0;
    let mut bytes_read = 0;
    for (index, (sample, path)) in samples.iter().zip(&paths).enumerate() {
        let mut source = CountedSource::open(path, sample.size)?;
        let mut verifier = File::open(path)?;
        let mut root = ready(Hnc8Reader::open(
            &mut source,
            &limits,
            &NeverCancel,
            Budget::default(),
        ))?;
        let header = root.header();
        let pages = header.page_count;
        let variant_index = match header.variant {
            Variant::C8 => 0,
            Variant::HnA => 1,
            Variant::HnB => 2,
        };
        let mut images = sample.images.iter();
        for page_number in 1..=pages {
            let mut page = ready(Hnc8Reader::probe_at_page(
                root.source_mut(),
                &limits,
                &NeverCancel,
                Budget::default(),
                page_number,
            ))?;
            match ready(page.next_page()) {
                Ok(Some(_)) => {}
                Ok(None) => return Err("declared page disappeared".into()),
                Err(error) if expected_anomaly(index, &error) => {
                    anomalies += 1;
                    continue;
                }
                Err(error) => return Err(error.into()),
            }
            loop {
                let record = match ready(page.next_image()) {
                    Ok(Some(record)) => record,
                    Ok(None) => break,
                    Err(error) if expected_anomaly(index, &error) => {
                        anomalies += 1;
                        break;
                    }
                    Err(error) => return Err(error.into()),
                };
                if record.record_type != 2 {
                    let slot = match record.record_type {
                        0 => 0,
                        1 => 1,
                        3 => 2,
                        _ => return Err("unmeasured non-type-2 descriptor".into()),
                    };
                    other_types[slot] += 1;
                    continue;
                }
                checked += 1;
                let expected = images.next().ok_or("unlisted type-2 record")?;
                if (
                    record.page_number,
                    record.image_number,
                    record.descriptor_offset,
                    record.payload.offset,
                    record.payload.length,
                ) != (
                    expected.page,
                    expected.number,
                    expected.descriptor,
                    expected.offset,
                    expected.length,
                ) {
                    return Err("type-2 descriptor differs from pinned inventory".into());
                }
                check_hash(
                    "JPEG span before parsing",
                    &hash_span(&mut verifier, expected.offset, expected.length)?,
                    &expected.sha,
                )?;
                let result = ready(read_type2_jpeg_info(
                    page.source_mut(),
                    record,
                    &limits,
                    &NeverCancel,
                    jpeg_budget,
                ));
                check_hash(
                    "JPEG span after parsing",
                    &hash_span(&mut verifier, expected.offset, expected.length)?,
                    &expected.sha,
                )?;
                match result {
                    Ok(info) => {
                        let color = match info.color {
                            JpegColor::Gray => {
                                gray += 1;
                                variant_profiles[variant_index][0] += 1;
                                expected.components == 1
                            }
                            JpegColor::Ycbcr => {
                                ycbcr += 1;
                                variant_profiles[variant_index][1] += 1;
                                expected.components == 3
                            }
                        };
                        if !color
                            || info.payload != record.payload
                            || info.width != expected.width
                            || info.height != expected.height
                            || info.precision != expected.precision
                            || info.components != expected.components
                            || info.app0_jfif != expected.jfif
                            || info.restart_interval.is_some()
                            || info.scans != expected.scans
                        {
                            failed += 1;
                            println!(
                                "IMAGE_FAIL\tsource_index={index}\tpage={}\timage={}\treason=header_mismatch",
                                expected.page, expected.number
                            );
                        } else {
                            matched += 1;
                        }
                    }
                    Err(error) if matches!(error.kind, ErrorKind::Unsupported { .. }) => {
                        unsupported += 1;
                        println!(
                            "IMAGE_UNSUPPORTED\tsource_index={index}\tpage={}\timage={}\toffset={}",
                            expected.page, expected.number, error.offset
                        );
                    }
                    Err(error) => {
                        failed += 1;
                        println!(
                            "IMAGE_FAIL\tsource_index={index}\tpage={}\timage={}\toffset={}\tfield={}\tkind={}",
                            expected.page,
                            expected.number,
                            error.offset,
                            error.kind.field(),
                            error.kind.as_str()
                        );
                    }
                }
                max_payload = max_payload.max(expected.length);
                if checked % 100 == 0 {
                    println!(
                        "PROGRESS\tchecked={checked}\tmatched={matched}\tfailed={failed}\tunsupported={unsupported}"
                    );
                }
            }
        }
        if images.next().is_some() {
            return Err("pinned type-2 record was not found".into());
        }
        verify_source(path, sample.size, &sample.sha)?;
        max_request = max_request.max(source.largest_request);
        bytes_read += source.bytes_read;
    }
    pinned_metadata("matrix.json", MAX_MATRIX, MATRIX_SHA256)?;
    pinned_metadata("jbig1_oracle.json", MAX_CATALOG, CATALOG_SHA256)?;
    pinned_metadata(
        "hnc8_type2_jpeg_inventory.tsv",
        MAX_INVENTORY,
        INVENTORY_SHA256,
    )?;
    let peak = peak_rss_kib()?;
    println!(
        "SUMMARY\tchecked={checked}\tmatched={matched}\tunsupported={unsupported}\tskipped={skipped}\tfailed={failed}\tsources_before={SOURCES}\tsources_after={SOURCES}\tgray={gray}\tycbcr={ycbcr}\tc8_gray={}\tc8_ycbcr={}\thna_gray={}\thna_ycbcr={}\thnb_gray={}\thnb_ycbcr={}\tanomalies={anomalies}\tother_type0={}\tother_type1={}\tother_type3={}\tmax_request_bytes={max_request}\tmax_payload_bytes={max_payload}\treader_bytes_read={bytes_read}\tprocess_vm_hwm_kib={peak}",
        variant_profiles[0][0],
        variant_profiles[0][1],
        variant_profiles[1][0],
        variant_profiles[1][1],
        variant_profiles[2][0],
        variant_profiles[2][1],
        other_types[0],
        other_types[1],
        other_types[2]
    );
    if checked != TYPE2_IMAGES
        || matched != TYPE2_IMAGES
        || unsupported != 0
        || skipped != 0
        || failed != 0
        || gray != 744
        || ycbcr != 341
        || variant_profiles != [[3, 27], [739, 314], [2, 0]]
        || anomalies != EXPECTED_ANOMALIES
        || other_types != OTHER_TYPES
    {
        return Err("type-2 profile or container totals differ from the pinned inventory".into());
    }
    Ok(())
}
