// SPDX-License-Identifier: MIT

//! Opt-in, independently extracted PDF pixel parity against the pinned #22
//! HN/C8 corpus. Private sources and the caller-supplied T.82 table stay out
//! of the repository and CI. A normal test run executes zero corpus cases.

use caj2pdf_core::{
    Limits, NeverCancel, RangedSource, SequentialSink,
    hnc8::{
        ErrorKind as Hnc8ErrorKind, Type0ImageSelection, Type0PdfError, Type0PdfErrorKind,
        Type0PdfOptions, Variant, convert_type0_image_pdf,
    },
    jbig1::{Type0Error, Type0ErrorKind},
    pdf::{BilevelImageSpec, PageSpec, PdfDocument},
    qm::{ContextState, QmState, QmTable},
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
    sync::atomic::{AtomicU64, Ordering},
    task::{Context, Poll, Waker},
};

type TestResult<T> = Result<T, Box<dyn Error>>;
const MANIFEST_SHA256: &str = "e88401f0d9cbd08608004c2a9e58577ab85c916b9a9891a4dd5466e346e9203a";
const IDS_SHA256: &str = "6d9c74816d763bad91f43b0e0ed8b7d6c05261e0997f855408c84ffc62daf8ed";
const TABLE_SHA256: &str = "11fe241dedbbf4faa542af4a1485566c2794fa69e5c06e2e5c8542adfe9b1ab7";
const EXPECTED_SOURCES: usize = 27;
const EXPECTED_IMAGES: usize = 1400;
const EXPECTED_MALFORMED_DISCOVERIES: usize = 3;
const IO_CHUNK: usize = 65_536;
const MAX_MANIFEST: u64 = 1_048_576;
const MAX_TABLE: u64 = 16_384;
const MAX_SOURCE: u64 = 8 * 1024 * 1024 * 1024;
const MAX_ENCODED: u64 = 64 * 1024 * 1024;
const MAX_RASTER: u64 = 128 * 1024 * 1024;
const MAX_PDF: u64 = 128 * 1024 * 1024;
// Stable (source index, page, image) identities in the SHA-pinned catalog.
// The three cases cover HN, C8, and a source page with multiple images.
const HN_RENDER: (usize, u32, u32) = (0, 2, 1);
const C8_RENDER: (usize, u32, u32) = (5, 1, 1);
const MULTI_RENDER: (usize, u32, u32) = (13, 3, 1);

#[derive(Debug)]
struct Image {
    page: u32,
    number: u32,
    offset: u64,
    length: u64,
    encoded_sha: String,
    width: usize,
    height: usize,
    stride: usize,
    raw_sha: String,
    visible_sha: String,
}

struct Sample {
    path: String,
    source_sha: String,
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
        return Err(
            format!("{label} SHA-256 mismatch: actual={actual} expected={expected}").into(),
        );
    }
    Ok(())
}

fn bounded_read(path: &Path, limit: u64) -> TestResult<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(format!("input exceeds {limit}-byte fixture bound").into());
    }
    Ok(bytes)
}

fn hash_span(file: &mut File, offset: u64, length: u64) -> TestResult<String> {
    let end = offset.checked_add(length).ok_or("source span overflows")?;
    if end > file.metadata()?.len() {
        return Err("source span exceeds file size".into());
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut remaining = length;
    let mut buffer = [0_u8; IO_CHUNK];
    let mut hash = Sha256::new();
    while remaining != 0 {
        let n = usize::try_from(remaining.min(buffer.len() as u64))?;
        file.read_exact(&mut buffer[..n])?;
        hash.update(&buffer[..n]);
        remaining -= n as u64;
    }
    Ok(hex(&hash.finalize()))
}

fn verify_source_unchanged(path: &Path, size_before: u64, expected_sha: &str) -> TestResult<()> {
    let mut file = File::open(path)?;
    let size_after = file.metadata()?.len();
    if size_after != size_before {
        return Err("source length changed after PDF conversions".into());
    }
    check_hash(
        "source after PDF conversions",
        &hash_span(&mut file, 0, size_after)?,
        expected_sha,
    )
}

fn external_table(path: &Path) -> TestResult<QmTable> {
    let bytes = bounded_read(path, MAX_TABLE)?;
    check_hash(
        "external T.82 fixture",
        &hex(&Sha256::digest(&bytes)),
        TABLE_SHA256,
    )?;
    let mut lines = std::str::from_utf8(&bytes)?.lines();
    if lines.next() != Some("T82-1993") || lines.next() != Some("113") {
        return Err("external T.82 fixture header is invalid".into());
    }
    let mut states = Vec::with_capacity(113);
    for _ in 0..113 {
        let fields: Vec<_> = lines
            .next()
            .ok_or("external T.82 state table is truncated")?
            .split_whitespace()
            .collect();
        if fields.len() != 4 {
            return Err("external T.82 state row is invalid".into());
        }
        let switch: u8 = fields[3].parse()?;
        if switch > 1 {
            return Err("external T.82 switch flag is invalid".into());
        }
        states.push(QmState {
            qe: fields[0].parse()?,
            next_lps: fields[1].parse()?,
            next_mps: fields[2].parse()?,
            switch_mps: switch == 1,
        });
    }
    if lines.next() != Some("3") || lines.count() != 6 {
        return Err("external T.82 fixture tail is invalid".into());
    }
    Ok(QmTable::new(states)?)
}

// The SHA-pinned catalog uses one JSON record per line. This parser accepts
// only that layout; a changed catalog needs a new digest and explicit review.
fn quoted(line: &str, key: &str) -> TestResult<String> {
    let marker = format!("\"{key}\":\"");
    let (_, rest) = line
        .split_once(&marker)
        .ok_or_else(|| format!("manifest is missing {key}"))?;
    let (value, _) = rest
        .split_once('"')
        .ok_or_else(|| format!("manifest has unterminated {key}"))?;
    if value.contains('\\') {
        return Err(format!("manifest escaped {key} is unsupported").into());
    }
    Ok(value.to_owned())
}

fn number(line: &str, key: &str) -> TestResult<u64> {
    let marker = format!("\"{key}\":");
    let (_, rest) = line
        .split_once(&marker)
        .ok_or_else(|| format!("manifest is missing {key}"))?;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return Err(format!("manifest has invalid {key}").into());
    }
    Ok(rest[..digits].parse()?)
}

