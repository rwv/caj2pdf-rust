# T.88 MQ arithmetic core

This note describes [issue #45](https://github.com/rwv/caj2pdf-rust/issues/45):
an original MIT Rust decoder for the arithmetic **control flow** of one
explicitly delimited ITU-T T.88 (02/2000) MQ stream. It emits individual
decisions for a caller-selected context. It does not parse a JBIG2 image
procedure, choose pixel contexts, assemble rows, or claim CAJ image support.
The exact 47 probability states remain caller-supplied while
[issue #44](https://github.com/rwv/caj2pdf-rust/issues/44) resolves their
redistribution status.

## Source and algorithm boundary

The primary reference is the [official T.88 (02/2000) edition](https://www.itu.int/rec/T-REC-T.88-200002-S/en).
The local official English PDF used for this implementation had SHA-256
`a94850aa659f4c5267051d1e17081dc4ffd04531c3d659c6bc2835802035ec69`.
Annex E.2.6/Table E.1 specifies the caller-supplied state fields. Annex
E.2.9–E.2.10 describes FLUSH, trimming, and the terminal marker. Annex
E.3.1–E.3.5 specifies the decoder registers, decision exchanges,
renormalization, byte input, and initialization. Annex H.2/Table H.1 provides
the opt-in 256-decision check. Annex E.3.6 describes marker resynchronization
as indicative of an error and does not standardize a recovery procedure.
Annex G is an **informative alternative** coding convention; it does not
establish that a T.82 arithmetic decoder or its state table can be reused.

The module implements those public algorithms independently and imports no
decoder source. It reuses only this repository's MIT `RangedSource`,
`read_exact_at`, `Limits`, and `Cancellation` contracts. No T.88 Table
E.1 row, Annex H vector byte, or derived pixel data is in source, tests,
generated files, binaries, or packages.

T.82 `qm.rs` has a caller-provided 113-state table and receives an already
unstuffed stripe span. Its three-byte initialization, virtual zero padding,
and context carry/reset rules follow T.82. T.88 `jbig2::mq` has 47 states,
initializes from the first byte and a `BYTEIN` call followed by a seven-bit
shift, and handles `0xFF` lookahead and seven-bit stuffing within its own
span. A `0xFF 0xAC` terminal pair supplies synthetic one-bits through
`BYTEIN`; it is not T.82 virtual zero padding. The image procedure, not this
core, decides when to reset or preserve each T.88 context.

## API and bounds

`MqTable::new` accepts exactly 47 `(qe, next_mps, next_lps, switch_mps)`
rows, rejecting zero or out-of-range `qe` and any transition beyond index
46. `MqContexts::new` allocates a caller-limited number of context records
initially at state zero and MPS zero. The caller selects an absolute
`MqSpan { offset, length }` in a stable-size `RangedSource` and a
`MqBudget`. `MqDecoder::new` checks the span end, source size, input limit,
and combined table/context/buffer allocation limit before reading. Each
`decode_bit(context)` returns one bit, updating the borrowed context bank.
`finish(expected_symbols)` verifies the count and the exact terminal
`0xFF 0xAC` pair at the declared span's end. It does not prove that every
preceding source byte is semantically required by an image model.

The decoder owns a fixed 256-byte input buffer, bounded by the configured
I/O chunk size, plus constant-size registers; it borrows the state table and
context bank. Working memory is `O(context_count + 256)`, independent of
the source document and decoded image area. The work budget charges symbol
decisions, renormalization shifts, byte-input operations, and every physical
byte fetched into the buffer. Separate caps cover span length, contexts,
symbols, work, and repeated synthetic terminal inputs. Short reads are
retried within the declared span. A zero read or physical truncation within
it is a located source error; a request past its end is a terminal error.
`0xFF` followed by a nonterminal marker or by `0xAC` before the span tail
is malformed. The decoder never borrows bytes from the next segment.

`MqSnapshot` reports A/C/CT, the current absolute source offset, physical
bytes fetched (including bounded lookahead), synthetic terminal inputs,
symbol count, work, and poisoned state. A future dropped while a decision is
pending leaves the decoder poisoned because its registers may already have
changed; it refuses further decoding. Errors carry the source coordinate
and context where available. If a caller wraps or spools a source, its
original document offset mapping remains that caller's responsibility.

The local read-only inventory for #9 found `0xFF 0xAC` at the ends of all
2,184 observed arithmetic segment-data spans across 546 type-3 images.
That observation supports the declared whole-span terminal policy for this
profile; it does not establish the boundaries of internal MQ substreams or
the general validity of every JBIG2 segment. Those questions remain with
the segment and pixel-model work in #42 and #43.

## Verification and status

The required `mq_core` integration tests use an **invented** 47-row state
machine and tiny synthetic streams. They assert independently calculated
bits and A/C/CT values, `0xFF` stuffing, terminal input, source and budget
errors, cancellation, pending-future safety, and fixed-budget mutations.
These establish bounded behavior, not standard compatibility.

The ignored [Annex H.2 harness](../crates/caj2pdf-core/tests/mq_t88_external.rs)
requires an externally held text fixture under `/tmp`. Its first line is
`T88-2000-H2`, followed by `47`, 47 decimal
`qe next_mps next_lps switch` lines, `4`, then four decimal
`before_symbol A C CT` lines for positions 0, 1, 2, and 7 in Table H.1.
The final two lines are lowercase hex for the 30-byte encoded stream and
32-byte expected decisions in Annex H.2. The test caps the file at 16 KiB
and verifies SHA-256
`bdf6eeeca3bc5d5a8dc1a13acc7698ec356c886b27f6526f3e09fc2c8520ac57`
**before parsing**. No official data is checked into Git.

```sh
CAJ2PDF_T88_H2_FIXTURE_FILE=/tmp/caj2pdf-t88-2000-h2-private.fixture \
  cargo test --locked -p caj2pdf-core --test mq_t88_external -- --ignored --nocapture
```

The local run on 2026-09-24 passed all 256 Annex H.2 decisions, four
Table H.1 A/C/CT checkpoints, and the terminal marker. Ordinary CI leaves
the ignored test **NOT_RUN**. Explicitly requesting it with a missing,
changed, oversized, or malformed fixture fails. An Annex H.2 pass neither
resolves Table E.1 redistribution in #44 nor establishes HN/C8 or CAJ
pixel parity. [Issue #49](jbig2-generic-template2.md) now uses this core for
one bounded template-2 generic-region slice; the exact table rights gate in
#44 and the missing symbol/text/page procedures still prevent a standalone
or integrated production pixel decoder.
