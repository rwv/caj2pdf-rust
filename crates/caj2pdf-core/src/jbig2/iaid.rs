// SPDX-License-Identifier: MIT

//! T.88 Annex A.3 fixed-length IAID decisions on an existing MQ stream.
//!
//! A symbol-dictionary or text-region coding unit keeps its `2^SBSYMCODELEN`
//! IAID contexts at [`IAID_BASE`], after the integer banks and the bitmap
//! contexts of [`super::integer`]. No probability table or compressed data
//! is bundled here.

use super::{
    integer::{BITMAP_BASE, BITMAP_CONTEXT_COUNT},
    mq::MqDecoder,
};
use crate::arith::INVALID_CONTEXT;
use crate::fallible::try_convert;
use crate::{Error, Result};

/// The first IAID context of a coding unit.
pub const IAID_BASE: usize = BITMAP_BASE + BITMAP_CONTEXT_COUNT;

/// Decode one raw IAID value of `code_len` bits without ending the shared MQ
/// stream.
///
/// The decoder's bank must hold the `2^code_len` IAID contexts at
/// [`IAID_BASE`]; missing capacity is rejected before a decision. Every call
/// consumes exactly `code_len` MQ symbols. Context adaptation persists across
/// calls until the caller resets the bank.
pub fn decode_iaid(decoder: &mut MqDecoder<'_>, code_len: u32) -> Result<u64> {
    let ids = 1usize.checked_shl(code_len);
    let last = ids.and_then(|ids| IAID_BASE.checked_add(ids - 1));
    if last.is_none_or(|last| decoder.context(last).is_none()) {
        return Err(decoder.at(INVALID_CONTEXT));
    }
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

pub const EMPTY_SYMBOL_SET: &str = "symbol count must be nonzero";
pub const TOO_MANY_SYMBOLS: &str = "symbol count exceeds the address space";
pub const SYMBOL_ARRAY_LENGTH: &str = "symbol array length differs from the declared count";
pub const SYMBOL_OUT_OF_RANGE: &str = "symbol ID is outside the symbol set";

/// Validate a raw fixed-length IAID result before indexing `SBSYMS`.
///
/// The caller supplies its active declared count and actual `SBSYMS.len()`.
/// One symbol permits `L = 0` and ID zero; zero symbols, a mismatched array,
/// and unused codewords are errors before any indexing or bitmap allocation.
pub fn checked_symbol_index(id: u64, symbol_count: u64, available_symbols: usize) -> Result<usize> {
    if symbol_count == 0 {
        return Err(Error::invalid(EMPTY_SYMBOL_SET));
    }
    let count: usize = try_convert(symbol_count, Error::invalid(TOO_MANY_SYMBOLS))?;
    if available_symbols != count {
        return Err(Error::invalid(SYMBOL_ARRAY_LENGTH));
    }
    if id >= symbol_count {
        return Err(Error::invalid(SYMBOL_OUT_OF_RANGE));
    }
    // `id < symbol_count`, and `symbol_count` already converted to `usize`.
    Ok(id as usize)
}

#[cfg(test)]
mod tests;
