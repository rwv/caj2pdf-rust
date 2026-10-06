// SPDX-License-Identifier: MIT

//! T.88 Annex A.3 fixed-length IAID decisions on an existing MQ stream.
//!
//! A symbol-dictionary or text-region coding unit keeps its `2^SBSYMCODELEN`
//! IAID contexts at [`IAID_BASE`], after the integer banks and the bitmap
//! contexts of [`super::integer`]. No probability table or compressed data
//! is bundled here.

use super::{
    integer::{BITMAP_BASE, BITMAP_CONTEXT_COUNT},
    mq::{ArithmeticError, ArithmeticErrorKind, ArithmeticResult, MqDecoder},
};
use crate::arith::Coder;
use crate::fallible::try_convert;
use crate::{Cancellation, RangedSource};
use std::{error, fmt};

/// The first IAID context of a coding unit.
pub const IAID_BASE: usize = BITMAP_BASE + BITMAP_CONTEXT_COUNT;

/// Decode one raw IAID value of `code_len` bits without ending the shared MQ
/// stream.
///
/// The decoder's bank must hold the `2^code_len` IAID contexts at
/// [`IAID_BASE`]; missing capacity is rejected before a decision. Every call
/// consumes exactly `code_len` MQ symbols. A zero-bit call still checks
/// cancellation and poisoned state. Context adaptation persists across calls
/// until the caller resets the bank.
pub fn decode_iaid<S: RangedSource, C: Cancellation>(
    decoder: &mut MqDecoder<'_, S, C>,
    code_len: u32,
) -> ArithmeticResult<u64> {
    let ids = 1usize.checked_shl(code_len);
    let last = ids.and_then(|ids| IAID_BASE.checked_add(ids - 1));
    if last.is_none_or(|last| decoder.context(last).is_none()) {
        return Err(ArithmeticError {
            coder: Some(Coder::T88),
            offset: Some(decoder.snapshot().input_offset),
            context: last,
            kind: ArithmeticErrorKind::InvalidContext,
        });
    }
    decoder.check_ready(Some(IAID_BASE))?;
    let mut prev = 1u64;
    for _ in 0..code_len {
        // Before each decision `prev < 2^code_len`, and the last context was
        // checked above. After the last one `prev < 2^(code_len + 1) <= 2^64`.
        let context = IAID_BASE + prev as usize;
        let bit = decoder.decode_bit(context)?;
        prev = prev * 2 + u64::from(bit);
    }
    // `ids` is a checked `usize`, so it fits u64.
    Ok(prev - ids.unwrap_or_default() as u64)
}

/// Failure at the text/dictionary symbol-array indexing boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SymbolIdError {
    EmptySymbolSet,
    TooManySymbols { count: u64 },
    SymbolArrayLength { declared: u64, actual: usize },
    OutOfRange { id: u64, count: u64 },
}

impl fmt::Display for SymbolIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySymbolSet => f.write_str("symbol count must be nonzero"),
            Self::TooManySymbols { count } => {
                write!(f, "symbol count {count} exceeds the address space")
            }
            Self::SymbolArrayLength { declared, actual } => {
                write!(f, "declared {declared} symbols, but the array has {actual}")
            }
            Self::OutOfRange { id, count } => {
                write!(f, "symbol ID {id} is outside 0..{count}")
            }
        }
    }
}

impl error::Error for SymbolIdError {}

/// Validate a raw fixed-length IAID result before indexing `SBSYMS`.
///
/// The caller supplies its active declared count and actual `SBSYMS.len()`.
/// One symbol permits `L = 0` and ID zero; zero symbols, a mismatched array,
/// and unused codewords are errors before any indexing or bitmap allocation.
pub fn checked_symbol_index(
    id: u64,
    symbol_count: u64,
    available_symbols: usize,
) -> Result<usize, SymbolIdError> {
    if symbol_count == 0 {
        return Err(SymbolIdError::EmptySymbolSet);
    }
    let too_many = SymbolIdError::TooManySymbols {
        count: symbol_count,
    };
    let count: usize = try_convert(symbol_count, too_many)?;
    if available_symbols != count {
        return Err(SymbolIdError::SymbolArrayLength {
            declared: symbol_count,
            actual: available_symbols,
        });
    }
    if id >= symbol_count {
        return Err(SymbolIdError::OutOfRange {
            id,
            count: symbol_count,
        });
    }
    // `id < symbol_count`, and `symbol_count` already converted to `usize`.
    Ok(id as usize)
}

#[cfg(test)]
mod tests;