fn catalog() -> TestResult<Vec<Sample>> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/conformance/jbig1_oracle.json");
    let bytes = bounded_read(&path, MAX_MANIFEST)?;
    check_hash(
        "#22 manifest",
        &hex(&Sha256::digest(&bytes)),
        MANIFEST_SHA256,
    )?;
    let mut samples: Vec<Sample> = Vec::new();
    let mut sample_ids = HashSet::new();
    let mut image_keys = HashSet::new();
    let mut id_hash = Sha256::new();
    let mut malformed = 0;
    for line in std::str::from_utf8(&bytes)?.lines() {
        if line.starts_with("    {\"id\":") {
            let id = quoted(line, "id")?;
            if !sample_ids.insert(id.clone()) {
                return Err("duplicate pinned source identity".into());
            }
            id_hash.update(id.as_bytes());
            id_hash.update([0]);
            samples.push(Sample {
                path: quoted(line, "path")?,
                source_sha: quoted(line, "source_sha256")?,
                images: Vec::new(),
            });
        } else if line.starts_with("      {\"page\":") {
            let sample_index = samples
                .len()
                .checked_sub(1)
                .ok_or("image precedes sample")?;
            let page = u32::try_from(number(line, "page")?)?;
            let image = u32::try_from(number(line, "image")?)?;
            if !image_keys.insert((sample_index, page, image)) {
                return Err("duplicate pinned image identity".into());
            }
            if quoted(line, "decoder_result")? != "PASS" {
                return Err("pinned image does not have a passing oracle".into());
            }
            samples[sample_index].images.push(Image {
                page,
                number: image,
                offset: number(line, "offset")?,
                length: number(line, "length")?,
                encoded_sha: quoted(line, "encoded_sha256")?,
                width: usize::try_from(number(line, "width")?)?,
                height: usize::try_from(number(line, "height")?)?,
                stride: usize::try_from(number(line, "stride")?)?,
                raw_sha: quoted(line, "raw_stride_sha256")?,
                visible_sha: quoted(line, "visible_bits_sha256")?,
            });
        } else if line.trim_start().starts_with("\"discovery_failures\":") {
            malformed += line.matches("{\"sample_id\":").count();
        }
    }
    check_hash("ordered source IDs", &hex(&id_hash.finalize()), IDS_SHA256)?;
    if samples.len() != EXPECTED_SOURCES
        || image_keys.len() != EXPECTED_IMAGES
        || malformed != EXPECTED_MALFORMED_DISCOVERIES
    {
        return Err(format!(
            "catalog counts differ: sources={} images={} malformed={malformed}",
            samples.len(),
            image_keys.len()
        )
        .into());
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
        return Err("unsafe catalog source path".into());
    }
    let source = root.join(path).canonicalize()?;
    if !source.starts_with(root) || !source.is_file() {
        return Err("source path escapes corpus or is not a file".into());
    }
    Ok(source)
}

fn pinned_variant(file: &mut File) -> TestResult<Variant> {
    let mut signature = [0; 8];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut signature)?;
    match signature {
        [0xc8, 0, 0, 0, ..] => Ok(Variant::C8),
        [b'H', b'N', 0, 0, 0x90, 0x01, 0, 0] => Ok(Variant::HnA),
        [b'H', b'N', 0, 0, 0xc8, 0, 0, 0] => Ok(Variant::HnB),
        _ => Err("pinned source has an unmeasured HN/C8 signature".into()),
    }
}

struct CountedSource {
    file: File,
    size: u64,
    largest_request: usize,
    bytes_read: u64,
}

