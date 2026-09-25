// SPDX-License-Identifier: MIT

//! Helpers shared by integration tests. Each test crate uses only some of
//! them, so unused items are expected.
#![allow(dead_code)]

use caj2pdf_core::Cancellation;
use std::fmt::{self, Display};
use std::{cell::Cell, rc::Rc};

/// The cancellation signal of these tests. One type serves ordinary runs,
/// checkpoint sweeps, and event-driven cancellation, so they all share one
/// decoder instantiation.
pub enum CancelAfter {
    /// Never trips. `&CancelAfter::Never` is a `'static` constant, so it can
    /// stand wherever `&NeverCancel` would.
    Never,
    /// Allows the given number of polls and reports cancellation from then
    /// on. Sweeping the count visits every cancellation checkpoint without
    /// knowing where they are.
    Polls(Cell<u64>),
    /// Reports cancellation while the shared flag is set, so a test source or
    /// sink can cancel at a chosen event.
    While(Rc<Cell<bool>>),
}

impl CancelAfter {
    pub fn new(allowed: u64) -> Self {
        Self::Polls(Cell::new(allowed))
    }
}

impl Cancellation for CancelAfter {
    fn is_cancelled(&self) -> bool {
        match self {
            Self::Never => false,
            Self::Polls(remaining) => match remaining.get() {
                0 => true,
                left => {
                    remaining.set(left - 1);
                    false
                }
            },
            Self::While(flag) => flag.get(),
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
