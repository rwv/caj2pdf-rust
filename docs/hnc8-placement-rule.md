<!-- SPDX-License-Identifier: MIT -->

# Source-derived HN-A/C8 placement profile

Current reader update: the former document-prefix digest check has been replaced
by structural compressed-header validation; see
[the header note](hnc8-compressed-text-header.md). The experiments below retain
their historical scope. No placement factor changed.


This records the frozen plan and measured results for [#112](https://github.com/rwv/caj2pdf-rust/issues/112),
following the [#111 text-source investigation](hnc8-text-source.md). The
candidate below was frozen before any new private conversion. Its discovery
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
evidence gate in the original plan. The results below establish unsigned
roles on the four intervened targets. Other text/record fields remain opaque
and are discarded.

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

## Numerical evaluation follow-up for #117

The current evaluator computes each pixel dimension as `pixels * 72 / 300`,
the exact nominal ratio `6/25` for the measured 0.24-point model. Every `u32`
dimension's numerator is an exactly representable integer below `2^53`;
only the division rounds. Multiplication by the already rounded binary
constant `0.24` can instead change the shortest serialized PDF number by
one ULP. Original three-by-two Gray controls show that this changes complete
Poppler edge pixels at an integral device boundary, even when box/CTM values
are well inside the metadata comparison tolerance. The pixel criterion
continues to require exact equality. Source DPI remains unproven.

The general affine PDF writer keeps its shortest round-trip number contract.
Original controls also establish that the frozen 300-DPI renderers can
produce different complete canvas dimensions for the same nominal integral
grid. The diagnostic reads bounded P6 headers and accepts only the observed
`N` or `N+1` boundary on each axis, requiring equal complete grids within
each renderer. It compares every actual channel, including the extra edge,
with exact payload/file lengths and zero pixel tolerance. This rule applies
to the documented zero-origin, integral-grid profile with both page extents
at least one point; other raster profiles are explicit unsupported failures.

The historical placement and comparison evidence above retains its original
identities. A new frozen full HN-A/C8/HN-B batch is required before accepting
the dimension revision's complete-page results.

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

## Retained high-bit outcomes

The frozen plan was committed as `b801897`; the original MIT harness and
13 synthetic tests were committed as `dbdf860`. One private batch ran on
2026-09-28 UTC, starting at `f1ec081`, with exactly four source copies and
eight converter-runner calls. No retry or recipe substitution occurred.
The metadata-only report is retained outside Git as
`/tmp/issue112-highbit-report.json`, 449,407 bytes, SHA-256
`201e5ee16b8435777b8d2b14808747b876d9f58b4b11a3f7498476586e927b94`.
The separate execution receipt preserves before/after hashes of the exact
harness, synthetic tests and preexecution plan; all three were unchanged.

| Case | Observed selected translation (pt) | Unsigned residual (pt) | Repeated PDF SHA-256 |
| --- | ---: | ---: | --- |
| C8 x bit 15 | 3760.2264 | -0.000045612616 | `90325a319637ebe8485efe308c156cf74c1a2a6cc0225de9fc66805368780445` |
| C8 y boundary | -2391.6728 | -0.000013910230 | `04214a3e99d69e46342ec35c52d860326797451f284980300030dc446202c60c` |
| HN-A x bit 15 | 3226.8500 | 0.000020218358 | `ac851b95574b5a2cfeb530e5c9dc8b9cacf2e71f96c500d21e7c98f2f79be9ac` |
| HN-A y bit 15 | -2886.5968 | 0.000045936110 | `1645ba1d6d0a3b4e61705f0ab22e1b4ef338f4521ff40fde114d1c590f75b29a` |

All four cases passed, repeated, and classified
`UNSIGNED_COORDINATE_FIELD_EFFECT`; zero failed, skipped, unsupported or
conversion-failed cases. Each pair produced an identical PDF. The maximum
unsigned residual is 0.000045936110382172046 pt; the smallest signed-i16
error is 6360.145526243428 pt. Every other CTM component, page box, image
identity/order/dimension/stream and original row/outer header stayed fixed.
Strict frame/decoded-byte and every-byte source-diff guards passed. All 27
original sources, nine matrix/oracle/report/PDF inputs, reference environment
and actual loaded libz/Python hashes passed before/after audits.

The harness used at most 65,536-byte ranged/copy/runtime-hash requests and
1,048,576-byte full-source audit requests. Its text/plain buffers were capped
at 65,536 bytes each. VmHWM was 27,312 KiB; maximum converter child VmHWM
was 39,856 KiB. PDF tools used at most 322,609 output bytes and 42,672 KiB
child RSS. The external artifact session retained 60,753,858 non-symlink
bytes, matching its reported peak session size; external library symlinks
are excluded from that byte count. Timeout was 180 seconds per converter;
none timed out. This diagnostic deliberately retains source copies/PDFs
outside Git for review rather than claiming zero experiment disk use.

An independent read-only audit rehashed every retained source copy and PDF,
reproduced all four predeclared one-shot frames, checked decoded changed
positions and recomputed ordered-geometry equality. It launched no converter.
The report still says `UNKNOWN_UNSIGNED_COORDINATES_TARGETS_ONLY`: these
interventions establish interpretation on the two measured documents.

## Native source-derived verification

The original MIT streaming parser is `hnc8::read_text_coordinates` with
`TextBudget`; pure geometry is exposed through
`hnc8::empirical_page_from_pixels`, `hnc8::empirical_page_from_type0` and
`hnc8::empirical_image_transform`. The metadata example
`hnc8_text_placement` receives only an original source path. It does not
receive reference CTMs or perform placement lookup by document/page/hash.
The native source candidate, including the final unsigned policy, was frozen
at `b9bffe1`. Its release build used Rust/Cargo 1.98.1:

```sh
CARGO_TARGET_DIR=/path/to/build cargo build --locked --release \
  -p caj2pdf-core --example hnc8_text_placement
sha256sum /path/to/build/release/examples/hnc8_text_placement
PYTHONPATH=scripts python3 -c \
  'import hnc8_placement_rule as r; print(r.source_fingerprint()["sha256"])'
```

The measured binary was 666,232 bytes, SHA-256
`9dc3f5b11a2bac60193a5ffc3d69b569050b8c4c703703d719d036ecb8884165`.
The bounded, sorted manifest contains all Rust files, every crate/root Cargo
manifest, Cargo.lock and rust-toolchain.toml: 133 files, 3,231,885 bytes,
aggregate SHA-256
`ac9311a06ef2f8695132441e93383e3460021daade8900a88ee18e4eda7a60ed`.
The aggregate hashes UTF-8 relative path, NUL and raw file SHA-256 in path
order. The separate external execution receipt records exact build/runtime
commands, compiler versions, per-file hashes and verifier/test hashes.

The optional `scripts/hnc8_placement_rule.py` runner requires every pinned
external path plus explicit binary and source SHA-256 pins. It compares
independently framed/hash-checked raw source words, image headers/streams
and source order, then all six native transform components with the reference
PDF metadata. qpdf, MuPDF and Poppler reproduce those reference observations.
Its native-only command is `[native_tool, source_path]`. Clean clones/CI are
`NOT_RUN` with every measured-work counter zero. Partial/missing/changed
explicit inputs fail; unavailable corpus tests are never passes.

```sh
python3 scripts/hnc8_placement_rule.py \
  --corpus-dir /external/CAJSamples \
  --reference-repo /external/caj2pdf-reference \
  --python-bin /pinned/python3 --pydeps-dir /external/reference-pydeps \
  --jbig-lib /external/libjbigdec.so \
  --reference-report /external/hnc8-layout-reference-report.json \
  --native-tool /path/to/build/release/examples/hnc8_text_placement \
  --native-sha256 BINARY_SHA256 --native-source-sha256 SOURCE_SHA256 \
  --git /pinned/git --qpdf /pinned/qpdf --mutool /pinned/mutool \
  --pdfinfo /pinned/pdfinfo --pdfimages /pinned/pdfimages --json
```

The placeholders must identify the versions/hashes in the pinned reference
protocol; unrelated local tools or reports fail the explicit audit.

The once-only 2026-09-28 UTC report is retained externally as
`/tmp/issue112-placement-rule-report.json`, 115,511 bytes, SHA-256
`6d892bc3d41f23f71d503c689411d2bc2f9f30ecd0fb8bcd61b1c95ec5a10048`:

| Comparison | Attempted / passing | Failing / skipped |
| --- | ---: | ---: |
| HN-A/C8 profiles | 2 / 2 | 0 / 0 |
| Strict source frames | 75 / 75 | 0 / 0 |
| Source image identities/order/dimensions/raw words | 125 / 125 | 0 / 0 |
| Source-derived MediaBoxes | 75 / 75 | 0 / 0 |
| First-image all-six transforms | 75 / 75 | 0 / 0 |
| Discovery supplemental all-six transforms | 36 / 36 | 0 / 0 |
| Validation supplemental all-six transforms | 14 / 14 | 0 / 0 |

HN-A has 68 pages/91 draws; C8 has seven pages/34 draws. All 63,977 marker
records and frame hashes match independent source checks. Maximum all-six
CTM residual is 0.00004913061099642846 pt; MediaBox residual is zero.
There are exactly two native launches/completed processes and zero reference
converters. HN-B's six source rows are separately unsupported/not executed,
with no compatibility passes. All 27 source identities, six baseline PDFs,
nine inputs, reference environment, native binary/manifest and effective
native environment passed before/after audits.

| Resource | Native metadata diagnostic | Independent Python verification |
| --- | ---: | ---: |
| Maximum source request | 4,096 bytes | 14,546 bytes for frames; 1 MiB full-source audit |
| Maximum parser-owned buffers | 8,212 bytes | Opaque text checked through a spool |
| Accounted native parser working memory | 143,380 bytes | Separate from native accounting |
| Native child peak RSS / harness VmHWM | 17,528 KiB | 24,960 KiB |
| Maximum/total native stdout | 34,669 / 43,509 bytes | 1 MiB per subprocess cap |
| Native temporary disk | 0 bytes | Peak decoded spool 33,688 bytes |
| Cumulative/retained decoded spool | 0 / 0 bytes | 1,028,032 / 0 bytes |

Native source reads total 13,346,960 bytes. These include bounded metadata
and payload-hash passes rather than a claim of one read per file byte.
Native calls have a 30-second timeout and 1 GiB virtual-address ceiling.
PDF-tool output peaks at 322,609 bytes; child RSS at 42,540 KiB. RSS measures
the whole child, while parser-owned/accounted storage measures only that
module; successful-process RSS/output accounting does not invent values
for failed processes. The runner retains only metadata and closed-spool
counts. It reports `EMPIRICAL_PROFILE_VALIDATED_SAME_DOCUMENTS`.

## Accepted API domain and remaining uncertainty

The parser's defaults cap compressed and decoded text independently at
1 MiB, repeated records at 65,536, images at 8,192 and accounted working
storage at 1 MiB. Both scratch chunks also respect `Limits::io_chunk_bytes`
and a 64 KiB ceiling. Retained coordinates require four bytes per image;
buffer capacities, a conservative 128 KiB decoder reservation and 4 KiB
fixed scratch are counted before returning a result. The locked native
backend's InflateState size was independently measured as 43,296 bytes;
the reservation must be reviewed when its dependency graph changes.
The backend's fixed allocation is infallible; these local budgets do not
make process-wide allocator exhaustion recoverable or bound compiler stack
and allocator overhead. Native decoding itself needs no temporary spool.

The caller must supply header/page values from the same stable ranged
source. The text reader rechecks counts, row arithmetic, source limits and
index-versus-page-text nonoverlap; it does not authenticate the container
index or every other page/image interval itself. The existing Hnc8Reader
supplies that container metadata. Forward-only input remains a platform
adapter's bounded seekable-spool responsibility, covered by the unchanged
native spool limit/interruption/cleanup tests.

Pure evaluation accepts raw words 0..65,535 without signed reinterpretation
or clipping, positive u32 pixel dimensions, and caller-supplied finite PDF
origins. Type-0 public stride/visible-byte consistency and padded-width u32
overflow are checked; zero dimensions, nonfinite/collapsed boxes and lost
page/selected-coordinate precision return typed errors. Combined y rounding
is checked so individually acceptable page/offset errors cannot accumulate
beyond 0.00005 pt. Full-precision f64 output preserves negative/off-page
PDF positions and source order/repeats.

This is the evaluator's mathematical domain. Original observed x/y ranges
were HN-A 0..952 / 0..6,497 and C8 0..6,007 / 0..7,833; the four mutations
add specifically measured 33,250/38,214/38,746/32,768 values. Endpoint
synthetics at 65,535 test arithmetic, not vendor document compatibility.
Negative caller origins are synthetic inputs; negative PDF y from the
positive high-bit words is observed. No negative signed source field or
authoritative physical source unit is established. The empirical factors,
matching prefix/marker layout and two inspected documents define this
profile; arbitrary layouts, HN-B text and other image types remain outside
the new verification scope. Production page composition, pixel parity,
outlines and CLI/browser/Node exposure still require #10's own gates.
