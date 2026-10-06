// SPDX-License-Identifier: MIT

//! One bounded cursor over the header fields read before segment data.
//!
//! Segment headers and the dictionary and text-region data headers read their
//! fields with this cursor; each locates a [`FieldFault`] in its segment.

use crate::fallible::len_u64;
use crate::{Cancellation, Error, RangedSource};

/// A forward position within `start..end` whose fields are read in requests
/// of at most `request_bytes`.
pub(super) struct FieldCursor {
    pub(super) start: u64,
    pub(super) at: u64,
    pub(super) end: u64,
    /// Bytes returned by the source, including any the caller counted first.
    pub(super) fetched: u64,
    pub(super) request_bytes: usize,
}

/// Why a header field could not be read at the cursor.
pub(super) enum FieldFault {
    /// The end of a field of `expected` bytes overflows 64 bits.
    Overflow {
        expected: u64,
    },
    /// The field of `expected` bytes would end past the cursor's `end`,
    /// with `available` bytes left.
    PastEnd {
        expected: u64,
        available: u64,
    },
    Cancelled,
    Source(Error),
    /// The source reported more bytes than requested.
    Overread,
    /// The source returned no bytes with `expected` bytes of the field
    /// left; `cancelled` reports whether cancellation was requested by then.
    Ended {
        cancelled: bool,
        expected: u64,
    },
}

impl FieldFault {
    /// The unlocated error for this fault while reading `field` at `at`.
    pub(super) fn error(self, at: u64, field: &'static str) -> Error {
        match self {
            Self::Overflow { .. } => Error::malformed(at, "header field offset overflows"),
            Self::PastEnd {
                expected,
                available,
            } => Error::truncated(at, expected, available).because(field),
            Self::Cancelled
            | Self::Ended {
                cancelled: true, ..
            } => Error::cancelled().at(at),
            Self::Ended {
                cancelled: false,
                expected,
            } => Error::truncated(at, expected, 0).because(field),
            Self::Source(error) => error,
            Self::Overread => Error::malformed(at, "source returned more bytes than requested"),
        }
    }
}

impl FieldCursor {
    /// Check that `additional` more header bytes fit the range.
    pub(super) fn check_room(&self, additional: u64) -> Result<(), FieldFault> {
        let future = self
            .at
            .checked_add(additional)
            .ok_or(FieldFault::Overflow {
                expected: additional,
            })?;
        if future > self.end {
            return Err(FieldFault::PastEnd {
                expected: additional,
                available: self.end - self.at,
            });
        }
        Ok(())
    }

    /// Fill `bytes` from the cursor and advance past them. Not generic over
    /// the field width, so every field shares one instantiation per source.
    pub(super) fn fill<S: RangedSource, C: Cancellation>(
        &mut self,
        source: &mut S,
        cancellation: &C,
        bytes: &mut [u8],
    ) -> Result<(), FieldFault> {
        self.check_room(len_u64(bytes.len()))?;
        let mut done = 0;
        while done < bytes.len() {
            if cancellation.is_cancelled() {
                return Err(FieldFault::Cancelled);
            }
            let request = (bytes.len() - done).min(self.request_bytes);
            let got = match source.read_at(self.at, &mut bytes[done..done + request]) {
                Ok(got) => got,
                Err(error) => return Err(FieldFault::Source(error)),
            };
            if got > request {
                return Err(FieldFault::Overread);
            }
            if got == 0 {
                return Err(FieldFault::Ended {
                    cancelled: cancellation.is_cancelled(),
                    expected: len_u64(bytes.len() - done),
                });
            }
            // `got <= request`, so `at` stays at or below the checked end.
            self.at += len_u64(got);
            self.fetched = self.fetched.saturating_add(len_u64(got));
            done += got;
        }
        if cancellation.is_cancelled() {
            return Err(FieldFault::Cancelled);
        }
        Ok(())
    }
}
