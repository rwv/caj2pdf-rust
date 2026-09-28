<!-- SPDX-License-Identifier: MIT -->

# Source-page composition evidence

[Issue #117](https://github.com/rwv/caj2pdf-rust/issues/117) and draft
[PR #121](https://github.com/rwv/caj2pdf-rust/pull/121) remain incomplete.
The experiments below have separate frozen plans, immutable external
receipts and actual counts. No failed experiment is a whole-profile
compatibility pass. Source documents, states, decoded arrays, PDFs, renders
and execution artifacts remain outside Git.

## Preserved first attempt

The [original protocol](hnc8-page-composition-protocol.md) ran at commit
`42125b6b2a98169083868bfa19f2595f12eb311f`. Report: 733,091 bytes, SHA-256
`7e9d0e6d43d1f0f3e428da636f82566f7fcac44e4868347093edd652735816e5`.

It remains **FAIL**. One native HN-A output completed; metadata passed for
68 source/output pages, 91 draws and 24 JPEG streams. The first required
complete Type0 comparison refused the reference dictionary's explicit
identity DecodeParms. Two Poppler extractions ran, but no complete array or
full-page pixel comparison passed. C8 and HN-B were unattempted.

A separately frozen [dictionary-only probe](hnc8-page-composition-dictionary-probe.md)
made two object queries and four startup identity probes, without conversion,
sample extraction or rendering. Its 21,470-byte report SHA-256 is
`ec6a38adda35e156ad5b67857471414a1208953c9aa8c7d3d50e0675ca513fa4`.
It established the narrow explicit Flate Predictor=1 profile. This is
dictionary evidence, not sample/page compatibility evidence.

## Controlled identity-parameter rerun

The independently reviewed [amendment](hnc8-page-composition-identity-params-rerun.md)
was committed before execution at
`8041ee43078e387c027d8ef317984fbf9d48cd04`. The Rust/Cargo fingerprint and
native binary were identical to the first attempt. The correction changed
only the original verifier, synthetic tests and evidence documentation.

- Report: 1,696,715 bytes, SHA-256
  `104cd225c58876e38a1d5a04d58de164d42a049d3a823632cadea4b7ac939291`.
- Outer receipt: 98,227 bytes, SHA-256
  `d6f8359f91cd791432aff8fb14c2845f4856daae6b0c2e79f9e4ac1a06cf1771`.
- Outer post-audit SHA-256:
  `cf4eca239a9e33fe9a624cca3a284eee557f716de63f0585874fbadad8bfa014`.

| Required work | Attempted | Passing | Failing | Remaining |
| --- | ---: | ---: | ---: | ---: |
| Profiles | 3 | 2 | 1 | 0 |
| Source rows | 81 | 81 | 0 | 0 |
| Output boxes | 77 | 77 | 0 | 0 |
| Ordered draws | 127 | 127 | 0 | 0 |
| Encoded JPEG streams | 53 | 53 | 0 | 0 |
| Complete padded Type0 arrays | 74 | 74 | 0 | 0 |
| Full-page/renderer comparisons | 151 | 150 | 1 | 3 |

Unsupported counters are zero. The 75 HN-A/C8 pages passed all 150 required
full-page comparisons with the frozen MuPDF and Poppler settings and exact
RGB equality. Every Type0 row and padding bit passed independent extraction;
all six CTM components and boxes met the frozen 0.00005 pt tolerance.

The overall result remains **FAIL** at HN-B output page 1 / MuPDF. That
2071-by-153 raster differs on 261,584 of 316,863 pixels, with maximum channel
difference 255. The other three HN-B page/renderer pairs were unattempted.
Both HN-B output boxes, transforms, dimensions, encoded streams and mapping
`[1, 6]` agree, with source rows 2–5 explicitly accounted as no-image rows.
The maximum box/CTM residual is 5.68e-14 pt.

The recorded reference image dictionaries declare DeviceRGB; native output
declares DeviceGray. Existing public JPEG inventory identifies the same
payloads as eight-bit, one-component JFIF. The metadata checks above compare
dimensions, streams and placement but exclude this color interpretation
difference. [Child #122](https://github.com/rwv/caj2pdf-rust/issues/122)
blocks the remaining reference-validity and complete HN-B page criterion.
No reference replacement, color inversion, malformed RGB declaration or
pixel-tolerance change follows from the recorded metadata alone.

## HN-B color interpretation investigation

The separately reviewed [dictionary/header plan](hnc8-page-composition-hnb-color-probe.md)
was frozen and committed at `456fc92a051392ec23df38ee633aeedf824d2def`
before its single execution. Its framing-only report is 74,626 bytes,
SHA-256 `ebc1fe08e2121e5322ad2d29e3bfd821070e25580b666e0bd860be7ff401bb49`.
Its immutable receipt is 33,225 bytes,
SHA-256 `f8b9c097524c54874401195d4d0a29c350362cab2258c2e23e4ecd9d83c78e7d`.

All four exact dictionary queries and both independently parsed marker
headers passed. Each source JPEG has an eight-bit, one-component SOF0 and
JFIF header; the observed SOI-through-first-SOS prefixes contain no APP14.
Both reference dictionaries declare DeviceRGB, while both native dictionaries
declare DeviceGray. None contains Decode, DecodeParms, ImageMask, Mask or
SMask. The source/output mapping and encoded identities remain those of the
preserved failed comparison.

These observed declarations are inconsistent with the DCT component count:
[PDF Reference 1.4](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.4.pdf)
§4.8.2 derives image components from ColorSpace, §3.3.7 obtains DCT components
from encoded data, and §4.8.4 identifies inconsistent image/color entries as
errors. DeviceGray is the valid interpretation of these one-component images.
This establishes a retained-reference defect; **legacy page parity remains
FAIL**. It does not establish pixels against an independently corrected
reference.

The probe made eight validator launches: four dictionary queries and four
startup library probes. Native/converter calls, private renders and sample
decodes were zero. Marker observations fetched 1,416 bytes in 60 exact reads,
with a 649-byte maximum request. Separately metered opaque provenance audits
read 86,754,185 bytes, including coded bytes without interpreting them.
Every public/private/library/receipt audit matched before and after. An
independent metadata-only audit confirmed all identities, counts and outcomes.
The probe took 0.370 seconds; all eight children had wait4 RSS measurements,
peaking at 28,176 KiB, with harness high-water mark 37,316 KiB and owned
storage peak 110,372 bytes.

Eleven original external controls passed before this probe. Both MuPDF and
Poppler detect the intentionally incompatible RGB wrapper on the same
original grayscale JPEG; their raw pixels are not assumed equal to each
other. A separate source-only CI regression retains this same-stream color
failure check. The main verifier now counts `jpeg_color_spaces` separately
and requires agreement between reference, native and pinned DeviceGray/RGB
declarations. Missing/other color profiles are unsupported failures. This
narrow identity gate does not normalize arbitrary Decode, DecodeParms or
mask semantics; complete page pixels remain a separate mandatory gate.

## Preserved corrected-reference attempt

The separately reviewed [corrected-reference plan](hnc8-page-composition-hnb-corrected-reference.md)
was frozen at `6a401e2afa29a4458960935907fe6eee74441813` and executed once.
Its report is 182,160 bytes, SHA-256
`fa2fc149139b6833d06747bbf279af1d4a100fae01a9c5f9c1783a72c342d204`;
its immutable receipt is 62,561 bytes, SHA-256
`fb2b01155bd5199c99931fbd82fa1de6d3a986052f58fa911b04cbebacb080b4`.
This attempt remains **FAIL** and does not satisfy all four HN-B page pairs.

The sole qpdf update changed image objects 7/9 from DeviceRGB to DeviceGray.
A complete bijection proved preservation of all nine objects and four raw
streams; only those two color declarations changed. The separately identified
corrected PDF is 826,724 bytes, SHA-256
`2d423e1262142030b9b042a54735edc1776b132ae2fc54a3cbfc4b5f4d6f10fd`.
The original legacy PDF and all earlier failed reports remain unchanged.

All six source rows, two output boxes, two ordered JPEG draws, encoded streams,
color interpretations and mapping `[1,6]` passed; rows `[2,3,4,5]` remained
explicit no-image rows. Two direct source decodes and all four complete
PDF/source sample comparisons passed exact equality. Page 1 compares 316,863
samples; source page 6 compares 7,279,272 samples. The independently run
`djpeg` and `pdfimages` commands share the installed libjpeg backend; these
results do not establish decoder-implementation independence.

The first MuPDF complete-page pair passed: 316,863 pixels, 950,589 channels,
73,186 nonwhite pixels, and zero differences. The following Poppler pair
failed the exact raster-dimension/payload guard; two remaining page pairs
were skipped. Both Poppler outputs contain 956,818 bytes, but their hashes
also differ. Equal lengths alone do not establish pixel equality. The
dimension guard is the first refusal, not proof of the only discrepancy.
Any renderer-sizing or numerical-boundary investigation requires a separately
reviewed amendment; cropping or a pixel tolerance cannot turn this into PASS.

The phase used 69 validators, including four renders and twelve startup
library probes; native/converter launches were zero. Every input/code/tool/
environment/library/generated-input/receipt before/after audit matched.
Two independent metadata-only audits verified these counts and identities.
The phase took 2.916 seconds, child wait4 RSS peaked at 29,928 KiB, harness
high-water mark at 40,260 KiB, and owned storage at 30,806,595 bytes. Exact
JPEG spooling fetched 825,381 bytes in 203 requests, each at most 4,096 bytes.
Twelve original-only controls passed before execution; their 138 public
fixture launches are separate from private compatibility evidence.

## Resources and audits

The controlled identity-parameter rerun made three native calls and zero
Python converter calls. Its
1,838 validator launches include 302 renders. Adding 22 outer probes and
one runner launch gives 1,864 aggregate launches, below the frozen 2,048
ceiling. The two comparison attempts together made four native calls; each
has its own counts, receipt and limits.

All six integrity groups passed before and after: 27 corpus files, six
reference PDFs, caller table, inputs, environment/tools and native code.
The outer unchanged flag is true; its status remains FAIL because the
experiment failed. No watchdog cleanup was needed.

The rerun took 167.8 seconds. Native peak RSS was 24,640 KiB, validator peak
76,272 KiB and harness high-water mark 30,640 KiB. Observed owned-session
storage peaked at 129,696,193 bytes against 512 MiB. These measured process
and storage figures are distinct from checked allocation/buffer limits.

## Independent review and hosted gates

Two independent reviewers checked the frozen verifier amendment and wrapper;
both passed all 30 focused original synthetic tests. The full conformance
suite passed 348 tests. A separate metadata audit confirmed the rerun's
failure, actual counts, hashes and before/after audit statuses without reading
private PDFs, source/header/entropy bytes or decoded samples.

All four hosted jobs passed on `8041ee43078e387c027d8ef317984fbf9d48cd04`
in [CI run 36372656098](https://github.com/rwv/caj2pdf-rust/actions/runs/36372656098):
native quality, WASM/browser/Node, MIT audit and Rust LCOV. The LCOV gate
recorded 27,410/27,410 lines, with 100% in every recorded file. Native examples
are compiled and checked but are outside the standard LCOV report. Later
changes require new exact-head gates and final independent review.

Clean-clone CI reports the optional document comparison as **NOT_RUN** with
zero actual work and zero compatibility passes. Parent #10, table-rights
gates and production CLI/JavaScript family routing remain open.
