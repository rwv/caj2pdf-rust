// SPDX-License-Identifier: MIT

//! Opt-in Annex H.2 check using only an independently held official fixture.
//! Ordinary CI reports NOT_RUN; a requested missing or changed file fails.

use caj2pdf_core::{
    Limits, NeverCancel,
    jbig2::mq::{MQ_STATE_COUNT, MqBudget, MqContexts, MqDecoder, MqSpan, MqState, MqTable},
    native::SeekableSource,
};
use sha2::{Digest, Sha256};
use std::{
    env,
    fs::File,
    future::Future,
    io::{Cursor, Read},
    path::Path,
    pin::pin,
    task::{Context, Poll, Waker},
};

// Digest of the local, official-PDF-derived text fixture; the normative rows,
// vector bytes, and trace values are intentionally absent from this repository.
const OFFICIAL_H2_FIXTURE_SHA256: &str =
    "bdf6eeeca3bc5d5a8dc1a13acc7698ec356c886b27f6526f3e09fc2c8520ac57";
const MAX_FIXTURE_BYTES: u64 = 16 * 1024;
const H2_SYMBOLS: usize = 256;

struct Checkpoint {
    before_symbol: usize,
    interval: u32,
    code: u32,
    bit_counter: u8,
}

struct Fixture {
    states: Vec<MqState>,
    checkpoints: Vec<Checkpoint>,
    compressed: Vec<u8>,
    decisions: Vec<u8>,
}

fn decimal<T: std::str::FromStr>(word: &str) -> T {
    word.parse()
        .unwrap_or_else(|_| panic!("invalid decimal field: {word}"))
}

fn hex_bytes(word: &str) -> Vec<u8> {
    assert_eq!(word.len() % 2, 0, "hex bytes have an odd number of digits");
    word.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let nibble = |byte: u8| match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => panic!("invalid hex digit"),
            };
            nibble(pair[0]) << 4 | nibble(pair[1])
        })
        .collect()
}

fn words(line: &str, expected: usize) -> Vec<&str> {
    let values: Vec<_> = line.split_whitespace().collect();
    assert_eq!(values.len(), expected, "invalid fixture row width");
    values
}

fn parse_fixture(text: &str) -> Fixture {
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("T88-2000-H2"));
    assert_eq!(lines.next(), Some("47"));
    let mut states = Vec::with_capacity(MQ_STATE_COUNT);
    for _ in 0..MQ_STATE_COUNT {
        let row = words(lines.next().expect("missing state row"), 4);
        let switch: u8 = decimal(row[3]);
        assert!(switch <= 1, "invalid state switch");
        states.push(MqState {
            qe: decimal(row[0]),
            next_mps: decimal(row[1]),
            next_lps: decimal(row[2]),
            switch_mps: switch == 1,
        });
    }
    let count: usize = decimal(lines.next().expect("missing checkpoint count"));
    assert_eq!(count, 4, "expected four selected H.1 checkpoints");
    let mut checkpoints = Vec::with_capacity(count);
    for _ in 0..count {
        let row = words(lines.next().expect("missing checkpoint row"), 4);
        checkpoints.push(Checkpoint {
            before_symbol: decimal(row[0]),
            interval: decimal(row[1]),
            code: decimal(row[2]),
            bit_counter: decimal(row[3]),
        });
    }
    assert_eq!(
        checkpoints
            .iter()
            .map(|point| point.before_symbol)
            .collect::<Vec<_>>(),
        [0, 1, 2, 7],
        "wrong selected H.1 rows"
    );
    let compressed = hex_bytes(lines.next().expect("missing H.2 encoded bytes"));
    let decisions = hex_bytes(lines.next().expect("missing H.2 decisions"));
    assert!(lines.next().is_none(), "extra fixture content");
    assert_eq!(compressed.len(), 30, "wrong H.2 encoded length");
    assert_eq!(decisions.len(), 32, "wrong H.2 decision length");
    Fixture {
        states,
        checkpoints,
        compressed,
        decisions,
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
        Poll::Pending => panic!("local fixture source unexpectedly yielded"),
    }
}

#[test]
#[ignore = "NOT_RUN in ordinary CI; set CAJ2PDF_T88_H2_FIXTURE_FILE for a requested official Annex H.2 check"]
fn official_2000_h2_decisions_and_h1_register_checkpoints() {
    let path = env::var("CAJ2PDF_T88_H2_FIXTURE_FILE")
        .expect("requested Annex H.2 run requires CAJ2PDF_T88_H2_FIXTURE_FILE");
    let canonical = Path::new(&path)
        .canonicalize()
        .expect("external H.2 fixture is missing");
    assert!(
        canonical.starts_with("/tmp"),
        "H.2 fixture must be held under /tmp"
    );
    let mut file = File::open(&canonical).expect("cannot open external H.2 fixture");
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_FIXTURE_BYTES + 1)
        .read_to_end(&mut bytes)
        .expect("cannot read external H.2 fixture");
    assert!(
        bytes.len() as u64 <= MAX_FIXTURE_BYTES,
        "external H.2 fixture is too large"
    );
    assert_eq!(
        &Sha256::digest(&bytes)[..],
        hex_bytes(OFFICIAL_H2_FIXTURE_SHA256).as_slice(),
        "external H.2 fixture differs from the pinned official extraction"
    );
    let text = String::from_utf8(bytes).expect("external H.2 fixture is not UTF-8");
    let fixture = parse_fixture(&text);
    let limits = Limits::default();
    let budget = MqBudget {
        max_span_bytes: 30,
        max_contexts: 1,
        max_symbols: H2_SYMBOLS as u64,
        max_work: 100_000,
        max_terminal_inputs: H2_SYMBOLS as u64,
    };
    let table = MqTable::new(fixture.states, &limits).expect("invalid external E.1 state table");
    let mut contexts = MqContexts::new(1, &limits, &budget).unwrap();
    let mut source = SeekableSource::new(Cursor::new(fixture.compressed)).unwrap();
    let mut decoder = run_ready(MqDecoder::new(
        &mut source,
        MqSpan {
            offset: 0,
            length: 30,
        },
        &table,
        &mut contexts,
        &limits,
        &NeverCancel,
        budget,
    ))
    .expect("Annex H.2 initialization failed");
    for symbol in 0..H2_SYMBOLS {
        for point in &fixture.checkpoints {
            if point.before_symbol == symbol {
                let snapshot = decoder.snapshot();
                assert_eq!(
                    snapshot.interval, point.interval,
                    "A before symbol {symbol}"
                );
                assert_eq!(snapshot.code, point.code, "C before symbol {symbol}");
                assert_eq!(
                    snapshot.bit_counter, point.bit_counter,
                    "CT before symbol {symbol}"
                );
            }
        }
        let actual = run_ready(decoder.decode_bit(0))
            .unwrap_or_else(|error| panic!("H.2 decode failed at symbol {symbol}: {error}"));
        assert_eq!(
            actual,
            bit_at(&fixture.decisions, symbol),
            "H.2 symbol {symbol}"
        );
    }
    run_ready(decoder.finish(H2_SYMBOLS as u64)).expect("H.2 terminal marker mismatch");
    println!("PASS: 256 official H.2 decisions and four H.1 A/C/CT checkpoints");
}
