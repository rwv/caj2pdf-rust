// SPDX-License-Identifier: MIT

//! Shared helpers for in-crate unit tests.

use crate::Cancellation;
use std::{
    future::Future,
    pin::pin,
    sync::atomic::{AtomicUsize, Ordering},
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
    queries: AtomicUsize,
    allowed: u64,
}

/// A shared signal that never trips. Tests that do not exercise cancellation
/// use it instead of `NeverCancel`, so their generic instantiations are the
/// same ones that the cancellation tests use.
pub(crate) static NEVER: CancelAfter = CancelAfter::new(u64::MAX);

impl CancelAfter {
    pub(crate) const fn new(allowed: u64) -> Self {
        Self {
            queries: AtomicUsize::new(0),
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
        self.queries.load(Ordering::Relaxed) as u64
    }
}

impl Cancellation for CancelAfter {
    fn is_cancelled(&self) -> bool {
        self.queries.fetch_add(1, Ordering::Relaxed) as u64 >= self.allowed
    }
}

#[test]
#[should_panic(expected = "in-memory test future unexpectedly yielded")]
fn ready_rejects_a_future_that_yields() {
    ready(std::future::pending::<()>());
}

/// Decode the bilevel XObjects in small, generated test PDFs. JPEG streams are
/// deliberately left to their existing passthrough assertions.
pub(crate) fn bilevel_pixels(pdf: &[u8]) -> Vec<Vec<u8>> {
    let mut images = Vec::new();
    let marker = b"/Subtype /Image";
    let mut from = 0;
    while let Some(at) = pdf[from..]
        .windows(marker.len())
        .position(|part| part == marker)
    {
        let at = from + at;
        let data = at
            + pdf[at..]
                .windows(7)
                .position(|part| part == b"stream\n")
                .unwrap()
            + 7;
        let dictionary = String::from_utf8_lossy(&pdf[at..data]);
        if dictionary.contains("/BitsPerComponent 1\n") {
            assert!(dictionary.contains("/Filter /FlateDecode"));
            let (pixels, consumed) = inflate(&pdf[data..]);
            images.push(pixels);
            from = data + consumed;
        } else {
            from = data;
        }
    }
    images
}

/// Inflate one zlib stream, returning its bytes and the input consumed.
fn inflate(stream: &[u8]) -> (Vec<u8>, usize) {
    use std::io::Read;
    let mut decoder = flate2::read::ZlibDecoder::new(stream);
    let mut bytes = Vec::new();
    decoder.read_to_end(&mut bytes).unwrap();
    (bytes, decoder.total_in() as usize)
}

/// Inflate the first Flate stream after the first occurrence of `marker`
/// in a small generated test PDF.
pub(crate) fn inflated_stream(pdf: &[u8], marker: &[u8]) -> Vec<u8> {
    let at = pdf
        .windows(marker.len())
        .position(|part| part == marker)
        .unwrap();
    let data = at
        + pdf[at..]
            .windows(7)
            .position(|part| part == b"stream\n")
            .unwrap()
        + 7;
    inflate(&pdf[data..]).0
}
