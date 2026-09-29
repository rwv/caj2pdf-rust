<!-- SPDX-License-Identifier: MIT -->

# Uncompressed HN-A page text

This reader extends the existing bounded text-coordinate API for the selected
issue-69 HN-A source. It does not extract searchable text or generalize the raw
profile to C8/HN-B. Source SHA-256:
`57a3c60e1d8639955c625452398d2c8a32a46a51b815a44cc9a5e0351fcb8ef4`.

## Record grammar

Read little-endian words at record boundaries, not arbitrary byte matches.
The currently admitted profile has:

1. Zero or more glyph runs. Each begins with three four-byte records: tag
   `0x8001`, then `0x8070`, then `0x8071`, each with one opaque payload word.
   Remaining four-byte glyph records have a first word below `0x8000` and an
   opaque second word. Neither character data nor glyph coordinates are kept.
2. Exactly the container's declared number of consecutive 28-byte image
   records. Each begins with tag `0x800a`; little-endian x/y coordinate words
   are at +4/+6. Other words remain uninterpreted by this reader.
3. A four-byte `0x8004` end record with an uninterpreted payload word.
4. Remaining indexed bytes are opaque. They are read and hashed within the
   same span/work limits, but never scanned for additional images.

The indexed text span must satisfy the existing container/text limits. Missing
records, wrong tag order, unknown control tags, extra images before the end
record and a missing end record fail with absolute source locations. Bounds
apply before allocation. Coordinates are published only after the complete
span has been read successfully; short reads and cancellation cannot return a
partial coordinate list.

## Independent observations and controls

All 81 pages fit this grammar, yielding 96 image records in source order:
79 type-0 images and 17 JPEGs, including 14 multi-image pages. Candidate x/y
values predict all 96 reference translations within the established
four-decimal PDF precision. Heights and orientation also agree for all draws.

Four external black-box controls changed only selected page-50 fields:

| Change | Reference output |
| --- | --- |
| Second image x: 739 to 740 | Only its horizontal translation increases, by 0.0970 pt after reference rounding. |
| Second image y: 672 to 673 | Only its vertical translation decreases, by 0.0971 pt after reference rounding. |
| Four bytes after the end record changed to `ff` | Complete output PDF remains byte-identical. |
| Glyph payload changed to the word `0x800a` | Complete output PDF remains byte-identical; it is not an image tag at a record boundary. |

These observations support the existing `240/2473` coordinate factor for this
profile. Unknown payload meanings and opaque-tail semantics remain unknown;
no vendor specification or universal layout claim is implied.

## Type-0 display width correction

The initial native run completed 81 pages and matched all 111 outlines, but
page-4 rendering failed: native width 2368 pixels, reference width 2364. Across
96 draws, 27 widths differed while every translation/height/orientation matched.
That failure and its first raster pair are retained externally.

The reference preserves the visible width when `ceil(width/8)` equals the DIB
stride. Otherwise it exposes whole padding bytes as `stride * 8` pixels. This
rule predicts all 96 draw widths. Page geometry and XObject metadata now share
one helper. It preserves all DIB row bytes; bits beyond the PDF image's visible
width are ignored by the PDF reader. Selected-image diagnostics retain their
existing visible-width contract. Synthetic tests cover byte/stride boundaries,
including widths 24, 25, 28, 31, 32 and 33, asymmetric rows and mixed images.

## API, memory and tests

**Unstable API change:** `TextCoordinates::zlib_frame` is now `Option<Span>`:
`Some(frame)` for compressed input and `None` for raw records. Both hash fields
cover the full indexed span for raw input, including the opaque tail;
`decoded_length` is that span length and `max_decoder_output_chunk_bytes` is
zero. Raw `record_count` counts glyph/control/image/end records, with an image
counted once despite its wider payload. The same `max_records` limit applies.

The raw path retains one input chunk (at most 64 KiB and the caller's I/O
limit), one 28-byte record and four bytes per declared image. It allocates no
inflater or complete text/page buffer. Working accounting includes fixed
record/hash scratch but excludes allocator overhead and process RSS. The
selected run uses at most 4 KiB per I/O request and reports 8,204 bytes of peak
text working memory. Type-0 row storage remains adapter-owned and bounded.

Original tests cover all small chunk boundaries, marker-looking glyph values,
opaque tails, record order/counts, partial records, source failures, budgets,
cancellation during records and tails, and image-only records. A synthetic
composition test produces byte-identical PDFs from equivalent compressed/raw
coordinates through the public conversion API.

All code and synthetic fixtures are original MIT work. The Python converter
was used only as a black box; no converter implementation was read or copied.
External controls used 120-second deadlines, 1 GiB address-space limits and
64 MiB file limits. Documents, copied text, PDFs, tables and rendered pages stay
outside Git under `caj2pdf-raw-text-20260929`.

## Legacy JPEG color declarations

After correcting width, the original reference matches the first 48 pages but
fails at page 49: its single-channel JPEG is declared `/DeviceRGB`. Sixteen
JPEGs in this reference have that mismatch. ImageMagick independently reports
Gray for each unchanged JPEG stream. Native output correctly uses
`/DeviceGray`, as the existing JPEG path already does; this change does not
introduce a color repair in the converter.

Following the documented Gray-reference approach from #122, a separate
external reference changes only these sixteen color declarations. All 96
encoded image streams and all 111 outlines are verified unchanged. The
original comparison remains FAIL. Comparisons against the corrected reference
are explicitly identified as such and do not establish official CAJViewer
parity; that remains #129 work.


## Final selected-source result

The corrected native run passes qpdf validation, produces all 81 pages and
111 matching ordered outlines, and matches every complete page at 300 DPI
with antialiasing disabled in both MuPDF and Poppler: **162/162** comparisons
against the separately corrected Gray reference. The source hash is unchanged.
Comparison report SHA-256: `7b4bd9e978b360e8c1c07874c766b20631df7db8317ea2d0f096218348b23be7`.
The two earlier comparison failures remain recorded. This is scoped Python /
corrected-reference evidence, not official-viewer or all-HN compatibility.

A source-header check of the previously validated issue-21 and issue-33
profiles found zero changed display widths across their 74 type-0 images.
It is a regression scope check, not a new complete-page rendering run.
