// SPDX-License-Identifier: MIT

//! Opt-in native adapter for the bounded source-page composition protocol.
//! Only caller paths and probability states are inputs; no oracle facts,
//! source identities, reference matrices or private samples are embedded.

#[path = "support/mod.rs"]
#[allow(dead_code)]
mod support;

use caj2pdf_core::{
    Error, Limits, NeverCancel, RangedSource, SequentialSink,
    hnc8::{
        Budget, ComposeBudget, ComposeOptions, ComposePage, ComposeVisitor, JpegBudget, TextBudget,
        convert_source_pages_pdf,
    },
    jbig1::Type0Budget,
    jbig2::text_composer::RandomAccessScratch,
    native::{SeekableSource, WriteSink},
    qm::{ArithmeticBudget, QM_STATE_COUNT, QmState, QmTable},
};
use std::{
    env,
    error::Error as StdError,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};
use support::{TempStore, ready};

const CHUNK: usize = 4096;
const ROW_BYTES: u64 = 2 * 1024 * 1024;
const TABLE_BYTES: u64 = 16 * 1024;

fn checked_total(total: &mut u64, count: usize) -> caj2pdf_core::Result<()> {
    *total = total.checked_add(count as u64).ok_or(Error::InvalidInput {
        reason: "diagnostic byte counter overflowed",
    })?;
    Ok(())
}

fn check_request(bytes: usize) -> caj2pdf_core::Result<()> {
    if bytes > CHUNK {
        return Err(Error::LimitExceeded {
            resource: "diagnostic I/O request bytes",
            limit: CHUNK as u64,
            attempted: bytes as u64,
        });
    }
    Ok(())
}

struct Source<S> {
    inner: S,
    bytes: u64,
    max_request: usize,
}

impl<S: RangedSource> RangedSource for Source<S> {
    fn size(&self) -> u64 {
        self.inner.size()
    }

    async fn read_at(&mut self, offset: u64, output: &mut [u8]) -> caj2pdf_core::Result<usize> {
        check_request(output.len())?;
        self.max_request = self.max_request.max(output.len());
        let count = self.inner.read_at(offset, output).await?;
        checked_total(&mut self.bytes, count)?;
        Ok(count)
    }
}

struct Sink<S> {
    inner: S,
    bytes: u64,
    max_request: usize,
}

impl<S: SequentialSink> SequentialSink for Sink<S> {
    async fn write(&mut self, bytes: &[u8]) -> caj2pdf_core::Result<usize> {
        check_request(bytes.len())?;
        self.max_request = self.max_request.max(bytes.len());
        let count = self.inner.write(bytes).await?;
        checked_total(&mut self.bytes, count)?;
        Ok(count)
    }

    async fn flush(&mut self) -> caj2pdf_core::Result<()> {
        self.inner.flush().await
    }
}

struct Scratch {
    file: File,
    max_request: usize,
    read_bytes: u64,
    written_bytes: u64,
    peak_bytes: u64,
}

impl Scratch {
    fn span(&mut self, offset: u64, count: usize) -> caj2pdf_core::Result<()> {
        check_request(count)?;
        self.max_request = self.max_request.max(count);
        let end = offset
            .checked_add(count as u64)
            .ok_or(Error::InvalidInput {
                reason: "diagnostic scratch offset overflowed",
            })?;
        if end > self.file.metadata()?.len() {
            return Err(Error::InvalidInput {
                reason: "diagnostic scratch request escapes declared rows",
            });
        }
        self.file.seek(SeekFrom::Start(offset))?;
        Ok(())
    }
}

impl RandomAccessScratch for Scratch {
    fn size(&self) -> caj2pdf_core::Result<u64> {
        Ok(self.file.metadata()?.len())
    }