impl CountedSource {
    fn open(path: &Path) -> TestResult<Self> {
        let file = File::open(path)?;
        let size = file.metadata()?.len();
        if size > MAX_SOURCE {
            return Err("source exceeds local 8 GiB bound".into());
        }
        Ok(Self {
            file,
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

    async fn read_at(&mut self, offset: u64, output: &mut [u8]) -> caj2pdf_core::Result<usize> {
        if output.len() > IO_CHUNK {
            return Err(caj2pdf_core::Error::LimitExceeded {
                resource: "corpus source request bytes",
                limit: IO_CHUNK as u64,
                attempted: output.len() as u64,
            });
        }
        self.largest_request = self.largest_request.max(output.len());
        self.file.seek(SeekFrom::Start(offset))?;
        let read = self.file.read(output)?;
        self.bytes_read += read as u64;
        Ok(read)
    }
}

struct BoundedPdf {
    file: File,
    written: u64,
}

impl BoundedPdf {
    fn create(path: &Path) -> TestResult<Self> {
        let file = OpenOptions::new().create_new(true).write(true).open(path)?;
        Ok(Self { file, written: 0 })
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
        let n = self.file.write(bytes)?;
        self.written += n as u64;
        Ok(n)
    }

    async fn flush(&mut self) -> caj2pdf_core::Result<()> {
        Ok(self.file.flush()?)
    }
}

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

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
                "caj2pdf-hn-pdf-{}-{}",
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

fn command_text(command: &mut Command, label: &str) -> TestResult<String> {
    const MAX_TOOL_TEXT: u64 = 32 * 1024;
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| format!("{label} is unavailable"))?;
    let mut bytes = Vec::new();
    child
        .stdout
        .take()
        .ok_or("tool stdout pipe is missing")?
        .take(MAX_TOOL_TEXT + 1)
        .read_to_end(&mut bytes)?;
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
        return Err(format!("tool output exceeds {limit}-byte raster bound").into());
    }
    Ok(size)
}

// P4 is one packed display-order row per scanline, where a set bit is black.
// The PDF's /Decode [1 0] makes black index 1; a synthetic test below checks
// that pdfimages preserves this polarity rather than assuming it.
fn pbm_header(file: &mut File) -> TestResult<(usize, usize, u64)> {
    file.seek(SeekFrom::Start(0))?;
    let mut fields = Vec::with_capacity(3);
    let mut token = Vec::new();
    let mut comment = false;
    loop {
        if file.stream_position()? > 4096 {
            return Err("PBM header exceeds 4096 bytes".into());
        }
        let mut byte = [0];
        file.read_exact(&mut byte)?;
        let byte = byte[0];
        if comment {
            if byte == b'\n' {
                comment = false;
            }
            continue;
        }
        if token.is_empty() && byte == b'#' {
            comment = true;
            continue;
        }
        if byte.is_ascii_whitespace() {
            if !token.is_empty() {
                fields.push(String::from_utf8(std::mem::take(&mut token))?);
                if fields.len() == 3 {
                    break;
                }
            }
        } else {
            token.push(byte);
        }
    }
    if fields[0] != "P4" {
        return Err("pdfimages did not emit binary PBM".into());
    }
    Ok((
        fields[1].parse()?,
        fields[2].parse()?,
        file.stream_position()?,
    ))
}

fn pbm_geometry(file: &mut File, width: usize, height: usize) -> TestResult<(usize, u64)> {
    let (actual_width, actual_height, start) = pbm_header(file)?;
    if (actual_width, actual_height) != (width, height) {
        return Err(format!(
            "PBM geometry differs: {actual_width}x{actual_height} versus {width}x{height}"
        )
        .into());
    }
    let row_bytes = width.div_ceil(8);
    let expected = start
        .checked_add(u64::try_from(
            row_bytes.checked_mul(height).ok_or("PBM size overflows")?,
        )?)
        .ok_or("PBM size overflows")?;
    if file.metadata()?.len() != expected {
        return Err("PBM raster byte count differs from geometry".into());
    }
    Ok((row_bytes, start))
}

fn read_row(file: &mut File, start: u64, row: usize, buffer: &mut [u8]) -> TestResult<()> {
    let offset = start
        .checked_add(u64::try_from(
            row.checked_mul(buffer.len())
                .ok_or("row offset overflows")?,
        )?)
        .ok_or("row offset overflows")?;
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(buffer)?;
    Ok(())
}

fn mask(width: usize) -> u8 {
    if width % 8 == 0 {
        0xff
    } else {
        0xff << (8 - width % 8)
    }
}

struct RasterHashes {
    visible: String,
    raw: String,
    black_pixels: u64,
}

fn oracle_hashes(pbm: &Path, image: &Image) -> TestResult<RasterHashes> {
    bounded_file_size(pbm, MAX_RASTER)?;
    if image.width == 0 || image.height == 0 || image.width > 10_000 || image.height > 20_000 {
        return Err("pinned image geometry exceeds local bounds".into());
    }
    let stride = image.width.checked_add(31).ok_or("stride overflows")? / 32 * 4;
    if image.stride != stride
        || u64::try_from(
            stride
                .checked_mul(image.height)
                .ok_or("raw length overflows")?,
        )? > MAX_RASTER
    {
        return Err("pinned DIB stride or raw length is invalid".into());
    }
    let mut file = File::open(pbm)?;
    let (visible_bytes, start) = pbm_geometry(&mut file, image.width, image.height)?;
    let mut row = vec![0; visible_bytes];
    let padding = [0_u8; 4];
    let mut raw = Sha256::new();
    let mut visible = Sha256::new();
    let mut black_pixels = 0_u64;
    // The #22 DIB-memory oracle hashes bottom-up rows. pdfimages exports
    // top-down display rows. Reverse full rows, keep visible bits, and append
    // zero bytes to each row's 32-bit DIB stride before hashing.
    for y in (0..image.height).rev() {
        read_row(&mut file, start, y, &mut row)?;
        let final_byte = row.last_mut().ok_or("empty PBM row")?;
        *final_byte &= mask(image.width);
        black_pixels += row.iter().map(|byte| byte.count_ones() as u64).sum::<u64>();
        visible.update(&row);
        raw.update(&row);
        raw.update(&padding[..stride - visible_bytes]);
    }
    Ok(RasterHashes {
        visible: hex(&visible.finalize()),
        raw: hex(&raw.finalize()),
        black_pixels,
    })
}

fn same_visible_pixels(
    expected: &Path,
    rendered: &Path,
    width: usize,
    height: usize,
    factor: usize,
) -> TestResult<()> {
    bounded_file_size(rendered, MAX_RASTER)?;
    let mut reference = File::open(expected)?;
    let (reference_stride, reference_start) = pbm_geometry(&mut reference, width, height)?;
    let mut candidate = File::open(rendered)?;
    let rendered_width = width.checked_mul(factor).ok_or("render width overflows")?;
    let rendered_height = height
        .checked_mul(factor)
        .ok_or("render height overflows")?;
    let (candidate_stride, candidate_start) =
        pbm_geometry(&mut candidate, rendered_width, rendered_height)?;
    let mut want = vec![0; reference_stride];
    let mut got = vec![0; candidate_stride];
    for y in 0..height {
        read_row(&mut reference, reference_start, y, &mut want)?;
        read_row(
            &mut candidate,
            candidate_start,
            y * factor + factor / 2,
            &mut got,
        )?;
        for x in 0..width {
            let a = want[x / 8] & (0x80 >> (x % 8)) != 0;
            let centre = x * factor + factor / 2;
            let b = got[centre / 8] & (0x80 >> (centre % 8)) != 0;
            if a != b {
                return Err(format!("render pixel differs at ({x},{y})").into());
            }
        }
    }
    Ok(())
}

struct RenderStorage {
    largest_file: u64,
    combined: u64,
}

fn render_independently(
    temp: &PrivateTemp,
    pdf: &Path,
    extracted: &Path,
    image: &Image,
) -> TestResult<RenderStorage> {
    let poppler_root = temp.path("poppler");
    command_quiet(
        Command::new("pdftoppm")
            .args(["-mono", "-r", "720", "-singlefile", "-f", "1", "-l", "1"])
            .arg(pdf)
            .arg(&poppler_root),
        "Poppler pdftoppm",
    )?;
    let poppler = poppler_root.with_extension("pbm");
    let poppler_size = bounded_file_size(&poppler, MAX_RASTER)?;
    same_visible_pixels(extracted, &poppler, image.width, image.height, 10)?;

    let mupdf = temp.path("mupdf.pbm");
    command_quiet(
        Command::new("mutool")
            .args(["draw", "-q", "-r", "72", "-o"])
            .arg(&mupdf)
            .arg(pdf)
            .arg("1"),
        "MuPDF mutool draw",
    )?;
    let mupdf_size = bounded_file_size(&mupdf, MAX_RASTER)?;
    same_visible_pixels(extracted, &mupdf, image.width, image.height, 1)?;
    Ok(RenderStorage {
        largest_file: poppler_size.max(mupdf_size),
        combined: poppler_size + mupdf_size,
    })
}

fn ensure_bilevel_black_one(pdf: &Path) -> TestResult<()> {
    let mut file = File::open(pdf)?;
    let mut buffer = [0_u8; IO_CHUNK];
    let needles: [&[u8]; 3] = [
        b"/Decode [1 0]",
        b"/ColorSpace /DeviceGray",
        b"/BitsPerComponent 1",
    ];
    let mut tail = Vec::new();
    let mut counts = [0; 3];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        let retained = tail.len();
        tail.extend_from_slice(&buffer[..n]);
        for (count, needle) in counts.iter_mut().zip(needles) {
            *count += tail
                .windows(needle.len())
                .enumerate()
                .filter(|(start, item)| *start + needle.len() > retained && *item == needle)
                .count();
        }
        let retain = needles.iter().map(|needle| needle.len()).max().unwrap() - 1;
        let split = tail.len().saturating_sub(retain);
        tail.drain(..split);
    }
    if counts != [1, 1, 1] {
        return Err(
            format!("expected one black-is-one bilevel XObject; marker counts={counts:?}").into(),
        );
    }
    Ok(())
}

