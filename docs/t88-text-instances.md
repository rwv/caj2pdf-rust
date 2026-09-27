<!-- SPDX-License-Identifier: MIT -->

# Bounded T.88 arithmetic text-instance stream

Issue [#86](https://github.com/rwv/caj2pdf-rust/issues/86) implements the
placement stream of a standards-valid JBIG2 type-6 text region. The format
source is [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en),
§§6.4.5–6.4.11, 7.4.3.1–7.4.3.2, Table 12, Annex A, and E.3.7. The English
PDF consulted during implementation has SHA-256
`a94850aa659f4c5267051d1e17081dc4ffd04531c3d659c6bc2835802035ec69`.
This is original MIT Rust using a caller-supplied MQ table. Exact T.88 Table
E.1 states remain external pending [#44](https://github.com/rwv/caj2pdf-rust/issues/44).

## Input and ownership

`TextInstanceDecoder::new` takes a type-6 `SegmentHeader` and parsed
`TextRegionHeader`, the referred completed #2
`RefinementDictionaryReport`, separate ranged views of the imported and new
symbol stores with explicit absolute bases, and a caller-owned append-only
temporary sink and base for refined instances. It also takes an `MqTable`,
typed integer/IAID/GR context owner, `Limits`, separate MQ, text-header,
refinement, and instance budgets, and cancellation. It rereads the header
and validates identity, page/reference relation, exact spans, dictionary
completion, ordered export handles, descriptor geometry, and backing-store
extent before MQ work. The caller must ensure each view actually names the
store that produced its dictionary handles and that the temporary sink
appends at the supplied base. The core can validate ranges and identities
but cannot prove two opaque adapters name the same physical file.

The consumer repeatedly awaits `next()` and handles one `TextInstance` at a
time. An event contains a checked top-left `(x, y)` in region-local
coordinates, symbol ID, strip index, RI bit, size, and a bitmap handle.
`TextBitmap::Stored` retains the imported/new store identity from #66;
`TextBitmap::Refined` points to bytes in the temporary store. A refined
instance never joins the dictionary catalog. Each RI=1 bitmap is flushed
before its event is returned, so a ranged view of that same temporary store
can read the handle immediately. The caller must provide an adapter whose
completed flush makes writes visible to that view. No vector of all placements
or full region bitmap is created. `None` is a successful end only after the
declared count and exact MQ terminal have been checked. Dropping or failing
a partially consumed session poisons it; the caller must discard its
temporary output.

## Decision order and geometry

One MQ coding unit spans the exact text body. All thirteen Annex A.2
integer banks, fixed-width IAID, and 1,024 template-1 GR contexts occupy
disjoint ranges. Their statistics reset at the region boundary and persist
through strips, instances, and refined bitmaps. The decoder first reads
IADT and negates it. Each strip then reads IADT multiplied by `SBSTRIPS`,
followed by IAFS for its first S coordinate. Subsequent instances use IADS;
its OOB value ends that strip. IAIT is implicit zero for one-pixel strips
and otherwise must lie inside the strip. IAID selects a checked exported
symbol. IARI is read only when `SBREFINE` is set.

For RI=1, the stream reads IARDW, IARDH, IARDX, and IARDY, derives positive
target dimensions, and applies Table 12 offsets using mathematical floor
for half of a negative delta. It calls the existing template-1, TPGRON=0
refinement decoder on the *same* MQ unit, writing packed rows to the bounded
temporary store and flushing once before the handle is emitted. The
symbol's final size determines T.88's pre-placement
and post-placement `CURS` updates. All four reference corners and transpose
modes are supported. Negative deltas, negative or overlapping placements,
and parts outside the region are valid events; clipping belongs to the
later composer. Table 11's signed 32-bit coordinate domain is checked
independently of the caller's smaller coordinate-magnitude budget. Empty
strips and arithmetic work have independent caps.

The decoder rejects Huffman, refinement template 0, malformed header
constraints, absent symbols for nonzero instances, invalid IDs or T offsets,
unexpected OOB, invalid geometry, overflow, exceeded budgets, invalid I/O,
truncation, MQ marker/terminal failures, and cancellation as located typed
errors. The optional diagnostic keeps stable nested failure tags for
header, MQ, refinement, and host I/O errors, and continues after a
standard-region header refusal. Progress includes completed instance and RI
counts, strip count, the next semantic decision, physical MQ and refinement
I/O, and temporary bytes. Budgets bound symbols, instances, strips,
coordinates, per-instance and cumulative pixels, metadata and resident
memory, store spans, temporary
bytes, row/request size, reference reads, output writes, and MQ work.
Forward-only inputs can be spooled by a platform adapter; the core requires
seekable or ranged views. Browser and Node.js adapters remain separate from
these conversion rules.

## Evidence and remaining work

The required clean-clone unit tests use small independently chosen symbol
bitmaps and invented 47-state MQ tables. They cover dirty context reset,
integer/IAID/GR state retention over two RI=1 instances, and real-MQ
negative odd refinement size deltas. They contain no Table E.1 states,
Annex H bytes, or external document payload. The optional diagnostic runs
against 27 separately held SHA-pinned CAJSamples files and a private table
under `/tmp`:

```sh
python3 scripts/jbig2_text_instance_diagnostic.py \
  --corpus-dir /path/to/CAJSamples \
  --table-fixture /tmp/private-t88-table.fixture --json
```

It validates the #42/#43/#66/#69/#85 coordinates, metadata, and selected
segment hashes, then rechecks all 27 source SHA-256 values and the private
table hash after execution. It records one per-region trace of completed
event counts, RI branches, strips, the final semantic decision, and a
SHA-256 fingerprint of the ordered event fields and bitmap handles. The
fingerprint is a local regression aid,
not an independent placement reference.

On 2026-09-27 UTC, the pinned local run attempted all 546 regions. All 545
standards-valid regions completed their MQ terminal: **353,829 instances**,
including **243,728 RI=0** and **110,101 RI=1**, across **58,220 strips**.
The one raw `0xa40c` header was rejected by the strict parser because
`SBRTEMPLATE=1` while `SBREFINE=0`; no bit is cleared or reinterpreted.
There were zero standard-region refusals. Source hashes passed 27/27 before
and after, and the private table hash passed both checks. This establishes
a complete decoder control trace for these inputs, not correct placement or
pixels. The later [#87 composer](t88-text-composer.md) matched the independent
[#85 text-only pixel baseline](jbig2-text-oracle.md) for all 545
standards-valid regions. Page composition and PDF integration remain under
[#9](https://github.com/rwv/caj2pdf-rust/issues/9).

A clean clone without either optional input reports `NOT_RUN` and zero
checked cases. An explicitly supplied missing, changed, or malformed
corpus, table, or manifest fails. The diagnostic keeps the external bytes
and table out of Git. This additive `v0.x.y` API change does not alter an
existing public signature; the project remains unstable and any later
breaking change must be documented with a Conventional Commit marker.
