<!-- SPDX-License-Identifier: MIT -->

# Direct compressed HN/C8 page records

This #118 change reuses the existing bounded text inflater for frames beginning
with `COMPRESSTEXT`, without the older two tagged words. It supplies image
coordinates to the existing source-page composer; it does not expose searchable
text or enable production CLI/JS routing.

## Observed framing and implementation

The direct header is the 12-byte ASCII marker followed by a little-endian
32-bit expanded length. The remaining indexed span must be exactly one zlib
frame with that output length and a valid checksum. The old 24-byte header
continues to select its existing fixed glyph/tail layout.

Expanded direct records use little-endian words:

| Record | Bytes | Retained information |
| --- | ---: | --- |
| `0x800a` | 28 | Unsigned x/y at +4/+6, in image-record order |
| `0x8004` | 4 | Logical terminator |
| `0x8001`, `0x801c`, `0x801d`, `0x80ff`, `0x8070`, `0x8071` | 4 | Recognized control; opaque payload |
| First word below `0x8000` | 4 | Opaque glyph record |

Unknown control tags, incomplete records, missing terminators, or a different
number of image records and source descriptors are errors. Tags inside a
28-byte image payload are never scanned as record starts. Indexed bytes after
the first complete terminator remain opaque, like the existing raw reader;
they are still decompressed, hashed and included in checksum/length validation.
No repetition factor or missing image position is inferred.

The parser retains a 28-byte record buffer and the bounded coordinate vector.
Inflater/input/output buffers and the existing text budgets are reused.
`record_count` counts logical records before and including the terminator for
this profile. Original tests cover chunk sizes 1–31, short reads, embedded
false markers, truncation at every record boundary, unknown tags, count/record
limits, a 128 KiB opaque tail, checksums, source faults and cancellation.
The original complete mixed 0/2/3-page fixture now exercises this framing.

## Four-page source comparison, 2026-09-29

External issue-58, `混凝土道面评价指标分析_谢永亮.caj`, is 161,293 bytes; its
SHA-256 is `8974d024e0cbb54009419aa8c91c9ba286dd74f056c3b19524ee5c626c947c85`,
checked before and after comparison. The Python reference is black-box commit
`8cbc3c5721acb762f739434eb3d206171dbb022a`, with its external decoder libraries.
No reference source was imported. The initial unsupported-header failure is
retained; the new run converts all four source pages with one type-3 image each.

Both PDFs pass qpdf 12.2.0. The Rust PDF has 3,992,137 bytes. Aggregate native
scratch backing peaks at 1,052,576 bytes; all four stores are empty afterward.
These counters are not process RSS or validator-memory measurements.
The native example binary SHA-256 was
`e8160b4393ef6a3886ce02272d6b37bb870591b71d24b9eb261a9ad2bdadf6e0`.
The diagnostic uses `--mq-table` with the separately held fixture and its
existing T.82 fixture argument; it does not distribute either normative table.

Render each PDF in grayscale at 300 ppi with MuPDF 1.25.1 (`mutool draw -A 0`)
and Poppler 25.03.0 (`pdftoppm -aa no -aaVector no`). Compare every page pixel
under identical settings; no image alignment or mismatch threshold is applied.

| Page | MuPDF compared pixels | MuPDF changed pixels | Poppler changed pixels |
| --- | ---: | ---: | ---: |
| 1 | 7,968,688 | 0 | 11,785 |
| 2 | 7,984,896 | 0 | 11,519 |
| 3 | 7,966,680 | 0 | 13,278 |
| 4 | 7,975,424 | 0 | 9,436 |

MuPDF matches all 31,895,688 pixels. Poppler's raw comparison is **DIFFERENCE**:
46,018 pixels differ, each by exactly one grayscale level, with identical
pure-black counts. Poppler gives page 2 one additional raster row; both PDFs
receive the same dimensions within each renderer.

A separate control reverses each Rust image's stored rows and replaces
`[W,0,0,H,0,0]` with `[W,0,0,-H,0,H]`. Image content and physical placement are
unchanged. Poppler then matches all four reference pages exactly. A first-page
control changing only DeviceGray/Decode to the reference Indexed RGB palette
retains all 11,785 differences. These controls isolate the observed difference
to equivalent orientation representations in this renderer, rather than lost
pixels or incorrect placement. The original differing result is preserved;
the production path keeps its streaming top-first representation.

Receipts, hashes, render outputs and control scripts/PDFs remain in the external
`caj2pdf-issue118/reference58` bundle. This is complete-page evidence for one
four-page C8 file, not general HN/C8 or mixed-page parity. A separate pull-72
Python-reference attempt failed with a GBK decoding error; its apparent
text-record/descriptor multiplicity is unresolved. #118 remains open for
actual mixed-page mapping/comparison; #10 owns production integration and
#123/#129 own the remaining CAJViewer comparisons.
