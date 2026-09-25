<!-- SPDX-License-Identifier: MIT -->

# Bounded embedded JBIG2 directory

Issue [#42](https://github.com/rwv/caj2pdf-rust/issues/42) adds the next
structure-only slice of [#9](https://github.com/rwv/caj2pdf-rust/issues/9)
after the [single-header reader](jbig2-segment-header.md). The format facts
come from the English [ITU-T Recommendation T.88 (02/2000)](https://www.itu.int/rec/T-REC-T.88-200002-S/en),
§§7.1–7.4 and Annex D.3. The code and synthetic test bytes are original MIT
work. No official tables, vectors, external decoder code, or corpus documents
are included.

## API and supported structure

`jbig2::read_embedded_directory` accepts a `RangedSource` and an **exact,
contiguous** `SegmentSpan` selected by the caller. It reads each segment
header once, skips the declared data length using checked arithmetic, and
returns a `SegmentDirectory` with headers in physical source order and
absolute data spans. It never reads compressed payload bytes. A segment
whose declared data exceeds the enclosing end fails; an incomplete trailing
header also fails. Zero-data segments still advance by their header length.
An empty span returns an empty directory without source reads.
The existing `read_segment_header` exact-span API uses the same internal
prefix parser and still rejects bytes after its one declared segment.

References and retention are interpreted in **segment-number order**. T.88
§7.1 and Annex D.3 allow embedded segments to appear in any physical order;
numbers may have gaps. Directory validation checks:

- Unique segment numbers and unique references within each header; every
  reference must name an existing, lower-numbered segment.
- A page-associated segment may reference a global segment or a segment on
  the same page. A global segment cannot reference a page-associated one.
- T.88 §7.3.1 target types: symbol dictionaries and text regions refer only
  to symbol dictionaries or tables, with at most four or eight tables
  respectively; halftone regions refer to a pattern dictionary; refinement
  regions refer to an intermediate region. An intermediate region has at
  most one non-extension user. Extension segments may name any type.
- T.88 §7.2.4 retention timing: a segment with its own retain bit clear is
  not referenced, and no higher-numbered segment refers to a target after
  an earlier reference clears that target's retain bit.

The reader validates only facts available from headers. It does not parse
payload-specific table selections, extension subtypes, deferred non-retain
decoder scheduling, page information contents, region dimensions, arithmetic
modes, or compressed pixels. It does
not require every page-associated segment to have a page-information segment;
an embedding profile can impose that requirement separately. HN/C8 record
discovery, standalone JBIG2 file headers, random-access file organization,
and embedded segments separated by container bytes are outside this API.
The caller must provide a contiguous span; a standalone or random-access
file must first be interpreted by an appropriate format adapter.

## Bounds and memory

`DirectoryLimits` defaults to 64 MiB of embedded span, 4,096 segments,
16,384 total references, and 1 MiB of combined directory metadata. The
effective metadata cap is the smaller of that value and the shared
`Limits.max_allocation_bytes`. Metadata accounting includes segment entries,
the sorted number index, owned reference/retention vectors, and temporary
duplicate-reference scratch space. Each header also observes its own
`HeaderLimits` (64 KiB header, 4,096 references, 64 MiB declared data by
default). Counts, byte ranges, and allocations are checked before use;
`try_reserve_exact` reports allocation failure. I/O remains bounded by
`Limits.io_chunk_bytes`, including one-byte short reads. Cancellation is
checked during reads, between segment skips, and during graph validation.

## Optional external metadata inventory

Run `python3 scripts/jbig2_directory_inventory.py --json` in a clean clone:
it reports **NOT_RUN** without `CAJ2PDF_CORPUS_DIR`. To inspect an external
copy of the SHA-pinned corpus, run:

```sh
python3 scripts/jbig2_directory_inventory.py --corpus-dir /path/to/CAJSamples --json
```

The read-only driver revalidates source SHA-256 and Git blob IDs against
[`matrix.json`](../tests/conformance/matrix.json) before reading HN/C8
container metadata and again after Rust parsing. Repository-owned MIT
container helpers locate type-3 image records; the Rust executable then
calls `read_embedded_directory` on each record after its 48-byte DIB/palette
wrapper. The JSON report includes each image's page/index, record offset,
length, DIB dimensions, and Rust-checked segment fields and absolute data
spans. No external source or generated PDF is written to this repository.

On the available pinned local corpus, the metadata inventory passed
**546/546** records in five files: HN issue-43 105, issue-76 208, and
pull-72 228; C8 issue-58 4 and issue-66 1. Every record had five contiguous
segments numbered `0..4`, types `48/0/0/6/38`, page association 1,
references `[]/[]/[1]/[2]/[]`, and an exact final end. This is **header and
directory evidence only**. One independently inspected payload, issue-43
page 11 image 1, has text flags `0xa40c`: SBREFINE is 0 while SBRTEMPLATE
is 1, contrary to T.88 §7.4.3.1.1. The directory does not inspect that
payload, and no payload conformance or decoded-pixel claim follows from the
inventory. A decoded-pixel oracle was **NOT_RUN as part of this directory
PR**, and this inventory makes no pixel-parity claim; issue #9 stays open.
Exact T.88 Annex E arithmetic-state bytes require a separate
MIT provenance decision before inclusion.
