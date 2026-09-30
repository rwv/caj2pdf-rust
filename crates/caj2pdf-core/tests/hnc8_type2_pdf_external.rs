// SPDX-License-Identifier: MIT

//! Opt-in, hash-pinned PDF passthrough and pixel checks for private HN/C8 JPEGs.
//! A normal test run reads no private document and reports zero matches.

use caj2pdf_core::{
    Limits, NeverCancel, RangedSource, SequentialSink,
    hnc8::{
        Budget, ErrorKind, Hnc8Error, Hnc8Reader, JpegBudget, Type2ImageSelection, Type2PdfError,
        Type2PdfErrorKind, Type2PdfOptions, Variant, convert_type2_image_pdf,
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
    process::{Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
    task::{Context, Poll, Waker},
    time::Instant,
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
const MAX_PDF: u64 = 16 * 1024 * 1024;
const MAX_RASTER: u64 = 64 * 1024 * 1024;
const MAX_TOOL_TEXT: u64 = 32 * 1024;
const GRAY_CANARY: (usize, u32, u32) = (12, 1, 1);
const COLOR_CANARY: (usize, u32, u32) = (0, 1, 1);
const MULTI_CANARY: (usize, u32, u32) = (5, 1, 2);
// Direct JPEG versus decoded image and MuPDF page raster uses an 8-level
// single-channel and 1.5-level mean bound for legal decoder rounding.
// Poppler page raster has separately observed local filtering: every channel
// must lie inside the source's clipped 3x3 neighborhood with zero slack.
// Fixed colorful canaries also require tight pointwise stable-region matches.
const MAX_CHANNEL_DIFFERENCE: u8 = 8;
const MAX_MEAN_DIFFERENCE: f64 = 1.5;
static NEXT_TEST_SOURCE: AtomicUsize = AtomicUsize::new(0);

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

struct BoundedPdf {
    file: File,
    written: u64,
}

impl BoundedPdf {
    fn create(path: &Path) -> TestResult<Self> {
        Ok(Self {
            file: OpenOptions::new().create_new(true).write(true).open(path)?,
            written: 0,
        })
    }
}

impl SequentialSink for BoundedPdf {
    async fn write(&mut self, bytes: &[u8]) -> caj2pdf_core::Result<usize> {
        let next = self.written.checked_add(bytes.len() as u64).ok_or(
            caj2pdf_core::Error::InvalidInput {
                reason: "PDF spool length overflows",
            },
        )?;
        if next > MAX_PDF {
            return Err(caj2pdf_core::Error::LimitExceeded {
                resource: "one-image PDF spool bytes",
                limit: MAX_PDF,
                attempted: next,
            });
        }
        let count = self.file.write(bytes)?;
        self.written += count as u64;
        Ok(count)
    }

    async fn flush(&mut self) -> caj2pdf_core::Result<()> {
        Ok(self.file.flush()?)
    }
}

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

struct PrivateTemp(PathBuf);

impl PrivateTemp {
    fn new() -> TestResult<Self> {
        let base = env::var_os("CAJ2PDF_HN_PDF_SPOOL_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(env::temp_dir);
        if !base.is_dir() {
            return Err("PDF spool directory is absent".into());
        }
        for _ in 0..100 {
            let path = base.join(format!(
                "caj2pdf-hn-type2-pdf-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            #[cfg(unix)]
            let mut builder = fs::DirBuilder::new();
            #[cfg(not(unix))]
            let builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt as _;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err("could not create a unique private PDF spool".into())
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for PrivateTemp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn command_quiet(command: &mut Command, label: &str) -> TestResult<()> {
    let status = command
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| format!("{label} is unavailable"))?;
    if !status.success() {
        return Err(format!("{label} failed with status {status}").into());
    }
    Ok(())
}

fn command_text(command: &mut Command, label: &str, stderr: bool) -> TestResult<String> {
    let mut child = command
        .stdout(if stderr {
            Stdio::null()
        } else {
            Stdio::piped()
        })
        .stderr(if stderr {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .spawn()
        .map_err(|_| format!("{label} is unavailable"))?;
    let mut bytes = Vec::new();
    let reader: Box<dyn Read> = if stderr {
        Box::new(child.stderr.take().ok_or("tool stderr pipe is missing")?)
    } else {
        Box::new(child.stdout.take().ok_or("tool stdout pipe is missing")?)
    };
    reader.take(MAX_TOOL_TEXT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_TOOL_TEXT {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("{label} text output exceeds 32 KiB").into());
    }
    let status = child.wait()?;
    if !status.success() {
        return Err(format!("{label} failed with status {status}").into());
    }
    Ok(String::from_utf8(bytes)?)
}

fn bounded_file_size(path: &Path, limit: u64) -> TestResult<u64> {
    let size = path.metadata()?.len();
    if size > limit {
        return Err(format!("temporary file exceeds {limit}-byte bound").into());
    }
    Ok(size)
}

fn require_pdf_markers(path: &Path, components: u8) -> TestResult<()> {
    let color: &[u8] = if components == 1 {
        b"/ColorSpace /DeviceGray"
    } else {
        b"/ColorSpace /DeviceRGB"
    };
    let mut needles = vec![
        b"/Filter /DCTDecode".as_slice(),
        b"/BitsPerComponent 8",
        color,
    ];
    if components == 3 {
        needles.push(b"/ColorTransform 1");
    }
    let mut counts = vec![0_u32; needles.len()];
    let mut file = File::open(path)?;
    let mut buffer = [0_u8; IO_CHUNK];
    let mut tail = Vec::new();
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let old = tail.len();
        tail.extend_from_slice(&buffer[..count]);
        for (matched, needle) in counts.iter_mut().zip(&needles) {
            *matched += tail
                .windows(needle.len())
                .enumerate()
                .filter(|(start, window)| *start + needle.len() > old && *window == *needle)
                .count() as u32;
        }
        let keep = needles.iter().map(|needle| needle.len()).max().unwrap() - 1;
        let remove = tail.len().saturating_sub(keep);
        tail.drain(..remove);
    }
    if counts.iter().any(|count| *count != 1) {
        return Err(format!("expected one JPEG image dictionary; marker counts={counts:?}").into());
    }
    Ok(())
}

fn pnm_token(file: &mut File) -> TestResult<String> {
    let mut token = Vec::new();
    let mut in_comment = false;
    loop {
        if file.stream_position()? > 4096 {
            return Err("PNM header exceeds 4096 bytes".into());
        }
        let mut one = [0_u8];
        file.read_exact(&mut one)?;
        let byte = one[0];
        if in_comment {
            if byte == b'\n' {
                in_comment = false;
            }
            continue;
        }
        if token.is_empty() && byte == b'#' {
            in_comment = true;
            continue;
        }
        if byte.is_ascii_whitespace() {
            if !token.is_empty() {
                return Ok(String::from_utf8(token)?);
            }
        } else {
            token.push(byte);
            if token.len() > 128 {
                return Err("PNM header token exceeds 128 bytes".into());
            }
        }
    }
}

fn pnm_start_as(
    file: &mut File,
    width: u16,
    height: u16,
    components: u8,
) -> TestResult<(usize, u64)> {
    file.seek(SeekFrom::Start(0))?;
    let magic = pnm_token(file)?;
    let expected_magic = if components == 1 { "P5" } else { "P6" };
    if magic != expected_magic
        || pnm_token(file)?.parse::<u16>()? != width
        || pnm_token(file)?.parse::<u16>()? != height
        || pnm_token(file)? != "255"
    {
        return Err("PNM format, geometry, or sample precision differs".into());
    }
    // pnm_token consumed exactly the maxval delimiter. Skipping further
    // whitespace here would discard valid leading raster pixels.
    let start = file.stream_position()?;
    let stride = usize::from(width)
        .checked_mul(usize::from(components))
        .ok_or("PNM stride overflows")?;
    let payload = stride
        .checked_mul(usize::from(height))
        .ok_or("PNM payload overflows")?;
    if payload as u64 > MAX_RASTER || file.metadata()?.len() != start + payload as u64 {
        return Err("PNM raster length differs from geometry or bound".into());
    }
    Ok((stride, start))
}

fn pnm_start(file: &mut File, image: &Image) -> TestResult<(usize, u64)> {
    pnm_start_as(file, image.width, image.height, image.components)
}

fn read_raster_row(file: &mut File, start: u64, row: usize, buffer: &mut [u8]) -> TestResult<()> {
    let offset = start
        .checked_add(u64::try_from(
            row.checked_mul(buffer.len())
                .ok_or("PNM row offset overflows")?,
        )?)
        .ok_or("PNM row offset overflows")?;
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(buffer)?;
    Ok(())
}

fn require_informative_canary(path: &Path, image: &Image) -> TestResult<()> {
    let mut file = File::open(path)?;
    let (stride, start) = pnm_start(&mut file, image)?;
    let mut row = vec![0_u8; stride];
    let mut opposite = vec![0_u8; stride];
    let mut nonwhite = false;
    let mut colorful = false;
    let mut vertical_asymmetry = false;
    let mut horizontal_asymmetry = false;
    for y in 0..usize::from(image.height) {
        read_raster_row(&mut file, start, y, &mut row)?;
        read_raster_row(
            &mut file,
            start,
            usize::from(image.height) - 1 - y,
            &mut opposite,
        )?;
        vertical_asymmetry |= row != opposite;
        let components = usize::from(image.components);
        for x in 0..usize::from(image.width) {
            let pixel = &row[x * components..(x + 1) * components];
            let reflected = &row[(usize::from(image.width) - 1 - x) * components
                ..(usize::from(image.width) - x) * components];
            nonwhite |= pixel.iter().any(|value| *value < 250);
            if components == 3 {
                colorful |= pixel.iter().max().unwrap() - pixel.iter().min().unwrap() >= 8;
            }
            horizontal_asymmetry |= pixel != reflected;
        }
    }
    if !nonwhite || !vertical_asymmetry || !horizontal_asymmetry {
        return Err("pixel canary is blank or orientation-symmetric".into());
    }
    if image.components == 3 && !colorful {
        return Err("color canary has no meaningfully chromatic pixel".into());
    }
    Ok(())
}

#[derive(Default)]
struct PixelDiff {
    worst: u8,
    sum: u64,
    channels: u64,
    different: u64,
    outside_neighborhood: u64,
    worst_neighborhood_excess: u8,
    stable_chromatic_pixels: u64,
    stable_chromatic_violations: u64,
}

impl PixelDiff {
    fn mean(&self) -> f64 {
        self.sum as f64 / self.channels as f64
    }
}

fn compare_rasters(
    direct: &Path,
    rendered: &Path,
    image: &Image,
    neighborhood: bool,
    require_stable_chromatic: bool,
) -> TestResult<PixelDiff> {
    let mut direct = File::open(direct)?;
    let (stride, direct_start) = pnm_start(&mut direct, image)?;
    let mut rendered = File::open(rendered)?;
    let (rendered_stride, rendered_start) = pnm_start(&mut rendered, image)?;
    if rendered_stride != stride {
        return Err("renderer changed PNM stride".into());
    }
    let mut want = vec![0_u8; stride];
    let mut got = vec![0_u8; stride];
    let mut above = vec![0_u8; stride];
    let mut below = vec![0_u8; stride];
    let mut diff = PixelDiff::default();
    for y in 0..usize::from(image.height) {
        read_raster_row(&mut direct, direct_start, y, &mut want)?;
        read_raster_row(&mut rendered, rendered_start, y, &mut got)?;
        if neighborhood {
            read_raster_row(&mut direct, direct_start, y.saturating_sub(1), &mut above)?;
            read_raster_row(
                &mut direct,
                direct_start,
                (y + 1).min(usize::from(image.height) - 1),
                &mut below,
            )?;
        }
        let components = usize::from(image.components);
        let mut chromatic_stable = true;
        let mut chromatic_pointwise_ok = true;
        for (i, (a, b)) in want.iter().zip(&got).enumerate() {
            let amount = a.abs_diff(*b);
            diff.worst = diff.worst.max(amount);
            diff.sum += u64::from(amount);
            diff.channels += 1;
            diff.different += u64::from(amount != 0);
            if neighborhood {
                let x = i / components;
                let channel = i % components;
                let mut smallest = u8::MAX;
                let mut largest = u8::MIN;
                for row in [&above, &want, &below] {
                    for neighbor in x.saturating_sub(1)..=(x + 1).min(usize::from(image.width) - 1)
                    {
                        let value = row[neighbor * components + channel];
                        smallest = smallest.min(value);
                        largest = largest.max(value);
                    }
                }
                let excess = smallest.saturating_sub(*b).max(b.saturating_sub(largest));
                diff.outside_neighborhood += u64::from(excess != 0);
                diff.worst_neighborhood_excess = diff.worst_neighborhood_excess.max(excess);
                if components == 3 {
                    chromatic_stable &= largest - smallest <= MAX_CHANNEL_DIFFERENCE;
                    chromatic_pointwise_ok &= amount <= MAX_CHANNEL_DIFFERENCE;
                    if channel == 2 {
                        let pixel = &want[i - 2..=i];
                        let chromatic = *pixel.iter().max().unwrap() - *pixel.iter().min().unwrap()
                            >= MAX_CHANNEL_DIFFERENCE;
                        if chromatic && chromatic_stable {
                            diff.stable_chromatic_pixels += 1;
                            diff.stable_chromatic_violations += u64::from(!chromatic_pointwise_ok);
                        }
                        chromatic_stable = true;
                        chromatic_pointwise_ok = true;
                    }
                }
            }
        }
    }
    if neighborhood && diff.outside_neighborhood != 0 {
        return Err(format!(
            "Poppler pixel is outside direct JPEG 3x3 neighborhood: violations={} worst_excess={} raw_worst={} raw_mean={:.4}",
            diff.outside_neighborhood,
            diff.worst_neighborhood_excess,
            diff.worst,
            diff.mean()
        )
        .into());
    }
    if require_stable_chromatic
        && image.components == 3
        && (diff.stable_chromatic_pixels < 10 || diff.stable_chromatic_violations != 0)
    {
        return Err(format!(
            "Poppler stable chromatic regions failed: stable_pixels={} pointwise_violations={}",
            diff.stable_chromatic_pixels, diff.stable_chromatic_violations
        )
        .into());
    }
    if !neighborhood && (diff.worst > MAX_CHANNEL_DIFFERENCE || diff.mean() > MAX_MEAN_DIFFERENCE) {
        return Err(format!(
            "independent JPEG/PDF pixel difference exceeds rule: worst={} mean={:.4} different={}",
            diff.worst,
            diff.mean(),
            diff.different
        )
        .into());
    }
    Ok(diff)
}

fn copy_selected_jpeg(source: &Path, image: &Image, destination: &Path) -> TestResult<()> {
    let mut reader = File::open(source)?;
    reader.seek(SeekFrom::Start(image.offset))?;
    let mut writer = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)?;
    let mut remain = image.length;
    let mut buffer = [0_u8; IO_CHUNK];
    while remain != 0 {
        let count = usize::try_from(remain.min(IO_CHUNK as u64))?;
        reader.read_exact(&mut buffer[..count])?;
        writer.write_all(&buffer[..count])?;
        remain -= count as u64;
    }
    writer.flush()?;
    if writer.metadata()?.len() != image.length {
        return Err("direct JPEG copy length differs from selected span".into());
    }
    check_hash(
        "direct JPEG copy",
        &hash_span(&mut File::open(destination)?, 0, image.length)?,
        &image.sha,
    )
}

fn compare_decoded_pdfimages(
    direct: &Path,
    decoded: &Path,
    image: &Image,
) -> TestResult<PixelDiff> {
    let mut direct = File::open(direct)?;
    let (direct_stride, direct_start) = pnm_start(&mut direct, image)?;
    let mut decoded = File::open(decoded)?;
    let (decoded_stride, decoded_start) = pnm_start_as(&mut decoded, image.width, image.height, 3)?;
    let mut want = vec![0_u8; direct_stride];
    let mut got = vec![0_u8; decoded_stride];
    let mut diff = PixelDiff::default();
    for y in 0..usize::from(image.height) {
        read_raster_row(&mut direct, direct_start, y, &mut want)?;
        read_raster_row(&mut decoded, decoded_start, y, &mut got)?;
        for (i, actual) in got.iter().enumerate() {
            let expected = if image.components == 1 {
                want[i / 3]
            } else {
                want[i]
            };
            let amount = expected.abs_diff(*actual);
            diff.worst = diff.worst.max(amount);
            diff.sum += u64::from(amount);
            diff.channels += 1;
            diff.different += u64::from(amount != 0);
        }
    }
    if diff.worst > MAX_CHANNEL_DIFFERENCE || diff.mean() > MAX_MEAN_DIFFERENCE {
        return Err(format!(
            "decoded PDF image differs from direct JPEG: worst={} mean={:.4}",
            diff.worst,
            diff.mean()
        )
        .into());
    }
    Ok(diff)
}

struct RenderResult {
    worst_decoded: u8,
    worst_poppler: u8,
    worst_mupdf: u8,
    mean_decoded: f64,
    mean_poppler: f64,
    mean_mupdf: f64,
    changed_decoded: u64,
    changed_poppler: u64,
    changed_mupdf: u64,
    stable_chromatic_pixels: u64,
}

fn render_pixels(
    temp: &PrivateTemp,
    source: &Path,
    pdf: &Path,
    image: &Image,
    canary: bool,
) -> TestResult<RenderResult> {
    let direct_jpeg = temp.path("direct.jpg");
    copy_selected_jpeg(source, image, &direct_jpeg)?;
    let extension = if image.components == 1 { "pgm" } else { "ppm" };
    let direct = temp.path(&format!("direct.{extension}"));
    command_quiet(
        Command::new("djpeg")
            .args(["-pnm", "-outfile"])
            .arg(&direct)
            .arg(&direct_jpeg),
        "libjpeg-turbo djpeg",
    )?;
    bounded_file_size(&direct, MAX_RASTER)?;
    if canary {
        require_informative_canary(&direct, image)?;
    }

    let decoded_root = temp.path("decoded");
    command_quiet(
        Command::new("pdfimages").arg(pdf).arg(&decoded_root),
        "Poppler pdfimages decoded image",
    )?;
    let decoded = temp.path("decoded-000.ppm");
    let decoded_count = fs::read_dir(&temp.0)?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("decoded-"))
        .count();
    if decoded_count != 1 || !decoded.is_file() {
        return Err("pdfimages did not decode exactly one PPM image".into());
    }
    bounded_file_size(&decoded, MAX_RASTER)?;
    let decoded_diff = compare_decoded_pdfimages(&direct, &decoded, image)?;

    let poppler_root = temp.path("poppler");
    let mut poppler_command = Command::new("pdftoppm");
    poppler_command.args([
        "-r",
        "72",
        "-singlefile",
        "-aa",
        "no",
        "-aaVector",
        "no",
        "-f",
        "1",
        "-l",
        "1",
    ]);
    if image.components == 1 {
        poppler_command.arg("-gray");
    }
    command_quiet(
        poppler_command.arg(pdf).arg(&poppler_root),
        "Poppler pdftoppm",
    )?;
    let poppler = poppler_root.with_extension(extension);
    bounded_file_size(&poppler, MAX_RASTER)?;
    let poppler_diff = compare_rasters(
        &direct,
        &poppler,
        image,
        true,
        canary && image.components == 3,
    )?;

    let mupdf = temp.path(&format!("mupdf.{extension}"));
    command_quiet(
        Command::new("mutool")
            .args(["draw", "-q", "-r", "72", "-A", "0", "-c"])
            .arg(if image.components == 1 { "gray" } else { "rgb" })
            .arg("-o")
            .arg(&mupdf)
            .arg(pdf)
            .arg("1"),
        "MuPDF mutool draw",
    )?;
    bounded_file_size(&mupdf, MAX_RASTER)?;
    let mupdf_diff = compare_rasters(&direct, &mupdf, image, false, false)?;
    Ok(RenderResult {
        worst_decoded: decoded_diff.worst,
        worst_poppler: poppler_diff.worst,
        worst_mupdf: mupdf_diff.worst,
        mean_decoded: decoded_diff.mean(),
        mean_poppler: poppler_diff.mean(),
        mean_mupdf: mupdf_diff.mean(),
        changed_decoded: decoded_diff.different,
        changed_poppler: poppler_diff.different,
        changed_mupdf: mupdf_diff.different,
        stable_chromatic_pixels: poppler_diff.stable_chromatic_pixels,
    })
}

struct ImageRun {
    max_request: usize,
    reader_bytes: u64,
    pdf_bytes: u64,
    jpeg_bytes: u64,
    max_raster_bytes: u64,
    temp_storage_bytes: u64,
    scratch_bound_bytes: u64,
    render: Option<RenderResult>,
}

fn run_image(
    path: &Path,
    sample: &Sample,
    image: &Image,
    variant: Variant,
    render: bool,
    canary: bool,
) -> TestResult<ImageRun> {
    let raster_bytes = u64::from(image.width)
        .checked_mul(u64::from(image.height))
        .and_then(|pixels| pixels.checked_mul(u64::from(image.components)))
        .ok_or("selected JPEG raster geometry overflows")?;
    if image.width == 0
        || image.height == 0
        || !matches!(image.components, 1 | 3)
        || raster_bytes + 4096 > MAX_RASTER
    {
        return Err("selected JPEG raster geometry exceeds local bound".into());
    }
    verify_source(path, sample.size, &sample.sha)?;
    let mut verifier = File::open(path)?;
    check_hash(
        "selected JPEG span before PDF conversion",
        &hash_span(&mut verifier, image.offset, image.length)?,
        &image.sha,
    )?;
    let temp = PrivateTemp::new()?;
    let pdf = temp.path("selected.pdf");
    let mut source = CountedSource::open(path, sample.size)?;
    let mut sink = BoundedPdf::create(&pdf)?;
    let limits = Limits {
        io_chunk_bytes: IO_CHUNK,
        max_input_bytes: MAX_SOURCE,
        max_output_bytes: MAX_PDF,
        max_allocation_bytes: MAX_JPEG,
        max_pages: 100_000,
        max_bookmarks: 100_000,
    };
    let options = Type2PdfOptions {
        pixels_per_inch: 72.0,
        container: Budget::default(),
        jpeg: JpegBudget {
            max_payload_bytes: MAX_JPEG,
            max_markers: 65_536,
            max_work_bytes: 128 * 1024 * 1024,
        },
    };
    let report = ready(convert_type2_image_pdf(
        &mut source,
        &mut sink,
        Type2ImageSelection {
            page_number: image.page,
            image_number: image.number,
        },
        options,
        &limits,
        &NeverCancel,
    ))?;
    ready(sink.flush())?;
    if report.image.page_number != image.page
        || report.image.image_number != image.number
        || report.image.descriptor_offset != image.descriptor
        || report.image.record_type != 2
        || report.image.payload.offset != image.offset
        || report.image.payload.length != image.length
        || report.source_variant != variant
        || report.source_pages < image.page
        || report.jpeg.payload != report.image.payload
        || report.jpeg.width != image.width
        || report.jpeg.height != image.height
        || report.jpeg.precision != image.precision
        || report.jpeg.components != image.components
        || report.jpeg.app0_jfif != image.jfif
        || report.jpeg.scans != image.scans
        || report.conversion.pages_converted != 1
        || report.conversion.input_bytes_read != source.bytes_read
        || report.conversion.output_bytes_written != sink.written
    {
        return Err("selected PDF report differs from pinned JPEG record".into());
    }
    drop(sink);
    let pdf_bytes = bounded_file_size(&pdf, MAX_PDF)?;
    require_pdf_markers(&pdf, image.components)?;
    command_quiet(
        Command::new("qpdf").arg("--check").arg(&pdf),
        "qpdf --check",
    )?;
    let pages = command_text(
        Command::new("qpdf").arg("--show-npages").arg(&pdf),
        "qpdf page count",
        false,
    )?;
    if pages.trim() != "1" {
        return Err("independent PDF page count differs from one".into());
    }
    let info = command_text(Command::new("pdfinfo").arg(&pdf), "Poppler pdfinfo", false)?;
    let page_size = info
        .lines()
        .find(|line| line.starts_with("Page size:"))
        .ok_or("pdfinfo did not report page geometry")?;
    let dimensions: Vec<_> = page_size.split_whitespace().collect();
    if dimensions.len() < 5
        || dimensions[2].parse::<f64>()? != f64::from(image.width)
        || dimensions[3] != "x"
        || dimensions[4].parse::<f64>()? != f64::from(image.height)
    {
        return Err("PDF page geometry differs from JPEG pixels at 72 ppi".into());
    }
    let listing = command_text(
        Command::new("pdfimages").arg("-list").arg(&pdf),
        "Poppler pdfimages -list",
        false,
    )?;
    let rows: Vec<_> = listing
        .lines()
        .filter(|line| line.trim_start().starts_with("1 "))
        .collect();
    if rows.len() != 1 {
        return Err("PDF must contain exactly one independently listed image".into());
    }
    let fields: Vec<_> = rows[0].split_whitespace().collect();
    if fields.len() < 9
        || fields[1] != "0"
        || fields[2] != "image"
        || fields[3].parse::<u16>()? != image.width
        || fields[4].parse::<u16>()? != image.height
        || fields[5] != if image.components == 1 { "gray" } else { "rgb" }
        || fields[6].parse::<u8>()? != image.components
        || fields[7] != "8"
        || fields[8] != "jpeg"
    {
        return Err("PDF image metadata differs from selected JPEG".into());
    }
    let extracted_root = temp.path("extracted");
    command_quiet(
        Command::new("pdfimages")
            .arg("-j")
            .arg(&pdf)
            .arg(&extracted_root),
        "Poppler pdfimages -j",
    )?;
    let extracted = temp.path("extracted-000.jpg");
    let count = fs::read_dir(&temp.0)?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("extracted-")
        })
        .count();
    if count != 1 || !extracted.is_file() {
        return Err("pdfimages -j did not extract exactly one JPEG".into());
    }
    let jpeg_bytes = bounded_file_size(&extracted, MAX_JPEG)?;
    if jpeg_bytes != image.length {
        return Err("extracted JPEG length differs from original span".into());
    }
    check_hash(
        "PDF embedded JPEG",
        &hash_span(&mut File::open(&extracted)?, 0, jpeg_bytes)?,
        &image.sha,
    )?;
    let render = if render {
        Some(render_pixels(&temp, path, &pdf, image, canary)?)
    } else {
        None
    };
    let mut max_raster_bytes = 0;
    let mut temp_storage_bytes = 0_u64;
    for entry in fs::read_dir(&temp.0)? {
        let entry = entry?;
        let size = entry.metadata()?.len();
        temp_storage_bytes = temp_storage_bytes
            .checked_add(size)
            .ok_or("temporary storage overflows")?;
        if entry
            .path()
            .extension()
            .is_some_and(|ext| ext == "pgm" || ext == "ppm")
        {
            max_raster_bytes = max_raster_bytes.max(size);
        }
    }
    if temp_storage_bytes > MAX_PDF + 2 * MAX_JPEG + 4 * MAX_RASTER {
        return Err("one-image temporary storage exceeds local bound".into());
    }
    check_hash(
        "selected JPEG span after PDF conversion",
        &hash_span(&mut verifier, image.offset, image.length)?,
        &image.sha,
    )?;
    verify_source(path, sample.size, &sample.sha)?;
    fs::remove_dir_all(&temp.0)?;
    Ok(ImageRun {
        max_request: source.largest_request,
        reader_bytes: source.bytes_read,
        pdf_bytes,
        jpeg_bytes,
        max_raster_bytes,
        temp_storage_bytes,
        scratch_bound_bytes: IO_CHUNK as u64 * 4
            + u64::from(image.width) * u64::from(image.components) * 4
            + 4096,
        render,
    })
}

fn tool_versions() -> TestResult<()> {
    let versions = [
        ("qpdf", "--version", false),
        ("pdfinfo", "-v", true),
        ("pdfimages", "-v", true),
        ("pdftoppm", "-v", true),
        ("mutool", "-v", true),
        ("djpeg", "-version", true),
    ];
    for (name, flag, stderr) in versions {
        let version = command_text(Command::new(name).arg(flag), name, stderr)?;
        let first = version
            .lines()
            .next()
            .ok_or("validator returned no version")?;
        println!("TOOL\t{name}\t{first}");
    }
    Ok(())
}

fn unsupported_error(error: &(dyn Error + 'static)) -> bool {
    let Some(error) = error.downcast_ref::<Type2PdfError>() else {
        return false;
    };
    match &error.kind {
        Type2PdfErrorKind::UnsupportedImageType(_) => true,
        Type2PdfErrorKind::Container(inner) | Type2PdfErrorKind::Jpeg(inner) => {
            matches!(inner.kind, ErrorKind::Unsupported { .. })
        }
        _ => false,
    }
}

fn fixed_canary(index: usize, image: &Image) -> bool {
    let identity = (index, image.page, image.number);
    identity == GRAY_CANARY || identity == COLOR_CANARY || identity == MULTI_CANARY
}

#[test]
fn optional_type2_pdf_corpus_is_not_run_by_default() {
    println!("NOT_RUN\tchecked=0\tmatched=0\trendered=0\tunsupported=0\tskipped=0\tfailed=0");
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
    assert!(samples[GRAY_CANARY.0].images.iter().any(|image| (
        image.page,
        image.number,
        image.components
    ) == (
        GRAY_CANARY.1,
        GRAY_CANARY.2,
        1
    )));
    for (index, page, number) in [COLOR_CANARY, MULTI_CANARY] {
        assert!(samples[index].images.iter().any(|image| (
            image.page,
            image.number,
            image.components
        ) == (page, number, 3)));
    }
    assert!(
        samples[MULTI_CANARY.0]
            .images
            .iter()
            .filter(|image| image.page == MULTI_CANARY.1)
            .count()
            > 1
    );
}

#[test]
fn source_identity_rejects_append_and_same_size_rewrite() {
    let path = env::temp_dir().join(format!(
        "caj2pdf-type2-pdf-identity-{}-{}",
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
fn binary_pnm_keeps_leading_whitespace_pixels_and_rejects_color_shift() {
    let temp = PrivateTemp::new().unwrap();
    let leading = temp.path("leading.pgm");
    fs::write(&leading, b"P5\n2 1\n255\n \n").unwrap();
    let mut file = File::open(&leading).unwrap();
    let (stride, start) = pnm_start_as(&mut file, 2, 1, 1).unwrap();
    let mut pixels = vec![0; stride];
    read_raster_row(&mut file, start, 0, &mut pixels).unwrap();
    assert_eq!(pixels, [b' ', b'\n']);

    let direct = temp.path("direct.ppm");
    let changed = temp.path("changed.ppm");
    let mut first = b"P6\n3 3\n255\n".to_vec();
    let mut second = first.clone();
    for _ in 0..9 {
        first.extend_from_slice(&[40, 120, 200]);
        second.extend_from_slice(&[200, 120, 40]);
    }
    fs::write(&direct, first).unwrap();
    fs::write(&changed, second).unwrap();
    let image = Image {
        page: 1,
        number: 1,
        descriptor: 0,
        offset: 0,
        length: 1,
        sha: String::new(),
        width: 3,
        height: 3,
        precision: 8,
        components: 3,
        jfif: true,
        scans: 1,
    };
    assert!(compare_rasters(&direct, &direct, &image, true, false).is_ok());
    assert!(compare_rasters(&direct, &changed, &image, true, false).is_err());
}

#[test]
#[ignore = "requires explicit private CAJ2PDF_CORPUS_DIR"]
fn three_pinned_pixel_canaries() {
    run_three_canaries().unwrap_or_else(|error| panic!("type-2 PDF canaries failed: {error}"));
}

fn corpus_and_paths() -> TestResult<(Vec<Sample>, Vec<PathBuf>)> {
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
    Ok((samples, paths))
}

fn run_three_canaries() -> TestResult<()> {
    let (samples, paths) = corpus_and_paths()?;
    tool_versions()?;
    for (index, page, number) in [GRAY_CANARY, COLOR_CANARY, MULTI_CANARY] {
        let sample = &samples[index];
        let path = &paths[index];
        let image = sample
            .images
            .iter()
            .find(|image| (image.page, image.number) == (page, number))
            .ok_or("pinned canary image is absent")?;
        let limits = Limits {
            io_chunk_bytes: IO_CHUNK,
            max_input_bytes: MAX_SOURCE,
            ..Limits::default()
        };
        let mut source = CountedSource::open(path, sample.size)?;
        let header = ready(Hnc8Reader::open(
            &mut source,
            &limits,
            &NeverCancel,
            Budget::default(),
        ))?
        .header();
        let run = run_image(path, sample, image, header.variant, true, true)?;
        let render = run.render.ok_or("canary renderer was not run")?;
        println!(
            "CANARY\tsource_index={index}\tpage={page}\timage={number}\tcomponents={}\tdecoded_worst={}\tdecoded_mean={:.4}\tdecoded_changed={}\tpoppler_raw_worst={}\tpoppler_raw_mean={:.4}\tpoppler_changed={}\tpoppler_neighborhood_violations=0\tstable_chromatic_pixels={}\tstable_chromatic_violations=0\tmupdf_worst={}\tmupdf_mean={:.4}\tmupdf_changed={}",
            image.components,
            render.worst_decoded,
            render.mean_decoded,
            render.changed_decoded,
            render.worst_poppler,
            render.mean_poppler,
            render.changed_poppler,
            render.stable_chromatic_pixels,
            render.worst_mupdf,
            render.mean_mupdf,
            render.changed_mupdf
        );
    }
    println!("SMOKE\tchecked=3\tmatched=3\trendered=3\tfailed=0");
    Ok(())
}

#[test]
#[ignore = "requires explicit private CAJ2PDF_CORPUS_DIR"]
fn all_pinned_type2_pdf_images() {
    run_all().unwrap_or_else(|error| panic!("HN/C8 type-2 PDF parity failed: {error}"));
}

fn run_all() -> TestResult<()> {
    let started = Instant::now();
    let render_all = match env::var("CAJ2PDF_HN_PDF_RENDER_ALL") {
        Ok(value) if value == "1" => true,
        Ok(value) if value.is_empty() => false,
        Err(env::VarError::NotPresent) => false,
        _ => return Err("CAJ2PDF_HN_PDF_RENDER_ALL must be 1 or unset".into()),
    };
    let render_mode = if render_all { "ALL" } else { "CANARIES" };
    let (samples, paths) = corpus_and_paths()?;
    tool_versions()?;
    println!(
        "START\trender_mode={render_mode}\texpected_sources={SOURCES}\texpected_images={TYPE2_IMAGES}\tcatalog_sha256={CATALOG_SHA256}\tinventory_sha256={INVENTORY_SHA256}\tpixel_rule_max_channel={MAX_CHANNEL_DIFFERENCE}\tpixel_rule_max_mean={MAX_MEAN_DIFFERENCE}"
    );
    let limits = Limits {
        io_chunk_bytes: IO_CHUNK,
        max_input_bytes: MAX_SOURCE,
        max_output_bytes: MAX_PDF,
        max_allocation_bytes: MAX_JPEG,
        max_pages: 100_000,
        max_bookmarks: 100_000,
    };
    let mut checked = 0;
    let mut matched = 0;
    let mut failed = 0;
    let mut unsupported = 0;
    let skipped = 0;
    let mut rendered = 0;
    let mut pixel_requested = 0;
    let mut rendered_gray = 0;
    let mut rendered_color = 0;
    let mut render_gray = 0;
    let mut render_color = 0;
    let mut render_multi = 0;
    let mut gray = 0;
    let mut ycbcr = 0;
    let mut profiles = [[0_usize; 2]; 3];
    let mut other_types = [0_usize; 3];
    let mut anomalies = 0;
    let mut max_request = 0_usize;
    let mut max_pdf = 0_u64;
    let mut max_jpeg = 0_u64;
    let mut max_raster = 0_u64;
    let mut max_temp_storage = 0_u64;
    let mut max_scratch = 0_u64;
    let mut reader_bytes = 0_u64;
    let mut identity_hash_bytes = samples.iter().map(|s| s.size).sum::<u64>();
    let mut pdf_bytes = 0_u64;
    let mut embedded_bytes = 0_u64;
    let mut worst_poppler = 0_u8;
    let mut worst_mupdf = 0_u8;
    let mut worst_decoded = 0_u8;
    let mut max_mean_poppler = 0.0_f64;
    let mut max_mean_mupdf = 0.0_f64;
    let mut max_mean_decoded = 0.0_f64;
    let mut stable_chromatic_pixels = 0_u64;
    let mut sources_after = 0;
    for (index, (sample, path)) in samples.iter().zip(&paths).enumerate() {
        let mut source = CountedSource::open(path, sample.size)?;
        let mut root = ready(Hnc8Reader::open(
            &mut source,
            &limits,
            &NeverCancel,
            Budget::default(),
        ))?;
        let header = root.header();
        let variant_index = match header.variant {
            Variant::C8 => 0,
            Variant::HnA => 1,
            Variant::HnB => 2,
        };
        let mut images = sample.images.iter();
        for page_number in 1..=header.page_count {
            let mut page = ready(Hnc8Reader::probe_at_page(
                root.source_mut(),
                &limits,
                &NeverCancel,
                Budget::default(),
                page_number,
            ))?;
            match ready(page.next_page()) {
                Ok(Some(_)) => {}
                Ok(None) => return Err("declared source page disappeared".into()),
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
                let image = images.next().ok_or("unlisted type-2 image")?;
                if (
                    record.page_number,
                    record.image_number,
                    record.descriptor_offset,
                    record.payload.offset,
                    record.payload.length,
                ) != (
                    image.page,
                    image.number,
                    image.descriptor,
                    image.offset,
                    image.length,
                ) {
                    return Err("type-2 descriptor differs from pinned inventory".into());
                }
                // Hash the complete source on both sides of every conversion.
                // The fixed inventory's weighted total is about 45 GiB of
                // sequential identity reads, with only a 64 KiB hash buffer.
                identity_hash_bytes = identity_hash_bytes
                    .checked_add(
                        sample
                            .size
                            .checked_mul(2)
                            .ok_or("identity bytes overflow")?,
                    )
                    .ok_or("identity bytes overflow")?;
                let canary = fixed_canary(index, image);
                let render = render_all || canary;
                pixel_requested += usize::from(render);
                let outcome = run_image(path, sample, image, header.variant, render, canary);
                match outcome {
                    Ok(run) => {
                        matched += 1;
                        let color_slot = if image.components == 1 {
                            gray += 1;
                            0
                        } else {
                            ycbcr += 1;
                            1
                        };
                        profiles[variant_index][color_slot] += 1;
                        max_request = max_request.max(run.max_request);
                        max_pdf = max_pdf.max(run.pdf_bytes);
                        max_jpeg = max_jpeg.max(run.jpeg_bytes);
                        max_raster = max_raster.max(run.max_raster_bytes);
                        max_temp_storage = max_temp_storage.max(run.temp_storage_bytes);
                        max_scratch = max_scratch.max(run.scratch_bound_bytes);
                        reader_bytes += run.reader_bytes;
                        pdf_bytes += run.pdf_bytes;
                        embedded_bytes += run.jpeg_bytes;
                        if let Some(render) = run.render {
                            rendered += 1;
                            if image.components == 1 {
                                rendered_gray += 1;
                            } else {
                                rendered_color += 1;
                            }
                            if (index, image.page, image.number) == GRAY_CANARY {
                                render_gray += 1;
                            } else if (index, image.page, image.number) == COLOR_CANARY {
                                render_color += 1;
                            } else if (index, image.page, image.number) == MULTI_CANARY {
                                render_multi += 1;
                            }
                            worst_poppler = worst_poppler.max(render.worst_poppler);
                            worst_mupdf = worst_mupdf.max(render.worst_mupdf);
                            worst_decoded = worst_decoded.max(render.worst_decoded);
                            max_mean_poppler = max_mean_poppler.max(render.mean_poppler);
                            max_mean_mupdf = max_mean_mupdf.max(render.mean_mupdf);
                            max_mean_decoded = max_mean_decoded.max(render.mean_decoded);
                            stable_chromatic_pixels += render.stable_chromatic_pixels;
                            if canary {
                                println!(
                                    "CANARY\tsource_index={index}\tpage={}\timage={}\tcomponents={}\tdecoded_worst={}\tdecoded_mean={:.4}\tdecoded_changed={}\tpoppler_raw_worst={}\tpoppler_raw_mean={:.4}\tpoppler_changed={}\tpoppler_neighborhood_violations=0\tstable_chromatic_pixels={}\tstable_chromatic_violations=0\tmupdf_worst={}\tmupdf_mean={:.4}\tmupdf_changed={}",
                                    image.page,
                                    image.number,
                                    image.components,
                                    render.worst_decoded,
                                    render.mean_decoded,
                                    render.changed_decoded,
                                    render.worst_poppler,
                                    render.mean_poppler,
                                    render.changed_poppler,
                                    render.stable_chromatic_pixels,
                                    render.worst_mupdf,
                                    render.mean_mupdf,
                                    render.changed_mupdf
                                );
                            }
                        }
                    }
                    Err(error) if unsupported_error(error.as_ref()) => {
                        unsupported += 1;
                        println!(
                            "IMAGE_UNSUPPORTED\tsource_index={index}\tpage={}\timage={}\terror={error}",
                            image.page, image.number
                        );
                    }
                    Err(error) => {
                        failed += 1;
                        println!(
                            "IMAGE_FAIL\tsource_index={index}\tpage={}\timage={}\terror={error}",
                            image.page, image.number
                        );
                    }
                }
                if checked % 100 == 0 {
                    println!(
                        "PROGRESS\tchecked={checked}\tmatched={matched}\tfailed={failed}\tunsupported={unsupported}\trendered={rendered}"
                    );
                }
            }
        }
        if images.next().is_some() {
            return Err("pinned type-2 image was not found".into());
        }
        verify_source(path, sample.size, &sample.sha)?;
        sources_after += 1;
        identity_hash_bytes += sample.size;
        max_request = max_request.max(source.largest_request);
        reader_bytes += source.bytes_read;
    }
    pinned_metadata("matrix.json", MAX_MATRIX, MATRIX_SHA256)?;
    pinned_metadata("jbig1_oracle.json", MAX_CATALOG, CATALOG_SHA256)?;
    pinned_metadata(
        "hnc8_type2_jpeg_inventory.tsv",
        MAX_INVENTORY,
        INVENTORY_SHA256,
    )?;
    let seconds = started.elapsed().as_secs_f64();
    let peak = peak_rss_kib()?;
    println!(
        "SUMMARY\trender_mode={render_mode}\tchecked={checked}\tmatched={matched}\tfailed={failed}\tunsupported={unsupported}\tskipped={skipped}\tpixel_requested={pixel_requested}\tpixel_matched={rendered}\trendered_gray={rendered_gray}\trendered_color={rendered_color}\tcanary_gray={render_gray}\tcanary_color={render_color}\tcanary_multi={render_multi}\tsources_before={SOURCES}\tsources_after={sources_after}\tanomalies={anomalies}\tgray={gray}\tycbcr={ycbcr}\tc8_gray={}\tc8_ycbcr={}\thna_gray={}\thna_ycbcr={}\thnb_gray={}\thnb_ycbcr={}\tother_type0={}\tother_type1={}\tother_type3={}\tmax_request_bytes={max_request}\tmax_pdf_bytes={max_pdf}\tmax_embedded_jpeg_bytes={max_jpeg}\tmax_raster_bytes={max_raster}\tmax_temp_storage_bytes={max_temp_storage}\tmax_accounted_scratch_bytes={max_scratch}\treader_bytes_read={reader_bytes}\tidentity_hash_bytes={identity_hash_bytes}\tpdf_bytes_written={pdf_bytes}\tembedded_jpeg_bytes={embedded_bytes}\telapsed_seconds={seconds:.3}\tselected_jpeg_mib_per_second={:.3}\tdecoded_worst_difference={worst_decoded}\tdecoded_max_mean_difference={max_mean_decoded:.4}\tpoppler_raw_worst_difference={worst_poppler}\tpoppler_raw_max_mean_difference={max_mean_poppler:.4}\tpoppler_neighborhood_violations=0\tstable_chromatic_pixels={stable_chromatic_pixels}\tstable_chromatic_violations=0\tmupdf_worst_difference={worst_mupdf}\tmupdf_max_mean_difference={max_mean_mupdf:.4}\tprocess_vm_hwm_kib={peak}",
        profiles[0][0],
        profiles[0][1],
        profiles[1][0],
        profiles[1][1],
        profiles[2][0],
        profiles[2][1],
        other_types[0],
        other_types[1],
        other_types[2],
        embedded_bytes as f64 / 1_048_576.0 / seconds
    );
    if checked != TYPE2_IMAGES
        || matched != TYPE2_IMAGES
        || failed != 0
        || unsupported != 0
        || skipped != 0
        || pixel_requested != if render_all { TYPE2_IMAGES } else { 3 }
        || rendered != pixel_requested
        || rendered_gray != if render_all { 744 } else { 1 }
        || rendered_color != if render_all { 341 } else { 2 }
        || (render_gray, render_color, render_multi) != (1, 1, 1)
        || sources_after != SOURCES
        || anomalies != EXPECTED_ANOMALIES
        || gray != 744
        || ycbcr != 341
        || profiles != [[3, 27], [739, 314], [2, 0]]
        || other_types != OTHER_TYPES
    {
        return Err("type-2 PDF, pixel, or container totals differ from pinned inventory".into());
    }
    Ok(())
}
