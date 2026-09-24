// SPDX-License-Identifier: MIT

use crate::{Error, Result};

/// Default payload per read or write call: 256 KiB.
pub const DEFAULT_IO_CHUNK: usize = 256 * 1024;
/// Hard payload ceiling per read or write call: 1 MiB.
pub const MAX_IO_CHUNK: usize = 1024 * 1024;

/// Resource limits applied before allocation and during I/O.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    /// Maximum payload per source/sink call. Must be in `1..=MAX_IO_CHUNK`.
    pub io_chunk_bytes: usize,
    /// Maximum selected input bytes: a whole source, PDF range, or sum of fragment spans.
    pub max_input_bytes: u64,
    /// Maximum total output bytes for an operation.
    pub max_output_bytes: u64,
    /// Maximum single dynamic allocation requested by a format handler.
    pub max_allocation_bytes: u64,
    /// Maximum pages accepted from an input document.
    pub max_pages: u32,
    /// Maximum bookmarks accepted from an input document.
    pub max_bookmarks: u32,
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
        }
    }
}

impl Limits {
    /// Reject invalid configurations before any I/O or allocation occurs.
    pub fn validate(&self) -> Result<()> {
        if self.io_chunk_bytes == 0 {
            return Err(Error::InvalidInput {
                reason: "I/O chunk size must be nonzero",
            });
        }
        if self.io_chunk_bytes > MAX_IO_CHUNK {
            return Err(Error::LimitExceeded {
                resource: "I/O chunk bytes",
                limit: MAX_IO_CHUNK as u64,
                attempted: self.io_chunk_bytes as u64,
            });
        }
        self.check_allocation(self.io_chunk_bytes as u64)
    }

    /// Check the size of a requested single allocation.
    pub fn check_allocation(&self, bytes: u64) -> Result<()> {
        if bytes > self.max_allocation_bytes {
            return Err(Error::LimitExceeded {
                resource: "allocation bytes",
                limit: self.max_allocation_bytes,
                attempted: bytes,
            });
        }
        Ok(())
    }

    /// Check the selected input byte count for one operation.
    pub fn check_input_size(&self, bytes: u64) -> Result<()> {
        if bytes > self.max_input_bytes {
            return Err(Error::LimitExceeded {
                resource: "input bytes",
                limit: self.max_input_bytes,
                attempted: bytes,
            });
        }
        Ok(())
    }

    /// Check a page count before indexing or allocating pages.
    pub fn check_pages(&self, count: u32) -> Result<()> {
        if count > self.max_pages {
            return Err(Error::LimitExceeded {
                resource: "pages",
                limit: u64::from(self.max_pages),
                attempted: u64::from(count),
            });
        }
        Ok(())
    }

    /// Check a bookmark count before indexing or allocating bookmarks.
    pub fn check_bookmarks(&self, count: u32) -> Result<()> {
        if count > self.max_bookmarks {
            return Err(Error::LimitExceeded {
                resource: "bookmarks",
                limit: u64::from(self.max_bookmarks),
                attempted: u64::from(count),
            });
        }
        Ok(())
    }
}
