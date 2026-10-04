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

## Final rational-dimension full rerun

The independently reviewed [full-rerun plan](hnc8-page-composition-rational-rerun.md)
was frozen and committed at `0bdd7f25e274d0a569f8cd3faf403b2bad93d9da`
before one direct, no-retry invocation. Its SHA-256 is
`47e6e0e1e632f80733d4e999d22e6160a6f5d4d40db4ab98cc084690bb6f39c0`.
The original external adapter is 57,754 bytes, SHA-256
`117f7f3d0139b7cc63ea256896c77c12116e8bb63d6ab737424150c97a642cb9`.
Its ten original controls passed separately for the author, root and independent
reviewer; each self-test invocation made 85 original child calls, zero private
native/converter calls, and explicitly mocked the three-profile events. Those
public fixtures are separate from the following private execution counts.

The new native binary is 940,680 bytes, SHA-256
`e76f557009fea368714fc5866b996dc13b660df52a44598c640d89e0a844cb90`;
its 139-file Rust/Cargo fingerprint is
`aa85136b67cc450957d610476c97aab76eb86b6c9a4730f7e261cdbbc5a9cb53`.
Checked pixel dimensions use `f64(pixel_count) * 72.0 / 300.0`, rounding the
rational dimension once instead of multiplying a rounded binary factor.
The empirical placement model and caller-defined affine PDF API remain those
of the reviewed source implementation. All three profiles were rerun against
this new binary; previous passes do not substitute for this execution.

The result is **PASS in the explicitly corrected-reference scope**. The
measured comparison basis is the pinned Python-converter PDF set. HN-A/C8
use its unchanged legacy references. HN-B uses only the separately pinned
826,724-byte Gray reference above. The new preflight again proved all nine
objects and four complete raw streams preserved, allowing only original image
objects 7/9 to change DeviceRGB to DeviceGray. All six legacy repeat PDFs
remain separate unchanged audit inputs. The public oracle is unchanged; only
the two declared in-memory comparison color facts use the corrected basis.

| Required work | Attempted | Passing | Failing / skipped / unsupported |
| --- | ---: | ---: | ---: |
| Fresh native profiles | 3 | 3 | 0 / 0 / 0 |
| Source rows | 81 | 81 | 0 / 0 / 0 |
| Output pages/boxes | 77 | 77 | 0 / 0 / 0 |
| Ordered image draws | 127 | 127 | 0 / 0 / 0 |
| Encoded JPEG streams | 53 | 53 | 0 / 0 / 0 |
| JPEG ColorSpace checks | 53 | 53 | 0 / 0 / 0 |
| Complete padded Type0 arrays | 74 | 74 | 0 / 0 / 0 |
| Complete page/renderer pairs | 154 | 154 | 0 / 0 / 0 |
| Preserved objects / raw streams | 9 / 4 | 9 / 4 | 0 / 0 / 0 |
| Direct HN-B source decodes | 2 | 2 | 0 / 0 / 0 |
| Complete HN-B Gray sample pairs | 4 | 4 | 0 / 0 / 0 |
| Located legacy rejection control | 1 | 1 | 0 / 0 / 0 |

HN-A covers all 68 pages, 91 draws and 136 complete renderer pairs; C8 covers
all seven pages, 34 draws and 14 pairs. HN-B accounts for all six source rows,
two pages/two JPEG draws and four pairs, with mapping `[1,6]` and explicit
no-image rows `[2,3,4,5]`. Every Type0 padding bit and row participates.
The maximum box residual is zero; the maximum residual over all six CTM
components is 0.0000491306105914191 pt, within the unchanged 0.00005 pt gate.

Every full page has identical same-renderer dimensions, exact payload length,
whole-file size/SHA, payload SHA and every RGB channel. All changed-pixel,
changed-channel, absolute/mean difference and maximum difference metrics are
zero. The 154 pairs compare 1,207,408,247 pixels / 3,622,224,741 channels;
none of the observed pages is all white. The complete nominal N or N+1 canvas
is checked, including any extra edge. No cropping or tolerance was introduced.
The four HN-B grids demonstrate the declared renderer-specific admission:

| HN-B output/source page | Renderer | Complete compared grid | Nonwhite pixels |
| --- | --- | --- | ---: |
| 1 / 1 | MuPDF | 2071 × 153 | 73186 |
| 1 / 1 | Poppler | 2071 × 154 | 102133 |
| 2 / 6 | MuPDF | 2222 × 3276 | 1794221 |
| 2 / 6 | Poppler | 2223 × 3276 | 2451475 |

