<!-- SPDX-License-Identifier: MIT -->

# T.88 fixed-length IAID decisions

Issue [#60](https://github.com/rwv/caj2pdf-rust/issues/60) adds an original
MIT implementation of [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en)
Annex A.3. The consulted official English PDF has SHA-256
`a94850aa659f4c5267051d1e17081dc4ffd04531c3d659c6bc2835802035ec69`.
The surrounding rules are Annex E.3 for MQ decisions, §§6.4.2 and 6.4.10
for text-region symbol IDs, §6.5.8.2.3 for the declared symbol-code length
in dictionary refinement/aggregation, and §§7.4.2–7.4.3 for segment context
ownership. This module is only the fixed-length decision layer. It is not a
symbol dictionary, text-region decoder, page renderer, or HN/C8 converter.

## Context map and API

`IaidContextBanks::new(L, limits, budget)` allocates one typed coding-unit
owner. `with_bitmap_contexts(L, B, ...)` also reserves `B` following slots.
The layout is fixed:

| Absolute context indexes | Owner |
| --- | --- |
| `0..6656` | Thirteen Annex A.2 integer banks, 512 slots each. |
| `6656..6656+2^L` | IAID bank, including unused local index zero. |
| `6656+2^L..6656+2^L+B` | Caller-requested bitmap/refinement models. |

`layout()` returns a validated, immutable `IaidLayout` with `code_len()`,
`iaid_base()`, `iaid_context_count()`, `bitmap_base()`, and `total_contexts()`.
There is no caller-selected IAID offset. The owner tags its `MqContexts` with
the fixed width; `decode_iaid` rejects an absent or changed tag and checks
the entire reserved range against the decoder's actual context capacity
before consuming a decision. To change `L`, finish/drop the decoder, discard
the owner, and create a fresh one. Retained IAID statistics cannot be read as
a different-width tree. A same-width IAID bank adapts across successive IDs
within the coding unit.

Construct the owner **before** `MqDecoder::new` to reject oversized `L` and
capacity limits without source I/O. `MqDecoder::new` itself performs bounded
initialization reads even when `L = 0`. The constructor checks `1usize << L`,
`1u64 << L`, sum of all context ranges, output representability, the actual
context allocation, `MqBudget.max_contexts`, and
`Limits.max_allocation_bytes`. It allocates fallibly. The #45 MQ allocation
formula is

```text
(6656 + 2^L + B) * size_of::<MqContext>()
  + 47 * size_of::<MqState>() + 256 bytes
```

The final 256 bytes are the existing fixed MQ input buffer. The table is
caller-supplied; its own constructor checks its 47 entries. Default
`max_contexts = 65,536` admits at most `L = 15` when IAID follows all integer
banks; other caller budgets may admit a different maximum. On wasm32, `L`
also must fit `usize` shifts. There is no per-ID allocation or whole-stream
byte buffer.

`decode_iaid(&mut mq, layout)` shares the caller's byte stream, 47-state
table, `Limits`, `MqBudget`, and cancellation token with adjacent A.2
decisions. It initializes `PREV = 1` for each ID; for each of the `L` bits it
decodes at absolute context `6656 + PREV` and appends the bit to `PREV`.
It removes the `2^L` sentinel and returns the raw unsigned `u64` result.
For `L = 3`, decisions `0,1,0` use local contexts `1,2,5` and return `2`.
The IAID call consumes exactly `L` MQ symbols; `L = 0` returns zero with no
decision, after checking cancellation and poisoned state. It does not call
`finish` or reinitialize the MQ stream. All source, marker, budget,
cancellation, and poisoned-state failures retain the MQ decoder's location
and error kind. A failure during an ID does **not** roll back earlier bits,
adapted contexts, or source position; the enclosing segment must abandon
that coding unit.

```rust
let mut banks = IaidContextBanks::with_bitmap_contexts(code_len, bitmap_slots,
                                                         limits, &budget)?;
let layout = banks.layout();
let mut mq = MqDecoder::new(source, span, table, banks.mq_contexts_mut(),
                            limits, cancellation, budget).await?;
let id = decode_iaid(&mut mq, layout).await?;
let index = checked_symbol_index(id, declared_symbol_count, symbols.len())?;
let symbol = &symbols[index];
// Decode the rest of the coding unit before calling mq.finish(...).
```

The example omits the enclosing segment parser and its other decisions.
`checked_symbol_index` rejects a zero declared count, a mismatch between
declared count and the actual `SBSYMS` array length, and a raw codeword at or
above the declared count. Thus `SBNUMSYMS = 1` can use `L = 0` and ID zero;
`SBNUMSYMS = 0` is invalid; and unused fixed-length codewords are never
silently clamped before indexing or symbol-bitmap allocation. The future
dictionary caller must compute `L` from its **declared** imported-plus-new
symbol maximum under §6.5.8.2.3, not from the number decoded so far.

T.88 §7.4.3.2 step 3 resets arithmetic statistics before each text region.
`reset_for_text_region()` clears every context through #54's `reset_all()`.
T.88 §7.4.2.2 step 5 resets **all** arithmetic-integer coders for a new
symbol dictionary, including IAID. `reset_for_symbol_dictionary()` clears
`0..6656` and IAID while preserving bitmap models that steps 3–4 and 7 may
restore or retain. The surgical `reset_non_iaid_integer_contexts()` clears
only `0..6656`; `reset_iaid_contexts()` clears only the IAID range.
The future segment decoder owns the standard's full reset/save/restore policy
for dictionary and refinement models. Finish or drop the borrowed decoder
before resetting the owner.

## Verification and remaining dependencies

Original synthetic tests cover all codewords at lengths 0–3, the official
Annex A.3 *decision* example, the default maximum admitted length 15,
context traces, disjoint adaptation with Annex A.2 on one MQ stream, repeated
IDs, resets, changed-width/capacity rejection, count-to-array validation,
32-bit shift boundary behavior, malformed/truncated input, symbol/work
budgets, cancellation, dropped pending futures, and a fixed-cap mutation
smoke. The real MQ tests use an **invented** 47-state table and original tiny
bytes. These tests prove the local control flow and failure contract, not
standard pixel compatibility.

No independently obtained IAID decision trace is currently available.
External IAID/Table E.1 compatibility is **NOT_RUN; checked cases = 0**.
The exact Table E.1 state rows, Annex H bytes, CAJSamples corpus, and derived
pixels remain outside Git, CI artifacts, packages, and releases. Bundling
the exact table remains blocked by [#44](https://github.com/rwv/caj2pdf-rust/issues/44).
Further symbol bitmap, text-region, and page-composition work is required
before any HN/C8 full-image parity claim.
