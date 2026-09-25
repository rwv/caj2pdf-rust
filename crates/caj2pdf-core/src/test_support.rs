// SPDX-License-Identifier: MIT

//! Shared helpers for in-crate unit tests.

use crate::Cancellation;
use std::{
    cell::Cell,
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};

/// Polls an in-memory future once and returns its output.
///
/// Test sources and sinks never wait, so a pending poll is a test bug.
pub(crate) fn ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("in-memory test future unexpectedly yielded"),
    }
}

/// Alias of [`ready`] for tests that read as "run this operation".
pub(crate) fn run<F: Future>(future: F) -> F::Output {
    ready(future)
}

/// Allows the first `allowed` cancellation queries and reports cancellation
/// from then on, counting every query.
#[derive(Debug)]
pub(crate) struct CancelAfter {
    queries: Cell<u64>,
    allowed: u64,
}

impl CancelAfter {
    pub(crate) fn new(allowed: u64) -> Self {
        Self {
            queries: Cell::new(0),
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
        self.queries.get()
    }
}

impl Cancellation for CancelAfter {
    fn is_cancelled(&self) -> bool {
        let seen = self.queries.get();
        self.queries.set(seen + 1);
        seen >= self.allowed
    }
}

#[test]
#[should_panic(expected = "in-memory test future unexpectedly yielded")]
fn ready_rejects_a_future_that_yields() {
    ready(std::future::pending::<()>());
}
