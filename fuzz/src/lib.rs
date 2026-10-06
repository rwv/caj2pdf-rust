// SPDX-License-Identifier: MIT

//! In-memory adapters that drive the public core entry points on arbitrary
//! bytes. Every result is ignored: a target fails only by panicking, aborting,
//! exceeding the configured limits or timing out.

use caj2pdf_core::{
    Bookmark, BookmarkVisitor, ConversionOptions, Error, InputFormat, Limits, NeverCancel, Result,
    SIGNATURE_BYTES, caj, detect_format,
    hnc8::{ComposeOptions, Hnc8Reader, convert_source_pages_pdf},
    jbig2::text::TextHeaderPolicy,
    kdh::convert_kdh,
    pdf::copy_pdf,
    qm::QmTable,
};
use std::io::Write;

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

/// Counts and discards output.
#[derive(Default)]
struct Sink(u64);

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 += bytes.len() as u64;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn format(data: &[u8]) -> Option<InputFormat> {
    detect_format(&data[..data.len().min(SIGNATURE_BYTES)])
}

/// Convert through the same routes as the CLI, HN/C8 image pages included.
pub fn convert(data: &[u8]) {
    let limits = limits();
    let (mut source, mut sink) = (data, Sink::default());
    let Some(format) = format(data) else { return };
    let options = ConversionOptions {
        include_bookmarks: true,
        allow_damaged: data.get(8).is_some_and(|byte| byte & 1 != 0),
    };
    let _ = match format {
        InputFormat::Pdf => copy_pdf(&mut source, &mut sink, &limits, &NeverCancel),
        InputFormat::Caj => caj::convert_caj(&mut source, &mut sink, options, &limits, &NeverCancel),
        InputFormat::Kdh => convert_kdh(&mut source, &mut sink, &limits, &NeverCancel),
        InputFormat::Hn | InputFormat::C8 => {
            let compose = ComposeOptions {
                text_header_policy: TextHeaderPolicy::HnC8UnusedRefinementTemplate,
                ..Default::default()
            };
            convert_source_pages_pdf(
                &mut source,
                &mut sink,
                Some(&QmTable::standard()),
                &mut (),
                compose,
                &limits,
                &NeverCancel,
            )
            .map(|report| report.conversion)
            .map_err(|_| Error::InvalidInput { reason: "compose" })
        }
        _ => Ok(Default::default()),
    };
}

struct Bookmarks;

impl BookmarkVisitor for Bookmarks {
    fn visit(&mut self, _: Bookmark) -> Result<()> {
        Ok(())
    }
}

/// Read every HN/C8 page row and, for HN-A, the outline.
fn hnc8_metadata(mut source: &[u8], limits: &Limits) -> Option<()> {
    let mut reader = Hnc8Reader::open(&mut source, limits, &NeverCancel).ok()?;
    let pages = reader.header().page_count;
    while reader.next_page().ok()?.is_some() {}
    if reader.declared_bookmark_count().is_some() {
        reader
            .visit_bookmarks(64, pages, |page| Some(page - 1), &mut Bookmarks)
            .ok()?;
    }
    Some(())
}

/// Metadata parsing used by `inspect` and `add-bookmarks`.
pub fn inspect(data: &[u8]) {
    let limits = limits();
    match format(data) {
        Some(InputFormat::Caj) => {
            let _ = caj::parse_metadata(&mut { data }, &limits, &NeverCancel);
        }
        Some(InputFormat::Hn | InputFormat::C8) => {
            let _ = hnc8_metadata(data, &limits);
        }
        _ => {}
    }
}
