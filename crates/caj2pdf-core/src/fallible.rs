// SPDX-License-Identifier: MIT

//! Crate-private helpers for defensive allocation and length conversion.
//!
//! Many decoders guard allocator refusal and address-space conversion with
//! the same shape. Keeping the shape here tests each failure path once, while
//! every call site still supplies its own located error value.

// Every supported target has a pointer width of at most 64 bits, so a `usize`
// length or index always converts to `u64` without loss.
const _: () = assert!(usize::BITS <= u64::BITS);

/// Convert an in-memory length or index to a 64-bit count or offset.
#[inline]
pub(crate) const fn len_u64(value: usize) -> u64 {
    value as u64
}

// Every supported target has a pointer width of at least 32 bits, so a `u32`
// count or index always converts to `usize` without loss.
const _: () = assert!(usize::BITS >= 32);

/// Convert a 32-bit count or index to an in-memory length or index.
#[inline]
pub(crate) const fn usize_from_u32(value: u32) -> usize {
    value as usize
}

/// Convert between integer types, returning `error` when `value` does not fit.
#[inline]
pub(crate) fn try_convert<T: TryFrom<U>, U, E>(value: U, error: E) -> Result<T, E> {
    T::try_from(value).map_err(|_| error)
}

/// Accept a source's reported read count, rejecting one larger than the
/// `requested` destination length with `InvalidInput { reason }`.
#[inline]
pub(crate) fn checked_read_count(
    read: usize,
    requested: usize,
    reason: &'static str,
) -> crate::Result<usize> {
    if read > requested {
        return Err(crate::Error::InvalidInput { reason });
    }
    Ok(read)
}

/// Reserve exactly `additional` more elements, returning `error` when the
/// request overflows the capacity or the allocator refuses it.
#[inline]
pub(crate) fn reserve_exact<T, E>(vec: &mut Vec<T>, additional: usize, error: E) -> Result<(), E> {
    vec.try_reserve_exact(additional).map_err(|_| error)
}

/// Reserve at least `additional` more elements, returning `error` when the
/// request overflows the capacity or the allocator refuses it.
#[inline]
pub(crate) fn reserve<T, E>(vec: &mut Vec<T>, additional: usize, error: E) -> Result<(), E> {
    vec.try_reserve(additional).map_err(|_| error)
}

/// Push `item`, growing `items` to at least double its capacity (minimum
/// four) when it is full. A target capacity over `max_bytes` fails as
/// `LimitExceeded { resource }` without growing.
pub(crate) fn push_bounded<T>(
    items: &mut Vec<T>,
    item: T,
    max_bytes: u64,
    resource: &'static str,
) -> crate::Result<()> {
    let next = items
        .len()
        .checked_add(1)
        .ok_or(crate::Error::InvalidInput {
            reason: "PDF index length overflows address space",
        })?;
    if next > items.capacity() {
        let target = items.capacity().saturating_mul(2).max(4).max(next);
        let attempted = target
            .checked_mul(size_of::<T>())
            .ok_or(crate::Error::InvalidInput {
                reason: "PDF index allocation overflows address space",
            })?;
        let limit = crate::Error::LimitExceeded {
            resource,
            limit: max_bytes,
            attempted: len_u64(attempted),
        };
        if len_u64(attempted) > max_bytes {
            return Err(limit);
        }
        reserve_exact(items, target - items.len(), limit)?;
    }
    items.push(item);
    Ok(())
}

#[cfg(test)]
mod tests;