    async fn set_len(&mut self, bytes: u64) -> caj2pdf_core::Result<()> {
        if bytes > ROW_BYTES {
            return Err(Error::LimitExceeded {
                resource: "diagnostic row-store bytes",
                limit: ROW_BYTES,
                attempted: bytes,
            });
        }
        self.file.set_len(bytes)?;
        self.peak_bytes = self.peak_bytes.max(bytes);
        Ok(())
    }

    async fn read_at(&mut self, offset: u64, output: &mut [u8]) -> caj2pdf_core::Result<usize> {
        self.span(offset, output.len())?;
        let count = self.file.read(output)?;
        checked_total(&mut self.read_bytes, count)?;
        Ok(count)
    }

    async fn write_at(&mut self, offset: u64, bytes: &[u8]) -> caj2pdf_core::Result<usize> {
        self.span(offset, bytes.len())?;
        let count = self.file.write(bytes)?;
        checked_total(&mut self.written_bytes, count)?;
        Ok(count)
    }

    async fn flush(&mut self) -> caj2pdf_core::Result<()> {
        self.file.flush()?;
        Ok(())
    }
}

struct Visitor;

impl ComposeVisitor for Visitor {
    async fn page(&mut self, page: ComposePage<'_>) -> caj2pdf_core::Result<()> {
        let source = page.source;
        let (output, box_values) = match (page.output_page, page.size) {
            (Some(number), Some(size)) => {
                (number, [0.0, 0.0, size.width_points, size.height_points])
            }
            (None, None) => (0, [0.0; 4]),
            _ => {
                return Err(Error::InvalidInput {
                    reason: "diagnostic page mapping and box disagree",
                });
            }
        };
        let mut stdout = std::io::stdout().lock();
        writeln!(
            stdout,
            "P\t{}\t{}\t{}\t{}\t{}\t{output}\t{}\t{}\t{}\t{}",
            source.page_number,
            source.row_offset,
            source.text.offset,
            source.text.length,
            source.image_count,
            box_values[0],
            box_values[1],
            box_values[2],
            box_values[3],
        )?;
        for image in page.images {
            let record = image.record;
            let matrix = image.transform;
            writeln!(
                stdout,
                "I\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                record.page_number,
                record.image_number,
                record.record_type,
                record.descriptor_offset,
                record.payload.offset,
                record.payload.length,
                image.visible_width,
                image.display_width,
                image.height,
                matrix[0],
                matrix[1],
                matrix[2],
                matrix[3],
                matrix[4],
                matrix[5],
            )?;
        }
        stdout.flush()?;
        Ok(())
    }
}

