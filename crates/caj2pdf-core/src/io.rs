// SPDX-License-Identifier: MIT

use crate::fallible::len_u64;
use crate::{Error, Limits, Result};
use std::io::Write;

/// A source with a stable size snapshot and positioned reads.
///
/// A read may return fewer bytes than requested, including zero at end of
/// input. Implementations must never write beyond `destination` or report
/// more than its length. All callers bound each request to `MAX_IO_CHUNK`.
/// The mutable receiver allows adapters over a seekable handle. Use
/// [`crate::native::SeekableSource`] for a `Read + Seek` handle such as a
/// `File` or a `Cursor<Vec<u8>>`; a byte slice is a source as it is.
pub trait RangedSource {
    fn size(&self) -> u64;
    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize>;
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

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        let read = self.source.read_at(offset, destination)?;
        if read <= destination.len() {
            *self.bytes_read = self.bytes_read.saturating_add(len_u64(read));
        } else if let Some(reason) = self.overread {
            return Err(Error::InvalidInput { reason });
        }
        Ok(read)
    }
}

/// A byte slice is a source of its own length.
impl RangedSource for &[u8] {
    fn size(&self) -> u64 {
        len_u64(self.len())
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        let start = usize::try_from(offset)
            .ok()
            .filter(|&start| start <= self.len())
            .ok_or(Error::InvalidInput {
                reason: "read starts beyond source size",
            })?;
        let count = destination.len().min(self.len() - start);
        destination[..count].copy_from_slice(&self[start..start + count]);
        Ok(count)
    }
}

/// Bytes read into memory, with the source offset of their first byte.
///
/// The image decoders read a payload from memory but address it in source
/// coordinates, so their errors keep naming source offsets. As a
/// [`RangedSource`] it has the size of its end offset; bytes before its start
/// are not readable.
#[derive(Clone, Copy, Debug, Default)]
pub struct Payload<'a> {
    offset: u64,
    bytes: &'a [u8],
}

impl<'a> Payload<'a> {
    /// `bytes` start at source offset `offset`; their end must fit a `u64`.
    pub fn new(offset: u64, bytes: &'a [u8]) -> Result<Self> {
        offset
            .checked_add(len_u64(bytes.len()))
            .ok_or(Error::InvalidInput {
                reason: "payload end overflows 64-bit offset",
            })?;
        Ok(Self { offset, bytes })
    }

    /// The source offset of the first byte.
    pub fn offset(&self) -> u64 {
        self.offset
    }

    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// The source offset just past the last byte.
    pub fn end(&self) -> u64 {
        // Checked by `new`.
        self.offset + len_u64(self.bytes.len())
    }

    /// The `length` bytes at source offset `start`, if all lie in this payload.
    pub fn get(&self, start: u64, length: u64) -> Option<&'a [u8]> {
        let first = usize::try_from(start.checked_sub(self.offset)?).ok()?;
        let last = first.checked_add(usize::try_from(length).ok()?)?;
        self.bytes.get(first..last)
    }
}

/// A slice starting at source offset zero.
impl<'a> From<&'a [u8]> for Payload<'a> {
    fn from(bytes: &'a [u8]) -> Self {
        Self { offset: 0, bytes }
    }
}

impl RangedSource for Payload<'_> {
    fn size(&self) -> u64 {
        self.end()
    }

    fn read_at(&mut self, offset: u64, destination: &mut [u8]) -> Result<usize> {
        let relative = offset.checked_sub(self.offset).ok_or(Error::InvalidInput {
            reason: "read starts before the payload",
        })?;
        self.bytes.read_at(relative, destination)
    }
}

/// Read `length` bytes at `offset` into `buffer`, replacing its contents, in
/// requests of at most one I/O chunk. The length must fit
/// `limits.max_allocation_bytes`; the buffer is reused between payloads.
pub fn read_payload<'a, S: RangedSource, C: Cancellation>(
    source: &mut S,
    offset: u64,
    length: u64,
    buffer: &'a mut Vec<u8>,
    limits: &Limits,
    cancellation: &C,
) -> Result<Payload<'a>> {
    limits.check_allocation(length)?;
    let size =
        usize::try_from(length).map_err(|_| limits.allocation_refused("payload bytes", length))?;
    buffer.clear();
    buffer
        .try_reserve_exact(size)
        .map_err(|_| limits.allocation_refused("payload bytes", length))?;
    buffer.resize(size, 0);
    for (index, chunk) in buffer.chunks_mut(limits.io_chunk_bytes).enumerate() {
        // Each chunk starts below `length`, which fits a `usize`.
        let at = offset
            .checked_add(len_u64(index * limits.io_chunk_bytes))
            .ok_or(Error::InvalidInput {
                reason: "payload offset overflows 64-bit offset",
            })?;
        read_exact_at(source, at, chunk, limits, cancellation)?;
    }
    Payload::new(offset, buffer)
}

