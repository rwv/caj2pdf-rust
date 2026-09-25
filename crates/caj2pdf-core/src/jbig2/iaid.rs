// SPDX-License-Identifier: MIT

//! T.88 Annex A.3 fixed-length IAID decisions on an existing MQ stream.
//!
//! The typed owner fixes `SBSYMCODELEN` for one coding unit. Its context map is
//! `0..6656` for Annex A.2 integers, `6656..6656+2^L` for IAID, followed by
//! any caller-requested bitmap/refinement contexts. No probability table or
//! compressed data is bundled here.

use super::{
    integer::{INTEGER_CONTEXT_COUNT, IntegerContextBanks},
    mq::{MqBudget, MqContext, MqDecoder, MqError, MqErrorKind, MqResult},
};
use crate::{Cancellation, Limits, RangedSource};
use std::{error, fmt};

/// Validated, immutable context map for one fixed `SBSYMCODELEN`.
///
/// Obtain this from [`IaidContextBanks::layout`]. It has no caller-selected
/// offset: the IAID bank always follows all thirteen Annex A.2 banks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IaidLayout {
    code_len: u32,
    count: usize,
    sentinel: u64,
    total_contexts: usize,
}

impl IaidLayout {
    pub fn code_len(self) -> u32 {
        self.code_len
    }

    pub fn iaid_base(self) -> usize {
        INTEGER_CONTEXT_COUNT
    }

    pub fn iaid_context_count(self) -> usize {
        self.count
    }

    /// First slot available to the caller's bitmap/refinement model.
    pub fn bitmap_base(self) -> usize {
        INTEGER_CONTEXT_COUNT + self.count
    }

    pub fn total_contexts(self) -> usize {
        self.total_contexts
    }
}

/// Context owner for one fixed-width IAID coding unit.
///
/// Keep this owner across successive IDs so IAID probabilities adapt. Drop it
/// and create a fresh owner to change `SBSYMCODELEN`. The borrowed MQ decoder
/// must be dropped or finished before calling any reset method.
#[derive(Debug)]
pub struct IaidContextBanks {
    integers: IntegerContextBanks,
    layout: IaidLayout,
}

impl IaidContextBanks {
    pub fn new(code_len: u32, limits: &Limits, budget: &MqBudget) -> MqResult<Self> {
        Self::with_bitmap_contexts(code_len, 0, limits, budget)
    }

    /// Reserve `2^L` IAID slots and `bitmap_contexts` later model slots.
    ///
    /// The complete bank, 47 caller-table states, and the fixed 256-byte MQ
    /// input buffer are checked by `MqContexts::new` against both resource
    /// policies before allocation. The context vector is allocated fallibly.
    pub fn with_bitmap_contexts(
        code_len: u32,
        bitmap_contexts: usize,
        limits: &Limits,
        budget: &MqBudget,
    ) -> MqResult<Self> {
        let invalid = || MqError {
            offset: None,
            context: None,
            kind: MqErrorKind::InvalidContext,
        };
        let count = 1usize.checked_shl(code_len).ok_or_else(invalid)?;
        let sentinel = 1u64.checked_shl(code_len).ok_or_else(invalid)?;
        // The final PREV and raw result must fit u64 on every target.
        u64::try_from(count).map_err(|_| invalid())?;
        let extra = count.checked_add(bitmap_contexts).ok_or_else(invalid)?;
        let total_contexts = INTEGER_CONTEXT_COUNT
            .checked_add(extra)
            .ok_or_else(invalid)?;
        let mut integers = IntegerContextBanks::with_extra_contexts(extra, limits, budget)?;
        integers.mq_contexts_mut().bind_iaid_code_len(code_len);
        Ok(Self {
            integers,
            layout: IaidLayout {
                code_len,
                count,
                sentinel,
                total_contexts,
            },
        })
    }

    pub fn layout(&self) -> IaidLayout {
        self.layout
    }

