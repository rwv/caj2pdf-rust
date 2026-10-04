<!-- SPDX-License-Identifier: MIT -->

# HN-A/C8 text framing and placement controls

This is the predeclared diagnostic plan for [issue #111](https://github.com/rwv/caj2pdf-rust/issues/111).
The source format observations below were made by bounded, read-only analysis
of two #107 reference documents, with all 27 SHA-pinned HN/C8 sources in the
[matrix](../../tests/conformance/matrix.json) audited before and after.
They are empirical invariants of that corpus, not a published HN/C8
specification or proof that the same layout holds for arbitrary documents.
No external converter source was inspected or copied. Private documents,
decoded text, modified sources and PDFs remain outside this repository.

Later [controlled geometry checks](cajviewer-hnc8-kdh.md#controlled-geometry-checks)
establish separate page/display extents for the observed HN-A/C8 profiles.
#184 reads those fields; the discovery notes below retain their historical scope.

## Read-only framing discovery

The old [#110 note](hnc8-placement-experiments.md) tested zlib starts at text
offsets 0, 20, 26, 28 and 32; it did not test `+24`. All 75 HN-A/C8 pages in
the #107 oracle have one complete [RFC 1950](https://www.rfc-editor.org/rfc/rfc1950.html)
zlib stream at text-relative `+24`,
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
were 14,546/14,522/33,688 bytes. The final `4+28*I` bytes may contain
image-related data, but no
field semantics are established by size or adjacency alone. The bounded
diagnostic validates framing, lengths, marker positions and resource ceilings;
it does not construct a PDF or expose a production text API.

The former proposed whole-donor overwrite at the beginning of the longer
target text span is invalid. Its zlib end would leave a suffix inside the
row-declared text span. It must not be submitted to the converter or counted
as a negative placement test. Virtual validation leaves 3,856 unused bytes
for C8 and 2,157 for HN-A; both candidates are `INVALID_FRAMING` with zero
mutations/converter runs. The external metadata-only validity report has
SHA-256 `c83cb86c5e336fb12ba610041eb1227f3aa3bc81ba414f0f963b0dfdf901966a`.
The #110 full-span donor transplant changed
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

## Protocol preflight correction

The initial optional harness request passed every before/after source,
environment and baseline-input audit, then failed while obtaining a page's
image count. The compact #107 oracle intentionally has an `images` list,
not the independent source extractor's extra `image_count` key. The harness
now uses the validated list length; its synthetic repeated-probe test uses
the same compact oracle schema. This failed request created **zero source
copies and zero converter runs**. Its external report SHA-256 is
`81cb76191f22492d85711fab0f5310c178e4c23c59c2ba3f4a04f9f72cf37e39`:
protocol attempts 1, completed 0, failing 1, skipped 1, private conversions 0.
Retain this failure separately from later successful comparisons. Retry the
unchanged two-case plan above, still bounded at two copies/four conversions;
no additional mutation or compression recipe is permitted.

## Fixed-row content batch result

The unchanged two-case plan ran at diagnostic revision `922e13b` after that
preflight correction. Both fixed-row source copies passed strict text-frame,
marker, container, image-identity and every-byte diff checks. Each was
accepted twice by the pinned converter; all four PDFs were repeatable and
qpdf, MuPDF and Poppler agreed. Their PDF hashes equal the earlier #110
compound transplants even though the original index rows stay unchanged.
All five target supplemental x/y translations exactly equal the donor
translations, while target scales, first-image CTMs, image bytes and all
73 non-target pages' geometry stay fixed. Thus the decoded text-content
component controls these placements independently of index-row address or
length on these two targets.

| Case | Mutated PDF SHA-256 | Changed source bytes / runs | Donor translation matches |
| --- | --- | --- | ---: |
| C8 fixed-row content | `81d684dd092727fb794426145e5bac5b48caf8985b8957bee991cea153c0f170` | 14,457 / 68 | 4/4 |
| HN-A fixed-row content | `9e4f111c9ef5c07698240aa5d33de7a1fb665e8758e23312344143c1afd98aae` | 7,442 / 38 | 1/1 |

The mutated source hashes equal the frozen plan above. Successful-batch
counts are planned/attempted/completed/passing/repeatable 2, failing/skipped/
unsupported 0, returned converter runs 4 and text-content effects 2. The
separate failed protocol preflight remains recorded above and is not a
compatibility match. All 27 sources, six baseline PDFs, matrix, oracle,
reference report and full pinned environment passed before/after audit.
The external report has SHA-256
`a8f580ec8ea1c5da4b130963febd60ab194f9d4075defbb2ecf024553d1104d3`;
it retains every ordered CTM/image identity and exact source diff run.
Maximum source range/copy request was 65,536 bytes, hash requests 1 MiB,
harness VmHWM 25,784 KiB, converter child VmHWM 40,904 KiB, PDF-tool RSS
42,680 KiB, tool output 322,609 bytes and temporary-session size
34,481,126 bytes (30,376,912 retained). Timeout was 180 seconds; none
occurred. The placement rule remained `UNKNOWN_TEXT_CONTENT_ONLY` at this
stage; that component result alone identifies no individual field.

## Bounded frame diagnostic result

The original MIT [frame diagnostic](../../scripts/hnc8_text_frame.py) validated
all 75 HN-A/C8 source text spans in the two reference documents under its
1 MiB span/output limits and 64 KiB read/output chunks. It checks prefix
and marker fingerprints, complete zlib EOF/Adler/no-tail, exact declared
length and the `8+16*N+4+28*I` layout. It returns constant-size section
metadata and hashes, with a scoped validated disk-spool callback when
needed; it returns no decoded document bytes by default. The standalone
CLI requires a full input SHA-256, verifies it before/after in bounded
chunks and caps the source at 1 GiB. `VALIDATED` means frame-profile
validation only; converter compatibility remains `NOT_RUN`.

The optional read-only run at parser revision `80e08d3` reports 75 attempted,
completed and structurally passing frames, 0 failing/skipped, 27-source
before/after matches, and 0 converter launches. Six HN-B rows were not
attempted under this different profile. Its metadata-only external report
SHA-256 is `fec27e7a926ca5625a379cc1d4a7ed44c56181dadd09c173b0f48b5bbdb145d5`.
Measured maximum parser source request was 14,522 bytes, decoder output
chunk/logical spool 33,688 bytes, allocated spool blocks 36,864 bytes, and
harness VmHWM 21,948 KiB. Source hashing used at most 1 MiB per request.
Each spool was closed before the next page; no decoded bytes were retained.

## Read-only coordinate candidate

Independent bounded reads found a stronger candidate in the decoded tail.
For one-based source image number `i`, its 28-byte record starts at
`base = decoded_length - 28*image_count + 28*(i-1)`. The little-endian
unsigned 16-bit values at `base+0` and `base+2` correlate with PDF x/y:

```text
x_pdf = x_u16 * 240 / 2473
y_pdf = MediaBox.height - y_u16 * 240 / 2473
```

The factor is a **retrospectively selected empirical candidate**. A fit
using the 36 discovery draws gives a nonempty four-decimal rounding interval;
`240/2473` is the simplest fraction with denominator at most 10,000 in that
interval. All reference transforms, including the 14 same-document
validation draws, were already public and inspected. All 100 supplemental
x/y components agree within 0.00005 pt, with maximum error
0.000049130611 pt; all 150 first-image x/y components also agree.
No counterexample appears in these two documents, but these observations
provide no independently established physical source unit or general rule.
The remaining 24 bytes of each image record stay opaque; `+4/+6` correlate
with dimensions and must not be used as decoded image pixel sizes.
An additional read-only check compared those two slots with pixel width/height
times `2473/1000`, rounded down, nearest or up. Across 250 dimension components,
the three candidates matched only 75, 84 and 91, respectively; the largest
unrounded error was 3.8 source units. This supplies no exact size interpretation
or independent physical-unit definition. All 27 source hashes were unchanged;
no mutation or converter run was made for that check.

The external metadata-only read-only report has SHA-256
`b27ff8d4b5b3dacb60e7de5aadc4bfaedfc521b9f3c8e3d0ffa5474b553c391b`.
It records the 75 complete frames, 63,977 marker sets, 125 draw predictions,
all per-component residuals, the split, field offsets and 27-source
before/after audits. It ran zero black-box conversions, wrote no private-byte
artifacts and skipped the six HN-B rows as outside this observed framing.
Maximum source-range request was 14,546 bytes, inflated output 33,688 bytes
under a 1 MiB ceiling, and observed harness VmHWM 22,584 KiB. Hash requests
were bounded at 1 MiB. This report is evidence for a future predeclared
field intervention; it does not enable composition.

## Second black-box batch: individual fields and wrapper controls

The successful fixed-row content experiment justifies a narrower batch.
Predeclare exactly **six** source copies, each converted twice (12 conversions
maximum), with no source or field search during execution. Four copies change
one decoded two-byte candidate value by `+100`; two change only a recognized
zlib wrapper field while keeping every decoded byte identical. Use the same
two discovery target pages, image number 2, fixed original rows/spans and
before/after audits as the first batch. No new document or unannounced
validation subset is introduced.

| Field case | Decoded two-byte span | Old → new value | One-shot level / memLevel | Expected mutated source SHA-256 |
| --- | --- | --- | --- | --- |
| C8 p1/i2 x | `[33576,33578)` | 5,978 → 6,078 | 9 / 8 | `4121247ecc7b4d3d3b86329f1d8504fffcc7f5768f2f3d67ce8dd3d268795a6a` |
| C8 p1/i2 y | `[33578,33580)` | 1,479 → 1,579 | 9 / 8 | `73ff3275dcbbe74c39e278e12ed43280f8570f6c473340425d4fa129dfcf3228` |
| HN-A p16/i2 x | `[17048,17050)` | 482 → 582 | 8 / 7 | `779ea5d1b13c147776171e23f61a9f925df8ea520424c03a17f3d09e15f38336` |
| HN-A p16/i2 y | `[17050,17052)` | 5,446 → 5,546 | 8 / 7 | `5dc6763070234b7b4a1d854949c14e4e5ee08d689f35e0ddbb40f2e63a628ed2` |

These offsets are **decoded offsets**, not direct source offsets. Require
each to equal the independently validated trailing-record base plus `+0`
or `+2`, with the original value exactly as shown. Every other decoded byte
must remain identical. Preserve the outer 24 text bytes, original source
length and all bytes outside `[244,14766)` for C8 or `[953344,960821)` for
HN-A. Recompress with zlib 1.3.1, `Z_DEFAULT_STRATEGY`, windowBits 15 and
one final `Z_FINISH`; no intermediate flush, extra frame, padding or tail.
Require the original 14,522/7,477-byte capacities and frozen full-source
hashes. The external read-only one-shot feasibility report SHA-256 is
`42a79d17e7af3a5233e40d69458c43531e5537ecefe2d23046be506a4c2d158c`;
its virtual checks ran zero converters and created no source copy.

For both wrapper controls, preserve CMF `0x78` and change FLG `0xDA` to
`0x01`: FLEVEL 3 becomes 0 and FCHECK is recomputed so the two-byte header is
divisible by 31. FDICT stays zero. This is the informational compression
level described by RFC 1950, not a changed DEFLATE coding or decoded field.

| Wrapper case | Sole changed absolute source byte | Expected mutated source SHA-256 |
| --- | ---: | --- |
| C8 p1 FLEVEL | 245 (`0xDA` → `0x01`) | `736bb4100fa4b6dd20110370ae14a3c0f7e4e738441ea9929330043b5202a974` |
| HN-A p16 FLEVEL | 953,345 (`0xDA` → `0x01`) | `2c017e210bd5f9e6845cc2242270c6674ad5cfdbf01bb40d9c66d4f9da275541` |

Require wrapper controls' DEFLATE payload, Adler-32, complete decoded hash,
row/span and every other source byte to be identical. Predict unchanged
ordered PDF image identity and all six CTM components for every draw. This
tests wrapper information only. A read-only bounded search found no altered,
exact-length representation among 450 one-shot recompression recipes per
variant; do not broaden that search or call these controls general
DEFLATE-representation invariance.

For x/y cases, freeze the empirical absolute prediction above with new
field value and unchanged baseline MediaBox. Expect only selected-image x
to increase or y to decrease by `100*240/2473 = 9.704811969268096 pt`.
Absolute predicted positions must match within 0.00005 pt (the reference's
four-decimal output precision). The selected other translation and first
four affine components, every other target draw, all non-target pages,
MediaBoxes, image dimensions/order/raw-stream hashes must be exactly
unchanged. Keep all ordered geometry and identities in each run's external
report, alongside full source diff runs and two repeat outcomes. A movement
in any other field/draw or an invalid/nonrepeatable conversion is
`UNSUPPORTED`. A wrapper-only movement invalidates the intended narrow
interpretation and blocks a field claim.

Passing predicted one-variable movements under these guards identifies
the two decoded slots' x/y role for these positive-valued profiles. It does
not establish all remaining record semantics or a universal unit/range.
Observed x/y values span 0–952/0–6,497 for HN-A and 0–6,007/0–7,833 for C8.
All also fit positive signed 16-bit integers: signedness when bit 15 is set,
negative source coordinates and physical units remain untested. Keep the
complete placement rule `PARTIAL` and #112 blocking production composition
until its range, parsing and validation gates pass.

## Individual-field batch result

The six-copy plan ran once at diagnostic revision `2e0fb46`, after plan
commit `031766a` and independent pre-execution review. Every source hash
matched the frozen plan. All six copies passed strict frame, marker, logical
decoded-byte and full-source diff checks; each converted twice with identical
PDF hashes. qpdf, MuPDF and Poppler agreed on the outputs. All four field
cases changed only the selected image's selected translation, while every
other CTM component, draw, MediaBox, image dimension/order/raw-stream hash,
original index row and source byte outside the allowed span stayed fixed.

| Case | Selected PDF translation before → after (pt) | Changed source bytes / runs | Repeated PDF SHA-256 |
| --- | --- | --- | --- |
| C8 p1/i2 x | 580.1537 → 589.8585 | 14,010 / 92 | `3c4857b991b3d643a1de5508c60f209454527bb11d614d5586c872e8b3884646` |
| C8 p1/i2 y | 644.8658 → 635.1610 | 5 / 3 | `071bf232060066f8c379f2ac3909a24c4a9dee60d853e46f25015bf78e944d15` |
| HN-A p16/i2 x | 46.7772 → 56.4820 | 18 / 2 | `6c1b5a5e8fce275a9423c233b09e7ce19b8e994e8f89012e8302ab2f7f2602b1` |
| HN-A p16/i2 y | 293.4759 → 283.7711 | 6,875 / 37 | `7eba465c50497ea219916238f7c5d345eba23258c6b8fcbcefeac6db8ce3feb4` |
| C8 p1 FLEVEL | All geometry unchanged | 1 / 1 | `acbd822358c08713a795f8c0ad82c6f23c4b43d9e137d515c3210114ca1bc885` |
| HN-A p16 FLEVEL | All geometry unchanged | 1 / 1 | `833f0c40881f0baf7f76f3505b6679e2dfa0b16c0e469dfbf10f791e687dec40` |

All four absolute before/after predictions agree within 0.00005 pt. The
largest absolute residual among these eight positions is 0.000040477153 pt.
Each field source differs at only one logical two-byte slot after decoding;
recompression explains the larger encoded source diffs. Each wrapper source
differs at exactly the predeclared single byte, with identical decoded bytes,
DEFLATE payload and Adler-32. Both wrapper PDF hashes equal their original
baselines. This is a negative control for FLEVEL/FCHECK information only.

The external report has SHA-256
`27a7ec1244c24b40ae6ad0f08e99b2c4b2eeadc3dd35370ec384cdd410372fd5`.
It retains every ordered six-component CTM and image identity for every run,
exact source diff runs, predicted positions and residuals. Counts are
planned/attempted/completed/passing/repeatable 6, failing/skipped/unsupported
0, returned runs and converter-runner launches 12, isolated coordinate-field
effects 4 and unchanged wrapper controls 2. All 27 sources, six baseline PDFs,
matrix, oracle, reference report, clean reference revision and pinned
environment passed before/after audits. None of the 180-second converter
timeouts expired. Maximum source range/copy request was 65,536 bytes, hash
request 1 MiB, harness VmHWM 28,056 KiB, converter child VmHWM 41,100 KiB,
PDF-tool RSS 42,732 KiB, captured tool output 322,609 bytes, and observed
temporary-session size 91,130,766 bytes (all retained externally). These
resource observations describe the optional diagnostic and external tools,
not a Rust runtime memory bound.

## Final finding and remaining gate

Overall result: **PARTIAL**. Complete zlib framing and decoded-length/layout
invariants are **IDENTIFIED for the 75 HN-A/C8 pages in two documents**. The
fixed-row content controls separate their placement effects from row address
and length. The second batch **identifies the positive-valued x/y roles** of
the trailing-record slots on both targets by repeated one-variable movements.
Across the two successful batches, eight copies completed with 16 converter
launches; none failed, skipped or was unsupported. The earlier zero-converter
protocol preflight failure remains separately recorded. HN-B was not tested
under this text profile; optional clean-clone runs remain `NOT_RUN`.

The full source-derived placement rule remains **UNKNOWN** outside this
observed profile. All reference draws and the fitted scale were inspected
retrospectively, and the new interventions use the same two documents. They
do not establish bit-15 signedness, negative-coordinate behavior, source
physical units, valid ranges, or unseen document layouts. [#112](https://github.com/rwv/caj2pdf-rust/issues/112)
must resolve or explicitly bound those questions, add a bounded original MIT
native parser and source-derived transforms, and pass its frozen validation
and platform gates before #10 can enable supplemental-image composition.
This issue adds original MIT diagnostics and synthetic tests only; production
conversion behavior remains unchanged.
