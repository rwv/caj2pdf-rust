// SPDX-License-Identifier: MIT

//! Interactive conversion progress for a terminal on standard error.
//!
//! Progress is the furthest input byte read so far as a share of the input
//! size. Converters read their indexes first and then page payloads in order,
//! so this advances with the conversion without a per-format page hook.

use caj2pdf_core::{RangedSource, Result};
use std::io::Write;

/// A source that reports read progress to `out`, or passes reads through
/// unchanged when `out` is `None`.
pub struct Progress<'a, S> {
    inner: S,
    out: Option<&'a mut dyn Write>,
    furthest: u64,
    shown: Option<u64>,
}

impl<'a, S: RangedSource> Progress<'a, S> {
    pub fn new(inner: S, out: Option<&'a mut dyn Write>) -> Self {
        Self {
            inner,
            out,
            furthest: 0,
            shown: None,
        }
    }

    fn show(&mut self) {
        let percent = self.furthest.saturating_mul(100) / self.inner.size().max(1);
        if let Some(out) = self.out.as_mut().filter(|_| self.shown != Some(percent)) {
            // A terminal write failure must not fail the conversion.
            let _ = write!(out, "\rcaj2pdf: reading input {percent:>3}%");
            let _ = out.flush();
            self.shown = Some(percent);
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

impl<S: RangedSource> RangedSource for Progress<'_, S> {
    fn size(&self) -> u64 {
        self.inner.size()
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        let count = self.inner.read_at(offset, destination).await?;
        let end = offset.saturating_add(count as u64);
        if end > self.furthest {
            self.furthest = end;
            self.show();
        }
        Ok(count)
    }
}
