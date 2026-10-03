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

## Paired raw page prefixes (#225)

Two additional HN-A inputs contain the same pair of four-byte `0x8003`
page-prefix records as the tagged compressed profile, followed directly by
raw records rather than `COMPRESSTEXT`:

- `issue-85` Zhouli source SHA256
  `3e22ca9ab78ac06ce0c1422fa26f7eea168b55a9cc56eed4188d77f1e437d560`:
  page 1, offset 12172, 40 bytes; one image record and a terminator.
- `issue-7/a.caj` SHA256
  `bac8e4d04f4f59a029cac852b674cd546255673f5d08cc1d60bac31be886bf57`:
  page 75, offset 5534084, 64 bytes; controls, one glyph record, one image
  record and a terminator.

Dispatch requires HN-A, both prefix tags, and `0x800a` or `0x801c` at the
next record boundary. Damaged compressed markers do not trigger a raw
fallback. The existing compact reader consumes the full indexed span and
hashes it, including the prefix and opaque tail; its image count, work limits,
short reads and cancellation contract remain unchanged. Prefix records count
against the record limit. Additional `0x8003` tags inside the body are errors.
The observed four-byte `0x80ce` control is admitted only in this raw profile
and only with its observed zero payload. Its glyph semantics are unknown.
No arbitrary unknown control is skipped and no marker scanning is used.

The existing 28-byte image record fields supply x/y/width/height. Raw text
allocates one bounded input chunk plus eight bytes per retained image and
fixed scratch, with no inflater or full-page text buffer.

A local bounded native run completes 160 pages/160 images for Zhouli and
125 pages/169 images for `a.caj`. qpdf and independently ordered image checks
pass for both. Full Node and Chromium Worker runs produce the same PDFs,
including 28 and 78 bookmarks respectively. All scratch stores are empty;
Node removes its scratch files and browser OPFS is empty after disposal.
The larger Zhouli source needs more than 64 MiB of forward-only input spooling:
that cap rejects it cleanly, while an explicit 128 MiB cap succeeds. This is
external temporary storage, not a full-source RAM allocation.

At 80% zoom in the pinned CAJViewer image, source and output page frames agree:
Zhouli page 1 is 636×899 screen pixels and `a.caj` page 75 is 602×870. Each
capture repeats identically. The latter is genuinely blank in the source
image and matches exactly. Zhouli has 552,575 changed pixels out of 571,764;
matching frames and encoded images do not establish renderer pixel parity.
This residual remains a fidelity observation under #219, not a hidden pass.
Original asymmetric mixed-image tests validate nonzero placement and draw
order independently of the real blank-page case. A process-level regression
checks a malformed second raw page after a successfully converted first page:
existing destinations survive, no partial final PDF is published, and pipe
input/native scratch are cleaned.

[Metadata and hashes](../tests/conformance/paired_raw_current.json) preserve
these distinctions and the post-conversion WASM memory observations.