fn caller_table(path: &Path, limits: &Limits) -> Result<QmTable, Box<dyn StdError>> {
    limits.check_allocation(TABLE_BYTES)?;
    let mut bytes = Vec::new();
    File::open(path)?
        .take(TABLE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > TABLE_BYTES {
        return Err("caller T.82 vector exceeds its byte cap".into());
    }
    let text = std::str::from_utf8(&bytes)?;
    let mut lines = text.lines();
    if lines.next() != Some("T82-1993") || lines.next() != Some("113") {
        return Err("caller T.82 vector framing is unsupported".into());
    }
    let mut states = Vec::new();
    states.try_reserve_exact(QM_STATE_COUNT)?;
    for _ in 0..QM_STATE_COUNT {
        let mut fields = lines
            .next()
            .ok_or("missing caller T.82 state")?
            .split_whitespace();
        let qe = fields.next().ok_or("missing caller Qe")?.parse()?;
        let next_lps = fields.next().ok_or("missing caller next LPS")?.parse()?;
        let next_mps = fields.next().ok_or("missing caller next MPS")?.parse()?;
        let switch: u8 = fields.next().ok_or("missing caller switch")?.parse()?;
        if fields.next().is_some() || switch > 1 {
            return Err("caller T.82 state row is malformed".into());
        }
        states.push(QmState {
            qe,
            next_lps,
            next_mps,
            switch_mps: switch == 1,
        });
    }
    // The caller's fixture additionally carries three public-vector checkpoints.
    // They are framing checks, never states used by this diagnostic decoder.
    if lines.next() != Some("3") {
        return Err("caller T.82 checkpoint count is unsupported".into());
    }
    for _ in 0..6 {
        if lines.next().is_none() {
            return Err("caller T.82 checkpoints are truncated".into());
        }
    }
    if lines.next().is_some() {
        return Err("caller T.82 vector has trailing lines".into());
    }
    Ok(QmTable::new(states)?)
}

fn run() -> Result<(), Box<dyn StdError>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() != 4 {
        return Err(
            "usage: hnc8_page_composition SOURCE OUTPUT_PDF TABLE SCRATCH_DIRECTORY".into(),
        );
    }
    let scratch_dir = Path::new(&args[3]).canonicalize()?;
    if env::temp_dir().canonicalize()? != scratch_dir {
        return Err("TMPDIR must equal the caller-owned scratch directory".into());
    }
    let limits = Limits {
        io_chunk_bytes: CHUNK,
        max_input_bytes: 8 * 1024 * 1024 * 1024,
        max_output_bytes: 128 * 1024 * 1024,
        max_allocation_bytes: 1024 * 1024,
        max_pages: 4096,
        max_bookmarks: 4096,
    };
    let options = ComposeOptions {
        container: Budget {
            max_outline_records: 4096,
            max_images_per_page: 256,
            max_images_total: 16384,
            max_text_span_bytes: 1024 * 1024,
            max_image_span_bytes: 64 * 1024 * 1024,
        },
        text: TextBudget {
            max_images: 256,
            ..TextBudget::default()
        },
        jpeg: JpegBudget::default(),
        image: Type0Budget::default(),
        arithmetic: ArithmeticBudget {
            max_symbols: 12032768,
            max_work: 385049600,
        },
        budget: ComposeBudget {
            max_page_metadata_bytes: 65536,
            max_row_store_bytes: ROW_BYTES,
            max_row_store_io_bytes: 256 * 1024 * 1024,
        },
    };
    let table = caller_table(Path::new(&args[2]), &limits)?;
    let mut source = Source {
        inner: SeekableSource::new(File::open(&args[0])?)?,
        bytes: 0,
        max_request: 0,
    };
    let mut sink = Sink {
        inner: WriteSink::new(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&args[1])?,
        ),
        bytes: 0,
        max_request: 0,
    };
    let (row_store, handle) = TempStore::create("caj2pdf-source-rows")?;
    drop(handle);
    let mut scratch = Scratch {
        file: OpenOptions::new()
            .read(true)
            .write(true)
            .open(row_store.path())?,
        max_request: 0,
        read_bytes: 0,
        written_bytes: 0,
        peak_bytes: 0,
    };
    let report = ready(convert_source_pages_pdf(
        &mut source,
        &mut sink,
        Some(&table),
        &mut scratch,
        &mut Visitor,
        options,
        &limits,
        &NeverCancel,
    ))?;
    if scratch.size()? != 0 {
        return Err("completed diagnostic retained row-store bytes".into());
    }
    if report.peak_row_store_bytes != scratch.peak_bytes {
        return Err("handler row-store peak differs from physical file lengths".into());
    }
    println!(
        "R\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        report.source_variant.as_str(),
        report.source_pages,
        report.output_pages,
        report.no_image_pages,
        report.type0_images,
        report.jpeg_images,
        report.peak_page_metadata_bytes,
        report.peak_text_working_bytes,
        report.peak_row_store_bytes,
        report.row_store_read_bytes,
        report.row_store_written_bytes,
        source.bytes,
        source.max_request,
        sink.bytes,
        sink.max_request,
        scratch.read_bytes,
        scratch.written_bytes,
        scratch.max_request,
    );
    drop(scratch);
    drop(row_store);
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("source-page diagnostic failed: {error}");
        std::process::exit(1);
    }
}
