// SPDX-License-Identifier: MIT

use crate::{ConversionReport, Error, Limits, Result};
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

fn checked_len(length: usize) -> Result<u64> {
    u64::try_from(length).map_err(|_| Error::InvalidInput {
        reason: "length cannot fit in a 64-bit offset",
    })
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
/// format handler must process larger ranges in a loop, as `copy_range` does.
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
    let length = checked_len(destination.len())?;
    limits.check_input_size(length)?;
    if destination.len() > limits.io_chunk_bytes {
        return Err(Error::LimitExceeded {
            resource: "I/O request bytes",
            limit: checked_len(limits.io_chunk_bytes)?,
            attempted: length,
        });
    }
    check_range(source.size(), offset, length)?;
    check_cancelled(cancellation)?;

    let mut done = 0;
    while done < destination.len() {
        let current = offset
            .checked_add(checked_len(done)?)
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
                available: checked_len(done)?,
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
    let length = checked_len(bytes.len())?;
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
        *output_bytes_written = output_bytes_written
            .checked_add(checked_len(written)?)
            .ok_or(Error::InvalidInput {
                reason: "output byte count overflows 64 bits",
            })?;
        check_cancelled(cancellation)?;
    }
    Ok(())
}

/// Copy a source range to a forward-only sink with one reusable chunk buffer.
///
/// This is an I/O proof, not format conversion. The report records zero pages
/// and bookmarks. It flushes the sink only after the requested range succeeds.
pub async fn copy_range<R: RangedSource, W: SequentialSink, C: Cancellation>(
    source: &mut R,
    sink: &mut W,
    offset: u64,
    length: u64,
    limits: &Limits,
    cancellation: &C,
) -> Result<ConversionReport> {
    limits.validate()?;
    limits.check_input_size(length)?;
    check_range(source.size(), offset, length)?;
    if length > limits.max_output_bytes {
        return Err(Error::LimitExceeded {
            resource: "output bytes",
            limit: limits.max_output_bytes,
            attempted: length,
        });
    }
    check_cancelled(cancellation)?;

    let initial_chunk = length.min(checked_len(limits.io_chunk_bytes)?) as usize;
    let mut buffer = vec![0; initial_chunk];
    let mut report = ConversionReport::default();
    while report.input_bytes_read < length {
        let remaining = length - report.input_bytes_read;
        let chunk_length = remaining.min(checked_len(buffer.len())?) as usize;
        let current = offset
            .checked_add(report.input_bytes_read)
            .ok_or(Error::InvalidInput {
                reason: "read offset overflows 64-bit offset",
            })?;
        read_exact_at(
            source,
            current,
            &mut buffer[..chunk_length],
            limits,
            cancellation,
        )
        .await?;
        report.input_bytes_read = report
            .input_bytes_read
            .checked_add(checked_len(chunk_length)?)
            .ok_or(Error::InvalidInput {
                reason: "input byte count overflows 64 bits",
            })?;
        write_all(
            sink,
            &buffer[..chunk_length],
            &mut report.output_bytes_written,
            limits,
            cancellation,
        )
        .await?;
    }
    check_cancelled(cancellation)?;
    sink.flush().await?;
    check_cancelled(cancellation)?;
    Ok(report)
}
