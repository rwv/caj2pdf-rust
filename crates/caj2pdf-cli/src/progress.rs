// SPDX-License-Identifier: MIT

//! The CLI's side of a core operation: the input format the core reports,
//! for diagnostics; process cancellation; and interactive conversion
//! progress on standard error.
//!
//! Progress is the furthest input byte read so far as a share of the input
//! size, as the core reports it. Converters read their indexes first and then
//! page payloads in order, so this advances with the conversion without a
//! per-format page hook.

use crate::signals::ProcessCancellation;
use caj2pdf_core::{Cancellation, InputFormat};
use std::io::Write;

/// Reports progress to `out`, or nothing when `out` is `None`.
pub struct Progress<'a> {
    out: Option<&'a mut dyn Write>,
    shown: Option<u64>,
    /// The format the core reported: `None` before detection completed,
    /// `Some(None)` for an empty or unrecognized input.
    pub format: Option<Option<InputFormat>>,
}

impl<'a> Progress<'a> {
    pub fn new(out: Option<&'a mut dyn Write>) -> Self {
        Self {
            out,
            shown: None,
            format: None,
        }
    }

    /// Erase the progress line so later diagnostics start on a clean line.
    pub fn finish(mut self) {
        if let Some(out) = self.out.as_mut().filter(|_| self.shown.is_some()) {
            let _ = write!(out, "\r{:30}\r", "");
            let _ = out.flush();
        }
    }
}

impl caj2pdf_core::Progress for Progress<'_> {
    fn format(&mut self, format: Option<InputFormat>) {
        self.format = Some(format);
    }

    fn input_read(&mut self, done: u64, total: u64) {
        let percent = done.saturating_mul(100) / total.max(1);
        if let Some(out) = self.out.as_mut().filter(|_| self.shown != Some(percent)) {
            // A terminal write failure must not fail the conversion.
            let _ = write!(out, "\rcaj2pdf: reading input {percent:>3}%");
            let _ = out.flush();
            self.shown = Some(percent);
        }
    }

    fn is_cancelled(&self) -> bool {
        ProcessCancellation.is_cancelled()
    }
}
