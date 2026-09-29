// SPDX-License-Identifier: MIT

use crate::{Error, MAX_IO_CHUNK, Result, jbig2::text_composer::RandomAccessScratch};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
};

/// Bounded random-access scratch backed by a caller-created regular file.
///
/// The caller chooses where and how to create temporary storage, and must not
/// access the file through another handle during conversion. This adapter
/// neither creates nor deletes paths; dropping it closes its owned handle.
/// Use an unlinked temporary file when storage should disappear on drop.
/// No bitmap or whole-file buffer is allocated here. Operations are blocking,
/// like the other native adapters, and may return short reads/writes.
pub struct FileScratch {
    file: File,
    max_bytes: u64,
}

impl FileScratch {
    /// Adopt a regular file whose current length fits `max_bytes`.
    /// Open it for both reading and writing before passing it here.
    pub fn new(file: File, max_bytes: u64) -> Result<Self> {
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(Error::InvalidInput {
                reason: "scratch backing must be a regular file",
            });
        }
        let scratch = Self { file, max_bytes };
        scratch.check_size(metadata.len())?;
        Ok(scratch)
    }

    pub fn into_inner(self) -> File {
        self.file
    }

    fn check_size(&self, bytes: u64) -> Result<()> {
        if bytes > self.max_bytes {
            return Err(Error::LimitExceeded {
                resource: "native scratch bytes",
                limit: self.max_bytes,
                attempted: bytes,
            });
        }
        Ok(())
    }

    fn position(&mut self, offset: u64, bytes: usize) -> Result<()> {
        if bytes > MAX_IO_CHUNK {
            return Err(Error::LimitExceeded {
                resource: "I/O request bytes",
                limit: MAX_IO_CHUNK as u64,
                attempted: bytes as u64,
            });
        }
        let end = offset
            .checked_add(bytes as u64)
            .ok_or(Error::InvalidInput {
                reason: "scratch request end overflows 64-bit offset",
            })?;
        if end > self.size()? {
            return Err(Error::InvalidInput {
                reason: "scratch request escapes declared size",
            });
        }
        self.file.seek(SeekFrom::Start(offset))?;
        Ok(())
    }
}

impl RandomAccessScratch for FileScratch {
    fn size(&self) -> Result<u64> {
        Ok(self.file.metadata()?.len())
    }

    async fn set_len(&mut self, bytes: u64) -> Result<()> {
        self.check_size(bytes)?;
        self.file.set_len(bytes)?;
        Ok(())
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        self.position(offset, destination.len())?;
        Ok(self.file.read(destination)?)
    }

    async fn write_at(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        self.position(offset, bytes.len())?;
        Ok(self.file.write(bytes)?)
    }

    async fn flush(&mut self) -> Result<()> {
        Ok(self.file.flush()?)
    }
}
