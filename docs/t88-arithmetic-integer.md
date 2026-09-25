<!-- SPDX-License-Identifier: MIT -->

# T.88 non-IAID arithmetic integers

Issue [#54](https://github.com/rwv/caj2pdf-rust/issues/54) adds an original
MIT decision layer for [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en)
Annex A.1–A.2. The local official English PDF consulted for this work had
SHA-256 `a94850aa659f4c5267051d1e17081dc4ffd04531c3d659c6bc2835802035ec69`.
Annex E.3 governs the existing MQ decision decoder. This layer consumes
decisions from one **existing** `MqDecoder`; it neither reads compressed bytes
nor ends a stream at an integer boundary. It does not implement Annex A.3
`IAID`, symbol dictionaries, bitmaps, text regions, or page composition.

## API and context ownership

`IntegerProcedure` has exactly thirteen variants: `Iaai`, `Iadh`, `Iads`,
`Iadt`, `Iadw`, `Iaex`, `Iafs`, `Iait`, `Iardh`, `Iardw`, `Iardx`, `Iardy`, and
`Iari`. There is deliberately no `Iaid` variant. `decode_integer` accepts
one of these variants and a mutable existing `MqDecoder`, then returns
`IntegerValue::Signed(i64)` or `IntegerValue::OutOfBand`. A caller must decide
whether OOB is legal for a particular dictionary/text field and must perform
any narrower range check before converting to `i32` or a size.

`IntegerContextBanks::new` allocates 13 initially zeroed, disjoint blocks
of 512 MQ contexts. The fixed map occupies indexes `0..6656` in Annex A.1
identifier order; a procedure's local context is its block start plus the
nine-bit `PREV`. `with_extra_contexts` appends slots for a caller's other MQ
models after those blocks, with checked count arithmetic. The constructor
uses #45's `MqContexts::new`, so `MqBudget.max_contexts` and
`Limits.max_allocation_bytes` apply before allocation. The decoder also
checks that its *actual* borrowed context bank reaches slot 6655 before any
integer decision, even if a caller constructed its MQ contexts directly.
There is no caller-selected per-integer offset that can accidentally choose
another integer procedure's block.

Keep one bank set across successive integers in the same coding unit so each
procedure preserves its adaptive probabilities. Call `reset` or construct a
fresh bank set at the next symbol-dictionary or segment boundary prescribed
by the enclosing decoder. The borrowed MQ decoder must be finished or dropped
before reset. A typical call sequence is:

```rust
let mut banks = IntegerContextBanks::new(limits, &budget)?;
let mut mq = MqDecoder::new(source, span, table, banks.mq_contexts_mut(),
                            limits, cancellation, budget).await?;
let delta_height = decode_integer(&mut mq, IntegerProcedure::Iadh).await?;
let delta_width = decode_integer(&mut mq, IntegerProcedure::Iadw).await?;
// Decode all other fields in the same MQ stream before finishing it.
let decisions = mq.snapshot().symbols_decoded;
mq.finish(decisions).await?;
banks.reset(); // Only when beginning the next coding unit.
```

The example uses the caller's already validated `MqSpan`, caller-provided
probability table, `Limits`, `MqBudget`, and cancellation token. It is an API
sequence, not a complete dictionary parser.

The 6,656 contexts occupy `6656 * size_of::<MqContext>()` bytes: 13,312
bytes with the 2-byte layout on this x86_64 build. The shared MQ decoder also
owns its existing fixed 256-byte input buffer and borrows the table and bank.
There is no allocation per integer and no whole-segment `Vec<u8>` path.
One integer uses at most **38** MQ decisions, all charged to the existing
symbol and work budgets. Located MQ source, marker, budget, and cancellation
errors propagate unchanged. A partial integer is not rolled back after an
error; its enclosing segment decode must stop.

## Decision rule and checks

Each invocation starts with `PREV = 1`. The sign, each selector bit, and each
payload bit are decoded at the current procedure block plus `PREV`; after
**every** bit, `PREV` is shifted with the decision and retains its leading
sentinel when the nine-bit history fills. The selector chooses payload
width/base pairs `(2, 0)`, `(4, 4)`, `(6, 20)`, `(8, 84)`, `(12, 340)`, or
`(32, 4436)`. Negative zero is OOB, distinct from numeric zero. The largest
possible magnitude, `2^32 - 1 + 4436 = 4,294,971,731`, fits `i64` but not
`i32`; checked arithmetic prevents accidental wrap or truncation.

Original synthetic tests cover the Annex A.2 IADW example, including
decision bits `0101000`, context indexes `1, 2, 5, 10, 21, 42, 84`, and
result `12`; every magnitude-band boundary with both signs; OOB; nine-bit
`PREV` rolling; all thirteen banks; repeated integers and reset; insufficient
contexts; MQ budget, cancellation, source exhaustion, and marker failures.
An invented 47-row MQ table and a synthetic encoded byte stream test the
public API on one shared MQ decoder. These tests verify decision wiring and
bounds, **not** standard-table or HN/C8 compatibility.

An independently measured symbol-dictionary integer trace is not yet
available, so that optional external comparison is **NOT_RUN** and the
compatibility count is **zero**. Annex H.2 in #45 verifies MQ decisions on
an external fixture, but it contains no Annex A.2 encoded integer trace.
The exact T.88 Table E.1 probability states and Annex H vector remain
outside this MIT repository while [#44](https://github.com/rwv/caj2pdf-rust/issues/44)
resolves their redistribution basis. No converter, private Rust, or external
decoder source was used. Later work under [#9](https://github.com/rwv/caj2pdf-rust/issues/9)
must add IAID, dictionary bitmap/refinement/aggregation models, text regions,
page composition, and external end-to-end parity.
