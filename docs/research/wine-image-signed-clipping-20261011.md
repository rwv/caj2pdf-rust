# Wine JPEG descriptor orientation and page-boundary clipping

Changing only the JPEG descriptor kind in original in-page controls isolates
the previously confounded orientation effect: **all five type-2 views become
exact with the declared endpoint extents alone**. Reversing their rows adds
differences. Independent negative-origin controls then localize remaining
differences to clipped cells. Neither signed endpoint rounding nor a
zoom-independent decoded-pixel crop supplies a general correction. The latter
regresses four actual views under every tested rounding rule. **Production is
unchanged.**

This continues [#543](https://github.com/rwv/caj2pdf-rust/issues/543), under #406
and related to #541, from the [sampling study](wine-image-routes-sampling-20261011.md).
The [receipt](wine-image-signed-clipping-20261011.json) retains every declaration,
admitted observation, comparison, cell diagnostic and reproduction hash.
Production remains `7f8b7e75e4a5a13dd041011b4ddc759a2b288b31`; all 77 existing
production cohort pages still differ. No new cohort or runtime sweep is claimed.

## Reference and admission

The unchanged reference is CAJViewer 7.3.151 Simple on Wine 8.0 with the original
resource-font profile, 96 DPI and a 1600×1200 display. Existing isolated
containers retain a read-only root, no network, 2 GiB memory, two CPUs,
256 processes and a 600-second bound. No driver or readiness rule changes.

| Stage | Cold sessions | Admitted visits |
| --- | ---: | ---: |
| Descriptor-only controls and repeats | 27 | 27 |
| Independent negative-origin source/PDF controls | 16 | 16 |
| Signed endpoint hypotheses on controls | 24 | 24 |
| Signed endpoint hypotheses on actual pages | 9 | 27 |
| Decoded-pixel clipping at several zooms | 21 | 39 |
| Total | **97** | **133** |

All visits are complete, stable and numeric-confirmed. All 97 inputs and
isolated copies retain their hashes, all containers are removed, and none
is OOM-killed. There are 118 comparisons, thirteen new native source views
and two exact cold repeats (type-2 gray at 150% and asymmetric RGB at 100%).
No GUI frame is rejected or retried. Earlier admitted actual/source references
are reused explicitly. Scores compare whole RGB page crops including borders,
without alignment, rescaling or tolerance; cell strips are separate diagnostics.

## Descriptor kind isolated from clipping

The project's existing `decode_native_image_coordinate` reads unsigned raw
image extents; the page origin can make relative positions negative.
Composition separately selects row orientation from JPEG descriptor kinds
1 and 2. These are facts from the project's original MIT implementation,
not a signed-height encoding inferred from PDF matrices.

The [original source generator](../../scripts/research/wine_image_signed_controls.py)
recreates the preceding gray grid and asymmetric RGB control, then changes
only each descriptor's kind byte from 1 to 2. All twelve gray and one RGB
JPEG payloads, page dimensions and unsigned image boxes remain identical.
The changed byte offsets are recorded. Every image is entirely in-page.

| Type-2 view | Current PDF | Y reversal | Extent only | Extent + Y reversal |
| --- | ---: | ---: | ---: | ---: |
| Gray 75% | 11,606 | 9,875 | **0** | 1,347 |
| Gray 150% | 25,736 | 25,150 | **0** | 3,006 |
| Asymmetric 50% | 625 | 356 | **0** | 319 |
| Asymmetric 75% | 937 | 533 | **0** | 383 |
| Asymmetric 100% | 1,394 | 1,931 | **0** | 510 |

The corresponding type-1 controls previously needed extent correction plus
compensating Y reversal to become exact. The type-2 controls already use a
negative vertical PDF matrix and need no extra reversal. Descriptor kind is
therefore an independent discriminator for these measured in-page controls.
All exact views still depend on physical extent changes at a declared zoom;
the retained cross-zoom counterexamples prevent a production-fix claim.

## Independent negative-origin controls

Four original 3900×3900-unit pages vary the clipped axis (x/y) and descriptor
kind (1/2). Each contains nine copies of the same 256×192 asymmetric RGB JPEG.
Along the tested axis, positions are `-123, -7, -1, 0, 1, 7, 123, 1484, 1484`.
The first eight extents are 1113 units and the last is 1112; the other extent
is 280. Panels are separated by 400 units along the other axis. The last two
positions are fully in-page independent references. Native header origins
4652/4274 keep all encoded coordinate fields valid unsigned values.

Every source and its current PDF is observed at 75% and 150% before applying
diagnostic corrections. The
[signed-grid helper](../../scripts/research/wine_image_signed_grid_diagnostic.py)
tests three predeclared endpoint hypotheses: floor with original affine
phase, truncation toward zero with that phase, and truncation with an
origin carry derived from truncation minus floor. Type-1 PDFs first receive
the previous compensating sample-row reversal to use negative vertical matrices.

For unit scale `u = 72/742` PDF points and device scale `q = zoom/75`, recover
integer source `(x,y,w,h)` from the current image matrix. A maximum error of
`1e-5` raw units admits the current six-decimal MediaBox serialization;
**all 216 recovered rectangles exactly equal their independent source
declarations**. Coordinates are not fitted from screenshots. For quantizer `Q`:

```text
x0 = Q(x*u*q)          x1 = Q((x+w)*u*q) + 1
y0 = Q(y*u*q)          y1 = Q((y+h)*u*q) + 1
a' = (x1-x0)/q         d' = sign(d)*(y1-y0)/q
dx = (Q(x*u*q) - floor(x*u*q))/q   # carry variant only
dy = (Q(y*u*q) - floor(y*u*q))/q   # carry variant only
e' = e + dx
f' = f + max(0,d) - max(0,d') - dy
```

The other variants use zero `dx,dy`. These are target-scale diagnostics that
deliberately change physical extents and sometimes origins, not equivalent
PDF geometry. JPEG payloads, original fonts, page boxes and nonimage operators
are checked unchanged at this stage; qpdf accepts every output.

| Control / zoom | Current PDF | Floor / keep | Truncate / keep | Truncate / carry |
| --- | ---: | ---: | ---: | ---: |
| x type 1 / 75% | 1,785 | 216 | 26 | 91 |
| x type 1 / 150% | 3,076 | 264 | 99 | 250 |
| x type 2 / 75% | 4,391 | 216 | 26 | 91 |
| x type 2 / 150% | 6,508 | 264 | 99 | 249 |
| y type 1 / 75% | 997 | 126 | 201 | **0** |
| y type 1 / 150% | 2,204 | 287 | 482 | 80 |
| y type 2 / 75% | 1,175 | 55 | 231 | **0** |
| y type 2 / 150% | 2,935 | 249 | 443 | 29 |

All **144 nonnegative-position cell comparisons are exact**, under all three
hypotheses. Every remaining cell difference occurs at a negative position.
For x/truncate/keep, both descriptor kinds leave only 26 pixels at position
-123 at 75%; at 150% they leave 50 at -123 and 49 at -7. Position -1 is exact.
For y/truncate/carry, both complete 75% frames are exact; at 150%, only -123
remains different (80 pixels for type 1, 29 for type 2).

Residuals thus appear in these controls after clipping exceeds a whole device
pixel. This localizes the next sampling investigation; it does not establish
a universal native clipping order or one rounding rule shared by both axes.

## Actual-page counterexamples to signed rounding

The unchanged actual source is
`7e3f2c0faebb8ddac2f4feff3cfc9249476ab170b43250ac00610b10aa46c74b`.
Pages 1/5 are image-only and page 10 is mixed, observed at 50/75/100%.
All their image matrices already have negative vertical coefficients.
The same three hypotheses are prepared from source/PDF geometry before
observing results, with no per-document fitted offset.

| Zoom / page | Current PDF | Floor / keep | Truncate / keep | Truncate / carry |
| --- | ---: | ---: | ---: | ---: |
| 50% / 1 | 60,368 | 58,014 | 60,368 | 56,531 |
| 50% / 5 | 83,964 | 81,330 | 83,964 | 80,727 |
| 50% / 10 | 52,162 | 26,270 | 26,270 | 26,270 |
| 75% / 1 | 128,982 | 116,594 | 121,295 | 114,157 |
| 75% / 5 | 183,080 | 171,054 | 176,313 | 170,191 |
| 75% / 10 | 78,226 | 18,417 | 18,417 | 18,417 |
| 100% / 1 | 183,148 | 177,231 | 183,148 | **186,534** |
| 100% / 5 | 284,197 | 275,183 | 284,197 | **289,599** |
| 100% / 10 | 163,116 | 56,812 | 56,812 | 56,812 |
| Total | **1,217,243** | **980,905** | **1,010,784** | **999,238** |

Floor/keep reproduces the previous extent-only results. Truncate/keep improves
five views and leaves four unchanged. Truncate/carry improves seven but
regresses both 100% image-only views. Every actual frame still differs;
aggregate improvement does not remove those counterexamples.

## Zoom-independent decoded-pixel clipping

The [pixel-clip helper](../../scripts/research/wine_image_pixel_clip_diagnostic.py)
tests a different hypothesis: crop decoded source pixels to the visible page
intersection before resizing. It accepts ordinary 8-bit gray/RGB JPEGs bounded
to ten million pixels per image. Fully in-page image draws remain unchanged.
For a clipped image, it computes the exact rational source-pixel window from
the mathematical image/page intersection, then independently tries floor,
ceil and nearest (`floor(v+1/2)`) for its edges. No zoom argument exists.

The cropped samples are painted as a lossless-Flate image stretched to that
intersection, with negative vertical matrix direction. Original JPEG resources
remain embedded. Exact arithmetic verifies the visible support; decoded-pixel
hashes, original fonts, page boxes and nonimage operators are checked. However,
rounding the pixel window changes the sampled field: **support preservation
does not establish content equivalence**. This is a hypothesis, not an assumed
viewer implementation. The same nine PDFs are reused at all declared zooms.

The x/y type-2 source controls are observed at 75/150%. Of their twelve
comparisons, only x/floor/150% improves (6,508 → 6,446); seven regress and four
are unchanged. No control frame is exact. For the actual pages:

| Mode | Before pixels, nine views | After pixels | Improved | Unchanged | Regressed |
| --- | ---: | ---: | ---: | ---: | ---: |
| Floor | 1,217,243 | 1,257,448 | 2 | 3 | 4 |
| Ceil | 1,217,243 | 1,249,254 | 2 | 3 | 4 |
| Nearest | 1,217,243 | 1,261,645 | 2 | 3 | 4 |

Every mode improves image-only pages 1/5 at 75%, regresses them at 50/100%,
and preserves all three mixed-page baselines. None is a general correction.
This rejects these specific integer-pixel-crop constructions as complete fixes;
it does not rule out every clipping/resampling interaction. Endpoint-size
differences remain in this experiment because no target-scale extent correction
is applied to its outputs.

## Reproduction, provenance and remaining work

All six original sources, six current baseline PDFs and **56 diagnostic PDFs**
reproduce byte-exactly with deterministic IDs; diagnostic metadata also agrees.
The two published PDF helpers retain function ASTs identical to the observed
prototypes and add command-line entry points. The source generator recreates
the unchanged gray/RGB patterns and the independently declared negative-origin
controls using adjacent project-owned MIT helpers.

An initial reproduction wrapper used `zoom` instead of the existing extent
helper's `target_zoom` metadata field and stopped with `KeyError`. Its partial
directory and log are retained. The corrected wrapper reproduced everything
in a new directory; no Wine input or observation was changed or retried.

Per-file provenance is original MIT work. No vendor implementation, foreign
converter or private HN/JBIG source is inspected or migrated. Sources, fonts,
PDFs and screenshots remain external. The helpers are research tools, separate
from production bounded I/O; no production dependency or API changes.

#543's general geometry-preserving correction, applicable bilevel boundaries,
actual-cohort nonregression, implementation/runtime checks and final review
remain outstanding. Descriptor direction is now independently controlled;
the next useful discriminator is clipped nonuniform sampling after endpoint
size and orientation are matched, with independent phases/resolutions and
cross-zoom counterexamples retained. #543/#541/#406 remain open. Closed #534
still denotes completed unresolved classification, not pixel fidelity.