Both new native HN-B image dictionaries require the observed Gray8 DCT profile
without Decode/DecodeParms/masks and retain the two source payload identities.
Two direct source decodes and four full PDF/source comparisons pass, covering
316,863 samples for source page 1 and 7,279,272 for source page 6 per pair.
Their source arrays are variable and nonwhite; extracted RGB expansion is
accepted only after all three channels are exactly equal. `djpeg` and
`pdfimages` share the recorded installed libjpeg backend, so these are
independently run checks rather than decoder-implementation independence.
The untouched legacy RGB basis is rejected specifically at page 1/image 1
`jpeg_color_spaces`; unrelated geometry or unsupported failures cannot satisfy
that control. **Legacy HN-B page parity and every historical FAIL remain FAIL.**

### Immutable execution records

The external session is
`/home/hzc/.cache/caj2pdf-issue117-rational-validation/rational-full-vhuua60h`.
Only safe metadata is summarized here; the files and all document, state,
sample, PDF and raster artifacts remain outside Git.

| Record in that session | Bytes | SHA-256 |
| --- | ---: | --- |
| `probe-report.json` | 909540 | `f9a399c0dd10d5cdeb758e1e18dda55e9019e68ad9795f11cfdbc05e576d7d67` |
| `execution-receipt.json` | 77088 | `2f30772a6b6b6deb40a30de04a558b85a15598c843067eb0d8190324f9ed99ae` |
| `main-composition-report.json` | 1853509 | `b29c01f24e7749b9892b90cd631cfc3f85d40ba369af012d43c4302fb8dea48b` |
| `main/hnc8-composition-hr6top6y/execution-receipt.json` | 28055 | `88611680e5c185f497fadf2a6f8db7ebe6068992465e9a5ec11611207891db8e` |

All four identities were verified from bounded JSON metadata reads. The outer
receipt binds code, tools, new native/source fingerprints, effective child
environment and canonical startup libraries before private byte audits. The
inner receipt also binds its exact native invocations and explicit corrected
reference. All 27 sources, six legacy PDFs, corrected PDF, table, historical
reports/receipts, code/helpers/tests/tools/environment/libraries, generated
inventories, three fresh outputs and both execution receipts agree before and
after. The 1,847 compact main-attempt references match the separately retained
full commands/record hashes; all 1,883 actual child records have consecutive
global numbers, successful exits and wait4 RSS measurements.

### Measured bounded resources and scope

Three native calls and 1,880 validators made 1,883 child launches; the
validators include 308 renders and the declared startup probes. Adding the
one invoking runner gives 1,884 aggregate launches, below 2,048. Converter
calls are zero. The measured `run()` phase, including final audits and report
persistence, completed in 225.210934 seconds against 1,800 seconds;
interpreter imports and pre-freeze public work are outside that timing scope.

Native wait4 peaks are HN-A 29,448 KiB, C8 31,684 KiB and HN-B 32,392 KiB.
Validators peak at 76,296 KiB; maximum 20-ms sampled child RSS is 76,400 KiB.
The outer harness self high-water mark is 47,232 KiB, separately measured
from child RSS. Full owned storage peaks at 129,663,823 bytes against 512 MiB.
The inner run peaks at 129,581,640 bytes and retains 79,436,428 bytes of
safe metadata and PDFs after successful sample/raster cleanup. Its streamed
child stdout totals 7,574,135,250 bytes; fixed full-pixel comparison buffers
are 131,072 bytes. These process/storage measurements are distinct from
checked native allocation and decoder accounting.

Native source/sink/scratch requests are at most 4,096 bytes. Maximum retained
page metadata is 1,632 bytes, accounted text working space 143,380 bytes and
temporary row storage 1,064,340 bytes. HN-B needs no row store. The additional
two source JPEG spools fetch 825,381 bytes in 203 requests, each at most 4,096;
all opaque audit/sample reads remain bounded at 65,536 bytes. Successful
decoded arrays and renders were removed. No private file bytes or table
values are committed.

The source revision's four hosted gates passed at
`8194264c048206b5f49b8c55714e06b6b1bd7cea`; its Rust LCOV report records
27,413/27,413 lines in 54 files, each at 100%. Examples are compiled/checked
and remain outside that standard coverage report. This measured three-source
diagnostic satisfies the declared corrected-reference compatibility scope;
it does not establish unseen HN variants, type-1/type-3 composition, HN-B
text/multiple-image rules, table distribution rights, outlines, production
CLI/JavaScript routing or a release. A vendor-application oracle remains a
separate validation strategy. Final evidence review and exact-head merge gates
remain separate.
