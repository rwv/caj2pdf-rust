// SPDX-License-Identifier: MIT

//! One bounded cursor over the header fields read before segment data.
//!
//! Segment headers and the dictionary and text-region data headers read their
//! fields with this cursor; each maps a [`FieldFault`] to its own error.

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
    /// The field end overflows 64 bits.
    Overflow,
    /// The field would end past the cursor's `end`.
    PastEnd,
    Cancelled,
    Source(Error),
    /// The source reported more bytes than requested.
    Overread,
    /// The source returned no bytes before the field was complete;
    /// `cancelled` reports whether cancellation was requested by then.
    Ended {
        cancelled: bool,
    },
}

impl FieldCursor {
    /// Check that `additional` more header bytes fit the range.
    pub(super) fn check_room(&self, additional: u64) -> Result<(), FieldFault> {
        let future = self
            .at
            .checked_add(additional)
            .ok_or(FieldFault::Overflow)?;
        if future > self.end {
            return Err(FieldFault::PastEnd);
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
                Err(Error::Cancelled) => return Err(FieldFault::Cancelled),
                Err(error) => return Err(FieldFault::Source(error)),
            };
            if got > request {
                return Err(FieldFault::Overread);
            }
            if got == 0 {
                return Err(FieldFault::Ended {
                    cancelled: cancellation.is_cancelled(),
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
