// SPDX-License-Identifier: MIT

use crate::{Error, MAX_IO_CHUNK, Result, jbig2::text_composer::RandomAccessScratch};
use std::{
    fs::File,
    io::{self, Write},
};

/// Bounded random-access scratch backed by a caller-created regular file.
///
/// The caller chooses where and how to create temporary storage, and must not
/// access the file through another handle during conversion. This adapter
/// neither creates nor deletes paths; dropping it closes its owned handle.
/// Use an unlinked temporary file when storage should disappear on drop.
/// No bitmap or whole-file buffer is allocated here. Operations are blocking,
/// like the other native adapters, and may return short reads/writes.
///
/// The logical length is read once on construction and then tracked through
/// [`RandomAccessScratch::set_len`]; requests never extend the file. Requests
/// use positioned I/O where the platform has it, so they never depend on the
/// handle's cursor and need no separate seek.
pub struct FileScratch {
    file: File,
    max_bytes: u64,
    len: u64,
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
        let len = metadata.len();
        let scratch = Self {
            file,
            max_bytes,
            len,
        };
        scratch.check_size(len)?;
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

    fn check_request(&self, offset: u64, bytes: usize) -> Result<()> {
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
        if end > self.len {
            return Err(Error::InvalidInput {
                reason: "scratch request escapes declared size",
            });
        }
        Ok(())
    }
}

impl RandomAccessScratch for FileScratch {
    fn size(&self) -> Result<u64> {
        Ok(self.len)
    }

    fn set_len(&mut self, bytes: u64) -> Result<()> {
        self.check_size(bytes)?;
        self.file.set_len(bytes)?;
        self.len = bytes;
        Ok(())
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        self.check_request(offset, destination.len())?;
        Ok(read_at(&self.file, offset, destination)?)
    }

    fn write_at(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        self.check_request(offset, bytes.len())?;
        Ok(write_at(&self.file, offset, bytes)?)
    }

    fn flush(&mut self) -> Result<()> {
        Ok(self.file.flush()?)
    }
}

#[cfg(unix)]
fn read_at(file: &File, offset: u64, destination: &mut [u8]) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, destination, offset)
}

#[cfg(unix)]
fn write_at(file: &File, offset: u64, bytes: &[u8]) -> io::Result<usize> {
    std::os::unix::fs::FileExt::write_at(file, bytes, offset)
}

// Windows positioned I/O also moves the handle cursor, which this adapter
// never relies on.
#[cfg(windows)]
fn read_at(file: &File, offset: u64, destination: &mut [u8]) -> io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(file, destination, offset)
}

#[cfg(windows)]
fn write_at(file: &File, offset: u64, bytes: &[u8]) -> io::Result<usize> {
    std::os::windows::fs::FileExt::seek_write(file, bytes, offset)
}

// Targets without positioned file I/O (for example wasm32-unknown-unknown,
// whose `File` cannot be opened) fall back to seeking the shared cursor.
#[cfg(not(any(unix, windows)))]
fn read_at(mut file: &File, offset: u64, destination: &mut [u8]) -> io::Result<usize> {
    use std::io::{Read, Seek, SeekFrom};
    file.seek(SeekFrom::Start(offset))?;
    file.read(destination)
}

#[cfg(not(any(unix, windows)))]
fn write_at(mut file: &File, offset: u64, bytes: &[u8]) -> io::Result<usize> {
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(offset))?;
    file.write(bytes)
}