The C8 issue-66 raw layout is **not** admitted by this HN-A extension. Viewer
inspection confirms visible text on its image-less page 2; page 1's sole
848×251 image is only a diagram within the text page. Thus complete conversion
requires visible native-text handling (#229), not just another image placement
header. Issue #225 remains open until that C8 requirement is satisfied.

## Raw HN-A image marker placement (#219)

Original one-page controls generated by
`tools/cajviewer/hna_image_fixture.py` establish an additional raw image form:
`800a/d300` carries `c000` marker bits in both x and width. For this observed
form, composition uses the low 14 bits of those two fields; y and height are
unchanged. This applies to the direct raw image stream and the paired raw
prefix. Public `read_text_coordinates` inspection continues returning every
original coordinate bit and unchanged source hashes. Compressed frames and
other variants are not normalized by this rule.

At main `854a7b4`, the original 320-by-240 full-page control produces a PDF
image x of 4770.109179134654 points and width of 4801.164577436312 points,
outside its approximately 31-by-23 point page. The corrected transform is
`[31.055398301657906, 0, 0, -23.29154872624343, 0, 23.29154872624343]`.
The JPEG bytes and negative-height orientation are preserved. This fixes a
reproducible off-page image, not every historical HN-A layout difference.

Pinned offline CAJViewer `cb5049d3448b6d5bcfd371cf195d522d637869dce075cb1650d74198875171de`
at 911% displayed the original marked/unmarked full-page inputs identically.
A separately authored x=20, y=30, width=280, height=180 control verified the
translation/extent behavior. The unprefixed control also matched the prefixed
source exactly. All three initial captured inputs reproduce byte-for-byte
from the committed generator. No external source document or font is used.

After the placement fix, both full-page and offset images are visible in the
same independently checked page frame. Exact pixel parity is **not** achieved:
within the unmodified page-interior crop `(648,521,1024,802)`, 5428/105656 pixels
change for the full-page control and 2342/105656 for the offset control.
Repeated source and PDF captures are each identical. Differences in raster
edges remain unresolved; they are not approved by a blanket tolerance or
used to tune coordinates. #219 retains that fidelity work and the historical
corpus comparisons. These controls alone do not close #249.

Regression tests exercise both raw framings, full-page and translated images,
short reads, unmarked values, other record prefixes and partial marker patterns.
Only the fully verified marker pattern is decoded; inspection preserves raw
values. Existing malformed/late-failure tests and resource limits still apply.
The change adds no document-sized allocation or extra source pass.

Public adapter regressions also exercise two marker-bearing images with
nonzero placement through actual WASM: 11 Node HN/C8 tests and the selected
Chromium Worker/OPFS test pass with no skips. The Worker reuses its scratch
stores, compares against the independently validated unmarked PDF, and
checks cleanup. Workspace Clippy and the existing full coverage gate pass
(30921/30921 lines). These original controls are separate from external
whole-document acceptance.


## Per-page dimensions in the paired prefix (#258)

The two initial `8003` payloads specify this HN-A page's width and height,
independently of the document header and image raster. Original controls change
only prefix width from 320 to 400 or height from 240 to 200, leaving the header,
image coordinates and JPEG bytes unchanged. Pinned offline CAJViewer changes
the page frame to the corresponding aspect ratio; the height control clips
content beyond its shorter page. Repeated interiors match. These controls
contradict treating the prefix as opaque or always using header dimensions.

`TextCoordinates::page_size` preserves the two words after complete framing
validation. The shared image-page composer uses them for the page box and image
y origin; unprefixed HN-A and other variants retain their existing geometry.
Zero words remain available for inspection but fail composition's existing
positive-dimension check. No additional source pass, page buffer or decoder is
introduced. This is a v0.x breaking metadata/output change.

The selected Zhouli page 160 has header height 8678 but prefix/image height 8676.
Earlier accepted-build output used 8678 for its page. Its old hashes and visual
receipts remain historical; revised whole-document/public-runtime acceptance is
required before closing #249. Exact pixel equality is a separate question:
original offset controls at confirmed 911% and 1862% retain a one-screen-pixel
top-edge difference, arguing against a fixed physical displacement correction.
No per-document adjustment or pixel tolerance is added.

The original generator `tools/cajviewer/hna_image_fixture.py` includes independent
width/height variants and a compressed paired control. External observations,
input identities and the excluded first height capture are recorded in
`caj2pdf-hna-zoom-residual-20261002`; source documents and captures remain outside
Git. Final corrected-frame and cross-runtime validation is still pending.


### Compressed image-marker correction

The original paired compressed control exposed an additional placement defect:
its page frame was correct, but the fixed-layout coordinate reader retained the
`c000` marker bits, placing the image outside the page. At confirmed 911%, pinned
CAJViewer renders the marked and unmarked compressed controls identically and
renders an independently offset image at the expected position. Repeated page
interiors are stable. These observations extend the existing raw HN-A rule to
the paired compressed image record `800a d300`; they do not establish a rule for
all compressed layouts or C8.

Composition now retains just the four preceding marker bytes across decoder
chunks and applies the shared marker rule to that verified record. Inspection
continues to report the original words. Nonmatching tags, partial marker bits,
C8 and the direct compressed-record path retain their prior behavior. No extra
source pass or page buffer is needed. Original two-image regression tests cover
one-byte chunks, short reads, marker variants and stable source/decoded hashes.
The generator includes compressed marked/unmarked and offset controls. Corrected
PDFs pass qpdf and show the image in the source page frame. At 911%, the full-page
control has matching black bounds and 269 differing interior pixels; repeated
captures match. The offset control still differs near the shortened page's lower
edge (5,659 pixels): its source black horizontal bar begins about nine screen
pixels above the PDF's. This remaining extent/clipping behavior is not classified
as renderer-only. #258/#261 retain that investigation and final required CI.
No pixel tolerance or compensating offset is applied. Full external measurements
are in `caj2pdf-hna-prefix-viewer-20261002/compressed-fixed-comparison.json`.