struct ImageRun {
    max_request: usize,
    max_pdf: u64,
    max_pbm: u64,
    max_render: u64,
    temp_storage: u64,
    accounted_scratch_bound: u64,
    rendered: bool,
    blank: bool,
}

fn run_image(
    path: &Path,
    source_size: u64,
    image: &Image,
    table: &QmTable,
    expected_variant: Variant,
    render: bool,
) -> TestResult<ImageRun> {
    let end = image
        .offset
        .checked_add(image.length)
        .ok_or("encoded span overflows")?;
    if image.length < 48 || image.length > MAX_ENCODED || end > source_size {
        return Err("encoded span exceeds source or local bound".into());
    }
    let mut verifier = File::open(path)?;
    check_hash(
        "encoded span before PDF conversion",
        &hash_span(&mut verifier, image.offset, image.length)?,
        &image.encoded_sha,
    )?;
    let temp = PrivateTemp::new()?;
    let pdf = temp.path("selected.pdf");
    let mut source = CountedSource::open(path)?;
    let mut sink = BoundedPdf::create(&pdf)?;
    let limits = Limits {
        io_chunk_bytes: IO_CHUNK,
        max_input_bytes: MAX_SOURCE,
        max_output_bytes: MAX_PDF,
        max_allocation_bytes: 64 * 1024 * 1024,
        max_pages: 100_000,
        max_bookmarks: 100_000,
    };
    // One image pixel equals one PDF point, so a 72 dpi render is 1:1.
    let mut options = Type0PdfOptions {
        pixels_per_inch: 72.0,
        ..Type0PdfOptions::default()
    };
    options.image.max_width = 10_000;
    options.image.max_height = 20_000;
    options.image.max_pixels = 12_000_000;
    options.arithmetic.max_symbols = 12_020_000;
    options.arithmetic.max_work = 12_020_000 * 32 + 1024;
    let report = ready(convert_type0_image_pdf(
        &mut source,
        &mut sink,
        table,
        Type0ImageSelection {
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
        || report.image.record_type != 0
        || report.image.payload.offset != image.offset
        || report.image.payload.length != image.length
        || report.source_variant != expected_variant
        || report.source_pages < image.page
        || report.conversion.pages_converted != 1
        || source.bytes_read != report.conversion.input_bytes_read
    {
        return Err(
            "selected reader record, span, page count, or input counter differs from catalog"
                .into(),
        );
    }
    if sink.written != report.conversion.output_bytes_written {
        return Err("PDF sink byte count differs from converter report".into());
    }
    drop(sink);
    let pdf_size = bounded_file_size(&pdf, MAX_PDF)?;
    ensure_bilevel_black_one(&pdf)?;
    command_quiet(
        Command::new("qpdf").arg("--check").arg(&pdf),
        "qpdf --check",
    )?;
    let pages = command_text(
        Command::new("qpdf").arg("--show-npages").arg(&pdf),
        "qpdf page count",
    )?;
    if pages.trim() != "1" {
        return Err("independent PDF page count differs from one".into());
    }
    let info = command_text(Command::new("pdfinfo").arg(&pdf), "Poppler pdfinfo")?;
    let page_size = info
        .lines()
        .find(|line| line.starts_with("Page size:"))
        .ok_or("pdfinfo did not report page geometry")?;
    let dimensions: Vec<_> = page_size.split_whitespace().collect();
    if dimensions.len() < 5
        || dimensions[2].parse::<f64>()? != image.width as f64
        || dimensions[3] != "x"
        || dimensions[4].parse::<f64>()? != image.height as f64
    {
        return Err("independent PDF page geometry differs from source pixels at 72 ppi".into());
    }
    let listing = command_text(
        Command::new("pdfimages").arg("-list").arg(&pdf),
        "pdfimages -list",
    )?;
    let records: Vec<_> = listing
        .lines()
        .filter(|line| line.trim_start().starts_with("1 "))
        .collect();
    if records.len() != 1 {
        return Err("PDF must contain exactly one independently listed image".into());
    }
    let fields: Vec<_> = records[0].split_whitespace().collect();
    if fields.len() < 8
        || fields[1] != "0"
        || fields[2] != "image"
        || fields[3].parse::<usize>()? != image.width
        || fields[4].parse::<usize>()? != image.height
        || fields[5] != "gray"
        || fields[6] != "1"
        || fields[7] != "1"
    {
        return Err("PDF image is not the selected one-bit grayscale geometry".into());
    }
    let root = temp.path("extracted");
    command_quiet(Command::new("pdfimages").arg(&pdf).arg(&root), "pdfimages")?;
    let pbm = temp.path("extracted-000.pbm");
    let pbm_count = fs::read_dir(&temp.0)?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("extracted-")
        })
        .count();
    if pbm_count != 1 || !pbm.is_file() {
        return Err("pdfimages did not extract exactly one bilevel image".into());
    }
    let pbm_size = bounded_file_size(&pbm, MAX_RASTER)?;
    let hashes = oracle_hashes(&pbm, image)?;
    check_hash("PDF visible bits", &hashes.visible, &image.visible_sha)?;
    check_hash("PDF raw-stride rows", &hashes.raw, &image.raw_sha)?;
    // Blank images are valid #22 controls. Select a nonblank fixed canary for
    // each independent renderer while still matching every blank oracle hash.
    let rendered = render && hashes.black_pixels != 0;
    let render_storage = if rendered {
        render_independently(&temp, &pdf, &pbm, image)?
    } else {
        RenderStorage {
            largest_file: 0,
            combined: 0,
        }
    };
    let temp_storage = pdf_size + pbm_size + render_storage.combined;
    if temp_storage > MAX_PDF + 3 * MAX_RASTER {
        return Err("one-image temporary storage exceeds local bound".into());
    }
    // Conservative accounting for named row/I/O buffers, context states,
    // and QM lookahead; process VmHWM below captures all other allocations.
    let visible_bytes = image.width.div_ceil(8) as u64;
    let render_row_bytes = image
        .width
        .checked_mul(10)
        .ok_or("render row overflows")?
        .div_ceil(8) as u64;
    let accounted_scratch_bound = IO_CHUNK as u64
        + 3 * image.stride as u64
        + 3 * visible_bytes
        + render_row_bytes
        + 1024 * std::mem::size_of::<ContextState>() as u64
        + 113 * std::mem::size_of::<QmState>() as u64
        + 256
        + 4096;
    check_hash(
        "encoded span after PDF conversion",
        &hash_span(&mut verifier, image.offset, image.length)?,
        &image.encoded_sha,
    )?;
    Ok(ImageRun {
        max_request: source.largest_request,
        max_pdf: pdf_size,
        max_pbm: pbm_size,
        max_render: render_storage.largest_file,
        temp_storage,
        accounted_scratch_bound,
        rendered,
        blank: hashes.black_pixels == 0,
    })
}

fn peak_rss_kib() -> TestResult<u64> {
    #[cfg(target_os = "linux")]
    {
        let status = fs::read_to_string("/proc/self/status")?;
        let line = status
            .lines()
            .find(|line| line.starts_with("VmHWM:"))
            .ok_or("Linux VmHWM is absent")?;
        Ok(line
            .split_whitespace()
            .nth(1)
            .ok_or("Linux VmHWM is invalid")?
            .parse()?)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(0)
    }
}

fn unsupported_error(error: &(dyn Error + 'static)) -> bool {
    error
        .downcast_ref::<Type0PdfError>()
        .is_some_and(|conversion| {
            matches!(&conversion.kind, Type0PdfErrorKind::UnsupportedImageType(_))
                || matches!(&conversion.kind, Type0PdfErrorKind::Container(source)
                if matches!(&source.kind, Hnc8ErrorKind::Unsupported { .. }))
                || matches!(&conversion.kind, Type0PdfErrorKind::Image(source)
                if matches!(&source.kind, Type0ErrorKind::Unsupported { .. }))
        })
}

#[test]
fn optional_corpus_is_not_run_by_default() {
    println!("NOT_RUN\tchecked=0\tmatched=0\tunsupported=0\tskipped=0\tfailed=0");
}

#[test]
#[ignore = "requires explicit private CAJ2PDF_CORPUS_DIR and CAJ2PDF_T82_VECTOR_FILE"]
fn all_pinned_type0_pdf_images() {
    run_all().unwrap_or_else(|error| panic!("HN/C8 PDF corpus parity failed: {error}"));
}

fn run_all() -> TestResult<()> {
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
    let table = external_table(&fixture)?;
    let samples = catalog()?;
    println!(
        "START\texpected_sources={EXPECTED_SOURCES}\texpected_images={EXPECTED_IMAGES}\tmanifest_sha256={MANIFEST_SHA256}\ttable_sha256={TABLE_SHA256}"
    );

    let mut matched = 0;
    let mut failed = 0;
    let mut skipped = 0;
    let mut unsupported = 0;
    let mut sources_before = 0;
    let mut sources_after = 0;
    let mut max_request = 0;
    let mut max_pdf = 0;
    let mut max_pbm = 0;
    let mut max_render = 0;
    let mut max_temp_storage = 0;
    let mut max_accounted_scratch = 0;
    let mut hn_rendered = false;
    let mut c8_rendered = false;
    let mut multi_rendered = false;
    let mut checked = 0;
    let mut blank = 0;
    let mut multi_pages_checked = 0;
    let mut matched_keys = HashSet::new();
    for (sample_index, sample) in samples.iter().enumerate() {
        let verified = (|| -> TestResult<(PathBuf, u64, Variant)> {
            let path = source_path(&corpus, &sample.path)?;
            let mut file = File::open(&path)?;
            let size = file.metadata()?.len();
            if size > MAX_SOURCE {
                return Err("source exceeds local bound".into());
            }
            check_hash(
                "source before PDF conversions",
                &hash_span(&mut file, 0, size)?,
                &sample.source_sha,
            )?;
            let variant = pinned_variant(&mut file)?;
            Ok((path, size, variant))
        })();
        if verified.is_ok() {
            sources_before += 1;
        }
        let is_c8 = matches!(&verified, Ok((_, _, Variant::C8)));
        let multi_pages: HashSet<_> = sample
            .images
            .iter()
            .filter_map(|image| {
                (sample
                    .images
                    .iter()
                    .filter(|candidate| candidate.page == image.page)
                    .count()
                    > 1)
                .then_some(image.page)
            })
            .collect();
        for image in &sample.images {
            checked += 1;
            let key = (sample_index, image.page, image.number);
            let render_hn = key == HN_RENDER && !is_c8;
            let render_c8 = key == C8_RENDER && is_c8;
            let render_multi = key == MULTI_RENDER && multi_pages.contains(&image.page);
            let render = render_hn || render_c8 || render_multi;
            let result = match &verified {
                Ok((path, size, variant)) => {
                    run_image(path, *size, image, &table, *variant, render)
                }
                Err(_) => {
                    skipped += 1;
                    println!(
                        "IMAGE_SKIP\tsource_index={sample_index}\tpage={}\timage={}",
                        image.page, image.number
                    );
                    continue;
                }
            };
            match result {
                Ok(bounds) => {
                    matched += 1;
                    max_request = max_request.max(bounds.max_request);
                    max_pdf = max_pdf.max(bounds.max_pdf);
                    max_pbm = max_pbm.max(bounds.max_pbm);
                    max_render = max_render.max(bounds.max_render);
                    max_temp_storage = max_temp_storage.max(bounds.temp_storage);
                    max_accounted_scratch =
                        max_accounted_scratch.max(bounds.accounted_scratch_bound);
                    hn_rendered |= render_hn && bounds.rendered;
                    c8_rendered |= render_c8 && bounds.rendered;
                    multi_rendered |= render_multi && bounds.rendered;
                    blank += usize::from(bounds.blank);
                    matched_keys.insert((sample_index, image.page, image.number));
                }
                Err(error) => {
                    if unsupported_error(error.as_ref()) {
                        unsupported += 1;
                        println!(
                            "IMAGE_UNSUPPORTED\tsource_index={sample_index}\tpage={}\timage={}",
                            image.page, image.number
                        );
                    } else {
                        failed += 1;
                        println!(
                            "IMAGE_FAIL\tsource_index={sample_index}\tpage={}\timage={}\t{error}",
                            image.page, image.number
                        );
                    }
                }
            }
            if checked % 100 == 0 {
                println!("PROGRESS\tchecked={checked}\tmatched={matched}\tfailed={failed}");
            }
        }
        multi_pages_checked += multi_pages
            .iter()
            .filter(|page| {
                sample
                    .images
                    .iter()
                    .filter(|image| image.page == **page)
                    .all(|image| matched_keys.contains(&(sample_index, image.page, image.number)))
            })
            .count();
        match verified {
            Ok((path, size, _)) => match verify_source_unchanged(&path, size, &sample.source_sha) {
                Ok(()) => sources_after += 1,
                Err(error) => {
                    failed += 1;
                    println!("SOURCE_FAIL\tsource_index={sample_index}\tchanged_after=1\t{error}");
                }
            },
            Err(_) => {
                println!("SOURCE_FAIL\tsource_index={sample_index}\tidentity_unverified=1");
            }
        }
    }
    check_hash(
        "T.82 fixture after PDF conversions",
        &hex(&Sha256::digest(bounded_read(&fixture, MAX_TABLE)?)),
        TABLE_SHA256,
    )?;
    check_hash(
        "#22 manifest after PDF conversions",
        &hex(&Sha256::digest(bounded_read(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/conformance/jbig1_oracle.json"),
            MAX_MANIFEST,
        )?)),
        MANIFEST_SHA256,
    )?;
    let peak_rss = peak_rss_kib()?;
    println!(
        "SUMMARY\tchecked={checked}\tmatched={matched}\tblank={blank}\tfailed={failed}\tskipped={skipped}\tunsupported={unsupported}\tsources_before={sources_before}\tsources_after={sources_after}\tmalformed_discoveries={EXPECTED_MALFORMED_DISCOVERIES}\tmulti_pages_checked={multi_pages_checked}\thn_rendered={hn_rendered}\tc8_rendered={c8_rendered}\tmulti_rendered={multi_rendered}\tmax_request_bytes={max_request}\tmax_pdf_bytes={max_pdf}\tmax_pbm_bytes={max_pbm}\tmax_render_bytes={max_render}\tmax_temp_storage_bytes={max_temp_storage}\tmax_accounted_row_io_scratch_bytes={max_accounted_scratch}\tprocess_vm_hwm_kib={peak_rss}"
    );
    if checked != EXPECTED_IMAGES
        || matched != EXPECTED_IMAGES
        || failed != 0
        || skipped != 0
        || unsupported != 0
        || sources_before != EXPECTED_SOURCES
        || sources_after != EXPECTED_SOURCES
        || blank != 2
        || multi_pages_checked != 6
        || !hn_rendered
        || !c8_rendered
        || !multi_rendered
    {
        return Err("pinned PDF parity totals or independent renders are incomplete".into());
    }
    Ok(())
}

#[test]
fn pdfimages_p4_preserves_black_one_and_row_order() {
    let temp = PrivateTemp::new().unwrap();
    let pdf = temp.path("synthetic.pdf");
    let mut sink = BoundedPdf::create(&pdf).unwrap();
    let limits = Limits::default();
    ready(async {
        let mut document = PdfDocument::new(&mut sink, &limits, &NeverCancel).await?;
        let mut rows = document
            .begin_bilevel_image(BilevelImageSpec {
                pixel_width: 9,
                pixel_height: 2,
                row_stride: 4,
            })
            .await?;
        // Top row: black at x=0 and x=8. Bottom row: black at x=7.
        rows.write(&[0x80, 0x80, 0, 0, 0x01, 0, 0, 0]).await?;
        let object = rows.finish().await?;
        document
            .add_page(
                PageSpec {
                    width_points: 9.0,
                    height_points: 2.0,
                },
                &[object],
            )
            .await?;
        document.finish().await
    })
    .unwrap();
    ensure_bilevel_black_one(&pdf).unwrap();
    command_quiet(
        Command::new("qpdf").arg("--check").arg(&pdf),
        "qpdf --check",
    )
    .unwrap();
    let root = temp.path("synthetic-image");
    command_quiet(Command::new("pdfimages").arg(&pdf).arg(&root), "pdfimages").unwrap();
    let pbm = temp.path("synthetic-image-000.pbm");
    let mut file = File::open(&pbm).unwrap();
    let (stride, start) = pbm_geometry(&mut file, 9, 2).unwrap();
    assert_eq!(stride, 2);
    let mut top = [0; 2];
    let mut bottom = [0; 2];
    read_row(&mut file, start, 0, &mut top).unwrap();
    read_row(&mut file, start, 1, &mut bottom).unwrap();
    assert_eq!(top, [0x80, 0x80]);
    assert_eq!(bottom, [0x01, 0]);

    let image = Image {
        page: 1,
        number: 1,
        offset: 0,
        length: 0,
        encoded_sha: String::new(),
        width: 9,
        height: 2,
        stride: 4,
        // DIB memory order is bottom-up, with zero 32-bit row padding.
        raw_sha: hex(&Sha256::digest([0x01, 0, 0, 0, 0x80, 0x80, 0, 0])),
        visible_sha: hex(&Sha256::digest([0x01, 0, 0x80, 0x80])),
    };
    let hashes = oracle_hashes(&pbm, &image).unwrap();
    assert_eq!(hashes.raw, image.raw_sha);
    assert_eq!(hashes.visible, image.visible_sha);
    assert_eq!(hashes.black_pixels, 3);
    let rendered = render_independently(&temp, &pdf, &pbm, &image).unwrap();
    assert!(rendered.largest_file > 0);

    // PBM's unused low seven bits do not belong to the visible image. Their
    // values cannot affect the canonical oracle hash or the zero DIB padding.
    let noisy = temp.path("noisy-unused-bits.pbm");
    fs::copy(&pbm, &noisy).unwrap();
    let mut file = OpenOptions::new().write(true).open(&noisy).unwrap();
    file.seek(SeekFrom::Start(start + 1)).unwrap();
    file.write_all(&[0xff]).unwrap();
    file.seek(SeekFrom::Start(start + 3)).unwrap();
    file.write_all(&[0x7f]).unwrap();
    drop(file);
    let masked = oracle_hashes(&noisy, &image).unwrap();
    assert_eq!(masked.raw, image.raw_sha);
    assert_eq!(masked.visible, image.visible_sha);
}

#[test]
fn post_conversion_source_check_rejects_appended_and_changed_bytes() {
    let temp = PrivateTemp::new().unwrap();
    let path = temp.path("synthetic-source");
    fs::write(&path, b"synthetic source").unwrap();
    let original_size = path.metadata().unwrap().len();
    let original_sha = hex(&Sha256::digest(b"synthetic source"));
    verify_source_unchanged(&path, original_size, &original_sha).unwrap();

    OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"!")
        .unwrap();
    assert!(
        verify_source_unchanged(&path, original_size, &original_sha)
            .unwrap_err()
            .to_string()
            .contains("length changed")
    );

    fs::write(&path, b"synthetic sourcE").unwrap();
    assert!(
        verify_source_unchanged(&path, original_size, &original_sha)
            .unwrap_err()
            .to_string()
            .contains("SHA-256 mismatch")
    );
}

#[test]
fn unsupported_classification_includes_type0_wrapper_fields() {
    let error = Type0PdfError {
        page: Some(1),
        image: Some(1),
        offset: Some(48),
        kind: Type0PdfErrorKind::Image(Box::new(Type0Error {
            offset: 48,
            rows_written: 0,
            output_bytes_written: 0,
            kind: Type0ErrorKind::Unsupported {
                field: "DIB compression",
                value: 1,
            },
        })),
    };
    assert!(unsupported_error(&error));
    let not_unsupported = Type0PdfError {
        page: Some(1),
        image: None,
        offset: Some(0),
        kind: Type0PdfErrorKind::NoImages,
    };
    assert!(!unsupported_error(&not_unsupported));
}
