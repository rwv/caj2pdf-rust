// SPDX-License-Identifier: MIT

//! One zlib stream of declared compressed and decoded lengths, inflated
//! through caller-owned input and output buffers.
//!
//! The caller reads each input range [`ExactInflate::next_read`] names,
//! checks cancellation, and maps each [`InflateFault`] to its own error; the
//! decoder's progress and the length checks live here.

use crate::fallible::len_u64;
use flate2::{Decompress, FlushDecompress, Status};

pub(super) struct ExactInflate {
    inflater: Decompress,
    offset: u64,
    compressed: u64,
    decoded: u64,
    fetched: u64,
    buffered: usize,
    used: usize,
}

/// What one [`ExactInflate::step`] produced.
pub(super) struct Inflated {
    /// The input offset the step started at.
    before_in: u64,
    /// The decoded offset of the first produced byte.
    pub(super) before_out: u64,
    /// The bytes written to the start of the output window.
    pub(super) produced: usize,
    ended: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum InflateFaultKind {
    /// The stream, its dictionary or its checksum is invalid.
    Invalid,
    /// The output passed the declared decoded length.
    Excess,
    /// The stream ended away from the declared lengths.
    EndMismatch,
    /// The stream is truncated or the decoder made no progress.
    Stalled,
}

/// An inflate failure at an absolute source offset.
pub(super) struct InflateFault {
    pub(super) kind: InflateFaultKind,
    pub(super) offset: u64,
}

impl ExactInflate {
    /// Inflate `compressed` source bytes at `offset` to `decoded` bytes.
    pub(super) fn new(offset: u64, compressed: u64, decoded: u64) -> Self {
        Self {
            inflater: Decompress::new(true),
            offset,
            compressed,
            decoded,
            fetched: 0,
            buffered: 0,
            used: 0,
        }
    }

    /// The source offset of the next undecoded input byte.
    pub(super) fn position(&self) -> u64 {
        self.offset + self.inflater.total_in()
    }

    /// The decoded bytes so far.
    pub(super) fn total_out(&self) -> u64 {
        self.inflater.total_out()
    }

    /// The source offset and length to read into the start of the input
    /// buffer, of `capacity` bytes, once the buffered input is used up.
    pub(super) fn next_read(&mut self, capacity: usize) -> Option<(u64, usize)> {
        if self.used < self.buffered || self.fetched == self.compressed {
            return None;
        }
        let at = self.offset + self.fetched;
        let length = (self.compressed - self.fetched).min(len_u64(capacity)) as usize;
        self.fetched += len_u64(length);
        self.buffered = length;
        self.used = 0;
        Some((at, length))
    }

    /// The output window length that leaves one sentinel byte past the
    /// declared length, so the decoder can consume its trailer and report
    /// the stream end, or report excess output.
    pub(super) fn writable(&self, capacity: usize) -> usize {
        (self.decoded - self.inflater.total_out() + 1).min(len_u64(capacity)) as usize
    }

    /// Inflate the buffered part of `input` into `output`.
    pub(super) fn step(
        &mut self,
        input: &[u8],
        output: &mut [u8],
    ) -> Result<Inflated, InflateFault> {
        let before_in = self.inflater.total_in();
        let before_out = self.inflater.total_out();
        let status = self
            .inflater
            .decompress(
                &input[self.used..self.buffered],
                output,
                FlushDecompress::None,
            )
            .map_err(|_| self.fault(InflateFaultKind::Invalid, before_in))?;
        self.used += (self.inflater.total_in() - before_in) as usize;
        Ok(Inflated {
            before_in,
            before_out,
            produced: (self.inflater.total_out() - before_out) as usize,
            ended: status == Status::StreamEnd,
        })
    }

    /// Refuse output past the declared decoded length.
    pub(super) fn check_length(&self, step: &Inflated) -> Result<(), InflateFault> {
        if self.inflater.total_out() > self.decoded {
            return Err(self.fault(InflateFaultKind::Excess, step.before_in));
        }
        Ok(())
    }

    /// Whether the stream ended, exactly at both declared lengths.
    pub(super) fn finished(&self, step: &Inflated) -> Result<bool, InflateFault> {
        let (total_in, total_out) = (self.inflater.total_in(), self.inflater.total_out());
        if step.ended {
            if total_in != self.compressed || total_out != self.decoded {
                return Err(self.fault(InflateFaultKind::EndMismatch, total_in));
            }
            return Ok(true);
        }
        if total_in == step.before_in && total_out == step.before_out {
            return Err(self.fault(InflateFaultKind::Stalled, step.before_in));
        }
        Ok(false)
    }

    fn fault(&self, kind: InflateFaultKind, relative: u64) -> InflateFault {
        InflateFault {
            kind,
            offset: self.offset + relative,
        }
    }
}
