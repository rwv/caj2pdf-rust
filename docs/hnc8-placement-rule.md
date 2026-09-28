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

## Predeclared high-bit black-box batch

The second virtual report has SHA-256
`725992cb7137669be92a98fadaa6c1a5c14a6ef3144311111f987a2dc71734b9`.
All three candidates fit one exact-length frame: 391/391/441 recipes,
1,223 total, 1,220 length mismatches and three exact frames. All 27 sources,
matrix, oracle, Python/runtime, implementation hashes and actual libz binary
passed before/after audits. The libz binary is
`/usr/lib/x86_64-linux-gnu/libz.so.1.3.1`, SHA-256
`85590dd58edf5445e18bc7193e5ebc01ac5841f1ae187e97705a662e90c6421e`.
Maximum source range/hash request was 14,546/65,536 bytes, decoded buffer
33,688 bytes and harness VmHWM 25,100 KiB. This was virtual feasibility only:
zero source copies, converter launches or private-byte artifacts.

Freeze **four** source copies and **eight** converter launches maximum,
two per copy. All targets are image 2 on the same discovery pages as #111.
Keep every original index row, source size, outer 24 text bytes, descriptors,
image streams and every other decoded byte unchanged. Only the selected
two-byte logical word changes. Recompress with zlib 1.3.1, windowBits 15,
strategy 0, one final Z_FINISH and no intermediate flush, using the recipe
and full-source hash below. Require strict frame/marker/decoded-size checks,
original exact frame capacities and an every-byte bounded source diff audit.

| Case | Decoded span | Old → new | Level / memLevel | Frozen mutated-source SHA-256 |
| --- | --- | --- | --- | --- |
| C8 p1/i2 x bit 15 | `[33576,33578)` | 5,978 → 38,746 | 9 / 8 | `0f15353f8ef1d7d7e5badc332f65be860ff05cc23115bfab484073c6a50b672d` |
| C8 p1/i2 y boundary | `[33578,33580)` | 1,479 → 32,768 | 9 / 8 | `73c3c70fcd8edcde47bf5833289f3faf01b7355b1653561c261be38a6fc25a75` |
| HN-A p16/i2 x bit 15 | `[17048,17050)` | 482 → 33,250 | 8 / 7 | `b691aee68b4c5e26a0ace026ab86d7762f096322f2d69252d5d5958725ae6130` |
| HN-A p16/i2 y bit 15 | `[17050,17052)` | 5,446 → 38,214 | 8 / 7 | `8ed08a34876ea5b40959b429f329ebbc787fd8490f1c4f23fbc519b19adcef31` |

C8 keeps row `[80,100)`, text `[220,14766)`, allowed encoded changes only
in `[244,14766)`, first descriptor 14,766 and frame length 14,522.
HN-A keeps row `[16664,16684)`, text `[953320,960821)`, allowed encoded
changes only in `[953344,960821)`, first descriptor 960,821 and frame
length 7,477. Three cases toggle only decoded bit 15; the C8 y boundary
changes one logical two-byte value, with no single-bit claim.

| Case | Unsigned candidate selected translation (pt) | Signed i16 alternative (pt) |
| --- | ---: | ---: |
| C8 x | 3760.226445612616 | -2599.919126566923 |
| C8 y | -2391.672786089770 | 3968.472786089770 |
| HN-A x | 3226.849979781642 | -3133.295592397898 |
| HN-A y | -2886.596845936110 | 3473.548726243429 |

For unsigned interpretation use the frozen formula with the new raw word;
for the signed alternative subtract 65,536 from each new word first. The
predictions differ by `65536*240/2473 = 6360.145572179539 pt`. Accept an
unsigned-role result only when both runs match the unsigned absolute
prediction within 0.00005 pt, disagree with the signed alternative, and
change only that selected translation. Preserve images outside the page;
do not clamp or treat their invisibility as missing source data. Require
all page counts/boxes, image identity/order/dimensions/stream hashes, first
four affine components, other translation, other target draws and every
non-target page's CTMs unchanged. qpdf, MuPDF and Poppler must agree;
metadata rejection, unrelated changes or nonrepeatability is UNSUPPORTED.

Audit all original sources, matrix/oracle, six baseline PDFs/reference report,
clean reference checkout, executable/package hashes, actual libz bytes,
exact command/environment and 180-second timeout before and after. Report
every attempted/completed/passing/failing/skipped/unsupported case and actual
converter-runner calls, retaining both signed/unsigned predictions, all
ordered CTMs and exact diff runs externally. Do not rerun or substitute any
recipe during this batch. Its scope is field interpretation on these two
documents; a pass would support a named empirical raw-u16 evaluator, not
claim authoritative physical units or complete HN/C8 document conversion.

## Acceptance and remaining support boundary

Follow #112's acceptance criteria and release policy: original MIT provenance,
meaningful malformed/range/short-read/cancellation/resource tests, native and
WASM builds, browser/Node regressions, quality/license audits, exact per-file
100% Rust LCOV, independent correctness and simplification reviews.
Report supported variants/ranges and empirical scope precisely. Any failed
rule gate leaves the result PARTIAL/UNKNOWN and #10 blocked. A validated
profile parser and transform helper alone do not prove full-page pixel parity,
other image types, text, outlines or production CLI/JS support.
