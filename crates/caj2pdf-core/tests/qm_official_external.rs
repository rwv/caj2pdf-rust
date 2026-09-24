// SPDX-License-Identifier: MIT

//! Opt-in conformance check against a locally supplied official T.82 vector.
//!
//! The fixture contains normative data and must remain outside this project.
//! The normal test run reports this test as ignored, never as a compatibility
//! pass. When explicitly requested, a missing or malformed fixture fails.

use caj2pdf_core::{Limits, NeverCancel, native::SeekableSource};
use sha2::{Digest, Sha256};
use std::{
    env,
    fs::File,
    future::Future,
    io::{Cursor, Read},
    pin::pin,
    task::{Context, Poll, Waker},
};

const OFFICIAL_FIXTURE_SHA256: &str =
    "11fe241dedbbf4faa542af4a1485566c2794fa69e5c06e2e5c8542adfe9b1ab7";
const MAX_FIXTURE_BYTES: u64 = 16 * 1024;

use caj2pdf_core::qm::{
    ArithmeticBudget, ArithmeticDecoder, ContextBank, EncodedSpan, QmState, QmTable, StripeMode,
};

struct Checkpoint {
    symbols_decoded: usize,
    interval: u32,
    code: u32,
    bit_counter: u8,
}

struct Fixture {
    states: Vec<QmState>,
    checkpoints: Vec<Checkpoint>,
    scd: Vec<u8>,
    contexts: Vec<u8>,
    expected: Vec<u8>,
}

fn decimal<T: std::str::FromStr>(field: &str) -> T {
    field
        .parse()
        .unwrap_or_else(|_| panic!("invalid decimal fixture field: {field}"))
}

fn hex_bytes(field: &str) -> Vec<u8> {
    let digits = field.as_bytes();
    assert_eq!(digits.len() % 2, 0, "fixture hex length must be even");
    digits
        .chunks_exact(2)
        .map(|pair| {
            let nibble = |byte: u8| match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => panic!("invalid fixture hex digit"),
            };
            (nibble(pair[0]) << 4) | nibble(pair[1])
        })
        .collect()
}

fn fields(line: &str, count: usize) -> Vec<&str> {
    let words: Vec<_> = line.split_whitespace().collect();
    assert_eq!(words.len(), count, "invalid fixture row width");
    words
}

fn fixture_from_text(text: &str) -> Fixture {
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("T82-1993"), "wrong standard edition");
    assert_eq!(lines.next(), Some("113"), "wrong normative table size");

    let mut states = Vec::with_capacity(113);
    for _ in 0..113 {
        let row = fields(lines.next().expect("missing state row"), 4);
        let switch: u8 = decimal(row[3]);
        assert!(switch <= 1, "invalid switch field");
        states.push(QmState {
            qe: decimal(row[0]),
            next_lps: decimal(row[1]),
            next_mps: decimal(row[2]),
            switch_mps: switch == 1,
        });
    }

    let checkpoint_count: usize = decimal(lines.next().expect("missing checkpoint count"));
    assert_eq!(checkpoint_count, 3, "expected the three pinned checkpoints");
    let mut checkpoints = Vec::with_capacity(checkpoint_count);
    for _ in 0..checkpoint_count {
        let row = fields(lines.next().expect("missing checkpoint row"), 4);
        checkpoints.push(Checkpoint {
            symbols_decoded: decimal(row[0]),
            interval: decimal(row[1]),
            code: decimal(row[2]),
            bit_counter: decimal(row[3]),
        });
    }
    assert_eq!(
        checkpoints
            .iter()
            .map(|checkpoint| checkpoint.symbols_decoded)
            .collect::<Vec<_>>(),
        [0, 2, 7],
        "unexpected checkpoint positions"
    );

    let scd = hex_bytes(lines.next().expect("missing SCD"));
    let contexts = hex_bytes(lines.next().expect("missing context vector"));
    let expected = hex_bytes(lines.next().expect("missing expected bits"));
    assert!(lines.next().is_none(), "extra official fixture data");
    assert_eq!(contexts.len(), 32);
    assert_eq!(expected.len(), 32);
    assert_eq!(scd.len(), 25);
    Fixture {
        states,
        checkpoints,
        scd,
        contexts,
        expected,
    }
}

fn bit_at(bytes: &[u8], index: usize) -> bool {
    bytes[index / 8] & (0x80 >> (index % 8)) != 0
}

fn run_ready<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut task = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut task) {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("local native source unexpectedly yielded"),
    }
}

#[test]
#[ignore = "requires CAJ2PDF_T82_VECTOR_FILE with external official T.82 data"]
fn official_1993_vector_and_register_checkpoints() {
    let path = env::var("CAJ2PDF_T82_VECTOR_FILE")
        .expect("explicit conformance run requires CAJ2PDF_T82_VECTOR_FILE");
    let mut file = File::open(&path).expect("requested external T.82 fixture is missing");
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_FIXTURE_BYTES + 1)
        .read_to_end(&mut bytes)
        .expect("cannot read external T.82 fixture");
    assert!(
        bytes.len() as u64 <= MAX_FIXTURE_BYTES,
        "external T.82 fixture exceeds the test size limit"
    );
    assert_eq!(
        &Sha256::digest(&bytes)[..],
        hex_bytes(OFFICIAL_FIXTURE_SHA256).as_slice(),
        "external T.82 fixture does not match the reviewed official extraction"
    );
    let text = String::from_utf8(bytes).expect("external T.82 fixture is not UTF-8");
    let fixture = fixture_from_text(&text);
    let table = QmTable::new(fixture.states).expect("invalid externally supplied state table");
    let limits = Limits::default();
    let mut contexts = ContextBank::new(2, &limits).expect("context bank allocation failed");
    let mut source = SeekableSource::new(Cursor::new(fixture.scd.clone())).unwrap();
    let mut decoder = run_ready(ArithmeticDecoder::new(
        &mut source,
        EncodedSpan {
            offset: 0,
            length: fixture.scd.len() as u64,
        },
        &table,
        &mut contexts,
        StripeMode::Reset,
        &limits,
        &NeverCancel,
        ArithmeticBudget {
            max_symbols: 256,
            max_work: 100_000,
        },
    ))
    .expect("standard stripe initialization failed");

    for symbol in 0..256 {
        for checkpoint in &fixture.checkpoints {
            if checkpoint.symbols_decoded == symbol {
                let snapshot = decoder.snapshot();
                assert_eq!(
                    snapshot.interval, checkpoint.interval,
                    "A before symbol {symbol}"
                );
                assert_eq!(snapshot.code, checkpoint.code, "C before symbol {symbol}");
                assert_eq!(
                    snapshot.bit_counter, checkpoint.bit_counter,
                    "CT before symbol {symbol}"
                );
            }
        }
        let context = usize::from(bit_at(&fixture.contexts, symbol));
        let actual = run_ready(decoder.decode_symbol(context))
            .unwrap_or_else(|error| panic!("standard decode failed at symbol {symbol}: {error}"));
        assert_eq!(actual, bit_at(&fixture.expected, symbol), "symbol {symbol}");
    }
    decoder.finish(256).expect("standard stripe did not finish");
    println!(
        "PASS: 256 official T.82 symbols and {} external A/C/CT checkpoints",
        fixture.checkpoints.len()
    );
}
