// SPDX-License-Identifier: MIT

use super::{checked_read_count, len_u64, reserve, reserve_exact, try_convert, usize_from_u32};
use crate::Error;

#[test]
fn length_conversion_is_lossless() {
    assert_eq!(len_u64(0), 0);
    assert_eq!(len_u64(usize::MAX), usize::MAX as u64);
    assert_eq!(usize_from_u32(0), 0);
    assert_eq!(usize_from_u32(u32::MAX), u32::MAX as usize);
}

#[test]
fn conversions_return_the_supplied_error_when_out_of_range() {
    assert_eq!(try_convert::<u8, _, _>(255_u32, "unused"), Ok(255_u8));
    assert_eq!(try_convert::<u8, _, _>(256_u32, "range"), Err("range"));
    assert_eq!(try_convert::<u32, _, _>(u64::MAX, "range"), Err("range"));
}

#[test]
fn read_counts_may_not_exceed_the_request() {
    assert!(matches!(checked_read_count(0, 0, "unused"), Ok(0)));
    assert!(matches!(checked_read_count(4, 4, "unused"), Ok(4)));
    assert!(matches!(
        checked_read_count(5, 4, "overread"),
        Err(Error::InvalidInput { reason: "overread" })
    ));
}

#[test]
fn reservations_succeed_within_capacity() {
    let mut values = Vec::<u8>::new();
    assert_eq!(reserve_exact(&mut values, 3, "unused"), Ok(()));
    assert!(values.capacity() >= 3);
    assert_eq!(reserve(&mut values, 8, "unused"), Ok(()));
    assert!(values.capacity() >= 8);
}

#[test]
fn reservation_failures_return_the_supplied_error() {
    // A request beyond `isize::MAX` bytes fails as a capacity overflow without
    // asking the allocator, which models an allocator refusal deterministically.
    let mut values = Vec::<u64>::new();
    assert_eq!(
        reserve_exact(&mut values, usize::MAX, "exact"),
        Err("exact")
    );
    assert_eq!(
        reserve(&mut values, usize::MAX, "amortized"),
        Err("amortized")
    );
    assert!(values.is_empty());
}
