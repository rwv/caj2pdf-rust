// SPDX-License-Identifier: MIT

use crate::fallible::len_u64;
use crate::{Error, Limits, Result};
use std::io;

/// A source with a stable size snapshot and positioned reads.
///
/// A read may return fewer bytes than requested, including zero at end of
/// input. Implementations must never write beyond `destination` or report
/// more than its length. All callers bound each request to `MAX_IO_CHUNK`.
/// The mutable receiver allows adapters to use a seekable handle or await a
/// JavaScript range read without requiring thread-safe futures.
#[allow(async_fn_in_trait)]
pub trait RangedSource {
    fn size(&self) -> u64;
    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize>;
}

/// A [`RangedSource`] adapter that adds each count its source returns to a
/// caller-owned byte counter.
///
/// The counter is borrowed, so it keeps the bytes of the successful reads
/// before a failing one and can accumulate across short-lived adapters. It
/// saturates; reaching `u64::MAX` would take more reads than any input allows.
///
/// A count larger than the destination is passed on uncounted, for the
/// caller's read helper to reject, unless [`Self::rejecting_overread`] makes
/// the adapter reject it first.
pub struct CountingSource<'a, S> {
    source: &'a mut S,
    bytes_read: &'a mut u64,
    overread: Option<&'static str>,
}

impl<'a, S> CountingSource<'a, S> {
    /// Count the bytes `source` returns into `bytes_read`.
    pub fn new(source: &'a mut S, bytes_read: &'a mut u64) -> Self {
        Self {
            source,
            bytes_read,
            overread: None,
        }
    }

    /// Reject a count larger than the destination as
    /// [`Error::InvalidInput`] with `reason`.
    pub fn rejecting_overread(self, reason: &'static str) -> Self {
        Self {
            overread: Some(reason),
            ..self
        }
    }
}

impl<S: RangedSource> RangedSource for CountingSource<'_, S> {
    fn size(&self) -> u64 {
        self.source.size()
    }

    async fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        let read = self.source.read_at(offset, destination).await?;
        if read <= destination.len() {
            *self.bytes_read = self.bytes_read.saturating_add(len_u64(read));
        } else if let Some(reason) = self.overread {
            return Err(Error::InvalidInput { reason });
        }
        Ok(read)
    }
}

/// A forward-only sink whose write and flush operations may apply backpressure.
///
/// As with `std::io::Write`, `write` may accept only a prefix of the supplied
/// bytes. Zero progress for a nonempty write is a `WriteZero` error in the
/// checked `write_all` helper.
#[allow(async_fn_in_trait)]
pub trait SequentialSink {
    async fn write(&mut self, bytes: &[u8]) -> Result<usize>;
    async fn flush(&mut self) -> Result<()>;
}

/// A platform-provided cancellation signal checked between awaited I/O calls.
pub trait Cancellation {
    fn is_cancelled(&self) -> bool;
}

/// A cancellation signal for operations that cannot be cancelled.
#[derive(Clone, Copy, Debug, Default)]
pub struct NeverCancel;

impl Cancellation for NeverCancel {
    fn is_cancelled(&self) -> bool {
        false
    }
}

fn check_cancelled<C: Cancellation>(cancellation: &C) -> Result<()> {
    if cancellation.is_cancelled() {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

fn check_range(source_size: u64, offset: u64, length: u64) -> Result<()> {
    let end = offset.checked_add(length).ok_or(Error::InvalidInput {
        reason: "range end overflows 64-bit offset",
    })?;
    if offset > source_size {
        return Err(Error::InvalidInput {
            reason: "range starts beyond source size",
        });
    }
    if end > source_size {
        return Err(Error::TruncatedInput {
            offset,
            expected: length,
            available: source_size - offset,
        });
    }
    Ok(())
}

/// Fill one bounded buffer from a positioned source, tolerating short reads.
///
/// This function rejects a request larger than the configured I/O chunk. A
/// format handler must process larger ranges in a loop.
/// The caller checks the selected operation size; unrelated source bytes do
/// not count against this individual read.
pub async fn read_exact_at<S: RangedSource, C: Cancellation>(
    source: &mut S,
    offset: u64,
    destination: &mut [u8],
    limits: &Limits,
    cancellation: &C,
) -> Result<()> {
    limits.validate()?;
    let length = len_u64(destination.len());
    limits.check_input_size(length)?;
    if destination.len() > limits.io_chunk_bytes {
        return Err(Error::LimitExceeded {
            resource: "I/O request bytes",
            limit: len_u64(limits.io_chunk_bytes),
            attempted: length,
        });
    }
    check_range(source.size(), offset, length)?;
    check_cancelled(cancellation)?;

    let mut done = 0;
    while done < destination.len() {
        let current = offset
            .checked_add(len_u64(done))
            .ok_or(Error::InvalidInput {
                reason: "read offset overflows 64-bit offset",
            })?;
        let remaining = &mut destination[done..];
        let read = source.read_at(current, remaining).await?;
        if read > remaining.len() {
            return Err(Error::InvalidInput {
                reason: "source reported more bytes than requested",
            });
        }
        done += read;
        check_cancelled(cancellation)?;
        if read == 0 {
            return Err(Error::TruncatedInput {
                offset,
                expected: length,
                available: len_u64(done),
            });
        }
    }
    Ok(())
}

/// Write all bytes in bounded calls, accounting for partial writes and limits.
///
/// `output_bytes_written` is an operation-wide counter. It is updated after
/// every successful write, including when a later write or cancellation fails.
pub async fn write_all<S: SequentialSink, C: Cancellation>(
    sink: &mut S,
    bytes: &[u8],
    output_bytes_written: &mut u64,
    limits: &Limits,
    cancellation: &C,
) -> Result<()> {
    limits.validate()?;
    let length = len_u64(bytes.len());
    let attempted = output_bytes_written
        .checked_add(length)
        .ok_or(Error::InvalidInput {
            reason: "output byte count overflows 64 bits",
        })?;
    if attempted > limits.max_output_bytes {
        return Err(Error::LimitExceeded {
            resource: "output bytes",
            limit: limits.max_output_bytes,
            attempted,
        });
    }
    check_cancelled(cancellation)?;

    let mut done = 0;
    while done < bytes.len() {
        let chunk_length = (bytes.len() - done).min(limits.io_chunk_bytes);
        let end = done.checked_add(chunk_length).ok_or(Error::InvalidInput {
            reason: "output slice offset overflows address space",
        })?;
        let chunk = &bytes[done..end];
        let written = sink.write(chunk).await?;
        if written > chunk.len() {
            return Err(Error::InvalidInput {
                reason: "sink reported more bytes than supplied",
            });
        }
        if written == 0 {
            return Err(Error::Io(io::Error::new(
                io::ErrorKind::WriteZero,
                "sink made no progress",
            )));
        }
        done += written;
        *output_bytes_written =
            output_bytes_written
                .checked_add(len_u64(written))
                .ok_or(Error::InvalidInput {
                    reason: "output byte count overflows 64 bits",
                })?;
        check_cancelled(cancellation)?;
    }
    Ok(())
}
