// SPDX-License-Identifier: MIT

//! Shared, small identifiers for PDF input and incremental output.

/// An indirect PDF object reference, including its generation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PdfRef {
    pub number: u32,
    pub generation: u16,
}

/// A byte range in a positioned source. `offset` is absolute in the source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdfRange {
    pub offset: u64,
    pub length: u64,
}

impl PdfRange {
    /// The first byte beyond this range, if it fits in a 64-bit offset.
    pub const fn end(self) -> Option<u64> {
        self.offset.checked_add(self.length)
    }
}
