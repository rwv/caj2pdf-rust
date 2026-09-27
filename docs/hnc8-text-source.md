<!-- SPDX-License-Identifier: MIT -->

# HN-A/C8 text framing and placement controls

This is the predeclared diagnostic plan for [issue #111](https://github.com/rwv/caj2pdf-rust/issues/111).
The source format observations below were made by bounded, read-only analysis of
the 27 SHA-pinned documents in the [#107 matrix](../tests/conformance/matrix.json).
They are empirical invariants of that corpus, not a published HN/C8
specification or proof that the same layout holds for arbitrary documents.
No external converter source was inspected or copied. Private documents,
decoded text, modified sources and PDFs remain outside this repository.

## Read-only framing discovery

The old [#110 note](hnc8-placement-experiments.md) tested zlib starts at text
offsets 0, 20, 26, 28 and 32; it did not test `+24`. All 75 HN-A/C8 pages in
the #107 oracle have one complete RFC 1950 zlib stream at text-relative `+24`,
ending exactly at the row-declared text end, with a valid Adler-32 check.
The little-endian unsigned value at `[+20,+24)` equals the decompressed byte
count. The first 20 bytes are constant within each variant and differ between
HN-A and C8. HN-B's six source pages do not have this observed framing.

For those 75 pages, decompressed size equals `8 + 16*N + 4 + 28*I`, where
`I` is the independently measured page image count. Three two-byte
little-endian marker values, `0x8070`, `0x8071` and `0x8001`, recur at
offsets `8+16*k`, `12+16*k` and `16+16*k` for every `k` in `[0,N)`.
This covers 63,977 repeated records. The largest observed HN-A text
span/frame/inflated output was 11,710/11,686/29,768 bytes; the C8 maxima
were 14,546/14,522/33,688 bytes. The final `4+28*I` bytes may contain image-related data, but no
field semantics are established by size or adjacency alone. The bounded
diagnostic validates framing, lengths, marker positions and resource ceilings;
it does not construct a PDF or expose a production text API.

The former proposed whole-donor overwrite at the beginning of the longer
target text span is invalid. Its zlib end would leave a suffix inside the
row-declared text span. It must not be submitted to the converter or counted
as a negative placement test. The #110 full-span donor transplant changed
the text bytes **and** index-row text address/length, so its 5/5 donor
translation matches implicate only that combined component.

## First black-box batch: valid fixed-row donor content

Use the two #110 discovery target/donor pairs below. Do not include other
pages or variants in this batch. Keep the target's 20-byte text prefix,
index row, text start/end, first descriptor, all image descriptors and
payloads, and total source size unchanged. Replace only the target text
`[+20,end)` with a little-endian donor decompressed size and a **single**
complete zlib frame containing the donor's decompressed text. Require the
frame's compressed byte count to equal the target's original `[+24,end)`
capacity, its Adler-32 to verify, and zero unused tail. The donor's decoded
layout must pass the same independent structural checks with the matching
image count. The compression representation is allowed to differ; this is a
text-content/framing control, not an individual coordinate-field mutation.

| Case | Target row, fixed | Target text, fixed | Donor text | Target frame capacity | Expected mutated source SHA-256 |
| --- | --- | --- | --- | ---: | --- |
| C8 p1 ← p2 | `[80,100)` | `[220,14766)` (14,546 bytes) | `[132124,142814)` (10,690 bytes) | 14,522 | `1a2eff7b1dffc3b81f559bed1c9c405fed552c5e966a1b62e6a90866d492f95f` |
| HN-A p16 ← p22 | `[16664,16684)` | `[953320,960821)` (7,501 bytes) | `[1354683,1360027)` (5,344 bytes) | 7,477 | `2f7b01ad1beaa619f5722cc88bde30ad8739423cdea2e90bbbbf5459934bc1e1` |

The deterministic in-memory feasibility check used Python `zlib.compressobj`.
For C8, use level 6, `memLevel=8`, `Z_DEFAULT_STRATEGY`, 384-byte nonempty
chunks and `Z_FULL_FLUSH` between chunks (58 flushes); for HN-A, use level 1,
`memLevel=1`, `Z_FIXED`, 488-byte chunks (24 flushes). The final source hashes
above are part of the precondition. A runtime whose zlib produces different
bytes fails before conversion; do not search for another compression schedule
during the batch. The source frame is checked by bounded decompression and
its decoded SHA-256 must equal the donor decoded SHA-256. The mutated source
must differ only inside the target original text span; the exact changed byte
runs are audited in 64 KiB chunks and recorded outside Git.

Freeze one temporary source copy per case and run the pinned black-box
converter twice on each (at most two copies and four conversions) in fresh
directories. The two resulting PDFs must agree byte for byte. Before and
after, recheck all 27 source hashes, matrix, #107 oracle, reference report,
six baseline PDFs, reference checkout revision and clean state, Python and
package/library hashes, qpdf/MuPDF/Poppler executable hashes, command,
environment and timeout. Missing or changed explicit inputs fail. A clean
clone reports `NOT_RUN` and zero private comparisons.

Parse each PDF independently with the pinned qpdf, MuPDF and Poppler tools.
Placement inference requires the original page/draw counts, MediaBoxes,
image order, dimensions and raw-stream hashes; the target's first-image CTM
and **all** non-target page CTMs must remain unchanged. Only target
supplemental translations may change, with scale/shear unchanged. Record all
ordered six-component CTMs and exact changed offset runs in the external
report. If the target supplemental translations change repeatably under these
guards, classify `TEXT_CONTENT_DEPENDENCY`; an unchanged result is negative
only for this control. A converter rejection, invalid frame, changed image
or nonlocal geometry is `UNSUPPORTED`, never placement evidence. Even donor
x/y copying with a fixed row cannot identify a coordinate field, units,
origin or general formula.

The #110 split remains descriptive: HN-A 16 discovery/7 validation and C8
20 discovery/7 validation additional draws. Their reference CTMs were
already inspected, so neither the read-only 75-page inventory nor this
two-target intervention constitutes independent validation of a placement
rule. Any later field-specific batch needs its own committed exact offsets,
predictions, guard checks and upper bound before new conversions.

## Reporting and release gate

Report `IDENTIFIED`, `PARTIAL` or `UNKNOWN`; planned, attempted, completed,
passing, failing, skipped and unsupported counts; variant/document/page
scope; counterexamples; maximum ranged request, working memory, temporary
disk and captured tool output. A skipped optional corpus run is not a pass.
If exact coordinate fields remain unknown, keep [#112](https://github.com/rwv/caj2pdf-rust/issues/112)
blocking HN/C8 supplemental-image composition. The diagnostic introduces no
production compositor or public API behavior. Release policy still requires
MIT provenance, synthetic tests, native/WASM/browser/Node/quality/license
gates, exact 100% Rust LCOV, independent review and simplification.
