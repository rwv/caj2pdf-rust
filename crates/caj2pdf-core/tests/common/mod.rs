// SPDX-License-Identifier: MIT

//! Helpers shared by integration tests. Each test crate uses only some of
//! them, so unused items are expected.
#![allow(dead_code)]

use caj2pdf_core::Cancellation;
use std::cell::Cell;
use std::fmt::{self, Display};

/// Allows the first `allowed` cancellation polls and reports cancellation
/// from then on. Sweeping `allowed` visits every cancellation checkpoint
/// without knowing where they are.
pub struct CancelAfter {
    remaining: Cell<u64>,
}

impl CancelAfter {
    pub fn new(allowed: u64) -> Self {
        Self {
            remaining: Cell::new(allowed),
        }
    }
}

impl Cancellation for CancelAfter {
    fn is_cancelled(&self) -> bool {
        match self.remaining.get() {
            0 => true,
            remaining => {
                self.remaining.set(remaining - 1);
                false
            }
        }
    }
}

/// Asserts that `value` reports a failing formatter instead of ignoring it.
pub fn assert_display_propagates_fmt_error(value: &impl Display) {
    struct Refuse;

    impl fmt::Write for Refuse {
        fn write_str(&mut self, _: &str) -> fmt::Result {
            Err(fmt::Error)
        }
    }

    assert!(fmt::write(&mut Refuse, format_args!("{value}")).is_err());
}