    pub fn mq_contexts_mut(&mut self) -> &mut super::mq::MqContexts {
        self.integers.mq_contexts_mut()
    }

    /// Clear only the thirteen Annex A.2 banks; IAID and bitmap states remain.
    /// This surgical operation is not the complete symbol-dictionary reset.
    pub fn reset_non_iaid_integer_contexts(&mut self) -> MqResult<()> {
        self.integers.reset_integer_contexts()
    }

    /// Clear only this fixed-width IAID bank; keep integer and bitmap states.
    pub fn reset_iaid_contexts(&mut self) -> MqResult<()> {
        for index in self.layout.iaid_base()..self.layout.bitmap_base() {
            self.integers
                .mq_contexts_mut()
                .set(index, MqContext::default())?;
        }
        Ok(())
    }

    /// Reset all arithmetic-integer coders for a new symbol dictionary.
    /// T.88 §7.4.2.2 step 5 includes IAID; retain appended bitmap contexts
    /// according to steps 3–4 and 7 under the enclosing segment's policy.
    pub fn reset_for_symbol_dictionary(&mut self) -> MqResult<()> {
        self.reset_non_iaid_integer_contexts()?;
        self.reset_iaid_contexts()
    }

    /// Reset every arithmetic statistic for a fresh text region.
    /// A future segment decoder owns the precise reset/save/restore policy.
    pub fn reset_for_text_region(&mut self) {
        self.integers.reset_all();
    }
}

trait DecisionSource {
    async fn bit(&mut self, context: usize) -> MqResult<bool>;
}

impl<S: RangedSource, C: Cancellation> DecisionSource for MqDecoder<'_, S, C> {
    async fn bit(&mut self, context: usize) -> MqResult<bool> {
        self.decode_bit(context).await
    }
}

async fn decode_decisions<D: DecisionSource>(source: &mut D, layout: IaidLayout) -> MqResult<u64> {
    let mut prev = 1u64;
    for _ in 0..layout.code_len {
        // The validated count guarantees PREV fits usize before each decision.
        let local = usize::try_from(prev)
            .map_err(|_| MqError::configuration(MqErrorKind::InvalidContext))?;
        let context = layout.iaid_base().checked_add(local).ok_or(MqError {
            offset: None,
            context: None,
            kind: MqErrorKind::InvalidContext,
        })?;
        let bit = source.bit(context).await?;
        prev = prev
            .checked_mul(2)
            .and_then(|value| value.checked_add(u64::from(bit)))
            .ok_or(MqError {
                offset: None,
                context: Some(context),
                kind: MqErrorKind::Invariant("IAID PREV exceeds u64"),
            })?;
    }
    Ok(prev - layout.sentinel)
}

/// Decode one raw IAID value without ending the shared MQ stream.
///
/// The decoder's contexts must come from a typed IAID owner with this width
/// and at least this layout's capacity. Both are checked before a decision.
/// Every call consumes exactly `layout.code_len()` MQ symbols. A zero-bit call
/// still checks cancellation and poisoned state. Context adaptation persists
/// across calls until the owner is reset or discarded.
pub async fn decode_iaid<S: RangedSource, C: Cancellation>(
    decoder: &mut MqDecoder<'_, S, C>,
    layout: IaidLayout,
) -> MqResult<u64> {
    let at = Some(decoder.snapshot().current_input_offset);
    if decoder.iaid_code_len() != Some(layout.code_len)
        || decoder.context(layout.total_contexts - 1).is_none()
    {
        return Err(MqError {
            offset: at,
            context: Some(layout.total_contexts - 1),
            kind: MqErrorKind::InvalidContext,
        });
    }
    decoder.check_ready(Some(layout.iaid_base()))?;
    decode_decisions(decoder, layout).await
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
    let count = usize::try_from(symbol_count)
        .ok()
        .ok_or(SymbolIdError::TooManySymbols {
            count: symbol_count,
        })?;
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
