<!-- SPDX-License-Identifier: MIT -->

# Bounded direct-coded T.88 symbol dictionaries

Issue [#62](https://github.com/rwv/caj2pdf-rust/issues/62) implements one
arithmetic, direct-coded symbol dictionary (segment type 0) as original MIT
Rust. The immediate target is segment #1 of the measured HN/C8 type-3 image
profile. Segment #2 uses refinement/aggregation and is classified, then
rejected before any arithmetic decision or bitmap write. This primitive
does not decode a text region, compose a page, or convert an HN/C8 document.

The algorithm source is [ITU-T T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en),
English PDF SHA-256
`a94850aa659f4c5267051d1e17081dc4ffd04531c3d659c6bc2835802035ec69`:
§§6.2.5, 6.5.1–6.5.10, 7.4.2.1–7.4.2.2, Tables 16 and 28, Annex A.2,
and E.3.7–E.3.8. Exact Table E.1 probability rows and Annex H bytes are
outside this MIT repository. The caller supplies a validated `MqTable`;
[#44](https://github.com/rwv/caj2pdf-rust/issues/44) tracks their MIT
redistribution basis.

## Measured profile and classifier

The optional [metadata inventory](../tests/conformance/README.md#optional-jbig2-dictionary-header-inventory)
pins 546 images in five SHA-verified documents. Each first dictionary has
flags `0x0800`, AT `(2,-1)`, no references, page association 1, and 3–514
new/exported symbols. Its 12-byte data header precedes one bounded MQ
body. Each second dictionary has flags `0x1802`, AT `(2,-1)`, a reference
to #1, 0–318 new symbols, and 3–542 exported symbols; 57 have no new
symbols. These are header observations, not decoded-symbol evidence.

`read_dictionary_data_header` re-reads and verifies the complete caller
`SegmentHeader` within its declared span, then parses the segment-data flags,
conditional signed AT fields, counts, and exact body range. It rejects
reserved/contradictory fields, truncation, overflowing spans, and
out-of-source data before creating MQ. `DictionaryMode` distinguishes
arithmetic direct, arithmetic refinement/aggregate, Huffman direct, and
Huffman refinement/aggregate. `DirectDictionaryDecoder::new` accepts only
arithmetic direct coding, template 2 with AT `(2,-1)`, no imported symbols,
no bitmap-context reuse/retention, and page association 1. Legal modes
outside this profile return located `Unsupported` errors. In particular,
the observed `0x1802` second dictionary is never treated as direct. Legal
zero-dimension symbols are also `Unsupported` in this first slice.

## API, ownership, and state

The caller provides an absolute `RangedSource`, validated segment metadata,
`Limits`, `MqBudget`, `DictionaryBudget`, cancellation, one `MqTable`, an
`IntegerContextBanks::with_extra_contexts(1024, ...)` owner, and an
append-only `SequentialSink` for packed bitmap bytes. A native adapter may
use a bounded temporary file; browser and Node adapters may supply their
own stores. Later ranged reopening is an adapter responsibility.

`decode()` owns one MQ coding unit over the exact body span. Contexts
`0..6656` are thirteen disjoint Annex A.2 integer banks; `6656..7680` are
the 1,024 template-2 bitmap contexts. An IAID owner cannot share this exact
layout. All contexts start in state zero for this observed no-carry
dictionary. Bitmap statistics continue adapting across symbols while each
symbol begins with fresh, zeroed row history. `IADH` supplies height-class
deltas, signed `IADW` supplies width deltas and out-of-band class
terminators, and alternating `IAEX` runs determine exports. OOB is accepted
only for `IADW`. The final class still consumes its width OOB; there is no
additional height delta after the requested new-symbol count. A zero-new
dictionary has no height-class decisions. T.88 §6.5.10 starts by decoding
one IAEX run even when the combined imported/new symbol count is zero; in
that case its length must also be zero. Other zero-length export runs are
permitted under a finite separate run cap.

Each bitmap uses the shared [template-2 pixel context](jbig2-generic-template2.md)
with current symbol dimensions, three row buffers, MSB-first bytes, and
zero padding. `DictionaryCatalog` returns all new-symbol descriptors and
the exported descriptors in IAEX order. Checked width, height, row stride,
`relative_store_offset`, and stored length identify each bitmap relative to
the first byte appended by this decoder, even when the caller's sink already
contains bytes. A later adapter must retain that store's base and identity
when reopening a descriptor; the catalog holds no pixel arrays. Progress
reports completed symbols, physical bytes written, decoded pixels, classes,
export runs, sink writes, and an MQ snapshot. The snapshot distinguishes
semantic MQ position from physical fetches, including prefetch and terminal
lookahead. If MQ construction fails after a partial body read, the separate
`mq_initialization_bytes_fetched` field preserves those physical bytes even
though no MQ snapshot exists; `source_bytes_fetched()` includes them. The
whole dictionary MQ tail is checked once, then the sink is
flushed once.

The decoder poisons itself before its first `decode()` await. An error or
dropped pending future leaves it poisoned, possibly with partial store
output. Only a successful `DictionaryReport` validates the catalog and
store content; otherwise the caller must discard appended bytes. Repeated
`decode()` calls after success or failure are rejected.

## Memory, I/O, and limits

Working memory comprises three packed rows, 7,680 MQ contexts, a
caller-owned 47-state table, a 256-byte MQ input buffer, and two bounded
descriptor vectors. Its asymptotic resident use is
`O(max row stride + new symbols + exported symbols + contexts)`, excluding
the caller's store and platform overhead. It is **not** `O(row width)`
total memory. Temporary store bytes may grow to the checked sum of packed
symbol sizes and must be accounted for separately. The runtime core has no
whole-file/dictionary byte vector, C/FFI call, or subprocess.

`DictionaryBudget.max_data_header_bytes` bounds the symbol dictionary's
segment-data header. The separately re-read segment framing header uses
`HeaderLimits.max_header_bytes` (64 KiB by default). `DictionaryBudget`
also bounds body bytes, new/exported counts,
height classes, dimensions, per-symbol and cumulative pixels/bytes,
descriptor bytes, export-run iterations, sink writes, source/sink request
sizes, and combined working bytes. `Limits` also bounds input, output,
allocation, and I/O chunks; `MqBudget` bounds contexts, decisions, and
work. Counts and geometry are checked before allocating rows or decoding
their pixels. Row and catalog allocations are fallible. Short reads and
partial writes are allowed; zero/overreported operations, cancellation,
truncation, malformed markers, and dropped futures produce typed located
errors and progress. The optional corpus harness performs pre/post SHA
checks to detect external source mutation beyond the generic ranged-source
contract.

The direct slice fixes its imported-symbol count at zero and rejects any
dictionary reference before MQ work.

An optional native measurement used the largest encoded #1 body in the
SHA-pinned inventory: `pull-72` page 129, image 1, 9,947 MQ body bytes and
310 new/exported symbols. The source SHA-256 is
`01558ff7e30c3131c79fc3267e6eb99c055cc9cb8e1b0e710bc6bd0791c888ab`.
Run the checked, release-mode probe on Linux with the private table fixture:

```sh
cargo run --locked --release -p caj2pdf-core --example jbig2_dictionary_metrics -- \
  /tmp/caj2pdf-ranged-corpus /tmp/caj2pdf-t88-2000-h2-private.fixture
```

On 2026-09-25 UTC, one run decoded 355,939 pixels and 357,701 MQ decisions.
The append-only temporary file held **50,211 logical bytes** and occupied
**53,248 filesystem-allocated bytes** on that Linux `/tmp` filesystem. The
process reached **1,940 KiB VmHWM**. This process peak includes the probe's
64 KiB hash buffer, table parsing, runtime, and decoder; it is not the
decoder's isolated heap peak. The decoder reported 9,970 fetched bytes for
its framing/data/MQ reads. The probe pins the source and selected data-span
SHA before and after, validates metadata counts and stored byte length, and
removes its temporary file after success or failure. It does not compare
symbol pixels with an independent oracle. The formula and caps above state
configured bounds, and this run measures one external dictionary only.
WASM runtime memory is **NOT_MEASURED**; browser/Node conversion has not
wired this primitive yet.

## Evidence and remaining work

Clean-clone tests use original tiny synthetic headers, an invented 47-state
MQ table and encoded bytes, and independently chosen control/bitmap
expectations. They verify API and failure behavior, not standard-table or
HN/C8 compatibility. The optional metadata runner confirms 546/546 #1/#2
header records and 1,638 image/dictionary span hashes in five type-3
sources. It also checks the identities of all 27 selected HN/C8 sources
before and after the run. Missing optional corpus means
`NOT_RUN` and zero matches; explicitly supplied invalid material fails.
There is no independent per-symbol pixel oracle, so symbol compatibility
is `NOT_RUN` with **zero** checked cases. The #43 whole-image and #50
generic-only pixel hashes cannot substitute for it.

The shared template-2 context function was exposed internally for the
dictionary without changing its pixel rule. A same-run optional #50
generic-only regression on 2026-09-25 still matched **546/546** Rust region
hashes and **546/546** black-box tool agreements, with zero failures and
27 source SHA checks before and after. It covered 4,263,929,076 generic
pixels and says nothing about dictionary symbol pixels.

[#9](https://github.com/rwv/caj2pdf-rust/issues/9) still requires the
observed `0x1802` refinement/aggregate dictionary, text-region instances,
and page composition. [#28](https://github.com/rwv/caj2pdf-rust/issues/28)
still owns HN/C8 document conversion and storage integration.
