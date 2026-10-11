# Wine image drawing routes and sample orientation

Matching the measured device extent and reversing decoded image rows with a
compensating PDF transform makes **two complete original grayscale grids and
three asymmetric-color views exact** against Wine. This separates an observable
sampling-direction effect from the previously measured endpoint-size effect.
However, the same combination regresses on all nine selected actual-document
views relative to the extent-only diagnostic. Reversing rows alone also
regresses two actual views and both gray-grid views. **No production correction
is adopted.**

This continues [#543](https://github.com/rwv/caj2pdf-rust/issues/543) under #406,
related to #541, from the [endpoint study](wine-image-endpoints-20261011.md).
Production remains `7f8b7e75e4a5a13dd041011b4ddc759a2b288b31`; all 77 existing
production cohort pages still differ. The
[receipt](wine-image-routes-sampling-20261011.json) retains every declaration,
admitted visit, comparison, failed model, graph invariant and reproduction.
Fonts, documents, PDFs and screenshots remain external.

## Reference and admission

The unchanged reference is CAJViewer 7.3.151 Simple on Wine 8.0, the original
resource-font profile, 96 DPI and a 1600×1200 display. The pinned existing
container/harness retains network isolation, read-only root, 2 GiB memory,
two CPUs, 256 processes and a 600-second session bound.

| Stage | Cold sessions | Admitted visits |
| --- | ---: | ---: |
| Clip, pattern and hairline pilot | 16 | 16 |
| Patterns in page coordinates | 3 | 3 |
| Original gray ramp/stripe controls | 7 | 7 |
| Sample-order and matrix-sign pilot | 11 | 13 |
| Other zooms, combined diagnostics and repeats | 14 | 26 |
| Total | **51** | **65** |

All visits are complete, stable and numeric-confirmed; every input and isolated
copy retains its hash. All 51 containers are removed with no OOM. There are
57 comparisons, two new native gray-grid views and six exact cold-repeat views:
the black grid at 75%, gray grid at 75% and 150%, and actual pages 1/5/10 at
100%. No GUI frame is rejected or retried. Existing admitted source/PDF
references are reused explicitly. These are not new 77-page cohort results.

Comparisons use unchanged complete RGB page crops, including the border,
without alignment, rescaling or tolerance. Interior-ink diagnostics exclude
only the fixed five-pixel page-border band; full-frame scoring retains it.

## Geometry-preserving paths that did not help

Reviewing the previous rectangle-fill controls shows that all **66 panel
bounds already equal the native bounds**, but every rectangle has gray edge
pixels absent from the native black rectangles. Correct bounds therefore did
not establish equal coverage or color.

The [pattern helper](../../scripts/research/wine_image_pattern_diagnostic.py)
tests a unit clip, colored tiling patterns of types 1/2/3, separated cells,
patterned hairline perimeters, and black rectangle hairlines with/without
stroke adjustment. Pattern matrices retain the original image transform.
A second construction omits the pattern matrix and puts the same transform
inside a cell expressed in page coordinates. Gray path-only pattern cells
are deliberate positive controls. These are original uses of the
[PDF Reference's pattern, matrix and hairline definitions](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.5_v6.pdf),
not implementations copied from a viewer. Hairlines deliberately add
device-dependent strokes; they are not equivalent fill-area claims.

At 75%, both clipped images reproduce baseline pixels. Both black hairline
variants regress from 659 to 2,069 differing pixels and are mutually identical.
**All 14 pattern views lose all interior ink**, including both gray path
positives. Black-grid differences become 22,887 and asymmetric-color differences
become 35,818. Changing pattern spacing/type or moving the matrix into the
cell does not restore painting in this scope. This does not prove a universal
absence of pattern support in the viewer.

Independent MuPDF 2× renders paint the patterns and preserve baseline pixels
for ordinary clip/pattern variants. The patterned RGB hairline changes pixels,
and the gray positives respond. These are secondary construction checks,
not Wine compatibility passes. The initial helper preparation failed when
slicing a pikepdf array directly; converting it to a list fixed the error
before any Wine run or PDF output. The failed directory is retained.

## Original nonuniform grayscale controls

The [source generator](../../scripts/research/wine_image_sampling_controls.py)
creates a 3000×2600-unit C8 page with twelve grayscale JPEG panels. Two columns
contain a linear ramp and seven irregular black/white bands. Six rows use
33, 65, 129, 257, 513 and 1025 square source pixels, while all destination
extents remain 1113×200 raw units. Black outer source-pixel borders expose
vertical sampling. JPEG quality is 100; the decoded values, rather than an
assumption of lossless JPEG encoding, are used for model comparisons.

Native source, current PDF and the earlier extent-only diagnostic are observed
at 75% and 150%. The latter still deliberately changes physical geometry for
one declared zoom; it is not proposed as a fixed PDF correction.

| Zoom | Current PDF differing pixels | Extent-only differing pixels |
| --- | ---: | ---: |
| 75% | 9,876 | 1,347 |
| 150% | 25,215 | 3,006 |

Using the previously measured endpoint rule to select windows, **all 24 native
and extent-only central horizontal rows are identical**, comprising 3,912
samples. The three central rows of every observed panel are mutually equal.
Remaining extent-only differences concentrate near the top/bottom source
borders. No screenshot-derived alignment is applied.

The receipt retains 56 model scores for each observed row: JPEG draft factors
1/2/4/8 combined with Pillow nearest/bilinear/box/bicubic/Lanczos resizing,
three nearest-coordinate conventions, and three linear-coordinate conventions
with floor/nearest output rounding. For source width `N` and destination width
`M`, coordinate families are `i*N/M`, `(i+0.5)*N/M-0.5`, and
`i*(N-1)/(M-1)`, clamped to source bounds. Draft requests use integer-divided
dimensions and record the actual decoded size. The model set is specified
before inspecting these measured rows; no fitted coefficients are introduced.

Full-resolution nearest sampling at `floor(i*N/M)` matches ten native rows
exactly, covering the horizontally enlarged cases. It has 210 differing
samples across all 3,912 native samples. **No tested model explains all 24
native rows.** Neither a general JPEG draft rule nor an internal viewer
decoder/resampler is established. The stronger direct observation is that
native and extent-only horizontal sample rows already agree.

## Compensating sample-order reversal

The [orientation helper](../../scripts/research/wine_image_orientation_diagnostic.py)
decodes each ordinary 8-bit gray/RGB JPEG at full resolution, bounded to ten
million pixels per image. It retains the original JPEG resource and paints a
separate lossless-Flate resource. The identity control changes only encoding.
An axis diagnostic reverses its decoded sample order and compensates the
axis-aligned matrix exactly:

```text
original: (a, 0, 0, d, e, f)
X flip:   (-a, 0, 0, d, e+a, f) with reversed columns
Y flip:   (a, 0, 0, -d, e, f+d) with reversed rows
```

Exact rational checks preserve the mathematical image support and oriented
decoded pixel field. Original font programs, page boxes and every nonimage
operator remain unchanged; qpdf accepts every output. Pixel identity here is
inverse-transform identity of Pillow-decoded samples, not a promise that all
renderers choose identical JPEG decoding or rasterization. Independent MuPDF
1× comparisons, including their differences, are retained in the receipt.

All three observed Flate identity views retain Wine baseline pixels. X reversal
regresses both gray views to 13,419 and 41,091 differing pixels. Y reversal
alone also regresses them, to 11,600 and 25,622. However, **Y reversal after
the declared extent correction makes both entire gray-grid frames exact**,
not merely their central rows. It also makes all three asymmetric RGB
extent-corrected views exact at 50/75/100%. This establishes a reproducible
interaction between destination sizing and sample order in these controls.

Reusing the combined gray PDF prepared for 75% at 150% leaves 23,376 differing
pixels; preparing at 150% and viewing at 75% leaves 1,631. Those improve on
their current-PDF baselines but lose exact equality. The target-scale geometry
correction still does not generalize across zooms.

## Actual-page and sign/clipping counterexamples

For unchanged actual source
`7e3f2c0faebb8ddac2f4feff3cfc9249476ab170b43250ac00610b10aa46c74b`,
pages 1 and 5 are image-only and page 10 is mixed. Unlike the new original
controls, these images have negative vertical matrix coefficients; some also
cross the page boundary. These differences are observations, not a demonstrated
explanation for the counterexamples.

| Zoom / page | Current PDF | Y reversal only | Extent only | Extent + Y reversal |
| --- | ---: | ---: | ---: | ---: |
| 50% / 1 | 60,368 | 52,992 | 58,014 | 65,982 |
| 50% / 5 | 83,964 | 74,656 | 81,330 | 88,411 |
| 50% / 10 | 52,162 | 51,285 | 26,270 | 37,869 |
| 75% / 1 | 128,982 | 116,289 | 116,594 | 125,686 |
| 75% / 5 | 183,080 | 169,948 | 171,054 | 180,104 |
| 75% / 10 | 78,226 | 61,095 | 18,417 | 45,590 |
| 100% / 1 | 183,148 | **185,656** | 177,231 | 214,428 |
| 100% / 5 | 284,197 | **286,111** | 275,183 | 313,234 |
| 100% / 10 | 163,116 | 116,735 | 56,812 | 107,170 |
| Total | **1,217,243** | **1,114,767** | **980,905** | **1,178,474** |

Y reversal alone improves seven views and regresses two. Combining it with
the target extent regresses **all nine** relative to the extent-only diagnostic.
Every actual view still differs. Aggregate improvement cannot erase these
counterexamples or justify a default conversion change.

The next discriminator is an independently authored signed-height/negative-
origin source control, with image contents away from clipped edges. It must
separate direction from clipping and test integer rounding across zero.
The all-positive, in-page black-grid endpoint model does not establish how
negative coordinates are rounded. The actual page alone cannot supply that
rule without confounding the two properties.

## Reproduction and remaining acceptance

The three public helpers are independently authored MIT research code using
project-owned C8 framing facts and installed Pillow/pikepdf/zlib/qpdf APIs.
No vendor, foreign converter or private HN/JBIG implementation is inspected
or migrated. They add no production dependency or I/O path. Original fonts
remain unchanged. The new source fixture, its twelve JPEGs and current-CLI PDF
reproduce byte-for-byte. All **32** pattern/orientation diagnostic PDFs and
their metadata also reproduce byte-for-byte with deterministic IDs. Final
helper formatting preserves the tested AST. No observed file or failed result
is replaced.

Source inventory, helper syntax, source/PDF reproduction, admission/source-
integrity checks, receipt aggregates and local documentation links are checked.
Native/JS production tests and the 77-page sweep are not rerun without an
implementation change. Preceding commit `d75703e` completed four quality jobs
and nine regular native jobs, with 15 successful checks including inputs/assets;
17 optional/release skips are excluded. Publication-head CI is tracked in
draft #542. General image fidelity, broader font/placement fidelity and final
review remain open under #543/#541/#406. No merge or release is included.
