<!-- SPDX-License-Identifier: MIT -->

# Bounded T.88 template-1 generic refinement bitmaps

Issue [#65](https://github.com/rwv/caj2pdf-rust/issues/65) adds an original
MIT bitmap primitive needed if a symbol in the observed HN/C8 `0x1802`
second dictionary takes the single-reference path. It is based on
[ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en),
official English PDF SHA-256
`a94850aa659f4c5267051d1e17081dc4ffd04531c3d659c6bc2835802035ec69`,
§§6.3.2–6.3.5, Table 6, Figure 13, and §6.5.8.2/Table 18. This does not
parse the `0x1802` dictionary, choose its imported symbol, decode IAAI/IAID,
or compose an image or page. Header flags alone cannot establish whether a
compressed symbol uses one-reference refinement or multi-instance aggregate
coding. The later dictionary integration must refuse unsupported aggregate
paths explicitly.

## API and context ownership

`jbig2::refinement::RefinementDecoder::new` borrows an existing caller-table
`MqDecoder`, an `IaidLayout`, one append-only `SequentialSink`, `Limits`, cancellation, and a
`RefinementBudget`. Build the layout with
`IaidContextBanks::with_bitmap_contexts(code_len, 1024, ...)` before creating
MQ. The constructor verifies the typed IAID code length, complete bank size,
and 1,024 following GR contexts. Those contexts are disjoint from the first
6,656 integer contexts and the `2^L` IAID slots. GR statistics continue
adapting across calls to `decode_bitmap`; target row history starts at zero
for each bitmap. The sink is bound for the session lifetime, so a later call
cannot silently redirect output while descriptor offsets keep accumulating.
`mq_mut()` lends the same coding unit back to the enclosing dictionary for
interleaved IADH/IADW/IAAI/IAID/RDX/RDY/IAEX decisions without losing GR
statistics, cumulative budgets, or the store offset. It refuses a poisoned
host or MQ. `flush_store()` explicitly flushes the bound sink before a later
symbol needs to reopen newly appended bytes as a reference; it does not
finish MQ. A failed or abandoned pending flush poisons the host and MQ. If
an interleaved dictionary decision fails semantically while MQ remains ready,
the dictionary must abandon the session itself. The primitive does not
initialize or finish MQ, reset contexts, or flush implicitly after each
bitmap. The host dictionary owns the coding-unit lifetime and standard
reset/carry policy.

`RefinementRequest` includes target geometry, signed `GRREFERENCEDX/Y`, the
template and TPGR flags, and a `RefinementReference`. The reference combines
a checked `SymbolDescriptor` with an explicit absolute `store_base`: the
descriptor's offset is relative to the first byte appended by its producing
dictionary. The adapter must reopen the **same** append-only storage as a
`RangedSource` after making prior writes visible. It owns temporary disk or
browser/Node storage space. The core cannot prove the identity of reference
and output adapter handles; it checks canonical row stride, exact stored length, checked base
addition, and the full descriptor range against the source's reported size
before any output. The bound target sink is sequential and may already contain
data. The returned descriptor's offset is relative to this refinement
session's first append in that same sink; the host must retain the sink's absolute base for
later reopening. A successful bitmap does not validate a later dictionary,
page, or PDF.

Only `GRTEMPLATE=1`, `TPGRON=0`, and no refinement AT fields are supported.
Template 0 and typical prediction return located `Unsupported` errors before
any pixel output. The caller must reject AT fields when dispatching this
request. T.88 Table 6 does not forbid zero dimensions and §6.3.5.6 stops
when all rows are decoded; zero geometry is therefore legal in the general
procedure but is `Unsupported` in this first bounded slice.

## Figure 13 pixel rule

The reference center for target `(x,y)` is
`(x - GRREFERENCEDX, y - GRREFERENCEDY)`. Both calculations use signed wide
coordinates, so positive, negative, and zero offsets have the same rule.
Every target or reference coordinate outside its own bitmap is zero. The
ten fixed taps are gathered in reading order, target first, into bits 9..0:

| Bit | Bitmap | Offset from target or aligned reference center |
| --- | --- | --- |
| 9 | Target | `(-1,-1)` |
| 8 | Target | `(0,-1)` |
| 7 | Target | `(+1,-1)` |
| 6 | Target | `(-1,0)` |
| 5 | Reference | `(0,-1)` |
| 4 | Reference | `(-1,0)` |
| 3 | Reference | `(0,0)` |
| 2 | Reference | `(+1,0)` |
| 1 | Reference | `(0,+1)` |
| 0 | Reference | `(+1,+1)` |

The standard permits any consistent context-bit assignment; this one also
places the reference-center tap at bit 3, as in its reading-order example.
One MQ decision is made per target pixel. No SLTP decision occurs because
`TPGRON=0`. Each row is packed MSB first, with unused low bits zero. A
sequential sink receives rows in raster order.

## Bounds, errors, and verification

Resident row storage is exactly two target rows and three reference rows:
`2*ceil(target_width/8) + 3*ceil(reference_width/8)` bytes, plus the complete
caller MQ context bank, 47-state table, and fixed 256-byte MQ input buffer.
Its bound is independent of bitmap height. Reference rows are cached by row
number, so consecutive target rows normally need at most one newly fetched
reference row after initial fill. Temporary store space is separate: the
reference owner needs the checked `reference_stride*reference_height` bytes;
the output owner needs the checked sum of target `stride*height` bytes across
session bitmaps. Platform allocator/file overhead is additional.

`RefinementBudget` and `Limits` check target/reference dimensions,
per-bitmap target/reference pixels and bytes, cumulative target pixels and
output bytes, context work, MQ decisions,
reference calls and physical bytes, source/sink request sizes, sink calls,
explicit flush calls,
single-row allocations, and combined working bytes. MQ's own budget also
limits its coding-unit decisions and work. The counter fields
(`max_pixels_per_bitmap`, `max_total_pixels`, `max_total_output_bytes`,
`max_reference_reads`, `max_reference_bytes_fetched`, `max_sink_writes`, and
`max_flushes`) must each be at most `MAX_BUDGET_COUNT` (2^48);
`RefinementDecoder::new` rejects a larger value as `LimitExceeded` naming the
field. Cumulative totals, ten context probes per pixel, and per-call counters
then fit u64 by construction. Arithmetic on caller- or input-supplied offsets
and geometry is checked; rows are allocated fallibly. Short reads and partial writes are retried. Physical truncation,
zero/overreported I/O, cancellation, MQ faults, and cap failures report typed
errors with bitmap/row/pixel progress and a source offset when applicable.
Reference call counts include attempted calls; fetched bytes count returned
bytes. Output bytes count only accepted write prefixes. The core cannot undo
partial writes. It marks the host poisoned before the first awaited step; a
failure poisons the borrowed MQ, and dropping a pending bitmap future leaves
the host poisoned so its `Drop` poisons MQ. The enclosing dictionary must
discard partial output and its coding unit.

Original tiny tests use invented 47-state MQ entries, synthetic streams,
and independently specified pixels. They verify context bits, edges,
geometry, storage, and faults, not Table E.1 or HN/C8 pixel compatibility.
The clean-clone optional refinement pixel corpus is **NOT_RUN; checked cases
= 0**. The #43 complete-image and #50 generic-only hashes cannot serve as a
per-symbol refinement oracle. A private table/corpus run without an
independent symbol-pixel reference is diagnostic only. Exact official Table
E.1 probability rows and Annex H bytes remain outside this MIT repository;
their redistribution status is tracked in
[#44](https://github.com/rwv/caj2pdf-rust/issues/44).
