// SPDX-License-Identifier: MIT

//! Shared helpers for in-crate unit tests.

use crate::Cancellation;
use std::{
    future::Future,
    pin::pin,
    sync::atomic::{AtomicU64, Ordering},
    task::{Context, Poll, Waker},
};

/// Polls an in-memory future once and returns its output.
///
/// Test sources and sinks never wait, so a pending poll is a test bug.
pub(crate) fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    let poll = future.as_mut().poll(&mut context);
    let Poll::Ready(output) = poll else { yielded() };
    output
}

/// Not generic, so every `ready` instantiation shares this failure path.
fn yielded() -> ! {
    panic!("in-memory test future unexpectedly yielded")
}

/// Alias of [`ready`] for tests that read as "run this operation".
pub(crate) fn run<F: Future>(future: F) -> F::Output {
    ready(future)
}

/// Allows the first `allowed` cancellation queries and reports cancellation
/// from then on, counting every query.
#[derive(Debug)]
pub(crate) struct CancelAfter {
    queries: AtomicU64,
    allowed: u64,
}

/// A shared signal that never trips. Tests that do not exercise cancellation
/// use it instead of `NeverCancel`, so their generic instantiations are the
/// same ones that the cancellation tests use.
pub(crate) static NEVER: CancelAfter = CancelAfter::new(u64::MAX);

impl CancelAfter {
    pub(crate) const fn new(allowed: u64) -> Self {
        Self {
            queries: AtomicU64::new(0),
            allowed,
        }
    }

    /// A signal that never trips, used to count a run's checkpoints.
    pub(crate) fn never() -> Self {
        Self::new(u64::MAX)
    }

    /// A signal that is cancelled from the first query.
    pub(crate) fn always() -> Self {
        Self::new(0)
    }

    /// The number of cancellation queries observed so far.
    pub(crate) fn queries(&self) -> u64 {
        self.queries.load(Ordering::Relaxed)
    }
}

impl Cancellation for CancelAfter {
    fn is_cancelled(&self) -> bool {
        self.queries.fetch_add(1, Ordering::Relaxed) >= self.allowed
    }
}

#[test]
#[should_panic(expected = "in-memory test future unexpectedly yielded")]
fn ready_rejects_a_future_that_yields() {
    ready(std::future::pending::<()>());
}
