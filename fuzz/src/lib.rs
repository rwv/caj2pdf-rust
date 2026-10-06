// SPDX-License-Identifier: MIT

//! In-memory adapters that drive the core facade on arbitrary bytes. Every
//! result is ignored: a target fails only by panicking, aborting, exceeding
//! the configured limits or timing out.

use caj2pdf_core::{
    ConversionOptions, Error, InspectOptions, Limits, NeverCancel, PageVisitor, Result,
    hnc8::{ImageRecord, PageRecord, TextStructure},
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

/// Convert through the facade, as the CLI and WASM adapters do.
pub fn convert(data: &[u8]) {
    let options = ConversionOptions {
        allow_damaged: data.get(8).is_some_and(|byte| byte & 1 != 0),
        ..ConversionOptions::default()
    };
    let _ = caj2pdf_core::convert(
        &mut { data },
        &mut Sink::default(),
        options,
        &limits(),
        &mut NeverCancel,
    );
}

/// Discards the per-page report.
struct Pages;

impl PageVisitor for Pages {
    fn begin(&mut self) -> Result<()> {
        Ok(())
    }

    fn page(&mut self, _: u32, _: Option<&PageRecord>) -> Result<()> {
        Ok(())
    }

    fn image(&mut self, _: &ImageRecord) -> Result<()> {
        Ok(())
    }

    fn end_page(
        &mut self,
        _: Option<&TextStructure>,
        _: Option<&Error>,
        _: Option<&Error>,
    ) -> Result<()> {
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Inspection with the outline and structure, the HN/C8 per-page report,
/// and the outline read of `add-bookmarks`.
pub fn inspect(data: &[u8]) {
    let limits = limits();
    let options = InspectOptions {
        format: None,
        bookmarks: true,
        structure: true,
    };
    if let Ok(info) = caj2pdf_core::inspect(&mut { data }, &options, &limits, &mut NeverCancel) {
        let _ = caj2pdf_core::inspect_pages(
            &mut { data },
            &info,
            &limits,
            &mut NeverCancel,
            &mut Pages,
        );
    }
    let _ = caj2pdf_core::read_outline(&mut { data }, &limits, &mut NeverCancel);
}
