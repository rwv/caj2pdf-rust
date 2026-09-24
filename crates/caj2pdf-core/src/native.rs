// SPDX-License-Identifier: MIT

//! Borrowable adapters for `std::io` handles.
//!
//! `SeekableSource::new(&mut input)` and `WriteSink::new(&mut output)` leave
//! ownership with the caller. The output handle needs only `Write`, not `Seek`.

use crate::{Error, MAX_IO_CHUNK, RangedSource, Result, SequentialSink};
use std::io::{Read, Seek, SeekFrom, Write};

/// A positioned source backed by a caller-supplied `Read + Seek` handle.
pub struct SeekableSource<R> {
    inner: R,
    size: u64,
}

impl<R: Read + Seek> SeekableSource<R> {
    /// Snapshot the size and restore the handle's original position.
    pub fn new(mut inner: R) -> Result<Self> {
        let original = inner.stream_position()?;
        let size = inner.seek(SeekFrom::End(0))?;
        inner.seek(SeekFrom::Start(original))?;
        Ok(Self { inner, size })
    }

    pub fn into_inner(self) -> R {
        self.inner
    }
}

impl<R: Read + Seek> RangedSource for SeekableSource<R> {
    fn size(&self) -> u64 {
        self.size
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        if destination.len() > MAX_IO_CHUNK {
            return Err(Error::LimitExceeded {
                resource: "I/O request bytes",
                limit: MAX_IO_CHUNK as u64,
                attempted: destination.len() as u64,
            });
        }
        if offset > self.size {
            return Err(Error::InvalidInput {
                reason: "read starts beyond source size",
            });
        }
        offset
            .checked_add(destination.len() as u64)
            .ok_or(Error::InvalidInput {
                reason: "read end overflows 64-bit offset",
            })?;
        self.inner.seek(SeekFrom::Start(offset))?;
        Ok(self.inner.read(destination)?)
    }
}

/// A sequential sink backed by a caller-supplied `Write` handle.
pub struct WriteSink<W> {
    inner: W,
}

impl<W: Write> WriteSink<W> {
    pub fn new(inner: W) -> Self {
        Self { inner }
    }

    pub fn into_inner(self) -> W {
        self.inner
    }
}

impl<W: Write> SequentialSink for WriteSink<W> {
    async fn write(&mut self, bytes: &[u8]) -> Result<usize> {
        if bytes.len() > MAX_IO_CHUNK {
            return Err(Error::LimitExceeded {
                resource: "I/O request bytes",
                limit: MAX_IO_CHUNK as u64,
                attempted: bytes.len() as u64,
            });
        }
        Ok(self.inner.write(bytes)?)
    }

    async fn flush(&mut self) -> Result<()> {
        Ok(self.inner.flush()?)
    }
}
