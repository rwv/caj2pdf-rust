<!-- SPDX-License-Identifier: MIT -->

# Source-derived HN-A/C8 placement profile

This is the implementation and experiment plan for [#112](https://github.com/rwv/caj2pdf-rust/issues/112),
following the [#111 text-source investigation](hnc8-text-source.md). The
candidate below is frozen before any new private conversion. Its discovery
and validation reference values were already public and inspected; those
same-document comparisons cannot be described as blind validation.

## Frozen candidate and evidence boundary

The observed text profile consists of a variant-specific 20-byte prefix,
little-endian decoded length at +20, and one complete RFC 1950 frame at +24.
The decoded sections have length `8 + 16*N + 4 + 28*image_count`, with the
three independently measured markers in every 16-byte record. A one-based
image's trailing record begins at
`decoded_length - 28*image_count + 28*(image_number-1)`. Read its +0/+2 words
as raw little-endian 16-bit fields; interpreting bit 15 remains a separate
evidence gate. Other text/record fields remain opaque and are discarded.

For the explicitly empirical profile, freeze:

```text
point_scale = 240 / 2473
pixel_scale = 0.24
page_width = first_image_display_width * pixel_scale
page_height = first_image_height * pixel_scale
image_ctm = [image_display_width * pixel_scale, 0, 0,
             -image_height * pixel_scale,
             x_word * point_scale,
             page_height - y_word * point_scale]
```

The page's observed PDF origin is (0,0). Positive x moves rightward;
positive source y moves downward from the top edge. Type-2 dimensions come
from independently checked JPEG headers. The first type-0 raster's display
width is `dib_stride*8`, including 32-bit row padding, rather than visible
width. Independently measured source dimensions predict all 75 page boxes
and all 125 scale/shear/orientation tuples in the two reference documents;
74 first-image widths include padding. Do not reinterpret the unproven
trailing-record +4/+6 words as pixel dimensions.

The coordinate factor is calibrated from the 36-draw discovery rounding
interval; no independently established physical source unit is claimed.
The 14 same-document validation transforms were already inspected. The four
#111 positive-coordinate probes supply prospective one-variable movement
evidence, but do not resolve high-bit behavior or arbitrary document layouts.
HN-B has a different text profile and remains unsupported by this parser.

Pure geometry helpers may accept a caller-supplied finite PDF origin to
exercise fractional/negative positions. That is a caller choice, not a
newly identified source field. Preserve off-page transforms without clipping;
reject malformed dimensions and origins whose floating-point precision
cannot preserve the selected offsets. Evaluation returns unrounded f64
values. Compare all six components at 0.00005 pt absolute tolerance, matching
the reference's four-decimal serialization.

## Bounded native implementation

Read one declared page span through `RangedSource`, with separate encoded,
decoded, record-count, image-count and working-memory ceilings. Strictly
validate the observed prefix fingerprint, zlib checksum/EOF/exact end,
declared decoded length, section arithmetic and every repeated marker.
Stream through bounded input/output buffers and retain only the page's
bounded coordinate words in source image order. No complete file, decoded
text page or vector of all source pages is retained. Return coordinates only
after complete structural validation; malformed spans, reads, limits and
cancellation return located typed errors.

Use the existing locked, MIT-selected flate2 Rust backend and sha2 graph;
copy no external codec implementation. Document the decoder's opaque fixed
allocation reservation separately from handler-owned buffers and limits.
This single-pass parser needs no decoded spool. Forward-only source spooling
is an adapter responsibility under the existing bounded I/O contract.
Pure transform evaluation stays separate from parsing, image decoding, PDF
writing and browser/Node/native adapters. It adds no production compositor.

The metadata-only development example will read these two source documents
one page at a time. Its input contains no oracle CTMs, source-ID dispatch,
page-number placement table or image-hash lookup. It derives source
coordinates, image identity/order, dimensions, page boxes and transforms,
then emits bounded TSV metadata for an independent comparison diagnostic.
Require 36/36 discovery and 14/14 validation supplemental all-six transforms,
plus all 75 boxes and 75 first-image transforms; count every failure, skip
and unsupported profile explicitly. qpdf, MuPDF and Poppler must reproduce
the pinned reference metadata. All 27 original source hashes, matrix,
oracle, requested PDFs, executable identities and exact command/timeout
must be checked before and after. Clean-clone runs are `NOT_RUN` with zero
private comparisons or converter launches.

## High-bit feasibility before black-box execution

The first virtual check is preserved as metadata-only report SHA-256
`836323aa67835ad43ee619edce41f49f366b1a57ea6c57ea8315a229ae668b9c`.
It tried exactly four declared bit-15-only logical candidates, stopping on
the first candidate without an exact one-shot frame. C8 x succeeded after
441 recipes (level 9, memLevel 8, strategy 0); C8 y had no exact-length
recipe among 450; HN-A x/y were not attempted. Totals: two attempted cases,
891 recipes, 890 length mismatches, one exact frame, two skipped cases,
zero source copies and zero converter launches. An inability to fit this
bounded compression family does not falsify a coordinate interpretation.
The initial zero-recipe built-in-zlib metadata setup error is recorded
separately. Python and zlib version were pinned before/after; actual libz
binary SHA was measured afterward only, so that first report does not claim
a before/after libz-byte audit.

The next virtual check is bounded at three independent candidates, 450
one-shot recipes each: HN-A x/y with only bit 15 toggled, and C8 y changed
from 1,479 to 32,768 as one two-byte logical field. The latter is a boundary
value intervention, not a single-bit edit. Use deterministic level 0..9,
memLevel 1..9 and strategy 0..4 order, windowBits 15, one Z_FINISH, no
intermediate flushes. Stop each at its first exact original frame length;
complete all three regardless of individual NONE outcomes. Pin actual libz
bytes before/after as well as Python/runtime and source/project inputs.
Do not construct a source copy or launch a converter during feasibility.

Before any subsequent black-box run, append a committed exact case table
with accepted recipes, decoded offsets/old/new values, full mutated-source
hashes, unsigned and signed predictions, unaffected-geometry guards and
copy/run limits. No unannounced recipe search or candidate substitution is
allowed during execution. Keep all source copies, text and PDFs external.

## Acceptance and remaining support boundary

Follow #112's acceptance criteria and release policy: original MIT provenance,
meaningful malformed/range/short-read/cancellation/resource tests, native and
WASM builds, browser/Node regressions, quality/license audits, exact per-file
100% Rust LCOV, independent correctness and simplification reviews.
Report supported variants/ranges and empirical scope precisely. Any failed
rule gate leaves the result PARTIAL/UNKNOWN and #10 blocked. A validated
profile parser and transform helper alone do not prove full-page pixel parity,
other image types, text, outlines or production CLI/JS support.
