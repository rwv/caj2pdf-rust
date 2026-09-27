// SPDX-License-Identifier: MIT

//! Native one-case adapter for the opt-in #106 SHA-pinned PDF parity harness.
//! It receives a caller-held T.88 table and source path; no private data is
//! embedded. `scripts/jbig2_page_pdf_parity.py` verifies all input identities.

#[path = "support/mod.rs"]
#[allow(dead_code)]
mod support;

use caj2pdf_core::{
    Error, Limits, NeverCancel, RangedSource, SequentialSink,
    hnc8::{
        Type3ImageSelection, Type3PdfErrorKind, Type3PdfOptions, Type3RefinedStore, Type3Stage,
        Type3Store, Type3Workspaces, convert_type3_image_pdf,
    },
    jbig2::{
        text::{TextHeaderPolicy, TextRegionError, TextRegionErrorKind},
        text_composer::RandomAccessScratch,
    },
    native::{SeekableSource, WriteSink},
};
use std::{
    env,
    error::Error as StdError,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
use support::{TempStore, peak_rss_kib, ready, table};

const MAX_SOURCE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_PDF_BYTES: u64 = 64 * 1024 * 1024;
const CHUNK: usize = 64 * 1024;
const STRICT: &str = "strict";
const OPT_IN: &str = "hn-c8-unused-refinement-template";

struct MeteredSource<S> {
    inner: S,
    bytes: u64,
    largest_request: usize,
}

impl<S: RangedSource> RangedSource for MeteredSource<S> {
    fn size(&self) -> u64 {
        self.inner.size()
    }

    async fn read_at(&mut self, offset: u64, output: &mut [u8]) -> caj2pdf_core::Result<usize> {
        if output.len() > CHUNK {
            return Err(Error::LimitExceeded {
                resource: "private diagnostic ranged request bytes",
                limit: CHUNK as u64,
                attempted: output.len() as u64,
            });
        }
        let got = self.inner.read_at(offset, output).await?;
        if got > output.len() {
            return Err(Error::InvalidInput {
                reason: "source overreported a ranged read",
            });
        }
        self.bytes = self
            .bytes
            .checked_add(got as u64)
            .ok_or(Error::InvalidInput {
                reason: "source byte counter overflowed",
            })?;
        if self.bytes > MAX_SOURCE_BYTES {
            return Err(Error::LimitExceeded {
                resource: "private diagnostic source bytes",
                limit: MAX_SOURCE_BYTES,
                attempted: self.bytes,
            });
        }
        self.largest_request = self.largest_request.max(output.len());
        Ok(got)
    }
}

/// A second file handle observes appended dictionary bytes during decoding.
struct GrowingFileSource(File);

impl RangedSource for GrowingFileSource {
    fn size(&self) -> u64 {
        self.0.metadata().map_or(0, |metadata| metadata.len())
    }

    async fn read_at(&mut self, offset: u64, output: &mut [u8]) -> caj2pdf_core::Result<usize> {
        if output.len() > CHUNK {
            return Err(Error::LimitExceeded {
                resource: "private scratch read request bytes",
                limit: CHUNK as u64,
                attempted: output.len() as u64,
            });
        }
        self.0.seek(SeekFrom::Start(offset))?;
        Ok(self.0.read(output)?)
    }
}

struct FileScratch(File);

impl RandomAccessScratch for FileScratch {
    fn size(&self) -> caj2pdf_core::Result<u64> {
        Ok(self.0.metadata()?.len())
    }

    async fn set_len(&mut self, bytes: u64) -> caj2pdf_core::Result<()> {
        self.0.set_len(bytes)?;
        Ok(())
    }

    async fn read_at(&mut self, offset: u64, output: &mut [u8]) -> caj2pdf_core::Result<usize> {
        if output.len() > CHUNK {
            return Err(Error::LimitExceeded {
                resource: "private scratch read request bytes",
                limit: CHUNK as u64,
                attempted: output.len() as u64,
            });
        }
        self.0.seek(SeekFrom::Start(offset))?;
        Ok(self.0.read(output)?)
    }

    async fn write_at(&mut self, offset: u64, bytes: &[u8]) -> caj2pdf_core::Result<usize> {
        if bytes.len() > CHUNK {
            return Err(Error::LimitExceeded {
                resource: "private scratch write request bytes",
                limit: CHUNK as u64,
                attempted: bytes.len() as u64,
            });
        }
        self.0.seek(SeekFrom::Start(offset))?;
        Ok(self.0.write(bytes)?)
    }

    async fn flush(&mut self) -> caj2pdf_core::Result<()> {
        self.0.flush()?;
        self.0.sync_data()?;
        Ok(())
    }
}

fn one_based(value: &str, name: &str) -> Result<u32, Box<dyn StdError>> {
    let parsed: u32 = value.parse()?;
    if parsed == 0 {
        return Err(format!("{name} must be one based").into());
    }
    Ok(parsed)
}

struct Args {
    fixture: PathBuf,
    source: PathBuf,
    pdf: PathBuf,
    page: u32,
    image: u32,
    policy: &'static str,
}

fn arguments() -> Result<Args, Box<dyn StdError>> {
    let mut args = env::args_os().skip(1);
    let fixture = PathBuf::from(args.next().ok_or("missing private table path")?);
    let source = PathBuf::from(args.next().ok_or("missing source path")?);
    let pdf = PathBuf::from(args.next().ok_or("missing PDF output path")?);
    let page = one_based(
        &args.next().ok_or("missing page")?.to_string_lossy(),
        "page",
    )?;
    let image = one_based(
        &args.next().ok_or("missing image")?.to_string_lossy(),
        "image",
    )?;
    let policy = args.next().ok_or("missing header policy")?;
    if args.next().is_some() {
        return Err("unexpected extra diagnostic argument".into());
    }
    let policy = match policy.to_string_lossy().as_ref() {
        STRICT => STRICT,
        OPT_IN => OPT_IN,
        _ => return Err("unknown text-header policy".into()),
    };
    Ok(Args {
        fixture,
        source,
        pdf,
        page,
        image,
        policy,
    })
}

fn output_file(path: &Path) -> Result<File, Box<dyn StdError>> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

fn run() -> Result<(), Box<dyn StdError>> {
    let Args {
        fixture,
        source: source_path,
        pdf: pdf_path,
        page,
        image,
        policy,
    } = arguments()?;
    let source_file = File::open(&source_path)?;
    let source_size = source_file.metadata()?.len();
    if source_size == 0 || source_size > 8 * 1024 * 1024 * 1024 {
        return Err("source size exceeds private diagnostic bound".into());
    }
    let limits = Limits {
        io_chunk_bytes: CHUNK,
        max_input_bytes: MAX_SOURCE_BYTES,
        max_output_bytes: MAX_PDF_BYTES,
        max_allocation_bytes: 64 * 1024 * 1024,
        max_pages: 100_000,
        max_bookmarks: 100_000,
    };
    let mq = table(&fixture, &limits)?;
    let (first_store, first_file) = TempStore::create("caj2pdf-type3-first")?;
    let (second_store, second_file) = TempStore::create("caj2pdf-type3-second")?;
    let (refined_store, refined_file) = TempStore::create("caj2pdf-type3-refined")?;
    let (text_store, text_file) = TempStore::create("caj2pdf-type3-text")?;
    drop(text_file);
    let mut first_reader = GrowingFileSource(File::open(first_store.path())?);
    let mut first_compose_reader = GrowingFileSource(File::open(first_store.path())?);
    let mut second_reader = GrowingFileSource(File::open(second_store.path())?);
    let mut second_compose_reader = GrowingFileSource(File::open(second_store.path())?);
    let mut refined_reader = GrowingFileSource(File::open(refined_store.path())?);
    let mut first_writer = WriteSink::new(first_file);
    let mut second_writer = WriteSink::new(second_file);
    let mut refined_writer = WriteSink::new(refined_file);
    let mut text_scratch = FileScratch(
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(text_store.path())?,
    );
    let mut workspaces = Type3Workspaces {
        first: Type3Store {
            reader: &mut first_reader,
            compose_reader: &mut first_compose_reader,
            writer: &mut first_writer,
        },
        second: Type3Store {
            reader: &mut second_reader,
            compose_reader: &mut second_compose_reader,
            writer: &mut second_writer,
        },
        refined: Type3RefinedStore {
            reader: &mut refined_reader,
            writer: &mut refined_writer,
        },
        text: &mut text_scratch,
    };
    let mut source = MeteredSource {
        inner: SeekableSource::new(source_file)?,
        bytes: 0,
        largest_request: 0,
    };
    let mut options = Type3PdfOptions {
        pixels_per_inch: 72.0,
        ..Type3PdfOptions::default()
    };
    if policy == OPT_IN {
        options.text_header_policy = TextHeaderPolicy::HnC8UnusedRefinementTemplate;
    }
    let mut pdf = WriteSink::new(output_file(&pdf_path)?);
    let result = ready(convert_type3_image_pdf(
        &mut source,
        &mut pdf,
        &mq,
        &mut workspaces,
        Type3ImageSelection {
            page_number: page,
            image_number: image,
        },
        options,
        &limits,
        &NeverCancel,
    ));
    let scratch_bytes = first_store.path().metadata()?.len()
        + second_store.path().metadata()?.len()
        + refined_store.path().metadata()?.len()
        + text_store.path().metadata()?.len();
    let rss = peak_rss_kib().unwrap_or(0);
    match result {
        Ok(report) => {
            ready(pdf.flush())?;
            drop(pdf);
            let pdf_bytes = fs::metadata(&pdf_path)?.len();
            if report.conversion.output_bytes_written != pdf_bytes
                || report.conversion.input_bytes_read != source.bytes
                || report.conversion.pages_converted != 1
            {
                return Err("type-3 PDF report byte or page counts disagree".into());
            }
            let marker = if report.text_header_anomaly.is_some() {
                "HN_C8_UNUSED_REFINEMENT_TEMPLATE"
            } else {
                "-"
            };
            let variant = report.source_variant.as_str();
            println!(
                "CASE\tCOMPLETE\t{page}\t{image}\t{variant}\t{}\t{}\t{}\t{}\t{}\t{pdf_bytes}\t{}\t{scratch_bytes}\t{rss}\t{marker}\t-\t0",
                report.page.width,
                report.page.height,
                report.image.payload.offset,
                report.image.payload.length,
                source.bytes,
                source.largest_request,
            );
        }
        Err(error) => {
            drop(pdf);
            fs::remove_file(&pdf_path)?;
            let strict_raw_header = matches!(
                &error.kind,
                Type3PdfErrorKind::Stage { stage: Type3Stage::TextHeader, source }
                    if source.downcast_ref::<TextRegionError>().is_some_and(|region| matches!(
                        region.kind,
                        TextRegionErrorKind::MalformedFlags {
                            field: "SBRTEMPLATE without SBREFINE", raw: 0xa40c
                        }
                    ))
            );
            if policy != STRICT || !strict_raw_header {
                return Err(error.into());
            }
            // Python verifies the pinned anomaly identity and field offset.
            let kind = "malformed_text_header";
            let offset = error.offset.unwrap_or(0);
            println!(
                "CASE\tREFUSED\t{page}\t{image}\t-\t0\t0\t0\t0\t{}\t0\t{}\t{scratch_bytes}\t{rss}\t-\t{kind}\t{offset}",
                source.bytes, source.largest_request,
            );
        }
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("type-3 PDF parity adapter failed: {error}");
        std::process::exit(1);
    }
}
