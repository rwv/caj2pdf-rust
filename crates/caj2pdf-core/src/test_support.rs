// SPDX-License-Identifier: MIT

//! Shared helpers for in-crate unit tests.

use crate::Cancellation;
pub(crate) use arith_encoder::{MqEncoder, QmEncoder};
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) mod arith_encoder;

/// An encoder for the standard T.88 MQ states.
pub(crate) fn mq_encoder() -> MqEncoder {
    MqEncoder::new(
        &crate::jbig2::mq::STANDARD_STATES
            .map(|state| (state.qe, state.next_mps, state.next_lps, state.switch_mps)),
    )
}

/// An encoder for the standard T.82 QM states.
pub(crate) fn qm_encoder() -> QmEncoder {
    QmEncoder::new(
        &crate::qm::STANDARD_STATES
            .map(|state| (state.qe, state.next_mps, state.next_lps, state.switch_mps)),
    )
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

/// Inflate one complete zlib stream, returning its bytes and the input
/// consumed; an invalid or truncated stream fails the test.
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

/// Text of a small generated test PDF with every content-like Flate stream
/// (one whose dictionary has only `/Length` and `/Filter`) inflated, so
/// tests can search page operators.
pub(crate) fn pdf_text(pdf: &[u8]) -> String {
    const DICTIONARY: &[u8] = b" 0 R\n/Filter /FlateDecode\n>>\nstream\n";
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = pdf[from..]
        .windows(DICTIONARY.len())
        .position(|part| part == DICTIONARY)
    {
        let data = from + at + DICTIONARY.len();
        out.extend_from_slice(&pdf[from..data]);
        let (bytes, consumed) = inflate(&pdf[data..]);
        out.extend(bytes);
        from = data + consumed;
    }
    out.extend_from_slice(&pdf[from..]);
    String::from_utf8_lossy(&out).into_owned()
}

/// A deterministic decision trace: `(context, bit)` pairs from a small LCG,
/// biased so that contexts adapt and both symbols occur.
fn trace(seed: u64, contexts: usize, length: usize) -> Vec<(usize, bool)> {
    let mut state = seed;
    (0..length)
        .map(|_| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let context = (state >> 33) as usize % contexts;
            let bit = (state >> 20).is_multiple_of(7);
            (context, bit ^ context.is_multiple_of(3))
        })
        .collect()
}

#[test]
fn encoded_mq_traces_decode_back_with_the_standard_states() {
    use crate::jbig2::mq::{CodedSpan, ContextBank, MqBudget, MqDecoder, MqTable};
    use crate::{Limits, NeverCancel, native::SeekableSource};
    for (seed, contexts, length) in [
        (1, 1, 0),
        (2, 1, 1),
        (3, 1, 500),
        (4, 7, 3000),
        (5, 300, 9000),
    ] {
        let decisions = trace(seed, contexts, length);
        let mut encoder = mq_encoder();
        for &(context, bit) in &decisions {
            encoder.encode(context, bit);
        }
        let bytes = encoder.finish();
        let limits = Limits::default();
        let budget = MqBudget::default();
        let mut bank = ContextBank::new(contexts, &limits).unwrap();
        let mut source = SeekableSource::new(std::io::Cursor::new(bytes.clone())).unwrap();
        let table = MqTable::standard();
        let mut decoder = MqDecoder::new(
            &mut source,
            CodedSpan {
                offset: 0,
                length: bytes.len() as u64,
            },
            &table,
            &mut bank,
            &limits,
            &NeverCancel,
            budget,
        )
        .unwrap();
        for &(context, bit) in &decisions {
            assert_eq!(decoder.decode_bit(context).unwrap(), bit);
        }
        decoder.finish(length as u64).unwrap();
    }
}

#[test]
fn encoded_qm_traces_decode_back_with_the_standard_states() {
    use crate::qm::{ArithmeticBudget, ArithmeticDecoder, CodedSpan, ContextBank, QmTable};
    use crate::{Limits, NeverCancel, native::SeekableSource};
    for (seed, contexts, length) in [
        (1, 1, 0),
        (2, 1, 1),
        (3, 1, 500),
        (4, 7, 3000),
        (5, 300, 9000),
    ] {
        let decisions = trace(seed, contexts, length);
        let mut encoder = qm_encoder();
        for &(context, bit) in &decisions {
            encoder.encode(context, bit);
        }
        assert_eq!(encoder.decisions(), length as u64);
        let bytes = encoder.finish();
        let limits = Limits::default();
        let mut bank = ContextBank::new(contexts, &limits).unwrap();
        let mut source = SeekableSource::new(std::io::Cursor::new(bytes.clone())).unwrap();
        let table = QmTable::standard();
        let mut decoder = ArithmeticDecoder::new(
            &mut source,
            CodedSpan {
                offset: 0,
                length: bytes.len() as u64,
            },
            &table,
            &mut bank,
            &limits,
            &NeverCancel,
            ArithmeticBudget {
                max_symbols: 10_000,
                max_work: 1_000_000,
            },
        )
        .unwrap();
        for &(context, bit) in &decisions {
            assert_eq!(decoder.decode_symbol(context).unwrap(), bit);
        }
        decoder.finish(length as u64).unwrap();
    }
}
