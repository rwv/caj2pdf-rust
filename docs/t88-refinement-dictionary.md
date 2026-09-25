<!-- SPDX-License-Identifier: MIT -->

# Bounded T.88 single-reference symbol dictionary

Issue [#66](https://github.com/rwv/caj2pdf-rust/issues/66) adds an original
MIT arithmetic decoder for the observed HN/C8 second symbol dictionary
(flags `0x1802`). Its algorithm source is the [official ITU-T T.88
(02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en) English PDF,
SHA-256 `a94850aa659f4c5267051d1e17081dc4ffd04531c3d659c6bc2835802035ec69`:
§§6.4.10–6.4.11, 6.5.5–6.5.10, 7.4.2.1–7.4.2.2, Tables 17–18, and
Annex A. It builds on the [direct dictionary](t88-symbol-dictionary-direct.md),
[IAID](t88-iaid.md), and [template-1 refinement bitmap](t88-refinement-template1.md)
primitives. The caller supplies MQ states. Exact official Table E.1 states
remain external pending [#44](https://github.com/rwv/caj2pdf-rust/issues/44).
This decoder is not a text-region or page renderer.

## Input and ownership

The caller supplies a validated #2 `SegmentHeader`, the referred #1
`SegmentHeader` and successful `DictionaryReport`, a ranged view of the #1
packed-bitmap store, and its absolute base. The #1 sink may already be
closed; the ranged view can be reopened independently. The constructor
re-reads and verifies #2 framing and dictionary-data headers, including its
exact body span. It accepts only one reference to the supplied earlier #1
segment, page association 1, flags `0x1802`, generic template 2 with AT
`(2,-1)`, refinement template 1 without refinement AT, and no bitmap-context
carry. It verifies #1's report/header association, complete decode status,
counts, canonical descriptor geometry, ordered subset of #1 new symbols,
nonoverlapping offsets, and each full store range before initializing #2 MQ.
The core cannot prove that an adapter's ranged view and prior sink identify
the same physical store; this is the caller's storage contract.

The new #2 symbols go to a separate append-only `SequentialSink`. For later
references to those new symbols, the caller also supplies a growing ranged
view of that same store and the absolute base at which this dictionary began
appending. The decoder calls `flush_store()` before reopening a new symbol;
the adapter must make completed writes visible after flush. A fixed-size
snapshot source of an initially empty file is unsuitable for this view.
Descriptor offsets are relative to the dictionary's first append, even if
the sink had earlier bytes. Successful exports are `StoredSymbol` handles
with `Imported` or `New` identity, their corresponding base, and a canonical
packed descriptor. An offset alone is never used to choose a store.

## Arithmetic sequence and state

One MQ decoder covers the exact #2 body. Contexts `0..6656` hold the thirteen
Annex A.2 integer procedures, followed by `2^L` IAID contexts and 1,024
nonoverlapping template-1 GR contexts. `L = ceil(log2(imported + declared
new))` is fixed throughout the dictionary, including while only a prefix
of the declared new symbols exists. `IaidContextBanks` must have exactly
this typed layout. At the dictionary boundary, integer and IAID statistics
are reset; GR statistics are cleared for the observed no-carry flags and
then retained across all refined bitmaps in #2. The target's row history
starts anew for each bitmap. The decoder never finishes or recreates MQ at
an integer, symbol, or bitmap boundary.

Height classes use signed IADH and cumulative class height. Each class uses
signed IADW and cumulative width, ending with a required IADW out-of-band
value even for the final class. Zero new symbols consume no height class.
For each new symbol, IAAI selects the next procedure. T.88 §6.5.8.2 only
defines cases greater than one (Table 17 embedded text-region aggregation)
and equal to one (Table 18 single-reference refinement). This slice treats
zero, negative, and OOB IAAI as typed malformed data, and `IAAI>1` as typed
unsupported aggregation. These are complete-value branch counts, not
header-based guesses. `IAAI=1` decodes fixed-width IAID, checks that the
ID names an imported or already completed new symbol, decodes signed IARDX
and IARDY, then refines that symbol with template 1, TPGRON=0. The current
bounded bitmap request accepts displacements representable as `i32` and
rejects wider values before output. No RDW/RDH decisions occur in this
single-reference path; Table 18 uses the given target size and signed
offsets directly.

After all requested new symbols, IAEX runs cover the concatenation of
imported and new symbols in that order. The first run is decoded even when
the total symbol count is zero. Zero-length runs are legal but bounded by a
separate run cap. Negative/OOB lengths, overshoot, or a result other than
`SDNUMEXSYMS` are malformed. Only after these checks does the decoder verify
the exact MQ terminal pair once and flush the output sink. A successful
report alone validates its new store and ordered export catalog.

## Bounds, failure, and evidence

`DictionaryBudget` limits the #2 body, new/exported counts, height classes,
dimensions, pixels, output bytes, row requests, and export runs.
`RefinementDictionaryBudget` adds imported/combined symbol, store-span,
catalog, and combined resident-memory caps. `RefinementBudget` limits
reference dimensions, reads and fetched bytes, target/reference row scratch,
GR decisions/work, output writes, explicit flushes, and per-request sizes.
`MqBudget` limits the single coding unit's contexts, decisions, work, and
terminal access. `Limits` supplies overall input/output/allocation and I/O
caps. All descriptor, span, row, cumulative, and signed-displacement
arithmetic is checked. The resident bound includes the complete MQ bank,
caller table, 256-byte input buffer, imported/new/exported descriptors, two
target rows, and three reference rows. Imported and new packed-bitmap stores
are separate temporary-space costs; no whole dictionary or image body is
loaded into a `Vec<u8>`.

Short reads and partial writes are retried by the existing bounded
primitives. Truncation, overreported/zero I/O, cancellation, marker failure,
resource caps, and malformed semantics return located typed errors with MQ
semantic position and physical fetched-byte counters. Complete IAAI branch
counts remain visible even when a later branch refuses decoding. The decoder
is poisoned before its first awaited decode step. A failure or abandoned
pending future invalidates its catalog and any partial new-store bytes; the
caller must discard the new store. The refinement host copies its final
physical I/O progress to the enclosing decoder on drop, including when a
pending future is abandoned.

Clean-clone tests use independently chosen small bitmaps, synthetic segment
headers, and an invented 47-state MQ table. They verify branch order,
reference pixels, store ownership, zero-new exports, and faults; they do not
embed Table E.1 rows or external document bytes. The optional SHA-pinned
five-file HN/C8 diagnostic decodes all 546 observed #2 bodies with a
private caller-supplied table: 546 complete dictionaries, 8,642 complete
`IAAI=1` values, zero `IAAI=0`, and zero `IAAI>1` at this pinned corpus.
This is decoder-trace evidence only. There is no independent per-symbol
pixel oracle, so external symbol-pixel compatibility is **NOT_RUN, zero
proven cases**. The #43 full-image and #50 generic-only hashes do not fill
that gap. Table 17 aggregation, text-region instances, page composition,
and integrated HN/C8-to-PDF parity remain under the open #9 hierarchy.
