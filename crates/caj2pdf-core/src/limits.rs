// SPDX-License-Identifier: MIT

use crate::{Error, Result};

/// Default payload per read or write call: 256 KiB.
pub const DEFAULT_IO_CHUNK: usize = 256 * 1024;
/// Hard payload ceiling per read or write call: 1 MiB.
pub const MAX_IO_CHUNK: usize = 1024 * 1024;
/// Resource limits applied before allocation and during I/O.
///
/// These are the only resource bounds of an operation. Each public entry
/// point validates them once; format handlers derive every other bound from
/// these fields or from the format itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    /// Maximum payload per source/sink call. Must be in `1..=MAX_IO_CHUNK`.
    pub io_chunk_bytes: usize,
    /// Maximum selected input bytes: a whole source, PDF range, or sum of fragment spans.
    pub max_input_bytes: u64,
    /// Maximum total output bytes for an operation.
    pub max_output_bytes: u64,
    /// Maximum single dynamic allocation requested by a format handler. It
    /// also caps one image payload, one JBIG2 symbol store and one page or
    /// region bitmap.
    pub max_allocation_bytes: u64,
    /// Maximum pages accepted from an input document.
    pub max_pages: u32,
    /// Maximum bookmarks accepted from an input document.
    pub max_bookmarks: u32,
    /// Maximum pixels of one decoded image, page, region or symbol bitmap.
    pub max_image_pixels: u64,
    /// Maximum symbols in one JBIG2 symbol dictionary, including imported
    /// symbols, and the bound on its height classes and export runs.
    pub max_symbols: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            io_chunk_bytes: DEFAULT_IO_CHUNK,
            max_input_bytes: 8 * 1024 * 1024 * 1024,
            max_output_bytes: 16 * 1024 * 1024 * 1024,
            max_allocation_bytes: 64 * 1024 * 1024,
            max_pages: 100_000,
            max_bookmarks: 100_000,
            max_image_pixels: 12_000_000,
            max_symbols: 8192,
        }
    }
}

impl Limits {
    /// Reject invalid configurations before any I/O or allocation occurs.
    pub fn validate(&self) -> Result<()> {
        if self.io_chunk_bytes == 0 {
            return Err(Error::invalid("I/O chunk size must be nonzero"));
        }
        if self.io_chunk_bytes > MAX_IO_CHUNK {
            return Err(Error::limit(
                "I/O chunk bytes",
                MAX_IO_CHUNK as u64,
                self.io_chunk_bytes as u64,
            ));
        }
        self.check_allocation(self.io_chunk_bytes as u64)
    }

    /// Check the size of a requested single allocation.
    pub fn check_allocation(&self, bytes: u64) -> Result<()> {
        if bytes > self.max_allocation_bytes {
            return Err(Error::limit(
                "allocation bytes",
                self.max_allocation_bytes,
                bytes,
            ));
        }
        Ok(())
    }

    /// The unlocated error for an allocation of `attempted` bytes that passed
    /// [`Self::check_allocation`] but was refused by the allocator.
    pub(crate) fn allocation_refused(&self, resource: &'static str, attempted: u64) -> Error {
        Error::limit(resource, self.max_allocation_bytes, attempted)
    }

    /// Check the selected input byte count for one operation.
    pub fn check_input_size(&self, bytes: u64) -> Result<()> {
        if bytes > self.max_input_bytes {
            return Err(Error::limit("input bytes", self.max_input_bytes, bytes));
        }
        Ok(())
    }

    /// Check a page count before indexing or allocating pages.
    pub fn check_pages(&self, count: u32) -> Result<()> {
        if count > self.max_pages {
            return Err(Error::limit(
                "pages",
                u64::from(self.max_pages),
                u64::from(count),
            ));
        }
        Ok(())
    }

    /// Check the pixel count of one image, page, region or symbol bitmap.
    pub fn check_image_pixels(&self, pixels: u64) -> Result<()> {
        if pixels > self.max_image_pixels {
            return Err(Error::limit("image pixels", self.max_image_pixels, pixels));
        }
        Ok(())
    }

    /// Check the symbol count of one JBIG2 symbol dictionary.
    pub fn check_symbols(&self, symbols: u64) -> Result<()> {
        if symbols > u64::from(self.max_symbols) {
            return Err(Error::limit(
                "symbols",
                u64::from(self.max_symbols),
                symbols,
            ));
        }
        Ok(())
    }

    /// Check a bookmark count before indexing or allocating bookmarks.
    pub fn check_bookmarks(&self, count: u32) -> Result<()> {
        if count > self.max_bookmarks {
            return Err(Error::limit(
                "bookmarks",
                u64::from(self.max_bookmarks),
                u64::from(count),
            ));
        }
        Ok(())
    }
}