/// A platform-provided cancellation signal, checked between rows, pages and
/// I/O chunks.
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
pub fn read_exact_at<S: RangedSource, C: Cancellation>(
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
        let read = source.read_at(current, remaining)?;
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

/// Write all bytes in chunks of at most `limits.io_chunk_bytes`, checking
/// the output limit before the first byte and cancellation between chunks.
///
/// `output_bytes_written` is an operation-wide counter. It counts every
/// completed chunk, including when a later chunk or cancellation fails.
pub(crate) fn write_counted<W: Write + ?Sized, C: Cancellation>(
    sink: &mut W,
    bytes: &[u8],
    output_bytes_written: &mut u64,
    limits: &Limits,
    cancellation: &C,
) -> Result<()> {
    limits.validate()?;
    let attempted = output_bytes_written
        .checked_add(len_u64(bytes.len()))
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
    for chunk in bytes.chunks(limits.io_chunk_bytes) {
        sink.write_all(chunk)?;
        // Bounded by `attempted` above.
        *output_bytes_written += len_u64(chunk.len());
        check_cancelled(cancellation)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::CancelAfter;
    use std::io;

    /// Accepts at most `limit` bytes in total, then fails.
    struct Bounded {
        bytes: Vec<u8>,
        limit: usize,
    }

    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let count = bytes.len().min(self.limit - self.bytes.len());
            if count == 0 {
                return Err(io::Error::other("sink full"));
            }
            self.bytes.extend_from_slice(&bytes[..count]);
            Ok(count)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn chunked(chunk: usize) -> Limits {
        Limits {
            io_chunk_bytes: chunk,
            ..Limits::default()
        }
    }

    #[test]
    fn counted_writes_check_the_output_limit_before_the_first_byte() {
        let mut sink = Vec::new();
        let limits = Limits {
            max_output_bytes: 5,
            ..Limits::default()
        };
        let mut count = 4;
        assert!(matches!(
            write_counted(&mut sink, b"xy", &mut count, &limits, &NeverCancel),
            Err(Error::LimitExceeded { attempted: 6, .. })
        ));
        assert_eq!((count, sink.len()), (4, 0));
        count = u64::MAX;
        assert!(matches!(
            write_counted(
                &mut sink,
                b"x",
                &mut count,
                &Limits::default(),
                &NeverCancel
            ),
            Err(Error::InvalidInput { .. })
        ));
    }

    #[test]
    fn counted_writes_count_completed_chunks_before_a_failure() {
        let mut sink = Bounded {
            bytes: Vec::new(),
            limit: 3,
        };
        let mut count = 0;
        let error =
            write_counted(&mut sink, b"abcde", &mut count, &chunked(2), &NeverCancel).unwrap_err();
        assert!(matches!(error, Error::Io(_)));
        assert_eq!((count, sink.bytes.as_slice()), (2, &b"abc"[..]));
    }

    #[test]
    fn counted_writes_observe_cancellation_between_chunks() {
        let mut sink = Vec::new();
        let mut count = 0;
        // The check before the first chunk passes; the one after it trips.
        let cancellation = CancelAfter::new(1);
        assert!(matches!(
            write_counted(&mut sink, b"abc", &mut count, &chunked(1), &cancellation),
            Err(Error::Cancelled)
        ));
        assert_eq!((count, sink.as_slice()), (1, &b"a"[..]));
    }

    #[test]
    fn core_errors_cross_an_io_write_adapter_unchanged() {
        let carried = io::Error::from(Error::Cancelled);
        assert!(matches!(Error::from(carried), Error::Cancelled));
        let plain = io::Error::other("disk failed");
        assert!(matches!(Error::from(plain), Error::Io(_)));
        let io = io::Error::from(Error::Io(io::Error::new(io::ErrorKind::BrokenPipe, "x")));
        assert_eq!(io.kind(), io::ErrorKind::BrokenPipe);
    }
}
