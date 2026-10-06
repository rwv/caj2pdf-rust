// SPDX-License-Identifier: MIT

//! In-memory adapters that drive the public core entry points on arbitrary
//! bytes. Every result is ignored: a target fails only by panicking, aborting,
//! exceeding the configured limits or timing out.

use caj2pdf_core::{
    Bookmark, BookmarkVisitor, ConversionOptions, Error, InputFormat, Limits, NeverCancel,
    RangedSource, Result, SIGNATURE_BYTES, SequentialSink, caj, detect_format,
    hnc8::{
        Budget, ComposeOptions, ComposeType3Workspaces, Hnc8Reader,
        Type3PdfOptions, convert_source_pages_pdf,
    },
    jbig2::{mq::MqTable, text::TextHeaderPolicy, text_composer::RandomAccessScratch},
    kdh::convert_kdh,
    pdf::copy_pdf,
    qm::QmTable,
};
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

/// Small enough that one input cannot exhaust a fuzzing worker.
const MAX_BYTES: u64 = 64 * 1024 * 1024;

fn limits() -> Limits {
    Limits {
        max_output_bytes: MAX_BYTES,
        max_allocation_bytes: MAX_BYTES,
        max_pages: 4_096,
        max_bookmarks: 4_096,
        ..Limits::default()
    }
}

/// The in-memory adapters never return `Pending`.
fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    loop {
        if let Poll::Ready(value) = future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            return value;
        }
    }
}

struct Source<'a>(&'a [u8]);

impl RangedSource for Source<'_> {
    fn size(&self) -> u64 {
        self.0.len() as u64
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        let start = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(self.0.len());
        let bytes = &self.0[start..];
        let count = bytes.len().min(destination.len());
        destination[..count].copy_from_slice(&bytes[..count]);
        Ok(count)
    }
}

/// Counts and discards output.
#[derive(Default)]
struct Sink(u64);

impl SequentialSink for Sink {
    async fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        self.0 += bytes.len() as u64;
        Ok(bytes.len())
    }

    async fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct Scratch(Vec<u8>);

impl RandomAccessScratch for Scratch {
    fn size(&self) -> Result<u64> {
        Ok(self.0.len() as u64)
    }

    async fn set_len(&mut self, bytes: u64) -> Result<()> {
        if bytes > MAX_BYTES {
            return Err(Error::InvalidInput {
                reason: "fuzz scratch limit",
            });
        }
        self.0.resize(bytes as usize, 0);
        Ok(())
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        Source(&self.0).read_at(offset, destination).await
    }

    async fn write_at(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        let start = offset as usize;
        let end = start
            .checked_add(bytes.len())
            .filter(|end| *end <= self.0.len());
        let Some(end) = end else {
            return Err(Error::InvalidInput {
                reason: "fuzz scratch write escapes its length",
            });
        };
        self.0[start..end].copy_from_slice(bytes);
        Ok(bytes.len())
    }

    async fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

fn format(data: &[u8]) -> Option<InputFormat> {
    detect_format(&data[..data.len().min(SIGNATURE_BYTES)])
}

/// Convert through the same routes as the CLI, HN/C8 image pages included.
pub fn convert(data: &[u8]) {
    let limits = limits();
    let (mut source, mut sink) = (Source(data), Sink::default());
    let Some(format) = format(data) else { return };
    let options = ConversionOptions {
        include_bookmarks: true,
        allow_damaged: data.get(8).is_some_and(|byte| byte & 1 != 0),
    };
    let _ = block_on(async {
        match format {
            InputFormat::Pdf => copy_pdf(&mut source, &mut sink, &limits, &NeverCancel).await,
            InputFormat::Caj => {
                caj::convert_caj(&mut source, &mut sink, options, &limits, &NeverCancel).await
            }
            InputFormat::Kdh => convert_kdh(&mut source, &mut sink, &limits, &NeverCancel).await,
            InputFormat::Hn | InputFormat::C8 => {
                let (qm, mq) = (QmTable::standard(), MqTable::standard());
                let mut stores: [Scratch; 4] = Default::default();
                let [text, first, second, refined] = &mut stores;
                let compose = ComposeOptions {
                    type3: Type3PdfOptions {
                        text_header_policy: TextHeaderPolicy::HnC8UnusedRefinementTemplate,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                let workspaces = Some(ComposeType3Workspaces {
                        table: &mq,
                        first,
                        second,
                        refined,
                        text,
                    });
                convert_source_pages_pdf(
                    &mut source,
                    &mut sink,
                    Some(&qm),
                    workspaces,
                    &mut (),
                    compose,
                    &limits,
                    &NeverCancel,
                )
                .await
                .map(|report| report.conversion)
                .map_err(|_| Error::InvalidInput { reason: "compose" })
            }
            _ => Ok(Default::default()),
        }
    });
}

struct Bookmarks;

impl BookmarkVisitor for Bookmarks {
    async fn visit(&mut self, _: Bookmark) -> Result<()> {
        Ok(())
    }
}

/// Read every HN/C8 page row and, for HN-A, the outline.
async fn hnc8_metadata(source: &mut Source<'_>, limits: &Limits) -> Option<()> {
    let mut reader = Hnc8Reader::open(source, limits, &NeverCancel, Budget::default())
        .await
        .ok()?;
    let pages = reader.header().page_count;
    while reader.next_page().await.ok()?.is_some() {}
    if reader.declared_bookmark_count().is_some() {
        reader
            .visit_bookmarks(64, pages, |page| Some(page - 1), &mut Bookmarks)
            .await
            .ok()?;
    }
    Some(())
}

/// Metadata parsing used by `inspect` and `add-bookmarks`.
pub fn inspect(data: &[u8]) {
    let limits = limits();
    let mut source = Source(data);
    block_on(async {
        match format(data) {
            Some(InputFormat::Caj) => {
                let _ = caj::parse_metadata(&mut source, &limits, &NeverCancel).await;
            }
            Some(InputFormat::Hn | InputFormat::C8) => {
                let _ = hnc8_metadata(&mut source, &limits).await;
            }
            _ => {}
        }
    });
}
