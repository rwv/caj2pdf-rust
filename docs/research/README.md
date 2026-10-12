# Research notes

The format investigations, oracle reports and validation runs behind the
decoders now live in the
[caj2pdf-samples repository](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes)
([#360](https://github.com/rwv/caj2pdf-rust/issues/360)), next to the scripts,
conformance harnesses and CAJViewer automation that produced them. They are
evidence, not user documentation: start with the [README](../../README.md) and
the [CLI reference](../cli.md). The project-wide record for this repository's
own source is [provenance](../provenance.md).

Source comments and older documents that cite `docs/research/<note>.md` mean
`research/notes/<note>.md` there; the [note index](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/README.md)
maps each note to the code it backs. The notes cited most often from this
repository's code are:

| Note | Backs |
| --- | --- |
| [CAJ container observations](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/caj-format.md) | `caj/`, `gb18030.rs`, CAJ tests |
| [KDH PDF wrapper observations](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/kdh-format.md) | `kdh.rs` |
| [Bounded HN/C8 container records](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/hnc8-container.md) | `hnc8.rs` reader |
| [Observed C8 native records](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/c8-native-records.md) | `hnc8/native.rs`, `hnc8/native_page.rs`, `hnc8/appinfo.rs` |
| [HN-A outline fields: observed profile](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/hnc8-outline-fields.md) | `hnc8/outline.rs` |
| [T.82 arithmetic core](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/t82-arithmetic-core.md) | `qm.rs` |
| [Bounded HN/C8 type-0 rows](https://github.com/rwv/caj2pdf-samples/tree/main/research/notes/jbig1-type0-rows.md) | `jbig1.rs` |

Notes and harnesses are pinned to the last caj2pdf-rust commit that carried
them, `0abee3862f01756ee15f69a1b174a35208fc1e41`. A new investigation goes to
caj2pdf-samples; a format fact that the code depends on is summarized in
[provenance](../provenance.md) with a link to its note.

The [2026-10-07 GitHub corpus validation](https://github.com/rwv/caj2pdf-samples/blob/043e52cd37389b3f903426cd0b8c6d7564aedf43/research/notes/github-sweep-fixes-20261007.md)
records all 1,277 post-fix native results, 50 remaining refusal classifications,
scoped runtime/fidelity evidence and explicit collection/test limits (#385).

The [973-original HN/C8/NH geometry report](https://github.com/rwv/caj2pdf-samples/blob/7dd99d385d334815c49afedcfe7929f5412bed8d/research/notes/source-image-geometry-20261008.md)
records source-declared page boxes and ordered image transforms/resource
identities for 16,548 accepted pages, with new NH bitmap evidence. Native
glyph/vector placement, complete rendered-page fidelity and unknown outlines
remain separate open obligations under #406.

The [60-page native composition checkpoint](https://github.com/rwv/caj2pdf-samples/blob/5a676fa58529249136b9b276e34ec466981067ac/research/notes/native-page-composition-20261008.md)
adds complete-page normal/marker-font observations and per-page text, vector,
image and font-state inventories for all ten native originals. It retains
all pixel differences, failed acquisition attempts and one normal-font cold
disagreement. This is scoped evidence under samples #51 and #441; complete
source fidelity remains open under #406.

The [retained native JPEG diagnostic](https://github.com/rwv/caj2pdf-samples/blob/0b67a13aa15854c9eb4b389dcb785aef9969b1c5/research/notes/native-page-jpeg-20261009.md) numerically maps the two
real-native page-5 rasters through the fixed JPEG profile. Historical API calls
were not observed; original-font fidelity and reliable viewer readiness remain
open. Old observations and all conversion totals are unchanged.

The [native vector follow-up](https://github.com/rwv/caj2pdf-samples/blob/4744a9360354d119facc20e7bd93d916afc54d24/research/notes/native-vector-geometry-20261008.md) verifies all 327 measured paths and
their order among glyph/image operations on those 60 pages. Original negative
controls, input/output hashes, process bounds and earlier checker refusals are
retained. That check excludes glyph placement, fonts, ornaments and raster fidelity.

The [native glyph/ornament follow-up](https://github.com/rwv/caj2pdf-samples/blob/82054fbe12e7a222a4e8e8a54f3694ee44361306/research/notes/native-glyph-model-20261009.md) verifies all 83,432 ordinary
glyphs and 212 ornament marks in both normal and original-marker PDF sets.
Positions, dimensions, shear, gray, clipping and complete paint-kind order match
existing measured models; diagnostic roles match original font programs and CID
maps. This fills the model-check gap while retaining original-font, ornament
appearance, raster, viewer, outline and remaining exception limits under #406.


The [TEB follow-up](https://github.com/rwv/caj2pdf-samples/blob/7fc1c5d4ce1b8c0b45fef68831149844a53fee14/research/notes/teb-container-boundary-20261009.md) verifies eight intact container inventories
and one publicly uploaded zero-filled suffix, correcting old framing and CRC
claims with original bounded controls. Readable metadata and valid stored-byte
checksums do not establish plaintext PDF recovery. Fresh offline viewer opens
and native/Node/Chromium refusals retain that boundary. #469 corrects the
unsupported encryption diagnosis; wrapping/credential semantics and conversion
remain open under #468 and #406.


The [TEB certificate follow-up](https://github.com/rwv/caj2pdf-samples/blob/5a9c67885df2d1edf58aa429b99d8891abbcd265/research/notes/teb-credential-boundary-20261009.md) establishes X.509 public-key and
opaque-field structure for eight complete sources, with bounded complete-payload
scans and original cryptographic/encoding controls. Actual key derivation,
validated credentials and whole-document recovery remain unknown under #468.
No private/vendor implementation or source values are imported.

The [normal caller-font subset audit](https://github.com/rwv/caj2pdf-samples/blob/42d7a54de99e66f099252d029893d9cce761328f/research/notes/native-font-subsets-20261009.md)
verifies used CID outlines/advances and PDF widths for all ten native normal
outputs: 6,140 resource/CID pairs across 83,644 draws. Generated TrueType/CFF
controls detect deliberate mutations. Only external caller fonts are compared;
original-viewer fonts, hinting, raster fidelity and #406 remain unresolved.

The [public page-buffer follow-up](https://github.com/rwv/caj2pdf-samples/blob/2cf840a94dd6cfd5b9b8e7414c7556692f19637b/research/notes/viewer-page-buffer-20261009.md)
locates both native variants in public Qt buffers and reproduces a difference
in an original PDF. Twelve retained sessions and seven API/PDF control groups
narrow the observation boundary; the corrected last-page crop and remaining
timing/readiness limits are explicit. No converter behavior or corpus count
changes.

The [font-free raster follow-up](https://github.com/rwv/caj2pdf-samples/blob/a69ac7caf21cb2f750d98734d1f7722712de906d/research/notes/viewer-raster-stages-20261009.md)
retains ten fresh sessions, including one aborted baseline. An original PDF
with no fonts/text operators differs by 3,655 buffer pixels; both displayed
variants recur without an observer (6,037 different pixels). Seven original
control tests pass. The measurements narrow font-only hypotheses without
establishing an internal cause, readiness, source-font fidelity or a new
conversion pass. #441/#406 and samples #51 remain open.

The [original native navigation controls](https://github.com/rwv/caj2pdf-samples/blob/0f608447ecf1df6eae204473fc31f77e5dad9779/research/notes/native-viewer-navigation-20261009.md)
reproduce completed-raster disagreement with no converter involved. An identical
route can also yield different results across fresh sessions. Original bounded
fixtures, preflighted routes and all observations are retained; #441 readiness,
PDF disagreement and broader #406 correctness remain open.

The [complete displayed-contents sweep](https://github.com/rwv/caj2pdf-samples/blob/449002c57e1694ec11813cd04ca97737f198ae6a/research/notes/viewer-outlines-20261009.md) covers all 849 C8/HN-B
originals under the fixed public Qt model/view protocol, with populated HN-A
controls before and after. Every selected model is empty; source identities,
three checkpoints, intervening samples and cleanup are verified. The report
retains preparation failures and original controls. #303 still needs a
positive original; stored-layout absence, rendering and #406 remain unresolved.


The [complete PDF-family profile audit](https://github.com/rwv/caj2pdf-samples/blob/d9016719872b394c88c5c67f616fbab699d9b100/research/notes/pdf-source-profile-proofs-20261009.md)
verifies selected-object, raw-stream and navigation scopes for all 279 accepted
PDF/KDH/CAJ originals (19,039 pages). The ten remaining profiles have individual
field proofs or complete source-body accounting; the earlier 269/10 checkpoint
and reader/geometry/recovery limitations remain visible. All 46 original
controls pass without corpus data or skips. See #480 provenance and samples
#65; no conversion pass or full visual-fidelity verdict is added, and #406
remains open.

The [complete missing-box page observations](https://github.com/rwv/caj2pdf-samples/blob/89c9f1a1720d88c0cccaf220ce2c1fa449b34617/research/notes/viewer-page-box-coverage-20261009.md)
extend the unchanged 75-page pair to two fixed orders: 133 equal, nine differing
and eight not-comparable source/output pairs. All 56 sessions, original controls,
wrong-page preflights and one viewer abort are retained. Unchanged-input repeats
also differ; #484 records this scoped evidence while #441/#406 remain open.

The [original RGB interpolation controls](https://github.com/rwv/caj2pdf-samples/blob/574f5e912af56177a15f756f94892461fd2df2e9/research/notes/viewer-rgb-resampling-20261009.md)
retain 18 original-only sessions and three analysis versions. Two exact RGB
variants differ at 71,650 buffer pixels and 79,837 displayed pixels, including
vector and image regions; no-observer sessions reproduce both. All 40
same-route flag-swap comparisons are equal. Earlier page-ID check rejections
remain recorded, and the final source-bit decoder never relaxes pixel equality.
See #486 provenance; internal cause, #441 readiness and #406 fidelity stay open.

The [original page JPEG transition report](https://github.com/rwv/caj2pdf-samples/blob/d106296558ea3cbc7a78320190da4f77c6f6d6ea/research/notes/viewer-jpeg-roundtrip-20261009.md)
connects original RGB/gray encoding input, JPEG bytes, returned decode rows and
selected display buffers through public APIs. It retains 96 diagnostic cases,
four native/PDF holdout directions and all 12 viewer sessions, including the
first observer's missing evidence and a separately frozen extension. A lossy
output counterexample prevents treating JPEG normalization as correctness.
See #488 provenance; the original #441 document, general readiness and broader
#406/native-source-font/ornament obligations remain open.

The [actual #441 page JPEG report](https://github.com/rwv/caj2pdf-samples/blob/d62394ff564be6001996b890edf1626b7013eada/research/notes/viewer-original7797-jpeg-20261009.md)
reproduces both historical source/PDF crops and links their pre-/post-JPEG values
through real-document public API observations. All seven observed pre-encoding
target values agree; twelve sessions retain one abort and the first collector's
corrected time-field assumption. The scoped #439 receipt is updated. See #490
provenance; general readiness, all-page original-viewer coverage, source fonts/
ornaments and broader #406 correctness remain open.


The [historical Windows CAA report](https://github.com/rwv/caj2pdf-samples/blob/f3cdf05e29bb2ac2627be3631e0309392c6194b0/research/notes/caa-windows-boundary-20261009.md) records all 18 originals under
a pinned offline Wine environment, with two-page PDF and invalid-CAA controls.
All 54 source checkpoints contain network/server/error keywords. Download,
build, MDAC installation and CJK display limits remain visible; no target,
opaque field or vendor implementation is inspected. See the provenance entry
for samples #79. Actual-document availability, credentials and native-Windows
equivalence remain unverified; #406 and the conversion ledger are unchanged.


The [public-web source verification](https://github.com/rwv/caj2pdf-samples/blob/9bbb9f5ddfe1cd340d2f0f758a0104a4e1e95022/research/notes/public-web-correctness-20261009.md) covers all four new HN/C8 originals
(203 pages / 236 images), the 20-page KDH, both new C8 displayed-contents models
and all 11 compressed streams behind #499's seven source-content warnings.
It supplements the 87-original conversion/runtime receipt while retaining
unknown outlines, missing content and broader #406 fidelity obligations.

The [public TTKN response sample](https://github.com/rwv/caj2pdf-samples/blob/e5099249de1ff39a1e11f16bf047371ff1eb893a/research/notes/public-ttkn-response-20261009.md)
adds one protected identity with native/Node/Chromium refusal and controlled
offline viewer observations. The matching author-disclosed response produces
a rights-file error in a diagnostic copy, without demonstrating recovery or
irrecoverability. The report preserves controls, failed web/Wayback requests
and the distinction between this copy and the unchanged source (#501/#415).


The [TTKN observed-IV research recovery](https://github.com/rwv/caj2pdf-samples/blob/e4f54a1d3645983ea842a58298efb7aedbfd1b08/research/notes/ttkn-unaltered-wrapper-20261009.md) preserves the unchanged
source during loopback-only replay, reproduces both wrapper layers and
establishes the PDF-key recipe for this source. All 234 independently decrypted
stream byte sequences match qpdf; 180 pages render and 97 outline destinations
are valid. Initial-IV initialization, production three-runtime support and
complete viewer equivalence remain unmet. The earlier URL-copy failures stay
historical evidence; no conversion totals or format support claims change.

The [Wine native font-resource follow-up](wine-font-resources-20261011.md)
identifies Huaguang Song/Hei/Kai and system Song selection, separates mode-2
Latin aliases from mode-0 Unicode queries, and verifies all 13 native documents
with measured caller fonts. It preserves remaining extent/pixel differences
and the viewer startup workaround; #541 remains open.

The [Wine ornament-spacing controls](wine-ornament-spacing-20261011.md)
provide an original reproducible C8/font fixture and 43 session receipts.
They isolate zoom-dependent integer repetition from font advances, retain
rejected PDF encodings, and leave production geometry unchanged.


The [CJK character-mode alias follow-up](wine-cjk-aliases-20261011.md) records
760 font queries, six real-font before/after improvements with exact source
repeats, and eleven negative geometry/PDF controls. Independent ActualText
keeps semantic characters intact; all 77 actual PDF pages remain byte-identical.
The original generator reproduces thirteen controls without external inputs.

The [original-font production comparison](wine-original-font-production-20261011.md)
validates bounded native/CLI/Node/browser font transport and explicit ordinary
aliases against all 77 Wine pages, including mode 0, with cold repeats and
independent glyph/semantic checks. Remaining pixel differences stay open.

The [mode-0 page-padding follow-up](wine-mode0-page-padding-20261011.md)
separates the ten-point page margin from the raw coordinate unit. Ten boundary
comparisons, including a held-out size, match Wine; four actual page rasters
remain unchanged at 75%. The eight-source generator, 61-session receipt and
26 native/Node byte comparisons preserve unresolved glyph-position hypotheses.

The [mode-0 font-metric and rounding controls](wine-mode0-font-rounding-20261011.md)
exclude vertical-metric changes and four equivalent PDF encodings as fixes.
Across seven zooms, 156 relative CJK/Latin measurements support separate device
rounding of the existing ten-unit offset. High-zoom and positive font controls
retain the limits of that inference; production geometry remains unchanged.

The [axis rasterization and character-origin study](wine-axis-rasterization-20261011.md)
records 73 synthetic Wine sessions, four exact cold repeats and negative
cross-zoom controls. Identical geometric glyphs isolate code-dependent Latin
origins. Original source/font/PDF reproducers retain rejected partial cells
and the limits of zoom-specific raster predictions.

The [mode-2 digit-origin correction](wine-mode2-digit-origins-20261011.md)
separates A0 digits from alphabet placement. All 316 measured digit cells improve;
three actual pages improve and 74 retain their prior pixels. The 59-session receipt
retains one rejected information-bar capture, runtime parity and unresolved
alphabet origins. Seventeen original source fixtures reproduce exactly.

The [mode-2 alphabet-origin correction](wine-mode2-alphabet-origins-20261011.md)
establishes integer character/size offsets using 1,612 geometric observations,
including 624 held-out predictions. All 880 selected production letter cells
improve. The existing 77-page cohort has 37 improvements, 40 unchanged rasters
and no regressions; remaining device rounding and raster differences stay open.
The 61-session receipt retains failed hypotheses, rejected captures and full
CLI/Node/Chromium parity. Nine original source controls reproduce exactly.

The [Type 3 rasterization controls](wine-type3-rasterization-20261011.md)
exclude further character-procedure encodings as a correction for the tested
Wine frames. Nested `d0` text retains baseline pixels; `d1` nested text is blank
despite visible rectangle controls. All 28 original-outline path frames regress.
The 62-session receipt and 32-PDF reproducer retain missing glyphs, a rejected
frame comparison and an interrupted observer resumed without restarting Wine.

The [grayscale JPEG and interpolation controls](wine-jpeg-decode-controls-20261011.md)
retain 17 Wine sessions and 51 complete page visits. Full-resolution decoded
Flate, explicit interpolation and their combination leave all 27 treatment
frames identical to their PDF baselines across three zooms. An inverted-image
control changes all three selected pages. Source differences remain, with no
production change or general claim about renderer behavior.

The [form and transparency controls](wine-transparency-controls-20261011.md)
retain 38 cold Wine sessions. Ordinary forms, isolated groups and equivalent
origin/axis decompositions preserve all 35 baseline frames across seven views.
A separate half-opacity control changes the frame but loses visible ink in
31 of 42 glyph cells. All 21 diagnostic PDFs reproduce exactly; neither
alternative supplies a production fidelity correction.

The [vertical CID origin controls](wine-vertical-origins-20261011.md) distinguish
responses to per-CID and default metric encodings in 35 Wine sessions. Responsive
range/default equivalents retain baseline pixels; four array-based equivalents
regress. Two cold source repeats and all 29 output reproductions are exact.
The observer's initial cell-boundary failure and deliberately displaced controls
remain separate from complete-frame fidelity; no production fix is adopted.

The [public GDI relay controls](wine-gdi-relay-controls-20261011.md) retain twelve
cold Wine sessions and independent API logging probes. All frames equal their
unobserved references. Four initial logs truncate; eight filtered traces retain
every selected record. Source/PDF final bitmap dimensions match at four views,
but glyph-query APIs seen in the positive probe are absent from the selected
viewer traces. This narrows the observation boundary without identifying an
internal renderer or fixing the remaining source/PDF pixel differences.

The [native image endpoint study](wine-image-endpoints-20261011.md) records
79 cold Wine sessions and 93 admitted visits. Original black grids isolate
inclusive native endpoint sizing on 132 axes. Target-zoom extent diagnostics
make six grid views exact and improve nine selected actual views, but two
cross-zoom controls regress. No production fix is adopted; #543 tracks the
remaining general endpoint/resampling correction under #406.

The [image-route and sampling study](wine-image-routes-sampling-20261011.md)
adds 51 cold sessions / 65 admitted visits. Tested patterns lose ink; target
extents plus reversed sample rows make five original control views exact.
Actual-page and cross-zoom counterexamples prevent a general production fix.
The subsequent study below independently varies descriptor orientation and clipping.

The [JPEG descriptor and signed-clipping study](wine-image-signed-clipping-20261011.md)
adds 97 cold sessions / 133 admitted visits. Descriptor-only changes make all
five type-2 in-page views exact with target extents alone. Independent signed
controls isolate residuals to negative positions; every nonnegative cell is
exact after the declared grid corrections. Signed rounding/origin carry and
zoom-independent integer-pixel clipping retain actual-page regressions.
Six source/current-PDF pairs and 56 diagnostics reproduce exactly. #543 remains
open; no production rendering change is adopted.

The [device-coordinate clipping study](wine-image-device-clipping-20261011.md)
adds 88 cold sessions / 88 admitted visits. Paired grayscale controls isolate
clipped sampling; device-derived pixel windows, vertical-origin carry and an
adjacent-row diagnostic make ten whole gray/RGB control views exact at declared
zooms. Held-out 125/175% failures and numeric-bound regressions remain.
Both new source/current-PDF pairs and 64 diagnostics reproduce exactly.
Research JPEG/mask admission checks are corrected with positive and negative
controls. #543 remains open for a general production correction.

The [image Form and direct-clipping study](wine-image-forms-20261011.md)
adds 44 cold sessions / 68 admitted visits. Tight ordinary Forms remove a few
gray and mixed-page edge differences; direct unit-square clipping reproduces
all ten corresponding frames without Forms. Tight transparency groups regress
at 100%, while image-only pages and black endpoint sizes remain unresolved.
All 30 diagnostic PDFs reproduce exactly, with eight exact cold repeat visits.
This supports further evaluation of clipping, not a general rendering fix;
#543 and the production fidelity obligations remain open.

The [full-cohort direct-clipping study](wine-image-clip-cohort-20261011.md)
adds fresh native and candidate views of all 77 pages. Five improve, 71 retain
pixels and one regresses, rejecting unconditional clipping. The counterexample
changes from regression at 75% to improvement at 95%; an equivalent
page-coordinate clip reproduces the same pixels. All 31 new comparison PDFs
reproduce, and all 77 fresh native crops equal the historical crops. The
85-session receipt admits 230 visits, retaining twelve incomplete captures and
six unobserved startup-failure visits separately. #543 remains open; no
production correction is adopted.
